//! Scripted event playback shared by avatars and NPC agents: the AI-state
//! branch of Avatar.Update / Agent.Update with AIStateIdle (event execution)
//! and AIStateMove (navmesh path following). Agent mode adds the avoidance,
//! the Attack/Defend orders, and the Wander/Panic orders (AIStateWander and
//! AIStatePanic over `magicka_sim::agent_steering`) and AIStateMove's leader
//! election / FollowGroup following; the barrier and shield checks of
//! AIStateMove are incomplete. See research/avatar-events.md,
//! research/npc-combat.md and research/npc-agents.md.
use crate::ai_event::{AiEvent, LoopType};
use magicka_sim::{
    agent_steering::{
        elect_leader, follow_group, group_members, panic_enter_angle, panic_steer, wander_steer,
        GroupCandidate, GroupWeights, WanderClock, WanderInputs, LEADER_GOAL_RADIUS_SQ,
        LEADER_MAX_AGE,
    },
    impact_motion::Vector,
    navmesh::{movement, PathNode},
};

/// CLR `float.Epsilon`.
const CLR_FLOAT_EPSILON: f32 = f32::from_bits(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiState {
    Idle,
    Move,
    /// AIStateAttack (agents; see npc_combat).
    Attack,
    /// AIStateWander (agents with the Wander order).
    Wander,
    /// AIStatePanic (agents with the Panic order, or stumbling).
    Panic,
}

/// What the avatar's AI asks of its character and world.
pub trait AvatarAiHost {
    fn position(&self) -> Vector;
    /// Body orientation forward (Character.Direction).
    fn direction(&self) -> Vector;
    fn touching_ground(&self) -> bool;
    fn set_movement(&mut self, movement: Vector);
    fn set_desired_direction(&mut self, direction: Vector);
    /// NavMesh.FindShortestPath with the avatar's MoveAbilities.
    fn find_path(&mut self, start: Vector, end: Vector) -> Vec<PathNode>;
    /// GameScene.ExecuteTrigger(id, avatar, ignoreConditions: false).
    fn execute_trigger(&mut self, trigger: i32);
    fn go_to_animation(&mut self, animation: u16, blend: f32);
    fn set_special_idle(&mut self, animation: u16);
    fn current_animation(&self) -> u16;
    fn animation_finished(&self) -> bool;
    fn entity_position(&self, unique_id: i32) -> Option<Vector>;
    fn locator(&self, id: i32) -> Option<[f32; 16]>;
    /// Trigger-area Box.GetDistanceToPoint's closest point (Defend order).
    fn closest_area_point(&self, _id: i32, _point: Vector) -> Option<Vector> {
        None
    }
    /// KillEvent: Terminate when `remove`, else Kill.
    fn kill(&mut self, remove: bool);
    /// Character.Jump(direction, elevation).
    fn jump(&mut self, direction: Vector, elevation: f32);
    /// Agent.GetAvoidance (agents only).
    fn avoidance(&self) -> Vector {
        [0.0; 3]
    }
    /// NavMesh.GetNearestPosition with the owner's MoveAbilities.
    fn nearest_nav_position(&self, point: Vector) -> Vector {
        point
    }
    /// AIStateIdle Attack order: Agent.ChooseTarget, and AddTarget when one
    /// is found (returns true; the caller pushes AIStateAttack).
    fn choose_and_add_target(&mut self) -> bool {
        false
    }
    /// AIStateAttack.OnExecute.
    fn attack_execute(&mut self, _order: u8) -> crate::npc_combat::AttackStep {
        crate::npc_combat::AttackStep::Pop { release: true }
    }
    /// Agent.ReleaseTarget.
    fn release_target(&mut self) {}
    /// AIStateAttack.OnExit: NonPlayerCharacter.StopAttacking.
    fn stop_attacking(&mut self) {}
    /// Agent.CurrentTarget position: Move waypoints are offsets from it.
    fn current_target_position(&self) -> Option<Vector> {
        None
    }
    /// AIStateMove with a target: the target is dead, or the next ability
    /// is null or no longer useful.
    fn move_target_invalid(&self) -> bool {
        false
    }
    /// Agent.CurrentTarget handle (leader election compares targets).
    fn current_target(&self) -> Option<u16> {
        None
    }
    /// The live NPC agents of the owner's `Type` other than itself, as
    /// AIStateMove's EntityManager.GetEntities queries see them (agents
    /// only; distance is filtered by the state).
    fn group_candidates(&self) -> &[GroupCandidate] {
        &[]
    }
    /// The leader's facts (position, state, target, waypoint, direction)
    /// when it still exists.
    fn leader(&self, _handle: u16) -> Option<GroupLeader> {
        None
    }
    /// NavMesh.GetNearestPosition with MovementProperties.Default
    /// (FollowGroup's clamp).
    fn nearest_default_nav_position(&self, point: Vector) -> Vector {
        point
    }
}

/// Agent.Leader as the follower reads it each AIStateMove update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupLeader {
    pub position: Vector,
    pub direction: Vector,
    /// `Leader.CurrentState is AIStateMove`.
    pub in_move: bool,
    /// `Leader.WayPoint + Leader.CurrentTarget.Position` (the target part
    /// only when one exists).
    pub goal: Vector,
}

