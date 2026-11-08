//! XNAnimation skinned models: skeleton/clip resolution and pose math.
//!
//! Source (decompiled XNAnimation.dll): `SkinnedModel.Read` stores bones and
//! clips as shared-resource references; `SkinnedModelBone` holds the bind
//! pose (local TRS) and inverse bind matrix; `AnimationController.
//! UpdateAbsoluteBoneTransforms` computes `abs[0] = local[0]·parent`,
//! `abs[i] = local[i]·abs[parent(i)]`, `skin[i] = invBind[i]·abs[i]`
//! (row-vector order); `Pose.GetMatrix` = rotation, translation row, then
//! the three matrix columns scaled by Scale.

use crate::assets::{AnimationClip, Asset, SkinnedModel};
use crate::xna::{Model, XnaObject};

/// Row-major 4x4, XNA row-vector convention.
pub type Mat = [f32; 16];

#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    pub name: String,
    /// Index of the parent within the skeleton (None = root).
    pub parent: Option<usize>,
    pub translation: [f32; 3],
    pub orientation: [f32; 4],
    pub scale: [f32; 3],
    pub inverse_bind: Mat,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

/// A skinned model with its shared references resolved.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub model: Option<Model>,
    pub skeleton: Skeleton,
    pub clips: Vec<AnimationClip>,
    /// Shared resources of the file (mesh-part effects are looked up here).
    pub shared: Vec<Option<XnaObject>>,
}

fn shared_asset(shared: &[Option<XnaObject>], r: u32) -> Option<&Asset> {
    match shared.get((r as usize).checked_sub(1)?)? {
        Some(XnaObject::Asset(a)) => Some(a),
        _ => None,
    }
}

/// Resolve bone/clip references of a `SkinnedModel` against its file's
/// shared resources. Errors describe the first inconsistency.
pub fn resolve(sm: &SkinnedModel, shared: Vec<Option<XnaObject>>) -> Result<Resolved, String> {
    let mut bones = Vec::with_capacity(sm.bones.len());
    // Shared index -> skeleton position.
    let pos: std::collections::HashMap<u32, usize> =
        sm.bones.iter().enumerate().map(|(i, r)| (*r, i)).collect();
    for (i, r) in sm.bones.iter().enumerate() {
        let Some(Asset::SkinnedModelBone(b)) = shared_asset(&shared, *r) else {
            return Err(format!("bone ref {r} is not a SkinnedModelBone"));
        };
        if b.index as usize != i {
            return Err(format!(
                "bone {} stored index {} != skeleton position {i}",
                b.name, b.index
            ));
        }
        let parent = if b.parent == 0 {
            None
        } else {
            Some(*pos.get(&b.parent).ok_or_else(|| {
                format!("bone {} parent ref {} not in skeleton", b.name, b.parent)
            })?)
        };
        if parent.is_some_and(|p| p >= i) {
            return Err(format!(
                "bone {} parent {parent:?} not before child {i}",
                b.name
            ));
        }
        bones.push(Bone {
            name: b.name.clone(),
            parent,
            translation: b.translation,
            orientation: b.orientation,
            scale: b.scale,
            inverse_bind: b.inverse_bind_pose,
        });
    }
    let mut clips = Vec::new();
    for r in &sm.clips {
        match shared_asset(&shared, *r) {
            Some(Asset::AnimationClip(c)) => clips.push(c.clone()),
            _ => return Err(format!("clip ref {r} is not an AnimationClip")),
        }
    }
    Ok(Resolved {
        model: sm.model.clone(),
        skeleton: Skeleton { bones },
        clips,
        shared,
    })
}

pub fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut r = [0f32; 16];
    for i in 0..4 {
        for j in 0..4 {
            r[i * 4 + j] = (0..4).map(|k| a[i * 4 + k] * b[k * 4 + j]).sum();
        }
    }
    r
}

pub const IDENTITY: Mat = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];

