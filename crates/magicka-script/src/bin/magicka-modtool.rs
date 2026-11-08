//! Mod author tool: `magicka-modtool [--base DIR] [--mods DIR] [--enable a,b] <command>`.
//!
//! - `validate`        definitions, Lua syntax, server VM boot; exit 1 on errors
//! - `order`           resolved load order
//! - `resolve <path>`  layers providing a logical path, and the winner
//! - `show <id>`       an effective (patched) definition as JSON
//! - `hash`            the match rules hash and presentation manifest
use magicka_mods::ModSet;
use std::{path::PathBuf, sync::Arc};

fn main() {
    match run() {
        Ok(ok) => std::process::exit(if ok { 0 } else { 1 }),
        Err(e) => {
            eprintln!("magicka-modtool: {e}");
            std::process::exit(2);
        }
    }
}

fn run() -> Result<bool, String> {
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
    let enabled: Option<Vec<String>> =
        take("--enable").map(|l| l.split(',').map(|s| s.trim().to_string()).collect());
    let set =
        ModSet::load(base.as_deref(), &mods, enabled.as_deref()).map_err(|e| e.to_string())?;
    match args.first().map(String::as_str) {
        Some("validate") => {
            let report = magicka_script::validate(Arc::new(set));
            for w in &report.warnings {
                println!("warning: {w}");
            }
            for e in &report.errors {
                println!("error: {e}");
            }
            for s in &report.summary {
                println!("{s}");
            }
            println!(
                "{}",
                if report.errors.is_empty() { "ok" } else { "FAILED" }
            );
            Ok(report.errors.is_empty())
        }
        Some("order") => {
            for m in &set.mods {
                println!("{}", m.label());
            }
            Ok(true)
        }
        Some("resolve") => {
            print!("{}", set.explain(args.get(1).ok_or("resolve needs a path")?)?);
            Ok(true)
        }
        Some("show") => {
            let id = args.get(1).ok_or("show needs an id")?;
            let defs = magicka_definitions::Definitions::load(&set).map_err(|e| e.to_string())?;
            let found = [
                defs.items.get_str(id).map(serde_json::to_string_pretty),
                defs.abilities.get_str(id).map(serde_json::to_string_pretty),
                defs.modifiers.get_str(id).map(serde_json::to_string_pretty),
                defs.magicks.get_str(id).map(serde_json::to_string_pretty),
                defs.spells.get_str(id).map(serde_json::to_string_pretty),
                defs.units.get_str(id).map(serde_json::to_string_pretty),
                defs.effects.get_str(id).map(serde_json::to_string_pretty),
                defs.levels.get_str(id).map(serde_json::to_string_pretty),
            ];
            let mut any = false;
            for text in found.into_iter().flatten() {
                println!("{}", text.map_err(|e| e.to_string())?);
                any = true;
            }
            if !any {
                return Err(format!("{id}: no such definition"));
            }
            Ok(true)
        }
        Some("hash") => {
            let m = set.content_manifest()?;
            println!("{}", serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?);
            Ok(true)
        }
        _ => Err("usage: magicka-modtool [--base DIR] [--mods DIR] [--enable a,b] validate|order|resolve <path>|show <id>|hash".into()),
    }
}
