//! The Character.Update subset shared by avatars and NPCs, in original order:
//! AnimationController update with Looped/CrossfadeFinished handling and
//! clip AnimationActions, speed resets, then the character state machine
//! (Idle, Move, Charge, Cast, Attack, Dead), and finally the move animation-speed
//! scaling.
//! Owner operations (spell casting, death) go through `CharacterHost`.
//! Also the AI host adapter both use for event playback.
//! See research/avatar-events.md and research/character-casting.md.
use crate::{
    animation_controller::{
        AnimationController, AnimationSet, ControllerSignal, ANIMATION_IDLE, ANIMATION_NONE,
    },
    avatar_ai::{AvatarAiHost, GroupLeader},
};
use magicka_content::character::{ActionKind, AnimationAction};
use magicka_sim::{
    agent_steering::{
        drift_angle, panic_state_movement, stumble_state_movement, GroupCandidate, PANIC_SPEED,
        STUMBLE_SPEED, WANDER_TURN_RATE,
    },
    character_physics::CharacterBody,
    damage::CastType,
    impact_motion::Vector,
    navmesh::{movement, NavMesh, PathNode},
    spells::SpellType,
};
use std::sync::Arc;

pub const MOVE_WALK: u16 = 135;
pub const MOVE_RUN: u16 = 137;
pub const MOVE_PANIC: u16 = 139;
pub const MOVE_STUMBLE: u16 = 140;
pub const DIE0: u16 = 14;
pub const DIE1: u16 = 15;
pub const DIE_DROWN: u16 = 16;
pub const DIE_DROWN1: u16 = 17;
pub const DIE_DROWN2: u16 = 18;
const CAST_FORCE_PUSH: u16 = 49;
const CAST_FORCE_SPRAY: u16 = 52;
const CAST_FORCE_PROJECTILE: u16 = 48;
const CAST_FORCE_SHIELD: u16 = 51;
const CAST_FORCE_RAILGUN: u16 = 50;
const CAST_FORCE_LIGHTNING: u16 = 47;
const CAST_AREA_PUSH: u16 = 45;
const CAST_AREA_BLAST: u16 = 41;
const CAST_AREA_GROUND: u16 = 43;
const CAST_AREA_FIREWORKS: u16 = 42;
const CAST_AREA_LIGHTNING: u16 = 44;
const CAST_SELF: u16 = 58;
const CAST_SWORD: u16 = 59;
const CAST_SWORD_LIGHTNING: u16 = 60;
const CAST_SWORD_PROJECTILE: u16 = 61;
const CAST_SWORD_RAILGUN: u16 = 62;
const CAST_SWORD_SPRAY: u16 = 64;
const ATTACK_RECOIL: u16 = 34;
/// Animations.block.
const ANIMATION_BLOCK: u16 = 35;
/// Animations.idle_grip / spec_entangled / spec_entangled_attack.
const ANIMATION_IDLE_GRIP: u16 = 13;
const ANIMATION_SPEC_ENTANGLED: u16 = 191;
pub const ANIMATION_SPEC_ENTANGLED_ATTACK: u16 = 192;
/// Animations.stunned / Animations.idle_wnd.
const ANIMATION_STUNNED: u16 = 227;
const ANIMATION_IDLE_WND: u16 = 9;
/// Animations.hit_stun_begin / hit_stun_end.
const ANIMATION_HIT_STUN_BEGIN: u16 = 21;
const ANIMATION_HIT_STUN_END: u16 = 22;
/// CLR float.Epsilon.
const CLR_EPSILON: f32 = f32::from_bits(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterState {
    Idle,
    Move,
    Cast,
    Charge,
    Attack,
    Jump,
    Land,
    Boost,
    Dead,
    /// RessurectionState: the SpawnAnimation of a revived or scripted spawn.
    Resurrection,
    /// PanicState: IsPanicing (status/ordered panic), IsFeared or
    /// IsStumbling (Ride grip), when not entangled.
    Panic,
    /// StunState: stunned (or idle_wnd), no movement or rotation.
    Stun,
    /// KnockDownState: hit_stun_begin, then hit_stun_end, then Idle/Move.
    KnockDown,
    /// DashState: NextDashAnimation, then Idle/Move when it finishes.
    Dash,
    /// BlockState: the block clip while IsBlocking.
    Block,
    /// GrippingState: holding a gripped character.
    Gripping,
    /// GripAttackState: NextGripAttackAnimation on the gripped character.
    GripAttack,
    /// EntangledState: held by a Hold/Ride gripper (entanglement itself is
    /// not modelled).
    Entangled,
}

/// The grip facts the states read (research/npc-grip.md).
#[derive(Clone, Copy, Debug, Default)]
pub struct GripFacts {
    /// IsGripping.
    pub gripping: bool,
    /// AttachedToGripped (Ride).
    pub attached_to_gripped: bool,
    /// GripperAttached (Pickup).
    pub gripper_attached: bool,
    /// ShouldReleaseGrip (true when not gripping).
    pub should_release: bool,
    /// IsGripping && (ShouldReleaseGrip || accumulation > HitTolerance).
    pub release_now: bool,
    /// Gripper != null && !GripperAttached && !AttachedToGripped.
    pub entangled_by_gripper: bool,
    /// Character.IsEntangled (a web).
    pub entangled: bool,
    /// A gripped-character hit landed during GripAttackState.
    pub attack_interrupted: bool,
}

/// Animations enum value of a clip name.
pub fn animation_index(name: &str) -> Option<u16> {
    crate::dialog_enums::ANIMATIONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, i)| *i as u16)
}

/// Owner facts the states read (Character/Avatar properties).
/// Animations.revive / Animations.hit.
const ANIMATION_REVIVE: u16 = 150;
const ANIMATION_HIT: u16 = 19;

#[derive(Clone, Copy, Debug)]
pub struct CharacterFacts {
    pub hit_points: f32,
    /// Character.UndyingTimer is not NaN.
    pub undie: bool,
    pub cannot_die_without_explicit_kill: bool,
    /// Character.mDead (set by Die()).
    pub dead: bool,
    /// Character.Overkilled.
    pub overkilled: bool,
    pub avatar: bool,
    /// Character.CastType (avatars; NPC casting is not reconstructed).
    pub cast_type: CastType,
    /// Spell.GetSpellType of the prepared spell.
    pub spell_type: SpellType,
    /// The prepared magick's SpecialAbility.Animation (Magick spells).
    pub magick_animation: Option<u16>,
    pub charge_unlocked: bool,
    pub in_cutscene: bool,
    /// HasStatus(Frozen): the cached status mask, not the live slot.
    pub frozen: bool,
    /// Uniform [0, 1) draw for DeadState's die0/die1 choice.
    pub random: f64,
    /// Equipment[0].Item.SpellCharged with an empty SpellQueue: the stored
    /// spell's PeekSpell().GetSpellType (CastState weapon animations).
    pub weapon_spell: Option<SpellType>,
    /// IsFeared && !IsEntangled: Character.FearPosition (research/fear.md).
    pub fear: Option<[f32; 3]>,
    /// Character.IsPanicing: mPanic − MaxPanic > float.Epsilon.
    pub panicking: bool,
    /// Character.IsStumbling: gripped with GripType.Ride.
    pub stumbling: bool,
    /// Character.Panic: the template MaxPanic (PanicState's walk blend).
    pub max_panic: f32,
    /// Character.IsStunned: StunTimer > 0 and not Dead.
    pub stunned: bool,
    /// Character.IsKnockedDown.
    pub knocked_down: bool,
    /// CharacterBody.IsTouchingGround.
    pub on_ground: bool,
    pub grip: GripFacts,
}

