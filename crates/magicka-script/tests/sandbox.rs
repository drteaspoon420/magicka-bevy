//! Plan suite `scripting/sandbox`.
mod common;
use common::*;
use magicka_script::{Command, Limits};
use serde_json::json;

#[test]
fn dangerous_libraries_are_absent_and_require_uses_only_the_mod_filesystem() {
    let t = Tree::new("forbidden");
    t.script_mod(
        "probe",
        r#"
        local util = require("probe.util")
        local again = require("probe.util")
        function OnGameStart()
            local absent = {}
            for _, name in ipairs({"os","io","debug","package","load","loadfile","dofile","collectgarbage","loadstring"}) do
                if _ENV[name] ~= nil then table.insert(absent, name) end
            end
            NetTables.Set("probe", "present", absent)
            NetTables.Set("probe", "dump", string.dump == nil)
            NetTables.Set("probe", "stringmeta", getmetatable("") == false)
            NetTables.Set("probe", "util", util.answer)
            NetTables.Set("probe", "cached", util == again)
            NetTables.Set("probe", "escape", (pcall(require, "../../etc/passwd")))
            NetTables.Set("probe", "missing", (pcall(require, "probe.nope")))
            NetTables.Set("probe", "cycle", (pcall(require, "probe.cycle")))
        end
        "#,
    )
    .file("mods/probe/scripts/vscripts/probe/util.lua", "return { answer = 42 }")
    .file(
        "mods/probe/scripts/vscripts/probe/cycle.lua",
        "return require('probe.cycle')",
    );
    let mut host = t.host(&["probe"], Limits::default());
    host.set_snapshot(snapshot(1));
    host.lifecycle(magicka_script::Lifecycle::GameStart, json!(null));
    let tables = host.net_tables();
    let probe = &tables["probe"];
    assert_eq!(probe["present"], json!({}), "{probe:?}");
    assert_eq!(probe["dump"], json!(true));
    assert_eq!(probe["stringmeta"], json!(true));
    assert_eq!(probe["util"], json!(42));
    assert_eq!(probe["cached"], json!(true));
    assert_eq!(probe["escape"], json!(false));
    assert_eq!(probe["missing"], json!(false));
    assert_eq!(probe["cycle"], json!(false));
}

#[test]
fn runaway_scripts_hit_budgets_without_taking_the_host_down() {
    let t = Tree::new("budget");
    t.script_mod(
        "spin",
        r#"
        Events.Subscribe("spin", function() while true do end end)
        Events.Subscribe("hog", function()
            local t = {}
            for i = 1, 1e9 do t[i] = string.rep("x", 1024) .. i end
        end)
        Events.Subscribe("deep", function() local function f(n) return f(n + 1) + 1 end return f(1) end)
        Events.Subscribe("ok", function(e) NetTables.Set("spin", "ok", e.n) end)
        "#,
    );
    let mut host = t.host(
        &["spin"],
        Limits {
            instructions: 200_000,
            memory_bytes: 16 << 20,
            ..Default::default()
        },
    );
    host.emit("spin", &json!({}));
    host.emit("hog", &json!({}));
    host.emit("deep", &json!({}));
    let errors = host.take_errors();
    assert!(
        errors[0].message.contains("instruction budget"),
        "{errors:?}"
    );
    assert!(
        errors[1].message.to_lowercase().contains("memory") || errors[1].message.contains("budget"),
        "{errors:?}"
    );
    assert!(errors[2].message.contains("stack overflow") || errors[2].message.contains("budget"));
    assert!(errors
        .iter()
        .all(|e| e.owner == "spin" && e.callback.starts_with("event ")));
    // The VM still works afterwards.
    host.emit("ok", &json!({"n": 5}));
    assert_eq!(host.net_tables()["spin"]["ok"], json!(5));
}

#[test]
fn stale_handles_resolve_to_nothing_and_errors_are_isolated_per_mod() {
    let t = Tree::new("isolation");
    t.script_mod(
        "broken",
        r#"Events.Subscribe("hit", function(e) error("boom on " .. tostring(e.target)) end, 10)"#,
    )
    .script_mod(
        "steady",
        r#"
        Events.Subscribe("hit", function(e)
            NetTables.Set("steady", "valid", e.target:IsValid())
            NetTables.Set("steady", "health", e.target:GetHealth())
            Entities.Damage(e.target, 5)
        end)
        "#,
    );
    let mut host = t.host(&["broken", "steady"], Limits::default());
    host.set_snapshot(snapshot(1));
    // A handle whose generation is gone.
    let stale = json!({"target": {"__entity": 9, "generation": 0}});
    host.emit("hit", &stale);
    let errors = host.take_errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].owner, "broken");
    assert!(
        errors[0].message.contains("boom on Entity(9#0)"),
        "{}",
        errors[0].message
    );
    assert!(
        errors[0].message.contains("broken/init.lua"),
        "traceback: {}",
        errors[0].message
    );
    let tables = host.net_tables();
    assert_eq!(tables["steady"]["valid"], json!(false));
    assert_eq!(tables["steady"].get("health"), None);
    // Commands are typed and carry their owner.
    let commands = host.take_commands();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].owner, "steady");
    assert!(matches!(commands[0].command, Command::Damage { amount, .. } if amount == 5.0));
}

#[test]
fn unloading_a_mod_removes_every_live_registration() {
    let t = Tree::new("unload");
    t.file(
        "mods/boost/scripts/npc/modifiers/m.kv",
        r#""Modifiers" { "haste" { "Duration" "100" "Properties" { "MoveSpeedPercent" "20" } } }"#,
    )
    .script_mod(
        "boost",
        r#"
        function OnGameStart()
            Events.Subscribe("tick", function() end)
            Events.AddFilter("damage", function() return false end)
            Timers.Every(1, function() end)
            NetTables.Set("boost", "gold", 10)
            Modifiers.Apply(Players.Get(0).entity, "boost:haste")
        end
        function OnModUnload() NetTables.Set("boost", "farewell", true) end
        "#,
    );
    let mut host = t.host(&["boost"], Limits::default());
    host.set_snapshot(snapshot(1));
    host.lifecycle(magicka_script::Lifecycle::GameStart, json!(null));
    assert_eq!(host.owned("boost"), (2, 1, 1, 1));
    assert_eq!(host.modifier_properties(HERO)["MoveSpeedPercent"], 20.0);
    assert_eq!(
        host.filter("damage", &json!({"amount": 10})),
        magicka_script::Verdict::Deny
    );
    host.unload_mod("boost");
    assert_eq!(host.owned("boost"), (0, 0, 0, 0));
    assert!(host.modifier_properties(HERO).is_empty());
    assert_eq!(
        host.filter("damage", &json!({"amount": 10})),
        magicka_script::Verdict::Allow
    );
    assert!(host.net_tables().get("boost").is_none_or(|t| t.is_empty()));
    assert!(host.take_net_table_changes().contains_key("boost"));
}
