//! `Magicka.GameLogic.Entities.Bosses.Parker`, the giant spider of the Tsar
//! DLC (Tsar_WizardCastle twc_s3BF), as a deterministic state machine:
//! intro, pivoting battle stance, swipes, rush and bite, web strands,
//! spiderling spawns, retreats into the cave (healing, calling spiders) and
//! death. Each update returns ordered [`ParkerCommand`]s; collision
//! callbacks are host-computed contacts fed back through
//! [`Parker::body_contact`] and [`Parker::mouth_contact`]. See
//! research/boss-parker.md.
use crate::animation_controller::{AnimationController, ControllerSignal};
use crate::character_pose::CharacterSkeleton;
use crate::xna_math::{self as m, V3};
use magicka_content::skinned::{self, Mat, Resolved};
use magicka_sim::boss_status::{BossEvent, BossStatus};
use magicka_sim::character_impact::result as R;
use magicka_sim::damage::{attack, Damage};
use magicka_sim::spells::Elements as E;
use magicka_sim::spray_pulse::HitList;
use magicka_sim::status::Status;

pub const MESH_MODEL: &str = "Models/Bosses/parker/parker";
pub const ANIMATION_MODEL: &str = "Models/Bosses/parker/parker_animation";
pub const MAX_HIT_POINTS: f32 = 55000.0;
pub const SPAWN_LOCATOR: &str = "parker_spawn";
pub const BATTLE_LOCATOR: &str = "parker_battle";
pub const DEFEND_LOCATOR: &str = "parker_defensive";
/// CallMinionState's fallback locators.
pub const MINION_LOCATORS: [&str; 4] = ["boss_start0", "boss_start1", "boss_start2", "boss_start3"];
pub const SPIDERLING: &str = "spider_baby";
pub const FOREST_SPIDER: &str = "spider_forest";
pub const POISON_SPIDER: &str = "spider_poison";
/// SprayEntity's Spray_web strand texture and its hit effect.
pub const WEB_TEXTURE: &str = "EffectTextures/Spray_web";

/// mAnimationClips (Parker.Animations).
pub mod clip {
    pub const WALK_BACKWARD: usize = 0;
    pub const WALK_FORWARD: usize = 1;
    pub const WALK_LEFT: usize = 2;
    pub const WALK_RIGHT: usize = 3;
    pub const IDLE: usize = 4;
    pub const ROTATE_LEFT: usize = 5;
    pub const ROTATE_RIGHT: usize = 6;
    pub const ATTACK_LEFT: usize = 7;
    pub const ATTACK_RIGHT: usize = 8;
    pub const ATTACK_FRONT: usize = 9;
    pub const BITE: usize = 10;
    pub const WEB: usize = 11;
    pub const SPAWN: usize = 12;
    pub const DEFEND: usize = 13;
    pub const STUN: usize = 14;
    pub const CAVE_ENTER: usize = 15;
    pub const CAVE_HELP: usize = 16;
    pub const CAVE_IDLE: usize = 17;
    pub const CAVE_EXIT: usize = 19;
    pub const INTRO: usize = 20;
    pub const DIE: usize = 22;
    pub const NAMES: [&str; 23] = [
        "walk_backward",
        "walk_forward",
        "walk_left",
        "walk_right",
        "idle",
        "rotate_left",
        "rotate_right",
        "attack_left",
        "attack_right",
        "attack_front",
        "bite",
        "web",
        "spawn",
        "defend",
        "stun",
        "cave_enter",
        "cave_help",
        "cave_idle",
        "cave_taunt",
        "cave_exit",
        "intro",
        "taunt",
        "die",
    ];
}

const WEBBING_TIME: [f32; 2] = [13.0 / 45.0, 0.8];
const BITE_TIME: [f32; 2] = [0.548_387_1, 0.935_483_9];
const ATTACK_LEFT_RIGHT_TIME: [f32; 2] = [0.28, 0.56];
const SPAWN_TIME: [f32; 2] = [11.0 / 38.0, 0.789_473_7];
const MINION_TIME: [f32; 2] = [11.0 / 38.0, 0.789_473_7];
const BITE_SOUND_TIME: f32 = 0.612_903_24;
const FOOTSTEP_TIMES: [f32; 4] = [0.0, 7.0 / 32.0, 15.0 / 32.0, 23.0 / 32.0];
const WEB_SEGMENTS: usize = 10;
const SPAWN_COUNT: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Intro,
    Battle,
    LeaveCave,
    EnterCave,
    Swipe,
    Bite,
    Rush,
    Backup,
    Stun,
    Web,
    Spawn,
    CallMinions,
    Defend,
    Rotate,
    MoveToBattle,
    MoveToCave,
    Cave,
    Dead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Intro,
    Battle,
    Critical,
    Final,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swipe {
    Left,
    Right,
    Front,
}

/// Which zone's AudioEmitter a cue follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emitter {
    Abdomen,
    Mouth,
}

/// A player slot as Parker reads it (Game.Instance.Players).
#[derive(Clone, Copy, Debug, Default)]
pub struct Player {
    pub playing: bool,
    /// The avatar's handle; `None` without an avatar.
    pub avatar: Option<u16>,
    pub dead: bool,
    pub position: V3,
    pub entangled: bool,
    pub poisoned: bool,
}