/// Owner operations requested by clip actions and state transitions,
/// executed synchronously at the original call sites.
pub trait CharacterHost {
    fn special_action(
        &mut self,
        _weapon: i32,
        _ability: Option<&magicka_content::common::SpecialAbility>,
    ) -> Option<u16> {
        None
    }
    fn jump_animation(
        &mut self,
        _body: &mut CharacterBody,
        _elevation: f32,
        _min: Option<f32>,
        _max: Option<f32>,
    ) {
    }
    fn wants_boost(&mut self) -> bool {
        false
    }
    /// Apply BoostState's healing and return whether the state remains active.
    fn update_boost(&mut self) -> bool {
        false
    }
    /// Character.CastSpell(fromStaff, joint); the CastSpell action only calls
    /// it while CurrentSpell is null.
    fn cast_spell(&mut self, from_staff: bool, source: Option<&str>);
    /// CastSpell action Kill: CurrentSpell?.AnimationEnd.
    fn spell_animation_end(&mut self);
    fn begin_charge(&mut self) {}
    fn entered_attack_state(&mut self) {}
    fn entered_cast_state(&mut self) {}
    fn entered_grip_attack_state(&mut self) {}
    /// Ethereal.InternalExecute, on the first action-window execution.
    fn ethereal_action(&mut self, _ethereal: bool, _alpha: f32, _speed: f32) {}
    /// PanicState.OnEnter: SetInvisible(0).
    fn entered_panic_state(&mut self) {}
    /// KnockDownState Update/OnExit: IsHit = IsKnockedDown = false.
    fn clear_knocked_down(&mut self) {}
    fn set_spell_power(&mut self, _power: f32) {}
    fn cast_button_held(&self, _kind: CastType) -> bool {
        false
    }
    fn end_charge(&mut self) {}
    /// CurrentSpell: None when null, else Some(Active).
    fn current_spell(&self) -> Option<bool>;
    /// CastState.OnExit: CurrentSpell.Stop and null, CastType = None.
    fn end_cast(&mut self);
    /// DeadState.OnEnter owner side (NPC: AI.Disable; equipment release).
    fn entered_dead_state(&mut self);
    /// Character.Die() (DeadState once the death clip finished).
    fn die(&mut self);
    /// `NonPlayerCharacter.AI.BusyAbility = null` (Idle OnEnter, Cast and
    /// Attack OnExit).
    fn release_busy_ability(&mut self) {}
    /// DealDamage action: Item.Execute on the equipment slot (first
    /// execution clears the item's hit list).
    fn deal_damage(&mut self, _weapon: i32, _targets: u8, _first: bool) {}
    /// DealDamage Kill: Item.StopExecute.
    fn stop_execute(&mut self, _weapon: i32) {}
    /// SpawnMissile first execution: Item.ExecuteRanged on the slot with
    /// the action's velocity (when its length² exceeds 1e-6).
    fn execute_gun(&mut self, _weapon: i32, _accuracy: f32) {}
    fn stop_gun(&mut self, _weapon: i32) {}
    fn stop_all_guns(&mut self) {}
    fn spawn_missile(&mut self, _weapon: i32, _velocity: Option<Vector>, _item_aligned: bool) {}
    /// DetachItem first execution: Equipment[slot].Release with the
    /// action velocity in body space.
    fn detach_item(&mut self, _slot: i32, _velocity: Vector) {}
    fn remove_status(&mut self, _mask: u16) {}
    /// Block action: BlockItem = weapon (every execution); an NPC's busy
    /// Block ability is released.
    fn block_action(&mut self, _weapon: i32) {}
    /// Block action Kill: BlockItem = -1.
    fn kill_block_action(&mut self) {}
    /// Character.KnockDown() for a gripper that must release: returns
    /// CanBeKnockedDown (the flag is then set).
    fn grip_knock_down(&mut self) -> bool {
        false
    }
    /// Grip, DamageGrip and ReleaseGrip animation actions.
    fn grip_request(&mut self, _request: crate::grip::GripRequest) {}
    fn immortal_action(&mut self, _collide: bool) {}
    fn kill_immortal_action(&mut self) {}
    fn play_animation_effect(&mut self, _index: usize, _action: &AnimationAction, _first: bool) {}
    fn stop_animation_effect(&mut self, _index: usize) {}
    /// PlaySound.InternalExecute plays only on the action's first execution.
    /// The bank is the native unchecked UInt16 conversion of its stored Int32.
    fn play_animation_sound(&mut self, _bank: u16, _sound: &str) {}
    /// Footstep.InternalExecute first execution on the ground while not
    /// levitating: PlayCue(Footsteps, cue, Weight/WaterDepth, emitter). The
    /// default routes the cue like a PlaySound action (variables dropped).
    fn play_footstep(&mut self, cue: &str, _variables: crate::character_sounds::FootstepVariables) {
        self.play_animation_sound(crate::audio::banks::FOOTSTEPS, cue);
    }
}

/// A host for characters that never cast (NPCs until NPC abilities exist).
pub struct NoCasting<'a> {
    pub died: &'a mut bool,
    pub entered_dead: &'a mut bool,
}
impl CharacterHost for NoCasting<'_> {
    fn cast_spell(&mut self, _: bool, _: Option<&str>) {}
    fn spell_animation_end(&mut self) {}
    fn current_spell(&self) -> Option<bool> {
        None
    }
    fn end_cast(&mut self) {}
    fn entered_dead_state(&mut self) {
        *self.entered_dead = true;
    }
    fn die(&mut self) {
        *self.died = true;
    }
}

/// Status-driven motion facts of one update (`update_damage_and_status_on`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StatusMotion {
    /// Cold slowdown (StatusEffect.GetSlowdown) when Cold is present.
    pub cold: Option<f32>,
    /// Frozen magnitude is positive.
    pub frozen: bool,
    /// ZapModifier applied this update.
    pub zap: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct CharacterMotion {
    pub state: CharacterState,
    /// Character.PreviousState (set by ChangeState).
    pub previous_state: CharacterState,
    pub controller: AnimationController,
    pub animations: Arc<AnimationSet>,
    pub special_idle: u16,
    /// Character.SpawnAnimation (RessurectionState's clip).
    pub spawn_animation: u16,
    /// Speed multiplier of the animation controller (Character.Update).
    pub animation_speed: f32,
    /// mCurrentActions with mExecutedActions / mDeadActions.
    pub actions: Vec<AnimationAction>,
    executed: Vec<bool>,
    dead_actions: Vec<bool>,
    /// Character.mDeadTimer (Initialize: 20 + 20·random).
    pub dead_timer: f32,
    /// Character.mDrowning (set by Drown).
    pub drowning: bool,
    /// Character.mAttacking / mNextAttackAnimation / mAllowAttackRotate.
    pub attacking: bool,
    pub next_attack_animation: u16,
    pub allow_attack_rotate: bool,
    /// Character.mDashing / mNextDashAnimation.
    pub dashing: bool,
    pub next_dash_animation: u16,
    /// Character.mBlock (IsBlocking) and NonPlayerCharacter.mBlockTimer.
    pub blocking: bool,
    pub block_timer: f32,
    /// Character.mGripAttack / mNextGripAttackAnimation.
    pub grip_attacking: bool,
    pub next_grip_attack_animation: u16,
    /// A ChangeState issued outside the state machine (Grip finishOnGrip,
    /// Character.OrderPanic), applied at the next state update.
    pub forced_state: Option<CharacterState>,
    /// Character.mWanderAngle: PanicState's own drifting heading (distinct
    /// from Agent.WanderAngle), never initialised natively (0).
    pub wander_angle: f32,
    /// The delta of the current `update`, for PanicState's angle drift.
    dt: f32,
}

fn length_sq(v: Vector) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

