//! `Magicka.GameLogic.Entities.Bosses.Fafnir` (Ruins ru_s6BF) as a
//! deterministic state machine: sleep/intro, the weighted attack decision,
//! tail, wings, confuse, fireball, high/low fire lance, ceiling debris,
//! earthquakes and defeat, with neck aiming and bone-bound damage zones.
//! Each update returns ordered [`FafnirCommand`]s for the scene host. See
//! research/boss-battles.md.
use crate::animation_controller::AnimationController;
use crate::boss_machine::{quat_mul, rotation_y_pi};
use crate::character_pose::{CharacterSkeleton, Pose};
use magicka_content::skinned::{self, Mat, Resolved};
use magicka_sim::boss_status::{BossEvent, BossStatus};
use magicka_sim::damage::{attack, Damage};
use magicka_sim::spells::Elements as E;

pub const MESH_MODEL: &str = "Models/Bosses/Fafnir/Fafnir_mesh";
pub const ANIMATION_MODEL: &str = "Models/Bosses/Fafnir/Fafnir_animation";
/// Content casing (the viewer loads gltf/<path>.glb verbatim).
pub const DEBRIS_MODEL: &str = "Models/Bosses/fafnir/Ceiling_Debri";
pub const MAX_HIT_POINTS: f32 = 35000.0;
const DIVISOR: f32 = 2.857_142_9E-5;
const NECK_BONES: usize = 6;
const TAIL_BONES: usize = 11;

/// mAnimationClips indices (Fafnir.Animations).
pub mod clip {
    pub const CEILING: usize = 1;
    pub const CHARM: usize = 2;
    pub const DEFEATED: usize = 3;
    pub const FIREBALL: usize = 4;
    pub const FIRE_HIGH: usize = 5;
    pub const FIRE_LOW: usize = 6;
    pub const IDLE: usize = 7;
    pub const INTRO: usize = 8;
    pub const SLEEP: usize = 9;
    pub const TAILWHIP: usize = 10;
    pub const WINGS: usize = 11;
    pub const NAMES: [&str; 12] = [
        "",
        "ceiling",
        "charm",
        "defeated",
        "fireball",
        "fire_high",
        "fire_low",
        "idle",
        "intro",
        "sleep",
        "tailwhip",
        "wings",
    ];
}

/// ANIMATION_TIMES (normalized clip windows).
fn times(clip: usize) -> &'static [f32] {
    const T: [&[f32]; 12] = [
        &[0.0, 0.0],
        &[1.0 / 15.0, 14.0 / 15.0],
        &[13.0 / 90.0, 8.0 / 9.0],
        &[18.0 / 35.0],
        &[25.0 / 36.0, 1.0],
        &[19.0 / 45.0, 37.0 / 45.0],
        &[0.3125, 0.75],
        &[],
        &[7.0 / 15.0],
        &[],
        &[15.0 / 32.0, 1.0],
        &[0.47, 0.51, 0.67, 0.67],
    ];
    T[clip]
}

pub const SOUNDS: [&str; 6] = [
    "boss_fafnir_attack",
    "boss_fafnir_breakfloor",
    "boss_fafnir_confuse",
    "boss_fafnir_death",
    "boss_fafnir_deathray",
    "boss_fafnir_pain",
];

/// hole_debris0..25 without 13; effect_lava likewise.
pub fn level_part(i: usize) -> String {
    let n = if i >= 13 { i + 1 } else { i };
    format!("hole_debris{n}")
}
pub fn lava_effect(i: usize) -> String {
    let n = if i >= 13 { i + 1 } else { i };
    format!("effect_lava{n}")
}
const FLOOR_GROUPS: [&[usize]; 4] = [
    &[2, 6, 10, 14, 19, 23],
    &[0, 5, 9, 13, 18, 20, 15],
    &[3, 4, 7, 12, 16, 22],
    &[1, 11, 8, 24, 17, 21],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Sleep,
    Intro,
    Idle,
    Decision,
    Tail,
    Wing,
    Confuse,
    Fireball,
    LanceHigh,
    LanceLow,
    Ceiling,
    EarthQuake,
    Defeated,
}

