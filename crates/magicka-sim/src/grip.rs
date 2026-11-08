//! Native Grip action and Character grip lifecycle, independent of scene/pose hosting.
//! See research/npc-grip.md; callers supply collision tests and skin/bone transforms.
use std::collections::BTreeMap;
pub type Handle = u16;
pub type Vector = [f32; 3];
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Pickup,
    Ride,
    Hold,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Actor {
    pub gripper: Option<Handle>,
    pub victim: Option<Handle>,
    pub kind: Kind,
    pub joint: i32,
    pub tolerance: f32,
    pub break_free_count: i32,
    pub damage_accumulation: f32,
    pub immovable: bool,
    pub allow_move: bool,
    pub allow_rotate: bool,
    pub grip_attacking: bool,
    pub next_grip_attack_animation: u16,
    pub allow_attack_rotate: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct ActionFacts {
    pub npc: bool,
    pub owner_position: Vector,
    pub owner_radius: f32,
    pub victim_position: Vector,
    pub victim_radius: f32,
    pub grip_attach_position: Option<Vector>,
    pub action_radius: f32,
    pub victim_dead: bool,
    pub victim_drowning: bool,
    pub victim_solid_shield: bool,
    pub scene_blocked: bool,
    /// Query the owner→victim segment against shields with action_radius,
    /// even when the no-attach reach test substitutes owner_radius.
    pub shield_blocked: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct AcquisitionFacts {
    pub client: bool,
    pub victim_dead: bool,
    pub victim_immortal: bool,
    pub victim_ethereal: bool,
    pub victim_polymorphed: bool,
    pub victim_mass: f32,
    pub owner_knocked_down: bool,
    pub victim_ride_animation: bool,
    /// State of a victim who is itself holding another character.
    pub victim_state: ReleaseState,
}
#[derive(Clone, Copy, Debug)]
pub struct Request {
    pub kind: Kind,
    pub grip_joint: i32,
    pub target_joint: i32,
    /// Already converted by the Grip constructor: truncating float→int,
    /// then zero→10. Character.GripCharacter separately maps <=0→3.
    pub tolerance: i32,
    pub finish_on_grip: bool,
}
/// The owner's state decides the released victim's AllowMove, not its own state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReleaseState {
    Busy,
    Dead,
    Drop,
    Land,
    Idle,
    Jump,
    Move,
    Entangled,
    GripAttack,
    Resurrection,
    #[default]
    Other,
}
impl ReleaseState {
    pub fn victim_may_move(self) -> bool {
        self == Self::Other
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Release {
    pub owner: Handle,
    pub victim: Handle,
    pub victim_may_move: bool,
}
impl Release {
    pub fn ground_probe(self, position: Vector) -> (Vector, Vector) {
        ([position[0], position[1] + 3., position[2]], [0., -6., 0.])
    }
    pub fn landing(self, ground: Vector, radius: f32, length: f32) -> Vector {
        [
            ground[0],
            ground[1] + radius + length * 0.5 + 0.1,
            ground[2],
        ]
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attempt {
    pub action_passed: bool,
    pub acquired: bool,
    /// Native action checks this after calling the void GripCharacter method,
    /// including a rejected acquisition or client-only call.
    pub enter_gripping: bool,
    pub released: Vec<Release>,
    pub release_victim_entanglement: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Lifecycle {
    pub actors: BTreeMap<Handle, Actor>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttackQueue {
    pub queued: bool,
    pub send_network_message: bool,
}
impl Lifecycle {
    /// Character.DamageGripped permits preloading before acquisition. The
    /// network condition compares ordinary NextAttackAnimation (native quirk).
    pub fn queue_attack(
        &mut self,
        owner: Handle,
        animation: u16,
        in_grip_attack: bool,
        networked: bool,
        next_attack_animation: u16,
    ) -> AttackQueue {
        let Some(a) = self.actors.get_mut(&owner) else {
            return AttackQueue::default();
        };
        if a.grip_attacking && a.next_grip_attack_animation != 0 && !in_grip_attack {
            return AttackQueue::default();
        }
        let send_network_message =
            networked && (!a.grip_attacking || animation != next_attack_animation);
        a.next_grip_attack_animation = animation;
        a.allow_attack_rotate = false;
        a.grip_attacking = true;
        AttackQueue {
            queued: true,
            send_network_message,
        }
    }
    /// Public/internal damage callers supply the native signed amount after
    /// their guards; only damage attributed to the held victim accumulates.
    pub fn accumulate_from(&mut self, owner: Handle, attacker: Handle, amount: f32) {
        if let Some(a) = self
            .actors
            .get_mut(&owner)
            .filter(|a| a.victim == Some(attacker))
        {
            a.damage_accumulation += amount;
        }
    }
    pub fn action_allowed(&self, owner: Handle, victim: Handle, f: ActionFacts) -> bool {
        let (Some(owner), Some(victim)) = (self.actors.get(&owner), self.actors.get(&victim))
        else {
            return false;
        };
        if !f.npc
            || owner.victim.is_some()
            || victim.gripper.is_some()
            || f.victim_dead
            || f.victim_drowning
            || f.victim_solid_shield
        {
            return false;
        }
        let (point, radius) = f
            .grip_attach_position
            .map_or((f.owner_position, f.owner_radius), |p| (p, f.action_radius));
        let x = point[0] - f.victim_position[0];
        let z = point[2] - f.victim_position[2];
        x * x + z * z < (radius + f.victim_radius) * (radius + f.victim_radius)
            && !f.scene_blocked
            && !f.shield_blocked
    }
    pub fn attempt(
        &mut self,
        owner: Handle,
        victim: Handle,
        request: Request,
        action: ActionFacts,
        facts: AcquisitionFacts,
    ) -> Attempt {
        if !self.action_allowed(owner, victim, action) {
            return Attempt::default();
        }
        let mut result = Attempt {
            action_passed: true,
            enter_gripping: request.finish_on_grip,
            ..Default::default()
        };
        if facts.victim_dead
            || facts.victim_immortal
            || facts.victim_ethereal
            || facts.victim_polymorphed
            || facts.victim_mass > 500.
            || facts.owner_knocked_down
            || (request.kind == Kind::Ride && !facts.victim_ride_animation)
            || facts.client
        {
            return result;
        }
        // Native releases a victim's own held character before assigning the pair.
        if self.actors.get(&victim).is_some_and(|a| a.victim.is_some()) {
            if let Some(release) = self.release(victim, facts.victim_state) {
                result.released.push(release);
            }
        }
        let a = self.actors.get_mut(&owner).unwrap();
        a.kind = request.kind;
        a.tolerance = if request.tolerance > 0 {
            request.tolerance as f32
        } else {
            3.
        };
        a.damage_accumulation = 0.;
        a.joint = if request.kind == Kind::Pickup {
            request.grip_joint
        } else {
            request.target_joint
        };
        a.victim = Some(victim);
        a.immovable = true;
        a.break_free_count = 0;
        let b = self.actors.get_mut(&victim).unwrap();
        b.gripper = Some(owner);
        b.kind = request.kind;
        b.break_free_count = 0;
        if request.kind == Kind::Pickup {
            b.immovable = true;
        }
        result.acquired = true;
        result.release_victim_entanglement = true;
        result
    }
    pub fn release(&mut self, owner: Handle, state: ReleaseState) -> Option<Release> {
        let a = self.actors.get_mut(&owner)?;
        a.damage_accumulation = 0.;
        let victim = a.victim.take()?;
        a.immovable = false;
        let b = self.actors.get_mut(&victim)?;
        b.allow_move = state.victim_may_move();
        b.allow_rotate = true;
        b.break_free_count = 0;
        b.gripper = None;
        b.immovable = false;
        Some(Release {
            owner,
            victim,
            victim_may_move: b.allow_move,
        })
    }
    pub fn should_release(
        &self,
        owner: Handle,
        victim_burning: bool,
        victim_immortal: bool,
        hit_tolerance: f32,
    ) -> bool {
        let Some(a) = self.actors.get(&owner) else {
            return true;
        };
        let Some(victim) = a.victim.and_then(|h| self.actors.get(&h)) else {
            return true;
        };
        victim_burning
            || victim_immortal
            || victim.break_free_count as f32 >= a.tolerance
            || a.damage_accumulation > hit_tolerance
    }
    /// Base Character.BreakFree truncates strength and wraps integer addition.
    /// Its separate DamageEntanglement(0.5, Earth) call remains a host request.
    pub fn break_free(&mut self, actor: Handle, strength: i32) {
        if let Some(a) = self.actors.get_mut(&actor) {
            a.break_free_count = a.break_free_count.wrapping_add(strength);
        }
    }
    pub fn damage_grip_useful(&self, owner: Handle, hit_tolerance: f32) -> bool {
        self.actors
            .get(&owner)
            .is_some_and(|a| a.victim.is_some() && a.damage_accumulation < hit_tolerance / 2.)
    }
    pub fn damage_grip_score(&self, owner: Handle, hit_tolerance: f32) -> f32 {
        self.actors
            .get(&owner)
            .filter(|a| a.victim.is_some())
            .map_or(f32::MIN, |a| {
                (-2. * a.damage_accumulation / hit_tolerance + 1.) * 2.
            })
    }
}
