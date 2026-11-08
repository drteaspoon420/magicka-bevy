//! Magicka.Physics.HollowSphereCapsule.CollDetect. Native new-axis origin
//! deliberately uses the old capsule position. See hollow-sphere-capsule.md.
use crate::{
    capsule_pair::{CapsuleMotion, PairContact},
    hollow_sphere_box::PrimitiveMotion,
    impact_motion::Vector,
    shield_intersection::HollowSphere,
    spray_pulse::Segment,
};

/// HollowSphere.THICKNESS, initialized to 0.5 in the original static ctor.
pub const THICKNESS: f32 = 0.5;

fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: Vector, b: Vector) -> Vector {
    std::array::from_fn(|i| a[i] - b[i])
}
fn point(segment: Segment, t: f32) -> Vector {
    std::array::from_fn(|i| segment.origin[i] + segment.delta[i] * t)
}

/// JigLibX Distance.PointSegmentDistanceSq, retaining the native endpoint
/// branches and order (including degenerate and unordered float behavior).
fn point_segment(p: Vector, segment: Segment) -> (f32, f32) {
    let mut offset = sub(p, segment.origin);
    let mut t = dot(offset, segment.delta);
    if t <= 0.0 {
        t = 0.0;
    } else {
        let length_sq = dot(segment.delta, segment.delta);
        if t >= length_sq {
            t = 1.0;
            offset = sub(offset, segment.delta);
        } else {
            t /= length_sq;
            offset = sub(offset, segment.delta.map(|v| v * t));
        }
    }
    (dot(offset, offset), t)
}

/// Canonical skin order: sphere is skin 0, capsule skin 1. Sphere forward
/// represents a unit rigid orientation. RNG supplies Random.Next(360) in
/// [0,360), lazily drawn only for a contact with old-axis distance <= 1e-6.
/// CapsuleMotion.new_axis uses the actual new position for angular filtering,
/// but the native distance predicate anchors it at old_axis.origin.
pub fn detect(
    sphere: PrimitiveMotion<HollowSphere>,
    capsule: CapsuleMotion,
    tolerance: f32,
    random_degrees: impl FnOnce() -> u16,
) -> Option<PairContact> {
    let direction = sub(capsule.new_axis.origin, sphere.new.center);
    let inverse_length = 1.0 / (dot(direction, direction) as f64).sqrt() as f32;
    let direction = direction.map(|v| v * inverse_length);
    let angle = (dot(direction, sphere.new.forward).clamp(-1.0, 1.0) as f64).acos() as f32;
    if angle > sphere.new.max_angle {
        return None;
    }
    let new_segment = Segment {
        origin: capsule.old_axis.origin,
        delta: capsule.new_axis.delta,
    };
    let (old_sq, old_t) = point_segment(sphere.old.center, capsule.old_axis);
    let (new_sq, _) = point_segment(sphere.new.center, new_segment);
    let sum_radius = capsule.new_radius + sphere.new.radius;
    let shell_threshold = (sphere.new.radius + tolerance).powi(2);
    let outer_threshold = (sum_radius + THICKNESS + tolerance).powi(2);
    let inner_threshold = (sphere.new.radius - capsule.new_radius - THICKNESS + tolerance).powi(2);
    let inside = old_sq < shell_threshold && new_sq > inner_threshold;
    let outside = old_sq > shell_threshold && new_sq < outer_threshold;
    if !inside && !outside {
        return None;
    }
    let axis_point = point(capsule.old_axis, old_t);
    let distance = (old_sq as f64).sqrt() as f32;
    let penetration = if inside {
        distance + capsule.new_radius - sphere.new.radius + THICKNESS
    } else {
        sum_radius - distance + THICKNESS
    };
    let normal = if distance > 1e-6 {
        let offset = if inside {
            sub(axis_point, sphere.old.center)
        } else {
            sub(sphere.old.center, axis_point)
        };
        offset.map(|v| v / distance)
    } else {
        let angle = f32::from(random_degrees()) * (std::f32::consts::PI / 180.0);
        [
            (angle as f64).sin() as f32,
            0.0,
            (angle as f64).cos() as f32,
        ]
    };
    let point = std::array::from_fn(|i| {
        axis_point[i] + (capsule.old_radius - 0.5 * penetration) * normal[i]
    });
    Some(PairContact {
        normal,
        point,
        offset0: sub(point, sphere.owner_old_position),
        offset1: sub(point, capsule.old_body_position),
        penetration,
    })
}
