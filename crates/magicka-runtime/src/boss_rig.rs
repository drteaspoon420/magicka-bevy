//! The skinned-model plumbing shared by named boss ports: an
//! XNAnimation AnimationController driven by clip names, the normalized
//! clip time the state machines test, and joint orientations read through
//! the bosses' `Invert(InverseBindPose · RotationY(π))` bind poses.
use crate::animation_controller::AnimationController;
use crate::boss_fafnir::ModelPose;
use crate::character_pose::CharacterSkeleton;
use crate::xna_math::{self as m, V3};
use magicka_content::skinned::{self, Mat, Resolved};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct BossRig {
    pub skeleton: CharacterSkeleton,
    pub controller: AnimationController,
    durations: HashMap<String, f32>,
    /// SkinnedBoneTransforms of the last update.
    pub skinned: Vec<Mat>,
    /// The local poses and parent drawn by the viewer.
    pub pose: ModelPose,
}

fn rotation_y_pi() -> Mat {
    let mut r = skinned::IDENTITY;
    r[0] = -1.0;
    r[10] = -1.0;
    r
}

impl BossRig {
    /// The animation model's skeleton and clips; every `clips` name must
    /// exist.
    pub fn new(animation: &Resolved, clips: &[&str]) -> Result<Self, String> {
        let skeleton = CharacterSkeleton::new(animation.skeleton.clone(), animation.clips.clone());
        let durations = skeleton.clip_durations();
        for name in clips {
            if !durations.contains_key(*name) {
                return Err(format!("animation lacks clip {name}"));
            }
        }
        Ok(Self {
            skeleton,
            controller: AnimationController::default(),
            durations,
            skinned: Vec::new(),
            pose: ModelPose::default(),
        })
    }

    /// A bone's index and `Invert(InverseBindPoseTransform · RotationY(π))`.
    pub fn bind(&self, name: &str) -> Result<(usize, Mat), String> {
        let index = self
            .skeleton
            .bone(name)
            .ok_or_else(|| format!("skeleton lacks {name}"))?;
        let inv = skinned::mul(
            &self.skeleton.skeleton.bones[index].inverse_bind,
            &rotation_y_pi(),
        );
        let bind =
            crate::live_collision::inverse(&inv).map_err(|e| format!("bind {name}: {e:?}"))?;
        Ok((index, bind))
    }

    fn duration(&self, name: &str) -> f32 {
        self.durations.get(name).copied().unwrap_or(0.0)
    }
    /// StartClip.
    pub fn start(&mut self, name: &str, looping: bool) {
        let d = self.duration(name);
        self.controller.start_named_clip(name, d, looping);
    }
    /// CrossFade.
    pub fn crossfade(&mut self, name: &str, fade: f32, looping: bool) {
        let d = self.duration(name);
        self.controller.crossfade_named_clip(name, d, fade, looping);
    }
    /// Stop (time is held).
    pub fn stop(&mut self) {
        self.controller.stop();
    }
    pub fn playing(&self, name: &str) -> bool {
        self.controller.clip_name() == Some(name)
    }
    /// Time / AnimationClip.Duration.
    pub fn normalized(&self) -> f32 {
        let d = self.controller.clip_duration();
        if d > 0.0 {
            self.controller.time / d
        } else {
            0.0
        }
    }
    pub fn finished(&self) -> bool {
        self.controller.has_finished()
    }
    pub fn fading(&self) -> bool {
        self.controller.crossfade_enabled()
    }
    /// Clip duration of the playing clip.
    pub fn clip_duration(&self) -> f32 {
        self.controller.clip_duration()
    }

    /// AnimationController.Update(elapsed, parent, true).
    pub fn update(&mut self, elapsed: f32, parent: &Mat) {
        self.controller.signals.clear();
        self.controller.update(elapsed);
        let local = self.skeleton.local_poses(self.controller.pose_sample());
        self.skinned = self.skeleton.skinned_from_local(&local, parent);
        self.pose = ModelPose {
            parent: *parent,
            local,
        };
    }

    /// `bind · SkinnedBoneTransforms[index]`: the joint's world orientation.
    pub fn joint(&self, index: usize, bind: &Mat) -> Mat {
        self.skinned
            .get(index)
            .map_or(*bind, |s| skinned::mul(bind, s))
    }
    /// `Vector3.Transform(bind.Translation, SkinnedBoneTransforms[index])`.
    pub fn point(&self, index: usize, bind: &Mat) -> V3 {
        self.skinned.get(index).map_or(m::translation(bind), |s| {
            m::transform_point(m::translation(bind), s)
        })
    }
}
