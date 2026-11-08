//! Controls options (keyboard bindings and modern controller prompt families) and
//! SubMenuOptionsKeyboard: the 17 KeyboardBindings in a scrolled list of
//! twelve rows, "???" while a key is caught, and restore defaults. Saved to
//! settings.json on leaving a changed page; play reads them each session.
use super::{
    book::{Atlas, Font, Measure, PageDraw, Widget},
    pages::{
        back_button, back_item, graphic, loc, menu_text_item, Item, List, MessageBox, PageInput,
        Purpose,
    },
    Frontend,
};
use crate::bindings::{self, KeyboardBindings};
use crate::frontend_tome::{MenuId, TomeView};
use magicka_runtime::text_geometry::TextAlign;

// ---------------------------------------------------------------------------
// SubMenuOptionsControls

#[derive(Default)]
pub(crate) struct ControlsPage {
    list: List,
}

impl ControlsPage {
    pub(crate) fn enter(&mut self) {
        self.list.selected = Some(0);
    }
    fn layout(&self, s: &Frontend, measure: &mut Measure) -> (Vec<Widget>, Vec<Item>) {
        let mut out = Vec::new();
        let mut items = Vec::new();
        let selected = self.list.selected;
        out.push(Widget::text(
            Font::MenuTitle,
            loc(s, "#menu_opt_07"),
            [512.0, 96.0],
            TextAlign::Center,
        ));
        let line = measure.line_height(Font::MenuOption) + 10.0;
        let labels = [
            loc(s, "#menu_opt_alt_07"),
            format!("Element input: {}", s.controller_element_style().label()),
        ]
        .into_iter()
        .chain(crate::controller::Family::ALL.map(|f| {
            let selected = s.controller_prompt_family() == Some(f);
            format!("{} prompts{}", f.label(), if selected { " *" } else { "" })
        }));
        for (i, label) in labels.enumerate() {
            items.push(menu_text_item(
                &mut out,
                measure,
                Font::MenuOption,
                label,
                [512.0, 1024.0 * 0.333 + line * i as f32],
                TextAlign::Center,
                selected == Some(i),
                true,
            ));
        }
        out.push(back_button(selected == Some(6)));
        items.push(back_item());
        out.push(graphic(
            [448.0, 976.0, 608.0, 48.0],
            [208.0, 220.0, 608.0, 48.0],
        ));
        (out, items)
    }
    pub(crate) fn update(
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
        let (_, items) = self.layout(s, measure);
        match self.list.update(&items, input) {
            Some(0) => tome.push(MenuId::Keyboard, 2),
            Some(6) => tome.pop(),
            Some(1) => s.set_controller_element_style(s.controller_element_style().other()),
            Some(i @ 2..=5) => {
                let family = crate::controller::Family::ALL[i - 2];
                let [attack, boost, interact, special] = family.face_names();
                if s.controller_device_present {
                    s.controller_prompt_request =
                        Some(if s.controller_prompt_family() == Some(family) {
                            None
                        } else {
                            Some(family)
                        });
                }
                let ok = loc(s, "#add_menu_ok");
                s.popup = Some(MessageBox::new(
                    Purpose::Notice,
                    format!("{} prompts for the last active controller. Select again for automatic detection.\nLeft stick: move and face.\n{}\n{interact}: interact. {attack}: weapon. {special}: self cast / staff. {boost}: boost / magick.\nRight trigger: cast. Left trigger: area cast. Right bumper: block.\nView / Create: inventory. Menu / Options: pause.\nSteam Input: use Gamepad template; right pad as right joystick.", family.label(),s.controller_element_style().instructions()),
                    vec![ok],
                ));
            }
            Some(_) => {}
            None => {}
        }
    }
    pub(crate) fn draw(&self, s: &Frontend, draw: &mut PageDraw, measure: &mut Measure) {
        let (out, _) = self.layout(s, measure);
        let mut page = draw.right(MenuId::Controls);
        for widget in out {
            page.push(widget);
        }
    }
}

// ---------------------------------------------------------------------------
// MenuScrollBar

/// MenuScrollBar: a 64-pixel wide bar of `height` centred on `position`,
/// `max` steps, drawn from the pages atlas.
pub(crate) struct ScrollBar {
    pub position: [f32; 2],
    pub height: f32,
    pub max: i32,
}

