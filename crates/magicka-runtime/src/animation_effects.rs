//! AnimationActions.PlayEffect bone orientation and retained effect lifecycle.
//! Evidence: research/animation-play-effect.md.
use crate::scene_loading::custom_hash;
use crate::{
    avatar::AvatarEvent,
    particles::{EffectRef, EffectSystem},
};
use magicka_content::skinned::{self, Mat, Skeleton};
use std::collections::BTreeMap;

/// Native ctor: invert(inverseBind * RotationY(pi)), then GetBoneOrientation.
pub fn bone_orientation(skeleton: &Skeleton, bones: &[Mat], name: &str) -> Option<Mat> {
    let index = skeleton
        .bones
        .iter()
        .position(|b| b.name.eq_ignore_ascii_case(name))?;
    let rotation = skinned::pose_matrix([0.0; 3], [0.0, 1.0, 0.0, 0.0], [1.0; 3]);
    let bind = crate::live_collision::inverse(&skinned::mul(
        &skeleton.bones[index].inverse_bind,
        &rotation,
    ))
    .ok()?;
    Some(skinned::mul(&bind, bones.get(index)?))
}

#[derive(Default)]
pub struct AnimationEffects {
    refs: BTreeMap<(u16, usize), EffectRef>,
}

impl AnimationEffects {
    pub fn event(
        &mut self,
        owner: u16,
        event: &AvatarEvent,
        orientation: Option<Mat>,
        effects: &mut EffectSystem,
    ) {
        match event {
            AvatarEvent::AnimationEffect {
                index,
                effect,
                retained,
                first,
                ..
            } => {
                let Some(m) = orientation else { return };
                let position = [m[12], m[13], m[14]];
                let forward = [-m[8], -m[9], -m[10]];
                let length = forward.iter().map(|v| v * v).sum::<f32>().sqrt();
                if length <= 0.0 {
                    return;
                }
                let direction = forward.map(|v| v / length);
                let key = (owner, *index);
                if let Some(reference) = self.refs.get_mut(&key) {
                    // A failed update removes memory and returns; native does not restart.
                    if !effects.update_position_direction(reference, position, direction) {
                        self.refs.remove(&key);
                    }
                } else if *first {
                    if let Some(reference) =
                        effects.start_at(custom_hash(effect), position, direction)
                    {
                        if *retained {
                            self.refs.insert(key, reference);
                        }
                    }
                }
            }
            AvatarEvent::AnimationEffectStopped(index) => {
                if let Some(mut reference) = self.refs.remove(&(owner, *index)) {
                    effects.stop(&mut reference);
                }
            }
            _ => {}
        }
    }
    pub fn retain_owners(&mut self, owners: &[u16], effects: &mut EffectSystem) {
        self.refs.retain(|(owner, _), reference| {
            if owners.contains(owner) {
                true
            } else {
                effects.stop(reference);
                false
            }
        });
    }
}
