//! Vortex magick: VortexEntity's mass, reach, pull and absorption rules.
//! Entity queries, killing and forces are host operations.
//! Evidence: research/vortex.md (Vortex.cs, VortexEntity.cs).
use crate::impact_motion::Vector;

/// Execute(owner): owner position + Direction · 10.
pub const SPAWN_DISTANCE: f32 = 10.0;
/// The absorbing core: BoundingSphere(position, 0.5).
pub const CORE_RADIUS: f32 = 0.5;
pub const INITIAL_MASS: f32 = 0.25;
/// mTargetMass decays by 0.05 per second.
pub const DECAY: f32 = 0.05;
/// Pull strength at the centre: 15 · (1 − d / radius).
pub const PULL: f32 = 15.0;
/// VortexEntity.InitializeCache(8): a ring of reused instances.
pub const POOL_SIZE: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct Vortex {
    pub owner: Option<u16>,
    pub center: Vector,
    pub mass: f32,
    pub target_mass: f32,
    pub radius: f32,
    /// mHawkingHoleCount: hostile characters absorbed (achievement only).
    pub hostile_absorbed: u32,
}

impl Vortex {
    pub fn new(owner: Option<u16>, center: Vector) -> Self {
        Self {
            owner,
            center,
            mass: INITIAL_MASS,
            target_mass: INITIAL_MASS,
            radius: 0.0,
            hostile_absorbed: 0,
        }
    }

    /// Execute(owner)'s centre.
    pub fn spawn_point(position: Vector, direction: Vector) -> Vector {
        std::array::from_fn(|i| position[i] + direction[i] * SPAWN_DISTANCE)
    }

    /// Dead/Removable: mMass < float.Epsilon.
    pub fn dead(&self) -> bool {
        self.mass < f32::from_bits(1)
    }

    /// Kill: the target mass drops to zero (the mass then eases out).
    pub fn kill(&mut self) {
        self.target_mass = 0.0;
    }

    /// The first part of Update: mass eases toward the target. Returns the
    /// query radius (10 · mass), or None below 1e-6 (Update returns early,
    /// skipping the target decay).
    pub fn begin_update(&mut self, dt: f32) -> Option<f32> {
        let t = 0.1f64.powf(f64::from(dt)) as f32;
        self.mass = self.target_mass + (self.mass - self.target_mass) * t;
        if self.mass < 1e-6 {
            return None;
        }
        self.radius = self.mass * 10.0;
        Some(self.radius)
    }

    /// Whether an entity's world bounding box touches the core sphere.
    pub fn core_touches(&self, min: Vector, max: Vector) -> bool {
        let d: f32 = (0..3)
            .map(|i| {
                let c = self.center[i].clamp(min[i], max[i]);
                (c - self.center[i]).powi(2)
            })
            .sum();
        d <= CORE_RADIUS * CORE_RADIUS
    }

    /// An absorbed entity that ended up Dead adds its body mass:
    /// target = 1 − 0.75^(log(1 − target)/log(0.75) + mass).
    pub fn absorb(&mut self, body_mass: f32) {
        let ln = 0.75f64.ln();
        let n = ((1.0 - f64::from(self.target_mass)).ln() / ln) as f32;
        self.target_mass = 1.0 - 0.75f64.powf(f64::from(n + body_mass)) as f32;
    }

    /// The pull toward the centre for an entity at `position` outside the
    /// core: AdditionalForce for characters, added velocity for bodies.
    pub fn pull(&self, position: Vector) -> Option<Vector> {
        let d: Vector = std::array::from_fn(|i| self.center[i] - position[i]);
        let distance = d.iter().map(|v| v * v).sum::<f32>().sqrt();
        if distance <= 1e-6 {
            return None;
        }
        let strength = PULL * (1.0 - distance / self.radius);
        (strength > 0.0).then(|| d.map(|v| v / distance * strength))
    }

    /// The tail of Update: the target mass decays.
    pub fn end_update(&mut self, dt: f32) {
        self.target_mass -= dt * DECAY;
    }

    /// The particle effect scale: radius / 10 · 2, at least 0.25.
    pub fn effect_scale(&self) -> f32 {
        (self.radius / 10.0 * 2.0).max(0.25)
    }
}
