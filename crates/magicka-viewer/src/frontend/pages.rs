//! Native submenus drawn on the Tome's pages: the generic SubMenu item list
//! and the original layouts of the campaign save-slot page and character
//! select. Coordinates are 1024-pixel right-page pixels as in the CIL.
use super::{
    book::{Atlas, Font, Measure, PageDraw, Widget},
    Frontend,
};
use crate::frontend_tome::{MenuId, PagePointer, TomeView};
use bevy::prelude::*;
use magicka_runtime::frontend_tome::TomeState;
use magicka_runtime::{
    menu_text_item::{MENU_COLOR, MENU_COLOR_DISABLED, MENU_COLOR_SELECTED},
    saves::new_player_id,
    scene_loading::custom_hash,
    text_geometry::TextAlign,
};
use serde_json::json;
use std::collections::HashMap;

/// Defines.DIALOGUE_COLOR_DEFAULT.
pub(crate) const DIALOGUE_COLOR: [f32; 4] = [0.7037, 0.5576, 0.3202, 1.0];
pub(crate) use crate::player_colors::{player_color, PLAYER_COLORS};

/// One frame of menu input in page terms (Tome.MousePickTome for pointers).
#[derive(Resource, Default)]
pub(crate) struct PageInput {
    /// The window cursor in pixels (message boxes are screen-space).
    pub screen_pointer: Option<[f32; 2]>,
    pub screen: [f32; 2],
    screen_last: Option<[f32; 2]>,
    pub screen_moved: bool,
    pub pointer: Option<[f32; 2]>,
    pub moved: bool,
    pub released: bool,
    pub pressed: bool,
    pub held: bool,
    pub confirm: bool,
    pub back: bool,
    /// Up, Down, Left, Right.
    pub direction: [bool; 4],
    last: Option<[f32; 2]>,
    /// Text typed this frame ('\u{8}' for Backspace), for text input boxes.
    pub typed: String,
    /// The first key or mouse button pressed this frame (key capture).
    pub captured: Option<crate::bindings::Binding>,
    /// Wheel steps this frame (positive up).
    pub wheel: f32,
    /// Every pad's menu edges (local co-op seats edit their own slots).
    pub pads: Vec<(Entity, crate::menu_input::PadEdges)>,
    /// The pad driving the shared menu, if any.
    pub controller: Option<Entity>,
    /// Connected pads (entity, device name) in a stable order.
    pub connected: Vec<(Entity, String)>,
}

pub(crate) fn collect_input(
    mut input: ResMut<PageInput>,
    (mut menu, frontend): (ResMut<crate::menu_input::MenuInput>, Res<Frontend>),
    mouse: Res<ButtonInput<MouseButton>>,
    pointer: Res<PagePointer>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut keys: MessageReader<bevy::input::keyboard::KeyboardInput>,
    (held_keys, scroll): (
        Res<ButtonInput<KeyCode>>,
        Res<bevy::input::mouse::AccumulatedMouseScroll>,
    ),
    gamepads: Query<(Entity, Option<&Name>), With<Gamepad>>,
) {
    let mut connected: Vec<(Entity, String)> = gamepads
        .iter()
        .map(|(e, name)| (e, name.map(|n| n.to_string()).unwrap_or_default()))
        .collect();
    connected.sort_by_key(|(e, _)| e.to_bits());
    input.connected = connected;
    input.wheel = scroll.delta.y;
    input.captured = crate::bindings::captured(&held_keys, &mouse, scroll.delta.y);
    use bevy::input::{keyboard::Key, ButtonState};
    input.typed.clear();
    for key in keys.read() {
        if key.state != ButtonState::Pressed {
            continue;
        }
        match &key.logical_key {
            Key::Character(text) => input.typed.push_str(text),
            Key::Space => input.typed.push(' '),
            Key::Backspace => input.typed.push('\u{8}'),
            _ => {}
        }
    }
    if let Ok(window) = windows.single() {
        input.screen = [window.width(), window.height()];
        let cursor = window.cursor_position().map(|c| [c.x, c.y]);
        // Movement within the window only: a cursor at the edge that
        // leaves and re-enters is not a mouse move (it would drop the
        // keyboard selection every frame).
        input.screen_moved = match (cursor, input.screen_last) {
            (Some(now), Some(before)) => now != before,
            _ => false,
        };
        if cursor.is_some() {
            input.screen_last = cursor;
        }
        input.screen_pointer = cursor;
    }
    let point = pointer.0.filter(|h| h.right_page).map(|h| h.position);
    input.moved = point != input.last;
    input.last = point;
    input.pointer = point;
    input.released = mouse.just_released(MouseButton::Left);
    input.pressed = mouse.just_pressed(MouseButton::Left);
    input.held = mouse.pressed(MouseButton::Left);
    input.confirm = menu.confirm;
    input.back = menu.back;
    input.direction = menu.direction;
    let mut pads: Vec<_> = menu.pads.iter().map(|(e, p)| (*e, *p)).collect();
    pads.sort_by_key(|(e, _)| e.to_bits());
    input.pads = pads;
    input.controller = menu.controller;
    // Lobby seats own their pads while the menu runs; play's local co-op
    // takes over once a session starts.
    if matches!(frontend.session, super::Session::Idle) {
        menu.seated = frontend.seated_pads.clone();
    }
}

/// A MenuItem's bounds and state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Item {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub enabled: bool,
}
impl Item {
    pub(crate) fn new(min: [f32; 2], max: [f32; 2], enabled: bool) -> Self {
        Self { min, max, enabled }
    }
    pub(crate) fn inside(&self, p: [f32; 2]) -> bool {
        p[0] >= self.min[0] && p[0] <= self.max[0] && p[1] >= self.min[1] && p[1] <= self.max[1]
    }
}

/// SubMenu's selection: wrapped Up/Down skipping disabled items, pointer
/// hover selects, a release over an item clicks it (ControllerMouseAction).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct List {
    pub selected: Option<usize>,
    /// This frame: the pointer moved the selection / an item was activated
    /// (ui_menu_mouseover / ui_menu_select).
    pub hovered: bool,
    pub activated: bool,
}
impl List {
    pub(crate) fn up(&mut self, items: &[Item]) {
        self.step(items, -1);
    }
    pub(crate) fn down(&mut self, items: &[Item]) {
        self.step(items, 1);
    }
    fn step(&mut self, items: &[Item], delta: isize) {
        if items.iter().all(|i| !i.enabled) {
            return;
        }
        let n = items.len() as isize;
        let mut i = self
            .selected
            .map_or(if delta > 0 { -1 } else { n }, |s| s as isize);
        loop {
            i = (i + delta).rem_euclid(n);
            if items[i as usize].enabled {
                break;
            }
        }
        self.selected = Some(i as usize);
    }
    /// Pointer handling; returns a clicked item.
    pub(crate) fn pointer(&mut self, items: &[Item], input: &PageInput) -> Option<usize> {
        let p = input.pointer?;
        // Screen cursor motion, not the picked point (the camera may move).
        if !(input.screen_moved || input.released) {
            return None;
        }
        let hit = items.iter().position(|i| i.enabled && i.inside(p))?;
        self.hovered |= self.selected != Some(hit);
        self.selected = Some(hit);
        input.released.then_some(hit)
    }
    /// The generic SubMenu frame: Up/Down, pointer, confirm.
    pub(crate) fn update(&mut self, items: &[Item], input: &PageInput) -> Option<usize> {
        let activated = self.navigate(items, input);
        self.activated = activated.is_some();
        activated
    }
    fn navigate(&mut self, items: &[Item], input: &PageInput) -> Option<usize> {
        if input.direction[0] {
            self.up(items);
        }
        if input.direction[1] {
            self.down(items);
        }
        if let Some(clicked) = self.pointer(items, input) {
            return Some(clicked);
        }
        input.confirm.then_some(self.selected).flatten()
    }
    /// Take this frame's cue requests.
    pub(crate) fn cues(&mut self, out: &mut Vec<&'static str>) {
        if std::mem::take(&mut self.activated) {
            out.push("ui_menu_select");
        } else if std::mem::take(&mut self.hovered) {
            out.push("ui_menu_mouseover");
        }
        self.hovered = false;
    }
}

pub(crate) fn text_color(selected: bool, enabled: bool) -> [f32; 4] {
    if !enabled {
        MENU_COLOR_DISABLED
    } else if selected {
        MENU_COLOR_SELECTED
    } else {
        MENU_COLOR
    }
}

/// SubMenu.BACK_*: the arrow at the bottom-left of the right page.
pub(crate) const BACK_POSITION: [f32; 2] = [96.0, 1024.0 - 96.0 - 16.0];
pub(crate) const BACK_SIZE: [f32; 2] = [320.0, 96.0];
pub(crate) fn back_item() -> Item {
    Item::new(
        BACK_POSITION,
        [
            BACK_POSITION[0] + BACK_SIZE[0],
            BACK_POSITION[1] + BACK_SIZE[1],
        ],
        true,
    )
}
/// MenuImageTextItem.Draw (no text): SelectedPower 1.5, saturations
/// 1/1.3/0 for normal/selected/disabled.
pub(crate) fn image_item(
    atlas: Atlas,
    source: [f32; 4],
    position: [f32; 2],
    size: [f32; 2],
    selected: bool,
    enabled: bool,
) -> Widget {
    let power = if selected { 1.5 } else { 1.0 };
    Widget::image(atlas, source, position, size)
        .color([power, power, power, 1.0])
        .saturation(if !enabled {
            0.0
        } else if selected {
            1.3
        } else {
            1.0
        })
}
pub(crate) fn back_button(selected: bool) -> Widget {
    image_item(
        Atlas::Pages,
        [
            2048.0 - BACK_SIZE[0] - 32.0,
            48.0,
            BACK_SIZE[0],
            BACK_SIZE[1],
        ],
        BACK_POSITION,
        BACK_SIZE,
        selected,
        true,
    )
}

/// MenuTextButtonItem with DEFAULT_UV_OFFSET/SIZE: left cap, stretched
/// middle and right cap around centred text. Returns its bounds.
#[allow(clippy::too_many_arguments)]
pub(crate) fn text_button(
    out: &mut Vec<Widget>,
    text: &str,
    text_size: [f32; 2],
    position: [f32; 2],
    align: TextAlign,
    min_width: f32,
    selected: bool,
    enabled: bool,
) -> Item {
    let size = [128.0, 64.0];
    let width = min_width.max(text_size[0] + size[0] * 0.5);
    let (left, middle) = match align {
        TextAlign::Left => (0.0, width * 0.5),
        TextAlign::Right => (-width, -width * 0.5),
        _ => (-width * 0.5, 0.0),
    };
    let saturation = if !enabled {
        0.0
    } else if selected {
        1.5
    } else {
        1.0
    };
    let top = position[1] - size[1] * 0.5;
    let x = position[0] + left;
    let cap = size[0] * 0.25;
    for (source_x, source_w, dest_x, dest_w) in [
        (768.0, cap, x, cap),
        (768.0 + cap, size[0] * 0.5, x + cap, width - size[0] * 0.5),
        (768.0 + cap * 3.0, cap, x + width - cap, cap),
    ] {
        out.push(
            Widget::image(
                Atlas::Pages,
                [source_x, 384.0, source_w, size[1]],
                [dest_x, top],
                [dest_w, size[1]],
            )
            .saturation(saturation),
        );
    }
    let color = if !enabled {
        DIALOGUE_COLOR
    } else if selected {
        MENU_COLOR_SELECTED
    } else {
        DIALOGUE_COLOR
    };
    out.push(
        Widget::text(
            Font::MenuOption,
            text,
            [position[0] + middle, position[1] - text_size[1] * 0.5],
            TextAlign::Center,
        )
        .color(color),
    );
    Item::new([x, top], [x + width, top + size[1]], enabled)
}

/// A nine-slice (TextBox-style) panel: 16-pixel margins from a 128-square
/// source, as SubMenuCharacterSelect.CreateVertices builds.
pub(crate) fn panel(
    out: &mut Vec<Widget>,
    position: [f32; 2],
    size: [f32; 2],
    color: [f32; 4],
    saturation: f32,
) {
    let m = 16.0;
    let (sx, sy, ss) = (832.0, 128.0, 128.0);
    let xs = [
        (0.0, m, sx, m),
        (m, size[0] - 2.0 * m, sx + m, ss - 2.0 * m),
        (size[0] - m, m, sx + ss - m, m),
    ];
    let ys = [
        (0.0, m, sy, m),
        (m, size[1] - 2.0 * m, sy + m, ss - 2.0 * m),
        (size[1] - m, m, sy + ss - m, m),
    ];
    for (dy, dh, uy, uh) in ys {
        for (dx, dw, ux, uw) in xs {
            out.push(
                Widget::image(
                    Atlas::Pages,
                    [ux, uy, uw, uh],
                    [position[0] + dx, position[1] + dy],
                    [dw, dh],
                )
                .color(color)
                .saturation(saturation),
            );
        }
    }
}

/// MenuTextItem: text whose position is its vertical centre; bounds at
/// least 20 pixels (MenuTextItem.UpdateBoundingBox).
#[allow(clippy::too_many_arguments)]
pub(crate) fn menu_text_item(
    out: &mut Vec<Widget>,
    measure: &mut Measure,
    font: Font,
    text: String,
    position: [f32; 2],
    align: TextAlign,
    selected: bool,
    enabled: bool,
) -> Item {
    let size = measure.text(font, &text);
    let line = measure.line_height(font);
    let [w, h] = [size[0].max(20.0), size[1].max(20.0)];
    let left = match align {
        TextAlign::Left => position[0],
        TextAlign::Right => position[0] - w,
        _ => position[0] - w * 0.5,
    };
    out.push(
        Widget::text(
            font,
            text,
            [position[0], position[1] - line * 0.5 + 0.5],
            align,
        )
        .color(text_color(selected, enabled)),
    );
    Item::new(
        [left, position[1] - h * 0.5],
        [left + w, position[1] + h * 0.5],
        enabled,
    )
}

/// SubMenu.DrawGraphics: an atlas rectangle drawn into a page rectangle.
pub(crate) fn graphic(source: [f32; 4], dest: [f32; 4]) -> Widget {
    Widget::image(Atlas::Pages, source, [dest[0], dest[1]], [dest[2], dest[3]])
}

/// MenuScrollSlider of `width` (64-pixel ends) centred on `position`.
pub(crate) struct Slider {
    pub position: [f32; 2],
    pub width: f32,
    pub max: i32,
    pub scale: f32,
}
impl Slider {
    fn inner(&self) -> f32 {
        self.width - 128.0
    }
    fn step(&self) -> f32 {
        self.inner() / self.max.max(1) as f32
    }
    fn left(&self) -> f32 {
        self.position[0] - self.width * 0.5 * self.scale
    }
    pub(crate) fn bounds(&self) -> Item {
        let h = 32.0 * self.scale;
        Item::new(
            [self.left(), self.position[1] - h],
            [
                self.position[0] + self.width * 0.5 * self.scale,
                self.position[1] + h,
            ],
            true,
        )
    }
    /// InsideLeftBounds / InsideRightBounds: the 64-pixel arrow ends.
    pub(crate) fn end(&self, p: [f32; 2]) -> Option<i32> {
        let b = self.bounds();
        if !b.inside(p) {
            return None;
        }
        if p[0] <= b.min[0] + 64.0 * self.scale {
            Some(-1)
        } else if p[0] >= b.max[0] - 64.0 * self.scale {
            Some(1)
        } else {
            None
        }
    }
    /// ScrollTo.
    pub(crate) fn value_at(&self, x: f32) -> i32 {
        let offset = (x - self.left() - 64.0 * self.scale) / self.scale;
        ((0.5 + offset / self.step()) as i32).clamp(0, self.max)
    }
    pub(crate) fn draw(&self, out: &mut Vec<Widget>, value: i32) {
        let s = self.scale;
        let w = self.inner();
        let [x, y] = self.position;
        let right = self.position[0] + self.width * 0.5 * s;
        let sprite = |source: [f32; 4], half: [f32; 2], m: [f32; 4], at: [f32; 2]| {
            Widget::sprite(Atlas::Pages, source, half, m, at)
        };
        // Track, its two caps, both arrows and the knob, in native order.
        out.push(sprite(
            [1216.0, 64.0, 64.0, 64.0],
            [32.0, 32.0],
            [0.0, -s, s * (w - 32.0) / 64.0, 0.0],
            [x, y],
        ));
        out.push(sprite(
            [1216.0, 32.0, 64.0, 32.0],
            [32.0, 16.0],
            [0.0, -s, s, 0.0],
            [self.left() + 64.0 * s, y],
        ));
        out.push(sprite(
            [1216.0, 128.0, 64.0, 32.0],
            [32.0, 16.0],
            [0.0, -s, s, 0.0],
            [right - 64.0 * s, y],
        ));
        out.push(sprite(
            [1280.0, 96.0, 64.0, 64.0],
            [32.0, 32.0],
            [0.0, s, -s, 0.0],
            [x + (32.0 + w * 0.5) * s, y],
        ));
        out.push(sprite(
            [1280.0, 96.0, 64.0, 64.0],
            [32.0, 32.0],
            [0.0, -s, s, 0.0],
            [x - (32.0 + w * 0.5) * s, y],
        ));
        out.push(sprite(
            [1280.0, 32.0, 64.0, 64.0],
            [32.0, 32.0],
            [0.0, s, -1.0, 0.0],
            [self.left() + (64.0 + value as f32 * self.step()) * s, y],
        ));
    }
}

pub(crate) fn loc(s: &Frontend, key: &str) -> String {
    s.strings
        .get_string(custom_hash(&key.to_lowercase()))
        .unwrap_or_else(|| key.to_string())
}

/// A level file's Name and Description keys (LevelNode).
pub(crate) fn level_keys(content: &str, level: &str) -> Option<(String, String)> {
    let path = magicka_runtime::play_content::resolve(
        std::path::Path::new(content),
        &std::path::Path::new("Levels").join(level),
    )?;
    let xml = std::fs::read_to_string(path).ok()?;
    let tag = |name: &str| {
        let open = format!("<{name}>");
        let start = xml.find(&open)? + open.len();
        let end = xml[start..].find(&format!("</{name}>"))? + start;
        Some(xml[start..end].trim().to_string())
    };
    Some((tag("Name")?, tag("Description")?))
}

// ---------------------------------------------------------------------------
// OptionsMessageBox (screen space, above the Tome)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// SubMenuMain.ShowRUSure.
    Quit,
    /// A service or mode this build does not provide.
    Notice,
    DeleteSlot(u8),
    /// SubMenuOnline's manual address (TextInputMessageBox): host[:port].
    JoinAddress,
    /// Rewrite extension: the passphrase of a private game.
    Passphrase,
    /// Rewrite extension: name a profile (`created`: a new, unsaved one).
    ProfileName {
        profile: usize,
        created: bool,
    },
}

/// The name rules the host applies to profiles (`PlayerProfile::valid`).
pub(crate) fn clean_profile_name(text: &str) -> Result<String, &'static str> {
    let name = text.trim();
    if name.is_empty() {
        Err("A profile needs a name.")
    } else if name.chars().count() > 32 {
        Err("Profile names are at most 32 characters.")
    } else if name.chars().any(char::is_control) {
        Err("Profile names cannot contain control characters.")
    } else {
        Ok(name.to_string())
    }
}

