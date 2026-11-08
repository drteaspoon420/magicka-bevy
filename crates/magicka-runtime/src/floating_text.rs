//! FloatingTextManager: floating damage/heal/XP numbers and the
//! GUIHardwareInstancing Numbers technique (`shader.asm.txt` shaders 1/2).
//! See research/floating-text.md. Positions are world space; quads are
//! top-left screen pixels like `hud::HudQuad`.
use crate::hud::HudQuad;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

pub const MAX_NUMBERS: usize = 1024;
pub const XP_NUMBER_DELAY: f32 = 0.5;
/// FloatingTextManager.DIGITSIZE (pixels of one `UI/HUD/Digits` cell).
pub const DIGIT_SIZE: [f32; 2] = [22.0, 32.0];
/// Digits atlas cells: 0-9, then '+' (negative values) and "XP".
pub const GLYPH_PLUS: u8 = 10;
pub const GLYPH_XP: u8 = 11;
/// Character direct-damage number lifetime (Character.Damage notify).
pub const DIRECT_TTL: f32 = 0.4;
/// Spell.FIRECOLOR, Spell.POISONCOLOR and Character.UpdateDamage's bleed colour.
pub const FIRE_COLOR: [f32; 3] = [1.8, 0.6, 0.4];
pub const POISON_COLOR: [f32; 3] = [1.0, 1.2, 0.0];
/// The Numbers shaders' own literals (c164.x, c2.x), not the exact constants.
#[allow(clippy::approx_constant)]
const SHADER_LOG10_2: f32 = 0.30103;
#[allow(clippy::approx_constant)]
const SHADER_LOG2_10: f32 = 3.32193;
pub const BLEED_COLOR: [f32; 3] = [1.25, 0.125_490_2, 0.062_745_1];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Number {
    pub locked: bool,
    pub velocity: f32,
    pub position: [f32; 3],
    pub value: f32,
    pub color: [f32; 3],
    pub scale: [f32; 2],
    /// (initial, remaining).
    pub ttl: [f32; 2],
    pub is_xp: bool,
}

/// One RenderData instance as Number.CopyToArrays writes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumberInstance {
    pub world_position: [f32; 3],
    pub scale: [f32; 2],
    /// Values.X: value + 0.5 (the shader's digit source).
    pub value: f32,
    pub is_xp: bool,
    pub color: [f32; 4],
}

/// FloatingTextManager.GetDefaultColors.
pub fn default_color(amount: f32) -> [f32; 3] {
    if amount < 0.0 {
        [0.0, 1.0, 0.0]
    } else if amount > 0.0 {
        [1.0, 0.9, 0.0]
    } else {
        [0.5, 0.5, 0.5]
    }
}

#[derive(Clone, Debug)]
pub struct FloatingTextManager {
    numbers: Vec<Number>,
    /// PolygonHead.Heap<int>: a min-heap, so Pop returns the lowest free slot.
    free: BinaryHeap<Reverse<usize>>,
    last: i32,
    /// GlobalSettings.DamageNumbers (default On).
    pub enabled: bool,
}
impl Default for FloatingTextManager {
    fn default() -> Self {
        Self {
            numbers: vec![Number::default(); MAX_NUMBERS],
            free: (0..MAX_NUMBERS).map(Reverse).collect(),
            last: -1,
            enabled: true,
        }
    }
}
impl FloatingTextManager {
    /// Clear / Scene setter: every slot becomes free; numbers keep their data.
    pub fn clear(&mut self) {
        self.free = (0..MAX_NUMBERS).map(Reverse).collect();
        self.last = -1;
    }
    pub fn last_number(&self) -> i32 {
        self.last
    }
    pub fn number(&self, index: usize) -> &Number {
        &self.numbers[index]
    }

