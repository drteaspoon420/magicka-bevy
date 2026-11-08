//! Closest-point and segment/triangle queries shared by character physics and
//! navmesh pathfinding (reimplementations of JigLibX Distance/Intersection).
use crate::impact_motion::Vector;

fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: Vector, s: f32) -> Vector {
    [a[0] * s, a[1] * s, a[2] * s]
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
fn length_squared(a: Vector) -> f32 {
    dot(a, a)
}
// Closest-point geometry (reimplementation of JigLibX Distance routines)

/// Closest point on triangle to `p`; returns (distance², s, t) with the point
/// `a + s*(b-a) + t*(c-a)` (JigLibX Triangle.GetPoint parameterisation).
pub fn point_triangle_distance_sq(p: Vector, tri: &[Vector; 3]) -> (f32, f32, f32) {
    // Ericson, Real-Time Collision Detection 5.1.5.
    let [a, b, c] = *tri;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    let (s, t) = if d1 <= 0.0 && d2 <= 0.0 {
        (0.0, 0.0)
    } else {
        let bp = sub(p, b);
        let d3 = dot(ab, bp);
        let d4 = dot(ac, bp);
        if d3 >= 0.0 && d4 <= d3 {
            (1.0, 0.0)
        } else {
            let vc = d1 * d4 - d3 * d2;
            if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
                (d1 / (d1 - d3), 0.0)
            } else {
                let cp = sub(p, c);
                let d5 = dot(ab, cp);
                let d6 = dot(ac, cp);
                if d6 >= 0.0 && d5 <= d6 {
                    (0.0, 1.0)
                } else {
                    let vb = d5 * d2 - d1 * d6;
                    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
                        (0.0, d2 / (d2 - d6))
                    } else {
                        let va = d3 * d6 - d5 * d4;
                        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
                            let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
                            (1.0 - w, w)
                        } else if va + vb + vc == 0.0 {
                            // Zero-area triangle: nearest point on its edges.
                            return closest_on_edges(p, tri);
                        } else {
                            let denom = 1.0 / (va + vb + vc);
                            (vb * denom, vc * denom)
                        }
                    }
                }
            }
        }
    };
    let q = add(a, add(scale(ab, s), scale(ac, t)));
    (length_squared(sub(p, q)), s, t)
}

/// Triangle edge start, delta and map from edge parameter to (s, t).
type TriangleEdge = (Vector, Vector, fn(f32) -> (f32, f32));

/// Degenerate-triangle fallback: closest point among the three edges, in the
/// same (s, t) parameterisation.
fn closest_on_edges(p: Vector, tri: &[Vector; 3]) -> (f32, f32, f32) {
    let [a, b, c] = *tri;
    let mut best = (f32::INFINITY, 0.0, 0.0);
    let edges: [TriangleEdge; 3] = [
        (a, sub(b, a), |u| (u, 0.0)),
        (a, sub(c, a), |u| (0.0, u)),
        (b, sub(c, b), |u| (1.0 - u, u)),
    ];
    for (start, d, map) in edges {
        let (dist, _, u) = segment_segment(p, [0.0; 3], start, d);
        if dist < best.0 {
            let (s, t) = map(u);
            best = (dist, s, t);
        }
    }
    best
}

/// Squared distance between segments p0+s·d0 and p1+t·d1
/// (Distance.SegmentSegmentDistanceSq, equivalent reimplementation).
pub fn segment_segment_distance_sq(p0: Vector, d0: Vector, p1: Vector, d1: Vector) -> f32 {
    segment_segment(p0, d0, p1, d1).0
}

/// Closest points between segments p0+s*d0 and p1+t*d1 (s,t in [0,1]).
fn segment_segment(p0: Vector, d0: Vector, p1: Vector, d1: Vector) -> (f32, f32, f32) {
    let r = sub(p0, p1);
    let a = dot(d0, d0);
    let e = dot(d1, d1);
    let f = dot(d1, r);
    let (s, t);
    if a <= 1e-12 && e <= 1e-12 {
        s = 0.0;
        t = 0.0;
    } else if a <= 1e-12 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = dot(d0, r);
        if e <= 1e-12 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = dot(d0, d1);
            let denom = a * e - b * b;
            let mut s0 = if denom != 0.0 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t0 = 0.0;
                s0 = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t0 = 1.0;
                s0 = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s0;
            t = t0;
        }
    }
    let q0 = add(p0, scale(d0, s));
    let q1 = add(p1, scale(d1, t));
    (length_squared(sub(q0, q1)), s, t)
}