/// `Pose.GetMatrix` (XNA `Matrix.CreateFromQuaternion` does not normalize).
pub fn pose_matrix(t: [f32; 3], q: [f32; 4], s: [f32; 3]) -> Mat {
    let [x, y, z, w] = q;
    let mut m = [
        1. - 2. * (y * y + z * z),
        2. * (x * y + z * w),
        2. * (x * z - y * w),
        0.,
        2. * (x * y - z * w),
        1. - 2. * (x * x + z * z),
        2. * (y * z + x * w),
        0.,
        2. * (x * z + y * w),
        2. * (y * z - x * w),
        1. - 2. * (x * x + y * y),
        0.,
        t[0],
        t[1],
        t[2],
        1.,
    ];
    for row in 0..3 {
        for (col, sc) in s.iter().enumerate() {
            m[row * 4 + col] *= sc;
        }
    }
    m
}

impl Skeleton {
    /// Absolute bind-pose matrices (`local · abs[parent]`).
    pub fn bind_absolute(&self) -> Vec<Mat> {
        let mut abs: Vec<Mat> = Vec::with_capacity(self.bones.len());
        for b in &self.bones {
            let local = pose_matrix(b.translation, b.orientation, b.scale);
            let m = match b.parent {
                Some(p) => mul(&local, &abs[p]),
                None => local,
            };
            abs.push(m);
        }
        abs
    }

    /// Max |invBind · absBind − I| over all bones: ~0 when the stored inverse
    /// bind matrices are consistent with the bind poses and hierarchy.
    pub fn bind_error(&self) -> f32 {
        let abs = self.bind_absolute();
        let mut worst = 0f32;
        for (b, a) in self.bones.iter().zip(&abs) {
            let m = mul(&b.inverse_bind, a);
            for (x, i) in m.iter().zip(IDENTITY.iter()) {
                worst = worst.max((x - i).abs());
            }
        }
        worst
    }

    /// Per-bone |invBind · absBind − I| (max element).
    pub fn bind_errors(&self) -> Vec<f32> {
        self.bones
            .iter()
            .zip(self.bind_absolute())
            .map(|(b, a)| {
                mul(&b.inverse_bind, &a)
                    .iter()
                    .zip(IDENTITY.iter())
                    .map(|(x, i)| (x - i).abs())
                    .fold(0f32, f32::max)
            })
            .collect()
    }

    /// True if bone `i` or any ancestor has a non-uniform bind scale.
    pub fn under_nonuniform_scale(&self, mut i: usize) -> bool {
        loop {
            let s = self.bones[i].scale;
            if (s[0] - s[1]).abs() > 1e-4 || (s[0] - s[2]).abs() > 1e-4 {
                return true;
            }
            match self.bones[i].parent {
                Some(p) => i = p,
                None => return false,
            }
        }
    }

    pub fn names(&self) -> Vec<&str> {
        self.bones.iter().map(|b| b.name.as_str()).collect()
    }
}

/// XNA `Quaternion.Lerp`: sign-corrected linear blend, then normalized.
pub fn xna_quat_lerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let t2 = 1.0 - t;
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let sign = if dot >= 0.0 { 1.0 } else { -1.0 };
    let r = [0, 1, 2, 3].map(|k| t2 * a[k] + sign * t * b[k]);
    let l = (r.iter().map(|x| x * x).sum::<f32>()).sqrt();
    r.map(|x| x / l)
}

/// `AnimationChannel.GetKeyframeIndexByTime` (binary search, then step back
/// if the found key is after `time`).
pub fn keyframe_index(keys: &[crate::assets::Keyframe], time: f32) -> usize {
    let (mut num, mut lo, mut hi) = (0i64, 0i64, keys.len() as i64 - 1);
    while hi >= lo {
        num = (lo + hi) / 2;
        let t = keys[num as usize].time;
        if t < time {
            lo = num + 1;
        } else if t > time {
            hi = num - 1;
        } else {
            break;
        }
    }
    if num > 0 && keys[num as usize].time > time {
        num -= 1;
    }
    num as usize
}

