//! Assatur, the final boss of EndofWorld2 (ew2_s2). A port of
//! Magicka.GameLogic.Entities.Bosses.Assatur (research/boss-assatur.md):
//! the floating caster's BattleState weighting and its fifteen cast states.
//! Scene, spell and effect operations are returned to the host as commands.
use crate::{
    animation_controller::AnimationController, boss_fafnir::ModelPose,
    character_pose::CharacterSkeleton,
};
use magicka_content::skinned::{self, Mat, Resolved};
use magicka_sim::{
    boss_status::{BossEvent, BossStatus},
    damage::{CastType, Damage},
    spells::{Elements as E, MagickType as M, Spell, SpellType},
    status::Status,
};

pub const MODEL: &str = "Models/Bosses/assatur/assatur";
pub const STAFF: &str = "Models/Bosses/assatur/assatur_staff";
pub const MAX_HP: f32 = 50000.;
/// mTentaclesIndex / mTentacleBones: local poses copied from the second
/// (always idle) controller.
const TENTACLES: std::ops::Range<usize> = 44..78;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Battle,
    MagickMissiles,
    Vortex,
    MeteorShower,
    Lightning,
    Life,
    Blizzard,
    Water,
    TimeWarp,
    ArcaneBlade,
    Shield,
    StaffBeam,
    Nullify,
    Barrier,
    Intro,
    Dead,
}
/// The States enum order (BattleState scans indices 0..16).
const STATES: [State; 16] = [
    State::Battle,
    State::MagickMissiles,
    State::Vortex,
    State::MeteorShower,
    State::Lightning,
    State::Life,
    State::Blizzard,
    State::Water,
    State::TimeWarp,
    State::ArcaneBlade,
    State::Shield,
    State::StaffBeam,
    State::Nullify,
    State::Barrier,
    State::Intro,
    State::Dead,
];

