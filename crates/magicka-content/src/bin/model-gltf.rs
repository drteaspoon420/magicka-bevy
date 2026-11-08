//! Export static XNA Models (e.g. `Models/Missiles/*`) to glTF. A
//! SkinnedModel asset (e.g. `Models/Effects/mine`) is exported with its own
//! skeleton and clips; `<mesh>@<animation>` takes the skeleton and clips from
//! a second SkinnedModel (e.g. barrier meshes and their animation models).
//! `<mesh>@@<animation>` keeps the mesh's own skeleton and takes only the
//! clips, bound by bone name as XNAnimation's controller does (WaveEntity
//! plays `rockpillar_animation` on `earthBarrier0_mesh`); the output is
//! `<mesh>+<animation name>.glb`.
//!
//!     model-gltf <content-dir> <textures-dir> <out-dir> [--missing] [--manifest <file>] (--missiles | --items | --physics | --gibs | <model asset>...)
//!
//! `--gibs` exports every CharacterTemplate and PhysicsEntityTemplate gib
//! model (Character.SpawnGibs, DamageablePhysicsEntity.SpawnGibs).
//!
//! Model assets are content-relative without extension, e.g.
//! `Models/Missiles/Earth0`; output mirrors that path with `.glb`.
use magicka_content::assets::Asset;
use magicka_content::character_scene::{export_skinned, export_static_model};
use magicka_content::export_manifest::ExportManifest;
use magicka_content::skinned::resolve;
use magicka_content::xna::XnaObject;
use magicka_content::{parse_primary, Content};
use magicka_xnb::xnb;
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

