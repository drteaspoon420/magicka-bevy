//! Checkpoint rollback of inactive scenes, using the Havindr house detour.
use magicka_runtime::{play_session::PlaySession, scene_loading::custom_hash};
use std::path::PathBuf;
const DT: f32 = 1. / 60.;

fn street() -> Option<PlaySession> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.player_ids[0] = Some("00000000000000000000000000000001".into());
    s.change_scene_named("havindr_s4", "start").unwrap();
    for _ in 0..30 {
        s.step(DT);
    }
    assert_eq!(
        s.checkpoint.as_ref().unwrap().scene,
        custom_hash("havindr_s4")
    );
    Some(s)
}
fn fired(s: &PlaySession, name: &str) -> bool {
    s.runtime
        .trigger(custom_hash(&name.to_ascii_lowercase()))
        .unwrap()
        .has_triggered
}
fn travel(s: &mut PlaySession, trigger: &str, scene: &str) {
    s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
    assert!(s.run_trigger(trigger, true).unwrap());
    for _ in 0..120 {
        s.step(DT);
        if s.scene_name() == scene {
            break;
        }
    }
    assert_eq!(s.scene_name(), scene);
    for _ in 0..30 {
        s.step(DT);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
}
fn progress_house(s: &mut PlaySession) {
    assert!(s.run_trigger("Warrior", true).unwrap());
    for _ in 0..180 {
        s.step(DT);
    }
    assert!(fired(s, "Warrior") && fired(s, "DoubleDoorsSmash"));
    assert!(s.world.npcs.values().any(|n| !n.dead()));
}
fn fires(s: &PlaySession) -> usize {
    s.world
        .entities
        .iter()
        .filter(|e| {
            e.template_name.as_deref() == Some("havindr_fire") && e.added && !e.removed && !e.dead
        })
        .count()
}

#[test]
fn street_checkpoint_resets_house_progress_from_both_sides_of_the_door() {
    for (return_to_street, reload_disk) in [(false, false), (true, false), (true, true)] {
        let Some(mut s) = street() else { return };
        if reload_disk {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../target/Magicka/Content");
            s = PlaySession::load_disk_save(&root, &s.disk_save().unwrap()).unwrap();
        }
        let generation = s.checkpoint_generation;
        travel(&mut s, "Exit to house", "havindr_s5");
        let initial_parts = s.world.collision.playback_states();
        let initial_fires = fires(&s);
        assert!(initial_fires > 0);
        let initial_books = s.world.book_pickups.len();
        assert!(initial_books > 0);
        progress_house(&mut s);
        assert_ne!(s.world.collision.playback_states(), initial_parts);
        if return_to_street {
            travel(&mut s, "Exit scene", "havindr_s4");
        }
        assert_eq!(s.checkpoint_generation, generation);
        assert!(s.restart_at_checkpoint().unwrap());
        assert_eq!(s.scene_name(), "havindr_s4");
        travel(&mut s, "Exit to house", "havindr_s5");
        eprintln!(
            "after restart from street={return_to_street}: warrior={}, smash={}, NPCs={}",
            fired(&s, "Warrior"),
            fired(&s, "DoubleDoorsSmash"),
            s.world.npcs.len()
        );
        assert!(
            !fired(&s, "Warrior"),
            "post-checkpoint encounter completion survived"
        );
        assert!(!fired(&s, "DoubleDoorsSmash"));
        assert_eq!(
            s.world.collision.playback_states(),
            initial_parts,
            "broken doors survived checkpoint rollback"
        );
        assert!(s.world.npcs.is_empty());
        assert!(
            fired(&s, "StartOfLevelOnce"),
            "house startup must run again"
        );
        assert_eq!(
            fires(&s),
            initial_fires,
            "house fire props must be recreated"
        );
        assert_eq!(
            s.world.book_pickups.len(),
            initial_books,
            "house magick pickup must be recreated"
        );
    }
}

#[test]
fn later_checkpoint_preserves_preexisting_house_progress_and_is_reusable() {
    let Some(mut s) = street() else { return };
    travel(&mut s, "Exit to house", "havindr_s5");
    progress_house(&mut s);
    let parts = s.world.collision.playback_states();
    travel(&mut s, "Exit scene", "havindr_s4");
    let matrices = s.checkpoint.as_ref().unwrap().matrices;
    s.world.pending_checkpoint = Some((matrices, Vec::new()));
    s.step(DT);
    for _ in 0..2 {
        travel(&mut s, "Exit to house", "havindr_s5");
        assert!(s.run_trigger("SingleDoorSmash", true).unwrap());
        for _ in 0..180 {
            s.step(DT);
        }
        assert!(fired(&s, "SingleDoorSmash"));
        travel(&mut s, "Exit scene", "havindr_s4");
        assert!(s.restart_at_checkpoint().unwrap());
        travel(&mut s, "Exit to house", "havindr_s5");
        assert!(fired(&s, "Warrior"), "pre-checkpoint progress was lost");
        assert!(
            !fired(&s, "SingleDoorSmash"),
            "later progress survived rollback"
        );
        assert_eq!(s.world.collision.playback_states(), parts);
        travel(&mut s, "Exit scene", "havindr_s4");
    }
}
