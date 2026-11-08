//! PhysAnimation trigger action and DamageablePhysicsEntity's health-driven
//! level-part playback (research/script-schema.md).
//!
//! **PROVEN (decompiled PhysAnimation, DamageablePhysicsEntity.Update):**
//! Execute (and QuickExecute) appends a PhysAnimationControl to the entity
//! named by `Id`; `Name` is a `/`-separated AnimatedLevelPart path. Whenever
//! hit points changed since the last update, each control plays its part
//! from the previous to the current damage fraction (1 - hp/max) mapped onto
//! [Start, End], reversing speed when that interval runs backwards. The
//! previous fraction starts at 1.
use crate::{
    animated_hierarchy::PlayCommand,
    scene_loading::custom_hash,
    script_attributes::{boolean, number, ScriptReadError},
};

#[derive(Clone, Debug, PartialEq)]
pub struct PhysAnimationAction {
    pub delay: f32,
    /// Unique-ID hash of the DamageablePhysicsEntity (`Id`).
    pub entity: i32,
    /// Hashed AnimatedLevelPart path (`Name`); empty when unset.
    pub path: Vec<i32>,
    pub control: PhysAnimationControl,
}

/// PhysAnimationControl without its part reference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysAnimationControl {
    pub start: f32,
    pub end: f32,
    pub speed: f32,
    pub children: bool,
}

impl PhysAnimationAction {
    pub fn read<'a>(
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        let mut action = Self {
            delay: 0.0,
            entity: 0,
            path: Vec::new(),
            control: PhysAnimationControl {
                start: 0.0,
                end: 0.0,
                speed: 1.0,
                children: true,
            },
        };
        for (key, value) in attrs {
            // String properties receive the lowercased attribute value.
            let lower = value.to_lowercase();
            match key.to_ascii_lowercase().as_str() {
                "delay" => action.delay = number(value, key)?,
                "id" => action.entity = custom_hash(&lower),
                "name" => action.path = lower.split('/').map(custom_hash).collect(),
                "speed" => action.control.speed = number(value, key)?,
                "start" => action.control.start = number(value, key)?,
                "end" => action.control.end = number(value, key)?,
                "children" => action.control.children = boolean(value, key)?,
                _ => {}
            }
        }
        Ok(action)
    }
}

/// The animation half of a DamageablePhysicsEntity.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhysAnimations {
    pub controls: Vec<(Vec<i32>, PhysAnimationControl)>,
    old_health: f32,
    old_fraction: f32,
}

impl PhysAnimations {
    /// Initialize: mOldHitPoints = MaxHitpoints, mOldPercentage = 1.
    pub fn new(maximum: f32) -> Self {
        Self {
            controls: Vec::new(),
            old_health: maximum,
            old_fraction: 1.0,
        }
    }

    pub fn add(&mut self, path: Vec<i32>, control: PhysAnimationControl) {
        self.controls.push((path, control));
    }

    /// The Play calls of one DamageablePhysicsEntity.Update, in control
    /// order: (path, all children, command). The change test compares
    /// against CLR float.Epsilon (the smallest subnormal).
    pub fn update(&mut self, health: f32, maximum: f32) -> Vec<(Vec<i32>, bool, PlayCommand)> {
        if self.controls.is_empty() || (self.old_health - health).abs() <= f32::from_bits(1) {
            return Vec::new();
        }
        let fraction = 1.0 - health / maximum;
        let plays = self
            .controls
            .iter()
            .map(|(path, c)| {
                let from = c.start + (c.end - c.start) * self.old_fraction;
                let to = c.start + (c.end - c.start) * fraction;
                let (start, end, speed) = if from > to {
                    (to, from, -c.speed)
                } else {
                    (from, to, c.speed)
                };
                (
                    path.clone(),
                    c.children,
                    PlayCommand {
                        start,
                        end,
                        speed,
                        looping: false,
                        resume: false,
                    },
                )
            })
            .collect();
        self.old_health = health;
        self.old_fraction = fraction;
        plays
    }
}
