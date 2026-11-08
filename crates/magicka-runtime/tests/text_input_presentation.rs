use magicka_content::assets::{BitmapFont, Glyph};
use magicka_runtime::{
    error_message_box::ok_title_id,
    localization::LocalizationTable,
    menu_text_item::MENU_COLOR,
    retained_text::RetainedText,
    scene_loading::custom_hash,
    text_geometry::{GeometryError, GeometryFont, TextAlign, TextStyle},
    text_input_draw::{InputButton, TextInputDrawContext},
    text_input_message_box::{TextInputHost, TextInputMessageBoxState},
    text_input_presentation::{
        cancel_title_id, PresentationDrawHost, PresentationEditHost, TextInputPresentation,
        TextTarget, BUTTON_CAPACITY, CAPTION_CAPACITY, CAPTION_WRAP_WIDTH,
    },
};

fn content() -> BitmapFont {
    BitmapFont {
        line_height: 10,
        base_line: 8,
        default_character: '?',
        texture: None,
        glyphs: vec![Glyph {
            character: '?',
            advance_width: 8,
            origin: [0; 2],
            size: [3, 7],
            left_side_bearing: 0,
            force_white: false,
        }],
        kerning: vec![],
    }
}
fn font() -> GeometryFont {
    GeometryFont::with_atlas_size(&content(), [64, 64]).unwrap()
}
fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}
fn strings(ok: &str, cancel: &str, caption: &str) -> LocalizationTable {
    let mut table = LocalizationTable::default();
    table.strings.insert(ok_title_id(), ok.into());
    table.strings.insert(cancel_title_id(), cancel.into());
    table
        .strings
        .insert(custom_hash("#caption"), caption.into());
    table
}
#[derive(Default)]
struct Editing {
    texts: Vec<Option<Vec<u16>>>,
}
impl TextInputHost for Editing {
    type Error = ();
    fn register(&mut self, _: &mut TextInputMessageBoxState) -> Result<(), ()> {
        Ok(())
    }
    fn set_text(&mut self, _: &mut TextInputMessageBoxState, t: Option<&[u16]>) -> Result<(), ()> {
        self.texts.push(t.map(<[u16]>::to_vec));
        Ok(())
    }
    fn complete(&mut self, _: &mut TextInputMessageBoxState, _: u64, _: &[u16]) -> Result<(), ()> {
        Ok(())
    }
}

#[test]
fn retained_text_set_text_keeps_stale_tail_grows_exactly_and_dirties() {
    let mut text = RetainedText::new(5, true, TextStyle::default());
    text.set_text(Some(&utf16("abcd"))).unwrap();
    text.cache.dirty = false;
    text.set_text(Some(&utf16("x"))).unwrap();
    assert_eq!(text.characters, utf16("x\0cd\0"));
    assert_eq!((text.end_index, text.cache.dirty), (1, true));
    // Exactly-full text has no terminator; endIndex is the array length.
    text.set_text(Some(&utf16("12345"))).unwrap();
    assert_eq!((text.characters.len(), text.end_index), (5, 5));
    // Reinitialise allocates exactly the new length, discarding the old tail.
    text.set_text(Some(&utf16("1234567"))).unwrap();
    assert_eq!((text.characters.len(), text.end_index), (7, 7));
    text.set_text(None).unwrap();
    assert_eq!(
        (text.characters[0], text.characters[1], text.end_index),
        (0, 50, 0)
    );
    assert!(RetainedText::new(0, false, TextStyle::default())
        .set_text(None)
        .is_err());
}

#[test]
fn geometry_rebuilds_only_when_dirty_and_failure_keeps_previous_geometry() {
    let f = font();
    let mut text = RetainedText::new(8, false, TextStyle::default());
    text.set_text(Some(&utf16("??"))).unwrap();
    assert_eq!(text.geometry(&f).unwrap().primitive_count(), 4);
    assert!(!text.cache.dirty);
    // Clean geometry is reused even if the buffer is modified underneath it.
    text.characters[2] = 63;
    assert_eq!(text.geometry(&f).unwrap().primitive_count(), 4);
    // An unterminated colour tag runs off the buffer during UpdateVertices.
    text.set_text(Some(&utf16("[c=1"))).unwrap();
    text.characters.truncate(4);
    assert!(matches!(
        text.geometry(&f),
        Err(GeometryError::BufferIndex(_))
    ));
    assert!(text.cache.dirty);
    assert_eq!(text.cache.geometry.primitive_count(), 4);
}

