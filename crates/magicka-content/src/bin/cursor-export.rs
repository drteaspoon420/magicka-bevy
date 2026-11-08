//! Export Magicka's `Cursors/*.ani` (user-supplied, next to Content) as PNG
//! frames plus a `cursors.json` index for the viewer:
//!
//!     cursor-export [--manifest <file>] <game-dir> <out-dir>
//!
//! Output: `<out-dir>/<Name>_<frame>.png` and `<out-dir>/cursors.json`
//! (`{ "Name": { "hotspot": [x, y], "frames": [..], "sequence": [..],
//! "rates": [jiffies..] } }`).
use magicka_content::export_manifest::ExportManifest;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    let mut manifest = ExportManifest::from_args(&mut args);
    if args.len() != 3 {
        eprintln!("usage: cursor-export [--manifest <file>] <game-dir> <out-dir>");
        return ExitCode::FAILURE;
    }
    let dir = PathBuf::from(&args[1]).join("Cursors");
    let out = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out).unwrap();
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .expect("Cursors directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ani")))
        .collect();
    names.sort();
    let mut json = String::from("{\n");
    let mut first = true;
    for path in names {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let cursor = match magicka_content::ani::parse(&std::fs::read(&path).unwrap()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{name}: {e}");
                continue;
            }
        };
        let mut files = Vec::new();
        for (i, f) in cursor.frames.iter().enumerate() {
            let file = format!("{name}_{i}.png");
            let png =
                magicka_content::texture::encode_png(f.width as usize, f.height as usize, &f.rgba);
            std::fs::write(out.join(&file), png).unwrap();
            manifest.record(&out.join(&file));
            files.push(format!("\"{file}\""));
        }
        let (hx, hy) = cursor.frames[0].hotspot;
        if !first {
            json.push_str(",\n");
        }
        first = false;
        json.push_str(&format!(
            "  \"{name}\": {{ \"hotspot\": [{hx}, {hy}], \"frames\": [{}], \"sequence\": [{}], \"rates\": [{}] }}",
            files.join(", "),
            cursor.sequence.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "),
            cursor.rates.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "),
        ));
        eprintln!(
            "{name}: {} frames, {} steps",
            cursor.frames.len(),
            cursor.sequence.len()
        );
    }
    json.push_str("\n}\n");
    std::fs::write(out.join("cursors.json"), json).unwrap();
    manifest.record(&out.join("cursors.json"));
    manifest.finish();
    ExitCode::SUCCESS
}
