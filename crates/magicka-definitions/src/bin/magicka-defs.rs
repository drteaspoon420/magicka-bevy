//! Definition diagnostics: `magicka-defs [--base DIR] [--mods DIR] <command>`.
//!
//! - `validate`   compile every definition; print errors and warnings
//! - `show <id>`  the effective (patched) definition and where it came from
//! - `hash`       the authoritative definitions hash
use magicka_definitions::Definitions;
use magicka_mods::ModSet;
use std::path::PathBuf;

fn main() {
    if let Err(e) = run() {
        eprint!("{e}");
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
    let set = ModSet::load(base.as_deref(), &mods, None).map_err(|e| format!("{e}\n"))?;
    let defs = Definitions::load(&set).map_err(|e| e.to_string())?;
    match args.first().map(String::as_str) {
        Some("validate") => {
            for w in &defs.warnings {
                println!("warning: {w}");
            }
            println!(
                "ok: {} items, {} abilities, {} modifiers, {} magicks, {} spells, {} units, {} effects, {} levels",
                defs.items.len(),
                defs.abilities.len(),
                defs.modifiers.len(),
                defs.magicks.len(),
                defs.spells.len(),
                defs.units.len(),
                defs.effects.len(),
                defs.levels.len()
            );
        }
        Some("show") => {
            let id = args.get(1).ok_or("show needs an id\n")?;
            let json = |v: serde_json::Result<serde_json::Value>| {
                serde_json::to_string_pretty(&v.unwrap()).unwrap()
            };
            let found = [
                defs.items
                    .get_str(id)
                    .map(|d| (json(serde_json::to_value(d)), &d.common.source)),
                defs.abilities
                    .get_str(id)
                    .map(|d| (json(serde_json::to_value(d)), &d.common.source)),
                defs.modifiers
                    .get_str(id)
                    .map(|d| (json(serde_json::to_value(d)), &d.common.source)),
                defs.magicks
                    .get_str(id)
                    .map(|d| (json(serde_json::to_value(d)), &d.common.source)),
                defs.spells
                    .get_str(id)
                    .map(|d| (json(serde_json::to_value(d)), &d.common.source)),
                defs.units
                    .get_str(id)
                    .map(|d| (json(serde_json::to_value(d)), &d.common.source)),
                defs.effects
                    .get_str(id)
                    .map(|d| (json(serde_json::to_value(d)), &d.common.source)),
                defs.levels
                    .get_str(id)
                    .map(|d| (json(serde_json::to_value(d)), &d.common.source)),
            ];
            let mut any = false;
            for (text, source) in found.into_iter().flatten() {
                any = true;
                println!("# {}:{}\n{text}", source.file, source.line);
            }
            if !any {
                return Err(format!("{id}: no such definition\n"));
            }
        }
        Some("hash") => println!("{}", defs.rules_hash()),
        _ => {
            return Err(
                "usage: magicka-defs [--base DIR] [--mods DIR] validate|show <id>|hash\n".into(),
            )
        }
    }
    Ok(())
}