/// Scene queries Parker makes while updating.
pub trait ParkerWorld {
    /// CollisionSkin.SegmentIntersect of the scene: the hit position.
    fn ground(&mut self, origin: V3, delta: V3) -> Option<V3>;
    /// Any shield intersects the segment.
    fn shield_blocks(&mut self, origin: V3, delta: V3) -> bool;
    /// GetTriggerArea("any").GetCount(type).
    fn count(&mut self, template: &str) -> usize;
    /// GameScene.GetLocator; `None` when missing.
    fn locator(&mut self, name: &str) -> Option<Mat>;
    /// NavMesh.GetNearestPosition (MovementProperties.Default).
    fn nav_nearest(&mut self, point: V3) -> V3;
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParkerCommand {
    /// AudioManager.PlayCue at a zone emitter; `characters` selects
    /// Banks.Characters over Banks.Additional.
    Cue {
        name: &'static str,
        emitter: Emitter,
        characters: bool,
    },
    /// NonPlayerCharacter spawned at `position`, facing `direction`,
    /// ordered Attack (or Idle), reacting to Attack|Proximity with Attack,
    /// in RessurectionState playing Animations.spawn.
    Spawn {
        template: &'static str,
        position: V3,
        direction: V3,
        attack: bool,
        dead_timer: f32,
    },
    /// SprayEntity.Initialize: strand `index` (1..=10) of the current web;
    /// strands 1..9 are tethered to the mouth until a later strand becomes
    /// their child.
    Web {
        index: usize,
        position: V3,
        direction: V3,
        velocity: f32,
        mouth_owner: bool,
    },
    /// SwipeState's GetEntities(center, radius) sweep with knockback of the
    /// target's mass (magnitude 3) and 50 earth damage.
    SwipeSweep {
        center: V3,
        radius: f32,
    },
    Boss(BossEvent),
}

/// The host-side outcome of a contact callback.
#[derive(Clone, Debug, PartialEq)]
pub enum Contact {
    /// Collision only.
    None,
    /// Damage collection on the target (attacker zone, attack position).
    Damage {
        damages: Vec<Damage>,
        mouth: bool,
        position: V3,
    },
    /// A shield or barrier caught in the bite: Kill() it.
    KillBlocker,
}

pub use crate::boss_fafnir::ModelPose;

#[derive(Clone, Debug, Default)]
struct Timers {
    swipe_weight: f32,
    web_weight: f32,
    spawn_weight: f32,
    minion_weight: f32,
}

#[derive(Clone, Debug)]
pub struct Parker {
    pub status: BossStatus,
    body_resistances: [magicka_sim::resistance::Resistance; 11],
    head_resistances: [magicka_sim::resistance::Resistance; 11],
    pub state: State,
    pub previous: State,
    pub stage: Stage,
    pub controller: AnimationController,
    pub skeleton: CharacterSkeleton,
    durations: std::collections::HashMap<String, f32>,
    mouth_bone: usize,
    abdomen_bone: usize,
    back_abdomen_bone: usize,
    foot_bones: [usize; 8],
    knee_bones: [usize; 8],
    body_bones: [usize; 8],
    binds: std::collections::HashMap<usize, Mat>,
    pub dead: bool,
    pub transform: Mat,
    battle: Mat,
    spawn: Mat,
    defend: Mat,
    pivot: Mat,
    pivot_distance: f32,
    pub pivot_target_direction: V3,
    movement: V3,
    desired_direction: V3,
    movement_speed: f32,
    stage_speed: f32,
    pub time_between_actions: f32,
    state_speed: f32,
    animation_speed: f32,
    movement_speed_mod: f32,
    pub target: Option<u16>,
    pub target_webbed: bool,
    player_target_position: V3,
    players_dead: bool,
    /// mIsHit: never set true by the class, so Defend and the Final
    /// stage's hit retreat are unreachable (kept for fidelity).
    is_hit: bool,
    hit_timer: f32,
    pub hit_list: HitList,
    rotate_direction: V3,
    rotation_transition: State,
    timers: Timers,
    battle_animation: usize,
    played: bool,
    pub swipe: Swipe,
    rush_target: V3,
    backup_target: V3,
    web_timer: f32,
    web_spray_time: f32,
    pub web_count: usize,
    spawn_timer: f32,
    spawn_count: usize,
    minion_timer: f32,
    minion_spawn_time: f32,
    minion_count: usize,
    minion_spawns: usize,
    cave_modifier: f32,
    cave_timer: f32,
    footstep: i32,
    pub pose: ModelPose,
    /// mMouthOrientation.
    pub mouth: Mat,
    /// mAbdomPosition (the abdomen zone's position; its sphere is 1 up).
    pub abdomen: V3,
    pub feet: [Mat; 8],
    /// BossCollisionZone capsules: (start, end, radius), knee→foot then
    /// body→knee for each leg.
    pub limbs: Vec<(V3, V3, f32)>,
    /// Per player slot (four in the original; more in larger lobbies).
    players: Vec<Player>,
    pending: Vec<ParkerCommand>,
    /// mBackAbdomBindPose when no "Ass" bone set it (the zero matrix).
    back_bind: Option<Mat>,
    rng: u64,
}

fn bones_have(skeleton: &CharacterSkeleton, name: &str) -> bool {
    skeleton
        .skeleton
        .bones
        .iter()
        .any(|b| b.name.eq_ignore_ascii_case(name))
}

fn rotation_y_pi() -> Mat {
    crate::boss_machine::rotation_y_pi()
}

fn resistances(f: impl Fn(E) -> (f32, f32)) -> [magicka_sim::resistance::Resistance; 11] {
    std::array::from_fn(|i| {
        let e = E::BY_INDEX[i];
        let (multiplier, modifier) = f(e);
        magicka_sim::resistance::Resistance {
            against: e,
            multiplier,
            modifier,
            status_resistance: false,
        }
    })
}

impl Parker {
    pub fn new(mesh: Resolved, animation: Resolved, seed: u64) -> Result<Self, String> {
        let _ = mesh;
        // The controller and the bone lookups use the animation skeleton.
        let skeleton = CharacterSkeleton::new(animation.skeleton.clone(), animation.clips.clone());
        let durations = skeleton.clip_durations();
        for name in clip::NAMES {
            if !durations.contains_key(name) {
                return Err(format!("Parker animation lacks clip {name}"));
            }
        }
        let bones = &skeleton.skeleton.bones;
        let find = |name: &str| {
            bones
                .iter()
                .position(|b| b.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("Parker skeleton lacks {name}"))
        };
        let (mut feet, mut knees, mut bodies) = (Vec::new(), Vec::new(), Vec::new());
        // Constructor order: skeleton order, feet *5, knees *1, bodies *0.
        for (i, b) in bones.iter().enumerate() {
            let n = b.name.to_ascii_lowercase();
            let leg = (0..4).any(|k| {
                n.starts_with(&format!("leftleg{k}")) || n.starts_with(&format!("rightleg{k}"))
            }) && n.len() == if n.starts_with("left") { 9 } else { 10 };
            if !leg {
                continue;
            }
            match n.as_bytes()[n.len() - 1] {
                b'5' => feet.push(i),
                b'1' => knees.push(i),
                b'0' => bodies.push(i),
                _ => {}
            }
        }
        let eight = |v: Vec<usize>, what: &str| -> Result<[usize; 8], String> {
            v.try_into()
                .map_err(|v: Vec<usize>| format!("Parker has {} {what} bones", v.len()))
        };
        let mut p = Self {
            status: BossStatus::default(),
            body_resistances: resistances(|e| match e {
                E::EARTH | E::ARCANE => (1.25, 0.0),
                E::COLD => (0.5, 0.0),
                E::FIRE => (1.5, 0.0),
                E::POISON => (0.0, 0.0),
                _ => (1.0, 0.0),
            }),
            // Every listed element is set; none is left at the zero default.
            head_resistances: resistances(|e| match e {
                E::EARTH => (0.5, 40.0),
                E::COLD => (0.5, 0.0),
                E::FIRE => (1.5, 0.0),
                E::POISON => (0.0, 0.0),
                _ => (1.0, 0.0),
            }),
            state: State::Intro,
            previous: State::Intro,
            stage: Stage::Intro,
            controller: AnimationController::default(),
            durations,
            mouth_bone: find("Mouth")?,
            abdomen_bone: find("Spine1")?,
            // Retail parker_animation has no "Ass" bone: the index stays 0
            // and its bind pose the zero matrix (see point_of).
            back_abdomen_bone: find("Ass").unwrap_or(0),
            foot_bones: eight(feet, "foot")?,
            knee_bones: eight(knees, "knee")?,
            body_bones: eight(bodies, "body")?,
            binds: Default::default(),
            skeleton,
            dead: false,
            transform: skinned::IDENTITY,
            battle: skinned::IDENTITY,
            spawn: skinned::IDENTITY,
            defend: skinned::IDENTITY,
            pivot: skinned::IDENTITY,
            pivot_distance: 0.0,
            pivot_target_direction: [0.0; 3],
            movement: [0.0; 3],
            desired_direction: [0.0, 0.0, -1.0],
            movement_speed: 8.0,
            stage_speed: 1.0,
            time_between_actions: 3.0,
            state_speed: 1.0,
            animation_speed: 1.0,
            movement_speed_mod: 1.0,
            target: None,
            target_webbed: false,
            player_target_position: [0.0; 3],
            players_dead: false,
            is_hit: false,
            hit_timer: 0.0,
            hit_list: HitList::default(),
            rotate_direction: [0.0; 3],
            rotation_transition: State::Battle,
            timers: Timers::default(),
            battle_animation: clip::INTRO,
            played: false,
            swipe: Swipe::Front,
            rush_target: [0.0; 3],
            backup_target: [0.0; 3],
            web_timer: 0.0,
            web_spray_time: 0.0,
            web_count: 0,
            spawn_timer: 0.0,
            spawn_count: 0,
            minion_timer: 0.0,
            minion_spawn_time: 0.0,
            minion_count: 0,
            minion_spawns: 0,
            cave_modifier: 1.0,
            cave_timer: 0.0,
            footstep: -1,
            pose: ModelPose::default(),
            mouth: skinned::IDENTITY,
            abdomen: [0.0; 3],
            feet: [skinned::IDENTITY; 8],
            limbs: Vec::new(),
            players: vec![Player::default(); 4],
            pending: Vec::new(),
            back_bind: None,
            rng: seed | 1,
        };
        let has_back = bones_have(&p.skeleton, "Ass");
        let all: Vec<usize> = [p.mouth_bone, p.abdomen_bone]
            .into_iter()
            .chain(has_back.then_some(p.back_abdomen_bone))
            .chain(p.foot_bones)
            .chain(p.knee_bones)
            .chain(p.body_bones)
            .collect();
        for b in all {
            let inv = skinned::mul(&p.skeleton.skeleton.bones[b].inverse_bind, &rotation_y_pi());
            let bind =
                crate::live_collision::inverse(&inv).map_err(|e| format!("bind {b}: {e:?}"))?;
            p.binds.insert(b, bind);
        }
        if !has_back {
            p.back_bind = Some([0.0; 16]);
        }
        Ok(p)
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

    fn duration(&self, index: usize) -> f32 {
        self.durations
            .get(clip::NAMES[index])
            .copied()
            .unwrap_or(0.0)
    }
    fn start(&mut self, index: usize, looping: bool) {
        let d = self.duration(index);
        self.controller
            .start_named_clip(clip::NAMES[index], d, looping);
    }
    fn crossfade(&mut self, index: usize, fade: f32, looping: bool) {
        let d = self.duration(index);
        self.controller
            .crossfade_named_clip(clip::NAMES[index], d, fade, looping);
    }
    fn playing(&self, index: usize) -> bool {
        self.controller.clip_name() == Some(clip::NAMES[index])
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
        self.controller.has_finished()
    }
    fn fading(&self) -> bool {
        self.controller.crossfade_enabled()
    }
    fn position(&self) -> V3 {
        m::translation(&self.transform)
    }
    fn forward(&self) -> V3 {
        m::forward(&self.transform)
    }
    fn battle_state_transform(&self) -> Mat {
        let mut r = self.pivot;
        let t = m::add(
            m::translation(&self.pivot),
            m::scale(m::forward(&self.pivot), self.pivot_distance),
        );
        m::set_translation(&mut r, t);
        r
    }
    fn reset_battle_transform(&mut self) {
        self.pivot = self.spawn;
        m::set_translation(&mut self.pivot, m::translation(&self.defend));
    }
    fn cold_speed(&self) -> f32 {
        self.status.statuses[magicka_sim::boss_status::status_index(Status::Cold)]
            .slowdown()
            .max(0.666)
    }
    /// The Movement setter: flattened; normalized above unit length, and
    /// the desired direction follows any non-zero movement.
    fn set_movement(&mut self, mut v: V3) {
        v[1] = 0.0;
        self.movement = v;
        let l2 = m::length_squared(v);
        if l2 > 1e-6 {
            let l = l2.sqrt();
            self.desired_direction = m::scale(v, 1.0 / l);
            if l > 1.0 {
                self.movement = self.desired_direction;
            }
        }
    }
    fn living(&self, slot: usize) -> Option<&Player> {
        let p = self.players.get(slot)?;
        (p.playing && p.avatar.is_some() && !p.dead).then_some(p)
    }
    fn target_player(&self) -> Option<&Player> {
        let t = self.target?;
        self.players.iter().find(|p| p.avatar == Some(t))
    }
    /// The target is null or dead (a departed avatar reads as dead).
    fn target_dead(&self) -> bool {
        self.target_player().is_none_or(|p| p.dead)
    }
    fn distance_squared(&self, p: V3) -> f32 {
        let a = self.position();
        let (dx, dz) = (a[0] - p[0], a[2] - p[2]);
        dx * dx + dz * dz
    }

    /// Initialize(orientation) after the zones were placed.
    pub fn initialize(&mut self, orientation: Mat, world: &mut dyn ParkerWorld) {
        self.dead = false;
        self.status.max_hit_points = MAX_HIT_POINTS;
        self.status.hit_points = MAX_HIT_POINTS;
        self.transform = orientation;
        // A missing locator leaves GetLocator's out parameter as identity.
        self.battle = world.locator(BATTLE_LOCATOR).unwrap_or(skinned::IDENTITY);
        self.spawn = world.locator(SPAWN_LOCATOR).unwrap_or(skinned::IDENTITY);
        self.defend = world.locator(DEFEND_LOCATOR).unwrap_or(skinned::IDENTITY);
        self.pivot_distance = m::length(m::sub(
            m::translation(&self.defend),
            m::translation(&self.battle),
        ));
        self.reset_battle_transform();
        self.transform = self.battle_state_transform();
        self.movement = [0.0; 3];
        self.is_hit = false;
        self.hit_timer = 0.0;
        self.players_dead = false;
        self.hit_list.clear();
        self.stage_speed = 1.0;
        self.state_speed = 1.0;
        self.animation_speed = 1.0;
        self.movement_speed_mod = 1.0;
        self.stage = Stage::Intro;
        self.enter_stage();
        self.state = State::Intro;
        let mut out = Vec::new();
        self.enter(world, &mut out);
        self.status.statuses = Default::default();
        self.status.current = 0;
        self.status.resistances = self.body_resistances;
        self.controller.speed = self.stage_speed;
        self.target_webbed = false;
        self.footstep = -1;
        self.pose_update(0.0);
        self.pending = out;
    }

    /// Commands raised by Initialize (the intro cue), drained by the host.
    pub fn take_pending(&mut self) -> Vec<ParkerCommand> {
        std::mem::take(&mut self.pending)
    }

    pub fn health(&self) -> (f32, f32) {
        (self.status.hit_points, MAX_HIT_POINTS)
    }

    /// Damage(part, Damage, ...): no damage while defending, calling
    /// minions or in the cave; part 0 is the abdomen (body resistances),
    /// part 1 the mouth (head resistances).
    pub fn damage(
        &mut self,
        part: usize,
        d: Damage,
        position: V3,
        features: u8,
        out: &mut Vec<ParkerCommand>,
    ) -> u16 {
        if matches!(self.state, State::Defend | State::CallMinions | State::Cave) {
            return 0;
        }
        match part {
            0 => self.status.resistances = self.body_resistances,
            1 => self.status.resistances = self.head_resistances,
            _ => {}
        }
        let mut events = Vec::new();
        let r = self
            .status
            .damage(d, self.dead, position, features, &mut events);
        out.extend(events.into_iter().map(ParkerCommand::Boss));
        r
    }

    /// Damage(part, amount, element): status ticks; the mouth takes none
    /// while defending.
    pub fn status_damage(&mut self, part: usize, amount: f32, element: E) {
        match part {
            0 => {
                self.status.resistances = self.body_resistances;
                self.status.status_damage(amount, element);
            }
            1 if self.state != State::Defend => {
                self.status.resistances = self.head_resistances;
                self.status.status_damage(amount, element);
            }
            _ => {}
        }
    }

    /// GetRandomTarget: random slots without repeats until a living avatar.
    fn random_target(&mut self) -> Option<u16> {
        let mut tried = 0u8;
        while tried != 0b1111 {
            let i = self.random_below(4);
            if tried & (1 << i) != 0 {
                continue;
            }
            tried |= 1 << i;
            if let Some(p) = self.living(i) {
                return p.avatar;
            }
        }
        None
    }

    /// SetTarget: kept while the current target is entangled or poisoned.
    fn set_target(&mut self, avatar: u16) {
        let keep = self
            .target_player()
            .is_some_and(|p| p.entangled || p.poisoned);
        if self.target.is_none() || !keep {
            self.target = Some(avatar);
        }
    }

    fn change(&mut self, state: State, world: &mut dyn ParkerWorld, out: &mut Vec<ParkerCommand>) {
        self.exit();
        self.previous = self.state;
        self.state = state;
        self.enter(world, out);
    }

    fn exit(&mut self) {
        match self.state {
            State::Battle => {
                self.state_speed = 1.0;
                self.animation_speed = 1.0;
            }
            State::Defend => {
                self.state_speed = 1.0;
                self.is_hit = false;
                self.hit_timer = 8.0;
            }
            State::Stun | State::Swipe | State::Bite | State::Rotate | State::EnterCave => {
                self.state_speed = 1.0
            }
            State::Rush => {
                self.movement_speed_mod = 1.0;
                self.state_speed = 1.0;
                self.set_movement([0.0; 3]);
            }
            State::Backup => {
                self.state_speed = 1.0;
                self.set_movement([0.0; 3]);
            }
            State::Web => {
                self.timers.web_weight = self.time_between_actions;
                self.state_speed = 1.0;
            }
            State::Spawn => self.timers.spawn_weight = 20.0,
            State::CallMinions => self.timers.minion_weight = 0.0,
            State::MoveToCave | State::MoveToBattle => {
                self.movement_speed_mod = 1.0;
                self.animation_speed = 1.0;
                self.state_speed = 1.0;
                self.set_movement([0.0; 3]);
            }
            State::Intro | State::LeaveCave | State::Cave | State::Dead => {}
        }
    }

    fn enter(&mut self, world: &mut dyn ParkerWorld, out: &mut Vec<ParkerCommand>) {
        let cue = |name, emitter| ParkerCommand::Cue {
            name,
            emitter,
            characters: false,
        };
        match self.state {
            State::Intro => {
                out.push(cue("parker_intro", Emitter::Abdomen));
                self.start(clip::INTRO, false);
                self.state_speed = 1.0;
            }
            State::Battle => {
                self.battle_animation = if self.playing(clip::WALK_LEFT) {
                    clip::WALK_LEFT
                } else if self.playing(clip::WALK_RIGHT) {
                    clip::WALK_RIGHT
                } else if self.playing(clip::IDLE) {
                    clip::IDLE
                } else {
                    clip::INTRO
                };
                self.state_speed = 1.0;
                self.animation_speed = 1.0;
                if self.target.is_some() && self.target_dead() {
                    self.target = None;
                }
                if self.target.is_none() {
                    if let Some(a) = self.random_target() {
                        self.set_target(a);
                    }
                }
                if self.rush_weight(world) > 0.0 {
                    self.change(State::Rush, world, out);
                } else if self.web_weight(0.0) > 0.0 {
                    self.change(State::Web, world, out);
                } else if let Some(a) = self.random_target() {
                    let keep = self
                        .target_player()
                        .is_some_and(|p| !self.target_webbed && !p.poisoned);
                    if keep {
                        self.set_target(a);
                    }
                }
            }
            State::Defend => {
                self.played = false;
                self.state_speed = 1.5;
                self.set_movement([0.0; 3]);
                self.crossfade(clip::DEFEND, 0.15, false);
            }
            State::Stun => {
                self.set_movement([0.0; 3]);
                self.crossfade(clip::STUN, 0.25, false);
            }
            State::Swipe => {
                self.set_movement([0.0; 3]);
                let c = match self.swipe {
                    Swipe::Left => clip::ATTACK_LEFT,
                    Swipe::Right => clip::ATTACK_RIGHT,
                    Swipe::Front => clip::ATTACK_FRONT,
                };
                self.crossfade(c, 0.25, false);
                self.played = false;
            }
            State::Rush => {
                self.state_speed = 1.0;
                self.set_movement([0.0; 3]);
                self.crossfade(clip::WALK_FORWARD, 0.5, true);
                self.rush_target = match self.target_player() {
                    Some(p) => p.position,
                    None => m::add(self.position(), m::scale(self.forward(), 13.0)),
                };
            }
            State::Backup => {
                self.state_speed = 1.0;
                self.backup_target = m::translation(&self.battle_state_transform());
                self.crossfade(clip::WALK_BACKWARD, 0.2, true);
            }
            State::Bite => {
                self.played = false;
                self.state_speed = 1.5;
                self.set_movement([0.0; 3]);
                self.crossfade(clip::BITE, 0.25, false);
            }
            State::Web => {
                self.state_speed = 1.75;
                self.set_movement([0.0; 3]);
                self.crossfade(clip::WEB, 0.75, false);
                self.web_timer = 0.0;
                self.web_count = 0;
                self.web_spray_time = self.duration(clip::WEB)
                    * (WEBBING_TIME[1] - WEBBING_TIME[0])
                    / (self.state_speed * self.stage_speed * self.animation_speed);
                out.push(ParkerCommand::Cue {
                    name: "chr_spider_web_pre",
                    emitter: Emitter::Mouth,
                    characters: true,
                });
            }
            State::Spawn => {
                self.state_speed = 1.0;
                self.crossfade(clip::SPAWN, 0.25, false);
                self.spawn_timer = 0.0;
                self.spawn_count = 0;
                // mSpawnTime is computed (from the web clip) but never used.
                self.set_movement([0.0; 3]);
                self.played = false;
            }
            State::Rotate => {
                let a = m::signed_angle(self.forward(), self.rotate_direction);
                if a.abs() < 0.05 {
                    let next = self.rotation_transition;
                    self.change(next, world, out);
                } else if a > 0.0 {
                    self.crossfade(clip::ROTATE_LEFT, 0.25, true);
                } else {
                    self.crossfade(clip::ROTATE_RIGHT, 0.25, true);
                }
            }
            State::MoveToCave => {
                self.state_speed = 1.0;
                self.crossfade(clip::WALK_BACKWARD, 0.25, true);
            }
            State::LeaveCave => {
                self.played = false;
                self.state_speed = 1.0;
                self.crossfade(clip::CAVE_EXIT, 0.5, false);
            }
            State::MoveToBattle => {
                self.reset_battle_transform();
                self.state_speed = 1.0;
                self.crossfade(clip::WALK_FORWARD, 0.2, true);
                self.movement_speed_mod = 0.0;
            }
            State::EnterCave => {
                self.state_speed = 1.0;
                self.crossfade(clip::CAVE_ENTER, 0.5, false);
                self.played = false;
                self.stop_statuses();
            }
            State::Cave => {
                self.state_speed = 1.0;
                self.set_movement([0.0; 3]);
                self.crossfade(clip::CAVE_IDLE, 0.25, true);
                self.cave_modifier = 1.0 + (3.0 - self.time_between_actions);
                self.cave_timer = self.cave_modifier;
                self.stop_statuses();
            }
            State::CallMinions => {
                self.set_movement([0.0; 3]);
                self.crossfade(clip::CAVE_HELP, 0.25, false);
                // mNrOfSpawns was cached by the weight's Count getter.
                self.minion_spawn_time = self.duration(clip::CAVE_ENTER)
                    * (MINION_TIME[1] - MINION_TIME[0])
                    / (self.minion_spawns as f32
                        * self.stage_speed
                        * self.state_speed
                        * self.cold_speed());
                self.minion_count = 0;
                self.minion_timer = 0.0;
                self.played = false;
            }
            State::Dead => {
                self.played = false;
                self.set_movement([0.0; 3]);
                self.crossfade(clip::DIE, 0.125, false);
            }
        }
    }

    fn stop_statuses(&mut self) {
        for s in &mut self.status.statuses {
            s.stop();
        }
    }

    fn enter_stage(&mut self) {
        (self.stage_speed, self.time_between_actions) = match self.stage {
            Stage::Intro => (1.0, 3.0),
            Stage::Battle => (1.0, 2.0),
            Stage::Critical => (1.5, 1.0),
            Stage::Final => (2.0, 0.0),
        };
    }

    fn players_count(&self) -> usize {
        self.players.iter().filter(|p| p.playing).count().max(1)
    }

    /// CallMinionState.Count: 3 + (PlayerCount − 1), cached once read. The
    /// count stops at the four-player value in larger lobbies.
    fn minion_spawn_count(&mut self) -> usize {
        if self.minion_spawns == 0 {
            self.minion_spawns = 3 + (crate::slots::encounter_players(self.players_count()) - 1);
        }
        self.minion_spawns
    }

    // --- weights -----------------------------------------------------

    fn swipe_weight(&mut self, dt: f32) -> f32 {
        if self.state == State::Swipe || self.state != State::Battle {
            return 0.0;
        }
        self.timers.swipe_weight -= dt;
        if self.timers.swipe_weight > 0.0 {
            return 0.0;
        }
        self.timers.swipe_weight = 0.5;
        let (position, forward) = (self.position(), self.forward());
        for i in 0..self.players.len() {
            let Some(p) = self.living(i).copied() else {
                continue;
            };
            if self.distance_squared(p.position) >= 49.0 {
                continue;
            }
            let a = m::signed_angle_to(position, forward, p.position);
            if a.abs() <= std::f32::consts::PI * 4.0 / 5.0 {
                self.set_target(p.avatar.unwrap_or_default());
                const FRONT: f32 = 0.471_238_94;
                if a.abs() <= FRONT {
                    self.swipe = Swipe::Front;
                    return 1.0;
                }
                if a > -FRONT {
                    self.swipe = Swipe::Left;
                    return 1.0;
                }
                if a < FRONT {
                    self.swipe = Swipe::Right;
                    return 1.0;
                }
            }
        }
        0.0
    }

    fn rush_weight(&mut self, world: &mut dyn ParkerWorld) -> f32 {
        let Some(t) = self.target_player().copied() else {
            return 0.0;
        };
        if self.state != State::Battle {
            return 0.0;
        }
        let (position, forward) = (self.position(), self.forward());
        let a = m::signed_angle_to(position, forward, t.position);
        let narrow = std::f32::consts::PI * 3.0 / 200.0;
        if matches!(self.stage, Stage::Intro | Stage::Battle) && self.target_webbed {
            if a.abs() > std::f32::consts::PI / 50.0 {
                return 0.0;
            }
        } else if self.stage == Stage::Final {
            if a.abs() > narrow {
                return 0.0;
            }
        } else {
            if !t.poisoned && !self.target_webbed {
                return 0.0;
            }
            if a.abs() > narrow {
                return 0.0;
            }
        }
        if world.shield_blocks(position, m::sub(t.position, position)) {
            return 0.0;
        }
        1.0
    }

    fn web_weight(&mut self, dt: f32) -> f32 {
        if self.state == State::Web {
            return 0.0;
        }
        self.timers.web_weight -= dt;
        let Some(t) = self.target_player().copied() else {
            return 0.0;
        };
        if self.timers.web_weight > 0.0 || self.state != State::Battle || t.dead || t.entangled {
            return 0.0;
        }
        let a = m::signed_angle_to(self.position(), self.forward(), t.position);
        if a.abs() < std::f32::consts::PI / 20.0 {
            1.0
        } else {
            0.0
        }
    }

    fn spawn_weight(&mut self, dt: f32, world: &mut dyn ParkerWorld) -> f32 {
        if self.state == State::Spawn {
            return 0.0;
        }
        self.timers.spawn_weight -= dt;
        if self.timers.spawn_weight > 0.0 || self.state != State::Battle {
            return 0.0;
        }
        // Native counts String.GetHashCode("spider_baby"), a key no trigger
        // area holds; the rewrite counts the spiderlings it spawns.
        if world.count(SPIDERLING) >= SPAWN_COUNT {
            return 0.0;
        }
        for i in 0..self.players.len() {
            if let Some(p) = self.living(i) {
                if self.distance_squared(p.position) < 64.0 {
                    return 0.0;
                }
            }
        }
        self.timers.spawn_weight = 20.0;
        1.0
    }

    fn minion_weight(&mut self, dt: f32, world: &mut dyn ParkerWorld) -> f32 {
        if self.state != State::Cave || self.previous == State::CallMinions {
            return 0.0;
        }
        if self.players_dead || self.cave_timer < 0.5 {
            return 0.0;
        }
        self.timers.minion_weight -= dt;
        if self.timers.minion_weight <= 0.0 {
            let n = world.count(FOREST_SPIDER) + world.count(POISON_SPIDER);
            self.timers.minion_weight = 0.25;
            if (n as f64) < self.minion_spawn_count() as f64 * 1.5 {
                return 1.0;
            }
        }
        0.0
    }

    /// DefendState.GetWeight: mIsHit is never set, so this only clears it.
    fn defend_weight(&mut self) -> f32 {
        if matches!(
            self.state,
            State::Defend | State::Spawn | State::CallMinions
        ) {
            return 0.0;
        }
        if self.state == State::Battle
            && self.is_hit
            && (self.stage == Stage::Intro || !self.target_webbed)
        {
            return 1.0;
        }
        self.is_hit = false;
        0.0
    }

    // --- stages ------------------------------------------------------

    fn check_behind(&self) -> bool {
        let (position, forward) = (self.position(), self.forward());
        (0..self.players.len())
            .filter_map(|i| self.living(i))
            .any(|p| {
                let mut d = m::sub(p.position, position);
                d[1] = 0.0;
                m::dot(m::normalize(d), forward) < -0.85
            })
    }

    fn transition_to_cave(&mut self, world: &mut dyn ParkerWorld, out: &mut Vec<ParkerCommand>) {
        let mut d = m::sub(self.position(), m::translation(&self.spawn));
        d[1] = 0.0;
        self.rotate_direction = m::normalize(d);
        self.rotation_transition = State::MoveToCave;
        self.change(State::Rotate, world, out);
    }

    fn update_stage(&mut self, dt: f32, world: &mut dyn ParkerWorld, out: &mut Vec<ParkerCommand>) {
        use State as S;
        let busy: &[State] = if self.stage == Stage::Intro {
            &[
                S::Intro,
                S::Rotate,
                S::MoveToCave,
                S::MoveToBattle,
                S::EnterCave,
                S::LeaveCave,
            ]
        } else {
            &[
                S::Rotate,
                S::CallMinions,
                S::MoveToCave,
                S::MoveToBattle,
                S::EnterCave,
                S::LeaveCave,
            ]
        };
        if busy.contains(&self.state) {
            return;
        }
        if self.players_dead {
            if self.state != S::Cave {
                self.transition_to_cave(world, out);
            }
            return;
        }
        if matches!(self.state, S::Battle | S::Swipe | S::Defend) && self.check_behind() {
            self.transition_to_cave(world, out);
            return;
        }
        let burning = self.status.has(Status::Burning)
            && self.status.statuses[magicka_sim::boss_status::status_index(Status::Burning)].dps
                > 250.0;
        type Check = fn(&mut Parker, f32, &mut dyn ParkerWorld) -> Option<State>;
        let minion: Check = |p, dt, w| (p.minion_weight(dt, w) > 0.0).then_some(S::CallMinions);
        let swipe: Check = |p, dt, _| (p.swipe_weight(dt) > 0.0).then_some(S::Swipe);
        let defend: Check = |p, _, _| (p.defend_weight() > 0.0).then_some(S::Defend);
        let web: Check = |p, dt, _| (p.web_weight(dt) > 0.0).then_some(S::Web);
        let rush: Check = |p, _, w| (p.rush_weight(w) > 0.0).then_some(S::Rush);
        let spawn: Check = |p, dt, w| (p.spawn_weight(dt, w) > 0.0).then_some(S::Spawn);
        let order: &[Check] = match self.stage {
            Stage::Intro => &[minion, swipe, defend, web, rush],
            Stage::Battle => {
                if burning {
                    self.transition_to_cave(world, out);
                    return;
                }
                &[minion, rush, swipe, web, spawn, defend]
            }
            Stage::Critical => {
                if burning {
                    self.transition_to_cave(world, out);
                    return;
                }
                &[minion, swipe, spawn, rush, web, defend]
            }
            Stage::Final => {
                if self.is_hit && self.state == S::Battle {
                    self.transition_to_cave(world, out);
                    return;
                }
                &[minion, rush, swipe, web, spawn, defend]
            }
        };
        for check in order {
            if let Some(next) = check(self, dt, world) {
                self.change(next, world, out);
                return;
            }
        }
    }

    // --- states ------------------------------------------------------

    fn update_state(&mut self, dt: f32, world: &mut dyn ParkerWorld, out: &mut Vec<ParkerCommand>) {
        let cue = |name, emitter| ParkerCommand::Cue {
            name,
            emitter,
            characters: false,
        };
        match self.state {
            State::Intro => {
                if self.fading() {
                    return;
                }
                if self.playing(clip::INTRO) {
                    if self.finished() {
                        self.crossfade(clip::IDLE, 0.4, false);
                    }
                } else {
                    self.change(State::Battle, world, out);
                }
            }
            State::Battle => self.update_battle(dt),
            State::Defend => {
                if self.fading() {
                    return;
                }
                if self.finished() {
                    let next = if self.previous == State::Spawn {
                        State::Battle
                    } else {
                        self.previous
                    };
                    self.change(next, world, out);
                } else if !self.played {
                    self.played = true;
                    out.push(cue("parker_defend", Emitter::Mouth));
                }
            }
            State::Stun => {
                if !self.fading() && self.finished() {
                    let next = self.previous;
                    self.change(next, world, out);
                }
            }
            State::Swipe => {
                if self.fading() {
                    return;
                }
                if self.finished() {
                    let next = self.previous;
                    self.change(next, world, out);
                    return;
                }
                let n = self.normalized();
                if self.swipe == Swipe::Front {
                    if !self.played && n >= BITE_SOUND_TIME {
                        out.push(cue("parker_bite", Emitter::Abdomen));
                        self.played = true;
                    }
                } else if (ATTACK_LEFT_RIGHT_TIME[0]..=ATTACK_LEFT_RIGHT_TIME[1]).contains(&n) {
                    let (a, b) = if self.swipe == Swipe::Left {
                        (2, 1)
                    } else {
                        (6, 5)
                    };
                    let from = m::translation(&self.feet[a]);
                    let to = m::translation(&self.feet[b]);
                    let half = m::scale(m::sub(to, from), 0.5);
                    out.push(ParkerCommand::SwipeSweep {
                        center: m::add(from, half),
                        radius: m::length(half) * 1.25,
                    });
                }
            }
            State::Rush => {
                let mut b = self.rush_target;
                let mut a = m::translation(&self.mouth);
                a[1] = 0.0;
                b[1] = 0.0;
                self.movement_speed_mod = if self.fading() {
                    0.0
                } else if self.target_player().is_some_and(|p| p.poisoned) {
                    1.5
                } else {
                    1.0
                };
                if m::approximately(a, b, 1.0) {
                    self.set_movement([0.0; 3]);
                    self.change(State::Bite, world, out);
                    return;
                }
                let d = m::sub(b, a);
                self.state_speed = 0.5 + m::length_squared(d).min(1.0) * 0.5;
                self.set_movement(m::normalize(d));
            }
            State::Backup => {
                let mut b = self.backup_target;
                let mut a = self.position();
                a[1] = 0.0;
                b[1] = 0.0;
                if m::approximately(a, b, 0.01) {
                    self.set_movement([0.0; 3]);
                    self.change(State::Battle, world, out);
                } else {
                    let d = m::sub(b, a);
                    self.state_speed = 0.5 + m::length_squared(d).min(1.0) * 0.5;
                    self.set_movement(m::normalize(d));
                }
            }
            State::Bite => {
                if self.fading() {
                    return;
                }
                if self.finished() {
                    // Rush's weight needs the Battle state: always Backup.
                    if self.rush_weight(world) > 0.0 {
                        self.crossfade(clip::BITE, 0.25, false);
                    } else {
                        self.change(State::Backup, world, out);
                    }
                } else if !self.played && self.normalized() >= BITE_SOUND_TIME {
                    out.push(cue("parker_bite", Emitter::Abdomen));
                    self.played = true;
                }
            }
            State::Web => self.update_web(dt, world, out),
            State::Spawn => self.update_spawn(dt, world, out),
            State::Rotate => {
                self.set_movement([0.0; 3]);
                if !self.fading() {
                    let fwd = self.forward();
                    let a = m::signed_angle(fwd, self.rotate_direction);
                    if a.abs() < 0.05 {
                        let next = self.rotation_transition;
                        self.change(next, world, out);
                    }
                    // The turn still applies this frame after the change.
                    let rate = (60.0 * self.stage_speed * self.state_speed).to_radians();
                    let step = if a < 0.0 { -rate } else { rate };
                    let t = self.position();
                    self.transform = m::yaw_matrix(&self.transform, step * dt);
                    m::set_translation(&mut self.transform, t);
                }
            }
            State::MoveToCave => {
                let mut a = self.position();
                let mut b = m::translation(&self.spawn);
                a[1] = 0.0;
                b[1] = 0.0;
                self.movement_speed_mod = if self.fading() { 0.0 } else { 1.0 };
                if !m::approximately(a, b, 0.01) {
                    let d = m::sub(b, a);
                    self.state_speed = m::length(d).clamp(0.25, 1.0);
                    self.set_movement(m::normalize(d));
                } else {
                    self.rotate_direction = m::forward(&self.spawn);
                    self.rotation_transition = State::EnterCave;
                    self.change(State::Rotate, world, out);
                }
            }
            State::LeaveCave => {
                if !self.fading() {
                    if !self.played {
                        out.push(cue("parker_exit_cave", Emitter::Abdomen));
                        self.played = true;
                    } else if self.finished() {
                        self.change(State::MoveToBattle, world, out);
                    }
                }
            }
            State::MoveToBattle => {
                if self.fading() {
                    self.state_speed = 1.0;
                    self.set_movement([0.0; 3]);
                } else if self.playing(clip::WALK_FORWARD) {
                    let mut a = self.position();
                    let mut b = m::translation(&self.battle);
                    a[1] = 0.0;
                    b[1] = 0.0;
                    self.movement_speed_mod = 1.0;
                    if !m::approximately(a, b, 0.01) {
                        let d = m::sub(b, a);
                        self.state_speed = m::length(d).clamp(0.25, 1.0);
                        self.set_movement(m::normalize(d));
                    } else {
                        self.crossfade(clip::IDLE, 0.4, false);
                    }
                } else {
                    self.change(State::Battle, world, out);
                }
            }
            State::EnterCave => {
                if !self.fading() {
                    if !self.played {
                        out.push(cue("parker_enter_cave", Emitter::Abdomen));
                        self.played = true;
                    } else if self.finished() {
                        self.change(State::Cave, world, out);
                    }
                }
            }
            State::Cave => {
                if !self.players_dead {
                    self.cave_timer -= dt;
                }
                self.status_damage_direct(dt * -425.0 * self.cave_modifier, E::LIFE);
                if self.cave_timer <= 0.0 {
                    if self.minion_weight(dt, world) > 0.0 {
                        self.change(State::CallMinions, world, out);
                    } else {
                        self.change(State::LeaveCave, world, out);
                    }
                }
            }
            State::CallMinions => self.update_minions(dt, world, out),
            State::Dead => {
                if !self.fading() {
                    if !self.played {
                        out.push(cue("parker_death", Emitter::Abdomen));
                        self.played = true;
                    }
                    if self.finished() {
                        self.dead = true;
                    }
                }
            }
        }
    }

    /// The protected Damage(float, Elements) on the current resistances.
    fn status_damage_direct(&mut self, amount: f32, element: E) {
        self.status.status_damage(amount, element);
    }

    fn update_battle(&mut self, dt: f32) {
        self.state_speed = if self
            .target_player()
            .is_some_and(|p| self.target_webbed || p.poisoned)
        {
            1.5
        } else {
            1.0
        };
        let translation = m::translation(&self.pivot);
        let forward = m::forward(&self.pivot);
        if self.target_dead() {
            if let Some(a) = self.random_target() {
                self.set_target(a);
            }
        }
        let mut d = m::sub(self.player_target_position, translation);
        d[1] = 0.0;
        let d = m::constrain_vector(m::normalize(d), [0.0, 0.0, 1.0], -0.7, 0.7);
        let angle = m::signed_angle(forward, d);
        self.pivot_target_direction = d;
        if angle.abs() > 0.005 && !self.fading() {
            let max = self.movement_speed.to_radians();
            let step = if angle < 0.0 {
                angle.max(-max)
            } else {
                angle.min(max)
            };
            let step = 2.0 * step * self.state_speed * self.stage_speed * self.cold_speed();
            let t = m::translation(&self.pivot);
            self.pivot = m::yaw_matrix(&self.pivot, step * dt);
            m::set_translation(&mut self.pivot, t);
        }
        self.transform = self.battle_state_transform();
        self.animation_speed = (0.25 + angle.abs() * 5.0).clamp(0.0, 1.0);
        if angle > 0.01 {
            if self.battle_animation != clip::WALK_LEFT {
                self.crossfade(clip::WALK_LEFT, 0.2, true);
                self.battle_animation = clip::WALK_LEFT;
            }
        } else if angle < -0.01 {
            if self.battle_animation != clip::WALK_RIGHT {
                self.crossfade(clip::WALK_RIGHT, 0.2, true);
                self.battle_animation = clip::WALK_RIGHT;
            }
        } else {
            self.animation_speed = 1.0;
            if self.battle_animation != clip::IDLE {
                self.crossfade(clip::IDLE, 0.2, true);
                self.battle_animation = clip::IDLE;
            }
        }
    }

    fn update_web(&mut self, dt: f32, world: &mut dyn ParkerWorld, out: &mut Vec<ParkerCommand>) {
        if self.fading() {
            return;
        }
        let n = self.normalized();
        self.web_timer -= dt;
        if self.finished() {
            // Rush's weight needs the Battle state.
            if self.rush_weight(world) > 0.0 {
                self.change(State::Rush, world, out);
            } else {
                self.change(State::Battle, world, out);
            }
            return;
        }
        if !(self.web_timer <= 0.0
            && self.web_count < WEB_SEGMENTS
            && (WEBBING_TIME[0]..=WEBBING_TIME[1]).contains(&n))
        {
            return;
        }
        let mouth = m::translation(&self.mouth);
        let distance = m::length(m::sub(self.player_target_position, mouth));
        let mut d = m::backward(&self.mouth);
        d[1] = 0.0;
        let d = m::constrain_vector(m::normalize(d), self.forward(), -0.65, 0.65);
        self.web_count += 1;
        out.push(ParkerCommand::Web {
            index: self.web_count,
            position: mouth,
            direction: d,
            velocity: distance * 3.0,
            mouth_owner: self.web_count != WEB_SEGMENTS,
        });
        out.push(ParkerCommand::Cue {
            name: "chr_spider_web",
            emitter: Emitter::Mouth,
            characters: true,
        });
        self.web_timer = 0.1 * self.web_spray_time;
    }

    fn update_spawn(&mut self, dt: f32, world: &mut dyn ParkerWorld, out: &mut Vec<ParkerCommand>) {
        if self.fading() {
            return;
        }
        if self.finished() {
            self.change(State::Battle, world, out);
            return;
        }
        if !self.played {
            out.push(ParkerCommand::Cue {
                name: "parker_spawn",
                emitter: Emitter::Abdomen,
                characters: false,
            });
            self.played = true;
        }
        let n = self.normalized();
        self.spawn_timer -= dt;
        // mTimer is never reloaded: one spiderling per frame in the window.
        if (SPAWN_TIME[0]..=SPAWN_TIME[1]).contains(&n)
            && self.spawn_timer <= 0.0
            && self.spawn_count < SPAWN_COUNT
        {
            let mut p = self.abdomen;
            p[2] -= 4.0;
            let r = self.random() * 2.0;
            let a = self.random() * std::f32::consts::TAU;
            p[0] += r * a.cos();
            p[2] += r * a.sin();
            out.push(ParkerCommand::Spawn {
                template: SPIDERLING,
                position: p,
                direction: self.forward(),
                attack: true,
                dead_timer: 2.0,
            });
            self.spawn_count += 1;
        }
    }

    fn update_minions(
        &mut self,
        dt: f32,
        world: &mut dyn ParkerWorld,
        out: &mut Vec<ParkerCommand>,
    ) {
        if self.fading() {
            return;
        }
        if self.finished() {
            self.change(State::LeaveCave, world, out);
            return;
        }
        if !self.played {
            out.push(ParkerCommand::Cue {
                name: "parker_call_minion",
                emitter: Emitter::Abdomen,
                characters: false,
            });
            self.played = true;
        }
        let n = self.normalized();
        self.minion_timer -= dt;
        if (SPAWN_TIME[0]..=SPAWN_TIME[1]).contains(&n)
            && self.minion_timer <= 0.0
            && self.minion_count < self.minion_spawns
        {
            let mut p = match self.random_target() {
                Some(a) => self
                    .players
                    .iter()
                    .find(|p| p.avatar == Some(a))
                    .map_or(self.position(), |p| p.position),
                None => {
                    let i = self.random_below(MINION_LOCATORS.len());
                    world
                        .locator(MINION_LOCATORS[i])
                        .map_or([0.0; 3], |l| m::translation(&l))
                }
            };
            let angle = self.random() * std::f32::consts::TAU;
            let r = self.random().sqrt();
            let (template, r) = if self.minion_count < 2 {
                (POISON_SPIDER, 4.0 + r * 3.0)
            } else {
                (FOREST_SPIDER, 2.0 + r * 2.0)
            };
            p[0] += r * angle.cos();
            p[2] += r * angle.sin();
            let p = world.nav_nearest(p);
            out.push(ParkerCommand::Spawn {
                template,
                position: p,
                direction: self.forward(),
                attack: false,
                dead_timer: 5.0,
            });
            self.minion_timer += self.minion_spawn_time;
            self.minion_count += 1;
        }
    }

    /// UpdateBoss.
    pub fn update(
        &mut self,
        dt: f32,
        players: Vec<Player>,
        world: &mut dyn ParkerWorld,
    ) -> Vec<ParkerCommand> {
        let mut out = std::mem::take(&mut self.pending);
        self.players = players;
        if self.status.hit_points <= 0.0 && self.state != State::Dead {
            self.change(State::Dead, world, &mut out);
        }
        let mut events = Vec::new();
        self.status.update_damage(dt, self.dead, &mut events);
        out.extend(events.into_iter().map(ParkerCommand::Boss));
        for (amount, element) in self.status.update_statuses(dt, self.dead) {
            // Ticks reach the abdomen zone (part 0).
            self.status_damage(0, amount, element);
        }
        if self.hit_timer > 0.0 {
            self.is_hit = false;
        }
        self.hit_timer -= dt;
        self.hit_list.update(dt);
        self.players_dead = !(0..self.players.len()).any(|i| self.living(i).is_some());
        if let Some(t) = self.target_player().copied() {
            self.player_target_position = t.position;
            self.target_webbed = t.entangled;
        }
        if self.state != State::Dead {
            let ratio = self.status.hit_points / MAX_HIT_POINTS;
            let stage = if ratio > 0.95 {
                Some(Stage::Intro)
            } else if ratio >= 0.5 {
                Some(Stage::Battle)
            } else if ratio > 0.15 {
                Some(Stage::Critical)
            } else if ratio > 0.0 {
                Some(Stage::Final)
            } else {
                None
            };
            if let Some(s) = stage.filter(|s| *s != self.stage) {
                self.stage = s;
                self.enter_stage();
            }
            self.update_stage(dt, world, &mut out);
        }
        self.update_state(dt, world, &mut out);
        let cold = if self.status.has(Status::Cold) {
            self.cold_speed()
        } else {
            1.0
        };
        let mut p = self.position();
        let k = self.movement_speed
            * self.stage_speed
            * self.movement_speed_mod
            * dt
            * self.state_speed
            * cold;
        p[0] += self.movement[0] * k;
        p[2] += self.movement[2] * k;
        if let Some(hit) = world.ground([p[0], p[1] + 1.0, p[2]], [0.0, -3.0, 0.0]) {
            p = hit;
        }
        p[1] += 0.2;
        m::set_translation(&mut self.transform, p);
        self.controller.speed = if self.state == State::Dead {
            1.0
        } else {
            self.state_speed * self.stage_speed * self.animation_speed * cold
        };
        self.pose_update(dt);
        out
    }

    fn bone_world(&self, skinned: &[Mat], bone: usize) -> Mat {
        skinned::mul(&self.binds[&bone], &skinned[bone])
    }

    /// Vector3.Transform(bindPose.Translation, skinned[bone]).
    fn point_of(&self, skinned: &[Mat], bone: usize, bind: &Mat) -> V3 {
        m::transform_point(m::translation(bind), &skinned[bone])
    }

    /// AnimationController.Update with the zone placement and footsteps.
    fn pose_update(&mut self, dt: f32) {
        self.controller.signals.clear();
        self.controller.update(dt);
        if self.controller.signals.iter().any(|s| {
            matches!(
                s,
                ControllerSignal::Looped | ControllerSignal::CrossfadeFinished
            )
        }) {
            self.footstep = -1;
        }
        let local = self.skeleton.local_poses(self.controller.pose_sample());
        let skinned = self.skeleton.skinned_from_local(&local, &self.transform);
        let point = |b: usize| self.point_of(&skinned, b, &self.binds[&b]);
        let up = [0.0, 1.0, 0.0];
        let mut limbs = Vec::with_capacity(16);
        let mut knee_body = Vec::with_capacity(8);
        let mut feet = [skinned::IDENTITY; 8];
        for (j, slot) in feet.iter_mut().enumerate() {
            let knee = point(self.knee_bones[j]);
            let foot = point(self.foot_bones[j]);
            let body = point(self.body_bones[j]);
            let mut f = m::create_world([0.0; 3], m::normalize(m::sub(knee, foot)), up);
            m::set_translation(&mut f, foot);
            *slot = f;
            limbs.push((knee, foot, 0.55));
            knee_body.push((body, knee, 0.8));
        }
        limbs.extend(knee_body);
        let abdomen = point(self.abdomen_bone);
        let back = match &self.back_bind {
            Some(bind) => self.point_of(&skinned, self.back_abdomen_bone, bind),
            None => point(self.back_abdomen_bone),
        };
        self.feet = feet;
        self.limbs = limbs;
        let mut a = m::lerp(abdomen, back, -1.0);
        a[1] = abdomen[1];
        self.abdomen = a;
        self.mouth = self.bone_world(&skinned, self.mouth_bone);
        self.pose = ModelPose {
            parent: self.transform,
            local,
        };
    }

    /// The footstep cue check after the pose update: walking clips (or a
    /// crossfade toward one) at the FOOTSTEP_TIMES fractions.
    pub fn footsteps(&mut self) -> usize {
        let walking = |name: Option<&str>| {
            name.is_some_and(|n| {
                [
                    clip::WALK_FORWARD,
                    clip::WALK_BACKWARD,
                    clip::WALK_LEFT,
                    clip::WALK_RIGHT,
                ]
                .iter()
                .any(|&c| clip::NAMES[c] == n)
            })
        };
        let sample = self.controller.pose_sample();
        let current = self.controller.clip_name();
        let fading = sample.and_then(|s| s.crossfade);
        if !walking(current) && !walking(fading) {
            return 0;
        }
        let n = self.normalized();
        let mut cues = 0;
        for i in (0..FOOTSTEP_TIMES.len()).rev() {
            if n >= FOOTSTEP_TIMES[i] && self.footstep < i as i32 {
                self.footstep = i as i32;
                cues += 1;
            }
        }
        cues
    }

    /// OnCollision of the abdomen and limb zones with a damageable (the
    /// collision itself is always kept). `character` selects the state
    /// gated character branch; `mass` is the target body's mass.
    pub fn body_contact(&mut self, handle: u16, character: bool, mass: f32) -> Contact {
        if self.dead || self.hit_list.contains(handle) {
            return Contact::None;
        }
        let damage = |attack, amount| Damage {
            attack,
            element: E::EARTH.0,
            amount,
            magnitude: 1.0,
        };
        let out = if character {
            if matches!(
                self.state,
                State::MoveToCave | State::Backup | State::Rotate | State::EnterCave
            ) {
                Contact::Damage {
                    damages: vec![
                        damage(attack::DAMAGE, 100.0),
                        damage(attack::KNOCKBACK, mass),
                    ],
                    mouth: false,
                    position: self.abdomen,
                }
            } else {
                Contact::None
            }
        } else {
            Contact::Damage {
                damages: vec![damage(attack::DAMAGE, 100.0)],
                mouth: false,
                position: self.abdomen,
            }
        };
        self.hit_list.add_for(handle, 1.0);
        out
    }

    /// OnMouthCollision with a damageable. `blocker` marks a shield or
    /// barrier.
    pub fn mouth_contact(&mut self, handle: u16, blocker: bool, mass: f32) -> Contact {
        if self.dead || self.fading() {
            return Contact::None;
        }
        let position = self.position();
        if self.state == State::Bite {
            let n = self.normalized();
            if n > 0.333 && blocker {
                self.controller.time = self.controller.clip_duration() - 0.3;
                return Contact::KillBlocker;
            }
            if (BITE_TIME[0]..=BITE_TIME[1]).contains(&n) && !self.hit_list.contains(handle) {
                self.hit_list.add(handle);
                return Contact::Damage {
                    damages: vec![
                        Damage {
                            attack: attack::DAMAGE,
                            element: E::EARTH.0,
                            amount: 2500.0,
                            magnitude: 1.0,
                        },
                        Damage {
                            attack: attack::STATUS,
                            element: E::POISON.0,
                            amount: 100.0,
                            magnitude: 1.0,
                        },
                    ],
                    mouth: true,
                    position,
                };
            }
        } else if self.state == State::Swipe && self.swipe == Swipe::Front {
            if blocker {
                return Contact::KillBlocker;
            }
            if !self.hit_list.contains(handle) {
                self.hit_list.add(handle);
                return Contact::Damage {
                    damages: vec![
                        Damage {
                            attack: attack::KNOCKBACK,
                            element: E::EARTH.0,
                            amount: mass,
                            magnitude: 3.0,
                        },
                        Damage {
                            attack: attack::STATUS,
                            element: E::POISON.0,
                            amount: 100.0,
                            magnitude: 1.0,
                        },
                    ],
                    mouth: true,
                    position,
                };
            }
        }
        Contact::None
    }

    /// SwipeState's first sweep hit plays parker_swipe_sides once.
    pub fn swipe_sound(&mut self) -> bool {
        !std::mem::replace(&mut self.played, true)
    }

    /// The mouth zone collides physically except while biting.
    pub fn mouth_solid(&self) -> bool {
        self.state != State::Bite
    }

    /// SwipeState's sweep damage for one entity (mass as knockback amount).
    pub fn swipe_damages(mass: f32) -> [Damage; 2] {
        [
            Damage {
                attack: attack::KNOCKBACK,
                element: E::EARTH.0,
                amount: mass,
                magnitude: 3.0,
            },
            Damage {
                attack: attack::DAMAGE,
                element: E::EARTH.0,
                amount: 50.0,
                magnitude: 1.0,
            },
        ]
    }

    /// Whether this frame's DamageResult flags count as damage for flashes.
    pub fn damaged(result: u16) -> bool {
        result & (R::HIT | R::DAMAGED) != 0
    }
}
