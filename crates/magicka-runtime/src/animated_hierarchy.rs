//! Playback/pose ownership for decoded animated level parts. Rendering, physics
//! side effects and network delivery remain responsibilities of the world host.
use crate::{
    animated_pose::{initial_world_pose, updated_world_pose, PartPlayback, PoseError, ResumeError},
    scene_loading::custom_hash,
};
use magicka_content::{
    level::{AnimatedLevelPart, Keyframe},
    skinned::Mat,
};
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, Copy, Debug)]
pub struct PlayCommand {
    pub start: f32,
    pub end: f32,
    pub speed: f32,
    pub looping: bool,
    pub resume: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HierarchyError {
    UnsupportedName(String),
    DuplicateId(i32),
    Pose { name: String, error: PoseError },
}

/// Owns the playback-relevant content (keyframes, duration, collision flag)
/// so live scenes can outlive the decoded LevelModel borrow.
#[derive(Debug)]
pub struct AnimatedPartState {
    pub animation: Arc<[Keyframe]>,
    pub animation_duration: f32,
    pub has_collision: bool,
    pub name: String,
    pub id: i32,
    pub playback: PartPlayback,
    pub previous_world: Mat,
    pub world: Mat,
    pub children: Vec<AnimatedPartState>,
}

impl AnimatedPartState {
    /// Load siblings with independent child ID scopes, preserving decoded order.
    pub fn load(parts: &[AnimatedLevelPart], parent: &Mat) -> Result<Vec<Self>, HierarchyError> {
        let mut ids = HashSet::new();
        let mut states = Vec::with_capacity(parts.len());
        for source in parts {
            if !source.name.is_ascii() {
                return Err(HierarchyError::UnsupportedName(source.name.clone()));
            }
            let name = source.name.to_ascii_lowercase();
            let id = custom_hash(&name);
            if !ids.insert(id) {
                return Err(HierarchyError::DuplicateId(id));
            }
            let playback = PartPlayback::new(source.animation_duration);
            let world =
                initial_world_pose(&source.animation, playback.time, parent).map_err(|error| {
                    HierarchyError::Pose {
                        name: name.clone(),
                        error,
                    }
                })?;
            let children = Self::load(&source.children, &world)?;
            states.push(Self {
                animation: source.animation.clone().into(),
                animation_duration: source.animation_duration,
                has_collision: source.collision.is_some(),
                name,
                id,
                playback,
                previous_world: world,
                world,
                children,
            });
        }
        Ok(states)
    }

    pub fn play(&mut self, all_children: bool, command: PlayCommand) {
        if all_children {
            for child in &mut self.children {
                child.play(
                    true,
                    PlayCommand {
                        resume: false,
                        ..command
                    },
                );
            }
        }
        self.playback.play(
            self.animation_duration,
            command.start,
            command.end,
            command.speed,
            command.looping,
            command.resume,
        );
    }

    pub fn stop(&mut self, all_children: bool) {
        if all_children {
            for child in &mut self.children {
                child.stop(true);
            }
        }
        self.playback.stop();
    }

    /// Child-first; a failed child prevents subsequent siblings/parent mutation,
    /// matching exception propagation in the original recursion.
    pub fn resume(
        &mut self,
        all_children: bool,
        length: f32,
        speed: f32,
        looping: Option<bool>,
    ) -> Result<(), ResumeError> {
        if all_children {
            for child in &mut self.children {
                child.resume(true, length, speed, looping)?;
            }
        }
        self.playback
            .resume(self.animation_duration, length, speed, looping)
    }

    /// Parent time/pose first, then children using the updated world transform.
    /// Stopped children still follow their parent's motion.
    pub fn update(&mut self, dt: f32, parent: &Mat) -> Result<(), PoseError> {
        self.playback.advance(dt);
        let world = updated_world_pose(&self.animation, self.playback.time, parent)?;
        self.previous_world = self.world;
        self.world = world;
        for child in &mut self.children {
            child.update(dt, &world)?;
        }
        Ok(())
    }

    /// Resolve the lowercase hierarchy paths used by the collision loader.
    pub fn find<'s>(siblings: &'s [Self], path: &[String]) -> Option<&'s Self> {
        let (name, rest) = path.split_first()?;
        let part = siblings.iter().find(|part| part.name == *name)?;
        if rest.is_empty() {
            Some(part)
        } else {
            Self::find(&part.children, rest)
        }
    }
}
