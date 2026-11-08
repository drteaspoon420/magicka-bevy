//! `Magicka.GameLogic.Entities.Bosses.Cthulhu` (Tsar_Rlyeh tr_s9BF) with
//! its `Tentacle`s and `CthulhuMist`, as deterministic state machines
//! (research/boss-cthulhu.md). Cthulhu surfaces at one of eight spawn
//! points, casts from fourteen states chosen by five health stages, and
//! keeps up to four tentacles that swipe, grab, squeeze and crush. Scene
//! queries and every world operation go through [`CthulhuWorld`].
use crate::boss_rig::BossRig;
use crate::xna_math::{self as m, V3};
use magicka_content::skinned::{self, Mat, Resolved};
use magicka_sim::boss_status::{BossEvent, BossRules, BossStatus};
use magicka_sim::character_impact::result as R;
use magicka_sim::damage::{attack, Damage};
use magicka_sim::spells::Elements as E;
use magicka_sim::spray_pulse::HitList;
use magicka_sim::status::Status;

pub const MESH_MODEL: &str = "Models/Bosses/cthulhu/cthulhu_mesh";
pub const ANIMATION_MODEL: &str = "Models/Bosses/cthulhu/cthulhu_animations";
pub const TENTACLE_MESH: &str = "Models/Bosses/cthulhu/tentacle_mesh";
pub const TENTACLE_ANIMATION: &str = "Models/Bosses/cthulhu/tentacle_animations";
pub const MAX_HIT_POINTS: f32 = 100000.0;
pub const TENTACLE_HIT_POINTS: f32 = 800.0;
/// BossSpawnPoints NORTH..NORTHEAST.
pub const SPAWN_LOCATORS: [&str; 8] = [
    "boss_spawn0",
    "boss_spawn6",
    "boss_spawn4",
    "boss_spawn5",
    "boss_spawn2",
    "boss_spawn1",
    "boss_spawn3",
    "boss_spawn7",
];
pub const DEEP_ONE_LOCATORS: [&str; 8] = [
    "spawn_deepone0",
    "spawn_deepone1",
    "spawn_deepone2",
    "spawn_deepone3",
    "spawn_deepone4",
    "spawn_deepone5",
    "spawn_deepone6",
    "spawn_deepone7",
];
pub const MIST_LOCATOR: &str = "boss_spawn_middle";
pub const STARSPAWN: &str = "starspawn";
pub const DEEP_ONE: &str = "deep_one";
pub const CULTIST: &str = "cultist";
pub const BLOOD: &str = "gore_splash_black";
/// The body zone: BossSpellCasterZone capsule radius 1.2, length 4.
pub const ZONE_RADIUS: f32 = 1.2;
pub const ZONE_LENGTH: f32 = 4.0;
/// The mist: a BossDamageZone sphere of radius 2.
pub const MIST_RADIUS: f32 = 2.0;
/// The tentacle capsules' radius (mRadius).
pub const TENTACLE_RADIUS: f32 = 0.8;
/// mArmZone capsules (radius, length): upper arms, then forearms.
pub const ARM_CAPSULES: [(f32, f32); 2] = [(0.5, 1.6), (0.4, 4.7)];
const COLD_COLOR: V3 = [1.0, 1.6, 2.0];
const ADDITIONAL: u16 = crate::audio::banks::ADDITIONAL;
const SPELLS: u16 = crate::audio::banks::SPELLS;

/// Cthulhu.Animations by index.
pub mod clip {
    pub const BOLT_START: &str = "cast_bolt_begin";
    pub const BOLT_MID: &str = "cast_bolt_mid";
    pub const BOLT_END: &str = "cast_bolt_end";
    pub const SUBMERGE: &str = "submerge";
    pub const MISTS: &str = "mists";
    pub const TIMEWARP: &str = "timewarp";
    pub const MESMERIZE: &str = "mesmerize";
    pub const EMERGE: &str = "emerge";
    pub const IDLE: &str = "idle";
    pub const DEVOUR: &str = "devour";
    pub const DEVOUR_HIT: &str = "devour_hit";
    pub const LIGHTNING: &str = "cast_lightning";
    pub const CALL: &str = "call_of_cthulhu";
    pub const DIE: &str = "die";
    pub const RAGE: &str = "rage";
    pub const ALL: [&str; 16] = [
        BOLT_START, BOLT_MID, BOLT_END, SUBMERGE, MISTS, TIMEWARP, MESMERIZE, "madness", EMERGE,
        IDLE, DEVOUR, DEVOUR_HIT, LIGHTNING, CALL, DIE, RAGE,
    ];
}

/// Tentacle.Animations.
pub mod tentacle_clip {
    pub const IDLE: &str = "idle";
    pub const SQUEEZE: &str = "squeeze";
    pub const CRUSH: &str = "crush";
    pub const EMERGE: &str = "emerge";
    pub const SUBMERGE: &str = "submerge0";
    pub const DIE: &str = "die";
    pub const GRIP: &str = "grip";
    pub const GRIP_SUCCESS: &str = "grip_success";
    pub const LET_GO: &str = "let_go";
    pub const ALL: [&str; 9] = [
        IDLE,
        SQUEEZE,
        CRUSH,
        EMERGE,
        SUBMERGE,
        DIE,
        GRIP,
        GRIP_SUCCESS,
        LET_GO,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Emerge,
    Submerge,
    Devour,
    DevourHit,
    Lightning,
    Mist,
    Call,
    LesserCall,
    Timewarp,
    Hypnotize,
    /// States.NewStage: mStates[11] is the RageState.
    Rage,
    OtherworldlyBolt,
    Death,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Intro,
    Battle,
    LateBattle,
    Critical,
    Final,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TentacleState {
    None,
    Idle,
    Emerge,
    Submerge,
    SwipeGrab,
    GrabSuccess,
    Squeeze,
    Release,
    Crush,
    Aim,
    Die,
}

/// What an entity returned by [`CthulhuWorld::entities`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Avatar,
    Npc,
    Shield,
    Barrier,
    Mine,
    Missile,
    /// Any other damageable (props, boss zones, tentacles).
    Other,
}

/// An entity near a query point (EntityManager.GetEntities).
#[derive(Clone, Copy, Debug)]
pub struct Near {
    pub handle: u16,
    pub kind: Kind,
    pub position: V3,
    pub radius: f32,
    pub mass: f32,
    pub ethereal: bool,
    pub invisible: bool,
    pub dead: bool,
    pub drowning: bool,
    pub burning: bool,
    /// (Faction & FRIENDLY) != 0.
    pub friendly: bool,
    pub starspawn: bool,
    /// Character.IsResistantAgainst(Lightning).
    pub lightning_resistance: f32,
    /// Character.IsGripped with a gripper.
    pub gripped: bool,
    /// The character's capsule length (DevourHit's upward offset).
    pub length: f32,
}

/// A player slot (Game.Instance.Players).
#[derive(Clone, Copy, Debug, Default)]
pub struct Player {
    pub playing: bool,
    pub avatar: Option<u16>,
    pub dead: bool,
    pub position: V3,
    pub ethereal: bool,
    pub hypnotized: bool,
}

/// Scene queries and operations used by Cthulhu, its tentacles and mist.
pub trait CthulhuWorld {
    fn random(&mut self) -> f32;
    fn locator(&mut self, name: &str) -> Option<Mat>;
    fn nav_nearest(&mut self, point: V3) -> V3;
    /// Liquids[0].SegmentIntersect: the hit height.
    fn water_height(&mut self, origin: V3, delta: V3) -> Option<f32>;
    /// GetTriggerArea("any").GetCount(type).
    fn count(&mut self, template: &str) -> usize;
    fn time_warp_active(&mut self) -> bool;
    fn entities(
        &mut self,
        center: V3,
        radius: f32,
        ignore_protected: bool,
        ignore_y: bool,
    ) -> Vec<Near>;
    /// One entity by handle (dead ones included).
    fn entity(&mut self, handle: u16) -> Option<Near>;
    /// IDamageable.SegmentIntersect.
    fn segment_hit(&mut self, handle: u16, origin: V3, delta: V3, radius: f32) -> Option<V3>;
    fn shield_blocks(&mut self, origin: V3, delta: V3) -> bool;
    fn shield_nearest(&mut self, handle: u16, point: V3) -> V3;

