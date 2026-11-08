//! Tabulate RenderDeferredEffect alpha flags of level geometry against the
//! diffuse texture's format and alpha content, over all levels.
//!     cargo run --release --example material_flags -- <content-dir>
use magicka_content::assets::Asset;
use magicka_content::texture::decode_rgba;
use magicka_content::xna::XnaObject;
use magicka_content::{mesh, parse_primary, Content};
use magicka_xnb::xnb;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

fn walk(d: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(d).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out)
        } else if p.extension().is_some_and(|x| x == "xnb") {
            out.push(p)
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap());
    let mut files = Vec::new();
    walk(&root, &mut files);
    let index: HashMap<String, PathBuf> = files
        .iter()
        .map(|p| {
            (
                p.strip_prefix(&root)
                    .unwrap()
                    .with_extension("")
                    .to_string_lossy()
                    .to_lowercase(),
                p.clone(),
            )
        })
        .collect();
    let mut tex_cache: HashMap<String, Option<(i32, f32)>> = HashMap::new();
    let mut table: BTreeMap<String, (usize, f32)> = BTreeMap::new();
    for f in &files {
        let x = xnb::parse(&std::fs::read(f).unwrap()).unwrap();
        let Some(Ok(p)) = parse_primary(&x) else {
            continue;
        };
        let Content::Level(l) = p.content else {
            continue;
        };
        let asset = f
            .strip_prefix(&root)
            .unwrap()
            .with_extension("")
            .to_string_lossy()
            .to_string();
        for r in l.model.iter().flat_map(|m| m.roots.iter()) {
            let Some(XnaObject::RenderDeferredEffect(e)) = &r.effect else {
                continue;
            };
            let tex = e
                .layer0
                .diffuse_texture
                .as_ref()
                .map(|t| mesh::resolve_reference(&asset, t).to_lowercase());
            let info = tex.as_ref().and_then(|t| {
                *tex_cache.entry(t.clone()).or_insert_with(|| {
                    let x = xnb::parse(&std::fs::read(index.get(t)?).ok()?).ok()?;
                    let p = parse_primary(&x)?.ok()?;
                    let Content::Xna(o) = p.content else {
                        return None;
                    };
                    let XnaObject::Asset(a) = *o else { return None };
                    let Asset::Texture2D(t) = *a else { return None };
                    let rgba =
                        decode_rgba(t.format, t.width as usize, t.height as usize, &t.mips[0])?;
                    let low = rgba.chunks(4).filter(|p| p[3] < 128).count() as f32
                        / (rgba.len() / 4) as f32;
                    Some((t.format, low))
                })
            });
            let key = format!(
                "mask={} alphaDisabled={} layer1={} vcolor={} fmt={:?}",
                e.layer0.alpha_mask_enabled,
                e.layer0.diffuse_alpha_disabled,
                e.layer1.is_some(),
                e.vertex_color_enabled,
                info.map(|i| i.0)
            );
            let ent = table.entry(key).or_insert((0, 0.0));
            ent.0 += 1;
            ent.1 = ent.1.max(info.map_or(0.0, |i| i.1));
        }
    }
    for (k, (n, low)) in table {
        println!("{n:6}  {k}  max frac alpha<0.5: {low:.3}");
    }
}
