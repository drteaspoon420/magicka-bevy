use magicka_runtime::{avatar::AvatarState, pickups::EquipmentExchange, play_session::PlaySession};

fn gear(s: &PlaySession, slot: usize) -> Vec<String> {
    s.world.players[slot]
        .as_ref()
        .unwrap()
        .items
        .iter()
        .filter(|(slot, _)| matches!(slot, 0 | 1))
        .map(|(_, item)| item.definition.name.clone())
        .collect()
}

#[test]
fn each_robe_supplies_its_own_starting_equipment() {
    let content = magicka_content::retail_content_dir();
    if !content.is_dir() {
        return;
    }
    let standard = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    let mut samurai = PlaySession::load(&content, "Havindr.lvl", "Wizard_Samurai").unwrap();
    let defaults = gear(&samurai, 0);
    let other = gear(&standard, 0);
    assert_ne!(
        defaults, other,
        "the fixture must have distinct robe equipment"
    );
    samurai
        .set_player_loadout(0, "Wizard_Samurai", &other[1], &other[0])
        .unwrap();
    assert_eq!(gear(&samurai, 0), other);
    samurai.respawn_player(0, "Wizard_Samurai").unwrap();
    assert_eq!(gear(&samurai, 0), defaults);
}

#[test]
fn death_releases_equipment_once_and_another_wizard_can_pick_it_up() {
    let content = magicka_content::retail_content_dir();
    if !content.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.join_player_at_start(1, "Wizard").unwrap();
    let defaults = gear(&s, 1);
    s.set_player_loadout(
        1,
        "Wizard",
        "staff_heirloom_grandfather_clock",
        &defaults[0],
    )
    .unwrap();
    let held = gear(&s, 1);
    let before = s.world.item_pickups.len();
    s.world.players[1].as_mut().unwrap().kill();
    for _ in 0..600 {
        s.step(1.0 / 60.0);
        if s.world.players[1].as_ref().unwrap().motion.state == AvatarState::Dead {
            break;
        }
    }
    assert_eq!(s.world.item_pickups.len(), before + 2);
    assert!(s.world.players[1]
        .as_ref()
        .unwrap()
        .items
        .iter()
        .all(|(_, i)| !i.attached));
    let drops: Vec<_> = s
        .world
        .item_pickups
        .iter()
        .filter_map(|(&handle, p)| {
            (p.item.as_ref()?.definition.name == "staff_heirloom_grandfather_clock")
                .then_some(handle)
        })
        .collect();
    assert_eq!(drops.len(), 1);
    let drop = drops[0];
    assert!(!s.world.item_pickups[&drop].immovable);
    assert!(s.world.item_pickups[&drop].body_enabled);
    for _ in 0..60 {
        s.step(1.0 / 60.0);
    }
    assert_eq!(
        s.world.item_pickups.len(),
        before + 2,
        "death must not duplicate items"
    );
    let at = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == drop)
        .unwrap()
        .position;
    let body = s.avatar().body;
    s.world.physics.bodies[body].position = at;
    assert_eq!(s.pickup_item(drop).unwrap(), EquipmentExchange::Swapped);
    assert_eq!(gear(&s, 0)[1], held[1]);
    assert!(s.avatar().items.iter().all(|(_, i)| i.attached));
    assert!(!s.world.item_pickups[&drop].item.as_ref().unwrap().attached);
    s.respawn_player(1, "Wizard").unwrap();
    assert_eq!(gear(&s, 1), defaults);
    assert_eq!(
        gear(&s, 0)[1],
        held[1],
        "respawning the owner cannot reclaim another player's pickup"
    );
}

#[test]
fn scene_and_campaign_changes_carry_equipment_while_respawn_uses_robe_defaults() {
    let content = magicka_content::retail_content_dir();
    if !content.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.join_player_at_start(1, "Wizard").unwrap();
    let defaults = gear(&s, 0);
    for slot in [0, 1] {
        s.set_player_loadout(
            slot,
            "Wizard",
            "staff_heirloom_grandfather_clock",
            &defaults[0],
        )
        .unwrap();
    }
    let held = gear(&s, 0);
    s.world.players[1].as_mut().unwrap().items[1]
        .1
        .special_cooldown = 7.0;
    let scene = s.scene_name().to_string();
    s.change_scene_named(&scene, "start").unwrap();
    for slot in [0, 1] {
        assert_eq!(gear(&s, slot), held);
    }
    s.change_level("Battlefield.lvl").unwrap();
    assert_eq!(
        s.world.players[1].as_ref().unwrap().items[1]
            .1
            .special_cooldown,
        7.0,
        "level changes carry live equipment state"
    );
    for slot in [0, 1] {
        assert_eq!(gear(&s, slot), held);
    }
    s.respawn_player(1, "Wizard").unwrap();
    assert_eq!(gear(&s, 1), defaults);
    assert_eq!(gear(&s, 0), held);
}
