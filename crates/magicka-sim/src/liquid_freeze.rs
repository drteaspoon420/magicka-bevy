//! Liquid.Freeze numeric dispatch for finite damage magnitudes.
use crate::{damage::Damage, spells::Elements};

pub fn magnitude(damages: &[Damage], lava: bool, multiplier: f32) -> f32 {
    let (mut cold, mut water, mut fire) = (0.0f32, 0.0f32, 0.0f32);
    for damage in damages {
        if damage.element & Elements::COLD.0 != 0 {
            cold += damage.magnitude;
        }
        if damage.element & Elements::WATER.0 != 0 {
            water += damage.magnitude;
        }
        if damage.element & Elements::FIRE.0 != 0 {
            fire += damage.magnitude;
        }
    }
    let mut strength = cold - fire;
    if lava {
        strength += water;
    }
    // Math.Pow uses doubles after the float exponent multiplication; the
    // result is converted back to float before subtraction and scaling.
    let amount = 1.0 - 0.75f64.powf((strength.abs() * 3.0) as f64) as f32;
    let sign = if strength > 0.0 {
        1.0
    } else if strength < 0.0 {
        -1.0
    } else {
        0.0
    };
    amount * 3.0 * sign * multiplier
}
