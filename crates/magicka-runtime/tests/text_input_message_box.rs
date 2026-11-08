use magicka_content::assets::{BitmapFont, Glyph};
use magicka_runtime::{
    bitmap_font::FontMetrics,
    message_box_device::BoxDirection,
    text_input_message_box::{
        TextInputError, TextInputHost, TextInputMessageBoxState, TextInputType, DATE_TEMPLATE,
    },
};

#[derive(Default)]
struct Host {
    events: Vec<&'static str>,
    texts: Vec<Option<Vec<u16>>>,
    completed: Vec<(u64, Vec<u16>, bool)>,
    fail: Option<&'static str>,
    reopen: bool,
}
impl TextInputHost for Host {
    type Error = &'static str;
    fn register(&mut self, state: &mut TextInputMessageBoxState) -> Result<(), Self::Error> {
        self.events.push("register");
        assert!(!state.base.killed);
        assert_eq!(state.base.alpha, 0.);
        if self.fail == Some("register") {
            Err("register")
        } else {
            Ok(())
        }
    }
    fn set_text(
        &mut self,
        _: &mut TextInputMessageBoxState,
        text: Option<&[u16]>,
    ) -> Result<(), Self::Error> {
        self.events.push("text");
        self.texts.push(text.map(<[u16]>::to_vec));
        if self.fail == Some("text") {
            Err("text")
        } else {
            Ok(())
        }
    }
    fn complete(
        &mut self,
        state: &mut TextInputMessageBoxState,
        callback: u64,
        text: &[u16],
    ) -> Result<(), Self::Error> {
        self.events.push("complete");
        self.completed
            .push((callback, text.to_vec(), state.base.killed));
        if self.fail == Some("complete") {
            return Err("complete");
        }
        if self.reopen {
            state.show(Some(99), self).unwrap();
        }
        Ok(())
    }
}
fn state(capacity: i32) -> TextInputMessageBoxState {
    TextInputMessageBoxState::new([800., 600.], capacity).unwrap()
}
fn type_ascii(s: &mut TextInputMessageBoxState, text: &str, h: &mut Host) {
    for unit in text.encode_utf16() {
        s.on_text_input(unit, (48..=57).contains(&unit), false, h)
            .unwrap();
    }
}

#[test]
fn show_preserves_order_failure_prefix_and_nonreset_fields() {
    let mut s = state(3);
    let mut h = Host::default();
    type_ascii(&mut s, "old", &mut h);
    s.secure_mode = true;
    s.character_index = 7;
    s.okay_selected = true;
    s.set_text_only(true);
    h.events.clear();
    h.fail = Some("register");
    assert_eq!(
        s.show(Some(5), &mut h),
        Err(TextInputError::Host("register"))
    );
    assert_eq!(s.text(), &[111, 108, 100]);
    assert_eq!(s.completion, Some(5));
    h.fail = None;
    s.show(Some(6), &mut h).unwrap();
    assert!(s.text().is_empty());
    assert_eq!(h.texts.last(), Some(&None));
    assert!(s.secure_mode && s.okay_selected && s.text_only());
    assert_eq!(s.character_index, 7);
    s.input_type = TextInputType::DATE;
    type_ascii(&mut s, "", &mut h);
    h.events.clear();
    assert_eq!(
        s.show(Some(7), &mut h),
        Err(TextInputError::CapacityExceeded)
    );
    assert_eq!(h.events, ["register"]);
    assert!(s.text().is_empty());
    assert_eq!(s.character_index, 0);
    assert!(!s.base.killed);
    assert_eq!(
        s.show_without_callback(),
        Err(TextInputError::InvalidOperation)
    );
    assert!(matches!(
        TextInputMessageBoxState::new([0.; 2], 0),
        Err(TextInputError::InvalidCapacity)
    ));
}

#[test]
fn standard_capacity_utf16_masking_and_failed_text_mutations() {
    let mut s = state(2);
    let mut h = Host::default();
    s.show(None, &mut h).unwrap();
    s.on_text_input(0xd83d, false, false, &mut h).unwrap();
    s.on_text_input(0xde00, false, false, &mut h).unwrap();
    s.secure_mode = true;
    assert_eq!(s.display_text(), [42, 42]);
    let calls = h.texts.len();
    s.on_text_input(65, false, false, &mut h).unwrap();
    assert_eq!(h.texts.len(), calls); // full input does not refresh
    s.on_text_input(8, false, false, &mut h).unwrap();
    assert_eq!(s.text(), [0xd83d]); // no scalar/grapheme normalization
    assert_eq!(s.display_text(), [42]);
    h.fail = Some("text");
    assert_eq!(
        s.on_text_input(65, false, false, &mut h),
        Err(TextInputError::Host("text"))
    );
    assert_eq!(s.text(), [0xd83d, 65]); // append precedes renderer failure
}

#[test]
fn numeric_category_fact_and_invalid_enum_are_not_ascii_substitutes() {
    let mut s = state(3);
    let mut h = Host::default();
    s.input_type = TextInputType::NUMERIC_ONLY;
    s.on_text_input(65, false, false, &mut h).unwrap();
    assert!(s.text().is_empty());
    assert_eq!(h.texts.len(), 1); // rejection still refreshes
    s.on_text_input(0x00b2, true, false, &mut h).unwrap(); // number, not digit
    assert_eq!(s.text(), [0x00b2]);
    s.input_type = TextInputType(99);
    s.on_text_input(49, true, false, &mut h).unwrap();
    assert_eq!(s.text(), [0x00b2]);
    let calls = h.texts.len();
    s.on_text_input(8, false, true, &mut h).unwrap();
    s.on_move(BoxDirection::Left, true, &mut h).unwrap();
    assert_eq!(h.texts.len(), calls);
}

