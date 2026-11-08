//! Rewrite party membership and identity survive world reconstruction.
use magicka_runtime::play_session::{NetworkMode, PlaySession};
use std::path::PathBuf;

fn session() -> PlaySession {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let level = root
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap();
    let mut s = PlaySession::load(
        &magicka_content::retail_content_dir(),
        level.to_str().unwrap(),
        "Wizard",
    )
    .unwrap();
    s.disable_scripts();
    s.step(1. / 60.);
    s
}
fn identify(s: &mut PlaySession, slot: usize) {
    s.player_ids[slot] = Some(format!("{slot:032x}"));
    s.player_profiles[slot] = Some((format!("Profile {slot}"), [slot as f32 / 8., 0.5, 0.7]));
    s.unlocked_magicks[slot] = 1 << slot;
}
fn checkpoint(s: &mut PlaySession) {
    let mut matrices = [magicka_content::skinned::IDENTITY; 4];
    for (slot, matrix) in matrices.iter_mut().enumerate() {
        let body = s
            .world
            .players
            .get(slot)
            .and_then(Option::as_ref)
            .map(|p| &s.world.physics.bodies[p.body])
            .unwrap_or_else(|| s.body());
        matrix[12..15].copy_from_slice(&body.feet());
    }
    s.world.pending_checkpoint = Some((matrices, vec![]));
    s.step(1. / 60.);
}
fn slots(s: &PlaySession) -> Vec<usize> {
    s.world
        .players
        .iter()
        .enumerate()
        .filter_map(|(slot, p)| p.as_ref().filter(|p| p.playing).map(|_| slot))
        .collect()
}
fn item(s: &PlaySession, slot: usize, item_slot: i32) -> String {
    s.world.players[slot]
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|(i, _)| *i == item_slot)
        .unwrap()
        .1
        .definition
        .name
        .clone()
}

#[test]
fn scene_level_checkpoint_and_initial_restart_preserve_six_profiles_and_parked_players() {
    let mut s = session();
    for slot in 0..7 {
        if slot != 0 {
            s.join_player_at_start(slot, "Wizard").unwrap();
        }
        identify(&mut s, slot);
    }
    s.park_player(6);
    s.depart_network_player(6).unwrap();
    let parked = s.parked_players.clone();
    let ids = s.player_ids.clone();
    let profiles = s.player_profiles.clone();
    let magicks = s.unlocked_magicks.clone();
    s.network_mode = NetworkMode::Server;
    s.camera_slots = Some([0, 1, 5].into_iter().collect());
    let check = |s: &PlaySession| {
        assert_eq!(slots(s), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(s.player_ids, ids);
        assert_eq!(s.player_profiles, profiles);
        assert_eq!(s.unlocked_magicks, magicks);
        assert_eq!(s.parked_players, parked);
        assert_eq!(s.network_mode, NetworkMode::Server);
        assert_eq!(s.camera_slots, Some([0, 1, 5].into_iter().collect()));
    };
    s.change_scene_named("corridor", "start").unwrap();
    check(&s);
    let level = s.level_file().to_owned();
    s.change_level(&level).unwrap();
    check(&s);
    checkpoint(&mut s);
    assert!(s.restart_at_checkpoint().unwrap());
    check(&s);
    s.restart_level().unwrap();
    assert!(s.checkpoint.is_none());
    check(&s);
}

#[test]
fn anonymous_checkpoint_inventory_is_not_given_to_a_named_replacement() {
    let mut s = session();
    identify(&mut s, 0);
    s.join_player_at_start(1, "Wizard").unwrap();
    s.set_player_loadout(1, "Wizard", "staff_of_fire", "weapon_stung")
        .unwrap();
    checkpoint(&mut s);
    s.depart_network_player(1).unwrap();
    s.join_player_at_start(1, "Wizard_Samurai").unwrap();
    identify(&mut s, 1);
    let expected = [item(&s, 1, 0), item(&s, 1, 1)];
    assert!(s.restart_at_checkpoint().unwrap());
    assert_eq!([item(&s, 1, 0), item(&s, 1, 1)], expected);
    assert_eq!(s.player_robe(1), Some("Wizard_Samurai"));
}

#[test]
fn checkpoint_matches_profiles_after_slot_reuse_and_does_not_spawn_departed_players() {
    let mut s = session();
    identify(&mut s, 0);
    for slot in [1, 2] {
        s.join_player_at_start(slot, "Wizard").unwrap();
        identify(&mut s, slot);
    }
    s.set_player_loadout(1, "Wizard", "staff_of_fire", "weapon_stung")
        .unwrap();
    checkpoint(&mut s);
    let returning_id = s.player_ids[1].clone().unwrap();
    let absent_id = s.player_ids[2].clone().unwrap();
    // The departing player's current progress remains safely parked, separate
    // from the earlier checkpoint inventory.
    s.set_player_loadout(
        1,
        "Wizard",
        "staff_heirloom_grandfather_clock",
        "weapon_stung",
    )
    .unwrap();
    s.unlocked_magicks[1] = 1 << 7;
    for slot in [1, 2] {
        s.park_player(slot);
        s.depart_network_player(slot).unwrap();
    }
    s.join_player_at_start(1, "Wizard_Samurai").unwrap();
    s.player_ids[1] = Some(format!("{:032x}", 3));
    s.player_profiles[1] = Some(("Replacement".into(), [0.7, 0.2, 0.1]));
    s.unlocked_magicks[1] = 1 << 4;
    let replacement_items = [item(&s, 1, 0), item(&s, 1, 1)];
    s.join_player_at_start(5, "Wizard_Necromancer").unwrap();
    assert!(s
        .claim_parked(5, &returning_id, "Wizard_Necromancer")
        .unwrap());
    s.player_profiles[5] = Some(("Returning".into(), [0.2, 0.3, 0.4]));
    assert!(s.restart_at_checkpoint().unwrap());
    assert_eq!(
        slots(&s),
        vec![0, 1, 5],
        "checkpoint must not revive departed slot 2"
    );
    assert_eq!([item(&s, 1, 0), item(&s, 1, 1)], replacement_items);
    assert_eq!(s.unlocked_magicks[1], 1 << 4);
    assert_eq!(s.player_robe(1), Some("Wizard_Samurai"));
    assert_eq!(item(&s, 5, 1), "staff_of_fire");
    assert_eq!(s.unlocked_magicks[5], 1 << 1);
    assert_eq!(s.player_robe(5), Some("Wizard_Necromancer"));
    assert_eq!(s.player_ids[5].as_deref(), Some(returning_id.as_str()));
    assert_eq!(s.player_profiles[5].as_ref().unwrap().0, "Returning");
    assert!(s.parked_players.iter().any(|p| p["id"] == absent_id));
}
