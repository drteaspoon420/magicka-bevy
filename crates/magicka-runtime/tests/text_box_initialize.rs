use magicka_content::assets::{BitmapFont, Glyph};
use magicka_runtime::{
    bitmap_font::FontMetrics,
    localization::{LocalizationError, LocalizationTable},
    scene_loading::custom_hash,
    text_box_initialize::*,
};
fn font(height: i32, advance: i32, spacing: i32) -> FontMetrics {
    let c = BitmapFont {
        line_height: height,
        base_line: height - 2,
        default_character: '?',
        texture: None,
        glyphs: ['A', 'B', '?']
            .into_iter()
            .map(|character| Glyph {
                character,
                origin: [0; 2],
                size: [advance, height],
                advance_width: advance,
                left_side_bearing: 0,
                force_white: false,
            })
            .collect(),
        kerning: vec![],
    };
    let mut font = FontMetrics::from_content(&c).unwrap();
    font.spacing = spacing;
    font
}
#[derive(Default)]
struct Host {
    calls: Vec<String>,
    fail: bool,
    character: bool,
    world_calls: Vec<String>,
}
impl magicka_runtime::message_playback::MessageHost for Host {
    type Error = &'static str;
    fn resolve_owner(
        &mut self,
        _: i32,
    ) -> Result<Option<magicka_runtime::message_playback::MessageOwner>, &'static str> {
        Ok(Some(magicka_runtime::message_playback::MessageOwner {
            id: 42,
            dead: false,
            character: self.character,
            dead_state: false,
            animation_allowed: true,
            special_idle: 0,
        }))
    }
    fn cue_stopping(&self) -> Option<bool> {
        None
    }
    fn effect(
        &mut self,
        _: &magicka_runtime::dialog_definition::MessageDefinition,
        effect: magicka_runtime::message_playback::MessageEffect,
    ) -> Result<(), &'static str> {
        self.world_calls.push(format!("{effect:?}"));
        Ok(())
    }
}
impl magicka_runtime::message_textbox::MessageTextBoxWorld for Host {
    fn entity_scene(&mut self, owner: i32) -> Result<Option<i32>, &'static str> {
        self.world_calls.push(format!("scene {owner}"));
        Ok(Some(73))
    }
}
impl TextBoxInitHost for Host {
    type Error = &'static str;
    fn load_background(&mut self, path: &str) -> Result<(), Self::Error> {
        self.calls.push(path.into());
        if self.fail {
            Err("load failed")
        } else {
            Ok(())
        }
    }
}
fn init(text: Option<&str>, name: i32) -> TextBoxInit<'_> {
    TextBoxInit {
        scene: Some(7),
        location: TextBoxLocation::Screen {
            position: [0.25, -0.5],
            name,
        },
        text,
        minimum_size: [100., 50.],
        force_on_screen: true,
        ttl: 1.,
    }
}
#[test]
fn cutscene_initialization_uses_name_font_for_body_and_forces_width_after_success() {
    use magicka_runtime::cutscene_text::CutsceneTextWidget;
    let (body, name) = (font(10, 2, 0), font(20, 7, 1));
    let mut widget = CutsceneTextWidget::default();
    assert_eq!(widget.widget.name_characters.len(), 32);
    let mut host = Host::default();
    let mut input = init(Some("AB\nAB"), 0);
    input.minimum_size = [0.; 2];
    widget
        .initialize_target(
            input,
            &LocalizationTable::default(),
            TextBoxFonts {
                body: &body,
                owner_name: &name,
            },
            &mut host,
            10,
        )
        .unwrap();
    assert_eq!(widget.widget.state.size, [640, 40]);
    widget.widget.state.size = [13, 77];
    input.text = None;
    widget
        .initialize_target(
            input,
            &LocalizationTable::default(),
            TextBoxFonts {
                body: &body,
                owner_name: &name,
            },
            &mut host,
            10,
        )
        .unwrap();
    assert_eq!(widget.widget.state.size, [640, 77]);
}
#[test]
fn cutscene_width_override_is_not_reached_after_base_background_failure() {
    use magicka_runtime::cutscene_text::CutsceneTextWidget;
    let name = font(20, 7, 1);
    let mut widget = CutsceneTextWidget::default();
    widget.widget.state.size = [13, 77];
    let mut host = Host {
        fail: true,
        ..Host::default()
    };
    assert_eq!(
        widget.initialize_target(
            init(Some("AB"), 0),
            &LocalizationTable::default(),
            TextBoxFonts {
                body: &name,
                owner_name: &name
            },
            &mut host,
            10
        ),
        Err(TextBoxInitError::Host("load failed"))
    );
    assert_eq!(widget.widget.state.size, [13, 77]);
    assert!(widget.widget.state.grow && widget.widget.state.scene_initialized);
}
#[test]
fn playback_initializes_real_box_for_screen_world_and_entity_paths() {
    use magicka_runtime::{
        bitmap_font::MessageTextLayout, dialog_definition::*, message_playback::*,
        message_textbox::*,
    };
    let (body, name) = (font(14, 2, 0), font(16, 7, 1));
    let mut strings = LocalizationTable::default();
    strings.strings.insert(9, "AA".into());
    strings.strings.insert(42, "BB".into());
    let definition = MessageDefinition {
        owner: 9,
        message_type: 0,
        force_on_screen: true,
        facing: false,
        width: 200,
        texts: vec![TextDefinition {
            ttl: 0.7,
            animation: 0,
            sound_bank: 0,
            sound: 0,
            localization: 0,
            arguments: None,
        }],
    };
    let layout = MessageTextLayout {
        texts: vec![Some("AB".into())],
        minimum_box_size: [30., 40.],
    };
    for (placement, character, owner, scene, position, screen) in [
        (
            MessagePlacement::Screen([0.2, -0.3]),
            false,
            None,
            17,
            [0.2, -0.3, 0.],
            true,
        ),
        (
            MessagePlacement::World {
                position: [4., 5., 6.],
                focus: false,
            },
            false,
            None,
            17,
            [4., 5., 6.],
            false,
        ),
        (
            MessagePlacement::World {
                position: [4., 5., 6.],
                focus: false,
            },
            true,
            Some(42),
            17,
            [0.; 3],
            false,
        ),
        (
            MessagePlacement::Entity {
                default_owner: Some(99),
                focus: false,
            },
            false,
            Some(42),
            73,
            [0.; 3],
            false,
        ),
    ] {
        let mut widget = TextBoxWidget::default();
        let mut world = Host {
            character,
            ..Host::default()
        };
        let mut playback = MessagePlayback::default();
        let mut host = MessageTextBoxHost {
            widget: &mut widget,
            layout: &layout,
            strings: &strings,
            fonts: TextBoxFonts {
                body: &body,
                owner_name: &name,
            },
            scene: Some(17),
            world: &mut world,
            substitution_budget: 10,
        };
        assert!(!playback.advance(&definition, placement, &mut host).unwrap());
        assert_eq!(playback.current, 1);
        assert_eq!(widget.state.owner, owner);
        assert_eq!(widget.scene, Some(scene));
        assert_eq!(widget.state.world_position, position);
        assert_eq!(widget.state.screen_position, screen);
        assert_eq!(widget.state.ttl, 0.7);
        assert!(
            widget.state.force_on_screen && widget.state.automatic_advance && widget.state.grow
        );
        assert_eq!(widget.state.text.as_deref(), Some("AB"));
        assert_eq!(
            widget.owner_name.as_deref(),
            Some(if owner.is_some() { "BB" } else { "AA" })
        );
        assert_eq!(widget.state.size, [50, 56]);
        assert_eq!(world.calls, ["UI/HUD/Dialog_Say"]);
        assert_eq!(
            world.world_calls.len(),
            usize::from(scene == 73 || (!screen && owner.is_none()))
        );
    }
}
#[test]
fn playback_load_failure_retains_prior_cue_and_box_prefix_then_completion_hides() {
    use magicka_runtime::{
        bitmap_font::MessageTextLayout, dialog_definition::*, message_playback::*,
        message_textbox::*,
    };
    let (body, name) = (font(14, 2, 0), font(16, 7, 1));
    let strings = LocalizationTable::default();
    let definition = MessageDefinition {
        owner: 9,
        message_type: 0,
        force_on_screen: false,
        facing: false,
        width: 200,
        texts: vec![TextDefinition {
            ttl: 0.7,
            animation: 0,
            sound_bank: 0,
            sound: 1,
            localization: 0,
            arguments: None,
        }],
    };
    let layout = MessageTextLayout {
        texts: vec![Some("AB".into())],
        minimum_box_size: [30., 40.],
    };
    let mut widget = TextBoxWidget::default();
    let mut world = Host {
        fail: true,
        ..Host::default()
    };
    let mut playback = MessagePlayback::default();
    let mut host = MessageTextBoxHost {
        widget: &mut widget,
        layout: &layout,
        strings: &strings,
        fonts: TextBoxFonts {
            body: &body,
            owner_name: &name,
        },
        scene: Some(17),
        world: &mut world,
        substitution_budget: 10,
    };
    assert_eq!(
        playback.advance(&definition, MessagePlacement::Screen([0.; 2]), &mut host),
        Err(MessagePlaybackError::Host(MessageTextBoxError::Initialize(
            TextBoxInitError::Host("load failed")
        )))
    );
    assert_eq!(playback.current, 0);
    assert!(host.widget.state.grow && host.widget.state.scene_initialized);
    assert_eq!(host.widget.state.text.as_deref(), Some("AB"));
    assert_eq!(host.world.world_calls.len(), 1);
    host.world.fail = false;
    assert!(!playback
        .advance(&definition, MessagePlacement::Screen([0.; 2]), &mut host)
        .unwrap());
    let calls = host.world.world_calls.len();
    assert!(playback
        .advance(&definition, MessagePlacement::Screen([0.; 2]), &mut host)
        .unwrap());
    assert!(!host.widget.state.grow);
    assert_eq!(playback.current, 0);
    assert_eq!(host.world.world_calls.len(), calls);
}
#[test]
fn owner_name_fonts_indent_raw_strings_and_draw_positions() {
    let mut strings = LocalizationTable::default();
    strings.strings.insert(9, "AA".into());
    let (body, name) = (font(14, 2, 0), font(16, 7, 1));
    let mut widget = TextBoxWidget::default();
    let mut host = Host::default();
    widget
        .initialize(
            init(Some("AB"), 9),
            &strings,
            TextBoxFonts {
                body: &body,
                owner_name: &name,
            },
            &mut host,
            10,
        )
        .unwrap();
    assert_eq!(widget.state.size, [120, 66]);
    assert!(widget.show_name);
    assert_eq!(widget.owner_name.as_deref(), Some("AA"));
    assert_eq!(widget.state.world_position, [0.25, -0.5, 0.]);
    assert!(widget.state.screen_position);
    assert_eq!(host.calls, ["UI/HUD/Dialog_Say"]);
    assert_eq!(
        widget.draw_origins([200., 100.], 0.5, 16),
        TextBoxDrawOrigins {
            owner_name: Some([170., 83.5]),
            body: [180., 91.5]
        }
    );
    strings.strings.insert(9, "A\\nB".into());
    widget
        .initialize(
            init(Some("AB"), 9),
            &strings,
            TextBoxFonts {
                body: &body,
                owner_name: &name,
            },
            &mut host,
            10,
        )
        .unwrap();
    assert_eq!(&widget.name_characters[..4], &[65, 92, 110, 66]);
    widget
        .initialize(
            init(Some("AB"), 99),
            &strings,
            TextBoxFonts {
                body: &body,
                owner_name: &name,
            },
            &mut host,
            10,
        )
        .unwrap();
    assert!(!widget.show_name);
    assert_eq!(widget.state.size, [100, 50]);
    assert_eq!(
        widget.draw_origins([200., 100.], 1., 16),
        TextBoxDrawOrigins {
            owner_name: None,
            body: [150., 75.]
        }
    );
}
#[test]
fn nullable_players_name_and_empty_body_keep_box_dimensions() {
    let strings = LocalizationTable::default();
    let (body, name) = (font(14, 2, 0), font(16, 7, 1));
    let mut widget = TextBoxWidget::default();
    let mut host = Host::default();
    widget
        .initialize(
            init(Some("AB"), custom_hash("#players")),
            &strings,
            TextBoxFonts {
                body: &body,
                owner_name: &name,
            },
            &mut host,
            10,
        )
        .unwrap();
    assert!(widget.show_name);
    assert_eq!(widget.owner_name, None);
    assert_eq!(widget.state.size, [120, 66]);
    assert_eq!(widget.name_characters[0], 0);
    widget.state.scale = 0.5;
    for text in [None, Some("")] {
        let mut i = init(text, 0);
        i.minimum_size = [1., 1.];
        widget
            .initialize(
                i,
                &strings,
                TextBoxFonts {
                    body: &body,
                    owner_name: &name,
                },
                &mut host,
                10,
            )
            .unwrap();
        assert_eq!(widget.state.size, [120, 66]);
        assert_eq!(widget.state.scale, 0.5);
    }
}
#[test]
fn background_failure_and_cycle_failure_preserve_native_mutation_prefixes() {
    let mut strings = LocalizationTable::default();
    strings.strings.insert(9, "A".into());
    strings
        .strings
        .insert(custom_hash("#cycle"), "#cycle;".into());
    let (body, name) = (font(14, 2, 0), font(16, 7, 1));
    let mut widget = TextBoxWidget::default();
    let mut host = Host::default();
    widget
        .initialize(
            init(Some("A"), 9),
            &strings,
            TextBoxFonts {
                body: &body,
                owner_name: &name,
            },
            &mut host,
            10,
        )
        .unwrap();
    host.fail = true;
    let mut i = init(Some("B"), 0);
    i.scene = Some(8);
    i.ttl = 3.;
    assert_eq!(
        widget.initialize(
            i,
            &strings,
            TextBoxFonts {
                body: &body,
                owner_name: &name
            },
            &mut host,
            10
        ),
        Err(TextBoxInitError::Host("load failed"))
    );
    assert_eq!(widget.scene, Some(8));
    assert_eq!(widget.state.typing.characters[0], 66);
    assert_eq!(widget.state.ttl, 3.);
    assert!(widget.show_name);
    assert_eq!(widget.state.size, [120, 66]);
    let calls = host.calls.len();
    i.text = Some("#cycle;");
    i.scene = Some(9);
    i.ttl = 4.;
    i.location = TextBoxLocation::World {
        position: [1., 2., 3.],
        name: 0,
    };
    assert_eq!(
        widget.initialize(
            i,
            &strings,
            TextBoxFonts {
                body: &body,
                owner_name: &name
            },
            &mut host,
            2
        ),
        Err(TextBoxInitError::Localization(
            LocalizationError::SubstitutionLimit
        ))
    );
    assert_eq!(widget.scene, Some(8));
    assert_eq!(widget.state.text.as_deref(), Some("#cycle;"));
    assert_eq!(widget.state.typing.characters[0], 66);
    assert_eq!(widget.state.world_position, [1., 2., 3.]);
    assert_eq!(widget.state.ttl, 4.);
    assert_eq!(host.calls.len(), calls);
}
#[test]
fn scene_guard_entity_location_and_integer_dimension_failures() {
    let strings = LocalizationTable::default();
    let (body, name) = (font(14, 2, 0), font(16, 7, 1));
    let mut widget = TextBoxWidget::default();
    let mut host = Host::default();
    widget.state.scale = 0.5;
    let mut i = init(Some("A"), 0);
    i.scene = None;
    assert_eq!(
        widget.initialize(
            i,
            &strings,
            TextBoxFonts {
                body: &body,
                owner_name: &name
            },
            &mut host,
            10
        ),
        Err(TextBoxInitError::MissingScene)
    );
    assert_eq!(widget.state.scale, 0.5);
    assert!(!widget.state.grow);
    i.scene = Some(1);
    i.location = TextBoxLocation::Entity(Some(11));
    widget.state.world_position = [2., 3., 4.];
    widget
        .initialize(
            i,
            &strings,
            TextBoxFonts {
                body: &body,
                owner_name: &name,
            },
            &mut host,
            10,
        )
        .unwrap();
    assert_eq!(widget.state.owner, Some(11));
    assert!(!widget.state.screen_position);
    assert_eq!(widget.state.world_position, [2., 3., 4.]);
    i.location = TextBoxLocation::Entity(None);
    i.minimum_size = [25.75, f32::INFINITY];
    assert_eq!(
        widget.initialize(
            i,
            &strings,
            TextBoxFonts {
                body: &body,
                owner_name: &name
            },
            &mut host,
            10
        ),
        Err(TextBoxInitError::UnsupportedBoxSize)
    );
    assert_eq!(widget.state.owner, None);
    assert_eq!(widget.state.size[0], 25);
    assert_eq!(widget.state.size[1], 50);
}
#[test]
fn raw_lookup_and_reference_pass_do_not_apply_numeric_arguments() {
    let mut strings = LocalizationTable::default();
    strings.strings.insert(1, "A\\nB".into());
    strings.strings.insert(custom_hash("#1"), "A".into());
    strings.strings.insert(custom_hash("#named"), "#1;".into());
    assert_eq!(strings.try_get_string(1).value, Some("A\\nB"));
    assert_eq!(strings.get_string(1).as_deref(), Some("A\nB"));
    assert!(!strings.try_get_string(2).found);
    assert!(strings.try_get_string(custom_hash("#players")).found);
    assert_eq!(
        strings.parse_references("#named;", 10).unwrap(),
        "[c=1,1,1][c=1,1,1]A[/c][/c]"
    );
    assert_eq!(
        strings
            .parse_references("#players; tail #unterminated", 10)
            .unwrap(),
        "[c=1,1,1][/c] tail #unterminated"
    );
}
