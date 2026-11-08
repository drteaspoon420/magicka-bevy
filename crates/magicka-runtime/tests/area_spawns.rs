//! Presence-triggered (non-cutscene) enemy spawns in normal play against
//! user-supplied retail content (skips when absent). See
//! research/feedback-status.md ("only cutscene triggers spawn enemies").
use magicka_runtime::play_session::PlaySession;
use std::path::PathBuf;

const DT: f32 = 1.0 / 60.0;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

fn enter(s: &mut PlaySession, area: &str) -> usize {
    let before = s.world.npcs.len();
    let center = s
        .world
        .area_center(area)
        .unwrap_or_else(|| panic!("{area}"));
    let i = s.avatar().body;
    let o = s.world.physics.bodies[i].orientation;
    s.world.physics.bodies[i].move_to(center, o);
    for _ in 0..120 {
        s.step(DT);
    }
    s.world.npcs.len().saturating_sub(before)
}

#[test]
fn walking_into_encounter_areas_spawns_enemies() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let mut s = PlaySession::load(&root, "Mines.lvl", "Wizard").unwrap();
    for _ in 0..600 {
        s.step(DT);
        s.skip_cutscene();
    }
    assert!(!s.world.cutscene.active && !s.world.flags.input_limited);
    let spawned = enter(&mut s, "trigger_camp1");
    assert!(spawned >= 10, "trigger_camp1 spawned {spawned}");
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    // Havindr s3 (reached by scene change): the bridge encounter.
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene_named("havindr_s3", "start").unwrap();
    for _ in 0..600 {
        s.step(DT);
        s.skip_cutscene();
    }
    let before = s
        .world
        .log
        .iter()
        .filter(|l| l.starts_with("spawn "))
        .count();
    for area in [
        "spawn_bridge",
        "trigger_bridgeencounter",
        "trigger_encounter",
        "checkbox_bridge",
    ] {
        enter(&mut s, area);
    }
    let after = s
        .world
        .log
        .iter()
        .filter(|l| l.starts_with("spawn "))
        .count();
    assert!(after > before, "no spawns in havindr_s3 encounters");
}
