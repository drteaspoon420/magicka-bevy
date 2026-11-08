//! Asset-backed MagickDeath animation/attachment host. The scene owns casts,
//! target queries, body collision, damage delivery, turning and presentation.
use crate::{animation_controller::AnimationController, character_pose::CharacterSkeleton};
use magicka_content::skinned::{self, Mat, Resolved};
use magicka_sim::{
    spray_pulse::Segment,
    summon_death::{self as death, AnimationClock, MagickDeath, Phase, Target},
};
use std::collections::BTreeMap;

pub const SCYTHE_MODEL: &str = "Models/Bosses/Death/death_Scythe";
pub const CAPSULE_RADIUS: f32 = 0.6;
pub const CAPSULE_LENGTH: f32 = 1.2;
pub const BODY_MASS: f32 = 500.0;

/// Matrix-to-quaternion conversion in row-vector convention. The basis may
/// be non-orthonormal: native Turn keeps scaled/vertical direction components.
pub(crate) fn rotation_quaternion(matrix: &Mat) -> [f32; 4] {
    let trace = matrix[0] + matrix[5] + matrix[10];
    if trace > 0.0 {
        let root = (trace + 1.0).sqrt();
        let inverse = 0.5 / root;
        return [
            (matrix[6] - matrix[9]) * inverse,
            (matrix[8] - matrix[2]) * inverse,
            (matrix[1] - matrix[4]) * inverse,
            root * 0.5,
        ];
    }
    let i = if matrix[0] >= matrix[5] && matrix[0] >= matrix[10] {
        0
    } else if matrix[5] > matrix[10] {
        1
    } else {
        2
    };
    let j = (i + 1) % 3;
    let k = (j + 1) % 3;
    let root = (1.0 + matrix[i * 5] - matrix[j * 5] - matrix[k * 5]).sqrt();
    let inverse = 0.5 / root;
    let mut q = [0.0; 4];
    q[i] = root * 0.5;
    q[j] = (matrix[i * 4 + j] + matrix[j * 4 + i]) * inverse;
    q[k] = (matrix[i * 4 + k] + matrix[k * 4 + i]) * inverse;
    q[3] = (matrix[j * 4 + k] - matrix[k * 4 + j]) * inverse;
    q
}

/// MagickDeath.Turn: Right=Cross(direction,Up), Forward=direction;
/// convert both matrices, sign-corrected quaternion Lerp, restore translation.
/// Deliberately retain the direction's magnitude and Y component.
pub fn turn_orientation(current: [f32; 4], direction: [f32; 3], speed: f32, dt: f32) -> [f32; 4] {
    let mut desired = skinned::IDENTITY;
    desired[0..3].copy_from_slice(&[-direction[2], 0.0, direction[0]]);
    desired[8..11].copy_from_slice(&direction.map(|v| -v));
    let current_matrix = skinned::pose_matrix([0.0; 3], current, [1.0; 3]);
    skinned::xna_quat_lerp(
        rotation_quaternion(&current_matrix),
        rotation_quaternion(&desired),
        (speed * dt).clamp(0.0, 1.0),
    )
}

/// Results sampled before phase transitions, as in MagickDeath.Update.
#[derive(Clone, Debug)]
pub struct DeathFrame {
    pub bones: Vec<Mat>,
    /// Snapshot before phase transitions, shared with the exported split-bone rig.
    pub local_poses: Vec<crate::character_pose::Pose>,
    pub model_parent: Mat,
    pub scythe_world: Mat,
    /// Native uses Origin=current tip and Delta=current tip−previous tip.
    /// Preserve that direction rather than silently reversing this segment.
    pub scythe_segment: Segment,
    pub events: death::Step,
    pub additive: bool,
    pub afterimages_visible: bool,
}

#[derive(Clone, Debug)]
pub struct DeathPose {
    pub model_parent: Mat,
    pub local_poses: Vec<crate::character_pose::Pose>,
}

pub struct DeathActor {
    pub previous_position: [f32; 3],
    pub previous_orientation: [f32; 4],
    pub body_orientation: [f32; 4],
    pub state: MagickDeath,
    pub controller: AnimationController,
    pub asset: Resolved,
    pub skeleton: CharacterSkeleton,
    /// The scene applies Turn's quaternion interpolation after each frame.
    pub orientation: [f32; 4],
    pub afterimages: death::Afterimages<DeathPose>,
    clip_durations: [f32; 4],
    oscillation_time: f32,
    last_scythe_tip: [f32; 3],
    hits: BTreeMap<u16, f32>,
}

impl DeathActor {
    /// The native singleton reuses its entity. Initialize resets animation,
    /// timers and hit list, but does not reset mLastScythePosition.
    pub fn reinitialize(&mut self, state: MagickDeath, orientation: [f32; 4]) {
        self.previous_position = state.position;
        self.previous_orientation = orientation;
        self.body_orientation = orientation;
        self.state = state;
        self.orientation = orientation;
        self.oscillation_time = 0.0;
        self.hits.clear();
        self.afterimages.clear_poses();
        self.controller
            .start_named_clip(death::CLIPS[0], self.clip_durations[0], true);
    }

