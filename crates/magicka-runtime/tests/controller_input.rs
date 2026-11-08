//! Tests the rewrite adapter against decoded content, without controller hardware.
use magicka_runtime::{avatar::PlayerInput, play_session::PlaySession};
use std::path::PathBuf;

fn session() -> PlaySession {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let content = root.join("../../../../target/Magicka/Content");
    let level = root
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap();
    let mut session = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
    session.disable_scripts();
    session.step(1.0 / 60.0);
    session
}
#[test]
fn sticks_move_and_face_independently_and_neutral_stops() {
    let mut s = session();
    let start = s.body().position;
    s.handle_input(
        PlayerInput {
            pad_movement: Some([0.5, 0.0]),
            ..Default::default()
        },
        16.0 / 9.0,
    );
    assert_eq!(s.body().movement(), [0.5, 0.0, 0.0]);
    assert_eq!(s.body().desired_direction(), [1.0, 0.0, 0.0]);
    // Host path supplies a world aim independently from walking.
    s.handle_remote_input(
        0,
        magicka_network::control::Controls {
            pad_movement: Some([0.5, 0.0]),
            aim: Some([start[0], start[1], start[2] - 3.0]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(s.body().movement(), [0.5, 0.0, 0.0]);
    assert_eq!(s.body().desired_direction(), [0.0, 0.0, -1.0]);
    for _ in 0..20 {
        s.step(1.0 / 60.0);
        s.handle_remote_input(
            0,
            magicka_network::control::Controls {
                pad_movement: Some([0.5, 0.0]),
                aim: Some([start[0], start[1], start[2] - 3.0]),
                ..Default::default()
            },
        )
        .unwrap();
    }
    assert!(s.body().position[0] > start[0] + 0.05);
    s.handle_input(
        PlayerInput {
            pad_movement: Some([0.0; 2]),
            ..Default::default()
        },
        16.0 / 9.0,
    );
    assert_eq!(s.body().movement(), [0.0; 3]);
    s.handle_input(
        PlayerInput {
            pad_movement: Some([0.5, 0.0]),
            ..Default::default()
        },
        1.0,
    );
    // A pad can disappear while paused, when its release frame was not consumed.
    // Keyboard mode with no cursor must still stop its last walk request.
    s.handle_input(PlayerInput::default(), 1.0);
    assert_eq!(s.body().movement(), [0.0; 3]);
}
#[test]
fn pad_movement_respects_locks_cutscenes_confuse_and_hypnotize() {
    let mut s = session();
    let input = PlayerInput {
        pad_movement: Some([0.4, -0.3]),
        ..Default::default()
    };
    s.world.flags.input_limited = true;
    s.handle_input(input, 1.0);
    assert_eq!(s.body().movement(), [0.0; 3]);
    s.world.flags.input_limited = false;
    s.world.flags.cutscene = true;
    s.handle_input(input, 1.0);
    assert_eq!(s.body().movement(), [0.0; 3]);
    s.world.flags.cutscene = false;
    s.world.players[0].as_mut().unwrap().controls_inverted = true;
    s.handle_input(input, 1.0);
    assert_eq!(s.body().movement(), [-0.4, 0.0, 0.3]);
    s.world.players[0].as_mut().unwrap().character.hypnotized = Some([0.0, 0.0, 1.0]);
    s.handle_input(input, 1.0);
    assert_eq!(s.body().movement(), [0.0, 0.0, 1.0]);
    s.world.players[0].as_mut().unwrap().character.hypnotized = None;
    s.world.players[0].as_mut().unwrap().character.fear.timer = 1.0;
    // Preserve the fear controller's movement instead of overwriting it.
    s.handle_input(input, 1.0);
    assert_eq!(s.body().movement(), [0.0, 0.0, 1.0]);
}
#[test]
fn wire_rejects_nonfinite_or_out_of_range_movement() {
    for movement in [[f32::NAN, 0.0], [0.0, f32::INFINITY], [1.01, 0.0]] {
        assert!(!magicka_network::control::Controls {
            pad_movement: Some(movement),
            ..Default::default()
        }
        .valid());
    }
    assert!(magicka_network::control::Controls {
        pad_movement: Some([0.6, 0.8]),
        ..Default::default()
    }
    .valid());
}

#[test]
fn original_controller_tutorial_localization_is_kept_alongside_keyboard() {
    use magicka_runtime::{script_runtime::ScriptWorld, tutorial::TutorialAction};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let action = TutorialAction::read(
        "sethint",
        [
            ("id", "#Tu_Text_hint_room0c"),
            ("appendcontroller", "true"),
            ("animation", "castself"),
        ],
    )
    .unwrap();
    s.world.tutorial_action(&action).unwrap().unwrap();
    s.world.tutorial.update(0.1);
    let hint = s.world.tutorial.hint.as_ref().unwrap();
    assert_eq!(hint.animation, 8192);
    assert!(!hint.text.is_empty());
    assert!(!hint.controller_text.as_ref().unwrap().is_empty());
    assert_ne!(hint.controller_text.as_ref().unwrap(), &hint.text);
}
#[test]
fn pad_attack_and_block_are_independent_from_interaction_and_cast_mode() {
    let mut s = session();
    let input = PlayerInput {
        pad_movement: Some([0.; 2]),
        pad_attack: true,
        pad_block: true,
        ..Default::default()
    };
    s.handle_input(input, 16. / 9.);
    assert!(s.avatar().motion.blocking);
    assert!(s.avatar().motion.attacking);
}

/// XInputController.Update: IsBlocking follows the held Block binding each
/// frame. Hardware report 2026-11-03: releasing R1 did not stop blocking.
#[test]
fn pad_block_lasts_while_held_and_ends_on_release() {
    use magicka_runtime::avatar::AvatarState;
    let mut s = session();
    let held = PlayerInput {
        pad_movement: Some([0.; 2]),
        pad_block: true,
        ..Default::default()
    };
    // Longer than the NPC one-second Block timer.
    for _ in 0..150 {
        s.handle_input(held, 16. / 9.);
        s.step(1. / 60.);
    }
    assert!(s.avatar().motion.blocking);
    assert_eq!(s.avatar().motion.state, AvatarState::Block);
    let released = PlayerInput {
        pad_movement: Some([0.; 2]),
        ..Default::default()
    };
    for _ in 0..30 {
        s.handle_input(released, 16. / 9.);
        s.step(1. / 60.);
    }
    assert!(!s.avatar().motion.blocking);
    assert_ne!(s.avatar().motion.state, AvatarState::Block);
}

#[test]
fn pad_interaction_selects_facing_pickups_without_mouse_hover() {
    let mut s = session();
    let p = s.body().position;
    let direction = s.body().direction();
    let mut transform = magicka_content::skinned::IDENTITY;
    transform[12..15].copy_from_slice(&[
        p[0] - direction[0] * 1.5,
        p[1],
        p[2] - direction[2] * 1.5,
    ]);
    let behind = s.spawn_magick_book(3, transform).unwrap();
    transform[12..15].copy_from_slice(&[
        p[0] + direction[0] * 1.5,
        p[1],
        p[2] + direction[2] * 1.5,
    ]);
    let front = s.spawn_magick_book(5, transform).unwrap();
    assert_eq!(s.pad_pickup_target(), Some(front));
    assert_eq!(
        s.pad_cursor(),
        (magicka_runtime::play_session::Cursor::PickUp, true)
    );
    s.handle_input(
        PlayerInput {
            pad_movement: Some([0.; 2]),
            move_down: true,
            ..Default::default()
        },
        16. / 9.,
    );
    assert!(s.world.book_pickups[&front].dead);
    assert!(!s.world.book_pickups[&behind].dead);
    assert_eq!(s.body().movement(), [0.; 3]);
}

#[test]
fn confuse_negates_controller_gestures_rather_than_keyboard_bindings() {
    let mut s = session();
    s.world.players[0].as_mut().unwrap().controls_inverted = true;
    let mut elements = [false; 8];
    elements[1] = true; // Up -> Left becomes Down -> Right (Shield).
    s.handle_input(
        PlayerInput {
            pad_movement: Some([0.; 2]),
            elements,
            ..Default::default()
        },
        16. / 9.,
    );
    assert_eq!(
        s.avatar().conjure.queue[0].element,
        magicka_sim::spells::Elements::SHIELD
    );
}

#[test]
fn extra_controller_interacts_and_browses_for_its_own_slot() {
    use magicka_network::control::Controls;
    use magicka_sim::spells::MagickType;
    let mut s = session();
    s.join_player_near_party(1, "Wizard").unwrap();
    s.unlocked_magicks[1] = u64::MAX;
    let player = s.world.players[1].as_ref().unwrap();
    let body = &s.world.physics.bodies[player.body];
    let p = body.position;
    let direction = body.direction();
    let mut transform = magicka_content::skinned::IDENTITY;
    transform[12..15].copy_from_slice(&[
        p[0] + direction[0] * 1.5,
        p[1],
        p[2] + direction[2] * 1.5,
    ]);
    let book = s.spawn_magick_book(5, transform).unwrap();
    s.handle_remote_input(
        1,
        Controls {
            pad_movement: Some([0.; 2]),
            move_down: true,
            magick_next: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        s.world.book_pickups[&book].dead,
        "extra controllers must dispatch Action, not just avatar movement"
    );
    assert_ne!(s.tome[1].displayed, MagickType::None);
    assert_eq!(s.tome[0].displayed, MagickType::None);
    assert_eq!(s.body().movement(), [0.; 3]);
}

#[test]
fn extra_controller_dismisses_tutorial_only_on_a_new_press_after_holdoff() {
    use magicka_network::control::Controls;
    let content =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    let mut s = PlaySession::load(&content, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    s.headless_dialogs = Some([1280., 720.]);
    s.join_player_near_party(1, "Wizard").unwrap();
    assert!(s.run_trigger("Intro hint", false).unwrap());
    s.step(1. / 60.);
    let held = Controls {
        pad_attack: true,
        pad_movement: Some([1., 0.]),
        ..Default::default()
    };
    s.handle_remote_input(1, held).unwrap();
    assert!(!s.world.tutorial.dialog_done());
    let body = s.world.players[1].as_ref().unwrap().body;
    assert_eq!(s.world.physics.bodies[body].movement(), [0.; 3]);
    s.world.tutorial.update(1.01);
    s.handle_remote_input(1, held).unwrap();
    assert!(
        !s.world.tutorial.dialog_done(),
        "held buttons must not dismiss"
    );
    s.handle_remote_input(1, Controls::default()).unwrap();
    s.handle_remote_input(1, held).unwrap();
    assert!(s.world.tutorial.dialog_done());
    assert!(
        !s.run_trigger("intro done", false).unwrap(),
        "continuation ran once"
    );
}

#[test]
fn controller_pickups_work_beyond_four_players_and_respect_that_players_lock() {
    use magicka_network::control::{Controls, PickupOutcome, PickupRejection, PickupTarget};
    let mut s = session();
    s.join_player_near_party(5, "Wizard").unwrap();
    let player = s.world.players[5].as_ref().unwrap();
    let body = &s.world.physics.bodies[player.body];
    let p = body.position;
    let d = body.direction();
    let mut transform = magicka_content::skinned::IDENTITY;
    transform[12..15].copy_from_slice(&[p[0] + d[0] * 1.5, p[1], p[2] + d[2] * 1.5]);
    let book = s.spawn_magick_book(5, transform).unwrap();
    let generation = s.world.book_pickups[&book].generation;
    s.world.player_input_locked[5] = true;
    assert_eq!(
        s.pickup_remote(
            5,
            PickupTarget {
                handle: book,
                generation
            }
        ),
        PickupOutcome::Rejected(PickupRejection::InputLimited)
    );
    s.handle_remote_input(
        5,
        Controls {
            pad_movement: Some([0.; 2]),
            move_down: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!s.world.book_pickups[&book].dead);
    s.world.player_input_locked[5] = false;
    assert!(matches!(
        s.pickup_remote(
            5,
            PickupTarget {
                handle: book,
                generation
            }
        ),
        PickupOutcome::Book { magick: 5 }
    ));
}
