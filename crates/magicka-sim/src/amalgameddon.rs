//! Amalgameddon (research/amalgameddon.md): two characters of one type are
//! lifted, pushed apart and smashed together; one absorbs the other.
use crate::impact_motion::Vector;

pub const TTL: f32 = 10.0;
pub const RANGE: f32 = 20.0;
/// Per-Update (not per-second) steps of the original.
pub const RISE_STEP: f32 = 0.015;
pub const RISE_UNTIL_Y: f32 = 5.0;
pub const PUSH_TIME: f32 = 0.2;
pub const PUSH_STEP: f32 = 0.25;
pub const SMASH_STEP: f32 = 0.6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Rising,
    PushApart,
    SmashTogether,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Amalgameddon {
    pub owner: u16,
    pub ttl: f32,
    pub phase: Phase,
    /// mFirstUnit / mSecondUnit; after rising, the first is the happy unit.
    pub first: u16,
    pub second: u16,
    pub timer: f32,
    /// mLifeAmount: the sorry unit's hit points when the push starts.
    pub life: f32,
}

/// What one Update did, for the host to apply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Set both positions (and stun both for 10 s while rising).
    Move {
        first: Vector,
        second: Vector,
        stun: bool,
    },
    /// The smash landed: overkill `sorry`, heal `happy` by `life`, gravity
    /// back on for happy and unstun it; the effect ends.
    Merge { happy: u16, sorry: u16, life: f32 },
}

impl Amalgameddon {
    pub fn new(owner: u16, first: u16, second: u16) -> Self {
        Self {
            owner,
            ttl: TTL,
            phase: Phase::Rising,
            first,
            second,
            timer: 0.0,
            life: 0.0,
        }
    }

    /// Amalgameddon.Update after the TTL decrement (the host restores both
    /// units' gravity when the TTL drops below 0). `positions` are the
    /// units' current positions and radii; `second_hp` the second unit's
    /// hit points.
    pub fn step(
        &mut self,
        dt: f32,
        first: (Vector, f32),
        second: (Vector, f32),
        second_hp: f32,
    ) -> Option<Step> {
        let (a, b) = (first.0, second.0);
        match self.phase {
            Phase::Rising => {
                let a = [a[0], a[1] + RISE_STEP, a[2]];
                let b = [b[0], b[1] + RISE_STEP, b[2]];
                if a[1] > RISE_UNTIL_Y {
                    self.phase = Phase::PushApart;
                    self.life = second_hp;
                    self.timer = PUSH_TIME;
                }
                Some(Step::Move {
                    first: a,
                    second: b,
                    stun: true,
                })
            }
            Phase::PushApart => {
                self.timer -= dt;
                let d = normalize(sub(a, b)).map(|v| v * PUSH_STEP);
                if self.timer < 0.0 {
                    self.phase = Phase::SmashTogether;
                }
                Some(Step::Move {
                    first: add(a, d),
                    second: sub(b, d),
                    stun: false,
                })
            }
            Phase::SmashTogether => {
                let d = normalize(sub(a, b)).map(|v| v * -SMASH_STEP);
                let (a, b) = (add(a, d), sub(b, d));
                let gap = sub(a, b);
                if (gap[0] * gap[0] + gap[1] * gap[1] + gap[2] * gap[2]).sqrt() < first.1 + second.1
                {
                    self.ttl = 0.0;
                    return Some(Step::Merge {
                        happy: self.first,
                        sorry: self.second,
                        life: self.life,
                    });
                }
                Some(Step::Move {
                    first: a,
                    second: b,
                    stun: false,
                })
            }
        }
    }
}

fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn normalize(v: Vector) -> Vector {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / l)
}
