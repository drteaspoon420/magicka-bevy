use magicka_content::assets::{BitmapFont, Glyph};
use magicka_runtime::{
    bitmap_font::{FontError, FontMetrics, MessageTextLayout},
    dialog_definition::DialogDefinition,
    localization::{LocalizationError, LocalizationTable},
    scene_loading::custom_hash,
};
fn content() -> BitmapFont {
    BitmapFont {
        line_height: 10,
        base_line: 8,
        default_character: '?',
        texture: None,
        glyphs: [('A', 5), ('B', 7), (' ', 2), ('[', 3), ('?', 4)]
            .into_iter()
            .map(|(character, advance_width)| Glyph {
                character,
                advance_width,
                origin: [0; 2],
                size: [1; 2],
                left_side_bearing: 0,
                force_white: false,
            })
            .collect(),
        kerning: vec![('A', 'B', -2), ('A', '\n', -3), ('\n', 'B', -1)],
    }
}
fn font() -> FontMetrics {
    let mut font = FontMetrics::from_content(&content()).unwrap();
    font.spacing = 1;
    font
}
#[test]
fn measurement_newline_glyph_kerning_tags_and_spacing() {
    let font = font();
    assert_eq!(font.measure("", true).unwrap(), [0.0, 10.0]);
    assert_eq!(font.measure("AB", true).unwrap(), [11.0, 10.0]);
    assert_eq!(font.measure("A[c=1,1,1]B[/c]", true).unwrap(), [11.0, 10.0]);
    assert_eq!(font.measure("A\nB", true).unwrap(), [8.0, 20.0]);
    assert_eq!(font.measure("A\nB", false).unwrap(), [12.0, 20.0]);
    assert_eq!(font.measure("[[A", true).unwrap(), [9.0, 10.0]);
    assert_eq!(font.measure("A[unterminated B", true).unwrap(), [5.0, 10.0]);
    assert_eq!(font.measure("A\0B", true).unwrap(), [5.0, 10.0]);
    assert_eq!(font.measure("[tag\n]A", true).unwrap(), [5.0, 10.0]);
}
#[test]
fn utf16_limits_default_glyph_and_source_order_fallback() {
    let font = font();
    assert_eq!(font.measure("😀", true).unwrap(), [9.0, 10.0]);
    assert_eq!(
        font.measure_utf16(&[0xd83d, 0xde00], true, 1).unwrap(),
        [4.0, 10.0]
    );
    assert_eq!(
        font.measure_utf16(&[65, 66], true, -1).unwrap(),
        [0.0, 10.0]
    );
    assert_eq!(font.glyph_advance(65).unwrap(), (true, 5));
    assert_eq!(font.glyph_advance(100).unwrap(), (false, 4));
    let mut c = content();
    c.default_character = 'X';
    let font = FontMetrics::from_content(&c).unwrap();
    assert_eq!(font.glyph_advance(100).unwrap(), (false, 5));
    c.glyphs.clear();
    assert_eq!(
        FontMetrics::from_content(&c).unwrap().glyph_advance(100),
        Err(FontError::EmptyFont)
    );
}
#[test]
fn wrap_restarts_for_word_width_and_preserves_unbroken_words() {
    let font = font();
    let wrapped = font.wrap("AA BB", 11, false).unwrap();
    assert_eq!(wrapped.text, "AA\nBB");
    assert_eq!(wrapped.line_width, 12);
    let wrapped = font.wrap("AAAA", 2, false).unwrap();
    assert_eq!(wrapped.text, "AAAA");
    assert_eq!(wrapped.line_width, 2);
    assert_eq!(font.wrap("A ", 6, false).unwrap().text, "A\n");
    assert_eq!(font.wrap("A\nB\n", 100, false).unwrap().text, "A\nB\n");
    assert_eq!(font.wrap("A\0B", 100, false).unwrap().text, "A\0B");
    assert_eq!(
        font.wrap("[c=1]AA[/c] BB", 12, true).unwrap().text,
        "[c=1]AA[/c]\nBB"
    );
    assert_eq!(font.wrap("", -1, true).unwrap().line_width, -1);
}
#[test]
fn duplicate_content_is_rejected() {
    let mut c = content();
    c.glyphs.push(c.glyphs[0].clone());
    assert!(matches!(
        FontMetrics::from_content(&c),
        Err(FontError::DuplicateGlyph(65))
    ));
    let mut c = content();
    c.kerning.push(c.kerning[0]);
    assert!(matches!(
        FontMetrics::from_content(&c),
        Err(FontError::DuplicateKerning(65, 66))
    ));
}
#[test]
fn dialog_initialization_maximum_survives_reinitialization_and_partial_failure() {
    let mut strings = LocalizationTable::default();
    strings
        .strings
        .insert(custom_hash("#first"), "AA BB".into());
    strings.strings.insert(custom_hash("#second"), "#1;".into());
    let defs = DialogDefinition::read_collection(&[b"<Dialogs><Dialog><Interact><Message width=\"11\"><Text>#first</Text><Text>#second</Text></Message></Interact></Dialog></Dialogs>"]).unwrap();
    let mut message = defs[0].interactions[0].messages[0].clone();
    let mut layout = MessageTextLayout::default();
    assert_eq!(
        layout.initialize(&message, &strings, &font(), 10),
        Err(FontError::Localization(LocalizationError::MissingArguments))
    );
    assert_eq!(layout.texts, [Some("AA\nBB".into()), None]);
    let old_size = layout.minimum_box_size;
    assert_eq!(old_size, [16.0, 20.0]);
    message.texts.clear();
    layout.initialize(&message, &strings, &font(), 10).unwrap();
    assert!(layout.texts.is_empty());
    assert_eq!(layout.minimum_box_size, old_size);
    message
        .texts
        .push(defs[0].interactions[0].messages[0].texts[0].clone());
    strings.strings.insert(custom_hash("#first"), "A".into());
    layout.initialize(&message, &strings, &font(), 10).unwrap();
    assert_eq!(layout.texts, [Some("A".into())]);
    assert_eq!(layout.minimum_box_size, old_size);
}