/// Local pose of a channel at `time` for a non-looping clip, as
/// `AnimationController` computes it with `InterpolationMode.Linear`.
pub fn sample_channel(
    keys: &[crate::assets::Keyframe],
    time: f32,
) -> ([f32; 3], [f32; 4], [f32; 3]) {
    let i = keyframe_index(keys, time);
    let mut j = (i + 1) % keys.len();
    if j == 0 {
        j = i; // non-looping: hold the last key
    }
    let (a, b) = (&keys[i], &keys[j]);
    if i == j {
        return (a.translation, a.orientation, a.scale);
    }
    let span = b.time - a.time;
    if span <= 0.0 {
        return (a.translation, a.orientation, a.scale);
    }
    let u = ((time - a.time) / span).clamp(0.0, 1.0);
    let lerp3 = |x: [f32; 3], y: [f32; 3]| [0, 1, 2].map(|k| x[k] + (y[k] - x[k]) * u);
    (
        lerp3(a.translation, b.translation),
        xna_quat_lerp(a.orientation, b.orientation, u),
        lerp3(a.scale, b.scale),
    )
}

/// `AnimationController.UpdateAbsoluteBoneTransforms` for `clip` at `time`
/// with world `parent`: returns the skinning matrices `invBind · abs`.
pub fn skin_matrices(
    skel: &Skeleton,
    clip: &crate::assets::AnimationClip,
    time: f32,
    parent: &Mat,
) -> Vec<Mat> {
    let channels: std::collections::HashMap<&str, &Vec<crate::assets::Keyframe>> =
        clip.channels.iter().map(|(n, k)| (n.as_str(), k)).collect();
    let mut abs: Vec<Mat> = Vec::with_capacity(skel.bones.len());
    for b in &skel.bones {
        let (t, q, s) = match channels.get(b.name.as_str()) {
            Some(keys) if !keys.is_empty() => sample_channel(keys, time),
            _ => (b.translation, b.orientation, b.scale),
        };
        let local = pose_matrix(t, q, s);
        let m = match b.parent {
            Some(p) => mul(&local, &abs[p]),
            None => mul(&local, parent),
        };
        abs.push(m);
    }
    skel.bones
        .iter()
        .zip(&abs)
        .map(|(b, a)| mul(&b.inverse_bind, a))
        .collect()
}

/// Linear-blend skin one vertex (row-vector `v · M`).
pub fn skin_vertex(p: [f32; 3], joints: [u8; 4], weights: [f32; 4], skins: &[Mat]) -> [f32; 3] {
    let mut out = [0f32; 3];
    for k in 0..4 {
        if weights[k] == 0.0 {
            continue;
        }
        let m = &skins[joints[k] as usize];
        for c in 0..3 {
            out[c] += weights[k] * (p[0] * m[c] + p[1] * m[4 + c] + p[2] * m[8 + c] + m[12 + c]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pose_matrix_matches_xna_conventions() {
        // 90 degrees about +Y: XNA maps +X to -Z (row-vector v·M).
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let m = pose_matrix([1., 2., 3.], [0., h, 0., h], [1., 1., 1.]);
        let v = [1f32, 0., 0., 1.];
        let out: Vec<f32> = (0..4)
            .map(|j| (0..4).map(|k| v[k] * m[k * 4 + j]).sum())
            .collect();
        assert!(
            (out[0] - 1.).abs() < 1e-6 && (out[1] - 2.).abs() < 1e-6 && (out[2] - 2.).abs() < 1e-6,
            "{out:?}"
        );
    }

    #[test]
    fn consistent_skeleton_has_zero_bind_error() {
        let root = pose_matrix([0., 1., 0.], [0., 0., 0., 1.], [1., 1., 1.]);
        let child_local = pose_matrix([0., 0., 2.], [0., 0., 0., 1.], [1., 1., 1.]);
        let inv = |m: Mat| {
            let mut r = m;
            r[12] = -m[12];
            r[13] = -m[13];
            r[14] = -m[14];
            r
        };
        let abs_child = mul(&child_local, &root);
        let s = Skeleton {
            bones: vec![
                Bone {
                    name: "root".into(),
                    parent: None,
                    translation: [0., 1., 0.],
                    orientation: [0., 0., 0., 1.],
                    scale: [1.; 3],
                    inverse_bind: inv(root),
                },
                Bone {
                    name: "c".into(),
                    parent: Some(0),
                    translation: [0., 0., 2.],
                    orientation: [0., 0., 0., 1.],
                    scale: [1.; 3],
                    inverse_bind: inv(abs_child),
                },
            ],
        };
        assert!(s.bind_error() < 1e-6);
    }
}
