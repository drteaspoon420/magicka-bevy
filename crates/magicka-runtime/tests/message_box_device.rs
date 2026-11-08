use magicka_runtime::{
    bitmap_font::FontMetrics,
    dialog_input::DialogInputKind,
    dialog_manager::DialogManager,
    error_message_box::ErrorMessageBoxState,
    message_box_collection::MessageBoxes,
    message_box_device::{BoxDeviceSample, BoxDeviceState, BoxDirection},
};
fn sample() -> BoxDeviceSample {
    BoxDeviceSample {
        focused: true,
        ..Default::default()
    }
}
#[test]
fn polling_priority_escape_then_left_or_wheel_then_hover_is_exclusive() {
    let mut state = BoxDeviceState::default();
    let mut current = sample();
    current.escape_down = true;
    current.left_down = true;
    current.wheel_changed = true;
    current.point = [10, 20];
    let event = state.update(current, true).unwrap();
    assert_eq!(event.kind, DialogInputKind::Escape);
    assert!(!event.show_mouse);
    current.wheel_changed = false;
    assert_eq!(state.update(current, true), None);
    current.left_down = false;
    current.point = [30, 40];
    let event = state.update(current, true).unwrap();
    assert_eq!(event.kind, DialogInputKind::MouseClick);
    assert!(event.left_released);
    assert!(!event.show_mouse);
    current.point = [31, 40];
    let event = state.update(current, true).unwrap();
    assert_eq!(event.kind, DialogInputKind::MouseMove);
    assert!(event.show_mouse);
}
#[test]
fn unfocused_and_intercepted_samples_are_consumed_before_routing() {
    let mut state = BoxDeviceState::default();
    let mut current = sample();
    current.focused = false;
    current.escape_down = true;
    current.left_down = true;
    current.point = [1, 2];
    assert_eq!(state.update(current, true), None);
    current.focused = true;
    assert_eq!(state.update(current, true), None);
    current.left_down = false;
    assert_eq!(state.update(current, false), None);
    assert_eq!(state.update(current, true), None);
}
#[test]
fn keyboard_callbacks_repeat_and_escape_space_and_other_keys_are_not_select() {
    for _ in 0..3 {
        assert_eq!(
            BoxDeviceState::key_down(13, true).unwrap().kind,
            DialogInputKind::Select
        );
    }
    for (key, direction) in [
        (38, BoxDirection::Up),
        (40, BoxDirection::Down),
        (37, BoxDirection::Left),
        (39, BoxDirection::Right),
    ] {
        let event = BoxDeviceState::key_down(key, true).unwrap();
        assert_eq!(event.kind, DialogInputKind::Move);
        assert_eq!(event.direction, Some(direction));
    }
    for key in [27, 32, 8, 65] {
        assert_eq!(BoxDeviceState::key_down(key, true), None);
    }
    assert_eq!(BoxDeviceState::key_down(13, false), None);
}
#[test]
fn wheel_over_error_ok_selects_without_left_transition_despite_holdoff() {
    use magicka_content::assets::{BitmapFont, Glyph};
    let font = FontMetrics::from_content(&BitmapFont {
        line_height: 10,
        base_line: 8,
        default_character: '?',
        texture: None,
        glyphs: vec![Glyph {
            character: '?',
            advance_width: 8,
            origin: [0; 2],
            size: [1; 2],
            left_side_bearing: 0,
            force_white: false,
        }],
        kerning: vec![],
    })
    .unwrap();
    let mut boxes = MessageBoxes::default();
    let mut manager = DialogManager::new(vec![]).unwrap();
    let handle = boxes.insert(
        ErrorMessageBoxState::new_with_ok_button([800., 600.], String::new(), "OK", &font).unwrap(),
    );
    boxes
        .show(handle, "test", &font, &mut manager, false)
        .unwrap();
    assert_eq!(manager.holdoff, 0.2);
    let mut state = BoxDeviceState::default();
    let mut current = sample();
    current.point = [400, 364];
    current.wheel_changed = true;
    let event = state.update(current, true).unwrap();
    assert_eq!(event.kind, DialogInputKind::MouseClick);
    boxes
        .dispatch_pointer(
            &mut manager,
            event.kind,
            event.left_released,
            event.point.map(|v| v as f32),
            || Ok::<_, ()>(()),
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(boxes.get(handle).unwrap().timer, 0.);
    assert!(!boxes.get(handle).unwrap().ok_selected); // No hover in same poll.
}