/// Agent.Order values (spawn_action::ORDERS).
pub const ORDER_ATTACK: u8 = 2;
pub const ORDER_DEFEND: u8 = 3;
pub const ORDER_WANDER: u8 = 5;
pub const ORDER_PANIC: u8 = 6;

/// Seeded xorshift in [0, 1); not the native System.Random sequence.
fn xorshift(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    ((*state >> 11) as f64 / (1u64 << 53) as f64) as f32
}

#[derive(Clone, Debug)]
pub struct AvatarAi {
    events: Option<Vec<AiEvent>>,
    pub current_event: usize,
    /// CurrentEventDelay; NaN waits for an event animation to finish.
    pub event_delay: f32,
    states: Vec<AiState>,
    pub waypoint: Vector,
    pub path: Vec<PathNode>,
    pub state_age: f32,
    /// NonPlayerCharacter Agent (vs Avatar) semantics.
    pub agent: bool,
    /// Agent.Order.
    pub order: u8,
    pub target_area: i32,
    /// Agent.WanderAngle (AIStateWander, AIStatePanic and FollowGroup).
    pub wander_angle: f32,
    /// Agent.WanderPause / WanderTimer / WanderPaused.
    pub wander: WanderClock,
    /// Agent.WanderSpeed (Spawn/GiveOrder `speed`, default 1).
    pub wander_speed: f32,
    /// Entity.Radius of the owner, refreshed before each agent update.
    pub radius: f32,
    /// Character.IsPanicing / IsStumbling / CharacterBody.RunBackward,
    /// refreshed by the owner before each agent update.
    pub panicking: bool,
    pub stumbling: bool,
    pub run_backward: bool,
    /// AIStatePanic.OnExit (Movement = 0) owed by a SetOrder pop, applied at
    /// the next agent update (INFERRED timing; the original zeroes it at
    /// once).
    pending_panic_exit: bool,
    /// The AIStateWander/AIStatePanic Random replacement (seeded by the owner).
    pub rng: u64,
    /// Agent.mLeader / mLeaderAge: the AIStateMove leader elected on entry
    /// (IsLeader while None); the age accumulates every agent update.
    pub leader: Option<u16>,
    pub leader_age: f32,
    /// Agent.AlertRadius and the template Group* weights, refreshed by the
    /// owner before each agent update.
    pub alert_radius: f32,
    pub group: GroupWeights,
}