    /// Fail explicitly on absent native clips/joint; synthetic durations or
    /// an unanimated scythe would change both gameplay timing and collision.
    pub fn new(asset: Resolved, state: MagickDeath, orientation: [f32; 4]) -> Result<Self, String> {
        let skeleton = CharacterSkeleton::new(asset.skeleton.clone(), asset.clips.clone());
        let durations = skeleton.clip_durations();
        let mut clip_durations = [0.0; 4];
        for (i, name) in death::CLIPS.iter().enumerate() {
            clip_durations[i] = *durations
                .get(*name)
                .ok_or_else(|| format!("Death model missing clip {name}"))?;
        }
        if skeleton.bone("RightAttach").is_none() {
            return Err("Death model missing RightAttach joint".into());
        }
        let mut controller = AnimationController::default();
        controller.start_named_clip(death::CLIPS[0], clip_durations[0], true);
        Ok(Self {
            previous_position: state.position,
            previous_orientation: orientation,
            body_orientation: orientation,
            state,
            controller,
            asset,
            skeleton,
            orientation,
            afterimages: death::Afterimages::default(),
            clip_durations,
            oscillation_time: 0.0,
            last_scythe_tip: [0.0; 3],
            hits: BTreeMap::new(),
        })
    }

    /// Ground probe uses the horizontally advanced point, not the old body.
    pub fn ground_probe(&self, dt: f32) -> Segment {
        let p = self.state.position;
        Segment {
            origin: [
                p[0] + self.state.movement[0] * self.state.speed * dt,
                p[1] + 1.0,
                p[2] + self.state.movement[2] * self.state.speed * dt,
            ],
            delta: [0.0, -3.0, 0.0],
        }
    }

    /// Caller first runs state.time_step and the scene segment intersection.
    /// Missing ground currently supplies zero Y (native miss output unverified);
    /// a successful scene intersection receives the native +1.2 m offset.
    pub fn update(
        &mut self,
        dt: f32,
        ground: Option<[f32; 3]>,
        target: Option<Target>,
    ) -> Result<DeathFrame, String> {
        self.previous_position = self.state.position;
        self.previous_orientation = self.body_orientation;
        self.body_orientation = self.orientation;
        self.hits.retain(|_, ttl| {
            *ttl -= dt;
            *ttl > 0.0
        });
        self.state
            .advance_position(dt, ground.map_or(0.0, |p| p[1] + 1.2));
        self.oscillation_time += dt;
        let mut visual_at = self.state.position;
        visual_at[1] += -1.2 + self.oscillation_time.cos() * 0.2;
        let parent = skinned::pose_matrix(visual_at, self.orientation, [1.2; 3]);
        self.controller.update(dt);
        let local_poses = self.skeleton.local_poses(self.controller.pose_sample());
        let bones = self.skeleton.skinned_for(&self.controller, &parent);
        let bind = crate::animation_effects::bone_orientation(
            &self.skeleton.skeleton,
            &bones,
            "RightAttach",
        )
        .ok_or_else(|| "Death RightAttach transform is unavailable".to_string())?;
        let translation = [bind[12], bind[13], bind[14]];
        let tip = std::array::from_fn(|i| translation[i] + bind[4 + i]);
        let segment = Segment {
            origin: tip,
            delta: std::array::from_fn(|i| tip[i] - self.last_scythe_tip[i]),
        };
        self.last_scythe_tip = tip;
        let inverse_scale = skinned::pose_matrix([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0 / 1.2; 3]);
        let mut scythe_world = skinned::mul(&bind, &inverse_scale);
        scythe_world[12..15].copy_from_slice(&translation);
        let additive = self.state.ethereal;
        let clock = AnimationClock {
            finished: self.controller.has_finished(),
            crossfade: self.controller.crossfade_enabled(),
            normalized_time: self.controller.time / self.controller.clip_duration(),
        };
        let events = self.state.update_after_movement(dt, target, clock);
        if let Some((phase, fade)) = events.crossfade {
            let index = match phase {
                Phase::Move => 0,
                Phase::ScytheRise => 1,
                Phase::ScytheFall => 2,
                Phase::Disappear => 3,
            };
            self.controller.crossfade_named_clip(
                death::CLIPS[index],
                self.clip_durations[index],
                fade,
                false,
            );
        }
        let afterimages_visible = target.is_some_and(|target| !target.dead)
            && self.afterimages.update(
                dt,
                self.state.movement.iter().map(|v| v * v).sum(),
                self.state.speed,
                DeathPose {
                    model_parent: parent,
                    local_poses: local_poses.clone(),
                },
            );
        Ok(DeathFrame {
            bones,
            local_poses,
            model_parent: parent,
            scythe_world,
            scythe_segment: segment,
            events,
            additive,
            afterimages_visible,
        })
    }

    /// Query/collision host invokes this only after the native scythe segment
    /// intersects an IStatusEffected body during the fall attack window.
    pub fn can_hit(&self, handle: u16) -> bool {
        !self.hits.contains_key(&handle)
    }

    pub fn record_hit(&mut self, handle: u16) {
        self.hits.insert(handle, 1.5);
    }
}
