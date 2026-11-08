//! Validation against the user-supplied retail content in `target/`.
//!
//! Skipped (passes with a notice) when the content is absent, so the crate
//! builds and tests without proprietary data. Set `MAGICKA_CONTENT` to point
//! at a `Content` directory elsewhere.

use magicka_xnb::xnb;
use std::path::{Path, PathBuf};

fn content_dir() -> Option<PathBuf> {
    let p = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_xnb::retail_content_dir());
    p.is_dir().then_some(p)
}

fn xnb_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            xnb_files(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xnb")) {
            out.push(p);
        }
    }
}

/// Every XNB must decompress to exactly its declared size and yield a
/// plausible type-reader manifest. Every Texture2D must be consumed exactly
/// by its declared mip sizes, a strong structural check that the LZX output
/// is byte-correct.
#[test]
fn all_retail_xnb_files_decompress_and_textures_are_exact() {
    let Some(dir) = content_dir() else {
        eprintln!("SKIP: target/Magicka/Content not present");
        return;
    };
    let mut files = Vec::new();
    xnb_files(&dir, &mut files);
    assert!(!files.is_empty());
    let mut failures = Vec::new();
    let mut textures = 0;
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        match xnb::parse(&bytes) {
            Ok(x) => {
                if x.readers
                    .iter()
                    .any(|r| r.name.is_empty() || !r.name.is_ascii())
                {
                    failures.push(format!("{}: implausible reader name", f.display()));
                }
                if x.primary_reader()
                    .is_some_and(|r| r.short_name().ends_with(".Texture2DReader"))
                    && x.shared_resource_count == 0
                {
                    textures += 1;
                    match xnb::texture2d_info(&x) {
                        Ok(t) if t.trailing == 0 => {}
                        Ok(t) => failures.push(format!(
                            "{}: texture trailing {} bytes",
                            f.display(),
                            t.trailing
                        )),
                        Err(e) => failures.push(format!("{}: texture {e}", f.display())),
                    }
                }
            }
            Err(e) => failures.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!(
        "checked {} xnb files ({textures} standalone textures)",
        files.len()
    );
    assert!(
        failures.is_empty(),
        "{} failures, first: {:#?}",
        failures.len(),
        &failures[..failures.len().min(10)]
    );
}
