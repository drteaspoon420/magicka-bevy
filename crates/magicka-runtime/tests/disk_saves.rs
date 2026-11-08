use magicka_runtime::{
    play_session::PlaySession,
    saves::{new_player_id, SaveStore},
    scene_loading::custom_hash,
    scene_script::SceneScriptLayout,
    script_runtime::ScriptRuntime,
};
use serde_json::json;
use std::path::Path;

#[test]
fn completed_dialog_script_fact_survives_boundary_restore() {
    use magicka_runtime::{dialog_definition::DialogDefinition, dialog_manager::DialogManager};
    let xml=br#"<Dialogs><Dialog id="completed" repeatFrom="-1"><interact><message><text>#TEST</text></message></interact></Dialog></Dialogs>"#;
    let mut source =
        DialogManager::new(DialogDefinition::read_collection(&[xml]).unwrap()).unwrap();
    let mut snapshot = source.disk_boundary().unwrap();
    snapshot[0][1] = json!(1);
    snapshot[0][2] = json!(1);
    snapshot[0][3] = json!(true);
    snapshot[0][4][0][1] = json!(1);
    source.restore_disk_boundary(&snapshot).unwrap();
    assert!(source.is_done(custom_hash("completed"), 0).unwrap());
    let mut restored =
        DialogManager::new(DialogDefinition::read_collection(&[xml]).unwrap()).unwrap();
    restored
        .restore_disk_boundary(&source.disk_boundary().unwrap())
        .unwrap();
    assert!(restored.is_done(custom_hash("completed"), 0).unwrap());
    assert_eq!(restored.disk_boundary().unwrap(), snapshot);
}

