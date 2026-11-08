//! Plan milestone M4 exit gate: a KV override changes a retail item's legal
//! numeric stat at runtime without Rust edits, and survives respawn.
use magicka_definitions::Definitions;
use magicka_mods::ModSet;
use magicka_runtime::play_session::PlaySession;
use std::{path::PathBuf, sync::Arc};

#[test]
fn kv_override_patches_held_items_and_their_replacements() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut session = PlaySession::load(
        &root.join("../../../../target/Magicka/Content"),
        "Havindr.lvl",
        "Wizard",
    )
    .unwrap();
    session.disable_scripts();
    let held: Vec<(i32, String, f32, i32)> = session.world.players[0]
        .as_ref()
        .unwrap()
        .items
        .iter()
        .map(|(slot, i)| {
            (
                *slot,
                i.definition.name.clone(),
                i.definition.cooldown_time,
                i.definition.block_value,
            )
        })
        .collect();
    let (slot, name, cooldown, _) = held
        .iter()
        .find(|(s, ..)| *s == 0 || *s == 1)
        .cloned()
        .expect("a held weapon or staff");
    // A mod outside the Rust tree, authored as data only.
    let dir = std::env::temp_dir().join(format!("magicka-item-defs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let files = [
        (
            "mods/tuning/mod.kv",
            "\"Mod\" { \"Id\" \"tuning\" \"Version\" \"1.0.0\" \"ApiVersion\" \"1\" }".to_string(),
        ),
        (
            "mods/tuning/scripts/npc/items/tuned.kv",
            format!(
                "\"Items\" {{ \"tuned_{0}\" {{ \"Retail\" \"{0}\" \"CooldownTime\" \"{1}\" \"BlockValue\" \"7\" \"MeleeRange\" \"3.5\" }} }}",
                name.to_ascii_lowercase(),
                cooldown + 2.0
            ),
        ),
    ];
    for (path, text) in &files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let set = ModSet::load(None, &dir.join("mods"), Some(&["tuning".to_string()])).unwrap();
    let defs = Arc::new(Definitions::load(&set).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
    session.set_definitions(defs);
    // (cooldown, block value, melee reach) of the overridden slot.
    let item = |session: &PlaySession| {
        let (_, i) = session.world.players[0]
            .as_ref()
            .unwrap()
            .items
            .iter()
            .find(|(s, _)| *s == slot)
            .unwrap();
        (
            i.definition.cooldown_time,
            i.definition.block_value,
            i.melee.range,
        )
    };
    assert_eq!(item(&session), (cooldown + 2.0, 7, 3.5));
    // Other items keep their retail values.
    for (s, n, c, b) in &held {
        if *s != slot && !n.eq_ignore_ascii_case(&name) {
            let other = session.world.players[0]
                .as_ref()
                .unwrap()
                .items
                .iter()
                .find(|(x, _)| x == s)
                .unwrap();
            assert_eq!(
                (
                    other.1.definition.cooldown_time,
                    other.1.definition.block_value
                ),
                (*c, *b)
            );
        }
    }
    // A respawn builds fresh items; the next step patches them before use.
    session.respawn_player(0, "Wizard").unwrap();
    session.step(1.0 / 60.0);
    assert_eq!(item(&session), (cooldown + 2.0, 7, 3.5));
}