impl CharacterMotion {
    /// Character.Initialize: StartClip(idle), then SetState(IdleState).
    pub fn new(animations: Arc<AnimationSet>) -> Self {
        let mut motion = Self {
            state: CharacterState::Idle,
            previous_state: CharacterState::Idle,
            controller: AnimationController::default(),
            animations,
            special_idle: ANIMATION_NONE,
            spawn_animation: ANIMATION_NONE,
            animation_speed: 1.0,
            actions: Vec::new(),
            executed: Vec::new(),
            dead_actions: Vec::new(),
            dead_timer: 20.0,
            drowning: false,
            attacking: false,
            next_attack_animation: ANIMATION_NONE,
            allow_attack_rotate: false,
            dashing: false,
            next_dash_animation: ANIMATION_NONE,
            blocking: false,
            block_timer: 0.0,
            grip_attacking: false,
            next_grip_attack_animation: ANIMATION_NONE,
            forced_state: None,
            wander_angle: 0.0,
            dt: 0.0,
        };
        let set = motion.animations.clone();
        motion.controller.start(&set, ANIMATION_IDLE);
        motion.crossfade_finished(&mut NoCasting {
            died: &mut false,
            entered_dead: &mut false,
        });
        motion.idle_enter();
        motion
    }

    /// Character.Attack (offline): queue the attack animation.
    pub fn attack(&mut self, animation: u16, allow_rotate: bool) {
        self.next_attack_animation = animation;
        self.allow_attack_rotate = allow_rotate;
        self.attacking = true;
    }
    /// Character.Dash (offline): it queues the attack animation and
    /// rotation like Attack, but raises Dashing; DashState plays
    /// NextDashAnimation, which Dash does not set.
    pub fn dash(&mut self, animation: u16, allow_rotate: bool) {
        self.next_attack_animation = animation;
        self.allow_attack_rotate = allow_rotate;
        self.dashing = true;
    }
    /// NonPlayerCharacter.Block: one second of IsBlocking.
    pub fn block(&mut self) {
        self.block_timer = 1.0;
        self.blocking = true;
    }
    /// NonPlayerCharacter.Update: the block timer runs out.
    pub fn tick_block_timer(&mut self, dt: f32) {
        self.block_timer -= dt;
        if (self.block_timer <= 0.0) & self.blocking {
            self.blocking = false;
        }
    }
    /// Character.DamageGripped (offline).
    pub fn damage_gripped(&mut self, animation: u16) {
        if !self.grip_attacking
            || self.next_grip_attack_animation == ANIMATION_NONE
            || self.state == CharacterState::GripAttack
        {
            self.next_grip_attack_animation = animation;
            self.allow_attack_rotate = false;
            self.grip_attacking = true;
        }
    }
    /// NonPlayerCharacter.StopAttacking.
    pub fn stop_attacking(&mut self) {
        self.attacking = false;
        self.grip_attacking = false;
        self.dashing = false;
        self.next_attack_animation = ANIMATION_NONE;
    }

    pub fn go_to_animation(&mut self, animation: u16, time: f32) {
        let set = self.animations.clone();
        self.controller.go_to_animation(&set, animation, time);
    }
    /// Character.ForceAnimation: StartClip, then OnCrossfadeFinished.
    pub fn force_animation(&mut self, animation: u16, host: &mut dyn CharacterHost) {
        let set = self.animations.clone();
        self.controller.force_animation(&set, animation);
        self.crossfade_finished(host);
    }
    pub fn has_animation(&self, animation: u16) -> bool {
        self.animations.lookup(animation).is_some()
    }

    /// OnAnimationLooped: every action may execute again.
    fn looped(&mut self) {
        self.executed = vec![false; self.actions.len()];
        self.dead_actions = vec![false; self.actions.len()];
    }
    /// OnCrossfadeFinished: kill live actions of a replaced list, then take
    /// the current animation's actions.
    fn crossfade_finished(&mut self, host: &mut dyn CharacterHost) {
        let actions = self
            .animations
            .lookup(self.controller.current_animation)
            .map(|a| a.actions.clone())
            .unwrap_or_default();
        if actions != self.actions {
            for (i, action) in self.actions.iter().enumerate() {
                if !self.dead_actions.get(i).copied().unwrap_or(false) {
                    kill_action(action, i, host);
                }
            }
        }
        self.actions = actions;
        self.looped();
    }

    fn idle_enter(&mut self) {
        // IdleState.OnEnter animation choice (network, gripped, poisoned and
        // aggressive branches are not modelled).
        if self.special_idle != ANIMATION_NONE {
            self.go_to_animation(self.special_idle, 0.2);
        } else {
            self.go_to_animation(ANIMATION_IDLE, 0.2);
        }
    }

    fn enter_state(
        &mut self,
        body: &mut CharacterBody,
        facts: &CharacterFacts,
        state: CharacterState,
        host: &mut dyn CharacterHost,
    ) {
        self.state = state;
        match state {
            CharacterState::Jump => {
                body.allow_move = false;
                body.allow_rotate = false;
                self.go_to_animation(if self.has_animation(144) { 144 } else { 143 }, 0.2);
            }
            CharacterState::Land => {
                body.allow_move = false;
                body.allow_rotate = false;
                self.go_to_animation(146, 0.13);
            }
            CharacterState::Boost => self.go_to_animation(36, 0.2),
            CharacterState::Idle => {
                host.release_busy_ability();
                body.allow_move = true;
                body.allow_rotate = true;
                self.idle_enter();
            }
            CharacterState::Attack => {
                host.entered_attack_state();
                let animation = if self.next_attack_animation == ANIMATION_NONE {
                    ANIMATION_IDLE
                } else {
                    self.next_attack_animation
                };
                self.go_to_animation(animation, 0.075);
                self.next_attack_animation = ANIMATION_NONE;
            }
            CharacterState::Move => {
                body.allow_move = true;
                body.allow_rotate = true;
            }
            CharacterState::Panic => {
                // ReleaseAttachedCharacter is not modelled.
                host.entered_panic_state();
                body.allow_move = true;
                body.allow_rotate = true;
                if facts.stumbling {
                    body.run_backwards = true;
                    self.go_to_animation(MOVE_STUMBLE, 0.2);
                } else {
                    self.go_to_animation(MOVE_PANIC, 0.2);
                }
            }
            CharacterState::Charge => {
                self.go_to_animation(
                    if facts.cast_type == CastType::Area {
                        37
                    } else {
                        39
                    },
                    0.075,
                );
                body.owner.turn_speed *= 0.125;
                host.begin_charge();
            }
            CharacterState::Cast => {
                host.entered_cast_state();
                self.cast_enter(body, facts);
                // CastState.OnEnter tail: a frozen Self cast fires at once.
                if facts.frozen && facts.cast_type == CastType::SelfCast {
                    host.cast_spell(true, Some(""));
                }
            }
            CharacterState::Dead => {
                body.allow_move = false;
                body.allow_rotate = false;
                host.entered_dead_state();
                // DeadState.OnEnter: drowning picks die_drown/1/2 by thirds
                // (or halves) and forces the clip.
                if self.drowning {
                    let r = facts.random;
                    let clip = if self.has_animation(DIE_DROWN1) {
                        if self.has_animation(DIE_DROWN2) {
                            if r > f64::from(0.666f32) {
                                DIE_DROWN2
                            } else if r > f64::from(0.333f32) {
                                DIE_DROWN1
                            } else {
                                DIE_DROWN
                            }
                        } else if r > 0.5 {
                            DIE_DROWN1
                        } else {
                            DIE_DROWN
                        }
                    } else {
                        DIE_DROWN
                    };
                    self.force_animation(clip, host);
                } else if self.has_animation(DIE1) && facts.random > 0.5 {
                    self.go_to_animation(DIE1, 0.05);
                } else {
                    self.go_to_animation(DIE0, 0.05);
                }
            }
            CharacterState::Resurrection => {
                body.allow_move = false;
                body.allow_rotate = false;
                self.force_animation(self.spawn_animation, host);
            }
            CharacterState::Dash => {
                self.go_to_animation(self.next_dash_animation, 0.2);
            }
            CharacterState::Block => self.go_to_animation(ANIMATION_BLOCK, 0.1),
            CharacterState::Gripping => {
                let clip = if self.has_animation(ANIMATION_IDLE_GRIP) {
                    ANIMATION_IDLE_GRIP
                } else {
                    ANIMATION_IDLE
                };
                self.go_to_animation(clip, 0.1);
                body.allow_rotate = false;
                host.release_busy_ability();
            }
            CharacterState::GripAttack => {
                host.entered_grip_attack_state();
                self.go_to_animation(self.next_grip_attack_animation, 0.075);
                self.next_grip_attack_animation = ANIMATION_NONE;
                body.allow_rotate = false;
                body.allow_move = false;
            }
            CharacterState::Entangled => {
                body.allow_move = false;
                body.allow_rotate = false;
                self.go_to_animation(ANIMATION_SPEC_ENTANGLED, 0.2);
                self.special_idle = ANIMATION_NONE;
            }
            CharacterState::KnockDown => {
                // ReleaseAttachedCharacter is not modelled.
                body.allow_rotate = false;
                self.go_to_animation(ANIMATION_HIT_STUN_BEGIN, 0.05);
                if self.previous_state == CharacterState::Charge {
                    host.cast_spell(true, Some(""));
                }
            }
            CharacterState::Stun => {
                let clip = if self.has_animation(ANIMATION_STUNNED) {
                    ANIMATION_STUNNED
                } else {
                    ANIMATION_IDLE_WND
                };
                self.go_to_animation(clip, 0.35);
                body.allow_move = false;
                body.allow_rotate = false;
            }
        }
    }

