//! Numeric state of `Magicka.GameLogic.Spells.StatusEffect`.
//! Damage and speed limits are returned for the owner to apply, separately
//! from presentation. Particle emission and its RNG are not implemented.
//! See `output/research/status.md` for evidence and integration limits.

use crate::spells::Elements;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u16)]
pub enum Status {
    #[default]
    None = 0,
    Burning = 1,
    Wet = 2,
    Frozen = 4,
    Cold = 8,
    Poisoned = 0x10,
    Healing = 0x20,
    Greased = 0x40,
    Steamed = 0x80,
    Bleeding = 0x100,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StatusEffect {
    pub kind: Status,
    pub magnitude: f32,
    pub dps: f32,
    /// Fractional damage retained between owner.Damage integer calls.
    pub damage_rest: f32,
    pub effect_stops: bool,
}

/// Owner properties read by the numerical part of StatusEffect.Update.
#[derive(Clone, Copy, Debug)]
pub struct StatusOwner {
    pub is_character: bool,
    pub in_cutscene: bool,
    pub hit_points: f32,
    pub max_hit_points: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StatusTick {
    /// Direct owner.Damage call, not a resistance-processed spell damage entry.
    pub damage: Option<(i32, Elements)>,
    /// Owner speed becomes min(existing speed multiplier, speed_limit).
    pub speed_limit: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DifferentStatusTypes(pub Status, pub Status);

fn min(a: f32, b: f32) -> f32 {
    if a < b || a.is_nan() {
        a
    } else {
        b
    }
}
fn max(a: f32, b: f32) -> f32 {
    if a > b || a.is_nan() {
        a
    } else {
        b
    }
}

impl StatusEffect {
    /// Numeric constructor. Pass false for the original five-argument overload.
    /// Radius/length describe the omitted particle emitter, not status lifetime.
    pub fn new(kind: Status, dps: f32, magnitude: f32, effect_stops: bool) -> Self {
        Self {
            kind,
            dps,
            magnitude: match kind {
                Status::Steamed => 1.0,
                Status::Healing => 0.5 + magnitude * 0.5,
                Status::Bleeding if !effect_stops => 1.0,
                _ => magnitude,
            },
            damage_rest: 0.0,
            effect_stops,
        }
    }

    pub fn dead(&self) -> bool {
        self.magnitude <= 0.0
    }

    pub fn stop(&mut self) {
        self.magnitude = 0.0;
    }

    /// Original equality compares only status type; Rust PartialEq instead
    /// compares the whole numerical state for deterministic fixture checks.
    pub fn same_kind(&self, other: &Self) -> bool {
        self.kind == other.kind
    }

    pub fn slowdown(&self) -> f32 {
        match self.kind {
            Status::Cold => 0.001f64.powf(self.magnitude as f64) as f32 * 0.667 + 0.333,
            Status::Poisoned => 0.5,
            _ => 1.0,
        }
    }

    /// Numeric portion of operator+: type check follows dead/None shortcuts.
    pub fn merged(self, other: Self) -> Result<Self, DifferentStatusTypes> {
        if self.dead() || self.kind == Status::None {
            return Ok(other);
        }
        if other.dead() || other.kind == Status::None {
            return Ok(self);
        }
        if self.kind != other.kind {
            return Err(DifferentStatusTypes(self.kind, other.kind));
        }
        let mut out = Self {
            kind: self.kind,
            magnitude: if self.kind == Status::Healing {
                max(self.magnitude, other.magnitude)
            } else {
                self.magnitude + other.magnitude
            },
            damage_rest: self.damage_rest + other.damage_rest,
            ..Default::default()
        };
        if self.effect_stops && other.effect_stops {
            out.effect_stops = true;
            out.dps = other.dps;
            out.magnitude = other.magnitude;
        } else {
            let inverse = 1.0 / out.magnitude;
            out.dps = if out.kind == Status::Burning {
                max(self.dps, other.dps)
            } else {
                self.magnitude * inverse * self.dps + other.magnitude * inverse * other.dps
            };
        }
        Ok(out)
    }

    /// Update once with the caller's delta time. Preserve operation order and
    /// final-tick damage even when decay crosses zero; no subdivision/clamping.
    pub fn update(&mut self, dt: f32, owner: StatusOwner) -> StatusTick {
        let mut tick = StatusTick::default();
        if self.dead() {
            return tick;
        }
        match self.kind {
            Status::Burning => {
                self.magnitude = min(self.magnitude, 3.0);
                self.magnitude -= dt * 0.2;
                self.dps = min(self.dps + dt * self.dps * 0.25, 500.0 * self.magnitude);
                self.damage_rest += self.dps * dt;
            }
            Status::Cold => {
                if owner.is_character {
                    tick.speed_limit = Some(self.slowdown());
                }
                self.magnitude -= dt * 0.1;
                self.magnitude = min(self.magnitude, 3.0);
            }
            Status::Wet => self.magnitude = min(self.magnitude, 1.0),
            Status::Frozen => {
                self.magnitude = min(self.magnitude, 3.0);
                self.magnitude -= dt * 0.25;
            }
            Status::Poisoned => {
                if self.magnitude > 1.0 && !self.effect_stops {
                    self.magnitude = 1.0;
                } else if self.effect_stops {
                    self.magnitude -= dt;
                }
                self.damage_rest += dt * self.dps * self.magnitude;
                if owner.is_character && !owner.in_cutscene {
                    tick.speed_limit = Some(self.slowdown());
                }
            }
            Status::Healing => {
                self.damage_rest += dt * self.dps;
                if self.magnitude > 3.0 {
                    self.magnitude = 3.0;
                }
                if owner.hit_points >= owner.max_hit_points {
                    self.magnitude = dt;
                }
                self.magnitude -= dt;
            }
            Status::Steamed => {
                self.magnitude = min(1.0, self.magnitude);
                self.magnitude -= dt;
            }
            Status::Bleeding => {
                if self.effect_stops {
                    self.magnitude -= dt;
                }
                self.damage_rest += dt * self.dps;
            }
            Status::None | Status::Greased => {}
        }
        let whole = self.damage_rest as i32;
        if whole != 0 {
            self.damage_rest -= whole as f32;
            let element = match self.kind {
                Status::Burning => Elements::FIRE,
                Status::Poisoned => Elements::POISON,
                Status::Healing => Elements::LIFE,
                _ => Elements::EARTH,
            };
            tick.damage = Some((whole, element));
        }
        tick
    }
}
