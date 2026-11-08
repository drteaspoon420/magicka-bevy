//! Actual Havindr wall-entry spawns must play their authored spawn clip.
use magicka_runtime::{character_motion::CharacterState, play_session::PlaySession};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[test]
fn havindr_orcs2_play_spawn_before_returning_to_live_ai() {
    let content = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !content.is_dir() {
        eprintln!("retail content absent; skipping");
        return;
    }
    let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene_named("havindr_s6", "start").unwrap();
    s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
    let initial: BTreeSet<_> = s.world.npcs.keys().copied().collect();
    assert!(s.run_trigger("Orcs2", true).unwrap());
    let mut seen = BTreeMap::new();
    let mut finished = BTreeSet::new();
    for frame in 0..360 {
        s.step(1. / 60.);
        for (&handle, npc) in &s.world.npcs {
            if initial.contains(&handle)
                || !s.world.entities.iter().any(|e| {
                    e.handle == handle && e.template_name.as_deref() == Some("orc_regular")
                })
            {
                continue;
            }
            let body = &s.world.physics.bodies[npc.body];
            if let std::collections::btree_map::Entry::Vacant(e) = seen.entry(handle) {
                eprintln!("entry frame={frame} handle={handle} state={:?} clip={:?} spawn={} position={:?}",
                    npc.motion.state,npc.motion.controller.clip_name(),npc.motion.spawn_animation,body.position);
                assert_eq!(
                    npc.motion.state,
                    CharacterState::Resurrection,
                    "spawn animation host must enter the native state"
                );
                assert!(npc
                    .motion
                    .controller
                    .clip_name()
                    .is_some_and(|name| name.contains("spawn")));
                e.insert(0usize);
            }
            if npc.motion.state == CharacterState::Resurrection {
                assert!(!body.allow_move && !body.allow_rotate);
                assert!(
                    npc.motion
                        .controller
                        .clip_name()
                        .is_some_and(|name| name.contains("spawn")),
                    "AI must not replace the entrance clip"
                );
                *seen.get_mut(&handle).unwrap() += 1;
            } else if !finished.contains(&handle) {
                assert!(body.allow_move && body.allow_rotate);
                eprintln!(
                    "entry complete frame={frame} handle={handle} state={:?} clip={:?}",
                    npc.motion.state,
                    npc.motion.controller.clip_name()
                );
                finished.insert(handle);
            }
        }
        if seen.len() == 3 && finished.len() == 3 {
            break;
        }
    }
    assert_eq!(
        seen.len(),
        3,
        "all authored delayed wall-entry orcs must spawn"
    );
    assert!(
        seen.values().all(|frames| *frames > 1),
        "entrances must remain active across updates"
    );
    assert_eq!(
        finished.len(),
        3,
        "each entrance must complete and restore live movement"
    );
}

#[test]
fn special_idle_does_not_replace_spawn_and_quick_replay_uses_same_state() {
    use magicka_runtime::{
        scene_script::{ActionDefinition, SceneScriptLayout},
        spawn_action::{SpawnState, SpawnedHealth},
    };
    let content = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !content.is_dir() {
        return;
    }
    for quick in [false, true] {
        let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
        s.change_scene_named("havindr_s6", "start").unwrap();
        let xml = br#"<Scene><trigger id="test"><then><spawn type="orc_regular" area="spawn_jumper2" order="idle" reactto="none" spawnAnimation="spawn" idleAnimation="spec_alert0"/></then></trigger></Scene>"#;
        let layout = SceneScriptLayout::read(xml, false).unwrap();
        let ActionDefinition::Spawn(action) = &layout.actions[0] else {
            panic!("spawn fixture")
        };
        let mut state = SpawnState::default();
        action.initialize(&mut state, &mut s.world).unwrap();
        if quick {
            // QuickExecute restores recorded living spawns; it does not create
            // records for a trigger that has never executed.
            state.spawned.push(SpawnedHealth {
                handle: None,
                health: 100.,
            });
            action.quick_execute(&mut state, &mut s.world).unwrap();
        } else {
            action.execute(&mut state, &mut s.world).unwrap();
        }
        let handle = state.spawned[0].handle.unwrap();
        let npc = &s.world.npcs[&handle];
        assert_eq!(npc.motion.state, CharacterState::Resurrection);
        assert_eq!(npc.motion.controller.clip_name(), Some("spawn"));
        assert_ne!(npc.motion.special_idle, 0);
        assert_ne!(npc.motion.special_idle, npc.motion.spawn_animation);
        assert!(!s.world.physics.bodies[npc.body].allow_move);
        assert!(!s.world.physics.bodies[npc.body].allow_rotate);
    }
}
