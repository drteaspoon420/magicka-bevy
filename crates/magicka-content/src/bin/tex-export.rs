//! Export Magicka textures (top mip) as PNG.
//!
//!     tex-export [--missing] [--manifest <file>] <content-dir> <out-dir>
//!
//! 2D textures -> `<rel>.png`, cube maps -> `<rel>.face<N>.png`, volume
//! textures -> first slice. Output is derived from proprietary data: write it
//! to an untracked location such as `output/.cache/`.

use magicka_content::assets::Asset;
use magicka_content::export_manifest::ExportManifest;
use magicka_content::texture::{decode_rgba, encode_png, mip_size};
use magicka_content::xna::XnaObject;
use magicka_content::{parse_primary, Content};
use magicka_xnb::xnb;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xnb")) {
            out.push(p);
        }
    }
}

fn write(out: &Path, fmt: i32, w: i32, h: i32, data: &[u8]) -> bool {
    let (w, h) = (w as usize, h as usize);
    let Some(rgba) = decode_rgba(fmt, w, h, data) else {
        return false;
    };
    if let Some(p) = out.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    std::fs::write(out, encode_png(w, h, &rgba)).is_ok()
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut manifest = ExportManifest::from_args(&mut args);
    if args.len() != 2 {
        eprintln!("usage: tex-export [--missing] [--manifest <file>] <content-dir> <out-dir>");
        return ExitCode::from(2);
    }
    let (root, out) = (Path::new(&args[0]), Path::new(&args[1]));
    let mut files = Vec::new();
    walk(root, &mut files);
    let (mut ok, mut failed) = (0, 0);
    for f in &files {
        let Ok(x) = xnb::parse(&std::fs::read(f).unwrap()) else {
            continue;
        };
        let Some(Ok(p)) = parse_primary(&x) else {
            continue;
        };
        let Content::Xna(o) = p.content else { continue };
        let XnaObject::Asset(a) = *o else { continue };
        let rel = f.strip_prefix(root).unwrap_or(f).with_extension("");
        let mut emit = |suffix: &str, fmt: i32, w: i32, h: i32, data: &[u8]| {
            let dest = out.join(format!("{}{suffix}.png", rel.display()));
            if manifest.skip(&[&dest]) {
                return;
            }
            if write(&dest, fmt, w, h, data) {
                manifest.record(&dest);
                ok += 1;
            } else {
                failed += 1;
                eprintln!("{}: cannot decode format {fmt}", f.display());
            }
        };
        match *a {
            Asset::Texture2D(t) if !t.mips.is_empty() => {
                emit("", t.format, t.width, t.height, &t.mips[0])
            }
            Asset::TextureCube {
                format,
                size,
                faces,
            } => {
                for (i, face) in faces.iter().enumerate() {
                    if let Some(m) = face.first() {
                        emit(&format!(".face{i}"), format, size, size, m);
                    }
                }
            }
            Asset::Texture3D {
                format,
                width,
                height,
                mips,
                ..
            } => {
                if let (Some(m), Some(n)) = (
                    mips.first(),
                    mip_size(format, width as usize, height as usize),
                ) {
                    emit(".slice0", format, width, height, &m[..n.min(m.len())]);
                }
            }
            _ => {}
        }
    }
    eprintln!("exported {ok} images, {failed} failed");
    manifest.finish();
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
