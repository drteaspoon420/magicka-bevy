//! CustomCapsuleCapsule.CollDetect, token 0x06003322. See capsule-pair.md.
use crate::{geometry::jiglib_segment_segment, impact_motion::Vector, spray_pulse::Segment};

#[derive(Clone, Copy, Debug)]
pub struct CapsuleMotion {
    pub old_axis: Segment,
    pub new_axis: Segment,
    pub old_radius: f32,
    pub new_radius: f32,
    pub old_body_position: Vector,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PairContact {
    pub normal: Vector,
    pub point: Vector,
    pub offset0: Vector,
    pub offset1: Vector,
    pub penetration: f32,
}

/// One JigLibX Normal-solver pass for zero-inertia, zero-friction character
/// bodies. The host supplies the preprocessed minimum separation velocity.
/// Native uses the inverse-mass sum even for the detector's non-unit normal.
pub fn solve_normal(
    contact: PairContact,
    min_separation_velocity: f32,
    inverse_masses: [f32; 2],
    velocities: &mut [Vector; 2],
) -> bool {
    let relative: Vector = std::array::from_fn(|i| velocities[0][i] - velocities[1][i]);
    let normal_velocity = relative[0] * contact.normal[0]
        + relative[1] * contact.normal[1]
        + relative[2] * contact.normal[2];
    if normal_velocity > min_separation_velocity {
        return false;
    }
    // Restitution is zero; native replaces its near-zero target with MinSeparationVel.
    let change = min_separation_velocity - normal_velocity;
    if change <= 0.001 {
        return false;
    }
    let denominator = (inverse_masses[0] + inverse_masses[1]).max(1e-6);
    let impulse = change / denominator;
    for (i, n) in contact.normal.iter().enumerate() {
        velocities[0][i] += n * impulse * inverse_masses[0];
        velocities[1][i] -= n * impulse * inverse_masses[1];
    }
    true
}

fn point(axis: Segment, t: f32) -> Vector {
    std::array::from_fn(|i| axis.origin[i] + axis.delta[i] * t)
}

/// The supplied random draw must be System.Random.Next(360), in [0,360).
/// It is consumed only for a contact whose old axis distance is <= 1e-6.
/// Native geometry deliberately uses old points even when only new axes touch.
pub fn detect(
    a: CapsuleMotion,
    b: CapsuleMotion,
    tolerance: f32,
    random_degrees: impl FnOnce() -> u16,
) -> Option<PairContact> {
    let (old_sq, t0, t1) = jiglib_segment_segment(
        a.old_axis.origin,
        a.old_axis.delta,
        b.old_axis.origin,
        b.old_axis.delta,
    );
    let (new_sq, _, _) = jiglib_segment_segment(
        a.new_axis.origin,
        a.new_axis.delta,
        b.new_axis.origin,
        b.new_axis.delta,
    );
    let radius = a.new_radius + b.new_radius;
    let threshold = (radius + tolerance) * (radius + tolerance);
    // bge.un rejects unordered values; do not accept NaN as a collision.
    if old_sq.is_nan()
        || new_sq.is_nan()
        || old_sq.min(new_sq).partial_cmp(&threshold) != Some(std::cmp::Ordering::Less)
    {
        return None;
    }
    let mut p0 = point(a.old_axis, t0);
    let mut p1 = point(b.old_axis, t1);
    p0[1] = (p0[1] + p1[1]) * 0.5;
    p1[1] = p0[1];
    let distance = (old_sq as f64).sqrt() as f32;
    let penetration = radius - distance;
    let normal = if distance > 1e-6 {
        std::array::from_fn(|i| (p0[i] - p1[i]) / distance)
    } else {
        let angle = (random_degrees() as f32).to_radians();
        let (sin, cos) = angle.sin_cos();
        [sin, 0.0, cos] // Backward rotated around Up.
    };
    let point = std::array::from_fn(|i| p1[i] + (b.old_radius - 0.5 * penetration) * normal[i]);
    Some(PairContact {
        normal,
        point,
        penetration,
        offset0: std::array::from_fn(|i| point[i] - a.old_body_position[i]),
        offset1: std::array::from_fn(|i| point[i] - b.old_body_position[i]),
    })
}
