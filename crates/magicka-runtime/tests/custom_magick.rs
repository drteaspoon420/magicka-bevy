//! Plan milestone M7: a mod registers a previously unknown Magick with KV +
//! Lua; it appears in the tome, is cast through its element combination
//! on the authoritative wizard, replicates by ID and survives save/restore.
use magicka_network::control::Controls;
use magicka_runtime::{mod_runtime::ModRuntime, play_session::PlaySession};
use std::path::{Path, PathBuf};

fn write(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}
fn content() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
}
fn mod_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("magicka-custom-magick-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(
        &dir,
        "mods/my_magicks/mod.kv",
        r#""Mod" { "Id" "example.my_magicks" "Version" "1.0.0" "ApiVersion" "1" "ServerScripts" "1" "ClientAssets" "1" }"#,
    );
    write(
        &dir,
        "mods/my_magicks/scripts/npc/magicks/storm.kv",
        r##""Magicks"
{
    "storm_lance"
    {
        "NameKey"           "#example_storm_lance"
        "Combination"       "lightning,arcane,shield"
        "UnlockedByDefault" "1"
        "ScriptFile"        "example.my_magicks/storm.lua"
        "Handler"           "OnSpellStart"
        "Particle"          "example.my_magicks:storm_fx"
    }
    "quiet_word" { "Combination" "earth,cold,shield" "ScriptFile" "example.my_magicks/storm.lua" "Handler" "OnQuiet" }
}"##,
    );
    write(
        &dir,
        "mods/my_magicks/scripts/npc/effects/fx.kv",
        r#""Effects" { "storm_fx" { "Kind" "emitter" "Asset" "content:Effects/fire_blast" } }"#,
    );
    write(
        &dir,
        "mods/my_magicks/localization/english/strings.kv",
        r##""Tokens" { "#example_storm_lance" "Storm Lance" }"##,
    );
    write(
        &dir,
        "mods/my_magicks/scripts/vscripts/example.my_magicks/storm.lua",
        r#"
        local casts = 0
        function OnSpellStart(ctx)
            casts = casts + 1
            NetTables.Set("storm", "casts", casts)
            Spells.CastElements(ctx.caster, {"lightning", "arcane"}, { delivery = "beam" })
            -- The second cast teaches the player another Magick.
            if casts == 1 then Magicks.Unlock(ctx.caster:GetPlayer(), "example.my_magicks:quiet_word") end
        end
        function OnQuiet(ctx) end
        "#,
    );
    write(
        &dir,
        "mods/my_magicks/scripts/vscripts/example.my_magicks/init.lua",
        "",
    );
    dir
}

/// One host tick: the player's input (ignored while a script drives the
/// wizard, as `HostedSession` does), mod logic, then the simulation.
fn press(session: &mut PlaySession, mods: &mut ModRuntime, controls: Controls) {
    if !session.scripted_input[0] {
        session.handle_remote_input(0, controls).unwrap();
    }
    mods.step(session, 1.0 / 60.0);
    session.step(1.0 / 60.0);
}

