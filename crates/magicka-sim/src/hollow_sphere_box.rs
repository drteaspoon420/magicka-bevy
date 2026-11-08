//! Magicka.Physics.HollowSphereBox.CollDetect. Contact generation only;
//! collision callbacks and JigLibX's solver are separate host operations.
//! See research/hollow-sphere-box.md.
use crate::{
    box_intersection::OrientedBox, impact_motion::Vector, shield_intersection::HollowSphere,
};

/// One primitive's old/new world geometry and its owner's old position.
/// Ownerless collision skins use [0; 3] for `owner_old_position`.
#[derive(Clone, Copy, Debug)]
pub struct PrimitiveMotion<T> {
    pub old: T,
    pub new: T,
    pub owner_old_position: Vector,
}

/// CollisionNotify with the hollow sphere as skin 0 and the box as skin 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellBoxContact {
    pub normal: Vector,
    pub point: Vector,
    pub sphere_offset: Vector,
    pub box_offset: Vector,
    pub penetration: f32,
}

fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: Vector, b: Vector) -> Vector {
    std::array::from_fn(|i| a[i] - b[i])
}
fn length(a: Vector) -> f32 {
    (dot(a, a) as f64).sqrt() as f32
}

/// Generate the native single contact. Sphere forward must be unit length
/// and describe a rigid transform. `random_degrees` supplies Random.Next(360)
/// (0..360), drawn only for a degenerate old-center direction.
/// The detector uses a box bounding radius, not its corners or swept edges.
pub fn detect(
    sphere: PrimitiveMotion<HollowSphere>,
    box_motion: PrimitiveMotion<OrientedBox>,
    tolerance: f32,
    mut random_degrees: impl FnMut() -> u16,
) -> Option<ShellBoxContact> {
    let old_radius = length(box_motion.old.side_lengths.map(|v| v * 0.5));
    let new_radius = length(box_motion.new.side_lengths.map(|v| v * 0.5));
    let direction = sub(sphere.old.center, box_motion.old.center);
    let new_direction = sub(sphere.new.center, box_motion.new.center);
    let outward = new_direction.map(|v| -v / length(new_direction));
    // TransformNormal into the sphere's inverse rigid orientation, then
    // Angle(localDirection, XNA Forward), equals this world-space dot.
    let angle = (dot(outward, sphere.new.forward).clamp(-1.0, 1.0) as f64).acos() as f32;
    if angle > sphere.new.max_angle {
        return None;
    }
    let old_distance_sq = dot(direction, direction);
    let new_distance_sq = dot(new_direction, new_direction);
    let outer_radius = sphere.new.radius + new_radius;
    let shell_threshold = (sphere.new.radius + tolerance).powi(2);
    let outer_threshold = (outer_radius + tolerance).powi(2);
    let inner_threshold = (sphere.new.radius - new_radius + tolerance).powi(2);
    let inside = old_distance_sq < shell_threshold && new_distance_sq > inner_threshold;
    let outside = old_distance_sq > shell_threshold && new_distance_sq < outer_threshold;
    if !inside && !outside {
        return None;
    }
    let distance = (old_distance_sq as f64).sqrt() as f32;
    let penetration = if inside {
        distance + new_radius - sphere.new.radius
    } else {
        outer_radius - distance
    };
    let direction = if distance > 1e-6 {
        direction.map(|v| v / distance)
    } else {
        // Transform Backward (+Z) by a Y-axis rotation; same unseeded RNG
        // fallback as native, with the sample supplied by the caller.
        let angle = f32::from(random_degrees()) * (std::f32::consts::PI / 180.0);
        [
            (angle as f64).sin() as f32,
            0.0,
            (angle as f64).cos() as f32,
        ]
    };
    let point = std::array::from_fn(|i| {
        box_motion.old.center[i] + (old_radius - 0.5 * penetration) * direction[i]
    });
    Some(ShellBoxContact {
        normal: if inside {
            direction.map(|v| -v)
        } else {
            direction
        },
        point,
        sphere_offset: sub(point, sphere.owner_old_position),
        box_offset: sub(point, box_motion.owner_old_position),
        penetration,
    })
}
