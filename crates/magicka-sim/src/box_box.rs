//! CollDetectBoxBox's new-world separating-axis gate and normal selection.
//! This supplies callback membership; native contact manifold generation and
//! constraint solving remain separate host responsibilities.
use crate::{box_intersection::OrientedBox, impact_motion::Vector};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxOverlap {
    /// Points toward box 0, as in the native collision functor.
    pub normal: Vector,
    pub depth: f32,
}

fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn span(b: OrientedBox, axis: Vector) -> (f32, f32) {
    let center = dot(b.center, axis);
    let half = (0..3)
        .map(|i| (dot(b.axes[i], axis) * 0.5 * b.side_lengths[i]).abs())
        .sum::<f32>();
    (center - half, center + half)
}

/// Native Disjoint: tolerance is applied to the unnormalized spans. Nested
/// intervals use the smaller exit distance, rather than their intersection.
fn depth(a: OrientedBox, b: OrientedBox, axis: Vector, tolerance: f32) -> Option<f32> {
    let (amin, amax) = span(a, axis);
    let (bmin, bmax) = span(b, axis);
    if amin > bmax + tolerance + 1e-6 || bmin > amax + tolerance + 1e-6 {
        return None;
    }
    if (amax > bmax && bmin > amin) || (bmax > amax && amin > bmin) {
        Some((amax - bmin).min(bmax - amin))
    } else {
        Some(amax.min(bmax) - amin.max(bmin))
    }
}

pub fn overlap(a: OrientedBox, b: OrientedBox, tolerance: f32) -> Option<BoxOverlap> {
    let axes: [Vector; 15] = std::array::from_fn(|i| match i {
        0..=2 => a.axes[i],
        3..=5 => b.axes[i - 3],
        _ => cross(a.axes[(i - 6) / 3], b.axes[(i - 6) % 3]),
    });
    let mut depths = [f32::MAX; 15];
    for (i, &axis) in axes.iter().enumerate() {
        if dot(axis, axis) < 1e-6 {
            continue;
        }
        depths[i] = depth(a, b, axis, tolerance)?;
    }
    let mut selected = None;
    let mut minimum = f32::MAX;
    for (i, &axis) in axes.iter().enumerate() {
        let squared = dot(axis, axis);
        if squared < 1e-6 {
            continue;
        }
        let inverse = 1.0 / (squared as f64).sqrt() as f32;
        let d = depths[i] * inverse;
        if d < minimum {
            minimum = d;
            selected = Some(axis.map(|v| v * inverse));
        }
    }
    let mut normal = selected?;
    let delta = std::array::from_fn(|i| b.center[i] - a.center[i]);
    if dot(delta, normal) > 0.0 {
        normal = normal.map(|v| -v);
    }
    Some(BoxOverlap {
        normal,
        depth: minimum,
    })
}