    /// Assign SpawnAnimation, then ChangeState(RessurectionState.Instance).
    pub fn begin_resurrection(
        &mut self,
        body: &mut CharacterBody,
        facts: &CharacterFacts,
        animation: u16,
        host: &mut dyn CharacterHost,
    ) {
        self.spawn_animation = animation;
        self.change_state(body, facts, CharacterState::Resurrection, host);
    }

    /// Character.ChangeState: OnExit of the old state, then OnEnter.
    pub fn change_state(
        &mut self,
        body: &mut CharacterBody,
        facts: &CharacterFacts,
        state: CharacterState,
        host: &mut dyn CharacterHost,
    ) {
        self.previous_state = self.state;
        match self.state {
            CharacterState::Idle => {
                body.allow_move = false;
                self.special_idle = ANIMATION_NONE;
            }
            CharacterState::Move => body.allow_move = false,
            CharacterState::Charge => host.end_charge(),
            CharacterState::Cast => {
                host.release_busy_ability();
                host.end_cast();
                body.allow_rotate = true;
            }
            CharacterState::Attack => {
                // AttackState.OnExit: busy ability, Attacking, current spell
                // and cast type; stop the equipment's gunfire.
                host.release_busy_ability();
                self.attacking = false;
                host.stop_all_guns();
                host.end_cast();
            }
            CharacterState::Resurrection => body.allow_rotate = true,
            CharacterState::Stun => {
                body.allow_move = true;
                body.allow_rotate = true;
            }
            CharacterState::KnockDown => host.clear_knocked_down(),
            CharacterState::Dash => self.dashing = false,
            CharacterState::Block | CharacterState::Entangled => {}
            CharacterState::Gripping => body.allow_rotate = true,
            CharacterState::GripAttack => {
                host.release_busy_ability();
                self.grip_attacking = false;
                body.allow_move = true;
                body.allow_rotate = true;
            }
            // PanicState.OnExit: RunBackward = false.
            CharacterState::Panic => body.run_backwards = false,
            CharacterState::Dead
            | CharacterState::Boost
            | CharacterState::Jump
            | CharacterState::Land => {}
        }
        self.enter_state(body, facts, state, host);
    }

    /// CastState.OnEnter (magicks, frozen self-casts and weapon spells are
    /// not reconstructed).
    fn cast_enter(&mut self, body: &mut CharacterBody, facts: &CharacterFacts) {
        if facts.spell_type == SpellType::Magick {
            // CastState.OnEnter: the magick's own animation, no rotation.
            if let Some(animation) = facts.magick_animation {
                self.go_to_animation(animation, 0.075);
            }
            body.allow_rotate = false;
            return;
        }
        let animation = match facts.cast_type {
            CastType::Force => match facts.spell_type {
                SpellType::Push => Some(CAST_FORCE_PUSH),
                SpellType::Spray => Some(CAST_FORCE_SPRAY),
                SpellType::Projectile => Some(CAST_FORCE_PROJECTILE),
                SpellType::Shield => Some(CAST_FORCE_SHIELD),
                SpellType::Beam => Some(CAST_FORCE_RAILGUN),
                SpellType::Lightning => Some(CAST_FORCE_LIGHTNING),
                SpellType::Magick => None,
            },
            CastType::Area => match facts.spell_type {
                SpellType::Push => Some(CAST_AREA_PUSH),
                SpellType::Spray | SpellType::Beam => Some(CAST_AREA_BLAST),
                SpellType::Projectile => Some(CAST_AREA_GROUND),
                SpellType::Shield => Some(CAST_AREA_FIREWORKS),
                SpellType::Lightning => Some(CAST_AREA_LIGHTNING),
                SpellType::Magick => None,
            },
            CastType::SelfCast => Some(CAST_SELF),
            CastType::Weapon => Some(match facts.weapon_spell {
                Some(SpellType::Spray) => CAST_SWORD_SPRAY,
                Some(SpellType::Projectile | SpellType::Shield) => CAST_SWORD_PROJECTILE,
                Some(SpellType::Beam) => CAST_SWORD_RAILGUN,
                Some(SpellType::Lightning) => CAST_SWORD_LIGHTNING,
                _ => CAST_SWORD,
            }),
            _ => None,
        };
        if let Some(animation) = animation {
            // Only the plain (enchanting) cast_sword blends over 0.15 s.
            let time = if facts.cast_type == CastType::Weapon && facts.weapon_spell.is_none() {
                0.15
            } else {
                0.075
            };
            self.go_to_animation(animation, time);
        }
    }

    /// BaseState.UpdateBloatDeath (bloating is not modelled).
    fn death(facts: &CharacterFacts) -> bool {
        facts.hit_points <= 0.0 && !facts.cannot_die_without_explicit_kill
    }

    /// BaseState.UpdateHit, stun branch only: Flying, Entangled, Pushed,
    /// KnockDown, grip release and Hit come first natively and are not part
    /// of this state machine, so they count as absent.
    /// BaseState.UpdateHit (Flying, Pushed, IsHit and bloat are not
    /// modelled).
    fn update_hit(
        &self,
        facts: &CharacterFacts,
        host: &mut dyn CharacterHost,
    ) -> Option<CharacterState> {
        if (facts.grip.entangled || facts.grip.entangled_by_gripper) && !self.attacking {
            return Some(CharacterState::Entangled);
        }
        if facts.knocked_down {
            return Some(CharacterState::KnockDown);
        }
        // A gripper that must let go is knocked down (a Pickup gripper on
        // the ground drops instead; DropState is not modelled).
        if facts.grip.release_now && !(facts.on_ground && facts.grip.gripper_attached) {
            host.grip_knock_down();
        }
        if facts.knocked_down {
            return Some(CharacterState::KnockDown);
        }
        facts.stunned.then_some(CharacterState::Stun)
    }

    /// Native Idle/Move choice: unlocked Force projectile or Force/Area push.
    fn cast_state(facts: &CharacterFacts) -> CharacterState {
        if facts.charge_unlocked
            && ((facts.spell_type == SpellType::Projectile && facts.cast_type == CastType::Force)
                || (facts.spell_type == SpellType::Push
                    && matches!(facts.cast_type, CastType::Force | CastType::Area)))
        {
            CharacterState::Charge
        } else {
            CharacterState::Cast
        }
    }

    fn wants_cast(facts: &CharacterFacts) -> bool {
        facts.avatar && facts.cast_type != CastType::Blast
    }

