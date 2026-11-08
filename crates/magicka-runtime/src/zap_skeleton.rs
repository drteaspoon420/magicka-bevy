//! Lightning "zap" skeleton (research/zap.md): CharacterTemplate
//! GenerateSkeletonModel line strips and SkinnedModelSkeletonEffect
//! technique 1's vertex transform.
use magicka_content::skinned::{Bone, Mat};

/// A world-space line segment.
pub type Segment = [[f32; 3]; 2];

/// CharacterTemplate.VertexPositionIndexWeight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkeletonVertex {
    pub position: [f32; 3],
    pub bone: usize,
    pub next_position: [f32; 3],
    pub next_bone: usize,
    pub offset: [f32; 3],
    /// Color.White (true) or packed 0 (transparent separator).
    pub white: bool,
}

type Q = [f32; 4];

/// XNA Quaternion.CreateFromYawPitchRoll.
fn yaw_pitch_roll(yaw: f32, pitch: f32, roll: f32) -> Q {
    let (sr, cr) = (roll * 0.5).sin_cos();
    let (sp, cp) = (pitch * 0.5).sin_cos();
    let (sy, cy) = (yaw * 0.5).sin_cos();
    [
        cy * sp * cr + sy * cp * sr,
        sy * cp * cr - cy * sp * sr,
        cy * cp * sr - sy * sp * cr,
        cy * cp * cr + sy * sp * sr,
    ]
}

/// XNA Vector3.Transform(Vector3, Quaternion).
fn rotate(v: [f32; 3], q: Q) -> [f32; 3] {
    let [x, y, z, w] = q;
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (wx, wy, wz) = (w * x2, w * y2, w * z2);
    let (xx, xy, xz) = (x * x2, x * y2, x * z2);
    let (yy, yz, zz) = (y * y2, y * z2, z * z2);
    [
        v[0] * (1.0 - yy - zz) + v[1] * (xy - wz) + v[2] * (xz + wy),
        v[0] * (xy + wz) + v[1] * (1.0 - xx - zz) + v[2] * (yz - wx),
        v[0] * (xz - wy) + v[1] * (yz + wx) + v[2] * (1.0 - xx - yy),
    ]
}

/// Row-vector point transform (XNA Vector3.Transform(Vector3, Matrix)).
pub fn transform_point(v: [f32; 3], m: &Mat) -> [f32; 3] {
    [
        v[0] * m[0] + v[1] * m[4] + v[2] * m[8] + m[12],
        v[0] * m[1] + v[1] * m[5] + v[2] * m[9] + m[13],
        v[0] * m[2] + v[1] * m[6] + v[2] * m[10] + m[14],
    ]
}

fn bind_pose(bone: &Bone) -> Mat {
    crate::live_collision::inverse(&bone.inverse_bind).unwrap_or(magicka_content::skinned::IDENTITY)
}

fn depth(bones: &[Bone], mut i: usize) -> f32 {
    let mut n = 0.0;
    while let Some(p) = bones[i].parent {
        i = p;
        n += 1.0;
    }
    n
}

/// GenerateSkeletonModel(bones, scale): a ball per bone after the root and
/// a pyramid to each non-first bone's parent. `scale` is
/// `0.125 · template radius / model scale`.
pub fn generate(bones: &[Bone], scale: f32) -> Vec<SkeletonVertex> {
    let mut out = Vec::new();
    for i in 1..bones.len() {
        let radius = 0.8f32.powf(depth(bones, i) - 1.0) * scale;
        ball(&mut out, &bind_pose(&bones[i]), i, radius);
        if i > 1 {
            if let Some(parent) = bones[i].parent {
                pyramid(&mut out, bones, i, parent, radius);
            }
        }
    }
    out
}

fn ball(out: &mut Vec<SkeletonVertex>, transform: &Mat, bone: usize, radius: f32) {
    let mut v = [0.0, 0.0, radius];
    let mut item = SkeletonVertex {
        position: transform_point(v, transform),
        bone,
        next_position: [0.0; 3],
        next_bone: 0,
        offset: [0.0; 3],
        white: false,
    };
    out.push(item);
    item.white = true;
    out.push(item);
    let half_pi = std::f32::consts::FRAC_PI_2;
    for i in 0..12 {
        let mut sign = 1.0;
        if i >= 4 && i != 7 && i != 11 {
            sign = -1.0;
        }
        let num = sign / 2.0;
        let q = match i % 3 {
            0 => yaw_pitch_roll(0.0, num * -half_pi, 0.0),
            1 => yaw_pitch_roll(0.0, 0.0, num * -half_pi),
            _ => yaw_pitch_roll(num * half_pi, 0.0, 0.0),
        };
        for _ in 0..2 {
            v = rotate(v, q);
            item.position = transform_point(v, transform);
            out.push(item);
        }
    }
    item.white = false;
    out.push(item);
}

