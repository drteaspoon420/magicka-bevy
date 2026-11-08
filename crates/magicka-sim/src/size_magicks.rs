//! Grow/Shrink lifetime, recast and body parameters. See research/size-magicks.md.
//! The runtime owns capsule/static transforms, the two retained ActiveAuras,
//! audio and particles. Buff multipliers are not interpolated with body size.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeKind {
    Grow,
    Shrink,
}

impl SizeKind {
    pub fn duration(self) -> f32 {
        match self {
            Self::Grow => 20.0,
            Self::Shrink => 10.0,
        }
    }

    /// Magnitude of the zero-bias, Earth-only BuffBoostDamage.
    pub fn earth_damage_multiplier(self) -> f32 {
        match self {
            Self::Grow => 3.0,
            Self::Shrink => 0.5,
        }
    }

    /// BuffModifyHitPoints multiplier (zero additive modifier).
    pub fn health_multiplier(self) -> f32 {
        match self {
            Self::Grow => 2.0,
            Self::Shrink => 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastResult {
    /// Play the requested cue and start its one-shot particle effect.
    Started,
    /// Keep the existing animation and auras; no cue or particle restart.
    Refreshed,
    /// Set the opposite effect's TTL to zero; play only the requested cue.
    CancelledOpposite,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeEffect {
    pub kind: SizeKind,
    pub ttl: f32,
    pub animation_time: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeFrame {
    /// Execute this kind's two retained self auras, even on the last update.
    pub kind: SizeKind,
    pub scale: f32,
    /// Multiply the current frame's speed; never accumulate across frames.
    pub speed_multiplier: f32,
    /// Absolute mass computed from the template, not from the previous frame.
    pub mass: f32,
    /// After applying the frame and auras, reset the capsule/static transform
    /// and template mass. Do not remove the independently expiring buffs.
    pub removed: bool,
}

/// One character's active size effect. Native opposite casts cancel the old
/// effect before checking same-kind refresh, including during its fade-out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SizeMagicks {
    active: Option<SizeEffect>,
}

impl SizeMagicks {
    pub fn active(&self) -> Option<SizeEffect> {
        self.active
    }

    pub fn cast(&mut self, kind: SizeKind) -> CastResult {
        if let Some(effect) = &mut self.active {
            if effect.kind != kind {
                effect.ttl = 0.0;
                return CastResult::CancelledOpposite;
            }
            effect.ttl = kind.duration();
            return CastResult::Refreshed;
        }
        self.active = Some(SizeEffect {
            kind,
            ttl: kind.duration(),
            animation_time: 0.0,
        });
        CastResult::Started
    }

    pub fn update(&mut self, dt: f32, hit_points: f32, template_mass: f32) -> Option<SizeFrame> {
        let effect = self.active.as_mut()?;
        effect.ttl -= dt;
        if effect.ttl <= 0.0 || hit_points <= 0.0 {
            effect.animation_time = (effect.animation_time - dt).max(0.0);
        } else if effect.animation_time < 1.0 {
            effect.animation_time = (effect.animation_time + dt).min(1.0);
        }
        // Native IL rounds the phase to float before Math.Sin(double), then
        // rounds the result back to float before applying the 0.125 amplitude.
        let phase = effect.animation_time * std::f32::consts::TAU * 4.0;
        let amount =
            (effect.animation_time + (f64::from(phase).sin() as f32) * 0.125).clamp(0.0, 1.0);
        let (scale, speed_multiplier, mass) = match effect.kind {
            SizeKind::Grow => (
                1.0 + amount,
                1.0 - 0.5 * amount,
                template_mass + 750.0 * amount,
            ),
            SizeKind::Shrink => (
                1.0 - 0.5 * amount,
                1.0 + amount * 0.5,
                template_mass * (0.5 + (1.0 - amount) * 0.5),
            ),
        };
        let removed = effect.ttl <= 0.0 && effect.animation_time <= 0.0;
        let frame = SizeFrame {
            kind: effect.kind,
            scale,
            speed_multiplier,
            mass,
            removed,
        };
        // SpellManager.Update calls Update, then IsDead, then OnRemove.
        if removed {
            self.active = None;
        }
        Some(frame)
    }

    /// ClearEffects/OnRemove: caller restores template dimensions and mass.
    /// Returns whether a restoration is required; buffs keep their own TTLs.
    pub fn clear(&mut self) -> bool {
        self.active.take().is_some()
    }
}
