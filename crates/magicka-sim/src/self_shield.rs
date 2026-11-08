//! SelfShield creation, impact filtering, update and removal.
//! Source: Character/SelfShield, documented in research/self-shield.md.
use crate::character_health::{SelfShield, SelfShieldType as T};
use crate::damage::{attack as A, Damage};
use crate::spells::{Elements as E, Spell};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum ShieldResult {
    None = 0,
    Hit = 2,
    Deflected = 0x100,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShieldEvent {
    SetMass(f32),
    PlayCreationCue(T),
    DirtyShieldMeshes,
    ResetBarrierMeshes,
    CircleDamage { radius: f32, damage: Damage },
    HitEffect(T),
    DeathEffect(T),
    ClearAura,
    HealthBar { ratio: f32, time_since_damage: f32 },
    RemoveTrackedBuff(T),
    UpdateTrackedBuff { kind: T, health: f32 },
}
#[derive(Clone, Copy, Debug)]
pub struct ShieldImpactContext {
    pub damage_enabled: bool,
    pub network_client: bool,
}

impl SelfShield {
    pub fn active(&self) -> bool {
        self.kind != T::None
    }
    pub fn solid(&self) -> bool {
        matches!(self.kind, T::Earth | T::Ice | T::IcedEarth)
    }

    pub fn from_spell(spell: &Spell, events: &mut Vec<ShieldEvent>) -> Self {
        let earth = spell.get(E::EARTH);
        let ice = spell.get(E::ICE);
        let kind = if earth > 0.0 {
            if ice > 0.0 {
                T::IcedEarth
            } else {
                T::Earth
            }
        } else if ice > 0.0 {
            T::Ice
        } else {
            T::Shield
        };
        let sum = earth + ice;
        let magnitude = if 1.0 > sum { 1.0 } else { sum };
        let mut shield = SelfShield {
            kind,
            spell: *spell,
            time_sensitive: true,
            magnitude,
            earth_magnitude: earth,
            ice_magnitude: ice,
            time_since_damage: 1.0,
            ..Default::default()
        };
        if kind == T::Shield {
            shield.health = 500.0;
            events.extend([
                ShieldEvent::PlayCreationCue(T::Shield),
                ShieldEvent::DirtyShieldMeshes,
            ]);
        } else {
            events.extend([
                ShieldEvent::SetMass(1001.0),
                ShieldEvent::ResetBarrierMeshes,
            ]);
            if ice > 0.0 {
                events.push(ShieldEvent::CircleDamage {
                    radius: 3.0,
                    damage: Damage {
                        attack: A::DAMAGE | A::PIERCING,
                        element: E::EARTH.0,
                        amount: 275.0,
                        magnitude,
                    },
                });
                events.push(ShieldEvent::PlayCreationCue(T::Ice));
                shield.health = 900.0;
            }
            if earth > 0.0 {
                events.push(ShieldEvent::PlayCreationCue(T::Earth));
                shield.health = 1500.0;
            }
        }
        shield.max_health = shield.health;
        shield
    }

    /// Ice/IcedEarth branch of Character.OnCollision, after own death/grip
    /// rejection and before ethereal/ignore filtering. Caller supplies only
    /// character targets and runs the shared character HitList.Update once
    /// per Character.Update. Delivery must call the public damage wrapper
    /// synchronously with this character as attacker, time 0 and position zero.
    /// Cooldown is added after delivery, regardless of its damage result.
    pub fn ice_contact(
        &self,
        target: u16,
        source_velocity: crate::impact_motion::Vector,
        target_velocity: crate::impact_motion::Vector,
        hits: &mut crate::spray_pulse::HitList,
        deliver: impl FnOnce(&[Damage]),
    ) -> Result<bool, crate::damage::DamageError> {
        if !matches!(self.kind, T::Ice | T::IcedEarth) || hits.contains(target) {
            return Ok(false);
        }
        let target_speed_squared = target_velocity.iter().map(|v| v * v).sum::<f32>();
        let source_speed_squared = source_velocity.iter().map(|v| v * v).sum::<f32>();
        // Native bge followed by blt.un: NaN alone never satisfies the gate.
        if !(target_speed_squared >= 100.0 || source_speed_squared >= 100.0) {
            return Ok(false);
        }
        let damage = crate::damage::calculate_damage(
            &self.spell,
            crate::spells::SpellType::Projectile,
            crate::damage::CastType::Force,
        )?;
        deliver(&damage);
        hits.add(target);
        Ok(true)
    }

    /// Setter resets both health and capacity each time false is assigned.
    pub fn set_time_sensitive(&mut self, value: bool) {
        self.time_sensitive = value;
        if !value {
            self.health = 250.0;
            self.max_health = 250.0;
        }
    }

    /// Full impact overload; distinct from the direct status damage method.
    /// An inactive shield is a no-op adapter (original caller checks Active).
    pub fn filter_impact(
        &mut self,
        damage: &mut Damage,
        ctx: ShieldImpactContext,
        events: &mut Vec<ShieldEvent>,
    ) -> ShieldResult {
        if !self.active() {
            return ShieldResult::None;
        }
        let physical = damage.element & (E::EARTH | E::ICE).0 != 0;
        let hurts = damage.attack & A::DAMAGE != 0;
        let status = damage.attack & A::STATUS != 0;
        let knockback = damage.attack & A::KNOCKBACK != 0;
        if self.kind == T::Shield {
            if hurts || status {
                let total = damage.amount * damage.magnitude;
                if ctx.damage_enabled && total >= 0.0 && !ctx.network_client {
                    self.health -= total;
                }
                self.time_since_damage = 0.0;
                damage.attack = 0;
                if physical && self.time_sensitive {
                    damage.attack = A::KNOCKBACK;
                    damage.magnitude = 0.6980619;
                }
                if hurts {
                    events.push(ShieldEvent::HitEffect(T::Shield));
                }
            }
            return ShieldResult::Deflected;
        }
        if hurts {
            if physical {
                damage.amount -= if self.kind == T::Earth { 10.0 } else { 70.0 };
                damage.amount *= 1.0 - (self.ice_magnitude * 0.05 + self.earth_magnitude * 0.15);
            }
            let total = damage.amount * damage.magnitude;
            let overflow = total - self.health;
            if ctx.damage_enabled && total >= 0.0 && !ctx.network_client {
                self.health -= total;
            }
            self.time_since_damage = 0.0;
            if self.earth_magnitude > 0.0 {
                events.push(ShieldEvent::HitEffect(T::Earth));
            }
            if self.ice_magnitude > 0.0 {
                events.push(ShieldEvent::HitEffect(T::Ice));
            }
            if overflow > 0.0 {
                // Original writes total overflow to Amount without resetting
                // Magnitude. Downstream damage multiplies it again.
                damage.amount = overflow;
                return ShieldResult::Hit;
            }
            damage.amount = 0.0;
            damage.attack = 0;
            return ShieldResult::Deflected;
        }
        if knockback {
            damage.amount = 0.0;
            damage.magnitude = 0.0;
        }
        ShieldResult::None
    }

    /// Update numerical shield state; solid shells ASSIGN the speed multiplier,
    /// not min it with the previous movement/status value.
    pub fn update(
        &mut self,
        dt: f32,
        speed: &mut f32,
        template_mass: f32,
        buff_tracker: bool,
        events: &mut Vec<ShieldEvent>,
    ) {
        self.time_since_damage += dt;
        if self.health > self.max_health {
            self.health = self.max_health;
        }
        match self.kind {
            T::Shield => {
                let n = dt * (0.5 * (self.max_health - self.health) / self.max_health + 0.05);
                self.noise_offsets[2][1] -= n;
                self.noise_offsets[0][1] += 0.3 * n;
                self.noise_offsets[0][0] -= 0.7 * n;
                self.noise_offsets[1][1] += 0.7 * n;
                self.noise_offsets[1][0] += 0.4 * n;
                if self.time_sensitive {
                    self.health -= dt * 100.0;
                }
            }
            T::Earth | T::Ice | T::IcedEarth => {
                *speed = 0.9 - (self.earth_magnitude * 0.075 + self.ice_magnitude * 0.025);
            }
            T::None => {}
        }
        events.push(ShieldEvent::HealthBar {
            ratio: self.health / self.max_health,
            time_since_damage: self.time_since_damage,
        });
        if self.health <= 0.0 {
            self.remove(template_mass, buff_tracker, events);
        }
        if buff_tracker {
            events.push(ShieldEvent::UpdateTrackedBuff {
                kind: self.kind,
                health: self.health,
            });
        }
    }

    pub fn remove(
        &mut self,
        template_mass: f32,
        buff_tracker: bool,
        events: &mut Vec<ShieldEvent>,
    ) {
        events.push(ShieldEvent::SetMass(template_mass));
        if buff_tracker {
            events.push(ShieldEvent::RemoveTrackedBuff(self.kind));
        }
        self.kind = T::None;
        if self.earth_magnitude > 0.0 {
            events.push(ShieldEvent::DeathEffect(T::Earth));
        }
        if self.ice_magnitude > 0.0 {
            events.push(ShieldEvent::DeathEffect(T::Ice));
        }
        events.push(ShieldEvent::ClearAura);
    }
}
