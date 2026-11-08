//! `Magicka.GameLogic.Entities.Bosses.Machine` (Havindr's drilling machine
//! and the warlock Gunter), as a deterministic state machine. Each update
//! returns ordered [`MachineCommand`]s for the scene host; damage arrives
//! through its two BossDamageZones. See research/boss-battles.md.
use crate::animation_controller::AnimationController;
use crate::character_pose::{CharacterSkeleton, Pose};
use magicka_content::skinned::{self, Mat, Resolved};
use magicka_sim::boss_status::{BossEvent, BossStatus};
use magicka_sim::damage::{attack, Damage};
use magicka_sim::spells::Elements as E;
use magicka_sim::status::Status;

pub const MACHINE_MODEL: &str = "Models/Bosses/theMachine/themachine";
pub const KING_MODEL: &str = "Models/Characters/Human/King_animation";
pub const WARLOCK_TEMPLATE: &str = "boss_machine_warlock";
const MAX_HIT_POINTS: f32 = 5000.0;
/// TOTAL_DURATION: drill seconds (at pedal speed 1) until the king dies.
pub const TOTAL_DURATION: f32 = 120.0;
const TAUNT_INTERVAL: f32 = 9.0;
const DRILL_LENGTH: f32 = 6.2;
const DRILLY_START: [f32; 3] = [19.44, 2.57, -57.97];
const DRILLY_END: [f32; 3] = [21.36, 1.7, -58.5];
const TAUNTS: [&str; 3] = ["warlocktaunt1", "warlocktaunt2", "warlocktaunt3"];

