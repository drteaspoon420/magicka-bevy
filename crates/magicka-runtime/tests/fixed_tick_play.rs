//! The simulation gives the same result on any display rate
//! (`research/fixed-timestep.md`). Drives the session the way the viewer does: frames
//! of the display's length, a sample of input per frame, ticks from the clock.
use magicka_runtime::avatar::PlayerInput;
use magicka_runtime::fixed_tick::{FixedClock, PendingSample, TICK_DT};
use magicka_runtime::play_session::PlaySession;
use std::path::Path;

fn lab() -> Option<PlaySession> {
    let content = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !content.is_dir() {
        return None;
    }
    let level = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap();
    let mut s = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
    s.disable_scripts();
    s.step(TICK_DT);
    Some(s)
}

/// Plays until `total` ticks have run. `input` gets the frame number.
fn play(
    s: &mut PlaySession,
    hz: f32,
    total: u32,
    mut input: impl FnMut(u32) -> PlayerInput,
) -> u32 {
    let mut clock = FixedClock::default();
    let mut pending = PendingSample::<PlayerInput>::default();
    let (mut frame, mut ran) = (0, 0);
    while ran < total {
        pending.sample(input(frame));
        for _ in 0..clock.advance(1.0 / hz) {
            if ran == total {
                break;
            }
            s.step(TICK_DT);
            s.handle_input(pending.take().unwrap(), 16.0 / 9.0);
            ran += 1;
        }
        frame += 1;
    }
    frame
}

#[test]
fn walking_ends_in_the_same_place_on_every_display() {
    let walk = |_| PlayerInput {
        pad_movement: Some([1.0, 0.0]),
        ..Default::default()
    };
    let Some(mut base) = lab() else { return };
    play(&mut base, 60.0, 120, walk);
    let expected = base.body().position;
    assert!(expected[0] != 0.0, "the wizard should have walked");
    for hz in [24.0, 30.0, 75.0, 144.0, 240.0] {
        let mut s = lab().unwrap();
        let frames = play(&mut s, hz, 120, walk);
        assert_eq!(s.body().position, expected, "{hz} Hz, {frames} frames");
    }
}

#[test]
fn a_key_tap_between_two_ticks_conjures_exactly_one_element() {
    let Some(mut s) = lab() else { return };
    // At 240 Hz most frames run no tick. Press on one of those, for one frame.
    let frames = play(&mut s, 240.0, 30, |frame| {
        let mut input = PlayerInput::default();
        if frame == 41 {
            input.elements[0] = true;
        }
        input
    });
    assert!(frames > 41);
    assert_eq!(s.avatar().conjure.queue.len(), 1);
}

#[test]
fn a_held_key_on_a_slow_display_conjures_as_on_a_fast_one() {
    let held = |_| PlayerInput {
        elements: {
            let mut e = [false; 8];
            e[0] = true;
            e
        },
        ..Default::default()
    };
    let Some(mut slow) = lab() else { return };
    play(&mut slow, 20.0, 60, held);
    let mut fast = lab().unwrap();
    play(&mut fast, 240.0, 60, held);
    assert_eq!(
        slow.avatar().conjure.queue.len(),
        fast.avatar().conjure.queue.len()
    );
}
