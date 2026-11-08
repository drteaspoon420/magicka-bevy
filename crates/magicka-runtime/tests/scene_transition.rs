//! Level.GoToScene/ChangeScene against user-supplied retail content (skips
//! when absent): Havindr's "Exit scene" trigger (present wizard in
//! `trigger_exit`) changes to havindr_s2 after the one-second fade-out, with
//! the same avatar state at the `start` spawn locators. See
//! research/scene-transition.md.
use magicka_runtime::{avatar::PlayerInput, play_session::PlaySession};
use std::path::{Path, PathBuf};

const DT: f32 = 1.0 / 60.0;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    root.is_dir().then_some(root)
}

#[test]
fn havindr_exit_trigger_fades_into_second_scene_with_same_avatar() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    assert_eq!(s.scene_name(), "havindr_s1");
    for _ in 0..30 {
        s.step(DT);
    }
    let exit = s.world.area_center("trigger_exit").expect("trigger_exit");
    // Mark the avatar damaged so state carry-over is observable.
    let hp = {
        let p = s.world.players[0].as_mut().unwrap();
        p.character.combat.health.hit_points -= 123.0;
        p.character.combat.health.hit_points
    };
    let index = s.avatar().body;
    let orientation = s.world.physics.bodies[index].orientation;
    s.world.physics.bodies[index].move_to(exit, orientation);

    let mut switched_at = None;
    let mut max_fade: f32 = 0.0;
    let mut cleared_at = None;
    for i in 0..1200 {
        s.step(DT);
        s.handle_input(PlayerInput::default(), 16.0 / 9.0);
        max_fade = max_fade.max(s.transition_fade());
        if switched_at.is_none() && s.scene_name() != "havindr_s1" {
            switched_at = Some(i);
            assert!(
                s.avatar().ai.events().is_none(),
                "old scene AvatarMove survived"
            );
            assert!(s.avatar().ai.path.is_empty(), "old scene path survived");
            // Fade is fully black at the switch and then fades back in.
            assert!(s.transition_fade() > 0.95, "{}", s.transition_fade());
        }
        if switched_at.is_some() && cleared_at.is_none() && s.transition_fade() == 0.0 {
            cleared_at = Some(i);
        }
    }
    let at = switched_at.expect("scene never changed");
    assert_eq!(s.scene_name(), "havindr_s2");
    // transitionTime="1": the switch follows ~60 frames of fade-out.
    assert!((55..=70).contains(&at), "switched at frame {at}");
    assert!(max_fade > 0.95);
    // havindr_s2 startup: <fade fadein="true" time="0" override="true"/>
    // (ClearTransition, so no automatic fade-in); its intro fades in with
    // <fade fadein="false" time="0.1" delay="13.5"/>.
    let cleared = cleared_at.expect("s2 never faded in") - at;
    assert!(
        (800..=830).contains(&cleared),
        "faded in {cleared} frames after switch"
    );
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert!(
        s.runtime
            .trigger(magicka_runtime::scene_loading::custom_hash("end cutscene"))
            .unwrap()
            .has_triggered,
        "destination get-up animation must still finish its script"
    );
    let p = s.world.players[0].as_ref().unwrap();
    assert!(p.character.combat.health.hit_points <= hp);
    assert!(p.character.combat.health.hit_points > 0.0);
    // Placed at the s2 `start` locators (ChangeScene lowercases the name).
    let model = magicka_runtime::play_content::load_level_scene(
        &root,
        "Havindr.lvl",
        Some(magicka_runtime::scene_loading::custom_hash("havindr_s2")),
    )
    .unwrap()
    .model;
    let start0 = model
        .locators
        .iter()
        .find(|l| l.name.eq_ignore_ascii_case("start0"))
        .expect("start0 locator");
    let body = s.body().position;
    let d = ((body[0] - start0.transform[12]).powi(2) + (body[2] - start0.transform[14]).powi(2))
        .sqrt();
    assert!(d < 6.0, "avatar {body:?} far from start0 ({d} m)");
}

#[test]
fn instant_scene_change_keeps_level_counters() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let counters_before = format!("{:?}", s.runtime.variables);
    let spawn = magicka_runtime::scene_flow::SpawnPoint {
        scene: magicka_runtime::scene_loading::custom_hash("havindr_s3"),
        locations: std::array::from_fn(|i| {
            magicka_runtime::scene_loading::custom_hash(&format!("start{i}"))
        }),
        spawn_players: true,
    };
    s.change_scene(spawn).unwrap();
    assert_eq!(s.scene_name(), "havindr_s3");
    assert_eq!(format!("{:?}", s.runtime.variables), counters_before);
    assert_eq!(s.script_error.as_deref(), Some("scripts disabled"));
    for _ in 0..60 {
        s.step(DT);
    }
    assert!(s.body().position.iter().all(|v| v.is_finite()));
}

