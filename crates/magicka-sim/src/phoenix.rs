//! Summon Phoenix: the SummonPhoenix singleton's static state machine
//! (sFinished, sCastFail, sFailTTL) and its blast timing. Positions,
//! damage and revival are host operations. Evidence: research/phoenix.md.
use crate::damage::{attack, Damage};
use crate::impact_motion::Vector;
use crate::spells::Elements;

/// Helper.CircleDamage range at the phoenix.
pub const BLAST_RANGE: f32 = 6.0;
/// The blast origin is 0.5 m above the landing point.
pub const BLAST_HEIGHT: f32 = 0.5;
pub const FAIL_TTL: f32 = 1.5;
/// Model clip that times the blast (Models/Magicks/Phoenix).
pub const CLIP: &str = "descend";

/// sDamage: Fire Damage 400×1, Fire Status 200×5, Fire Knockdown 400×2.
pub fn damage() -> [Damage; 5] {
    let fire = Elements::FIRE.0;
    let mut d = [Damage::default(); 5];
    d[0] = Damage {
        attack: attack::DAMAGE,
        element: fire,
        amount: 400.0,
        magnitude: 1.0,
    };
    d[1] = Damage {
        attack: attack::STATUS,
        element: fire,
        amount: 200.0,
        magnitude: 5.0,
    };
    d[2] = Damage {
        attack: attack::KNOCKDOWN,
        element: fire,
        amount: 400.0,
        magnitude: 2.0,
    };
    d
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cast {
    /// Indoors: the fail cue; sFinished is cleared but nothing is added to
    /// the SpellManager, so a lone indoor cast leaves the singleton busy.
    Indoors,
    /// Outdoors while sFinished: the descent starts and the effect is added.
    Started,
    /// Outdoors while a previous cast is unfinished: nothing happens.
    Busy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhoenixEvent {
    /// The fail timer ran out: CameraShake(4, 0.2).
    FailShake,
    /// The descend clip finished: blast, shake and revival.
    Blast,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SummonPhoenix {
    pub finished: bool,
    pub cast_fail: bool,
    pub fail_ttl: f32,
    /// Whether the singleton is in the SpellManager's effect list.
    pub registered: bool,
    pub owner: Option<u16>,
    /// sMatrix translation (the landing point).
    pub point: Vector,
    /// sController time in the non-looping descend clip.
    pub clip_time: f32,
    pub timestamp: f64,
}

impl Default for SummonPhoenix {
    fn default() -> Self {
        Self {
            finished: true,
            cast_fail: false,
            fail_ttl: 0.0,
            registered: false,
            owner: None,
            point: [0.0; 3],
            clip_time: 0.0,
            timestamp: 0.0,
        }
    }
}

impl SummonPhoenix {
    /// Execute (both overloads) up to the landing point, which the host
    /// computes only for `Started`.
    pub fn cast(&mut self, indoors: bool, owner: Option<u16>, timestamp: f64) -> Cast {
        if indoors {
            self.cast_fail = true;
            self.finished = false;
            self.fail_ttl = FAIL_TTL;
            return Cast::Indoors;
        }
        if !self.finished {
            return Cast::Busy;
        }
        self.cast_fail = false;
        self.owner = owner;
        self.finished = false;
        self.clip_time = 0.0;
        self.timestamp = timestamp;
        self.registered = true;
        Cast::Started
    }

    /// The landing offset before navmesh snapping: owner casts spread
    /// ±2 m, position casts ±1.5 m (rolls are NextDouble, X then Z).
    pub fn offset(owner_cast: bool, x_roll: f32, z_roll: f32) -> Vector {
        let spread = if owner_cast { 4.0 } else { 3.0 };
        [(x_roll - 0.5) * spread, 0.0, (z_roll - 0.5) * spread]
    }

    /// SpellManager.Update for the registered singleton; afterwards the
    /// effect is removed when `finished` (IsDead).
    pub fn update(&mut self, dt: f32, clip_duration: f32) -> Option<PhoenixEvent> {
        if !self.registered {
            return None;
        }
        let event = if self.cast_fail {
            self.fail_ttl -= dt;
            if self.fail_ttl <= 0.0 {
                self.finished = true;
                Some(PhoenixEvent::FailShake)
            } else {
                None
            }
        } else {
            self.clip_time += dt;
            if self.clip_time >= clip_duration && !self.finished {
                self.finished = true;
                Some(PhoenixEvent::Blast)
            } else {
                None
            }
        };
        if self.finished {
            self.registered = false;
        }
        event
    }

    /// OnRemove (SpellManager.ClearEffects): sFinished = true.
    pub fn remove(&mut self) {
        if self.registered {
            self.registered = false;
            self.finished = true;
        }
    }

    /// The revival point offset for player slot `i`: CreateRotationY(i·π/2)
    /// Forward added to the landing point.
    pub fn revive_offset(slot: usize) -> Vector {
        let a = slot as f32 * std::f32::consts::TAU * 0.25;
        // XNA CreateRotationY Forward = (−sin a, 0, −cos a).
        [-a.sin(), 0.0, -a.cos()]
    }
}
