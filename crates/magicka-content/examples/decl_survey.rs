//! Survey vertex declarations of level BiTree roots by material kind.
use magicka_content::xna::XnaObject;
use magicka_content::{parse_primary, Content};
use magicka_xnb::xnb;
use std::collections::BTreeMap;
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
    let mut files = Vec::new();
    walk(Path::new(&std::env::args().nth(1).unwrap()), &mut files);
    let mut t: BTreeMap<String, usize> = BTreeMap::new();
    for f in &files {
        let x = xnb::parse(&std::fs::read(f).unwrap()).unwrap();
        let Some(Ok(p)) = parse_primary(&x) else {
            continue;
        };
        let Content::Level(l) = p.content else {
            continue;
        };
        for r in l.model.iter().flat_map(|m| m.roots.iter()) {
            let Some(XnaObject::RenderDeferredEffect(e)) = &r.effect else {
                continue;
            };
            let decl: Vec<String> = r
                .vertex_declaration
                .iter()
                .flatten()
                .map(|e| format!("u{}i{}f{}", e.usage, e.usage_index, e.format))
                .collect();
            *t.entry(format!(
                "layer1={} vcolor={} | {}",
                e.layer1.is_some(),
                e.vertex_color_enabled,
                decl.join(" ")
            ))
            .or_default() += 1;
        }
    }
    for (k, n) in t {
        println!("{n:6} {k}");
    }
}
