//! Highlands village defense: checkpoint props and native chieftain health bar.
use magicka_runtime::{play_session::PlaySession, scene_loading::custom_hash};
const DT: f32 = 1. / 60.;

fn battle() -> Option<PlaySession> {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Highlands.lvl", "Wizard").unwrap();
    s.player_ids[0] = Some("00000000000000000000000000000001".into());
    s.change_scene_named("hl_s5BF", "start").unwrap();
    s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
    s.step(DT);
    // Use the authored skip endpoint to isolate the battle checkpoint from
    // the preceding village walk and interactive dialogs.
    assert!(s.run_trigger("skip_intro", true).unwrap());
    for _ in 0..300 {
        s.step(DT);
        assert!(s.script_error.is_none(), "{:?}", s.script_error);
        if s.checkpoint.is_some() && s.world.boss_fight.running {
            break;
        }
    }
    assert!(s.checkpoint.is_some() && s.world.boss_fight.running);
    Some(s)
}

fn houses(s: &PlaySession) -> Vec<(String, f32)> {
    (0..5)
        .filter_map(|i| {
            let name = format!("house{i}");
            let e =
                s.world.entities.iter().rev().find(|e| {
                    e.unique_id == custom_hash(&name) && e.added && !e.removed && !e.dead
                })?;
            Some((name, s.world.physics_damage.get(&e.handle)?.health))
        })
        .collect()
}

#[test]
fn respawn_keeps_five_living_buildings_from_the_battle_checkpoint() {
    let Some(mut s) = battle() else {
        return;
    };
    let before = houses(&s);
    assert_eq!(before.len(), 5);
    assert!(before.iter().all(|(_, health)| *health > 0.));
    s.world.players[0].as_mut().unwrap().terminate();
    s.step(DT);
    assert!(
        s.avatar().dead(),
        "exercise respawn after an actual avatar death"
    );
    assert!(s.restart_at_checkpoint().unwrap());
    s.step(DT);
    let after = houses(&s);
    eprintln!("Highlands checkpoint buildings: before={before:?}, after={after:?}");
    assert_eq!(
        after.len(),
        5,
        "respawn must rebuild living checkpoint buildings"
    );
    assert!(after.iter().all(|(_, health)| *health > 0.));
    assert!(!s.avatar().dead());
    assert!(s.restart_at_checkpoint().unwrap());
    s.step(DT);
    assert_eq!(
        houses(&s).len(),
        5,
        "repeated respawn must not drop or duplicate buildings"
    );
}

#[test]
fn country_defense_bar_tracks_chieftain_health() {
    let Some(mut s) = battle() else {
        return;
    };
    let h = s
        .world
        .entities
        .iter()
        .rev()
        .find(|e| e.unique_id == custom_hash("#ent_beast_chieftain") && e.added && !e.removed)
        .unwrap()
        .handle;
    let n = s.world.npcs.get_mut(&h).unwrap();
    n.character.combat.health.hit_points = n.definition.max_hit_points * 0.5;
    s.step(DT);
    assert!(
        (s.world.boss_fight.normalized_health - 0.5).abs() < 0.001,
        "bar={} while chieftain is at half health",
        s.world.boss_fight.normalized_health
    );
}

#[test]
fn legacy_save_recovers_buildings_without_physics_snapshots() {
    let Some(s) = battle() else {
        return;
    };
    let mut saved = s.disk_save().unwrap();
    saved.as_object_mut().unwrap().remove("physics");
    saved.as_object_mut().unwrap().remove("physics_version");
    saved["checkpoint"]
        .as_object_mut()
        .unwrap()
        .remove("physics_version");
    saved["checkpoint"]
        .as_object_mut()
        .unwrap()
        .remove("physics");
    let root = magicka_content::retail_content_dir();
    let mut loaded = PlaySession::load_disk_save(&root, &saved).unwrap();
    assert_eq!(houses(&loaded).len(), 5, "recover the current scene");
    assert!(loaded.restart_at_checkpoint().unwrap());
    loaded.step(DT);
    assert_eq!(houses(&loaded).len(), 5, "recover the saved checkpoint");
    assert!(loaded.restart_at_checkpoint().unwrap());
    loaded.step(DT);
    assert_eq!(
        houses(&loaded).len(),
        5,
        "recovery must not duplicate props"
    );
}

