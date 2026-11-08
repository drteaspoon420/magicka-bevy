//! KeyboardMouseController's KeyboardBindings: the 17 bindable functions,
//! LoadDefaults, KeyToString and IsValidKey's conflict rule. Bindings are
//! stored in settings.json as XNA key codes or mouse button numbers, and
//! read by play for each session.
use bevy::prelude::*;
use serde_json::{json, Value};

/// KeyboardBindings: Water, Life, Shield, Cold, Lightning, Arcane, Earth,
/// Fire, Cast, CastSelf, Boost, Move, Block, Shift, Inventory, PrevMagick,
/// NextMagick.
pub(crate) const COUNT: usize = 17;
pub(crate) const CAST: usize = 8;
pub(crate) const CAST_SELF: usize = 9;
pub(crate) const BOOST: usize = 10;
pub(crate) const MOVE: usize = 11;
pub(crate) const SHIFT: usize = 13;
pub(crate) const INVENTORY: usize = 14;
pub(crate) const PREV_MAGICK: usize = 15;
pub(crate) const NEXT_MAGICK: usize = 16;

/// KeyboardMouseController.MouseButton.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mouse {
    Left = 1,
    Middle = 2,
    Right = 3,
    X1 = 4,
    X2 = 5,
    ScrollUp = 6,
    ScrollDown = 7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Binding {
    None,
    /// An XNA `Keys` code.
    Key(u32),
    Mouse(Mouse),
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub(crate) struct KeyboardBindings(pub [Binding; COUNT]);

impl Default for KeyboardBindings {
    /// KeyboardMouseController.LoadDefaults.
    fn default() -> Self {
        use Binding::{Key, Mouse as M};
        Self([
            Key(81), // Q water
            Key(87), // W life
            Key(69), // E shield
            Key(82), // R cold
            Key(65), // A lightning
            Key(83), // S arcane
            Key(68), // D earth
            Key(70), // F fire
            M(Mouse::Right),
            M(Mouse::Middle),
            Key(32), // Space boost
            M(Mouse::Left),
            Key(162), // LeftControl block
            Key(160), // LeftShift
            Key(9),   // Tab inventory
            M(Mouse::ScrollUp),
            M(Mouse::ScrollDown),
        ])
    }
}

impl KeyboardBindings {
    /// settings.json "keyboard": [{"key": code} | {"mouse": n} | null; 17].
    pub(crate) fn from_settings(settings: &Value) -> Self {
        let mut out = Self::default();
        if let Some(list) = settings["keyboard"].as_array().filter(|l| l.len() == COUNT) {
            for (slot, v) in out.0.iter_mut().zip(list) {
                *slot = if let Some(k) = v["key"].as_u64() {
                    Binding::Key(k as u32)
                } else if let Some(m) = v["mouse"].as_u64() {
                    MOUSES
                        .into_iter()
                        .find(|b| *b as u64 == m)
                        .map_or(Binding::None, Binding::Mouse)
                } else {
                    Binding::None
                };
            }
        }
        out
    }
    pub(crate) fn to_settings(&self) -> Value {
        Value::Array(
            self.0
                .iter()
                .map(|b| match b {
                    Binding::None => Value::Null,
                    Binding::Key(k) => json!({ "key": k }),
                    Binding::Mouse(m) => json!({ "mouse": *m as u8 }),
                })
                .collect(),
        )
    }
    /// IsValidKey: Escape and the Windows/menu keys cannot be bound; any
    /// function already on the key or button loses it.
    pub(crate) fn bind(&mut self, function: usize, binding: Binding) -> bool {
        if matches!(binding, Binding::Key(27 | 91 | 92 | 93) | Binding::None) {
            return false;
        }
        for slot in &mut self.0 {
            if *slot == binding {
                *slot = Binding::None;
            }
        }
        self.0[function] = binding;
        true
    }
    pub(crate) fn pressed(
        &self,
        function: usize,
        keys: &ButtonInput<KeyCode>,
        mouse: &ButtonInput<MouseButton>,
        wheel: f32,
    ) -> bool {
        match self.0[function] {
            Binding::None => false,
            Binding::Key(code) => keys.get_pressed().any(|k| xna_key(*k) == Some(code)),
            Binding::Mouse(m) => mouse_state(m, mouse, wheel, false),
        }
    }
    pub(crate) fn just_pressed(
        &self,
        function: usize,
        keys: &ButtonInput<KeyCode>,
        mouse: &ButtonInput<MouseButton>,
        wheel: f32,
    ) -> bool {
        match self.0[function] {
            Binding::None => false,
            Binding::Key(code) => keys.get_just_pressed().any(|k| xna_key(*k) == Some(code)),
            Binding::Mouse(m) => mouse_state(m, mouse, wheel, true),
        }
    }
}

const MOUSES: [Mouse; 7] = [
    Mouse::Left,
    Mouse::Middle,
    Mouse::Right,
    Mouse::X1,
    Mouse::X2,
    Mouse::ScrollUp,
    Mouse::ScrollDown,
];

fn mouse_state(m: Mouse, mouse: &ButtonInput<MouseButton>, wheel: f32, edge: bool) -> bool {
    let button = match m {
        Mouse::Left => MouseButton::Left,
        Mouse::Middle => MouseButton::Middle,
        Mouse::Right => MouseButton::Right,
        Mouse::X1 => MouseButton::Back,
        Mouse::X2 => MouseButton::Forward,
        // One wheel step per frame, as IsPressed(ScrollUp/Down) compares
        // consecutive wheel values.
        Mouse::ScrollUp => return wheel > 0.0,
        Mouse::ScrollDown => return wheel < 0.0,
    };
    if edge {
        mouse.just_pressed(button)
    } else {
        mouse.pressed(button)
    }
}

/// The first binding pressed this frame, for key capture.
pub(crate) fn captured(
    keys: &ButtonInput<KeyCode>,
    mouse: &ButtonInput<MouseButton>,
    wheel: f32,
) -> Option<Binding> {
    if let Some(code) = keys.get_just_pressed().find_map(|k| xna_key(*k)) {
        return Some(Binding::Key(code));
    }
    MOUSES
        .into_iter()
        .find(|m| mouse_state(*m, mouse, wheel, true))
        .map(Binding::Mouse)
}

/// KeyToString: character keys as their upper-case character, other keys
/// by their XNA `Keys` name, mouse buttons by `#mouse_*`.
pub(crate) fn name(binding: Binding, loc: impl Fn(&str) -> String) -> String {
    match binding {
        Binding::None => " ".into(),
        Binding::Mouse(m) => loc(match m {
            Mouse::Left => "#mouse_left",
            Mouse::Middle => "#mouse_middle",
            Mouse::Right => "#mouse_right",
            Mouse::X1 => "#mouse_4",
            Mouse::X2 => "#mouse_5",
            Mouse::ScrollUp => "#mouse_scroll_up",
            Mouse::ScrollDown => "#mouse_scroll_down",
        }),
        Binding::Key(code) => match code {
            48..=57 | 65..=90 => char::from(code as u8).to_string(),
            96..=105 => format!("NumPad{}", code - 96),
            112..=123 => format!("F{}", code - 111),
            186 => ";".into(),
            187 => "=".into(),
            188 => ",".into(),
            189 => "-".into(),
            190 => ".".into(),
            191 => "/".into(),
            192 => "`".into(),
            219 => "[".into(),
            220 => "\\".into(),
            221 => "]".into(),
            222 => "'".into(),
            8 => "Back".into(),
            9 => "Tab".into(),
            13 => "Enter".into(),
            20 => "CapsLock".into(),
            32 => "Space".into(),
            33 => "PageUp".into(),
            34 => "PageDown".into(),
            35 => "End".into(),
            36 => "Home".into(),
            37 => "Left".into(),
            38 => "Up".into(),
            39 => "Right".into(),
            40 => "Down".into(),
            45 => "Insert".into(),
            46 => "Delete".into(),
            160 => "LeftShift".into(),
            161 => "RightShift".into(),
            162 => "LeftControl".into(),
            163 => "RightControl".into(),
            164 => "LeftAlt".into(),
            165 => "RightAlt".into(),
            other => other.to_string(),
        },
    }
}

/// The XNA `Keys` code of a key.
pub(crate) fn xna_key(key: KeyCode) -> Option<u32> {
    use KeyCode as K;
    let letters = [
        K::KeyA,
        K::KeyB,
        K::KeyC,
        K::KeyD,
        K::KeyE,
        K::KeyF,
        K::KeyG,
        K::KeyH,
        K::KeyI,
        K::KeyJ,
        K::KeyK,
        K::KeyL,
        K::KeyM,
        K::KeyN,
        K::KeyO,
        K::KeyP,
        K::KeyQ,
        K::KeyR,
        K::KeyS,
        K::KeyT,
        K::KeyU,
        K::KeyV,
        K::KeyW,
        K::KeyX,
        K::KeyY,
        K::KeyZ,
    ];
    if let Some(i) = letters.iter().position(|k| *k == key) {
        return Some(65 + i as u32);
    }
    let digits = [
        K::Digit0,
        K::Digit1,
        K::Digit2,
        K::Digit3,
        K::Digit4,
        K::Digit5,
        K::Digit6,
        K::Digit7,
        K::Digit8,
        K::Digit9,
    ];
    if let Some(i) = digits.iter().position(|k| *k == key) {
        return Some(48 + i as u32);
    }
    let numpad = [
        K::Numpad0,
        K::Numpad1,
        K::Numpad2,
        K::Numpad3,
        K::Numpad4,
        K::Numpad5,
        K::Numpad6,
        K::Numpad7,
        K::Numpad8,
        K::Numpad9,
    ];
    if let Some(i) = numpad.iter().position(|k| *k == key) {
        return Some(96 + i as u32);
    }
    let function = [
        K::F1,
        K::F2,
        K::F3,
        K::F4,
        K::F5,
        K::F6,
        K::F7,
        K::F8,
        K::F9,
        K::F10,
        K::F11,
        K::F12,
    ];
    if let Some(i) = function.iter().position(|k| *k == key) {
        return Some(112 + i as u32);
    }
    Some(match key {
        K::Backspace => 8,
        K::Tab => 9,
        K::Enter | K::NumpadEnter => 13,
        K::Escape => 27,
        K::Space => 32,
        K::ArrowLeft => 37,
        K::ArrowUp => 38,
        K::ArrowRight => 39,
        K::ArrowDown => 40,
        K::Delete => 46,
        K::ShiftLeft => 160,
        K::ShiftRight => 161,
        K::ControlLeft => 162,
        K::ControlRight => 163,
        K::AltLeft => 164,
        K::AltRight => 165,
        K::Semicolon => 186,
        K::Equal => 187,
        K::Comma => 188,
        K::Minus => 189,
        K::Period => 190,
        K::Slash => 191,
        K::Backquote => 192,
        K::BracketLeft => 219,
        K::Backslash => 220,
        K::BracketRight => 221,
        K::Quote => 222,
        K::Home => 36,
        K::End => 35,
        K::PageUp => 33,
        K::PageDown => 34,
        K::Insert => 45,
        K::CapsLock => 20,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_a_used_key_clears_it_elsewhere_and_round_trips() {
        let mut keys = KeyboardBindings::default();
        // Escape never binds (IsValidKey).
        assert!(!keys.bind(0, Binding::Key(27)));
        // F (fire) moves to water; fire loses it.
        assert!(keys.bind(0, Binding::Key(70)));
        assert_eq!(keys.0[0], Binding::Key(70));
        assert_eq!(keys.0[7], Binding::None);
        assert!(keys.bind(BOOST, Binding::Mouse(Mouse::Left)));
        assert_eq!(keys.0[MOVE], Binding::None);
        let settings = json!({ "keyboard": keys.to_settings() });
        assert_eq!(KeyboardBindings::from_settings(&settings), keys);
        assert_eq!(name(Binding::Key(160), |k| k.into()), "LeftShift");
        assert_eq!(name(Binding::Key(81), |k| k.into()), "Q");
        assert_eq!(
            name(Binding::Mouse(Mouse::Right), |k| k.into()),
            "#mouse_right"
        );
    }
}
