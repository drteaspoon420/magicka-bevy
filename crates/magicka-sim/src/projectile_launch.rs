//! Native ProjectileSpell Force scheduling, before missile physics hosting.
//! Evidence and scope: research/projectile-launch.md.
use crate::{
    damage::CastType,
    impact_motion::Vector,
    spell_cast::{EffectKind, EffectLifecycle, EffectOwner},
    spells::{Elements as E, Spell, SpellType},
    spray_pulse::Segment,
};

fn clr_max(a: f32, b: f32) -> f32 {
    if a > b || a.is_nan() {
        a
    } else {
        b
    }
}
impl Spell {
    /// Spell.BlastSize. Magnitudes survive mask edits, as in the original.
    pub fn blast_size(&self) -> f32 {
        let m = match self.spell_type() {
            SpellType::Spray => clr_max(
                self.get(E::STEAM),
                clr_max(
                    self.get(E::FIRE),
                    clr_max(
                        self.get(E::COLD),
                        clr_max(self.get(E::WATER), self.get(E::POISON)),
                    ),
                ),
            ),
            SpellType::Projectile => clr_max(self.get(E::EARTH), self.get(E::ICE)),
            SpellType::Beam => clr_max(self.get(E::ARCANE), self.get(E::LIFE)),
            _ => 0.,
        };
        ((m / 5.) as f64).sqrt() as f32
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectileLaunch {
    pub position: Vector,
    pub velocity: Vector,
    pub spell: Spell,
    pub splash: f32,
    pub total_projectiles: i32,
}
#[derive(Clone, Copy, Debug)]
pub struct LaunchSource {
    pub owner_position: Vector,
    pub owner_direction: Vector,
    pub cast_position: Vector,
    /// Native network/local owner predicate; false still consumes shots/RNG.
    pub may_spawn: bool,
}
pub trait ProjectileLaunchHost {
    fn shields(&mut self) -> Vec<u16>;
    fn shield_intersection(&mut self, shield: u16, segment: Segment, radius: f32)
        -> Option<Vector>;
    fn blocked_effect(&mut self, element: E, position: Vector, direction: Vector);
    fn next_double(&mut self) -> f64;
    fn spawn(&mut self, launch: ProjectileLaunch);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchError {
    InvalidPhysicalSpell,
    InvalidShotCount,
    InvalidInterval,
}
#[derive(Clone, Debug)]
pub struct ForceProjectile {
    pub lifecycle: EffectLifecycle,
    pub spell: Spell,
    pub charge: f32,
    pub force: f32,
    pub mass: f32,
    pub radius: f32,
    pub cooldown: f32,
    pub interval: f32,
    pub remaining: i32,
    pub total: i32,
}
impl ForceProjectile {
    /// ProjectileSpell.CastSelf shares Force initialization except charge=.5
    /// and the Self lifecycle; Update owns the distinct falling launch geometry.
    pub fn cast_self(
        spell: Spell,
        power: &mut f32,
        from_staff: bool,
        timestamp: f64,
    ) -> Result<Self, LaunchError> {
        let mut projectile = Self::cast(spell, power, true, from_staff, timestamp)?;
        projectile.charge = 0.5;
        projectile.lifecycle =
            EffectLifecycle::begin(CastType::SelfCast, from_staff, timestamp).unwrap();
        Ok(projectile)
    }
    pub fn cast(
        spell: Spell,
        power: &mut f32,
        avatar_or_boss_zone: bool,
        from_staff: bool,
        timestamp: f64,
    ) -> Result<Self, LaunchError> {
        let charge = if avatar_or_boss_zone { *power } else { 0.5 };
        let mut force = 25. + 85. * *power;
        let mass = 1. + clr_max(0., 50. * spell.get(E::EARTH));
        let mut splash_spell = spell;
        splash_spell.element = E(splash_spell.element.0 & !E::EARTH.0);
        let radius = splash_spell.blast_size() * 10.;
        *power = 0.;
        let total = if spell.element.has(E::EARTH) {
            force *= 0.5 + 0.5 / spell.get(E::EARTH);
            1
        } else if spell.element.has(E::ICE) {
            let count = 3. * spell.get(E::ICE);
            if !count.is_finite() || count < 0. || count as f64 >= 2147483648. {
                return Err(LaunchError::InvalidShotCount);
            }
            count as i32
        } else {
            return Err(LaunchError::InvalidPhysicalSpell);
        };
        let interval = (0.0875 + spell.total_magnitude() / 5. * 0.35 * 0.75) / total as f32;
        if total > 0 && (!interval.is_finite() || interval <= 0.) {
            return Err(LaunchError::InvalidInterval);
        }
        Ok(Self {
            lifecycle: EffectLifecycle::begin(CastType::Force, from_staff, timestamp).unwrap(),
            spell,
            charge,
            force,
            mass,
            radius,
            cooldown: 0.,
            interval,
            remaining: total,
            total,
        })
    }
    pub fn deinitialize(&mut self) {
        if self.lifecycle.active {
            self.lifecycle.active = false;
            self.cooldown = 0.;
        }
    }
    pub fn update(
        &mut self,
        dt: f32,
        owner: EffectOwner,
        source: LaunchSource,
        host: &mut impl ProjectileLaunchHost,
    ) -> (bool, f32) {
        let mut direction = source.owner_direction;
        let self_cast = self.lifecycle.cast_type == CastType::SelfCast;
        self.cooldown -= dt;
        while self.cooldown <= 0. {
            self.cooldown += self.interval;
            self.remaining -= 1;
            let mut position = source.cast_position;
            let segment = Segment {
                origin: source.owner_position,
                delta: std::array::from_fn(|i| source.cast_position[i] - source.owner_position[i]),
            };
            let mut blocked = false;
            for shield in if self_cast { vec![] } else { host.shields() } {
                if let Some(position) = host.shield_intersection(
                    shield,
                    segment,
                    0.1 + 0.075 * self.spell.get(E::EARTH),
                ) {
                    let length = (segment.delta.iter().map(|v| v * v).sum::<f32>()).sqrt();
                    let backwards = segment.delta.map(|v| -v / length);
                    let element = if self.spell.get(E::EARTH) > 0. {
                        E::EARTH
                    } else {
                        E::ICE
                    };
                    host.blocked_effect(element, position, backwards);
                    blocked = true;
                    break;
                }
            }
            if blocked {
                continue;
            }
            let speed = if self.remaining > 0 {
                if self_cast {
                    let angle = host.next_double() as f32 * std::f32::consts::TAU;
                    position[0] += (6.0 * (angle as f64).cos()) as f32;
                    position[1] += 10.0;
                    position[2] += (6.0 * (angle as f64).sin()) as f32;
                    direction = std::array::from_fn(|i| source.owner_position[i] - position[i]);
                    let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
                    direction = direction.map(|v| v / length);
                } else {
                    let angle = (host.next_double() as f32 - 0.5)
                        * std::f32::consts::FRAC_PI_2
                        * (1. - self.charge);
                    let (sin, cos) = ((angle as f64).sin() as f32, (angle as f64).cos() as f32);
                    direction = [
                        direction[0] * cos + direction[2] * sin,
                        direction[1],
                        -direction[0] * sin + direction[2] * cos,
                    ];
                }
                self.force * (1.25 + 0.75 * host.next_double() as f32)
            } else {
                if self_cast {
                    position[1] += 8.0;
                    direction = [0.001, -1.0, 0.0];
                }
                self.force
            };
            if self.remaining < 0 {
                continue;
            }
            if source.may_spawn {
                host.spawn(ProjectileLaunch {
                    position,
                    velocity: direction.map(|v| v * speed),
                    spell: self.spell,
                    splash: self.radius,
                    total_projectiles: self.total,
                });
            }
        }
        if self.remaining <= 0 {
            self.deinitialize();
        }
        let mut deinitialized = false;
        let (keep, _) = self
            .lifecycle
            .update(dt, EffectKind::Projectile, owner, |l| {
                if l.active {
                    l.active = false;
                    deinitialized = true;
                }
            });
        if deinitialized {
            self.cooldown = 0.;
        }
        (keep, 0.)
    }
}
