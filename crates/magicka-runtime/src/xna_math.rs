//! XNA Vector3/Matrix/Quaternion helpers on plain arrays (row vectors,
//! translation in elements 12..15, Forward = −row 2), shared by the named
//! boss ports.
use magicka_content::skinned::Mat;

pub type V3 = [f32; 3];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}
pub fn length_squared(a: V3) -> f32 {
    dot(a, a)
}
/// Vector3.Normalize (a zero vector becomes NaN natively; kept as is here).
pub fn normalize(a: V3) -> V3 {
    let l = length(a);
    if l > 0.0 {
        scale(a, 1.0 / l)
    } else {
        a
    }
}
pub fn lerp(a: V3, b: V3, t: f32) -> V3 {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}
pub fn translation(m: &Mat) -> V3 {
    [m[12], m[13], m[14]]
}
pub fn set_translation(m: &mut Mat, t: V3) {
    m[12..15].copy_from_slice(&t);
}
pub fn forward(m: &Mat) -> V3 {
    [-m[8], -m[9], -m[10]]
}
pub fn backward(m: &Mat) -> V3 {
    [m[8], m[9], m[10]]
}
/// Vector3.Transform(point, matrix).
pub fn transform_point(p: V3, m: &Mat) -> V3 {
    [0, 1, 2].map(|c| p[0] * m[c] + p[1] * m[4 + c] + p[2] * m[8 + c] + m[12 + c])
}
/// Vector3.TransformNormal.
pub fn transform_normal(v: V3, m: &Mat) -> V3 {
    [0, 1, 2].map(|c| v[0] * m[c] + v[1] * m[4 + c] + v[2] * m[8 + c])
}
/// Matrix.CreateWorld(position, forward, up).
pub fn create_world(position: V3, fwd: V3, up: V3) -> Mat {
    let z = normalize(scale(fwd, -1.0));
    let x = normalize(cross(up, z));
    let y = cross(z, x);
    [
        x[0],
        x[1],
        x[2],
        0.0,
        y[0],
        y[1],
        y[2],
        0.0,
        z[0],
        z[1],
        z[2],
        0.0,
        position[0],
        position[1],
        position[2],
        1.0,
    ]
}
/// Vector3.Transform(v, Quaternion.CreateFromYawPitchRoll(yaw, 0, 0)).
pub fn yaw(v: V3, angle: f32) -> V3 {
    let (s, c) = angle.sin_cos();
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}
/// Matrix.Transform(m, yaw quaternion) with the translation restored:
/// every basis row turned about world Y.
pub fn yaw_matrix(m: &Mat, angle: f32) -> Mat {
    let mut out = *m;
    for row in 0..3 {
        let r = yaw([m[row * 4], m[row * 4 + 1], m[row * 4 + 2]], angle);
        out[row * 4..row * 4 + 3].copy_from_slice(&r);
    }
    out
}
/// MagickaMath.IsApproximately(Vector3, Vector3, precision).
pub fn approximately(a: V3, b: V3, precision: f32) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= precision)
}
/// MagickaMath.Angle(Vector2) of (x, z): acos(dot with UnitX), negated
/// for negative z.
pub fn angle_xz(v: V3) -> f32 {
    let a = v[0].clamp(-1.0, 1.0).acos();
    if v[2] < 0.0 {
        -a
    } else {
        a
    }
}
/// MagickaMath.ConstrainVector: turn `v` about Y so its xz angle relative
/// to `normal` lies within [min, max].
pub fn constrain_vector(v: V3, normal: V3, min: f32, max: f32) -> V3 {
    let d = angle_xz(v) - angle_xz(normal);
    if d < min {
        yaw(v, d - min)
    } else if d > max {
        yaw(v, d - max)
    } else {
        v
    }
}
/// The bosses' GetAngle(forward, target direction): acos(Cross(target,
/// forward).Y) − π/2, a signed yaw (positive turns left). Unclamped: a
/// cross product past ±1 yields NaN as in .NET.
pub fn signed_angle(forward: V3, target_direction: V3) -> f32 {
    let c = cross(target_direction, forward);
    (c[1] as f64).acos() as f32 - std::f32::consts::FRAC_PI_2
}
/// GetAngle(position, forward, target): the flattened direction first.
pub fn signed_angle_to(position: V3, forward: V3, target: V3) -> f32 {
    let mut d = sub(target, position);
    d[1] = 0.0;
    signed_angle(forward, normalize(d))
}
/// Closest distance squared between point `p` and segment [a, b].
pub fn point_segment_distance_squared(p: V3, a: V3, b: V3) -> f32 {
    let ab = sub(b, a);
    let l = length_squared(ab);
    let t = if l > 0.0 {
        (dot(sub(p, a), ab) / l).clamp(0.0, 1.0)
    } else {
        0.0
    };
    length_squared(sub(add(a, scale(ab, t)), p))
}
/// Closest distance squared between segments [a0, a1] and [b0, b1].
pub fn segment_segment_distance_squared(a0: V3, a1: V3, b0: V3, b1: V3) -> f32 {
    let d1 = sub(a1, a0);
    let d2 = sub(b1, b0);
    let r = sub(a0, b0);
    let a = dot(d1, d1);
    let e = dot(d2, d2);
    let f = dot(d2, r);
    let (s, t) = if a <= f32::EPSILON && e <= f32::EPSILON {
        (0.0, 0.0)
    } else if a <= f32::EPSILON {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = dot(d1, r);
        if e <= f32::EPSILON {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = dot(d1, d2);
            let denom = a * e - b * b;
            let mut s = if denom != 0.0 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    length_squared(sub(add(a0, scale(d1, s)), add(b0, scale(d2, t))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_angle_is_positive_to_the_left() {
        let fwd = [0.0, 0.0, -1.0];
        // Target to the left (−X) of a −Z-facing boss.
        assert!(signed_angle(fwd, [-1.0, 0.0, 0.0]) > 1.5);
        assert!(signed_angle(fwd, [1.0, 0.0, 0.0]) < -1.5);
        assert!(signed_angle(fwd, fwd).abs() < 1e-3);
        // A positive yaw turns forward toward the left target.
        let turned = yaw(fwd, 0.3);
        assert!(signed_angle(turned, [-1.0, 0.0, 0.0]) < signed_angle(fwd, [-1.0, 0.0, 0.0]));
    }

    #[test]
    fn constrain_vector_clamps_the_xz_angle() {
        let n = [0.0, 0.0, 1.0];
        let v = constrain_vector([1.0, 0.0, 0.0], n, -0.7, 0.7);
        let rel = angle_xz(v) - angle_xz(n);
        assert!((rel + 0.7).abs() < 1e-4, "{rel}");
    }

    #[test]
    fn segment_distance() {
        let d = segment_segment_distance_squared(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.5, 1.0, -1.0],
            [0.5, 1.0, 1.0],
        );
        assert!((d - 1.0).abs() < 1e-6);
    }
}

/// Matrix.Invert (cofactor expansion; a singular matrix yields non-finite
/// values as in XNA).
pub fn invert(m: &Mat) -> Mat {
    let a = |r: usize, c: usize| m[r * 4 + c];
    let (b0, b1, b2) = (
        a(0, 0) * a(1, 1) - a(0, 1) * a(1, 0),
        a(0, 0) * a(1, 2) - a(0, 2) * a(1, 0),
        a(0, 0) * a(1, 3) - a(0, 3) * a(1, 0),
    );
    let (b3, b4, b5) = (
        a(0, 1) * a(1, 2) - a(0, 2) * a(1, 1),
        a(0, 1) * a(1, 3) - a(0, 3) * a(1, 1),
        a(0, 2) * a(1, 3) - a(0, 3) * a(1, 2),
    );
    let (b6, b7, b8) = (
        a(2, 0) * a(3, 1) - a(2, 1) * a(3, 0),
        a(2, 0) * a(3, 2) - a(2, 2) * a(3, 0),
        a(2, 0) * a(3, 3) - a(2, 3) * a(3, 0),
    );
    let (b9, b10, b11) = (
        a(2, 1) * a(3, 2) - a(2, 2) * a(3, 1),
        a(2, 1) * a(3, 3) - a(2, 3) * a(3, 1),
        a(2, 2) * a(3, 3) - a(2, 3) * a(3, 2),
    );
    let det = b0 * b11 - b1 * b10 + b2 * b9 + b3 * b8 - b4 * b7 + b5 * b6;
    let inv = 1.0 / det;
    [
        (a(1, 1) * b11 - a(1, 2) * b10 + a(1, 3) * b9) * inv,
        (-a(0, 1) * b11 + a(0, 2) * b10 - a(0, 3) * b9) * inv,
        (a(3, 1) * b5 - a(3, 2) * b4 + a(3, 3) * b3) * inv,
        (-a(2, 1) * b5 + a(2, 2) * b4 - a(2, 3) * b3) * inv,
        (-a(1, 0) * b11 + a(1, 2) * b8 - a(1, 3) * b7) * inv,
        (a(0, 0) * b11 - a(0, 2) * b8 + a(0, 3) * b7) * inv,
        (-a(3, 0) * b5 + a(3, 2) * b2 - a(3, 3) * b1) * inv,
        (a(2, 0) * b5 - a(2, 2) * b2 + a(2, 3) * b1) * inv,
        (a(1, 0) * b10 - a(1, 1) * b8 + a(1, 3) * b6) * inv,
        (-a(0, 0) * b10 + a(0, 1) * b8 - a(0, 3) * b6) * inv,
        (a(3, 0) * b4 - a(3, 1) * b2 + a(3, 3) * b0) * inv,
        (-a(2, 0) * b4 + a(2, 1) * b2 - a(2, 3) * b0) * inv,
        (-a(1, 0) * b9 + a(1, 1) * b7 - a(1, 2) * b6) * inv,
        (a(0, 0) * b9 - a(0, 1) * b7 + a(0, 2) * b6) * inv,
        (-a(3, 0) * b3 + a(3, 1) * b1 - a(3, 2) * b0) * inv,
        (a(2, 0) * b3 - a(2, 1) * b1 + a(2, 2) * b0) * inv,
    ]
}

/// Vector4.Transform((x, y, z, w), matrix).
pub fn transform_vector4(v: [f32; 4], m: &Mat) -> [f32; 4] {
    [0, 1, 2, 3].map(|c| v[0] * m[c] + v[1] * m[4 + c] + v[2] * m[8 + c] + v[3] * m[12 + c])
}

#[cfg(test)]
mod invert_tests {
    #[test]
    fn invert_round_trips_a_view_projection() {
        let m = [
            2.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.3, 0.0, 0.0, -0.2, 1.1, -1.0, 3.0, -4.0, 7.0, 1.0,
        ];
        let i = super::invert(&m);
        let p = crate::camera::multiply(&m, &i);
        for r in 0..4 {
            for c in 0..4 {
                let want = if r == c { 1.0 } else { 0.0 };
                assert!(
                    (p[r * 4 + c] - want).abs() < 1e-5,
                    "{r},{c}: {}",
                    p[r * 4 + c]
                );
            }
        }
    }
}
