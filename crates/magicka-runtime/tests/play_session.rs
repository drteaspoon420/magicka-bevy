//! Play session against user-supplied retail content (skips when absent):
//! every level's start scene spawns the Wizard on the ground, it idles in
//! place and walks under cursor input without falling through geometry.
use magicka_runtime::{
    avatar::{AvatarEvent, AvatarState, PlayerInput},
    play_session::{PlaySession, SessionEvent},
};
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

fn levels(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(root.join("Levels"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.to_ascii_lowercase().ends_with(".lvl"))
        .collect();
    out.sort();
    out
}

fn session(root: &Path, level: &str) -> PlaySession {
    let mut s =
        PlaySession::load(root, level, "Wizard").unwrap_or_else(|e| panic!("{level}: {e:?}"));
    // Physics/camera invariants are checked without scene scripts, which may
    // legitimately move the avatar (intros) or lock input.
    s.disable_scripts();
    s
}

fn ground_below(s: &PlaySession) -> Option<f32> {
    let p = s.body().position;
    s.world
        .collision
        .collision()
        .segment_intersect(
            magicka_sim::spray_pulse::Segment {
                origin: p,
                delta: [0.0, -20.0, 0.0],
            },
            false,
        )
        .map(|h| h.hit.position[1])
}

/// A level surface crossing the capsule axis between its top and 0.25 above
/// the feet means the body sank into geometry.
fn sunk_into(s: &PlaySession) -> Option<f32> {
    let b = s.body();
    let top = b.position[1] + 0.5 * b.length + b.radius;
    let stop = b.feet()[1] + 0.25;
    s.world
        .collision
        .collision()
        .segment_intersect(
            magicka_sim::spray_pulse::Segment {
                origin: [b.position[0], top, b.position[2]],
                delta: [0.0, stop - top, 0.0],
            },
            false,
        )
        .map(|h| h.hit.position[1])
}

#[test]
fn retail_start_scenes_spawn_idle_and_walk() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let all = levels(&root);
    assert!(all.len() >= 10, "{all:?}");
    for level in all {
        let mut s = session(&root, &level);
        let spawn = s.body().position;
        assert_eq!(s.avatar().motion.state, AvatarState::Idle);
        assert!(matches!(
            s.events.first(),
            Some(SessionEvent::Clip(c)) if c.clip == "idle"
        ));
        s.events.clear();
        // Idle: settle onto the floor (a spawn overlapping a wall is pushed
        // out by at most the capsule radius), then stay put.
        for _ in 0..60 {
            s.handle_input(PlayerInput::default(), 16.0 / 9.0);
            s.step(DT);
        }
        let settled = s.body().position;
        for _ in 0..120 {
            s.handle_input(PlayerInput::default(), 16.0 / 9.0);
            s.step(DT);
        }
        let b = s.body();
        assert!(!s.dead(), "{level}: died idling {:?}", s.events);
        assert!(
            b.is_touching_ground(),
            "{level}: not grounded at {:?}",
            b.position
        );
        let horizontal =
            |a: [f32; 3], c: [f32; 3]| ((a[0] - c[0]).powi(2) + (a[2] - c[2]).powi(2)).sqrt();
        let push = horizontal(settled, spawn);
        assert!(push < b.radius, "{level}: spawn depenetration {push}");
        let drift = horizontal(b.position, settled);
        assert!(drift < 0.01, "{level}: idle drift {drift}");
        let floor = ground_below(&s).unwrap_or_else(|| panic!("{level}: no floor"));
        assert!(
            (b.feet()[1] - floor).abs() < 0.05,
            "{level}: feet {:?} floor {floor}",
            b.feet()
        );
        assert!(s
            .events
            .iter()
            .all(|e| !matches!(e, SessionEvent::Avatar(AvatarEvent::FallDamage { .. }))));

        // Walk toward four screen directions from the idle position.
        let mut best = 0.0f32;
        for ndc in [[0.6, 0.0], [-0.6, 0.0], [0.0, 0.6], [0.0, -0.6]] {
            let mut w = session(&root, &level);
            for _ in 0..120 {
                w.handle_input(PlayerInput::default(), 16.0 / 9.0);
                w.step(DT);
            }
            let from = w.body().position;
            let mut moved_state = false;
            for _ in 0..120 {
                w.handle_input(
                    PlayerInput {
                        cursor_ndc: Some(ndc),
                        move_down: true,
                        ..PlayerInput::default()
                    },
                    16.0 / 9.0,
                );
                w.step(DT);
                moved_state |= w.avatar().motion.state == AvatarState::Move;
                assert!(
                    sunk_into(&w).is_none(),
                    "{level} {ndc:?}: capsule inside geometry at {:?}",
                    w.body().position
                );
            }
            let to = w.body().position;
            let d = ((to[0] - from[0]).powi(2) + (to[2] - from[2]).powi(2)).sqrt();
            // Walking off a platform edge is possible (EndofWorld); the only
            // permitted death is the scene kill plane.
            if w.dead() {
                assert!(
                    w.events
                        .contains(&SessionEvent::Avatar(AvatarEvent::KilledByKillPlane)),
                    "{level} {ndc:?}: died walking {:?}",
                    w.events
                );
                eprintln!("{level} {ndc:?}: walked off an edge into the kill plane");
            }
            if d > 1.0 {
                assert!(moved_state, "{level} {ndc:?}: moved without Move state");
            }
            best = best.max(d);
        }
        // 2 s at 5 m/s is 10 m unobstructed; require real movement somewhere.
        assert!(best > 3.0, "{level}: best walk {best}");
        eprintln!("{level}: spawn {spawn:?} best walk {best:.2} m");
    }
}

#[test]
fn camera_follows_avatar_onto_its_feet() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let mut s = session(&root, "Wizard_Castle.lvl");
    for _ in 0..600 {
        s.handle_input(PlayerInput::default(), 16.0 / 9.0);
        s.step(DT);
    }
    let g = s.camera().ground_position();
    let feet = s.body().feet();
    // Without a camera mesh the target is the grounded feet position; a mesh
    // snaps it to the nearest confinement point instead.
    let horizontal = ((g[0] - feet[0]).powi(2) + (g[2] - feet[2]).powi(2)).sqrt();
    eprintln!("camera ground {g:?} feet {feet:?} horizontal {horizontal}");
    assert!(horizontal < 15.0);
    let pick = s
        .camera()
        .mouse_pick([0.0, 0.0], 16.0 / 9.0, s.body().position[1]);
    // The screen centre ray hits the avatar plane at the camera ground point.
    assert!(
        (pick[0] - g[0]).abs() < 0.05 && (pick[2] - g[2]).abs() < 2.0,
        "{pick:?} {g:?}"
    );
}
