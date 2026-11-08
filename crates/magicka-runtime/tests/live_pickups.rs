use magicka_runtime::{
    level_variables::LevelVariables, pickups::EquipmentExchange, play_session::PlaySession,
    scene_loading::custom_hash, scene_script::SceneScriptLayout, script_runtime::ScriptRuntime,
};
use std::path::PathBuf;
fn content() -> Option<PathBuf> {
    let path = magicka_content::retail_content_dir();
    path.is_dir().then_some(path)
}
#[test]
fn dungeon_scripts_spawn_real_equipment_records() {
    let Some(content) = content() else {
        return;
    };
    let mut s = PlaySession::load(&content, "Challenges/ch_dungeons_ch1.lvl", "Wizard").unwrap();
    s.change_scene_named("chs_dungeons_ch1_s5", "start")
        .unwrap();
    for _ in 0..60 {
        s.step(1.0 / 60.0);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    let items: Vec<_> = s
        .world
        .entities
        .iter()
        .filter(|e| e.added && !e.removed)
        .filter_map(|e| s.world.item_pickups.get(&e.handle))
        .collect();
    assert_eq!(items.len(), 5);
    assert!(items.iter().all(|i| i.body_enabled && i.item.is_some()));
    assert_eq!(items.iter().filter(|i| i.immovable).count(), 1);
    assert!(items
        .iter()
        .any(|i| i.item.as_ref().unwrap().definition.name == "weapon_kris"));
    for &index in &s.runtime.skipped {
        assert!(!matches!(
            s.runtime.layout().actions[index],
            magicka_runtime::scene_script::ActionDefinition::SpawnItem(_)
        ));
    }
}
#[test]
fn pickup_changes_equipment_without_resetting_player_and_drops_old_item() {
    let Some(content) = content() else {
        return;
    };
    let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 321.0;
    let old_name = s
        .avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1
        .definition
        .name
        .clone();
    let position = s.body().position;
    let handle = s.avatar().handle;
    let body = s.avatar().body;
    let mut transform = magicka_content::skinned::IDENTITY;
    transform[12..15].copy_from_slice(&position);
    let item = s.spawn_item_pickup("weapon_kris", transform).unwrap();
    assert_eq!(s.nearby_item(), Some(item));
    assert_eq!(s.pickup_item(item).unwrap(), EquipmentExchange::Swapped);
    assert_eq!(s.avatar().handle, handle);
    assert_eq!(s.avatar().body, body);
    assert_eq!(s.body().position, position);
    assert_eq!(s.avatar().character.combat.health.hit_points, 321.0);
    assert_eq!(
        s.avatar()
            .items
            .iter()
            .find(|(slot, _)| *slot == 0)
            .unwrap()
            .1
            .definition
            .name,
        "weapon_kris"
    );
    let dropped = &s.world.item_pickups[&item];
    assert_eq!(dropped.item.as_ref().unwrap().definition.name, old_name);
    assert!(!dropped.immovable);
    assert_eq!(s.avatar().motion.next_attack_animation, 147);
    assert!(s.avatar().motion.attacking);
}
#[test]
fn bound_equipment_stays_and_pickup_callback_is_still_triggered() {
    let Some(content) = content() else {
        return;
    };
    let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    s.runtime=ScriptRuntime::new(SceneScriptLayout::read(br#"<Scene><trigger id="collected" repeat="false"><if/><then><counter name="picked" action="inc" value="1"/></then></trigger></Scene>"#,false).unwrap(),LevelVariables::default());
    let mut t = magicka_content::skinned::IDENTITY;
    t[12..15].copy_from_slice(&s.body().position);
    let item = s.spawn_item_pickup("weapon_kris", t).unwrap();
    s.world.item_pickups.get_mut(&item).unwrap().on_pickup = custom_hash("collected");
    let held = &mut s.world.players[0]
        .as_mut()
        .unwrap()
        .items
        .iter_mut()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1;
    held.definition.bound = true;
    let old = held.definition.name.clone();
    assert_eq!(s.pickup_item(item).unwrap(), EquipmentExchange::Bound);
    assert_eq!(
        s.avatar()
            .items
            .iter()
            .find(|(slot, _)| *slot == 0)
            .unwrap()
            .1
            .definition
            .name,
        old
    );
    assert!(
        s.runtime
            .trigger(custom_hash("collected"))
            .unwrap()
            .has_triggered
    );
    s.step(1.0 / 60.0);
    assert_eq!(s.runtime.variables.counter(custom_hash("picked")), 1);
}

#[test]
fn removed_and_out_of_range_items_cannot_exchange_equipment() {
    let Some(content) = content() else {
        return;
    };
    let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let original = s
        .avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1
        .definition
        .name
        .clone();
    let mut t = magicka_content::skinned::IDENTITY;
    let position = s.body().position;
    t[12..15].copy_from_slice(&position);
    t[12] += 100.0;
    let distant = s.spawn_item_pickup("weapon_kris", t).unwrap();
    assert!(s.pickup_item(distant).is_err());
    t[12..15].copy_from_slice(&position);
    let removed = s.spawn_item_pickup("weapon_kris", t).unwrap();
    s.world
        .entities
        .iter_mut()
        .find(|e| e.handle == removed)
        .unwrap()
        .removed = true;
    assert_eq!(s.nearby_item(), None);
    assert!(s.pickup_item(removed).is_err());
    assert_eq!(
        s.avatar()
            .items
            .iter()
            .find(|(slot, _)| *slot == 0)
            .unwrap()
            .1
            .definition
            .name,
        original
    );
}
