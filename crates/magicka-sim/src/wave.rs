//! Wave magick (Wave + WaveEntity, research/wave.md): a line of six rock
//! pillars rising one after another in front of the caster, each a pooled
//! Solid Barrier that lifts grounded characters once per cast, plus a 1 s
//! forward distortion. The pillar entity itself is
//! `barrier::Barrier::initialize_wave`; probes, contacts and models live in
//! the host.
use crate::damage::{attack, calculate_damage, CastType, Damage, DamageError};
use crate::spells::{Elements as E, Spell, SpellType};

pub type Vector = [f32; 3];

/// Wave.TTL: the ability (distortion) lifetime.
pub const TTL: f32 = 1.0;
/// Wave.EFFECT_RANGE: the distortion travels time² · 20 m.
pub const EFFECT_RANGE: f32 = 20.0;
/// Wave.RADIUS: distance between pillars, passed to Initialize.
pub const RADIUS: f32 = 2.5;
/// Wave.RANGE: the chain range is RADIUS · RANGE.
pub const RANGE: f32 = 6.0;
/// Wave.RANGE_INBETWEEN: each next pillar stands this far along the facing.
pub const RANGE_INBETWEEN: f32 = 1.5;
/// OnCollision's mass limit for the upward impulse (Wave.WEIGHT_THRESHOLD).
pub const WEIGHT_THRESHOLD: f32 = 3000.0;
/// OnCollision's AddImpulseVelocity(0, 10, 0).
pub const LIFT: f32 = 10.0;
/// CreateEntity probe: 2 m above the caster's front, 4 m down.
pub const PROBE_RISE: f32 = 2.0;
pub const PROBE_DROP: f32 = 4.0;
/// WaveEntity.Initialize ends with mNextBarrierTTL = 0.1 (overwriting the
/// 0.075 it set earlier).
pub const NEXT_TTL: f32 = 0.1;
/// WaveEntity.InitializeCache(16) / Wave.InitializeCache(16).
pub const POOL_SIZE: usize = 16;

fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(v: Vector, s: f32) -> Vector {
    v.map(|x| x * s)
}

/// CreateEntity's spell: Earth | Steam with magnitudes 3 and 2.
pub fn spell() -> Spell {
    let mut s = Spell::default_for(E(E::EARTH.0 | E::STEAM.0));
    s.mag = [0.0; 11];
    for (i, e) in E::BY_INDEX.iter().enumerate() {
        if *e == E::EARTH {
            s.mag[i] = 3.0;
        } else if *e == E::STEAM {
            s.mag[i] = 2.0;
        }
    }
    s
}

/// iSpell.CalculateDamage(SpellType.Shield, CastType.Weapon): the
/// collection every pillar receives before its own overwrite.
pub fn cast_damage() -> Result<[Damage; 5], DamageError> {
    let mut out = [Damage::default(); 5];
    for (slot, d) in out.iter_mut().zip(calculate_damage(
        &spell(),
        SpellType::Shield,
        CastType::Weapon,
    )?) {
        *slot = d;
    }
    Ok(out)
}

/// WaveEntity.Initialize's mDamage overwrite: every slot Earth with amount
/// 0; slot A is Knockback | Damage at magnitude 1, slots B–E Knockback with
/// their incoming magnitudes kept.
pub fn pillar_damage(incoming: [Damage; 5]) -> [Damage; 5] {
    let mut out = incoming;
    for (i, d) in out.iter_mut().enumerate() {
        d.element = E::EARTH.0;
        d.amount = 0.0;
        d.attack = if i == 0 {
            attack::KNOCKBACK | attack::DAMAGE
        } else {
            attack::KNOCKBACK
        };
    }
    out[0].magnitude = 1.0;
    out
}

/// The first pillar of an owner cast (Wave.CreateEntity).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FirstPillar {
    /// Segment origin; the delta is (0, −PROBE_DROP, 0).
    pub probe_origin: Vector,
    /// Owner Direction (the chain advances along it).
    pub facing: Vector,
}

impl FirstPillar {
    /// The probe starts Radius in front of the owner, 2 m up.
    pub fn new(owner_position: Vector, owner_direction: Vector, owner_radius: f32) -> Self {
        let front = add(owner_position, scale(owner_direction, owner_radius));
        Self {
            probe_origin: [front[0], front[1] + PROBE_RISE, front[2]],
            facing: owner_direction,
        }
    }

