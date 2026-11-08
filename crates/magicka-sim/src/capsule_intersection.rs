//! JigLibX Intersection.SegmentCapsuleIntersection; see blast-skin-clip.md.
use crate::{impact_motion::Vector, spray_pulse::Segment};
fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: Vector, b: Vector) -> Vector {
    std::array::from_fn(|i| a[i] - b[i])
}
fn sphere_fraction(seg: Segment, center: Vector, radius: f32) -> f32 {
    let offset = sub(seg.origin, center);
    let a = dot(seg.delta, seg.delta);
    let r2 = radius * radius;
    // Native returns false with fraction=0 here; Capsule ignores the boolean.
    if a < r2 {
        return 0.0;
    }
    let b = dot(offset, seg.delta);
    let disc = b * b - a * (dot(offset, offset) - r2);
    if disc < 0.0 {
        return f32::MAX;
    }
    let root = (disc as f64).sqrt() as f32;
    let near = (-b - root) / a;
    let far = (-b + root) / a;
    if near > 1.0 || far < 0.0 {
        return f32::MAX;
    }
    near.max(0.0)
}

/// `axis` is the capsule's unit Orientation.Backward; `start` is Position.
pub fn segment_fraction(
    seg: Segment,
    start: Vector,
    axis: Vector,
    length: f32,
    radius: f32,
) -> Option<f32> {
    let offset = sub(start, seg.origin);
    let nn = dot(axis, axis);
    if nn.abs() < 1e-6 {
        return None;
    }
    let nd = dot(axis, seg.delta);
    let od = dot(offset, seg.delta);
    let no = dot(axis, offset);
    let oo = dot(offset, offset);
    let perpendicular: Vector = std::array::from_fn(|i| offset[i] - no * axis[i] / nn);
    let mut cylinder = if dot(perpendicular, perpendicular) < radius * radius {
        0.0
    } else {
        let a = nn * dot(seg.delta, seg.delta) - nd * nd;
        if a.abs() < 1e-6 {
            return None;
        }
        let b = 2.0 * (no * nd - nn * od);
        let c = nn * (oo - radius * radius) - no * no;
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            return None;
        }
        let t = (-b - (disc as f64).sqrt() as f32) / (2.0 * a);
        if !(0.0..=1.0).contains(&t) {
            return None;
        }
        t
    };
    let end: Vector = std::array::from_fn(|i| start[i] + axis[i] * length);
    let hit = std::array::from_fn(|i| seg.origin[i] + seg.delta[i] * cylinder);
    if dot(sub(hit, start), axis) < 0.0 || dot(sub(hit, end), axis) > 0.0 {
        cylinder = f32::MAX;
    }
    let fraction = cylinder
        .min(sphere_fraction(seg, start, radius))
        .min(sphere_fraction(seg, end, radius));
    (fraction <= 1.0).then_some(fraction)
}