/// Segment/triangle closest points: (distance², segT, triS, triT).
/// Mathematically equivalent to Distance.SegmentTriangleDistanceSq; ties may
/// select a different but equidistant pair.
pub fn segment_triangle_distance_sq(
    origin: Vector,
    delta: Vector,
    tri: &[Vector; 3],
) -> (f32, f32, f32, f32) {
    let [a, b, c] = *tri;
    let e0 = sub(b, a);
    let e1 = sub(c, a);
    if let Some((seg_t, s, t)) = segment_triangle_hit(origin, delta, a, e0, e1) {
        return (0.0, seg_t, s, t);
    }
    let mut best = {
        let (d, s, t) = point_triangle_distance_sq(origin, tri);
        (d, 0.0, s, t)
    };
    let end = add(origin, delta);
    let (d, s, t) = point_triangle_distance_sq(end, tri);
    if d < best.0 {
        best = (d, 1.0, s, t);
    }
    // Triangle edges as (start, delta, map edge param -> (s,t)).
    let edges: [TriangleEdge; 3] = [
        (a, e0, |u| (u, 0.0)),
        (a, e1, |u| (0.0, u)),
        (b, sub(c, b), |u| (1.0 - u, u)),
    ];
    for (start, d1, map) in edges {
        let (d, seg_t, u) = segment_segment(origin, delta, start, d1);
        if d < best.0 {
            let (s, t) = map(u);
            best = (d, seg_t, s, t);
        }
    }
    best
}

