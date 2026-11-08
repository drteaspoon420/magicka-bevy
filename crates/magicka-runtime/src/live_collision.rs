//! Animated pose-to-collision bridge. Matrix inversion is mathematical, not
//! verified bit-for-bit against the external XNA framework.
use crate::{
    animated_hierarchy::{AnimatedPartState, HierarchyError, PlayCommand},
    animated_pose::PoseError,
    level_collision::{AnimatedCollisionPart, SceneCollision},
    scene_loading::SceneLoadError,
};
use magicka_content::{level::LevelModel, skinned::Mat};
use magicka_sim::mesh_octree::MeshTransform;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InverseError {
    NonFinite,
    Singular,
}

/// General row-major 4x4 adjugate inverse, retaining scale and shear.
pub fn inverse(matrix: &Mat) -> Result<Mat, InverseError> {
    if matrix.iter().any(|x| !x.is_finite()) {
        return Err(InverseError::NonFinite);
    }
    let cofactor = |row: usize, col: usize| {
        let mut m = [0.0; 9];
        let mut n = 0;
        for r in 0..4 {
            for c in 0..4 {
                if r != row && c != col {
                    m[n] = matrix[r * 4 + c];
                    n += 1;
                }
            }
        }
        let determinant = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
            + m[2] * (m[3] * m[7] - m[4] * m[6]);
        if (row + col).is_multiple_of(2) {
            determinant
        } else {
            -determinant
        }
    };
    let determinant = matrix[0] * cofactor(0, 0)
        + matrix[1] * cofactor(0, 1)
        + matrix[2] * cofactor(0, 2)
        + matrix[3] * cofactor(0, 3);
    if determinant == 0.0 {
        return Err(InverseError::Singular);
    }
    let result = std::array::from_fn(|i| cofactor(i % 4, i / 4) / determinant);
    if result.iter().any(|x| !x.is_finite()) {
        return Err(InverseError::NonFinite);
    }
    Ok(result)
}

#[derive(Debug, PartialEq, Eq)]
pub enum LiveCollisionError {
    MissingAnimationId(Vec<i32>),
    Resume(crate::animated_pose::ResumeError),
    Hierarchy(HierarchyError),
    Scene(SceneLoadError),
    Pose(PoseError),
    Inverse {
        path: Vec<String>,
        error: InverseError,
    },
    MissingPart(Vec<String>),
}

fn collect_transforms(
    parts: &[AnimatedPartState],
    path: &mut Vec<String>,
    out: &mut BTreeMap<Vec<String>, MeshTransform>,
) -> Result<(), LiveCollisionError> {
    for part in parts {
        path.push(part.name.clone());
        if part.has_collision {
            let inv = inverse(&part.world).map_err(|error| LiveCollisionError::Inverse {
                path: path.clone(),
                error,
            })?;
            out.insert(
                path.clone(),
                MeshTransform {
                    forward: part.world,
                    inverse: inv,
                },
            );
        }
        collect_transforms(&part.children, path, out)?;
        path.pop();
    }
    Ok(())
}

fn apply_transforms(
    parts: &mut [AnimatedCollisionPart],
    path: &mut Vec<String>,
    states: &[AnimatedPartState],
    transforms: &BTreeMap<Vec<String>, MeshTransform>,
) {
    for (collision, state) in parts.iter_mut().zip(states) {
        path.push(state.name.clone());
        if let Some(mesh) = &mut collision.collision {
            mesh.transform = transforms[path];
        }
        apply_transforms(&mut collision.children, path, &state.children, transforms);
        path.pop();
    }
}

fn find_mut<'s>(
    parts: &'s mut [AnimatedPartState],
    path: &[String],
) -> Option<&'s mut AnimatedPartState> {
    let (name, rest) = path.split_first()?;
    let part = parts.iter_mut().find(|p| p.name == *name)?;
    if rest.is_empty() {
        Some(part)
    } else {
        find_mut(&mut part.children, rest)
    }
}

