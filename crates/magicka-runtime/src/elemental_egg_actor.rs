//! Dedicated animated ElementalEgg host. Native geometry comes from the Fire
//! template, skeleton from the mesh, and spawn clip from the animation asset.
//! Scene/cache/spell/network integration remains in research/summon-elemental.md.
use crate::{
    animation_controller::AnimationController,
    character_pose::{CharacterSkeleton, Pose},
};
use magicka_content::{
    character::CharacterTemplate,
    skinned::{self, Mat, Resolved},
};
use magicka_sim::{
    elemental_egg::{self as egg, ElementalEgg},
    spells::Elements,
    spray_pulse::Segment,
};
use std::sync::Arc;

/// EntityManager constructor passes128 to ElementalEgg.InitializeCache.
pub const NATIVE_CACHE_SIZE: usize = 128;

pub struct CachedEgg {
    pub handle: u16,
    pub actor: ElementalEggActor,
    pub frame: Option<EggFrame>,
}

/// Stable actor slots implement native GetInstance's remove-front/add-back
/// cycle without moving actors or treating active eggs as unavailable.
pub struct ElementalEggCache {
    pub slots: [CachedEgg; NATIVE_CACHE_SIZE],
    next: usize,
}

impl ElementalEggCache {
    /// Recognized entity even when its payload is rejected. Callers must not
    /// fall through to an unrelated entity-type damage route on rejection.
    pub fn deliver(
        &mut self,
        handle: u16,
        damages: &[magicka_sim::damage::Damage],
    ) -> Option<bool> {
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.handle == handle && slot.actor.registered)?;
        let mut hit = false;
        for &damage in damages {
            hit |= slot.actor.state.internal_damage(damage);
        }
        Some(hit)
    }

    pub fn new(
        mesh: Resolved,
        animation: Resolved,
        geometry: EggGeometry,
        handles: [u16; NATIVE_CACHE_SIZE],
    ) -> Result<Self, String> {
        let prototype = ElementalEggActor::new(mesh, animation, geometry)?;
        let slots = std::array::from_fn(|index| CachedEgg {
            handle: handles[index],
            actor: prototype.clone(),
            frame: None,
        });
        Ok(Self { slots, next: 0 })
    }

    /// Checkout mutates only queue order. Initialize is a separate native
    /// operation; forced reuse does not first hatch/deinitialize the old egg.
    pub fn acquire(&mut self) -> &mut CachedEgg {
        let index = self.next;
        self.next = (self.next + 1) % self.slots.len();
        &mut self.slots[index]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EggGeometry {
    pub radius: f32,
    pub length: f32,
    pub scale: f32,
}

impl EggGeometry {
    pub fn from_fire_template(template: &CharacterTemplate) -> Result<Self, String> {
        let model = template
            .models
            .first()
            .ok_or("Fire elemental has no model properties")?;
        Ok(Self {
            radius: template.radius,
            length: template.length.max(0.01),
            scale: model.scale,
        })
    }
}

#[derive(Clone, Debug)]
pub struct EggFrame {
    pub model_parent: Mat,
    pub bones: Vec<Mat>,
    pub local_poses: Vec<Pose>,
}

#[derive(Clone, Debug)]
pub struct EggHatch {
    pub element: Elements,
    pub position: [f32; 3],
    pub orientation: Mat,
    /// Host links only Character summoners, as native Spawn does.
    pub summoner: Option<u16>,
}

#[derive(Clone)]
pub struct ElementalEggActor {
    pub state: ElementalEgg,
    pub geometry: EggGeometry,
    pub mesh: Arc<Resolved>,
    pub skeleton: Arc<CharacterSkeleton>,
    pub controller: AnimationController,
    pub position: [f32; 3],
    pub orientation: Mat,
    pub velocity: [f32; 3],
    pub registered: bool,
    clip_duration: f32,
}

impl ElementalEggActor {
    /// No fallback skeleton, clip or duration: missing native assets fail.
    pub fn new(mesh: Resolved, animation: Resolved, geometry: EggGeometry) -> Result<Self, String> {
        let clip = animation
            .clips
            .iter()
            .find(|clip| clip.name == egg::CLIP)
            .ok_or("Elemental animation model is missing spawn clip")?
            .clone();
        if mesh.skeleton.bones.is_empty() {
            return Err("Elemental mesh has no skeleton".into());
        }
        let clip_duration = clip.duration;
        let skeleton = CharacterSkeleton::new(mesh.skeleton.clone(), vec![clip]);
        Ok(Self {
            state: ElementalEgg::default(),
            geometry,
            mesh: Arc::new(mesh),
            skeleton: Arc::new(skeleton),
            controller: AnimationController::default(),
            position: [0.0; 3],
            orientation: skinned::IDENTITY,
            velocity: [0.0; 3],
            registered: false,
            clip_duration,
        })
    }

    pub fn ground_probe(requested: [f32; 3]) -> Segment {
        Segment {
            origin: [requested[0], requested[1] + 1.0, requested[2]],
            delta: [0.0, -10.0, 0.0],
        }
    }

    /// Native Initialize uses a zero-initialized matrix with only basis rows
    /// assigned. Keep scaled/vertical direction, without quaternion conversion.
    pub fn initialize(
        &mut self,
        requested: [f32; 3],
        direction: [f32; 3],
        scene_hit: Option<[f32; 3]>,
    ) {
        self.position = egg::egg_position(requested, scene_hit, self.geometry.radius);
        let mut orientation = [0.0; 16];
        orientation[0..3].copy_from_slice(&[-direction[2], 0.0, direction[0]]);
        orientation[4..7].copy_from_slice(&[0.0, 1.0, 0.0]);
        orientation[8..11].copy_from_slice(&direction.map(|v| -v));
        self.orientation = orientation;
        // JigLibX Body.MoveTo resets velocity/angular velocity and old state.
        self.velocity = [0.0; 3];
        self.state.initialize();
        self.controller
            .start_named_clip(egg::CLIP, self.clip_duration, true);
        self.registered = true;
    }

    /// Animation/render sampling precedes damage/proximity hatch selection.
    /// The returned frame remains valid for the final egg draw on removal.
    pub fn update(
        &mut self,
        dt: f32,
        avatar_in_proximity: bool,
        random_index: impl FnOnce(usize) -> usize,
    ) -> Option<EggFrame> {
        if !self.registered {
            return None;
        }
        let mut parent = self.orientation.map(|v| v * self.geometry.scale);
        parent[12..15].copy_from_slice(&self.position);
        self.controller.update(dt);
        let frame = EggFrame {
            model_parent: parent,
            bones: self.skeleton.skinned_for(&self.controller, &parent),
            local_poses: self.skeleton.local_poses(self.controller.pose_sample()),
        };
        self.state.update(
            dt,
            self.velocity.iter().map(|v| v * v).sum(),
            avatar_in_proximity,
            random_index,
        );
        Some(frame)
    }

    pub fn segment_intersect(&self, segment: Segment, segment_radius: f32) -> Option<[f32; 3]> {
        self.registered
            .then(|| {
                egg::segment_intersect(
                    self.position,
                    self.geometry.length,
                    self.geometry.radius,
                    segment,
                    segment_radius,
                )
            })
            .flatten()
    }

    /// Deinitialize unregisters even for client/Shield removal; only authority
    /// gets an accepted-element hatch request, with the retained body pose.
    pub fn deinitialize(&mut self, client: bool) -> Option<EggHatch> {
        if !self.registered {
            return None;
        }
        self.registered = false;
        self.state.hatch_element(client).map(|element| EggHatch {
            element,
            position: self.position,
            orientation: self.orientation,
            summoner: self.state.summoner,
        })
    }
}
