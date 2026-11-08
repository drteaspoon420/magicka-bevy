//! Opt-in synthetic devices driving the real gameplay input/HUD path.
//! No separate UI or proprietary executable is involved.
use super::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use std::path::PathBuf;

#[derive(Resource)]
struct Audit {
    directory: PathBuf,
    frame: u32,
    pads: [Entity; 4],
    records: Vec<serde_json::Value>,
}
pub(super) fn install(app: &mut App) {
    let args: Vec<_> = std::env::args().collect();
    let Some(directory) = args
        .windows(2)
        .find(|a| a[0] == "--controller-audit")
        .map(|a| PathBuf::from(&a[1]))
    else {
        return;
    };
    std::fs::create_dir_all(&directory).expect("controller audit directory");
    let pads = [
        "Xbox Series Controller",
        "DualSense Wireless Controller",
        "Steam Deck",
        "Steam Controller",
    ]
    .map(|name| {
        app.world_mut()
            .spawn((Gamepad::default(), Name::new(name)))
            .id()
    });
    app.insert_resource(Audit {
        directory,
        frame: 0,
        pads,
        records: Vec::new(),
    })
    .add_systems(PreUpdate, inject.after(InputSystems).before(collect))
    .add_systems(Update, capture.after(ControllerPrompts));
}
fn inject(
    mut audit: ResMut<Audit>,
    state: Res<ControllerInput>,
    hud: Query<(), With<crate::play_hud::KeyboardGeometry>>,
    mut pads: Query<&mut Gamepad>,
    mut preferences: ResMut<settings::Preferences>,
    mut play: Option<ResMut<crate::play::Play>>,
) {
    if !state.in_game || hud.iter().count() != 8 {
        return;
    }
    audit.frame += 1;
    let frame = audit.frame;
    if frame >= 40 && (frame - 40).is_multiple_of(60) && frame < 520 {
        use magicka_runtime::{script_runtime::ScriptWorld, tutorial::TutorialAction};
        if let Some(play) = play.as_mut() {
            let action = TutorialAction::read(
                "sethint",
                [
                    ("id", "#Tu_Text_hint_room0b"),
                    ("appendcontroller", "true"),
                    ("animation", "conjurelife"),
                ],
            )
            .unwrap();
            play.session
                .world
                .tutorial_action(&action)
                .expect("tutorial host")
                .expect("original tutorial localization");
        }
    }

    if frame == 65 {
        if let Some(play) = play.as_mut() {
            let body = play.session.body();
            let p = body.position;
            let d = body.direction();
            let mut transform = magicka_content::skinned::IDENTITY;
            transform[12..15].copy_from_slice(&[p[0] + d[0] * 1.5, p[1], p[2] + d[2] * 1.5]);
            play.session
                .spawn_magick_book(5, transform)
                .expect("contextual pickup prompt");
        }
    }
    let case = frame.saturating_sub(40) / 60;
    preferences.style = if case < 4 {
        ElementStyle::RightStick
    } else {
        ElementStyle::BumperDpad
    };
    for (index, entity) in audit.pads.into_iter().enumerate() {
        let mut pad = pads.get_mut(entity).unwrap();
        pad.digital_mut().release_all();
        pad.digital_mut().clear();
        for axis in [
            GamepadAxis::LeftStickX,
            GamepadAxis::LeftStickY,
            GamepadAxis::RightStickX,
            GamepadAxis::RightStickY,
        ] {
            pad.analog_mut().set(axis, 0.0);
        }
        if frame >= 40 && ((frame - 40) / 60) as usize % 4 == index && case < 8 {
            let phase = (frame - 40) % 60;

            if phase < 10 {
                if preferences.style == ElementStyle::BumperDpad {
                    pad.digital_mut().press(GamepadButton::DPadUp);
                } else {
                    pad.analog_mut().set(GamepadAxis::RightStickX, 1.);
                }
            }
            if (10..20).contains(&phase) {
                if preferences.style == ElementStyle::BumperDpad {
                    pad.digital_mut().press(GamepadButton::LeftTrigger);
                    pad.digital_mut().press(GamepadButton::DPadRight);
                } else {
                    pad.analog_mut().set(GamepadAxis::RightStickY, -1.);
                }
            }
            if (30..40).contains(&phase) && preferences.style == ElementStyle::RightStick {
                pad.analog_mut().set(GamepadAxis::RightStickX, 0.4);
            }
            if (20..25).contains(&phase) {
                pad.digital_mut().press(GamepadButton::RightTrigger2);
            }
        }
    }
}
fn capture(
    mut commands: Commands,
    state: Res<ControllerInput>,
    mut audit: ResMut<Audit>,
    icons: Query<(&PromptIcon, &ImageNode)>,
    loaded: Option<Res<PromptImages>>,
    placements: Res<NativePrompts>,
    elements: Query<(&ElementPrompt, &PromptIcon, &Node)>,
    mut exit: MessageWriter<AppExit>,
) {
    if audit.frame < 40 {
        return;
    }
    let phase = (audit.frame - 40) % 60;
    let case = ((audit.frame - 40) / 60) as usize;
    let index = case % 4;
    if case >= 8 {
        std::fs::write(
            audit.directory.join("controller-audit.json"),
            serde_json::to_vec_pretty(&audit.records).unwrap(),
        )
        .unwrap();
        exit.write(AppExit::Success);
        return;
    }
    if phase != 35 {
        return;
    }
    let expected = [Family::Xbox, Family::Ps5, Family::Deck, Family::Steam][index];
    assert_eq!(state.family, expected);
    assert_eq!(state.entity, Some(audit.pads[index]));
    assert!(!state.show_legend);
    for element in 0..8 {
        let mut actual: Vec<_> = elements
            .iter()
            .filter(|(e, _, _)| e.0 == element)
            .map(|(_, p, n)| (if let Val::Px(x) = n.left { x } else { 0. }, p.glyph))
            .collect();
        actual.sort_by(|a, b| a.0.total_cmp(&b.0));
        let actual: Vec<_> = actual.into_iter().map(|(_, glyph)| glyph).collect();
        assert_eq!(
            actual,
            native::element_glyphs(element, state.element_style),
            "displayed element binding {element}"
        );
    }

    assert!(!icons.is_empty(), "actual tutorial glyphs must be present");
    let loaded = loaded.unwrap();
    for (icon, image) in &icons {
        assert_eq!(icon.family, expected);
        assert_eq!(icon.controller, state.entity);
        assert_eq!(icon.player_slot, state.player_slot);
        assert_eq!(image.image, loaded.0[index][icon.glyph]);
    }
    let owner = state.entity.unwrap().to_bits();
    audit.records.push(serde_json::json!({"player_slot":state.player_slot,"controller_entity":owner,"family":expected.label(),"element_style":state.element_style.key(),"element_mappings":8,"native_tutorial":true,"prompt_images":icons.iter().count(),"legend":false,"context_action":state.interaction,"glyph_bounds":placements.0.iter().map(|p|p.bounds).collect::<Vec<_>>()}));
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(audit.directory.join(format!(
            "{}-{}.png",
            expected.label().replace(' ', "-"),
            state.element_style.key()
        ))));
}