#[test]
fn legacy_recovery_uses_each_scenes_health_and_executed_actions() {
    use magicka_runtime::scene_script::ActionDefinition;
    let Some(s) = battle() else {
        return;
    };
    let mut saved = s.disk_save().unwrap();
    saved.as_object_mut().unwrap().remove("physics");
    saved.as_object_mut().unwrap().remove("physics_version");
    saved["checkpoint"]
        .as_object_mut()
        .unwrap()
        .remove("physics_version");
    saved["checkpoint"]
        .as_object_mut()
        .unwrap()
        .remove("physics");
    let index = s
        .runtime
        .layout()
        .actions
        .iter()
        .position(
            |a| matches!(a, ActionDefinition::SpawnPhys(a) if a.unique_id == custom_hash("house0")),
        )
        .unwrap();
    // The scene lost house0 after the checkpoint, which still contains it.
    saved["scripts"]["phys"][index][1] = serde_json::json!(0.);
    // An initialized action that never executed must not create a prop.
    let history = saved["scripts"]["history"].as_array_mut().unwrap();
    let other = s
        .runtime
        .layout()
        .actions
        .iter()
        .position(
            |a| matches!(a, ActionDefinition::SpawnPhys(a) if a.unique_id == custom_hash("house1")),
        )
        .unwrap();
    history.retain(|i| i.as_u64() != Some(other as u64));
    let root = magicka_content::retail_content_dir();
    let mut loaded = PlaySession::load_disk_save(&root, &saved).unwrap();
    assert_eq!(houses(&loaded).len(), 3);
    assert!(!houses(&loaded)
        .iter()
        .any(|(name, _)| name == "house0" || name == "house1"));
    assert!(loaded.restart_at_checkpoint().unwrap());
    loaded.step(DT);
    assert_eq!(houses(&loaded).len(), 5);
}

#[test]
fn explicit_empty_physics_snapshot_does_not_use_legacy_recovery() {
    let Some(s) = battle() else {
        return;
    };
    let mut saved = s.disk_save().unwrap();
    saved["physics"] = serde_json::json!([]);
    saved["checkpoint"]["physics"] = serde_json::json!([]);
    let root = magicka_content::retail_content_dir();
    let mut loaded = PlaySession::load_disk_save(&root, &saved).unwrap();
    assert!(houses(&loaded).is_empty());
    assert!(loaded.restart_at_checkpoint().unwrap());
    assert!(houses(&loaded).is_empty());
    loaded.step(DT);
    let icons = loaded
        .world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            magicka_runtime::scene_world::boss_live::BossRef::CountryDefense(c) => Some(c.slices),
            _ => None,
        })
        .unwrap();
    assert_eq!(icons, [3; 5], "all missing houses must have burnt icons");
}

/// Old saves could persist the missing-prop bug as an explicit empty list,
/// while the checkpoint's executed SpawnPhys tags still record living houses.
#[test]
fn unversioned_empty_checkpoint_recovers_its_positive_house_records() {
    let Some(s) = battle() else { return };
    let mut saved = s.disk_save().unwrap();
    saved["checkpoint"]["physics"] = serde_json::json!([]);
    saved["checkpoint"]
        .as_object_mut()
        .unwrap()
        .remove("physics_version");
    let root = magicka_content::retail_content_dir();
    let mut loaded = PlaySession::load_disk_save(&root, &saved).unwrap();
    assert!(loaded.restart_at_checkpoint().unwrap());
    loaded.step(DT);
    assert_eq!(houses(&loaded).len(), 5);
    let upgraded = loaded.disk_save().unwrap();
    assert_eq!(upgraded["physics_version"], 1);
    assert_eq!(upgraded["checkpoint"]["physics_version"], 1);
    assert_eq!(
        upgraded["checkpoint"]["physics"].as_array().unwrap().len(),
        5
    );
    let mut reloaded = PlaySession::load_disk_save(&root, &upgraded).unwrap();
    assert!(reloaded.restart_at_checkpoint().unwrap());
    reloaded.step(DT);
    assert_eq!(houses(&reloaded).len(), 5);
}

#[test]
fn unversioned_partial_snapshot_merges_without_duplicates_or_reviving_dead_props() {
    let Some(s) = battle() else { return };
    let mut saved = s.disk_save().unwrap();
    let cp = &mut saved["checkpoint"];
    cp.as_object_mut().unwrap().remove("physics_version");
    cp["physics"]
        .as_array_mut()
        .unwrap()
        .retain(|p| p["unique_id"] == custom_hash("house2"));
    let i = s.runtime.layout().actions.iter().position(|a| matches!(a, magicka_runtime::scene_script::ActionDefinition::SpawnPhys(a) if a.unique_id == custom_hash("house0"))).unwrap();
    cp["scripts"]["phys"][i][1] = serde_json::json!(0.);
    let root = magicka_content::retail_content_dir();
    let mut loaded = PlaySession::load_disk_save(&root, &saved).unwrap();
    assert!(loaded.restart_at_checkpoint().unwrap());
    loaded.step(DT);
    assert_eq!(houses(&loaded).len(), 4);
    assert!(!houses(&loaded).iter().any(|(name, _)| name == "house0"));
    let icons = loaded
        .world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            magicka_runtime::scene_world::boss_live::BossRef::CountryDefense(c) => Some(c.slices),
            _ => None,
        })
        .unwrap();
    assert_eq!(icons, [3, 0, 0, 0, 0]);
    let h2 = loaded
        .world
        .entities
        .iter()
        .find(|e| e.unique_id == custom_hash("house2") && e.added && !e.removed)
        .unwrap()
        .handle;
    let state = loaded.world.physics_damage.get_mut(&h2).unwrap();
    state.health = state.maximum * 0.5;
    loaded.step(DT);
    let icons = loaded
        .world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            magicka_runtime::scene_world::boss_live::BossRef::CountryDefense(c) => Some(c.slices),
            _ => None,
        })
        .unwrap();
    assert_eq!(icons, [3, 0, 1, 0, 0]);
}

