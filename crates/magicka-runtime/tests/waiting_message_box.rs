use magicka_content::assets::{BitmapFont, Glyph};
use magicka_runtime::{
    bitmap_font::{FontError, FontMetrics},
    dialog_input::DialogInputKind as Input,
    dialog_manager::DialogManager,
    error_message_box::ErrorMessageBoxState,
    localization::LocalizationTable,
    message_box_collection::{
        MessageBoxDispatchError, MessageBoxDrawKind, MessageBoxInput, MessageBoxes,
    },
    waiting_message_box::{WaitingMessageBoxState, WaitingShowError},
};
fn font(empty: bool) -> FontMetrics {
    FontMetrics::from_content(&BitmapFont {
        line_height: 10,
        base_line: 8,
        default_character: '?',
        texture: None,
        glyphs: if empty {
            vec![]
        } else {
            vec![Glyph {
                character: '?',
                advance_width: 8,
                origin: [0; 2],
                size: [1; 2],
                left_side_bearing: 0,
                force_white: false,
            }]
        },
        kerning: vec![],
    })
    .unwrap()
}
fn strings() -> LocalizationTable {
    let mut s = LocalizationTable::default();
    s.strings.insert(1, "AA".into());
    s.strings.insert(2, "AAAA".into());
    s
}
#[test]
fn live_show_changes_text_without_resetting_alpha_dots_or_registering() {
    let mut s = WaitingMessageBoxState::new([800., 600.]);
    let f = font(false);
    let text = strings();
    let mut registrations = 0;
    s.on_abort = Some(7);
    s.disable_back = true;
    s.show(1, &text, &f, |_| {
        registrations += 1;
        Ok::<_, ()>(())
    })
    .unwrap();
    assert_eq!(s.dot_position, [408., 300.]);
    s.draw(0.5, [800., 600.]);
    assert_eq!(s.dot_count(), 1);
    s.draw(0.1, [800., 600.]);
    let timer = s.timer;
    s.show(2, &text, &f, |_| {
        registrations += 1;
        Ok::<_, ()>(())
    })
    .unwrap();
    assert_eq!(registrations, 1);
    assert_eq!(s.base.alpha, 1.);
    assert_eq!(s.timer, timer);
    assert_eq!(s.dots(), ".");
    assert_eq!(s.dot_position, [416., 300.]);
    assert_eq!(s.on_abort, Some(7));
    assert!(s.disable_back);
    s.kill();
    assert_eq!(s.on_abort, None);
    s.show(1, &text, &f, |_| {
        registrations += 1;
        Ok::<_, ()>(())
    })
    .unwrap();
    assert_eq!(registrations, 2);
    assert_eq!(s.base.alpha, 0.);
    assert_eq!(s.dots(), ".");
}
#[test]
fn draw_dot_threshold_is_strict_and_large_deltas_do_not_catch_up() {
    let mut s = WaitingMessageBoxState::new([800., 600.]);
    s.base.show();
    s.draw(0.4, [800., 600.]);
    assert_eq!(s.dot_count(), 0);
    assert_eq!(s.timer, 1.);
    s.draw(0.0001, [800., 600.]);
    assert_eq!(s.dots(), ".");
    assert_eq!(s.timer, 0.);
    s.draw(10., [800., 600.]);
    assert_eq!(s.dots(), "..");
    assert_eq!(s.timer, 0.);
    for _ in 0..3 {
        s.draw(0.5, [800., 600.]);
    }
    assert_eq!(s.dots(), "");
    assert_eq!(s.dot_count(), 0);
}
#[test]
fn message_draw_failure_keeps_alpha_prefix_without_advancing_dots() {
    let mut state = WaitingMessageBoxState::new([800., 600.]);
    state.base.show();
    let prefix = state.begin_draw(0.5, [800., 600.]);
    assert_eq!(prefix.text_color, [1.; 4]);
    assert_eq!(state.timer, 0.);
    assert_eq!(state.dot_count(), 0);
    // The renderer fails its message command here: it does not call advance_dots.
    assert_eq!(state.base.alpha, 1.);
    state.advance_dots(0.5);
    assert_eq!(state.dots(), ".");
}
#[test]
fn draw_uses_fresh_screen_for_overlay_but_retained_text_and_dot_positions() {
    let mut s = WaitingMessageBoxState::new([800., 600.]);
    s.show(1, &strings(), &font(false), |_| Ok::<_, ()>(()))
        .unwrap();
    assert!(!s.base.update([1920., 1080.], true)); // native base Update DaisyWheel gate
    let draw = s.draw(0.25, [1920., 1080.]);
    assert_eq!(draw.screen, [1920., 1080.]);
    assert_eq!(draw.message_position, [400., 300.]);
    assert_eq!(draw.dot_position, [408., 300.]);
    assert_eq!(draw.overlay_color, [1., 1., 1., 0.5]);
    assert_eq!(draw.text_color, [1.; 4]);
}
#[test]
fn language_changes_skip_zero_id_and_keep_native_font_failure_prefix() {
    let mut s = WaitingMessageBoxState::new([800., 600.]);
    let mut text = strings();
    let f = font(false);
    s.show(1, &text, &f, |_| Ok::<_, ()>(())).unwrap();
    text.strings.insert(1, "AAAA".into());
    s.language_changed(&text, &f).unwrap();
    assert_eq!(s.dot_position, [416., 300.]);
    s.message_id = 0;
    text.strings.insert(0, "long".into());
    s.language_changed(&text, &f).unwrap();
    assert_eq!(s.message.as_deref(), Some("AAAA"));
    s.kill();
    let result = s.show(1, &text, &font(true), |_| {
        panic!("registration before measurement")
    });
    assert_eq!(
        result,
        Err(WaitingShowError::<()>::Font(FontError::EmptyFont))
    );
    assert_eq!(s.message_id, 1);
    assert_eq!(s.dot_position, [400., 300.]);
    assert!(s.base.killed);
}
#[test]
fn escape_callback_failure_and_reentry_preserve_prefix_and_clear_on_success() {
    let mut s = WaitingMessageBoxState::new([800., 600.]);
    s.base.show();
    s.on_abort = Some(7);
    s.disable_back = true;
    s.controller_escape::<()>(|_, _| panic!("disabled back callback"))
        .unwrap();
    assert!(!s.base.killed);
    s.disable_back = false;
    assert_eq!(
        s.controller_escape(|id, state| {
            assert_eq!(id, 7);
            state.on_abort = Some(8);
            Err("failure")
        }),
        Err("failure")
    );
    assert!(!s.base.killed);
    assert_eq!(s.on_abort, Some(8));
    s.controller_escape(|id, state| {
        assert_eq!(id, 8);
        state.disable_back = true;
        state.on_abort = Some(9);
        Ok::<_, ()>(())
    })
    .unwrap();
    assert!(s.base.killed);
    assert_eq!(s.on_abort, None); // DisableBack is not reread after callback
}
#[test]
fn shared_collection_orders_waiting_before_error_and_dispatches_real_abort() {
    let mut boxes = MessageBoxes::default();
    let mut manager = DialogManager::new(vec![]).unwrap();
    let e = boxes.insert(ErrorMessageBoxState::new([800., 600.], String::new()));
    let w = boxes.insert_waiting(WaitingMessageBoxState::new([800., 600.]));
    boxes
        .show(e, "AA", &font(false), &mut manager, false)
        .unwrap();
    boxes
        .show_waiting(w, 1, &strings(), &font(false), &mut manager, false)
        .unwrap();
    boxes
        .show_waiting(w, 2, &strings(), &font(false), &mut manager, false)
        .unwrap();
    assert_eq!(boxes.active(), &[e, w]);
    for i in 0..2 {
        boxes.update_index(i, 0, [800., 600.], false).unwrap();
    }
    assert_eq!(boxes.submissions(0).unwrap(), &[w, e]);
    let draws = boxes
        .draw_submitted_all(0, 0.25, [800., 600.], false)
        .unwrap();
    assert!(matches!(draws[0].1, MessageBoxDrawKind::Waiting(_)));
    assert!(matches!(draws[1].1, MessageBoxDrawKind::Error(_)));
    boxes
        .dispatch_pointer(
            &mut manager,
            Input::MouseClick,
            true,
            [0.; 2],
            || Ok::<_, ()>(()),
            |_| panic!("fallback"),
        )
        .unwrap();
    boxes.get_waiting_mut(w).unwrap().on_abort = Some(5);
    assert_eq!(
        boxes.dispatch(
            &mut manager,
            Input::Escape,
            false,
            false,
            || Ok::<_, ()>(()),
            |_| Ok(())
        ),
        Err(MessageBoxDispatchError::MissingAbortHost)
    );
    assert!(!boxes.get_waiting(w).unwrap().base.killed);
    let mut called = false;
    boxes
        .dispatch_native(
            &mut manager,
            MessageBoxInput {
                kind: Input::Escape,
                left_released: false,
                inside_ok: false,
            },
            || Ok::<_, ()>(()),
            |_| Ok(()),
            |id, state| {
                assert_eq!(id, 5);
                assert!(!state.base.killed);
                called = true;
                Ok(())
            },
        )
        .unwrap();
    assert!(called);
    assert!(boxes.get_waiting(w).unwrap().base.killed);
    assert_eq!(boxes.get_waiting(w).unwrap().on_abort, None);
}