pub(crate) struct MessageBox {
    pub purpose: Purpose,
    message: String,
    options: Vec<String>,
    selected: usize,
    keyboard: bool,
    /// The pointer rests on the selected option (mouse selection).
    hovered: bool,
    alpha: f32,
    killed: bool,
    /// TextInputMessageBox: the edited line, drawn under the message.
    pub input: Option<String>,
    /// Seconds open (text caret blink).
    age: f32,
}
impl MessageBox {
    pub(crate) fn new(purpose: Purpose, message: String, options: Vec<String>) -> Self {
        Self {
            purpose,
            message,
            options,
            selected: 0,
            keyboard: false,
            hovered: false,
            alpha: 0.0,
            killed: false,
            input: None,
            age: 0.0,
        }
    }
    pub(crate) fn text_input(
        purpose: Purpose,
        message: String,
        text: String,
        options: Vec<String>,
    ) -> Self {
        let mut popup = Self::new(purpose, message, options);
        popup.input = Some(text);
        popup
    }
    pub(crate) fn keyboard_select(&mut self, index: usize) {
        self.selected = index;
        self.keyboard = true;
    }
    /// MessageBox.Dead: killed and faded out.
    pub(crate) fn dead(&self) -> bool {
        self.killed && self.alpha <= 0.0
    }
    fn layout(&self, screen: [f32; 2], measure: &mut Measure) -> (String, f32, f32, Vec<[f32; 4]>) {
        // MessageBox: MenuDefault wrapped to 90% of the 448-pixel box.
        let wrapped = measure.assets.wrap(
            Font::MenuDefault,
            &self.message,
            (448.0 * 0.9) as i32,
            measure.images,
        );
        let height = measure.text(Font::MenuDefault, &wrapped)[1].max(1.0);
        let line = measure.line_height(Font::MenuDefault);
        let shown = height.min(460.0);
        let center = [screen[0] * 0.5, screen[1] * 0.5];
        // A text box takes one line between the message and the options.
        let field = if self.input.is_some() {
            1.5 * line
        } else {
            0.0
        };
        let top = center[1] - 0.5 * (shown + field + line * self.options.len() as f32);
        // MenuTextItem positions are vertical centres.
        let mut y = top + shown + field + 0.5 * line;
        let mut bounds = Vec::new();
        for option in &self.options {
            let size = measure.text(Font::MenuDefault, option);
            let [w, h] = [size[0].max(20.0), size[1].max(20.0)];
            bounds.push([
                center[0] - w * 0.5,
                y - h * 0.5,
                center[0] + w * 0.5,
                y + h * 0.5,
            ]);
            y += line;
        }
        (wrapped, top, shown / height, bounds)
    }
    /// OnMove/OnMouseMove/OnMouseClick/OnSelect; Esc kills without a choice.
    pub(crate) fn update(
        &mut self,
        input: &PageInput,
        dt: f32,
        measure: &mut Measure,
    ) -> Option<usize> {
        self.age += dt;
        self.alpha = if self.killed {
            (self.alpha - dt * 4.0).max(0.0)
        } else {
            (self.alpha + dt * 4.0).min(1.0)
        };
        if self.killed {
            return None;
        }
        if input.direction[0] || input.direction[1] {
            if !self.keyboard {
                self.selected = 0;
                self.keyboard = true;
            } else {
                let n = self.options.len();
                self.selected = if input.direction[0] {
                    (self.selected + n - 1) % n
                } else {
                    (self.selected + 1) % n
                };
            }
        }
        let (_, _, _, bounds) = self.layout(input.screen, measure);
        if let Some(p) = input.screen_pointer {
            if input.screen_moved || input.released {
                let hit = bounds
                    .iter()
                    .position(|b| p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]);
                self.hovered = hit.is_some();
                if let Some(i) = hit {
                    self.keyboard = false;
                    self.selected = i;
                    if input.released {
                        self.killed = true;
                        return Some(i);
                    }
                }
            }
        }
        if input.back {
            self.killed = true;
            return None;
        }
        // Text boxes: typed characters and Backspace edit the entry.
        if let Some(text) = self.input.as_mut() {
            for c in input.typed.chars() {
                match c {
                    '\u{8}' => {
                        text.pop();
                    }
                    c if !c.is_control() && c != ' ' && text.chars().count() < 64 => text.push(c),
                    _ => {}
                }
            }
        }
        // In a text box Space types instead of confirming.
        if input.confirm && !(self.input.is_some() && input.typed.contains(' ')) {
            self.killed = true;
            return Some(self.selected);
        }
        None
    }
    pub(crate) fn draw(&self, input: &PageInput, draw: &mut PageDraw, measure: &mut Measure) {
        let (wrapped, top, squash, bounds) = self.layout(input.screen, measure);
        let shown_height = measure.text(Font::MenuDefault, &wrapped)[1].min(460.0);
        let center = [input.screen[0] * 0.5, input.screen[1] * 0.5];
        let mut screen = draw.screen();
        screen.push(
            Widget::image(
                Atlas::Pages,
                [0.0, 464.0, 448.0, 560.0],
                [center[0] - 224.0, center[1] - 280.0],
                [448.0, 560.0],
            )
            .color([1.0, 1.0, 1.0, self.alpha]),
        );
        let mut message = Widget::text(
            Font::MenuDefault,
            wrapped,
            [center[0], top],
            TextAlign::Center,
        );
        message.color[3] *= self.alpha;
        message.scale = squash.min(1.0);
        screen.push(message);
        let line = measure.line_height(Font::MenuDefault);
        if let Some(text) = &self.input {
            // The entry on a darkened strip, with a blinking caret.
            let y = top + shown_height + 0.25 * line;
            screen.push(Widget::fill(
                [center[0] - 180.0, y - 4.0],
                [360.0, line + 8.0],
                [0.0, 0.0, 0.0, 0.35 * self.alpha],
            ));
            let caret = if ((self.age * 2.0) as u32).is_multiple_of(2) {
                "_"
            } else {
                " "
            };
            // A passphrase is not shown to whoever is watching the screen.
            let shown = if matches!(self.purpose, Purpose::Passphrase) {
                "*".repeat(text.chars().count())
            } else {
                text.clone()
            };
            let mut entry = Widget::text(
                Font::MenuDefault,
                format!("{shown}{caret}"),
                [center[0], y],
                TextAlign::Center,
            )
            .color(text_color(true, true));
            entry.color[3] *= self.alpha;
            screen.push(entry);
        }
        for (i, (option, b)) in self.options.iter().zip(&bounds).enumerate() {
            // Without keyboard selection only a hovered option highlights.
            let mut color = text_color(i == self.selected && (self.keyboard || self.hovered), true);
            color[3] *= self.alpha;
            let y = (b[1] + b[3]) * 0.5 - line * 0.5 + 0.5;
            screen.push(
                Widget::text(
                    Font::MenuDefault,
                    option.clone(),
                    [center[0], y],
                    TextAlign::Center,
                )
                .color(color),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// SubMenuCampaignSelect_SaveSlotSelect

/// The original page has three campaign slots.
pub(crate) const PAGE_SLOTS: u8 = 3;

/// The Mythos campaign keeps its own three saves after the campaign's
/// (SaveManager's separate Mythos slots).
pub(crate) fn slot_base(s: &Frontend) -> u8 {
    match s.mode {
        super::GameMode::Mythos => PAGE_SLOTS + 1,
        _ => 0,
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SlotInfo {
    /// Chapter name key and description key; None for a new campaign.
    pub chapter: Option<(String, String)>,
    pub level: Option<String>,
    pub play_time: u32,
    pub magicks: u32,
    pub player_id: Option<String>,
}

#[derive(Default)]
pub(crate) struct SaveSlotsPage {
    list: List,
    pub slots: Vec<SlotInfo>,
}

impl SaveSlotsPage {
    /// OnEnter: select the first slot and re-read the saves.
    pub(crate) fn enter(&mut self, s: &Frontend) {
        self.list.selected = Some(3);
        let base = slot_base(s);
        self.slots = (0..PAGE_SLOTS)
            .map(|slot| match s.store.load_slot(base + slot) {
                Ok(Some(v)) => {
                    let level = v["level"].as_str().map(str::to_owned);
                    SlotInfo {
                        chapter: level.as_deref().and_then(|l| level_keys(&s.content, l)),
                        level,
                        play_time: v["play_time"].as_f64().unwrap_or(0.0) as u32,
                        magicks: v["players"][0]["magicks"]
                            .as_array()
                            .map_or(0, |m| m.len() as u32),
                        player_id: v["players"][0]["id"].as_str().map(str::to_owned),
                    }
                }
                _ => SlotInfo::default(),
            })
            .collect();
    }
    fn center(slot: usize) -> [f32; 2] {
        [512.0, 192.0 + 256.0 * slot as f32]
    }
    fn items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        for i in 0..3 {
            // The delete buttons follow Update's placement.
            let [x, y] = Self::center(i);
            let enabled = self.slots.get(i).is_some_and(|s| s.chapter.is_some());
            items.push(Item::new(
                [x + 340.0, y + 44.0],
                [x + 404.0, y + 108.0],
                enabled,
            ));
        }
        for i in 0..3 {
            let [x, y] = Self::center(i);
            items.push(Item::new(
                [x - 384.0, y - 128.0],
                [x + 384.0, y + 128.0],
                true,
            ));
        }
        items.push(back_item());
        items
    }

    pub(crate) fn update(&mut self, s: &mut Frontend, tome: &mut TomeView, input: &PageInput) {
        let items = self.items();
        // ControllerLeft/Right swap between a slot and its delete button.
        if input.direction[2] || input.direction[3] {
            self.list.selected = Some(match self.list.selected {
                Some(i @ 0..=2) => i + 3,
                Some(i @ 3..=5) if items[i - 3].enabled => i - 3,
                Some(i @ 3..=5) => i,
                _ => items.len() - 1,
            });
        }
        if input.back {
            tome.pop();
            return;
        }
        let Some(activated) = self.list.update(&items, input) else {
            return;
        };
        match activated {
            0..=2 => {
                if self.slots[activated].chapter.is_some() {
                    // mRUSure: "#add_menu_rus_del" with No preselected.
                    let mut popup = MessageBox::new(
                        Purpose::DeleteSlot(slot_base(s) + activated as u8),
                        loc(s, "#add_menu_rus_del"),
                        vec![loc(s, "#add_menu_yes"), loc(s, "#add_menu_no")],
                    );
                    popup.keyboard_select(1);
                    s.popup = Some(popup);
                }
            }
            3..=5 => {
                let slot = activated - 3;
                let info = &self.slots[slot];
                s.slot = slot_base(s) + slot as u8;
                s.fresh = info.chapter.is_none();
                s.campaign = info.level.clone().unwrap_or_else(|| {
                    match s.mode {
                        super::GameMode::Mythos => magicka_runtime::end_game::MYTHOS_CAMPAIGN[0],
                        _ => magicka_runtime::end_game::VANILLA_CAMPAIGN[0],
                    }
                    .into()
                });
                if let Some(id) = &info.player_id {
                    if let Some(i) = s.settings["profiles"]
                        .as_array()
                        .and_then(|p| p.iter().position(|p| p["id"] == json!(id)))
                    {
                        s.profile = i;
                    }
                }
                tome.push(MenuId::Character, 1);
            }
            _ => tome.pop(),
        }
    }

    pub(crate) fn draw(&self, s: &Frontend, draw: &mut PageDraw, _measure: &mut Measure) {
        let mut out = Vec::new();
        let selected = self.list.selected;
        for (i, slot) in self.slots.iter().enumerate() {
            let [x, y] = Self::center(i);
            let on = selected == Some(i + 3);
            let empty = slot.chapter.is_none();
            let power = if on { 1.1 } else { 1.0 };
            out.push(
                Widget::image(
                    Atlas::Pages,
                    [0.0, 0.0, 784.0, 240.0],
                    [x - 384.0, y - 128.0],
                    [784.0, 240.0],
                )
                .color([power, power, power, 1.0])
                .saturation(if empty {
                    0.0
                } else if on {
                    1.2
                } else {
                    1.0
                }),
            );
            let color = if on { MENU_COLOR_SELECTED } else { MENU_COLOR };
            if let Some((name, description)) = &slot.chapter {
                out.push(Widget::image(
                    Atlas::Pages,
                    [1344.0, 96.0, 64.0, 64.0],
                    [x - 288.0, y + 22.0],
                    [64.0, 64.0],
                ));
                // The clock icon draws with Saturation = looped (never set here).
                out.push(
                    Widget::image(
                        Atlas::Pages,
                        [1408.0, 96.0, 64.0, 64.0],
                        [x - 96.0, y + 22.0],
                        [64.0, 64.0],
                    )
                    .saturation(0.0),
                );
                out.push(
                    Widget::text(
                        Font::MenuOption,
                        format!("{}/22", slot.magicks),
                        [x - 208.0, y + 40.0],
                        TextAlign::Left,
                    )
                    .color(color),
                );
                let t = slot.play_time.min(359_999);
                out.push(
                    Widget::text(
                        Font::MenuOption,
                        // SaveSlot.Set → GetPlayTime (':' separators).
                        format!("{:02}:{:02}:{:02}", t / 3600 % 60, t / 60 % 60, t % 60),
                        [x - 32.0, y + 40.0],
                        TextAlign::Left,
                    )
                    .color(color),
                );
                out.push(
                    Widget::text(
                        Font::MenuOption,
                        loc(s, description),
                        [x - 224.0, y - 12.0],
                        TextAlign::Left,
                    )
                    .color(color),
                );
                out.push(
                    Widget::text(
                        Font::MenuOption,
                        format!("{}: ", loc(s, name)),
                        [x - 288.0, y - 52.0],
                        TextAlign::Left,
                    )
                    .color(color),
                );
            } else {
                out.push(
                    Widget::text(
                        Font::MenuOption,
                        loc(s, "#add_menu_newcamp"),
                        [x - 288.0, y - 52.0],
                        TextAlign::Left,
                    )
                    .color(color),
                );
            }
        }
        out.push(back_button(selected == Some(6)));
        for i in 0..3 {
            if self.slots.get(i).is_some_and(|s| s.chapter.is_some()) {
                let [x, y] = Self::center(i);
                out.push(image_item(
                    Atlas::Pages,
                    [1536.0, 96.0, 64.0, 64.0],
                    [x + 340.0, y + 44.0],
                    [64.0, 64.0],
                    selected == Some(i),
                    true,
                ));
            }
        }
        let mut page = draw.right(MenuId::SaveSlot);
        for widget in out {
            page.push(widget);
        }
    }
}

// ---------------------------------------------------------------------------
// SubMenuCharacterSelect (campaign, local players)

/// A playable avatar (Data/Characters/Playable/*.xml with allowCampaign).
#[derive(Clone, Debug)]
pub(crate) struct Avatar {
    /// The character template, used as the robe.
    pub character: String,
    pub thumb: String,
}

pub(crate) fn campaign_avatars(content: &str, robes: &[String]) -> Vec<Avatar> {
    // Profile's default avatar is the plain wizard.
    let mut avatars = vec![Avatar {
        character: "Wizard".into(),
        thumb: "Models/Characters/Wizard/Thumb".into(),
    }];
    let dir = std::path::Path::new(content).join("Data/Characters/Playable");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xml")))
        .collect();
    files.sort_by_key(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()));
    for path in files {
        let Ok(xml) = std::fs::read_to_string(&path) else {
            continue;
        };
        let tag = |name: &str| {
            let lower = xml.to_lowercase();
            let open = format!("<{}>", name.to_lowercase());
            let start = lower.find(&open)? + open.len();
            let end = lower[start..].find('<')? + start;
            Some(xml[start..end].trim().to_string())
        };
        if tag("allowCampaign").as_deref() != Some("true") {
            continue;
        }
        let (Some(character), Some(thumb)) = (tag("character"), tag("thumb")) else {
            continue;
        };
        // Play needs the converted robe model.
        if robes.iter().any(|r| r.eq_ignore_ascii_case(&character)) {
            avatars.push(Avatar { character, thumb });
        }
    }
    avatars
}

/// SubMenuCharacterSelect.GamerState for the local player's slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum SlotState {
    #[default]
    Ready,
    ChoosingGamer,
    CustomizingAvatar,
    CustomizingColor,
}

/// One slot's editor: its GamerState and the highlighted/scrolled choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SlotEditor {
    pub state: SlotState,
    pub item: usize,
    pub scroll: usize,
}

/// An extra local player seated with a pad (rewrite extension: the original
/// lobby had four slots; any number of pads can join here).
#[derive(Clone, Debug)]
pub(crate) struct LocalSeat {
    pub pad: Entity,
    pub profile: usize,
    pub editor: SlotEditor,
}

/// The lobby rows shown on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKind {
    Primary,
    Seat(usize),
    Join,
}
#[derive(Clone, Copy, Debug)]
struct Row {
    kind: RowKind,
    x: f32,
    y: f32,
    height: f32,
}

/// Lobby options (rewrite extension) shown under the map preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setting {
    Players,
    HotJoin,
    BossScale,
    Spectators,
    DungeonMasters,
    Invaders,
    /// Keyboard and mouse join (as player one) or leave the lobby.
    KeyboardSeat,

    /// Edit the selected profile's display name.
    Rename,
}

#[derive(Default)]
pub(crate) struct CharacterPage {
    list: List,
    /// The selection before a new profile was started (restored on cancel).
    prior_profile: usize,
    pub avatars: Vec<Avatar>,
    pub state: SlotState,
    /// The item highlighted inside a customisation row.
    pub item: usize,
    /// Visible choices stay fixed while hovering; only navigation scrolls them.
    scroll: usize,
    chapter: Option<(String, String)>,
    /// The map preview's TextureScale (xy) and TextureOffset (zw).
    preview: Option<[f32; 4]>,
    /// Pads seated as further local players.
    pub seats: Vec<LocalSeat>,
    pub lobby: magicka_network::lobby::LobbySettings,
    /// First visible row of the compact (more than four rows) layout.
    first_row: usize,
    grabbed: bool,
    /// PackMan's "Allowed drops" of a Challenge/Versus lobby.
    packs: Option<super::pack_panel::PackPanel>,
    /// Player one's device: None for keyboard and mouse, else a pad
    /// (rewrite extension; handed to play as `--local-primary`).
    primary_pad: Option<Entity>,
    /// `primary_pad` was chosen for this visit (first update: the pad then
    /// driving the menu, else keyboard and mouse).
    primary_decided: bool,
    /// Player one's slot is taken (the lobby may be empty).
    primary_present: bool,
    /// Player one's role badge: a player, or (online) a role without a
    /// wizard. Extra local seats play.
    primary_role: magicka_network::lobby::Role,
    /// Connected pads (entity, device name), refreshed every update.
    pads: Vec<(Entity, String)>,
    /// UIControllers sprites by name (keyboard, gp1..gp4).
    controller_sprites: HashMap<String, [f32; 4]>,
}

const SLOT_X: f32 = 544.0;
const SLOT_Y: f32 = 89.0;
const SLOT_STEP: f32 = 152.0;
const START: [f32; 2] = [992.0, BACK_POSITION[1] + 40.0];
/// Compact rows: the same nine-slice panel at half height, shifted left so
/// the original MenuScrollBar fits beside them.
const COMPACT_X: f32 = 512.0;
const COMPACT_H: f32 = 56.0;
const ROW_GAP: f32 = 8.0;
const WINDOW_BOTTOM: f32 = SLOT_Y + 3.0 * SLOT_STEP + 112.0;
const BAR_X: f32 = COMPACT_X + 448.0 + 34.0;
const SETTINGS: [f32; 2] = [64.0, 712.0];
const SETTINGS_STEP: f32 = 36.0;
/// Player-limit choices: the original four first among the small counts, up to
/// `IN_GAME_PLAYER_LIMIT`. Larger games are a dedicated server's `--lobby` option.
const LIMITS: [u16; 8] = [1, 2, 3, 4, 6, 8, 12, 16];
/// Items: the primary slot, three pad rows, Start, Back, then the pack row
/// of a Challenge/Versus lobby (native position 5) and the settings.
const PACK_ITEM: usize = 6;

/// The chosen server's LAN beacon, while it is heard.
fn join_advert(s: &Frontend) -> Option<&magicka_network::discovery::Advert> {
    let entry = s.join_entry.as_deref()?;
    let wanted: Option<std::net::SocketAddr> = entry.parse().ok();
    s.lan_games
        .iter()
        // The same address, or this machine's game heard over loopback.
        .find(|g| {
            g.address.to_string() == entry
                || wanted.is_some_and(|w| {
                    w.port() == g.address.port()
                        && (g.address.ip().is_loopback() || w.ip().is_loopback())
                })
        })
        .map(|g| &g.advert)
}