    /// AddNumber without a colour: GetDefaultColors from the value's sign.
    pub fn add(&mut self, value: f32, position: [f32; 3], ttl: f32, locked: bool) -> i32 {
        self.add_colored(value, position, ttl, locked, default_color(value))
    }
    /// AddNumber(iValue, iPosition, iTTL, iLocked, iColor). Disabled numbers
    /// return 0 (not -1) as in the source; a full table returns -1.
    pub fn add_colored(
        &mut self,
        value: f32,
        position: [f32; 3],
        ttl: f32,
        locked: bool,
        color: [f32; 3],
    ) -> i32 {
        if !self.enabled {
            return 0;
        }
        let Some(Reverse(index)) = self.free.pop() else {
            return -1;
        };
        self.numbers[index] = Number {
            locked,
            velocity: 0.02,
            position,
            value,
            color,
            scale: [1.0, 1.0],
            ttl: [ttl, ttl],
            is_xp: false,
        };
        self.last = self.last.max(index as i32);
        index as i32
    }
    /// The experience-point overload: XP_SCALE and +0.5 s delay before fading.
    /// A disabled or full table indexes slot 0 / -1 like the source; -1 is
    /// skipped here instead of throwing.
    pub fn add_xp(
        &mut self,
        value: f32,
        position: [f32; 3],
        ttl: f32,
        locked: bool,
        color: [f32; 3],
    ) -> i32 {
        let index = self.add_colored(value, position, ttl, locked, color);
        if let Some(n) = usize::try_from(index).ok().map(|i| &mut self.numbers[i]) {
            n.is_xp = true;
            n.scale = [1.0, 1.0];
            n.ttl[1] += XP_NUMBER_DELAY;
            n.ttl[0] += XP_NUMBER_DELAY;
        }
        index
    }
    pub fn add_to(&mut self, index: i32, value: f32) {
        if let Some(n) = self.slot(index) {
            n.value += value;
        }
    }
    pub fn update_position(&mut self, index: i32, position: [f32; 3]) {
        if let Some(n) = self.slot(index) {
            n.position = position;
        }
    }
    pub fn release(&mut self, index: i32) {
        if let Some(n) = self.slot(index) {
            n.locked = false;
        }
    }
    fn slot(&mut self, index: i32) -> Option<&mut Number> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.numbers.get_mut(i))
    }

    /// FloatingTextManager.Update: snapshots the instances to draw this frame
    /// (before aging, as CopyToArrays runs first), then ages/scales numbers.
    pub fn update(&mut self, dt: f32, paused: bool) -> Vec<NumberInstance> {
        let count = (self.last + 1) as usize;
        let instances = self.numbers[..count].iter().map(instance).collect();
        for (i, n) in self.numbers[..count].iter_mut().enumerate() {
            if !n.is_xp {
                let s = (1.0 + (n.value / 1000.0).abs().min(2.0)) * 0.5;
                n.scale = [s, s];
            }
            if n.locked {
                continue;
            }
            let before = n.ttl[1];
            n.ttl[1] = before - dt;
            if !paused && n.is_xp && n.ttl[0] - n.ttl[1] >= XP_NUMBER_DELAY {
                n.velocity -= dt * 0.03;
                n.position[1] += n.velocity;
            }
            if n.ttl[1] < f32::EPSILON && before >= f32::EPSILON {
                self.free.push(Reverse(i));
            }
        }
        while self.last >= 0 && self.numbers[self.last as usize].ttl[1] < 0.0 {
            self.last -= 1;
        }
        instances
    }
}

/// Number.CopyToArrays.
fn instance(n: &Number) -> NumberInstance {
    let mut alpha = 0.0;
    if !n.is_xp || n.ttl[0] - n.ttl[1] >= XP_NUMBER_DELAY {
        alpha = (1.0 - f64::from(1.0 - n.ttl[1] / n.ttl[0]).powi(5) as f32).max(0.0);
    }
    NumberInstance {
        world_position: n.position,
        scale: n.scale,
        value: n.value + 0.5,
        is_xp: n.is_xp,
        color: [n.color[0], n.color[1], n.color[2], alpha],
    }
}