    fn cue(&mut self, bank: u16, name: &'static str, at: V3) -> u32;
    fn stop_cue(&mut self, key: u32);
    fn effect(&mut self, name: &'static str, transform: Mat);
    /// A persistent effect started, moved or (None) stopped under `key`.
    fn track(&mut self, key: &str, effect: &'static str, transform: Option<Mat>);
    /// Helper.CircleDamage owned by and ignoring `owner`.
    fn circle_damage(&mut self, owner: u16, position: V3, radius: f32, damage: Damage);
    /// Liquid.Freeze(origin, direction, 2π, 2, damage).
    fn freeze(&mut self, origin: V3, direction: V3, damage: Damage);
    fn time_warp(&mut self, caster: u16);
    /// A NonPlayerCharacter ordered to attack, spawned in RessurectionState.
    fn spawn_npc(
        &mut self,
        template: &'static str,
        position: V3,
        orientation: Mat,
        desired: V3,
        animation: &str,
    );
    /// LightningBolt.Cast at a target or along a direction.
    #[allow(clippy::too_many_arguments)]
    fn lightning(
        &mut self,
        caster: u16,
        from: V3,
        target: Option<u16>,
        direction: V3,
        range: f32,
        damage: Damage,
        hits: &mut HitList,
    );
    fn hypnotize(&mut self, avatar: u16, direction: V3);
    /// Charm (7.5 s) and Confuse the avatar (CthulhuMist.CharmAndConfuse).
    fn charm_and_confuse(&mut self, avatar: u16, mist: u16);
    fn damage(&mut self, target: u16, damage: Damage, attacker: Option<u16>, position: V3);
    fn kill(&mut self, target: u16);
    fn overkill(&mut self, target: u16);
    fn set_velocity(&mut self, target: u16, velocity: V3);
    fn move_body(&mut self, target: u16, position: V3);
    fn additional_force(&mut self, target: u16, force: V3);
    fn missile_impulse(&mut self, target: u16, force: V3, dt: f32);
    fn release_grip(&mut self, target: u16);
    fn terminate(&mut self, target: u16);
    fn scale_model(&mut self, target: u16, scale: f32);
    fn trigger(&mut self, name: &str);
    fn camera_shake(&mut self, at: V3, magnitude: f32, time: f32);
    fn deflect(&mut self, at: V3, strength: f32, dt: f32);
    /// Cthulhu.SpawnCultistMissile: an OtherworldlyBolt; returns its id.
    fn spawn_bolt(&mut self, caster: u16, position: V3, direction: V3, speed: f32) -> usize;
    fn bolt_hunt(&mut self, bolt: usize);
    /// KillBolts: every bolt destroyed in silence.
    fn kill_bolts(&mut self);
    /// BossStatusEffected presentation for Cthulhu (`None`) or a tentacle.
    fn boss_event(&mut self, tentacle: Option<usize>, event: BossEvent, notifier: V3);
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

fn xorshift(rng: &mut u64) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    (*rng >> 40) as f32 / (1u64 << 24) as f32
}

fn flat(v: V3) -> V3 {
    [v[0], 0.0, v[2]]
}
fn right(t: &Mat) -> V3 {
    [t[0], t[1], t[2]]
}
fn translation_matrix(p: V3) -> Mat {
    let mut t = skinned::IDENTITY;
    m::set_translation(&mut t, p);
    t
}
fn dmg(attack: u16, element: E, amount: f32, magnitude: f32) -> Damage {
    Damage {
        attack,
        element: element.0,
        amount,
        magnitude,
    }
}

// --- the tentacle -------------------------------------------------------

/// TentacleStatusEffect: Cold, Wet, Frozen, Steamed and Bleeding update;
/// Burning, Poisoned and the rest only hold their magnitude.
#[derive(Clone, Copy, Debug, Default)]
struct TentacleStatus {
    kind: Status,
    magnitude: f32,
    dps: f32,
    damage_rest: f32,
    wet_timer: f32,
}

impl TentacleStatus {
    fn new(kind: Status, dps: f32, magnitude: f32, wet_timer: f32) -> Self {
        let magnitude = if kind == Status::Steamed {
            1.0
        } else {
            magnitude
        };
        Self {
            kind,
            magnitude,
            dps,
            damage_rest: 0.0,
            wet_timer,
        }
    }
    fn dead(&self) -> bool {
        self.magnitude <= 0.0
    }
    fn slowdown(&self) -> f32 {
        match self.kind {
            Status::Cold => 0.001f32.powf(self.magnitude) * 0.667 + 0.333,
            Status::Poisoned => 0.5,
            _ => 1.0,
        }
    }
    /// operator+.
    fn add(self, b: Self) -> Self {
        if self.dead() || self.kind == Status::None {
            return b;
        }
        if b.dead() || b.kind == Status::None {
            return self;
        }
        let magnitude = if self.kind == Status::Healing {
            self.magnitude.max(b.magnitude)
        } else {
            self.magnitude + b.magnitude
        };
        let inv = 1.0 / magnitude;
        Self {
            kind: self.kind,
            magnitude,
            dps: self.magnitude * inv * self.dps + b.magnitude * inv * b.dps,
            damage_rest: self.damage_rest + b.damage_rest,
            wet_timer: self.wet_timer.max(b.wet_timer),
        }
    }
    /// Update: the earth damage it deals this frame.
    fn update(&mut self, dt: f32) -> i32 {
        if self.magnitude <= 0.0 {
            return 0;
        }
        match self.kind {
            Status::Cold => {
                self.magnitude -= dt * 0.1;
                self.magnitude = self.magnitude.min(3.0);
            }
            Status::Wet => {
                if self.wet_timer >= 0.0 {
                    self.wet_timer -= dt;
                }
                self.magnitude = 1.0;
            }
            Status::Frozen => {
                self.magnitude = self.magnitude.min(3.0);
                self.magnitude -= dt * 0.25;
            }
            Status::Steamed => {
                self.magnitude = self.magnitude.min(1.0);
                self.magnitude -= dt;
            }
            Status::Bleeding => self.damage_rest += dt * self.dps,
            _ => {}
        }
        let whole = self.damage_rest as i32;
        self.damage_rest -= whole as f32;
        whole
    }
}

fn index_of(kind: Status) -> usize {
    magicka_sim::boss_status::status_index(kind)
}

#[derive(Clone, Debug, Default)]
struct TentacleTimers {
    idle: f32,
    // EmergeState.
    emerge_effect_done: bool,
    unfreeze_done: bool,
    bubbles_done: bool,
    bubble_timer: f32,
    // SubmergeState / DieState.
    regeneration: f32,
    submerge_effect_done: bool,
    die_effect_done: bool,
    // SwipeGrabState / CrushState.
    swipe_done: bool,
    swipe_deflected: bool,
    crush_done: bool,
    crush_deflected: bool,
    squeeze: f32,
    aim_radius: f32,
}

#[derive(Clone, Debug)]
pub struct Tentacle {
    pub id: u8,
    pub handle: u16,
    pub rig: BossRig,
    pub state: TentacleState,
    pub previous: TentacleState,
    pub transform: Mat,
    pub grabbed: Option<u16>,
    hit_list: HitList,
    grab_bind: (usize, Mat),
    spine: [(usize, Mat); 11],
    pub grab_transform: Mat,
    pub top_joint: Mat,
    crush_target: Option<u16>,
    original_forward: V3,
    search_for_target: bool,
    seconds_out_of_range: i32,
    check_timer: f32,
    cthulhu_killed: bool,
    pub hit_flash: f32,
    pub hit_points: f32,
    pub wetness: f32,
    hp_on_last_emerge: f32,
    time_until_submerge: f32,
    time_since_last_emerge: f32,
    /// mTentacleLength: spine joint 0 to joint 10 in the bind pose.
    pub length: f32,
    statuses: [TentacleStatus; 9],
    current: u16,
    dry_timer: f32,
    resistances: [magicka_sim::resistance::Resistance; 11],
    locked_number: bool,
    time_since_last_damage: f32,
    pub dead: bool,
    /// In the active list (spawned by Cthulhu).
    pub active: bool,
    /// The ten spine capsules (start, end) this frame.
    pub capsules: Vec<(V3, V3)>,
    t: TentacleTimers,
}

impl Tentacle {
    fn new(id: u8, animation: &Resolved) -> Result<Self, String> {
        let rig = BossRig::new(animation, &tentacle_clip::ALL)?;
        let grab_bind = rig.bind("joint24")?;
        let names = [
            "joint12", "joint2", "joint3", "joint4", "joint5", "joint6", "joint7", "joint8",
            "joint9", "joint10", "joint11",
        ];
        let mut spine = [(0, skinned::IDENTITY); 11];
        for (slot, name) in spine.iter_mut().zip(names) {
            *slot = rig.bind(name)?;
        }
        let length = m::length(m::sub(
            m::translation(&spine[0].1),
            m::translation(&spine[10].1),
        ));
        Ok(Self {
            id,
            handle: 0,
            rig,
            state: TentacleState::None,
            previous: TentacleState::None,
            transform: skinned::IDENTITY,
            grabbed: None,
            hit_list: HitList::default(),
            grab_bind,
            spine,
            grab_transform: skinned::IDENTITY,
            top_joint: skinned::IDENTITY,
            crush_target: None,
            original_forward: [0.0, 0.0, -1.0],
            search_for_target: false,
            seconds_out_of_range: 0,
            check_timer: 0.0,
            cthulhu_killed: false,
            hit_flash: 0.0,
            hit_points: 0.0,
            wetness: 0.0,
            hp_on_last_emerge: 0.0,
            time_until_submerge: 0.0,
            time_since_last_emerge: 0.0,
            length,
            statuses: [TentacleStatus::default(); 9],
            current: 0,
            dry_timer: 0.0,
            resistances: resistances(|e| {
                if e == E::WATER || e == E::POISON {
                    (0.0, 0.0)
                } else {
                    (1.0, 0.0)
                }
            }),
            locked_number: false,
            time_since_last_damage: 0.0,
            dead: true,
            active: false,
            capsules: Vec::new(),
            t: TentacleTimers::default(),
        })
    }

    pub fn has(&self, kind: Status) -> bool {
        self.current & kind as u16 == kind as u16
    }
    pub fn magnitude(&self, kind: Status) -> f32 {
        self.statuses[index_of(kind)].magnitude
    }
    fn cold_slowdown(&self) -> f32 {
        let s = &self.statuses[index_of(Status::Cold)];
        if s.kind == Status::Cold {
            s.slowdown()
        } else {
            1.0
        }
    }
    fn position(&self) -> V3 {
        m::translation(&self.transform)
    }
    fn notifier(&self) -> V3 {
        m::add(self.position(), [0.0, 3.0, 0.0])
    }
    fn animation_finished(&self) -> bool {
        !self.rig.fading() && self.rig.finished()
    }
    fn cue(&self, world: &mut dyn CthulhuWorld, name: &'static str) {
        world.cue(ADDITIONAL, name, self.position());
    }
    fn effect_key(&self, what: &str) -> String {
        format!("cthulhu_tentacle{}_{what}", self.id)
    }
    fn water_point(&self, water_y: f32) -> Mat {
        let mut p = self.position();
        p[1] = water_y;
        translation_matrix(p)
    }

    /// Tentacle.Start at a spawn transform.
    fn start(&mut self, transform: Mat, owner: &Owner, world: &mut dyn CthulhuWorld) {
        self.transform = transform;
        self.original_forward = m::forward(&self.transform);
        self.hit_list.clear();
        self.hit_points = TENTACLE_HIT_POINTS;
        self.dead = false;
        if self.state == TentacleState::None {
            self.previous = TentacleState::Idle;
            self.state = TentacleState::Emerge;
            self.enter(owner, world);
        } else {
            self.change(TentacleState::Emerge, owner, world);
        }
    }

    fn change(&mut self, state: TentacleState, owner: &Owner, world: &mut dyn CthulhuWorld) {
        if self.state != TentacleState::None {
            self.exit(owner, world);
        }
        self.previous = self.state;
        self.state = state;
        self.enter(owner, world);
    }

    fn enter(&mut self, owner: &Owner, world: &mut dyn CthulhuWorld) {
        use tentacle_clip as c;
        use TentacleState as S;
        match self.state {
            S::None => {}
            S::Idle => {
                let fade = match self.previous {
                    S::Submerge => 1.0,
                    S::GrabSuccess | S::Squeeze => 2.0,
                    S::Aim => -1.0,
                    _ => 0.6,
                };
                if fade != -1.0 && !self.rig.playing(c::IDLE) {
                    self.rig.crossfade(c::IDLE, fade, true);
                }
                self.t.idle = owner.time_between_actions;
            }
            S::Emerge => {
                self.dead = false;
                self.add_status(TentacleStatus::new(Status::Wet, 0.0, 1.0, 2.0));
                self.hp_on_last_emerge = self.hit_points;
                self.time_since_last_emerge = 0.0;
                self.time_until_submerge = 30.0 + world.random() * 5.0;
                let fire = dmg(attack::STATUS, E::FIRE, 100.0, 4.0);
                world.freeze(self.position(), [2.5, 0.0, 0.0], fire);
                self.t.unfreeze_done = false;
                self.t.emerge_effect_done = false;
                self.rig.start(c::EMERGE, false);
                if owner.initial_emerge {
                    self.t.bubbles_done = true;
                    self.t.bubble_timer = 0.0;
                } else {
                    self.t.bubbles_done = false;
                    self.t.bubble_timer = 1.5;
                    self.rig.stop();
                }
                self.cue(world, "cthulhu_tentacle_emerge");
            }
            S::Submerge => {
                self.rig.crossfade(c::SUBMERGE, 0.5, false);
                self.t.regeneration = if self.hit_points <= 0.0 {
                    10.0 + (world.random() * 6.0).floor().min(5.0)
                } else {
                    0.0
                };
                self.t.submerge_effect_done = false;
                self.cue(world, "cthulhu_tentacle_submerge");
            }
            S::Die => {
                self.rig.crossfade(c::DIE, 0.5, false);
                self.grabbed = None;
                self.t.die_effect_done = false;
                self.cue(world, "cthulhu_tentacle_submerge");
                // LeaveSpawnTransform is done by the owner (see leave).
            }
            S::SwipeGrab => {
                self.rig.crossfade(c::GRIP, 0.2, false);
                self.grabbed = None;
                self.t.swipe_done = false;
                self.t.swipe_deflected = false;
                self.cue(world, "cthulhu_tentacle_grab_success");
            }
            S::GrabSuccess => {
                self.rig.crossfade(c::GRIP_SUCCESS, 0.1, false);
                self.cue(world, "cthulhu_tentacle_grab_success");
            }
            S::Squeeze => {
                self.rig.crossfade(c::SQUEEZE, 0.15, true);
                self.t.squeeze = 5.0 - owner.time_between_actions;
                self.cue(world, "cthulhu_tentacle_squeeze");
            }
            S::Release => {
                self.rig.crossfade(c::LET_GO, 0.2, false);
                self.cue(world, "cthulhu_tentacle_release");
            }
            S::Aim => {
                self.t.aim_radius = self.length;
                self.search_for_target = true;
                self.rig.crossfade(c::IDLE, 0.6, true);
            }
            S::Crush => {
                self.rig.crossfade(c::CRUSH, 0.15, false);
                self.t.crush_done = false;
                self.t.crush_deflected = false;
            }
        }
    }

    fn exit(&mut self, _owner: &Owner, world: &mut dyn CthulhuWorld) {
        match self.state {
            TentacleState::Submerge => {
                self.dead = true;
                world.track(&self.effect_key("idle"), "", None);
            }
            TentacleState::Die => self.dead = true,
            _ => {}
        }
    }

    fn characters_within(&self, radius: f32, world: &mut dyn CthulhuWorld) -> bool {
        world
            .entities(self.position(), radius, true, false)
            .iter()
            .any(|e| e.kind == Kind::Avatar && !e.ethereal && !e.invisible)
    }

    fn submerge_active(&self) -> bool {
        self.hp_on_last_emerge - self.hit_points > 0.5 * TENTACLE_HIT_POINTS
            || self.time_until_submerge < 0.0
            || (self.time_since_last_emerge > 5.0 && self.seconds_out_of_range > 0)
    }

    fn swipe_active(&self, world: &mut dyn CthulhuWorld) -> bool {
        if world.random() >= 0.5 {
            return false;
        }
        let mut flag = false;
        for e in world.entities(self.position(), self.length - 1.2, true, false) {
            if matches!(e.kind, Kind::Shield | Kind::Barrier) {
                return false;
            }
            if !flag {
                flag = e.kind == Kind::Avatar && !e.ethereal && !e.invisible;
            }
        }
        flag
    }

    /// Tentacle.Update (after base Entity.Update).
    fn update(&mut self, dt: f32, owner: &Owner, world: &mut dyn CthulhuWorld) -> Option<Request> {
        let mut request = None;
        if self.hit_points <= 0.0
            && !self.cthulhu_killed
            && self.state != TentacleState::Submerge
            && self.state != TentacleState::Release
        {
            let next = if self.grabbed.is_none() {
                TentacleState::Submerge
            } else {
                TentacleState::Release
            };
            self.change(next, owner, world);
        }
        if self.state == TentacleState::None {
            return None;
        }
        self.update_damage(dt, world);
        self.update_statuses(dt);
        self.time_until_submerge -= dt;
        self.time_since_last_emerge += dt;
        self.hit_list.update(dt);
        self.update_state(dt, owner, world, &mut request);
        let elapsed = owner.stage_speed * self.cold_slowdown() * dt;
        let parent = self.transform;
        self.rig.update(elapsed, &parent);
        self.grab_transform = self.rig.joint(self.grab_bind.0, &self.grab_bind.1);
        self.top_joint = self.rig.joint(self.spine[10].0, &self.spine[10].1);
        self.capsules = (0..10)
            .map(|i| {
                (
                    self.rig.point(self.spine[i].0, &self.spine[i].1),
                    self.rig.point(self.spine[i + 1].0, &self.spine[i + 1].1),
                )
            })
            .collect();
        if let Some(g) = self.grabbed {
            if world.entity(g).is_none_or(|e| e.dead) {
                world.overkill(g);
                self.grabbed = None;
            } else {
                world.move_body(g, m::translation(&self.grab_transform));
            }
        }
        self.hit_flash = (self.hit_flash - dt * 5.0).max(0.0);
        self.wet_specular(dt);
        self.check_in_range(dt, world);
        if world_effect_active(self) {
            world.track(
                &self.effect_key("idle"),
                "cthulhu_tentacle_water_surface",
                Some(self.water_point(owner.water_y)),
            );
        }
        request
    }

    fn wet_specular(&mut self, dt: f32) {
        if self.magnitude(Status::Wet) > 0.0 {
            if self.wetness < 1.0 {
                self.wetness += dt;
            } else {
                self.wetness = 1.0;
            }
        } else if self.wetness > 0.0 {
            self.wetness -= dt;
        } else {
            self.wetness = 0.0;
        }
    }

    fn check_in_range(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        self.check_timer -= dt;
        if self.check_timer < 0.0 {
            if self.characters_within(self.length, world) {
                self.seconds_out_of_range = 0;
            } else {
                self.seconds_out_of_range += 1;
            }
            self.check_timer = 1.0;
        }
    }

    fn update_state(
        &mut self,
        dt: f32,
        owner: &Owner,
        world: &mut dyn CthulhuWorld,
        request: &mut Option<Request>,
    ) {
        use TentacleState as S;
        match self.state {
            S::None => {}
            S::Idle => {
                if self.t.idle <= 0.0 && owner.ok_to_fight {
                    if self.submerge_active() {
                        self.change(S::Submerge, owner, world);
                    } else if self.swipe_active(world) {
                        self.change(S::SwipeGrab, owner, world);
                    } else if self.characters_within(self.length, world) {
                        self.change(S::Aim, owner, world);
                    }
                }
                self.t.idle -= dt;
            }
            S::Emerge => {
                if self.t.bubble_timer >= 0.0 {
                    self.t.bubble_timer -= dt;
                    if self.t.bubble_timer <= 0.0 {
                        self.rig.start(tentacle_clip::EMERGE, false);
                        world.track(&self.effect_key("bubbles"), "", None);
                    } else if !self.t.bubbles_done {
                        world.track(
                            &self.effect_key("bubbles"),
                            "cthulhu_tentacle_emerge_bubbles",
                            Some(self.water_point(owner.water_y)),
                        );
                        self.t.bubbles_done = true;
                    }
                    return;
                }
                if self.rig.fading() {
                    return;
                }
                if self.animation_finished() {
                    self.change(S::Idle, owner, world);
                }
                let num = self.rig.normalized();
                if !self.t.unfreeze_done && num >= 1.0 / 11.0 {
                    let fire = dmg(attack::STATUS, E::FIRE, 100.0, 4.0);
                    world.freeze(self.position(), [4.0, 0.0, 0.0], fire);
                    self.t.unfreeze_done = true;
                }
                if !self.t.emerge_effect_done && num >= 0.0 {
                    world.effect(
                        "cthulhu_tentacle_emerge_water_splash",
                        self.water_point(owner.water_y),
                    );
                    self.t.emerge_effect_done = true;
                    world.track(
                        &self.effect_key("idle"),
                        "cthulhu_tentacle_water_surface",
                        Some(self.water_point(owner.water_y)),
                    );
                }
            }
            S::Submerge => {
                if self.rig.fading() {
                    return;
                }
                if self.animation_finished() {
                    self.t.regeneration -= dt;
                    if self.t.regeneration <= 0.0 {
                        // Revive, then SpawnTentacleAtGoodPoint(this).
                        if self.hit_points <= 0.0 {
                            self.hit_points = TENTACLE_HIT_POINTS;
                        }
                        *request = Some(Request::Respawn);
                        return;
                    }
                }
                let num = self.rig.normalized();
                if !self.t.submerge_effect_done && num >= 0.465_116_3 {
                    world.effect(
                        "cthulhu_tentacle_emerge_water_splash",
                        self.water_point(0.0),
                    );
                    world.track(&self.effect_key("idle"), "", None);
                    self.t.submerge_effect_done = true;
                }
            }
            S::Die => {
                if self.rig.fading() {
                    return;
                }
                let num = self.rig.normalized();
                if !self.t.die_effect_done && num >= 0.465_116_3 {
                    world.effect(
                        "cthulhu_tentacle_emerge_water_splash",
                        self.water_point(0.0),
                    );
                    self.t.die_effect_done = true;
                    world.track(&self.effect_key("idle"), "", None);
                }
            }
            S::SwipeGrab => self.update_swipe(owner, world),
            S::GrabSuccess => {
                if self.animation_finished() {
                    self.change(S::Squeeze, owner, world);
                }
            }
            S::Squeeze => {
                let burning = self
                    .grabbed
                    .and_then(|g| world.entity(g))
                    .is_some_and(|e| e.burning);
                if burning {
                    self.t.squeeze = 0.0;
                }
                if self.t.squeeze <= 0.0 && self.grabbed.is_some() {
                    self.change(S::Release, owner, world);
                    return;
                }
                self.t.squeeze -= dt;
                let Some(g) = self.grabbed else {
                    self.change(S::Idle, owner, world);
                    return;
                };
                if !self.rig.fading() && !self.hit_list.contains(g) {
                    let num = self.rig.normalized();
                    if (0.4..=0.7).contains(&num) {
                        let squeeze = dmg(attack::DAMAGE | attack::BLEED, E::EARTH, 150.0, 1.0);
                        world.damage(g, squeeze, None, m::translation(&self.grab_transform));
                        self.hit_list.add(g);
                    }
                }
            }
            S::Release => {
                if self.rig.fading() {
                    return;
                }
                if self.rig.finished() {
                    self.change(S::Idle, owner, world);
                    return;
                }
                if let Some(g) = self.grabbed {
                    if self.rig.normalized() >= 0.405_405_4 {
                        let forward = m::scale(flat(m::forward(&self.transform)), 12.0);
                        world.set_velocity(g, forward);
                        self.grabbed = None;
                    }
                }
            }
            S::Aim => self.update_aim(dt, owner, world),
            S::Crush => self.update_crush(owner, world),
        }
    }

    fn update_swipe(&mut self, owner: &Owner, world: &mut dyn CthulhuWorld) {
        if self.grabbed.is_some() {
            self.change(TentacleState::GrabSuccess, owner, world);
            return;
        }
        if self.rig.fading() {
            return;
        }
        if self.rig.finished() || self.t.swipe_deflected {
            self.change(TentacleState::Idle, owner, world);
        }
        if self.t.swipe_done {
            return;
        }
        let num = self.rig.normalized();
        let origin = self.position();
        let delta = m::sub(m::translation(&self.top_joint), origin);
        let center = m::add(origin, m::scale(delta, 0.5));
        let reach = m::length(delta) + 1.5;
        let top = m::translation(&self.top_joint);
        let swipe = dmg(attack::DAMAGE, E::EARTH, 150.0, 1.0);
        for e in world.entities(center, reach / 2.0, false, false) {
            if self.t.swipe_done {
                break;
            }
            if e.handle == self.handle {
                continue;
            }
            match e.kind {
                Kind::Shield | Kind::Barrier => {
                    world.damage(e.handle, swipe, None, center);
                    self.t.swipe_done = true;
                    self.t.swipe_deflected = true;
                    self.rig.crossfade(tentacle_clip::IDLE, 0.15, true);
                }
                Kind::Mine => {}
                _ => {
                    if !(5.0 / 28.0..=9.0 / 28.0).contains(&num) {
                        continue;
                    }
                    let touching = m::length(m::sub(top, e.position)) <= 1.0 + e.radius;
                    if touching && e.kind == Kind::Avatar {
                        self.t.swipe_done = true;
                        self.grabbed = Some(e.handle);
                    } else if let Some(mut p) = world.segment_hit(e.handle, origin, delta, 0.6) {
                        if !self.hit_list.contains(e.handle) {
                            p = m::add(p, right(&self.transform));
                            world.damage(e.handle, swipe, None, p);
                            let knock = dmg(attack::KNOCKBACK, E::EARTH, e.mass, 2.0);
                            world.damage(e.handle, knock, Some(self.handle), p);
                            self.hit_list.add(e.handle);
                        }
                    }
                    // The native loop breaks after the first damageable in
                    // the grab window.
                    break;
                }
            }
        }
    }

    fn update_aim(&mut self, dt: f32, owner: &Owner, world: &mut dyn CthulhuWorld) {
        let translation = self.position();
        let forward = m::forward(&self.transform);
        let mut target_direction = [0.0; 3];
        let mut flag = false;
        if self.search_for_target {
            let radius = self.t.aim_radius;
            let near = world.entities(translation, radius, false, true);
            let mut avatar = self
                .crush_target
                .and_then(|h| near.iter().find(|e| e.handle == h).copied());
            if self.crush_target.is_some() && avatar.is_none_or(|a| a.dead || a.drowning) {
                self.crush_target = None;
                avatar = None;
            }
            if let Some(a) = avatar {
                if !a.dead && !a.ethereal {
                    let dx = translation[0] - a.position[0];
                    let dz = translation[2] - a.position[2];
                    if dx * dx + dz * dz > radius * radius {
                        avatar = None;
                    }
                }
            }
            match avatar {
                None => {
                    for e in near.iter().filter(|e| e.handle != self.handle) {
                        if flag {
                            break;
                        }
                        flag = e.kind == Kind::Avatar && !e.ethereal;
                        if flag {
                            self.crush_target = Some(e.handle);
                            target_direction = flat(m::sub(e.position, translation));
                            if m::length_squared(target_direction) <= 1e-6 {
                                flag = false;
                            } else {
                                target_direction = m::normalize(target_direction);
                            }
                        }
                    }
                }
                Some(a) => {
                    flag = true;
                    target_direction = flat(m::sub(a.position, translation));
                    if m::length_squared(target_direction) <= 1e-6 {
                        flag = false;
                    } else {
                        target_direction = m::normalize(target_direction);
                    }
                }
            }
        }
        if !flag {
            target_direction = self.original_forward;
        }
        // GetAngle: acos(clamp(cross(target, forward).Y)) − π/2.
        let cross = m::cross(target_direction, forward);
        let angle = cross[1].clamp(-1.0, 1.0).acos() - std::f32::consts::FRAC_PI_2;
        if angle.abs() < 0.01 && flag {
            self.change(TentacleState::Crush, owner, world);
        }
        let limit = 30f32.to_radians();
        let step = if angle < 0.0 {
            angle.max(-limit)
        } else {
            angle.min(limit)
        };
        let yaw = 2.0 * step * owner.stage_speed * self.cold_slowdown() * dt;
        let position = self.position();
        self.transform = m::yaw_matrix(&self.transform, yaw);
        m::set_translation(&mut self.transform, position);
        if angle.abs() < 0.01 && !flag {
            self.change(TentacleState::Idle, owner, world);
        }
    }

    fn update_crush(&mut self, owner: &Owner, world: &mut dyn CthulhuWorld) {
        if self.rig.fading() {
            return;
        }
        if self.rig.finished() || self.t.crush_deflected {
            self.change(TentacleState::Aim, owner, world);
            self.crush_target = None;
            self.search_for_target = false;
        }
        if self.t.crush_done {
            return;
        }
        let num = self.rig.normalized();
        let origin = self.position();
        let delta = m::sub(m::translation(&self.top_joint), origin);
        let center = m::add(origin, m::scale(delta, 0.5));
        let crush = dmg(attack::DAMAGE, E::EARTH, 500.0, 1.0);
        for e in world.entities(center, m::length(delta) / 2.0, false, false) {
            if e.handle == self.handle {
                continue;
            }
            let Some(p) = world.segment_hit(e.handle, origin, delta, 0.6) else {
                continue;
            };
            match e.kind {
                Kind::Shield => {
                    self.t.crush_done = true;
                    world.kill(e.handle);
                    self.t.crush_deflected = true;
                    self.rig.crossfade(tentacle_clip::IDLE, 0.15, true);
                }
                Kind::Mine | Kind::Barrier => world.kill(e.handle),
                _ => {
                    if num >= 17.0 / 48.0 && !self.hit_list.contains(e.handle) {
                        world.damage(e.handle, crush, None, p);
                        let knock = dmg(attack::KNOCKBACK, E::EARTH, e.mass, 2.0);
                        world.damage(e.handle, knock, None, p);
                        self.hit_list.add(e.handle);
                    }
                }
            }
        }
        if num >= 17.0 / 48.0 && !self.t.crush_done {
            self.t.crush_done = true;
            world.effect(
                "cthulhu_tentacle_crush_dust",
                translation_matrix(m::translation(&self.top_joint)),
            );
            self.cue(world, "cthulhu_tentacle_crush");
        }
    }

    /// KillTentacle (Cthulhu's DeathState).
    fn kill(&mut self, owner: &Owner, world: &mut dyn CthulhuWorld) {
        if !self.dead {
            self.cthulhu_killed = true;
            self.dead = true;
            self.hit_points = 0.0;
            self.change(TentacleState::Die, owner, world);
        }
    }

    fn update_damage(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        self.time_since_last_damage += dt;
        if self.locked_number {
            if self.time_since_last_damage > 0.333 || self.dead {
                world.boss_event(
                    Some(self.id as usize),
                    BossEvent::ReleaseNumber,
                    self.notifier(),
                );
                self.locked_number = false;
            } else {
                world.boss_event(
                    Some(self.id as usize),
                    BossEvent::MoveNumber,
                    self.notifier(),
                );
            }
        }
    }

    fn update_statuses(&mut self, dt: f32) {
        self.dry_timer -= dt;
        let mut mask = 0u16;
        if self.dead {
            self.statuses = [TentacleStatus::default(); 9];
        } else {
            for i in 0..self.statuses.len() {
                let earth = self.statuses[i].update(dt);
                if earth != 0 {
                    // Damage(num, Earth): no grease, clamp to max.
                    self.hit_points = (self.hit_points - earth as f32).min(TENTACLE_HIT_POINTS);
                }
                let s = self.statuses[i];
                if s.dead() {
                    self.statuses[i] = TentacleStatus::default();
                } else if s.kind == Status::Wet {
                    if s.magnitude >= 1.0 {
                        mask |= s.kind as u16;
                    }
                } else {
                    mask |= s.kind as u16;
                }
            }
        }
        self.current = mask;
    }

    fn add_status(&mut self, mut effect: TentacleStatus) -> u16 {
        if effect.dead() {
            return 0;
        }
        let alive = |s: &Self, k: Status| !s.statuses[index_of(k)].dead();
        let clear = |s: &mut Self, k: Status| s.statuses[index_of(k)] = TentacleStatus::default();
        let mut removed = false;
        match effect.kind {
            Status::Burning => {
                if alive(self, Status::Cold)
                    || alive(self, Status::Wet)
                    || alive(self, Status::Frozen)
                    || self.dry_timer > 0.0
                {
                    clear(self, Status::Cold);
                    clear(self, Status::Wet);
                    clear(self, Status::Frozen);
                    removed = true;
                }
            }
            Status::Cold => {
                let mut magnitude = effect.magnitude;
                if alive(self, Status::Burning) || self.dry_timer > 0.0 {
                    clear(self, Status::Burning);
                    self.dry_timer = 0.9;
                    removed = true;
                }
                magnitude *= self.resistances[index_of_element(E::COLD)].multiplier;
                if self.has(Status::Wet) {
                    self.statuses[index_of(Status::Frozen)] =
                        TentacleStatus::new(Status::Frozen, 0.0, magnitude, 0.0);
                    clear(self, Status::Wet);
                }
                if self.has(Status::Frozen) {
                    self.statuses[index_of(Status::Frozen)].magnitude += magnitude;
                    magnitude = 0.0;
                }
                effect.magnitude = magnitude;
            }
            Status::Wet => {
                if self.has(Status::Burning) || self.dry_timer > 0.0 {
                    clear(self, Status::Burning);
                    removed = true;
                }
                if self.has(Status::Greased) {
                    clear(self, Status::Greased);
                }
            }
            Status::Greased => {
                if self.has(Status::Wet) {
                    clear(self, Status::Wet);
                }
            }
            Status::Poisoned => {
                effect.magnitude *= self.resistances[index_of_element(E::POISON)].multiplier
            }
            Status::Healing if self.has(Status::Poisoned) => {
                clear(self, Status::Poisoned);
            }
            _ => {}
        }
        if removed {
            return 0x40;
        }
        let i = index_of(effect.kind);
        self.statuses[i] = self.statuses[i].add(effect);
        0x20
    }

    /// Tentacle.InternalDamage for one Damage.
    pub fn damage(&mut self, mut d: Damage, position: V3, world: &mut dyn CthulhuWorld) -> u16 {
        let element = E(d.element);
        let mut out = 0;
        let (mut multipliers, mut count) = (0.0f32, 0.0f32);
        for (i, r) in self.resistances.iter().enumerate() {
            if element.has(E::BY_INDEX[i]) {
                if element == E::EARTH && r.modifier != 0.0 {
                    d.amount = ((d.amount + r.modifier).max(0.0)) as i32 as f32;
                } else {
                    d.amount += r.modifier as i32 as f32;
                }
                multipliers += r.multiplier;
                count += 1.0;
            }
        }
        if count != 0.0 {
            d.magnitude *= multipliers / count;
        }
        let eps = f32::from_bits(1);
        if d.magnitude.abs() <= eps {
            return R::DEFLECTED;
        }
        let resist = |s: &Self, e: E| s.resistances[index_of_element(e)].multiplier;
        if d.attack & attack::STATUS != 0 && multipliers.abs() > eps {
            for (e, kind) in [
                (E::FIRE, Status::Burning),
                (E::COLD, Status::Cold),
                (E::WATER, Status::Wet),
                (E::STEAM, Status::Steamed),
            ] {
                if element.has(e) && resist(self, e) > eps {
                    out |= self.add_status(TentacleStatus::new(kind, d.amount, d.magnitude, 0.0));
                }
            }
        }
        if d.attack & attack::DAMAGE != 0 {
            if element.has(E::LIGHTNING) && self.has(Status::Wet) {
                d.amount *= 2.0;
            }
            if element.has(E::LIFE) {
                let p = &mut self.statuses[index_of(Status::Poisoned)];
                p.magnitude -= d.magnitude;
                if p.magnitude <= 0.0 {
                    p.magnitude = 0.0;
                }
            }
            if element.intersects(E::PHYSICAL_ELEMENTS) && !self.has(Status::Frozen) {
                world.boss_event(
                    Some(self.id as usize),
                    BossEvent::Blood { position },
                    self.notifier(),
                );
            }
            d.amount *= d.magnitude;
            self.hit_points -= d.amount;
            if d.amount > 0.0 {
                self.hit_flash = 0.5;
            }
            if d.attack & attack::PIERCING != 0 && d.magnitude > 0.0 && d.amount > 0.0 {
                out |= R::PIERCED;
            }
            if d.amount > 0.0 {
                out |= R::DAMAGED;
            }
            if d.amount == 0.0 {
                out |= R::DEFLECTED;
            }
            if d.amount < 0.0 {
                out |= R::HEALED;
            }
            out |= R::HIT;
            if d.amount != 0.0 {
                self.time_since_last_damage = 0.0;
            }
            let event = if self.locked_number {
                BossEvent::AddToNumber { amount: d.amount }
            } else {
                self.locked_number = true;
                BossEvent::LockedNumber { amount: d.amount }
            };
            world.boss_event(Some(self.id as usize), event, self.notifier());
        }
        if self.hit_points > TENTACLE_HIT_POINTS {
            self.hit_points = TENTACLE_HIT_POINTS;
        }
        if d.amount == 0.0 {
            out |= R::DEFLECTED;
        }
        if self.hit_points <= 0.0 {
            out |= R::KILLED;
        }
        out
    }

    /// The cold colorize (Colorize.W) drawn by the viewer.
    pub fn cold_tint(&self) -> [f32; 4] {
        let w = (self.magnitude(Status::Cold) * 10.0).min(1.0);
        [COLD_COLOR[0], COLD_COLOR[1], COLD_COLOR[2], w]
    }
}

fn world_effect_active(t: &Tentacle) -> bool {
    // The water-surface effect follows from Emerge's PFX until the splash.
    matches!(
        t.state,
        TentacleState::Idle
            | TentacleState::SwipeGrab
            | TentacleState::GrabSuccess
            | TentacleState::Squeeze
            | TentacleState::Release
            | TentacleState::Aim
            | TentacleState::Crush
    )
}

fn index_of_element(e: E) -> usize {
    e.0.trailing_zeros() as usize
}

/// A tentacle asks its owner to respawn it (SpawnTentacleAtGoodPoint).
#[derive(Clone, Copy, Debug)]
enum Request {
    Respawn,
}

/// The owner values a tentacle reads.
#[derive(Clone, Copy, Debug)]
struct Owner {
    time_between_actions: f32,
    stage_speed: f32,
    ok_to_fight: bool,
    initial_emerge: bool,
    water_y: f32,
}

// --- the mist ----------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Mist {
    pub handle: u16,
    pub position: V3,
    pub active: bool,
    timer: f32,
    hit_list: HitList,
}

impl Mist {
    fn initialize(&mut self, position: V3, world: &mut dyn CthulhuWorld) {
        self.position = position;
        world.track(
            "cthulhu_mist",
            "cthulhu_mist",
            Some(translation_matrix(position)),
        );
        self.active = true;
        self.timer = 10.0;
    }
    fn deactivate(&mut self, world: &mut dyn CthulhuWorld) {
        world.track("cthulhu_mist", "", None);
        self.active = false;
    }
    fn set_on_fire(&mut self, world: &mut dyn CthulhuWorld) {
        world.track("cthulhu_mist", "", None);
        world.effect("cthulhu_mist_on_fire", translation_matrix(self.position));
        self.active = false;
        let fire = dmg(attack::STATUS, E::FIRE, 100.0, 2.0);
        world.circle_damage(self.handle, self.position, MIST_RADIUS * 1.5, fire);
    }
    fn update(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        self.hit_list.update(dt);
        if self.active {
            self.timer -= dt;
            if self.timer <= 0.0 {
                self.deactivate(world);
            }
        }
    }
    /// CthulhuMist.OnCollision for an overlapping damageable.
    pub fn contact(
        &mut self,
        e: &Near,
        missile_fire: bool,
        immortal: bool,
        charmed: bool,
        world: &mut dyn CthulhuWorld,
    ) {
        if !self.active {
            return;
        }
        if e.burning {
            self.set_on_fire(world);
        }
        if self.hit_list.contains(e.handle) {
            return;
        }
        if e.kind == Kind::Missile {
            self.hit_list.add(e.handle);
            if missile_fire {
                self.set_on_fire(world);
            }
            return;
        }
        if e.starspawn {
            return;
        }
        // The hit list is not filled for characters (native behaviour):
        // each frame inside poisons again.
        let poison = dmg(attack::STATUS, E::POISON, 50.0, 1.0);
        world.damage(e.handle, poison, None, [0.0; 3]);
        if e.kind == Kind::Avatar && !charmed && !e.dead && !immortal {
            world.charm_and_confuse(e.handle, self.handle);
        }
    }
    /// CthulhuMist.Damage: fire sets it alight; it is always OverKilled.
    pub fn damage(&mut self, d: Damage, world: &mut dyn CthulhuWorld) -> u16 {
        if d.element == E::FIRE.0 {
            self.set_on_fire(world);
        }
        R::OVERKILLED
    }
}

// --- Cthulhu -------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct StateData {
    idle_timer: f32,
    idle_done: bool,
    mist_casted: bool,
    mist_cooldown: f32,
    // EmergeState.
    emerge_logic: bool,
    emerge_effect: bool,
    emerge_unfreeze: bool,
    emerge_bubbles: bool,
    emerge_bubble_timer: f32,
    // SubmergeState.
    submerge_timer: f32,
    submerge_logic: bool,
    submerge_effect: bool,
    // TimewarpState.
    timewarp_cooldown: f32,
    timewarp_casted: bool,
    timewarp_sound: bool,
    // LightningState.
    lightning_hits: HitList,
    lightning_timers: Vec<(f32, usize)>,
    lightning_timer: f32,
    lightning_cue: Option<u32>,
    lightning_cooldown: f32,
    // CallState.
    call_casted: bool,
    // LesserCallState.
    lesser_timer: f32,
    lesser_timers: Vec<(f32, usize)>,
    lesser_closed: Vec<usize>,
    lesser_cooldown: f32,
    // DevourState / DevourHitState.
    devour_sound: bool,
    devour_effect: bool,
    devour_suck: Option<u32>,
    eat_sound: bool,
    eat_effect: bool,
    eat_terminated: bool,
    eat_scale: f32,
    eat_offset: V3,
    eat_offset_up: V3,
    // HypnotizeState.
    hypnotize_casted: bool,
    hypnotize_effect: bool,
    hypnotize_cooldown: f32,
    // RageState.
    rage_called: bool,
    rage_rumble: bool,
    rage_spawns: bool,
    // OtherworldlyBoltState.
    bolt_state: u8,
    bolt_casted: bool,
    bolts_casted: usize,
    bolts_to_cast: usize,
    bolt: Option<usize>,
    bolt_cooldown: f32,
    // DeathState.
    death_effect: bool,
}

#[derive(Clone, Debug)]
pub struct Cthulhu {
    pub status: BossStatus,
    pub rig: BossRig,
    pub state: State,
    pub previous: State,
    pub stage: Stage,
    pub transform: Mat,
    stage_speed: f32,
    time_between_actions: f32,
    tentacle_count: usize,
    pub tentacles: Vec<Tentacle>,
    /// mActiveTentacles (indices into `tentacles`, in activation order).
    active: Vec<usize>,
    inactive: Vec<usize>,
    pub mist: Mist,
    pub zone: u16,
    /// Spawn point occupancy by handle.
    occupied: [Option<u16>; 8],
    spawn_transforms: [Mat; 8],
    deep_one_transforms: [Mat; 8],
    mist_transform: Mat,
    desired_spawn: Option<usize>,
    deflection: f32,
    character_to_eat: Option<(u16, f32)>,
    hp_on_last_emerge: f32,
    time_until_submerge: f32,
    time_since_last_emerge: f32,
    pub hit_flash: f32,
    seconds_out_of_range: i32,
    check_range_timer: f32,
    pub wetness: f32,
    activate_mist: bool,
    mist_desirability: f32,
    mist_timer: f32,
    pub water_y: f32,
    initial_emerge: bool,
    ok_to_fight: bool,
    king_fallen: bool,
    star_spawn_spawned: bool,
    pub dead: bool,
    players: Vec<Player>,
    player_count: usize,
    lesser_spawns: usize,
    // Joints of the last update.
    mouth: Mat,
    head: Mat,
    left_hand: Mat,
    right_hand: Mat,
    left_fingers: [Mat; 6],
    right_fingers: [Mat; 6],
    /// The body zone: spine1 position, spine1→spine5 axis.
    pub zone_origin: V3,
    pub zone_axis: V3,
    /// The arm capsules (start, end, radius): left upper/fore, right
    /// upper/fore.
    pub arms: [(V3, V3, f32); 4],
    binds: Binds,
    s: StateData,
    rng: u64,
}

#[derive(Clone, Debug)]
struct Binds {
    head: (usize, Mat),
    mouth: (usize, Mat),
    left_hand: (usize, Mat),
    right_hand: (usize, Mat),
    left_fingers: [(usize, Mat); 6],
    right_fingers: [(usize, Mat); 6],
    left_arm: [(usize, Mat); 3],
    right_arm: [(usize, Mat); 3],
    spine: [(usize, Mat); 5],
}

impl Cthulhu {
    /// Cthulhu's constructor: the rig, four tentacles and the mist.
    /// `player_count` is Game.Instance.PlayerCount at construction. It fixes
    /// the Lesser Call spawn count and the Otherworldly Bolt rules, both of
    /// which stop at their four-player values in larger lobbies.
    pub fn new(
        animation: &Resolved,
        tentacle_animation: &Resolved,
        player_count: usize,
        seed: u64,
    ) -> Result<Self, String> {
        let rig = BossRig::new(animation, &clip::ALL)?;
        let finger = |side: &str| -> Result<[(usize, Mat); 6], String> {
            let names = [
                format!("{side}thumb2"),
                format!("{side}finger1_3"),
                format!("{side}finger2_3"),
                format!("{side}finger3_3"),
                format!("{side}finger4_3"),
                format!("{side}finger5_3"),
            ];
            let mut out = [(0, skinned::IDENTITY); 6];
            for (slot, n) in out.iter_mut().zip(&names) {
                *slot = rig.bind(n)?;
            }
            Ok(out)
        };
        let arm = |side: &str| -> Result<[(usize, Mat); 3], String> {
            Ok([
                rig.bind(&format!("{side}shoulder"))?,
                rig.bind(&format!("{side}elbow"))?,
                rig.bind(&format!("{side}hand"))?,
            ])
        };
        let mut spine = [(0, skinned::IDENTITY); 5];
        for (i, slot) in spine.iter_mut().enumerate() {
            *slot = rig.bind(&format!("spine{}", i + 1))?;
        }
        let left_fingers = finger("left")?;
        let right_fingers = finger("right")?;
        let binds = Binds {
            head: rig.bind("head")?,
            mouth: rig.bind("attach")?,
            // The hands are the third fingers (finger3_3).
            left_hand: left_fingers[3],
            right_hand: right_fingers[3],
            left_fingers,
            right_fingers,
            left_arm: arm("left")?,
            right_arm: arm("right")?,
            spine,
        };
        let tentacles = (0..4)
            .map(|i| Tentacle::new(i as u8, tentacle_animation))
            .collect::<Result<Vec<_>, _>>()?;
        let player_count = crate::slots::encounter_players(player_count);
        let lesser_spawns = match player_count {
            1 => 2,
            2 => 4,
            3 => 5,
            4 => 6,
            _ => 1,
        };
        let mut status = BossStatus::default();
        status.rules = BossRules::Cthulhu;
        status.hit_points = MAX_HIT_POINTS;
        status.max_hit_points = MAX_HIT_POINTS;
        status.resistances = resistances(|e| match e {
            E::WATER | E::POISON => (0.0, 0.0),
            E::COLD => (0.25, 0.0),
            E::EARTH => (0.25, -500.0),
            _ => (1.0, 0.0),
        });
        Ok(Self {
            status,
            rig,
            state: State::Emerge,
            previous: State::Emerge,
            stage: Stage::Intro,
            transform: skinned::IDENTITY,
            stage_speed: 1.0,
            time_between_actions: 1.5,
            tentacle_count: 2,
            tentacles,
            active: Vec::new(),
            inactive: (0..4).collect(),
            mist: Mist::default(),
            zone: 0,
            occupied: [None; 8],
            spawn_transforms: [skinned::IDENTITY; 8],
            deep_one_transforms: [skinned::IDENTITY; 8],
            mist_transform: skinned::IDENTITY,
            desired_spawn: None,
            deflection: 0.0,
            character_to_eat: None,
            hp_on_last_emerge: 0.0,
            time_until_submerge: 0.0,
            time_since_last_emerge: 0.0,
            hit_flash: 0.0,
            seconds_out_of_range: 0,
            check_range_timer: 0.0,
            wetness: 0.0,
            activate_mist: false,
            mist_desirability: 0.0,
            mist_timer: 0.0,
            water_y: -2.0,
            initial_emerge: true,
            ok_to_fight: false,
            king_fallen: false,
            star_spawn_spawned: false,
            dead: false,
            players: vec![Player::default(); 4],
            player_count,
            lesser_spawns,
            mouth: skinned::IDENTITY,
            head: skinned::IDENTITY,
            left_hand: skinned::IDENTITY,
            right_hand: skinned::IDENTITY,
            left_fingers: [skinned::IDENTITY; 6],
            right_fingers: [skinned::IDENTITY; 6],
            zone_origin: [0.0; 3],
            zone_axis: [0.0, 1.0, 0.0],
            arms: [([0.0; 3], [0.0; 3], 0.0); 4],
            binds,
            s: StateData::default(),
            rng: seed | 1,
        })
    }

