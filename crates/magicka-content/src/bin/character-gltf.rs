//! Export characters (CharacterTemplates) and animated props
//! (Data/AnimatedProps PhysicsEntityTemplates) as skinned, animated glTF.
//!
//!     character-gltf <content-dir> <textures-dir> <out-dir> [--reference] [--missing] [--manifest <file>] (--all | <Data/Characters/x.xnb>...)
//!
//! Prints automated self-check statistics per character (single) or in
//! aggregate (--all). Output is proprietary-derived; keep it untracked.

use magicka_content::assets::Asset;
use magicka_content::character_scene::{export_animated_prop, export_character, CharStats};
use magicka_content::export_manifest::ExportManifest;
use magicka_content::skinned::{resolve, Resolved};
use magicka_content::xna::XnaObject;
use magicka_content::{parse_primary, Content};
use magicka_xnb::xnb;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

fn walk(dir: &Path, root: &Path, ext: &str, map: &mut HashMap<String, PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, root, ext, map);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
            if let Ok(rel) = p.strip_prefix(root) {
                map.insert(
                    rel.with_extension("")
                        .to_string_lossy()
                        .replace('\\', "/")
                        .to_lowercase(),
                    p.clone(),
                );
            }
        }
    }
}

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
        eprintln!("usage: character-gltf <content-dir> <textures-dir> <out-dir> [--reference] [--missing] [--manifest <file>] (--all | <template.xnb>...)");
        return ExitCode::from(2);
    }
    let content = std::fs::canonicalize(&args[0]).expect("content dir");
    let textures_dir = std::fs::canonicalize(&args[1]).expect("textures dir");
    std::fs::create_dir_all(&args[2]).expect("out dir");
    let out = std::fs::canonicalize(&args[2]).unwrap();
    let (mut assets, mut textures) = (HashMap::new(), HashMap::new());
    walk(&content, &content, "xnb", &mut assets);
    walk(&textures_dir, &textures_dir, "png", &mut textures);

    // --reference: also write <glb>.ref.json with game-math skinned samples.
    let reference = args.iter().any(|a| a == "--reference");
    let args: Vec<String> = args.into_iter().filter(|a| a != "--reference").collect();
    let templates: Vec<PathBuf> = if args[3] == "--all" {
        let mut v: Vec<PathBuf> = assets
            .iter()
            .filter(|(k, _)| {
                k.starts_with("data/characters/") || k.starts_with("data/animatedprops/")
            })
            .map(|(_, p)| p.clone())
            .collect();
        v.sort();
        v
    } else {
        args[3..]
            .iter()
            .map(|a| std::fs::canonicalize(a).expect("template file"))
            .collect()
    };

    let cache: RefCell<HashMap<String, Option<Resolved>>> = RefCell::new(HashMap::new());
    let load = |asset: &str| -> Option<Resolved> {
        let key = asset.to_lowercase();
        if let Some(r) = cache.borrow().get(&key) {
            return r.clone();
        }
        let r = (|| {
            let x = xnb::parse(&std::fs::read(assets.get(&key)?).ok()?).ok()?;
            let p = parse_primary(&x)?.ok()?;
            let Content::Xna(o) = p.content else {
                return None;
            };
            let XnaObject::Asset(a) = *o else { return None };
            let Asset::SkinnedModel(sm) = *a else {
                return None;
            };
            resolve(&sm, p.shared).ok()
        })();
        cache.borrow_mut().insert(key, r.clone());
        r
    };

    let mut total = CharStats::default();
    let (mut ok, mut skipped) = (0, Vec::new());
    for file in &templates {
        let asset = file
            .strip_prefix(&content)
            .unwrap_or(file)
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        let glb_path = out.join(format!("{asset}.glb"));
        let ref_path = glb_path.with_extension("ref.json");
        let outputs: &[&Path] = if reference {
            &[&glb_path, &ref_path]
        } else {
            &[&glb_path]
        };
        if manifest.skip(outputs) {
            continue;
        }
        let parsed = xnb::parse(&std::fs::read(file).unwrap())
            .ok()
            .and_then(|x| parse_primary(&x))
            .and_then(|r| r.ok());
        let template = match parsed.map(|p| p.content) {
            Some(Content::Character(t)) => Ok(t),
            Some(Content::PhysicsEntity(t)) => match t.animated {
                Some(part) => Err((t.id.clone(), part)),
                None => {
                    skipped.push(format!("{asset}: physics template without animation"));
                    continue;
                }
            },
            _ => {
                skipped.push(format!("{asset}: not a CharacterTemplate"));
                continue;
            }
        };
        let glb_dir = glb_path.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&glb_dir).unwrap();
        let texture_uri = |t: &str| {
            textures
                .get(&t.to_lowercase())
                .map(|p| magicka_content::gltf::uri(&relative(&glb_dir, p)))
        };
        let exported = match &template {
            Ok(t) => export_character(t, &asset, &load, &texture_uri, reference),
            Err((id, part)) => {
                export_animated_prop(id, part, &asset, &load, &texture_uri, reference)
            }
        };
        match exported {
            Ok((glb, st)) => {
                std::fs::write(&glb_path, glb).unwrap();
                manifest.record(&glb_path);
                if let Some(r) = &st.reference_json {
                    std::fs::write(&ref_path, r).unwrap();
                    manifest.record(&ref_path);
                }
                ok += 1;
                if templates.len() == 1 {
                    eprint!("{asset}:\n{}", st.report());
                }
                let g = &st.geometry;
                total.geometry.primitives += g.primitives;
                total.geometry.triangles += g.triangles;
                total.geometry.normal_agree += g.normal_agree;
                total.geometry.normal_disagree += g.normal_disagree;
                total.geometry.renormalized_weights += g.renormalized_weights;
                total.geometry.zero_weights += g.zero_weights;
                total.geometry.non_finite_components += g.non_finite_components;
                total.bones += st.bones;
                total.clips += st.clips;
                total.channels += st.channels;
                total.unmatched_channels += st.unmatched_channels;
                total.skeleton_name_mismatch |= st.skeleton_name_mismatch;
                if st.skeleton_name_mismatch {
                    total
                        .missing_textures
                        .push(format!("[skeleton name mismatch] {asset}"));
                }
                total.bad_joint_indices += st.bad_joint_indices;
                total.anisotropic += st.anisotropic;
                total.dropped_keyframes += st.dropped_keyframes;
                total.densified_keys += st.densified_keys;
                total.textured_materials += st.textured_materials;
                total
                    .missing_textures
                    .extend(st.missing_textures.iter().map(|t| format!("{asset}: {t}")));
            }
            Err(e) => skipped.push(format!("{asset}: {e}")),
        }
    }
    manifest.finish();
    if templates.len() > 1 {
        eprint!(
            "{ok} characters exported, {} skipped\n{}",
            skipped.len(),
            total.report()
        );
        let mut reasons: HashMap<String, usize> = HashMap::new();
        for s in &skipped {
            let r = s
                .split(": ")
                .nth(1)
                .unwrap_or("")
                .split(' ')
                .take(3)
                .collect::<Vec<_>>()
                .join(" ");
            *reasons.entry(r).or_default() += 1;
        }
        eprintln!("skip reasons: {reasons:?}");
    } else {
        for s in &skipped {
            eprintln!("{s}");
        }
    }
    ExitCode::SUCCESS
}
