use magicka_runtime::{
    cutscene_text::*,
    text_box::{TextBoxError, TextBoxRenderState, TextBoxState},
};
#[test]
fn fixed_position_ignores_world_force_and_scale_but_clamps_to_blackbars() {
    let mut state = TextBoxState::default();
    state.channels[0] = TextBoxRenderState {
        size: [640, 78],
        scale: 0.5,
        world_position: [900.; 3],
        screen_position: false,
        force_on_screen: false,
    };
    assert_eq!(
        pre_render(&mut state, 0, 1., [800., 240.]).unwrap(),
        [400., 161.]
    );
    state.channels[0].scale = 0.99;
    state.channels[0].force_on_screen = true;
    assert_eq!(
        pre_render(&mut state, 0, 1., [800., 240.]).unwrap(),
        [400., 161.]
    );
    state.channels[0].size = [640, 200];
    assert_eq!(
        pre_render(&mut state, 0, 0., [320., 100.]).unwrap(),
        [-16., -26.]
    );
    assert_eq!(
        pre_render(&mut state, 3, 0., [800., 600.]),
        Err(TextBoxError::InvalidChannel(3))
    );
}
#[test]
fn typing_uses_strict_snapshot_scale_threshold() {
    let mut state = TextBoxState::default();
    state.typing.set_text(Some("AB"));
    state.typing.publish_primitives(4);
    state.channels[1] = TextBoxRenderState {
        scale: 0.99,
        size: [640, 40],
        ..Default::default()
    };
    pre_render(&mut state, 1, 0.1, [800., 600.]).unwrap();
    assert_eq!(state.typing.visible_characters, 0);
    state.channels[1].scale = 0.990001;
    pre_render(&mut state, 1, 0.01, [800., 600.]).unwrap();
    assert_eq!(state.typing.visible_characters, 1);
}
