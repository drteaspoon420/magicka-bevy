//! LightningBolt render geometry (research/lightning.md): a 34-vertex
//! triangle strip per pooled bolt, transformed by InitializeEffect's
//! orientation and drawn with `mDrawCount` primitives.
use crate::particles::DotNetRandom;

type Vector = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StripVertex {
    pub position: Vector,
    pub uv: [f32; 2],
}

/// The LightningBolt constructor's vertex buffer: a random texture column
/// offset (Next(8)·0.125), then a lateral jitter (RandomBetween(−0.75,
/// 0.75)) for each vertex pair after the first.
pub fn strip(rng: &mut DotNetRandom) -> [StripVertex; 34] {
    let offset = rng.next_below(8) as f32 * 0.125;
    let mut jitter = 0.0f32;
    std::array::from_fn(|i| {
        if i > 0 && i % 2 == 0 {
            let t = rng.next_double() as f32;
            jitter = -0.75 * (1.0 - t) + 0.75 * t;
        }
        StripVertex {
            position: [
                -((i % 2) as f32 - 0.5 + jitter),
                0.0,
                -((i / 2) as f32) * 2.0,
            ],
            uv: [
                (i / 2) as f32 * 0.125 + offset,
                0.375 + (i % 2) as f32 / 8.0,
            ],
        }
    })
}

/// mDrawCount: `min(floor(d/scale/2 + 0.5)·2, 32)` strip primitives.
pub fn draw_count(from: Vector, to: Vector, scale: f32) -> usize {
    let d = distance(from, to) / scale;
    ((d / 2.0 + 0.5).floor() as i32 * 2).clamp(0, 32) as usize
}

fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn distance(a: Vector, b: Vector) -> f32 {
    let d = sub(a, b);
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}
fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(v: Vector) -> Vector {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / l)
}

/// World-space triangle list (both windings, as the strip is drawn
/// unculled) for a bolt from `from` to `to` seen from `eye`:
/// `Scale(s, s, s·(d/s)/count) × [Right, Up, Forward]`, translated to
/// `from`; Right = Forward × (eye − from), Up = Right × Forward.
pub fn triangles(
    strip: &[StripVertex; 34],
    from: Vector,
    to: Vector,
    eye: Vector,
    scale: f32,
) -> Vec<StripVertex> {
    let count = draw_count(from, to, scale);
    if count == 0 {
        return Vec::new();
    }
    let along = distance(from, to) / scale / count as f32;
    let forward = normalize(sub(to, from));
    let right = normalize(cross(forward, sub(eye, from)));
    let up = normalize(cross(right, forward));
    // XNA Matrix.Forward stores −forward in the third row.
    let world = |v: &StripVertex| StripVertex {
        position: std::array::from_fn(|k| {
            from[k] + v.position[0] * scale * right[k] + v.position[1] * scale * up[k]
                - v.position[2] * scale * along * forward[k]
        }),
        uv: v.uv,
    };
    let mut out = Vec::with_capacity(count * 6);
    for t in 0..count {
        let (a, b, c) = (world(&strip[t]), world(&strip[t + 1]), world(&strip[t + 2]));
        out.extend([a, b, c, a, c, b]);
    }
    out
}