/// Numbers vertex shader: character count of an instance (digits of
/// |value|, plus one cell when negative or XP).
pub fn cell_count(value: f32, is_xp: bool) -> u32 {
    let digits = ((value.abs().log2() * SHADER_LOG10_2).floor()).max(0.0) + 1.0;
    let extra = value < -0.1 || is_xp;
    digits as u32 + u32::from(extra)
}

/// Numbers pixel shader: atlas glyph of cell `k`, counted from the right.
pub fn glyph(value: f32, is_xp: bool, count: u32, k: u32) -> u8 {
    if is_xp && k == 0 {
        return GLYPH_XP;
    }
    if value + 0.1 < 0.0 && k + 1 >= count {
        return GLYPH_PLUS;
    }
    let place = k as f32 - f32::from(u8::from(is_xp));
    let shifted = (place * -SHADER_LOG2_10).exp2() * value.abs() * 0.1;
    let digit = shifted.fract() * 10.0;
    digit.floor() as u8
}

/// Screen-pixel glyph quads of one instance centred on `screen` (the
/// MagickaMath.WorldToScreenPosition of its world position).
pub fn quads(n: &NumberInstance, screen: [f32; 2]) -> Vec<HudQuad> {
    let count = cell_count(n.value, n.is_xp);
    let w = DIGIT_SIZE[0] * n.scale[0];
    let h = DIGIT_SIZE[1] * n.scale[1];
    let left = screen[0] - w * count as f32 * 0.5;
    (0..count)
        .map(|j| {
            let g = f32::from(glyph(n.value, n.is_xp, count, count - 1 - j));
            let x = left + w * j as f32;
            HudQuad {
                bounds: [x, screen[1] - h * 0.5, x + w, screen[1] + h * 0.5],
                atlas: [
                    g * DIGIT_SIZE[0],
                    0.0,
                    (g + 1.0) * DIGIT_SIZE[0],
                    DIGIT_SIZE[1],
                ],
                color: n.color,
            }
        })
        .collect()
}

/// Character damage-number requests in their original call order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumberRequest {
    /// Character.Damage with the Notify feature: add to the locked number or
    /// create one (TTL 0.4, locked) at the capsule top.
    Direct { amount: f32 },
    /// Character.UpdateDamage numbers, moves and releases.
    Update(magicka_sim::damage_update::DamageUpdateEvent),
    /// Character.Deinitialize: release the locked number.
    Release,
}

/// Character.Position.Y + Capsule.Length * 0.5 + mRadius.
pub fn character_top(position: [f32; 3], length: f32, radius: f32) -> [f32; 3] {
    [
        position[0],
        position[1] + length * 0.5 + radius,
        position[2],
    ]
}

impl FloatingTextManager {
    /// Apply one character's queued requests against its locked index.
    pub fn apply(
        &mut self,
        last_damage_index: &mut i32,
        requests: &mut Vec<NumberRequest>,
        top: [f32; 3],
    ) {
        use magicka_sim::damage_update::{DamageUpdateEvent as E, NumberKind as K};
        for request in requests.drain(..) {
            match request {
                NumberRequest::Direct { amount } => {
                    if *last_damage_index >= 0 {
                        self.add_to(*last_damage_index, amount);
                    } else {
                        *last_damage_index = self.add(amount, top, DIRECT_TTL, true);
                    }
                }
                NumberRequest::Release => {
                    if *last_damage_index >= 0 {
                        self.release(*last_damage_index);
                    }
                    *last_damage_index = -1;
                }
                NumberRequest::Update(E::MoveNumber(i)) => self.update_position(i, top),
                NumberRequest::Update(E::ReleaseNumber(i)) => self.release(i),
                NumberRequest::Update(E::Number { kind, amount, ttl }) => {
                    let color = match kind {
                        K::Regeneration | K::Healing => default_color(amount),
                        K::Fire => FIRE_COLOR,
                        K::Poison => POISON_COLOR,
                        K::Bleeding => BLEED_COLOR,
                    };
                    self.add_colored(amount, top, ttl, false, color);
                }
            }
        }
    }
}