    fn random(&mut self) -> f32 {
        xorshift(&mut self.rng)
    }
    fn random_below(&mut self, n: usize) -> usize {
        ((self.random() * n as f32) as usize).min(n.saturating_sub(1))
    }
    pub fn position(&self) -> V3 {
        m::translation(&self.transform)
    }
    pub fn health(&self) -> (f32, f32) {
        (self.status.hit_points, MAX_HIT_POINTS)
    }
    pub fn head(&self) -> Mat {
        self.head
    }
    /// The mouth attach orientation (mMouthAttachOrientation).
    pub fn mouth(&self) -> Mat {
        self.mouth
    }
    pub fn deflecting(&self) -> bool {
        self.deflection > 0.0
    }
    fn notifier(&self) -> V3 {
        m::add(m::translation(&self.head), [0.0, 3.0, 0.0])
    }
    fn owner(&self) -> Owner {
        Owner {
            time_between_actions: self.time_between_actions,
            stage_speed: self.stage_speed,
            ok_to_fight: self.ok_to_fight,
            initial_emerge: self.initial_emerge,
            water_y: self.water_y,
        }
    }
    fn cue(&self, world: &mut dyn CthulhuWorld, name: &'static str) -> u32 {
        world.cue(ADDITIONAL, name, self.zone_origin)
    }

