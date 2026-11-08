//! Plan suite `scripting/abilities`: modifiers, stacking, scripted spells,
//! timers, filters, RNG replay and saved state.
mod common;
use common::*;
use magicka_definitions::{DefinitionId, Handler};
use magicka_script::{Command, Lifecycle, Limits, Verdict};
use serde_json::json;

fn tree() -> Tree {
    let t = Tree::new("abilities");
    t.file(
        "mods/ex/scripts/npc/modifiers/m.kv",
        r#""Modifiers"
{
    "regen"  { "Duration" "3" "TickInterval" "1" "ScriptFile" "ex/regen.lua" "Properties" { "HealthRegen" "5" } }
    "rage"   { "Stacking" "stack" "MaxStacks" "3" "Duration" "10" "Properties" { "DamagePercent" "10" } }
    "shiny"  { "Stacking" "independent" }
}"#,
    )
    .file(
        "mods/ex/scripts/npc/magicks/m.kv",
        r#""Magicks" { "storm_lance" { "Combination" "lightning,arcane,shield" "ScriptFile" "ex/storm.lua" "Handler" "OnSpellStart" } }"#,
    )
    .file(
        "mods/ex/scripts/vscripts/ex/regen.lua",
        r#"
        function OnCreated(ctx) NetTables.Set("ex", "created", ctx.modifier) end
        function OnIntervalThink(ctx) Entities.Heal(ctx.target, 5) end
        function OnDestroy(ctx) NetTables.Set("ex", "destroyed", ctx.instance) end
        "#,
    )
    .file(
        "mods/ex/scripts/vscripts/ex/storm.lua",
        r#"
        function OnSpellStart(ctx)
            Spells.CastElements(ctx.caster, {"lightning", "arcane"}, {
                delivery = "projectile", target_position = ctx.target_position })
            Particles.Create("ex:storm_fx", { owner = ctx.caster, position = ctx.target_position })
            return { cast = true }
        end
        "#,
    )
    .script_mod(
        "ex",
        r#"
        function OnGameStart()
            Events.AddFilter("damage", function(e)
                if e.amount > 100 then return false end
                return { amount = e.amount * 2, target = "ignored" }
            end)
            Events.AddFilter("purchase", function(e) error("shop closed") end)
            Timers.After(0.5, function() NetTables.Set("ex", "after", GameRules.GetGameTime()) end)
            local n = 0
            Timers.Every(1, function() n = n + 1; NetTables.Set("ex", "every", n); if n == 3 then return false end end)
            NetTables.Set("ex", "rolls", { math.random(1, 100), Random.Int(1, 100), math.random(1, 100) })
        end
        function SaveState() return { bank = NetTables.Get("ex", "every") or 0 } end
        function LoadState(s) NetTables.Set("ex", "restored", s.bank) end
        "#,
    );
    t
}

#[test]
fn modifiers_follow_stacking_rules_tick_and_expire_through_lua() {
    let t = tree();
    let mut host = t.host(&["ex"], Limits::default());
    host.set_snapshot(snapshot(1));
    let id = |s: &str| DefinitionId::parse(s, "ex").unwrap();
    host.apply_modifier(HERO, &id("regen"), None, None, "ex")
        .unwrap();
    assert_eq!(host.net_tables()["ex"]["created"], json!("ex:regen"));
    for _ in 0..5 {
        host.apply_modifier(GOBLIN, &id("rage"), None, Some(HERO), "ex")
            .unwrap();
    }
    host.apply_modifier(GOBLIN, &id("shiny"), None, None, "ex")
        .unwrap();
    host.apply_modifier(GOBLIN, &id("shiny"), None, None, "ex")
        .unwrap();
    let store = host.modifiers();
    assert_eq!(store.stacks(GOBLIN, &id("rage")), 3, "capped at MaxStacks");
    assert_eq!(
        store
            .on(GOBLIN)
            .filter(|m| m.modifier == id("shiny"))
            .count(),
        2
    );
    assert_eq!(host.modifier_properties(GOBLIN)["DamagePercent"], 30.0);
    for _ in 0..4 {
        host.update_modifiers(1.0);
    }
    let heals: Vec<_> = host
        .take_commands()
        .into_iter()
        .filter(|c| matches!(c.command, Command::Heal { target, amount } if target == HERO && amount == 5.0))
        .collect();
    assert_eq!(heals.len(), 3, "one heal per TickInterval for 3 s");
    assert!(!host.modifiers().has(HERO, &id("regen")));
    assert!(host.net_tables()["ex"].contains_key("destroyed"));
}

