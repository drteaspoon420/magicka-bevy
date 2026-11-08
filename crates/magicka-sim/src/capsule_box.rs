//! JigLibX CapsuleBox detector and its segment/box closest-point queries.
//! Native line/box cases are folded by axis permutation, including the
//! corner-relative normalized parameters on segment intersections.
use crate::{
    box_intersection::OrientedBox,
    capsule_pair::{CapsuleMotion, PairContact},
    hollow_sphere_box::PrimitiveMotion,
    impact_motion::Vector,
    spray_pulse::Segment,
};

fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: Vector, b: Vector) -> Vector {
    std::array::from_fn(|i| a[i] - b[i])
}
fn point(s: Segment, t: f32) -> Vector {
    std::array::from_fn(|i| s.origin[i] + s.delta[i] * t)
}
fn world(b: OrientedBox, local: Vector) -> Vector {
    std::array::from_fn(|i| {
        b.center[i] + local[0] * b.axes[0][i] + local[1] * b.axes[1][i] + local[2] * b.axes[2][i]
    })
}
fn clamp_point(mut p: Vector, h: Vector) -> (f32, Vector) {
    let mut sq = 0.0;
    for i in 0..3 {
        if p[i] < -h[i] {
            let e = p[i] + h[i];
            sq += e * e;
            p[i] = -h[i];
        } else if p[i] > h[i] {
            let e = p[i] - h[i];
            sq += e * e;
            p[i] = h[i];
        }
    }
    (sq.max(0.0), p)
}

// FaceA/B/C differ only by cyclic axis order. A selected positive face
// projects onto its interior, one of its two negative edges, or a corner.
fn face(p: Vector, d: Vector, h: Vector) -> (f32, f32, Vector) {
    let minus = sub(p, h);
    let plus: Vector = std::array::from_fn(|i| p[i] + h[i]);
    let inside_y = d[0] * plus[1] >= d[1] * minus[0];
    let inside_z = d[0] * plus[2] >= d[2] * minus[0];
    if inside_y && inside_z {
        let inv = 1.0 / d[0];
        return (
            0.0,
            -minus[0] * inv,
            [
                h[0],
                p[1] - d[1] * minus[0] * inv,
                p[2] - d[2] * minus[0] * inv,
            ],
        );
    }
    let yz_edge = |free: usize| {
        let other = 3 - free;
        let dsq = d[0] * d[0] + d[other] * d[other];
        let projection = dsq * plus[free] - d[free] * (d[0] * minus[0] + d[other] * plus[other]);
        (dsq, projection)
    };
    let (free, dsq, projection) = if inside_y {
        let (dsq, n) = yz_edge(1);
        (1, dsq, n)
    } else if inside_z {
        let (dsq, n) = yz_edge(2);
        (2, dsq, n)
    } else {
        let (dsq, n) = yz_edge(1);
        if n >= 0.0 {
            (1, dsq, n)
        } else {
            let (dsq, n) = yz_edge(2);
            if n >= 0.0 {
                (2, dsq, n)
            } else {
                let denom = dsq + d[2] * d[2];
                let proj = d[0] * minus[0] + d[1] * plus[1] + d[2] * plus[2];
                let t = -proj / denom;
                let sq = minus[0] * minus[0] + plus[1] * plus[1] + plus[2] * plus[2] + proj * t;
                return (sq, t, [h[0], -h[1], -h[2]]);
            }
        }
    };
    let mut residual = [minus[0], plus[1], plus[2]];
    let mut closest = [h[0], -h[1], -h[2]];
    if projection <= 2.0 * dsq * h[free] {
        let along = projection / dsq;
        residual[free] = plus[free] - along;
        closest[free] = along - h[free];
    } else {
        residual[free] = minus[free];
        closest[free] = h[free];
    }
    let denom = dsq + d[free] * d[free];
    let proj = dot(d, residual);
    let t = -proj / denom;
    (dot(residual, residual) + proj * t, t, closest)
}

