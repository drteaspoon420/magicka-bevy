//! StatusEffect particle emitters (research/status-visuals.md).
//!
//! Each status owns one emitter (StatusEffect.mEmitterA) scaled to the
//! character capsule and moved to the character each update. Burning and
//! Cold switch among three intensity variants and restart when the variant
//! changes or the emitter ended; the others start once with the status.
use magicka_sim::status::{Status, StatusEffect};

pub const EFFECT_BURNING: [&str; 3] = [
    "statuseffect_burning",
    "statuseffect_burning_2",
    "statuseffect_burning_3",
];
pub const EFFECT_COLD: [&str; 3] = [
    "statuseffect_cold",
    "statuseffect_cold_2",
    "statuseffect_cold_3",
];

/// The emitter a status wants this update: (effect name, Y scale kind,
/// restart when inactive).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusEmitter {
    pub name: &'static str,
    /// True for Burning/Cold's `1 + radius` height, false for the
    /// constructor's `0.5·length + radius`.
    pub unit_height: bool,
    /// Burning/Cold restart an ended emitter; constructor emitters do not.
    pub restart: bool,
}

/// StatusEffect constructor and Update emitter selection.
pub fn emitter(effect: &StatusEffect) -> Option<StatusEmitter> {
    let constructor = |name| {
        Some(StatusEmitter {
            name,
            unit_height: false,
            restart: false,
        })
    };
    match effect.kind {
        Status::Burning => {
            // (int)Clamp(DPS / 100, 0, 2)
            let variant = (effect.dps / 100.0).clamp(0.0, 2.0) as usize;
            Some(StatusEmitter {
                name: EFFECT_BURNING[variant],
                unit_height: true,
                restart: true,
            })
        }
        Status::Cold => {
            let count = effect.magnitude.ceil() as i32;
            (count > 0).then(|| StatusEmitter {
                name: EFFECT_COLD[(count - 1).clamp(0, 2) as usize],
                unit_height: true,
                restart: true,
            })
        }
        Status::Wet => constructor("statuseffect_wet"),
        Status::Frozen => constructor("statuseffect_frozen"),
        Status::Healing => constructor("statuseffect_healing"),
        Status::Steamed => constructor("drying_steam"),
        // BLEED_EFFECT[0] regardless of blood type.
        Status::Bleeding => constructor("gore_bleed_regular"),
        Status::Poisoned | Status::Greased | Status::None => None,
    }
}

/// The emitter transform: scale (radius, height, radius) at `position`.
pub fn transform(e: StatusEmitter, radius: f32, length: f32, position: [f32; 3]) -> [f32; 16] {
    let height = if e.unit_height {
        1.0 + radius
    } else {
        0.5 * length + radius
    };
    [
        radius,
        0.0,
        0.0,
        0.0,
        0.0,
        height,
        0.0,
        0.0,
        0.0,
        0.0,
        radius,
        0.0,
        position[0],
        position[1],
        position[2],
        1.0,
    ]
}

/// Statuses that own an emitter, in StatusIndex order.
pub const STATUSES: [Status; 9] = [
    Status::Burning,
    Status::Wet,
    Status::Frozen,
    Status::Cold,
    Status::Poisoned,
    Status::Healing,
    Status::Greased,
    Status::Steamed,
    Status::Bleeding,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn effect(kind: Status, magnitude: f32, dps: f32) -> StatusEffect {
        StatusEffect {
            kind,
            magnitude,
            dps,
            ..StatusEffect::default()
        }
    }

    #[test]
    fn burning_variant_follows_dps() {
        let name = |dps| emitter(&effect(Status::Burning, 1.0, dps)).unwrap().name;
        assert_eq!(name(50.0), "statuseffect_burning");
        assert_eq!(name(150.0), "statuseffect_burning_2");
        assert_eq!(name(900.0), "statuseffect_burning_3");
    }

    #[test]
    fn cold_variant_follows_rounded_up_magnitude() {
        let name = |m| emitter(&effect(Status::Cold, m, 0.0)).map(|e| e.name);
        assert_eq!(name(0.0), None);
        assert_eq!(name(0.2), Some("statuseffect_cold"));
        assert_eq!(name(1.5), Some("statuseffect_cold_2"));
        assert_eq!(name(3.0), Some("statuseffect_cold_3"));
    }

    #[test]
    fn constructor_emitters_scale_to_the_capsule() {
        let wet = emitter(&effect(Status::Wet, 1.0, 0.0)).unwrap();
        let t = transform(wet, 0.5, 1.0, [1.0, 2.0, 3.0]);
        assert_eq!([t[0], t[5], t[10]], [0.5, 1.0, 0.5]);
        assert_eq!([t[12], t[13], t[14]], [1.0, 2.0, 3.0]);
        let burning = emitter(&effect(Status::Burning, 1.0, 0.0)).unwrap();
        assert_eq!(transform(burning, 0.5, 1.0, [0.0; 3])[5], 1.5);
        assert!(emitter(&effect(Status::Poisoned, 1.0, 0.0)).is_none());
    }
}