    /// Initialize arguments for a probe hit: the pillar 3 m beyond the hit,
    /// facing sideways (Direction.Z, 0, −Direction.X), scale 1, range 15,
    /// next offset Direction · 1.5, identity rotation, spacing 2.5.
    pub fn placement(&self, hit: Vector) -> crate::barrier::BarrierPlacement {
        let d = self.facing;
        crate::barrier::BarrierPlacement {
            probe_origin: add(hit, scale(d, RANGE_INBETWEEN * 2.0)),
            direction: [d[2], 0.0, -d[0]],
            scale: 1.0,
            range: RADIUS * RANGE,
            next_direction: scale(d, RANGE_INBETWEEN),
            next_yaw: 0.0,
            distance: RADIUS,
        }
    }
}

/// The Wave ability effect (SpellManager): only the distortion post effect
/// and its audio emitter. Its hit list (characters lifted this cast) lives
/// with it, shared by every pillar.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveEffect {
    pub ttl: f32,
    pub time: f32,
    /// mPosition: the owner position pushed 3 m along the direction.
    pub position: Vector,
    pub start: Vector,
    pub direction: Vector,
    pub hit_list: Vec<u16>,
    pub timestamp: f64,
}

impl WaveEffect {
    /// Execute(ISpellCaster): position = owner + 3 · Direction.
    pub fn from_owner(position: Vector, direction: Vector, timestamp: f64) -> Self {
        Self {
            ttl: TTL,
            time: 0.0,
            position: add(position, scale(direction, 3.0)),
            start: position,
            direction,
            hit_list: Vec::new(),
            timestamp,
        }
    }

    /// Execute(Vector3 position): a random direction from two
    /// RANDOM.NextDouble() draws in [0, 1), so it only points into +X/+Z,
    /// and no pillars (CreateEntity is not called).
    pub fn from_position(position: Vector, random: [f64; 2], timestamp: f64) -> Self {
        let mut d = [random[0] as f32, 0.0, random[1] as f32];
        let l = (d[0] * d[0] + d[2] * d[2]).sqrt();
        // XNA Vector3.Normalize of zero yields NaN; the host never passes it.
        d = d.map(|v| v / l);
        Self {
            position: add(position, scale(d, 3.0)),
            ..Self::from_owner(position, d, timestamp)
        }
    }

    pub fn is_dead(&self) -> bool {
        self.ttl <= 0.0
    }

    /// Wave.Update's timers.
    pub fn update(&mut self, dt: f32) {
        self.ttl -= dt;
        self.time += dt * (1.0 / TTL);
    }

    pub fn in_hit_list(&self, handle: u16) -> bool {
        self.hit_list.contains(&handle)
    }

    /// RenderData: CreateWorld at position + direction · time² · 20 (up Y),
    /// the third row scaled by 1 + time; distortion = t²(1 − t) · 6.75
    /// (peaks at 1 when t = 2/3).
    pub fn distortion(&self) -> Distortion {
        let t = self.time;
        Distortion {
            center: add(self.position, scale(self.direction, t * t * EFFECT_RANGE)),
            forward_scale: 1.0 + t,
            time: t,
            strength: t * t * (1.0 - t) * 6.75,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Distortion {
    pub center: Vector,
    /// Matrix row 3 (M31..M33) multiplier.
    pub forward_scale: f32,
    pub time: f32,
    pub strength: f32,
}

/// The character facts WaveEntity.OnCollision reads.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ContactTarget {
    pub dead: bool,
    pub ethereal: bool,
    pub grounded: bool,
    pub levitating: bool,
    pub immortal: bool,
    pub in_event: bool,
    pub mass: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contact {
    /// No damage (dead, ethereal, airborne or already lifted this cast).
    None,
    /// Damage(mDamage) and hit-listing; `lift` adds the upward impulse.
    Hit { lift: bool },
}

/// WaveEntity.OnCollision against a character. Physical blocking is the
/// Barrier capsule's and unaffected.
pub fn contact(target: ContactTarget, listed: bool) -> Contact {
    if target.dead || target.ethereal || !target.grounded || listed {
        return Contact::None;
    }
    Contact::Hit {
        lift: !target.levitating
            && !target.immortal
            && !target.in_event
            && target.mass < WEIGHT_THRESHOLD,
    }
}
