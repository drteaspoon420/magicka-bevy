//! NPC agent steering orders recovered from Magicka.AI.AgentStates:
//! AIStateWander (wander order), AIStatePanic (panic order / stumbling),
//! AIStateMove.FollowGroup (flocking) and the Entity.GetDanger values that
//! feed Agent.GetAvoidance, plus the Character PanicState movement for the
//! panic and stumble branches. Pure and deterministic: callers supply the
//! random draws. See research/npc-agents.md (2026-10-30 section).
use crate::math_approximation::fast_sin_cos;

pub type Vector = [f32; 3];

/// CLR `float.Epsilon`.
pub const CLR_FLOAT_EPSILON: f32 = f32::from_bits(1);
/// AIStateWander.OnEnter: WanderPause = 10 + 10·random.
pub const WANDER_PAUSE_BASE: f32 = 10.0;
pub const WANDER_PAUSE_RANDOM: f32 = 10.0;
/// AIStateWander.OnExecute: the pause lasts 2 + 3·random seconds.
pub const WANDER_REST_BASE: f32 = 2.0;
pub const WANDER_REST_RANDOM: f32 = 3.0;
/// AIStateWander/AIStatePanic/PanicState: WanderAngle drifts by
/// (random − 0.5) · 60 · dt radians per update.
pub const WANDER_TURN_RATE: f32 = 60.0;
/// AIStateMove.FollowGroup: (random − 0.5) · 30 · dt.
pub const GROUP_TURN_RATE: f32 = 30.0;
/// AIStateWander: Direction · 4 plus the wander unit vector.
pub const WANDER_FORWARD_WEIGHT: f32 = 4.0;
/// AIStateWander: Direction · 1.5 after reversing at the target area edge.
pub const WANDER_AREA_REVERSE_WEIGHT: f32 = 1.5;
/// AIStatePanic / FollowGroup: Direction · ±1.25 plus the wander vector.
pub const PANIC_FORWARD_WEIGHT: f32 = 1.25;
/// PanicState.Update: Direction · 4 plus the character wander vector.
pub const PANIC_STATE_FORWARD_WEIGHT: f32 = 4.0;
/// PanicState: SpeedMultiplier *= 0.8 while IsPanicing.
pub const PANIC_SPEED: f32 = 0.8;
/// PanicState: SpeedMultiplier *= 0.6 while stumbling.
pub const STUMBLE_SPEED: f32 = 0.6;
/// PanicCastState: SpeedMultiplier *= 0.666 while stumbling.
pub const STUMBLE_CAST_SPEED: f32 = 0.666;
/// Character.OrderPanic: mPanic = 10.
pub const ORDER_PANIC_MAGNITUDE: f32 = 10.0;
/// AIStateMove.FollowGroup samples at most 8 distinct group members.
pub const GROUP_SAMPLE: usize = 8;
/// AIStateMove.OnEnter / OnExecute group query radius: AlertRadius · 0.333.
pub const GROUP_RADIUS_FACTOR: f32 = 0.333;
/// MissileEntity.Initialize: mDanger = 2 until a definition sets Danger.
pub const MISSILE_DEFAULT_DANGER: f32 = 2.0;
/// AIStateMove: a leader candidate without a target must have its WayPoint
/// within 1 m (squared) of the follower's; the OnExecute member filter uses
/// the same bound on the flattened waypoints.
pub const GROUP_WAYPOINT_RADIUS_SQ: f32 = 1.0;
/// AIStateMove.OnExecute: a follower pops once Agent.LeaderAge exceeds 1 s.
pub const LEADER_MAX_AGE: f32 = 1.0;
/// AIStateMove.OnExecute: a follower pops once its leader is within 0.5 m
/// (squared 0.25) of the leader's goal.
pub const LEADER_GOAL_RADIUS_SQ: f32 = 0.25;

fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(v: Vector, s: f32) -> Vector {
    [v[0] * s, v[1] * s, v[2] * s]
}
fn length_squared(v: Vector) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}
/// XNA Vector3.Normalize: a zero vector becomes NaN, as in the original.
fn normalize(v: Vector) -> Vector {
    scale(v, 1.0 / length_squared(v).sqrt())
}

/// MathHelper.WrapAngle: reduce to (−π, π].
pub fn wrap_angle(angle: f32) -> f32 {
    let two_pi = std::f32::consts::PI * 2.0;
    // IEEERemainder(angle, 2π): angle − 2π · round-half-even(angle / 2π).
    let q = angle / two_pi;
    let mut n = q.round();
    if (q - q.trunc()).abs() == 0.5 && n % 2.0 != 0.0 {
        n -= q.signum();
    }
    let mut a = angle - n * two_pi;
    if a <= -std::f32::consts::PI {
        a += two_pi;
    } else if a > std::f32::consts::PI {
        a -= two_pi;
    }
    a
}