fn pyramid(out: &mut Vec<SkeletonVertex>, bones: &[Bone], child: usize, parent: usize, r: f32) {
    let child_t = {
        let m = bind_pose(&bones[child]);
        [m[12], m[13], m[14]]
    };
    let parent_t = {
        let m = bind_pose(&bones[parent]);
        [m[12], m[13], m[14]]
    };
    let mut apex = SkeletonVertex {
        position: child_t,
        bone: child,
        next_position: parent_t,
        next_bone: parent,
        offset: [0.0, 0.0, -r],
        white: true,
    };
    let mut base = SkeletonVertex {
        position: parent_t,
        bone: parent,
        next_position: child_t,
        next_bone: child,
        offset: [r / 0.8, 0.0, -r / 0.8],
        white: false,
    };
    let q = yaw_pitch_roll(0.0, 0.0, std::f32::consts::FRAC_PI_2);
    for _ in 0..4 {
        base.white = false;
        out.push(base);
        base.white = true;
        out.push(base);
        base.offset = rotate(base.offset, q);
        out.push(base);
        apex.white = true;
        out.push(apex);
        apex.white = false;
        out.push(apex);
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

/// SkinnedModelSkeletonEffect technique 1 vertex shader: both endpoints
/// skinned by one bone each, the offset expressed in a basis whose Z is the
/// direction from the next point to this one.
pub fn skin_vertex(v: &SkeletonVertex, skinned: &[Mat]) -> [f32; 3] {
    let this = transform_point(v.position, &skinned[v.bone]);
    if v.offset == [0.0; 3] {
        return this;
    }
    let next = transform_point(v.next_position, &skinned[v.next_bone]);
    let d = normalize([this[0] - next[0], this[1] - next[1], this[2] - next[2]]);
    let l = (d[0] * d[0] + d[2] * d[2]).sqrt();
    let x = [d[2] / l, 0.0, -d[0] / l];
    let y = [-d[1] * d[0] / l, l, -d[1] * d[2] / l];
    let o = v.offset;
    [
        this[0] + o[0] * x[0] + o[1] * y[0] + o[2] * d[0],
        this[1] + o[0] * x[1] + o[1] * y[1] + o[2] * d[1],
        this[2] + o[0] * x[2] + o[1] * y[2] + o[2] * d[2],
    ]
}

/// The visible LineStrip segments (both ends white) in world space.
pub fn world_lines(vertices: &[SkeletonVertex], skinned: &[Mat]) -> Vec<Segment> {
    vertices
        .windows(2)
        .filter(|w| w[0].white && w[1].white)
        .map(|w| [skin_vertex(&w[0], skinned), skin_vertex(&w[1], skinned)])
        .filter(|[a, b]| a.iter().chain(b).all(|v| v.is_finite()))
        .collect()
}

/// One LightningZapRenderData draw: the zapped character, its collision
/// radius (used by the viewer's line occlusion stand-in), skeleton lines and
/// the LightningHit flash sphere (absent when the model's bounding sphere is
/// unknown).
#[derive(Clone, Debug, PartialEq)]
pub struct ZapDraw {
    pub handle: u16,
    pub radius: f32,
    pub lines: Vec<Segment>,
    pub flash: Option<FlashSphere>,
}

/// Character render bounding sphere: Center = Position, Radius = model
/// Meshes[0] radius · mBoundingScale · Transform.M11.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlashSphere {
    pub center: [f32; 3],
    pub radius: f32,
}

/// sFlashVertexBuffer (VertexPositionTexture, drawn as a TriangleFan of two
/// triangles): position and texture coordinate.
pub const FLASH_QUAD: [([f32; 3], [f32; 2]); 4] = [
    ([1.0, 1.0, 0.0], [0.0, 0.0]),
    ([-1.0, 1.0, 0.0], [1.0, 0.0]),
    ([-1.0, -1.0, 0.0], [1.0, 1.0]),
    ([1.0, -1.0, 0.0], [0.0, 1.0]),
];

/// Fan (0,1,2), (0,2,3) as a triangle list.
pub const FLASH_TRIANGLES: [u32; 6] = [0, 1, 2, 0, 2, 3];

/// The flash scale relative to the bounding radius.
pub const FLASH_SCALE: f32 = 0.85;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalized(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|c| c / l)
}

/// XNA Matrix.CreateBillboard(object, camera, up, forward).
pub fn billboard(object: [f32; 3], camera: [f32; 3], up: [f32; 3], forward: [f32; 3]) -> Mat {
    let d = sub(object, camera);
    let l2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    let z = if l2 < 1e-4 {
        forward.map(|c| -c)
    } else {
        d.map(|c| c / l2.sqrt())
    };
    let x = normalized(cross(up, z));
    let y = cross(z, x);
    [
        x[0], x[1], x[2], 0.0, y[0], y[1], y[2], 0.0, z[0], z[1], z[2], 0.0, object[0], object[1],
        object[2], 1.0,
    ]
}