/// Native Distance.SqrDistance(Line, Box): distance², line parameter and
/// centre-relative box point. The line parameter may lie outside [0,1].
pub fn line_distance(s: Segment, b: OrientedBox) -> (f32, f32, Vector) {
    let offset = sub(s.origin, b.center);
    let mut p: Vector = std::array::from_fn(|i| dot(offset, b.axes[i]));
    let mut d: Vector = std::array::from_fn(|i| dot(s.delta, b.axes[i]));
    let h = b.side_lengths.map(|v| v * 0.5);
    let mut reflected = [false; 3];
    for i in 0..3 {
        if d[i] < 0.0 {
            p[i] = -p[i];
            d[i] = -d[i];
            reflected[i] = true;
        }
    }
    let active: Vec<_> = (0..3).filter(|&i| d[i] > 0.0).collect();
    let (sq, t, mut q) = match active.len() {
        3 => {
            let m = sub(p, h);
            let i = if d[1] * m[0] >= d[0] * m[1] {
                if d[2] * m[0] >= d[0] * m[2] {
                    0
                } else {
                    2
                }
            } else if d[2] * m[1] >= d[1] * m[2] {
                1
            } else {
                2
            };
            let order = [i, (i + 1) % 3, (i + 2) % 3];
            let (sq, t, local) = face(
                order.map(|j| p[j]),
                order.map(|j| d[j]),
                order.map(|j| h[j]),
            );
            let mut q = [0.0; 3];
            for j in 0..3 {
                q[order[j]] = local[j];
            }
            (sq, t, q)
        }
        2 => {
            let (mut i, mut j) = (active[0], active[1]);
            let m = sub(p, h);
            if d[j] * m[i] < d[i] * m[j] {
                std::mem::swap(&mut i, &mut j);
            }
            let cross = d[j] * m[i];
            let plus = p[j] + h[j];
            let corner = cross - d[i] * plus;
            let mut q = p;
            q[i] = h[i];
            let (mut sq, t) = if corner >= 0.0 {
                let inv = 1.0 / (d[i] * d[i] + d[j] * d[j]);
                q[j] = -h[j];
                (corner * corner * inv, -(d[i] * m[i] + d[j] * plus) * inv)
            } else {
                let inv = 1.0 / d[i];
                q[j] -= cross * inv;
                (0.0, -m[i] * inv)
            };
            let k = 3 - i - j;
            if q[k] < -h[k] {
                let e = q[k] + h[k];
                sq += e * e;
                q[k] = -h[k];
            } else if q[k] > h[k] {
                let e = q[k] - h[k];
                sq += e * e;
                q[k] = h[k];
            }
            (sq, t, q)
        }
        1 => {
            let i = active[0];
            let t = (h[i] - p[i]) / d[i];
            p[i] = h[i];
            let (sq, q) = clamp_point(p, h);
            (sq, t, q)
        }
        _ => {
            let (sq, q) = clamp_point(p, h);
            (sq, 0.0, q)
        }
    };
    for i in 0..3 {
        if reflected[i] {
            q[i] = -q[i];
        }
    }
    (sq.max(0.0), t, q)
}

/// Native Distance.SegmentBoxDistanceSq, preserving its intersection branch
/// even though those box parameters differ from ordinary closest points.
pub fn segment_distance(s: Segment, b: OrientedBox) -> (f32, f32, Vector) {
    let (sq, mut t, mut q) = line_distance(s, b);
    if sq < 1e-6 {
        if let Some(hit) = b.segment_intersect(s) {
            t = hit.fraction;
            let corner = world(b, b.side_lengths.map(|v| -v * 0.5));
            let offset = sub(hit.position, corner);
            q = std::array::from_fn(|i| dot(offset, b.axes[i].map(|v| v / b.side_lengths[i])));
        }
    }
    if (0.0..=1.0).contains(&t) {
        (sq.max(0.0), t, q)
    } else {
        let t = if t >= 0.0 { 1.0 } else { 0.0 };
        let offset = sub(point(s, t), b.center);
        let p = std::array::from_fn(|i| dot(offset, b.axes[i]));
        let (sq, q) = clamp_point(p, b.side_lengths.map(|v| v * 0.5));
        (sq, t, q)
    }
}

/// Canonical skin order is capsule 0, box 1. Only the old radius enters
/// the predicate; both old and new geometry are tested, old points emitted.
pub fn detect(
    c: CapsuleMotion,
    b: PrimitiveMotion<OrientedBox>,
    tolerance: f32,
    random_degrees: impl FnOnce() -> u16,
) -> Option<PairContact> {
    let (old_sq, t, q) = segment_distance(c.old_axis, b.old);
    let (new_sq, _, _) = segment_distance(c.new_axis, b.new);
    let threshold = (c.old_radius + tolerance) * (c.old_radius + tolerance);
    if old_sq.min(new_sq).partial_cmp(&threshold) != Some(std::cmp::Ordering::Less) {
        return None;
    }
    let axis = point(c.old_axis, t);
    let closest = world(b.old, q);
    let distance = (old_sq as f64).sqrt() as f32;
    let offset = if distance > 1e-6 {
        sub(axis, closest)
    } else {
        sub(axis, b.old.center)
    };
    let normal = if distance > 1e-6 || dot(offset, offset) > 1e-6 {
        let sq = dot(offset, offset);
        if sq != 0.0 {
            let inv = 1.0 / (sq as f64).sqrt() as f32;
            offset.map(|v| v * inv)
        } else {
            offset
        }
    } else {
        let angle = f32::from(random_degrees()) * (std::f32::consts::PI / 180.0);
        [
            (angle as f64).sin() as f32,
            0.0,
            (angle as f64).cos() as f32,
        ]
    };
    Some(PairContact {
        normal,
        point: closest,
        offset0: sub(closest, c.old_body_position),
        offset1: sub(closest, b.owner_old_position),
        penetration: c.old_radius - distance,
    })
}