/// The wander unit vector of an angle: FastSinCos(angle, out Z, out X).
pub fn wander_vector(angle: f32) -> Vector {
    let (sin, cos) = fast_sin_cos(angle);
    [cos, 0.0, sin]
}

/// WanderAngle drift: WrapAngle(angle + (random − 0.5) · rate · dt).
pub fn drift_angle(angle: f32, random: f32, rate: f32, dt: f32) -> f32 {
    wrap_angle(angle + (random - 0.5) * rate * dt)
}

/// Agent.Initialize: WanderAngle = random · 2π.
pub fn initial_wander_angle(random: f32) -> f32 {
    random * std::f32::consts::PI * 2.0
}

/// Agent.WanderPause / WanderTimer / WanderPaused.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WanderClock {
    pub pause: f32,
    pub timer: f32,
    pub paused: bool,
}

impl WanderClock {
    /// AIStateWander.OnEnter.
    pub fn enter(&mut self, random: f32) {
        self.pause = WANDER_PAUSE_BASE + random * WANDER_PAUSE_RANDOM;
        self.timer = 0.0;
        self.paused = false;
    }

    /// AIStateWander.OnExecute timer part: returns the movement scale (the
    /// wander speed while walking, 0 while resting). `random` is drawn only
    /// when a rest starts; pass the agent's next draw lazily.
    pub fn tick(&mut self, dt: f32, wander_speed: f32, random: impl FnOnce() -> f32) -> f32 {
        if self.paused {
            self.timer -= dt;
            if self.timer <= 0.0 {
                self.paused = false;
            }
            0.0
        } else {
            self.timer += dt;
            if self.timer >= self.pause {
                self.timer = WANDER_REST_BASE + WANDER_REST_RANDOM * random();
                self.paused = true;
                0.0
            } else {
                wander_speed
            }
        }
    }
}

/// What AIStateWander.OnExecute reads from the owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WanderInputs {
    pub position: Vector,
    pub direction: Vector,
    /// Entity.Radius.
    pub radius: f32,
    pub dt: f32,
}

/// AIStateWander.OnExecute steering (after the ground/order pop test and the
/// WanderClock tick). `nearest` is NavMesh.GetNearestPosition with the
/// owner's move abilities; `area_closest` is the target trigger area box's
/// GetDistanceToPoint closest point (None when TryGetTriggerArea fails).
/// Returns the new wander angle and the normalized movement before the
/// speed scale.
pub fn wander_steer(
    angle: f32,
    inputs: WanderInputs,
    random: f32,
    nearest: impl Fn(Vector) -> Vector,
    area_closest: impl Fn(Vector) -> Option<Vector>,
) -> (f32, Vector) {
    let mut angle = drift_angle(angle, random, WANDER_TURN_RATE, inputs.dt);
    let radius = inputs.radius;
    let mut position = inputs.position;
    position[1] = 0.0;
    let mut movement = add(
        scale(inputs.direction, WANDER_FORWARD_WEIGHT),
        wander_vector(angle),
    );
    let mut point = nearest(add(position, movement));
    point[1] = 0.0;
    if length_squared(sub(position, point)) > radius * radius {
        movement = sub(point, position);
    }
    if let Some(mut edge) = area_closest(add(position, movement)) {
        edge[1] = 0.0;
        if length_squared(sub(position, edge)) > radius * radius {
            movement = sub(edge, position);
        } else {
            angle = wrap_angle(angle + std::f32::consts::PI);
            movement = add(
                scale(inputs.direction, WANDER_AREA_REVERSE_WEIGHT),
                wander_vector(angle),
            );
        }
    }
    (angle, normalize(movement))
}

/// AIStatePanic.OnEnter: WanderAngle = (random − 0.5) · 2π.
pub fn panic_enter_angle(random: f32) -> f32 {
    (random - 0.5) * std::f32::consts::PI * 2.0
}

/// AIStatePanic.OnExecute steering: the drifted angle and the normalized
/// movement (Direction · ±1.25 plus the wander vector; backwards while
/// CharacterBody.RunBackward).
pub fn panic_steer(
    angle: f32,
    direction: Vector,
    run_backward: bool,
    random: f32,
    dt: f32,
) -> (f32, Vector) {
    let angle = drift_angle(angle, random, WANDER_TURN_RATE, dt);
    let weight = if run_backward {
        -PANIC_FORWARD_WEIGHT
    } else {
        PANIC_FORWARD_WEIGHT
    };
    (
        angle,
        normalize(add(scale(direction, weight), wander_vector(angle))),
    )
}