#[test]
fn scripted_magick_handler_casts_a_real_element_spell() {
    let t = tree();
    let mut host = t.host(&["ex"], Limits::default());
    host.set_snapshot(snapshot(1));
    let handler = Handler::Lua {
        file: "ex/storm.lua".into(),
        function: "OnSpellStart".into(),
    };
    let ctx = json!({
        "caster": magicka_script::convert::entity_json(HERO),
        "target_position": {"x": 1.0, "y": 0.0, "z": 2.0},
    });
    let result = host
        .call_handler(&handler, &ctx, Some(HERO), false)
        .unwrap();
    assert_eq!(result, json!({"cast": true}));
    let commands: Vec<_> = host
        .take_commands()
        .into_iter()
        .map(|c| c.command)
        .collect();
    assert_eq!(
        commands[0],
        Command::CastElements {
            caster: HERO,
            elements: vec!["lightning".into(), "arcane".into()],
            delivery: "projectile".into(),
            target_position: Some([1.0, 0.0, 2.0]),
        }
    );
    assert!(
        matches!(&commands[1], Command::Particle { effect, owner: Some(HERO), .. } if effect == "ex:storm_fx")
    );
    let missing = Handler::Lua {
        file: "ex/storm.lua".into(),
        function: "Nope".into(),
    };
    assert!(host.call_handler(&missing, &ctx, None, false).is_err());
    assert_eq!(
        host.call_handler(&missing, &ctx, None, true).unwrap(),
        json!(null)
    );
}

#[test]
fn filters_modify_only_allowed_fields_and_fail_closed_where_required() {
    let t = tree();
    let mut host = t.host(&["ex"], Limits::default());
    host.set_snapshot(snapshot(1));
    host.lifecycle(Lifecycle::GameStart, json!(null));
    match host.filter("damage", &json!({"amount": 10.0, "target": "hero"})) {
        Verdict::Modified(m) => {
            assert_eq!(m["amount"], json!(20.0));
            assert_eq!(m["target"], json!("hero"), "immutable field kept");
        }
        v => panic!("{v:?}"),
    }
    assert!(host
        .take_errors()
        .iter()
        .any(|e| e.message.contains("target is not modifiable")));
    assert_eq!(
        host.filter("damage", &json!({"amount": 500.0})),
        Verdict::Deny
    );
    // An erroring authority-sensitive filter denies.
    assert_eq!(host.filter("purchase", &json!({"price": 3})), Verdict::Deny);
    assert!(host
        .take_errors()
        .iter()
        .any(|e| e.message.contains("shop closed")));
    assert_eq!(host.filter("heal", &json!({"amount": 3})), Verdict::Allow);
}

#[test]
fn timers_run_on_simulation_time_and_rng_replays_from_the_seed() {
    let t = tree();
    let run = || {
        let mut host = t.host(&["ex"], Limits::default());
        host.set_snapshot(snapshot(1));
        host.lifecycle(Lifecycle::GameStart, json!(null));
        for step in 1..=10 {
            host.tick(step as f64 * 0.5);
        }
        host.net_tables()["ex"].clone()
    };
    let a = run();
    // Created at 1/60 s, due at 0.5167 s: the first tick at or past it.
    assert_eq!(a["after"], json!(1.0));
    assert_eq!(a["every"], json!(3), "Every stops after returning false");
    let rolls = a["rolls"].as_array().unwrap();
    assert!(rolls
        .iter()
        .all(|r| (1..=100).contains(&r.as_i64().unwrap())));
    assert_eq!(a, run(), "same seed, same outcome");
}

#[test]
fn mod_state_saves_without_vm_objects_and_restores() {
    let t = tree();
    let mut host = t.host(&["ex"], Limits::default());
    host.set_snapshot(snapshot(1));
    host.lifecycle(Lifecycle::GameStart, json!(null));
    for step in 1..=4 {
        host.tick(step as f64);
    }
    host.apply_modifier(
        GOBLIN,
        &DefinitionId::parse("ex:rage", "ex").unwrap(),
        None,
        None,
        "ex",
    )
    .unwrap();
    let saved = host.save_state();
    assert_eq!(saved["mods"]["ex"], json!({"bank": 3}));
    let text = serde_json::to_string(&saved).unwrap();
    let mut fresh = t.host(&["ex"], Limits::default());
    fresh.set_snapshot(snapshot(1));
    fresh.load_state(&serde_json::from_str(&text).unwrap());
    assert_eq!(fresh.net_tables()["ex"]["restored"], json!(3));
    assert_eq!(
        fresh
            .modifiers()
            .stacks(GOBLIN, &DefinitionId::parse("ex:rage", "ex").unwrap()),
        1
    );
}
