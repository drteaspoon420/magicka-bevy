use magicka_content::{assets::Asset, parse_primary, xna::XnaObject, Content};
use magicka_runtime::bitmap_font::FontMetrics;
use magicka_runtime::text_geometry::{GeometryFont, TextGeometry, TextStyle};
use std::path::{Path, PathBuf};
fn walk(dir: &Path, paths: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            walk(&p, paths);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xnb")) {
            paths.push(p);
        }
    }
}
#[test]
fn retail_font_metrics_construct_without_graphics_or_target_execution() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    let mut paths = vec![];
    walk(&root.join("Languages"), &mut paths);
    walk(&root.join("UI/Font"), &mut paths);
    paths.sort();
    let (mut fonts, mut glyphs, mut pairs) = (0, 0, 0);
    for path in paths {
        let bytes = std::fs::read(&path).unwrap();
        let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
        if !xnb
            .primary_reader()
            .is_some_and(|r| r.short_name() == "PolygonHead.Pipeline.BitmapFontReader")
        {
            continue;
        }
        let parsed = parse_primary(&xnb).unwrap().unwrap();
        assert_eq!(parsed.trailing, 0);
        let Content::Xna(object) = parsed.content else {
            panic!("expected asset")
        };
        let XnaObject::Asset(asset) = *object else {
            panic!("expected asset")
        };
        let Asset::BitmapFont(content) = *asset else {
            panic!("expected font")
        };
        let metrics = FontMetrics::from_content(&content).unwrap();
        let geometry_font = GeometryFont::from_content(&content).unwrap();
        fonts += 1;
        glyphs += content.glyphs.len();
        pairs += content.kerning.len();
        for glyph in &content.glyphs {
            assert_eq!(
                metrics.glyph_advance(glyph.character as u16).unwrap(),
                (true, glyph.advance_width)
            );
        }
        for &(left, right, amount) in &content.kerning {
            assert_eq!(metrics.calc_kern(left as u16, right as u16), amount);
        }
        for text in ["Wizard A B", "[c=1,1,1]Wizard[/c]\nB", "[[Wizard", "😀"] {
            let wrapped = metrics.wrap(text, 100, true).unwrap();
            assert!(metrics
                .measure(&wrapped.text, true)
                .unwrap()
                .iter()
                .all(|v| v.is_finite()));
            let geometry = TextGeometry::build(
                &geometry_font,
                &wrapped.text.encode_utf16().collect::<Vec<_>>(),
                TextStyle::default(),
            )
            .unwrap();
            assert_eq!(geometry.indices().len(), geometry.quads.len() * 6);
            assert!(geometry
                .quads
                .iter()
                .flat_map(|q| q.vertices.iter())
                .all(|v| v
                    .position
                    .iter()
                    .chain(v.uv.iter())
                    .chain(v.color.iter())
                    .all(|n| n.is_finite())));
        }
    }
    eprintln!("font corpus: {fonts} fonts, {glyphs} glyphs, {pairs} kerning pairs; zero failures");
    assert!(fonts > 0);
}