/// Intersection.SegmentTriangleIntersection without backface rejection.
pub(crate) fn segment_triangle_hit(
    origin: Vector,
    delta: Vector,
    a: Vector,
    e0: Vector,
    e1: Vector,
) -> Option<(f32, f32, f32)> {
    let p = cross(delta, e1);
    let det = dot(e0, p);
    if det > -1e-6 && det < 1e-6 {
        return None;
    }
    let inv = 1.0 / det;
    let off = sub(origin, a);
    let u = dot(off, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(off, e0);
    let v = dot(delta, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e1, q) * inv;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some((t, u, v))
}

pub fn tri_point(tri: &[Vector; 3], s: f32, t: f32) -> Vector {
    add(
        tri[0],
        add(scale(sub(tri[1], tri[0]), s), scale(sub(tri[2], tri[0]), t)),
    )
}

/// JigLibX Distance.SegmentSegmentDistanceSq(out t0, out t1, seg0, seg1),
/// ported branch for branch (Eberly's region classification with JigLibX's
/// 1e-6 parallel threshold). Returns (distance², t0, t1). Callers that use
/// the parameters (Railgun crossing) need these exact branches: the generic
/// `segment_segment` picks different parameters for parallel segments.
pub fn jiglib_segment_segment(o0: Vector, d0: Vector, o1: Vector, d1: Vector) -> (f32, f32, f32) {
    let diff = sub(o0, o1);
    let a00 = length_squared(d0);
    let a01 = -dot(d0, d1);
    let a11 = length_squared(d1);
    let b0 = dot(diff, d0);
    let c = length_squared(diff);
    let det = (a00 * a11 - a01 * a01).abs();
    let (s, t, value);
    if det >= 1e-6 {
        let b1 = -dot(diff, d1);
        let mut s0 = a01 * b1 - a11 * b0;
        let mut t0 = a01 * b0 - a00 * b1;
        // Shared tails of the JigLibX branches.
        let s_end = |a: f32| -> (f32, f32, f32) {
            // t = 1 edge, minimise over s.
            let tmp = a01 + a;
            if tmp >= 0.0 {
                (0.0, 1.0, a11 + 2.0 * b1 + c)
            } else if -tmp >= a00 {
                (1.0, 1.0, a00 + a11 + c + 2.0 * (b1 + tmp))
            } else {
                let s = -tmp / a00;
                (s, 1.0, tmp * s + a11 + 2.0 * b1 + c)
            }
        };
        let t_zero_s = || -> (f32, f32, f32) {
            // t = 0 edge, minimise over s.
            if b0 >= 0.0 {
                (0.0, 0.0, c)
            } else if -b0 >= a00 {
                (1.0, 0.0, a00 + 2.0 * b0 + c)
            } else {
                let s = -b0 / a00;
                (s, 0.0, b0 * s + c)
            }
        };
        let s_one_t = || -> (f32, f32, f32) {
            // s = 1 edge, minimise over t.
            let tmp = a01 + b1;
            if tmp >= 0.0 {
                (1.0, 0.0, a00 + 2.0 * b0 + c)
            } else if -tmp >= a11 {
                (1.0, 1.0, a00 + a11 + c + 2.0 * (b0 + tmp))
            } else {
                let t = -tmp / a11;
                (1.0, t, tmp * t + a00 + 2.0 * b0 + c)
            }
        };
        let s_zero_t = || -> (f32, f32, f32) {
            // s = 0 edge, minimise over t.
            if b1 >= 0.0 {
                (0.0, 0.0, c)
            } else if -b1 >= a11 {
                (0.0, 1.0, a11 + 2.0 * b1 + c)
            } else {
                let t = -b1 / a11;
                (0.0, t, b1 * t + c)
            }
        };
        let r = if s0 >= 0.0 {
            if s0 <= det {
                if t0 >= 0.0 {
                    if t0 <= det {
                        let inv = 1.0 / det;
                        s0 *= inv;
                        t0 *= inv;
                        let v = s0 * (a00 * s0 + a01 * t0 + 2.0 * b0)
                            + t0 * (a01 * s0 + a11 * t0 + 2.0 * b1)
                            + c;
                        (s0, t0, v)
                    } else {
                        s_end(b0)
                    }
                } else {
                    t_zero_s()
                }
            } else if t0 >= 0.0 {
                if t0 <= det {
                    s_one_t()
                } else {
                    let tmp = a01 + b0;
                    if -tmp <= a00 {
                        if tmp >= 0.0 {
                            (0.0, 1.0, a11 + 2.0 * b1 + c)
                        } else {
                            let s = -tmp / a00;
                            (s, 1.0, tmp * s + a11 + 2.0 * b1 + c)
                        }
                    } else {
                        s_one_t()
                    }
                }
            } else if -b0 < a00 {
                if b0 >= 0.0 {
                    (0.0, 0.0, c)
                } else {
                    let s = -b0 / a00;
                    (s, 0.0, b0 * s + c)
                }
            } else {
                s_one_t()
            }
        } else if t0 >= 0.0 {
            if t0 <= det {
                s_zero_t()
            } else {
                let tmp = a01 + b0;
                if tmp < 0.0 {
                    if -tmp >= a00 {
                        (1.0, 1.0, a00 + a11 + c + 2.0 * (b1 + tmp))
                    } else {
                        let s = -tmp / a00;
                        (s, 1.0, tmp * s + a11 + 2.0 * b1 + c)
                    }
                } else {
                    s_zero_t()
                }
            }
        } else if b0 < 0.0 {
            if -b0 >= a00 {
                (1.0, 0.0, a00 + 2.0 * b0 + c)
            } else {
                let s = -b0 / a00;
                (s, 0.0, b0 * s + c)
            }
        } else {
            s_zero_t()
        };
        (s, t, value) = r;
    } else if a01 > 0.0 {
        if b0 >= 0.0 {
            (s, t, value) = (0.0, 0.0, c);
        } else if -b0 <= a00 {
            let s0 = -b0 / a00;
            (s, t, value) = (s0, 0.0, b0 * s0 + c);
        } else {
            let b1 = -dot(diff, d1);
            let tmp = a00 + b0;
            if -tmp >= a01 {
                (s, t, value) = (1.0, 1.0, a00 + a11 + c + 2.0 * (a01 + b0 + b1));
            } else {
                let t0 = -tmp / a01;
                (s, t, value) = (
                    1.0,
                    t0,
                    a00 + 2.0 * b0 + c + t0 * (a11 * t0 + 2.0 * (a01 + b1)),
                );
            }
        }
    } else if -b0 >= a00 {
        (s, t, value) = (1.0, 0.0, a00 + 2.0 * b0 + c);
    } else if b0 <= 0.0 {
        let s0 = -b0 / a00;
        (s, t, value) = (s0, 0.0, b0 * s0 + c);
    } else {
        let b1 = -dot(diff, d1);
        if b0 >= -a01 {
            (s, t, value) = (0.0, 1.0, a11 + 2.0 * b1 + c);
        } else {
            let t0 = -b0 / a01;
            (s, t, value) = (0.0, t0, c + t0 * (2.0 * b1 + a11 * t0));
        }
    }
    (value.abs(), s, t)
}
