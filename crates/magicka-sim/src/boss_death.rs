//! Numerical contracts of the Niflheim Death boss, distinct from SummonDeath.
//! Native provenance and remaining encounter work: research/boss-death.md.
use crate::{
    character_impact::result,
    damage::{attack, Damage},
    spells::Elements,
};

pub const MAX_HIT_POINTS: f32 = 10000.;

/// World-owned visibility and collision state at the instant Damage runs.
#[derive(Clone, Copy, Debug, Default)]
pub struct Vulnerability {
    pub drawn: bool,
    pub ethereal: bool,
    /// Only applies during MultiDeath, rather than an arbitrary clone render.
    pub fake_multi_attack: bool,
}

/// Host must start sound/effect and allocate an unlocked number on first_hit,
/// then add amount to that number if allocation succeeded. Native adds the
/// first amount twice to the displayed number, but once to health.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub amount: f32,
    pub first_hit: bool,
}

#[derive(Clone, Debug)]
pub struct DeathCombat {
    hit_points: f32,
    pub hit_pending: bool,
    pub flash_timer: f32,
    defeated: bool,
}

impl Default for DeathCombat {
    fn default() -> Self {
        Self {
            hit_points: MAX_HIT_POINTS,
            hit_pending: false,
            flash_timer: 0.,
            defeated: false,
        }
    }
}

impl DeathCombat {
    pub fn hit_points(&self) -> f32 {
        self.hit_points
    }

    /// Native mDead is initialized false and never assigned by local states.
    pub fn dead(&self) -> bool {
        false
    }

    pub fn defeated(&self) -> bool {
        self.defeated
    }

    /// Death.Damage, MethodDef 0x0600369c. Always returns Deflected, even when
    /// Life changes HP. Feature bits, magnitude and part index are not read.
    pub fn damage(
        &mut self,
        damage: Damage,
        visibility: Vulnerability,
        _features: u8,
    ) -> (u16, Option<Hit>) {
        if !visibility.drawn
            || visibility.fake_multi_attack
            || visibility.ethereal
            || damage.attack & attack::DAMAGE == 0
            || damage.element & Elements::LIFE.0 == 0
        {
            return (result::DEFLECTED, None);
        }
        let amount = damage.amount.abs();
        let hit = if amount >= 0. {
            let first_hit = !self.hit_pending;
            self.hit_pending = true;
            self.hit_points -= amount;
            self.flash_timer = 0.1;
            Some(Hit { amount, first_hit })
        } else {
            None
        };
        if self.hit_points < 1. {
            self.hit_points = 1.;
        }
        (result::DEFLECTED, hit)
    }

    /// UpdateBoss ages the flash with incoming dt, before removing world time
    /// scaling from the dt used by the animation/state update.
    pub fn age_flash(&mut self, dt: f32) {
        self.flash_timer = (self.flash_timer - dt).max(0.);
    }

    /// Host enters Hit on the next UpdateBoss, then calls this only on Hit's
    /// exit (finished animation and crossfade disabled).
    pub fn exit_hit(&mut self) {
        self.hit_pending = false;
        if self.hit_points < 1. {
            self.hit_points = 1.;
        }
    }

    /// UpdateBoss's once-only defeat edge precedes ordinary hit recovery.
    /// Host kills summons and enters DefeatPreSit, retaining HP=1 meanwhile.
    pub fn begin_defeat(&mut self) -> bool {
        if self.hit_points <= 1. && !self.defeated {
            self.defeated = true;
            true
        } else {
            false
        }
    }