#[test]
fn reported_slot_restores_checkpoint_houses() {
    let Some(path) = std::env::var_os("MAGICKA_HIGHLANDS_SAVE") else {
        return;
    };
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let root = magicka_content::retail_content_dir();
    let mut loaded = PlaySession::load_disk_save(&root, &saved).unwrap();
    assert!(loaded.restart_at_checkpoint().unwrap());
    loaded.step(DT);
    eprintln!(
        "reported checkpoint houses: {:?}; boss {:?}",
        houses(&loaded),
        loaded.world.boss_fight.bosses
    );
    assert_eq!(houses(&loaded).len(), 5);
    let chief = loaded
        .world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            magicka_runtime::scene_world::boss_live::BossRef::CountryDefense(c) => c.chieftain,
            _ => None,
        })
        .unwrap();
    loaded
        .world
        .npcs
        .get_mut(&chief)
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 0.;
    loaded.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .immortal_time = f32::MAX;
    for _ in 0..600 {
        loaded.step(DT);
        if loaded
            .runtime
            .trigger(custom_hash("startend"))
            .is_some_and(|t| t.has_triggered)
        {
            break;
        }
    }
    for _ in 0..10 {
        loaded.step(DT);
    }
    assert!(
        loaded
            .runtime
            .trigger(custom_hash("startend"))
            .unwrap()
            .has_triggered
    );
    assert_eq!(houses(&loaded).len(), 5);
    assert!(
        !loaded
            .runtime
            .trigger(custom_hash("defeat condition"))
            .unwrap()
            .has_triggered
    );
}

#[test]
fn saved_checkpoint_restores_only_surviving_props_and_their_damage_callbacks() {
    use magicka_sim::{
        damage::{attack, Damage},
        spells::Elements,
    };
    let Some(mut s) = battle() else {
        return;
    };
    let handle = |s: &PlaySession, name: &str| {
        s.world
            .entities
            .iter()
            .rev()
            .find(|e| e.unique_id == custom_hash(name) && e.added && !e.removed)
            .unwrap()
            .handle
    };
    let h0 = handle(&s, "house0");
    let h1 = handle(&s, "house1");
    let fire = |amount| Damage {
        attack: attack::DAMAGE,
        element: Elements::FIRE.0,
        amount,
        magnitude: 1.,
    };
    s.world
        .damage_physics(h0, fire(s.world.physics_damage[&h0].maximum * 100.), 15)
        .unwrap();
    s.world.damage_physics(h1, fire(100.), 15).unwrap();
    assert!(s.world.physics_damage[&h0].health <= 0.);
    assert!(s.world.physics_damage[&h1].health < s.world.physics_damage[&h1].maximum);
    let matrices = s.checkpoint.as_ref().unwrap().matrices;
    // A fresh checkpoint after one destroyed and one damaged building.
    s.world.pending_checkpoint = Some((matrices, Vec::new()));
    s.step(DT);
    assert_eq!(houses(&s).len(), 4);
    // The native snapshot initializes surviving props at template health.
    assert!(s.restart_at_checkpoint().unwrap());
    s.step(DT);
    assert_eq!(houses(&s).len(), 4);
    assert!(!houses(&s).iter().any(|(name, _)| name == "house0"));
    let restored = handle(&s, "house1");
    assert_eq!(
        s.world.physics_damage[&restored].health,
        s.world.physics_damage[&restored].maximum
    );
    assert_eq!(
        s.world.physics_damage[&restored].on_death,
        custom_hash("house1 explodes")
    );
    let saved = s.disk_save().unwrap();
    let root = magicka_content::retail_content_dir();
    let mut loaded = PlaySession::load_disk_save(&root, &saved).unwrap();
    assert_eq!(houses(&loaded).len(), 4);
    assert!(loaded.restart_at_checkpoint().unwrap());
    loaded.step(DT);
    assert_eq!(houses(&loaded).len(), 4);
    assert!(!houses(&loaded).iter().any(|(name, _)| name == "house0"));
    let h1 = handle(&loaded, "house1");
    let lethal = loaded.world.physics_damage[&h1].maximum * 100.;
    loaded.world.damage_physics(h1, fire(lethal), 15).unwrap();
    assert!(
        loaded
            .world
            .pending_triggers
            .contains(&(custom_hash("house1 explodes"), None)),
        "the restored building must still dispatch its authored death trigger"
    );
}