impl Default for AvatarAi {
    fn default() -> Self {
        Self {
            events: None,
            current_event: 0,
            event_delay: 0.0,
            // Avatar.Initialize: mStates = [AIStateIdle].
            states: vec![AiState::Idle],
            waypoint: [0.0; 3],
            path: Vec::new(),
            state_age: 0.0,
            agent: false,
            order: 0,
            target_area: 0,
            wander_angle: 0.0,
            wander: WanderClock::default(),
            wander_speed: 1.0,
            radius: 0.0,
            panicking: false,
            stumbling: false,
            run_backward: false,
            pending_panic_exit: false,
            rng: 0x9E37_79B9_7F4A_7C15,
            leader: None,
            leader_age: 0.0,
            alert_radius: 0.0,
            group: GroupWeights::default(),
        }
    }
}

fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn flat(mut v: Vector) -> Vector {
    v[1] = 0.0;
    v
}
/// XNA Vector3.Normalize (zero becomes NaN, as in the original).
fn normalize(v: Vector) -> Vector {
    let inv = 1.0 / dot(v, v).sqrt();
    [v[0] * inv, v[1] * inv, v[2] * inv]
}
fn event_trigger(event: &AiEvent) -> i32 {
    // Every event struct starts with (Trigger, Delay); the original reads the
    // MoveEvent view of the union, which aliases each type's own Trigger.
    match event {
        AiEvent::Move(e) => e.trigger,
        AiEvent::Animation(e) => e.trigger,
        AiEvent::Face(e) => e.trigger,
        AiEvent::Kill(e) => e.trigger,
        AiEvent::Loop(e) => e.trigger,
    }
}

impl AvatarAi {
    pub fn events(&self) -> Option<&[AiEvent]> {
        self.events.as_deref()
    }
    pub fn state(&self) -> AiState {
        *self.states.last().expect("AI state stack is never empty")
    }
    /// Avatar.Events setter, AI part. Returns true when the caller must
    /// ChangeState(IdleState) and GoToAnimation(idle, 0.05) (null/empty list);
    /// the caller also ends the avatar's dialog.
    pub fn set_events(&mut self, events: Option<Vec<AiEvent>>) -> bool {
        let empty = events.as_ref().is_none_or(Vec::is_empty);
        self.events = events;
        self.current_event = 0;
        self.event_delay = 0.0;
        empty
    }
    /// `mEvents != null && CurrentEvent < mEvents.Length`: blocks player input.
    /// Agent.SetOrder event part: events and index reset, the delay only
    /// when events are given; a changed order pops back to idle.
    /// Returns how many AIStateAttack entries were popped: the owner then
    /// releases that many targets and stops attacking (OnExit).
    pub fn set_order(&mut self, order: u8, events: Option<Vec<AiEvent>>) -> usize {
        let mut attacks = 0;
        if self.order != order {
            while self.states.len() > 1 {
                if self.state() == AiState::Attack {
                    attacks += 1;
                }
                if self.state() == AiState::Panic {
                    self.pending_panic_exit = true;
                }
                self.pop_state_raw();
            }
        }
        self.order = order;
        if events.is_some() {
            self.event_delay = 0.0;
        }
        self.events = events;
        self.current_event = 0;
        attacks
    }
    /// NPC IsInAEvent / avatar input block.
    pub fn controls_avatar(&self) -> bool {
        self.events
            .as_ref()
            .is_some_and(|e| self.current_event < e.len())
    }

    fn push_state<H: AvatarAiHost>(&mut self, host: &mut H, state: AiState) {
        self.state_age = 0.0;
        if self.state() != state {
            // OnExit of the state being covered.
            if self.state() == AiState::Attack {
                host.stop_attacking();
            }
            self.states.push(state);
            match state {
                AiState::Move => self.move_enter(host),
                // AIStateWander.OnEnter.
                AiState::Wander => {
                    let r = self.random();
                    self.wander.enter(r);
                }
                // AIStatePanic.OnEnter.
                AiState::Panic => {
                    let r = self.random();
                    self.wander_angle = panic_enter_angle(r);
                }
                AiState::Idle | AiState::Attack => {}
            }
        }
    }
    fn random(&mut self) -> f32 {
        xorshift(&mut self.rng)
    }
    /// NonPlayerCharacter.Charm/EndCharm/Hypnotize: pop to AIStateIdle,
    /// releasing the target of each AIStateAttack first.
    pub fn pop_to_idle<H: AvatarAiHost>(&mut self, host: &mut H) {
        while self.states.len() > 1 {
            if self.state() == AiState::Attack {
                host.release_target();
            }
            self.pop_state(host);
        }
    }

