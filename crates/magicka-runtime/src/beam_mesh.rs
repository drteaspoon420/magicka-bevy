//! Railgun beam geometry: ArcaneEffect's Beam technique evaluated on the CPU
//! (preshader + vs_2_0, research/beams.md) over Railgun's 512-vertex strip,
//! with RenderData.Draw's per-draw parameters. The pixel stage is
//! `1 - exp2(-(tex.r·ColorEdge + tex.g·ColorCenter)·alpha·tex.a)`, drawn
//! SrcAlpha/One with alpha 1 (additive).
use magicka_sim::railgun::RailRender;

type Vector = [f32; 3];

/// Railgun.VERTEXCOUNT: 256 pairs, drawn as a 510-triangle strip.
pub const VERTEX_COUNT: usize = 512;

/// One ArcaneEffect draw (effect parameters at CommitChanges).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeamDraw {
    pub origin: Vector,
    pub direction: Vector,
    pub length: f32,
    pub cut: f32,
    pub time: f32,
    pub clockwise: bool,
    pub wave_scale: f32,
    pub min_radius: f32,
    pub max_radius: f32,
    pub start_length: f32,
    pub dropoff: f32,
    pub ray_radius: f32,
    pub texture_scale: f32,
    pub alpha: f32,
    pub color_center: Vector,
    pub color_edge: Vector,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeamVertex {
    pub position: Vector,
    /// oD0.x: the pixel shader's colour multiplier.
    pub alpha: f32,
    pub uv: [f32; 2],
}

fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: Vector, s: f32) -> Vector {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn nrm(v: Vector) -> Vector {
    scale(v, 1.0 / (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt())
}

/// The Railgun vertex buffer: X = (i/255)², Y = ±1, texture (i/8, 0.5|0.625).
pub fn strip_input(i: usize) -> (f32, f32, f32) {
    let pair = (i / 2) as f32 / 255.0;
    if i.is_multiple_of(2) {
        (pair * pair, 1.0, 0.5)
    } else {
        (pair * pair, -1.0, 0.625)
    }
}

/// The Beam vertex shader for one vertex (`x` = position.X, `side` =
/// normal.X, `v` = texcoord.Y).
pub fn vertex(d: &BeamDraw, camera: Vector, x: f32, side: f32, v: f32) -> BeamVertex {
    // Preshader constants.
    let c0 = 1.0 - d.cut;
    let c1 = d.time * 0.5;
    let c2 = if d.clockwise { 1.0 } else { -1.0 };
    let flat = [-d.direction[2], 0.0, d.direction[0]];
    let inv = 1.0 / (flat[0] * flat[0] + flat[2] * flat[2]).sqrt();
    let c5 = [flat[0] * inv, flat[2] * inv];
    let up = [
        -c5[1] * d.direction[1],
        -c5[0] * d.direction[2] + c5[1] * d.direction[0],
        c5[0] * d.direction[1],
    ];
    let c4 = scale(
        up,
        1.0 / (up[0] * up[0] + up[1] * up[1] + up[2] * up[2]).sqrt(),
    );
    let c6 = d.dropoff.log2();
    let c7 = d.max_radius - d.min_radius;
    let c8 = 1.0 / d.start_length;
    let c9 = d.time * 0.1;
    // vs_2_0 body.
    let s = x * c0 + d.cut;
    let dist = s * d.length;
    let root = 1.0 / (1.0 / dist.sqrt());
    let wave = root * d.wave_scale - c1;
    // Shader constants are the compiled literals, not exact TAU/PI.
    #[allow(clippy::approx_constant)]
    let wrapped = (wave * 0.159155 + 0.5).fract_floor() * 6.28319 - 3.14159;
    let (sin, cos) = (wrapped.sin(), wrapped.cos());
    let mut offset = scale(c4, sin);
    offset[0] += c5[0] * cos * c2;
    offset[2] += c5[1] * cos * c2;
    let radius = (dist * c6).exp2() * c7 + d.min_radius;
    let ramp = (dist * c8).clamp(0.0, 1.0).powf(0.333);
    let center = add(d.origin, scale(d.direction, s * d.length));
    let p = add(center, scale(offset, radius * ramp));
    let across = nrm(cross(sub(p, camera), d.direction));
    BeamVertex {
        position: add(p, scale(across, side * d.ray_radius)),
        alpha: (dist * c8 * 3.0).clamp(0.0, 1.0) * d.alpha,
        uv: [dist * d.texture_scale - c9, v],
    }
}

trait FractFloor {
    fn fract_floor(self) -> Self;
}
impl FractFloor for f32 {
    /// D3D frc: x - floor(x).
    fn fract_floor(self) -> f32 {
        self - self.floor()
    }
}

/// All 512 vertices of one draw, in strip order.
pub fn strip(d: &BeamDraw, camera: Vector) -> Vec<BeamVertex> {
    (0..VERTEX_COUNT)
        .map(|i| {
            let (x, side, v) = strip_input(i);
            vertex(d, camera, x, side, v)
        })
        .collect()
}

/// Triangle-list order of a 510-triangle strip (alternating winding).
pub fn strip_triangles() -> Vec<u32> {
    let mut out = Vec::with_capacity(510 * 3);
    for t in 0..(VERTEX_COUNT as u32 - 2) {
        if t % 2 == 0 {
            out.extend([t, t + 1, t + 2]);
        } else {
            out.extend([t + 1, t, t + 2]);
        }
    }
    out
}

/// RenderData.Draw's parameter sequence for one plain (no additional
/// element) rail: the straight ray, then `Nr` spirals. Parameters persist
/// between draws on the effect; the ray draws with zero spiral radius, so
/// its stale WaveScale/Clockwice do not move vertices.
pub fn rail_draws(r: &RailRender) -> Vec<BeamDraw> {
    let mut d = BeamDraw {
        origin: r.position,
        direction: r.direction,
        length: r.length,
        cut: r.cut,
        time: r.time,
        clockwise: false,
        wave_scale: 0.0,
        min_radius: 0.0,
        max_radius: 0.0,
        start_length: 4.0,
        dropoff: 0.666,
        ray_radius: 0.333,
        texture_scale: 0.1,
        alpha: 1.0,
        color_center: r.color_center,
        color_edge: r.color_edge,
    };
    let mut out = vec![d];
    let mut num = 0.5f32;
    for i in 0..r.spirals {
        let root = num.sqrt();
        d.time = (r.time + num * 10.0) * root;
        d.clockwise = i % 2 == 0;
        d.wave_scale = 4.0 * root;
        d.min_radius = 0.15 * root;
        d.max_radius = if r.branch { 0.15 * root } else { 1.25 * root };
        d.ray_radius = 0.1 / root;
        out.push(d);
        num += 0.25;
    }
    out
}

// ---------------------------------------------------------------------------
// Railgun.AdditionalRenderData (beams carrying Water/Cold/Fire/Lightning/
// Steam/Poison): drawn *instead of* RenderData when mAdditionalElements is
// nonzero. See research/beam-additional.md.

/// Spell.LIGHTNINGCOLOR.
pub const LIGHTNING_COLOR: Vector = [0.75, 0.5, 1.0];

/// Railgun.ELEMENT_OFFSET_LOOKUP V offsets (rows of EffectTextures/Beams).
pub const ELEMENT_OFFSET_V: [f32; 11] = [
    0.0, 0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.0, 0.0, 0.75, 0.875,
];

/// One AdditiveEffect quad (TriangleFan v0..v3) with texture transform and
/// ColorTint applied: world positions, final UVs and the RGB tint (alpha 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdditionalQuad {
    pub positions: [Vector; 4],
    pub uvs: [[f32; 2]; 4],
    pub tint: Vector,
}

/// sAdditionalVertexBuffer: (x, y) positions and (u, v) texture coordinates.
const QUAD: [([f32; 2], [f32; 2]); 4] = [
    ([1.0, 1.0], [1.0, 0.125]),
    ([-1.0, 1.0], [1.0, 0.0]),
    ([-1.0, 0.0], [0.0, 0.0]),
    ([1.0, 0.0], [0.0, 0.125]),
];

fn dot3(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn normalized(a: Vector) -> Vector {
    let l = dot3(a, a).sqrt();
    if l > 0.0 {
        [a[0] / l, a[1] / l, a[2] / l]
    } else {
        a
    }
}

/// XNA Matrix.CreateConstrainedBillboard (objectForward null): returns the
/// Right and Up (= rotate axis) rows.
fn constrained_billboard(
    position: Vector,
    eye: Vector,
    axis: Vector,
    eye_forward: Vector,
) -> (Vector, Vector) {
    let d = [
        position[0] - eye[0],
        position[1] - eye[1],
        position[2] - eye[2],
    ];
    let len_sq = dot3(d, d);
    let to_object = if len_sq < 0.0001 {
        [-eye_forward[0], -eye_forward[1], -eye_forward[2]]
    } else {
        let s = 1.0 / len_sq.sqrt();
        [d[0] * s, d[1] * s, d[2] * s]
    };
    let right = if dot3(axis, to_object).abs() > 0.998_254_7 {
        // Nearly parallel: Forward, or Right when the axis is along Z.
        let fallback = if axis[2].abs() > 0.998_254_7 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 0.0, -1.0]
        };
        normalized(cross(axis, fallback))
    } else {
        normalized(cross(axis, to_object))
    };
    (right, axis)
}

