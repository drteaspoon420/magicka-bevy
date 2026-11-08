//! Read-only validation of user-owned decoded collision data.
use magicka_content::{level::AnimatedLevelPart, parse_primary, Content};
use magicka_runtime::{
    force_field::force_field_collision,
    level_collision::{collision_mesh, StaticLevelCollision},
    live_collision::LiveCollisionScene,
    presence::PresenceAreas,
};
use std::path::{Path, PathBuf};
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, out)
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xnb")) {
            out.push(p)
        }
    }
}
fn animated(part: &AnimatedLevelPart, count: &mut usize, errors: &mut Vec<String>, path: &Path) {
    if let Some((_, mesh)) = &part.collision {
        *count += 1;
        if let Err(e) = collision_mesh(mesh) {
            errors.push(format!("{} animated {}: {e:?}", path.display(), part.name));
        }
    }
    for child in &part.children {
        animated(child, count, errors, path)
    }
}
#[test]
fn retail_level_collision_meshes_construct() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    let mut paths = vec![];
    walk(&root.join("Levels"), &mut paths);
    paths.sort();
    let (mut levels, mut fields, mut parts) = (0, 0, 0);
    let mut errors = vec![];
    let mut areas = 0;
    for path in paths {
        let bytes = std::fs::read(&path).unwrap();
        let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
        let Some(parsed) = parse_primary(&xnb) else {
            continue;
        };
        let parsed = parsed.unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if let Content::Level(level) = parsed.content {
            levels += 1;
            areas += level.trigger_areas.len();
            if let Err(e) = PresenceAreas::from_content(&level.trigger_areas) {
                errors.push(format!("{} trigger areas: {e:?}", path.display()));
            }
            match LiveCollisionScene::load(&level, &magicka_content::skinned::IDENTITY) {
                Ok(mut live) => {
                    if let Err(e) = live.update(0.0, &magicka_content::skinned::IDENTITY) {
                        errors.push(format!("{} live update: {e:?}", path.display()));
                    }
                }
                Err(e) => errors.push(format!("{} live load: {e:?}", path.display())),
            }
            if let Err(e) = StaticLevelCollision::from_layers(&level.collision_layers) {
                errors.push(format!("{} static: {e:?}", path.display()));
            }
            for (i, field) in level.force_fields.iter().enumerate() {
                fields += 1;
                if let Err(e) = force_field_collision(field) {
                    errors.push(format!("{} field {i}: {e:?}", path.display()));
                }
            }
            for part in &level.animated_parts {
                animated(part, &mut parts, &mut errors, &path)
            }
        }
    }
    eprintln!("collision corpus: {levels} levels, {fields} force fields, {parts} animated collision meshes; {} failures",errors.len());
    eprintln!("presence areas constructed from {areas} named areas plus {levels} global any areas");
    assert!(levels > 0);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