    fn pop_state<H: AvatarAiHost>(&mut self, host: &mut H) {
        if self.states.len() > 1 {
            match self.state() {
                AiState::Attack => host.stop_attacking(),
                // AIStatePanic.OnExit.
                AiState::Panic => host.set_movement([0.0; 3]),
                _ => {}
            }
        }
        self.pop_state_raw();
    }
    fn pop_state_raw(&mut self) {
        self.state_age = 0.0;
        // AIStateMove.OnExit only clears the path for agents; Idle.OnEnter is
        // empty. The idle base state is never popped.
        if self.states.len() > 1 {
            if self.agent && self.state() == AiState::Move {
                self.path.clear();
            }
            self.states.pop();
        }
    }

    /// Agent.Update state part: age, then execute the top state with the
    /// time accumulated since the agent's last update.
    pub fn agent_update<H: AvatarAiHost>(&mut self, host: &mut H, dt: f32) {
        self.state_age += dt;
        // Agent.Update: mLeaderAge += mTimeSinceLastUpdate.
        self.leader_age += dt;
        if std::mem::take(&mut self.pending_panic_exit) {
            host.set_movement([0.0; 3]);
        }
        match self.state() {
            AiState::Idle => {
                // AIStateIdle agent prelude.
                if self.order != ORDER_DEFEND {
                    let a = host.avoidance();
                    if dot(a, a).sqrt() > 20.0 {
                        let p = host.position();
                        self.waypoint = [p[0] + a[0], p[1] + a[1], p[2] + a[2]];
                        self.push_state(host, AiState::Move);
                        return;
                    }
                }
                // Defend first returns to its area/locator, then chooses a
                // target just like Attack. Distance ignores the Y component.
                if self.order == ORDER_DEFEND {
                    let p = host.position();
                    let area = host.closest_area_point(self.target_area, p);
                    let locator = host
                        .locator(self.target_area)
                        .map(|m| [m[12], m[13], m[14]]);
                    for destination in [area, locator].into_iter().flatten() {
                        if dot(flat(sub(p, destination)), flat(sub(p, destination))) >= 1.0 {
                            self.waypoint = destination;
                            self.push_state(host, AiState::Move);
                            return;
                        }
                    }
                }
                if matches!(self.order, ORDER_ATTACK | ORDER_DEFEND) && host.choose_and_add_target()
                {
                    self.push_state(host, AiState::Attack);
                    return;
                }
                // Wander and Panic push their state and fall through to the
                // event branch (which ends with Movement = 0); the new state
                // runs from the next update.
                match self.order {
                    ORDER_WANDER => self.push_state(host, AiState::Wander),
                    ORDER_PANIC => self.push_state(host, AiState::Panic),
                    _ => {}
                }
                self.idle_execute(host, dt)
            }
            AiState::Move => self.move_execute(host, dt),
            AiState::Wander => self.wander_execute(host, dt),
            AiState::Panic => self.panic_execute(host, dt),
            AiState::Attack => match host.attack_execute(self.order) {
                crate::npc_combat::AttackStep::Stay => {}
                crate::npc_combat::AttackStep::Pop { release } => {
                    self.pop_state(host);
                    if release {
                        host.release_target();
                    }
                }
                crate::npc_combat::AttackStep::Move(offset) => {
                    self.waypoint = offset;
                    self.push_state(host, AiState::Move);
                }
            },
        }
    }