    fn state_update(
        &mut self,
        body: &mut CharacterBody,
        facts: &CharacterFacts,
        host: &mut dyn CharacterHost,
    ) -> Option<CharacterState> {
        if let Some(state) = self.forced_state.take() {
            // Character.ChangeState ignores the current state.
            if state != self.state {
                return Some(state);
            }
        }
        match self.state {
            CharacterState::Idle => {
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                if let Some(next) = self.update_hit(facts, host) {
                    return Some(next);
                }
                // BaseState.UpdateActions: jumping before panic and boost.
                if body.is_jumping {
                    return Some(CharacterState::Jump);
                }
                if Self::wants_panic(facts) {
                    return Some(CharacterState::Panic);
                }
                if host.wants_boost() {
                    return Some(CharacterState::Boost);
                }
                if facts.grip.gripping && !facts.grip.attached_to_gripped {
                    return Some(CharacterState::Gripping);
                }
                if body.moving() {
                    return Some(CharacterState::Move);
                }
                if Self::wants_cast(facts) && !facts.in_cutscene {
                    return Some(Self::cast_state(facts));
                }
                if self.dashing {
                    return Some(CharacterState::Dash);
                }
                if self.attacking {
                    return Some(CharacterState::Attack);
                }
                if self.grip_attacking {
                    return Some(CharacterState::GripAttack);
                }
                if self.blocking {
                    return Some(CharacterState::Block);
                }
                if self.controller.has_finished() && !self.controller.crossfade_enabled() {
                    // IdleState.Update: replay idle after a finished clip.
                    if self.special_idle != ANIMATION_NONE {
                        self.go_to_animation(self.special_idle, 0.1);
                    } else {
                        self.go_to_animation(ANIMATION_IDLE, 0.5);
                    }
                }
                None
            }
            CharacterState::Move => {
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                if let Some(next) = self.update_hit(facts, host) {
                    return Some(next);
                }
                // BaseState.UpdateActions: jumping before panic and boost.
                if body.is_jumping {
                    return Some(CharacterState::Jump);
                }
                if Self::wants_panic(facts) {
                    return Some(CharacterState::Panic);
                }
                if host.wants_boost() {
                    return Some(CharacterState::Boost);
                }
                if facts.grip.gripping && !facts.grip.attached_to_gripped {
                    return Some(CharacterState::Gripping);
                }
                // MoveState: Dashing, Attacking, Blocking, then avatar casting.
                if self.dashing {
                    return Some(CharacterState::Dash);
                }
                if self.attacking {
                    return Some(CharacterState::Attack);
                }
                if self.blocking {
                    return Some(CharacterState::Block);
                }
                if Self::wants_cast(facts) {
                    return Some(Self::cast_state(facts));
                }
                if length_sq(body.movement()) <= 0.01 {
                    return Some(CharacterState::Idle);
                }
                let v = body.velocity;
                let normalized = length_sq(v).sqrt() / body.max_speed;
                if normalized >= 0.6 {
                    self.go_to_animation(MOVE_RUN, 0.25);
                } else if normalized < 0.4
                    || (self.controller.current_animation != MOVE_RUN
                        && self.controller.current_animation != MOVE_WALK)
                {
                    self.go_to_animation(MOVE_WALK, 0.25);
                }
                None
            }
            CharacterState::Charge => {
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                if let Some(next) = self.update_hit(facts, host) {
                    return Some(next);
                }
                if facts.in_cutscene {
                    return Some(CharacterState::Idle);
                }
                if !self.controller.is_looping() {
                    if self.controller.has_finished() {
                        self.go_to_animation(
                            if facts.cast_type == CastType::Area {
                                38
                            } else {
                                40
                            },
                            0.075,
                        );
                        host.set_spell_power(1.);
                    } else if !self.controller.crossfade_enabled() {
                        if let Some(pose) = self.controller.pose_sample() {
                            host.set_spell_power(self.controller.time / pose.duration);
                        }
                    }
                }
                if facts.avatar
                    && matches!(facts.cast_type, CastType::Force | CastType::Area)
                    && !host.cast_button_held(facts.cast_type)
                {
                    return Some(CharacterState::Cast);
                }
                None
            }
            CharacterState::Cast => {
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                if self.controller.current_animation == ANIMATION_IDLE {
                    return Some(CharacterState::Idle);
                }
                let moving = length_sq(body.movement()).sqrt() > CLR_EPSILON;
                let leave = |motion: &mut Self| {
                    if moving {
                        motion.go_to_animation(MOVE_WALK, 0.25);
                        Some(CharacterState::Move)
                    } else {
                        Some(CharacterState::Idle)
                    }
                };
                if self.controller.is_looping()
                    && !self.controller.crossfade_enabled()
                    && facts.cast_type != CastType::Force
                {
                    return leave(self);
                }
                if self.controller.has_finished() {
                    return leave(self);
                }
                // (Attacking → AttackState is not reconstructed.)
                if host.current_spell() == Some(false) {
                    return leave(self);
                }
                if facts.frozen {
                    return Some(CharacterState::Idle);
                }
                None
            }
            CharacterState::Jump => {
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                if !body.is_jumping {
                    return Some(CharacterState::Land);
                }
                // GrippingState transition awaits the grip host.
                if body.velocity[1] < 0.
                    && self.controller.current_animation == 144
                    && self.has_animation(145)
                    && !self.controller.crossfade_enabled()
                {
                    self.go_to_animation(145, 0.3);
                }
                None
            }
            CharacterState::Land => {
                if body.is_touching_ground() {
                    if self.controller.current_animation != 146
                        && self.controller.current_animation != ATTACK_RECOIL
                    {
                        self.go_to_animation(146, 0.1);
                    } else if self.controller.has_finished() && !self.controller.crossfade_enabled()
                    {
                        return Some(if length_sq(body.movement()).sqrt() < CLR_EPSILON {
                            CharacterState::Idle
                        } else {
                            CharacterState::Move
                        });
                    }
                }
                // IsGripped fallback awaits the grip host.
                None
            }
            CharacterState::Attack => {
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                if let Some(next) = self.update_hit(facts, host) {
                    return Some(next);
                }
                body.allow_rotate = self.controller.crossfade_enabled()
                    | self.allow_attack_rotate
                    | (self.controller.time < 0.1);
                if self.next_attack_animation != ANIMATION_NONE
                    && self.attacking
                    && !self.controller.crossfade_enabled()
                    && self.controller.has_finished()
                {
                    if self.controller.current_animation == ATTACK_RECOIL {
                        self.next_attack_animation = ANIMATION_NONE;
                    } else {
                        self.go_to_animation(self.next_attack_animation, 0.2);
                        self.next_attack_animation = ANIMATION_NONE;
                    }
                }
                if self.next_attack_animation == ANIMATION_NONE
                    && (self.controller.has_finished()
                        || (self.controller.is_looping() && !self.controller.crossfade_enabled()))
                {
                    self.attacking = false;
                    if length_sq(body.movement()).sqrt() < CLR_EPSILON {
                        return Some(CharacterState::Idle);
                    }
                    return Some(CharacterState::Move);
                }
                None
            }
            CharacterState::Boost => {
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                if host.update_boost() {
                    None
                } else if length_sq(body.movement()).sqrt() < CLR_EPSILON {
                    Some(CharacterState::Idle)
                } else {
                    Some(CharacterState::Move)
                }
            }
            CharacterState::Dead => {
                // DeadState.Update: an undied character (HP restored, timer
                // armed) revives once its clip stopped playing.
                if facts.hit_points > 0.0 && facts.undie && !self.controller.is_playing() {
                    self.spawn_animation = if self.has_animation(ANIMATION_REVIVE) {
                        ANIMATION_REVIVE
                    } else {
                        ANIMATION_HIT
                    };
                    return Some(CharacterState::Resurrection);
                }
                if self.controller.has_finished() && !facts.dead {
                    host.die();
                }
                None
            }
            CharacterState::Resurrection => {
                // (Bloating → BloatState is not modelled.)
                if facts.overkilled && !facts.cannot_die_without_explicit_kill {
                    return Some(CharacterState::Dead);
                }
                if (self.controller.has_finished() && !self.controller.crossfade_enabled())
                    || self.controller.is_looping()
                {
                    if facts.hit_points <= 0.0 {
                        return Some(CharacterState::Dead);
                    }
                    if length_sq(body.movement()).sqrt() < CLR_EPSILON {
                        return Some(CharacterState::Idle);
                    }
                    return Some(CharacterState::Move);
                }
                None
            }
            CharacterState::KnockDown => {
                // KnockDownState.Update clears the flags before UpdateHit.
                host.clear_knocked_down();
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                let cleared = CharacterFacts {
                    knocked_down: false,
                    ..*facts
                };
                if let Some(next) = self.update_hit(&cleared, host) {
                    return Some(next);
                }
                if (self.controller.has_finished() && !self.controller.crossfade_enabled())
                    || facts.frozen
                {
                    if self.controller.current_animation != ANIMATION_HIT_STUN_BEGIN {
                        if length_sq(body.movement()).sqrt() < CLR_EPSILON {
                            return Some(CharacterState::Idle);
                        }
                        return Some(CharacterState::Move);
                    }
                    self.go_to_animation(ANIMATION_HIT_STUN_END, 0.05);
                }
                None
            }
            CharacterState::Dash => {
                // DashState.Update: bloat death (not modelled), UpdateHit,
                // gripping, then Idle/Move after the clip.
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                if let Some(next) = self.update_hit(facts, host) {
                    return Some(next);
                }
                if facts.grip.gripping {
                    return Some(CharacterState::Gripping);
                }
                if !self.controller.crossfade_enabled() && self.controller.has_finished() {
                    if length_sq(body.movement()).sqrt() < CLR_EPSILON {
                        return Some(CharacterState::Idle);
                    }
                    return Some(CharacterState::Move);
                }
                None
            }
            CharacterState::Block => {
                // BlockState.Update: bloat death (not modelled), UpdateHit,
                // Attacking, then Idle/Move once IsBlocking ends.
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                if let Some(next) = self.update_hit(facts, host) {
                    return Some(next);
                }
                if self.attacking {
                    return Some(CharacterState::Attack);
                }
                if !self.blocking {
                    if length_sq(body.movement()).sqrt() < CLR_EPSILON {
                        return Some(CharacterState::Idle);
                    }
                    return Some(CharacterState::Move);
                }
                None
            }
            CharacterState::Gripping => {
                // GrippingState.Update (no bloat/death test of its own).
                if let Some(next) = self.update_hit(facts, host) {
                    return Some(next);
                }
                if self.grip_attacking {
                    return Some(CharacterState::GripAttack);
                }
                if self.attacking {
                    return Some(CharacterState::Attack);
                }
                if facts.grip.release_now {
                    if (!facts.on_ground || !facts.grip.gripper_attached) && host.grip_knock_down()
                    {
                        return Some(CharacterState::KnockDown);
                    }
                    // DropState is not modelled.
                    return Some(CharacterState::Idle);
                }
                if !facts.grip.gripping {
                    if length_sq(body.movement()).sqrt() < CLR_EPSILON {
                        return Some(CharacterState::Idle);
                    }
                    return Some(CharacterState::Move);
                }
                None
            }
            CharacterState::GripAttack => {
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                if let Some(next) = self.update_hit(facts, host) {
                    return Some(next);
                }
                // Character.Damage from the gripped character (applied at the
                // next update here).
                if facts.grip.attack_interrupted {
                    return Some(CharacterState::Gripping);
                }
                if !self.controller.crossfade_enabled()
                    && self.controller.has_finished()
                    && self.grip_attacking
                    && self.next_grip_attack_animation != ANIMATION_NONE
                {
                    if self.controller.current_animation == ATTACK_RECOIL {
                        self.next_grip_attack_animation = ANIMATION_NONE;
                    } else {
                        self.go_to_animation(self.next_grip_attack_animation, 0.2);
                        self.next_grip_attack_animation = ANIMATION_NONE;
                    }
                } else {
                    if facts.grip.should_release {
                        self.grip_attacking = false;
                        if length_sq(body.movement()).sqrt() < CLR_EPSILON {
                            return Some(CharacterState::Idle);
                        }
                        return Some(CharacterState::Move);
                    }
                    if self.controller.has_finished() {
                        host.release_busy_ability();
                    }
                }
                None
            }
            CharacterState::Entangled => {
                // EntangledState.Update: bloat death, avatar casting,
                // Attacking, then Idle/Move once no longer held.
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                if Self::wants_cast(facts) {
                    return Some(CharacterState::Cast);
                }
                if self.attacking {
                    return Some(CharacterState::Attack);
                }
                if !facts.grip.entangled && !facts.grip.entangled_by_gripper {
                    if length_sq(body.movement()).sqrt() < CLR_EPSILON {
                        return Some(CharacterState::Idle);
                    }
                    return Some(CharacterState::Move);
                }
                if self.controller.current_animation != ANIMATION_SPEC_ENTANGLED
                    && self.controller.has_finished()
                {
                    self.go_to_animation(ANIMATION_SPEC_ENTANGLED, 0.1);
                }
                None
            }
            CharacterState::Stun => {
                // StunState.Update: bloat death (not modelled), UpdateHit
                // (a renewed stun stays here), then Idle once it ends.
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                if !facts.stunned {
                    return Some(CharacterState::Idle);
                }
                None
            }
            CharacterState::Panic => {
                // PanicState.Update. Not reconstructed: BloatState, the
                // avatar PanicCastState and EntangledState.
                if Self::death(facts) {
                    return Some(CharacterState::Dead);
                }
                // The character wander heading drifts by (random − .5)·60·dt
                // (MagickaMath.Random; the facts draw stands in for it).
                self.wander_angle = drift_angle(
                    self.wander_angle,
                    facts.random as f32,
                    WANDER_TURN_RATE,
                    self.dt,
                );
                let direction = body.direction();
                // INFERRED for avatars: the original mixes DesiredInputDirection,
                // the rewrite the body's Movement for every owner.
                let movement = body.movement();
                if facts.panicking {
                    body.set_speed_multiplier(body.speed_multiplier() * PANIC_SPEED);
                    body.set_movement(panic_state_movement(
                        self.wander_angle,
                        direction,
                        movement,
                        facts.max_panic,
                    ));
                } else if let Some(source) = facts.fear {
                    body.set_speed_multiplier(
                        body.speed_multiplier() * magicka_sim::fear::FEARED_SPEED,
                    );
                    body.set_movement(magicka_sim::fear::flee_movement(
                        body.position,
                        source,
                        movement,
                    ));
                } else if facts.stumbling {
                    body.set_speed_multiplier(body.speed_multiplier() * STUMBLE_SPEED);
                    body.set_movement(stumble_state_movement(self.wander_angle, direction));
                } else {
                    return Some(CharacterState::Idle);
                }
                None
            }
        }
    }