/// A living avatar as Fafnir sees it.
#[derive(Clone, Copy, Debug)]
pub struct Avatar {
    pub handle: u16,
    pub position: [f32; 3],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FafnirInputs {
    pub fight_started: bool,
}

/// Host operations in native order.
#[derive(Clone, Debug, PartialEq)]
pub enum FafnirCommand {
    Trigger(&'static str),
    /// Character bank cue at the head (or global when None).
    Cue {
        name: &'static str,
        position: Option<[f32; 3]>,
    },
    SpellCue {
        name: &'static str,
    },
    Effect {
        name: &'static str,
        position: [f32; 3],
        direction: [f32; 3],
    },
    /// Push every damageable within 30 m behind the spine base (dot with
    /// Backward > 0.5) not on the hit list, at position − 2 Z.
    WingPush {
        spine_base: [f32; 3],
    },
    /// Tail knockback against characters touching a tail segment.
    TailSweep {
        segments: Vec<([f32; 3], [f32; 3], f32)>,
    },
    CircleDamage {
        position: [f32; 3],
        radius: f32,
        damages: [Damage; 3],
    },
    ArcDamage {
        position: [f32; 3],
        direction: [f32; 3],
        range: f32,
        angle: f32,
        damages: [Damage; 3],
    },
    Fireball {
        position: [f32; 3],
        velocity: [f32; 3],
    },
    Debris {
        position: [f32; 3],
    },
    InvertControls(bool),
    CameraShake {
        magnitude: f32,
        time: f32,
    },
    PlayLevelPart(String),
    /// Initialize: Play then Stop every hole_debris part (reset to start).
    ResetLevelPart(String),
    StartLevelEffect(String),
    Boss(BossEvent),
}

#[derive(Clone, Debug, Default)]
pub struct ModelPose {
    pub parent: Mat,
    pub local: Vec<Pose>,
}

#[derive(Clone, Debug)]
pub struct Fafnir {
    pub status: BossStatus,
    pub state: State,
    pub previous: State,
    pub controller: AnimationController,
    pub skeleton: CharacterSkeleton,
    durations: std::collections::HashMap<String, f32>,
    orientation: Mat,
    parent: Mat,
    head: usize,
    mouth: usize,
    eyes: [usize; 2],
    spine_base: usize,
    right_heel: usize,
    right_toe: usize,
    right_wrist: usize,
    neck: [usize; NECK_BONES + 1],
    tail: [usize; TAIL_BONES],
    binds: std::collections::HashMap<usize, Mat>,
    pub aim_for_target: bool,
    pub aim_weight: f32,
    pub aim_target: [f32; 3],
    pub target: Option<u16>,
    pub flash: f32,
    decision_timer: f32,
    sound_a: bool,
    sound_b: bool,
    pub inverted: bool,
    confused_timer: f32,
    fireball_shot: bool,
    /// FirelanceHigh/CeilingState.mLanceTime (counts up per state).
    lance_time: f32,
    /// Fafnir.mLanceTime: a separate field Update counts down while the
    /// lance draws; sharing one variable cancelled the state timer and
    /// froze the lance at 8% length.
    owner_lance_time: f32,
    total_lance_time: f32,
    damage_timer: f32,
    ceiling_timer: f32,
    pub quake_started: bool,
    pub quakes: usize,
    pub quake_threshold: f32,
    pub draw_lance: bool,
    pub lance_scale: f32,
    /// mLanceSegment (origin, delta), clipped by the host after update.
    pub lance: ([f32; 3], [f32; 3]),
    /// FirelanceLowState's mFireBreath this frame (position, direction):
    /// fafnir_fire_breath follows it while the breath window lasts.
    pub breath: Option<([f32; 3], [f32; 3])>,
    hit_list: Vec<(u16, f32)>,
    pub pose: ModelPose,
    /// Head (mouth), body, right heel and right wrist zone transforms.
    pub zones: [Mat; 4],
    /// Tail capsule segments: (from, to, radius).
    pub tail_segments: Vec<([f32; 3], [f32; 3], f32)>,
    pub spine_base_position: [f32; 3],
    pub mouth_transform: Mat,
    pub eye_transforms: [Mat; 2],
    rng: u64,
}

pub(crate) fn mat_translation(m: &Mat) -> [f32; 3] {
    [m[12], m[13], m[14]]
}
pub(crate) fn forward(m: &Mat) -> [f32; 3] {
    [-m[8], -m[9], -m[10]]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    if l > 0.0 {
        a.map(|v| v / l)
    } else {
        a
    }
}
fn flat(a: [f32; 3]) -> [f32; 3] {
    normalize([a[0], 0.0, a[2]])
}
/// Vector3.TransformNormal (row vector).
fn transform_normal(v: [f32; 3], m: &Mat) -> [f32; 3] {
    [0, 1, 2].map(|c| v[0] * m[c] + v[1] * m[4 + c] + v[2] * m[8 + c])
}
/// Matrix.CreateWorld(position, forward, up).
fn create_world(position: [f32; 3], fwd: [f32; 3], up: [f32; 3]) -> Mat {
    let z = normalize(fwd.map(|v| -v));
    let x = normalize(cross(up, z));
    let y = cross(z, x);
    [
        x[0],
        x[1],
        x[2],
        0.0,
        y[0],
        y[1],
        y[2],
        0.0,
        z[0],
        z[1],
        z[2],
        0.0,
        position[0],
        position[1],
        position[2],
        1.0,
    ]
}
fn rotation_y(a: f32) -> Mat {
    let (s, c) = a.sin_cos();
    [
        c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}
fn lance_damages() -> [Damage; 3] {
    [
        Damage {
            attack: attack::DAMAGE,
            element: E::FIRE.0,
            amount: 200.0,
            magnitude: 1.0,
        },
        Damage {
            attack: attack::STATUS,
            element: E::FIRE.0,
            amount: 200.0,
            magnitude: 3.0,
        },
        Damage {
            attack: attack::PUSHED,
            element: E::FIRE.0,
            amount: 300.0,
            magnitude: 3.0,
        },
    ]
}

impl Fafnir {
    pub fn new(mesh: Resolved, animation: Resolved, seed: u64) -> Result<Self, String> {
        let skeleton = CharacterSkeleton::new(mesh.skeleton.clone(), animation.clips.clone());
        let durations = skeleton.clip_durations();
        for name in &clip::NAMES[1..] {
            durations
                .get(*name)
                .ok_or_else(|| format!("{ANIMATION_MODEL} missing clip {name}"))?;
        }
        let bone = |name: &str| {
            skeleton
                .bone(name)
                .ok_or_else(|| format!("{MESH_MODEL} missing bone {name}"))
        };
        let head = bone("Head")?;
        let mut neck = [head; NECK_BONES + 1];
        for (k, slot) in neck.iter_mut().take(NECK_BONES).enumerate() {
            *slot = bone(&format!("Neck{}", k + 1))?;
        }
        let mut tail = [0; TAIL_BONES];
        for (k, slot) in tail.iter_mut().enumerate() {
            *slot = bone(&format!("Tail{}", k + 1))?;
        }
        let mut f = Self {
            status: BossStatus::default(),
            state: State::Sleep,
            previous: State::Sleep,
            controller: AnimationController::default(),
            durations,
            orientation: skinned::IDENTITY,
            parent: skinned::IDENTITY,
            head,
            mouth: bone("Mouth")?,
            eyes: [bone("RightEye")?, bone("LeftEye")?],
            spine_base: bone("SpineBase")?,
            right_heel: bone("RightHeel")?,
            right_toe: bone("RightIndextoeEnd")?,
            right_wrist: bone("RightWrist")?,
            neck,
            tail,
            binds: Default::default(),
            skeleton,
            aim_for_target: false,
            aim_weight: 0.0,
            aim_target: [0.0; 3],
            target: None,
            flash: 0.0,
            decision_timer: 0.0,
            sound_a: false,
            sound_b: false,
            inverted: false,
            confused_timer: 0.0,
            fireball_shot: false,
            lance_time: 0.0,
            owner_lance_time: 0.0,
            total_lance_time: 0.0,
            damage_timer: 0.0,
            ceiling_timer: 0.0,
            quake_started: false,
            quakes: 0,
            quake_threshold: 0.0,
            draw_lance: false,
            lance_scale: 0.0,
            breath: None,
            lance: ([0.0; 3], [0.0; 3]),
            hit_list: Vec::new(),
            pose: ModelPose::default(),
            zones: [skinned::IDENTITY; 4],
            tail_segments: Vec::new(),
            spine_base_position: [0.0; 3],
            mouth_transform: skinned::IDENTITY,
            eye_transforms: [skinned::IDENTITY; 2],
            rng: seed | 1,
        };
        // Bind poses: invert(InverseBindPose · RotY(π)).
        let bones: Vec<usize> = [f.head, f.mouth, f.eyes[0], f.eyes[1], f.spine_base]
            .into_iter()
            .chain([f.right_heel, f.right_toe, f.right_wrist])
            .chain(f.neck)
            .chain(f.tail)
            .collect();
        for b in bones {
            let inv = skinned::mul(&f.skeleton.skeleton.bones[b].inverse_bind, &rotation_y_pi());
            let bind =
                crate::live_collision::inverse(&inv).map_err(|e| format!("bind {b}: {e:?}"))?;
            f.binds.insert(b, bind);
        }
        Ok(f)
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }
    fn random_below(&mut self, n: usize) -> usize {
        ((self.random() * n as f32) as usize).min(n.saturating_sub(1))
    }

    fn clip(&mut self, index: usize, fade: f32, looping: bool) {
        let name = clip::NAMES[index];
        let d = self.durations.get(name).copied().unwrap_or(0.0);
        if fade > 0.0 {
            self.controller.crossfade_named_clip(name, d, fade, looping);
        } else {
            self.controller.start_named_clip(name, d, looping);
        }
    }
    fn normalized(&self) -> f32 {
        let d = self.controller.clip_duration();
        if d > 0.0 {
            self.controller.time / d
        } else {
            0.0
        }
    }
    fn finished(&self) -> bool {
        self.controller.has_finished() && !self.controller.crossfade_enabled()
    }

    /// Initialize(orientation): `ground` is the snapped translation (10 m
    /// down, −0.333) when the host found ground.
    pub fn initialize(
        &mut self,
        mut orientation: Mat,
        ground: Option<[f32; 3]>,
    ) -> Vec<FafnirCommand> {
        if let Some(mut g) = ground {
            g[1] -= 0.333;
            orientation[12..15].copy_from_slice(&g);
        }
        self.orientation = orientation;
        self.parent = skinned::mul(&rotation_y(std::f32::consts::FRAC_PI_2), &orientation);
        self.state = State::Sleep;
        self.previous = State::Sleep;
        self.clip(clip::SLEEP, 0.0, true);
        self.status.max_hit_points = MAX_HIT_POINTS;
        self.status.hit_points = MAX_HIT_POINTS;
        self.status.statuses = Default::default();
        self.status.current = 0;
        for (i, r) in self.status.resistances.iter_mut().enumerate() {
            let e = E::BY_INDEX[i];
            (r.modifier, r.multiplier) = if e == E::COLD {
                (10.0, 2.0)
            } else if e == E::WATER {
                (0.0, 0.0)
            } else if e == E::FIRE {
                (0.0, -3.0)
            } else {
                (0.0, 1.0)
            };
        }
        self.aim_for_target = false;
        self.owner_lance_time = 0.0;
        self.draw_lance = false;
        self.hit_list.clear();
        self.quakes = 0;
        self.set_quake_threshold();
        self.update_pose(0.0);
        // Every hole_debris part is played then stopped.
        (0..25)
            .map(|i| FafnirCommand::ResetLevelPart(level_part(i)))
            .collect()
    }

    fn set_quake_threshold(&mut self) {
        self.quake_threshold = MAX_HIT_POINTS * (5 - (1 + self.quakes as i32)) as f32 * 0.2 + 2.0;
    }

    /// The boss bar's (HitPoints, MaxHitPoints); Dead is never set.
    pub fn health(&self) -> (f32, f32) {
        (self.status.hit_points, MAX_HIT_POINTS)
    }

    fn bone_world(&self, skinned: &[Mat], bone: usize) -> Mat {
        skinned::mul(&self.binds[&bone], &skinned[bone])
    }

    /// Animation update, neck aiming and the bone-bound transforms.
    fn update_pose(&mut self, dt: f32) {
        self.controller.signals.clear();
        self.controller.update(dt);
        let mut local = self.skeleton.local_poses(self.controller.pose_sample());
        let mut skinned = self.skeleton.skinned_from_local(&local, &self.parent);
        if self.aim_weight > f32::from_bits(1) {
            let target = self.aim_target;
            for j in 0..self.neck.len() {
                let num = self.neck[j];
                let mouth = self.bone_world(&skinned, self.mouth);
                let to = sub(target, mat_translation(&mouth));
                let (fwd, dir) = (flat(forward(&mouth)), flat(to));
                let c = dot(fwd, dir).clamp(-1.0, 1.0);
                let axis = cross(fwd, dir);
                let absolute = |s: &[Mat]| {
                    let bind = crate::live_collision::inverse(
                        &self.skeleton.skeleton.bones[num].inverse_bind,
                    )
                    .unwrap_or(skinned::IDENTITY);
                    let abs = skinned::mul(&bind, &s[num]);
                    crate::live_collision::inverse(&abs).unwrap_or(skinned::IDENTITY)
                };
                let rotate = |axis: [f32; 3], angle: f32, s: &[Mat], local: &mut Vec<Pose>| {
                    let a = normalize(transform_normal(axis, &absolute(s)));
                    let (sn, cs) = (angle * 0.5).sin_cos();
                    let q = [a[0] * sn, a[1] * sn, a[2] * sn, cs];
                    // Quaternion.Concatenate(q, local) = Multiply(local, q).
                    local[num].1 = quat_mul(local[num].1, q);
                };
                if dot(axis, axis) > 1e-6 {
                    let angle = c.acos() * self.aim_weight / (self.neck.len() - j) as f32;
                    rotate(axis, angle, &skinned, &mut local);
                    skinned = self.skeleton.skinned_from_local(&local, &self.parent);
                }
                if j == self.neck.len() - 1 {
                    let mouth = self.bone_world(&skinned, self.mouth);
                    let f = forward(&mouth);
                    let d = normalize(sub(target, mat_translation(&mouth)));
                    let axis = cross(f, d);
                    if dot(axis, axis) > 1e-6 {
                        let angle = dot(f, d).clamp(-1.0, 1.0).acos() * self.aim_weight;
                        rotate(axis, angle, &skinned, &mut local);
                        skinned = self.skeleton.skinned_from_local(&local, &self.parent);
                    }
                }
            }
        }
        self.eye_transforms = [
            self.bone_world(&skinned, self.eyes[0]),
            self.bone_world(&skinned, self.eyes[1]),
        ];
        self.mouth_transform = self.bone_world(&skinned, self.mouth);
        // Tail segments from Tail1..Tail11.
        let points: Vec<[f32; 3]> = self
            .tail
            .iter()
            .map(|&b| mat_translation(&self.bone_world(&skinned, b)))
            .collect();
        self.spine_base_position = points[0];
        self.tail_segments = points
            .windows(2)
            .enumerate()
            .map(|(k, w)| (w[0], w[1], ((TAIL_BONES - k) as f32 * 0.15).max(0.333)))
            .collect();
        let heel = mat_translation(&self.bone_world(&skinned, self.right_heel));
        let toe = mat_translation(&self.bone_world(&skinned, self.right_toe));
        let wrist = self.bone_world(&skinned, self.right_wrist);
        let mut spine = self.binds[&self.spine_base];
        spine[13] -= 1.0;
        let body = skinned::mul(&spine, &skinned[self.spine_base]);
        self.zones = [
            self.mouth_transform,
            body,
            create_world(heel, normalize(sub(toe, heel)), [0.0, 1.0, 0.0]),
            wrist,
        ];
        self.pose = ModelPose {
            parent: self.parent,
            local,
        };
    }

    /// Fafnir.Damage(part, ...): only the body (part 1) takes damage.
    pub fn damage(
        &mut self,
        part: usize,
        d: Damage,
        from_zone: bool,
        attack_position: [f32; 3],
        features: u8,
        out: &mut Vec<FafnirCommand>,
    ) -> u16 {
        if matches!(self.state, State::Sleep | State::Intro) || from_zone || part != 1 {
            return magicka_sim::character_impact::result::DEFLECTED;
        }
        let mut events = Vec::new();
        let r = self
            .status
            .damage(d, false, attack_position, features, &mut events);
        out.extend(events.into_iter().map(FafnirCommand::Boss));
        if r & (magicka_sim::character_impact::result::HIT
            | magicka_sim::character_impact::result::DAMAGED)
            != 0
        {
            self.flash = 0.1;
        }
        if self.status.hit_points < 0.0 {
            self.status.hit_points = 1.0;
        }
        r
    }

    /// ScriptMessage: FafnirFight starts the decisions.
    pub fn message(&mut self, name: &str, avatars: &[Avatar], out: &mut Vec<FafnirCommand>) {
        if name == "FafnirFight" {
            self.change(State::Decision, avatars, out);
        }
    }

    /// Fafnir.SelectTarget: a random start among the living avatars.
    fn select_target(&mut self, avatars: &[Avatar]) {
        if avatars.is_empty() {
            return;
        }
        let i = self.random_below(4);
        let a = avatars[i % avatars.len()];
        self.target = Some(a.handle);
        self.aim_target = a.position;
    }

    fn change(&mut self, state: State, avatars: &[Avatar], out: &mut Vec<FafnirCommand>) {
        // OnExit.
        match self.state {
            State::Intro | State::Decision => self.aim_for_target = false,
            State::Fireball => self.aim_for_target = false,
            State::Confuse => self.aim_for_target = false,
            State::LanceHigh | State::LanceLow | State::Ceiling => self.draw_lance = false,
            _ => {}
        }
        self.previous = self.state;
        self.state = state;
        let head = mat_translation(&self.zones[0]);
        match state {
            State::Sleep => self.clip(clip::SLEEP, 0.0, true),
            State::Intro => self.clip(clip::INTRO, 0.5, false),
            State::Idle => {
                self.clip(clip::IDLE, 2.0, true);
                self.aim_for_target = false;
            }
            State::Decision => {
                self.clip(clip::IDLE, 2.0, false);
                self.decision_timer = self.random() * (1.0 - self.status.hit_points * DIVISOR);
                self.select_target(avatars);
            }
            State::Tail => {
                self.clip(clip::TAILWHIP, 0.5, false);
                self.aim_for_target = false;
                self.sound_a = false;
            }
            State::Wing => {
                self.clip(clip::WINGS, 0.5, false);
                self.aim_for_target = false;
                self.sound_a = false;
                self.sound_b = false;
            }
            State::Confuse => {
                self.clip(clip::CHARM, 0.5, false);
                out.push(FafnirCommand::Cue {
                    name: SOUNDS[2],
                    position: Some(head),
                });
                // The host keeps the two infinite fafnir_charm effects on the
                // eyes while Confuse lasts (sync_fafnir_breath).
                self.aim_for_target = true;
            }
            State::Fireball => {
                self.clip(clip::FIREBALL, 0.4, false);
                self.fireball_shot = false;
                self.aim_for_target = true;
                self.select_target(avatars);
            }
            State::LanceHigh | State::LanceLow | State::Ceiling => {
                let c = match state {
                    State::LanceHigh => clip::FIRE_HIGH,
                    State::LanceLow => clip::FIRE_LOW,
                    _ => clip::CEILING,
                };
                self.clip(c, if state == State::Ceiling { 0.25 } else { 0.5 }, false);
                self.aim_for_target = false;
                self.draw_lance = false;
                self.lance_scale = 0.0;
                self.lance_time = 0.0;
                self.damage_timer = 0.5;
                self.ceiling_timer = 0.0;
                let w = times(c);
                self.total_lance_time =
                    (w[1] - w[0]) * self.durations.get(clip::NAMES[c]).copied().unwrap_or(0.0);
            }
            State::EarthQuake => {
                self.clip(clip::CHARM, 0.4, false);
                self.quake_started = false;
            }
            State::Defeated => {
                self.clip(clip::DEFEATED, 0.35, false);
                out.push(FafnirCommand::Cue {
                    name: SOUNDS[3],
                    position: Some(head),
                });
                self.aim_for_target = false;
                self.aim_weight = 0.0;
            }
        }
    }

    fn weights(&mut self, avatars: &[Avatar]) -> [f32; 7] {
        // Wing, Confuse, Ceiling, Tail, Fireball, LanceHigh, LanceLow.
        let health = self.status.hit_points * DIVISOR;
        let pw = 1.0 / avatars.len().max(1) as f32;
        let prev = self.previous;
        let back = [0.0, 0.0, 1.0];
        let mut wing = 0.0;
        if prev != State::Wing {
            for a in avatars {
                let d = flat(sub(a.position, self.spine_base_position));
                let dist = dot(
                    sub(a.position, self.spine_base_position),
                    sub(a.position, self.spine_base_position),
                )
                .sqrt();
                let c = dot(d, back);
                if c > 0.5 && dist < 10.0 {
                    wing += 0.5 * pw + 0.75 * pw * c;
                }
            }
        }
        let confuse = if health <= 0.6 && prev != State::Confuse && !self.inverted {
            self.random()
        } else {
            0.0
        };
        let ceiling = if health <= 0.4 && prev != State::Ceiling {
            self.random()
        } else {
            0.0
        };
        let mut tail = 0.0;
        if prev != State::Tail {
            let mut t = self.tail_segments.get(5).map_or([0.0; 3], |s| s.1);
            t[1] = 0.0;
            for a in avatars {
                let mut p = a.position;
                p[1] = 0.0;
                if dot(sub(p, t), sub(p, t)) <= 25.0 {
                    tail += pw;
                }
            }
        }
        let fireball = if prev != State::Fireball {
            self.random()
        } else {
            0.0
        };
        let high = if prev != State::LanceHigh {
            self.random()
        } else {
            0.0
        };
        let mut low = 0.0f32;
        if prev != State::LanceLow {
            let h = mat_translation(&self.zones[0]);
            for a in avatars {
                let d2 = ((a.position[0] - h[0]).powi(2) + (a.position[2] - h[2]).powi(2)).max(9.0);
                if d2 <= 73.0 {
                    low = ((64.0 - (d2 - 9.0)) / 64.0).max(low) + 0.2;
                }
            }
        }
        [wing, confuse, ceiling, tail, fireball, high, low]
    }

    fn decide(&mut self, avatars: &[Avatar], out: &mut Vec<FafnirCommand>) {
        if avatars.is_empty() {
            self.change(State::Idle, avatars, out);
            return;
        }
        if self.status.hit_points <= self.quake_threshold {
            self.change(State::EarthQuake, avatars, out);
            return;
        }
        let w = self.weights(avatars);
        let sum: f32 = w.iter().sum();
        let [wing, confuse, ceiling, tail, fireball, high, low] = w.map(|v| v / sum);
        let next = if wing >= confuse
            && wing >= ceiling
            && wing >= fireball
            && wing >= high
            && wing >= tail
            && wing >= low
        {
            State::Wing
        } else if confuse >= ceiling
            && confuse >= fireball
            && confuse >= high
            && confuse >= tail
            && confuse >= low
        {
            State::Confuse
        } else if tail >= ceiling && tail >= fireball && tail >= high && tail >= low {
            State::Tail
        } else if ceiling >= fireball && ceiling >= high && ceiling >= low {
            State::Ceiling
        } else if high >= fireball && high >= low {
            State::LanceHigh
        } else if low >= fireball {
            State::LanceLow
        } else if self.previous != State::Fireball {
            State::Fireball
        } else {
            loop {
                let s = match self.random_below(6) {
                    1 => State::Ceiling,
                    2 => State::Tail,
                    3 => State::LanceLow,
                    4 => State::LanceHigh,
                    5 => State::Confuse,
                    _ => State::Wing,
                };
                if s != self.previous {
                    break s;
                }
            }
        };
        self.change(next, avatars, out);
    }

    fn lance_scale(&mut self, dt: f32) {
        self.lance_time += dt;
        let t = if self.total_lance_time > 0.0 {
            self.lance_time / self.total_lance_time
        } else {
            0.0
        };
        self.lance_scale = (2.0 - 32.0 * (t - 0.5).powi(4)).min(1.0);
        self.draw_lance = true;
    }

    fn update_state(&mut self, dt: f32, avatars: &[Avatar], out: &mut Vec<FafnirCommand>) {
        let n = self.normalized();
        let head = mat_translation(&self.zones[0]);
        let target_alive = self
            .target
            .is_some_and(|t| avatars.iter().any(|a| a.handle == t));
        match self.state {
            State::Sleep => {}
            State::Intro => {
                if !self.controller.is_looping() && self.finished() {
                    out.push(FafnirCommand::Trigger("fafnir_awake"));
                    self.clip(clip::IDLE, 0.5, true);
                }
            }
            State::Idle => {
                if !avatars.is_empty() {
                    self.change(State::Decision, avatars, out);
                }
            }
            State::Decision => {
                self.decision_timer -= dt;
                if self.decision_timer > 0.0
                    && (self.controller.crossfade_enabled() || !self.controller.has_finished())
                {
                    return;
                }
                self.decide(avatars, out);
            }
            State::Tail => {
                if !self.controller.crossfade_enabled() {
                    if self.controller.has_finished() {
                        self.change(State::Decision, avatars, out);
                        return;
                    }
                    if !self.sound_a && n > 0.5 {
                        self.sound_a = true;
                        out.push(FafnirCommand::Cue {
                            name: SOUNDS[0],
                            position: Some(head),
                        });
                    }
                    // OnTailCollision within the tailwhip window.
                    let w = times(clip::TAILWHIP);
                    if n >= w[0] && n <= w[1] {
                        out.push(FafnirCommand::TailSweep {
                            segments: self.tail_segments.clone(),
                        });
                    }
                }
            }
            State::Wing => {
                if self.finished() {
                    self.change(State::Decision, avatars, out);
                    return;
                }
                let w = times(clip::WINGS);
                let first = !self.controller.crossfade_enabled() && n >= w[0] && n <= w[1];
                let second = n >= w[2] && n <= w[3];
                if !first && !second {
                    return;
                }
                if n <= w[1] && !self.sound_a {
                    out.push(FafnirCommand::Cue {
                        name: SOUNDS[0],
                        position: Some(head),
                    });
                    self.sound_a = true;
                } else if n >= w[2] && !self.sound_b {
                    out.push(FafnirCommand::Cue {
                        name: SOUNDS[0],
                        position: Some(head),
                    });
                    self.sound_b = true;
                }
                out.push(FafnirCommand::WingPush {
                    spine_base: self.spine_base_position,
                });
            }
            State::Confuse => {
                if self.controller.crossfade_enabled() {
                    return;
                }
                let w = times(clip::CHARM);
                if !self.inverted && n >= w[0] && n <= w[1] {
                    out.push(FafnirCommand::InvertControls(true));
                    self.inverted = true;
                    self.confused_timer = 20.0;
                } else if self.controller.has_finished() {
                    self.change(State::Decision, avatars, out);
                }
            }
            State::Fireball => {
                if !target_alive {
                    self.change(State::Decision, avatars, out);
                    return;
                }
                let w = times(clip::FIREBALL);
                if n >= w[0]
                    && n <= w[1]
                    && !self.controller.crossfade_enabled()
                    && !self.fireball_shot
                {
                    out.push(FafnirCommand::Cue {
                        name: SOUNDS[0],
                        position: Some(head),
                    });
                    let f = forward(&self.zones[0]);
                    let p = [0, 1, 2].map(|i| head[i] + 2.0 * f[i]);
                    out.push(FafnirCommand::Fireball {
                        position: p,
                        velocity: f.map(|v| v * 30.0),
                    });
                    self.fireball_shot = true;
                } else if self.finished() {
                    self.change(State::Decision, avatars, out);
                }
            }
            State::LanceHigh => {
                if !target_alive {
                    self.change(State::Decision, avatars, out);
                    return;
                }
                let w = times(clip::FIRE_HIGH);
                if n >= w[0] && n <= w[1] && !self.controller.crossfade_enabled() {
                    self.damage_timer -= dt;
                    self.aim_for_target = true;
                    self.lance_scale(dt);
                    // fafnir_lance_weld is one looping effect the host keeps at
                    // the clipped lance end (sync_fafnir_breath), not a start per
                    // frame: that leaked an emitter wherever the end had been.
                    let end = [0, 1, 2].map(|i| self.lance.0[i] + self.lance.1[i]);
                    if self.damage_timer <= 0.0 {
                        self.damage_timer = 0.25;
                        out.push(FafnirCommand::CircleDamage {
                            position: end,
                            radius: 2.0,
                            damages: lance_damages(),
                        });
                    }
                } else if n > w[1] {
                    self.draw_lance = false;
                }
                if self.finished() {
                    self.change(State::Decision, avatars, out);
                }
            }
            State::LanceLow => {
                if !target_alive {
                    self.change(State::Decision, avatars, out);
                    return;
                }
                if self.controller.crossfade_enabled() {
                    return;
                }
                let w = times(clip::FIRE_LOW);
                if n >= w[0] && n <= w[1] {
                    self.damage_timer -= dt;
                    let f = normalize(forward(&self.zones[0]));
                    let mouth = mat_translation(&self.mouth_transform);
                    let p = [0, 1, 2].map(|i| mouth[i] + f[i]);
                    self.breath = Some((p, f));
                    if self.damage_timer <= 0.0 {
                        out.push(FafnirCommand::ArcDamage {
                            position: p,
                            direction: f,
                            range: 20.0,
                            angle: std::f32::consts::FRAC_PI_4,
                            damages: lance_damages(),
                        });
                        self.damage_timer += 0.25;
                    }
                } else if n > w[1] {
                    self.draw_lance = false;
                }
                if self.controller.has_finished() {
                    self.change(State::Decision, avatars, out);
                }
            }
            State::Ceiling => {
                if !target_alive {
                    self.change(State::Decision, avatars, out);
                    return;
                }
                let w = times(clip::CEILING);
                if n >= w[0] && n <= w[1] && !self.controller.crossfade_enabled() {
                    out.push(FafnirCommand::CameraShake {
                        magnitude: 0.5,
                        time: dt,
                    });
                    self.lance_scale(dt);
                    self.ceiling_timer += dt;
                    if self.ceiling_timer > 0.5 {
                        self.ceiling_timer -= 0.5;
                        if let Some(t) = avatars.iter().find(|a| Some(a.handle) == self.target) {
                            let mut p = t.position;
                            p[1] += 50.0;
                            p[0] += (self.random() - 0.5) * 6.0;
                            p[2] += (self.random() - 0.5) * 6.0;
                            out.push(FafnirCommand::Debris { position: p });
                        }
                    }
                } else if n > w[1] {
                    self.draw_lance = false;
                }
                if self.finished() {
                    self.change(State::Decision, avatars, out);
                }
            }
            State::EarthQuake => {
                if self.controller.crossfade_enabled() {
                    return;
                }
                if n >= 0.5 && !self.quake_started {
                    loop {
                        if let Some(group) = FLOOR_GROUPS.get(self.quakes) {
                            for &i in group.iter() {
                                out.push(FafnirCommand::PlayLevelPart(level_part(i)));
                                out.push(FafnirCommand::StartLevelEffect(lava_effect(i)));
                            }
                        }
                        self.quakes += 1;
                        self.set_quake_threshold();
                        if self.status.hit_points > self.quake_threshold
                            || self.quakes >= FLOOR_GROUPS.len()
                        {
                            break;
                        }
                    }
                    out.push(FafnirCommand::SpellCue {
                        name: "magick_meteor_rumble",
                    });
                    out.push(FafnirCommand::Cue {
                        name: SOUNDS[1],
                        position: None,
                    });
                    // The host keeps the looping fafnir_earthquake on the head
                    // zone until the state ends (sync_fafnir_breath).
                    self.quake_started = true;
                    out.push(FafnirCommand::CameraShake {
                        magnitude: 1.5,
                        time: 2.75,
                    });
                } else if self.controller.has_finished() {
                    self.change(State::Decision, avatars, out);
                }
            }
            State::Defeated => {
                if !self.controller.is_looping() && self.finished() {
                    out.push(FafnirCommand::Trigger("fafnir_defeated"));
                    self.clip(clip::IDLE, 1.0, true);
                }
            }
        }
    }

    /// The wing/tail hit list: true (and entered for `ttl`) when free.
    pub fn hit(&mut self, handle: u16, ttl: f32) -> bool {
        if self.hit_list.iter().any(|(h, _)| *h == handle) {
            return false;
        }
        self.hit_list.push((handle, ttl));
        true
    }

    /// Fafnir.UpdateBoss.
    pub fn update(
        &mut self,
        dt: f32,
        inputs: FafnirInputs,
        avatars: &[Avatar],
    ) -> Vec<FafnirCommand> {
        let mut out = Vec::new();
        self.breath = None;
        if inputs.fight_started && self.state == State::Sleep {
            self.change(State::Intro, avatars, &mut out);
        } else if self.status.hit_points <= 1.0 && self.state != State::Defeated {
            self.change(State::Defeated, avatars, &mut out);
        }
        if self.inverted {
            self.confused_timer -= dt;
            if self.confused_timer <= 0.0 {
                out.push(FafnirCommand::InvertControls(false));
                self.inverted = false;
            }
        }
        self.aim_weight = if self.aim_for_target {
            (self.aim_weight + dt).min(1.0)
        } else {
            (self.aim_weight - dt).max(0.0)
        };
        if let Some(t) = self
            .target
            .and_then(|t| avatars.iter().find(|a| a.handle == t))
        {
            for i in 0..3 {
                self.aim_target[i] += (t.position[i] - self.aim_target[i]) * dt;
            }
        }
        self.update_pose(dt);
        self.flash = (self.flash - dt).max(0.0);
        for (_, t) in &mut self.hit_list {
            *t -= dt;
        }
        self.hit_list.retain(|(_, t)| *t > 0.0);
        self.update_state(dt, avatars, &mut out);
        if self.draw_lance {
            self.owner_lance_time -= dt;
            let f = forward(&self.zones[0]);
            self.lance = (
                mat_translation(&self.mouth_transform),
                f.map(|v| v * 30.0 * self.lance_scale),
            );
        }
        for (amount, element) in self.status.update_statuses(dt, false) {
            self.status.status_damage(amount, element);
        }
        let mut events = Vec::new();
        self.status.update_damage(dt, false, &mut events);
        out.extend(events.into_iter().map(FafnirCommand::Boss));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quake_thresholds_are_fifths_plus_two() {
        let t = |q: i32| MAX_HIT_POINTS * (5 - (1 + q)) as f32 * 0.2 + 2.0;
        assert_eq!(t(0), 28002.0);
        assert_eq!(t(3), 7002.0);
        assert_eq!(level_part(12), "hole_debris12");
        assert_eq!(level_part(13), "hole_debris14");
        assert_eq!(lava_effect(24), "effect_lava25");
    }
}