#[test]
fn atomic_store_keeps_identity_settings_and_slot_separate() {
    let root = std::env::temp_dir().join(format!("magicka-save-test-{}", new_player_id().unwrap()));
    let store = SaveStore::new(&root);
    let id = new_player_id().unwrap();
    assert_ne!(id, new_player_id().unwrap());
    let mut settings = store.settings().unwrap();
    settings["music"] = json!(0.25);
    settings["sfx"] = json!(0.75);
    settings["profiles"] =
        json!([{"id":id,"name":"Tester","robe":"Wizard","colour":[0.2,0.5,0.9]}]);
    settings["history"] = json!(["localhost:27015"]);
    store.save_settings(&settings).unwrap();
    assert_eq!(store.settings().unwrap(), settings);
    let save =
        json!({"version":1,"players":[{"id":id,"equipment":["weapon_sword"]}],"scene":"wc_s3"});
    store.save_slot(2, &save).unwrap();
    assert_eq!(store.load_slot(2).unwrap(), Some(save.clone()));
    assert!(store.save_slot(2, &json!({"version":900})).is_err());
    assert_eq!(store.load_slot(2).unwrap(), Some(save));
    assert!(store.slot_path(8).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn script_state_roundtrip_keeps_queues_flags_and_variables() {
    let xml=br#"<Scene><trigger id="once"><then><counter name="flag" action="set" value="1" delay="3"/></then></trigger></Scene>"#;
    let mut source = ScriptRuntime::new(
        SceneScriptLayout::read(xml, false).unwrap(),
        Default::default(),
    );
    source.variables.set_counter(custom_hash("persistent"), 77);
    source.variables.add_timer(19, false, 4.25);
    source.queue_mut(0).unwrap().trigger();
    source.queue_mut(0).unwrap().state.countdown = 1.25;
    let state = source.disk_state();
    let mut restored = ScriptRuntime::new(
        SceneScriptLayout::read(xml, false).unwrap(),
        Default::default(),
    );
    restored.restore_disk_state(&state).unwrap();
    assert_eq!(restored.disk_state(), state);
    let mut invalid = state.clone();
    invalid["history"] = json!([999999]);
    assert!(restored.restore_disk_state(&invalid).is_err());
    assert_eq!(restored.disk_state(), state);
    let different=br#"<Scene><trigger id="once"><then><counter name="flag" action="set" value="2" delay="3"/></then></trigger></Scene>"#;
    let mut other = ScriptRuntime::new(
        SceneScriptLayout::read(different, false).unwrap(),
        Default::default(),
    );
    assert!(other.restore_disk_state(&state).is_err());
}

#[test]
fn campaign_disk_reload_preserves_players_equipment_magicks_and_checkpoint() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        eprintln!("retail content absent; skipping");
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene_named("havindr_s3", "start").unwrap();
    for _ in 0..5 {
        s.step(1.0 / 60.0);
    }
    let defaults = s
        .avatar()
        .items
        .iter()
        .map(|(slot, item)| (*slot, item.definition.name.clone()))
        .collect::<Vec<_>>();
    s.join_player_at_start(1, "Wizard").unwrap();
    s.set_player_loadout(0, "Wizard", "staff_of_fire", "weapon_stung")
        .unwrap();
    s.player_ids[0] = Some(new_player_id().unwrap());
    s.player_ids[1] = Some(new_player_id().unwrap());
    s.unlocked_magicks = [0x107, 0x801, 0, 0].into();
    s.runtime
        .variables
        .set_counter(custom_hash("disk save flag"), 123);
    s.world.flags.tutorial_elements_disabled = 5;
    s.world.flags.achievements.insert("save-test".into());
    let equipment = |s: &PlaySession| {
        s.world
            .players
            .iter()
            .flatten()
            .filter(|p| p.playing)
            .map(|p| {
                p.items
                    .iter()
                    .map(|(slot, item)| (*slot, item.definition.name.clone()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let before = equipment(&s);
    let state = s.disk_save().unwrap();
    let bytes = serde_json::to_vec(&state).unwrap();
    let stored = serde_json::from_slice(&bytes).unwrap();
    let mut restored = PlaySession::load_disk_save(&root, &stored).unwrap();
    assert_eq!(restored.scene_name(), s.scene_name());
    assert_eq!(restored.player_ids, s.player_ids);
    assert_eq!(restored.unlocked_magicks, s.unlocked_magicks);
    assert_eq!(equipment(&restored), before);
    assert_eq!(
        restored
            .runtime
            .variables
            .counter(custom_hash("disk save flag")),
        123
    );
    assert_eq!(restored.world.flags.tutorial_elements_disabled, 5);
    assert!(restored.world.flags.achievements.contains("save-test"));
    assert_eq!(
        restored.checkpoint.as_ref().map(|c| c.scene),
        s.checkpoint.as_ref().map(|c| c.scene)
    );
    restored.change_scene_named("havindr_s4", "start").unwrap();
    assert_eq!(restored.player_ids, s.player_ids);
    assert!(restored.restart_at_checkpoint().unwrap());
    assert_eq!(
        equipment(&restored)[0],
        defaults,
        "checkpoint restores its own equipment snapshot"
    );
    assert_eq!(restored.player_ids, s.player_ids);
}

/// Exercise the public incoming-damage route rather than only comparing names.
fn assert_robe_damage(session: &mut PlaySession, necromancer: bool) {
    use magicka_sim::{
        damage::{attack, Damage},
        spells::Elements,
    };
    let avatar = session.world.players[0].as_mut().unwrap();
    avatar.character.immortal_time = 0.;
    avatar.character.combat.health.hit_points = avatar.character.definition.max_hit_points * 0.5;
    let body = &mut session.world.physics.bodies[avatar.body];
    let position = body.position;
    let initial = avatar.character.combat.health.hit_points;
    avatar.receive_damage(
        body,
        Damage {
            attack: attack::DAMAGE,
            element: Elements::ARCANE.0,
            amount: 20.,
            magnitude: 1.,
        },
        999,
        position,
        0.,
    );
    let arcane = avatar.character.combat.health.hit_points;
    assert_eq!(
        arcane > initial,
        necromancer,
        "Arcane must heal only the Necromancer control"
    );
    avatar.receive_damage(
        body,
        Damage {
            attack: attack::DAMAGE,
            element: Elements::LIFE.0,
            amount: -20.,
            magnitude: 1.,
        },
        999,
        position,
        0.,
    );
    assert_eq!(
        avatar.character.combat.health.hit_points < arcane,
        necromancer,
        "Life healing must damage the Necromancer control"
    );
}

#[test]
fn selected_profile_robe_controls_damage_and_survives_saved_checkpoint() {
    use magicka_runtime::{play_content::load_character, CharacterDefinition};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        eprintln!("retail content absent; skipping");
        return;
    }
    let mut source = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    source.change_scene_named("havindr_s3", "start").unwrap();
    for _ in 0..5 {
        source.step(1. / 60.);
    }
    source.join_player_at_start(1, "Wizard").unwrap();
    source
        .set_player_loadout(0, "Wizard", "staff_of_fire", "weapon_stung")
        .unwrap();
    source.player_ids[0] = Some(new_player_id().unwrap());
    source.player_ids[1] = Some(new_player_id().unwrap());
    source.unlocked_magicks = [0x107, 0x801, 0, 0].into();
    source
        .runtime
        .variables
        .set_counter(custom_hash("robe regression"), 123);
    let mut saved = source.disk_save().unwrap();
    // Pin a restart boundary to the actual current scene/player/script state.
    saved["checkpoint"] = json!({"scene":custom_hash(source.scene_name()),
        "matrices":saved["matrices"], "ignored":[], "scripts":saved["scripts"],
        "animations":saved["animations"], "players":saved["players"], "dialogs":saved["dialogs"]});
    let unchanged = saved.clone();
    let equipment = |s: &PlaySession| {
        s.avatar()
            .items
            .iter()
            .map(|(slot, item)| (*slot, item.definition.name.clone()))
            .collect::<Vec<_>>()
    };
    let expected_equipment = equipment(&source);
    let template = load_character(&root, "Wizard_Necromancer").unwrap();
    let expected = CharacterDefinition::from_content(&template, 1).unwrap();
    let mut restored =
        PlaySession::load_disk_save_with_robe(&root, &saved, "Wizard_Necromancer").unwrap();
    assert_eq!(
        saved, unchanged,
        "profile selection must not modify the source save"
    );
    assert_eq!(restored.player_robe(0), Some("Wizard_Necromancer"));
    assert_eq!(restored.player_robe(1), Some("Wizard"));
    assert_eq!(
        restored.avatar().character.definition.resistances,
        expected.resistances
    );
    assert_eq!(
        restored.avatar().character.definition.max_hit_points,
        expected.max_hit_points
    );
    assert_eq!(restored.player_ids, source.player_ids);
    assert_eq!(restored.unlocked_magicks, source.unlocked_magicks);
    assert_eq!(equipment(&restored), expected_equipment);
    assert_robe_damage(&mut restored, true);
    assert!(restored.restart_at_checkpoint().unwrap());
    assert_eq!(restored.player_robe(0), Some("Wizard_Necromancer"));
    assert_eq!(restored.player_robe(1), Some("Wizard"));
    assert_eq!(equipment(&restored), expected_equipment);
    assert_eq!(restored.unlocked_magicks, source.unlocked_magicks);
    assert_eq!(
        restored
            .runtime
            .variables
            .counter(custom_hash("robe regression")),
        123
    );
    assert_robe_damage(&mut restored, true);
    let resaved = restored.disk_save().unwrap();
    let mut direct = PlaySession::load_disk_save(&root, &resaved).unwrap();
    assert_eq!(direct.player_robe(0), Some("Wizard_Necromancer"));
    assert_robe_damage(&mut direct, true);
    let mut normal = PlaySession::load_disk_save_with_robe(&root, &resaved, "Wizard").unwrap();
    assert_eq!(normal.player_robe(0), Some("Wizard"));
    assert_robe_damage(&mut normal, false);
}

#[test]
fn profile_robe_load_preserves_absent_checkpoint_and_rejects_bad_template() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        eprintln!("retail content absent; skipping");
        return;
    }
    let mut source = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    source.player_ids[0] = Some(new_player_id().unwrap());
    let mut saved = source.disk_save().unwrap();
    saved["checkpoint"] = json!(null);
    let original = saved.clone();
    let loaded =
        PlaySession::load_disk_save_with_robe(&root, &saved, "Wizard_Necromancer").unwrap();
    assert!(loaded.checkpoint.is_none());
    assert_eq!(loaded.player_robe(0), Some("Wizard_Necromancer"));
    assert!(PlaySession::load_disk_save_with_robe(
        &root,
        &saved,
        "Wizard_missing_regression_template"
    )
    .is_err());
    assert_eq!(saved, original);
}