    /// Cthulhu.Initialize at the spawn locator.
    pub fn initialize(&mut self, orientation: Mat, world: &mut dyn CthulhuWorld) {
        self.status.hit_points = MAX_HIT_POINTS;
        self.transform = orientation;
        self.initial_emerge = true;
        self.king_fallen = false;
        for (i, name) in SPAWN_LOCATORS.iter().enumerate() {
            self.spawn_transforms[i] = world.locator(name).unwrap_or(skinned::IDENTITY);
        }
        self.mist_transform = world.locator(MIST_LOCATOR).unwrap_or(skinned::IDENTITY);
        for (i, name) in DEEP_ONE_LOCATORS.iter().enumerate() {
            self.deep_one_transforms[i] = world.locator(name).unwrap_or(skinned::IDENTITY);
        }
        self.status.statuses = Default::default();
        self.dead = false;
        self.star_spawn_spawned = false;
        self.hit_flash = 0.0;
        self.occupied = [None; 8];
        self.occupied[0] = Some(self.zone);
        self.desired_spawn = None;
        self.active.clear();
        self.inactive = (0..self.tentacles.len()).collect();
        self.deflection = 0.0;
        self.mist.active = false;
        self.stage = Stage::Intro;
        // The water height under the north spawn point.
        let mut origin = m::translation(&self.spawn_transforms[0]);
        origin[1] = -10.0;
        self.water_y = world.water_height(origin, [0.0, 20.0, 0.0]).unwrap_or(-2.0);
        self.enter_stage(world);
        self.state = State::Emerge;
        self.previous = State::Emerge;
        self.enter(world);
        self.ok_to_fight = false;
        let parent = self.parent();
        self.rig.update(0.0, &parent);
        self.read_joints();
    }

