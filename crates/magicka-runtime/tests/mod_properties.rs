//! Modifier stat properties reach native gameplay: damage taken through the
//! native impact path, and maximum health (reverted exactly on removal).
use magicka_definitions::DefinitionId;
use magicka_runtime::{mod_runtime::ModRuntime, play_session::PlaySession};
use std::path::PathBuf;

#[test]
fn damage_taken_and_max_health_properties_apply_natively_and_revert() {
    let dir = std::env::temp_dir().join(format!("magicka-mod-props-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let write = |p: &str, t: &str| {
        let p = dir.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    };
    write(
        "mods/frail/mod.kv",
        r#""Mod" { "Id" "frail" "Version" "1.0.0" "ApiVersion" "1" }"#,
    );
    write(
        "mods/frail/scripts/npc/modifiers/m.kv",
        r#""Modifiers" { "glass" { "Properties" { "DamageTakenPercent" "100" "MaxHealthBonus" "250" "DamagePercent" "100" "CooldownPercent" "-50" } } }"#,
    );
    write(
        "mods/frail/scripts/npc/items/staff.kv",
        r#""Items" { "slow_staff" { "Retail" "staff_basic" "CooldownTime" "4" } }"#,
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut s = PlaySession::load(
        &root.join("../../../../target/Magicka/Content"),
        "Havindr.lvl",
        "Wizard",
    )
    .unwrap();
    s.disable_scripts();
    let mut mods = ModRuntime::load(None, &dir.join("mods"), Some(&["frail".into()]), 1).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    mods.attach(&mut s);
    let tick = |s: &mut PlaySession, m: &mut ModRuntime| {
        m.step(s, 1.0 / 60.0);
        s.step(1.0 / 60.0);
    };
    tick(&mut s, &mut mods);
    let handle = s.avatar().handle;
    let hp = |s: &PlaySession| s.avatar().character.combat.health.hit_points;
    let max = |s: &PlaySession| s.avatar().character.combat.health.max_hit_points;
    let hit = magicka_sim::damage::Damage {
        attack: magicka_sim::damage::attack::DAMAGE,
        element: 0,
        amount: 40.0,
        magnitude: 1.0,
    };
    let staff_cooldown = |s: &PlaySession| {
        s.avatar()
            .items
            .iter()
            .find(|(_, i)| i.definition.name.eq_ignore_ascii_case("staff_basic"))
            .unwrap()
            .1
            .definition
            .cooldown_time
    };
    let fire = magicka_sim::damage_delivery::DamagePayload::Single(magicka_sim::damage::Damage {
        attack: magicka_sim::damage::attack::DAMAGE,
        element: magicka_sim::spells::Elements::FIRE.0,
        amount: 30.0,
        magnitude: 1.0,
    });
    let boosted =
        |s: &PlaySession| match fire.with_boosts(&s.avatar().character.buffs.damage_boosts()) {
            magicka_sim::damage_delivery::DamagePayload::Single(d) => d.amount,
            _ => unreachable!(),
        };
    assert_eq!((boosted(&s), staff_cooldown(&s)), (30.0, 4.0));
    let (max0, before) = (max(&s), hp(&s));
    assert!(s.world.mod_damage(handle, hit, [0.0; 3]));
    let plain = before - hp(&s);
    assert!(plain > 0.0);
    let glass = DefinitionId::parse("frail:glass", "x").unwrap();
    let me = mods.entity(handle).unwrap();
    mods.scripts
        .apply_modifier(me, &glass, None, None, "frail")
        .unwrap();
    tick(&mut s, &mut mods);
    assert_eq!(max(&s), max0 + 250.0);
    let before = hp(&s);
    assert!(s.world.mod_damage(handle, hit, [0.0; 3]));
    assert!(
        (before - hp(&s) - 2.0 * plain).abs() < 1e-3,
        "doubled damage taken"
    );
    assert_eq!(boosted(&s), 60.0, "DamagePercent on the native boost list");
    assert_eq!(staff_cooldown(&s), 2.0, "CooldownPercent on held items");
    mods.scripts.remove_modifier(me, &glass);
    tick(&mut s, &mut mods);
    assert_eq!(max(&s), max0, "maximum restored");
    assert_eq!(s.avatar().character.combat.health.damage_taken_percent, 0.0);
    assert_eq!(
        (boosted(&s), staff_cooldown(&s)),
        (30.0, 4.0),
        "both revert"
    );
}
