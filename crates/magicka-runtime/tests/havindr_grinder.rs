//! Town Grinder's authored guard-movement and cutscene-finish path.
use magicka_runtime::{play_session::PlaySession, scene_loading::custom_hash};
use std::path::Path;

#[test]
fn town_grinder_guards_reach_kill_box_and_release_cutscene() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        eprintln!("retail content absent; skipping");
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene_named("havindr_s4", "start").unwrap();
    s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
    for _ in 0..120 {
        s.step(1. / 60.);
    }
    for locator in &s.world.level.locators {
        if locator.name.starts_with("move_soldier") {
            eprintln!(
                "grinder locator {} transform={:?}",
                locator.name, locator.transform
            );
        }
    }
    for area in &s.world.level.trigger_areas {
        if area.name == "trigger_kill" {
            eprintln!("grinder kill area {area:?}");
        }
    }
    let mut guards = Vec::new();
    for name in ["#SolN4", "#SolN5", "#SolN6"] {
        let id = custom_hash(&name.to_ascii_lowercase());
        let e = s
            .world
            .entities
            .iter()
            .find(|e| e.unique_id == id && e.added && !e.removed)
            .unwrap_or_else(|| panic!("missing authored guard {name}"));
        guards.push((name, e.handle));
    }
    s.run_trigger("Towngrinder", true).unwrap();
    // Isolate the movement/death gate from the preceding interactive dialog.
    // The real trigger still supplies every order, delay, waypoint and animation.
    s.run_trigger("Grinder comes in", true).unwrap();
    s.step(1. / 60.);
    assert!(s.world.flags.cutscene);
    for frame in 0..3600 {
        s.step(1. / 60.);
        if frame % 60 == 0 {
            for &(name, handle) in &guards {
                if let Some(npc) = s.world.npcs.get(&handle) {
                    let body = &s.world.physics.bodies[npc.body];
                    eprintln!("grinder frame={frame} guard={name} position={:?} velocity={:?} dead={} overkilled={} ai={:?} event={} path={} clip={:?}",
                        body.position, body.velocity, npc.dead(),
                        s.world.overkilled.contains(&handle), npc.ai.state(), npc.ai.current_event,
                        npc.ai.path.len(), npc.motion.controller.clip_name());
                    if frame < 480 {
                        eprintln!(
                            "grinder guard={name} radius={} length={} waypoint={:?} path={:?}",
                            body.radius, body.length, npc.ai.waypoint, npc.ai.path
                        );
                    }
                }
            }
        }
        if !s.world.flags.cutscene {
            break;
        }
    }
    for &(name, handle) in &guards {
        let absent = s
            .world
            .entities
            .iter()
            .find(|e| e.handle == handle)
            .is_none_or(|e| e.removed || !e.added || e.dead);
        assert!(
            absent,
            "{name} still present after the authored grinder sequence"
        );
        assert!(
            s.world.overkilled.contains(&handle),
            "{name} must reach the actual overkill path"
        );
    }
    assert!(!s.world.flags.cutscene, "Town Grinder cutscene must finish");
    for _ in 0..180 {
        if !s.world.flags.input_limited {
            break;
        }
        s.step(1. / 60.);
    }
    assert!(
        !s.world.flags.input_limited,
        "authored EnableInput must restore player control"
    );
    eprintln!("grinder complete: all three guards overkilled; cutscene=false; input_limited=false");
}