#[test]
fn scene_placement_resets_script_events_and_paths_for_each_carried_player() {
    use magicka_runtime::{
        ai_event::{AiEvent, MoveEvent},
        character_motion::CharacterState,
    };
    use magicka_sim::navmesh::{movement, PathNode};
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.join_player(1, "Wizard", "start").unwrap();
    let mut health = Vec::new();
    for p in s.world.players.iter_mut().flatten() {
        let b = &mut s.world.physics.bodies[p.body];
        let waypoint = [b.position[0] + 10., b.position[1], b.position[2] + 2.];
        p.set_events(
            b,
            Some(vec![AiEvent::Move(MoveEvent {
                waypoint,
                ..Default::default()
            })]),
        );
        // Cached route from the departing scene, distinct from event storage.
        p.ai.path.push(PathNode {
            position: waypoint,
            properties: movement::DEFAULT,
        });
        p.character.combat.health.hit_points -= 123.;
        health.push(p.character.combat.health.hit_points);
        assert!(p.ai.controls_avatar());
    }
    s.change_scene_named("havindr_s2", "start").unwrap();
    for (p, hp) in s.world.players.iter().flatten().zip(health) {
        assert!(p.ai.events().is_none());
        assert!(p.ai.path.is_empty());
        assert_eq!(p.ai.current_event, 0);
        assert_eq!(p.ai.event_delay, 0.);
        assert!(!p.ai.controls_avatar());
        assert_eq!(p.motion.state, CharacterState::Idle);
        assert_eq!(p.character.combat.health.hit_points, hp);
    }
}

#[test]
fn scene_event_reset_respects_spawn_players_and_dead_guards() {
    use magicka_runtime::{
        ai_event::{AiEvent, MoveEvent},
        scene_flow::SpawnPoint,
        scene_loading::custom_hash,
    };
    use magicka_sim::navmesh::{movement, PathNode};
    let Some(root) = content() else { return };
    for (spawn_players, dead) in [(false, false), (true, true)] {
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.join_player(1, "Wizard", "start").unwrap();
        for p in s.world.players.iter_mut().flatten() {
            let body = &mut s.world.physics.bodies[p.body];
            let waypoint = [body.position[0] + 10., body.position[1], body.position[2]];
            p.set_events(
                body,
                Some(vec![AiEvent::Move(MoveEvent {
                    waypoint,
                    ..Default::default()
                })]),
            );
            p.ai.path.push(PathNode {
                position: waypoint,
                properties: movement::DEFAULT,
            });
            p.character.combat.dead = dead;
        }
        s.change_scene(SpawnPoint {
            scene: custom_hash("havindr_s2"),
            locations: std::array::from_fn(|i| custom_hash(&format!("start{i}"))),
            spawn_players,
        })
        .unwrap();
        for p in s.world.players.iter().flatten() {
            assert_eq!(p.dead(), dead);
            assert_eq!(p.ai.events().unwrap().len(), 1);
            assert_eq!(p.ai.path.len(), 1);
        }
    }
}

/// Developer-tool helpers behind the viewer's debug menu.
#[test]
fn debug_helpers_list_scenes_triggers_and_switch() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    assert_eq!(s.level_file(), "Havindr.lvl");
    let scenes = s.level_scene_names();
    assert_eq!(scenes.len(), 7, "{scenes:?}");
    assert_eq!(scenes[0], "havindr_s1");
    let triggers = s.trigger_names();
    assert!(triggers.iter().any(|t| t == "skip_intro"), "{triggers:?}");
    assert!(triggers.iter().any(|t| t == "Exit scene"));
    // Forcing the exit trigger requests the s2 scene change.
    assert_eq!(s.run_trigger("Exit scene", true), Ok(true));
    // Its actions run on the next updates (delay queues), then the fade.
    for _ in 0..90 {
        s.step(DT);
    }
    assert_eq!(s.scene_name(), "havindr_s2");
    s.change_scene_named("havindr_s4", "start").unwrap();
    assert_eq!(s.scene_name(), "havindr_s4");
    assert!(s.trigger_names() != triggers);
    assert!(!s.world.area_names().is_empty());
}

/// Robe templates keep the player a "wizard" for trigger areas: a
/// Wizard_Tentacle avatar (Template.ID "wizard_tentacle") still fires
/// Havindr's `present type="wizard"` exit trigger. Before the fix, campaign
/// scripts silently stopped advancing for any non-default robe.
#[test]
fn robe_avatar_counts_as_wizard_for_trigger_areas() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard_Tentacle").unwrap();
    for _ in 0..30 {
        s.step(DT);
    }
    let exit = s.world.area_center("trigger_exit").expect("trigger_exit");
    let index = s.avatar().body;
    let orientation = s.world.physics.bodies[index].orientation;
    s.world.physics.bodies[index].move_to(exit, orientation);
    for _ in 0..240 {
        s.step(DT);
        s.handle_input(PlayerInput::default(), 16.0 / 9.0);
    }
    assert_eq!(s.scene_name(), "havindr_s2");
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
}

/// Player-only checks stay player-only: an NPC spawned from the Wizard
/// template standing in `trigger_exit` does not fire the exit trigger
/// (rewrite design decision, `scene_world::npc_trigger_type`).
#[test]
fn wizard_template_npc_does_not_fire_player_presence_trigger() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    for _ in 0..30 {
        s.step(DT);
    }
    let handle = s.spawn_npc("Wizard", "idle", 3.0).unwrap();
    for _ in 0..5 {
        s.step(DT);
    }
    let exit = s.world.area_center("trigger_exit").expect("trigger_exit");
    let body = s.world.npcs[&handle].body;
    let orientation = s.world.physics.bodies[body].orientation;
    for _ in 0..240 {
        if s.scene_name() != "havindr_s1" {
            break;
        }
        s.world.physics.bodies[body].move_to(exit, orientation);
        s.step(DT);
        s.handle_input(PlayerInput::default(), 16.0 / 9.0);
    }
    assert_eq!(
        s.scene_name(),
        "havindr_s1",
        "NPC wizard fired the exit trigger"
    );
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
}
