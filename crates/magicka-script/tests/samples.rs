//! Plan M11: every shipped sample mod validates alone and together
//! (definitions, Lua syntax, server VM boot) with no warnings.
use magicka_mods::ModSet;
use std::{path::PathBuf, sync::Arc};

#[test]
fn shipped_sample_mods_validate_alone_and_together() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let all = ["boosted", "example.my_magicks", "example.arena"];
    let mut sets: Vec<Vec<&str>> = all.iter().map(|m| vec![*m]).collect();
    sets.push(all.to_vec());
    for enabled in sets {
        let list: Vec<String> = enabled.iter().map(|s| s.to_string()).collect();
        let set = ModSet::load(
            Some(&root.join("game/base")),
            &root.join("mods"),
            Some(&list),
        )
        .unwrap();
        let report = magicka_script::validate(Arc::new(set));
        assert!(
            report.errors.is_empty(),
            "{enabled:?}: {:#?}",
            report.errors
        );
        assert!(
            report.warnings.is_empty(),
            "{enabled:?}: {:#?}",
            report.warnings
        );
    }
    // A broken mod is reported with its location, not booted silently.
    let dir = std::env::temp_dir().join(format!("magicka-broken-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let write = |p: &str, t: &str| {
        let p = dir.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    };
    write(
        "broken/mod.kv",
        r#""Mod" { "Id" "broken" "Version" "1.0.0" "ApiVersion" "1" "ServerScripts" "1" }"#,
    );
    write(
        "broken/scripts/vscripts/broken/init.lua",
        "function oops(\n",
    );
    let set = ModSet::load(None, &dir, Some(&["broken".into()])).unwrap();
    let report = magicka_script::validate(Arc::new(set));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        report
            .errors
            .iter()
            .any(|e| e.contains("scripts/vscripts/broken/init.lua")),
        "{:?}",
        report.errors
    );
}
