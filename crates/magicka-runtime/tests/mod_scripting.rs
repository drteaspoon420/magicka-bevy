//! Plan milestone M5 exit gate: a scripted passive and a scripted active work
//! through the authoritative simulation, and disabling the mod removes them.
use magicka_runtime::{mod_runtime::ModRuntime, play_session::PlaySession};
use std::path::{Path, PathBuf};

fn write(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn lua_passive_heals_and_hastes_the_holder_and_a_scripted_cast_uses_real_input() {
    let _root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut session = PlaySession::load(
        &magicka_content::retail_content_dir(),
        "Havindr.lvl",
        "Wizard",
    )
    .unwrap();
    session.disable_scripts();
    let staff = session.world.players[0]
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|(s, _)| *s == 1)
        .map(|(_, i)| i.definition.name.clone())
        .expect("a staff");
    let dir = std::env::temp_dir().join(format!("magicka-mod-scripting-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(
        &dir,
        "mods/blessed/mod.kv",
        r#""Mod" { "Id" "blessed" "Version" "1.0.0" "ApiVersion" "1" "ServerScripts" "1" }"#,
    );
    write(
        &dir,
        "mods/blessed/scripts/npc/items/staff.kv",
        &format!(r#""Items" {{ "staff" {{ "Retail" "{staff}" "Modifiers" "blessed:aura" }} }}"#),
    );
    write(
        &dir,
        "mods/blessed/scripts/npc/modifiers/aura.kv",
        r#""Modifiers" { "aura" { "TickInterval" "1" "ScriptFile" "blessed/aura.lua" "Properties" { "MoveSpeedPercent" "50" } } }"#,
    );
    write(
        &dir,
        "mods/blessed/scripts/vscripts/blessed/aura.lua",
        "function OnIntervalThink(ctx) Entities.Heal(ctx.target, 50) end",
    );
    write(
        &dir,
        "mods/blessed/scripts/vscripts/blessed/init.lua",
        r#"
        function OnPlayerJoin(p)
            NetTables.Set("blessed", "joined", p.name)
            Timers.After(0.5, function()
                local hero = Players.Get(p.slot).entity
                Spells.CastElements(hero, {"fire"}, { delivery = "spray" })
            end)
        end
        "#,
    );
    let mut mods =
        ModRuntime::load(None, &dir.join("mods"), Some(&["blessed".to_string()]), 7).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(mods.content.rules_hash.is_some());
    mods.attach(&mut session);
    let max = {
        let h = &mut session.world.players[0]
            .as_mut()
            .unwrap()
            .character
            .combat
            .health;
        h.hit_points = h.max_hit_points * 0.25;
        h.max_hit_points
    };
    let start = session.world.players[0]
        .as_ref()
        .unwrap()
        .character
        .combat
        .health
        .hit_points;
    let mut cast_seen = false;
    for _ in 0..(60 * 3 + 5) {
        mods.step(&mut session, 1.0 / 60.0);
        session.step(1.0 / 60.0);
        let p = session.world.players[0].as_ref().unwrap();
        cast_seen |= p
            .casting_spell()
            .is_some_and(|(_, s)| s.element.has(magicka_sim::spells::Elements::FIRE));
    }
    let p = session.world.players[0].as_ref().unwrap();
    let healed = p.character.combat.health.hit_points - start;
    assert!(
        (150.0..=150.0 + 1.0).contains(&healed) || p.character.combat.health.hit_points >= max,
        "three interval heals: {healed}"
    );
    assert_eq!(p.character.mod_speed, 1.5);
    assert_eq!(mods.scripts.net_tables()["blessed"]["joined"], "Player 1");
    assert!(
        cast_seen,
        "the scripted Fire spray went through the wizard's own cast path"
    );
    assert!(
        !session.world.log.iter().any(|l| l.contains("mod error")),
        "{:?}",
        session.world.log
    );
    // Disabling the mod removes its live effects.
    mods.scripts.unload_mod("blessed");
    mods.step(&mut session, 1.0 / 60.0);
    session.step(1.0 / 60.0);
    assert_eq!(
        session.world.players[0]
            .as_ref()
            .unwrap()
            .character
            .mod_speed,
        1.0
    );
    assert!(mods.scripts.modifiers().active().is_empty());
}