/// |DRILLY_END − DRILLY_START|.
pub fn drill_distance() -> f32 {
    (0..3)
        .map(|i| (DRILLY_END[i] - DRILLY_START[i]).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// INV_SCENE_ROTATION = Matrix.CreateRotationY(11π/100).
pub fn inv_scene_rotation() -> Mat {
    let (s, c) = (std::f32::consts::PI * 11.0 / 100.0).sin_cos();
    [
        c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

/// The two BossDamageZone boxes (min corner, sides) and radii.
pub const ZONES: [([f32; 3], [f32; 3], f32); 2] = [
    ([-4.25, 0.0, -3.75], [8.5, 3.0, 8.0], 4.0),
    ([-1.5, -1.0, -0.75], [2.5, 3.75, 2.5], 3.0),
];

/// Machine.States (machine and fight stages share the enum).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Drill,
    Broken,
    KillKing,
    Intro,
    PreGunter,
    Warlock,
    Final,
    PostFinal,
    KingKilled,
}

/// Host operations, in the order the native class performs them.
#[derive(Clone, Debug, PartialEq)]
pub enum MachineCommand {
    Trigger(&'static str),
    /// EffectManager.StartEffect with a full transform.
    EffectTransform {
        name: &'static str,
        transform: Mat,
    },
    Effect {
        name: &'static str,
        position: [f32; 3],
        direction: [f32; 3],
    },
    Cue {
        bank: u16,
        name: &'static str,
        position: [f32; 3],
    },
    /// DialogManager.StartDialog(dialog, warlock).
    WarlockDialog(&'static str),
    /// Kill every Barrier, MissileEntity, SpellMine and GreaseField.
    KillSpellEntities,
    /// Agent.SetOrder(order, reactTo, reaction); `emperor` attaches the two
    /// spec_action0 animation events; `pop_to_idle` first pops the AI to
    /// AIStateIdle (releasing an attack target).
    WarlockOrder {
        order: u8,
        react_to: u8,
        reaction: u8,
        emperor: bool,
        pop_to_idle: bool,
    },
    WarlockImmortal(f32),
    WarlockFullHealth,
    WarlockStopStatuses(u16),
    /// Machine.TeleportWarlock to the locator (effects, sounds, ground snap).
    WarlockTeleport(Mat),
    /// FinalStage: hold the warlock on the ground below the locator.
    WarlockHold(Mat),
    /// Two emperor lightning bolts from the warlock's hands.
    EmperorLightning,
    /// GetAnimatedLevelPart(id).Play(all children, -1, -1, 1, no loop).
    PlayLevelPart(&'static str),
    /// PostFinal: move away, CannotDie = false, immortal 0, Terminate.
    WarlockTerminate,
    /// PlayState.Endgame(Defeat, freeze, not phony, 0).
    EndGameDefeat,
    /// OverKillKing at the drill tip (warlock gib list, wood blood).
    GibKing {
        transform: Mat,
    },
    Boss(BossEvent),
}

pub mod order {
    pub const IDLE: u8 = 0;
    pub const ATTACK: u8 = 2;
}
pub mod react {
    pub const NONE: u8 = 0;
    pub const ATTACK: u8 = 1;
    pub const PROXIMITY: u8 = 2;
}

/// Facts the host supplies each update.
#[derive(Clone, Copy, Debug, Default)]
pub struct MachineInputs {
    pub fight_started: bool,
    pub warlock_hit_points: f32,
    pub warlock_max_hit_points: f32,
}

/// A posed model for presentation.
#[derive(Clone, Debug, Default)]
pub struct ModelPose {
    pub parent: Mat,
    pub local: Vec<Pose>,
}

/// The MachineRuler: transform, drill time and alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ruler {
    pub transform: Mat,
    pub time: f32,
    pub alpha: f32,
}

#[derive(Clone, Debug)]
pub struct Machine {
    pub status: BossStatus,
    pub stage: State,
    pub machine_state: State,
    pub drill_time: f32,
    idle_timer: f32,
    pub pedal_speed: f32,
    pub pedal_target: f32,
    emperor_timer: f32,
    level_animation: bool,
    bipedal_done: bool,
    pub ruler_alpha: f32,
    pub render_king: bool,
    pub orientation: Mat,
    pub king_orientation: Mat,
    pub warlock_emperor: Mat,
    pub machine: AnimationController,
    pub orc: AnimationController,
    pub king: AnimationController,
    pub machine_skeleton: CharacterSkeleton,
    pub king_skeleton: CharacterSkeleton,
    durations: std::collections::HashMap<String, f32>,
    king_durations: std::collections::HashMap<String, f32>,
    platform: usize,
    drilly: usize,
    drilly_bind: Mat,
    /// BicycleRoot bind translation rotated by its orientation · RotY(π).
    pub bicycle_position: [f32; 3],
    /// Last presentation poses (machine with the drill offset, orc, king).
    pub machine_pose: ModelPose,
    pub orc_pose: ModelPose,
    pub king_pose: ModelPose,
    pub ruler: Option<Ruler>,
    rng: u64,
}

/// XNA Quaternion.Multiply.
pub(crate) fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [x1, y1, z1, w1] = a;
    let [x2, y2, z2, w2] = b;
    [
        x1 * w2 + x2 * w1 + (y1 * z2 - z1 * y2),
        y1 * w2 + y2 * w1 + (z1 * x2 - x1 * z2),
        z1 * w2 + z2 * w1 + (x1 * y2 - y1 * x2),
        w1 * w2 - (x1 * x2 + y1 * y2 + z1 * z2),
    ]
}
/// XNA Vector3.Transform(v, q).
pub(crate) fn quat_rotate(v: [f32; 3], q: [f32; 4]) -> [f32; 3] {
    let m = skinned::pose_matrix([0.0; 3], q, [1.0; 3]);
    [
        v[0] * m[0] + v[1] * m[4] + v[2] * m[8],
        v[0] * m[1] + v[1] * m[5] + v[2] * m[9],
        v[0] * m[2] + v[1] * m[6] + v[2] * m[10],
    ]
}
pub fn rotation_y_pi() -> Mat {
    [
        -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}
fn translation(m: &Mat) -> [f32; 3] {
    [m[12], m[13], m[14]]
}
fn right(m: &Mat) -> [f32; 3] {
    [m[0], m[1], m[2]]
}

impl Machine {
    /// Machine(PlayState): the models, bones and clip lookups.
    pub fn new(machine: Resolved, king: Resolved, seed: u64) -> Result<Self, String> {
        let machine_skeleton =
            CharacterSkeleton::new(machine.skeleton.clone(), machine.clips.clone());
        let king_skeleton = CharacterSkeleton::new(king.skeleton.clone(), king.clips.clone());
        let durations = machine_skeleton.clip_durations();
        let king_durations = king_skeleton.clip_durations();
        for clip in ["pedal", "breakfree", "nod", "idle"] {
            durations
                .get(clip)
                .ok_or_else(|| format!("{MACHINE_MODEL} missing clip {clip}"))?;
        }
        for clip in [
            "sit_bound",
            "king_vader",
            "talk_oldandweak",
            "talk_strikeatdawn",
        ] {
            king_durations
                .get(clip)
                .ok_or_else(|| format!("{KING_MODEL} missing clip {clip}"))?;
        }
        let bone = |name: &str| {
            machine_skeleton
                .bone(name)
                .ok_or_else(|| format!("{MACHINE_MODEL} missing bone {name}"))
        };
        let platform = bone("Platform")?;
        let drilly = bone("Drilly")?;
        let bicycle = &machine.skeleton.bones[bone("BicycleRoot")?];
        // Quaternion.CreateFromRotationMatrix(RotY(π)) = (0, 1, 0, 0).
        let q = quat_mul(bicycle.orientation, [0.0, 1.0, 0.0, 0.0]);
        let bicycle_position = quat_rotate(bicycle.translation, q);
        let drilly_bind = crate::live_collision::inverse(&skinned::mul(
            &machine.skeleton.bones[drilly].inverse_bind,
            &rotation_y_pi(),
        ))
        .map_err(|e| format!("Drilly bind pose: {e:?}"))?;
        let mut status = BossStatus::default();
        status.resistances[0].modifier = -200.0;
        status.resistances[0].multiplier = 0.75;
        for e in [E::ARCANE, E::LIGHTNING, E::FIRE] {
            status.resistances[e.0.trailing_zeros() as usize].multiplier = 0.5;
        }
        Ok(Self {
            status,
            stage: State::Intro,
            machine_state: State::Idle,
            drill_time: 0.0,
            idle_timer: 0.0,
            pedal_speed: 1.0,
            pedal_target: 1.0,
            emperor_timer: 0.0,
            level_animation: false,
            bipedal_done: false,
            ruler_alpha: 0.0,
            render_king: true,
            orientation: skinned::IDENTITY,
            king_orientation: skinned::IDENTITY,
            warlock_emperor: skinned::IDENTITY,
            machine: AnimationController::default(),
            orc: AnimationController::default(),
            king: AnimationController::default(),
            machine_skeleton,
            king_skeleton,
            durations,
            king_durations,
            platform,
            drilly,
            drilly_bind,
            bicycle_position,
            machine_pose: ModelPose::default(),
            orc_pose: ModelPose::default(),
            king_pose: ModelPose::default(),
            ruler: None,
            rng: seed | 1,
        })
    }

    /// Initialize(orientation): the fight is reset to Intro/Idle. `king` is
    /// the spawn_king locator (scaled 1.3 here); `emperor` is
    /// teleport_warlock.
    pub fn initialize(&mut self, orientation: Mat, king: Mat, emperor: Mat) {
        self.render_king = true;
        self.status.max_hit_points = MAX_HIT_POINTS;
        self.status.hit_points = MAX_HIT_POINTS;
        self.idle_timer = 0.0;
        self.drill_time = 0.0;
        self.pedal_speed = 1.0;
        self.pedal_target = 1.0;
        self.orientation = orientation;
        let mut k = king;
        for v in &mut k[0..11] {
            *v *= 1.3;
        }
        k[3] = king[3];
        k[7] = king[7];
        self.king_orientation = k;
        self.warlock_emperor = emperor;
        self.orc.speed = 1.0;
        self.machine.speed = 1.0;
        self.bipedal_done = false;
        self.status.statuses = Default::default();
        self.status.current = 0;
        self.stage = State::Intro;
        self.king_clip("sit_bound", 0.0, true);
        self.machine_state = State::Idle;
        self.machine_clip("idle", 0.0, true);
        self.orc_clip("idle", 0.0, true);
        self.ruler_alpha = 0.0;
    }

    fn clip(
        c: &mut AnimationController,
        durations: &std::collections::HashMap<String, f32>,
        name: &str,
        fade: f32,
        looping: bool,
    ) {
        let d = durations.get(name).copied().unwrap_or(0.0);
        if fade > 0.0 {
            c.crossfade_named_clip(name, d, fade, looping);
        } else {
            c.start_named_clip(name, d, looping);
        }
    }
    fn machine_clip(&mut self, name: &str, fade: f32, looping: bool) {
        Self::clip(&mut self.machine, &self.durations, name, fade, looping);
    }
    fn orc_clip(&mut self, name: &str, fade: f32, looping: bool) {
        Self::clip(&mut self.orc, &self.durations, name, fade, looping);
    }
    fn king_clip(&mut self, name: &str, fade: f32, looping: bool) {
        Self::clip(&mut self.king, &self.king_durations, name, fade, looping);
    }

    /// The boss bar's MaxHitPoints and HitPoints.
    pub fn health(&self, warlock_hp: f32, warlock_max: f32) -> (f32, f32) {
        match self.stage {
            State::Intro | State::PreGunter => {
                (self.status.hit_points.max(0.0) + 1.0, MAX_HIT_POINTS)
            }
            State::PostFinal => (0.0, 1.0),
            _ => (warlock_hp.max(0.0) + 1.0, warlock_max),
        }
    }

    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// The drill tip: Drilly bind · skinned bone, moved DRILL_LENGTH along Right.
    fn drill_tip(&self, skinned: &[Mat]) -> Mat {
        let mut t = skinned::mul(&self.drilly_bind, &skinned[self.drilly]);
        let r = right(&t);
        for i in 0..3 {
            t[12 + i] += r[i] * DRILL_LENGTH;
        }
        t
    }

    fn change_machine_state(&mut self, state: State, out: &mut Vec<MachineCommand>) {
        self.machine_state = state;
        match state {
            State::Idle => {
                self.machine_clip("idle", 0.0, true);
                self.orc_clip("idle", 0.0, true);
            }
            State::Drill => {
                self.orc_clip("nod", 0.2, false);
                self.orc.speed = 1.0;
            }
            State::Broken => {
                let mut t = self.orientation;
                t[13] = 0.0;
                out.push(MachineCommand::EffectTransform {
                    name: "machine_break",
                    transform: t,
                });
                self.machine_clip("breakfree", 0.2, false);
                self.machine.speed = 1.0;
                out.push(MachineCommand::KillSpellEntities);
                out.push(MachineCommand::Trigger("machine_destroyed"));
                out.push(MachineCommand::Cue {
                    bank: crate::audio::banks::MISC,
                    name: "misc_machine_break",
                    position: self.zone_position(1),
                });
                self.ruler_alpha = 0.0;
            }
            State::KillKing => {
                self.orc_clip("idle", 0.2, true);
                self.orc.speed = 1.0;
                self.machine_clip("idle", 0.2, true);
                self.orc.speed = 1.0;
                self.change_stage(State::KingKilled, out);
            }
            _ => {}
        }
    }

    fn change_stage(&mut self, stage: State, out: &mut Vec<MachineCommand>) {
        self.stage = stage;
        match stage {
            State::Intro => self.king_clip("sit_bound", 0.0, true),
            State::PreGunter => {
                self.idle_timer = 0.0;
                self.king_clip("sit_bound", 0.1, true);
                out.push(MachineCommand::WarlockOrder {
                    order: order::IDLE,
                    react_to: react::NONE,
                    reaction: order::IDLE,
                    emperor: false,
                    pop_to_idle: false,
                });
                out.push(MachineCommand::WarlockImmortal(f32::INFINITY));
            }
            State::Warlock => {
                out.push(MachineCommand::WarlockOrder {
                    order: order::ATTACK,
                    react_to: react::ATTACK | react::PROXIMITY,
                    reaction: order::ATTACK,
                    emperor: false,
                    pop_to_idle: false,
                });
                out.push(MachineCommand::WarlockImmortal(0.0));
                out.push(MachineCommand::WarlockFullHealth);
            }
            State::Final => {
                out.push(MachineCommand::Trigger("gunther_pissed"));
                out.push(MachineCommand::WarlockStopStatuses(
                    Status::Wet as u16
                        | Status::Burning as u16
                        | Status::Cold as u16
                        | Status::Steamed as u16,
                ));
                out.push(MachineCommand::WarlockTeleport(self.warlock_emperor));
                out.push(MachineCommand::WarlockOrder {
                    order: order::IDLE,
                    react_to: react::NONE,
                    reaction: order::IDLE,
                    emperor: true,
                    pop_to_idle: true,
                });
                out.push(MachineCommand::Cue {
                    bank: crate::audio::banks::MISC,
                    name: "misc_trapdoor",
                    position: self.zone_position(1),
                });
                self.king_clip("king_vader", 0.25, false);
                self.emperor_timer = 0.0;
                self.idle_timer = 0.0;
                self.level_animation = false;
            }
            State::PostFinal => {
                self.king_clip("talk_oldandweak", 0.1, false);
                out.push(MachineCommand::WarlockTerminate);
                out.push(MachineCommand::Trigger("gunther_dead"));
            }
            State::KingKilled => {
                self.idle_timer = 0.0;
                out.push(MachineCommand::EndGameDefeat);
                let skinned = self
                    .machine_skeleton
                    .skinned_from_local(&self.machine_pose.local, &self.orientation);
                if skinned.len() > self.drilly {
                    out.push(MachineCommand::GibKing {
                        transform: self.drill_tip(&skinned),
                    });
                }
                self.render_king = false;
            }
            _ => {}
        }
    }

    /// The zone body positions (machine translation, and the orc zone at the
    /// BicycleRoot offset).
    pub fn zone_position(&self, index: usize) -> [f32; 3] {
        let t = translation(&self.orientation);
        if index == 0 {
            t
        } else {
            [0, 1, 2].map(|i| t[i] + self.bicycle_position[i])
        }
    }

    /// Machine.Damage(part, Damage, ...): forwarded zone damage.
    pub fn damage(
        &mut self,
        part: usize,
        mut d: Damage,
        attack_position: [f32; 3],
        features: u8,
        out: &mut Vec<MachineCommand>,
    ) -> u16 {
        if self.machine_state == State::Broken {
            return 0;
        }
        if part != 1 && d.attack & attack::STATUS != 0 {
            d.attack &= !attack::STATUS;
            if d.attack == 0 {
                return 0;
            }
        }
        let mut events = Vec::new();
        let result = self
            .status
            .damage(d, false, attack_position, features, &mut events);
        out.extend(events.into_iter().map(MachineCommand::Boss));
        if self.status.hit_points <= 0.0 {
            self.change_machine_state(State::Broken, out);
        }
        result
    }

    /// Machine.ScriptMessage.
    pub fn message(&mut self, name: &str) {
        match name {
            "OrcBipedal" => {
                self.orc.speed = 1.0;
                self.orc_clip("breakfree", 0.25, false);
            }
            "KingTalk1" => self.king_clip("talk_oldandweak", 0.25, false),
            "KingTalk2" => self.king_clip("talk_strikeatdawn", 0.25, false),
            _ => {}
        }
    }

    fn update_stage(&mut self, dt: f32, inputs: &MachineInputs, out: &mut Vec<MachineCommand>) {
        match self.stage {
            State::PreGunter => {
                if self.idle_timer > TAUNT_INTERVAL {
                    self.idle_timer -= TAUNT_INTERVAL;
                    let i = (self.random() % TAUNTS.len() as u64) as usize;
                    out.push(MachineCommand::WarlockDialog(TAUNTS[i]));
                }
                self.idle_timer += dt;
            }
            State::Warlock => {
                if inputs.warlock_hit_points <= 0.0 {
                    self.change_stage(State::Final, out);
                }
            }
            State::Final => {
                self.idle_timer += dt;
                self.emperor_timer += dt;
                out.push(MachineCommand::WarlockHold(self.warlock_emperor));
                if self.emperor_timer > 5.5 {
                    if !self.level_animation && self.emperor_timer > 9.0 {
                        out.push(MachineCommand::PlayLevelPart("hatch"));
                        self.level_animation = true;
                    }
                    if self.idle_timer > 0.25 {
                        self.idle_timer -= 0.25;
                        out.push(MachineCommand::EmperorLightning);
                    }
                    // FinalStage advances the timer a second time here.
                    self.idle_timer += dt;
                }
                if !self.king.crossfade_enabled() && self.king.has_finished() {
                    self.change_stage(State::PostFinal, out);
                }
            }
            _ => {}
        }
    }

    fn update_machine_state(&mut self, dt: f32, out: &mut Vec<MachineCommand>) {
        match self.machine_state {
            State::Drill => {
                self.ruler_alpha += dt;
                if self.orc.clip_name() == Some("nod") {
                    if self.orc.has_finished() && !self.orc.crossfade_enabled() {
                        self.orc_clip("pedal", 0.2, true);
                        self.orc.speed = 0.1;
                        self.machine_clip("pedal", 0.2, true);
                        self.machine.speed = 0.1;
                    }
                } else {
                    if self.drill_time >= TOTAL_DURATION {
                        self.change_machine_state(State::KillKing, out);
                    }
                    self.drill_time += dt * self.pedal_speed;
                    self.machine.speed = self.pedal_speed;
                    self.orc.speed = self.pedal_speed;
                }
            }
            State::Broken => {
                self.pedal_target = 1.0;
                self.ruler_alpha -= dt;
                if self.orc.has_finished()
                    && !self.orc.crossfade_enabled()
                    && self.orc.clip_name() == Some("breakfree")
                    && !self.bipedal_done
                {
                    self.bipedal_done = true;
                    out.push(MachineCommand::Trigger("bipedal_done"));
                    self.change_stage(State::Warlock, out);
                }
            }
            _ => {}
        }
    }

    /// Machine.UpdateBoss.
    pub fn update(&mut self, dt: f32, inputs: MachineInputs) -> Vec<MachineCommand> {
        let mut out = Vec::new();
        if inputs.fight_started && self.stage == State::Intro {
            self.change_machine_state(State::Drill, &mut out);
            self.change_stage(State::PreGunter, &mut out);
        }
        self.update_stage(dt, &inputs, &mut out);
        self.update_machine_state(dt, &mut out);
        self.pedal_speed += (self.pedal_target - self.pedal_speed) * dt * 4.0;
        let s = &self.status;
        if s.has(Status::Burning) {
            self.pedal_target += (3.0 - self.pedal_target) * dt * 4.0;
        } else if s.has(Status::Frozen) {
            self.pedal_speed = 0.0;
            self.pedal_target = 0.0;
        } else if s.has(Status::Cold) {
            self.pedal_target += (1.0 - s.magnitude(Status::Cold) - self.pedal_target) * dt;
        } else {
            self.pedal_target += (1.0 - self.pedal_target) * dt * 0.25;
        }
        // UpdateStatusEffects: status ticks go to the orc zone, whose damage
        // only counts during Intro/PreGunter.
        for (amount, element) in self.status.update_statuses(dt, false) {
            if matches!(self.stage, State::Intro | State::PreGunter) {
                self.status.status_damage(amount, element);
            }
        }
        let mut events = Vec::new();
        self.status.update_damage(dt, false, &mut events);
        out.extend(events.into_iter().map(MachineCommand::Boss));
        self.update_animation(dt);
        out
    }

    /// Controllers and presentation poses; the platform is pushed along its
    /// local Z by the drill progress.
    fn update_animation(&mut self, dt: f32) {
        for c in [&mut self.machine, &mut self.orc, &mut self.king] {
            c.signals.clear();
        }
        self.machine.update(dt);
        let mut local = self
            .machine_skeleton
            .local_poses(self.machine.pose_sample());
        if let Some(p) = local.get_mut(self.platform) {
            let offset = quat_rotate(
                [
                    0.0,
                    0.0,
                    self.drill_time / TOTAL_DURATION * drill_distance(),
                ],
                p.1,
            );
            for (v, o) in p.0.iter_mut().zip(offset) {
                *v += o;
            }
        }
        let skinned = self
            .machine_skeleton
            .skinned_from_local(&local, &self.orientation);
        self.machine_pose = ModelPose {
            parent: self.orientation,
            local,
        };
        self.orc.update(dt);
        self.orc_pose = ModelPose {
            parent: self.orientation,
            local: self.machine_skeleton.local_poses(self.orc.pose_sample()),
        };
        if self.render_king {
            self.king.update(dt);
            self.king_pose = ModelPose {
                parent: self.king_orientation,
                local: self.king_skeleton.local_poses(self.king.pose_sample()),
            };
        }
        self.ruler = Some(Ruler {
            transform: self.drill_tip(&skinned),
            time: self.drill_time,
            alpha: self.ruler_alpha.clamp(0.0, 1.0),
        });
    }
}

/// RulerRenderData.PreRenderUpdate geometry: the eight strip vertices
/// (world space, TriangleStrip with u = 0, 1/3, 2/3, 1 per pair and v = 1
/// at the bottom) and the text's world anchor. The transform's Up is forced
/// to world Up before use.
pub fn ruler_geometry(r: &Ruler) -> ([[f32; 3]; 8], [[f32; 2]; 8], [f32; 3]) {
    let n = (TOTAL_DURATION - r.time) * (1.0 / TOTAL_DURATION) * drill_distance();
    let m = &r.transform;
    let right = [m[0], m[1], m[2]];
    let up = [0.0, 1.0, 0.0];
    let origin = translation(m);
    let xs = [-0.25, -0.25, 0.0, 0.0, n, n, 0.25 + n, 0.25 + n];
    let ys = [-0.1, 0.9, -0.1, 0.9, -0.1, 0.9, -0.1, 0.9];
    let us = [
        0.0,
        0.0,
        1.0 / 3.0,
        1.0 / 3.0,
        2.0 / 3.0,
        2.0 / 3.0,
        1.0,
        1.0,
    ];
    let positions =
        std::array::from_fn(|i| [0, 1, 2].map(|k| origin[k] + right[k] * xs[i] + up[k] * ys[i]));
    let uvs = std::array::from_fn(|i| [us[i], if i % 2 == 0 { 1.0 } else { 0.0 }]);
    let anchor = [origin[0] + 0.5, origin[1] + 1.3, origin[2]];
    (positions, uvs, anchor)
}

/// RulerRenderData's distance text: "d.ddd" + "m" of the remaining drill
/// distance (digits by integer truncation, as the native character math).
pub fn ruler_text(time: f32) -> String {
    let n = (TOTAL_DURATION - time) * (1.0 / TOTAL_DURATION) * drill_distance();
    let d0 = n as i32;
    let d1 = (n * 10.0 % 10.0) as i32;
    let d2 = (n * 100.0 % 10.0) as i32;
    let d3 = (n * 1000.0 % 10.0) as i32;
    [d0, d1, d2, d3]
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let c = char::from_u32((48 + d) as u32).unwrap_or('?');
            if i == 0 {
                format!("{c}.")
            } else {
                c.to_string()
            }
        })
        .collect::<String>()
        + "m"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruler_strip_shrinks_with_the_remaining_distance() {
        let mut transform = skinned::IDENTITY;
        transform[12..15].copy_from_slice(&[1.0, 2.0, 3.0]);
        // A tilted Up row is replaced by world Up.
        transform[4..7].copy_from_slice(&[0.3, 0.9, 0.1]);
        let (p, uv, anchor) = ruler_geometry(&Ruler {
            transform,
            time: 60.0,
            alpha: 1.0,
        });
        let half = drill_distance() * 0.5;
        assert_eq!(p[0], [0.75, 1.9, 3.0]);
        assert_eq!(p[1], [0.75, 2.9, 3.0]);
        assert!((p[4][0] - (1.0 + half)).abs() < 1e-5);
        assert!((p[7][0] - (1.25 + half)).abs() < 1e-5);
        assert_eq!(uv[2], [1.0 / 3.0, 1.0]);
        assert_eq!(uv[7], [1.0, 0.0]);
        assert_eq!(anchor, [1.5, 3.3, 3.0]);
    }

    #[test]
    fn ruler_counts_down_the_drill_distance() {
        assert!((drill_distance() - 2.1735).abs() < 1e-3);
        assert_eq!(ruler_text(0.0), "2.173m");
        assert_eq!(ruler_text(60.0), "1.086m");
        assert_eq!(ruler_text(120.0), "0.000m");
    }
}