    /// BossMessages.StartFight.
    pub fn message(&mut self, name: &str) {
        if name.eq_ignore_ascii_case("startfight") {
            self.ok_to_fight = true;
        }
    }

    fn parent(&self) -> Mat {
        let mut scale = skinned::IDENTITY;
        scale[0] = 1.25;
        scale[5] = 1.25;
        scale[10] = 1.25;
        skinned::mul(&scale, &self.transform)
    }

    // --- spawn points --------------------------------------------------

    fn change_spawn_point(&mut self, point: usize, handle: u16) -> Mat {
        if let Some(slot) = self.occupied.iter().position(|o| *o == Some(handle)) {
            self.occupied[slot] = None;
        }
        self.occupied[point] = Some(handle);
        self.spawn_transforms[point]
    }
    fn leave_spawn(&mut self, handle: u16) {
        if let Some(slot) = self.occupied.iter().position(|o| *o == Some(handle)) {
            self.occupied[slot] = None;
        }
    }
    fn find_good_spot(&mut self, radius: f32, world: &mut dyn CthulhuWorld) -> Option<usize> {
        let mut best = None;
        let mut best_sum = f32::MAX;
        for i in 0..self.spawn_transforms.len() {
            if self.occupied[i].is_some() {
                continue;
            }
            let at = m::translation(&self.spawn_transforms[i]);
            let mut sum = 0.0;
            let mut any = false;
            for e in world.entities(at, radius, true, false) {
                if e.kind == Kind::Avatar && !e.ethereal && !e.invisible {
                    any = true;
                    sum += m::length_squared(m::sub(e.position, at));
                }
            }
            if any && sum < best_sum {
                best = Some(i);
                best_sum = sum;
            }
        }
        best
    }
    fn find_random_spot(&mut self) -> Option<usize> {
        let mut index = None;
        for _ in 0..200 {
            let i = self.random_below(self.spawn_transforms.len());
            index = Some(i);
            if self.occupied[i].is_none() {
                return index;
            }
        }
        index
    }

    /// SpawnTentacleAtGoodPoint(null): the next inactive tentacle.
    fn spawn_new_tentacle(&mut self, world: &mut dyn CthulhuWorld) {
        if self.inactive.is_empty() {
            return;
        }
        let t = self.inactive.remove(0);
        self.active.push(t);
        if !self.tentacles[t].active {
            self.tentacles[t].active = true;
            self.tentacles[t].dead = true;
        }
        self.spawn_tentacle(t, world);
    }
    fn spawn_tentacle(&mut self, t: usize, world: &mut dyn CthulhuWorld) -> bool {
        let point = match self.find_good_spot(18.0, world) {
            Some(p) => p,
            None => match self.find_random_spot() {
                Some(p) => p,
                None => return false,
            },
        };
        let handle = self.tentacles[t].handle;
        let transform = self.change_spawn_point(point, handle);
        let owner = self.owner();
        self.tentacles[t].start(transform, &owner, world);
        true
    }

    // --- stages and states -------------------------------------------

    fn change_stage(&mut self, stage: Stage, world: &mut dyn CthulhuWorld) {
        self.stage = stage;
        self.enter_stage(world);
    }

    fn enter_stage(&mut self, world: &mut dyn CthulhuWorld) {
        let cold = index_of_element(E::COLD);
        let (speed, between, tentacles, cold_multiplier) = match self.stage {
            Stage::Intro => (1.0, 1.5, 2, 0.9),
            Stage::Battle => (1.0, 1.0, 2, 0.7),
            Stage::LateBattle => (1.0, 1.0, 4, 0.5),
            Stage::Critical => (1.2, 0.75, 4, 0.25),
            Stage::Final => (1.3, 0.5, 4, 0.25),
        };
        self.stage_speed = speed;
        self.time_between_actions = between;
        self.tentacle_count = tentacles;
        self.status.resistances[cold].multiplier = cold_multiplier;
        match self.stage {
            Stage::Intro => {
                for _ in 0..self.tentacle_count {
                    self.spawn_new_tentacle(world);
                }
                self.s.rage_spawns = false;
            }
            Stage::Battle => {
                self.s.rage_spawns = false;
                self.change(State::Rage, world);
            }
            _ => {
                self.s.rage_spawns = true;
                self.change(State::Rage, world);
            }
        }
    }

    fn change(&mut self, state: State, world: &mut dyn CthulhuWorld) {
        self.exit(world);
        self.previous = self.state;
        self.state = state;
        self.enter(world);
    }

