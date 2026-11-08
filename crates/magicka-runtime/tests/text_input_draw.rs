use magicka_content::assets::{BitmapFont, Glyph};
use magicka_runtime::{
    bitmap_font::{FontError, FontMetrics},
    text_geometry::{GeometryFont, TextGeometry, TextStyle},
    text_input_draw::{
        InputButton, InputDrawTransform, TextInputDrawCommand as C, TextInputDrawContext,
        TextInputDrawError, TextInputDrawHost,
    },
    text_input_message_box::{TextInputHost, TextInputMessageBoxState, TextInputType},
};

fn content(advance: i32) -> BitmapFont {
    BitmapFont {
        line_height: 10,
        base_line: 8,
        default_character: '?',
        texture: None,
        glyphs: vec![Glyph {
            character: '?',
            advance_width: advance,
            origin: [0; 2],
            size: [3, 7],
            left_side_bearing: 0,
            force_white: false,
        }],
        kerning: vec![],
    }
}
fn font() -> FontMetrics {
    FontMetrics::from_content(&content(8)).unwrap()
}
fn context(f: &FontMetrics) -> TextInputDrawContext<'_> {
    TextInputDrawContext {
        screen: [800., 600.],
        daisy_wheel: false,
        base_font: f,
        input_font: f,
        message_height: 20.,
    }
}
struct Host {
    calls: Vec<C>,
    chars: Vec<u16>,
    blinker: Vec<u16>,
    fail: Option<&'static str>,
    title_calls: usize,
    geometries: Vec<(InputDrawTransform, TextGeometry)>,
    geometry_font: GeometryFont,
}
impl Default for Host {
    fn default() -> Self {
        Self {
            calls: vec![],
            chars: vec![0; 34],
            blinker: vec![0; 34],
            fail: None,
            title_calls: 0,
            geometries: vec![],
            geometry_font: GeometryFont::with_atlas_size(&content(8), [64, 64]).unwrap(),
        }
    }
}
fn set_buffer(buffer: &mut Vec<u16>, text: &[u16]) {
    if buffer.len() < text.len() {
        buffer.resize(text.len(), 0);
    }
    buffer[..text.len()].copy_from_slice(text);
    if text.len() < buffer.len() {
        buffer[text.len()] = 0;
    }
}
impl TextInputDrawHost for Host {
    type Error = &'static str;
    fn command(&mut self, command: C) -> Result<(), Self::Error> {
        let fail = match &command {
            C::ScreenSize(_) => "screen",
            C::Background => "page",
            C::SetBlinker(_) => "blinker",
            C::SetInput(_) => "set-input",
            C::Input(_) => "input",
            _ => "other",
        };
        self.calls.push(command.clone());
        if self.fail == Some(fail) {
            return Err(fail);
        }
        match command {
            C::SetInput(text) => set_buffer(&mut self.chars, &text),
            C::SetBlinker(text) => set_buffer(&mut self.blinker, &text),
            C::Input(matrix) => {
                let geometry = TextGeometry::build(
                    &self.geometry_font,
                    &self.chars,
                    TextStyle {
                        formatting: false,
                        ..TextStyle::default()
                    },
                )
                .unwrap();
                self.geometries.push((matrix, geometry));
            }
            _ => {}
        }
        Ok(())
    }
    fn input_characters(&self) -> &[u16] {
        &self.chars
    }
    fn okay_title(&mut self) -> Result<Vec<u16>, Self::Error> {
        self.title_calls += 1;
        if self.fail == Some("lookup") {
            Err("lookup")
        } else {
            Ok(vec![79, 75])
        }
    }
}
struct Editing;
impl TextInputHost for Editing {
    type Error = ();
    fn register(&mut self, _: &mut TextInputMessageBoxState) -> Result<(), ()> {
        Ok(())
    }
    fn set_text(&mut self, _: &mut TextInputMessageBoxState, _: Option<&[u16]>) -> Result<(), ()> {
        Ok(())
    }
    fn complete(&mut self, _: &mut TextInputMessageBoxState, _: u64, _: &[u16]) -> Result<(), ()> {
        Ok(())
    }
}
fn state(text: &str, capacity: i32) -> TextInputMessageBoxState {
    let mut s = TextInputMessageBoxState::new([800., 600.], capacity).unwrap();
    s.show(None, &mut Editing).unwrap();
    for unit in text.encode_utf16() {
        s.on_text_input(unit, (48..=57).contains(&unit), false, &mut Editing)
            .unwrap();
    }
    s
}
fn cursor(host: &Host) -> Option<InputDrawTransform> {
    host.calls.iter().find_map(|c| {
        if let C::Blinker(m) = c {
            Some(*m)
        } else {
            None
        }
    })
}

