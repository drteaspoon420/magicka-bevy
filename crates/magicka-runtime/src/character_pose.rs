//! Character skeletal pose and equipment attach points.
//!
//! XNAnimation AnimationController.UpdateChannelPoses (looping key wrap,
//! crossfade toward the target clip's first key with the controller's
//! interpolation amount) and UpdateAbsoluteBoneTransforms; Attachment's bind
//! pose; Item's attach0 point; Character.GetOrientation / GetItemAttachOrientation
//! and the staff orb (CastSource). See research/character-casting.md.
use crate::animation_controller::{AnimationController, PoseSample};
use magicka_content::{
    assets::{AnimationClip, Keyframe},
    skinned::{self, Mat, Skeleton},
};
use std::collections::HashMap;

/// TRS pose (XNAnimation Pose).
pub type Pose = ([f32; 3], [f32; 4], [f32; 3]);

#[derive(Clone, Debug)]
pub struct CharacterSkeleton {
    pub skeleton: Skeleton,
    clips: HashMap<String, AnimationClip>,
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}
/// Pose.Interpolate with Linear modes (amount clamped to [0, 1]).
fn interpolate(a: Pose, b: Pose, amount: f32) -> Pose {
    let t = amount.clamp(0.0, 1.0);
    (
        lerp3(a.0, b.0, t),
        skinned::xna_quat_lerp(a.1, b.1, t),
        lerp3(a.2, b.2, t),
    )
}
fn key_pose(k: &Keyframe) -> Pose {
    (k.translation, k.orientation, k.scale)
}

/// AnimationController.InterpolateChannelPose.
fn channel_pose(keys: &[Keyframe], duration: f32, time: f32, index: usize, next: usize) -> Pose {
    if index == next {
        return key_pose(&keys[index]);
    }
    let (a, b) = (&keys[index], &keys[next]);
    let span = if next != 0 {
        b.time - a.time
    } else {
        duration - a.time
    };
    if span > 0.0 {
        interpolate(key_pose(a), key_pose(b), (time - a.time) / span)
    } else {
        key_pose(a)
    }
}

impl CharacterSkeleton {
    pub fn new(skeleton: Skeleton, clips: Vec<AnimationClip>) -> Self {
        Self {
            skeleton,
            clips: clips.into_iter().map(|c| (c.name.clone(), c)).collect(),
        }
    }
    /// Case-insensitive bone lookup (Attachment / joint names).
    pub fn bone(&self, name: &str) -> Option<usize> {
        self.skeleton
            .bones
            .iter()
            .position(|b| b.name.eq_ignore_ascii_case(name))
    }
    pub fn clip_durations(&self) -> HashMap<String, f32> {
        self.clips
            .iter()
            .map(|(n, c)| (n.clone(), c.duration))
            .collect()
    }

    /// UpdateChannelPoses then UpdateAbsoluteBoneTransforms: skinned bone
    /// transforms `invBind · abs` with `parent` as the root's parent.
    pub fn skinned(&self, sample: Option<PoseSample<'_>>, parent: &Mat) -> Vec<Mat> {
        self.skinned_from_local(&self.local_poses(sample), parent)
    }

    /// UpdateAbsoluteBoneTransforms over already sampled (possibly edited)
    /// local poses: skinned transforms `invBind · abs`.
    pub fn skinned_from_local(&self, local: &[Pose], parent: &Mat) -> Vec<Mat> {
        let mut abs: Vec<Mat> = Vec::with_capacity(self.skeleton.bones.len());
        for (bone, pose) in self.skeleton.bones.iter().zip(local.iter().copied()) {
            let local = skinned::pose_matrix(pose.0, pose.1, pose.2);
            let m = match bone.parent {
                Some(p) => skinned::mul(&local, &abs[p]),
                None => skinned::mul(&local, parent),
            };
            abs.push(m);
        }
        self.skeleton
            .bones
            .iter()
            .zip(&abs)
            .map(|(b, a)| skinned::mul(&b.inverse_bind, a))
            .collect()
    }

    /// The native channel poses, shared by body rendering and attachments.
    pub fn local_poses(&self, sample: Option<PoseSample<'_>>) -> Vec<Pose> {
        let clip = sample.and_then(|s| self.clips.get(s.clip).map(|c| (s, c)));
        let fade = sample
            .and_then(|s| s.crossfade)
            .and_then(|name| self.clips.get(name));
        self.skeleton
            .bones
            .iter()
            .map(|bone| {
                let bind = (bone.translation, bone.orientation, bone.scale);
                let mut pose = match clip.and_then(|(s, c)| {
                    c.channels
                        .iter()
                        .find(|(n, _)| *n == bone.name)
                        .map(|(_, k)| (s, k))
                }) {
                    Some((s, keys)) if !keys.is_empty() => {
                        let i = skinned::keyframe_index(keys, s.time);
                        let mut j = (i + 1) % keys.len();
                        if j == 0 && !s.looping {
                            j = i;
                        }
                        channel_pose(keys, s.duration, s.time, i, j)
                    }
                    _ => bind,
                };
                if let (Some(fade), Some(s)) = (fade, sample) {
                    let target = fade
                        .channels
                        .iter()
                        .find(|(n, _)| *n == bone.name)
                        .and_then(|(_, k)| k.first())
                        .map_or(bind, key_pose);
                    pose = interpolate(pose, target, s.crossfade_amount);
                }
                pose
            })
            .collect()
    }