impl ScrollBar {
    fn track(&self) -> f32 {
        self.height - 128.0
    }
    fn step(&self) -> f32 {
        self.track() / self.max.max(1) as f32
    }
    fn top(&self) -> f32 {
        self.position[1] - self.height * 0.5
    }
    fn bottom(&self) -> f32 {
        self.position[1] + self.height * 0.5
    }
    pub(crate) fn bounds(&self) -> Item {
        Item::new(
            [self.position[0] - 32.0, self.top()],
            [self.position[0] + 32.0, self.bottom()],
            true,
        )
    }
    /// -1 for the up arrow, 1 for the down arrow, None on the track.
    pub(crate) fn arrow(&self, p: [f32; 2]) -> Option<i32> {
        if (p[0] - self.position[0]).abs() > 32.0 {
            return None;
        }
        if (self.top()..self.top() + 64.0).contains(&p[1]) {
            Some(-1)
        } else if (self.bottom() - 64.0..=self.bottom()).contains(&p[1]) {
            Some(1)
        } else {
            None
        }
    }
    /// ScrollTo.
    pub(crate) fn value_at(&self, y: f32) -> i32 {
        ((0.5 + (y - self.top() - 64.0) / self.step()) as i32).clamp(0, self.max)
    }
    pub(crate) fn draw(&self, out: &mut Vec<Widget>, value: i32) {
        let x = self.position[0];
        // The track, stretched between the end caps.
        let stretch = (self.track() - 32.0) / 64.0;
        out.push(Widget::sprite(
            Atlas::Pages,
            [1216.0, 64.0, 64.0, 64.0],
            [32.0, 32.0],
            [1.0, 0.0, 0.0, stretch],
            self.position,
        ));
        out.push(Widget::sprite(
            Atlas::Pages,
            [1216.0, 32.0, 64.0, 32.0],
            [32.0, 16.0],
            [1.0, 0.0, 0.0, 1.0],
            [x, self.top() + 64.0],
        ));
        out.push(Widget::sprite(
            Atlas::Pages,
            [1216.0, 128.0, 64.0, 32.0],
            [32.0, 16.0],
            [1.0, 0.0, 0.0, 1.0],
            [x, self.bottom() - 64.0],
        ));
        // Up arrow, then the same sprite turned for down; the knob keeps
        // the turned transform (as the native draw leaves it).
        out.push(Widget::sprite(
            Atlas::Pages,
            [1280.0, 96.0, 64.0, 64.0],
            [32.0, 32.0],
            [1.0, 0.0, 0.0, 1.0],
            [x, self.top() + 32.0],
        ));
        out.push(Widget::sprite(
            Atlas::Pages,
            [1280.0, 96.0, 64.0, 64.0],
            [32.0, 32.0],
            [-1.0, 0.0, 0.0, -1.0],
            [x, self.bottom() - 32.0],
        ));
        out.push(Widget::sprite(
            Atlas::Pages,
            [1280.0, 64.0, 64.0, 64.0],
            [32.0, 32.0],
            [-1.0, 0.0, 0.0, -1.0],
            [
                x,
                self.position[1] - self.track() * 0.5 + value as f32 * self.step(),
            ],
        ));
    }
}

// ---------------------------------------------------------------------------
// SubMenuOptionsKeyboard

/// Rows in KeyboardBindings order: labels, some joined with '/'.
const ROWS: [&[&str]; bindings::COUNT] = [
    &["#element_water"],
    &["#element_life"],
    &["#element_shield"],
    &["#element_cold"],
    &["#element_lightning"],
    &["#element_arcane"],
    &["#element_earth"],
    &["#element_fire"],
    &["#menu_ctrl_01", "#menu_ctrl_02"],
    &["#menu_ctrl_03"],
    &["#ctrl_kb_boost", "#ctrl_kb_cast_magick"],
    &["#ctrl_move", "#menu_ctrl_09"],
    &["#ctrl_kb_block"],
    &["#ctrl_kb_shift"],
    &["#ctrl_kb_inventory"],
    &["#ctrl_magick_prev"],
    &["#ctrl_magick_next"],
];
const VISIBLE: usize = 12;
const DEFAULTS: usize = bindings::COUNT;
const BACK: usize = bindings::COUNT + 1;

#[derive(Default)]
pub(crate) struct KeyboardPage {
    list: List,
    scroll: i32,
    /// The function whose key is being caught (ActivateCatchKeyIndicator).
    catching: Option<usize>,
    changed: bool,
    grabbed: bool,
}