    /// Avatar.Update: run the current AI state while events are pending.
    pub fn update<H: AvatarAiHost>(&mut self, host: &mut H, dt: f32) {
        let Some(len) = self.events.as_ref().map(Vec::len) else {
            return;
        };
        if (self.current_event < len)
            | (self.event_delay >= CLR_FLOAT_EPSILON)
            | self.event_delay.is_nan()
        {
            match self.state() {
                AiState::Idle => self.idle_execute(host, dt),
                AiState::Move => self.move_execute(host, dt),
                // Avatars never enter AIStateAttack, Wander or Panic.
                AiState::Attack | AiState::Wander | AiState::Panic => {}
            }
            self.state_age += dt;
        }
    }

    /// AIStateIdle.OnExecute for a non-agent owner.
    fn idle_execute<H: AvatarAiHost>(&mut self, host: &mut H, dt: f32) {
        let Some(events) = self.events.clone() else {
            host.set_movement([0.0; 3]);
            return;
        };
        let len = events.len();
        if len > 0
            && ((self.current_event < len) | (self.event_delay >= 0.0) | self.event_delay.is_nan())
        {
            if self.event_delay < 0.0 {
                match events[self.current_event] {
                    AiEvent::Move(m) => {
                        let d = flat(sub(m.waypoint, host.position()));
                        if dot(d, d) < 0.25 {
                            if m.fixed_direction && dot(host.direction(), m.direction) < 0.99 {
                                host.set_desired_direction(m.direction);
                                return;
                            }
                            self.event_delay = m.delay;
                            if m.delay < CLR_FLOAT_EPSILON && m.trigger != 0 {
                                host.execute_trigger(m.trigger);
                            }
                            self.current_event += 1;
                            if self.next_is_immediate_move() {
                                return;
                            }
                        } else {
                            self.waypoint = m.waypoint;
                            self.push_state(host, AiState::Move);
                            return;
                        }
                    }
                    AiEvent::Animation(a) => {
                        if a.idle_animation != crate::animation_controller::ANIMATION_NONE {
                            host.set_special_idle(a.idle_animation);
                        }
                        host.go_to_animation(a.animation, a.blend_time);
                        self.event_delay = if a.delay < CLR_FLOAT_EPSILON {
                            f32::NAN
                        } else {
                            a.delay
                        };
                        self.current_event += 1;
                    }
                    AiEvent::Face(f) => {
                        let mut direction = f.direction;
                        if f.target_id != 0 {
                            let target = host.entity_position(f.target_id).or_else(|| {
                                host.locator(f.target_id).map(|m| [m[12], m[13], m[14]])
                            });
                            match target {
                                Some(t) => direction = normalize(flat(sub(t, host.position()))),
                                None => {
                                    self.event_delay = f.delay;
                                    self.current_event += 1;
                                    host.set_movement([0.0; 3]);
                                    return;
                                }
                            }
                        }
                        host.set_desired_direction(direction);
                        if dot(direction, host.direction()) < 0.99 {
                            return;
                        }
                        self.event_delay = f.delay;
                        self.current_event += 1;
                    }
                    AiEvent::Kill(k) => host.kill(k.remove),
                    AiEvent::Loop(l) => {
                        if l.kind == LoopType::Reverse {
                            if let Some(events) = self.events.as_mut() {
                                let n = events.len();
                                if n > 2 {
                                    for i in 0..(n - 2) / 2 {
                                        events.swap(i, n - 2 - i);
                                    }
                                }
                            }
                        }
                        self.current_event = 0;
                    }
                }
            } else if self.event_delay.is_nan()
                && (host.animation_finished()
                    || matches!(events.get(self.current_event.wrapping_sub(1)),
                        Some(AiEvent::Animation(a)) if host.current_animation() != a.animation))
            {
                self.event_delay = -1.0;
                if self.current_event > 0 {
                    let trigger = event_trigger(&events[self.current_event - 1]);
                    if trigger != 0 {
                        host.execute_trigger(trigger);
                    }
                }
            } else {
                self.event_delay -= dt;
                if (self.event_delay < 0.0) & (self.current_event > 0) {
                    let trigger = event_trigger(&events[self.current_event - 1]);
                    if trigger != 0 {
                        host.execute_trigger(trigger);
                    }
                }
                if self.next_is_immediate_move() {
                    return;
                }
            }
        }
        host.set_movement([0.0; 3]);
    }

