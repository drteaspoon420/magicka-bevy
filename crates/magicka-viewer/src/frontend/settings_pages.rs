//! SubMenuOptionsGame and SubMenuOptionsGraphics: a title, rows of a
//! right-aligned label and a left-aligned value 40 pixels to its right,
//! cycled with ControllerRight/Left, ControllerA or a click, and the back
//! arrow. Values persist in settings.json (saved on leaving a changed
//! page, as OnExit does); the window settings apply to the game window.
use super::{
    book::{Font, Measure, PageDraw, Widget},
    pages::{back_button, back_item, graphic, loc, menu_text_item, Item, List, PageInput},
    Frontend,
};
use crate::frontend_tome::{MenuId, TomeView};
use bevy::{
    prelude::*,
    window::{PresentMode, PrimaryWindow, WindowMode},
};
use magicka_runtime::text_geometry::TextAlign;
use serde_json::{json, Value};

/// SubMenu.GetSettingLoc.
const ON: (&str, &str) = ("on", "#menu_opt_alt_01");
const OFF: (&str, &str) = ("off", "#menu_opt_alt_02");
const PLAYERS_ONLY: (&str, &str) = ("players_only", "#menu_opt_alt_03");
const LOW: (&str, &str) = ("low", "#menu_opt_alt_04");
const MEDIUM: (&str, &str) = ("medium", "#menu_opt_alt_05");
const HIGH: (&str, &str) = ("high", "#menu_opt_alt_06");

/// ResolutionMessageBox's choices (the display modes of a common monitor).
const RESOLUTIONS: [[u32; 2]; 8] = [
    [800, 600],
    [960, 720],
    [1024, 768],
    [1280, 720],
    [1280, 960],
    [1366, 768],
    [1600, 900],
    [1920, 1080],
];

#[derive(Clone, Copy)]
enum Values {
    /// Settings values in ControllerRight's order, with their labels.
    Cycle(&'static [(&'static str, &'static str)]),
    /// "Windowed": On when not fullscreen.
    Windowed,
    Resolution,
    /// The current language's native name (only English is installed).
    Language,
}

struct Row {
    label: &'static str,
    key: &'static str,
    values: Values,
}

pub(crate) struct Spec {
    menu: MenuId,
    title: &'static str,
    /// settings.json section.
    section: &'static str,
    rows: &'static [Row],
    /// The Graphics page's stains (pages atlas only; the stains texture is
    /// a separate atlas).
    header: bool,
}

pub(crate) const GAME: Spec = Spec {
    menu: MenuId::GameOptions,
    title: "#menu_opt_12",
    section: "game",
    rows: &[
        Row {
            label: "#menu_opt_game_01",
            key: "gore",
            values: Values::Cycle(&[ON, OFF]),
        },
        Row {
            label: "#menu_opt_game_02",
            key: "damage_numbers",
            values: Values::Cycle(&[ON, OFF]),
        },
        Row {
            label: "#menu_opt_game_03",
            key: "health_bars",
            values: Values::Cycle(&[PLAYERS_ONLY, ON, OFF]),
        },
        Row {
            label: "#menu_opt_game_04",
            key: "spell_wheel",
            values: Values::Cycle(&[ON, OFF]),
        },
        Row {
            label: "#menu_opt_game_05",
            key: "language",
            values: Values::Language,
        },
    ],
    header: true,
};

pub(crate) const GRAPHICS: Spec = Spec {
    menu: MenuId::Graphics,
    title: "#menu_opt_10",
    section: "graphics",
    rows: &[
        Row {
            label: "#menu_opt_gfx_01",
            key: "fullscreen",
            values: Values::Windowed,
        },
        Row {
            label: "#menu_opt_gfx_02",
            key: "resolution",
            values: Values::Resolution,
        },
        Row {
            label: "#menu_opt_gfx_04",
            key: "shadows",
            values: Values::Cycle(&[MEDIUM, HIGH, LOW]),
        },
        Row {
            label: "#menu_opt_gfx_13",
            key: "decals",
            values: Values::Cycle(&[MEDIUM, HIGH, LOW]),
        },
        Row {
            label: "#menu_opt_gfx_particles",
            key: "particles",
            values: Values::Cycle(&[MEDIUM, HIGH, LOW]),
        },
        Row {
            label: "#menu_opt_gfx_particlelights",
            key: "particle_lights",
            values: Values::Cycle(&[ON, OFF]),
        },
        Row {
            label: "#menu_vsync",
            key: "vsync",
            values: Values::Cycle(&[ON, OFF]),
        },
    ],
    header: true,
};

fn setting<'a>(s: &'a Frontend, spec: &Spec, key: &str) -> Option<&'a Value> {
    s.settings.get(spec.section).and_then(|v| v.get(key))
}

/// The defaults GlobalSettings' constructor sets (Particles is not set
/// there; High is assumed).
fn default_value(key: &str) -> Value {
    match key {
        "fullscreen" => json!(false),
        "resolution" => json!([960, 720]),
        "shadows" | "decals" => json!("medium"),
        "particles" => json!("high"),
        _ => json!("on"),
    }
}

fn value(s: &Frontend, spec: &Spec, row: &Row) -> Value {
    setting(s, spec, row.key)
        .cloned()
        .unwrap_or_else(|| default_value(row.key))
}

fn value_text(s: &Frontend, spec: &Spec, row: &Row) -> String {
    let v = value(s, spec, row);
    match row.values {
        Values::Cycle(values) => values
            .iter()
            .find(|(name, _)| v.as_str() == Some(name))
            .map(|(_, label)| loc(s, label))
            .unwrap_or_default(),
        Values::Windowed => loc(
            s,
            if v.as_bool() == Some(true) {
                OFF.1
            } else {
                ON.1
            },
        ),
        Values::Resolution => {
            let [w, h] = serde_json::from_value::<[u32; 2]>(v).unwrap_or([960, 720]);
            format!("{w} x {h}")
        }
        Values::Language => "English".into(),
    }
}

