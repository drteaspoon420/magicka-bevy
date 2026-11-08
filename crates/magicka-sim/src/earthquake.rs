//! EarthQuake singleton. Scene queries and synchronous delivery belong to the
//! host; timing, random draw order and target filtering follow the static CIL.
//! Evidence and pending live integration: research/earthquake.md.
use crate::{
    damage::{attack, Damage},
    impact_motion::Vector,
    spells::Elements,
    spray_pulse::HitList,
};

pub const TTL: f32 = 16.0;
pub const HIT_PERIOD: f32 = 0.5;
pub const QUAKE_RADIUS: f32 = 10.0;
pub const CENTER_RADIUS: f32 = 15.0;

#[derive(Clone, Copy, Debug)]
pub struct CharacterTarget {
    pub levitating: bool,
    pub immortal: bool,
    pub ethereal: bool,
    pub in_event: bool,
    pub grounded: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub handle: u16,
    pub position: Vector,
    pub mass: f32,
    pub damageable: bool,
    pub character: Option<CharacterTarget>,
}

pub trait EarthquakeHost {
    /// Uniform [0, 1), corresponding to SpecialAbility.RANDOM.NextDouble.
    fn random(&mut self) -> f64;
    fn client(&self) -> bool;
    /// Scene SegmentIntersect(position, [0, -20, 0]).
    fn ground(&mut self, position: Vector) -> Option<Vector>;
    /// Native GetEntities: radius 10, ignoreProtected=false, ignoreYAxis=false.
    fn targets(&mut self, position: Vector, radius: f32) -> Vec<u16>;
    /// Fetch immediately before delivery so prior callbacks remain visible.
    fn target(&self, handle: u16) -> Option<Target>;
    fn impulse_velocity(&mut self, handle: u16, impulse: Vector);
    fn damage(
        &mut self,
        handle: u16,
        owner: Option<u16>,
        damage: Damage,
        timestamp: f64,
        point: Vector,
    );
    fn add_velocity(&mut self, handle: u16, velocity: Vector);
    /// NewQuake: CameraShake(position, 0.5, duration); server message if needed.
    fn new_quake(&mut self, owner: Option<u16>, position: Vector, duration: f32);
    /// Stop prior quake cue/effect, then Additional-bank cue and scaled effect.
    fn pulse(&mut self, position: Vector, scale: Vector);
    fn stop_presentation(&mut self);
    fn failed(&mut self, position: Vector);
}

#[derive(Clone, Debug, Default)]
pub struct Earthquake {
    pub registered: bool,
    pub ttl: f32,
    pub owner: Option<u16>,
    /// Position Execute does not call the timestamp-setting owner overload.
    pub timestamp: f64,
    pub epicenter: Vector,
    pub quake_position: Vector,
    pub quake_ttl: f32,
    pub hit_timer: f32,
    pub hits: HitList,
}

impl Earthquake {
    pub fn cast<H: EarthquakeHost>(
        &mut self,
        owner: Option<u16>,
        position: Vector,
        host: &mut H,
    ) -> bool {
        // Execute(position) resets these before probing, even on failure.
        // Recasts retain quake position, quake TTL and the shared HitList.
        self.owner = None;
        self.ttl = TTL;
        self.hit_timer = HIT_PERIOD;
        let Some(ground) = host.ground(position) else {
            if owner.is_some() {
                host.failed(position);
            }
            return false;
        };
        self.epicenter = ground;
        self.registered = true;
        self.owner = owner;
        true
    }

    /// Network NewQuake does not reset the global lifetime or hit timer.
    pub fn new_quake<H: EarthquakeHost>(&mut self, position: Vector, ttl: f32, host: &mut H) {
        self.quake_position = position;
        self.quake_ttl = ttl;
        host.new_quake(self.owner, position, ttl);
    }

    pub fn update<H: EarthquakeHost>(&mut self, dt: f32, host: &mut H) {
        if !self.registered {
            return;
        }
        self.ttl -= dt;
        self.hit_timer -= dt;
        self.quake_ttl -= dt;
        self.hits.update(dt);
        if !host.client() && self.quake_ttl <= 0.0 {
            let radius = host.random().sqrt() as f32;
            let angle = host.random() as f32 * std::f32::consts::TAU;
            let x = (f64::from(radius) * f64::from(angle).cos()) as f32;
            let z = (f64::from(radius) * f64::from(angle).sin()) as f32;
            let position = [
                self.epicenter[0] + x * CENTER_RADIUS,
                self.epicenter[1] + 0.5,
                self.epicenter[2] + z * CENTER_RADIUS,
            ];
            let ttl = host.random() as f32 + 1.0;
            self.new_quake(position, ttl, host);
        }
        // Native does at most one pulse per update, including its last frame;
        // it resets to 0.5 rather than retaining overshoot or catching up.
        if self.hit_timer <= 0.0 {
            self.quake(host);
            self.hit_timer = HIT_PERIOD;
        }
        if self.ttl <= 0.0 {
            self.remove(host);
        }
    }

    fn quake<H: EarthquakeHost>(&mut self, host: &mut H) {
        for handle in host.targets(self.quake_position, QUAKE_RADIUS) {
            if self.hits.contains(handle) {
                continue;
            }
            let Some(target) = host.target(handle) else {
                continue;
            };
            if target.damageable {
                if let Some(c) = target.character {
                    if c.levitating
                        || target.mass >= 3000.0
                        || c.immortal
                        || c.ethereal
                        || c.in_event
                        || !c.grounded
                    {
                        continue;
                    }
                    host.impulse_velocity(handle, [0.0, 6.0, 0.0]);
                }
                // Position is sampled after the impulse, just like the native
                // public damage wrapper. That callback can alter body state.
                let Some(target) = host.target(handle) else {
                    continue;
                };
                let mut point = target.position;
                point[0] += host.random() as f32 - 0.5;
                point[2] += host.random() as f32 - 0.5;
                host.damage(
                    handle,
                    self.owner,
                    Damage {
                        attack: attack::KNOCKDOWN,
                        element: Elements::EARTH.0,
                        amount: target.mass,
                        magnitude: 1.0,
                    },
                    self.timestamp,
                    point,
                );
            } else {
                let x = host.random() as f32 - 0.5;
                let z = host.random() as f32 - 0.5;
                host.add_velocity(handle, [x, 5.0, z]);
            }
            // HIT_TTL (0.5) is unused: the one-argument Add uses 0.25.
            self.hits.add(handle);
        }
        host.pulse(
            self.quake_position,
            [QUAKE_RADIUS / 5.0, 1.0, QUAKE_RADIUS / 5.0],
        );
    }

    /// Nullify/expiry stops presentation without resetting cached fields.
    pub fn remove<H: EarthquakeHost>(&mut self, host: &mut H) {
        if self.registered {
            host.stop_presentation();
        }
        self.registered = false;
    }
}