#[test]
fn date_cursor_crosses_slashes_underflows_and_reuses_template() {
    let mut s = state(10);
    let mut h = Host::default();
    s.input_type = TextInputType::DATE;
    s.show(None, &mut h).unwrap();
    assert_eq!(s.text(), DATE_TEMPLATE);
    type_ascii(&mut s, "12345678", &mut h);
    assert_eq!(s.text(), "12/34/5678".encode_utf16().collect::<Vec<_>>());
    assert_eq!(s.character_index, 10);
    let calls = h.texts.len();
    type_ascii(&mut s, "9", &mut h);
    assert_eq!(h.texts.len(), calls);
    s.on_text_input(8, false, false, &mut h).unwrap();
    assert_eq!(s.character_index, 9);
    assert_eq!(s.text()[9], 89);
    for _ in 0..9 {
        s.on_text_input(8, false, false, &mut h).unwrap();
    }
    assert_eq!(s.character_index, -1);
    assert_eq!(s.text(), DATE_TEMPLATE);
    s.on_text_input(8, false, false, &mut h).unwrap();
    assert_eq!(s.character_index, -1);
    type_ascii(&mut s, "9", &mut h);
    assert_eq!(s.character_index, 1);
    assert_eq!(s.text()[0], 57);
    let calls = h.texts.len();
    s.on_move(BoxDirection::Left, false, &mut h).unwrap();
    assert_eq!(h.texts.len(), calls + 2);
    assert_eq!(s.text()[0], 77);
    s.on_move(BoxDirection::Right, false, &mut h).unwrap();
    assert_eq!(h.texts.len(), calls + 3);
}

#[test]
fn changing_to_date_does_not_repair_buffer_and_reports_native_index_failure() {
    let mut s = state(10);
    let mut h = Host::default();
    s.show(None, &mut h).unwrap();
    s.input_type = TextInputType::DATE;
    s.character_index = -1;
    assert_eq!(
        s.on_text_input(49, true, false, &mut h),
        Err(TextInputError::CharacterIndex(0))
    );
    assert_eq!(s.character_index, 0); // Math.Max precedes failing setter
    assert!(s.text().is_empty());
}

#[test]
fn cancel_bypasses_select_gates_hover_ignores_enabled_and_flag_assignment_is_conditional() {
    let mut s = state(5);
    let mut h = Host::default();
    s.show(Some(8), &mut h).unwrap();
    s.on_select(false, &mut h).unwrap();
    assert!(h.completed.is_empty()); // alpha zero
    s.okay_enabled = false;
    s.cancel_enabled = false;
    s.on_mouse_move(true, true, false);
    assert!(s.okay_selected && !s.cancel_selected);
    s.set_text_only(false); // same value: disabled flags stay disabled
    assert!(!s.okay_enabled && !s.cancel_enabled);
    s.set_text_only(true);
    s.cancel_enabled = true;
    s.on_mouse_click(true, false, true, false, &mut h).unwrap();
    assert_eq!(h.completed, [(8, vec![], false)]);
    assert!(s.base.killed);
    s.set_text_only(false);
    s.base.killed = false;
    s.base.alpha = f32::NAN;
    s.okay_enabled = false;
    s.on_select(false, &mut h).unwrap(); // Enter does not check enabled
    assert_eq!(h.completed.len(), 2);
}

#[test]
fn completion_failure_and_reentry_precede_final_kill() {
    let mut s = state(5);
    let mut h = Host::default();
    s.show(Some(4), &mut h).unwrap();
    type_ascii(&mut s, "ab", &mut h);
    s.base.alpha = 1.;
    h.fail = Some("complete");
    assert_eq!(
        s.on_select(false, &mut h),
        Err(TextInputError::Host("complete"))
    );
    assert!(!s.base.killed);
    h.fail = None;
    h.reopen = true;
    s.on_select(false, &mut h).unwrap();
    assert_eq!(h.completed.last(), Some(&(4, vec![97, 98], false)));
    assert_eq!(s.completion, Some(99));
    assert!(s.text().is_empty());
    assert!(s.base.killed); // outer callback kills the reopened object
    assert_eq!(s.base.alpha, 0.);
}

fn font(advance: i32) -> FontMetrics {
    FontMetrics::from_content(&BitmapFont {
        line_height: 10,
        base_line: 8,
        default_character: '?',
        texture: None,
        glyphs: vec![Glyph {
            character: '?',
            advance_width: advance,
            origin: [0; 2],
            size: [1; 2],
            left_side_bearing: 0,
            force_white: false,
        }],
        kerning: vec![],
    })
    .unwrap()
}
#[test]
fn wrapping_keeps_exact_total_boundary_but_splits_per_character_at_equality() {
    let mut s = state(5);
    let mut h = Host::default();
    type_ascii(&mut s, "ab", &mut h);
    assert_eq!(s.wrapped_text(&font(179)).unwrap(), [97, 98]); // total=358 stays whole
    type_ascii(&mut s, "c", &mut h);
    assert_eq!(s.wrapped_text(&font(179)).unwrap(), [97, 10, 98, 10, 99]);
    assert_eq!(
        s.wrapped_text(&font(400)).unwrap(),
        [10, 97, 10, 98, 10, 99]
    );
    s.secure_mode = true;
    assert_eq!(s.wrapped_text(&font(179)).unwrap(), [42, 10, 42, 10, 42]);
}
