//! CharacterBody.ProcessCollisionPoints liquid tail, for finite inputs.
/// Defines.FOOTSTEP_WATER_OFFSET gates contact status, independently of drowning.
pub const STATUS_DEPTH: f32 = 0.2;

pub fn status(depth: f32, dt: f32, lava: bool) -> crate::status::StatusEffect {
    use crate::status::{Status, StatusEffect};
    if lava {
        StatusEffect::new(Status::Burning, 100.0, depth * dt * 20.0, false)
    } else {
        StatusEffect::new(Status::Wet, 0.0, depth * dt, false)
    }
}

pub fn should_drown(hit_points: f32, in_water: bool, depth: f32, height_offset: f32) -> bool {
    hit_points > 0.0 && in_water && depth >= -height_offset
}

pub fn velocity_multiplier(depth: f32, dt: f32) -> f32 {
    let base = ((1.0 - depth).max(0.0) as f64).powf(100.0) as f32;
    (base.max(0.1) as f64).powf(dt as f64) as f32
}

/// Native projects position+HeightOffset and the first contact's R1 onto Y,
/// then computes absolute distance. R1 is consumed literally, not converted
/// to a world-space surface coordinate here.
pub fn contact_depth(position_y: f32, height_offset: f32, contact_r1_y: f32) -> f32 {
    let delta = position_y + height_offset - contact_r1_y;
    ((delta * delta) as f64).sqrt() as f32
}