/// AdditionalRenderData.PreRenderUpdate + Draw for one rail: per additional
/// element two quads, the second with Right ×0.666, offset
/// `u·1.3256675 + 0.3546517`, V +0.125 and V scale −1.
pub fn additional_quads(r: &RailRender, eye: Vector, eye_forward: Vector) -> Vec<AdditionalQuad> {
    let (right, up) = constrained_billboard(r.position, eye, r.direction, eye_forward);
    let span = r.length * (1.0 - r.cut);
    let origin = [
        r.position[0] + up[0] * r.length * r.cut,
        r.position[1] + up[1] * r.length * r.cut,
        r.position[2] + up[2] * r.length * r.cut,
    ];
    let mut out = Vec::new();
    for (i, &v) in ELEMENT_OFFSET_V.iter().enumerate() {
        let bit = 1u32 << i;
        if r.additional.0 & bit != bit {
            continue;
        }
        let tint = if bit == magicka_sim::spells::Elements::LIGHTNING.0 {
            LIGHTNING_COLOR.map(|c| c * 2.0)
        } else {
            [2.0; 3]
        };
        let scale_u = r.length * 0.0333;
        let offset_u = r.time * -0.05 + r.cut;
        let quad = |width: f32, offset: [f32; 2], scale_v: f32| AdditionalQuad {
            positions: QUAD.map(|([x, y], _)| {
                std::array::from_fn(|k| origin[k] + right[k] * x * width + up[k] * y * span)
            }),
            uvs: QUAD.map(|(_, [u, t])| [u * scale_u + offset[0], t * scale_v + offset[1]]),
            tint,
        };
        out.push(quad(1.0, [offset_u, v], 1.0));
        out.push(quad(
            0.666,
            [offset_u * 1.325_667_5 + 0.354_651_7, v + 0.125],
            -1.0,
        ));
    }
    out
}

#[cfg(test)]
mod additional_tests {
    use super::*;
    use magicka_sim::spells::Elements as E;

    fn rail(additional: E) -> RailRender {
        RailRender {
            rail: Default::default(),
            position: [0.0, 1.0, 0.0],
            direction: [0.0, 0.0, -1.0],
            length: 10.0,
            cut: 0.2,
            time: 2.0,
            color_center: [1.0; 3],
            color_edge: [1.0; 3],
            spirals: 4,
            branch: false,
            additional,
            light: ([0.0; 3], [0.0; 3]),
            light_color: [1.0; 3],
        }
    }

    #[test]
    fn fire_and_lightning_each_draw_two_textured_quads() {
        let r = rail(E(E::FIRE.0 | E::LIGHTNING.0));
        let q = additional_quads(&r, [0.0, 20.0, 0.0], [0.0, -1.0, 0.0]);
        assert_eq!(q.len(), 4);
        // Fire (bit 3) first: V row 0.25, tint 2; Lightning (bit 4): row
        // 0.375, 2·LIGHTNINGCOLOR.
        assert_eq!(q[0].tint, [2.0; 3]);
        assert_eq!(q[2].tint, [1.5, 1.0, 2.0]);
        assert_eq!(q[0].uvs[1], [10.0 * 0.0333 + (2.0 * -0.05 + 0.2), 0.25]);
        assert_eq!(q[2].uvs[2][1], 0.375);
        // The beam spans Position + Up·Length·Cut .. +Up·Length (cut 0.2).
        assert!(
            (q[0].positions[2][2] - -2.0).abs() < 1e-5,
            "{:?}",
            q[0].positions
        );
        assert!((q[0].positions[1][2] - -10.0).abs() < 1e-5);
        // Billboard: Right ⟂ beam and view, width ±1 (second quad ±0.666).
        assert!((q[0].positions[0][0] - q[0].positions[1][0]).abs() - 2.0 < 1e-5);
        assert!(((q[1].positions[0][0] - q[1].positions[1][0]).abs() - 1.332).abs() < 1e-5);
        // Second quad: V flipped within the next row.
        assert_eq!(q[1].uvs[0][1], 0.25 + 0.125 - 0.125);
    }

    #[test]
    fn pure_beams_have_no_additional_quads() {
        // mAdditionalElements excludes the beam elements themselves.
        assert!(additional_quads(&rail(E::NONE), [0.0, 20.0, 0.0], [0.0, -1.0, 0.0]).is_empty());
    }
}
