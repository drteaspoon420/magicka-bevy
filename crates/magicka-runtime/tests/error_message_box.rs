use magicka_content::assets::{BitmapFont, Glyph};
use magicka_runtime::{
    bitmap_font::{FontError, FontMetrics},
    dialog_input::{DialogInputKind as Input, DialogInputRoute as Route, EmptyMessageBoxStack},
    dialog_manager::DialogManager,
    error_message_box::{ErrorMessageBoxShowError, ErrorMessageBoxState},
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
            ['?', 'A', ' ']
                .into_iter()
                .map(|character| Glyph {
                    character,
                    advance_width: 8,
                    origin: [0; 2],
                    size: [1; 2],
                    left_side_bearing: 0,
                    force_white: false,
                })
                .collect()
        },
        kerning: vec![],
    })
    .unwrap()
}
#[test]
fn show_registration_precedes_wrapping_and_failures_preserve_prefix() {
    let mut b = ErrorMessageBoxState::new([800., 600.], "old".into());
    b.timer = 2.;
    let mut manager = DialogManager::new(vec![]).unwrap();
    let result = b.show("A", &font(true), |state| {
        assert!(!state.base.killed);
        assert_eq!(state.message, "old");
        manager.register_message_box(false);
        Ok::<_, ()>(())
    });
    assert_eq!(
        result,
        Err(ErrorMessageBoxShowError::Font(FontError::EmptyFont))
    );
    assert_eq!(manager.holdoff, 0.2);
    assert_eq!(b.timer, 2.);
    assert_eq!(b.message, "old");
    assert_eq!(
        b.show("A", &font(false), |_| Err(7)),
        Err(ErrorMessageBoxShowError::Registration(7))
    );
    assert!(!b.base.killed);
    assert_eq!(b.message, "old");
    b.show("A\n", &font(false), |_| Ok::<_, ()>(())).unwrap();
    assert_eq!(b.line_offset, 10.); // Split retains trailing empty line
    assert_eq!(b.timer, 5.);
}
#[test]
fn selection_and_timeout_kill_after_base_draw_then_fade_next_draw() {
    let mut b = ErrorMessageBoxState::new([800., 600.], String::new());
    b.show("A", &font(false), |_| Ok::<_, ()>(())).unwrap();
    b.draw(0.25, [800., 600.], false);
    b.on_select();
    assert!(!b.base.killed);
    let selected = b.draw(0.1, [800., 600.], false);
    assert_eq!(selected.background.unwrap().color[3], 1.);
    assert!(b.base.killed);
    assert!(!b.base.dead());
    b.draw(0.25, [800., 600.], false);
    assert!(b.base.dead());
    b.show("A", &font(false), |_| Ok::<_, ()>(())).unwrap();
    b.draw(5., [800., 600.], false);
    assert_eq!(b.timer, 0.);
    assert!(!b.base.killed);
    b.draw(0., [800., 600.], false);
    assert!(b.base.killed);
    assert_eq!(b.base.alpha, 1.);
}
#[test]
fn derived_draw_continues_with_daisy_wheel_and_button_keeps_constructor_position() {
    let mut b = ErrorMessageBoxState::new([800., 600.], String::new());
    b.show("A", &font(false), |_| Ok::<_, ()>(())).unwrap();
    b.timer = 0.;
    let draw = b.draw(0.1, [1920., 1080.], true);
    assert_eq!(draw.background, None);
    assert_eq!(draw.message_position, [400., 295.]);
    assert_eq!(draw.ok_position, [400., 364.]);
    assert!(b.base.killed);
    b.draw(0., [1920., 1080.], false);
    assert_eq!(b.base.center, [960., 540.]);
    assert_eq!(b.ok_position, [400., 364.]);
}
#[test]
fn mouse_release_inside_selects_and_hover_is_independent() {
    let mut b = ErrorMessageBoxState::new([800., 600.], String::new());
    b.timer = 5.;
    b.on_mouse_move(true);
    assert!(b.ok_selected);
    b.on_mouse_click(false, true);
    b.on_mouse_click(true, false);
    assert_eq!(b.timer, 5.);
    b.on_mouse_click(true, true);
    assert_eq!(b.timer, 0.);
    b.on_mouse_move(false);
    assert!(!b.ok_selected);
}
#[test]
fn input_routing_preserves_top_box_gates_and_paused_registration() {
    let mut m = DialogManager::new(vec![]).unwrap();
    m.holdoff = 3.;
    m.register_message_box(true);
    assert_eq!(m.holdoff, 3.);
    m.register_message_box(false);
    assert_eq!(m.holdoff, 0.2);
    for input in [Input::Select, Input::Move, Input::Type, Input::MouseRelease] {
        assert_eq!(m.input_route(input, 2), Ok(Route::Ignore));
    }
    for input in [Input::MouseClick, Input::Escape, Input::MouseMove] {
        assert_eq!(
            m.input_route(input, 2),
            Ok(Route::MessageBox {
                index: 1,
                hide_mouse: false
            })
        );
        assert_eq!(m.input_route(input, 0), Err(EmptyMessageBoxStack));
    }
    m.holdoff = 0.;
    assert_eq!(
        m.input_route(Input::Select, 0),
        Ok(Route::AdvanceController)
    );
    assert_eq!(m.input_route(Input::Move, 0), Ok(Route::Ignore));
    assert_eq!(m.input_route(Input::Type, 0), Ok(Route::Ignore));
    assert_eq!(
        m.input_route(Input::Move, 3),
        Ok(Route::MessageBox {
            index: 2,
            hide_mouse: true
        })
    );
    assert_eq!(
        m.input_route(Input::Type, 3),
        Ok(Route::MessageBox {
            index: 2,
            hide_mouse: false
        })
    );
    m.holdoff = f32::NAN;
    assert_eq!(
        m.input_route(Input::Select, 0),
        Ok(Route::AdvanceController)
    );
}
