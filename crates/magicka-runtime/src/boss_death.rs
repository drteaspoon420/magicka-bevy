//! Niflheim Death's asset-backed encounter state and animation poses.
//! Scene, collision and presentation commands are explicit host obligations.
//! Static provenance and unresolved integration: research/boss-death.md.
use crate::{
    animation_controller::AnimationController, boss_fafnir::ModelPose,
    character_pose::CharacterSkeleton,
};
use magicka_content::skinned::{self, Mat, Resolved};
use magicka_sim::{
    boss_death::{
        self as policy, Ability, DeathCombat, RaiseDeadPulse, RiseIntroGate, Vulnerability,
    },
    damage::Damage,
};

pub const MODEL: &str = "Models/Bosses/Death/Death";
pub const SCYTHE: &str = "Models/Bosses/Death/Death_Scythe";
pub const CLIPS: [&str; 12] = [
    "appear",
    "attack_scythe_fall",
    "attack_scythe_rise",
    "defeated",
    "hit",
    "idle",
    "move_glide",
    "sit",
    "sit_talk",
    "summon_scythe",
    "summon_skeleton",
    "timewarp",
];

/// Death's five shared skin snapshots, distinct from Avatar's Haste history.
/// Empty native matrices collapse the mesh; absent poses represent that here.
#[derive(Clone, Debug, Default)]
pub struct DeathAfterimages {
    pub poses: [Option<ModelPose>; 5],
    pub drawn: bool,
    timer: f32,
    intensity: f32,
}

impl DeathAfterimages {
    fn clear_poses(&mut self) {
        self.poses.fill(None);
    }

    fn update(&mut self, dt: f32, drawn: bool, movement: [f32; 3], pose: &ModelPose) {
        self.drawn = drawn && movement[0] * movement[0] + movement[2] * movement[2] > 0.1;
        if !self.drawn {
            // Native neither ages nor resets history while outside this gate.
            return;
        }
        self.timer -= dt;
        self.intensity -= dt;
        while self.timer <= 0. {
            self.timer += 0.05;
            while self.intensity <= 0. {
                self.intensity += 0.05;
            }
            self.poses.rotate_right(1);
            self.poses[0] = Some(pose.clone());
        }
    }