    fn enter(&mut self, world: &mut dyn CthulhuWorld) {
        use clip as c;
        match self.state {
            State::Idle => {
                let fade = if self.previous == State::Devour {
                    0.4
                } else {
                    0.05
                };
                self.rig.crossfade(c::IDLE, fade, true);
                self.s.idle_timer = self.time_between_actions;
                self.s.idle_done = false;
            }
            State::Mist => {
                self.rig.crossfade(c::MISTS, 0.4, false);
                self.s.mist_casted = false;
                self.cue(world, "cthulhu_mist");
            }
            State::Emerge => {
                self.s.emerge_effect = false;
                self.s.emerge_logic = false;
                self.hp_on_last_emerge = self.status.hit_points;
                self.time_since_last_emerge = 0.0;
                self.time_until_submerge = 30.0 + self.random() * 5.0;
                if let Some(point) = self.desired_spawn {
                    self.transform = self.change_spawn_point(point, self.zone);
                }
                self.status
                    .add_status(magicka_sim::status::StatusEffect::new(
                        Status::Wet,
                        0.0,
                        1.0,
                        false,
                    ));
                self.s.emerge_unfreeze = false;
                self.rig.start(c::EMERGE, false);
                if self.initial_emerge || self.king_fallen {
                    self.s.emerge_bubbles = true;
                    self.s.emerge_bubble_timer = -1.0;
                } else {
                    self.s.emerge_bubbles = false;
                    self.s.emerge_bubble_timer = 1.5;
                    self.rig.stop();
                }
                self.cue(world, "cthulhu_emerge");
            }
            State::Submerge => {
                self.rig.crossfade(c::SUBMERGE, 0.4, false);
                self.s.submerge_effect = false;
                self.s.submerge_logic = false;
                self.s.submerge_timer = 2.0;
                self.deflection = 0.0;
                self.cue(world, "cthulhu_submerge");
            }
            State::Timewarp => {
                self.cue(world, "cthulhu_howl");
                self.rig.crossfade(c::TIMEWARP, 0.4, false);
                self.s.timewarp_casted = false;
                self.s.timewarp_sound = false;
            }
            State::Lightning => {
                self.rig.crossfade(c::LIGHTNING, 0.4, false);
                self.s.lightning_hits.clear();
                self.s.lightning_timers = (0..12).map(|i| (i as f32 / 1000.0, i)).collect();
                self.s.lightning_timer = 0.0;
                self.s.lightning_cue = None;
            }
            State::Call => {
                self.rig.crossfade(c::CALL, 0.4, false);
                self.s.call_casted = false;
                self.cue(world, "cthulhu_call_of_cthulhu");
            }
            State::LesserCall => {
                self.rig.crossfade(c::CALL, 0.4, false);
                self.cue(world, "cthulhu_lesser_call_of_cthulhu");
                let n = self.lesser_spawns;
                let mut closed: Vec<usize> = Vec::with_capacity(n);
                for _ in 0..n {
                    loop {
                        let i = self.random_below(DEEP_ONE_LOCATORS.len());
                        if !closed.contains(&i) {
                            closed.push(i);
                            break;
                        }
                    }
                }
                self.s.lesser_closed = closed;
                let mut timers: Vec<(f32, usize)> = Vec::with_capacity(n);
                for l in 0..n {
                    let key = loop {
                        let k = self.random() + 0.15;
                        if !timers.iter().any(|t| t.0 == k) {
                            break k;
                        }
                    };
                    timers.push((key, l));
                }
                timers.sort_by(|a, b| a.0.total_cmp(&b.0));
                self.s.lesser_timers = timers;
                self.s.lesser_timer = 0.0;
            }
            State::Devour => {
                self.rig.crossfade(c::DEVOUR, 0.4, false);
                self.s.devour_sound = false;
                self.s.devour_effect = false;
            }
            State::DevourHit => {
                self.rig.crossfade(c::DEVOUR_HIT, 0.2, false);
                self.s.eat_sound = false;
                self.s.eat_effect = false;
                self.s.eat_terminated = self.character_to_eat.is_none();
                self.s.eat_scale = 1.0;
                if let Some((h, length)) = self.character_to_eat {
                    let p = world.entity(h).map(|e| e.position);
                    match p {
                        Some(p) => {
                            self.s.eat_offset_up = [0.0, length, 0.0];
                            self.s.eat_offset = m::sub(p, m::translation(&self.mouth));
                        }
                        None => self.s.eat_terminated = true,
                    }
                }
            }
            State::Hypnotize => {
                self.rig.crossfade(c::MESMERIZE, 0.4, false);
                self.s.hypnotize_casted = false;
                self.s.hypnotize_effect = false;
                self.cue(world, "cthulhu_hypnotize");
            }
            State::Rage => {
                self.rig.crossfade(c::RAGE, 0.5, false);
                self.s.rage_called = false;
                self.s.rage_rumble = false;
                self.cue(world, "cthulhu_rage");
            }
            State::OtherworldlyBolt => {
                self.s.bolts_to_cast = self.player_count;
                self.cue(world, "cthulhu_howl");
                self.s.bolts_casted = 0;
                self.s.bolt_casted = false;
                self.rig.crossfade(c::BOLT_START, 0.2, false);
                self.s.bolt_state = 0;
            }
            State::Death => {
                let owner = self.owner();
                for &t in &self.active.clone() {
                    let handle = self.tentacles[t].handle;
                    if !self.tentacles[t].dead {
                        self.leave_spawn(handle);
                    }
                    self.tentacles[t].kill(&owner, world);
                }
                world.kill_bolts();
                if self.occupied[0] != Some(self.zone) {
                    self.desired_spawn = Some(0);
                    self.change(State::Submerge, world);
                    return;
                }
                self.rig.crossfade(c::DIE, 0.5, false);
                self.cue(world, "cthulhu_death");
                self.s.death_effect = false;
                world.trigger("bossdead");
            }
        }
    }

    fn exit(&mut self, world: &mut dyn CthulhuWorld) {
        match self.state {
            State::Mist => self.s.mist_cooldown = 10.0,
            State::Emerge => {
                self.initial_emerge = false;
                world.track("cthulhu_bubbles", "", None);
            }
            State::Submerge => self.status.statuses = Default::default(),
            State::Timewarp => self.s.timewarp_cooldown = 20.0,
            State::Lightning => {
                if let Some(k) = self.s.lightning_cue.take() {
                    world.stop_cue(k);
                }
                self.s.lightning_cooldown = 10.0;
            }
            State::LesserCall => self.s.lesser_cooldown = 25.0,
            State::Devour => {
                world.track("cthulhu_pull", "", None);
                if let Some(k) = self.s.devour_suck.take() {
                    world.stop_cue(k);
                }
            }
            State::DevourHit => world.track("cthulhu_is_eating", "", None),
            State::Hypnotize => {
                world.track("cthulhu_hypnotize", "", None);
                self.s.hypnotize_cooldown = 5.0;
            }
            State::OtherworldlyBolt => self.s.bolt_cooldown = 5.0,
            State::Death => {
                world.track("cthulhu_idle", "", None);
                self.dead = true;
            }
            _ => {}
        }
    }

    /// The stage's NonActiveUpdate pass over every state.
    fn non_active_update(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        if self.s.mist_cooldown >= 0.0 {
            self.s.mist_cooldown -= dt;
        }
        if world.time_warp_active() && self.s.timewarp_cooldown >= 0.0 {
            self.s.timewarp_cooldown -= dt;
        }
        if self.s.lightning_cooldown >= 0.0 {
            self.s.lightning_cooldown -= dt;
        }
        if self.s.lesser_cooldown >= 0.0 {
            self.s.lesser_cooldown -= dt;
        }
        if self.s.hypnotize_cooldown >= 0.0 {
            self.s.hypnotize_cooldown -= dt;
        }
        if self.s.bolt_cooldown >= 0.0 {
            self.s.bolt_cooldown -= dt;
        }
    }

    // --- Active checks -----------------------------------------------

    fn mist_active(&self) -> bool {
        self.s.mist_cooldown <= 0.0 && self.activate_mist && !self.mist.active
    }
    fn submerge_active(&self) -> bool {
        self.hp_on_last_emerge - self.status.hit_points > 0.1 * MAX_HIT_POINTS
            || self.time_until_submerge < 0.0
            || (self.time_since_last_emerge > 5.0 && self.seconds_out_of_range > 1)
    }
    fn timewarp_active(&self, world: &mut dyn CthulhuWorld) -> bool {
        self.state != State::Timewarp
            && self.s.timewarp_cooldown <= 0.0
            && !world.time_warp_active()
            && self.status.time_since_last_damage > 2.0
    }
    /// LightningState.ShouldDoLightning(19, 5).
    fn lightning_active(&self, world: &mut dyn CthulhuWorld) -> bool {
        if self.s.lightning_cooldown >= 0.0 {
            return false;
        }
        let at = self.position();
        let forward = m::forward(&self.transform);
        let tentacles: Vec<u16> = self
            .active
            .iter()
            .map(|&t| self.tentacles[t].handle)
            .collect();
        let mut sum = 0.0;
        for e in world.entities(at, 19.0, true, false) {
            if e.handle == self.zone || tentacles.contains(&e.handle) {
                continue;
            }
            if !matches!(e.kind, Kind::Avatar | Kind::Npc) || e.ethereal {
                continue;
            }
            if self.s.lightning_hits.contains(e.handle) {
                continue;
            }
            let d = flat(m::sub(e.position, at));
            if m::length_squared(d) > 25.0 && m::dot(m::normalize(d), forward) > 0.7 {
                if e.friendly {
                    sum += e.lightning_resistance;
                } else {
                    sum -= e.lightning_resistance;
                }
            }
        }
        sum > 0.5
    }
    fn call_active(&self, world: &mut dyn CthulhuWorld) -> bool {
        world.count(STARSPAWN) + world.count(DEEP_ONE) + world.count(CULTIST) == 0
    }
    fn lesser_call_active(&mut self, world: &mut dyn CthulhuWorld) -> bool {
        if self.s.lesser_cooldown >= 0.0 {
            return false;
        }
        if world.count(DEEP_ONE) + world.count(STARSPAWN) == 0 {
            return self.random() < 0.8;
        }
        false
    }
    /// DevourState.FindTarget(23, 2).
    fn devour_active(&self, world: &mut dyn CthulhuWorld) -> bool {
        let at = self.position();
        let forward = m::forward(&self.transform);
        for e in world.entities(at, 23.0, true, false) {
            if e.kind == Kind::Avatar && !e.ethereal {
                let d = flat(m::sub(e.position, at));
                if m::length_squared(d) > 4.0 && m::dot(m::normalize(d), forward) > 0.7 {
                    return true;
                }
            }
        }
        false
    }
    fn hypnotize_active(&self) -> bool {
        if self.player_count <= 1 || self.s.hypnotize_cooldown >= 0.0 {
            return false;
        }
        let mut living = 0;
        for p in &self.players {
            if p.playing && p.avatar.is_some() {
                if p.hypnotized {
                    return false;
                }
                if !p.dead {
                    living += 1;
                }
            }
        }
        living >= 2
    }
    fn bolt_active(&self, world: &mut dyn CthulhuWorld) -> bool {
        self.s.bolt_cooldown <= 0.0
            && world.count(CULTIST) + world.count(DEEP_ONE) <= self.player_count
    }

    /// The stage OnUpdate: NonActiveUpdate, then the next action when
    /// idle and done.
    fn update_stage(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        self.non_active_update(dt, world);
        if !(self.state == State::Idle && self.s.idle_done) {
            return;
        }
        use State as S;
        let next = match self.stage {
            Stage::Intro => {
                if self.submerge_active() {
                    Some(S::Submerge)
                } else if self.lightning_active(world) {
                    Some(S::Lightning)
                } else if self.devour_active(world) {
                    Some(S::Devour)
                } else {
                    None
                }
            }
            Stage::Battle | Stage::LateBattle => {
                if self.mist_active() {
                    Some(S::Mist)
                } else if self.submerge_active() {
                    Some(S::Submerge)
                } else if self.lesser_call_active(world) {
                    Some(S::LesserCall)
                } else if self.lightning_active(world) {
                    Some(S::Lightning)
                } else if self.hypnotize_active() {
                    Some(S::Hypnotize)
                } else if self.bolt_active(world) {
                    Some(S::OtherworldlyBolt)
                } else if self.devour_active(world) {
                    Some(S::Devour)
                } else {
                    None
                }
            }
            Stage::Critical => {
                if !self.star_spawn_spawned && self.call_active(world) {
                    self.star_spawn_spawned = true;
                    Some(S::Call)
                } else if self.timewarp_active(world) {
                    Some(S::Timewarp)
                } else if self.mist_active() {
                    Some(S::Mist)
                } else if self.submerge_active() {
                    Some(S::Submerge)
                } else if self.lesser_call_active(world) {
                    Some(S::LesserCall)
                } else if self.lightning_active(world) {
                    Some(S::Lightning)
                } else if self.hypnotize_active() {
                    Some(S::Hypnotize)
                } else if self.bolt_active(world) {
                    Some(S::OtherworldlyBolt)
                } else if self.devour_active(world) {
                    Some(S::Devour)
                } else {
                    None
                }
            }
            Stage::Final => {
                if !self.star_spawn_spawned && self.call_active(world) {
                    self.star_spawn_spawned = true;
                    Some(S::Call)
                } else if self.mist_active() {
                    Some(S::Mist)
                } else if self.timewarp_active(world) {
                    Some(S::Timewarp)
                } else if self.lesser_call_active(world) {
                    Some(S::LesserCall)
                } else if self.submerge_active() {
                    Some(S::Submerge)
                } else if self.lightning_active(world) {
                    Some(S::Lightning)
                } else if self.hypnotize_active() {
                    Some(S::Hypnotize)
                } else if self.bolt_active(world) {
                    Some(S::OtherworldlyBolt)
                } else if self.devour_active(world) {
                    Some(S::Devour)
                } else {
                    None
                }
            }
        };
        // Native ordering puts Call first in Critical, and Emerge (always
        // false) before Timewarp; Call sets mStarSpawnSpawned only when
        // chosen, which `star_spawn_spawned` mirrors above.
        if let Some(state) = next {
            // Call's flag is set when chosen; undo for non-Call choices.
            self.change(state, world);
        }
    }