    /// DefeatPreSit's completed clip enters Defeat. This is the scene's
    /// bosshealth<=0 gate; elapsed fight time never produces it.
    pub fn enter_defeat(&mut self) {
        self.hit_points = 0.;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ability {
    RaiseDead,
    MultiDeath,
    WarpTime,
    Move,
}

/// Prep's ordered branch selection. Random roll is supplied only when the
/// idle timer has expired and a target exists; native calls Next(10).
pub fn choose_ability(roll: u8, health: f32, previous: Option<Ability>, warp_ttl: f32) -> Ability {
    assert!(roll < 10);
    if roll < 3 && previous != Some(Ability::RaiseDead) {
        Ability::RaiseDead
    } else if roll < 6 && health / MAX_HIT_POINTS < 0.8 && previous != Some(Ability::MultiDeath) {
        Ability::MultiDeath
    } else if roll < 9
        && health / MAX_HIT_POINTS < 0.5
        && previous != Some(Ability::WarpTime)
        && warp_ttl <= -5.
    {
        Ability::WarpTime
    } else {
        Ability::Move
    }
}

/// Death.PrepState casts the random double expression to float before applying
/// the float health factor. The host owns shared RNG and when draws occur.
pub fn preparation_delay(health: f32, random_unit: f64) -> f32 {
    1. + 3. * (0.5 + random_unit * 0.5) as f32 * (health / MAX_HIT_POINTS)
}

pub fn placement_distance(health: f32, ability: Ability) -> f32 {
    let difficulty = match ability {
        Ability::RaiseDead | Ability::WarpTime => 1.,
        Ability::MultiDeath | Ability::Move => health / MAX_HIT_POINTS,
    };
    5. + (10. - 5.) * difficulty
}

pub fn movement_speed(health: f32) -> f32 {
    5. * (2. - health / MAX_HIT_POINTS)
}

pub fn clone_count(health: f32) -> usize {
    let fraction = health / MAX_HIT_POINTS;
    if fraction > 2. / 3. {
        8
    } else if fraction > 1. / 3. {
        12
    } else {
        16
    }
}

/// RaiseDead uses an unclamped Lerp with hp/max - 1, including its subtraction.
pub fn raise_animation_speed(health: f32) -> f32 {
    2. + (0.65 - 2.) * (health / MAX_HIT_POINTS - 1.)
}

/// RiseIntro's 0.15-second hold starts aging only after the summon_scythe
/// animation has finished and its crossfade is disabled.
#[derive(Clone, Debug)]
pub struct RiseIntroGate {
    remaining: f32,
}
impl Default for RiseIntroGate {
    fn default() -> Self {
        Self { remaining: 0.15 }
    }
}
impl RiseIntroGate {
    pub fn update(&mut self, dt: f32, finished: bool, crossfading: bool) -> bool {
        if finished && !crossfading {
            self.remaining -= dt;
        }
        self.remaining <= 0.
    }
}

/// RaiseDead attempts one spawn per update at most. It resets the timer even
/// when the authored four skeleton types already fill the six-entity cap.
#[derive(Clone, Debug)]
pub struct RaiseDeadPulse {
    remaining: f32,
}
impl Default for RaiseDeadPulse {
    fn default() -> Self {
        Self { remaining: 0.2 }
    }
}
impl RaiseDeadPulse {
    pub fn update(&mut self, dt: f32, present_skeletons: i32) -> bool {
        self.remaining -= dt;
        if self.remaining <= 0. {
            self.remaining = 0.2;
            present_skeletons < 6
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PlayerSlot {
    pub playing: bool,
    pub living_avatar: Option<u16>,
}

/// SelectTarget scans all four slots from a random start, and leaves mTarget
/// untouched when none qualifies. PlayerCount controls Next's bound, not the
/// scan length. The caller supplies a start produced using that bound.
/// Rewrite extension: larger lobbies pass more than four slots; all are
/// scanned the same way.
pub fn select_target(slots: &[PlayerSlot], start: usize, previous: Option<u16>) -> Option<u16> {
    let n = slots.len();
    assert!(start < n.max(1));
    (0..n)
        .find_map(|i| {
            let p = slots[(i + start) % n];
            p.playing.then_some(p.living_avatar).flatten()
        })
        .or(previous)
}

/// RaiseDead tries at most ten random area points against every entity center,
/// including dead entities. A tenth sample is rejected even if clear. Native
/// compares squared distance with 3, not a three-metre radius.
pub fn skeleton_position(
    entity_positions: &[[f32; 3]],
    mut random_point: impl FnMut() -> [f32; 3],
) -> Option<[f32; 3]> {
    for attempt in 1..=10 {
        let point = random_point();
        let nearest = entity_positions.iter().fold(f32::MAX, |nearest, p| {
            let delta: [f32; 3] = std::array::from_fn(|i| point[i] - p[i]);
            let squared = delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2];
            if squared.is_nan() || nearest.is_nan() {
                f32::NAN
            } else {
                nearest.min(squared)
            }
        });
        if attempt == 10 {
            return None;
        }
        if nearest.partial_cmp(&3.) != Some(std::cmp::Ordering::Less) {
            return Some(point);
        }
    }
    None
}