#[test]
fn daisy_and_screen_failure_preserve_distinct_mutation_prefixes() {
    let f = font();
    let mut s = state("A", 3);
    let mut h = Host::default();
    let mut ctx = context(&f);
    ctx.screen = [1024., 768.];
    ctx.daisy_wheel = true;
    assert_eq!(s.draw_input(0.25, ctx, &mut h), Ok(false));
    assert!(h.calls.is_empty());
    assert_eq!(s.base.center, [400., 300.]);
    assert_eq!(s.base.alpha, 0.);
    assert_eq!(s.blink_timer, 0.);
    h.fail = Some("screen");
    let mut ctx = context(&f);
    ctx.screen = [1024., 768.];
    assert_eq!(
        s.draw_input(0.25, ctx, &mut h),
        Err(TextInputDrawError::Host("screen"))
    );
    assert_eq!(s.base.center, [512., 384.]);
    assert_eq!(s.base.alpha, 0.);
    h.fail = Some("page");
    assert_eq!(
        s.draw_input(0.25, context(&f), &mut h),
        Err(TextInputDrawError::Host("page"))
    );
    assert_eq!(s.base.alpha, 1.);
    assert_eq!(s.blink_timer, 0.);
    assert!(!s.blink_line);
}

#[test]
fn page_rotation_blink_strict_boundary_catchup_and_reshow_retention() {
    let f = font();
    let mut s = state("", 3);
    let mut h = Host::default();
    s.draw_input(0., context(&f), &mut h).unwrap();
    assert!(!h.calls.iter().any(|c| matches!(c, C::SetBlinker(_))));
    assert!(h.calls.contains(&C::PreparePage {
        transform: [
            [0., 1., 0., 0.],
            [-1., 0., 0., 0.],
            [0., 0., 1., 0.],
            [400., 300., 0., 1.]
        ],
        color: [1., 1., 1., 0.],
    }));
    h.calls.clear();
    s.draw_input(1., context(&f), &mut h).unwrap();
    let blink: Vec<_> = h
        .calls
        .iter()
        .filter(|c| matches!(c, C::SetBlinker(_) | C::MarkInputDirty))
        .cloned()
        .collect();
    assert_eq!(
        blink,
        [
            C::SetBlinker(vec![95, 0]),
            C::MarkInputDirty,
            C::SetBlinker(vec![0]),
            C::MarkInputDirty
        ]
    );
    assert_eq!(s.blink_timer, 0.);
    assert!(!s.blink_line);
    s.draw_input(0.125, context(&f), &mut h).unwrap();
    assert_eq!(s.blink_timer, 0.375);
    assert!(s.blink_line);
    s.show(None, &mut Editing).unwrap();
    assert_eq!(s.blink_timer, 0.375);
    assert!(s.blink_line);
    h.calls.clear();
    s.draw_input(0.375, context(&f), &mut h).unwrap();
    assert!(!h.calls.iter().any(|c| matches!(c, C::SetBlinker(_))));
}

#[test]
fn multiline_retains_last_line_empty_rows_and_button_spread() {
    let f = font();
    let mut s = state("AB\nC\n\nD", 32);
    let mut h = Host::default();
    s.draw_input(0.25, context(&f), &mut h).unwrap();
    assert!(h.calls.contains(&C::Message([400., 260.])));
    let matrices: Vec<_> = h
        .calls
        .iter()
        .filter_map(|c| if let C::Input(m) = c { Some(*m) } else { None })
        .collect();
    assert_eq!(matrices.len(), 4);
    assert_eq!(
        matrices
            .iter()
            .map(|m| m[3][..2].to_vec())
            .collect::<Vec<_>>(),
        [
            vec![392., 280.],
            vec![396., 290.],
            vec![400., 300.],
            vec![396., 310.]
        ]
    );
    assert_eq!(cursor(&h).unwrap()[3][..2], [404., 310.]);
    assert_eq!(&h.chars[..2], &[68, 0]);
    assert_eq!(s.text(), "AB\nC\n\nD".encode_utf16().collect::<Vec<_>>());
    assert!(h
        .calls
        .contains(&C::ButtonPosition(InputButton::Okay, [350., 423.])));
    assert!(h
        .calls
        .contains(&C::ButtonPosition(InputButton::Cancel, [450., 423.])));
    assert_eq!(
        h.geometries
            .iter()
            .map(|(_, g)| g.quads.len())
            .collect::<Vec<_>>(),
        [2, 1, 0, 1]
    );
    assert_eq!(h.calls.last(), Some(&C::End));
}

