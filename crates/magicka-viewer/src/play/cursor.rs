//! Game.SetCursor(active, Cursors): Magicka's animated `.ani` cursors,
//! exported by `cursor-export` into `cursors/`. Inactive (out of reach)
//! Talk/Examine/PickUp/Interact use the Gray variants; Default and Attack
//! share one image (Game.LoadContent's cursor table).
use super::Play;
use bevy::prelude::*;
use bevy::window::{CursorIcon, CustomCursor, CustomCursorImage, PrimaryWindow};
use magicka_runtime::play_session::Cursor;
use std::collections::HashMap;

struct Animation {
    hotspot: (u16, u16),
    frames: Vec<Handle<Image>>,
    sequence: Vec<usize>,
    /// Step durations in seconds (jiffies / 60).
    rates: Vec<f32>,
}

#[derive(Resource, Default)]
pub(super) struct Cursors {
    animations: HashMap<String, Animation>,
    current: Option<String>,
    elapsed: f32,
    shown: Option<(String, usize)>,
}

pub(super) fn load_cursors(mut commands: Commands, play: Res<Play>, assets: Res<AssetServer>) {
    let mut cursors = Cursors::default();
    let index = play.cache.join("cursors/cursors.json");
    let Ok(text) = std::fs::read_to_string(&index) else {
        warn!(
            "Magicka cursors missing ({}); run cursor-export",
            index.display()
        );
        commands.insert_resource(cursors);
        return;
    };
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&text)
    else {
        commands.insert_resource(cursors);
        return;
    };
    for (name, value) in map {
        let list = |key: &str| value[key].as_array().cloned().unwrap_or_default();
        let hotspot = list("hotspot");
        let animation = Animation {
            hotspot: (
                hotspot.first().and_then(|v| v.as_u64()).unwrap_or(0) as u16,
                hotspot.get(1).and_then(|v| v.as_u64()).unwrap_or(0) as u16,
            ),
            frames: list("frames")
                .iter()
                .filter_map(|f| f.as_str())
                .map(|f| assets.load(format!("cursors/{f}")))
                .collect(),
            sequence: list("sequence")
                .iter()
                .filter_map(|v| v.as_u64())
                .map(|v| v as usize)
                .collect(),
            rates: list("rates")
                .iter()
                .filter_map(|v| v.as_u64())
                .map(|v| v as f32 / 60.0)
                .collect(),
        };
        if !animation.frames.is_empty() {
            cursors.animations.insert(name, animation);
        }
    }
    commands.insert_resource(cursors);
}

fn file(cursor: Cursor, active: bool) -> &'static str {
    match (cursor, active) {
        (Cursor::Default, _) => "Default",
        (Cursor::Attack, _) => "Attack",
        (Cursor::Talk, true) => "Talk",
        (Cursor::Talk, false) => "TalkGray",
        (Cursor::Examine, true) => "Examine",
        (Cursor::Examine, false) => "ExamineGray",
        (Cursor::PickUp, true) => "PickUp",
        (Cursor::PickUp, false) => "PickUpGray",
        (Cursor::Interact, true) => "Interact",
        (Cursor::Interact, false) => "InteractGray",
    }
}

pub(super) fn update_cursor(
    mut commands: Commands,
    play: Res<Play>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    cursors: Option<ResMut<Cursors>>,
    windows: Query<Entity, With<PrimaryWindow>>,
    #[cfg(feature = "editor")] egui: Option<Res<bevy_egui::input::EguiWantsInput>>,
) {
    let Some(mut cursors) = cursors else {
        return;
    };
    // Over the developer UI, egui owns the cursor icon. bevy_egui writes it
    // only when egui's icon changes, so animating the game cursor here too
    // would flicker between the two; hand over once and re-apply on leaving.
    #[cfg(feature = "editor")]
    if egui.is_some_and(|e| e.is_pointer_over_area() || e.is_using_pointer()) {
        if cursors.shown.take().is_some() {
            let icon: CursorIcon = bevy::window::SystemCursorIcon::Default.into();
            for entity in &windows {
                commands.entity(entity).insert(icon.clone());
            }
        }
        return;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let (cursor, active) = play.session.cursor(shift);
    let name = file(cursor, active).to_string();
    if cursors.current.as_deref() != Some(name.as_str()) {
        cursors.current = Some(name.clone());
        cursors.elapsed = 0.0;
    }
    cursors.elapsed += time.delta_secs();
    let Some(animation) = cursors.animations.get(&name) else {
        return;
    };
    // Step through the sequence by its rates, looping.
    let total: f32 = animation.rates.iter().sum::<f32>().max(1e-3);
    let mut t = cursors.elapsed % total;
    let mut step = 0;
    for (i, rate) in animation.rates.iter().enumerate() {
        if t < *rate {
            step = i;
            break;
        }
        t -= rate;
    }
    let frame = animation.sequence.get(step).copied().unwrap_or(0);
    let key = (name.clone(), frame);
    if cursors.shown.as_ref() == Some(&key) {
        return;
    }
    let Some(handle) = animation.frames.get(frame).cloned() else {
        return;
    };
    let icon: CursorIcon = CustomCursor::Image(CustomCursorImage {
        handle,
        hotspot: animation.hotspot,
        ..Default::default()
    })
    .into();
    for entity in &windows {
        commands.entity(entity).insert(icon.clone());
    }
    cursors.shown = Some(key);
}