#[test]
fn constructor_matches_native_layout_capacities_and_subscriptions() {
    let f = font();
    let table = strings("OK", "A\nB", "#unused");
    let state = TextInputMessageBoxState::new([800., 600.], 30).unwrap();
    let long = ["????"; 12].join(" ");
    let p = TextInputPresentation::new(&state, &long, &table, &f.metrics).unwrap();
    assert_eq!(CAPTION_WRAP_WIDTH, 403);
    // The retained caption holds BitmapFont.Wrap output (two lines here).
    let expected = utf16(&f.metrics.wrap(&long, 403, true).unwrap().text);
    assert_eq!(expected.iter().filter(|&&c| c == 10).count(), 1);
    assert_eq!(p.caption.characters.len(), CAPTION_CAPACITY);
    assert_eq!(&p.caption.characters[..expected.len()], &expected[..]);
    assert_eq!(p.caption.characters[expected.len()], 0);
    assert_eq!(p.message_height, 20.);
    assert_eq!(p.caption.style.default_color, MENU_COLOR);
    assert_eq!(p.caption.style.align, TextAlign::Center);
    assert!(p.caption.style.formatting && !p.caption.dynamic);
    for text in [&p.input, &p.blinker] {
        assert_eq!(text.characters.len(), 32);
        assert!(text.dynamic && !text.style.formatting);
        assert_eq!(text.style.align, TextAlign::Left);
        assert_eq!(text.style.default_color, [1.; 4]);
    }
    // Cancel is two lines high but still uses OK's height for its Y.
    assert_eq!(p.okay.item.position, [350., 418.]);
    assert_eq!(p.cancel.item.position, [450., 418.]);
    assert_eq!(p.okay.item.characters.len(), BUTTON_CAPACITY);
    assert_eq!(
        (p.okay.item.align, p.cancel.item.align),
        (TextAlign::Right, TextAlign::Left)
    );
    assert_eq!(p.okay.item.color_selected, [1.; 4]);
    assert_eq!(p.cancel.item.color, MENU_COLOR);
    assert_eq!((p.message_hash, p.language_subscriptions), (0, 2));
    let p =
        TextInputPresentation::new_localized(&state, custom_hash("#caption"), &table, &f.metrics)
            .unwrap();
    assert_eq!(
        (p.message_hash, p.language_subscriptions),
        (custom_hash("#caption"), 3)
    );
    // Missing keys are "NOT FOUND: id" strings, not nulls, so construction proceeds.
    let missing = LocalizationTable::default();
    let p = TextInputPresentation::new(&state, "x", &missing, &f.metrics).unwrap();
    let expected = utf16(&format!("NOT FOUND: {}\0", ok_title_id()));
    assert_eq!(&p.okay.item.characters[..expected.len()], &expected[..]);
}

#[test]
fn language_change_uses_raw_caption_once_per_subscription_without_relayout() {
    let f = font();
    let id = custom_hash("#caption");
    let state = TextInputMessageBoxState::new([800., 600.], 30).unwrap();
    let mut p =
        TextInputPresentation::new_localized(&state, id, &strings("OK", "No", "?"), &f.metrics)
            .unwrap();
    p.okay.geometry.dirty = false;
    p.caption.cache.dirty = false;
    let long = "?".repeat(60);
    let next = strings("OKAY\nOK", "Back", &long);
    p.language_changed(&next, &f.metrics).unwrap();
    // Raw string: no wrapping newline, no height or position recomputation.
    assert_eq!(
        &p.caption.characters[..61],
        &utf16(&format!("{long}\0"))[..]
    );
    assert_eq!(p.message_height, 10.);
    assert_eq!(p.okay.item.position, [350., 418.]);
    assert_eq!(&p.okay.item.characters[..8], &utf16("OKAY\nOK\0")[..]);
    assert!(p.okay.geometry.dirty && p.caption.cache.dirty);
    // The cached MenuTextItem Name is not refreshed by LanguageChanged.
    assert_eq!(p.okay.item.text.as_deref(), Some("OK"));

    // A string-constructed box never touches its caption.
    let mut p =
        TextInputPresentation::new(&state, "?", &strings("OK", "No", "?"), &f.metrics).unwrap();
    p.language_changed(&next, &f.metrics).unwrap();
    assert_eq!(&p.caption.characters[..2], &utf16("?\0")[..]);
}

#[test]
fn change_resolution_moves_center_and_buttons_but_keeps_editor_state() {
    let f = font();
    let table = strings("OK", "No", "?");
    let mut state = TextInputMessageBoxState::new([800., 600.], 30).unwrap();
    let mut p = TextInputPresentation::new(&state, "?", &table, &f.metrics).unwrap();
    state.blink_timer = 0.25;
    state.blink_line = true;
    let tall = strings("A\nB", "No", "?");
    p.change_resolution(&mut state, [1281, 720], &tall, &f.metrics)
        .unwrap();
    assert_eq!(state.base.center, [640.5, 360.]);
    assert_eq!(p.okay.item.position, [590.5, 468.]);
    assert_eq!(p.cancel.item.position, [690.5, 468.]);
    assert_eq!((state.blink_timer, state.blink_line), (0.25, true));
}