#[test]
fn cursor_is_suppressed_at_capacity_but_negative_date_prefix_is_drawn() {
    let f = font();
    let mut s = state("AB", 2);
    let mut h = Host::default();
    s.draw_input(0.25, context(&f), &mut h).unwrap();
    assert!(cursor(&h).is_none());
    s.input_type = TextInputType::DATE;
    s.character_index = -1;
    h.calls.clear();
    s.draw_input(0., context(&f), &mut h).unwrap();
    assert_eq!(cursor(&h).unwrap()[3][..2], [392., 300.]);
    s.character_index = 1;
    h.calls.clear();
    s.draw_input(0., context(&f), &mut h).unwrap();
    assert_eq!(cursor(&h).unwrap()[3][..2], [400., 300.]);
    s.character_index = 10;
    h.calls.clear();
    s.draw_input(0., context(&f), &mut h).unwrap();
    assert!(cursor(&h).is_none());
}

#[test]
fn text_only_skips_primitives_but_keeps_begin_lookup_positions_and_cursor() {
    let f = font();
    let mut s = state("A", 3);
    let mut h = Host::default();
    s.set_text_only(true);
    s.draw_input(0.25, context(&f), &mut h).unwrap();
    assert!(h.calls.contains(&C::Begin));
    assert!(!h.calls.contains(&C::Background));
    assert!(cursor(&h).is_some());
    assert_eq!(h.title_calls, 1);
    assert!(h
        .calls
        .contains(&C::ButtonPosition(InputButton::Okay, [350., 418.])));
    assert!(!h
        .calls
        .iter()
        .any(|c| matches!(c, C::Button(_) | C::ButtonAlpha(_, _))));
    assert_eq!(h.calls.last(), Some(&C::End));
}

#[test]
fn blink_and_last_input_draw_failures_keep_native_partial_mutations() {
    let f = font();
    let mut s = state("AB\nC", 32);
    let mut h = Host {
        fail: Some("blinker"),
        ..Host::default()
    };
    assert_eq!(
        s.draw_input(0.125, context(&f), &mut h),
        Err(TextInputDrawError::Host("blinker"))
    );
    assert_eq!(s.blink_timer, 0.375);
    assert!(s.blink_line);
    assert!(!h.calls.contains(&C::MarkInputDirty));
    assert_eq!(h.title_calls, 0);
    h.fail = Some("input");
    h.calls.clear();
    assert_eq!(
        s.draw_input(0., context(&f), &mut h),
        Err(TextInputDrawError::Host("input"))
    );
    assert_eq!(&h.chars[..3], &[65, 66, 0]);
    assert_eq!(h.title_calls, 0);
    assert!(!h.calls.contains(&C::End));
    h.fail = Some("lookup");
    h.calls.clear();
    assert_eq!(
        s.draw_input(0., context(&f), &mut h),
        Err(TextInputDrawError::Host("lookup"))
    );
    assert_eq!(&h.chars[..2], &[67, 0]);
    assert!(cursor(&h).is_some());
    assert!(!h.calls.iter().any(|c| matches!(c, C::ButtonPosition(_, _))));
}

#[test]
fn discarded_measure_can_fail_before_first_line_mutation() {
    let f = font();
    let mut empty = content(8);
    empty.glyphs.clear();
    let broken = FontMetrics::from_content(&empty).unwrap();
    let mut s = state("A", 3);
    let mut h = Host::default();
    h.chars[0] = 66;
    let mut ctx = context(&f);
    ctx.input_font = &broken;
    assert_eq!(
        s.draw_input(0., ctx, &mut h),
        Err(TextInputDrawError::Font(FontError::EmptyFont))
    );
    assert!(h.calls.contains(&C::Message([400., 280.])));
    assert!(!h.calls.iter().any(|c| matches!(c, C::SetInput(_))));
    assert_eq!(h.chars[0], 66);
    h.chars[0] = 0;
    h.calls.clear();
    let mut ctx = context(&f);
    ctx.input_font = &broken;
    assert_eq!(
        s.draw_input(0., ctx, &mut h),
        Err(TextInputDrawError::Font(FontError::EmptyFont))
    );
    assert!(h.calls.contains(&C::SetInput(vec![65])));
    assert_eq!(h.chars[0], 65);
}

#[test]
fn literal_brackets_draw_glyphs_while_native_font_measure_skips_tags() {
    let f = font();
    let mut s = state("[A]", 8);
    let mut h = Host::default();
    s.draw_input(0.25, context(&f), &mut h).unwrap();
    assert_eq!(h.geometries.len(), 1);
    let (matrix, geometry) = &h.geometries[0];
    assert_eq!(
        geometry.quads.iter().map(|q| q.input).collect::<Vec<_>>(),
        [91, 65, 93]
    );
    assert_eq!(matrix[3][0], 400.); // MeasureText's independent formatting rules
    assert_eq!(cursor(&h).unwrap()[3][0], 400.);
}
