//! Tornado magick: TornadoEntity's wandering, alpha, lifetime and hit
//! cooldowns. Navmesh paths, the ground probe and damage delivery are host
//! callbacks. Evidence: research/tornado.md (Tornado.cs, TornadoEntity.cs).
use crate::damage::{attack, Damage};
use crate::impact_motion::Vector;
use crate::spells::Elements;
use crate::spray_pulse::HitList;

pub const RADIUS: f32 = 2.5;
/// Capsule length; the capsule runs from the body position upwards.
pub const LENGTH: f32 = 6.0;
pub const TTL: f32 = 15.0;
/// Execute(owner): the spawn point is 4 m ahead of the caster.
pub const SPAWN_DISTANCE: f32 = 4.0;
/// TornadoEntity.InitializeCache(16): a ring of reused instances.
pub const POOL_SIZE: usize = 16;
/// mHitList.Add(handle, 1f) (SortedList.Add behind ContainsKey).
pub const HIT_COOLDOWN: f32 = 1.0;

/// mDamage: one Knockback Earth entry, 800 × 2.
pub fn damage() -> [Damage; 5] {
    let mut d = [Damage::default(); 5];
    d[0] = Damage {
        attack: attack::KNOCKBACK,
        element: Elements::EARTH.0,
        amount: 800.0,
        magnitude: 2.0,
    };
    d
}

#[derive(Clone, Debug)]
pub struct Tornado {
    pub owner: Option<u16>,
    pub position: Vector,
    /// mHeading: Forward of the spawn orientation, then the last move
    /// direction (three-dimensional, normalized).
    pub heading: Vector,
    pub ttl: f32,
    pub alpha: f32,
    /// PlayTime at Initialize, used as the damage timestamp.
    pub timestamp: f64,
    path: Vec<Vector>,
    target: Vector,
    pub hits: HitList,
    /// Rewrite enhancement: characters recently flung from the top, kept
    /// out of the lift for FLING_COOLDOWN.
    pub flung: HitList,
}

/// MagickaMath.Angle: acos(dot(v, UnitX)) clamped, negated for v.Y < 0.
fn angle(x: f32, y: f32) -> f32 {
    let a = x.clamp(-1.0, 1.0).acos();
    if y < 0.0 {
        -a
    } else {
        a
    }
}

fn normalize2(x: f32, y: f32) -> (f32, f32) {
    let inv = 1.0 / (x * x + y * y).sqrt();
    (x * inv, y * inv)
}

fn distance_squared(a: Vector, b: Vector) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

impl Tornado {
    /// TornadoEntity.Initialize at `position` with the orientation's Forward.
    pub fn new(position: Vector, heading: Vector, owner: Option<u16>, timestamp: f64) -> Self {
        Self {
            owner,
            position,
            heading,
            ttl: TTL,
            alpha: 0.0,
            timestamp,
            path: Vec::new(),
            target: position,
            hits: HitList::default(),
            flung: HitList::default(),
        }
    }

    pub fn dead(&self) -> bool {
        self.ttl < 0.0
    }

    /// TornadoEntity.Kill: one more second.
    pub fn kill(&mut self) {
        self.ttl = 1.0;
    }

    /// The vertical capsule segment (bottom, top) and radius.
    pub fn capsule(&self) -> (Vector, Vector, f32) {
        let p = self.position;
        (p, [p[0], p[1] + LENGTH, p[2]], RADIUS)
    }

    /// TornadoEntity.Update. `random` is RANDOM.NextDouble; `find_path`
    /// returns NavMesh.FindShortestPath nodes when it succeeds; `ground`
    /// is the scene hit Y of a 3 m downward probe from a point.
    pub fn update(
        &mut self,
        dt: f32,
        mut random: impl FnMut() -> f32,
        mut find_path: impl FnMut(Vector, Vector) -> Option<Vec<Vector>>,
        mut ground: impl FnMut(Vector) -> Option<f32>,
    ) {
        self.alpha = (TTL - self.ttl).min(0.5) * self.ttl.min(1.0) * 2.0;
        self.ttl -= dt;
        self.hits.update(dt);
        self.flung.update(dt);
        let mut start = self.position;
        if !self.path.is_empty() {
            self.target = self.path[0];
            let mut d = distance_squared(start, self.target);
            while !self.path.is_empty() && d <= 1.0 {
                self.target = self.path.remove(0);
                d = distance_squared(start, self.target);
            }
        } else {
            let (hx, hz) = normalize2(self.heading[0], self.heading[2]);
            let mut a = angle(hx, hz);
            a += (random() - 0.5) * std::f32::consts::FRAC_PI_2;
            let distance = 5.0 + random() * 10.0;
            self.target = [
                start[0] + a.cos() * distance,
                start[1],
                start[2] + a.sin() * distance,
            ];
            if let Some(mut nodes) = find_path(start, self.target) {
                // Native indexes mPath[0] after removing the start node; a
                // one-node path would throw there, so it keeps the random
                // target instead (documented divergence).
                if nodes.len() >= 2 {
                    nodes.remove(0);
                    self.target = nodes.remove(0);
                    self.path = nodes;
                }
            }
        }
        let to: Vector = std::array::from_fn(|i| self.target[i] - start[i]);
        let length = to.iter().map(|v| v * v).sum::<f32>().sqrt();
        let speed = (f64::from(self.ttl).sin() * 4.0).abs() as f32;
        // XNA Normalize of a zero vector is NaN; the rewrite holds still
        // instead of propagating NaN into the scene (documented divergence).
        if length > 0.0 {
            let v = to.map(|c| c / length);
            self.heading = v;
            start[0] += v[0] * speed * dt;
            start[2] += v[2] * speed * dt;
        }
        if let Some(y) = ground(start) {
            start[1] += (y - start[1]) * dt;
        }
        self.position = start;
    }