pub struct LiveCollisionScene {
    parts: Vec<AnimatedPartState>,
    collision: SceneCollision,
    carried_bodies: BTreeMap<(u32, usize), f32>,
}
impl LiveCollisionScene {
    /// AnimatedLevelPart lookup by id anywhere in the hierarchy
    /// (LevelModel.GetAnimatedLevelPart): the id path from the root.
    pub fn part_path(&self, id: i32) -> Option<Vec<i32>> {
        fn walk(parts: &[AnimatedPartState], id: i32, path: &mut Vec<i32>) -> bool {
            for p in parts {
                path.push(p.id);
                if p.id == id || walk(&p.children, id, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut path = Vec::new();
        walk(&self.parts, id, &mut path).then_some(path)
    }

    /// Execute after trigger delay scheduling. Resolve hashes independently at
    /// each hierarchy scope, matching GetAnimatedLevelPart/GetChild.
    /// GetAnimatedLevelPart(path[0]).GetChild(path[1])...: dictionary
    /// lookups that throw on a missing ID.
    pub fn part_by_ids_mut(
        &mut self,
        path: &[i32],
    ) -> Result<&mut AnimatedPartState, LiveCollisionError> {
        let mut siblings = &mut self.parts;
        let mut selected = None;
        for (index, id) in path.iter().enumerate() {
            let part = siblings
                .iter_mut()
                .find(|p| p.id == *id)
                .ok_or_else(|| LiveCollisionError::MissingAnimationId(path.to_vec()))?;
            if index + 1 == path.len() {
                selected = Some(part);
                break;
            }
            siblings = &mut part.children;
        }
        selected.ok_or_else(|| LiveCollisionError::MissingAnimationId(path.to_vec()))
    }
    pub fn execute_animation_action(
        &mut self,
        action: &crate::animation_action::AnimationAction,
    ) -> Result<(), LiveCollisionError> {
        let part = self.part_by_ids_mut(&action.path)?;
        match action.operation {
            crate::animation_action::AnimationOperation::Play(command) => {
                part.play(action.children, command)
            }
            crate::animation_action::AnimationOperation::Stop => part.stop(action.children),
            crate::animation_action::AnimationOperation::Resume {
                length,
                speed,
                looping,
            } => part
                .resume(action.children, length, speed, looping)
                .map_err(LiveCollisionError::Resume)?,
        }
        Ok(())
    }
    pub fn load(source: &LevelModel, parent: &Mat) -> Result<Self, LiveCollisionError> {
        let parts = AnimatedPartState::load(&source.animated_parts, parent)
            .map_err(LiveCollisionError::Hierarchy)?;
        let mut transforms = BTreeMap::new();
        collect_transforms(&parts, &mut vec![], &mut transforms)?;
        let collision = SceneCollision::from_content(source, |path| transforms.get(path).copied())
            .map_err(LiveCollisionError::Scene)?;
        Ok(Self {
            parts,
            collision,
            carried_bodies: BTreeMap::new(),
        })
    }
    pub fn collision(&self) -> &SceneCollision {
        &self.collision
    }
    pub fn collision_mut(&mut self) -> &mut SceneCollision {
        &mut self.collision
    }
    pub fn parts(&self) -> &[AnimatedPartState] {
        &self.parts
    }
    /// AnimatedLevelPart.AddStateTo for every part, pre-order.
    pub fn playback_states(&self) -> Vec<crate::animated_pose::PartPlayback> {
        fn walk(parts: &[AnimatedPartState], out: &mut Vec<crate::animated_pose::PartPlayback>) {
            for p in parts {
                out.push(p.playback);
                walk(&p.children, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.parts, &mut out);
        out
    }
    /// GameScene.RestoreSavedAnimations: apply pre-order playback states of
    /// the same hierarchy, then refresh poses and collision without advancing.
    pub fn restore_playback(
        &mut self,
        states: &[crate::animated_pose::PartPlayback],
        parent: &Mat,
    ) -> Result<(), LiveCollisionError> {
        fn walk(
            parts: &mut [AnimatedPartState],
            states: &mut std::slice::Iter<'_, crate::animated_pose::PartPlayback>,
        ) {
            for p in parts {
                if let Some(s) = states.next() {
                    p.playback = *s;
                }
                walk(&mut p.children, states);
            }
        }
        walk(&mut self.parts, &mut states.iter());
        self.update(0.0, parent)
    }
    pub fn play(
        &mut self,
        path: &[String],
        all_children: bool,
        command: PlayCommand,
    ) -> Result<(), LiveCollisionError> {
        find_mut(&mut self.parts, path)
            .ok_or_else(|| LiveCollisionError::MissingPart(path.to_vec()))?
            .play(all_children, command);
        Ok(())
    }
    pub fn stop(&mut self, path: &[String], all_children: bool) -> Result<(), LiveCollisionError> {
        find_mut(&mut self.parts, path)
            .ok_or_else(|| LiveCollisionError::MissingPart(path.to_vec()))?
            .stop(all_children);
        Ok(())
    }
    /// On inverse failure playback/poses have advanced but collision retains its
    /// last valid transforms. Caller must handle the error before querying again.
    pub fn update(&mut self, dt: f32, parent: &Mat) -> Result<(), LiveCollisionError> {
        for part in &mut self.parts {
            part.update(dt, parent).map_err(LiveCollisionError::Pose)?;
        }
        let mut transforms = BTreeMap::new();
        collect_transforms(&self.parts, &mut vec![], &mut transforms)?;
        apply_transforms(
            &mut self.collision.animated_parts,
            &mut vec![],
            &self.parts,
            &transforms,
        );
        // The live host advances a presented scene. Explicit channel-less
        // queries can use SceneCollision::update_water(..., false).
        self.collision.update_water(dt, true);
        Ok(())
    }
    /// AnimatedLevelPart.PostCollision / Update: retain supporting contacts
    /// for 0.25 seconds, then transform bodies by inverse(old) * new. Velocity
    /// is preserved; desired facing rotates with the surface.
    pub fn carry_characters(
        &mut self,
        dt: f32,
        physics: &mut magicka_sim::character_physics::PhysicsWorld,
    ) -> Result<(), LiveCollisionError> {
        fn walk<'a>(parts: &'a [AnimatedPartState], out: &mut Vec<&'a AnimatedPartState>) {
            for part in parts {
                out.push(part);
                walk(&part.children, out);
            }
        }
        let mut parts = vec![];
        walk(&self.parts, &mut parts);
        let key = crate::character_collision::ANIMATED_PART_KEY;
        for (body, primitive) in physics.supporting_primitives() {
            if let Some(owner) = primitive
                .checked_sub(key)
                .filter(|p| (*p as usize) < parts.len())
            {
                if parts[owner as usize].playback.playing {
                    self.carried_bodies.insert((owner, body), 0.25);
                }
            }
        }
        self.carried_bodies.retain(|&(owner, body), remaining| {
            *remaining -= dt;
            *remaining > 0.0
                && (owner as usize) < parts.len()
                && physics.bodies.get(body).is_some_and(|b| b.active)
        });
        for &(owner, body) in self.carried_bodies.keys() {
            let part = parts[owner as usize];
            let old_inverse =
                inverse(&part.previous_world).map_err(|error| LiveCollisionError::Inverse {
                    path: vec![part.name.clone()],
                    error,
                })?;
            let delta = magicka_content::skinned::mul(&old_inverse, &part.world);
            let vector = |v: [f32; 3]| -> [f32; 3] {
                std::array::from_fn(|i| v[0] * delta[i] + v[1] * delta[4 + i] + v[2] * delta[8 + i])
            };
            let b = &mut physics.bodies[body];
            let rotated = vector(b.position);
            b.set_position(std::array::from_fn(|i| rotated[i] + delta[12 + i]));
            b.orientation = magicka_sim::character_physics::Orientation {
                right: vector(b.orientation.right),
                up: vector(b.orientation.up),
                backward: vector(b.orientation.backward),
            };
            b.set_desired_direction(vector(b.desired_direction()));
        }
        Ok(())
    }

    /// Pre-order `inverse(previous world) * world` per part (the per-update
    /// delta AnimatedLevelPart.Update applies to its colliding entities);
    /// identity for a part that did not move. Indexes match
    /// `SceneHit::animated_owner`.
    pub fn part_deltas(&self) -> Result<Vec<Mat>, LiveCollisionError> {
        fn walk(parts: &[AnimatedPartState], out: &mut Vec<Mat>) -> Result<(), LiveCollisionError> {
            for part in parts {
                let old_inverse =
                    inverse(&part.previous_world).map_err(|error| LiveCollisionError::Inverse {
                        path: vec![part.name.clone()],
                        error,
                    })?;
                out.push(magicka_content::skinned::mul(&old_inverse, &part.world));
                walk(&part.children, out)?;
            }
            Ok(())
        }
        let mut out = Vec::new();
        walk(&self.parts, &mut out)?;
        Ok(out)
    }

    /// Scene phase including the native decal side effect of each part update.
    pub fn update_with_decals(
        &mut self,
        dt: f32,
        parent: &Mat,
        decals: &mut crate::decals::Decals,
    ) -> Result<(), LiveCollisionError> {
        self.update(dt, parent)?;
        fn walk(
            parts: &[AnimatedPartState],
            owner: &mut u32,
            path: &mut Vec<String>,
            decals: &mut crate::decals::Decals,
        ) -> Result<(), LiveCollisionError> {
            for part in parts {
                let id = *owner;
                *owner += 1;
                path.push(part.name.clone());
                if decals.has_owner(id) {
                    let old_inverse = inverse(&part.previous_world).map_err(|error| {
                        LiveCollisionError::Inverse {
                            path: path.clone(),
                            error,
                        }
                    })?;
                    let delta = magicka_content::skinned::mul(&old_inverse, &part.world);
                    decals.transform_owner(id, &delta);
                }
                walk(&part.children, owner, path, decals)?;
                path.pop();
            }
            Ok(())
        }
        walk(&self.parts, &mut 0, &mut vec![], decals)
    }
}
