//! Plan suite `definitions/validate`.
use magicka_definitions::{
    schema::{self, Change, Contribution, Layer},
    DefinitionId, Definitions, Handler, ITEM_FIELDS,
};
use magicka_mods::ModSet;
use magicka_sim::spells::{Elements, MAGICK_COMBOS};
use std::path::PathBuf;

struct Tree(PathBuf);
impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("magicka-defs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("mods")).unwrap();
        Self(root)
    }
    fn file(&self, path: &str, text: &str) -> &Self {
        let p = self.0.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
        self
    }
    fn mod_(&self, id: &str) -> &Self {
        self.file(
            &format!("mods/{id}/mod.kv"),
            &format!("\"Mod\" {{ \"Id\" \"{id}\" \"Version\" \"1.0.0\" \"ApiVersion\" \"1\" }}"),
        )
    }
    fn load(&self, enabled: &[&str]) -> Result<Definitions, Vec<String>> {
        let enabled: Vec<String> = enabled.iter().map(|s| s.to_string()).collect();
        let set = ModSet::load(
            Some(&self.0.join("base")),
            &self.0.join("mods"),
            Some(&enabled),
        )
        .map_err(|e| vec![e.to_string()])?;
        Definitions::load(&set).map_err(|e| e.0)
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn has(errors: &[String], needle: &str) -> bool {
    errors.iter().any(|e| e.contains(needle))
}

#[test]
fn builtins_register_every_native_magick_with_legacy_ids() {
    let defs = Definitions::builtin();
    assert_eq!(defs.magicks.len(), MAGICK_COMBOS.len());
    let revive = defs.magicks.get_str("base:revive").unwrap();
    assert_eq!(revive.legacy, Some(1));
    assert_eq!(revive.handler, Handler::Builtin("Revive".into()));
    assert_eq!(revive.combination, [Elements::LIFE, Elements::LIGHTNING]);
    assert_eq!(
        defs.magick_for(&[Elements::LIFE, Elements::LIGHTNING])
            .unwrap()
            .common
            .id
            .as_str(),
        "base:revive"
    );
}

#[test]
fn item_override_and_patches_change_numbers_without_rust() {
    let t = Tree::new("items");
    t.file(
        "base/scripts/npc/items/staffs.kv",
        r##"#base "../include/common.kv"
"Items"
{
    "base:clock_staff"
    {
        "Retail"        "staff_heirloom_grandfather_clock"
        "Slot"          "staff"
        "CooldownTime"  "4"
        "BlockValue"    "2"
    }
}"##,
    )
    .file(
        "base/scripts/npc/include/common.kv",
        r##""Items" { "base:plain_stick" { "Slot" "staff" } }"##,
    )
    .mod_("tuning")
    .file(
        "mods/tuning/scripts/npc/patches/tuning.kv",
        r##""Patches" { "Items" { "base:clock_staff" { "CooldownTime" "1.5" "Remove" "BlockValue" } } }"##,
    );
    let base = t.load(&[]).unwrap();
    let clock = base
        .retail_item("STAFF_heirloom_grandfather_clock")
        .unwrap();
    assert_eq!(clock.common.numbers["CooldownTime"], 4.0);
    assert_eq!(clock.common.numbers["BlockValue"], 2.0);
    assert!(
        base.items.get_str("base:plain_stick").is_some(),
        "#base include"
    );
    let tuned = t.load(&["tuning"]).unwrap();
    let clock = tuned
        .retail_item("staff_heirloom_grandfather_clock")
        .unwrap();
    assert_eq!(clock.common.numbers["CooldownTime"], 1.5);
    assert!(!clock.common.numbers.contains_key("BlockValue"));
    assert_ne!(base.rules_hash(), tuned.rules_hash());
    assert_eq!(base.rules_hash(), t.load(&[]).unwrap().rules_hash());
}

#[test]
fn errors_are_collected_with_locations_before_a_match() {
    let t = Tree::new("errors");
    t.file(
        "base/scripts/npc/items/bad.kv",
        r##""Items"
{
    "base:a" { "CooldownTime" "-3" }
    "base:b" { "GunClip" "1.5" "Speed" "3" }
    "base:c" { "Ability" "base:missing" "Model" "models/nothing.glb" }
}"##,
    )
    .file(
        "base/scripts/npc/items/dupe.kv",
        r##""Items" { "base:a" { } }"##,
    )
    .file(
        "base/scripts/npc/abilities/a.kv",
        r##""Abilities" { "base:zap" { "ScriptFile" "zap.lua" "Behavior" "sometimes" } }"##,
    );
    let errors = t.load(&[]).unwrap_err();
    assert!(has(&errors, "scripts/npc/items/bad.kv:3:"), "{errors:#?}");
    assert!(has(&errors, "CooldownTime = -3 is outside"), "{errors:#?}");
    assert!(has(&errors, "GunClip must be an integer"));
    assert!(has(&errors, "unknown field Speed"));
    assert!(has(&errors, "unknown ability base:missing"));
    assert!(has(&errors, "asset models/nothing.glb not found"));
    assert!(has(&errors, "duplicate id base:a"));
    assert!(has(&errors, "scripts/vscripts/zap.lua not found"));
    assert!(has(&errors, "Behavior must be one of"));
}

#[test]
fn magick_combinations_must_be_typeable_and_unique() {
    let t = Tree::new("magicks");
    t.mod_("ex")
        .file("mods/ex/scripts/vscripts/magicks/storm.lua", "function OnSpellStart(ctx) end")
        .file(
            "mods/ex/scripts/npc/magicks/m.kv",
            r##""Magicks"
{
    "storm_lance" { "Combination" "lightning,arcane,shield" "ScriptFile" "magicks/storm.lua" "Handler" "OnSpellStart" "NameKey" "#storm" }
    "steam_trick" { "Combination" "fire,water" }
    "copycat"     { "Combination" "life,lightning" }
}"##,
        );
    let errors = t.load(&["ex"]).unwrap_err();
    assert!(
        has(
            &errors,
            "ex:steam_trick: combination fire,water cannot be typed"
        ),
        "{errors:#?}"
    );
    assert!(
        has(&errors, "is already base:revive") || has(&errors, "ex:copycat"),
        "{errors:#?}"
    );
    // Valid alone; the custom Magick follows every built-in.
    t.file(
        "mods/ex/scripts/npc/magicks/m.kv",
        r##""Magicks" { "storm_lance" { "Combination" "lightning,arcane,shield" "ScriptFile" "magicks/storm.lua" "NameKey" "#storm" } }"##,
    );
    let defs = t.load(&["ex"]).unwrap();
    let id = DefinitionId::parse("ex:storm_lance", "x").unwrap();
    assert_eq!(
        defs.magicks.index_of(&id).unwrap().0 as usize,
        MAGICK_COMBOS.len()
    );
    assert!(defs
        .warnings
        .iter()
        .any(|w| w.contains("missing localization #storm")));
    t.file(
        "mods/ex/localization/english/strings.kv",
        r##""Tokens" { "#storm" "Storm Lance" }"##,
    );
    let defs = t.load(&["ex"]).unwrap();
    assert!(defs.warnings.is_empty(), "{:?}", defs.warnings);
    assert_eq!(
        defs.localization.lookup("finnish", "#storm"),
        Some("Storm Lance")
    );
}

#[test]
fn builtin_magick_config_moves_to_kv_while_keeping_its_rust_handler() {
    let t = Tree::new("builtin");
    t.mod_("cfg").file(
        "mods/cfg/scripts/npc/patches/haste.kv",
        r##""Patches" { "Magicks" { "base:haste" { "Cooldown" "3" } } }"##,
    );
    let defs = t.load(&["cfg"]).unwrap();
    let haste = defs.magicks.get_str("base:haste").unwrap();
    assert_eq!(haste.common.numbers["Cooldown"], 3.0);
    assert_eq!(haste.handler, Handler::Builtin("Haste".into()));
    assert_eq!(haste.legacy, Some(3));
    assert_eq!(
        haste.combination,
        [Elements::LIGHTNING, Elements::ARCANE, Elements::FIRE]
    );
}

#[test]
fn stat_resolution_layers_upgrades_then_temporary_effects_and_clamps() {
    let cooldown = schema::find(ITEM_FIELDS, "cooldowntime").unwrap();
    let c = |layer, change| Contribution {
        layer,
        change,
        source: "test".into(),
    };
    assert_eq!(schema::resolve(cooldown, 4.0, &[]), 4.0);
    assert_eq!(
        schema::resolve(
            cooldown,
            4.0,
            &[
                c(Layer::Temporary, Change::Percent(-50.0)),
                c(Layer::Upgrade, Change::Add(-1.0))
            ]
        ),
        1.5
    );
    // Never below the legal range.
    assert_eq!(
        schema::resolve(cooldown, 4.0, &[c(Layer::Upgrade, Change::Add(-10.0))]),
        0.0
    );
    let clip = schema::find(ITEM_FIELDS, "GunClip").unwrap();
    assert_eq!(
        schema::resolve(clip, 10.0, &[c(Layer::Upgrade, Change::Multiply(1.26))]),
        13.0
    );
    let damage =
        &schema::float("Damage", "×", 0.0, 100.0).aggregation(schema::Aggregation::Multiplicative);
    assert!(
        (schema::resolve(
            damage,
            1.0,
            &[
                c(Layer::Upgrade, Change::Percent(10.0)),
                c(Layer::Upgrade, Change::Percent(10.0))
            ]
        ) - 1.21)
            .abs()
            < 1e-9
    );
}