#[test]
fn draw_publishes_per_line_geometry_snapshots_in_native_order() {
    let f = font();
    let table = strings("OK", "No", "?");
    let mut state = TextInputMessageBoxState::new([800., 600.], 60).unwrap();
    let mut p = TextInputPresentation::new(&state, "??", &table, &f.metrics).unwrap();
    let mut editing = Editing::default();
    {
        let mut host = PresentationEditHost {
            inner: &mut editing,
            input: &mut p.input,
        };
        state.show(Some(7), &mut host).unwrap();
        for _ in 0..50 {
            state.on_text_input(63, false, false, &mut host).unwrap();
        }
    }
    assert_eq!(editing.texts.len(), 51);
    assert_eq!(p.input.end_index, 50);
    let context = TextInputDrawContext {
        screen: [800., 600.],
        daisy_wheel: false,
        base_font: &f.metrics,
        input_font: &f.metrics,
        message_height: p.message_height,
    };
    let mut host = PresentationDrawHost::new(&mut p, &table, &f);
    assert!(state.draw_input(0.1, context, &mut host).unwrap());
    let targets: Vec<_> = host.draws.iter().map(|d| d.target).collect();
    assert_eq!(
        targets,
        [
            TextTarget::Caption,
            TextTarget::Input,
            TextTarget::Input,
            TextTarget::Blinker,
            TextTarget::Button(InputButton::Okay),
            TextTarget::Button(InputButton::Cancel),
        ]
    );
    let d = &host.draws;
    // Wrapped at 44 glyphs (358px); each snapshot retains its own line.
    assert_eq!(
        (d[1].geometry.quads.len(), d[2].geometry.quads.len()),
        (44, 6)
    );
    assert!(d[0].vertex_color && !d[1].vertex_color);
    assert_eq!(d[0].geometry.quads[0].vertices[0].color, MENU_COLOR);
    assert_eq!(d[0].transform[3][..2], [400., 280.]);
    assert_eq!(d[1].transform[3][..2], [(400f32 - 176.).floor(), 290.]);
    assert_eq!(d[2].transform[3][1], 300.);
    // Blink toggled on: the cursor follows the last line.
    assert_eq!(d[3].geometry.quads.len(), 1);
    assert_eq!(d[3].transform[3][0], (400f32 - 24.).floor() + 48.);
    // Alpha after one 0.1s fade step is shared by page, glyph effect and buttons.
    let alpha = state.base.alpha;
    assert!(alpha > 0.);
    assert_eq!(d[1].color, [1., 1., 1., alpha]);
    assert_eq!(
        d[4].color,
        [
            MENU_COLOR[0],
            MENU_COLOR[1],
            MENU_COLOR[2],
            MENU_COLOR[3] * alpha
        ]
    );
    assert_eq!(host.page.unwrap().1[3], alpha);
    assert!(host.background);
    // Retained input holds only the final line plus the earlier stale tail.
    assert_eq!(
        &host.presentation.input.characters[..7],
        &utf16("??????\0")[..]
    );
    assert_eq!(host.presentation.input.characters[7], 63);
}

#[test]
fn hidden_blinker_and_empty_input_issue_no_text_draws() {
    let f = font();
    let table = strings("OK", "No", "?");
    let mut state = TextInputMessageBoxState::new([800., 600.], 8).unwrap();
    let mut p = TextInputPresentation::new(&state, "?", &table, &f.metrics).unwrap();
    let mut editing = Editing::default();
    state
        .show(
            None,
            &mut PresentationEditHost {
                inner: &mut editing,
                input: &mut p.input,
            },
        )
        .unwrap();
    // Two toggles in one frame: on then off; the blinker ends as "\0".
    let context = TextInputDrawContext {
        screen: [800., 600.],
        daisy_wheel: false,
        base_font: &f.metrics,
        input_font: &f.metrics,
        message_height: p.message_height,
    };
    let mut host = PresentationDrawHost::new(&mut p, &table, &f);
    state.draw_input(0.75, context, &mut host).unwrap();
    assert!(!state.blink_line);
    let targets: Vec<_> = host.draws.iter().map(|d| d.target).collect();
    assert_eq!(
        targets,
        [
            TextTarget::Caption,
            TextTarget::Button(InputButton::Okay),
            TextTarget::Button(InputButton::Cancel),
        ]
    );
    // Geometry was still rebuilt for the zero-primitive texts.
    assert!(!host.presentation.input.cache.dirty && !host.presentation.blinker.cache.dirty);
}