/// Character.IsPanicing: mPanic − mMaxPanic > float.Epsilon.
pub fn is_panicing(panic: f32, max_panic: f32) -> bool {
    panic - max_panic > CLR_FLOAT_EPSILON
}

/// PanicState.Update panic branch: the character wander walk
/// normalize(Direction · 4 + wander) blended with the existing Movement by
/// Character.Panic (the template MaxPanic): normalize(walk · p + movement ·
/// (1 − p)); the walk alone when the movement is (near) zero. The caller
/// multiplies SpeedMultiplier by PANIC_SPEED.
pub fn panic_state_movement(
    angle: f32,
    direction: Vector,
    movement: Vector,
    max_panic: f32,
) -> Vector {
    let walk = normalize(add(
        scale(direction, PANIC_STATE_FORWARD_WEIGHT),
        wander_vector(angle),
    ));
    if length_squared(movement) > 1e-6 {
        normalize(add(
            scale(walk, max_panic),
            scale(movement, 1.0 - max_panic),
        ))
    } else {
        walk
    }
}

/// PanicState.Update stumble branch (gripped with GripType.Ride, neither
/// panicking nor feared): normalize(normalize(Direction · 4 + wander) −
/// Direction · 4). The caller multiplies SpeedMultiplier by STUMBLE_SPEED
/// and runs backwards (RunBackward set on entry).
pub fn stumble_state_movement(angle: f32, direction: Vector) -> Vector {
    let forward = scale(direction, PANIC_STATE_FORWARD_WEIGHT);
    let walk = normalize(add(forward, wander_vector(angle)));
    normalize(add(walk, scale(forward, -1.0)))
}

/// Character.IsStumbling: a gripper holds this character with GripType.Ride.
pub fn is_stumbling(gripped: bool, grip_type_ride: bool) -> bool {
    gripped && grip_type_ride
}

/// The template flocking weights (CharacterTemplate Group* fields).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GroupWeights {
    pub chase: f32,
    pub separation: f32,
    pub cohesion: f32,
    pub alignment: f32,
    pub wander: f32,
}

/// AIStateMove.FollowGroup. `group` holds the positions of the same-type
/// NPCs (within AlertRadius · 0.333, same target / waypoint) and
/// `leader_direction` the leader's Direction. `pick(count)` is
/// Random.Next(count): up to 8 distinct members are sampled (re-drawing
/// duplicates, as the original does). Returns the drifted wander angle and
/// the desired movement before NavMesh clamping.
pub fn follow_group(
    position: Vector,
    direction: Vector,
    leader_direction: Vector,
    group: &[Vector],
    weights: &GroupWeights,
    angle: f32,
    dt: f32,
    mut pick: impl FnMut(usize) -> usize,
    random: f32,
) -> (f32, Vector) {
    let alignment = scale(leader_direction, weights.alignment);
    let count = group.len().min(GROUP_SAMPLE);
    let mut chosen: Vec<usize> = Vec::with_capacity(count);
    while chosen.len() < count {
        let b = pick(group.len());
        if !chosen.contains(&b) {
            chosen.push(b);
        }
    }
    let (mut separation, mut cohesion) = ([0.0; 3], [0.0; 3]);
    for &index in &chosen {
        let delta = sub(group[index], position);
        let len = length_squared(delta).sqrt();
        separation = add(separation, scale(delta, weights.separation / (0.0 - len)));
        cohesion = add(cohesion, scale(delta, weights.cohesion));
    }
    let angle = drift_angle(angle, random, GROUP_TURN_RATE, dt);
    let wander = scale(
        normalize(add(
            scale(direction, PANIC_FORWARD_WEIGHT),
            wander_vector(angle),
        )),
        weights.wander,
    );
    let mut out = add(add(alignment, separation), add(cohesion, wander));
    let len_sq = length_squared(out);
    if len_sq > 1.0 {
        out = scale(out, 1.0 / len_sq.sqrt());
    }
    (angle, out)
}

/// A same-type NPC as AIStateMove's leader election and member filter see
/// it (the host pre-filters `Type`; everything else is decided here).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupCandidate {
    pub handle: u16,
    pub position: Vector,
    pub radius: f32,
    /// `AI.CurrentState == AIStateMove`.
    pub in_move: bool,
    /// `AI.IsLeader` (Agent.mLeader == null).
    pub is_leader: bool,
    /// `AI.CurrentTarget` handle (None when null).
    pub target: Option<u16>,
    /// `AI.WayPoint` (an offset from the target when one exists).
    pub waypoint: Vector,
}

