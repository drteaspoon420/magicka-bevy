//! Napalm (research/napalm.md): a smoke target, a 5 s wait, then a 2 s
//! airstrike whose six hits each lay a burning grease field and blast.
use crate::damage::{attack, Damage};
use crate::spells::Elements;

/// Napalm.NAPALM_WAIT_TIME, NAPALM_TIME and NAPALM_HITS.
pub const WAIT_TIME: f32 = 5.0;
pub const STRIKE_TIME: f32 = 2.0;
pub const HITS: u8 = 6;
/// The target distance ahead of the caster and the half track length.
pub const TARGET_DISTANCE: f32 = 15.0;
pub const TRACK_HALF: f32 = 120.0;
/// NapalmBlast's entity radius.
pub const BLAST_RADIUS: f32 = 5.0;

/// Napalm.sNapalmDamage: 2000 Fire damage, 500 Earth knockback (magnitude
/// 2) and 300 Fire status (magnitude 3).
pub fn damage() -> [Damage; 5] {
    let mut d = [Damage::default(); 5];
    d[0] = Damage {
        attack: attack::DAMAGE,
        element: Elements::FIRE.0,
        amount: 2000.0,
        magnitude: 1.0,
    };
    d[1] = Damage {
        attack: attack::KNOCKBACK,
        element: Elements::EARTH.0,
        amount: 500.0,
        magnitude: 2.0,
    };
    d[2] = Damage {
        attack: attack::STATUS,
        element: Elements::FIRE.0,
        amount: 300.0,
        magnitude: 3.0,
    };
    d
}

/// The singleton's state between Execute and IsDead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Napalm {
    pub owner: u16,
    /// mFocalPoint: the target and the track direction (the caster's facing
    /// turned 90° about Y).
    pub target: [f32; 3],
    pub forward: [f32; 3],
    pub timer: f32,
    pub airstrike: bool,
    pub hits: u8,
}

/// What one Napalm.Update asks the host to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NapalmEvent {
    /// The wait ended: camera shake (0.5, 2) and the plane cue.
    PlanePass,
    /// A hit (host only): probe 10 m above to 10 m below this point.
    Strike([f32; 3]),
}

/// Matrix.CreateRotationY(PI/2) applied to a direction.
pub fn track_direction(facing: [f32; 3]) -> [f32; 3] {
    // RotationY(θ): x' = x cosθ + z sinθ, z' = -x sinθ + z cosθ.
    [facing[2], facing[1], -facing[0]]
}

impl Napalm {
    /// Napalm.Execute while dead: the target 15 m ahead, the track turned
    /// 90°, the 5 s wait.
    pub fn new(owner: u16, position: [f32; 3], facing: [f32; 3]) -> Self {
        Self {
            owner,
            target: [
                position[0] + facing[0] * TARGET_DISTANCE,
                position[1] + facing[1] * TARGET_DISTANCE,
                position[2] + facing[2] * TARGET_DISTANCE,
            ],
            forward: track_direction(facing),
            timer: WAIT_TIME,
            airstrike: false,
            hits: 0,
        }
    }

    /// Napalm.Update. `host` is false on a network client (no hits). The
    /// airstrike lerps from target + forward·120 to target - forward·120
    /// over 2 s; from 70.5% (then every further 1.5%) one hit per frame
    /// probes the track 25% behind. Returns whether it is dead afterwards.
    pub fn update(&mut self, dt: f32, host: bool, events: &mut Vec<NapalmEvent>) -> bool {
        self.timer -= dt;
        if self.airstrike {
            let t = 1.0 - self.timer / STRIKE_TIME;
            let lag = 0.25;
            let first = 0.5 + lag - 0.045;
            let next = first + f32::from(self.hits) * 0.015;
            if host && t >= first && t >= next && self.hits < HITS {
                let s = t - lag;
                let start = self.forward.map(|v| v * TRACK_HALF);
                let p = [0, 1, 2].map(|i| {
                    let a = self.target[i] + start[i];
                    let b = self.target[i] - start[i];
                    a + (b - a) * s
                });
                events.push(NapalmEvent::Strike(p));
                self.hits += 1;
            }
            self.timer <= 0.0
        } else {
            if self.timer <= 0.0 {
                self.airstrike = true;
                self.timer = STRIKE_TIME;
                events.push(NapalmEvent::PlanePass);
            }
            false
        }
    }

    /// The plane's position along the track (render/shadow only).
    pub fn plane_position(&self) -> Option<[f32; 3]> {
        self.airstrike.then(|| {
            let t = 1.0 - self.timer / STRIKE_TIME;
            [0, 1, 2].map(|i| {
                let a = self.target[i] + self.forward[i] * TRACK_HALF;
                let b = self.target[i] - self.forward[i] * TRACK_HALF;
                a + (b - a) * t
            })
        })
    }
}