impl KeyboardPage {
    pub(crate) fn enter(&mut self) {
        self.list.selected = Some(0);
        self.scroll = 0;
        self.catching = None;
        self.changed = false;
        self.grabbed = false;
    }
    fn scroll_bar(measure: &mut Measure) -> ScrollBar {
        ScrollBar {
            position: [928.0, 512.0],
            height: measure.line_height(Font::MenuOption) * 13.0,
            max: (bindings::COUNT - VISIBLE) as i32,
        }
    }
    fn layout(&self, s: &Frontend, measure: &mut Measure) -> (Vec<Widget>, Vec<Item>) {
        let mut out = Vec::new();
        let mut items = Vec::new();
        let selected = self.list.selected;
        let keys = KeyboardBindings::from_settings(&s.settings);
        out.push(Widget::text(
            Font::MenuTitle,
            loc(s, "#menu_opt_11"),
            [512.0, 96.0],
            TextAlign::Center,
        ));
        let line = measure.line_height(Font::MenuOption);
        let top = 1024.0 * 0.333 - line * 0.5;
        let first = self.scroll as usize;
        for (i, labels) in ROWS.iter().enumerate() {
            let visible = (first..first + VISIBLE).contains(&i);
            if !visible {
                // Off the page: reachable by keyboard (the list scrolls),
                // never hovered.
                items.push(Item::new([-1e4, -1e4], [-1e4, -1e4], true));
                continue;
            }
            let y = top + line * (i - first) as f32;
            let on = selected == Some(i);
            let label = labels
                .iter()
                .map(|k| loc(s, k))
                .collect::<Vec<_>>()
                .join("/");
            let mut value = bindings::name(keys.0[i], |k| loc(s, k));
            if self.catching == Some(i) {
                value.push_str(" ???");
            }
            let a = menu_text_item(
                &mut out,
                measure,
                Font::MenuOption,
                label,
                [492.0, y],
                TextAlign::Right,
                on,
                true,
            );
            let b = menu_text_item(
                &mut out,
                measure,
                Font::MenuOption,
                value,
                [532.0, y],
                TextAlign::Left,
                on,
                true,
            );
            items.push(Item::new(
                [a.min[0].min(b.min[0]), a.min[1]],
                [a.max[0].max(b.max[0]), a.max[1]],
                true,
            ));
        }
        items.push(menu_text_item(
            &mut out,
            measure,
            Font::MenuOption,
            loc(s, "#menu_restore_d"),
            [512.0, top + line * VISIBLE as f32 + 20.0],
            TextAlign::Center,
            selected == Some(DEFAULTS),
            true,
        ));
        out.push(back_button(selected == Some(BACK)));
        items.push(back_item());
        out.push(graphic(
            [448.0, 976.0, 608.0, 48.0],
            [208.0, 220.0, 608.0, 48.0],
        ));
        Self::scroll_bar(measure).draw(&mut out, self.scroll);
        (out, items)
    }
    /// OnExit: save changed bindings.
    fn exit(&self, s: &mut Frontend) {
        if self.changed {
            if let Err(e) = s.store.save_settings(&s.settings) {
                s.message = e;
            }
        }
    }
    pub(crate) fn update(
        &mut self,
        s: &mut Frontend,
        tome: &mut TomeView,
        input: &PageInput,
        measure: &mut Measure,
    ) {
        // KeyboardMouseController's catch takes every key and button until
        // one binds (Escape and the Windows keys never do).
        // Controller Back cancels capture; keyboard Escape retains the recovered
        // IsValidKey behavior (it is captured but never binds).
        if self.catching.is_some() && input.back && input.captured.is_none() {
            self.catching = None;
            return;
        }
        if let Some(function) = self.catching {
            if let Some(binding) = input.captured {
                let mut keys = KeyboardBindings::from_settings(&s.settings);
                if keys.bind(function, binding) {
                    s.settings["keyboard"] = keys.to_settings();
                    self.catching = None;
                    self.changed = true;
                }
            }
            return;
        }
        if input.back {
            self.exit(s);
            tome.pop();
            return;
        }
        let bar = Self::scroll_bar(measure);
        if input.wheel != 0.0 {
            self.scroll = (self.scroll - input.wheel.signum() as i32).clamp(0, bar.max);
        }
        if !input.held {
            self.grabbed = false;
        }
        if let Some(p) = input.pointer {
            if self.grabbed {
                self.scroll = bar.value_at(p[1]);
            } else if input.pressed && bar.bounds().inside(p) {
                match bar.arrow(p) {
                    Some(d) => self.scroll = (self.scroll + d).clamp(0, bar.max),
                    None => {
                        self.scroll = bar.value_at(p[1]);
                        self.grabbed = true;
                    }
                }
            }
        }
        let (_, items) = self.layout(s, measure);
        let chosen = self.list.update(&items, input);
        // Keep the keyboard selection on the page.
        if let Some(i) = self.list.selected.filter(|i| *i < bindings::COUNT) {
            let first = self.scroll as usize;
            if i < first {
                self.scroll = i as i32;
            } else if i >= first + VISIBLE {
                self.scroll = (i + 1 - VISIBLE) as i32;
            }
        }
        match chosen {
            Some(BACK) => {
                self.exit(s);
                tome.pop();
            }
            Some(DEFAULTS) => {
                // KeyboardMouseController.LoadDefaults.
                s.settings["keyboard"] = KeyboardBindings::default().to_settings();
                self.changed = true;
            }
            Some(i) => self.catching = Some(i),
            None => {}
        }
    }
    pub(crate) fn draw(&self, s: &Frontend, draw: &mut PageDraw, measure: &mut Measure) {
        let (out, _) = self.layout(s, measure);
        let mut page = draw.right(MenuId::Keyboard);
        for widget in out {
            page.push(widget);
        }
    }
}
