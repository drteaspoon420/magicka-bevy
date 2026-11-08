//! The physical Grimnir encounter, separate from the six dream venues.
//! Native provenance: research/boss-grimnir2.md. Scene and spell operations
//! are returned to a host; no elapsed-time victory is synthesized.
use crate::{
    animation_controller::AnimationController, boss_fafnir::ModelPose,
    character_pose::CharacterSkeleton,
};
use magicka_content::skinned::{self, Mat, Resolved};
use magicka_sim::{
    boss_status::{BossEvent, BossStatus},
    damage::{CastType, Damage},
    spells::{Elements as E, MagickType as M, Spell},
    status::Status,
};

pub const MODEL: &str = "Models/Bosses/grimnir/grimnir_mesh";
pub const ANIMATION: &str = "Models/Bosses/grimnir/grimnir_animation";
pub const ASSATUR: &str = "Models/Bosses/assatur/assatur";
pub const MAX_HP: f32 = 35000.;
pub const HEAL_HP: f32 = 3500.;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Intro,
    Idle,
    Spirit,
    Magick,
    Spell,
    Heal,
    Die,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpiritState {
    Intro,
    Idle,
    Cast,
    Heal,
    Die,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Trigger(&'static str),
    Cast {
        part: usize,
        spell: Spell,
        cast: CastType,
        power: f32,
    },
    Magick(M),
    StopSpell(usize),
    CameraBias,
    HealEffect,
    Boss(BossEvent),
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Inputs {
    pub fight_started: bool,
    pub nearest_distance_squared: f32,
    pub tornado: bool,
    pub rain: bool,
    pub wet_players: f32,
    pub conflagration: bool,
    pub blizzard: bool,
    pub target: [f32; 3],
}
#[derive(Clone, Debug)]
pub struct Spirit {
    pub state: SpiritState,
    pub controller: AnimationController,
    pub pose: ModelPose,
    pub drawn: bool,
    idle: f32,
    last_spell: usize,
    has_cast: bool,
}
impl Default for Spirit {
    fn default() -> Self {
        Self {
            state: SpiritState::Intro,
            controller: Default::default(),
            pose: Default::default(),
            drawn: false,
            idle: 0.,
            last_spell: 0,
            has_cast: false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Grimnir2 {
    pub status: BossStatus,
    pub state: State,
    pub skeleton: CharacterSkeleton,
    pub assatur_skeleton: CharacterSkeleton,
    pub controller: AnimationController,
    pub assatur_controller: AnimationController,
    pub pose: ModelPose,
    pub assatur_pose: ModelPose,
    pub spirits: [Spirit; 2],
    pub orientation: Mat,
    pub assatur_orientation: Mat,
    pub healing: bool,
    pub corporeal: bool,
    pub both_spirits: bool,
    pub spectral_alpha: f32,
    pub assatur_alpha: f32,
    pub spirit_timer: f32,
    pub damage_flash: f32,
    idle: f32,
    last_spell: usize,
    last_magick: M,
    spectral_target: f32,
    spectral_fade: f32,
    float: [f32; 2],
    target: [f32; 3],
    loop_fight: bool,
    called_appear: bool,
    rng: u64,
}
impl Grimnir2 {
    pub fn new(model: Resolved, assatur: Resolved, orientation: Mat) -> Result<Self, String> {
        let skeleton = CharacterSkeleton::new(model.skeleton, model.clips);
        let assatur_skeleton = CharacterSkeleton::new(assatur.skeleton, assatur.clips);
        for clip in [
            "idle",
            "die",
            "spirit_idle",
            "spirit_die",
            "spirit_cast_projectile",
            "spirit_cast_railgun",
            "cast_magick_global",
            "cast_magick_direct",
            "summon_spirits",
            "cast_shield",
        ] {
            if !skeleton.clip_durations().contains_key(clip) {
                return Err(format!("Grimnir2 missing {clip}"));
            }
        }
        let mut asa = orientation;
        for i in 0..3 {
            asa[12 + i] += orientation[8 + i] * 5.;
        }
        asa[13] -= 6.;
        let mut status = BossStatus::default();
        status.hit_points = MAX_HP;
        status.max_hit_points = MAX_HP;
        let mut g = Self {
            status,
            state: State::Intro,
            skeleton,
            assatur_skeleton,
            controller: Default::default(),
            assatur_controller: Default::default(),
            pose: Default::default(),
            assatur_pose: Default::default(),
            spirits: Default::default(),
            orientation,
            assatur_orientation: asa,
            healing: false,
            corporeal: false,
            both_spirits: false,
            spectral_alpha: 0.,
            assatur_alpha: 0.,
            spirit_timer: 25.1,
            damage_flash: 0.,
            idle: 0.,
            last_spell: 0,
            last_magick: M::MeteorS,
            spectral_target: 0.,
            spectral_fade: 0.,
            float: [0.; 2],
            target: [0.; 3],
            loop_fight: true,
            called_appear: false,
            rng: 0x6772696d6e697232,
        };
        let d = g.skeleton.clip_durations()["idle"];
        g.controller.start_named_clip("idle", d, false);
        g.assatur_controller.start_named_clip(
            "idle",
            g.assatur_skeleton.clip_durations()["idle"],
            true,
        );
        for spirit in &mut g.spirits {
            spirit.controller.start_named_clip(
                "spirit_idle",
                g.skeleton.clip_durations()["spirit_idle"],
                true,
            );
        }
        Ok(g)
    }
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u32 << 24) as f32
    }
    fn clip(&mut self, name: &str, fade: f32, looping: bool) {
        self.controller.crossfade_named_clip(
            name,
            self.skeleton.clip_durations()[name],
            fade,
            looping,
        );
    }
    fn spirit_state(&mut self, index: usize, state: SpiritState, out: &mut Vec<Command>) {
        let previous = self.spirits[index].state;
        if previous == SpiritState::Cast {
            out.push(Command::StopSpell(index + 1));
        }
        if previous == SpiritState::Heal {
            self.spectral_target = 1.;
        }
        let (name, fade, looping) = match state {
            SpiritState::Intro => ("spirit_idle", 0., true),
            SpiritState::Idle => {
                self.spirits[index].idle = 2. + self.random() * 2.;
                self.spectral_target = 1.;
                ("spirit_idle", 0.25, true)
            }
            SpiritState::Cast => {
                // Only two choices, with the previous one explicitly excluded.
                self.spirits[index].last_spell = 1 - self.spirits[index].last_spell;
                self.spirits[index].has_cast = false;
                (
                    if self.spirits[index].last_spell == 0 {
                        "spirit_cast_railgun"
                    } else {
                        "spirit_cast_projectile"
                    },
                    0.15,
                    false,
                )
            }
            SpiritState::Heal => {
                self.spirits[index].idle = 0.25;
                ("cast_magick_global", 0.25, true)
            }
            SpiritState::Die => {
                self.spectral_target = 0.;
                ("spirit_die", 0.15, true)
            }
        };
        self.spirits[index].state = state;
        self.spirits[index].controller.crossfade_named_clip(
            name,
            self.skeleton.clip_durations()[name],
            fade,
            looping,
        );
    }
    fn change(&mut self, state: State, inputs: Inputs, out: &mut Vec<Command>) {
        if self.state == State::Magick {
            out.push(Command::Magick(self.last_magick));
        }
        if self.state == State::Spell {
            out.push(Command::StopSpell(0));
        }
        self.state = state;
        match state {
            State::Intro => self.clip("idle", 0., false),
            State::Idle => {
                self.clip("idle", 0.5, true);
                self.idle = 2. + self.random() * 2.;
            }
            State::Spirit => self.clip("summon_spirits", 0.25, false),
            State::Magick => {
                self.idle = 2.;
                let mut weights = [0.; 4];
                if self.last_magick != M::Blizzard {
                    weights[0] = self.random();
                    if inputs.tornado {
                        weights[0] = 0.;
                    }
                    if !inputs.rain {
                        weights[1] = self.random();
                    }
                    if inputs.wet_players > 0. {
                        weights[1] = 0.;
                    }
                    if !inputs.conflagration && !inputs.blizzard {
                        weights[2] = self.random();
                    }
                    weights[3] = inputs.wet_players;
                }
                let total: f32 = weights.iter().sum();
                weights = weights.map(|w| w / total);
                let selected = if weights[0] > weights[1]
                    && weights[0] > weights[2]
                    && weights[0] > weights[3]
                {
                    0
                } else if weights[1] > weights[2] && weights[1] > weights[3] {
                    1
                } else if weights[2] > weights[3] {
                    2
                } else {
                    3
                };
                self.last_magick = [M::Tornado, M::Rain, M::Conflagration, M::ThunderB][selected];
                self.clip(
                    if selected < 2 {
                        "cast_magick_global"
                    } else {
                        "cast_magick_direct"
                    },
                    if selected == 2 { 0.25 } else { 0.2 },
                    true,
                );
            }
            State::Spell => {
                self.clip("cast_shield", 0.25, false);
                self.last_spell = if self.status.time_since_last_damage < 1. {
                    if self.random() > 0.5 {
                        1
                    } else {
                        2
                    }
                } else if inputs.nearest_distance_squared < 25. {
                    0
                } else if self.random() > 0.5 {
                    2
                } else {
                    1
                };
            }
            State::Heal => {
                self.clip("idle", 0.25, true);
                out.push(Command::CameraBias);
            }
            State::Die => {
                self.status.hit_points = 0.;
                self.clip("idle", 0.25, false);
            }
        }
    }
    pub fn receive_damage(
        &mut self,
        part: usize,
        d: Damage,
        position: [f32; 3],
        features: u8,
    ) -> Vec<Command> {
        let mut events = Vec::new();
        if part != 0 || self.state == State::Die || self.healing {
            return events;
        }
        let mut damage = Vec::new();
        let result = self
            .status
            .damage(d, false, position, features, &mut damage);
        if result & 3 != 0 {
            self.status.time_since_last_damage = 0.;
            self.damage_flash = 0.1;
        }
        events.extend(damage.into_iter().map(Command::Boss));
        self.clamp_health();
        events
    }
    fn clamp_health(&mut self) {
        if self.status.hit_points < HEAL_HP {
            self.healing = true;
            self.status.hit_points = HEAL_HP;
        }
    }
    pub fn message(&mut self, name: &str) -> Vec<Command> {
        let mut out = Vec::new();
        match name {
            "KillGrimnir" => self.clip("die", 0.25, false),
            "GrimnirHurt" => {
                self.healing = true;
                self.loop_fight = false;
                self.assatur_alpha = 1.;
                self.status.hit_points = HEAL_HP;
            }
            "FutureVladDone" if !self.corporeal => {
                self.loop_fight = true;
                self.called_appear = false;
                self.both_spirits = true;
                self.status.hit_points = MAX_HP;
                self.assatur_controller.crossfade_named_clip(
                    "idle",
                    self.assatur_skeleton.clip_durations()["idle"],
                    0.5,
                    true,
                );
                self.healing = false;
                out.push(Command::HealEffect);
            }
            "CloneDone" => {
                self.loop_fight = true;
                self.called_appear = true;
            }
            "Assatur_Cut" => self.assatur_controller.crossfade_named_clip(
                "cast_ground",
                self.assatur_skeleton.clip_durations()["cast_ground"],
                0.25,
                false,
            ),
            _ => {}
        }
        out
    }
    pub fn corporealize(&mut self, talks: bool) -> Vec<Command> {
        let mut out = Vec::new();
        if self.spirit_timer <= 25. {
            self.spirit_timer = 24.;
        }
        if self.healing {
            self.corporeal = true;
            if talks {
                out.push(Command::Trigger("assatur_talks"));
                self.change(State::Die, Inputs::default(), &mut out);
                for i in 0..2 {
                    self.spirit_state(i, SpiritState::Die, &mut out);
                }
                self.healing = false;
                self.assatur_controller.crossfade_named_clip(
                    "idle",
                    self.assatur_skeleton.clip_durations()["idle"],
                    1.,
                    true,
                );
                out.push(Command::Magick(M::Nullify));
            }
        }
        out
    }
    pub fn nullify(&mut self) -> Vec<Command> {
        let mut out = Vec::new();
        if self.spirit_timer <= 25. {
            self.spirit_timer = 24.;
            self.spirit_state(0, SpiritState::Idle, &mut out);
            if self.both_spirits {
                self.spirit_state(1, SpiritState::Idle, &mut out);
            }
        }
        out
    }
    pub fn update(&mut self, dt: f32, inputs: Inputs) -> Vec<Command> {
        let mut out = Vec::new();
        match self.state {
            State::Idle => {
                if self.status.time_since_last_damage < 1. {
                    self.idle = 0.;
                }
                if self.idle <= 0. {
                    let spirit_chance = ((self.spirit_timer - 25.) / 25.).max(0.);
                    let state = if self.random() <= spirit_chance {
                        State::Spirit
                    } else if self.random() < self.status.time_since_last_damage.min(0.75) {
                        State::Spell
                    } else {
                        State::Magick
                    };
                    self.change(state, inputs, &mut out);
                }
                self.idle -= dt;
            }
            State::Spirit
                if self.controller.has_finished() && !self.controller.crossfade_enabled() =>
            {
                self.spirit_timer = 0.;
                self.float = [0.; 2];
                self.change(State::Idle, inputs, &mut out);
            }
            State::Magick => {
                if self.idle <= 0. {
                    self.change(State::Idle, inputs, &mut out);
                }
                self.idle -= dt;
            }
            State::Spell
                if self.controller.has_finished() && !self.controller.crossfade_enabled() =>
            {
                let (spell, cast, power) = main_spell(self.last_spell);
                out.push(Command::Cast {
                    part: 0,
                    spell,
                    cast,
                    power,
                });
                self.change(State::Idle, inputs, &mut out);
            }
            State::Heal if !self.healing => self.change(State::Idle, inputs, &mut out),
            _ => {}
        }
        self.status.time_since_last_damage += dt;
        if inputs.fight_started && self.state == State::Intro {
            self.change(State::Idle, inputs, &mut out);
            for i in 0..2 {
                self.spirit_state(i, SpiritState::Idle, &mut out);
            }
        }
        let mut events = Vec::new();
        self.status.update_damage(dt, false, &mut events);
        for (amount, element) in self.status.update_statuses(dt, false) {
            self.status.status_damage(amount, element);
            self.clamp_health();
            self.status.time_since_last_status_damage = 0.;
            if self.damage_flash <= 0. {
                self.damage_flash = 0.1;
            }
        }
        out.extend(events.into_iter().map(Command::Boss));
        self.controller.speed = if self.status.has(Status::Frozen) {
            0.
        } else if self.status.has(Status::Cold) {
            0.2
        } else {
            1.
        };
        self.controller.update(dt);
        self.pose = ModelPose {
            parent: self.orientation,
            local: self.skeleton.local_poses(self.controller.pose_sample()),
        };
        self.damage_flash = (self.damage_flash - dt).max(0.);
        for i in 0..3 {
            self.target[i] += (inputs.target[i] - self.target[i]) * dt * 2.;
        }
        if !self.healing {
            self.spirit_timer += dt;
            self.float[0] += dt * 0.075;
            self.float[1] += dt * 0.05;
        }
        for i in 0..2 {
            // Native starts both casters before FutureVladDone but hides the
            // right one. Keep those attacks and give every active caster a
            // visible source (research/grimnir-invisible-spirit.md).
            self.spirits[i].drawn = self.spirit_timer <= 25.
                && (i == 0 || self.both_spirits || self.spirits[i].state != SpiritState::Intro);
            if self.spirit_timer <= 25. {
                self.update_spirit(i, dt, &mut out);
            } else {
                out.push(Command::StopSpell(i + 1));
            }
        }
        if self.spirit_timer <= 25. {
            self.spectral_fade += (self.spectral_target - self.spectral_fade) * dt;
            self.spectral_alpha = (self.spirit_timer.min(1.) * (25. - self.spirit_timer).max(0.))
                .min(1.)
                * self.spectral_fade;
        }
        if self.healing {
            if self.state != State::Heal {
                self.called_appear = false;
                self.change(State::Heal, inputs, &mut out);
                for i in 0..2 {
                    self.spirit_state(i, SpiritState::Heal, &mut out);
                }
            }
            self.assatur_alpha = (self.assatur_alpha + dt * 0.25).min(1.);
            if self.assatur_alpha >= 1. {
                if self.loop_fight && !self.called_appear {
                    self.called_appear = true;
                    out.push(Command::Trigger("assatur_appears"));
                }
                if self.assatur_controller.clip_name() == Some("idle")
                    && !self.assatur_controller.crossfade_enabled()
                {
                    self.assatur_controller.crossfade_named_clip(
                        "cast_self",
                        self.assatur_skeleton.clip_durations()["cast_self"],
                        0.5,
                        true,
                    );
                }
            }
        } else {
            self.assatur_alpha = (self.assatur_alpha - dt * 2.).max(0.);
        }
        self.assatur_controller.update(dt);
        let mut parent = self.assatur_orientation;
        parent[13] += self.float[1].sin() * 0.5;
        self.assatur_pose = ModelPose {
            parent,
            local: self
                .assatur_skeleton
                .local_poses(self.assatur_controller.pose_sample()),
        };
        out
    }
    fn update_spirit(&mut self, index: usize, dt: f32, out: &mut Vec<Command>) {
        match self.spirits[index].state {
            SpiritState::Idle => {
                self.spirits[index].idle -= dt;
                if self.spirits[index].idle <= 0. {
                    self.spirit_state(index, SpiritState::Cast, out);
                }
            }
            SpiritState::Cast => {
                let s = &mut self.spirits[index];
                if !s.controller.crossfade_enabled() {
                    let t = s.controller.time / s.controller.clip_duration();
                    if !s.has_cast && t >= if s.last_spell == 0 { 0.15 } else { 0.6 } {
                        s.has_cast = true;
                        let (spell, cast, power) = spirit_spell(s.last_spell);
                        out.push(Command::Cast {
                            part: index + 1,
                            spell,
                            cast,
                            power,
                        });
                    }
                    if s.controller.has_finished() {
                        self.spirit_state(index, SpiritState::Idle, out);
                    }
                }
            }
            SpiritState::Heal => {
                self.spirits[index].idle -= dt;
                if self.spirits[index].idle <= 0. {
                    self.spectral_target = if self.random() > 0. { 0. } else { 1. };
                    self.spirits[index].idle = 0.25;
                }
                if !self.healing {
                    self.spirit_state(index, SpiritState::Idle, out);
                }
            }
            _ => {}
        }
        let phase = self.float[if index == 0 { 1 } else { 0 }].sin().powi(2);
        let amount = (self.spirit_timer * 0.25).min(1.);
        let sign = if index == 0 { 1. } else { -1. };
        let angle = std::f32::consts::PI
            + sign * phase.abs() * amount * std::f32::consts::FRAC_PI_2
            + sign * std::f32::consts::PI / 5. * amount;
        let rotation = skinned::pose_matrix(
            [0.; 3],
            [0., (angle * 0.5).sin(), 0., (angle * 0.5).cos()],
            [1.; 3],
        );
        let mut parent = skinned::mul(&rotation, &self.orientation);
        let distance = amount * (10. + phase * 0.25);
        let forward = [
            -self.orientation[8] * distance,
            -self.orientation[9] * distance,
            -self.orientation[10] * distance,
        ];
        let rotated: [f32; 3] = std::array::from_fn(|i| {
            rotation[i] * forward[0] + rotation[4 + i] * forward[1] + rotation[8 + i] * forward[2]
        });
        for i in 0..3 {
            parent[12 + i] += rotated[i] - self.orientation[8 + i] * amount * 10.;
        }
        parent[13] = self.target[1] - 1.5 * amount;
        let direction = [self.target[0] - parent[12], 0., self.target[2] - parent[14]];
        let len = direction[0].hypot(direction[2]);
        if len > 1e-6 {
            let f = direction.map(|v| v / len);
            parent[0] = -f[2];
            parent[1] = 0.;
            parent[2] = f[0];
            parent[4] = 0.;
            parent[5] = 1.;
            parent[6] = 0.;
            parent[8] = -f[0];
            parent[9] = 0.;
            parent[10] = -f[2];
        }
        self.spirits[index].controller.update(dt);
        self.spirits[index].pose = ModelPose {
            parent,
            local: self
                .skeleton
                .local_poses(self.spirits[index].controller.pose_sample()),
        };
    }
}
fn recipe(parts: &[(E, f32)]) -> Spell {
    let mut s = Spell::default();
    for &(e, m) in parts {
        s.element |= e;
        let i = E::BY_INDEX.iter().position(|v| *v == e).unwrap();
        s.mag[i] = m;
    }
    s
}
pub fn spirit_spell(index: usize) -> (Spell, CastType, f32) {
    if index == 0 {
        (
            recipe(&[(E::LIGHTNING, 3.), (E::ARCANE, 1.)]),
            CastType::Force,
            1.,
        )
    } else {
        (
            recipe(&[(E::EARTH, 2.), (E::FIRE, 2.)]),
            CastType::Force,
            0.1,
        )
    }
}
pub fn main_spell(index: usize) -> (Spell, CastType, f32) {
    match index {
        0 => (recipe(&[(E::ICE, 5.)]), CastType::Area, 1.),
        1 => (
            recipe(&[(E::EARTH, 4.), (E::SHIELD, 1.)]),
            CastType::Force,
            1.,
        ),
        _ => (recipe(&[(E::SHIELD, 1.)]), CastType::Force, 1.),
    }
}
