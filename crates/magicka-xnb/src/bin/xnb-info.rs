//! Survey `.xnb` files: decompress each one and print its type-reader
//! manifest as TSV on stdout, with a summary on stderr.
//!
//!     cargo run --release --bin xnb-info -- <file-or-dir>... [--extract DIR]
//!
//! `--extract DIR` writes each decompressed payload to DIR (mirroring the
//! input tree, `.xnb` -> `.bin`). Payloads are derived from proprietary files:
//! keep them in a private, untracked location such as `output/.cache/`.

use magicka_xnb::xnb::{self, Xnb};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for p in entries {
            collect(&p, out);
        }
    } else if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xnb"))
    {
        out.push(path.to_path_buf());
    }
}

fn describe(x: &Xnb) -> String {
    if x.primary_reader()
        .is_some_and(|r| r.short_name().ends_with(".Texture2DReader"))
    {
        match xnb::texture2d_info(x) {
            Ok(t) => format!(
                "tex fmt={} {}x{} mips={} trailing={}",
                t.surface_format,
                t.width,
                t.height,
                t.mip_sizes.len(),
                t.trailing
            ),
            Err(e) => format!("tex PARSE-ERROR {e}"),
        }
    } else {
        String::new()
    }
}

fn main() -> ExitCode {
    let mut inputs = Vec::new();
    let mut extract: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--extract" {
            extract = args.next().map(PathBuf::from);
        } else {
            inputs.push(PathBuf::from(a));
        }
    }
    if inputs.is_empty() {
        eprintln!("usage: xnb-info <file-or-dir>... [--extract DIR]");
        return ExitCode::from(2);
    }
    let mut files = Vec::new();
    for i in &inputs {
        collect(i, &mut files);
    }
    println!("path\tplatform\tversion\tcompressed\tfile_size\tpayload_size\tshared\tprimary_reader\tall_readers\tnotes");
    let (mut ok, mut failed) = (0usize, 0usize);
    let mut primary_hist: BTreeMap<String, usize> = BTreeMap::new();
    for f in &files {
        let bytes = match std::fs::read(f) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{}: read error {e}", f.display());
                failed += 1;
                continue;
            }
        };
        match xnb::parse(&bytes) {
            Ok(x) => {
                ok += 1;
                let primary = x
                    .primary_reader()
                    .map(|r| r.short_name().to_string())
                    .unwrap_or_else(|| "<null>".into());
                *primary_hist.entry(primary.clone()).or_default() += 1;
                let all: Vec<_> = x
                    .readers
                    .iter()
                    .map(|r| format!("{}@{}", r.short_name(), r.version))
                    .collect();
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    f.display(),
                    x.header.platform as char,
                    x.header.version,
                    x.header.compressed(),
                    x.header.file_size,
                    x.payload.len(),
                    x.shared_resource_count,
                    primary,
                    all.join(";"),
                    describe(&x)
                );
                if let Some(dir) = &extract {
                    let rel = inputs
                        .iter()
                        .find_map(|i| f.strip_prefix(i).ok())
                        .unwrap_or(f);
                    let dest = dir.join(rel).with_extension("bin");
                    if let Some(parent) = dest.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(e) = std::fs::write(&dest, &x.payload) {
                        eprintln!("{}: extract error {e}", dest.display());
                    }
                }
            }
            Err(e) => {
                failed += 1;
                println!("{}\t\t\t\t{}\t\t\tERROR\t\t{e}", f.display(), bytes.len());
            }
        }
    }
    eprintln!("files: {} ok: {ok} failed: {failed}", files.len());
    eprintln!("primary reader histogram:");
    let mut hist: Vec<_> = primary_hist.into_iter().collect();
    hist.sort_by_key(|h| std::cmp::Reverse(h.1));
    for (name, n) in hist {
        eprintln!("{n:6}  {name}");
    }
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