/// LightningZapRenderData.PreRenderUpdate: a billboard at the sphere centre
/// facing the camera, its 3×3 part scaled by 0.85·radius, then
/// CreateRotationZ(angle)·transform with angle = RandomBetween(0, 2π) drawn
/// every frame (the caller supplies it).
pub fn flash_transform(
    sphere: &FlashSphere,
    camera: [f32; 3],
    forward: [f32; 3],
    angle: f32,
) -> Mat {
    let mut m = billboard(sphere.center, camera, [0.0, 1.0, 0.0], forward);
    for row in 0..3 {
        for col in 0..3 {
            m[row * 4 + col] *= sphere.radius * FLASH_SCALE;
        }
    }
    let (s, c) = angle.sin_cos();
    let rz: Mat = [
        c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    magicka_content::skinned::mul(&rz, &m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use magicka_content::skinned::IDENTITY;

    fn bone(name: &str, parent: Option<usize>, x: f32, y: f32) -> Bone {
        let mut inverse_bind = IDENTITY;
        inverse_bind[12] = -x;
        inverse_bind[13] = -y;
        Bone {
            name: name.into(),
            parent,
            translation: [x, y, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
            inverse_bind,
        }
    }

    #[test]
    fn ball_ring_stays_on_its_sphere_and_counts_match_the_original() {
        let bones = [
            bone("root", None, 0.0, 0.0),
            bone("hip", Some(0), 0.0, 1.0),
            bone("spine", Some(1), 0.3, 2.0),
        ];
        let v = generate(&bones, 0.1);
        // Ball: 1 + 1 + 24 + 1 vertices; pyramid: 4 × 5.
        assert_eq!(v.len(), 27 + 27 + 20);
        // Hip ball (depth 1 → radius 0.1) around y = 1.
        for p in &v[1..26] {
            let d =
                ((p.position[0]).powi(2) + (p.position[1] - 1.0).powi(2) + p.position[2].powi(2))
                    .sqrt();
            assert!((d - 0.1).abs() < 1e-5, "{d}");
        }
    }

    #[test]
    fn yaw_pitch_roll_and_rotation_follow_xna() {
        // Roll π/2 rotates +X to +Y about Z.
        let q = yaw_pitch_roll(0.0, 0.0, std::f32::consts::FRAC_PI_2);
        let r = rotate([1.0, 0.0, 0.0], q);
        assert!((r[0]).abs() < 1e-6 && (r[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pyramid_apex_sits_toward_the_parent_from_the_child() {
        let bones = [
            bone("root", None, 0.0, 0.0),
            bone("hip", Some(0), 0.0, 1.0),
            bone("spine", Some(1), 0.3, 2.0),
        ];
        let skinned = vec![IDENTITY; 3];
        let v = generate(&bones, 0.1);
        let apex = v
            .iter()
            .find(|x| x.bone == 2 && x.offset[0] == 0.0 && (x.offset[2] + 0.08).abs() < 1e-6)
            .unwrap();
        // depth(spine) = 2 → radius 0.08; −Z moves 0.08 from the child
        // towards the parent.
        let p = skin_vertex(apex, &skinned);
        let to_child = [0.3 - p[0], 2.0 - p[1], -p[2]];
        let len = (to_child[0].powi(2) + to_child[1].powi(2) + to_child[2].powi(2)).sqrt();
        assert!((len - 0.08).abs() < 1e-5, "{p:?}");
        assert!(p[1] < 2.0);
        let lines = world_lines(&v, &skinned);
        assert!(!lines.is_empty());
    }

    #[test]
    fn billboard_faces_away_from_the_camera_with_world_up() {
        // Camera on +Z looking toward the origin: local Z points away from
        // the camera (−Z), local X = Up × Z = −X, local Y = +Y.
        let m = billboard(
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 10.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, -1.0],
        );
        let close = |a: &[f32], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-6);
        assert!(close(&m[0..3], [-1.0, 0.0, 0.0]), "{m:?}");
        assert!(close(&m[4..7], [0.0, 1.0, 0.0]), "{m:?}");
        assert!(close(&m[8..11], [0.0, 0.0, -1.0]), "{m:?}");
        assert!(close(&m[12..15], [0.0, 1.0, 0.0]), "{m:?}");
        // Degenerate distance: fall back to −forward.
        let m = billboard([0.0; 3], [0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]);
        assert!(close(&m[8..11], [0.0, 0.0, 1.0]), "{m:?}");
    }

    #[test]
    fn flash_quad_spans_085_radius_and_stays_camera_facing_when_rotated() {
        let sphere = FlashSphere {
            center: [2.0, 1.0, -3.0],
            radius: 2.0,
        };
        let camera = [2.0, 11.0, 7.0];
        let to_camera = normalized(sub(camera, sphere.center));
        for angle in [0.0, 0.7, 2.5, 6.0] {
            let m = flash_transform(&sphere, camera, [0.0, 0.0, -1.0], angle);
            for (p, _) in FLASH_QUAD {
                let w = transform_point(p, &m);
                let d = sub(w, sphere.center);
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                // Quad corner at distance √2 · 0.85 · radius in the view plane.
                assert!((len - 2f32.sqrt() * 0.85 * 2.0).abs() < 1e-4, "{len}");
                let along = d[0] * to_camera[0] + d[1] * to_camera[1] + d[2] * to_camera[2];
                assert!(along.abs() < 1e-4, "{along}");
            }
        }
    }
}
