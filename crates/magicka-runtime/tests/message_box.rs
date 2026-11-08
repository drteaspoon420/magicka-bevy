use magicka_runtime::message_box::{background_vertices, MessageBoxState};

#[test]
fn fade_runs_in_draw_and_death_is_observed_after_render() {
    let mut state = MessageBoxState::new([801., 601.]);
    assert!(state.dead());
    state.show();
    assert!(!state.dead());
    assert!(state.update([1000., 700.], false));
    assert_eq!(state.alpha, 0.);
    assert_eq!(
        state.draw(0.125, [801., 601.], false).unwrap().center,
        [400.5, 300.5]
    );
    assert_eq!(state.alpha, 0.5);
    state.kill();
    assert!(!state.dead());
    assert!(state.update([801., 601.], false));
    assert!(!state.dead()); // manager removal precedes this frame's draw
    state.draw(0.125, [801., 601.], false).unwrap();
    assert!(state.dead());
    state.show();
    assert_eq!(state.alpha, 0.);
    assert!(!state.killed);
}

#[test]
fn daisy_wheel_freezes_center_submission_and_fade() {
    let mut state = MessageBoxState::new([800., 600.]);
    state.show();
    state.draw(0.1, [800., 600.], false);
    let before = state;
    assert!(!state.update([1920., 1080.], true));
    assert_eq!(state.draw(10., [1920., 1080.], true), None);
    assert_eq!(state, before);
}

#[test]
fn dead_uses_clr_subnormal_threshold_and_nan_fade_is_retained() {
    let mut state = MessageBoxState::new([800., 600.]);
    state.alpha = f32::from_bits(1);
    assert!(!state.dead());
    state.alpha = f32::NAN;
    assert!(!state.dead());
    state.draw(0., [800., 600.], false);
    assert!(state.alpha.is_nan());
    state.show();
    state.draw(-0.5, [800., 600.], false);
    assert_eq!(state.alpha, -2.);
    state.kill();
    state.draw(-1., [800., 600.], false);
    assert_eq!(state.alpha, 2.);
}

#[test]
fn background_uses_original_page_rectangle() {
    assert_eq!(
        background_vertices([1024., 1024.]),
        [
            [-224., -280., 0., 0.453125],
            [224., -280., 0.4375, 0.453125],
            [224., 280., 0.4375, 1.],
            [-224., 280., 0., 1.],
        ]
    );
}