    fn update_state(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        use State as S;
        match self.state {
            S::Idle => {
                self.s.idle_timer -= dt;
                if self.s.idle_timer < 0.0 && self.ok_to_fight {
                    self.s.idle_done = true;
                }
            }
            S::Mist => {
                if self.rig.fading() {
                    return;
                }
                let num = self.rig.normalized();
                if self.rig.finished() {
                    self.change(S::Idle, world);
                } else if !self.s.mist_casted && num >= 0.263_157_9 {
                    self.activate_mist_cloud(world);
                    self.s.mist_casted = true;
                }
            }
            S::Emerge => self.update_emerge(dt, world),
            S::Submerge => self.update_submerge(dt, world),
            S::Timewarp => {
                if self.rig.fading() {
                    return;
                }
                let num = self.rig.normalized();
                if self.rig.finished() {
                    self.change(S::Idle, world);
                } else if !self.s.timewarp_casted && num >= 13.0 / 33.0 {
                    if !self.s.timewarp_sound {
                        world.cue(SPELLS, "magick_timewarp", self.zone_origin);
                        self.s.timewarp_sound = true;
                    }
                    world.time_warp(self.zone);
                    self.s.timewarp_casted = true;
                }
            }
            S::Lightning => self.update_lightning(dt, world),
            S::Call => {
                if self.rig.fading() {
                    return;
                }
                let num = self.rig.normalized();
                if self.rig.finished() {
                    self.change(S::Idle, world);
                } else if !self.s.call_casted && num >= 0.486_486_5 {
                    // One spawn; the closed list only guards repeats.
                    let i = self.random_below(DEEP_ONE_LOCATORS.len());
                    let point = world.nav_nearest(m::translation(&self.deep_one_transforms[i]));
                    world.spawn_npc(
                        STARSPAWN,
                        point,
                        skinned::IDENTITY,
                        m::forward(&self.transform),
                        "spawn",
                    );
                    self.s.call_casted = true;
                }
            }
            S::LesserCall => {
                if self.rig.fading() {
                    return;
                }
                let num = self.rig.normalized();
                if self.rig.finished() {
                    self.change(S::Idle, world);
                } else if num >= 0.486_486_5 {
                    self.s.lesser_timer -= dt;
                    while let Some(&(key, value)) = self.s.lesser_timers.first() {
                        if key + self.s.lesser_timer >= 0.0 {
                            break;
                        }
                        self.s.lesser_timers.remove(0);
                        let locator = self.deep_one_transforms[self.s.lesser_closed[value]];
                        let point = world.nav_nearest(m::translation(&locator));
                        let mut orientation = locator;
                        m::set_translation(&mut orientation, [0.0; 3]);
                        world.spawn_npc(
                            DEEP_ONE,
                            point,
                            orientation,
                            m::forward(&self.transform),
                            "special0",
                        );
                    }
                }
            }
            S::Devour => self.update_devour(dt, world),
            S::DevourHit => self.update_devour_hit(dt, world),
            S::Hypnotize => self.update_hypnotize(world),
            S::Rage => {
                if self.rig.fading() {
                    return;
                }
                if self.rig.finished() {
                    self.change(S::Idle, world);
                }
                let num = self.rig.normalized();
                if num >= 0.5 && !self.s.rage_called {
                    self.s.rage_called = true;
                    if self.s.rage_spawns {
                        let missing = self.tentacle_count.saturating_sub(self.active.len());
                        for _ in 0..missing {
                            self.spawn_new_tentacle(world);
                        }
                        self.s.rage_spawns = false;
                    }
                }
                if num >= 0.25 && !self.s.rage_rumble {
                    self.s.rage_rumble = true;
                    world.camera_shake(self.position(), 0.5, 2.5);
                }
            }
            S::OtherworldlyBolt => self.update_bolt(world),
            S::Death => {
                if self.rig.fading() {
                    return;
                }
                if self.rig.finished() {
                    world.track("cthulhu_idle", "", None);
                    self.dead = true;
                } else if !self.s.death_effect {
                    self.s.death_effect = true;
                }
                if self.s.death_effect {
                    world.track("cthulhu_death", "cthulhu_death", Some(self.transform));
                }
            }
        }
    }

    fn water_point(&self, y: f32) -> Mat {
        let mut p = self.position();
        p[1] = y;
        translation_matrix(p)
    }

    fn update_emerge(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        if self.s.emerge_bubble_timer >= 0.0 {
            self.s.emerge_bubble_timer -= dt;
            if self.s.emerge_bubble_timer <= 0.0 {
                self.rig.start(clip::EMERGE, false);
                world.track("cthulhu_bubbles", "", None);
            } else if !self.s.emerge_bubbles {
                world.track(
                    "cthulhu_bubbles",
                    "cthulhu_intro_bubbles",
                    Some(self.water_point(self.water_y)),
                );
                self.s.emerge_bubbles = true;
            }
            return;
        }
        let num = self.rig.normalized();
        if self.king_fallen && num >= 0.42 {
            self.change(State::Death, world);
            return;
        }
        if self.rig.finished() {
            self.change(State::Idle, world);
            return;
        }
        if !self.s.emerge_unfreeze && num >= 4.0 / 55.0 {
            let fire = dmg(attack::STATUS, E::FIRE, 100.0, 4.0);
            world.freeze(self.position(), [4.0, 0.0, 0.0], fire);
            self.s.emerge_unfreeze = true;
        }
        if !self.s.emerge_logic && num >= 12.0 / 55.0 {
            let at = self.zone_origin;
            let knock = dmg(attack::KNOCKBACK, E::EARTH, 100.0, 4.0);
            world.circle_damage(self.zone, at, 6.0, knock);
            let soak = dmg(attack::STATUS, E::WATER, 100.0, 1.0);
            world.circle_damage(self.zone, at, 10.0, soak);
            self.s.emerge_logic = true;
        } else if !self.s.emerge_effect && num >= 0.0 {
            world.effect("cthulhu_emerge_water_splash", self.water_point(0.0));
            self.s.emerge_effect = true;
            world.track(
                "cthulhu_idle",
                "cthulhu_water_surface",
                Some(self.water_point(self.water_y)),
            );
        }
    }

    fn update_submerge(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        if self.rig.fading() {
            return;
        }
        let num = self.rig.normalized();
        if self.rig.finished() {
            if self.s.submerge_timer <= 0.0 {
                let mut point = self.desired_spawn.take();
                if point.is_none() {
                    point = self.find_good_spot(18.0, world);
                }
                if point.is_none() {
                    point = self.find_random_spot();
                }
                if let Some(p) = point {
                    self.transform = self.change_spawn_point(p, self.zone);
                }
                self.change(State::Emerge, world);
            }
            self.s.submerge_timer -= dt;
        } else if !self.s.submerge_logic && num >= 0.697_674_4 {
            let at = self.position();
            let push = dmg(attack::PUSHED, E::EARTH, 100.0, 4.0);
            world.circle_damage(self.zone, at, 6.0, push);
            let soak = dmg(attack::STATUS, E::WATER, 100.0, 1.0);
            world.circle_damage(self.zone, at, 6.0, soak);
            self.s.submerge_logic = true;
        } else if !self.s.submerge_effect && num >= 0.581_395_3 {
            world.effect("cthulhu_emerge_water_splash", self.water_point(0.0));
            world.track("cthulhu_idle", "", None);
            self.s.submerge_effect = true;
        }
    }

    fn update_lightning(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        if self.rig.fading() {
            return;
        }
        self.s.lightning_hits.update(dt);
        let num = self.rig.normalized();
        if self.rig.finished() {
            self.change(State::Idle, world);
            return;
        }
        if (23.0 / 86.0..=0.837_209_3).contains(&num) {
            if self.s.lightning_cue.is_none() {
                self.s.lightning_cue =
                    Some(world.cue(SPELLS, "spell_lightning_spray", self.zone_origin));
            }
            self.s.lightning_timer -= dt;
            let forward = m::forward(&self.transform);
            let r = right(&self.transform);
            let bolt = dmg(attack::DAMAGE, E::LIGHTNING, 8.0, 1.0);
            while let Some(&(key, value)) = self.s.lightning_timers.first() {
                if key + self.s.lightning_timer >= 0.0 {
                    break;
                }
                self.s.lightning_timers.remove(0);
                let finger = if value < 6 {
                    self.left_fingers[value]
                } else {
                    self.right_fingers[value - 6]
                };
                let from = m::add(m::translation(&finger), forward);
                let ttl = self.random() / 4.0 + 0.15;
                self.s.lightning_hits.add(self.zone);
                for &t in &self.active {
                    self.s.lightning_hits.add(self.tentacles[t].handle);
                }
                // FindTarget(6, 0): the first avatar in front of the finger.
                let mut target = None;
                for e in world.entities(from, 6.0, true, false) {
                    if e.kind == Kind::Avatar
                        && !e.ethereal
                        && !self.s.lightning_hits.contains(e.handle)
                    {
                        let d = flat(m::sub(e.position, from));
                        if m::length_squared(d) > 0.0 && m::dot(m::normalize(d), forward) > 0.7 {
                            target = Some(e.handle);
                            break;
                        }
                    }
                }
                let direction = if target.is_none() {
                    let spread = 1.0 - 2.0 * self.random();
                    m::normalize(m::add(forward, m::scale(r, 0.7 * spread)))
                } else {
                    forward
                };
                let range = 6.0 + self.random() * 6.0;
                let mut hits = std::mem::take(&mut self.s.lightning_hits);
                world.lightning(self.zone, from, target, direction, range, bolt, &mut hits);
                self.s.lightning_hits = hits;
                let mut key = ttl - self.s.lightning_timer;
                while self.s.lightning_timers.iter().any(|t| t.0 == key) {
                    key += 1e-6;
                }
                let at = self
                    .s
                    .lightning_timers
                    .iter()
                    .position(|t| t.0 > key)
                    .unwrap_or(self.s.lightning_timers.len());
                self.s.lightning_timers.insert(at, (key, value));
            }
        } else if num >= 0.837_209_3 {
            if let Some(k) = self.s.lightning_cue.take() {
                world.stop_cue(k);
            }
        }
    }

    fn update_devour(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        if self.rig.fading() {
            return;
        }
        let num = self.rig.normalized();
        if self.rig.finished() {
            self.change(State::Idle, world);
            return;
        }
        if (0.08..=0.9).contains(&num) {
            if !self.s.devour_sound {
                self.cue(world, "cthulhu_devour");
                self.s.devour_suck = Some(self.cue(world, "cthulhu_devour_suck"));
                self.s.devour_sound = true;
            }
            if self.custom_vortex(dt, world) {
                return;
            }
        }
        if num >= 0.08 && !self.s.devour_effect {
            self.s.devour_effect = true;
        }
        if num > 0.9 {
            world.track("cthulhu_pull", "", None);
        } else if self.s.devour_effect {
            world.track("cthulhu_pull", "cthulhu_pull", Some(self.mouth));
        }
    }

    /// DevourState.CustomVortex: true when the state changed.
    fn custom_vortex(&mut self, dt: f32, world: &mut dyn CthulhuWorld) -> bool {
        let forward = m::forward(&self.transform);
        let center = m::add(m::translation(&self.mouth), m::scale(forward, -0.8));
        let mouth_forward = m::forward(&self.mouth);
        for e in world.entities(center, 30.0, true, false) {
            if e.handle == self.zone {
                continue;
            }
            let character = matches!(e.kind, Kind::Avatar | Kind::Npc);
            if character {
                if e.ethereal {
                    continue;
                }
                if e.gripped {
                    world.release_grip(e.handle);
                }
                if e.starspawn {
                    continue;
                }
            } else if e.kind == Kind::Shield {
                let nearest = world.shield_nearest(e.handle, center);
                let d = flat(m::sub(center, nearest));
                if m::length_squared(d) <= 1e-6 {
                    continue;
                }
                if m::length(d) < 1.8 {
                    self.change(State::Idle, world);
                    return true;
                }
                continue;
            } else if e.kind != Kind::Missile {
                continue;
            }
            let to_mouth = flat(m::sub(center, e.position));
            if m::length_squared(to_mouth) <= 1e-6 {
                continue;
            }
            let distance = m::length(to_mouth);
            let eaten = if e.kind == Kind::Missile && distance < 1.25 {
                world.kill(e.handle);
                true
            } else {
                distance < 0.4
            };
            if eaten {
                // DIVERGENCE: a swallowed missile clears mCharacterToEat
                // (native keeps the previous victim and would move it).
                self.character_to_eat = character.then_some((e.handle, e.length));
                self.change(State::DevourHit, world);
                return true;
            }
            let pull = 7.5 * (1.0 - distance / 30.0);
            if pull <= 0.0 {
                continue;
            }
            let direction = m::normalize(to_mouth);
            if m::dot(direction, mouth_forward) < 0.7 {
                continue;
            }
            let force = m::scale(direction, pull);
            if e.kind == Kind::Missile {
                world.missile_impulse(e.handle, m::scale(force, 100.0), dt);
            } else {
                world.additional_force(e.handle, force);
            }
        }
        false
    }

