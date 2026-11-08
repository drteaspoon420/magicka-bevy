use magicka_runtime::text_box::*;
const IDENTITY: [[f32; 4]; 4] = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];
#[test]
fn typing_strict_time_punctuation_pause_tags_newline_and_utf16() {
    let mut t = TypingState::new(64, 40.0);
    t.set_text(Some("A, B[p=.5]\n[[😀"));
    t.publish_primitives(14);
    t.update(0.0).unwrap();
    assert_eq!(t.visible_characters, 0);
    t.update(0.001).unwrap();
    assert_eq!(t.visible_characters, 1);
    t.update(0.03).unwrap();
    assert_eq!(t.visible_characters, 2);
    assert!(t.next_character > 0.24);
    t.update(0.3).unwrap();
    assert_eq!(t.visible_characters, 4);
    assert!(t.next_character > 0.46);
    t.update(0.6).unwrap();
    assert_eq!(t.visible_characters, 7);
    assert!(t.finished());
    let mut t = TypingState::new(32, 40.0);
    t.set_text(Some("[c=1,1,1]A[/c]B"));
    t.publish_primitives(4);
    t.update(0.1).unwrap();
    assert!(t.finished());
}
#[test]
fn typing_failure_keeps_cursor_and_finish_observes_old_geometry_count() {
    let mut t = TypingState::new(16, 40.0);
    t.set_text(Some("[p=-1]A"));
    t.publish_primitives(2);
    assert_eq!(t.update(0.1), Err(TextBoxError::PauseSyntax));
    assert_eq!(t.character_index, 3);
    t.set_text(Some("[broken"));
    assert_eq!(t.update(0.1), Err(TextBoxError::BufferIndex(16)));
    t.set_text(Some("ABC"));
    t.finish();
    assert!(t.finished());
    assert_eq!(t.visible_characters, 1);
    t.publish_primitives(6);
    assert!(!t.finished());
    assert_eq!(t.update(0.1), Err(TextBoxError::BufferIndex(17)));
}
#[test]
fn box_growth_reveal_ttl_and_synchronous_reinitialization_phases() {
    let mut b = TextBoxState::default();
    assert!(b.begin_update(1., 99, None).unwrap().is_none());
    b.initialize_state(Some("AB"), [80, 20], 0.1);
    b.typing.publish_primitives(4);
    let u = b.begin_update(0.25, 0, None).unwrap().unwrap();
    assert!(!u.advance);
    b.finish_update(u);
    assert_eq!(b.ttl, 0.1);
    b.pre_render(0, 0.1, [800., 600.], IDENTITY).unwrap();
    assert!(b.typing.finished());
    let u = b.begin_update(0.11, 0, None).unwrap().unwrap();
    assert!(u.advance);
    b.initialize_state(Some("C"), [40, 20], 1.0); // Native callback can replace current text synchronously.
    let render = b.finish_update(u);
    assert_eq!(render.size, [40, 20]);
    assert_eq!(render.scale, 1.);
    assert_eq!(b.ttl, 1.);
    assert_eq!(b.typing.visible_characters, 0);
    b.hide();
    b.finish_animation(false, true);
    assert!(!b.visible());
    assert!(b.begin_update(1., 0, None).unwrap().is_none());
    assert!(b.automatic_advance); // Hide/Finish(false) do not cancel TTL policy.
}
#[test]
fn threshold_channel_owner_position_and_force_clamping() {
    let mut b = TextBoxState::default();
    b.initialize_state(Some("A"), [100, 40], -1.);
    b.typing.publish_primitives(2);
    b.scale = 0.99;
    let u = b.begin_update(0., 0, None).unwrap().unwrap();
    b.finish_update(u);
    assert!(b.animating()); // Typing is still unfinished.
    b.pre_render(0, 1., [800., 600.], IDENTITY).unwrap();
    assert_eq!(b.typing.visible_characters, 0);
    b.typing.finish();
    assert!(!b.animating());
    b.owner = Some(5);
    let u = b
        .begin_update(
            0.1,
            1,
            Some(TextBoxOwner {
                position: [1., 2., 3.],
                capsule: Some((4., 0.5)),
            }),
        )
        .unwrap()
        .unwrap();
    let snapshot = b.finish_update(u);
    assert_eq!(snapshot.world_position, [1., 4.5, 3.]);
    assert_eq!(b.channels[0].world_position, [0.; 3]);
    b.owner = None;
    b.screen_position = true;
    b.world_position = [-1., -1., 0.];
    b.force_on_screen = true;
    let u = b.begin_update(0., 2, None).unwrap().unwrap();
    b.finish_update(u);
    assert_eq!(
        b.pre_render(2, 0., [800., 600.], IDENTITY).unwrap(),
        [106., 66.]
    );
    b.size = [1000, 1000];
    let u = b.begin_update(0., 2, None).unwrap().unwrap();
    b.finish_update(u);
    assert_eq!(
        b.pre_render(2, 0., [800., 600.], IDENTITY).unwrap(),
        [244., 54.]
    );
}
#[test]
fn null_empty_size_and_graphics_absence_preserve_prior_state() {
    let mut b = TextBoxState::default();
    b.initialize_state(Some("AB"), [80, 20], 0.);
    b.typing.publish_primitives(4);
    b.scale = 0.5;
    b.initialize_state(None, [1, 1], 1.);
    assert_eq!(b.size, [80, 20]);
    assert_eq!(b.scale, 0.5);
    assert_eq!(b.typing.characters[0], 65);
    b.finish_animation(true, false);
    assert!(!b.automatic_advance);
    assert!(!b.typing.finished());
    b.initialize_state(Some(""), [1, 1], -1.);
    assert_eq!(b.size, [80, 20]);
    b.hide();
    b.initialize_state(Some("A"), [10, 10], 1.);
    assert_eq!(b.scale, 0.);
}
