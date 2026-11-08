//! Character grips (Character.GripCharacter / ReleaseAttachedCharacter /
//! ShouldReleaseGrip and the Grip, DamageGrip and ReleaseGrip animation
//! actions). Only the Hold type is hosted; Pickup and Ride need joint
//! transforms. See research/npc-grip.md.
use magicka_sim::damage::Damage;

/// Grip.GripType.
pub const GRIP_PICKUP: u8 = 0;
pub const GRIP_RIDE: u8 = 1;
pub const GRIP_HOLD: u8 = 2;

/// The grip fields of one character (both roles share mGripType).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GripLinks {
    /// mGrippedCharacter.
    pub gripped: Option<u16>,
    /// mGripper.
    pub gripper: Option<u16>,
    /// mGripType.
    pub grip_type: u8,
    /// mBreakFreeTolerance (as the gripper).
    pub tolerance: f32,
    /// mBreakFreeCounter (as the victim).
    pub break_free_counter: i32,
    /// mGripDamageAccumulation (as the gripper).
    pub damage_accumulation: f32,
    /// A gripped-character hit landed while in GripAttackState (Damage
    /// changes the state to GrippingState).
    pub grip_attack_interrupted: bool,
    /// ShouldReleaseGrip, refreshed by the scene before the gripper's update.
    pub should_release: bool,
}

impl GripLinks {
    /// The state facts of this character's grip links.
    pub fn facts(&mut self) -> crate::character_motion::GripFacts {
        let should_release = !self.gripping() || self.should_release;
        crate::character_motion::GripFacts {
            gripping: self.gripping(),
            attached_to_gripped: self.attached_to_gripped(),
            gripper_attached: self.gripper_attached(),
            should_release,
            release_now: self.gripping() && should_release,
            entangled_by_gripper: self.entangled_by_gripper(),
            entangled: false,
            attack_interrupted: std::mem::take(&mut self.grip_attack_interrupted),
        }
    }
    pub fn gripping(&self) -> bool {
        self.gripped.is_some()
    }
    /// AttachedToGripped: Ride.
    pub fn attached_to_gripped(&self) -> bool {
        self.grip_type == GRIP_RIDE
    }
    /// GripperAttached: Pickup.
    pub fn gripper_attached(&self) -> bool {
        self.grip_type == GRIP_PICKUP
    }
    /// BaseState.UpdateHit's grip half of the EntangledState test.
    pub fn entangled_by_gripper(&self) -> bool {
        self.gripper.is_some() && !self.gripper_attached() && !self.attached_to_gripped()
    }
}

/// The victim facts ShouldReleaseGrip reads.
#[derive(Clone, Copy, Debug)]
pub struct VictimFacts {
    pub burning: bool,
    pub immortal: bool,
    pub break_free_counter: i32,
}

/// Character.ShouldReleaseGrip (true when not gripping).
pub fn should_release(links: &GripLinks, victim: Option<VictimFacts>, hit_tolerance: f32) -> bool {
    let (Some(_), Some(v)) = (links.gripped, victim) else {
        return true;
    };
    v.burning
        || v.immortal
        || v.break_free_counter as f32 >= links.tolerance
        || links.damage_accumulation > hit_tolerance
}

/// Animation action requests resolved by the scene after the owner's update.
#[derive(Clone, Debug)]
pub enum GripRequest {
    /// AnimationActions.Grip.InternalExecute.
    Grip {
        grip_type: u8,
        radius: f32,
        /// mBreakFreeTolerance (0 is read as 10).
        tolerance: i32,
        /// A grip attach bone is authored (its joint position is not hosted).
        uses_attach: bool,
        finish_on_grip: bool,
    },
    /// AnimationActions.DamageGrip.
    Damage {
        to_owner: bool,
        damages: Vec<Damage>,
    },
    /// AnimationActions.ReleaseGrip.
    Release,
}

/// The Grip action's break-free tolerance after the constructor's
/// (int) truncation and its zero default.
pub fn action_tolerance(authored: f32) -> i32 {
    match authored as i32 {
        0 => 10,
        t => t,
    }
}