fn load_skinned(
    assets: &HashMap<String, PathBuf>,
    key: &str,
) -> Result<magicka_content::skinned::Resolved, String> {
    let path = assets.get(key).ok_or_else(|| format!("{key}: not found"))?;
    let parsed = std::fs::read(path)
        .ok()
        .and_then(|b| xnb::parse(&b).ok())
        .and_then(|x| parse_primary(&x))
        .and_then(|r| r.ok())
        .ok_or_else(|| format!("{key}: unreadable"))?;
    let Content::Xna(object) = parsed.content else {
        return Err(format!("{key}: not an XNA object"));
    };
    let XnaObject::Asset(asset) = *object else {
        return Err(format!("{key}: not a SkinnedModel"));
    };
    let Asset::SkinnedModel(sm) = *asset else {
        return Err(format!("{key}: not a SkinnedModel"));
    };
    resolve(&sm, parsed.shared)
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut manifest = ExportManifest::from_args(&mut args);
    if args.len() < 4 {
        eprintln!("usage: model-gltf <content-dir> <textures-dir> <out-dir> [--missing] [--manifest <file>] (--missiles | --items | --physics | --gibs | <model asset>...)");
        return ExitCode::from(2);
    }
    let content = std::fs::canonicalize(&args[0]).expect("content dir");
    let textures_dir = std::fs::canonicalize(&args[1]).expect("textures dir");
    std::fs::create_dir_all(&args[2]).expect("out dir");
    let out = std::fs::canonicalize(&args[2]).unwrap();
    let (mut assets, mut textures) = (HashMap::new(), HashMap::new());
    walk(&content, &content, "xnb", &mut assets);
    walk(&textures_dir, &textures_dir, "png", &mut textures);
    let mut models: Vec<String> = if args[3] == "--missiles" {
        assets
            .keys()
            .filter(|k| k.starts_with("models/missiles/"))
            .cloned()
            .collect()
    } else if args[3] == "--physics" {
        assets
            .keys()
            .filter(|k| k.starts_with("data/physicsentities/"))
            .cloned()
            .collect()
    } else if args[3] == "--items" {
        assets
            .iter()
            .filter(|(key, _)| key.starts_with("data/items/"))
            .filter_map(|(key, path)| {
                let bytes = std::fs::read(path).ok()?;
                let parsed = xnb::parse(&bytes).ok()?;
                let Content::Item(item) = parse_primary(&parsed)?.ok()?.content else {
                    return None;
                };
                // Equipped models and ranged projectile models.
                Some(
                    item.model
                        .iter()
                        .chain(&item.projectile_model)
                        .map(|model| {
                            magicka_content::mesh::resolve_reference(key, model).to_lowercase()
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect()
    } else if args[3] == "--gibs" {
        assets
            .iter()
            .filter(|(key, _)| {
                key.starts_with("data/characters/") || key.starts_with("data/physicsentities/")
            })
            .filter_map(|(key, path)| {
                let bytes = std::fs::read(path).ok()?;
                let parsed = xnb::parse(&bytes).ok()?;
                let gibs = match parse_primary(&parsed)?.ok()?.content {
                    Content::Character(template) => template.gibs,
                    Content::PhysicsEntity(template) => template.gibs,
                    _ => return None,
                };
                Some(
                    gibs.iter()
                        .filter_map(|gib| gib.model.as_ref())
                        .map(|model| {
                            magicka_content::mesh::resolve_reference(key, model).to_lowercase()
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect()
    } else {
        args[3..].iter().map(|a| a.to_lowercase()).collect()
    };
    models.sort();
    models.dedup();
    let (mut ok, mut failed) = (0, 0);
    for key in models {
        let (key, animation, clips_only) = match key.split_once("@@") {
            Some((mesh, animation)) => (mesh.to_string(), Some(animation.to_string()), true),
            None => match key.split_once('@') {
                Some((mesh, animation)) => (mesh.to_string(), Some(animation.to_string()), false),
                None => (key, None, false),
            },
        };
        let Some(path) = assets.get(&key) else {
            eprintln!("{key}: not found");
            failed += 1;
            continue;
        };
        let asset = path
            .strip_prefix(&content)
            .unwrap_or(path)
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        let glb_path = match (&animation, clips_only) {
            (Some(a), true) => {
                out.join(format!("{asset}+{}.glb", a.rsplit('/').next().unwrap_or(a)))
            }
            _ => out.join(format!("{asset}.glb")),
        };
        if manifest.skip(&[&glb_path]) {
            continue;
        }
        let parsed = std::fs::read(path)
            .ok()
            .and_then(|b| xnb::parse(&b).ok())
            .and_then(|x| parse_primary(&x))
            .and_then(|r| r.ok());
        let Some(parsed) = parsed else {
            eprintln!("{asset}: unreadable");
            failed += 1;
            continue;
        };
        let object = match parsed.content {
            Content::Xna(object) => object,
            Content::PhysicsEntity(t) => match t.model {
                Some(model) => Box::new(XnaObject::Model(Box::new(model))),
                None => {
                    eprintln!("{asset}: no static model (animated prop export pending)");
                    continue;
                }
            },
            _ => {
                eprintln!("{asset}: not a model or physics template");
                failed += 1;
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
        let result = match *object {
            XnaObject::Model(model) => {
                export_static_model(&model, &parsed.shared, &asset, &texture_uri)
            }
            XnaObject::Asset(a) => match *a {
                Asset::SkinnedModel(sm) => resolve(&sm, parsed.shared).and_then(|r| {
                    let name = asset.rsplit('/').next().unwrap_or(&asset);
                    let skeleton = match &animation {
                        Some(a) if clips_only => Some(magicka_content::skinned::Resolved {
                            clips: load_skinned(&assets, a)?.clips,
                            ..r.clone()
                        }),
                        Some(a) => Some(load_skinned(&assets, a)?),
                        None => None,
                    };
                    export_skinned(
                        name,
                        skeleton.as_ref().unwrap_or(&r),
                        &asset,
                        &r,
                        1.0,
                        &texture_uri,
                        false,
                    )
                    .map(|(glb, _)| glb)
                }),
                _ => Err("not a Model or SkinnedModel".into()),
            },
            _ => Err("not a Model or SkinnedModel".into()),
        };
        match result {
            Ok(glb) => {
                std::fs::write(&glb_path, glb).unwrap();
                manifest.record(&glb_path);
                ok += 1;
            }
            Err(e) => {
                eprintln!("{asset}: {e}");
                failed += 1;
            }
        }
    }
    eprintln!("{ok} models exported, {failed} skipped");
    manifest.finish();
    ExitCode::SUCCESS
}