    fn update_devour_hit(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        if !self.s.eat_terminated {
            if let Some((h, _)) = self.character_to_eat {
                self.s.eat_scale -= dt;
                let k = (self.s.eat_scale - 2.0 / 3.0) * 3.0;
                let mut p = m::add(m::translation(&self.mouth), m::scale(self.s.eat_offset, k));
                p = m::add(
                    p,
                    m::scale(self.s.eat_offset_up, (1.0 - self.s.eat_scale) * 2.0),
                );
                world.move_body(h, p);
                world.scale_model(h, self.s.eat_scale);
                if self.s.eat_scale <= 0.1 {
                    self.s.eat_terminated = true;
                    world.terminate(h);
                    world.scale_model(h, 1.0);
                }
            }
        }
        if self.rig.fading() {
            return;
        }
        let num = self.rig.normalized();
        if self.rig.finished() {
            self.change(State::Idle, world);
        } else if num >= 0.1 && !self.s.eat_effect {
            self.s.eat_effect = true;
        } else if num >= 0.08 && !self.s.eat_sound {
            self.cue(world, "cthulhu_devour_hit");
            self.s.eat_sound = true;
        }
        if self.s.eat_effect && self.state == State::DevourHit {
            world.track("cthulhu_is_eating", "cthulhu_is_eating", Some(self.mouth));
        }
    }

    fn update_hypnotize(&mut self, world: &mut dyn CthulhuWorld) {
        if self.rig.fading() {
            return;
        }
        if self.rig.finished() {
            self.change(State::Idle, world);
            return;
        }
        let num = self.rig.normalized();
        if num >= 29.0 / 48.0 && !self.s.hypnotize_casted {
            let source = self.position();
            if let Some((avatar, position)) = self.random_target(source, world) {
                let middle = m::translation(&self.mist_transform);
                let mut direction = flat(m::sub(position, middle));
                if m::length_squared(direction) < 1e-6 {
                    direction = flat(m::sub(self.position(), position));
                }
                if m::length_squared(direction) < 1e-6 {
                    return;
                }
                world.hypnotize(avatar, m::normalize(direction));
            }
            self.s.hypnotize_casted = true;
        } else if !self.s.hypnotize_effect && num >= 0.0 {
            self.s.hypnotize_effect = true;
        }
        if self.s.hypnotize_effect {
            world.track("cthulhu_hypnotize", "cthulhu_hypnotize", Some(self.mouth));
        }
    }

    /// GetRandomTarget(iIgnoreProtected false): a random living player not
    /// behind a shield.
    fn random_target(&mut self, source: V3, world: &mut dyn CthulhuWorld) -> Option<(u16, V3)> {
        // The original tracks tried slots in a bit mask of its four players.
        let mut tried = vec![false; self.players.len()];
        loop {
            let i = self.random_below(self.players.len());
            if tried.iter().all(|t| *t) {
                return None;
            }
            if tried[i] {
                continue;
            }
            tried[i] = true;
            let p = self.players[i];
            let Some(avatar) = p.avatar.filter(|_| p.playing && !p.dead) else {
                continue;
            };
            if world.shield_blocks(source, m::sub(p.position, source)) {
                continue;
            }
            return Some((avatar, p.position));
        }
    }

    fn update_bolt(&mut self, world: &mut dyn CthulhuWorld) {
        match self.s.bolt_state {
            0 => {
                if !self.rig.fading() && self.rig.finished() {
                    self.s.bolt_state = 1;
                    self.rig.crossfade(clip::BOLT_MID, 0.0, false);
                }
            }
            1 => {
                if self.rig.fading() {
                    return;
                }
                if self.rig.finished() {
                    if self.s.bolts_casted >= self.s.bolts_to_cast {
                        self.s.bolt_state = 2;
                        self.rig.crossfade(clip::BOLT_END, 0.0, false);
                    } else {
                        self.s.bolt_casted = false;
                        self.rig.crossfade(clip::BOLT_MID, 0.0, false);
                    }
                } else if !self.s.bolt_casted {
                    let num = self.rig.normalized();
                    if num >= 0.0 && self.s.bolt.is_none() {
                        let l = m::translation(&self.left_hand);
                        let r = m::translation(&self.right_hand);
                        let at = m::add(l, m::scale(m::sub(r, l), 0.5));
                        let speed = 4.0 * self.stage_speed;
                        self.s.bolt = Some(world.spawn_bolt(
                            self.zone,
                            at,
                            m::forward(&self.transform),
                            speed,
                        ));
                    } else if num >= 0.27 {
                        self.s.bolts_casted += 1;
                        if let Some(b) = self.s.bolt.take() {
                            world.bolt_hunt(b);
                        }
                        self.s.bolt_casted = true;
                    }
                }
            }
            _ => {
                if !self.rig.fading() && self.rig.finished() {
                    self.change(State::Idle, world);
                }
            }
        }
    }

    fn activate_mist_cloud(&mut self, world: &mut dyn CthulhuWorld) {
        if !self.mist.active {
            let at = m::translation(&self.mist_transform);
            self.mist.initialize(at, world);
        }
    }

    fn check_range(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        self.check_range_timer -= dt;
        if self.check_range_timer >= 0.0 {
            return;
        }
        let near = world
            .entities(self.position(), 21.0, true, false)
            .iter()
            .any(|e| e.kind == Kind::Avatar && !e.ethereal && !e.invisible);
        if near {
            self.seconds_out_of_range = 0;
        } else {
            self.seconds_out_of_range += 1;
        }
        self.check_range_timer = 1.0;
    }

    fn check_mist(&mut self, dt: f32, world: &mut dyn CthulhuWorld) {
        self.mist_timer -= dt;
        if self.mist_timer > 0.0 {
            return;
        }
        let at = m::translation(&self.mist_transform);
        let per = 0.5 / 2.5;
        let sum: f32 = world
            .entities(at, MIST_RADIUS, true, true)
            .iter()
            .filter(|e| e.kind == Kind::Avatar && !e.invisible)
            .map(|_| per)
            .sum();
        self.mist_desirability += sum;
        self.mist_desirability -= (per - sum) / 2.0;
        self.mist_desirability = self.mist_desirability.clamp(0.0, 1.0);
        if self.mist_desirability == 1.0 {
            self.activate_mist = true;
        } else if self.mist_desirability == 0.0 {
            self.activate_mist = false;
        }
        self.mist_timer += 0.5;
    }

    fn read_joints(&mut self) {
        let b = &self.binds;
        self.mouth = self.rig.joint(b.mouth.0, &b.mouth.1);
        self.head = self.rig.joint(b.head.0, &b.head.1);
        self.left_hand = self.rig.joint(b.left_hand.0, &b.left_hand.1);
        self.right_hand = self.rig.joint(b.right_hand.0, &b.right_hand.1);
        for i in 0..6 {
            self.left_fingers[i] = self.rig.joint(b.left_fingers[i].0, &b.left_fingers[i].1);
            self.right_fingers[i] = self.rig.joint(b.right_fingers[i].0, &b.right_fingers[i].1);
        }
        let spine0 = self.rig.point(b.spine[0].0, &b.spine[0].1);
        let spine4 = self.rig.point(b.spine[4].0, &b.spine[4].1);
        self.zone_origin = spine0;
        // CreateWorld(0, spine1 − spine5, Up): the capsule runs along
        // Backward, i.e. from spine1 toward spine5.
        self.zone_axis = m::normalize(m::sub(spine4, spine0));
        for (side, arm) in [(0, b.left_arm), (2, b.right_arm)] {
            for k in 0..2 {
                let a = self.rig.point(arm[k].0, &arm[k].1);
                let next = self.rig.point(arm[k + 1].0, &arm[k + 1].1);
                let (radius, length) = ARM_CAPSULES[k];
                let axis = m::normalize(m::sub(next, a));
                self.arms[side + k] = (a, m::add(a, m::scale(axis, length)), radius);
            }
        }
    }

    /// Damage to part 0 (the body zone) or part 5 (the mist).
    pub fn damage(
        &mut self,
        part: usize,
        d: Damage,
        position: V3,
        features: u8,
        world: &mut dyn CthulhuWorld,
    ) -> u16 {
        match part {
            0 => {
                if self.dead {
                    return R::DEFLECTED;
                }
                let mut events = Vec::new();
                let before = self.status.hit_points;
                let r = self
                    .status
                    .damage(d, self.dead, position, features, &mut events);
                if self.status.hit_points < before {
                    self.hit_flash = 0.5;
                }
                let notifier = self.notifier();
                for e in events {
                    world.boss_event(None, e, notifier);
                }
                r
            }
            5 => self.mist.damage(d, world),
            _ => 0,
        }
    }

    /// A tentacle's damage (by tentacle index).
    pub fn tentacle_damage(
        &mut self,
        index: usize,
        d: Damage,
        position: V3,
        world: &mut dyn CthulhuWorld,
    ) -> u16 {
        self.tentacles[index].damage(d, position, world)
    }

    /// OnCollision with a missile: the deflection aura for 8 s.
    pub fn missile_contact(&mut self) {
        self.deflection = 8.0;
    }

    /// The mist's collision callback for one overlapping damageable.
    pub fn mist_contact(
        &mut self,
        e: &Near,
        missile_fire: bool,
        immortal: bool,
        charmed: bool,
        world: &mut dyn CthulhuWorld,
    ) {
        self.mist.contact(e, missile_fire, immortal, charmed, world);
    }

    /// The cold colorize (Colorize) drawn by the viewer.
    pub fn cold_tint(&self) -> [f32; 4] {
        let w = (self.status.magnitude(Status::Cold) * 10.0).min(1.0);
        [COLD_COLOR[0], COLD_COLOR[1], COLD_COLOR[2], w]
    }

    /// UpdateBoss. `time_scale` is TimeModifier · TimeMultiplier.
    pub fn update(
        &mut self,
        dt: f32,
        time_scale: f32,
        players: Vec<Player>,
        world: &mut dyn CthulhuWorld,
    ) {
        let dt = dt / time_scale.max(f32::EPSILON);
        self.players = players;
        if self.status.hit_points <= 0.0 && !self.king_fallen {
            self.king_fallen = true;
            self.change(State::Death, world);
        }
        let mut events = Vec::new();
        self.status.update_damage(dt, self.dead, &mut events);
        let notifier = self.notifier();
        for e in events {
            world.boss_event(None, e, notifier);
        }
        for (amount, element) in self.status.update_statuses(dt, self.dead) {
            self.status.status_damage(amount, element);
        }
        self.time_until_submerge -= dt;
        self.time_since_last_emerge += dt;
        self.check_range(dt, world);
        self.check_mist(dt, world);
        if self.state != State::Death && self.state == State::Idle {
            let ratio = self.status.hit_points / MAX_HIT_POINTS;
            let target = if ratio > 0.95 {
                Stage::Intro
            } else if ratio >= 0.65 {
                Stage::Battle
            } else if ratio >= 0.35 {
                Stage::LateBattle
            } else if ratio > 0.15 {
                Stage::Critical
            } else {
                Stage::Final
            };
            if ratio > 0.0 && self.stage < target {
                self.change_stage(target, world);
            }
        }
        self.update_stage(dt, world);
        self.update_state(dt, world);
        let parent = self.parent();
        let slowdown = if self.status.has(Status::Cold) {
            self.status.statuses[magicka_sim::boss_status::status_index(Status::Cold)].slowdown()
        } else {
            1.0
        };
        self.rig.update(self.stage_speed * slowdown * dt, &parent);
        self.read_joints();
        self.hit_flash = (self.hit_flash - dt * 5.0).max(0.0);
        self.wet_specular(dt);
        let cold = self.status.has(Status::Cold);
        for (key, transform) in [
            ("body", self.transform),
            ("head", self.head),
            ("mouth", self.mouth),
            ("right", self.right_hand),
            ("left", self.left_hand),
        ] {
            world.track(
                &format!("cthulhu_cold_{key}"),
                "cthulhu_cold",
                cold.then_some(transform),
            );
        }
        if matches!(self.state, State::Submerge | State::Death) && self.deflection > 0.0 {
            self.deflection = self.deflection.min(0.2);
        }
        if self.deflection > 0.0 {
            self.deflection -= dt;
            world.track("cthulhu_deflect", "cthulhu_deflect", Some(self.transform));
            world.deflect(self.zone_origin, 5.0, dt);
        } else {
            world.track("cthulhu_deflect", "", None);
        }
        // The tentacles and the mist update as entities of their own.
        let owner = self.owner();
        for t in 0..self.tentacles.len() {
            if !self.tentacles[t].active {
                continue;
            }
            if let Some(Request::Respawn) = self.tentacles[t].update(dt, &owner, world) {
                self.spawn_tentacle(t, world);
            }
            if self.tentacles[t].state == TentacleState::Die && self.tentacles[t].dead {
                let h = self.tentacles[t].handle;
                self.leave_spawn(h);
            }
        }
        self.mist.update(dt, world);
    }

    fn wet_specular(&mut self, dt: f32) {
        if self.status.magnitude(Status::Wet) > 0.0 {
            if self.wetness < 1.0 {
                self.wetness += dt;
            } else {
                self.wetness = 1.0;
            }
        } else if self.wetness > 0.0 {
            self.wetness -= dt;
        } else {
            self.wetness = 0.0;
        }
    }
}