    pub fn skinned_for(&self, controller: &AnimationController, parent: &Mat) -> Vec<Mat> {
        self.skinned(controller.pose_sample(), parent)
    }
}

fn rotation_x(a: f32) -> Mat {
    let (s, c) = a.sin_cos();
    [
        1.0, 0.0, 0.0, 0.0, 0.0, c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}
fn rotation_y(a: f32) -> Mat {
    let (s, c) = a.sin_cos();
    [
        c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

/// An equipment slot attached to a skeleton bone.
#[derive(Clone, Debug, PartialEq)]
pub struct EquipmentAttachment {
    pub slot: i32,
    pub bone: usize,
    /// Attachment.BindBose after TransformBindPose(inverse static transform).
    pub bind: Mat,
    /// Item.mAttach0 (identity without an attach0 bone).
    pub attach0: Mat,
    pub attach1: Mat,
    /// Item.WeaponClass (Default when the slot is empty).
    pub weapon_class: u8,
    /// `mEquipment[slot].Item != null`.
    pub has_item: bool,
    /// Item.mEffect0Pos / mEffect1Pos (model bones effect0/effect1 walked to
    /// the root; zero without the bone).
    pub effect0: [f32; 3],
    pub effect1: [f32; 3],
}

impl EquipmentAttachment {
    /// Attachment(ContentReader): `Rx(r.x)·Ry(r.y+π)·Rx(r.z)·invert(inverseBind)`
    /// (the third rotation is about X with the Z value, as in the original),
    /// then TransformBindPose(invert(static transform)).
    pub fn new(
        slot: i32,
        bone: usize,
        inverse_bind: &Mat,
        rotation: [f32; 3],
        inverse_static: &Mat,
        attach0: Mat,
        weapon_class: Option<u8>,
    ) -> Option<Self> {
        let bind_pose = crate::live_collision::inverse(inverse_bind).ok()?;
        let r = skinned::mul(
            &skinned::mul(
                &rotation_x(rotation[0]),
                &rotation_y(rotation[1] + std::f32::consts::PI),
            ),
            &rotation_x(rotation[2]),
        );
        let bind = skinned::mul(inverse_static, &skinned::mul(&r, &bind_pose));
        Some(Self {
            slot,
            bone,
            bind,
            attach0,
            attach1: magicka_content::skinned::IDENTITY,
            weapon_class: weapon_class.unwrap_or(0),
            has_item: weapon_class.is_some(),
            effect0: [0.0; 3],
            effect1: [0.0; 3],
        })
    }
    /// Character.GetItemAttachOrientation: `bind · skinned[bone]`.
    pub fn item_transform(&self, skinned: &[Mat]) -> Mat {
        skinned::mul(&self.bind, &skinned[self.bone])
    }
    /// Item.AttachAbsoluteTransform: `attach0 · item transform`.
    pub fn attach_absolute(&self, skinned: &[Mat]) -> Mat {
        skinned::mul(&self.attach0, &self.item_transform(skinned))
    }
}

/// Item model attach0: ModelBone transforms combined while walking to the
/// root as `parent · accumulated` (the original's order, not child·parent).
pub fn item_attach0(bones: &[magicka_content::xna::Bone]) -> Mat {
    item_bone(bones, "attach0").unwrap_or(magicka_content::skinned::IDENTITY)
}

/// Item.Read's bone walk for `name` (case-insensitive), as for attach0.
pub fn item_bone(bones: &[magicka_content::xna::Bone], name: &str) -> Option<Mat> {
    let mut index = bones.iter().position(|b| {
        b.name
            .as_deref()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    })?;
    let mut transform = bones[index].transform;
    while let Some(parent) = bones[index].parent {
        transform = skinned::mul(&bones[parent].transform, &transform);
        index = parent;
    }
    Some(transform)
}

/// Item model points: attach0 and the effect0/effect1 translations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemPoints {
    pub attach0: Mat,
    pub attach1: Mat,
    pub effect0: [f32; 3],
    pub effect1: [f32; 3],
}

impl ItemPoints {
    pub const NONE: Self = Self {
        attach0: magicka_content::skinned::IDENTITY,
        attach1: magicka_content::skinned::IDENTITY,
        effect0: [0.0; 3],
        effect1: [0.0; 3],
    };

    pub fn from_bones(bones: &[magicka_content::xna::Bone]) -> Self {
        let at = |name| item_bone(bones, name).map_or([0.0; 3], |m| [m[12], m[13], m[14]]);
        Self {
            attach0: item_attach0(bones),
            attach1: item_bone(bones, "attach1").unwrap_or(magicka_content::skinned::IDENTITY),
            effect0: at("effect0"),
            effect1: at("effect1"),
        }
    }
}

/// Character.GetOrientation: body orientation, translation at the body
/// position lowered by HeightOffset, premultiplied by the static transform.
pub fn character_orientation(
    static_transform: &Mat,
    orientation: &magicka_sim::character_physics::Orientation,
    position: [f32; 3],
    height_offset: f32,
) -> Mat {
    let o = orientation;
    let body = [
        o.right[0],
        o.right[1],
        o.right[2],
        0.0,
        o.up[0],
        o.up[1],
        o.up[2],
        0.0,
        o.backward[0],
        o.backward[1],
        o.backward[2],
        0.0,
        position[0],
        position[1] + height_offset,
        position[2],
        1.0,
    ];
    skinned::mul(static_transform, &body)
}
