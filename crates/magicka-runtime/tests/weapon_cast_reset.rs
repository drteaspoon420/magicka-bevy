//! Regression: enchant, release on a swing, then cast without another swing.
use magicka_runtime::{
    avatar::{AvatarState, PlayerInput},
    play_session::PlaySession,
};
use magicka_sim::damage::CastType;
use std::path::{Path, PathBuf};
const DT: f32 = 1.0 / 60.0;

fn session() -> Option<PlaySession> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    Some(s)
}
fn frame(s: &mut PlaySession, input: PlayerInput) {
    s.step(DT);
    s.handle_input(input, 16.0 / 9.0);
}
fn fire(s: &mut PlaySession) {
    let mut input = PlayerInput::default();
    input.elements[7] = true;
    frame(s, input);
    for _ in 0..14 {
        frame(s, PlayerInput::default());
    }
}
fn charged(s: &PlaySession) -> bool {
    s.avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1
        .spells
        .charged()
}

#[test]
fn spell_casting_works_after_enchanted_swing_without_an_extra_swing() {
    for keep_shift in [true, false] {
        let Some(mut s) = session() else { return };
        fire(&mut s);
        for i in 0..90 {
            frame(
                &mut s,
                PlayerInput {
                    shift_down: true,
                    move_down: i == 0,
                    ..Default::default()
                },
            );
        }
        assert!(charged(&s));
        for i in 0..120 {
            frame(
                &mut s,
                PlayerInput {
                    shift_down: keep_shift || i == 0,
                    move_down: i == 0,
                    ..Default::default()
                },
            );
        }
        assert!(!charged(&s));
        assert!(s.avatar().current_spell.is_none());
        assert_eq!(s.avatar().motion.state, AvatarState::Idle);
        assert_eq!(s.avatar().cast.cast_type, CastType::Blast);
        fire(&mut s);
        s.handle_input(
            PlayerInput {
                cast_down: true,
                ..Default::default()
            },
            16.0 / 9.0,
        );
        assert_eq!(
            s.avatar().cast.cast_type,
            CastType::Force,
            "keep_shift={keep_shift}, pressed={:?}",
            s.avatar().cast.pressed
        );
        assert!(s.avatar().conjure.queue.is_empty());
        for _ in 0..30 {
            frame(
                &mut s,
                PlayerInput {
                    cast_down: true,
                    ..Default::default()
                },
            );
        }
        assert!(
            s.avatar().current_spell.is_some(),
            "Fire spray must actually start"
        );
    }
}