/// ANIMATION_TIMERS by Animations index.
fn timers(clip: &str) -> &'static [f32] {
    match clip {
        "cast_mm_left" | "cast_mm_right" => &[13. / 33., 28. / 33.],
        "cast_mm_sides" => &[0.3125, 0.625],
        "cast_emperor" => &[13. / 32., 1.],
        "cast_self" => &[1. / 6., 1.],
        "cast_spray" => &[0., 1.],
        "cast_blade" => &[0.25, 0.675, 0.45, 0.55],
        "cast_shield" => &[0.25],
        "cast_staff" => &[0.1, 0.15, 0.9, 0.95],
        _ => &[],
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Trigger(&'static str),
    Magick(M),
    /// EffectManager.StartEffect at a world transform (one-shot).
    Effect {
        name: &'static str,
        transform: Mat,
    },
    Cue {
        bank: u16,
        name: &'static str,
    },
    StopCue(&'static str),
    CameraShake {
        magnitude: f32,
        time: f32,
    },
    /// CastMMState.SpawnMissile: a homing arcane missile at the target.
    Missile {
        position: [f32; 3],
        velocity: [f32; 3],
        tolerance: f32,
    },
    /// Two LightningBolt.Cast at the target (cast and weapon sources).
    Lightning {
        damages: [Damage; 5],
    },
    /// CastBlade's ArcIntersect sweep (range 18, 81°, height 2).
    Blade {
        position: [f32; 3],
        direction: [f32; 3],
        damages: [Damage; 5],
    },
    /// CastBlade's arcane_detonation where the blade meets the scene.
    BladeDetonation {
        position: [f32; 3],
        direction: [f32; 3],
    },
    /// CastStaffBeamState's Railgun (Arcane 5, Beam/Force), steered.
    Beam {
        position: [f32; 3],
        direction: [f32; 3],
    },
    StopBeam,
    /// CastWater: Spell(Water 5).Cast(staff, Force) and its stop.
    Water,
    StopSpell,
    /// CastShield: a DISC Shield, radius 5, 5000 HP, at the boss.
    Shield {
        position: [f32; 3],
        direction: [f32; 3],
    },
    /// mProtectiveShield.Damage(100) every 0.2 s after its 15 s.
    ShieldDecay,
    Boss(BossEvent),
}

#[derive(Clone, Debug, Default)]
pub struct Inputs {
    pub fight_started: bool,
    /// The current target's position (None when it has no body).
    pub target: Option<[f32; 3]>,
    pub target_dead: bool,
    /// Playing, live avatars (handles) for SelectTarget, slot order.
    pub players: Vec<Option<u16>>,
    pub vortex: bool,
    pub meteor_shower: bool,
    pub blizzard: bool,
    pub time_warp: bool,
    /// The protective shield exists and is alive.
    pub shield_alive: bool,
    /// EntityManager.Shields.Count plus Shield self-shields within 50 m.
    pub shields: f32,
    /// GetEntities(position, 10, ignore protected).Count.
    pub near: f32,
    /// GetEntities(position, 50, ignore / keep protected).Count.
    pub far_unprotected: f32,
    pub far_all: f32,
}

#[derive(Clone, Debug, Default)]
struct StateData {
    casted: bool,
    timer: f32,
    cast_dir: u8,
    missile_delay: f32,
    cue: bool,
    blade: bool,
    blade_hit: f32,
    beam: bool,
}

#[derive(Clone, Debug)]
pub struct Assatur {
    pub status: BossStatus,
    pub state: State,
    last_state: State,
    pub skeleton: CharacterSkeleton,
    pub controller: AnimationController,
    tentacles: AnimationController,
    pub pose: ModelPose,
    /// mOrientation (the floating body) and mTargetOrientation.
    pub orientation: Mat,
    target_orientation: Mat,
    pub target: Option<u16>,
    target_position: [f32; 3],
    targetting_power: f32,
    cast_cooldown: f32,
    pub shield_timer: f32,
    pub damage_flash: f32,
    pub death_scale: f32,
    pub dead: bool,
    /// mUseRailStaffOrientation / mRailStaffOrientation.
    pub rail_staff: Option<Mat>,
    /// Effects that follow the boss: (key, effect, transform).
    pub tracked: Vec<(&'static str, &'static str, Mat)>,
    spine: Mat,
    magick_effect: Mat,
    data: StateData,
    death_start: f32,
    rng: u64,
}

impl Assatur {
    pub fn new(model: Resolved, orientation: Mat) -> Result<Self, String> {
        let skeleton = CharacterSkeleton::new(model.skeleton, model.clips);
        for clip in [
            "cast_blade",
            "cast_blizzard",
            "cast_ground",
            "idle",
            "cast_self",
            "cast_emperor",
            "cast_sky",
            "cast_mm_left",
            "cast_mm_right",
            "cast_mm_sides",
            "cast_mm_prepare",
            "cast_nullify",
            "cast_shield",
            "cast_staff",
            "cast_spray",
            "cast_focus",
            "die",
        ] {
            if !skeleton.clip_durations().contains_key(clip) {
                return Err(format!("Assatur missing {clip}"));
            }
        }
        let mut status = BossStatus::default();
        status.hit_points = MAX_HP;
        status.max_hit_points = MAX_HP;
        for r in &mut status.resistances {
            r.modifier = 0.;
            r.multiplier = if r.against == E::WATER {
                0.
            } else if r.against == E::COLD {
                0.05
            } else {
                1.
            };
        }
        // Initialize: the body starts 40 m above its spawn and sinks to it.
        let mut start = orientation;
        start[13] += 40.;
        let mut a = Self {
            status,
            state: State::Intro,
            last_state: State::Intro,
            skeleton,
            controller: Default::default(),
            tentacles: Default::default(),
            pose: Default::default(),
            orientation: start,
            target_orientation: orientation,
            target: None,
            target_position: [orientation[12], orientation[13], orientation[14]],
            targetting_power: 1.,
            cast_cooldown: 0.,
            shield_timer: 0.,
            damage_flash: 0.,
            death_scale: 1.,
            dead: false,
            rail_staff: None,
            tracked: Vec::new(),
            spine: orientation,
            magick_effect: orientation,
            data: Default::default(),
            death_start: 0.,
            rng: 0x61_73_73_61_74_75_72,
        };
        // IntroState.OnEnter: both controllers loop idle.
        let idle = a.duration("idle");
        a.controller.start_named_clip("idle", idle, true);
        a.tentacles.start_named_clip("idle", idle, true);
        a.data.timer = 3.;
        a.update_pose(0.);
        Ok(a)
    }
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u32 << 24) as f32
    }
    fn duration(&self, clip: &str) -> f32 {
        self.skeleton.clip_durations()[clip]
    }
    fn crossfade(&mut self, clip: &str, fade: f32, looping: bool) {
        let d = self.duration(clip);
        self.controller.crossfade_named_clip(clip, d, fade, looping);
    }
    fn normalized(&self) -> f32 {
        self.controller.time / self.controller.clip_duration().max(f32::EPSILON)
    }
    fn timer(&self, i: usize) -> f32 {
        self.controller
            .clip_name()
            .map_or(0., |c| timers(c).get(i).copied().unwrap_or(0.))
    }
    pub fn health(&self) -> (f32, f32) {
        (self.status.hit_points, MAX_HP)
    }
    pub fn position(&self) -> [f32; 3] {
        [
            self.orientation[12],
            self.orientation[13],
            self.orientation[14],
        ]
    }
    fn forward(m: &Mat) -> [f32; 3] {
        [-m[8], -m[9], -m[10]]
    }
    /// BindJoint transform: RotationY(π)·Bind·SkinnedBoneTransforms[bone].
    pub fn joint(&self, bone: &str) -> Option<Mat> {
        let index = self.skeleton.bone(bone)?;
        let bind =
            crate::live_collision::inverse(&self.skeleton.skeleton.bones[index].inverse_bind)
                .ok()?;
        let yrot = skinned::pose_matrix([0.; 3], [0., 1., 0., 0.], [1.; 3]);
        let matrices = self
            .skeleton
            .skinned_from_local(&self.pose.local, &self.pose.parent);
        Some(skinned::mul(
            &skinned::mul(&yrot, &bind),
            matrices.get(index)?,
        ))
    }
    /// Assatur.StaffOrientation (the joint1 bone).
    pub fn staff(&self) -> Mat {
        self.joint("joint1").unwrap_or(self.orientation)
    }
    pub fn cast_source(&self) -> Mat {
        self.joint("leftattach").unwrap_or(self.orientation)
    }
    pub fn weapon_source(&self) -> Mat {
        self.joint("rightattach").unwrap_or(self.orientation)
    }
    fn select_target(&mut self, inputs: &Inputs) {
        // Four slots in the original; every slot of a larger lobby.
        let n = inputs.players.len().max(1);
        let start = ((self.random() * n as f32) as usize).min(n - 1);
        for i in 0..n {
            if let Some(h) = inputs.players.get((i + start) % n).copied().flatten() {
                self.target = Some(h);
                break;
            }
        }
    }

    pub fn change(&mut self, state: State, inputs: &Inputs, out: &mut Vec<Command>) {
        self.exit(out);
        self.last_state = self.state;
        self.state = state;
        self.data = StateData::default();
        match state {
            State::Intro => {
                let idle = self.duration("idle");
                self.controller.start_named_clip("idle", idle, true);
                self.tentacles.start_named_clip("idle", idle, true);
                self.data.timer = 3.;
            }
            State::Battle => {
                self.crossfade("idle", 0.4, true);
                self.select_target(inputs);
            }
            State::Nullify => self.crossfade("cast_nullify", 0.3, false),
            State::Blizzard => {
                self.crossfade("cast_blizzard", 0.4, false);
                self.generic_magick(out);
            }
            State::MeteorShower => {
                self.crossfade("cast_sky", 0.4, false);
                self.generic_magick(out);
                out.push(Command::Magick(M::MeteorS));
            }
            State::Vortex => {
                self.crossfade("cast_focus", 0.4, false);
                self.generic_magick(out);
            }
            State::TimeWarp => {
                self.crossfade("cast_ground", 0.4, false);
                out.push(Command::Magick(M::TimeWarp));
            }
            State::ArcaneBlade => self.crossfade("cast_blade", 0.4, false),
            State::Life => self.crossfade("cast_self", 0.4, false),
            State::MagickMissiles => {
                self.data.missile_delay = 0.15;
                self.crossfade("cast_mm_prepare", 0.5, false);
            }
            State::Lightning => self.crossfade("cast_emperor", 0.4, false),
            State::Shield => self.crossfade("cast_shield", 0.4, false),
            State::StaffBeam => self.crossfade("cast_staff", 0.4, false),
            State::Water | State::Barrier => self.crossfade("cast_spray", 0.4, false),
            State::Dead => {
                out.push(Command::Trigger("assatur_defeated"));
                out.push(Command::StopSpell);
                self.death_start = self.orientation[13];
                self.crossfade("die", 0.2, false);
                let mut t = self.orientation;
                t[12..15].copy_from_slice(&self.position());
                self.tracked.retain(|e| e.0 != "death");
                self.tracked.push(("death", "assatur_death", t));
            }
        }
    }
    /// The leaving state's OnExit.
    fn exit(&mut self, out: &mut Vec<Command>) {
        match self.state {
            State::Nullify => self.tracked.retain(|e| e.0 != "nullify"),
            State::ArcaneBlade => out.push(Command::StopCue("spell_arcane_ray_stage2")),
            State::MagickMissiles => out.push(Command::StopCue("magick_vortex")),
            State::Lightning => out.push(Command::StopCue("spell_lightning_spray")),
            State::StaffBeam => {
                if self.data.beam {
                    out.push(Command::StopBeam);
                }
                out.push(Command::StopSpell);
                self.rail_staff = None;
            }
            State::Water => out.push(Command::StopSpell),
            _ => {}
        }
    }
    /// EffectManager.StartEffect(GENERIC_MAGICK, mMagickEffect). The native
    /// reference follows mMagickEffect while the state lasts; the slowly
    /// floating boss makes a one-shot at the cast transform equivalent.
    fn generic_magick(&self, out: &mut Vec<Command>) {
        out.push(Command::Effect {
            name: "magick_generic",
            transform: self.magick_effect,
        });
    }

    /// Assatur.Damage(iPartIndex, Damage, ...): BossCollisionZone attackers
    /// are ignored by the host; a hit interrupts CastLife.
    pub fn receive_damage(
        &mut self,
        d: Damage,
        position: [f32; 3],
        features: u8,
        inputs: &Inputs,
    ) -> Vec<Command> {
        let mut out = Vec::new();
        let mut events = Vec::new();
        let result = self
            .status
            .damage(d, self.dead, position, features, &mut events);
        out.extend(events.into_iter().map(Command::Boss));
        if result & 3 != 0 {
            self.status.time_since_last_damage = 0.;
            if self.state == State::Life {
                self.change(State::Battle, inputs, &mut out);
            }
            self.damage_flash = 0.1;
        }
        out
    }
    /// ScriptMessage(KillAssatur).
    pub fn message(&mut self, name: &str, inputs: &Inputs) -> Vec<Command> {
        let mut out = Vec::new();
        if name == "KillAssatur" {
            self.change(State::Dead, inputs, &mut out);
        }
        out
    }

    /// BattleState.OnUpdate: the highest random + situational weight wins;
    /// the previous state, Battle and Dead are skipped.
    fn decide(&mut self, inputs: &Inputs, out: &mut Vec<Command>) {
        let mut best = 0.;
        let mut chosen = None;
        let health = self.status.hit_points / MAX_HP;
        for state in STATES {
            let mut w = self.random();
            if state == self.last_state || state == State::Dead || state == State::Battle {
                continue;
            }
            w += match state {
                State::MagickMissiles => {
                    if inputs.shield_alive {
                        0.5
                    } else {
                        1.
                    }
                }
                State::Vortex => {
                    if inputs.vortex {
                        -1.
                    } else {
                        0.5
                    }
                }
                State::MeteorShower => {
                    if inputs.meteor_shower {
                        -1.
                    } else {
                        0.5
                    }
                }
                State::Lightning => 0.5,
                State::Life => ((1. - health) * 2.).max(0.),
                State::Blizzard => {
                    if inputs.blizzard {
                        -1.
                    } else {
                        0.5
                    }
                }
                State::Water => -10.,
                State::TimeWarp => {
                    if inputs.time_warp {
                        -1.
                    } else {
                        0.25
                    }
                }
                State::ArcaneBlade => {
                    let n = inputs.near - 2.;
                    if n > 0. {
                        1. / n
                    } else {
                        0.
                    }
                }
                State::Shield => (1. - health) * 0.75 + 0.25,
                State::StaffBeam => {
                    if inputs.far_all > 0. {
                        1. - inputs.far_unprotected / inputs.far_all
                    } else {
                        0.
                    }
                }
                State::Nullify => inputs.shields - 1. - if inputs.shield_alive { 1. } else { 0. },
                _ => 0.,
            };
            if w > best {
                best = w;
                chosen = Some(state);
            }
        }
        if let Some(state) = chosen {
            self.change(state, inputs, out);
        }
    }

    pub fn update(&mut self, dt: f32, inputs: Inputs) -> Vec<Command> {
        let mut out = Vec::new();
        let mut events = Vec::new();
        self.status.update_damage(dt, self.dead, &mut events);
        out.extend(events.into_iter().map(Command::Boss));
        for (amount, element) in self.status.update_statuses(dt, self.dead) {
            self.status.status_damage(amount, element);
        }
        self.status.time_since_last_damage += dt;
        self.shield_timer -= dt;
        if self.status.hit_points <= 0. && self.state != State::Dead {
            self.change(State::Dead, &inputs, &mut out);
        }
        self.orientation[13] += (self.target_orientation[13] - self.orientation[13]) * dt * 2.;
        if let Some(p) = inputs.target {
            for (t, p) in self.target_position.iter_mut().zip(p) {
                *t += (p - *t) * dt * self.targetting_power;
            }
        }
        if self.shield_timer <= 0. && inputs.shield_alive {
            self.shield_timer += 0.2;
            out.push(Command::ShieldDecay);
        }
        self.magick_effect = self.spine;
        self.magick_effect[13] += 3.;
        self.magick_effect[14] += 3.;
        if inputs.fight_started {
            self.update_state(dt, &inputs, &mut out);
        }
        self.controller.speed = if self.status.has(Status::Frozen) {
            0.
        } else if self.status.has(Status::Cold) {
            0.2
        } else {
            1.
        };
        self.update_pose(dt);
        self.damage_flash = (self.damage_flash - dt).max(0.);
        self.spine = self.joint("spinelower").unwrap_or(self.orientation);
        out
    }

    fn update_pose(&mut self, dt: f32) {
        self.controller.update(dt);
        self.tentacles.update(dt);
        // The model is drawn 5 m below mOrientation, scaled by mDeathScale.
        let mut parent = self.orientation;
        for v in &mut parent[..12] {
            *v *= self.death_scale;
        }
        parent[13] -= 5.;
        let mut local = self.skeleton.local_poses(self.controller.pose_sample());
        let tentacles = self.skeleton.local_poses(self.tentacles.pose_sample());
        for i in TENTACLES {
            if let (Some(dst), Some(src)) = (local.get_mut(i), tentacles.get(i)) {
                *dst = *src;
            }
        }
        self.pose = ModelPose { parent, local };
    }

    fn update_state(&mut self, dt: f32, inputs: &Inputs, out: &mut Vec<Command>) {
        let fading = self.controller.crossfade_enabled();
        let finished = self.controller.has_finished();
        let n = self.normalized();
        match self.state {
            State::Intro => {
                if self.data.timer <= 0. {
                    self.change(State::Battle, inputs, out);
                }
                self.data.timer -= dt;
            }
            State::Battle => self.decide(inputs, out),
            State::Nullify => {
                if fading {
                    return;
                }
                let staff = self.staff();
                if let Some(e) = self.tracked.iter_mut().find(|e| e.0 == "nullify") {
                    e.2 = staff;
                }
                if self.controller.time > 0.5 && !self.data.casted {
                    self.data.casted = true;
                    self.tracked.push(("nullify", "assatur_nullify", staff));
                    out.push(Command::CameraShake {
                        magnitude: 0.3,
                        time: 1.5,
                    });
                    out.push(Command::Magick(M::Nullify));
                }
                if finished {
                    self.change(State::Battle, inputs, out);
                }
            }
            State::Blizzard | State::Vortex => {
                if fading {
                    return;
                }
                let at = if self.state == State::Blizzard {
                    2.
                } else {
                    1.
                };
                if self.controller.time > at && !self.data.casted {
                    self.data.casted = true;
                    out.push(Command::Magick(if self.state == State::Blizzard {
                        M::Blizzard
                    } else {
                        M::Vortex
                    }));
                }
                if finished {
                    self.change(State::Battle, inputs, out);
                }
            }
            State::MeteorShower | State::TimeWarp => {
                if finished && !fading {
                    self.change(State::Battle, inputs, out);
                }
            }
            State::ArcaneBlade => self.update_blade(dt, n, inputs, out),
            State::Life => {
                if fading {
                    return;
                }
                if n >= self.timer(0) && n <= self.timer(1) {
                    if !self.data.casted {
                        self.data.casted = true;
                        let mut t = skinned::pose_matrix([0.; 3], [0., 0., 0., 1.], [1.; 3]);
                        t[12..15].copy_from_slice(&self.spine[12..15]);
                        // Direction −UnitZ: CreateWorld's default facing.
                        out.push(Command::Effect {
                            name: "assatur_life",
                            transform: t,
                        });
                    }
                    if self.data.timer <= 0. {
                        // Damage(1, −1000, Life): the status-damage path.
                        self.status.status_damage(-1000., E::LIFE);
                        self.data.timer += 0.25;
                    }
                }
                self.data.timer -= dt;
                if finished {
                    self.change(State::Battle, inputs, out);
                }
            }
            State::MagickMissiles => self.update_missiles(dt, inputs, out),
            State::Lightning => {
                if fading {
                    return;
                }
                self.data.timer -= dt;
                if finished {
                    self.change(State::Battle, inputs, out);
                    return;
                }
                let (t0, t1) = (self.timer(0), self.timer(1));
                if n >= t0 && !self.data.cue {
                    self.data.cue = true;
                    out.push(Command::Cue {
                        bank: crate::audio::banks::SPELLS,
                        name: "spell_lightning_spray",
                    });
                }
                if n >= t0 && n <= t1 && self.data.timer <= 0. && inputs.target.is_some() {
                    let mut spell = Spell::default();
                    spell.element |= E::LIGHTNING;
                    spell.mag[E::LIGHTNING.index() as usize] = 1.;
                    let mut damages = damage5(&spell, SpellType::Lightning, CastType::Force);
                    for d in &mut damages {
                        d.magnitude *= (t1 - t0) * 0.15 * 0.5;
                    }
                    out.push(Command::Lightning { damages });
                    self.data.timer += 0.15;
                }
            }
            State::Shield => {
                if fading {
                    return;
                }
                if n >= self.timer(0) && !self.data.casted && !inputs.shield_alive {
                    self.data.casted = true;
                    out.push(Command::Shield {
                        position: self.position(),
                        direction: Self::forward(&self.orientation),
                    });
                    self.shield_timer = 15.;
                }
                if finished {
                    self.change(State::Battle, inputs, out);
                }
            }
            State::StaffBeam => self.update_beam(inputs, out),
            State::Water => {
                if fading {
                    return;
                }
                if n >= self.timer(0) && !self.data.casted {
                    self.data.casted = true;
                    out.push(Command::Water);
                } else if n >= self.timer(1) && self.data.casted {
                    out.push(Command::StopSpell);
                    self.data.casted = false;
                }
                if finished {
                    self.change(State::Battle, inputs, out);
                }
            }
            State::Barrier => {
                // CastBarrier only probes the chasm with a segment; no spell.
                if !fading && finished {
                    self.change(State::Battle, inputs, out);
                }
            }
            State::Dead => {
                let n = n.clamp(0., 1.);
                self.orientation[13] = self.death_start + 4. * n;
                self.target_orientation[13] = self.orientation[13];
                let mut t = self.orientation;
                t[13] -= n * 4.;
                if let Some(e) = self.tracked.iter_mut().find(|e| e.0 == "death") {
                    e.2 = t;
                }
                if finished && !fading && !self.dead {
                    self.dead = true;
                    out.push(Command::Trigger("assatur_killed"));
                }
                self.death_scale = 1.001 - n;
            }
        }
    }

    fn update_missiles(&mut self, dt: f32, inputs: &Inputs, out: &mut Vec<Command>) {
        if self.controller.crossfade_enabled() || self.target.is_none() {
            return;
        }
        let Some(target) = inputs.target else {
            return;
        };
        if self.data.cast_dir == 0 {
            if self.controller.has_finished() {
                let p = self.position();
                let mut to = [target[0] - p[0], 0., target[2] - p[2]];
                let l = to[0].hypot(to[2]).max(1e-6);
                to = to.map(|v| v / l);
                let mut f = Self::forward(&self.orientation);
                f[1] = 0.;
                let fl = f[0].hypot(f[2]).max(1e-6);
                f = f.map(|v| v / fl);
                let mut angle = (f[0] * to[0] + f[2] * to[2]).clamp(-1., 1.).acos();
                // Cross(forward, toTarget).Y <= 0 negates the angle.
                if f[2] * to[0] - f[0] * to[2] <= 0. {
                    angle = -angle;
                }
                let eighth = std::f32::consts::PI / 8.;
                let (clip, dir) = if angle >= eighth {
                    ("cast_mm_left", 2)
                } else if angle <= -eighth {
                    ("cast_mm_right", 3)
                } else {
                    ("cast_mm_sides", 1)
                };
                self.crossfade(clip, 0.5, false);
                self.data.cast_dir = dir;
            }
            return;
        }
        if self.controller.has_finished() {
            self.data.cast_dir = 0;
            self.change(State::Battle, inputs, out);
            return;
        }
        if self.data.missile_delay <= 0. {
            let n = self.normalized();
            let (t0, t1) = (self.timer(0), self.timer(1));
            if n >= timers("cast_mm_sides")[0] && !self.data.cue {
                self.data.cue = true;
                out.push(Command::Cue {
                    bank: crate::audio::banks::SPELLS,
                    name: "magick_vortex",
                });
            }
            if n >= t0 && n <= t1 {
                let sources: &[(Mat, f32)] = &match self.data.cast_dir {
                    1 => vec![(self.cast_source(), -0.75), (self.weapon_source(), -0.75)],
                    2 => vec![(self.cast_source(), -0.25)],
                    _ => vec![(self.weapon_source(), -0.25)],
                };
                for &(m, tolerance) in sources {
                    out.push(Command::Cue {
                        bank: crate::audio::banks::CHARACTERS,
                        name: "boss_assatur_barrage",
                    });
                    out.push(Command::Missile {
                        position: [m[12], m[13], m[14]],
                        velocity: Self::forward(&m).map(|v| v * 30.),
                        tolerance,
                    });
                }
            }
            self.data.missile_delay += 0.15;
        }
        self.data.missile_delay -= dt;
    }

    fn update_blade(&mut self, dt: f32, n: f32, inputs: &Inputs, out: &mut Vec<Command>) {
        if self.controller.crossfade_enabled() {
            return;
        }
        if n >= self.timer(0) && n <= self.timer(1) {
            if !self.data.blade {
                self.data.blade = true;
                out.push(Command::Cue {
                    bank: crate::audio::banks::SPELLS,
                    name: "spell_arcane_ray_stage2",
                });
            }
            self.cast_cooldown -= dt;
            self.data.blade_hit -= dt;
            // CastSource rotated −90° about X: the blade points along Up.
            let source = self.cast_source();
            let rot = skinned::pose_matrix(
                [0.; 3],
                [
                    -(std::f32::consts::FRAC_PI_4).sin(),
                    0.,
                    0.,
                    (std::f32::consts::FRAC_PI_4).cos(),
                ],
                [1.; 3],
            );
            let blade = skinned::mul(&rot, &source);
            let position = [blade[12], blade[13], blade[14]];
            let up = [blade[4], blade[5], blade[6]];
            if n >= self.timer(2) && n <= self.timer(3) {
                if self.data.blade_hit <= 0. {
                    // Each target is hit at most once per 0.333 s.
                    self.data.blade_hit = 0.333;
                    let mut spell = Spell {
                        element: E::ARCANE,
                        ..Spell::default()
                    };
                    spell.mag[E::ARCANE.index() as usize] = 5.;
                    out.push(Command::Blade {
                        position,
                        direction: up,
                        damages: damage5(&spell, SpellType::Beam, CastType::Weapon),
                    });
                }
                if self.cast_cooldown <= 0. {
                    out.push(Command::BladeDetonation {
                        position,
                        direction: up,
                    });
                    self.cast_cooldown += 0.005;
                }
            }
        } else if n > self.timer(1) && self.data.blade {
            self.data.blade = false;
            out.push(Command::StopCue("spell_arcane_ray_stage2"));
        }
        if self.controller.has_finished() {
            self.change(State::Battle, inputs, out);
        }
    }

    fn update_beam(&mut self, inputs: &Inputs, out: &mut Vec<Command>) {
        if self.controller.crossfade_enabled() {
            return;
        }
        let n = self.normalized();
        let (t0, t1, t2, t3) = (self.timer(0), self.timer(1), self.timer(2), self.timer(3));
        let blend = ((n - t0) / (t1 - t0)).clamp(0., 1.) * ((t2 - n) / (t3 - t2)).clamp(0., 1.);
        self.targetting_power = 2.;
        let staff = self.staff();
        let origin = [staff[12], staff[13], staff[14]];
        let mut to: [f32; 3] = std::array::from_fn(|i| self.target_position[i] - origin[i]);
        let l = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2])
            .sqrt()
            .max(1e-6);
        to = to.map(|v| v / l);
        // Lerp the staff toward a frame whose Up is the target direction.
        let right = [staff[0], staff[1], staff[2]];
        let forward = cross(to, right);
        let aimed = crate::particles::create_world(origin, forward, to);
        let q0 = rotation_of(&staff);
        let q1 = rotation_of(&aimed);
        let mut rail = skinned::pose_matrix([0.; 3], nlerp(q0, q1, blend), [1.; 3]);
        rail[12..15].copy_from_slice(&origin);
        self.rail_staff = Some(rail);
        let direction = to.map(|v| v * blend);
        let position: [f32; 3] = std::array::from_fn(|i| origin[i] + 2. * direction[i]);
        self.data.beam = true;
        out.push(Command::Beam {
            position,
            direction,
        });
        if self.controller.has_finished() {
            self.change(State::Battle, inputs, out);
        }
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// Quaternion.CreateFromRotationMatrix (XNA row-vector layout).
fn rotation_of(m: &Mat) -> [f32; 4] {
    let (m11, m12, m13) = (m[0], m[1], m[2]);
    let (m21, m22, m23) = (m[4], m[5], m[6]);
    let (m31, m32, m33) = (m[8], m[9], m[10]);
    let trace = m11 + m22 + m33;
    if trace > 0. {
        let s = (trace + 1.).sqrt();
        let w = s * 0.5;
        let s = 0.5 / s;
        [(m23 - m32) * s, (m31 - m13) * s, (m12 - m21) * s, w]
    } else if m11 >= m22 && m11 >= m33 {
        let s = (1. + m11 - m22 - m33).sqrt();
        let i = 0.5 / s;
        [0.5 * s, (m12 + m21) * i, (m13 + m31) * i, (m23 - m32) * i]
    } else if m22 > m33 {
        let s = (1. + m22 - m11 - m33).sqrt();
        let i = 0.5 / s;
        [(m21 + m12) * i, 0.5 * s, (m32 + m23) * i, (m31 - m13) * i]
    } else {
        let s = (1. + m33 - m11 - m22).sqrt();
        let i = 0.5 / s;
        [(m31 + m13) * i, (m32 + m23) * i, 0.5 * s, (m12 - m21) * i]
    }
}
/// Quaternion.Lerp (normalized, shortest arc).
fn nlerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
    let s = if dot < 0. { -1. } else { 1. };
    let q: [f32; 4] = std::array::from_fn(|i| a[i] * (1. - t) + b[i] * t * s);
    let l = q.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
    q.map(|v| v / l)
}
/// Spell.CalculateDamage into a DamageCollection5.
fn damage5(spell: &Spell, st: SpellType, ct: CastType) -> [Damage; 5] {
    let mut out = [Damage::default(); 5];
    if let Ok(list) = magicka_sim::damage::calculate_damage(spell, st, ct) {
        for (slot, d) in out.iter_mut().zip(list) {
            *slot = d;
        }
    }
    out
}