    /// Character.Update from the animation controller to the state machine.
    /// Call `finish` after the owner's spell CastUpdate.
    pub fn update(
        &mut self,
        body: &mut CharacterBody,
        facts: &CharacterFacts,
        dt: f32,
        host: &mut dyn CharacterHost,
    ) {
        let dead = facts.dead && !facts.cannot_die_without_explicit_kill;
        if !dead
            || self.dead_timer > -10.0
            || !self.controller.has_finished()
            || self.controller.crossfade_enabled()
        {
            self.controller.update(dt);
            for signal in std::mem::take(&mut self.controller.signals) {
                match signal {
                    ControllerSignal::Looped => self.looped(),
                    ControllerSignal::CrossfadeFinished => self.crossfade_finished(host),
                }
            }
            self.execute_actions(body, host);
        }
        body.set_speed_multiplier(1.0);
        self.animation_speed = 1.0;
        self.controller.speed = 1.0;
        self.dt = dt;
        if let Some(next) = self.state_update(body, facts, host) {
            self.change_state(body, facts, next, host);
        }
    }

    /// BaseState.UpdateActions: (IsPanicing | IsStumbling | IsFeared) &&
    /// !IsEntangled enters PanicState (`fear` already excludes entangled).
    fn wants_panic(facts: &CharacterFacts) -> bool {
        (facts.panicking || facts.stumbling || facts.fear.is_some()) && !facts.grip.entangled
    }

