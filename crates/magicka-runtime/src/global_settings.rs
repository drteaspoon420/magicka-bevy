//! GlobalSettings values the simulation and HUD read directly, as the
//! original reads its singleton. Defaults are the constructor's.
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

static BLOOD_AND_GORE: AtomicBool = AtomicBool::new(true);
static HEALTH_BARS: AtomicU8 = AtomicU8::new(HealthBars::On as u8);

/// SettingOptions for GlobalSettings.HealthBars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealthBars {
    Off = 0,
    PlayersOnly = 1,
    On = 2,
}

/// GlobalSettings.BloodAndGore == On: overkills, bloat kills and gibs.
pub fn blood_and_gore() -> bool {
    BLOOD_AND_GORE.load(Ordering::Relaxed)
}
pub fn set_blood_and_gore(on: bool) {
    BLOOD_AND_GORE.store(on, Ordering::Relaxed);
}

/// Avatar bars unless Off; NonPlayerCharacter bars only when On.
pub fn health_bars() -> HealthBars {
    match HEALTH_BARS.load(Ordering::Relaxed) {
        0 => HealthBars::Off,
        1 => HealthBars::PlayersOnly,
        _ => HealthBars::On,
    }
}
pub fn set_health_bars(value: HealthBars) {
    HEALTH_BARS.store(value as u8, Ordering::Relaxed);
}

/// GlobalSettings' Low, Medium and High quality options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Low = 0,
    Medium = 1,
    High = 2,
}

impl Quality {
    /// Defines.DecalLimit.
    pub fn decal_limit(self) -> usize {
        match self {
            Self::Low => 150,
            Self::Medium => 300,
            Self::High => 600,
        }
    }
    /// Defines.ParticleMultiplyer, given to ParticleSystem.SetSpawnModifier.
    pub fn particle_multiplier(self) -> f32 {
        match self {
            Self::Low => 0.5,
            Self::Medium => 0.75,
            Self::High => 1.0,
        }
    }
    /// GlobalSettings.ModShadowResolution.
    pub fn shadow_resolution(self, base: usize) -> usize {
        match self {
            Self::Low => base / 2,
            Self::Medium => base,
            Self::High => base * 2,
        }
    }
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Low,
            2 => Self::High,
            _ => Self::Medium,
        }
    }
}

/// The graphics options and the spell wheel, read when a session starts as
/// PlayState's constructor reads them. Constructor defaults; Particles is not set
/// there, and a value of High is assumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QualitySettings {
    pub shadows: Quality,
    pub decals: Quality,
    pub particles: Quality,
    /// PlayState enables ParticleLightBatcher only when this is on.
    pub particle_lights: bool,
    /// Off: IconRenderer takes no queue icons and SpellWheel does not draw.
    pub spell_wheel: bool,
}

impl Default for QualitySettings {
    fn default() -> Self {
        Self {
            shadows: Quality::Medium,
            decals: Quality::Medium,
            particles: Quality::High,
            particle_lights: true,
            spell_wheel: true,
        }
    }
}

static SHADOWS: AtomicU8 = AtomicU8::new(Quality::Medium as u8);
static DECALS: AtomicU8 = AtomicU8::new(Quality::Medium as u8);
static PARTICLES: AtomicU8 = AtomicU8::new(Quality::High as u8);
static PARTICLE_LIGHTS: AtomicBool = AtomicBool::new(true);
static SPELL_WHEEL: AtomicBool = AtomicBool::new(true);

pub fn set_quality(q: QualitySettings) {
    SHADOWS.store(q.shadows as u8, Ordering::Relaxed);
    DECALS.store(q.decals as u8, Ordering::Relaxed);
    PARTICLES.store(q.particles as u8, Ordering::Relaxed);
    PARTICLE_LIGHTS.store(q.particle_lights, Ordering::Relaxed);
    SPELL_WHEEL.store(q.spell_wheel, Ordering::Relaxed);
}
pub fn shadow_quality() -> Quality {
    Quality::from_u8(SHADOWS.load(Ordering::Relaxed))
}
pub fn decal_quality() -> Quality {
    Quality::from_u8(DECALS.load(Ordering::Relaxed))
}
pub fn particle_quality() -> Quality {
    Quality::from_u8(PARTICLES.load(Ordering::Relaxed))
}
pub fn particle_lights() -> bool {
    PARTICLE_LIGHTS.load(Ordering::Relaxed)
}
pub fn spell_wheel() -> bool {
    SPELL_WHEEL.load(Ordering::Relaxed)
}

#[cfg(test)]
mod quality_tests {
    use super::*;

    #[test]
    fn quality_tables_are_the_native_ones() {
        let q = [Quality::Low, Quality::Medium, Quality::High];
        assert_eq!(q.map(Quality::decal_limit), [150, 300, 600]);
        assert_eq!(q.map(Quality::particle_multiplier), [0.5, 0.75, 1.0]);
        assert_eq!(q.map(|q| q.shadow_resolution(256)), [128, 256, 512]);
    }
}