/// ControllerRight (`step` 1) / ControllerLeft (`step` -1).
fn cycle(s: &mut Frontend, spec: &Spec, row: &Row, step: isize) {
    let current = value(s, spec, row);
    let next = match row.values {
        Values::Cycle(values) => {
            let i = values
                .iter()
                .position(|(name, _)| current.as_str() == Some(name))
                .unwrap_or(0) as isize;
            json!(values[(i + step).rem_euclid(values.len() as isize) as usize].0)
        }
        Values::Windowed => json!(current.as_bool() != Some(true)),
        Values::Resolution => {
            let size = serde_json::from_value::<[u32; 2]>(current).unwrap_or([960, 720]);
            let i = RESOLUTIONS.iter().position(|r| *r == size).unwrap_or(1) as isize;
            json!(RESOLUTIONS[(i + step).rem_euclid(RESOLUTIONS.len() as isize) as usize])
        }
        Values::Language => return,
    };
    if !s.settings[spec.section].is_object() {
        s.settings[spec.section] = json!({});
    }
    s.settings[spec.section][row.key] = next;
}

#[derive(Default)]
pub(crate) struct SettingsPage {
    list: List,
    entered: Value,
}

impl SettingsPage {
    pub(crate) fn enter(&mut self, s: &Frontend, spec: &Spec) {
        self.list.selected = Some(0);
        self.entered = s.settings.get(spec.section).cloned().unwrap_or(Value::Null);
    }
    /// OnExit: save when anything changed.
    fn exit(&self, s: &mut Frontend, spec: &Spec) {
        if s.settings.get(spec.section).cloned().unwrap_or(Value::Null) != self.entered {
            if let Err(e) = s.store.save_settings(&s.settings) {
                s.message = e;
            }
        }
    }
    fn layout(&self, s: &Frontend, spec: &Spec, measure: &mut Measure) -> (Vec<Widget>, Vec<Item>) {
        let mut out = Vec::new();
        let mut items = Vec::new();
        let selected = self.list.selected;
        out.push(Widget::text(
            Font::MenuTitle,
            loc(s, spec.title),
            [512.0, 96.0],
            TextAlign::Center,
        ));
        let line = measure.line_height(Font::MenuOption) + 10.0;
        for (i, row) in spec.rows.iter().enumerate() {
            let y = 1024.0 * 0.333 + line * i as f32;
            let on = selected == Some(i);
            let label = menu_text_item(
                &mut out,
                measure,
                Font::MenuOption,
                loc(s, row.label),
                [512.0, y],
                TextAlign::Right,
                on,
                true,
            );
            let value = menu_text_item(
                &mut out,
                measure,
                Font::MenuOption,
                value_text(s, spec, row),
                [552.0, y],
                TextAlign::Left,
                on,
                true,
            );
            // Either the label or its value selects the row.
            items.push(Item::new(
                [
                    label.min[0].min(value.min[0]),
                    label.min[1].min(value.min[1]),
                ],
                [
                    label.max[0].max(value.max[0]),
                    label.max[1].max(value.max[1]),
                ],
                true,
            ));
        }
        out.push(back_button(selected == Some(spec.rows.len())));
        items.push(back_item());
        if spec.header {
            out.push(graphic(
                [448.0, 976.0, 608.0, 48.0],
                [208.0, 220.0, 608.0, 48.0],
            ));
        }
        (out, items)
    }
    pub(crate) fn update(
        &mut self,
        s: &mut Frontend,
        spec: &Spec,
        tome: &mut TomeView,
        input: &PageInput,
        measure: &mut Measure,
    ) {
        if input.back {
            self.exit(s, spec);
            tome.pop();
            return;
        }
        if let Some(i) = self.list.selected.filter(|i| *i < spec.rows.len()) {
            if input.direction[3] {
                cycle(s, spec, &spec.rows[i], 1);
            }
            if input.direction[2] {
                cycle(s, spec, &spec.rows[i], -1);
            }
        }
        let (_, items) = self.layout(s, spec, measure);
        match self.list.update(&items, input) {
            Some(i) if i == spec.rows.len() => {
                self.exit(s, spec);
                tome.pop();
            }
            Some(i) => cycle(s, spec, &spec.rows[i], 1),
            None => {}
        }
    }
    pub(crate) fn draw(
        &self,
        s: &Frontend,
        spec: &Spec,
        draw: &mut PageDraw,
        measure: &mut Measure,
    ) {
        let (out, _) = self.layout(s, spec, measure);
        let mut page = draw.right(spec.menu);
        for widget in out {
            page.push(widget);
        }
    }
}

/// GlobalSettings.Fullscreen/Resolution/VSync on the game window, whenever
/// they change (and once at start).
pub(crate) fn apply_window(
    state: Res<Frontend>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut applied: Local<Option<Value>>,
) {
    let graphics = state
        .settings
        .get("graphics")
        .cloned()
        .unwrap_or(Value::Null);
    if applied.as_ref() == Some(&graphics) {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    let fullscreen = graphics["fullscreen"].as_bool() == Some(true);
    window.mode = if fullscreen {
        WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    };
    if !fullscreen {
        if let Ok([w, h]) = serde_json::from_value::<[u32; 2]>(graphics["resolution"].clone()) {
            window.resolution.set(w as f32, h as f32);
        }
    }
    window.present_mode = if graphics["vsync"].as_str() == Some("off") {
        PresentMode::AutoNoVsync
    } else {
        PresentMode::AutoVsync
    };
    *applied = Some(graphics);
}