    /// Character.Update tail: dead timer and Move/Panic animation speed.
    pub fn finish(&mut self, body: &mut CharacterBody, facts: &CharacterFacts, dt: f32) {
        self.finish_dead(body, facts.dead, dt);
    }
    pub(crate) fn finish_dead(&mut self, body: &mut CharacterBody, dead: bool, dt: f32) {
        if dead {
            self.dead_timer -= dt;
        }
        if matches!(self.state, CharacterState::Move | CharacterState::Panic) {
            if body.max_speed > 0.0 {
                let v = [body.velocity[0], 0.0, body.velocity[2]];
                let ratio = (v[0] * v[0] + v[2] * v[2]).sqrt() / body.max_speed;
                if !ratio.is_nan() {
                    self.animation_speed *= ratio;
                }
            } else {
                self.animation_speed = 1.0;
            }
        }
        self.controller.speed = self.animation_speed;
    }

    /// Character.Update after UpdateStatusEffects (research/status-motion.md):
    /// outside Move/Panic a Cold status scales the animation speed by its
    /// slowdown; a positive Frozen magnitude (or bloating) stops animation,
    /// movement speed, turning and the requested movement.
    pub fn status_tail(&mut self, body: &mut CharacterBody, status: StatusMotion, bloating: bool) {
        // mZapTimer: AnimationController.Speed *= ZapModifier.
        if let Some(modifier) = status.zap {
            self.animation_speed *= modifier;
        }
        if !matches!(self.state, CharacterState::Move | CharacterState::Panic) {
            if let Some(slowdown) = status.cold {
                self.animation_speed *= slowdown;
            }
        }
        if status.frozen || bloating {
            self.animation_speed = 0.0;
            body.set_speed_multiplier(0.0);
            body.owner.turn_speed = 0.0;
            body.set_movement([0.0; 3]);
        }
        self.controller.speed = self.animation_speed;
    }

    /// AnimationAction.Execute over mCurrentActions with the controller's
    /// normalized time.
    fn execute_actions(&mut self, body: &mut CharacterBody, host: &mut dyn CharacterHost) {
        let duration = self.controller.clip_duration();
        let t = self.controller.time / duration;
        for i in 0..self.actions.len() {
            let (start, end) = (self.actions[i].start_time, self.actions[i].end_time);
            if t >= start && (!self.executed[i] || t <= end) {
                let first = !self.executed[i];
                let animation = execute_action(
                    &self.actions[i],
                    i,
                    first,
                    body,
                    self.controller.speed,
                    host,
                );
                if let Some(animation) = animation {
                    self.attack(animation, false);
                }
                self.executed[i] = true;
                self.dead_actions[i] = false;
            } else if (t >= end) & self.executed[i] & !self.dead_actions[i] {
                kill_action(&self.actions[i], i, host);
                self.dead_actions[i] = true;
            }
        }
    }
}

/// InternalExecute of the reconstructed action kinds. Footstep effects
/// (sEffectNames) still need a presentation host; PlayEffect has an avatar one.
fn execute_action(
    action: &AnimationAction,
    index: usize,
    first: bool,
    body: &mut CharacterBody,
    speed: f32,
    host: &mut dyn CharacterHost,
) -> Option<u16> {
    match &action.kind {
        ActionKind::SpecialAbility { weapon, ability } if first => {
            return host.special_action(*weapon, ability.as_ref())
        }
        ActionKind::Jump {
            elevation_deg,
            min_range,
            max_range,
        } if first => {
            host.jump_animation(body, elevation_deg.to_radians(), *min_range, *max_range);
        }
        ActionKind::Ethereal {
            ethereal,
            alpha,
            speed,
        } if first => {
            host.ethereal_action(*ethereal, *alpha, *speed);
        }
        ActionKind::Gunfire { weapon, accuracy } if first => host.execute_gun(*weapon, *accuracy),
        ActionKind::Block { weapon } => host.block_action(*weapon),
        ActionKind::Move { velocity } => {
            let o = body.orientation;
            body.set_additional_force([0, 1, 2].map(|i| {
                (velocity[0] * o.right[i] + velocity[1] * o.up[i] + velocity[2] * o.backward[i])
                    * speed
            }));
        }
        ActionKind::CastSpell { from_staff, source } => {
            if first && host.current_spell().is_none() {
                host.cast_spell(*from_staff, source.as_deref());
            }
        }
        ActionKind::DealDamage { weapon, target } => host.deal_damage(*weapon, *target, first),
        ActionKind::SpawnMissile {
            weapon,
            velocity,
            item_aligned,
        } if first => {
            let v = *velocity;
            let velocity = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2] > 1e-6).then_some(v);
            host.spawn_missile(*weapon, velocity, *item_aligned);
        }
        ActionKind::DetachItem { item, velocity } if first => host.detach_item(*item, *velocity),
        ActionKind::RemoveStatus { status } if first => {
            host.remove_status(status_mask(status).expect("invalid authored status enum"));
        }
        ActionKind::Immortal { collide } => host.immortal_action(*collide),
        ActionKind::Grip {
            grip_type,
            radius,
            break_free_tolerance,
            grip_attach,
            finish_on_grip,
            ..
        } => host.grip_request(crate::grip::GripRequest::Grip {
            grip_type: *grip_type,
            radius: *radius,
            tolerance: crate::grip::action_tolerance(*break_free_tolerance),
            uses_attach: !grip_attach.is_empty(),
            finish_on_grip: *finish_on_grip,
        }),
        ActionKind::DamageGrip {
            damage_to_owner,
            damages,
        } => host.grip_request(crate::grip::GripRequest::Damage {
            to_owner: *damage_to_owner,
            damages: damages
                .iter()
                .map(|d| magicka_sim::damage::Damage {
                    attack: d.attack_property as u16,
                    element: d.element as u32,
                    amount: d.amount,
                    magnitude: d.magnitude,
                })
                .collect(),
        }),
        ActionKind::ReleaseGrip => host.grip_request(crate::grip::GripRequest::Release),
        ActionKind::PlayEffect { .. } => host.play_animation_effect(index, action, first),
        ActionKind::PlaySound { sound, bank } if first => {
            host.play_animation_sound(*bank as u16, sound)
        }
        ActionKind::Footstep if first => {
            if let Some((cue, variables)) = crate::character_sounds::footstep(body) {
                host.play_footstep(cue, variables);
            }
        }
        _ => {}
    }
    None
}
fn kill_action(action: &AnimationAction, index: usize, host: &mut dyn CharacterHost) {
    match action.kind {
        ActionKind::Gunfire { weapon, .. } => host.stop_gun(weapon),
        ActionKind::Block { .. } => host.kill_block_action(),
        ActionKind::CastSpell { .. } => host.spell_animation_end(),
        ActionKind::DealDamage { weapon, .. } => host.stop_execute(weapon),
        ActionKind::Immortal { .. } => host.kill_immortal_action(),
        ActionKind::PlayEffect { .. } => host.stop_animation_effect(index),
        _ => {}
    }
}

/// World access for one character's AI step (borrowed from the SceneWorld).
pub struct ActorFrame<'a> {
    pub body: &'a mut CharacterBody,
    pub nav: Option<&'a NavMesh>,
    pub move_abilities: u8,
    pub triggers: &'a mut Vec<(i32, Option<u16>)>,
    pub sender: u16,
    pub entity_position: &'a dyn Fn(i32) -> Option<Vector>,
    pub locator: &'a dyn Fn(i32) -> Option<[f32; 16]>,
    pub closest_area_point: &'a dyn Fn(i32, Vector) -> Option<Vector>,
    /// Agent.GetAvoidance for NPCs (zero for avatars, which never ask).
    pub avoidance: &'a dyn Fn(Vector) -> Vector,
    pub in_cutscene: bool,
    /// PlayState play time (spell timestamps).
    pub play_time: f64,
}