    /// OnCollision for a damageable whose handle is not on the hit list:
    /// the hit point RADIUS towards it, then the hit list entry. Returns
    /// None when it was hit within the last second.
    pub fn hit(&mut self, handle: u16, target_position: Vector) -> Option<Vector> {
        if self.hits.contains(handle) {
            return None;
        }
        let p = self.position;
        let d: Vector = std::array::from_fn(|i| target_position[i] - p[i]);
        let inv = 1.0 / d.iter().map(|v| v * v).sum::<f32>().sqrt();
        self.hits.add_for(handle, HIT_COOLDOWN);
        Some(std::array::from_fn(|i| p[i] + d[i] * inv * RADIUS))
    }
}

/// Distance test between the tornado's vertical capsule and another
/// vertical capsule (`bottom`..`top`, `radius`), or a sphere when equal.
pub fn touches(tornado: &Tornado, bottom: Vector, top: Vector, radius: f32) -> bool {
    let (b0, b1, r) = tornado.capsule();
    let horizontal = (bottom[0] - b0[0]).powi(2) + (bottom[2] - b0[2]).powi(2);
    let (lo, hi) = (bottom[1].min(top[1]), bottom[1].max(top[1]));
    let gap = (lo - b1[1]).max(b0[1] - hi).max(0.0);
    horizontal + gap * gap < (r + radius).powi(2)
}

/// Tornado.Execute(owner)'s spawn point before navmesh snapping. Forward
/// keeps the 4× scaled direction (the native code reuses the vector).
pub fn spawn(position: Vector, direction: Vector) -> (Vector, Vector) {
    let forward = direction.map(|v| v * SPAWN_DISTANCE);
    (std::array::from_fn(|i| position[i] + forward[i]), forward)
}

/// User-requested enhancement (2026-10-11, not native): the native entity
/// only knocks back what touches it. The rewrite adds a swirling suction
/// field, lifts characters up the funnel and flings them from the top.
pub mod field {
    use super::*;

    /// Suction reach from the axis (horizontal).
    pub const SUCTION_RADIUS: f32 = 9.0;
    /// Inward pull (velocity added per frame, AdditionalForce) at the axis.
    pub const PULL: f32 = 6.0;
    /// Counter-clockwise swirl speed at the axis.
    pub const SWIRL: f32 = 4.0;
    /// Net upward acceleration inside the funnel (rewrite enhancement).
    pub const LIFT: f32 = 22.0;
    /// Swirl speed while carried up.
    pub const CARRY_SWIRL: f32 = 7.0;
    /// Fling speed outward and upward at the top.
    pub const FLING_OUT: f32 = 14.0;
    pub const FLING_UP: f32 = 5.0;
    pub const FLING_COOLDOWN: f32 = 2.5;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Effect {
        /// Ground suction: an AdditionalForce for this frame.
        Pull(Vector),
        /// Inside the funnel: an impulse velocity for this frame.
        Lift(Vector),
        /// At the top: a one-off impulse velocity.
        Fling(Vector),
    }

    /// The field effect on a character centred at `position` with `radius`.
    pub fn effect(
        tornado: &Tornado,
        handle: u16,
        position: Vector,
        radius: f32,
        dt: f32,
    ) -> Option<Effect> {
        let axis = tornado.position;
        let (dx, dz) = (position[0] - axis[0], position[2] - axis[2]);
        let d = (dx * dx + dz * dz).sqrt();
        if d > SUCTION_RADIUS + radius {
            return None;
        }
        let (rx, rz) = if d > 1e-4 {
            (dx / d, dz / d)
        } else {
            (1.0, 0.0)
        };
        // Counter-clockwise tangent seen from above.
        let (tx, tz) = (-rz, rx);
        let height = position[1] - axis[1];
        let inside = d < RADIUS + radius && height < LENGTH;
        if inside && !tornado.flung.contains(handle) {
            if height >= LENGTH * 0.85 {
                return Some(Effect::Fling([rx * FLING_OUT, FLING_UP, rz * FLING_OUT]));
            }
            // Pulled toward the axis while spiralling upward.
            // The host integrates gravity separately. Compensate it so
            // LIFT describes the rise acceleration, not the gross force.
            let inward = (d - 0.5).max(0.0) * 2.0;
            return Some(Effect::Lift([
                (tx * CARRY_SWIRL - rx * inward) * dt,
                (LIFT - crate::character_physics::GRAVITY[1]) * dt,
                (tz * CARRY_SWIRL - rz * inward) * dt,
            ]));
        }
        if inside {
            return None;
        }
        let falloff = 1.0 - d / (SUCTION_RADIUS + radius);
        Some(Effect::Pull([
            (-rx * PULL + tx * SWIRL) * falloff,
            0.0,
            (-rz * PULL + tz * SWIRL) * falloff,
        ]))
    }
}