/// Lines describing the chosen server (left page of the join lobby).
fn server_lines(s: &Frontend) -> Vec<String> {
    use magicka_network::lobby::Role;
    let Some(a) = join_advert(s) else {
        return vec![
            "Server details show for games heard on the LAN.".into(),
            "The host admits you when you press Join.".into(),
        ];
    };
    let allowed: Vec<&str> = [
        (a.lobby.spectators, "spectators"),
        (a.lobby.dungeon_masters, "dungeon masters"),
        (a.lobby.invaders, "invaders"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    let mut lines = vec![
        format!("Level: {}", a.level.trim_end_matches(".lvl")),
        format!(
            "Players {}/{}, {} open{}",
            a.players,
            a.lobby.max_players,
            a.open_slots(),
            if a.open_slots() == 0 {
                " (players queue as spectators)"
            } else {
                ""
            }
        ),
        format!(
            "Also allows: {}",
            if allowed.is_empty() {
                "players only".to_string()
            } else {
                allowed.join(", ")
            }
        ),
        format!(
            "Join in progress: {}",
            if a.lobby.hot_join { "open" } else { "locked" }
        ),
    ];
    if !a.participants.is_empty() {
        lines.push(String::new());
        lines.push("In the game:".into());
        for p in &a.participants {
            lines.push(format!(
                "{}  -  {}{}",
                p.name,
                p.role.label(),
                if p.queued && p.role != Role::Player {
                    " (waiting for a slot)"
                } else {
                    ""
                }
            ));
        }
    }
    lines
}

/// The join lobby's server description in place of the chapter and map.
fn server_panel(s: &Frontend, out: &mut Vec<Widget>) {
    let title = join_advert(s)
        .map(|a| a.name.clone())
        .or_else(|| s.join_entry.clone())
        .unwrap_or_default();
    // Long names and raw addresses shrink to the left page's width.
    let chars = title.chars().count().max(1) as f32;
    let scale = (14.0 / chars).clamp(0.5, 1.0);
    out.push(Widget::text(Font::MenuTitle, title, [288.0, 96.0], TextAlign::Center).scale(scale));
    // Lines between the title and the settings rows.
    let lines = server_lines(s);
    let step = 30.0f32.min(470.0 / lines.len().max(1) as f32);
    for (i, line) in lines.into_iter().enumerate() {
        out.push(
            Widget::text(
                Font::MenuOption,
                line,
                [64.0, 170.0 + step * i as f32],
                TextAlign::Left,
            )
            .color(MENU_COLOR)
            .scale(0.7),
        );
    }
}

/// The next (or previous) entry after `current` in `options`, wrapping.
fn step_choice<T: PartialEq + Clone>(options: &[T], current: &T, forward: bool) -> T {
    let n = options.len();
    match options.iter().position(|o| o == current) {
        Some(i) if n > 0 => options[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
        .clone(),
        _ => options.first().cloned().unwrap_or_else(|| current.clone()),
    }
}

/// Settings row spacing: the original 36 px while the rows fit above Back.
fn settings_step(rows: usize) -> f32 {
    let room = BACK_POSITION[1] - 16.0 - SETTINGS[1];
    if rows <= 1 {
        SETTINGS_STEP
    } else {
        SETTINGS_STEP.min(room / (rows - 1) as f32)
    }
}

/// UIControllers.json (Content/UI/Spritesheets): sprite rectangles by name.
fn controller_sprites(content: &str) -> HashMap<String, [f32; 4]> {
    let path = std::path::Path::new(content).join("UI/Spritesheets/UIControllers.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return HashMap::new();
    };
    json["sprites"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|sprite| {
            let rect: Vec<f32> = sprite["rect"]
                .as_str()?
                .split_whitespace()
                .filter_map(|v| v.parse().ok())
                .collect();
            let rect: [f32; 4] = rect.try_into().ok()?;
            Some((sprite["name"].as_str()?.to_string(), rect))
        })
        .collect()
}

/// Centres of the two rows of swatches drawn in a slot at `(x, y)`.
fn color_position_at(index: usize, x: f32, y: f32) -> [f32; 2] {
    let half = PLAYER_COLORS.len() / 2;
    let row = usize::from(index > half);
    let column = if row == 0 { index } else { index - half - 1 };
    [
        x + 448.0 - 56.0 - (half as f32 - 0.5) * 34.0 + column as f32 * 34.0,
        y + 56.0 - 17.0 + row as f32 * 34.0,
    ]
}

/// `--lobby` launch spec for the settings.
pub(crate) fn lobby_spec(l: &magicka_network::lobby::LobbySettings) -> String {
    let on = |b: bool| if b { "on" } else { "off" };
    format!(
        "players={},hotjoin={},spectators={},dm={},invaders={},respawn={},bossscale={}",
        l.max_players,
        on(l.hot_join),
        on(l.spectators),
        on(l.dungeon_masters),
        on(l.invaders),
        on(l.debug_respawn),
        on(l.boss_scale)
    )
}

impl CharacterPage {
    fn settings_key(s: &Frontend) -> &'static str {
        if s.mode == super::GameMode::Versus {
            "lobby_versus"
        } else {
            "lobby"
        }
    }
    pub(crate) fn enter(&mut self, s: &Frontend) {
        if self.avatars.is_empty() {
            self.avatars = campaign_avatars(&s.content, &s.robes);
        }
        if self.controller_sprites.is_empty() {
            self.controller_sprites = controller_sprites(&s.content);
        }
        self.primary_decided = false;
        self.primary_role = if s.joining.is_some() {
            s.join_role
        } else {
            magicka_network::lobby::Role::Player
        };
        self.state = SlotState::Ready;
        self.scroll = 0;
        self.first_row = 0;
        self.list.selected = Some(4);
        use magicka_network::lobby::LobbySettings;
        let base = if s.mode == super::GameMode::Versus {
            LobbySettings::versus()
        } else {
            LobbySettings::default()
        };
        self.lobby = s.settings[Self::settings_key(s)]
            .as_str()
            .and_then(|spec| base.parse_onto(spec).ok())
            .unwrap_or(base);
        // Settings saved when the menu offered more than it does now.
        self.lobby.max_players = self
            .lobby
            .max_players
            .min(magicka_network::lobby::IN_GAME_PLAYER_LIMIT);
        let profiles = s.settings["profiles"].as_array().map_or(0, Vec::len);
        self.seats.retain(|seat| seat.profile < profiles);
        if s.qa_seats > 0 && self.seats.is_empty() && profiles > 0 {
            let mut world = World::new();
            self.lobby.max_players = self.lobby.max_players.max(s.qa_seats as u16 + 2);
            self.seats = (0..s.qa_seats)
                .map(|i| LocalSeat {
                    pad: world.spawn_empty().id(),
                    profile: (i + 1) % profiles,
                    editor: SlotEditor::default(),
                })
                .collect();
        }
        for seat in &mut self.seats {
            seat.editor = SlotEditor::default();
        }
        self.packs = Self::lobby_packs(s);
        // Audit entry (`--menu-packs-open`): show the pack list at once.
        if std::env::args().any(|a| a == "--menu-packs-open") {
            if let Some(panel) = self.packs.as_mut() {
                panel.open();
            }
        }
        self.chapter = level_keys(&s.content, &s.campaign);
        // The map at the chapter cutscene's last camera: a source square of
        // 1024 / zoom map pixels about its position, on a 2048 map.
        // Campaign levels only (Level > -1): not the Dungeons maps.
        self.preview = magicka_runtime::end_game::campaign_position(&s.campaign)
            .and_then(|_| s.map_previews.get(&s.campaign.to_ascii_lowercase()))
            .map(|e| e.cutscene.camera(f32::MAX))
            .map(|c| {
                let size = 1024.0 / c.zoom;
                let x = (c.position[0] - 512.0 / c.zoom).trunc();
                let y = (c.position[1] - 512.0 / c.zoom).trunc();
                let size = size.trunc();
                [size / 2048.0, size / 2048.0, x / 2048.0, y / 2048.0]
            });
    }
    fn profile_at(s: &Frontend, index: usize) -> &serde_json::Value {
        &s.settings["profiles"][index]
    }
    fn avatar_index_of(&self, s: &Frontend, profile: usize) -> usize {
        let robe = Self::profile_at(s, profile)["robe"]
            .as_str()
            .unwrap_or("Wizard");
        self.avatars
            .iter()
            .position(|a| a.character.eq_ignore_ascii_case(robe))
            .unwrap_or(0)
    }
    fn avatar_index(&self, s: &Frontend) -> usize {
        self.avatar_index_of(s, s.profile)
    }
    fn color_index_of(s: &Frontend, profile: usize) -> usize {
        let colour: [f32; 3] =
            serde_json::from_value(Self::profile_at(s, profile)["colour"].clone())
                .unwrap_or([1.0; 3]);
        (0..PLAYER_COLORS.len())
            .min_by(|&a, &b| {
                let d = |i: usize| {
                    let c = player_color(i);
                    (0..3).map(|k| (c[k] - colour[k]).powi(2)).sum::<f32>()
                };
                d(a).total_cmp(&d(b))
            })
            .unwrap_or(0)
    }
    fn color_index(s: &Frontend) -> usize {
        Self::color_index_of(s, s.profile)
    }
    fn primary_editor(&self) -> SlotEditor {
        SlotEditor {
            state: self.state,
            item: self.item,
            scroll: self.scroll,
        }
    }
    fn editor_of(&self, kind: RowKind) -> SlotEditor {
        match kind {
            RowKind::Primary => self.primary_editor(),
            RowKind::Seat(i) => self.seats[i].editor,
            RowKind::Join => SlotEditor::default(),
        }
    }
    /// The primary, seated pads, then open "join" slots: four rows as in
    /// the original, more once more players are allowed and seated.
    fn row_kinds(&self) -> Vec<RowKind> {
        let seated = 1 + self.seats.len();
        let limit = usize::from(self.lobby.max_players).max(1);
        let total = seated.max(4.min(limit)).max((seated + 1).min(limit));
        std::iter::once(RowKind::Primary)
            .chain((0..self.seats.len()).map(RowKind::Seat))
            .chain(std::iter::repeat_n(RowKind::Join, total - seated))
            .collect()
    }
    fn compact(&self) -> bool {
        self.row_kinds().len() > 4
    }
    /// Rows from the first visible one that fit the original slot area.
    fn rows(&self) -> Vec<Row> {
        let kinds = self.row_kinds();
        if kinds.len() <= 4 {
            return kinds
                .into_iter()
                .enumerate()
                .map(|(i, kind)| Row {
                    kind,
                    x: SLOT_X,
                    y: SLOT_Y + SLOT_STEP * i as f32,
                    height: 112.0,
                })
                .collect();
        }
        let mut out = Vec::new();
        let mut y = SLOT_Y;
        for kind in kinds.into_iter().skip(self.first_row) {
            // The row being edited opens to the original full panel.
            let height = if self.editor_of(kind).state == SlotState::Ready {
                COMPACT_H
            } else {
                112.0
            };
            if y + height > WINDOW_BOTTOM {
                break;
            }
            out.push(Row {
                kind,
                x: COMPACT_X,
                y,
                height,
            });
            y += height + ROW_GAP;
        }
        out
    }
    fn scroll_max(&self) -> usize {
        let fit = ((WINDOW_BOTTOM - SLOT_Y + ROW_GAP) / (COMPACT_H + ROW_GAP)) as usize;
        self.row_kinds().len().saturating_sub(fit)
    }
    fn scroll_bar(&self) -> super::controls_pages::ScrollBar {
        super::controls_pages::ScrollBar {
            position: [BAR_X, (SLOT_Y + WINDOW_BOTTOM) * 0.5],
            height: WINDOW_BOTTOM - SLOT_Y,
            max: self.scroll_max() as i32,
        }
    }
    /// Scroll so `kind`'s row is visible.
    fn reveal(&mut self, kind: RowKind) {
        let Some(index) = self.row_kinds().iter().position(|k| *k == kind) else {
            return;
        };
        if index < self.first_row {
            self.first_row = index;
        }
        while self.first_row < index && !self.rows().iter().any(|r| r.kind == kind) {
            self.first_row += 1;
        }
    }
    fn row_of(&self, kind: RowKind) -> Option<Row> {
        self.rows().into_iter().find(|r| r.kind == kind)
    }
    fn items(&self, start: Item, pack: Option<Item>, settings: &[(Setting, Item)]) -> Vec<Item> {
        let mut items = vec![match self.row_of(RowKind::Primary) {
            Some(r) => Item::new(
                [r.x, r.y],
                [r.x + 448.0, r.y + r.height],
                self.primary_present,
            ),
            None => Item::new([0.0; 2], [0.0; 2], false),
        }];
        // Only the first slot holds the keyboard/mouse player; seated pads
        // edit their own rows.
        items.extend((1..4).map(|_| Item::new([0.0; 2], [0.0; 2], false)));
        items.push(start);
        items.push(back_item());
        items.extend(pack);
        items.extend(settings.iter().map(|(_, item)| *item));
        items
    }
    /// The list index of the pack row, when the lobby has one.
    fn pack_index(&self) -> Option<usize> {
        self.packs.as_ref().map(|_| PACK_ITEM)
    }
    fn first_setting(&self) -> usize {
        PACK_ITEM + usize::from(self.packs.is_some())
    }
    fn pack_labels(s: &Frontend) -> super::pack_panel::Labels {
        super::pack_panel::Labels {
            allowed: loc(s, "#menu_enabledpacks"),
            items: loc(s, "#menu_tome_03"),
            magicks: loc(s, "#menu_tome_02"),
            ok: loc(s, "#add_menu_ok"),
        }
    }
    /// The overview row's widgets; its label is the keyboard item.
    fn pack_overview(
        &self,
        s: &Frontend,
        out: &mut Vec<Widget>,
        measure: &mut Measure,
    ) -> Option<Item> {
        let panel = self.packs.as_ref()?;
        let labels = Self::pack_labels(s);
        let selected = self.list.selected == self.pack_index();
        Some(panel.overview_widgets(&labels, selected, out, measure))
    }
    /// PackMan's packs for a Challenge or Versus lobby (SubMenuCharacterSelect
    /// draws them for every GameType but Campaign, StoryChallange and Mythos,
    /// and not for Krietor). Enabled flags come from the saved settings,
    /// else every licensed pack, as ItemPack/MagickPack construct enabled.
    fn lobby_packs(s: &Frontend) -> Option<super::pack_panel::PackPanel> {
        use magicka_runtime::ruleset::{PackCatalogue, VersusMode, VersusSettings};
        match s.mode {
            super::GameMode::Challenge => {}
            super::GameMode::Versus => {
                let krietor = s
                    .versus
                    .as_deref()
                    .and_then(|spec| VersusSettings::parse(spec).ok())
                    .is_some_and(|v| v.mode == VersusMode::Krietor);
                if krietor {
                    return None;
                }
            }
            _ => return None,
        }
        let catalogue = PackCatalogue::for_content(std::path::Path::new(&s.content)).ok()?;
        let saved = &s.settings["packs"];
        let enabled = catalogue
            .packs
            .iter()
            .map(|p| saved[&p.name].as_bool().unwrap_or(true))
            .collect();
        let licensed = catalogue
            .packs
            .iter()
            .map(|p| s.dlc.licensed(p.app))
            .collect();
        let seen = s.settings["packs_seen"].as_bool().unwrap_or(false);
        Some(super::pack_panel::PackPanel::new(
            catalogue, enabled, licensed, seen,
        ))
    }
    /// Persist the pack flags (by pack name) and hand the launch spec on.
    fn save_packs(&self, s: &mut Frontend) {
        let Some(panel) = &self.packs else { return };
        let flags: serde_json::Map<String, serde_json::Value> = panel
            .catalogue
            .packs
            .iter()
            .zip(&panel.enabled)
            .map(|(p, on)| (p.name.clone(), json!(*on)))
            .collect();
        s.settings["packs"] = serde_json::Value::Object(flags);
        s.settings["packs_seen"] = json!(panel.seen);
        s.packs = Some(panel.spec());
        Self::save_profile(s);
    }
    /// State.ChangingPacks: the list owns the frame. Up/Down/Left/Right move
    /// the selected pack, confirm toggles it (or closes on OK), Back closes;
    /// clicks toggle a thumb, close on OK or drive the scroll bar.
    fn pack_list_input(&mut self, s: &mut Frontend, input: &PageInput, measure: &mut Measure) {
        let labels = Self::pack_labels(s);
        let Some(panel) = self.packs.as_mut() else {
            return;
        };
        if input.back {
            panel.close(false);
            return;
        }
        let mut changed = false;
        if input.direction[0] {
            panel.move_up();
        }
        if input.direction[1] {
            panel.move_down();
        }
        if input.direction[2] {
            panel.move_left();
        }
        if input.direction[3] {
            panel.move_right();
        }
        if input.confirm {
            match panel.selected {
                Some(i) => changed |= panel.toggle(i),
                None => {
                    panel.close(true);
                    changed = true;
                }
            }
            self.list.activated = true;
        }
        if let Some(p) = input.pointer {
            let bar = panel.scroll_bar();
            if input.pressed && bar.max > 0 && bar.bounds().inside(p) {
                match bar.arrow(p) {
                    Some(d) => panel.set_list_scroll(panel.list_scroll + d),
                    None => panel.set_list_scroll(bar.value_at(p[1])),
                }
            } else if input.released {
                let ok = panel.list_widgets(&labels, &mut Vec::new(), measure);
                if ok.inside(p) {
                    panel.close(true);
                    changed = true;
                    self.list.activated = true;
                } else if let Some(i) = panel.list_hit(p) {
                    panel.selected = Some(i);
                    changed |= panel.toggle(i);
                    self.list.activated = true;
                }
            }
        }
        if changed {
            self.save_packs(s);
        }
    }
    fn save_profile(s: &mut Frontend) {
        if let Err(e) = s.store.save_settings(&s.settings) {
            s.message = e;
        }
    }

    fn start_button(&self, s: &Frontend, out: &mut Vec<Widget>, measure: &mut Measure) -> Item {
        // Joining: the same page, its Start reads Join.
        let text = if s.joining.is_some() {
            loc(s, "#add_menu_join")
        } else {
            loc(s, "#add_menu_start")
        };
        let size = measure.text(Font::MenuOption, &text);
        text_button(
            out,
            &text,
            size,
            START,
            TextAlign::Right,
            200.0,
            self.list.selected == Some(4),
            self.can_start(s),
        )
    }

    fn settings(&self, s: &Frontend) -> Vec<Setting> {
        // Joining another host: its lobby rules are not ours to change, and
        // a joining client launches without local seats.
        if s.joining.is_some() {
            return vec![Setting::Rename];
        }
        let mut out = vec![Setting::Players, Setting::HotJoin];
        // Only matters above the original's four.
        if self.lobby.max_players > 4 {
            out.push(Setting::BossScale);
        }
        if s.online {
            out.extend([
                Setting::Spectators,
                Setting::DungeonMasters,
                Setting::Invaders,
            ]);
        }
        out.push(Setting::KeyboardSeat);
        out.push(Setting::Rename);
        out
    }
    fn setting_text(&self, setting: Setting) -> String {
        let on = |b: bool, yes: &str, no: &str| if b { yes } else { no }.to_string();
        let l = &self.lobby;
        match setting {
            Setting::Players => format!("Players: {}", l.max_players),
            Setting::HotJoin => format!("Join in progress: {}", on(l.hot_join, "open", "locked")),
            Setting::BossScale => format!(
                "Boss health for party size: {}",
                on(l.boss_scale, "on", "off")
            ),
            Setting::Spectators => format!("Spectators: {}", on(l.spectators, "on", "off")),
            Setting::DungeonMasters => {
                format!("Dungeon master: {}", on(l.dungeon_masters, "on", "off"))
            }
            Setting::Invaders => format!("Invaders: {}", on(l.invaders, "on", "off")),
            Setting::KeyboardSeat => if self.keyboard_seated() {
                "Keyboard & mouse: leave"
            } else {
                "Keyboard & mouse: join"
            }
            .into(),

            Setting::Rename => "Rename profile".into(),
        }
    }
    fn settings_layout(
        &self,
        s: &Frontend,
        out: &mut Vec<Widget>,
        measure: &mut Measure,
    ) -> Vec<(Setting, Item)> {
        let settings = self.settings(s);
        // Rows must stay above Back; long (online) lists tighten.
        let step = settings_step(settings.len());
        settings
            .into_iter()
            .enumerate()
            .map(|(i, setting)| {
                let item = menu_text_item(
                    out,
                    measure,
                    Font::MenuOption,
                    self.setting_text(setting),
                    [SETTINGS[0], SETTINGS[1] + step * i as f32],
                    TextAlign::Left,
                    self.list.selected == Some(self.first_setting() + i),
                    true,
                );
                (setting, item)
            })
            .collect()
    }
    fn change_setting(&mut self, s: &mut Frontend, setting: Setting, forward: bool) {
        let l = &mut self.lobby;
        match setting {
            Setting::Players => {
                let seated = 1 + self.seats.len();
                let allowed: Vec<u16> = LIMITS
                    .iter()
                    .copied()
                    .filter(|n| usize::from(*n) >= seated)
                    .collect();
                let at = allowed
                    .iter()
                    .position(|n| *n >= l.max_players)
                    .unwrap_or(allowed.len() - 1);
                let next = if forward {
                    (at + 1) % allowed.len()
                } else {
                    (at + allowed.len() - 1) % allowed.len()
                };
                l.max_players = allowed[next];
            }
            Setting::HotJoin => l.hot_join = !l.hot_join,
            Setting::BossScale => l.boss_scale = !l.boss_scale,
            Setting::Spectators => l.spectators = !l.spectators,
            Setting::DungeonMasters => l.dungeon_masters = !l.dungeon_masters,
            Setting::Invaders => l.invaders = !l.invaders,
            Setting::Rename => {
                if forward {
                    Self::ask_name(s, s.profile, false, None);
                }
                return;
            }
            Setting::KeyboardSeat => {
                if self.keyboard_seated() {
                    self.leave_primary(s);
                } else {
                    self.join_keyboard(s);
                }
                return;
            }
        }
        let key = Self::settings_key(s);
        s.settings[key] = json!(lobby_spec(&self.lobby));
        self.first_row = self.first_row.min(self.scroll_max());
        Self::save_profile(s);
    }

    /// Geometry shared with the drawn profile rows, robe cells and colour
    /// grid, for an editor drawn in a slot at `(x, y)`.
    fn choices_at(
        &self,
        editor: SlotEditor,
        profiles: usize,
        x: f32,
        y: f32,
    ) -> Vec<(usize, Item)> {
        match editor.state {
            SlotState::ChoosingGamer => (editor.scroll..(profiles + 1).min(editor.scroll + 4))
                .enumerate()
                .map(|(row, index)| {
                    let y = y - 6.0 + 31.0 * row as f32;
                    (
                        index,
                        Item::new([x + 144.0, y], [x + 448.0, y + 31.0], true),
                    )
                })
                .collect(),
            SlotState::CustomizingAvatar => (editor.scroll
                ..self.avatars.len().min(editor.scroll + 4))
                .enumerate()
                .map(|(cell, index)| {
                    let x = x + 32.0 + 96.0 * cell as f32;
                    (index, Item::new([x, y - 20.0], [x + 96.0, y + 132.0], true))
                })
                .collect(),
            SlotState::CustomizingColor => (0..PLAYER_COLORS.len())
                .map(|index| {
                    let [cx, cy] = color_position_at(index, x, y);
                    (
                        index,
                        Item::new([cx - 17.0, cy - 17.0], [cx + 17.0, cy + 17.0], true),
                    )
                })
                .collect(),
            SlotState::Ready => Vec::new(),
        }
    }
    /// The primary slot's choices where its row is drawn.
    fn choices(&self, profiles: usize) -> Vec<(usize, Item)> {
        let (x, y) = self
            .row_of(RowKind::Primary)
            .map_or((SLOT_X, SLOT_Y), |r| (r.x, r.y));
        self.choices_at(self.primary_editor(), profiles, x, y)
    }

    fn show(editor: &mut SlotEditor, count: usize) {
        if editor.item < editor.scroll {
            editor.scroll = editor.item;
        } else if editor.item >= editor.scroll + 4 {
            editor.scroll = editor.item - 3;
        }
        editor.scroll = editor.scroll.min(count.saturating_sub(4));
    }
    fn show_item(&mut self, count: usize) {
        let mut editor = self.primary_editor();
        Self::show(&mut editor, count);
        self.scroll = editor.scroll;
    }

    /// Profiles taken by another local slot than `except`.
    fn profile_in_use(&self, s: &Frontend, profile: usize, except: RowKind) -> bool {
        (except != RowKind::Primary && self.primary_present && s.profile == profile)
            || self
                .seats
                .iter()
                .enumerate()
                .any(|(i, seat)| except != RowKind::Seat(i) && seat.profile == profile)
    }
    fn new_profile(s: &mut Frontend) -> Option<usize> {
        let count = s.settings["profiles"].as_array().map_or(0, Vec::len);
        let id = match new_player_id() {
            Ok(id) => id,
            Err(error) => {
                s.message = error;
                return None;
            }
        };
        s.settings["profiles"].as_array_mut()?.push(json!({
            "id":id, "name":format!("Player {}", count + 1),
            "robe":"Wizard", "colour":player_color(count % PLAYER_COLORS.len())
        }));
        Some(count)
    }

    fn accept_choice(&mut self, s: &mut Frontend) {
        match self.state {
            SlotState::ChoosingGamer => {
                let count = s.settings["profiles"].as_array().map_or(0, Vec::len);
                if self.item < count && self.profile_in_use(s, self.item, RowKind::Primary) {
                    // Another local player holds that profile.
                    return;
                }
                let created = self.item == count;
                if created && Self::new_profile(s).is_none() {
                    return;
                }
                self.prior_profile = s.profile;
                s.profile = self
                    .item
                    .min(s.settings["profiles"].as_array().unwrap().len() - 1);
                if created {
                    // A new profile is named before it is saved.
                    Self::ask_name(s, s.profile, true, None);
                    return;
                }
                self.item = self.avatar_index(s);
                self.scroll = self.item.saturating_sub(1);
                self.show_item(self.avatars.len());
                self.state = SlotState::CustomizingAvatar;
            }
            SlotState::CustomizingAvatar => {
                if let Some(avatar) = self.avatars.get(self.item) {
                    s.settings["profiles"][s.profile]["robe"] = json!(avatar.character);
                }
                self.item = Self::color_index(s);
                self.state = SlotState::CustomizingColor;
            }
            SlotState::CustomizingColor => {
                s.settings["profiles"][s.profile]["colour"] = json!(player_color(self.item));
                self.state = SlotState::Ready;
                self.list.selected = Some(0);
            }
            SlotState::Ready => return,
        }
        Self::save_profile(s);
    }

    /// Open the name prompt for `profile` (rewrite extension).
    fn ask_name(s: &mut Frontend, profile: usize, created: bool, problem: Option<&str>) {
        let current = s.settings["profiles"][profile]["name"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let message = match problem {
            Some(problem) => format!("{problem} Name this profile:"),
            None => "Name this profile:".to_string(),
        };
        let options = vec![loc(s, "#add_menu_ok"), loc(s, "#add_menu_cancel")];
        s.popup = Some(MessageBox::text_input(
            Purpose::ProfileName { profile, created },
            message,
            current,
            options,
        ));
    }
    /// The name prompt closed: `text` is the confirmed line (None: cancel).
    pub(crate) fn name_answered(
        &mut self,
        s: &mut Frontend,
        profile: usize,
        created: bool,
        text: Option<&str>,
    ) {
        let Some(text) = text else {
            if created {
                // An unnamed new profile is discarded, never saved.
                if let Some(list) = s.settings["profiles"].as_array_mut() {
                    if profile + 1 == list.len() {
                        list.pop();
                    }
                }
                let count = s.settings["profiles"].as_array().map_or(0, Vec::len);
                s.profile = self.prior_profile.min(count.saturating_sub(1));
                self.item = profile;
                self.state = SlotState::ChoosingGamer;
            }
            return;
        };
        match clean_profile_name(text) {
            Ok(name) => {
                s.settings["profiles"][profile]["name"] = json!(name);
                Self::save_profile(s);
                if created {
                    self.item = self.avatar_index(s);
                    self.scroll = self.item.saturating_sub(1);
                    self.show_item(self.avatars.len());
                    self.state = SlotState::CustomizingAvatar;
                }
            }
            Err(problem) => Self::ask_name(s, profile, created, Some(problem)),
        }
    }

    /// A seated pad's accept (the same GamerState steps as the primary).
    fn accept_seat(&mut self, i: usize, s: &mut Frontend) {
        let editor = self.seats[i].editor;
        match editor.state {
            SlotState::ChoosingGamer => {
                let count = s.settings["profiles"].as_array().map_or(0, Vec::len);
                if editor.item < count && self.profile_in_use(s, editor.item, RowKind::Seat(i)) {
                    return;
                }
                let profile = if editor.item == count {
                    match Self::new_profile(s) {
                        Some(p) => p,
                        None => return,
                    }
                } else {
                    editor.item
                };
                let item = self.avatar_index_of(s, profile);
                let seat = &mut self.seats[i];
                seat.profile = profile;
                seat.editor = SlotEditor {
                    state: SlotState::CustomizingAvatar,
                    item,
                    scroll: item.saturating_sub(1),
                };
                let count = self.avatars.len();
                Self::show(&mut self.seats[i].editor, count);
            }
            SlotState::CustomizingAvatar => {
                let profile = self.seats[i].profile;
                if let Some(avatar) = self.avatars.get(editor.item) {
                    s.settings["profiles"][profile]["robe"] = json!(avatar.character);
                }
                self.seats[i].editor = SlotEditor {
                    state: SlotState::CustomizingColor,
                    item: Self::color_index_of(s, profile),
                    scroll: 0,
                };
            }
            SlotState::CustomizingColor => {
                let profile = self.seats[i].profile;
                s.settings["profiles"][profile]["colour"] = json!(player_color(editor.item));
                self.seats[i].editor = SlotEditor::default();
            }
            SlotState::Ready => return,
        }
        Self::save_profile(s);
    }

    /// Start on an unseated pad seats it with the first free profile.
    fn join(&mut self, s: &mut Frontend, pad: Entity) -> bool {
        if 1 + self.seats.len() >= usize::from(self.lobby.max_players) {
            return false;
        }
        let count = s.settings["profiles"].as_array().map_or(0, Vec::len);
        let profile = match (0..count).find(|p| !self.profile_in_use(s, *p, RowKind::Join)) {
            Some(p) => p,
            None => match Self::new_profile(s) {
                Some(p) => {
                    Self::save_profile(s);
                    p
                }
                None => return false,
            },
        };
        self.seats.push(LocalSeat {
            pad,
            profile,
            editor: SlotEditor::default(),
        });
        self.list.hovered = true;
        self.reveal(RowKind::Seat(self.seats.len() - 1));
        true
    }

    /// Keyboard and mouse hold player one's slot.
    fn keyboard_seated(&self) -> bool {
        self.primary_present && self.primary_pad.is_none()
    }

    /// Local players in the lobby (player one and the seats).
    fn local_players(&self) -> usize {
        usize::from(self.primary_present) + self.seats.len()
    }

    /// A profile no other local slot holds, creating one when all are taken.
    fn free_profile(&self, s: &mut Frontend, except: RowKind) -> Option<usize> {
        let count = s.settings["profiles"].as_array().map_or(0, Vec::len);
        match (0..count).find(|p| !self.profile_in_use(s, *p, except)) {
            Some(p) => Some(p),
            None => {
                let p = Self::new_profile(s)?;
                Self::save_profile(s);
                Some(p)
            }
        }
    }

    fn refuse_join(&self, s: &mut Frontend) -> bool {
        if s.joining.is_some() && self.primary_present {
            s.message = "Joining another computer's game seats one local player.".into();
            return true;
        }
        if self.primary_present && self.primary_role != magicka_network::lobby::Role::Player {
            s.message = format!(
                "Player 1 is the {}: local players can join while player 1 plays.",
                self.primary_role.label().to_lowercase()
            );
            return true;
        }
        if self.local_players() >= usize::from(self.lobby.max_players) {
            s.message = format!(
                "The lobby is full ({} players). Raise \"Players\" to add more.",
                self.lobby.max_players
            );
            return true;
        }
        false
    }

    /// Keyboard and mouse join. They play as player one (extra local
    /// players use controllers), so a controller holding player one moves
    /// down to a seat with its profile.
    fn join_keyboard(&mut self, s: &mut Frontend) {
        if self.keyboard_seated() {
            return;
        }
        if let Some(pad) = self.primary_pad.filter(|_| self.primary_present) {
            if self.primary_role != magicka_network::lobby::Role::Player {
                return;
            }
            let profile = s.profile;
            self.primary_pad = None;
            self.seats.insert(
                0,
                LocalSeat {
                    pad,
                    profile,
                    editor: SlotEditor::default(),
                },
            );
            if let Some(p) = self.free_profile(s, RowKind::Primary) {
                s.profile = p;
            }
            return;
        }
        if self.refuse_join(s) {
            return;
        }
        if let Some(p) = self.free_profile(s, RowKind::Primary) {
            s.profile = p;
            self.primary_pad = None;
            self.primary_present = true;
        }
    }

    /// Start on an unseated pad: player one when that slot is empty, else a
    /// further seat.
    fn join_pad(&mut self, s: &mut Frontend, pad: Entity) {
        if self.primary_present {
            if !self.refuse_join(s) {
                self.join(s, pad);
            }
            return;
        }
        if self.refuse_join(s) {
            return;
        }
        if let Some(p) = self.free_profile(s, RowKind::Primary) {
            s.profile = p;
            self.primary_pad = Some(pad);
            self.primary_present = true;
        }
    }

    /// Player one leaves; the next seated player moves up.
    fn leave_primary(&mut self, s: &mut Frontend) {
        self.state = SlotState::Ready;
        if self.seats.is_empty() {
            self.primary_present = false;
            self.primary_pad = None;
            return;
        }
        let seat = self.seats.remove(0);
        s.profile = seat.profile;
        self.primary_pad = Some(seat.pad);
        self.primary_role = magicka_network::lobby::Role::Player;
    }

    /// Roles player one may take here: online hosts can run without a
    /// wizard as a dungeon master or spectator; joiners request any role;
    /// offline play is players only.
    fn primary_roles(&self, s: &Frontend) -> Vec<magicka_network::lobby::Role> {
        use magicka_network::lobby::Role;
        if s.joining.is_some() {
            // What the chosen host allows (all when its beacon is unheard).
            match join_advert(s) {
                Some(a) => Role::ALL
                    .into_iter()
                    .filter(|r| match r {
                        Role::Player => true,
                        Role::Spectator => a.lobby.spectators,
                        Role::DungeonMaster => a.lobby.dungeon_masters,
                        Role::Invader => a.lobby.invaders,
                    })
                    .collect(),
                None => Role::ALL.to_vec(),
            }
        } else if s.online {
            vec![Role::Player, Role::DungeonMaster, Role::Spectator]
        } else {
            vec![Role::Player]
        }
    }

    /// Step player one's role badge. A role without a wizard seats nobody
    /// else locally.
    fn cycle_role(&mut self, s: &mut Frontend, forward: bool) {
        let roles = self.primary_roles(s);
        if roles.len() < 2 || !self.primary_present {
            return;
        }
        self.primary_role = step_choice(&roles, &self.primary_role, forward);
        if self.primary_role != magicka_network::lobby::Role::Player && !self.seats.is_empty() {
            self.seats.clear();
            s.message = format!(
                "Local players left: player 1 is now the {}.",
                self.primary_role.label().to_lowercase()
            );
        }
        if s.joining.is_some() {
            s.join_role = self.primary_role;
            s.lan = None;
        }
    }

    /// Where the role badge sits in a slot row.
    fn role_badge(&self, row: Row) -> Item {
        if row.height < 112.0 {
            Item::new(
                [row.x + 100.0, row.y + 4.0],
                [row.x + 290.0, row.y + 30.0],
                true,
            )
        } else {
            Item::new(
                [row.x + 255.0, row.y + 6.0],
                [row.x + 440.0, row.y + 38.0],
                true,
            )
        }
    }

    /// Start: some local player, or an online host without a wizard.
    fn can_start(&self, _s: &Frontend) -> bool {
        self.primary_present
    }

    /// The UIControllers sprite for a slot's device: keyboard, or gp1..gp4
    /// by controller number (the original's four pads).
    fn control_sprite(&self, pad: Option<Entity>) -> Option<[f32; 4]> {
        let name = match pad {
            None => "keyboard".to_string(),
            Some(pad) => {
                let i = self.pads.iter().position(|(e, _)| *e == pad)?;
                format!("gp{}", i % 4 + 1)
            }
        };
        self.controller_sprites.get(&name).copied()
    }

    /// Where a slot row's controller icon is drawn.
    fn control_icon(&self, row: Row) -> Item {
        let (centre, scale) = if row.height < 112.0 {
            ([row.x + 320.0, row.y + 28.0], 0.4)
        } else {
            // SubMenuCharacterSelect: the icon at slot + (282, 56); raised
            // and scaled here so the name below stays clear.
            ([row.x + 205.0, row.y + 34.0], 0.6)
        };
        let half = [75.0 * scale, 45.0 * scale];
        Item::new(
            [centre[0] - half[0], centre[1] - half[1]],
            [centre[0] + half[0], centre[1] + half[1]],
            true,
        )
    }

    /// The pad of a slot row (None: keyboard and mouse or no player).
    fn row_pad(&self, kind: RowKind) -> Option<Option<Entity>> {
        match kind {
            RowKind::Primary => self.primary_present.then_some(self.primary_pad),
            RowKind::Seat(i) => Some(Some(self.seats[i].pad)),
            RowKind::Join => None,
        }
    }

    /// One seated pad's menu edges. Returns true to start the game.
    fn seat_input(&mut self, i: usize, e: crate::menu_input::PadEdges, s: &mut Frontend) -> bool {
        let editor = self.seats[i].editor;
        if editor.state == SlotState::Ready {
            if e.start {
                return true;
            }
            if e.back {
                self.seats.remove(i);
                self.list.hovered = true;
                return false;
            }
            if e.confirm {
                let profile = self.seats[i].profile;
                self.seats[i].editor = SlotEditor {
                    state: SlotState::ChoosingGamer,
                    item: profile,
                    scroll: profile.saturating_sub(3),
                };
                self.list.activated = true;
                self.reveal(RowKind::Seat(i));
            }
            return false;
        }
        if e.start {
            if editor.state != SlotState::ChoosingGamer {
                self.accept_seat(i, s);
            }
            return true;
        }
        if e.back {
            self.seats[i].editor = SlotEditor::default();
            return false;
        }
        let profiles = s.settings["profiles"].as_array().map_or(0, Vec::len);
        let count = match editor.state {
            SlotState::ChoosingGamer => profiles + 1,
            SlotState::CustomizingAvatar => self.avatars.len().max(1),
            _ => PLAYER_COLORS.len(),
        };
        let backwards = e.direction[0] || e.direction[2];
        let forwards = e.direction[1] || e.direction[3];
        if backwards || forwards {
            let seat = &mut self.seats[i];
            seat.editor.item = (seat.editor.item + if backwards { count - 1 } else { 1 }) % count;
            Self::show(&mut seat.editor, count);
            self.list.hovered = true;
        }
        if e.confirm {
            self.accept_seat(i, s);
            self.list.activated = true;
        }
        false
    }

    /// The local players handed to play (pads, profiles) and the lobby.
    fn publish(&self, s: &mut Frontend) {
        s.local_players = self
            .seats
            .iter()
            .map(|seat| {
                let p = Self::profile_at(s, seat.profile);
                json!({
                    "pad": seat.pad.to_bits(),
                    "id": p["id"],
                    "name": p["name"].as_str().unwrap_or("Player"),
                    "robe": p["robe"].as_str().unwrap_or("Wizard"),
                    "colour": p["colour"].clone(),
                })
            })
            .collect();
        s.seated_pads = self.seats.iter().map(|seat| seat.pad).collect();
        s.lobby_spec = Some(lobby_spec(&self.lobby));
        s.packs = self.packs.as_ref().map(|p| p.spec());
    }

    fn launch(&mut self, s: &mut Frontend, tome: &mut TomeView) {
        if !self.can_start(s) {
            s.message = "Join first: press Start on a controller, or click an open slot to play with keyboard and mouse.".into();
            return;
        }
        use magicka_network::lobby::Role;
        s.host_role = (s.online && s.joining.is_none() && self.primary_role != Role::Player)
            .then_some(self.primary_role);
        if s.joining.is_some() {
            s.join_role = self.primary_role;
        }
        s.continue_primary = Some(
            self.primary_pad
                .map_or_else(|| "keyboard".into(), |p| p.to_bits().to_string()),
        );
        // Commit a previewed robe/colour before starting with that profile.
        if matches!(
            self.state,
            SlotState::CustomizingAvatar | SlotState::CustomizingColor
        ) {
            self.accept_choice(s);
        }
        self.state = SlotState::Ready;
        self.publish(s);
        // Joining: the profile chosen here is submitted with the hello.
        super::launch_game(s, s.joining.is_some(), tome);
    }

    pub(crate) fn update(
        &mut self,
        s: &mut Frontend,
        tome: &mut TomeView,
        input: &PageInput,
        measure: &mut Measure,
    ) {
        self.pads = input.connected.clone();
        if s.joining.is_some() {
            if let Some(browser) = s.lan.as_mut() {
                s.lan_games = browser.poll(std::time::Instant::now());
            }
            // The host may have closed the chosen role meanwhile.
            let roles = self.primary_roles(s);
            if !roles.contains(&self.primary_role) {
                self.primary_role = magicka_network::lobby::Role::Player;
                s.join_role = self.primary_role;
            }
        }
        if !self.primary_decided {
            // The device that opened the lobby holds player one: the pad
            // driving the menu, else keyboard and mouse.
            self.primary_pad = input
                .controller
                .filter(|p| !self.seats.iter().any(|seat| seat.pad == *p));
            self.primary_present = true;
            self.primary_decided = true;
        }
        // A disconnected pad leaves player one's slot.
        if self.primary_present
            && self
                .primary_pad
                .is_some_and(|p| !self.pads.iter().any(|(e, _)| *e == p))
        {
            self.leave_primary(s);
        }
        // Player one's pad: Back on its Ready slot leaves the lobby (the
        // shared menu's Back then waits for the next press).
        let primary_left = self.primary_present
            && self.state == SlotState::Ready
            && self
                .primary_pad
                .is_some_and(|p| input.pads.iter().any(|(pad, e)| *pad == p && e.back));
        if primary_left {
            self.leave_primary(s);
            self.publish(s);
            return;
        }
        // Audit: the first N connected pads press Start.
        if s.qa_add_players > 0 && !self.pads.is_empty() {
            let n = std::mem::take(&mut s.qa_add_players);
            let pads: Vec<Entity> = self.pads.iter().map(|(e, _)| *e).take(n).collect();
            for pad in pads {
                self.join_pad(s, pad);
            }
        }
        // Pads: seated ones edit their own rows; Start on a free pad joins.
        for (pad, edges) in &input.pads {
            if let Some(i) = self.seats.iter().position(|seat| seat.pad == *pad) {
                if self.seat_input(i, *edges, s) {
                    self.launch(s, tome);
                    return;
                }
            } else if edges.start && !(self.primary_present && Some(*pad) == self.primary_pad) {
                self.join_pad(s, *pad);
            }
        }
        self.publish(s);
        if self.packs.as_ref().is_some_and(|p| p.open) {
            self.pack_list_input(s, input, measure);
            return;
        }
        let start = self.start_button(s, &mut Vec::new(), measure);
        let pack = self.pack_overview(s, &mut Vec::new(), measure);
        let settings = self.settings_layout(s, &mut Vec::new(), measure);
        let items = self.items(start, pack, &settings);
        // The compact list's MenuScrollBar.
        if self.compact() && self.state == SlotState::Ready {
            let bar = self.scroll_bar();
            if !input.held {
                self.grabbed = false;
            }
            if let Some(p) = input.pointer {
                if self.grabbed {
                    self.first_row = bar.value_at(p[1]) as usize;
                } else if input.pressed && bar.bounds().inside(p) {
                    match bar.arrow(p) {
                        Some(d) => {
                            self.first_row = (self.first_row as i32 + d).clamp(0, bar.max) as usize
                        }
                        None => {
                            self.first_row = bar.value_at(p[1]) as usize;
                            self.grabbed = true;
                        }
                    }
                }
                let area = Item::new([COMPACT_X, SLOT_Y], [BAR_X + 32.0, WINDOW_BOTTOM], true);
                if input.wheel != 0.0 && area.inside(p) {
                    self.first_row = if input.wheel > 0.0 {
                        self.first_row.saturating_sub(1)
                    } else {
                        (self.first_row + 1).min(self.scroll_max())
                    };
                }
            }
        }
        // An open slot clicked: keyboard and mouse join (rewrite extension;
        // pads join with Start). Player one's role badge cycles.
        if self.state == SlotState::Ready {
            if let Some(p) = input.pointer.filter(|_| input.released) {
                let badge = self
                    .row_of(RowKind::Primary)
                    .filter(|_| self.primary_present && self.primary_roles(s).len() > 1)
                    .map(|r| self.role_badge(r));
                if badge.is_some_and(|b| b.inside(p)) {
                    self.cycle_role(s, true);
                    self.list.activated = true;
                    return;
                }
                if self.rows().iter().any(|r| {
                    (r.kind == RowKind::Join
                        || (r.kind == RowKind::Primary && !self.primary_present))
                        && Item::new([r.x, r.y], [r.x + 448.0, r.y + r.height], true).inside(p)
                }) {
                    // Once seated, the click means nothing: further players
                    // join with Start on their own controller.
                    if !self.keyboard_seated() {
                        self.join_keyboard(s);
                    }
                    self.list.activated = true;
                    return;
                }
            }
        }
        // Start/Back remain available while editing. A release outside a
        // choice must never silently accept the keyboard selection.
        let clicked = self.list.pointer(&items, input);
        let action = clicked.or_else(|| input.confirm.then_some(self.list.selected).flatten());
        self.list.activated = clicked.is_some() || input.confirm;
        if action == Some(4) {
            self.launch(s, tome);
            return;
        }
        if input.back || action == Some(5) {
            if self.state == SlotState::Ready {
                tome.pop();
            } else {
                self.state = SlotState::Ready;
            }
            return;
        }
        if self.state == SlotState::Ready {
            // Pointer handling above already processed this frame's release.
            if input.direction[0] {
                self.list.up(&items);
            }
            if input.direction[1] {
                self.list.down(&items);
            }
            // Position 5: A opens the pack list, Left/Right scroll the
            // overview, and its arrows and thumbs take clicks.
            if let Some(pack) = self.pack_index() {
                let mut open = action == Some(pack);
                if self.list.selected == Some(pack) {
                    if input.direction[2] {
                        self.packs.as_mut().unwrap().scroll_pack(-1);
                    }
                    if input.direction[3] {
                        self.packs.as_mut().unwrap().scroll_pack(1);
                    }
                }
                if let Some(p) = input.pointer.filter(|_| input.released && !open) {
                    use super::pack_panel::OverviewHit;
                    let width = measure.text(Font::MenuOption, &loc(s, "#menu_enabledpacks"))[0];
                    let panel = self.packs.as_mut().unwrap();
                    match panel.overview_hit(p, width) {
                        Some(OverviewHit::ScrollLeft) => panel.scroll_pack(-1),
                        Some(OverviewHit::ScrollRight) => panel.scroll_pack(1),
                        Some(OverviewHit::Open) => open = true,
                        None => {}
                    }
                }
                if open {
                    self.packs.as_mut().unwrap().open();
                    self.list.selected = Some(pack);
                    self.list.activated = true;
                    return;
                }
            }
            let first = self.first_setting();
            let setting = |i: Option<usize>| {
                i.and_then(|i| i.checked_sub(first))
                    .and_then(|i| settings.get(i))
                    .map(|(setting, _)| *setting)
            };
            if let Some(chosen) = setting(action) {
                self.change_setting(s, chosen, true);
            } else if let Some(chosen) =
                setting(self.list.selected).filter(|c| *c != Setting::Rename)
            {
                if input.direction[2] {
                    self.change_setting(s, chosen, false);
                } else if input.direction[3] {
                    self.change_setting(s, chosen, true);
                }
            }
            if self.list.selected == Some(0) && (input.direction[2] || input.direction[3]) {
                self.cycle_role(s, input.direction[3]);
            }
            if action == Some(0) {
                self.item = s.profile;
                self.scroll = self.item.saturating_sub(3);
                self.state = SlotState::ChoosingGamer;
                self.reveal(RowKind::Primary);
            }
            return;
        }
        let profiles = s.settings["profiles"].as_array().map_or(0, Vec::len);
        let count = match self.state {
            SlotState::ChoosingGamer => profiles + 1,
            SlotState::CustomizingAvatar => self.avatars.len().max(1),
            _ => PLAYER_COLORS.len(),
        };
        let backwards = input.direction[0] || input.direction[2];
        let forwards = input.direction[1] || input.direction[3];
        if backwards || forwards {
            self.item = (self.item + if backwards { count - 1 } else { 1 }) % count;
            self.show_item(count);
            self.list.selected = Some(0);
        }
        if input.wheel != 0.0 && input.pointer.is_some_and(|p| items[0].inside(p)) {
            self.scroll = if input.wheel > 0.0 {
                self.scroll.saturating_sub(1)
            } else {
                (self.scroll + 1).min(count.saturating_sub(4))
            };
        }
        let choices = self.choices(profiles);
        let hit = input
            .pointer
            .and_then(|p| choices.iter().find(|(_, item)| item.inside(p)))
            .map(|(index, _)| *index);
        if input.screen_moved || input.released || input.wheel != 0.0 {
            if let Some(index) = hit {
                self.list.hovered |= self.item != index;
                self.item = index;
                self.list.selected = Some(0);
            }
        }
        if input.confirm || (input.released && hit.is_some()) {
            self.accept_choice(s);
            self.list.activated = true;
        }
    }

    /// One slot in the original layout (full height) or the compact list.
    #[allow(clippy::too_many_arguments)]
    fn draw_slot(
        &self,
        out: &mut Vec<Widget>,
        s: &Frontend,
        row: Row,
        number: usize,
        profile_index: Option<usize>,
        editor: SlotEditor,
        on: bool,
    ) {
        let (x, y) = (row.x, row.y);
        let in_use = profile_index.is_some();
        let power = if on { 1.5 } else { 1.0 };
        panel(
            out,
            [x, y],
            [448.0, row.height],
            [power, power, power, if in_use { 1.0 } else { 0.66 }],
            if on { 1.5 } else { 1.0 },
        );
        let compact = row.height < 112.0;
        let Some(profile_index) = profile_index else {
            let text = Widget::text(
                Font::MenuOption,
                loc(s, "#add_menu_join"),
                [x + 112.0, y + if compact { 10.0 } else { 16.0 }],
                TextAlign::Left,
            )
            .color(MENU_COLOR);
            out.push(if compact { text.scale(0.8) } else { text });
            return;
        };
        // Player one without a wizard (dungeon master, spectator, invader).
        let wizardless = row.kind == RowKind::Primary
            && self.primary_role != magicka_network::lobby::Role::Player;
        let profile = Self::profile_at(s, profile_index);
        let colour: [f32; 3] =
            serde_json::from_value(profile["colour"].clone()).unwrap_or([1.0; 3]);
        let avatar_index = self.avatar_index_of(s, profile_index);
        let name = profile["name"].as_str().unwrap_or("Player").to_string();
        if compact {
            // Player number, small portrait and name on one half-height row.
            out.push(
                Widget::text(
                    Font::MenuDefault,
                    format!("{}", number + 1),
                    [x + 22.0, y + 14.0],
                    TextAlign::Center,
                )
                .color(MENU_COLOR)
                .scale(0.8),
            );
            if let Some(avatar) = self.avatars.get(avatar_index) {
                out.push(
                    Widget::avatar(
                        avatar.thumb.clone(),
                        colour,
                        [x + 72.0, y + 28.0],
                        [42.0, 50.0],
                    )
                    .saturation(if wizardless { 0.15 } else { 1.0 }),
                );
            }
            let c = DIALOGUE_COLOR;
            for (offset, color) in [
                (2.0, [0.0, 0.0, 0.0, 1.0]),
                (0.0, [c[0] * 1.2, c[1] * 1.2, c[2] * 1.2, 1.0]),
            ] {
                out.push(
                    Widget::text(
                        Font::MenuOption,
                        name.clone(),
                        [x + 448.0 - 15.0 + offset, y + 10.0 + offset],
                        TextAlign::Right,
                    )
                    .color(color)
                    .scale(0.8),
                );
            }
            return;
        }
        match editor.state {
            SlotState::CustomizingAvatar => {
                // Four thumbnails from the scroll position.
                let n = self.avatars.len().max(1);
                let start = editor.scroll.min(n.saturating_sub(4));
                for k in 0..4.min(n) {
                    let index = (start + k) % n;
                    let current = index == editor.item;
                    let scale = if current { 0.5 } else { 0.4 };
                    if let Some(avatar) = self.avatars.get(index) {
                        out.push(
                            Widget::avatar(
                                avatar.thumb.clone(),
                                colour,
                                [x + 80.0 + 96.0 * k as f32, y + 56.0],
                                [252.0 * scale, 302.0 * scale],
                            )
                            .saturation(if current {
                                1.0
                            } else {
                                0.25
                            }),
                        );
                    }
                }
            }
            SlotState::CustomizingColor => {
                if let Some(avatar) = self.avatars.get(avatar_index) {
                    out.push(Widget::avatar(
                        avatar.thumb.clone(),
                        player_color(editor.item),
                        [x + 80.0, y + 56.0],
                        [126.0, 151.0],
                    ));
                }
                for c in 0..PLAYER_COLORS.len() {
                    let [cx, cy] = color_position_at(c, x, y);
                    let size = if c == editor.item { 50.0 } else { 38.0 };
                    let power = if c == editor.item { 1.5 } else { 1.0 };
                    let at = [cx - size * 0.5, cy - size * 0.5];
                    out.push(
                        Widget::image(Atlas::Pages, [1408.0, 32.0, 64.0, 64.0], at, [size, size])
                            .color([power, power, power, 1.0]),
                    );
                    let [r, g, b] = player_color(c);
                    out.push(
                        Widget::image(Atlas::Pages, [1344.0, 32.0, 64.0, 64.0], at, [size, size])
                            .color([r, g, b, 1.0]),
                    );
                }
            }
            SlotState::ChoosingGamer => {
                let names: Vec<String> = s.settings["profiles"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|p| p["name"].as_str().unwrap_or("Player").to_string())
                    .chain([loc(s, "#add_menu_prof_new")])
                    .collect();
                let first = editor.scroll;
                for (row, k) in (first..names.len().min(first + 4)).enumerate() {
                    let color = if k == editor.item {
                        MENU_COLOR_SELECTED
                    } else {
                        let c = DIALOGUE_COLOR;
                        [c[0] * 1.333, c[1] * 1.333, c[2] * 1.333, 1.0]
                    };
                    out.push(
                        Widget::text(
                            Font::MenuDefault,
                            names[k].clone(),
                            [x + 256.0, y + 56.0 - 62.0 + 31.0 * row as f32 + 8.0],
                            TextAlign::Center,
                        )
                        .color(color)
                        .scale(0.9),
                    );
                }
            }
            SlotState::Ready => {
                if let Some(avatar) = self.avatars.get(avatar_index) {
                    out.push(
                        Widget::avatar(
                            avatar.thumb.clone(),
                            colour,
                            [x + 80.0, y + 56.0],
                            [126.0, 151.0],
                        )
                        .saturation(if wizardless { 0.15 } else { 1.0 }),
                    );
                }
                out.push(
                    Widget::text(
                        Font::MenuOption,
                        name.clone(),
                        [x + 448.0 - 15.0 + 2.0, y + 56.0 + 10.0 + 2.0],
                        TextAlign::Right,
                    )
                    .color([0.0, 0.0, 0.0, 1.0]),
                );
                let c = DIALOGUE_COLOR;
                out.push(
                    Widget::text(
                        Font::MenuOption,
                        name,
                        [x + 448.0 - 15.0, y + 56.0 + 10.0],
                        TextAlign::Right,
                    )
                    .color([c[0] * 1.2, c[1] * 1.2, c[2] * 1.2, 1.0]),
                );
            }
        }
    }

    pub(crate) fn draw(&self, s: &Frontend, draw: &mut PageDraw, measure: &mut Measure) {
        let mut out = Vec::new();
        let selected = self.list.selected;
        if s.joining.is_some() {
            server_panel(s, &mut out);
        } else if let Some(uv) = self.preview {
            // SubMenuCharacterSelect.Draw: the masked map at (64, 224).
            out.push(
                Widget::masked(
                    Atlas::Content("UI/Menu/CampaignMap".into()),
                    Atlas::Content("UI/Menu/MapMask".into()),
                    [64.0, 224.0],
                    [448.0, 448.0],
                )
                .uv_transform(uv),
            );
        }
        // ChangingPacks replaces the level box with the pack list.
        let packs_open = self.packs.as_ref().is_some_and(|p| p.open);
        if let Some((name, description)) = self
            .chapter
            .as_ref()
            .filter(|_| !packs_open && s.joining.is_none())
        {
            let line = measure.line_height(Font::MenuTitle);
            out.push(Widget::text(
                Font::MenuTitle,
                loc(s, name),
                [288.0, 96.0],
                TextAlign::Center,
            ));
            // Campaigns show the short title at 0.8; the challenges' longer
            // descriptions draw at 0.45 (the level box's scale), wrapped to
            // the left half of the page.
            let challenge = matches!(
                s.mode,
                super::GameMode::StoryChallenge
                    | super::GameMode::Challenge
                    | super::GameMode::Versus
            );
            let scale = if challenge { 0.45 } else { 0.8 };
            let text = if challenge {
                measure.assets.wrap(
                    Font::MenuTitle,
                    &loc(s, description),
                    (448.0 / scale) as i32,
                    measure.images,
                )
            } else {
                loc(s, description)
            };
            out.push(
                Widget::text(
                    Font::MenuTitle,
                    text,
                    [288.0, 96.0 + line],
                    TextAlign::Center,
                )
                .scale(scale),
            );
        }
        let kinds = self.row_kinds();
        for row in self.rows() {
            let number = kinds.iter().position(|k| *k == row.kind).unwrap_or(0);
            match row.kind {
                RowKind::Primary => self.draw_slot(
                    &mut out,
                    s,
                    row,
                    number,
                    self.primary_present.then_some(s.profile),
                    self.primary_editor(),
                    selected == Some(0) && self.state == SlotState::Ready,
                ),
                RowKind::Seat(i) => self.draw_slot(
                    &mut out,
                    s,
                    row,
                    number,
                    Some(self.seats[i].profile),
                    self.seats[i].editor,
                    self.seats[i].editor.state != SlotState::Ready,
                ),
                RowKind::Join => {
                    self.draw_slot(&mut out, s, row, number, None, SlotEditor::default(), false)
                }
            }
            let ready = self.editor_of(row.kind).state == SlotState::Ready;
            // Role badge (rewrite extension): player one's is cyclable where
            // other roles exist; seats always play.
            let role = match row.kind {
                RowKind::Primary if self.primary_present => Some(self.primary_role),
                RowKind::Seat(_) => Some(magicka_network::lobby::Role::Player),
                _ => None,
            };
            let cyclable = row.kind == RowKind::Primary && self.primary_roles(s).len() > 1;
            if let (Some(role), true) = (role, ready) {
                let compact = row.height < 112.0;
                // Compact rows show only roles other than player.
                if !compact || role != magicka_network::lobby::Role::Player {
                    let badge = self.role_badge(row);
                    let text = if cyclable {
                        format!("{} >", role.label())
                    } else {
                        role.label().to_string()
                    };
                    let (at, align) = if compact {
                        ([badge.min[0], badge.min[1] + 4.0], TextAlign::Left)
                    } else {
                        ([badge.max[0], badge.min[1] + 4.0], TextAlign::Right)
                    };
                    let colour = if cyclable { MENU_COLOR } else { DIALOGUE_COLOR };
                    out.push(
                        Widget::text(Font::MenuOption, text, at, align)
                            .color(colour)
                            .scale(if compact { 0.55 } else { 0.65 }),
                    );
                }
            }
            // The slot's device (SubMenuCharacterSelect.ControllerIcon,
            // CONTROLLER_COLOUR = white at half alpha), on Ready slots.
            if let (Some(pad), true) = (self.row_pad(row.kind), ready) {
                if let Some(source) = self.control_sprite(pad) {
                    let icon = self.control_icon(row);
                    out.push(
                        Widget::image(
                            Atlas::Content("UI/Spritesheets/UIControllers".into()),
                            source,
                            icon.min,
                            [icon.max[0] - icon.min[0], icon.max[1] - icon.min[1]],
                        )
                        .color([1.0, 1.0, 1.0, 0.5]),
                    );
                }
            }
        }
        if self.compact() && self.scroll_max() > 0 {
            self.scroll_bar().draw(&mut out, self.first_row as i32);
        }
        if let Some(panel) = &self.packs {
            // DrawPacksOverview under the level box; the list while open.
            // The overview, like the level box, draws at mOptionsAlpha, which
            // fades out outside State.Normal/CountDown (fade not modelled).
            let labels = Self::pack_labels(s);
            if panel.open {
                panel.list_widgets(&labels, &mut out, measure);
            } else {
                panel.overview_widgets(&labels, selected == self.pack_index(), &mut out, measure);
            }
        }
        self.settings_layout(s, &mut out, measure);
        self.start_button(s, &mut out, measure);
        out.push(back_button(selected == Some(5)));
        let mut page = draw.right(MenuId::Character);
        for widget in out {
            page.push(widget);
        }
    }
}

// ---------------------------------------------------------------------------
// SubMenuOptions

const OPTIONS: [&str; 6] = [
    "#menu_opt_01",
    "#menu_opt_03",
    "#menu_opt_04",
    "#menu_opt_02",
    "#paradox_account",
    "#menu_hud_styles",
];

#[derive(Default)]
pub(crate) struct OptionsPage {
    list: List,
}
impl OptionsPage {
    fn enter(&mut self) {
        self.list.selected = Some(0);
    }
    fn layout(
        s: &Frontend,
        measure: &mut Measure,
        selected: Option<usize>,
    ) -> (Vec<Widget>, Vec<Item>) {
        let mut out = Vec::new();
        let mut items = Vec::new();
        out.push(Widget::text(
            Font::MenuTitle,
            loc(s, "#menu_main_06"),
            [512.0, 96.0],
            TextAlign::Center,
        ));
        let line = measure.line_height(Font::MenuOption) + 10.0;
        for (i, key) in OPTIONS.iter().enumerate() {
            // The Paradox account entry needs a logged-in account.
            let enabled = i != 4;
            items.push(menu_text_item(
                &mut out,
                measure,
                Font::MenuOption,
                loc(s, key),
                [512.0, 1024.0 * 0.333 + line * i as f32],
                TextAlign::Center,
                selected == Some(i),
                enabled,
            ));
        }
        out.push(back_button(selected == Some(6)));
        items.push(back_item());
        out.push(graphic(
            [448.0, 976.0, 608.0, 48.0],
            [208.0, 220.0, 608.0, 48.0],
        ));
        out.push(graphic(
            [448.0, 768.0, 496.0, 208.0],
            [264.0, 620.0, 496.0, 208.0],
        ));
        (out, items)
    }
    fn update(
        &mut self,
        s: &mut Frontend,
        tome: &mut TomeView,
        input: &PageInput,
        measure: &mut Measure,
    ) {
        if input.back {
            tome.pop();
            return;
        }
        let (_, items) = Self::layout(s, measure, self.list.selected);
        match self.list.update(&items, input) {
            // SubMenuOptions.ControllerA: each page riffles its own count.
            Some(0) => tome.push(MenuId::GameOptions, 1),
            Some(1) => tome.push(MenuId::Sound, 3),
            Some(2) => tome.push(MenuId::Graphics, 4),
            Some(3) => tome.push(MenuId::Controls, 1),
            Some(6) => tome.pop(),
            Some(_) => {
                let ok = loc(s, "#add_menu_ok");
                s.popup = Some(MessageBox::new(
                    Purpose::Notice,
                    "These options are not available yet.".into(),
                    vec![ok],
                ));
            }
            None => {}
        }
    }
    fn draw(&self, s: &Frontend, draw: &mut PageDraw, measure: &mut Measure) {
        let (out, _) = Self::layout(s, measure, self.list.selected);
        let mut page = draw.right(MenuId::Options);
        for widget in out {
            page.push(widget);
        }
    }
}

// ---------------------------------------------------------------------------
// SubMenuOptionsSound

#[derive(Default)]
pub(crate) struct SoundPage {
    list: List,
    /// Music and effect volume levels, 0..=10.
    levels: [i32; 2],
    entered: [i32; 2],
    grabbed: Option<usize>,
}
impl SoundPage {
    fn enter(&mut self, s: &Frontend) {
        self.list.selected = Some(0);
        self.levels =
            ["music", "sfx"].map(|k| (s.settings[k].as_f64().unwrap_or(1.0) * 10.0).round() as i32);
        self.entered = self.levels;
        self.grabbed = None;
    }
    /// OnExit: store and save changed volumes.
    fn exit(&self, s: &mut Frontend) {
        if self.levels != self.entered {
            if let Err(e) = s.store.save_settings(&s.settings) {
                s.message = e;
            }
        }
    }
    fn sliders(measure: &mut Measure) -> [Slider; 2] {
        let line = measure.line_height(Font::MenuOption) + 10.0;
        let scale = (line / 64.0).min(1.0);
        std::array::from_fn(|i| Slider {
            position: [472.0 + 200.0, 1024.0 * 0.333 + line * i as f32],
            width: 320.0,
            max: 10,
            scale,
        })
    }
    fn layout(&self, s: &Frontend, measure: &mut Measure) -> (Vec<Widget>, Vec<Item>) {
        let mut out = Vec::new();
        let mut items = Vec::new();
        let selected = self.list.selected;
        out.push(Widget::text(
            Font::MenuTitle,
            loc(s, "#menu_opt_08"),
            [512.0, 96.0],
            TextAlign::Center,
        ));
        let line = measure.line_height(Font::MenuOption) + 10.0;
        for (i, key) in ["#menu_opt_sfx_01", "#menu_opt_sfx_02"].iter().enumerate() {
            items.push(menu_text_item(
                &mut out,
                measure,
                Font::MenuOption,
                loc(s, key),
                [472.0, 1024.0 * 0.333 + line * i as f32],
                TextAlign::Right,
                selected == Some(i),
                true,
            ));
        }
        out.push(back_button(selected == Some(2)));
        items.push(back_item());
        out.push(graphic(
            [448.0, 976.0, 608.0, 48.0],
            [208.0, 220.0, 608.0, 48.0],
        ));
        for (slider, value) in Self::sliders(measure).iter().zip(self.levels) {
            slider.draw(&mut out, value);
        }
        (out, items)
    }
    fn update(
        &mut self,
        s: &mut Frontend,
        tome: &mut TomeView,
        input: &PageInput,
        mouse_down: bool,
        measure: &mut Measure,
    ) {
        let sliders = Self::sliders(measure);
        if input.back {
            self.exit(s);
            tome.pop();
            return;
        }
        let before = self.levels;
        // ControllerLeft/Right adjust the selected volume by one level.
        if let Some(i @ 0..=1) = self.list.selected {
            if input.direction[2] {
                self.levels[i] = (self.levels[i] - 1).max(0);
            }
            if input.direction[3] {
                self.levels[i] = (self.levels[i] + 1).min(10);
            }
        }
        // A grabbed knob follows the pointer until the button is released.
        if !mouse_down {
            self.grabbed = None;
        }
        if let (Some(i), Some(p)) = (self.grabbed, input.pointer) {
            self.levels[i] = sliders[i].value_at(p[0]);
        }
        if let (Some(p), true) = (input.pointer, input.pressed) {
            for (i, slider) in sliders.iter().enumerate() {
                if slider.bounds().inside(p) {
                    match slider.end(p) {
                        Some(d) => self.levels[i] = (self.levels[i] + d).clamp(0, 10),
                        None => {
                            self.levels[i] = slider.value_at(p[0]);
                            self.grabbed = Some(i);
                        }
                    }
                }
            }
        }
        let (_, items) = self.layout(s, measure);
        match self.list.update(&items, input) {
            Some(2) => {
                self.exit(s);
                tome.pop();
            }
            Some(i) => self.levels[i] = (self.levels[i] + 1).min(10),
            None => {}
        }
        // AudioManager.VolumeMusic/VolumeSound take effect at once; the
        // settings file is written on leaving the page.
        if self.levels != before {
            for (k, v) in ["music", "sfx"].iter().zip(self.levels) {
                s.settings[*k] = json!(v as f64 / 10.0);
            }
        }
    }
    fn draw(&self, s: &Frontend, draw: &mut PageDraw, measure: &mut Measure) {
        let (out, _) = self.layout(s, measure);
        let mut page = draw.right(MenuId::Sound);
        for widget in out {
            page.push(widget);
        }
    }
}

// ---------------------------------------------------------------------------
// SubMenuOnline (direct connections: the history stands in for the server list)

#[derive(Default)]
pub(crate) struct OnlinePage {
    list: List,
    /// The highlighted server row.
    server: Option<usize>,
}
const LIST_BG: [f32; 4] = [144.0, 288.0, 800.0, 416.0];
const LIST_HEADER: [f32; 2] = [128.0 + 36.0, 160.0 + 72.0];
const ONLINE_ROWS: usize = 12;
impl OnlinePage {
    fn enter(&mut self, s: &mut Frontend) {
        self.list.selected = Some(ONLINE_ROWS + 1);
        self.server = None;
        s.joining = None;
    }
    /// Join addresses: LAN games heard by the browser (rewrite extension),
    /// then the address history.
    fn history(s: &Frontend) -> Vec<String> {
        s.lan_games
            .iter()
            .map(|g| g.address.to_string())
            .chain(
                s.settings["history"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned)),
            )
            .take(ONLINE_ROWS)
            .collect()
    }
    /// Column texts for a row: LAN games fill name/level/players/"LAN".
    fn columns(s: &Frontend, row: usize, entry: &str) -> Vec<(String, f32)> {
        match s.lan_games.get(row) {
            Some(g) => vec![
                (g.advert.name.clone(), 0.0),
                (g.advert.level.trim_end_matches(".lvl").to_string(), 330.0),
                (
                    format!(
                        "{}/{} \u{00B7} {} open",
                        g.advert.players,
                        g.advert.lobby.max_players,
                        g.advert.open_slots()
                    ),
                    560.0,
                ),
                (
                    if g.advert.private {
                        "LAN, private".into()
                    } else {
                        "LAN".into()
                    },
                    760.0,
                ),
            ],
            None => vec![(entry.to_string(), 0.0)],
        }
    }
    fn layout(&self, s: &Frontend, measure: &mut Measure) -> (Vec<Widget>, Vec<Item>) {
        let mut out = Vec::new();
        let mut items = Vec::new();
        let selected = self.list.selected;
        out.push(
            Widget::text(
                Font::MenuTitle,
                loc(s, "#menu_main_05"),
                [512.0, 96.0],
                TextAlign::Center,
            )
            .color([0.1037, 0.1576, 0.0202, 1.0]),
        );
        out.push(Widget::fill(
            [LIST_BG[0], LIST_BG[1]],
            [LIST_BG[2], LIST_BG[3]],
            [0.0, 0.0, 0.0, 0.25],
        ));
        for (key, dx) in [
            ("#add_menu_gamename", 0.0),
            ("#menu_vs_13", 330.0),
            ("###", 630.0),
            ("#network_13", 690.0),
        ] {
            let text = if key.starts_with('#') && key != "###" {
                loc(s, key)
            } else {
                key.to_string()
            };
            menu_text_item(
                &mut out,
                measure,
                Font::Maiandra18,
                text,
                [LIST_HEADER[0] + dx, LIST_HEADER[1] + 14.0],
                TextAlign::Left,
                false,
                true,
            );
        }
        let history = Self::history(s);
        for row in 0..ONLINE_ROWS {
            let y = 288.0 + 35.0 * row as f32;
            let entry = history.get(row);
            if let Some(entry) = entry {
                let on = self.server == Some(row) || selected == Some(row);
                for (text, dx) in Self::columns(s, row, entry) {
                    out.push(
                        Widget::text(
                            Font::Maiandra18,
                            text,
                            [150.0 + 14.0 + dx, y + 6.0],
                            TextAlign::Left,
                        )
                        .color(text_color(on, true)),
                    );
                }
            }
            items.push(Item::new(
                [150.0, y],
                [150.0 + 768.0, y + 35.0],
                entry.is_some(),
            ));
        }
        let buttons = [
            (
                "#network_26",
                [LIST_BG[0] + LIST_BG[2] * 0.5, 816.0],
                TextAlign::Center,
            ),
            ("#network_34", [LIST_BG[0], 816.0], TextAlign::Left),
            (
                "#add_menu_ip",
                [LIST_BG[0] + LIST_BG[2], 816.0],
                TextAlign::Right,
            ),
            (
                "#add_menu_join",
                [LIST_BG[0] + LIST_BG[2], BACK_POSITION[1] + 40.0],
                TextAlign::Right,
            ),
        ];
        // Rewrite extension: the role asked for when joining (player,
        // spectator, dungeon master, invader); the host may refuse it.
        let role = format!("Join as: {}", s.join_role.label());
        let buttons = buttons
            .into_iter()
            .map(|(key, p, a)| (key.to_string(), p, a))
            .chain([
                (
                    role,
                    [LIST_BG[0] + LIST_BG[2] * 0.5, 880.0],
                    TextAlign::Center,
                ),
                // Rewrite extension: private games.
                (
                    if s.passphrase.is_some() {
                        "Passphrase: set".to_string()
                    } else {
                        "Passphrase: none".to_string()
                    },
                    // Clear of "Join as", whose longest role is wide.
                    [LIST_BG[0] + LIST_BG[2] * 0.5 + 3.0, BACK_POSITION[1] + 40.0],
                    TextAlign::Center,
                ),
            ]);
        for (i, (key, position, align)) in buttons.enumerate() {
            let text = if key.starts_with('#') {
                loc(s, &key)
            } else {
                key
            };
            let size = measure.text(Font::MenuOption, &text);
            let index = ONLINE_ROWS + i;
            // Quick start and Join need a known server.
            let enabled = match i {
                0 => !history.is_empty(),
                3 => self.server.is_some(),
                _ => true,
            };
            items.push(text_button(
                &mut out,
                &text,
                size,
                position,
                align,
                260.0,
                selected == Some(index),
                enabled,
            ));
        }
        out.push(back_button(selected == Some(ONLINE_ROWS + 6)));
        items.push(back_item());
        (out, items)
    }
    fn join(s: &mut Frontend, tome: &mut TomeView, entry: &str) {
        // The browser keeps listening: the join page shows the server's
        // beacon (players, roles) while a profile and role are chosen.
        s.join_entry = Some(entry.to_string());
        if let Some((host, port)) = entry.rsplit_once(':') {
            s.host = host.trim_matches(['[', ']']).into();
            if let Ok(port) = port.parse() {
                s.port = port;
            }
        } else {
            s.host = entry.into();
        }
        // Protocol 11: pick, name and dress the profile before connecting;
        // the host admits it before any wizard is created.
        let advert = s
            .lan_games
            .iter()
            .find(|g| g.address.to_string() == entry)
            .map(|g| {
                format!(
                    "{} - {} ({}/{})",
                    g.advert.name,
                    g.advert.level.trim_end_matches(".lvl"),
                    g.advert.players,
                    g.advert.lobby.max_players
                )
            });
        s.joining = Some(advert.unwrap_or_else(|| entry.to_string()));
        tome.push(MenuId::Character, 3);
    }
    fn update(
        &mut self,
        s: &mut Frontend,
        tome: &mut TomeView,
        input: &PageInput,
        measure: &mut Measure,
    ) {
        // LAN discovery while the page is open.
        let lan = s
            .lan
            .get_or_insert_with(|| {
                magicka_network::discovery::LanBrowser::bind(
                    magicka_network::discovery::DISCOVERY_PORT,
                )
            })
            .poll(std::time::Instant::now());
        s.lan_games = lan;
        if input.back {
            s.lan = None;
            s.lan_games.clear();
            tome.pop();
            return;
        }
        let (_, items) = self.layout(s, measure);
        let Some(activated) = self.list.update(&items, input) else {
            return;
        };
        let history = Self::history(s);
        match activated {
            row if row < ONLINE_ROWS => self.server = Some(row),
            // Quick start: the most recent server.
            r if r == ONLINE_ROWS => {
                if let Some(entry) = history.first() {
                    Self::join(s, tome, entry);
                }
            }
            // Host: a campaign for others to join (OnHostGame → save slots).
            r if r == ONLINE_ROWS + 1 => {
                s.online = true;
                tome.push(MenuId::SaveSlot, 3);
            }
            r if r == ONLINE_ROWS + 2 => {
                let options = vec![loc(s, "#add_menu_ok"), loc(s, "#add_menu_cancel")];
                let current = if s.host.is_empty() {
                    String::new()
                } else {
                    format!("{}:{}", s.host, s.port)
                };
                s.popup = Some(MessageBox::text_input(
                    Purpose::JoinAddress,
                    loc(s, "#add_menu_ip"),
                    current,
                    options,
                ));
            }
            r if r == ONLINE_ROWS + 3 => {
                if let Some(entry) = self.server.and_then(|i| history.get(i)) {
                    Self::join(s, tome, &entry.clone());
                }
            }
            r if r == ONLINE_ROWS + 4 => {
                use magicka_network::lobby::Role;
                let at = Role::ALL
                    .iter()
                    .position(|r| *r == s.join_role)
                    .unwrap_or(0);
                s.join_role = Role::ALL[(at + 1) % Role::ALL.len()];
            }
            r if r == ONLINE_ROWS + 5 => {
                let options = vec![loc(s, "#add_menu_ok"), loc(s, "#add_menu_cancel")];
                s.popup = Some(MessageBox::text_input(
                    Purpose::Passphrase,
                    "Passphrase for private games (empty for an open game)".into(),
                    s.passphrase.clone().unwrap_or_default(),
                    options,
                ));
            }
            _ => tome.pop(),
        }
    }
    fn draw(&self, s: &Frontend, draw: &mut PageDraw, measure: &mut Measure) {
        let (out, _) = self.layout(s, measure);
        let mut page = draw.right(MenuId::Online);
        for widget in out {
            page.push(widget);
        }
    }
}

/// The native submenus' state.
#[derive(Resource, Default)]
pub(crate) struct Pages {
    last: Option<MenuId>,
    pub slots: SaveSlotsPage,
    pub character: CharacterPage,
    pub options: OptionsPage,
    pub sound: SoundPage,
    pub online: OnlinePage,
    pub game: super::settings_pages::SettingsPage,
    pub graphics: super::settings_pages::SettingsPage,
    pub controls: super::controls_pages::ControlsPage,
    pub keyboard: super::controls_pages::KeyboardPage,
    pub dlc: super::dlc_page::DlcPage,
    pub challenge: super::ruleset_pages::ChallengePage,
    pub versus: super::ruleset_pages::VersusPage,
}

/// OnEnter when a page becomes current; input for the settled page; draw
/// every page the Tome shows (including both sides of a turning page).
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    mut state: ResMut<Frontend>,
    mut tome: ResMut<TomeView>,
    input: Res<PageInput>,
    mut pages: ResMut<Pages>,
    mut draw: ResMut<PageDraw>,
    mut assets: ResMut<super::book::BookAssets>,
    mut images: ResMut<Assets<Image>>,
    time: Res<Time>,
    mut exit: MessageWriter<bevy::app::AppExit>,
    mut cues: ResMut<super::sound::MenuCues>,
    mut devices: ResMut<crate::controller::LocalDevices>,
) {
    let s = &mut *state;
    if !s.message.is_empty() && s.popup.is_none() {
        s.popup = Some(MessageBox::new(
            Purpose::Notice,
            std::mem::take(&mut s.message),
            vec![loc(s, "#add_menu_ok")],
        ));
    }
    let pages = &mut *pages;
    // DialogManager: an open message box takes the menu input, including on the
    // frame it closes. The click on its button must not reach the page beneath.
    let mut popup = s.popup.take();
    let blocking = popup.as_ref().is_some_and(|p| !p.killed);
    if let Some(open) = popup.as_mut() {
        let mut measure = Measure {
            assets: &mut assets,
            images: &mut images,
        };
        if let Some(choice) = open.update(&input, time.delta_secs(), &mut measure) {
            match (open.purpose, choice) {
                (Purpose::Quit, 0) => {
                    exit.write(bevy::app::AppExit::Success);
                }
                (Purpose::JoinAddress, 0) => {
                    let entry = open.input.clone().unwrap_or_default();
                    if !entry.trim().is_empty() {
                        OnlinePage::join(s, &mut tome, entry.trim());
                    }
                }
                (Purpose::Passphrase, 0) => {
                    let typed = open.input.clone().unwrap_or_default();
                    let typed = typed.trim();
                    let least = magicka_network::security::MIN_PASSPHRASE_CHARS;
                    if !typed.is_empty() && typed.chars().count() < least {
                        s.message = format!("The passphrase needs at least {least} characters.");
                    } else {
                        s.passphrase = (!typed.is_empty()).then(|| typed.to_string());
                    }
                }
                (Purpose::ProfileName { profile, created }, choice) => {
                    let text = open.input.clone().unwrap_or_default();
                    pages.character.name_answered(
                        s,
                        profile,
                        created,
                        (choice == 0).then_some(text.as_str()),
                    );
                }
                (Purpose::DeleteSlot(slot), 0) => {
                    if let Err(e) = s.store.delete_slot(slot) {
                        s.message = e;
                    }
                    pages.slots.enter(s);
                    pages.slots.list.selected = Some((slot - slot_base(s)) as usize + 3);
                }
                _ => {}
            }
        }
        open.draw(&input, &mut draw, &mut measure);
    }
    s.popup = popup.filter(|p| !p.dead());
    let current = tome.current();
    if matches!(s.session, super::Session::Idle) {
        devices.occupied = s.seated_pads.clone();
        if current == Some(MenuId::Character) {
            if !devices.locked {
                devices.primary = input.controller;
                devices.locked = true;
            }
        } else {
            *devices = default();
        }
    }
    if current != pages.last {
        pages.last = current;
        match current {
            Some(MenuId::SaveSlot) => pages.slots.enter(s),
            Some(MenuId::Character) => {
                s.continue_primary = None;
                pages.character.enter(s);
            }
            Some(MenuId::Options) => pages.options.enter(),
            Some(MenuId::Sound) => pages.sound.enter(s),
            Some(MenuId::Online) => pages.online.enter(s),
            Some(MenuId::GameOptions) => pages.game.enter(s, &super::settings_pages::GAME),
            Some(MenuId::Graphics) => pages.graphics.enter(s, &super::settings_pages::GRAPHICS),
            Some(MenuId::Controls) => pages.controls.enter(),
            Some(MenuId::Keyboard) => pages.keyboard.enter(),
            Some(MenuId::Dlc) => pages.dlc.enter(),
            Some(MenuId::Challenge) => pages.challenge.enter(s),
            Some(MenuId::Versus) => pages.versus.enter(s),
            _ => {}
        }
    }
    let mut measure = Measure {
        assets: &mut assets,
        images: &mut images,
    };
    // The hidden Tome stays open under a session: Space/Enter in play must
    // not reach the page (the lobby's Start would launch a second session).
    let idle = matches!(s.session, super::Session::Idle);
    if tome.interactive() && !blocking && idle {
        match current {
            Some(MenuId::SaveSlot) => pages.slots.update(s, &mut tome, &input),
            Some(MenuId::Character) => pages.character.update(s, &mut tome, &input, &mut measure),
            Some(MenuId::Options) => pages.options.update(s, &mut tome, &input, &mut measure),
            Some(MenuId::Online) => pages.online.update(s, &mut tome, &input, &mut measure),
            Some(MenuId::GameOptions) => pages.game.update(
                s,
                &super::settings_pages::GAME,
                &mut tome,
                &input,
                &mut measure,
            ),
            Some(MenuId::Controls) => pages.controls.update(s, &mut tome, &input, &mut measure),
            Some(MenuId::Keyboard) => pages.keyboard.update(s, &mut tome, &input, &mut measure),
            Some(MenuId::Dlc) => pages.dlc.update(s, &mut tome, &input, &mut measure),
            Some(MenuId::Challenge) => pages.challenge.update(s, &mut tome, &input, &mut measure),
            Some(MenuId::Versus) => pages.versus.update(s, &mut tome, &input, &mut measure),
            Some(MenuId::Graphics) => pages.graphics.update(
                s,
                &super::settings_pages::GRAPHICS,
                &mut tome,
                &input,
                &mut measure,
            ),
            Some(MenuId::Sound) => {
                pages
                    .sound
                    .update(s, &mut tome, &input, input.held, &mut measure)
            }
            _ => {}
        }
    }
    // SubMenuEndGame.Update: once the camera settles, turn back, or (a
    // campaign) go on to the next chapter's map.
    if current == Some(MenuId::EndGame)
        && tome
            .tome
            .as_ref()
            .is_some_and(|t| t.state == TomeState::Open && !t.camera.is_playing())
    {
        if std::mem::take(&mut s.continue_campaign) {
            super::launch_game(s, false, &mut tome);
        } else {
            tome.pop();
        }
    }
    for list in [
        &mut pages.slots.list,
        &mut pages.character.list,
        &mut pages.options.list,
        &mut pages.sound.list,
        &mut pages.online.list,
    ] {
        list.cues(&mut cues.0);
    }
    // Transitions.CrossFade from the captured game frame.
    if s.end_fade > 0.0 {
        s.end_fade = (s.end_fade - time.delta_secs()).max(0.0);
        if let Some(shot) = &s.end_shot {
            draw.screen().push(
                Widget::texture(shot.clone(), [0.0, 0.0, 1.0, 1.0], [0.0, 0.0], input.screen)
                    .color([1.0, 1.0, 1.0, s.end_fade]),
            );
        }
    }
    let shown: Vec<MenuId> = tome
        .tome
        .as_ref()
        .map(|t| t.draws().iter().map(|d| d.menu).collect())
        .unwrap_or_default();
    for menu in shown {
        match menu {
            MenuId::SaveSlot => pages.slots.draw(s, &mut draw, &mut measure),
            MenuId::Character => pages.character.draw(s, &mut draw, &mut measure),
            MenuId::Options => pages.options.draw(s, &mut draw, &mut measure),
            MenuId::Sound => pages.sound.draw(s, &mut draw, &mut measure),
            MenuId::Online => pages.online.draw(s, &mut draw, &mut measure),
            MenuId::GameOptions => {
                pages
                    .game
                    .draw(s, &super::settings_pages::GAME, &mut draw, &mut measure)
            }
            MenuId::Controls => pages.controls.draw(s, &mut draw, &mut measure),
            MenuId::Keyboard => pages.keyboard.draw(s, &mut draw, &mut measure),
            MenuId::Dlc => pages.dlc.draw(s, &mut draw, &mut measure),
            MenuId::Challenge => pages.challenge.draw(s, &mut draw, &mut measure),
            MenuId::Versus => pages.versus.draw(s, &mut draw, &mut measure),
            MenuId::Graphics => {
                pages
                    .graphics
                    .draw(s, &super::settings_pages::GRAPHICS, &mut draw, &mut measure)
            }
            MenuId::EndGame => {
                // SubMenuEndGame.Draw: the capture, cropped to 800x640.
                if let Some(shot) = &s.end_shot {
                    let aspect = measure
                        .images
                        .get(shot)
                        .map(|i| i.width() as f32 / i.height().max(1) as f32)
                        .unwrap_or(1.25);
                    let width = (1.25 / aspect).min(1.0);
                    draw.right(MenuId::EndGame).push(Widget::texture(
                        shot.clone(),
                        [(1.0 - width) * 0.5, 0.0, width, 1.0],
                        [112.0, 192.0],
                        [800.0, 640.0],
                    ));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<Item> {
        vec![
            Item::new([0.0, 0.0], [10.0, 10.0], true),
            Item::new([0.0, 20.0], [10.0, 30.0], false),
            Item::new([0.0, 40.0], [10.0, 50.0], true),
        ]
    }

    #[test]
    fn up_and_down_wrap_and_skip_disabled_items() {
        let mut list = List::default();
        list.down(&items());
        assert_eq!(list.selected, Some(0));
        list.down(&items());
        assert_eq!(list.selected, Some(2), "disabled item 1 is skipped");
        list.down(&items());
        assert_eq!(list.selected, Some(0), "wraps to the start");
        list.up(&items());
        assert_eq!(list.selected, Some(2), "wraps to the end");
    }

    #[test]
    fn hover_follows_screen_motion_and_release_clicks() {
        let mut list = List {
            selected: Some(0),
            ..default()
        };
        let mut input = PageInput {
            pointer: Some([5.0, 45.0]),
            ..default()
        };
        // The picked point alone (camera drift) does not move the selection.
        assert_eq!(list.update(&items(), &input), None);
        assert_eq!(list.selected, Some(0));
        input.screen_moved = true;
        assert_eq!(list.update(&items(), &input), None);
        assert_eq!(list.selected, Some(2));
        let mut cues = Vec::new();
        list.cues(&mut cues);
        assert_eq!(cues, ["ui_menu_mouseover"]);
        input.released = true;
        assert_eq!(list.update(&items(), &input), Some(2));
        cues.clear();
        list.cues(&mut cues);
        assert_eq!(cues, ["ui_menu_select"]);
        // Disabled items never take the pointer.
        input.pointer = Some([5.0, 25.0]);
        assert_eq!(list.update(&items(), &input), None);
    }

    struct Lobby {
        state: Frontend,
        page: CharacterPage,
        tome: TomeView,
        assets: super::super::book::BookAssets,
        images: Assets<Image>,
        root: std::path::PathBuf,
    }
    impl Lobby {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("magicka-lobby-{}", new_player_id().unwrap()));
            let state = Frontend {
                controller_prompt_override: None,
                controller_prompt_request: None,
                controller_device_present: false,
                strings: default(),
                cursor: default(),
                popup: None,
                content: String::new(),
                cache: root.to_string_lossy().into(),
                store: magicka_runtime::saves::SaveStore::new(root.join("saves")),
                settings: json!({"version":1,"music":0.8,"sfx":1.0,"history":[],"profiles":[
                    {"id":"00000000000000000000000000000001", "name":"First", "robe":"Wizard", "colour":[1.0,1.0,1.0]},
                    {"id":"00000000000000000000000000000002", "name":"Second", "robe":"Robe1", "colour":[1.0,1.0,1.0]}
                ]}),
                page: 9,
                slot: 0,
                profile: 0,
                campaign: "test.lvl".into(),
                robes: vec![],
                online: false,
                fresh: true,
                host: String::new(),
                port: 27015,
                message: String::new(),
                launch: None,
                pending_launch: None,
                intro_pending: false,
                hide_menu_after: None,
                map_previews: default(),
                cutscene_level: None,
                mode: super::super::GameMode::Campaign,
                versus: None,
                dlc: default(),
                packs: None,
                continue_campaign: false,
                continue_primary: None,
                session: super::super::Session::Idle,
                launch_diagnostics: vec![],
                repeat: 0,
                last_launch: None,
                end_shot: None,
                end_fade: 0.0,
                audit: None,
                audit_events: vec![],
                local_players: vec![],
                seated_pads: default(),
                lobby_spec: None,
                join_role: magicka_network::lobby::Role::Player,
                passphrase: None,
                joining: None,
                qa_seats: 0,
                qa_add_players: 0,
                host_role: None,
                join_entry: None,
                lan: None,
                lan_games: vec![],
            };
            let mut page = CharacterPage::default();
            page.avatars = (0..8)
                .map(|i| Avatar {
                    character: if i == 0 {
                        "Wizard".into()
                    } else {
                        format!("Robe{i}")
                    },
                    thumb: String::new(),
                })
                .collect();
            page.enter(&state);
            Self {
                state,
                page,
                tome: default(),
                assets: super::super::book::BookAssets::new(""),
                images: default(),
                root,
            }
        }
        fn input(&mut self, input: PageInput) {
            self.page.update(
                &mut self.state,
                &mut self.tome,
                &input,
                &mut Measure {
                    assets: &mut self.assets,
                    images: &mut self.images,
                },
            );
        }
        fn click(&mut self, point: [f32; 2]) {
            self.input(PageInput {
                pointer: Some(point),
                screen_moved: true,
                released: true,
                ..default()
            });
        }
        fn click_choice(&mut self, index: usize) {
            let (_, item) = self
                .page
                .choices(2)
                .into_iter()
                .find(|(i, _)| *i == index)
                .unwrap();
            self.click([
                (item.min[0] + item.max[0]) * 0.5,
                (item.min[1] + item.max[1]) * 0.5,
            ]);
        }
    }
    impl Drop for Lobby {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn new_profiles_are_named_before_saving_and_can_be_renamed() {
        let mut lobby = Lobby::new();
        lobby.click([800.0, 150.0]);
        assert_eq!(lobby.page.state, SlotState::ChoosingGamer);
        // "Create profile" opens the name prompt instead of a silent name.
        lobby.click_choice(2);
        let prompt = |lobby: &Lobby| match lobby.state.popup.as_ref().map(|p| p.purpose) {
            Some(Purpose::ProfileName { profile, created }) => Some((profile, created)),
            _ => None,
        };
        assert_eq!(prompt(&lobby), Some((2, true)));
        assert_eq!(lobby.page.state, SlotState::ChoosingGamer);
        let saved = |lobby: &Lobby| {
            lobby
                .state
                .store
                .settings()
                .ok()
                .and_then(|v| v["profiles"].as_array().map(Vec::len))
        };
        assert_ne!(saved(&lobby), Some(3), "unnamed profile was saved");
        // Cancelling discards the unnamed profile.
        lobby.state.popup = None;
        lobby.page.name_answered(&mut lobby.state, 2, true, None);
        assert_eq!(
            lobby.state.settings["profiles"].as_array().unwrap().len(),
            2
        );
        assert_eq!(lobby.state.profile, 0);
        // A blank name is refused with a reason and asked again.
        lobby.click_choice(2);
        lobby.state.popup = None;
        lobby
            .page
            .name_answered(&mut lobby.state, 2, true, Some("   "));
        assert_eq!(prompt(&lobby), Some((2, true)));
        lobby.state.popup = None;
        lobby
            .page
            .name_answered(&mut lobby.state, 2, true, Some("  Zed the Grey "));
        assert_eq!(lobby.state.settings["profiles"][2]["name"], "Zed the Grey");
        assert_eq!(lobby.state.profile, 2);
        assert_eq!(lobby.page.state, SlotState::CustomizingAvatar);
        assert_eq!(saved(&lobby), Some(3));
        let id = lobby.state.settings["profiles"][2]["id"].clone();
        // Renaming keeps the persistent identity.
        lobby.page.state = SlotState::Ready;
        CharacterPage::ask_name(&mut lobby.state, 2, false, None);
        assert_eq!(prompt(&lobby), Some((2, false)));
        lobby.state.popup = None;
        lobby
            .page
            .name_answered(&mut lobby.state, 2, false, Some("Zed"));
        assert_eq!(lobby.state.settings["profiles"][2]["name"], "Zed");
        assert_eq!(lobby.state.settings["profiles"][2]["id"], id);
        assert_eq!(lobby.page.state, SlotState::Ready);
        assert!(clean_profile_name(&"x".repeat(33)).is_err());
        assert!(clean_profile_name("a\u{7}b").is_err());
    }

    #[test]
    fn the_passphrase_goes_only_to_online_launches_and_never_to_settings() {
        let has = |args: &[String]| args.iter().any(|a| a == "--passphrase");
        let mut lobby = Lobby::new();
        lobby.state.passphrase = Some("correct-horse-battery".into());
        super::super::launch_game(&mut lobby.state, false, &mut lobby.tome);
        assert!(!has(lobby.state.launch.as_ref().unwrap()), "offline play");

        let mut lobby = Lobby::new();
        lobby.state.passphrase = Some("correct-horse-battery".into());
        lobby.state.online = true;
        super::super::launch_game(&mut lobby.state, false, &mut lobby.tome);
        assert!(has(lobby.state.launch.as_ref().unwrap()), "hosting");
        assert!(!lobby.state.settings.to_string().contains("correct-horse"));

        let mut lobby = Lobby::new();
        lobby.state.passphrase = Some("correct-horse-battery".into());
        lobby.state.host = "127.0.0.1".into();
        super::super::launch_game(&mut lobby.state, true, &mut lobby.tome);
        assert!(has(lobby.state.launch.as_ref().unwrap()), "joining");
    }

    #[test]
    fn chapter_launch_hands_primary_owner_to_play_once() {
        for owner in ["keyboard", "4294967294"] {
            let mut lobby = Lobby::new();
            lobby.state.continue_primary = Some(owner.into());
            super::super::launch_game(&mut lobby.state, false, &mut lobby.tome);
            let args = lobby.state.launch.as_ref().expect("chapter launch");
            assert!(args
                .windows(2)
                .any(|a| a[0] == "--local-primary" && a[1] == owner));
            assert!(lobby.state.continue_primary.is_none());
        }
    }
    #[test]
    fn joining_picks_the_profile_first_and_launches_as_a_client() {
        let mut lobby = Lobby::new();
        lobby.state.joining = Some("192.0.2.1:27015".into());
        lobby.state.host = "192.0.2.1".into();
        lobby.page.enter(&lobby.state);
        // Only the profile setting: the host's lobby rules are not ours.
        assert_eq!(lobby.page.settings(&lobby.state), vec![Setting::Rename]);
        lobby.click([950.0, START[1]]);
        let args = lobby.state.launch.as_ref().expect("Start joins");
        let at = args.iter().position(|a| a == "--network-join").unwrap();
        assert_eq!(args[at + 1], "192.0.2.1:27015");
        let name = args.iter().position(|a| a == "--name").unwrap();
        assert_eq!(args[name + 1], "First");
    }

    #[test]
    fn lobby_mouse_selects_profile_robe_and_every_colour_then_starts() {
        for colour in 0..PLAYER_COLORS.len() {
            let mut lobby = Lobby::new();
            lobby.click([800.0, 150.0]);
            assert_eq!(lobby.page.state, SlotState::ChoosingGamer);
            lobby.click_choice(1);
            assert_eq!(lobby.state.profile, 1);
            assert_eq!(lobby.page.state, SlotState::CustomizingAvatar);
            lobby.click_choice(3);
            assert_eq!(lobby.state.settings["profiles"][1]["robe"], "Robe3");
            assert_eq!(lobby.page.state, SlotState::CustomizingColor);
            lobby.click_choice(colour);
            assert_eq!(lobby.page.state, SlotState::Ready);
            assert_eq!(
                lobby.state.settings["profiles"][1]["colour"],
                json!(player_color(colour))
            );
            lobby.click([950.0, START[1]]);
            let args = lobby.state.launch.as_ref().expect("Start launches");
            assert!(args
                .windows(2)
                .any(|a| a == ["--profile-id", "00000000000000000000000000000002"]));
            assert!(args.windows(2).any(|a| a == ["--character", "Robe3"]));
            assert!(args.iter().any(|a| a == "--new-campaign"));
        }
    }

    #[test]
    fn lobby_hover_keeps_visible_robe_cells_stable_and_wheel_reaches_last_robe() {
        let mut lobby = Lobby::new();
        lobby.click([800.0, 150.0]);
        lobby.click_choice(0);
        let choices = lobby.page.choices(2);
        let point = [900.0, 145.0];
        lobby.input(PageInput {
            pointer: Some(point),
            screen_moved: true,
            ..default()
        });
        assert_eq!(lobby.page.item, 3);
        assert_eq!(
            lobby.page.scroll, 0,
            "hover must not slide the choices under the cursor"
        );
        assert_eq!(choices[3].1.min, lobby.page.choices(2)[3].1.min);
        for _ in 0..5 {
            lobby.input(PageInput {
                pointer: Some(point),
                wheel: -1.0,
                ..default()
            });
        }
        assert_eq!(lobby.page.scroll, 4);
        lobby.click_choice(7);
        assert_eq!(lobby.state.settings["profiles"][0]["robe"], "Robe7");
    }

    #[test]
    fn lobby_blank_click_does_not_accept_and_start_works_during_each_edit() {
        for state in [
            SlotState::ChoosingGamer,
            SlotState::CustomizingAvatar,
            SlotState::CustomizingColor,
        ] {
            for keyboard in [false, true] {
                let mut lobby = Lobby::new();
                lobby.page.state = state;
                lobby.page.item = 1;
                lobby.page.list.selected = Some(0);
                lobby.click([400.0, 600.0]);
                assert_eq!(lobby.page.state, state, "blank click cannot accept an edit");
                assert!(lobby.state.launch.is_none());
                let start = [950.0, START[1]];
                if keyboard {
                    lobby.input(PageInput {
                        pointer: Some(start),
                        screen_moved: true,
                        ..default()
                    });
                    assert_eq!(lobby.page.list.selected, Some(4));
                    lobby.input(PageInput {
                        confirm: true,
                        ..default()
                    });
                } else {
                    lobby.click(start);
                }
                assert!(
                    lobby.state.launch.is_some(),
                    "Start must work during {state:?}"
                );
            }
        }
    }

    #[test]
    fn lobby_resume_retains_save_and_selected_player_identity() {
        let mut lobby = Lobby::new();
        lobby.state.fresh = false;
        lobby
            .state
            .store
            .save_slot(
                0,
                &json!({"version":1,"players":[{"id":"00000000000000000000000000000002"}]}),
            )
            .unwrap();
        lobby.state.profile = 1;
        lobby.input(PageInput {
            confirm: true,
            ..default()
        });
        let args = lobby.state.launch.as_ref().expect("Enter on Start resumes");
        assert!(!args.iter().any(|a| a == "--new-campaign"));
        assert!(args.windows(2).any(|a| a == ["--save-slot", "0"]));
        assert!(args
            .windows(2)
            .any(|a| a == ["--profile-id", "00000000000000000000000000000002"]));
    }

    fn pads(n: usize) -> Vec<Entity> {
        let mut world = World::new();
        (0..n).map(|_| world.spawn_empty().id()).collect()
    }
    fn press(pad: Entity, edges: crate::menu_input::PadEdges) -> PageInput {
        PageInput {
            pads: vec![(pad, edges)],
            ..default()
        }
    }
    fn start() -> crate::menu_input::PadEdges {
        crate::menu_input::PadEdges {
            start: true,
            ..default()
        }
    }

    fn connected(pads: &[Entity]) -> Vec<(Entity, String)> {
        pads.iter()
            .enumerate()
            .map(|(i, p)| (*p, format!("Pad {i}")))
            .collect()
    }
    /// One frame with these pads connected (and `menu_pad` driving the menu).
    fn frame(pads: &[Entity], menu_pad: Option<Entity>) -> PageInput {
        PageInput {
            connected: connected(pads),
            controller: menu_pad,
            ..default()
        }
    }
    fn click_at(pads: &[Entity], point: [f32; 2]) -> PageInput {
        PageInput {
            pointer: Some(point),
            screen_moved: true,
            released: true,
            ..frame(pads, None)
        }
    }
    fn centre(item: Item) -> [f32; 2] {
        [
            (item.min[0] + item.max[0]) * 0.5,
            (item.min[1] + item.max[1]) * 0.5,
        ]
    }
    fn join_row(page: &CharacterPage) -> Item {
        let row = *page
            .rows()
            .iter()
            .find(|r| r.kind == RowKind::Join)
            .expect("an open slot");
        Item::new([row.x, row.y], [row.x + 448.0, row.y + row.height], true)
    }
    fn launched_primary(lobby: &mut Lobby) -> String {
        lobby.page.launch(&mut lobby.state, &mut lobby.tome);
        let args = lobby.state.launch.clone().expect("launch");
        args.windows(2)
            .find(|a| a[0] == "--local-primary")
            .map(|a| a[1].clone())
            .expect("--local-primary")
    }

    fn back() -> crate::menu_input::PadEdges {
        crate::menu_input::PadEdges {
            back: true,
            ..default()
        }
    }
    fn with_pad(pads: &[Entity], pad: Entity, edges: crate::menu_input::PadEdges) -> PageInput {
        PageInput {
            pads: vec![(pad, edges)],
            ..frame(pads, None)
        }
    }

    #[test]
    fn devices_join_themselves_and_the_lobby_can_be_empty() {
        let mut lobby = Lobby::new();
        let p = pads(2);
        // Opened with the keyboard: it holds player one.
        lobby.input(frame(&p, None));
        assert!(lobby.page.keyboard_seated());
        assert_eq!(lobby.page.local_players(), 1);
        // Start on a controller joins it as player two.
        lobby.input(with_pad(&p, p[0], start()));
        assert_eq!(lobby.page.seats.len(), 1);
        assert_eq!(lobby.page.seats[0].pad, p[0]);
        assert_eq!(lobby.page.seats[0].profile, 1, "the free profile");
        // Keyboard and mouse leave: the controller player moves up.
        lobby
            .page
            .change_setting(&mut lobby.state, Setting::KeyboardSeat, true);
        assert!(lobby.page.seats.is_empty());
        assert_eq!(lobby.page.primary_pad, Some(p[0]));
        assert_eq!(lobby.state.profile, 1, "its profile came along");
        // That pad leaves with Back: the lobby is empty and cannot start.
        lobby.input(with_pad(&p, p[0], back()));
        assert_eq!(lobby.page.local_players(), 0);
        assert!(!lobby.page.can_start(&lobby.state));
        lobby.page.launch(&mut lobby.state, &mut lobby.tome);
        assert!(lobby.state.launch.is_none());
        assert!(lobby.state.message.contains("Join first"));
        lobby.state.message.clear();
        // Start on the other controller now takes player one.
        lobby.input(with_pad(&p, p[1], start()));
        assert_eq!(lobby.page.primary_pad, Some(p[1]));
        assert!(lobby.page.primary_present);
        assert_eq!(launched_primary(&mut lobby), p[1].to_bits().to_string());
    }

    #[test]
    fn keyboard_joins_by_click_and_takes_player_one_from_a_controller() {
        let mut lobby = Lobby::new();
        let pad = pads(1)[0];
        // Opened with the pad: it holds player one.
        lobby.input(frame(&[pad], Some(pad)));
        assert_eq!(lobby.page.primary_pad, Some(pad));
        let pad_profile = lobby.state.profile;
        // Its Start does not seat it twice.
        lobby.input(PageInput {
            pads: vec![(pad, start())],
            ..frame(&[pad], Some(pad))
        });
        assert!(lobby.page.seats.is_empty());
        // Clicking an open slot seats keyboard and mouse as player one; the
        // controller player keeps its profile on the next slot.
        let join = centre(join_row(&lobby.page));
        lobby.input(click_at(&[pad], join));
        lobby.input(frame(&[pad], None));
        assert!(lobby.page.keyboard_seated());
        assert_eq!(lobby.page.seats.len(), 1);
        assert_eq!(lobby.page.seats[0].pad, pad);
        assert_eq!(lobby.page.seats[0].profile, pad_profile);
        assert_ne!(lobby.state.profile, pad_profile);
        assert!(lobby.state.seated_pads.contains(&pad));
        // A second click does nothing, and opens no notification.
        lobby.input(click_at(&[pad], centre(join_row(&lobby.page))));
        assert!(lobby.state.message.is_empty(), "{}", lobby.state.message);
        assert_eq!(lobby.page.seats.len(), 1);
        assert_eq!(launched_primary(&mut lobby), "keyboard");
        let args = lobby.state.launch.clone().unwrap();
        assert!(args.iter().any(|a| a == "--local-players"));
    }

    #[test]
    fn an_empty_player_one_slot_takes_a_click_and_disconnects_free_it() {
        let mut lobby = Lobby::new();
        let pad = pads(1)[0];
        lobby.input(frame(&[pad], Some(pad)));
        // The pad disconnects: player one's slot empties.
        lobby.input(frame(&[], None));
        assert!(!lobby.page.primary_present);
        assert_eq!(
            lobby.page.setting_text(Setting::KeyboardSeat),
            "Keyboard & mouse: join"
        );
        // Clicking the empty first slot seats keyboard and mouse.
        let first = lobby.page.rows()[0];
        let at = centre(Item::new(
            [first.x, first.y],
            [first.x + 448.0, first.y + first.height],
            true,
        ));
        lobby.input(click_at(&[], at));
        assert!(lobby.page.keyboard_seated());
        assert_eq!(
            lobby.page.state,
            SlotState::Ready,
            "the click joined, not edited"
        );
    }

    #[test]
    fn player_ones_role_badge_cycles_and_a_wizardless_host_launches_dedicated() {
        use magicka_network::lobby::Role;
        let mut lobby = Lobby::new();
        lobby.state.online = true;
        lobby.page.enter(&lobby.state);
        let p = pads(1);
        lobby.input(frame(&p, None));
        lobby.input(with_pad(&p, p[0], start()));
        assert_eq!(lobby.page.seats.len(), 1);
        // Click player one's badge: dungeon master; the local player leaves.
        let badge = centre(lobby.page.role_badge(lobby.page.rows()[0]));
        lobby.input(click_at(&p, badge));
        assert_eq!(lobby.page.primary_role, Role::DungeonMaster);
        assert!(lobby.page.seats.is_empty());
        assert_eq!(
            lobby.page.state,
            SlotState::Ready,
            "the badge, not the chooser"
        );
        lobby.state.message.clear();
        // Controllers cannot join a wizardless player one.
        lobby.input(with_pad(&p, p[0], start()));
        assert!(lobby.page.seats.is_empty());
        assert!(lobby.state.message.contains("dungeon master"));
        // Right on the selected first row steps to spectator, then player.
        lobby.page.list.selected = Some(0);
        lobby.input(PageInput {
            direction: [false, false, false, true],
            ..frame(&p, None)
        });
        assert_eq!(lobby.page.primary_role, Role::Spectator);
        lobby.input(click_at(&p, badge));
        assert_eq!(lobby.page.primary_role, Role::Player);
        lobby.input(click_at(&p, badge));
        lobby.page.launch(&mut lobby.state, &mut lobby.tome);
        let args = lobby.state.launch.clone().expect("host launch");
        assert!(args
            .windows(2)
            .any(|a| a == ["--host-role", "dungeonmaster"]));
        assert!(!args.iter().any(|a| a == "--local-players"));
    }

    #[test]
    fn joining_role_comes_from_player_ones_badge_and_offline_only_plays() {
        use magicka_network::lobby::Role;
        let mut lobby = Lobby::new();
        assert_eq!(lobby.page.primary_roles(&lobby.state), [Role::Player]);
        lobby.state.joining = Some("192.0.2.1:27015".into());
        lobby.state.join_role = Role::Spectator;
        lobby.page.enter(&lobby.state);
        lobby.input(frame(&[], None));
        assert_eq!(lobby.page.primary_role, Role::Spectator);
        lobby.page.cycle_role(&mut lobby.state, true);
        assert_eq!(lobby.state.join_role, lobby.page.primary_role);
        assert_eq!(
            lobby.page.primary_roles(&lobby.state).len(),
            4,
            "invader too"
        );
    }

    #[test]
    fn the_join_page_shows_the_servers_beacon_and_offers_its_roles() {
        use magicka_network::discovery::{Advert, AdvertSeat, LanGame};
        use magicka_network::lobby::{LobbySettings, Role};
        let mut lobby = Lobby::new();
        let address: std::net::SocketAddr = "192.0.2.7:27015".parse().unwrap();
        let advert = Advert::new(
            27015,
            "Grav's game",
            "Levels/Havindr.lvl",
            3,
            LobbySettings {
                max_players: 4,
                spectators: true,
                dungeon_masters: false,
                invaders: true,
                ..LobbySettings::default()
            },
        )
        .with_participants([
            AdvertSeat {
                name: "Gravitas".into(),
                role: Role::Player,
                queued: false,
            },
            AdvertSeat {
                name: "Watcher".into(),
                role: Role::Spectator,
                queued: false,
            },
        ]);
        lobby.state.lan_games = vec![LanGame {
            address,
            advert,
            seen: std::time::Instant::now(),
        }];
        lobby.state.join_entry = Some(address.to_string());
        lobby.state.joining = Some("Grav's game".into());
        lobby.state.host = "192.0.2.7".into();
        lobby.state.port = 27015;
        lobby.page.enter(&lobby.state);
        lobby.input(frame(&[], None));
        // The host allows players, spectators and invaders, not DMs.
        assert_eq!(
            lobby.page.primary_roles(&lobby.state),
            [Role::Player, Role::Spectator, Role::Invader]
        );
        let lines = server_lines(&lobby.state);
        assert!(
            lines.contains(&"Players 3/4, 1 open".to_string()),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l == "Gravitas  -  Player"));
        assert!(lines.iter().any(|l| l == "Watcher  -  Spectator"));
        assert!(lines.iter().any(|l| l.contains("spectators, invaders")));
        // One local player joins another computer's game.
        let pad = pads(1)[0];
        lobby.input(with_pad(&[pad], pad, start()));
        assert!(lobby.page.seats.is_empty());
        // Start reads Join and launches the client with the badge's role.
        let mut out = Vec::new();
        let item = lobby.page.start_button(
            &lobby.state,
            &mut out,
            &mut Measure {
                assets: &mut lobby.assets,
                images: &mut lobby.images,
            },
        );
        assert!(item.enabled);
        lobby.page.cycle_role(&mut lobby.state, true);
        assert_eq!(lobby.state.join_role, Role::Spectator);
        lobby.page.launch(&mut lobby.state, &mut lobby.tome);
        let args = lobby.state.launch.clone().expect("join launch");
        assert!(args.windows(2).any(|a| a == ["--role", "spectator"]));
        // A beacon not heard: every role, with a note instead of details.
        lobby.state.lan_games.clear();
        assert_eq!(lobby.page.primary_roles(&lobby.state).len(), 4);
        assert!(server_lines(&lobby.state)[0].contains("LAN"));
    }

    #[test]
    fn slots_show_their_device_icon() {
        let mut lobby = Lobby::new();
        lobby.page.controller_sprites = ["keyboard", "gp1", "gp2", "gp3", "gp4"]
            .iter()
            .enumerate()
            .map(|(i, n)| (n.to_string(), [0.0, 100.0 * i as f32, 128.0, 90.0]))
            .collect();
        let p = pads(3);
        lobby.input(frame(&p, None));
        assert_eq!(
            lobby.page.control_sprite(None),
            Some([0.0, 0.0, 128.0, 90.0])
        );
        // Controller numbers follow the connected order: the third is gp3.
        assert_eq!(
            lobby.page.control_sprite(Some(p[2])),
            Some([0.0, 300.0, 128.0, 90.0])
        );
        // Clicking the slot itself still opens the profile chooser.
        lobby.input(PageInput {
            pointer: Some([800.0, 150.0]),
            screen_moved: true,
            released: true,
            ..frame(&p, None)
        });
        assert_eq!(lobby.page.state, SlotState::ChoosingGamer);
    }

    #[test]
    fn controller_sprites_come_from_the_game_spritesheet() {
        let content = magicka_content::retail_content_dir();
        if !content.is_dir() {
            return;
        }
        let sprites = controller_sprites(&content.to_string_lossy());
        for name in ["keyboard", "gp1", "gp2", "gp3", "gp4"] {
            let rect = sprites.get(name).unwrap_or_else(|| panic!("{name}"));
            assert!(rect[2] > 0.0 && rect[3] > 0.0);
        }
    }

    #[test]
    fn settings_rows_stay_above_back() {
        for rows in 1..12 {
            let last = SETTINGS[1] + settings_step(rows) * (rows - 1) as f32;
            assert!(last <= BACK_POSITION[1] - 16.0 + 1e-3, "{rows} rows");
        }
        assert_eq!(settings_step(5), SETTINGS_STEP);
    }

    #[test]
    fn pads_join_with_start_and_edit_their_own_slot() {
        let mut lobby = Lobby::new();
        let pad = pads(1)[0];
        lobby.input(press(pad, start()));
        assert_eq!(lobby.page.seats.len(), 1);
        // The free profile, not the keyboard player's.
        assert_eq!(lobby.page.seats[0].profile, 1);
        assert!(lobby.state.seated_pads.contains(&pad));
        let confirm = crate::menu_input::PadEdges {
            confirm: true,
            ..default()
        };
        lobby.input(press(pad, confirm));
        assert_eq!(lobby.page.seats[0].editor.state, SlotState::ChoosingGamer);
        // The keyboard player's profile is taken: accepting it does nothing.
        lobby.page.seats[0].editor.item = 0;
        lobby.input(press(pad, confirm));
        assert_eq!(lobby.page.seats[0].editor.state, SlotState::ChoosingGamer);
        lobby.page.seats[0].editor.item = 1;
        lobby.input(press(pad, confirm));
        assert_eq!(
            lobby.page.seats[0].editor.state,
            SlotState::CustomizingAvatar
        );
        let down = crate::menu_input::PadEdges {
            direction: [false, true, false, false],
            ..default()
        };
        lobby.page.seats[0].editor.item = 4;
        lobby.input(press(pad, confirm));
        assert_eq!(lobby.state.settings["profiles"][1]["robe"], "Robe4");
        lobby.input(press(pad, down));
        lobby.input(press(pad, confirm));
        assert_eq!(lobby.page.seats[0].editor.state, SlotState::Ready);
        // Start on a seated pad starts the game with both players.
        lobby.input(press(pad, start()));
        let args = lobby.state.launch.as_ref().expect("seated Start launches");
        let i = args.iter().position(|a| a == "--local-players").unwrap();
        let seats: serde_json::Value = serde_json::from_str(&args[i + 1]).unwrap();
        assert_eq!(seats[0]["pad"], pad.to_bits());
        assert_eq!(seats[0]["robe"], "Robe4");
        assert_eq!(seats[0]["id"], "00000000000000000000000000000002");
        assert!(args
            .windows(2)
            .any(|a| a[0] == "--lobby" && a[1].contains("players=4")));
        // Back on a ready seat leaves.
        let mut lobby = Lobby::new();
        lobby.input(press(pad, start()));
        lobby.input(press(
            pad,
            crate::menu_input::PadEdges {
                back: true,
                ..default()
            },
        ));
        assert!(lobby.page.seats.is_empty());
    }

    #[test]
    fn more_than_four_players_switch_to_compact_scrolling_rows() {
        let mut lobby = Lobby::new();
        let pads = pads(12);
        // The original limit: three pads join, the fourth is refused.
        for pad in &pads[..4] {
            lobby.input(press(*pad, start()));
        }
        assert_eq!(lobby.page.seats.len(), 3);
        assert!(!lobby.page.compact());
        assert_eq!(lobby.page.rows().len(), 4);
        assert_eq!(
            lobby.page.rows()[0].y,
            SLOT_Y,
            "original layout kept at four"
        );
        // Raise the limit through the Players setting (click it).
        let players = [SETTINGS[0] + 5.0, SETTINGS[1]];
        for _ in 0..LIMITS.len() {
            if lobby.page.lobby.max_players >= 12 {
                break;
            }
            lobby.click(players);
        }
        assert_eq!(lobby.page.lobby.max_players, 12);
        assert!(lobby.state.settings["lobby"]
            .as_str()
            .unwrap()
            .contains("players=12"));
        for pad in &pads[4..] {
            lobby.input(press(*pad, start()));
        }
        assert_eq!(lobby.page.seats.len(), 11);
        assert!(lobby.page.compact());
        let rows = lobby.page.rows();
        assert!(rows.len() >= 8, "compact rows fit the slot area");
        assert!(rows.iter().all(|r| r.height == COMPACT_H));
        assert!(rows.last().unwrap().y + COMPACT_H <= WINDOW_BOTTOM);
        assert!(lobby.page.scroll_max() > 0);
        // The newest seat was scrolled into view; the wheel scrolls back.
        assert!(rows.iter().any(|r| r.kind == RowKind::Seat(10)));
        for _ in 0..12 {
            lobby.input(PageInput {
                pointer: Some([COMPACT_X + 100.0, SLOT_Y + 100.0]),
                wheel: 1.0,
                ..default()
            });
        }
        assert_eq!(lobby.page.first_row, 0);
        assert_eq!(lobby.page.rows()[0].kind, RowKind::Primary);
        // Editing the keyboard player opens its row to full size.
        lobby.click([COMPACT_X + 200.0, SLOT_Y + 20.0]);
        assert_eq!(lobby.page.state, SlotState::ChoosingGamer);
        assert_eq!(lobby.page.rows()[0].height, 112.0);
        // A limit below the seated count is skipped.
        lobby.page.state = SlotState::Ready;
        lobby.click(players);
        assert!(usize::from(lobby.page.lobby.max_players) >= 12);
    }

    #[test]
    fn text_boxes_take_typing_and_backspace() {
        let mut lobby = Lobby::new();
        let mut popup = MessageBox::text_input(
            Purpose::JoinAddress,
            "IP".into(),
            "10.0".into(),
            vec!["OK".into(), "Cancel".into()],
        );
        let mut measure = Measure {
            assets: &mut lobby.assets,
            images: &mut lobby.images,
        };
        for typed in [".0.5", "\u{8}", "7:177", " "] {
            let input = PageInput {
                typed: typed.into(),
                ..default()
            };
            assert_eq!(popup.update(&input, 0.016, &mut measure), None);
        }
        assert_eq!(popup.input.as_deref(), Some("10.0.0.7:177"));
        let enter = PageInput {
            confirm: true,
            ..default()
        };
        assert_eq!(popup.update(&enter, 0.016, &mut measure), Some(0));
    }

    #[test]
    fn the_menu_never_offers_more_than_the_in_game_limit() {
        assert_eq!(
            LIMITS.iter().max(),
            Some(&magicka_network::lobby::IN_GAME_PLAYER_LIMIT)
        );
    }
}