#[test]
fn registered_magick_is_typed_cast_replicated_and_saved() {
    let dir = mod_dir();
    let mut session = PlaySession::load(&content(), "Havindr.lvl", "Wizard").unwrap();
    session.disable_scripts();
    let mut mods = ModRuntime::load(
        None,
        &dir.join("mods"),
        Some(&["example.my_magicks".into()]),
        3,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    mods.attach(&mut session);
    let storm =
        magicka_definitions::DefinitionId::parse("example.my_magicks:storm_lance", "x").unwrap();
    let storm_index = mods.definitions.magicks.index_of(&storm).unwrap().0;
    assert!(storm_index as usize >= magicka_sim::spells::MAGICK_COMBOS.len());
    for _ in 0..10 {
        press(&mut session, &mut mods, Controls::default());
    }
    assert_eq!(session.registered_unlocked[0], vec![storm_index]);
    // The tome cycles natives, then this player's registered Magicks.
    let allowed = |_| false;
    session.tome[0].next_with(allowed, &session.registered_unlocked[0].clone());
    assert_eq!(session.tome[0].registered, Some(storm_index));
    for _ in 0..30 {
        session.tome[0].update(1.0 / 60.0);
    }
    assert_eq!(session.tome[0].shown_registered, Some(storm_index));
    // Type Lightning, Arcane, Shield (ELEMENT_KEYS 4, 5, 2), then Magick.
    for key in [4, 5, 2] {
        let mut down = Controls::default();
        down.elements[key] = true;
        press(&mut session, &mut mods, down);
        press(&mut session, &mut mods, Controls::default());
    }
    let queue: Vec<_> = session.world.players[0]
        .as_ref()
        .unwrap()
        .conjure
        .queue
        .iter()
        .map(|s| s.element)
        .collect();
    assert_eq!(queue.len(), 3, "{queue:?}");
    press(
        &mut session,
        &mut mods,
        Controls {
            boost_down: true,
            ..Default::default()
        },
    );
    let mut beam_seen = false;
    for _ in 0..120 {
        press(&mut session, &mut mods, Controls::default());
        let p = session.world.players[0].as_ref().unwrap();
        beam_seen |= p.casting_spell().is_some_and(|(_, s)| {
            s.element.has(magicka_sim::spells::Elements::ARCANE)
                && s.element.has(magicka_sim::spells::Elements::LIGHTNING)
        });
    }
    assert_eq!(
        mods.scripts.net_tables()["storm"]["casts"],
        1,
        "{:?}",
        session.world.log
    );
    assert!(
        beam_seen,
        "the handler's Lightning+Arcane beam went through the wizard's cast path"
    );
    assert!(
        mods.presentation
            .iter()
            .any(|e| e.name == "example.my_magicks:storm_fx"),
        "the Magick's particle is a presentation event"
    );
    // Replication: unlocked registered Magicks travel by ID.
    let mut host = magicka_runtime::network_session::HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        "Havindr.lvl",
        false,
        Default::default(),
        &session,
    )
    .unwrap();
    host.publish_mods(&mut mods);
    let frame = host.frame(&session).unwrap();
    let ids = &frame
        .players
        .iter()
        .find(|p| p.slot == 0)
        .unwrap()
        .registered_magicks;
    // Registry order: built-ins, then mod Magicks by ID.
    assert_eq!(
        ids,
        &[
            "example.my_magicks:quiet_word",
            "example.my_magicks:storm_lance"
        ]
    );
    // Save and restore: the script-granted unlock survives in a fresh match.
    press(&mut session, &mut mods, Controls::default());
    for _ in 0..40 {
        press(&mut session, &mut mods, Controls::default());
    }
    // Campaign saves belong to persistent profiles (the frontend's IDs).
    session.player_ids[0] = Some("00000000000000000000000000000001".into());
    let save = session.disk_save().unwrap();
    assert!(save["mods"]["custom_unlocks"]["0"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "example.my_magicks:quiet_word"));
    let mut restored = PlaySession::load_disk_save(&content(), &save).unwrap();
    restored.disable_scripts();
    let mut mods2 = ModRuntime::load(
        None,
        &dir.join("mods"),
        Some(&["example.my_magicks".into()]),
        3,
    )
    .unwrap();
    mods2.attach(&mut restored);
    mods2.step(&mut restored, 1.0 / 60.0);
    assert_eq!(restored.registered_unlocked[0].len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn more_than_sixty_four_registered_magicks_coexist_with_the_natives() {
    use magicka_definitions::{check_combination, element_name, Definitions};
    use magicka_sim::spells::{Elements, MAGICK_COMBOS};
    let basics = &Elements::BY_INDEX[..8];
    let natives: Vec<Vec<Elements>> = MAGICK_COMBOS.iter().map(|(_, c)| c.to_vec()).collect();
    let mut combos = Vec::new();
    'search: for a in basics {
        for b in basics {
            for c in basics {
                for d in basics {
                    let combo = vec![*a, *b, *c, *d, Elements::SHIELD];
                    if check_combination(&combo).is_ok() && !natives.contains(&combo) {
                        combos.push(combo);
                        if combos.len() == 70 {
                            break 'search;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(combos.len(), 70);
    let dir = std::env::temp_dir().join(format!("magicka-many-magicks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(
        &dir,
        "mods/many/mod.kv",
        r#""Mod" { "Id" "many" "Version" "1.0.0" "ApiVersion" "1" }"#,
    );
    let mut kv = String::from("\"Magicks\"\n{\n");
    for (i, combo) in combos.iter().enumerate() {
        let names: Vec<_> = combo.iter().map(|e| element_name(*e)).collect();
        kv.push_str(&format!(
            "  \"m{i}\" {{ \"Combination\" \"{}\" }}\n",
            names.join(",")
        ));
    }
    kv.push('}');
    write(&dir, "mods/many/scripts/npc/magicks/many.kv", &kv);
    let set = magicka_mods::ModSet::load(None, &dir.join("mods"), Some(&["many".into()])).unwrap();
    let defs = Definitions::load(&set).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(defs.magicks.len(), MAGICK_COMBOS.len() + 70);
    let last = defs
        .magicks
        .at(magicka_definitions::RegistryIndex(
            (defs.magicks.len() - 1) as u16,
        ))
        .unwrap();
    let index = defs.magicks.index_of(&last.common.id).unwrap().0;
    assert!(index > 64);
    // The index survives the spell and its network form.
    let spell = magicka_sim::spells::Spell::registered_magick(index);
    let wire = magicka_network::session::QueuedSpell::from(&spell);
    assert_eq!(wire.spell().unwrap().registered_index(), Some(index));
    // Native Magicks are untouched by the registered ones.
    assert_eq!(
        defs.magick_for(&[Elements::LIFE, Elements::LIGHTNING])
            .unwrap()
            .legacy,
        Some(1)
    );
}
