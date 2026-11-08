//! Export level scenes as glTF binary (.glb).
//!
//!     level-gltf <content-dir> <textures-dir> <out-dir> [--missing] [--manifest <file>] (--all | <level.xnb>...)
//!
//! Each scene goes to `<out-dir>/<asset>.glb`; textures are referenced by
//! relative URI into `<textures-dir>` (from `tex-export`). Prints the
//! automated self-check statistics (winding vs. stored normals, missing
//! references, non-uniform pose scales). Output is proprietary-derived:
//! keep it in an untracked location such as `output/.cache/`.

use magicka_content::export_manifest::ExportManifest;
use magicka_content::scene::{export_level, Context, Stats};
use magicka_content::{parse_primary, Content};
use magicka_xnb::xnb;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

/// Index files with extension `ext` by lowercase path without extension
/// (asset names are extensionless and case-insensitive).
fn walk(dir: &Path, root: &Path, ext: &str, map: &mut HashMap<String, PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, root, ext, map);
        } else if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
            continue;
        } else if let Ok(rel) = p.strip_prefix(root) {
            let key = rel
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/")
                .to_lowercase();
            map.insert(key, p.clone());
        }
    }
}

/// Relative path from directory `from` to file `to` (both absolute).
fn relative(from: &Path, to: &Path) -> String {
    let a: Vec<Component> = from.components().collect();
    let b: Vec<Component> = to.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = vec!["..".into(); a.len() - common];
    parts.extend(
        b[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut manifest = ExportManifest::from_args(&mut args);
    if args.len() < 4 {
        eprintln!(
            "usage: level-gltf <content-dir> <textures-dir> <out-dir> [--missing] [--manifest <file>] (--all | <level.xnb>...)"
        );
        return ExitCode::from(2);
    }
    let content = std::fs::canonicalize(&args[0]).expect("content dir");
    let textures_dir = std::fs::canonicalize(&args[1]).expect("textures dir");
    let out = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out).expect("out dir");
    let out = std::fs::canonicalize(out).unwrap();
    let (mut assets, mut textures) = (HashMap::new(), HashMap::new());
    walk(&content, &content, "xnb", &mut assets);
    walk(&textures_dir, &textures_dir, "png", &mut textures);

    let mut levels: Vec<PathBuf> = Vec::new();
    if args[3] == "--all" {
        for p in assets.values() {
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xnb")) {
                if let Ok(x) = xnb::parse(&std::fs::read(p).unwrap()) {
                    if x.primary_reader().is_some_and(|r| {
                        r.short_name() == "Magicka.ContentReaders.LevelModelReader"
                    }) {
                        levels.push(p.clone());
                    }
                }
            }
        }
        levels.sort();
    } else {
        levels = args[3..]
            .iter()
            .map(|a| std::fs::canonicalize(a).expect("level file"))
            .collect();
    }

    // Physics templates are shared by many scenes: parse each once.
    let mut prop_cache: HashMap<String, Option<magicka_content::scene::PropData>> = HashMap::new();
    let mut total = Stats::default();
    let mut failed = 0;
    for file in &levels {
        let asset = file
            .strip_prefix(&content)
            .unwrap_or(file)
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        let glb_path = out.join(format!("{asset}.glb"));
        if manifest.skip(&[&glb_path]) {
            continue;
        }
        let parsed = xnb::parse(&std::fs::read(file).unwrap())
            .ok()
            .and_then(|x| parse_primary(&x))
            .and_then(|r| r.ok());
        let Some(parsed) = parsed else {
            eprintln!("{asset}: parse failed");
            failed += 1;
            continue;
        };
        let Content::Level(level) = &parsed.content else {
            continue;
        };
        let glb_dir = glb_path.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&glb_dir).unwrap();
        for (_, name) in &level.physics_entities {
            prop_cache.entry(name.to_lowercase()).or_insert_with(|| {
                let key = format!("data/physicsentities/{}", name.to_lowercase());
                let path = assets.get(&key)?;
                let x = xnb::parse(&std::fs::read(path).ok()?).ok()?;
                let p = parse_primary(&x)?.ok()?;
                let Content::PhysicsEntity(t) = p.content else {
                    return None;
                };
                let rel = path
                    .strip_prefix(&content)
                    .ok()?
                    .with_extension("")
                    .to_string_lossy()
                    .replace('\\', "/");
                Some((rel, t.model?, p.shared))
            });
        }
        let texture_uri = |t: &str| {
            textures
                .get(&t.to_lowercase())
                .map(|p| magicka_content::gltf::uri(&relative(&glb_dir, p)))
        };
        let prop = |name: &str| prop_cache.get(&name.to_lowercase()).cloned().flatten();
        let ctx = Context {
            asset: &asset,
            texture_uri: &texture_uri,
            prop: &prop,
        };
        let (glb, stats) = export_level(level, &parsed.shared, &ctx);
        std::fs::write(&glb_path, glb).unwrap();
        manifest.record(&glb_path);
        if levels.len() == 1 {
            eprint!("{asset}:\n{}", stats.report());
        }
        total.primitives += stats.primitives;
        total.triangles += stats.triangles;
        total.normal_agree += stats.normal_agree;
        total.normal_disagree += stats.normal_disagree;
        total.animated_parts += stats.animated_parts;
        total.animations += stats.animations;
        total.anisotropic_scales += stats.anisotropic_scales;
        total.dropped_keyframes += stats.dropped_keyframes;
        total.zero_quaternions += stats.zero_quaternions;
        total.non_unit_quaternions += stats.non_unit_quaternions;
        total.non_finite_components += stats.non_finite_components;
        total.props += stats.props;
        total.liquids += stats.liquids;
        total.textured_materials += stats.textured_materials;
        total
            .missing_props
            .extend(stats.missing_props.iter().map(|p| format!("{asset}: {p}")));
        total.missing_textures.extend(
            stats
                .missing_textures
                .iter()
                .map(|t| format!("{asset}: {t}")),
        );
    }
    if levels.len() > 1 {
        eprint!(
            "{} scenes exported, {failed} failed\n{}",
            levels.len() - failed,
            total.report()
        );
    }
    manifest.finish();
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
