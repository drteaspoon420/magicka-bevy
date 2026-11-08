//! Mod diagnostics: `magicka-mod [--base DIR] [--mods DIR] <command>`.
//!
//! - `order`           resolved load order
//! - `resolve <path>`  every layer providing a logical path, and the winner
//! - `hash`            rules hash and presentation manifest (JSON)
//! - `list [prefix]`   merged file listing
use magicka_mods::ModSet;
use std::path::PathBuf;

fn main() {
    if let Err(e) = run() {
        eprintln!("magicka-mod: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut take = |flag: &str| -> Option<String> {
        let at = args.iter().position(|a| a == flag)?;
        args.remove(at);
        (at < args.len()).then(|| args.remove(at))
    };
    let base = take("--base").map(PathBuf::from);
    let mods = take("--mods")
        .map(PathBuf::from)
        .unwrap_or_else(|| "mods".into());
    let set = ModSet::load(base.as_deref(), &mods, None).map_err(|e| e.to_string())?;
    match args.first().map(String::as_str) {
        Some("order") => {
            for m in &set.mods {
                println!(
                    "{}{}",
                    m.label(),
                    if m.client_only { " (client only)" } else { "" }
                );
            }
        }
        Some("resolve") => {
            let path = args.get(1).ok_or("resolve needs a logical path")?;
            print!("{}", set.explain(path)?);
        }
        Some("hash") => {
            let manifest = set.content_manifest()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?
            );
        }
        Some("list") => {
            for p in set.vfs.list(args.get(1).map_or("", String::as_str)) {
                println!("{p}");
            }
        }
        _ => return Err(
            "usage: magicka-mod [--base DIR] [--mods DIR] order|resolve <path>|hash|list [prefix]"
                .into(),
        ),
    }
    Ok(())
}
