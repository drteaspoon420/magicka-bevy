//! `Magicka.ContentReaders.PhysicsEntityTemplateReader` ->
//! `Entities.PhysicsEntityTemplate.Read` (`Data/PhysicsEntities`,
//! `Data/AnimatedProps`).

use crate::character::{read_animation_sets, AnimationClipAction, Gib, ModelProperties};
use crate::common::*;
use crate::level::TriMesh;
use crate::xna::{self, Model, XnaObject};

#[derive(Debug, Clone, PartialEq)]
pub struct CollisionBox {
    /// Name string (read and discarded by the game).
    pub name: String,
    pub position: [f32; 3],
    pub sides: [f32; 3],
    pub orientation: [f32; 4],
}

/// Skinned/animated extension present when the trailing flag is true.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimatedPart {
    pub radius: f32,
    pub models: Vec<ModelProperties>,
    pub skeleton_model: Option<String>,
    pub attached_effects: Vec<(String, String)>,
    pub animation_sets: Vec<Vec<AnimationClipAction>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysicsEntityTemplate {
    pub movable: bool,
    pub pushable: bool,
    pub solid: bool,
    pub mass: f32,
    pub max_hitpoints: i32,
    pub can_have_status: bool,
    /// element, multiplier, modifier (no status flag, unlike characters).
    pub resistances: Vec<(i32, f32, f32)>,
    pub gibs: Vec<Gib>,
    pub hit_effect: String,
    /// `"Bank/cue"` or `"cue"` (bank defaults to Weapons).
    pub hit_sound: String,
    pub gib_trail_effect: String,
    pub model: Option<Model>,
    /// Optional triangle collision mesh: vertices + index triples.
    pub collision_mesh: Option<TriMesh>,
    /// Only the first box is used by the game; the rest are read and skipped.
    pub boxes: Vec<CollisionBox>,
    pub effects: Vec<(String, [f32; 16])>,
    pub conditions: ConditionCollection,
    pub id: String,
    /// `None` when the stream ends before the flag (the original catches the
    /// read failure and treats it as false).
    pub animated: Option<AnimatedPart>,
}

impl PhysicsEntityTemplate {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let movable = r.bool()?;
        let pushable = r.bool()?;
        let solid = r.bool()?;
        let mass = r.f32()?;
        let max_hitpoints = r.i32()?;
        let can_have_status = r.bool()?;
        let n = r.count()?;
        let resistances = (0..n)
            .map(|_| Ok((r.i32()?, r.f32()?, r.f32()?)))
            .collect::<Result<_>>()?;
        let n = r.count()?;
        let gibs = (0..n)
            .map(|_| {
                Ok(Gib {
                    model: r.external()?,
                    mass: r.f32()?,
                    scale: r.f32()?,
                })
            })
            .collect::<Result<_>>()?;
        let hit_effect = r.string()?;
        let hit_sound = r.string()?;
        let gib_trail_effect = r.string()?;
        let model = match xna::read_object(r)? {
            None => None,
            Some(XnaObject::Model(m)) => Some(*m),
            Some(o) => return invalid(format!("expected Model, got {o:?}")),
        };
        let collision_mesh = if r.bool()? {
            Some(TriMesh::read(r)?)
        } else {
            None
        };
        let n = r.i32()?.max(0);
        let boxes = (0..n)
            .map(|_| {
                Ok(CollisionBox {
                    name: r.string()?,
                    position: r.vec3()?,
                    sides: r.vec3()?,
                    orientation: xna::quaternion(r)?,
                })
            })
            .collect::<Result<_>>()?;
        let n = r.count()?;
        let effects = (0..n)
            .map(|_| Ok((r.string()?, xna::matrix(r)?)))
            .collect::<Result<_>>()?;
        if r.i32()? > 0 {
            return invalid(
                "PhysicsEntityTemplate lights (original throws NotImplementedException)",
            );
        }
        let conditions = ConditionCollection::read(r)?;
        let id = r.string()?;
        let animated = if r.remaining() > 0 && r.bool()? {
            let radius = r.f32()?;
            let n = r.i32()?.max(0);
            let models = (0..n)
                .map(|_| {
                    Ok(ModelProperties {
                        model: r.external()?,
                        scale: r.f32()?,
                        tint: r.vec3()?,
                    })
                })
                .collect::<Result<_>>()?;
            let skeleton_model = if r.bool()? { r.external()? } else { None };
            let n = r.i32()?.max(0);
            let attached_effects = (0..n)
                .map(|_| Ok((r.string()?, r.string()?)))
                .collect::<Result<_>>()?;
            let animation_sets = read_animation_sets(r)?;
            Some(AnimatedPart {
                radius,
                models,
                skeleton_model,
                attached_effects,
                animation_sets,
            })
        } else {
            None
        };
        Ok(PhysicsEntityTemplate {
            movable,
            pushable,
            solid,
            mass,
            max_hitpoints,
            can_have_status,
            resistances,
            gibs,
            hit_effect,
            hit_sound,
            gib_trail_effect,
            model,
            collision_mesh,
            boxes,
            effects,
            conditions,
            id,
            animated,
        })
    }
}