/// AvatarAiHost over a character's motion and its frame.
pub struct ActorAi<'a, 'b> {
    pub motion: &'a mut CharacterMotion,
    pub frame: &'a mut ActorFrame<'b>,
    pub killed: Option<bool>,
    /// The NPC agent's combat state and world snapshot (None for avatars).
    pub combat: Option<CombatCtx<'a>>,
    /// AIStateMove group queries: the same-type agents and the elected
    /// leader's facts (empty / None for avatars).
    pub group: &'a [GroupCandidate],
    pub leader: Option<(u16, GroupLeader)>,
}

/// Agent combat access for one agent update.
pub struct CombatCtx<'a> {
    pub agent: &'a mut crate::npc_combat::CombatAgent,
    pub view: &'a crate::npc_combat::CombatView<'a>,
    /// Index of the owner in the view.
    pub owner: usize,
}

/// AttackOwner over the character body and motion.
struct AttackHost<'x, 'y> {
    dead: bool,
    motion: &'x mut CharacterMotion,
    frame: &'x mut ActorFrame<'y>,
    order: u8,
}
impl crate::npc_combat::AttackOwner for AttackHost<'_, '_> {
    fn set_movement(&mut self, movement: Vector) {
        self.frame.body.set_movement(movement);
    }
    fn set_desired_direction(&mut self, direction: Vector) {
        self.frame.body.set_desired_direction(direction);
    }
    fn attacking(&self) -> bool {
        self.motion.attacking
    }
    fn attack(&mut self, animation: u16, allow_rotate: bool) {
        self.motion.attack(animation, allow_rotate);
    }
    fn dashing(&self) -> bool {
        self.motion.dashing
    }
    fn dash(&mut self, animation: u16, next_dash_animation: u16) {
        self.motion.dash(animation, false);
        self.motion.next_dash_animation = next_dash_animation;
    }
    fn block(&mut self) {
        self.motion.block();
    }
    fn damage_gripped(&mut self, animation: u16) {
        self.motion.damage_gripped(animation);
    }
    fn jump(&mut self, delta: Vector, elevation: f32) {
        magicka_sim::jump::launch(
            self.frame.body,
            self.dead,
            delta,
            elevation,
            magicka_sim::character_physics::GRAVITY[1],
        );
    }
    fn avoidance_length(&self) -> f32 {
        let a = (self.frame.avoidance)(self.frame.body.position);
        length_sq(a).sqrt()
    }
    fn order(&self) -> u8 {
        self.order
    }
}

impl AvatarAiHost for ActorAi<'_, '_> {
    fn position(&self) -> Vector {
        self.frame.body.position
    }
    fn direction(&self) -> Vector {
        self.frame.body.direction()
    }
    fn touching_ground(&self) -> bool {
        self.frame.body.is_touching_ground()
    }
    fn set_movement(&mut self, movement: Vector) {
        self.frame.body.set_movement(movement);
    }
    fn set_desired_direction(&mut self, direction: Vector) {
        self.frame.body.set_desired_direction(direction);
    }
    fn find_path(&mut self, start: Vector, end: Vector) -> Vec<PathNode> {
        // Liquids are not composed yet: water triangles never count as ice.
        let abilities = self.frame.move_abilities;
        self.frame
            .nav
            .and_then(|nav| {
                nav.find_shortest_path(start, end, abilities, &|_| false)
                    .ok()
            })
            .map(|(_, path)| path)
            .unwrap_or_default()
    }
    fn execute_trigger(&mut self, trigger: i32) {
        self.frame.triggers.push((trigger, Some(self.frame.sender)));
    }
    fn go_to_animation(&mut self, animation: u16, blend: f32) {
        self.motion.go_to_animation(animation, blend);
    }
    fn set_special_idle(&mut self, animation: u16) {
        self.motion.special_idle = animation;
    }
    fn current_animation(&self) -> u16 {
        self.motion.controller.current_animation
    }
    fn animation_finished(&self) -> bool {
        self.motion.controller.has_finished()
    }
    fn entity_position(&self, unique_id: i32) -> Option<Vector> {
        (self.frame.entity_position)(unique_id)
    }
    fn locator(&self, id: i32) -> Option<[f32; 16]> {
        (self.frame.locator)(id)
    }
    fn closest_area_point(&self, id: i32, point: Vector) -> Option<Vector> {
        (self.frame.closest_area_point)(id, point)
    }
    fn kill(&mut self, remove: bool) {
        self.killed = Some(remove);
    }
    fn jump(&mut self, _direction: Vector, _elevation: f32) {
        // Character.Jump is not recovered yet.
    }
    fn avoidance(&self) -> Vector {
        (self.frame.avoidance)(self.frame.body.position)
    }
    fn nearest_nav_position(&self, point: Vector) -> Vector {
        self.frame
            .nav
            .and_then(|nav| nav.nearest(point, self.frame.move_abilities))
            .map_or(point, |(_, p, _)| p)
    }
    fn choose_and_add_target(&mut self) -> bool {
        let Some(c) = self.combat.as_mut() else {
            return false;
        };
        let gripping = c.view.combatants[c.owner].gripping;
        let (_, target, ability) = c.agent.choose_target(c.view, c.owner, gripping);
        match target {
            Some(t) => {
                c.agent.add_target(t, ability);
                true
            }
            None => false,
        }
    }
    fn attack_execute(&mut self, order: u8) -> crate::npc_combat::AttackStep {
        let Some(c) = self.combat.as_mut() else {
            return crate::npc_combat::AttackStep::Pop { release: true };
        };
        let mut host = AttackHost {
            dead: c.view.combatants[c.owner].dead,
            motion: self.motion,
            frame: self.frame,
            order,
        };
        c.agent.attack_execute(c.view, c.owner, &mut host)
    }
    fn release_target(&mut self) {
        if let Some(c) = self.combat.as_mut() {
            c.agent.release_target();
        }
    }
    fn stop_attacking(&mut self) {
        if self.combat.is_some() {
            self.motion.stop_attacking();
        }
    }
    fn current_target_position(&self) -> Option<Vector> {
        let c = self.combat.as_ref()?;
        c.view.get(c.agent.current_target()?).map(|t| t.position)
    }
    fn move_target_invalid(&self) -> bool {
        let Some(c) = self.combat.as_ref() else {
            return false;
        };
        let dead = c
            .agent
            .current_target()
            .and_then(|t| c.view.get(t))
            .is_none_or(|t| t.dead);
        dead || c.agent.next_ability.is_none()
    }
    fn current_target(&self) -> Option<u16> {
        self.combat.as_ref().and_then(|c| c.agent.current_target())
    }
    fn group_candidates(&self) -> &[GroupCandidate] {
        self.group
    }
    fn leader(&self, handle: u16) -> Option<GroupLeader> {
        self.leader
            .filter(|(h, _)| *h == handle)
            .map(|(_, facts)| facts)
    }
    fn nearest_default_nav_position(&self, point: Vector) -> Vector {
        self.frame
            .nav
            .and_then(|nav| nav.nearest(point, movement::DEFAULT))
            .map_or(point, |(_, p, _)| p)
    }
}

/// Enum.Parse(StatusEffects, ignoreCase:true), including comma-separated flags.
pub fn status_mask(value: &str) -> Result<u16, String> {
    if let Ok(number) = value.trim().parse::<i16>() {
        return Ok(number as u16);
    }
    value.split(',').try_fold(0, |mask, name| {
        let bit = match name.trim().to_ascii_lowercase().as_str() {
            "none" => 0,
            "burning" => 1,
            "wet" => 2,
            "frozen" => 4,
            "cold" => 8,
            "poisoned" => 16,
            "healing" | "life" => 32,
            "greased" => 64,
            "steamed" => 128,
            "bleeding" => 256,
            "numberoftypes" => 512,
            _ => return Err(format!("unknown StatusEffects flag {name}")),
        };
        Ok(mask | bit)
    })
}
