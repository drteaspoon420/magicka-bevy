//! Plan suite `level/lua` (milestone M9): a custom level's encounter and
//! win condition run from mod data and Lua on the server, through the
//! native trigger runtime; a client joining mid-level sees the outcome
//! without executing Lua.
use magicka_network::{
    session::{PlayerProfile, ProfileSubmit},
    transport::{JoinRequest, LoopbackClient},
};
use magicka_runtime::{
    mod_runtime::ModRuntime, network_session::HostedSession, play_session::PlaySession,
    scene_loading::custom_hash,
};
use serde_json::json;
use std::path::{Path, PathBuf};

fn write(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn lua_level_runs_an_encounter_through_native_triggers_and_late_joiners_see_it() {
    let dir = std::env::temp_dir().join(format!("magicka-mod-level-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(
        &dir,
        "mods/arena/mod.kv",
        r#""Mod" { "Id" "arena" "Version" "1.0.0" "ApiVersion" "1" "ServerScripts" "1" }"#,
    );
    write(
        &dir,
        "mods/arena/scripts/levels/arena.kv",
        r#""Levels" { "havindr_arena" { "Level" "Havindr.lvl" "ScriptFile" "arena/level.lua" } }"#,
    );
    write(
        &dir,
        "mods/arena/scripts/vscripts/arena/level.lua",
        r#"
        local start, spawned, kills, won
        function OnLevelStart(e)
            Levels.SetObjective("clear", "Defeat the goblins", false)
            start = Players.Get(0).entity:GetPosition()
            Levels.WatchArea("arena", start, 4)
            won = Levels.DefineCondition("Counter", { name = "LuaWins", comparemethod = "EQUAL", value = 1 })
            Timers.Every(0.1, function()
                if Levels.ConditionMet(won) then NetTables.Set("arena", "won", true); return false end
            end)
        end
        function OnTriggerEnter(e)
            if e.area ~= "arena" or spawned then return end
            spawned = true
            Levels.SpawnWave("goblins", {
                { unit = "Goblin_pirate", position = { x = start.x + 4, y = start.y, z = start.z } },
                { unit = "Goblin_pirate", position = { x = start.x - 4, y = start.y, z = start.z } },
            })
        end
        function OnWaveStart(e) NetTables.Set("arena", "wave", e.wave) end
        function OnNpcKilled(e) kills = (kills or 0) + 1; NetTables.Set("arena", "kills", kills) end
        function OnWaveComplete(e)
            Levels.SetObjective("clear", "Defeat the goblins", true)
            Levels.Action("Counter", { name = "LuaWins", action = "set", value = 1 })
        end
        "#,
    );
    write(&dir, "mods/arena/scripts/vscripts/arena/init.lua", "");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut session = PlaySession::load(
        &root.join("../../../../target/Magicka/Content"),
        "Havindr.lvl",
        "Wizard",
    )
    .unwrap();
    // Authored triggers stay quiet; host calls still reach the runtime.
    session.disable_scripts();
    let mut mods = ModRuntime::load(None, &dir.join("mods"), Some(&["arena".into()]), 11)
        .unwrap_or_else(|e| panic!("{e}"));
    let _ = std::fs::remove_dir_all(&dir);
    mods.attach(&mut session);
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        "Havindr.lvl",
        false,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    host.set_content(mods.content.rules_hash.clone(), mods.content.mods.clone());
    let tick = |session: &mut PlaySession, mods: &mut ModRuntime, host: &mut HostedSession| {
        host.poll(session).unwrap();
        mods.step(session, 1.0 / 60.0);
        session.step(1.0 / 60.0);
        host.publish_mods(mods);
        host.publish(session).unwrap();
    };
    for _ in 0..10 {
        tick(&mut session, &mut mods, &mut host);
    }
    let tables = mods.scripts.net_tables();
    assert_eq!(
        tables["arena"]["wave"],
        json!("goblins"),
        "{:?}",
        session.world.log
    );
    let wave: Vec<u16> = mods.levels.waves["goblins"]
        .members
        .iter()
        .map(|e| e.handle)
        .collect();
    assert_eq!(wave.len(), 2);
    assert!(wave.iter().all(|h| session.world.npcs.contains_key(h)));
    assert_eq!(tables["objectives"]["clear"]["done"], json!(false));
    for h in &wave {
        session.smite(*h).unwrap();
    }
    // Death follows in Character.Update (after the spawn animation).
    for _ in 0..300 {
        tick(&mut session, &mut mods, &mut host);
        if mods.scripts.net_tables()["arena"].contains_key("won") {
            break;
        }
    }
    let tables = mods.scripts.net_tables();
    assert_eq!(tables["arena"]["kills"], json!(2));
    assert_eq!(tables["objectives"]["clear"]["done"], json!(true));
    // The native Counter action ran and the native condition saw it.
    assert_eq!(session.runtime.variables.counter(custom_hash("luawins")), 1);
    assert_eq!(tables["arena"]["won"], json!(true));
    assert!(
        !session.world.log.iter().any(|l| l.contains("mod error")),
        "{:?}",
        session.world.log
    );
    // A client joins after the encounter: it receives the outcome (mod UI
    // state and world) and runs no Lua at all.
    let mut client = LoopbackClient::join(
        host.transport.address(),
        JoinRequest {
            name: "Late".into(),
            profile: Some(ProfileSubmit {
                profile: PlayerProfile {
                    name: "Late".into(),
                    ..Default::default()
                },
                profile_id: None,
            }),
            rules_hash: mods.content.rules_hash.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    let mut frame = None;
    for _ in 0..400 {
        client.update();
        tick(&mut session, &mut mods, &mut host);
        if let Some(f) = client.take_game_frames().pop() {
            if client
                .player_slot()
                .is_some_and(|s| f.players.iter().any(|p| p.slot == s))
            {
                frame = Some(f);
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let frame = frame.expect("late joiner received the world");
    let entry = |table: &str, key: &str| {
        frame
            .mod_ui
            .iter()
            .find(|e| e.table == table && e.key == key)
            .map(|e| serde_json::from_str::<serde_json::Value>(&e.json).unwrap())
    };
    assert_eq!(entry("objectives", "clear").unwrap()["done"], json!(true));
    assert_eq!(entry("arena", "kills"), Some(json!(2)));
    // Exactly one wave was spawned: joining did not rerun level scripts.
    assert_eq!(mods.levels.waves.len(), 1);
    assert_eq!(mods.scripts.net_tables()["arena"]["kills"], json!(2));
}
