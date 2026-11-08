//! BoostState.ShieldToBoost: nearest live shield hit along the facing ray.
use crate::{impact_motion::Vector, shield_entity::ShieldEntity as Shield, spray_pulse::Segment};

/// CollisionSkin.SegmentIntersect visits enabled primitives in reverse order.
/// Every Shield retains both its fixed box and hollow sphere, regardless of kind.
fn skin_hit(shield: &Shield, mut segment: Segment) -> Option<Vector> {
    let box_hit = crate::box_intersection::OrientedBox {
        center: shield.position,
        axes: [shield.right, shield.up, shield.forward.map(|v| -v)],
        side_lengths: [0.4, 6.0, 12.0],
    }
    .segment_intersect(segment);
    let mut position =
        box_hit.map(|h| std::array::from_fn(|i| segment.origin[i] + segment.delta[i] * h.fraction));
    if let Some(hit) = box_hit {
        segment.delta = segment.delta.map(|v| v * hit.fraction);
    }
    let sphere = crate::shield_intersection::HollowSphere {
        center: shield.position,
        radius: shield.radius,
        forward: shield.forward,
        max_angle: shield.kind.max_angle(),
    };
    if let Some(t) = sphere.segment_fraction(segment) {
        position = Some(std::array::from_fn(|i| {
            segment.origin[i] + segment.delta[i] * t
        }));
    }
    position
}

pub fn shield_to_boost<'a>(
    position: Vector,
    mut direction: Vector,
    shields: impl IntoIterator<Item = &'a Shield>,
) -> Option<usize> {
    direction[1] = 0.0;
    let segment = Segment {
        origin: position,
        delta: direction.map(|v| v * 20.0),
    };
    let mut nearest = 400.0;
    let mut result = None;
    for (index, shield) in shields.into_iter().enumerate() {
        if shield.hit_points <= 0.0 {
            continue;
        }
        let Some(hit) = skin_hit(shield, segment) else {
            continue;
        };
        let offset: Vector =
            std::array::from_fn(|i| if i == 1 { 0.0 } else { hit[i] - position[i] });
        let distance = offset.iter().map(|v| v * v).sum::<f32>();
        if distance >= nearest {
            continue;
        }
        let inverse = 1.0 / distance.sqrt();
        let dot = offset[0] * inverse * direction[0] + offset[2] * inverse * direction[2];
        let angle = (dot.clamp(-1.0, 1.0) as f64).acos() as f32;
        if distance <= f32::from_bits(1) || angle < std::f32::consts::FRAC_PI_4 {
            nearest = distance;
            result = Some(index);
        }
    }
    result
}