    /// Draw visits every slot, including zero. Unlike Avatar's draw loop,
    /// exact zero skips a slot without decrementing the running alpha.
    pub fn draw_alphas(&self) -> [f32; 5] {
        let step = 0.333 / (self.poses.len() as f32 + 1.);
        let mut alpha = 0.333 + self.intensity * 20. * step;
        std::array::from_fn(|_| {
            let value = alpha;
            if alpha != 0. {
                alpha -= step;
            }
            value
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    SitIntro,
    RiseIntro,
    Idle,
    Prep,
    Appear,
    Move,
    Attack,
    Hit,
    RaiseDead,
    WarpTime,
    PreMultiDeath,
    MultiDeath,
    DefeatPreSit,
    Defeat,
}
impl State {
    fn ability(self) -> Option<Ability> {
        match self {
            Self::RaiseDead => Some(Ability::RaiseDead),
            Self::WarpTime => Some(Ability::WarpTime),
            Self::MultiDeath => Some(Ability::MultiDeath),
            Self::Move => Some(Ability::Move),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Avatar {
    pub handle: u16,
    pub slot: usize,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub radius: f32,
    pub playing: bool,
    pub dead: bool,
}

pub trait Environment {
    /// NavMesh.GetNearestPosition: distance and the returned point.
    fn nearest(&mut self, point: [f32; 3]) -> Option<(f32, [f32; 3])>;
    /// Scene.SegmentIntersect from point + Y to point - 3Y.
    fn ground(&mut self, point: [f32; 3]) -> Option<[f32; 3]>;
    /// Presence count of the four darksoul types, not merely owned summons.
    fn skeleton_count(&self) -> i32;
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Effect {
        name: &'static str,
        position: [f32; 3],
        direction: [f32; 3],
    },
    StopEffect(&'static str),
    Sound {
        bank: u16,
        name: &'static str,
    },
    StopSwing,
    Hit {
        amount: f32,
        first: bool,
        position: [f32; 3],
    },
    KillSkeletons,
    RaiseSkeleton,
    TimeWarp,
    Flash(f32),
    /// Slow world to 0.5, set target TimeWarpModifier=2, lock other slots and
    /// make other playing avatars ethereal. Camera attaches to target at 1.2.
    BeginClones {
        target: u16,
    },
    /// CharacterBody.SpeedMultiplier *= 2 each PreMulti/Multi update.
    TargetSpeed {
        target: u16,
    },
    /// Make all playing avatars corporeal as the real attacker reaches them.
    RevealReal,
    /// Restore world/target speed, input, avatar ethereality and camera.
    EndClones {
        target: Option<u16>,
    },
}

#[derive(Clone, Debug)]
pub struct Death {
    pub combat: DeathCombat,
    pub state: State,
    pub previous: State,
    pub controller: AnimationController,
    pub clone_controller: AnimationController,
    pub skeleton: CharacterSkeleton,
    pub orientation: Mat,
    start: Mat,
    pub drawn: bool,
    pub ethereal: bool,
    pub update_main: bool,
    pub movement: [f32; 3],
    pub speed: f32,
    pub target: Option<u16>,
    pub visible: [bool; 16],
    pub poses: Vec<ModelPose>,
    pub afterimages: DeathAfterimages,
    main_pose: ModelPose,
    pub scythes: [Mat; 16],
    pub body_position: [f32; 3],
    pub alpha: f32,
    pub current_slot: usize,
    pub clone_count: usize,
    pub real_clone: usize,
    pub fake_attack: bool,
    durations: std::collections::HashMap<String, f32>,
    hand: usize,
    hand_bind: Mat,
    prep_time: f32,
    rise: RiseIntroGate,
    pulse: RaiseDeadPulse,
    next: State,
    scythe_lifted: bool,
    flux: f32,
    appear_duration: f32,
    clone_orientation: Mat,
    clone_distance: f32,
    attacker: Option<usize>,
    approaching: bool,
    within_reach: bool,
    attacks: usize,
    rng: u64,
}

fn translation(m: &Mat) -> [f32; 3] {
    [m[12], m[13], m[14]]
}
fn forward(m: &Mat) -> [f32; 3] {
    [-m[8], -m[9], -m[10]]
}
fn flat_delta(a: [f32; 3], b: [f32; 3]) -> ([f32; 3], f32) {
    let d = [a[0] - b[0], 0., a[2] - b[2]];
    let n = (d[0] * d[0] + d[2] * d[2]).sqrt();
    (d.map(|v| v / n), n)
}
fn facing(p: [f32; 3], d: [f32; 3]) -> Mat {
    [
        -d[2], 0., d[0], 0., 0., 1., 0., 0., -d[0], 0., -d[2], 0., p[0], p[1], p[2], 1.,
    ]
}

impl Death {
    pub fn new(model: Resolved, orientation: Mat, seed: u64) -> Result<Self, String> {
        let skeleton = CharacterSkeleton::new(model.skeleton, model.clips);
        let durations = skeleton.clip_durations();
        for clip in CLIPS {
            if !durations.contains_key(clip) {
                return Err(format!("Death missing clip {clip}"));
            }
        }
        let hand = skeleton
            .bone("RightAttach")
            .ok_or("Death missing RightAttach")?;
        let inv = skinned::mul(
            &skeleton.skeleton.bones[hand].inverse_bind,
            &crate::boss_machine::rotation_y_pi(),
        );
        let hand_bind =
            crate::live_collision::inverse(&inv).map_err(|e| format!("Death hand bind: {e:?}"))?;
        let mut d = Self {
            combat: DeathCombat::default(),
            state: State::SitIntro,
            previous: State::SitIntro,
            controller: AnimationController::default(),
            clone_controller: AnimationController::default(),
            skeleton,
            orientation,
            start: orientation,
            drawn: true,
            ethereal: false,
            update_main: true,
            movement: [0.; 3],
            speed: 0.,
            target: None,
            visible: [false; 16],
            poses: vec![ModelPose::default(); 16],
            afterimages: DeathAfterimages::default(),
            main_pose: ModelPose::default(),
            scythes: [skinned::IDENTITY; 16],
            body_position: translation(&orientation),
            alpha: 0.,
            current_slot: 0,
            clone_count: 0,
            real_clone: 0,
            fake_attack: false,
            durations,
            hand,
            hand_bind,
            prep_time: 0.,
            rise: RiseIntroGate::default(),
            pulse: RaiseDeadPulse::default(),
            next: State::Move,
            scythe_lifted: false,
            flux: 0.,
            appear_duration: 0.,
            clone_orientation: orientation,
            clone_distance: 0.,
            attacker: None,
            approaching: false,
            within_reach: false,
            attacks: 0,
            rng: seed | 1,
        };
        d.visible[0] = true;
        d.clip(8, 0., true);
        d.pose_main(0.);
        Ok(d)
    }
    // INFERRED deterministic stream; shared System.Random ownership is not ported.
    fn random(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }
    fn below(&mut self, n: usize) -> usize {
        (self.random() * n as f64) as usize
    }
    fn clip(&mut self, index: usize, fade: f32, looping: bool) {
        let name = CLIPS[index];
        let duration = self.durations[name];
        if fade > 0. {
            self.controller
                .crossfade_named_clip(name, duration, fade, looping);
        } else {
            self.controller.start_named_clip(name, duration, looping);
        }
    }
    fn clone_clip(&mut self, index: usize, fade: f32, looping: bool) {
        let name = CLIPS[index];
        let duration = self.durations[name];
        if fade > 0. {
            self.clone_controller
                .crossfade_named_clip(name, duration, fade, looping);
        } else {
            self.clone_controller
                .start_named_clip(name, duration, looping);
        }
    }
    fn finished(&self) -> bool {
        self.controller.has_finished() && !self.controller.crossfade_enabled()
    }
    fn avatar<'a>(&self, avatars: &'a [Avatar]) -> Option<&'a Avatar> {
        avatars.iter().find(|a| Some(a.handle) == self.target)
    }
    fn select(&mut self, avatars: &[Avatar]) {
        let n = avatars.iter().map(|a| a.slot + 1).max().unwrap_or(0).max(4);
        let mut slots = vec![
            policy::PlayerSlot {
                playing: false,
                living_avatar: None,
            };
            n
        ];
        for a in avatars {
            slots[a.slot] = policy::PlayerSlot {
                playing: a.playing,
                living_avatar: (!a.dead).then_some(a.handle),
            };
        }
        let count = avatars.iter().filter(|a| a.playing).count();
        if count > 0 {
            let start = self.below(count);
            self.target = policy::select_target(&slots, start, self.target);
        }
    }
    fn sound(out: &mut Vec<Command>, bank: u16, name: &'static str) {
        out.push(Command::Sound { bank, name });
    }
    fn effect(&self, out: &mut Vec<Command>, name: &'static str, direction: [f32; 3]) {
        out.push(Command::Effect {
            name,
            position: translation(&self.orientation),
            direction,
        });
    }
    fn change(&mut self, state: State, avatars: &[Avatar], out: &mut Vec<Command>) {
        match self.state {
            State::RiseIntro => self.speed = policy::movement_speed(self.combat.hit_points()),
            State::Move => {
                self.ethereal = false;
                self.movement = [0.; 3];
            }
            State::Attack => {
                self.speed = policy::movement_speed(self.combat.hit_points());
                self.movement = [0.; 3];
                self.controller.speed = 1.;
                self.afterimages.clear_poses();
            }
            State::Hit => {
                out.push(Command::StopEffect("death_stunned"));
                self.drawn = false;
                self.combat.exit_hit();
            }
            State::DefeatPreSit => out.push(Command::StopEffect("death_defeated")),
            State::Appear => self.ethereal = false,
            State::WarpTime | State::RaiseDead => self.controller.speed = 1.,
            State::MultiDeath => {
                self.afterimages.clear_poses();
                out.push(Command::EndClones {
                    target: self.target,
                });
                self.movement = [0.; 3];
                self.drawn = true;
                self.visible = [false; 16];
                self.visible[0] = true;
                self.controller.speed = 1.;
                self.update_main = true;
                self.current_slot = 0;
                self.real_clone = 0;
            }
            _ => {}
        }
        self.previous = self.state;
        self.state = state;
        match state {
            State::SitIntro => {
                self.orientation = self.start;
                self.speed = 0.;
                self.drawn = true;
                self.clip(8, 0., true);
            }
            State::RiseIntro => {
                self.drawn = true;
                self.clip(9, 0.15, false);
                self.rise = RiseIntroGate::default();
                self.ethereal = true;
            }
            State::Idle => {
                self.drawn = true;
                self.clip(5, 0.2, true);
            }
            State::Prep => {
                self.drawn = false;
                self.ethereal = true;
                self.clip(6, 0., true);
                self.prep_time = policy::preparation_delay(self.combat.hit_points(), self.random());
                self.effect(out, "death_ethereal_despawn", forward(&self.orientation));
                self.select(avatars);
            }
            State::Appear => {
                self.ethereal = true;
                self.drawn = true;
                self.clip(0, 0., false);
                self.movement = [0.; 3];
                self.appear_duration = self.durations[CLIPS[0]];
                self.alpha = 0.;
            }
            State::Move => {
                self.ethereal = true;
                self.drawn = true;
                self.clip(6, 0.2, true);
            }
            State::Attack => {
                self.scythe_lifted = false;
                self.clip(2, 0.2, false);
                Self::sound(out, 128, "wep_death_scythe_swing");
            }
            State::Hit => {
                self.clip(4, 0.2, false);
                self.effect(out, "death_stunned", forward(&self.orientation));
            }
            State::DefeatPreSit => {
                self.ethereal = false;
                self.drawn = true;
                self.clip(3, 0.2, false);
                self.effect(out, "death_defeated", forward(&self.orientation));
            }
            State::Defeat => {
                self.ethereal = false;
                self.drawn = true;
                self.orientation = self.start;
                self.clip(8, 0.2, false);
                self.combat.enter_defeat();
            }
            State::WarpTime => {
                self.drawn = true;
                self.clip(11, 0.2, false);
                self.movement = [0.; 3];
            }
            State::RaiseDead => {
                self.drawn = true;
                self.controller.speed = policy::raise_animation_speed(self.combat.hit_points());
                self.clip(10, 0.2, false);
                self.movement = [0.; 3];
                self.pulse = RaiseDeadPulse::default();
            }
            State::PreMultiDeath => {
                self.ethereal = true;
                self.drawn = true;
                self.select(avatars);
                out.push(Command::KillSkeletons);
                if let Some(a) = self.avatar(avatars) {
                    out.push(Command::BeginClones { target: a.handle });
                    self.clone_distance = flat_delta(a.position, translation(&self.orientation)).1;
                }
                self.clone_clip(0, 0., false);
                self.update_main = false;
                self.movement = [0.; 3];
                self.clone_orientation = self.orientation;
                self.clone_count = policy::clone_count(self.combat.hit_points());
                for i in 0..self.clone_count {
                    self.visible[i] = true;
                }
                self.alpha = 0.;
                Self::sound(out, crate::audio::banks::MISC, "misc_flash");
                out.push(Command::Flash(0.2));
            }
            State::MultiDeath => {
                self.clone_clip(6, 0.2, true);
                self.movement = [0.; 3];
                self.real_clone = self.below(self.clone_count);
                self.attacks = 0;
                self.attacker = None;
                self.approaching = false;
                self.within_reach = false;
                self.update_main = false;
                self.scythe_lifted = false;
            }
        }
    }
    fn turn(&mut self, d: [f32; 3], speed: f32, dt: f32) {
        use magicka_sim::character_physics::{quaternion_lerp, Orientation};
        let old = Orientation {
            right: [
                self.orientation[0],
                self.orientation[1],
                self.orientation[2],
            ],
            up: [0., 1., 0.],
            backward: [
                self.orientation[8],
                self.orientation[9],
                self.orientation[10],
            ],
        };
        let new = Orientation {
            right: [-d[2], 0., d[0]],
            up: [0., 1., 0.],
            backward: d.map(|v| -v),
        };
        let q = quaternion_lerp(
            old.to_quaternion(),
            new.to_quaternion(),
            (dt * speed).clamp(0., 1.),
        );
        let o = Orientation::from_quaternion(q);
        self.orientation[..3].copy_from_slice(&o.right);
        self.orientation[4..7].copy_from_slice(&o.up);
        self.orientation[8..11].copy_from_slice(&o.backward);
    }
    fn position_behind(&mut self, a: Avatar, ability: Ability, world: &mut impl Environment) {
        let distance = policy::placement_distance(self.combat.hit_points(), ability);
        let mut p = std::array::from_fn(|i| a.position[i] - a.direction[i] * distance);
        if let Some((distance, point)) = world.nearest(p) {
            if distance < 7. {
                p = point;
            }
        }
        let d = flat_delta(a.position, p).0;
        self.orientation = facing(p, d);
    }
    fn chase(&mut self, a: Avatar, dt: f32) {
        let (d, distance) = flat_delta(a.position, translation(&self.orientation));
        if a.radius + 0.75 < distance {
            self.speed = distance / 2.5 * policy::movement_speed(self.combat.hit_points());
            self.movement = d;
            self.turn(d, self.speed.max(8.), dt);
        } else {
            self.speed = 0.1;
        }
    }
    fn set_pose(&mut self, slot: usize, parent: Mat, clone: bool) {
        let controller = if clone {
            &self.clone_controller
        } else {
            &self.controller
        };
        let local = self.skeleton.local_poses(controller.pose_sample());
        let skin = self.skeleton.skinned_from_local(&local, &parent);
        self.scythes[slot] = skinned::mul(&self.hand_bind, &skin[self.hand]);
        self.poses[slot] = ModelPose { parent, local };
    }
    fn pose_main(&mut self, dt: f32) {
        self.controller.signals.clear();
        self.controller.update(dt);
        if !matches!(self.state, State::SitIntro | State::Defeat) {
            self.flux += dt;
        } else {
            self.flux = 0.;
        }
        let mut parent = self.orientation;
        parent[13] += self.flux.cos() * 0.2;
        for i in [0, 1, 2, 4, 5, 6, 8, 9, 10] {
            parent[i] *= 1.2;
        }
        self.set_pose(self.current_slot, parent, false);
        self.main_pose = self.poses[self.current_slot].clone();
    }
    fn clone_transform(&self, index: usize, target: [f32; 3]) -> Mat {
        let direction = forward(&self.clone_orientation);
        let offset = direction.map(|v| -v * self.clone_distance);
        let angle = (index + 1) as f32 / self.clone_count as f32 * std::f32::consts::TAU;
        let (sin, cos) = angle.sin_cos();
        let rotate = |v: [f32; 3]| [v[0] * cos + v[2] * sin, v[1], -v[0] * sin + v[2] * cos];
        let o = rotate(offset);
        let p = std::array::from_fn(|k| target[k] + o[k]);
        facing(p, rotate(direction))
    }
    fn ring(&mut self, target: [f32; 3], dt: f32) {
        self.clone_controller.signals.clear();
        self.clone_controller.update(dt);
        for i in 0..self.clone_count {
            let mut parent = self.clone_transform(i, target);
            for k in [0, 1, 2, 4, 5, 6, 8, 9, 10] {
                parent[k] *= 1.2;
            }
            if self.state != State::MultiDeath || self.attacker != Some(i) {
                self.set_pose(i, parent, true);
            }
        }
    }
    fn multi(&mut self, dt: f32, avatars: &[Avatar], out: &mut Vec<Command>) {
        let Some(a) = self.avatar(avatars).copied().filter(|a| !a.dead) else {
            self.change(State::Prep, avatars, out);
            return;
        };
        if self.attacks >= self.clone_count {
            self.change(State::Prep, avatars, out);
            return;
        }
        if self.speed <= 0.1 {
            self.speed = policy::movement_speed(self.combat.hit_points());
        }
        out.push(Command::TargetSpeed { target: a.handle });
        if !self.approaching && !self.within_reach {
            self.approaching = true;
            while self.attacker.is_none_or(|i| !self.visible[i]) {
                self.attacker = Some(self.below(self.clone_count));
            }
            let i = self.attacker.unwrap();
            self.fake_attack = i != self.real_clone;
            if let Some(sample) = self.clone_controller.pose_sample() {
                self.controller
                    .start_named_clip(sample.clip, sample.duration, true);
                self.controller.time = sample.time;
            }
            self.clip(6, 0.2, true);
            self.current_slot = i;
            self.update_main = true;
            self.orientation = self.clone_transform(i, a.position);
        }
        self.ring(a.position, dt);
        if self.within_reach {
            if !self.fake_attack {
                self.ethereal = false;
                out.push(Command::RevealReal);
                for i in 0..16 {
                    if Some(i) != self.attacker {
                        self.visible[i] = false;
                    }
                }
            }
            self.chase(a, dt);
            if self.controller.has_finished() && self.fake_attack {
                self.effect(out, "death_ethereal_spawn", [1., 0., 0.]);
                self.visible[self.attacker.unwrap()] = false;
                self.approaching = false;
                self.within_reach = false;
                self.update_main = false;
                self.movement = [0.; 3];
                self.speed = policy::movement_speed(self.combat.hit_points());
                self.attacks += 1;
            } else if self.finished() && !self.fake_attack {
                if !self.scythe_lifted {
                    self.clip(1, 0.2, false);
                    self.scythe_lifted = true;
                } else {
                    self.change(State::Prep, avatars, out);
                    return;
                }
            }
        }
        if self.approaching {
            let (d, distance) = flat_delta(a.position, translation(&self.orientation));
            self.movement = d;
            self.turn(d, 8., dt);
            if distance < 2.5 {
                self.approaching = false;
                self.within_reach = true;
                self.clip(2, 0.2, false);
                self.movement = [0.; 3];
            }
        }
    }
    pub fn damage(&mut self, damage: Damage, features: u8) -> (u16, Vec<Command>) {
        let visibility = Vulnerability {
            drawn: self.drawn,
            ethereal: self.ethereal,
            fake_multi_attack: self.state == State::MultiDeath && self.fake_attack,
        };
        let (result, hit) = self.combat.damage(damage, visibility, features);
        let mut out = Vec::new();
        if let Some(hit) = hit {
            if hit.first_hit {
                out.push(Command::StopSwing);
                Self::sound(&mut out, 128, "wep_death_scythe");
                self.effect(&mut out, "death_hit", [1., 0., 0.]);
            }
            out.push(Command::Hit {
                amount: hit.amount,
                first: hit.first_hit,
                position: [
                    self.orientation[12],
                    self.orientation[13] + 2.2,
                    self.orientation[14],
                ],
            });
        }
        (result, out)
    }
    /// Native states run before the controller advances. Incoming dt ages the
    /// flash; world time factors are removed for movement/state/animation.
    pub fn update(
        &mut self,
        incoming_dt: f32,
        fight_started: bool,
        time_scale: f32,
        avatars: &[Avatar],
        world: &mut impl Environment,
    ) -> Vec<Command> {
        let mut out = Vec::new();
        self.combat.age_flash(incoming_dt);
        let dt = incoming_dt / time_scale;
        if fight_started && self.state == State::SitIntro {
            self.change(State::RiseIntro, avatars, &mut out);
        }
        if matches!(self.state, State::Appear | State::PreMultiDeath) {
            self.alpha += dt / self.appear_duration;
        }
        if self.combat.begin_defeat() {
            out.push(Command::KillSkeletons);
            self.effect(&mut out, "death_hit", forward(&self.orientation));
            Self::sound(
                &mut out,
                crate::audio::banks::CHARACTERS,
                "boss_death_death",
            );
            self.change(State::DefeatPreSit, avatars, &mut out);
        }
        if !self.combat.defeated() {
            if !avatars.iter().any(|a| a.playing && !a.dead)
                && self.state != State::Idle
                && self.finished()
            {
                self.change(State::Idle, avatars, &mut out);
            } else if self.combat.hit_pending && self.state != State::Hit {
                self.change(State::Hit, avatars, &mut out);
            }
        }
        match self.state {
            State::SitIntro | State::Defeat => {}
            State::RiseIntro => {
                self.movement = forward(&self.orientation);
                self.speed = 1.;
                if self.rise.update(
                    dt,
                    self.controller.has_finished(),
                    self.controller.crossfade_enabled(),
                ) {
                    self.change(State::Prep, avatars, &mut out);
                }
            }
            State::Idle => {
                if avatars.iter().any(|a| a.playing && !a.dead) {
                    self.change(State::Prep, avatars, &mut out);
                }
            }
            State::Prep => {
                self.prep_time -= dt;
                if self.prep_time <= 0. {
                    if let Some(a) = self.avatar(avatars).copied() {
                        let roll = self.below(10) as u8;
                        let ability = policy::choose_ability(
                            roll,
                            self.combat.hit_points(),
                            self.previous.ability(),
                            -99.,
                        );
                        self.position_behind(a, ability, world);
                        self.next = match ability {
                            Ability::RaiseDead => State::RaiseDead,
                            Ability::WarpTime => State::WarpTime,
                            _ => State::Move,
                        };
                        self.change(
                            if ability == Ability::MultiDeath {
                                State::PreMultiDeath
                            } else {
                                State::Appear
                            },
                            avatars,
                            &mut out,
                        );
                    } else {
                        self.prep_time =
                            policy::preparation_delay(self.combat.hit_points(), self.random());
                    }
                }
            }
            State::Appear => {
                if self.finished() {
                    self.change(self.next, avatars, &mut out);
                }
            }
            State::Move => {
                if let Some(a) = self.avatar(avatars).copied() {
                    if self.speed <= 0.1 {
                        self.speed = policy::movement_speed(self.combat.hit_points());
                    }
                    let (d, distance) = flat_delta(a.position, translation(&self.orientation));
                    self.movement = d;
                    self.turn(d, 8., dt);
                    if distance < 2.5 {
                        self.change(State::Attack, avatars, &mut out);
                    }
                } else {
                    self.change(State::Prep, avatars, &mut out);
                }
            }
            State::Attack => {
                if self.speed <= 0.1 {
                    self.speed = 1.;
                }
                if let Some(a) = self.avatar(avatars).copied() {
                    self.chase(a, dt);
                    if self.finished() {
                        if !self.scythe_lifted {
                            self.clip(1, 0.2, false);
                            self.scythe_lifted = true;
                        } else {
                            self.change(State::Prep, avatars, &mut out);
                        }
                    }
                } else {
                    self.change(State::Prep, avatars, &mut out);
                }
            }
            State::Hit => {
                if self.finished() {
                    self.change(State::Prep, avatars, &mut out);
                }
            }
            State::DefeatPreSit => {
                if self.finished() {
                    self.change(State::Defeat, avatars, &mut out);
                }
            }
            State::RaiseDead => {
                if self.pulse.update(dt, world.skeleton_count()) {
                    out.push(Command::RaiseSkeleton);
                }
                if self.finished() {
                    self.change(State::Prep, avatars, &mut out);
                }
            }
            State::WarpTime => {
                if self.finished() {
                    out.push(Command::TimeWarp);
                    self.change(State::Prep, avatars, &mut out);
                }
            }
            State::PreMultiDeath => {
                if let Some(a) = self.avatar(avatars).copied().filter(|a| !a.dead) {
                    if self.clone_controller.has_finished()
                        && !self.clone_controller.crossfade_enabled()
                    {
                        self.change(State::MultiDeath, avatars, &mut out);
                    }
                    out.push(Command::TargetSpeed { target: a.handle });
                    self.ring(a.position, dt);
                } else {
                    self.change(State::Prep, avatars, &mut out);
                }
            }
            State::MultiDeath => self.multi(dt, avatars, &mut out),
        }
        let p = translation(&self.orientation);
        let requested = std::array::from_fn(|i| p[i] + self.movement[i] * self.speed * dt);
        let mut ground_hit = world.ground(requested);
        if let Some(ground) = ground_hit.as_mut() {
            if matches!(self.state, State::SitIntro | State::Defeat) {
                ground[1] += 1.;
            }
            self.orientation[12..15].copy_from_slice(ground);
        }
        if self.update_main {
            self.pose_main(dt);
            // Native uses the local capsule length/2 then smooths toward oPos.Y.
            let mut p = requested;
            p[1] += 0.6;
            p[1] += (ground_hit.map_or(0., |ground| ground[1]) - p[1]) * dt;
            self.body_position = p;
        } else {
            self.body_position = [0., 50., 0.];
        }
        self.afterimages
            .update(dt, self.drawn, self.movement, &self.main_pose);
        out
    }
    pub fn scythe_can_strike(&self) -> bool {
        matches!(self.state, State::Attack | State::MultiDeath)
            && !self.combat.hit_pending
            && self.controller.clip_name() == Some(CLIPS[1])
    }
    /// UpdateBoss suppresses the seated actor's scythe independently of the
    /// body visibility list. Collision hosting must also honor update_main.
    pub fn scythe_visible(&self, slot: usize) -> bool {
        self.drawn
            && self.visible[slot]
            && !(slot == 0 && matches!(self.state, State::SitIntro | State::Defeat))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magicka_sim::{damage::attack, spells::Elements};
    const DT: f32 = 1. / 60.;
    struct Floor {
        count: i32,
    }
    impl Environment for Floor {
        fn nearest(&mut self, p: [f32; 3]) -> Option<(f32, [f32; 3])> {
            Some((0., p))
        }
        fn ground(&mut self, p: [f32; 3]) -> Option<[f32; 3]> {
            Some([p[0], 0., p[2]])
        }
        fn skeleton_count(&self) -> i32 {
            self.count
        }
    }
    fn actor() -> Option<Death> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../target/Magicka/Content");
        if !root.is_dir() {
            return None;
        }
        let model = crate::play_content::load_skinned_model(
            &root,
            &std::path::Path::new(MODEL).with_extension("xnb"),
        )
        .unwrap();
        let mut placement = skinned::IDENTITY;
        placement[14] = -10.;
        Some(Death::new(model, placement, 0x6465_6174_68).unwrap())
    }
    fn avatars() -> [Avatar; 1] {
        [Avatar {
            handle: 1,
            slot: 0,
            position: [0.; 3],
            direction: [0., 0., -1.],
            radius: 0.5,
            playing: true,
            dead: false,
        }]
    }
    fn life(amount: f32) -> Damage {
        Damage {
            attack: attack::DAMAGE,
            element: Elements::LIFE.0,
            amount,
            magnitude: 0.,
        }
    }

    #[test]
    fn afterimage_history_keeps_native_gates_cadence_and_five_draw_slots() {
        let mut a = DeathAfterimages::default();
        let mut pose = ModelPose {
            parent: skinned::IDENTITY,
            local: vec![],
        };
        pose.parent[12] = 1.;
        a.update(0.01, true, [1., 0., 0.], &pose);
        assert_eq!(a.poses[0].as_ref().unwrap().parent[12], 1.);
        assert!(a.poses[1..].iter().all(Option::is_none));
        let alphas = a.draw_alphas();
        for (i, alpha) in alphas.into_iter().enumerate() {
            let expected = 0.333 + (0.8 - i as f32) * (0.333 / 6.);
            assert!((alpha - expected).abs() < 1e-6);
        }
        pose.parent[12] = 2.;
        a.update(0.02, true, [1., 0., 0.], &pose);
        assert_eq!(a.poses[0].as_ref().unwrap().parent[12], 1.);
        let (timer, intensity) = (a.timer, a.intensity);
        for (drawn, movement) in [(true, [0., 100., 0.]), (false, [1., 0., 0.])] {
            a.update(5., drawn, movement, &pose);
            assert!(!a.drawn);
            assert_eq!((a.timer, a.intensity), (timer, intensity));
        }
        a.update(0.03, true, [1., 0., 0.], &pose);
        assert!(a.drawn);
        assert_eq!(a.poses[0].as_ref().unwrap().parent[12], 2.);
        assert_eq!(a.poses[1].as_ref().unwrap().parent[12], 1.);
        pose.parent[12] = 3.;
        a.update(0.2, true, [1., 0., 0.], &pose);
        assert!(a.poses[..4]
            .iter()
            .all(|p| p.as_ref().unwrap().parent[12] == 3.));
        assert_eq!(a.poses[4].as_ref().unwrap().parent[12], 2.);
        let (timer, intensity) = (a.timer, a.intensity);
        a.clear_poses();
        assert!(a.poses.iter().all(Option::is_none));
        assert_eq!((a.timer, a.intensity), (timer, intensity));
    }

    #[test]
    fn moving_actor_records_world_pose_and_native_exit_boundaries() {
        let Some(mut d) = actor() else { return };
        let players = avatars();
        let mut floor = Floor { count: 6 };
        // RiseIntro moves naturally and emits history from the sampled main rig.
        d.update(DT, true, 0.5, &players, &mut floor);
        assert!(d.afterimages.drawn);
        let pose = d.afterimages.poses[0].as_ref().unwrap();
        assert_eq!(pose.parent, d.poses[0].parent);
        assert_eq!(pose.local, d.poses[0].local);
        assert!((d.afterimages.timer - (0.05 - DT / 0.5)).abs() < 1e-6);
        let before = pose.parent;
        for _ in 0..4 {
            d.update(DT, true, 1., &players, &mut floor);
        }
        assert_ne!(d.afterimages.poses[0].as_ref().unwrap().parent, before);
        assert!(d.afterimages.poses[1].is_some());
        d.change(State::Attack, &players, &mut Vec::new());
        d.change(State::Prep, &players, &mut Vec::new());
        assert!(d.afterimages.poses.iter().all(Option::is_none));

        let mut commands = Vec::new();
        d.change(State::PreMultiDeath, &players, &mut commands);
        assert!(commands
            .iter()
            .any(|c| matches!(c, Command::BeginClones { .. })));
        commands.clear();
        d.change(State::Prep, &players, &mut commands);
        assert!(
            !commands
                .iter()
                .any(|c| matches!(c, Command::EndClones { .. })),
            "native PreMultiDeath.OnExit is empty, even on early abort"
        );
        d.change(State::PreMultiDeath, &players, &mut commands);
        d.change(State::MultiDeath, &players, &mut commands);
        d.afterimages.update(DT, true, [1., 0., 0.], &d.main_pose);
        commands.clear();
        d.change(State::Prep, &players, &mut commands);
        assert!(d.afterimages.poses.iter().all(Option::is_none));
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c, Command::EndClones { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn death_retail_rig_advances_intro_and_attack_clips_without_timed_victory() {
        let Some(mut d) = actor() else {
            return;
        };
        let players = avatars();
        let mut floor = Floor { count: 6 };
        assert_eq!(d.state, State::SitIntro);
        assert_eq!(d.controller.clip_name(), Some("sit_talk"));
        assert!(!d.scythe_visible(0));
        let mut states = std::collections::HashSet::new();
        let mut attacks = std::collections::HashSet::new();
        for _ in 0..3600 {
            d.update(DT, true, 1., &players, &mut floor);
            states.insert(format!("{:?}", d.state));
            if d.state == State::Attack {
                attacks.insert(d.controller.clip_name().unwrap().to_owned());
            }
        }
        for required in ["RiseIntro", "Prep", "Appear", "Move", "Attack", "RaiseDead"] {
            assert!(states.contains(required), "missing {required}: {states:?}");
        }
        assert!(
            attacks.contains("attack_scythe_rise") && attacks.contains("attack_scythe_fall"),
            "{attacks:?}"
        );
        assert_eq!(d.combat.hit_points(), 10000.);
        assert!(!d.combat.dead());
        assert!(!d.poses[0].local.is_empty());
        assert!(d.scythes[0].iter().all(|v| v.is_finite()));
    }

    #[test]
    fn death_life_interrupts_real_attack_and_defeat_waits_for_original_clip() {
        let Some(mut d) = actor() else {
            return;
        };
        let players = avatars();
        let mut floor = Floor { count: 6 };
        d.target = Some(1);
        d.change(State::Attack, &players, &mut Vec::new());
        let (result, events) = d.damage(life(100.), 0);
        assert_eq!(result, magicka_sim::character_impact::result::DEFLECTED);
        assert!(events.contains(&Command::Hit {
            amount: 100.,
            first: true,
            position: [0., 2.2, -10.],
        }));
        d.update(DT, true, 1., &players, &mut floor);
        assert_eq!(d.state, State::Hit);
        for _ in 0..600 {
            d.update(DT, true, 1., &players, &mut floor);
            if d.state == State::Prep {
                break;
            }
        }
        assert_eq!(d.state, State::Prep);
        assert!(!d.combat.hit_pending);
        d.change(State::Appear, &players, &mut Vec::new());
        assert!(
            d.damage(life(20000.), 15).1.is_empty(),
            "appearing Death is ethereal"
        );
        d.change(State::Attack, &players, &mut Vec::new());
        d.damage(life(20000.), 15);
        let commands = d.update(DT, true, 1., &players, &mut floor);
        assert!(commands.contains(&Command::KillSkeletons));
        assert_eq!(d.state, State::DefeatPreSit);
        assert_eq!(d.combat.hit_points(), 1.);
        for _ in 0..600 {
            d.update(DT, true, 1., &players, &mut floor);
            if d.state == State::Defeat {
                break;
            }
        }
        assert_eq!(d.state, State::Defeat);
        assert_eq!(d.combat.hit_points(), 0.);
        assert!(!d.combat.dead());
        // OnEnter changes health immediately; the seated clip becomes the
        // active clip only after the native 0.2-second crossfade completes.
        for _ in 0..20 {
            d.update(DT, true, 1., &players, &mut floor);
        }
        assert_eq!(d.controller.clip_name(), Some("sit_talk"));
        assert!(!d.scythe_visible(0));
    }

    #[test]
    fn death_clone_ring_uses_all_original_poses_and_restores_on_real_hit() {
        let Some(mut d) = actor() else {
            return;
        };
        let players = avatars();
        let mut floor = Floor { count: 0 };
        d.damage(life(3000.), 15);
        d.combat.exit_hit();
        d.target = Some(1);
        let mut commands = Vec::new();
        d.change(State::PreMultiDeath, &players, &mut commands);
        assert_eq!(d.clone_count, 8);
        assert!(commands.contains(&Command::BeginClones { target: 1 }));
        d.update(DT, true, 1., &players, &mut floor);
        for i in 0..8 {
            assert!(d.visible[i]);
            assert_eq!(d.poses[i].local.len(), d.skeleton.skeleton.bones.len());
            let p = translation(&d.poses[i].parent);
            assert!(((p[0] * p[0] + p[2] * p[2]).sqrt() - 10.).abs() < 0.001);
            assert!(d.scythes[i].iter().all(|v| v.is_finite()));
        }
        let mut reached_real = false;
        for _ in 0..7200 {
            let commands = d.update(DT, true, 1., &players, &mut floor);
            if commands.contains(&Command::RevealReal) {
                reached_real = true;
                break;
            }
        }
        assert!(reached_real, "the real attacker must eventually approach");
        assert_eq!(d.visible.iter().filter(|&&v| v).count(), 1);
        assert!(!d.ethereal);
        assert!(!d.fake_attack);
        d.damage(life(1.), 15);
        let commands = d.update(DT, true, 1., &players, &mut floor);
        assert_eq!(d.state, State::Hit);
        assert!(commands.contains(&Command::EndClones { target: Some(1) }));
        assert_eq!(d.current_slot, 0);
        assert!(d.update_main);
        assert!(d.visible[0]);
        assert!(d.visible[1..].iter().all(|v| !*v));
    }

    #[test]
    fn death_summon_and_timewarp_commands_follow_their_animation_gates() {
        let Some(mut d) = actor() else {
            return;
        };
        let players = avatars();
        let mut floor = Floor { count: 0 };
        d.target = Some(1);
        d.change(State::RaiseDead, &players, &mut Vec::new());
        let mut summons = 0;
        for _ in 0..600 {
            let events = d.update(DT, true, 1., &players, &mut floor);
            summons += events
                .iter()
                .filter(|e| **e == Command::RaiseSkeleton)
                .count();
            if d.state == State::Prep {
                break;
            }
        }
        assert!(summons > 0);
        assert_eq!(d.controller.speed, 1.);
        d.change(State::WarpTime, &players, &mut Vec::new());
        assert!(!d
            .update(DT, true, 1., &players, &mut floor)
            .contains(&Command::TimeWarp));
        let mut warps = 0;
        for _ in 0..600 {
            let events = d.update(DT, true, 1., &players, &mut floor);
            warps += events.iter().filter(|e| **e == Command::TimeWarp).count();
            if d.state == State::Prep {
                break;
            }
        }
        assert_eq!(warps, 1);
        assert_eq!(d.state, State::Prep);
    }

    #[test]
    fn death_state_clock_removes_world_scaling_but_flash_uses_incoming_time() {
        let Some(mut d) = actor() else {
            return;
        };
        let players = avatars();
        let mut floor = Floor { count: 6 };
        d.damage(life(1.), 15);
        let mut control = d.clone();
        d.update(0.01, true, 0.5, &players, &mut floor);
        control.update(0.02, true, 1., &players, &mut floor);
        assert_eq!(d.controller.time, control.controller.time);
        assert_eq!(d.orientation, control.orientation);
        assert!((d.combat.flash_timer - 0.09).abs() < 1e-6);
        assert!((control.combat.flash_timer - 0.08).abs() < 1e-6);
    }

    #[test]
    fn death_clone_attacker_uses_current_target_position_for_all_ring_sizes() {
        for (damage, count) in [(3000., 8), (5000., 12), (7000., 16)] {
            let Some(mut d) = actor() else {
                return;
            };
            let mut players = avatars();
            let mut floor = Floor { count: 0 };
            d.damage(life(damage), 15);
            d.combat.exit_hit();
            d.target = Some(1);
            d.change(State::PreMultiDeath, &players, &mut Vec::new());
            d.update(0., true, 1., &players, &mut floor);
            assert_eq!(d.clone_count, count);
            assert!(d.poses[..count].iter().all(|pose| !pose.local.is_empty()));
            d.change(State::MultiDeath, &players, &mut Vec::new());
            players[0].position = [20., 0., 0.];
            d.update(0., true, 1., &players, &mut floor);
            let p = translation(&d.orientation);
            let radius = ((p[0] - 20.).powi(2) + p[2] * p[2]).sqrt();
            assert!(
                (radius - 10.).abs() < 0.001,
                "{count} clones: stale attacker {p:?}"
            );
        }
    }

    #[test]
    fn a_large_party_divides_the_life_damage_death_takes() {
        let Some(death) = actor() else { return };
        let actor = std::sync::Arc::new(std::sync::Mutex::new(death));
        let mut zone = crate::physics_damage::PhysicsDamage::boss_zone(
            0,
            magicka_content::physics::CollisionBox {
                name: String::new(),
                position: [0.; 3],
                sides: [0.; 3],
                orientation: [0., 0., 0., 1.],
            },
        );
        zone.death_boss = Some(actor.clone());
        zone.death_damage_divisor = 4.0;
        let before = actor.lock().unwrap().combat.hit_points();
        let hit = magicka_sim::damage::Damage {
            attack: magicka_sim::damage::attack::DAMAGE,
            element: magicka_sim::spells::Elements::LIFE.0,
            amount: 400.0,
            magnitude: 1.0,
        };
        zone.deliver(&[hit], 15, &mut Vec::new());
        assert_eq!(actor.lock().unwrap().combat.hit_points(), before - 100.0);
    }
}
