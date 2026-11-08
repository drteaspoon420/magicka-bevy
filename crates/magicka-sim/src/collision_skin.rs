//! CollisionSkin.SegmentIntersect reverse traversal and literal fraction arithmetic.
use crate::{impact_motion::Vector, spray_pulse::Segment};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimitiveHit {
    pub fraction: f32,
    pub position: Vector,
    pub normal: Vector,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinHit {
    pub primitive: usize,
    pub hit: PrimitiveHit,
}
/// The callback returns None for a disabled primitive. Triangle callbacks apply
/// ignore_backfaces; other shapes ignore it, matching the original overload.
pub fn segment_intersect(
    count: usize,
    mut segment: Segment,
    ignore_backfaces: bool,
    mut query: impl FnMut(usize, Segment, bool) -> Option<PrimitiveHit>,
) -> Option<SkinHit> {
    let mut scale = 1.0;
    let mut result = None;
    for primitive in (0..count).rev() {
        if let Some(mut hit) = query(primitive, segment, ignore_backfaces) {
            let fraction = hit.fraction;
            hit.fraction = fraction * scale;
            segment.delta = segment.delta.map(|v| v * fraction);
            scale *= hit.fraction;
            result = Some(SkinHit { primitive, hit });
        }
    }
    result.filter(|r| r.hit.fraction <= 1.0)
}