    /// AIStateWander.OnExecute: pop when airborne or the order changed; else
    /// tick the walk/rest clock, drift the wander angle and steer through the
    /// navmesh and the target trigger area.
    fn wander_execute<H: AvatarAiHost>(&mut self, host: &mut H, dt: f32) {
        if !host.touching_ground() || self.order != ORDER_WANDER {
            self.pop_state(host);
            return;
        }
        let mut rng = self.rng;
        let speed = self
            .wander
            .tick(dt, self.wander_speed, || xorshift(&mut rng));
        self.rng = rng;
        let inputs = WanderInputs {
            position: host.position(),
            direction: host.direction(),
            radius: self.radius,
            dt,
        };
        let r = self.random();
        let area = self.target_area;
        let (angle, movement) = wander_steer(
            self.wander_angle,
            inputs,
            r,
            |p| host.nearest_nav_position(p),
            |p| host.closest_area_point(area, p),
        );
        self.wander_angle = angle;
        host.set_movement(movement.map(|v| v * speed));
    }

    /// AIStatePanic.OnExecute: pop once neither panicking nor stumbling;
    /// else drift the wander angle and run (backwards while stumbling).
    fn panic_execute<H: AvatarAiHost>(&mut self, host: &mut H, dt: f32) {
        if !self.panicking && !self.stumbling {
            self.pop_state(host);
            return;
        }
        let r = self.random();
        let (angle, movement) = panic_steer(
            self.wander_angle,
            host.direction(),
            self.run_backward,
            r,
            dt,
        );
        self.wander_angle = angle;
        host.set_movement(movement);
    }

    fn next_is_immediate_move(&self) -> bool {
        self.events.as_ref().is_some_and(|e| {
            self.current_event < e.len()
                && matches!(e[self.current_event], AiEvent::Move(_))
                && self.event_delay <= 0.0
        })
    }

    /// Agent.IsLeader: no leader was elected on the last AIStateMove entry.
    pub fn is_leader(&self) -> bool {
        self.leader.is_none()
    }

    /// AIStateMove.OnEnter: an agent first elects a leader among the
    /// same-type NPCs (Leader = result, LeaderAge = 0); only a leader (or a
    /// non-agent) plans a path from the current position.
    fn move_enter<H: AvatarAiHost>(&mut self, host: &mut H) {
        self.path.clear();
        if self.agent {
            self.leader = elect_leader(
                host.position(),
                host.direction(),
                self.alert_radius,
                host.current_target(),
                self.waypoint,
                host.group_candidates(),
            );
            self.leader_age = 0.0;
            if !self.is_leader() {
                return;
            }
        }
        let goal = self.goal(host);
        self.path = host.find_path(host.position(), goal);
    }
    /// WayPoint, offset by the agent's current target position.
    fn goal<H: AvatarAiHost>(&self, host: &H) -> Vector {
        match host.current_target_position() {
            Some(t) => [
                self.waypoint[0] + t[0],
                self.waypoint[1] + t[1],
                self.waypoint[2] + t[2],
            ],
            None => self.waypoint,
        }
    }