/// EntityManager.GetEntities(position, radius) as the rewrite's agent
/// queries apply it: within `radius` plus the entity's own radius.
pub fn within_query(position: Vector, radius: f32, other: Vector, other_radius: f32) -> bool {
    let reach = radius + other_radius;
    length_squared(sub(other, position)) <= reach * reach
}

/// AIStateMove.OnEnter leader election for an agent: among the same-type
/// NPCs within AlertRadius · 0.333 that are in AIStateMove, leaders and
/// share the agent's CurrentTarget, the last one in front of the agent
/// (dot(Direction, unit delta) > 0) leads; without a target its WayPoint
/// must also lie within 1 m of the agent's. Returns the leader's handle.
pub fn elect_leader(
    position: Vector,
    direction: Vector,
    alert_radius: f32,
    target: Option<u16>,
    waypoint: Vector,
    candidates: &[GroupCandidate],
) -> Option<u16> {
    let radius = alert_radius * GROUP_RADIUS_FACTOR;
    let mut leader = None;
    for c in candidates {
        if !within_query(position, radius, c.position, c.radius)
            || !c.in_move
            || !c.is_leader
            || c.target != target
        {
            continue;
        }
        let to = normalize(sub(c.position, position));
        let facing = direction[0] * to[0] + direction[1] * to[1] + direction[2] * to[2];
        if !(facing > 0.0) {
            continue;
        }
        if target.is_some() || length_squared(sub(waypoint, c.waypoint)) <= GROUP_WAYPOINT_RADIUS_SQ
        {
            leader = Some(c.handle);
        }
    }
    leader
}

/// AIStateMove.OnExecute follower member filter: the same-type NPCs within
/// AlertRadius · 0.333 with the same CurrentTarget; without a target their
/// flattened WayPoint must lie within 1 m of the follower's flattened goal.
/// Returns the member positions FollowGroup samples from.
pub fn group_members(
    position: Vector,
    alert_radius: f32,
    target: Option<u16>,
    flat_goal: Vector,
    candidates: &[GroupCandidate],
) -> Vec<Vector> {
    let radius = alert_radius * GROUP_RADIUS_FACTOR;
    candidates
        .iter()
        .filter(|c| within_query(position, radius, c.position, c.radius) && c.target == target)
        .filter(|c| {
            target.is_some() || {
                let w = [c.waypoint[0], 0.0, c.waypoint[2]];
                length_squared(sub(w, flat_goal)) <= GROUP_WAYPOINT_RADIUS_SQ
            }
        })
        .map(|c| c.position)
        .collect()
}

/// Agent.GetAvoidance sources other than plain characters: the
/// Entity.GetDanger overrides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DangerSource {
    /// Character.GetDanger: SpellQueue.Count.
    Character { spell_queue: usize },
    /// Avatar.GetDanger: 10 in ChargeState, else the Character value.
    Avatar { charging: bool, spell_queue: usize },
    /// SpellMine.GetDanger: 1 while armed, 10000 once the detonation timer
    /// runs.
    SpellMine { detonating: bool },
    /// MissileEntity.GetDanger: mDanger (2 by default; definitions set it).
    Missile { danger: f32 },
    /// TornadoEntity.GetDanger: 10 · alpha.
    Tornado { alpha: f32 },
    /// Barrier.GetDanger: the damage's total magnitude.
    Barrier { damage_magnitude: f32 },
    /// Grease.GreaseField.GetDanger: 0.5 unlit, temperature · 10 burning.
    Grease { burning: bool, temperature: f32 },
    /// VortexEntity.GetDanger: mass · 20.
    Vortex { mass: f32 },
    /// SummonDeath.MagickDeath.GetDanger: 1000.
    MagickDeath,
    /// ElementalEgg.GetDanger and the Entity default: 0.
    Harmless,
}

impl DangerSource {
    pub fn danger(&self) -> f32 {
        match *self {
            DangerSource::Character { spell_queue } => spell_queue as f32,
            DangerSource::Avatar {
                charging,
                spell_queue,
            } => {
                if charging {
                    10.0
                } else {
                    spell_queue as f32
                }
            }
            DangerSource::SpellMine { detonating } => {
                if detonating {
                    10000.0
                } else {
                    1.0
                }
            }
            DangerSource::Missile { danger } => danger,
            DangerSource::Tornado { alpha } => 10.0 * alpha,
            DangerSource::Barrier { damage_magnitude } => damage_magnitude,
            DangerSource::Grease {
                burning,
                temperature,
            } => {
                if burning {
                    temperature * 10.0
                } else {
                    0.5
                }
            }
            DangerSource::Vortex { mass } => mass * 20.0,
            DangerSource::MagickDeath => 1000.0,
            DangerSource::Harmless => 0.0,
        }
    }
}
