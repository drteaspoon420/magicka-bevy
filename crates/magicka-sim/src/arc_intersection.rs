//! Entity.ArcIntersect and Character's ethereal rejection.
use crate::impact_motion::Vector;
#[derive(Clone, Copy, Debug)]
pub struct ArcQuery {
    pub origin: Vector,
    pub direction: Vector,
    pub range: f32,
    pub angle: f32,
    pub height_difference: f32,
}
/// Preserves unclamped acos and unnormalized horizontal direction. Degenerate
/// inputs may produce NaN and fail the original strict angular comparison.
pub fn entity_arc(position: Vector, radius: f32, query: ArcQuery) -> Option<Vector> {
    if (position[1] - query.origin[1]).abs() > query.height_difference {
        return None;
    }
    let mut offset = [
        query.origin[0] - position[0],
        0.0,
        query.origin[2] - position[2],
    ];
    let distance = ((offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]) as f64)
        .sqrt() as f32;
    if distance - radius > query.range {
        return None;
    }
    // XNA Vector3.Divide uses reciprocal multiplication.
    let inverse = 1.0 / distance;
    for value in &mut offset {
        *value *= inverse;
    }
    let dot = offset[0] * query.direction[0] + offset[1] * 0.0 + offset[2] * query.direction[2];
    let angle = ((0.0 - dot) as f64).acos() as f32;
    let denominator = -2.0 * distance * distance;
    let extent = (((radius * radius + denominator) / denominator) as f64).acos() as f32;
    if angle - extent < query.angle {
        Some(std::array::from_fn(|i| position[i] + offset[i] * radius))
    } else {
        None
    }
}
pub fn character_arc(
    position: Vector,
    radius: f32,
    ethereal: bool,
    query: ArcQuery,
) -> Option<Vector> {
    if ethereal {
        None
    } else {
        entity_arc(position, radius, query)
    }
}