    /// AIStateMove.OnExecute: path following for leaders and non-agents,
    /// FollowGroup for agents with a leader.
    fn move_execute<H: AvatarAiHost>(&mut self, host: &mut H, dt: f32) {
        if !host.touching_ground() {
            self.pop_state(host);
            return;
        }
        let limit = match self.path.first() {
            Some(n) if n.properties & movement::DYNAMIC == movement::DYNAMIC => 0.333,
            _ => 1.0,
        };
        if self.state_age >= limit {
            self.pop_state(host);
            return;
        }
        if host.current_target_position().is_some() && host.move_target_invalid() {
            self.pop_state(host);
            return;
        }
        let position = flat(host.position());
        let goal = self.goal(host);
        let mut waypoint = flat(goal);
        let d = sub(position, waypoint);
        let mut num = dot(d, d);
        if num <= 0.25 {
            self.pop_state(host);
            return;
        }
        if self.path.is_empty() && num > 9.0 && (!self.agent || self.is_leader()) {
            self.pop_state(host);
            return;
        }
        let mut direction;
        if self.agent && !self.is_leader() {
            // Follower: the leader must still be in AIStateMove, young
            // (LeaderAge <= 1) and away from its goal; else pop. A leader
            // that no longer exists counts as having left the state
            // (INFERRED: the original dereferences it).
            let Some(leader) = self.leader.and_then(|h| host.leader(h)) else {
                self.pop_state(host);
                return;
            };
            if !leader.in_move {
                self.pop_state(host);
                return;
            }
            let members = group_members(
                host.position(),
                self.alert_radius,
                host.current_target(),
                waypoint,
                host.group_candidates(),
            );
            if self.leader_age > LEADER_MAX_AGE {
                self.pop_state(host);
                return;
            }
            let d = sub(leader.position, leader.goal);
            if dot(d, d) <= LEADER_GOAL_RADIUS_SQ {
                self.pop_state(host);
                return;
            }
            let p = host.position();
            let mut rng = self.rng;
            let r = xorshift(&mut rng);
            let (angle, dir) = follow_group(
                p,
                host.direction(),
                leader.direction,
                &members,
                &self.group,
                self.wander_angle,
                dt,
                |count| ((xorshift(&mut rng) * count as f32) as usize).min(count - 1),
                r,
            );
            self.rng = rng;
            self.wander_angle = angle;
            let nearest =
                host.nearest_default_nav_position([p[0] + dir[0], p[1] + dir[1], p[2] + dir[2]]);
            direction = sub(nearest, p);
        } else if !self.path.is_empty() {
            let mut val = if self.path.len() <= 1 {
                waypoint
            } else {
                flat(self.path[1].position)
            };
            waypoint = flat(self.path[0].position);
            let dist = |a: Vector, b: Vector| {
                let d = sub(a, b);
                dot(d, d)
            };
            num = dist(position, waypoint);
            let mut num2 = dist(position, val);
            let mut num3 = dist(waypoint, val);
            while !self.path.is_empty() && (num <= 0.25 || num2 - num3 < 0.0) {
                self.path.remove(0);
                if self.path.len() > 1 {
                    waypoint = val;
                    val = flat(self.path[1].position);
                } else {
                    val = flat(goal);
                    waypoint = val;
                }
                num = dist(position, waypoint);
                num2 = dist(position, val);
                num3 = dist(waypoint, val);
            }
            direction = sub(waypoint, position);
            if self
                .path
                .first()
                .is_some_and(|n| n.properties == movement::JUMP)
            {
                host.jump(direction, std::f32::consts::PI / 3.0);
                return;
            }
        } else {
            direction = sub(waypoint, position);
        }
        if !self.agent || self.is_leader() {
            direction = normalize(direction);
            direction = direction.map(|v| v * 0.9);
        }
        if let Some(AiEvent::Move(m)) = self.events.as_ref().and_then(|e| e.get(self.current_event))
        {
            direction = direction.map(|v| v * m.speed);
        }
        if self.agent {
            let mut a = host.avoidance();
            let len_sq = dot(a, a);
            if len_sq > 1e-6 {
                let unit = a.map(|v| v / len_sq.sqrt());
                let weight = (dot(direction, unit) + 0.5).clamp(0.0, 1.0);
                a = a.map(|v| v * weight);
                direction = [
                    direction[0] + a[0],
                    direction[1] + a[1],
                    direction[2] + a[2],
                ];
                let p = host.position();
                let target = [
                    p[0] + direction[0],
                    p[1] + direction[1],
                    p[2] + direction[2],
                ];
                let nearest = host.nearest_nav_position(target);
                direction = sub(nearest, p);
                direction[1] = 0.0;
            }
        }
        host.set_movement(direction);
    }
}
