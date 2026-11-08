use magicka_content::assets::{BitmapFont, Glyph};
use magicka_runtime::{
    bitmap_font::{FontError, FontMetrics},
    dialog_input::DialogInputKind,
    dialog_manager::DialogManager,
    error_message_box::ErrorMessageBoxState,
    localization::LocalizationTable,
    menu_text_item::{MenuTextItemState, MENU_COLOR_DISABLED, MENU_COLOR_SELECTED},
    message_box_collection::ErrorMessageBoxes,
    scene_loading::custom_hash,
    text_geometry::TextAlign,
};
fn font() -> FontMetrics {
    FontMetrics::from_content(&BitmapFont {
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
    .unwrap()
}
#[test]
fn bounds_minimum_alignment_and_width_limit_are_independent_of_draw_scale() {
    let f = font();
    let mut item = MenuTextItemState::new("AAAA", [100., 50.], TextAlign::Center, 40, &f).unwrap();
    item.set_scale(2., &f).unwrap();
    item.set_max_width(10., &f).unwrap();
    assert_eq!(item.size, [10., 20.]);
    assert_eq!(item.top_left, [90., 30.]);
    assert_eq!(item.bottom_right, [110., 70.]);
    assert!(item.inside_bounds([90., 30.]));
    assert!(item.inside_bounds([110., 70.]));
    assert!(!item.inside_bounds([f32::NAN, 50.]));
    let draw = item.draw_plan(0.5, &f).unwrap();
    assert_eq!(
        draw.transform,
        [
            [0.15625, 0., 0., 0.],
            [0., 0.5, 0., 0.],
            [0.; 4],
            [100., 48., 0., 1.]
        ]
    );
    item.align = TextAlign::Left;
    item.update_bounds(&f).unwrap();
    assert_eq!(item.top_left[0], 100.);
    item.align = TextAlign::Right;
    item.update_bounds(&f).unwrap();
    assert_eq!(item.bottom_right[0], 100.);
}
#[test]
fn retained_buffer_grows_without_shrinking_and_measurement_stops_at_nul() {
    let f = font();
    let mut item = MenuTextItemState::new("A", [0.; 2], TextAlign::Left, 40, &f).unwrap();
    item.set_text(&"A".repeat(50), &f).unwrap();
    assert_eq!(item.characters.len(), 50);
    item.set_text("A", &f).unwrap();
    assert_eq!(item.characters.len(), 50);
    assert_eq!(item.characters[1], 0);
    assert_eq!(item.size, [20., 20.]);
    item.set_text("", &f).unwrap();
    assert_eq!(item.characters[0], 0);
}
#[test]
fn color_selection_and_clr_epsilon_width_gate_preserve_native_behavior() {
    let f = font();
    let mut item = MenuTextItemState::new("AAAA", [0.; 2], TextAlign::Center, 40, &f).unwrap();
    item.selected = true;
    assert_eq!(item.current_color(), MENU_COLOR_SELECTED);
    item.alpha = 0.25;
    assert_eq!(item.draw_plan(1., &f).unwrap().color, [1.5, 1.5, 1.5, 0.25]);
    item.enabled = false;
    assert_eq!(item.current_color(), MENU_COLOR_DISABLED);
    assert!(item.inside_bounds([0.; 2])); // disabled does not invalidate bounds
    item.set_max_width(f32::from_bits(1), &f).unwrap();
    assert_eq!(item.size[0], 32.);
    item.set_max_width(f32::from_bits(2), &f).unwrap();
    assert_eq!(item.size[0], f32::from_bits(2));
}
#[test]
fn pointer_dispatch_hits_original_ok_bounds_and_updates_button_selection() {
    let f = font();
    let mut boxes = ErrorMessageBoxes::default();
    let mut manager = DialogManager::new(vec![]).unwrap();
    let h = boxes.insert(
        ErrorMessageBoxState::new_with_ok_button([800., 600.], String::new(), "OK", &f).unwrap(),
    );
    boxes.show(h, "A", &f, &mut manager, false).unwrap();
    boxes
        .dispatch_pointer(
            &mut manager,
            DialogInputKind::MouseMove,
            false,
            [390., 354.],
            || Ok::<_, ()>(()),
            |_| Ok(()),
        )
        .unwrap();
    assert!(boxes.get(h).unwrap().ok_button.as_ref().unwrap().selected);
    boxes
        .dispatch_pointer(
            &mut manager,
            DialogInputKind::MouseClick,
            true,
            [410., 374.],
            || Ok::<_, ()>(()),
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(boxes.get(h).unwrap().timer, 0.);
    boxes.get_mut(h).unwrap().timer = 5.;
    boxes
        .dispatch_pointer(
            &mut manager,
            DialogInputKind::MouseClick,
            true,
            [410.01, 374.],
            || Ok::<_, ()>(()),
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(boxes.get(h).unwrap().timer, 5.);
}

fn strings(value: &str) -> LocalizationTable {
    let mut strings = LocalizationTable::default();
    strings.strings.insert(custom_hash("#title"), value.into());
    strings
}
#[test]
fn language_refresh_updates_display_and_bounds_but_preserves_cached_name() {
    let f = font();
    let id = custom_hash("#title");
    let mut item =
        MenuTextItemState::new_localized(id, [0.; 2], TextAlign::Left, 40, &strings("first"), &f)
            .unwrap();
    assert_eq!(item.text.as_deref(), Some("first"));
    assert_eq!(item.size[0], 40.);
    item.language_changed(&strings("B"), &f).unwrap();
    assert_eq!(item.characters[..2], [66, 0]);
    assert_eq!(item.size[0], 20.);
    assert_eq!(item.text.as_deref(), Some("first"));
    item.set_hash(0);
    let mut changed_font = f.clone();
    changed_font.line_height = 40;
    item.language_changed(&strings("other"), &changed_font)
        .unwrap();
    assert_eq!(item.characters[..2], [66, 0]);
    assert_eq!(item.size, [20., 20.]); // zero hash skips even the bounds update
}
#[test]
fn hash_assignment_and_literal_override_preserve_future_language_binding() {
    let f = font();
    let mut item = MenuTextItemState::new("literal", [0.; 2], TextAlign::Left, 64, &f).unwrap();
    let old_chars = item.characters.clone();
    let old_bounds = item.bottom_right;
    item.set_hash(custom_hash("#title"));
    assert_eq!(item.characters, old_chars);
    assert_eq!(item.bottom_right, old_bounds);
    item.set_text("override", &f).unwrap();
    assert_eq!(item.hash, custom_hash("#title"));
    item.language_changed(&strings("A"), &f).unwrap();
    assert_eq!(item.characters[..2], [65, 0]);
    assert_eq!(item.text.as_deref(), Some("override"));
    item.set_title(custom_hash("#title"), &strings("new title"), &f)
        .unwrap();
    assert_eq!(item.text.as_deref(), Some("new title"));
    item.set_localized_text(custom_hash("#title"), &strings("new text"), &f)
        .unwrap();
    assert_eq!(item.text.as_deref(), Some("new text"));
}
#[test]
fn wrapping_uses_retained_hash_and_language_refresh_discards_display_wrap() {
    let f = font();
    let mut item = MenuTextItemState::new("override", [0.; 2], TextAlign::Left, 40, &f).unwrap();
    item.set_hash(custom_hash("#title"));
    let table = strings("AAA BBB CCC");
    item.wrap_text(24, &table, &f).unwrap();
    assert_eq!(item.text.as_deref(), Some("AAA\nBBB\nCCC"));
    item.language_changed(&table, &f).unwrap();
    assert_eq!(
        item.characters[..11],
        "AAA BBB CCC".encode_utf16().collect::<Vec<_>>()
    );
    assert_eq!(item.text.as_deref(), Some("AAA\nBBB\nCCC"));
}
#[test]
fn nullable_players_are_empty_display_without_erasing_the_stale_name_on_event() {
    let f = font();
    let id = custom_hash("#players");
    let table = LocalizationTable::default();
    let mut item =
        MenuTextItemState::new_localized(id, [0.; 2], TextAlign::Left, 40, &table, &f).unwrap();
    assert_eq!(item.text, None);
    assert_eq!(item.characters[0], 0);
    item.set_text("override", &f).unwrap();
    item.language_changed(&table, &f).unwrap();
    assert_eq!(item.text.as_deref(), Some("override"));
    assert_eq!(item.characters[0], 0);
    assert_eq!(item.size, [20., 20.]);
    item.set_localized_text(id, &table, &f).unwrap();
    assert_eq!(item.text, None);
    assert!(item.wrap_text(30, &table, &f).is_err());
}
#[test]
fn null_zero_capacity_failure_preserves_different_title_and_set_text_prefixes() {
    let f = font();
    let id = custom_hash("#players");
    let table = LocalizationTable::default();
    let mut item = MenuTextItemState::new("", [0.; 2], TextAlign::Left, 0, &f).unwrap();
    assert_eq!(
        item.set_localized_text(id, &table, &f),
        Err(FontError::EmptyTextBuffer)
    );
    assert_eq!(item.hash, id);
    assert_eq!(item.text.as_deref(), Some(""));
    assert_eq!(
        item.set_title(id, &table, &f),
        Err(FontError::EmptyTextBuffer)
    );
    assert_eq!(item.text, None);
}
#[test]
fn font_copy_discards_source_spacing_and_uses_new_metrics_for_event_bounds() {
    let mut old = font();
    old.spacing = 1;
    let mut source = font();
    source.spacing = 7;
    source.line_height = 40;
    let table = strings("AAAA");
    let mut item = MenuTextItemState::new_localized(
        custom_hash("#title"),
        [0.; 2],
        TextAlign::Left,
        40,
        &table,
        &old,
    )
    .unwrap();
    assert_eq!(item.size, [35., 20.]);
    old.read_from(&source);
    assert_eq!(old.spacing, 0);
    assert_eq!(item.size, [35., 20.]); // font copy itself does not dirty Text
    item.language_changed(&table, &old).unwrap();
    assert_eq!(item.size, [32., 40.]);
}
#[test]
fn global_box_callback_updates_killed_waiting_objects_and_leaves_error_ok_alone() {
    use magicka_runtime::{
        error_message_box::ok_title_id, waiting_message_box::WaitingMessageBoxState,
    };
    let f = font();
    let mut table = strings("first");
    table.strings.insert(ok_title_id(), "old OK".into());
    let mut boxes = ErrorMessageBoxes::default();
    let error =
        boxes.insert(ErrorMessageBoxState::new_localized([800., 600.], &table, &f).unwrap());
    let mut waiting = WaitingMessageBoxState::new([800., 600.]);
    waiting
        .show(custom_hash("#title"), &table, &f, |_| Ok::<_, ()>(()))
        .unwrap();
    waiting.kill();
    let waiting = boxes.insert_waiting(waiting);
    table.strings.insert(ok_title_id(), "new OK".into());
    table
        .strings
        .insert(custom_hash("#title"), "new waiting".into());
    boxes.language_changed(error, &table, &f).unwrap();
    boxes.language_changed(waiting, &table, &f).unwrap();
    assert_eq!(
        boxes
            .get(error)
            .unwrap()
            .ok_button
            .as_ref()
            .unwrap()
            .text
            .as_deref(),
        Some("old OK")
    );
    assert_eq!(
        boxes.get_waiting(waiting).unwrap().message.as_deref(),
        Some("new waiting")
    );
    assert!(boxes.get_waiting(waiting).unwrap().base.killed);
    assert!(boxes.active().is_empty());
}

#[test]
#[ignore = "requires the private original MenuDefault asset"]
fn private_original_menu_font_button_measurement() {
    use magicka_content::{assets::Asset, xna::XnaObject, Content};
    let path = magicka_content::retail_content_dir().join("Languages/eng/Font/MenuDefault.xnb");
    let bytes = std::fs::read(path).unwrap();
    let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
    let parsed = magicka_content::parse_primary(&xnb).unwrap().unwrap();
    assert_eq!(parsed.trailing, 0);
    let Content::Xna(object) = parsed.content else {
        panic!("not an XNA object")
    };
    let XnaObject::Asset(asset) = *object else {
        panic!("not an asset")
    };
    let Asset::BitmapFont(content) = *asset else {
        panic!("not a font")
    };
    let mut font = FontMetrics::from_content(&content).unwrap();
    font.spacing = 1; // FontManager custom spacing for MenuDefault
    let item = MenuTextItemState::new("OK", [400., 364.], TextAlign::Center, 40, &font).unwrap();
    println!(
        "line_height={} glyph_count={} measured_OK={:?} bounds={:?}..{:?} transform={:?}",
        content.line_height,
        content.glyphs.len(),
        font.measure("OK", true).unwrap(),
        item.top_left,
        item.bottom_right,
        item.draw_plan(1., &font).unwrap().transform
    );
    assert!(item.inside_bounds([400., 364.]));
    assert!(!item.inside_bounds([0., 0.]));
    assert_eq!(content.line_height, 35);
    assert_eq!(content.glyphs.len(), 183);
    assert_eq!(font.measure("OK", true).unwrap(), [53., 35.]);
    assert_eq!(item.top_left, [373.5, 346.5]);
    assert_eq!(item.bottom_right, [426.5, 381.5]);
    assert_eq!(
        item.draw_plan(1., &font).unwrap().transform[3],
        [400., 347., 0., 1.]
    );
}
