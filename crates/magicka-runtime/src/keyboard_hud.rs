//! KeyboardHUD state and draw layout, independent of renderer and key polling.
//! See research/keyboard-hud.md and evidence/keyboard-hud/{static,draw}-il.txt.
use crate::hud::HudQuad;
use magicka_sim::spells::Elements;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyboardIcon {
    pub enabled: bool,
    pub saturation: f32,
    pub cooldown: f32,
    pub reset_timer: f32,
    pub intensity: f32,
}
impl Default for KeyboardIcon {
    fn default() -> Self {
        Self {
            enabled: true,
            saturation: 1.,
            cooldown: 0.,
            reset_timer: 0.,
            intensity: 1.,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct KeyboardHud {
    /// Earth, Water, Cold, Fire, Lightning, Arcane, Life, Shield.
    pub icons: [KeyboardIcon; 8],
}
impl KeyboardHud {
    /// KeyboardHUD.Reset does not reset timers or colors.
    pub fn reset(&mut self) {
        for icon in &mut self.icons {
            icon.enabled = true;
        }
    }
    /// MagickaElement.OnEnabledStateChanged → icon Enabled, from the
    /// TutorialManager disabled-element mask (bit i = icon i). Disabled
    /// icons desaturate in `update`.
    pub fn set_disabled_elements(&mut self, disabled: u32) {
        for (i, icon) in self.icons.iter_mut().enumerate() {
            icon.enabled = disabled & (1 << i) == 0;
        }
    }
    /// Basic keyboard elements only (original indexes the eight-icon array).
    pub fn cooldown(&mut self, element: Elements) -> Result<(), Elements> {
        if !element.0.is_power_of_two() || element.0 > Elements::SHIELD.0 {
            return Err(element);
        }
        self.icons[element.0.trailing_zeros() as usize].cooldown = 0.125;
        Ok(())
    }
    pub fn update(&mut self, dt: f32) {
        for icon in &mut self.icons {
            if !icon.enabled {
                icon.saturation = (icon.saturation - dt * 4.).max(0.);
            } else if icon.cooldown > 0. {
                icon.cooldown -= dt;
                if icon.cooldown <= 0. {
                    icon.reset_timer = 0.125;
                }
                icon.saturation = (icon.saturation + dt * 4.).min(0.5);
            } else if icon.reset_timer > 0. {
                icon.reset_timer -= dt;
                icon.intensity = (icon.intensity + dt * 20.).min(20.);
                icon.saturation = (icon.saturation + dt * 4.).min(1.);
            } else {
                icon.saturation = (icon.saturation + dt * 10.).min(1.);
                icon.intensity = (icon.intensity + dt * 4.).min(1.);
            }
        }
    }
}

pub const DEFAULT_KEY_LABELS: [&str; 8] = ["D", "Q", "R", "F", "A", "S", "W", "E"];
pub const DRAW_ORDER: [usize; 8] = [1, 4, 6, 5, 7, 0, 2, 3];

/// Unshifted icon top-left in screen pixels; original staggered keyboard.
pub fn anchor(index: usize, screen_height: f32) -> [f32; 2] {
    let (column, lower) = match index {
        1 => (0, false),
        4 => (0, true),
        6 => (1, false),
        5 => (1, true),
        7 => (2, false),
        0 => (2, true),
        2 => (3, false),
        3 => (3, true),
        _ => panic!("KeyboardHUD element index outside eight icons"),
    };
    [
        140. + (column - 2) as f32 * 50. + if lower { 25. } else { 0. },
        screen_height - 140. + if lower { 60. } else { 0. },
    ]
}
pub fn key_anchor(index: usize, screen_height: f32) -> [f32; 2] {
    let [x, y] = anchor(index, screen_height);
    [x + 2., y - 12.]
}
/// Shadow, element and key background in original DrawIcon order.
pub fn quads(index: usize, screen_height: f32, icon: KeyboardIcon) -> [HudQuad; 3] {
    let [x, y] = anchor(index, screen_height);
    let color = [icon.intensity, icon.intensity, icon.intensity, 1.];
    let u = (index % 5) as f32 * 50.;
    let v = 156. + (index / 5) as f32 * 50.;
    [
        HudQuad {
            bounds: [x - 5., y + 5., x + 45., y + 55.],
            atlas: [25., 0., 50., 25.],
            color,
        },
        HudQuad {
            bounds: [x, y, x + 50., y + 50.],
            atlas: [u, v, u + 50., v + 50.],
            color,
        },
        HudQuad {
            bounds: [x - 12., y - 12., x + 13., y + 13.],
            atlas: [0., 0., 25., 25.],
            color,
        },
    ]
}
