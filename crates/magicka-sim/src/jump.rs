//! Native Jump ability/action aiming and Character.Jump ballistic launch.
//! See research/npc-jump.md; the action intentionally scales squared distance.
use crate::{character_physics::CharacterBody, impact_motion::Vector};

const EPSILON: f32 = f32::from_bits(1);

#[derive(Clone, Copy, Debug)]
pub struct JumpTarget {
    pub position: Vector,
    pub velocity: Vector,
}
fn squared(v: Vector) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}
fn predicted(target: JumpTarget) -> Vector {
    if squared(target.velocity) > EPSILON {
        std::array::from_fn(|i| target.position[i] + target.velocity[i] * 0.5)
    } else {
        target.position
    }
}
fn min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else {
        a.min(b)
    }
}
fn max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else {
        a.max(b)
    }
}

pub fn ability_delta(
    owner: Vector,
    target: JumpTarget,
    minimum: f32,
    maximum: f32,
) -> Option<Vector> {
    let target = predicted(target);
    let delta: Vector = std::array::from_fn(|i| target[i] - owner[i]);
    let length = squared(delta).sqrt();
    if length < EPSILON {
        return None;
    }
    let mut distance = length * 1.1;
    if distance > maximum {
        distance = maximum;
    }
    if distance < minimum {
        distance = minimum;
    }
    Some(delta.map(|v| v / length * distance))
}

pub fn animation_delta(
    owner: Vector,
    direction: Vector,
    target: Option<JumpTarget>,
    minimum: Option<f32>,
    maximum: Option<f32>,
) -> Option<Vector> {
    // Native no-target fallback treats Direction*midrange as a world point.
    let target = target.map(predicted).unwrap_or_else(|| {
        let low = minimum.unwrap_or(0.);
        let mid = low + (maximum.unwrap_or(0.) - low) * 0.5;
        direction.map(|v| v * mid)
    });
    let delta: Vector = std::array::from_fn(|i| target[i] - owner[i]);
    let length_squared = squared(delta);
    if length_squared.is_nan() || length_squared < EPSILON {
        return None;
    }
    let length = length_squared.sqrt();
    let mut distance = length_squared * 1.1;
    if let Some(low) = minimum {
        distance = max(low, distance);
    }
    if let Some(high) = maximum {
        distance = min(high, distance);
    }
    Some(delta.map(|v| v / length * distance))
}

pub fn velocity(mut delta: Vector, elevation: f32, gravity_y: f32) -> Option<Vector> {
    let height = delta[1];
    delta[1] = 0.;
    let length = squared(delta).sqrt();
    delta = delta.map(|v| v / length);
    let sin = f64::from(elevation).sin() as f32;
    let cos = f64::from(elevation).cos() as f32;
    delta[1] = sin;
    delta[0] *= cos;
    delta[2] *= cos;
    let speed =
        f64::from(-gravity_y * length * length / (2. * (length * sin / cos - height) * cos * cos))
            .sqrt() as f32;
    if !speed.is_finite() {
        return None;
    }
    Some(delta.map(|v| v * speed))
}

pub fn launch(body: &mut CharacterBody, dead: bool, delta: Vector, elevation: f32, gravity_y: f32) {
    if dead || body.owner.entangled {
        return;
    }
    if let Some(velocity) = velocity(delta, elevation, gravity_y) {
        body.add_jump(velocity);
    }
}
