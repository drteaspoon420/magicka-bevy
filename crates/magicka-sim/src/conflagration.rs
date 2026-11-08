//! Conflagration's retained sweep, from MethodDefs 0x06003c8d–0x06003c96.
//! See research/conflagration.md. The world owns entity queries, shields,
//! damage delivery and the retained particle reference.
use crate::{
    damage::{attack, Damage},
    impact_motion::Vector,
    spells::Elements,
    spray_pulse::Segment,
};

pub const INSTANT_DAMAGE: Damage = Damage {
    attack: attack::DAMAGE,
    element: Elements::FIRE.0,
    amount: 600.0,
    magnitude: 1.0,
};
pub const STATUS_DAMAGE: Damage = Damage {
    attack: attack::STATUS,
    element: Elements::FIRE.0,
    amount: 75.0,
    magnitude: 2.0,
};

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub handle: u16,
    pub position: Vector,
    pub radius: f32,
    /// Only Characters have this exclusion; other damageables pass false.
    pub ethereal_character: bool,
}

/// Original ownership gate: a remote Avatar never delivers damage here.
/// A local Avatar does so on either peer; non-Avatars do so off the client.
pub fn owns_damage(client: bool, avatar: bool, remote_gamer: bool) -> bool {
    if client {
        avatar && !remote_gamer
    } else {
        !avatar || !remote_gamer
    }
}

pub trait World {
    /// GetEntities(center, 10, ignoreProtectedEntities=false), in native order.
    /// Return damageables only. Dead filtering belongs to the entity query.
    fn targets(&mut self, center: Vector, radius: f32) -> Vec<Target>;
    /// Any Shield.SegmentIntersect(segment, scale=1), with no scenery test.
    fn shielded(&mut self, segment: Segment) -> bool;
    fn damage(
        &mut self,
        target: u16,
        damage: Damage,
        owner: Option<u16>,
        timestamp: f64,
        source: Vector,
    );
    fn move_effect(&mut self, position: Vector, direction: Vector);
}

#[derive(Clone, Debug, PartialEq)]
pub struct Conflagration {
    pub owner: Option<u16>,
    pub timestamp: f64,
    /// Unshifted cast position: the origin of shield probes.
    pub start: Vector,
    /// Fixed emitter/damage-source position. Not the moving wavefront.
    pub position: Vector,
    pub direction: Vector,
    pub ttl: f32,
    pub time: f32,
    hit: Vec<u16>,
}

fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalized(v: Vector) -> Vector {
    let length = dot(v, v).sqrt();
    v.map(|x| x / length)
}

impl Conflagration {
    /// Execute(ISpellCaster), or Execute(position) with its already generated,
    /// normalized random direction. Both shift mPosition three units forward.
    pub fn from_caster(
        owner: Option<u16>,
        position: Vector,
        direction: Vector,
        timestamp: f64,
    ) -> Self {
        let mut result = Self::directed(position, direction, timestamp);
        result.owner = owner;
        // Preserve the native three separate additions.
        for _ in 0..3 {
            for (p, d) in result.position.iter_mut().zip(direction) {
                *p += d;
            }
        }
        result
    }

    /// Execute(position, direction): no three-unit shift, no owner.
    pub fn directed(position: Vector, direction: Vector, timestamp: f64) -> Self {
        Self {
            owner: None,
            timestamp,
            start: position,
            position,
            direction,
            ttl: 1.0,
            time: 0.0,
            hit: Vec::new(),
        }
    }

    pub fn is_dead(&self) -> bool {
        self.ttl <= 0.0
    }
    pub fn center(&self) -> Vector {
        std::array::from_fn(|i| {
            self.position[i] + self.direction[i] * (self.time * self.time * 20.0)
        })
    }

    /// Matrix.CreateWorld axes before the visual-only backward-axis stretch.
    pub fn axes(&self) -> [Vector; 3] {
        let backward = normalized(self.direction.map(|x| -x));
        let right = normalized(cross([0.0, 1.0, 0.0], backward));
        [right, cross(backward, right), backward]
    }

    /// Sphere/box non-disjoint test for local bounds ±(5.5,1,1).
    pub fn touches(&self, position: Vector, radius: f32) -> bool {
        let center = self.center();
        let delta = std::array::from_fn(|i| position[i] - center[i]);
        let axes = self.axes();
        let extents = [5.5, 1.0, 1.0];
        let mut distance_squared = 0.0;
        for i in 0..3 {
            let excess = (dot(delta, axes[i]).abs() - extents[i]).max(0.0);
            distance_squared += excess * excess;
        }
        distance_squared <= radius * radius
    }

    /// RenderData.Draw's distortion envelope (no clamping in native code).
    pub fn distortion(&self) -> f32 {
        self.time * self.time * (1.0 - self.time) * 6.75
    }
    pub fn visual_axes(&self) -> [Vector; 3] {
        let mut axes = self.axes();
        axes[2] = axes[2].map(|x| x * (1.0 + self.time));
        axes
    }

    /// Update runs even on the frame TTL crosses zero; SpellManager removes
    /// the effect afterwards. Shielded/ethereal targets do not enter the hit list.
    pub fn update(&mut self, dt: f32, deliver_damage: bool, world: &mut impl World) {
        self.ttl -= dt;
        self.time += dt;
        let center = self.center();
        if deliver_damage {
            for target in world.targets(center, 10.0) {
                if Some(target.handle) == self.owner || self.hit.contains(&target.handle) {
                    continue;
                }
                // The native shield query happens even for a box miss.
                let blocked = world.shielded(Segment {
                    origin: self.start,
                    delta: std::array::from_fn(|i| target.position[i] - self.start[i]),
                });
                if blocked
                    || !self.touches(target.position, target.radius)
                    || target.ethereal_character
                {
                    continue;
                }
                world.damage(
                    target.handle,
                    INSTANT_DAMAGE,
                    self.owner,
                    self.timestamp,
                    self.position,
                );
                world.damage(
                    target.handle,
                    STATUS_DAMAGE,
                    self.owner,
                    self.timestamp,
                    self.position,
                );
                self.hit.push(target.handle);
            }
        }
        world.move_effect(center, self.direction);
    }
}
