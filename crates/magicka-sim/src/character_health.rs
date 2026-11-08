//! Direct Character.Damage and ordered Character.UpdateStatusEffects.
//! See output/research/character-health.md. InternalDamage (collision/spell
//! delivery) and the character death state machine are separate, unported paths.
use crate::character_status::CharacterStatuses;
use crate::spells::Elements as E;
use crate::status::{Status as S, StatusEffect, StatusOwner};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelfShieldType {
    #[default]
    None,
    Shield,
    Earth,
    Ice,
    IcedEarth,
}

#[derive(Clone, Debug, Default)]
pub struct SelfShield {
    pub kind: SelfShieldType,
    /// Original spell retained by the native constructor for contact damage.
    pub spell: crate::spells::Spell,
    pub health: f32,
    pub ice_magnitude: f32,
    pub earth_magnitude: f32,
    pub max_health: f32,
    pub magnitude: f32,
    pub noise_offsets: [[f32; 2]; 3],
    pub time_sensitive: bool,
    pub time_since_damage: f32,
}
/// Compatibility name for the original direct-damage-only API.
pub type DirectShield = SelfShield;

impl SelfShield {
    /// Character.HealSelfShield routes signed damage through the Shield element.
    pub fn heal(&mut self, amount: f32) {
        self.damage(&mut -amount, E::SHIELD);
    }

    fn damage(&mut self, amount: &mut f32, element: E) {
        if self.kind == SelfShieldType::None {
            return;
        }
        self.time_since_damage = 0.0;
        match self.kind {
            SelfShieldType::Shield => {
                if *amount >= 0.0 || element.intersects(E::SHIELD) {
                    if *amount < 0.0 && !self.time_sensitive {
                        self.health -= *amount / 2.0;
                    } else {
                        self.health -= *amount;
                    }
                }
            }
            SelfShieldType::Earth | SelfShieldType::Ice | SelfShieldType::IcedEarth => {
                if *amount < 0.0 {
                    *amount = 0.0;
                    return;
                }
                if element.intersects(E::FIRE) {
                    *amount *= 1.0 + self.ice_magnitude;
                }
                self.health -= *amount;
            }
            SelfShieldType::None => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum DirectResult {
    None = 0,
    Damaged = 1,
    Healed = 0x80,
    Killed = 0x200,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HealthEvent {
    StopHypnotize,
    ProfileHealed(f32),
    Statistics {
        amount: i32,
        element: E,
        result: DirectResult,
    },
    DecreaseEntanglement {
        amount: f32,
        element: E,
    },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct HealthContext {
    pub immortal: bool,
    pub network_client: bool,
    pub avatar: bool,
    pub has_last_damager: bool,
    /// IsGripping && last accumulation damager's avatar == gripped character.
    pub gripping_last_damager: bool,
    pub entangled: bool,
}

/// Initialized by the owner/template, not a fabricated default character.
#[derive(Clone, Debug, Default)]
pub struct CharacterHealth {
    pub hit_points: f32,
    pub max_hit_points: f32,
    pub invisibility_timer: f32,
    pub shield: DirectShield,
    pub grip_damage: f32,
    pub fire_damage: f32,
    pub poison_damage: f32,
    pub healing: f32,
    pub bleed_damage: f32,
    /// Rewrite extension: mod modifiers' summed `DamageTakenPercent`
    /// (0: none). Scales incoming positive damage only.
    pub damage_taken_percent: f32,
    /// Rewrite extension: mod modifiers' summed `MaxHealthBonus`, added
    /// after native buffs whenever the maximum is recomputed.
    pub max_health_bonus: f32,
}
impl CharacterHealth {
    /// Damage after mod `DamageTakenPercent` (healing and zero untouched).
    pub fn mod_scaled(&self, amount: f32) -> f32 {
        if amount > 0.0 && self.damage_taken_percent != 0.0 {
            amount * (1.0 + self.damage_taken_percent / 100.0).max(0.0)
        } else {
            amount
        }
    }
    pub fn damage(
        &mut self,
        mut amount: f32,
        element: E,
        status_mask: u16,
        ctx: HealthContext,
        events: &mut Vec<HealthEvent>,
    ) -> DirectResult {
        if ctx.immortal {
            return DirectResult::None;
        }
        self.invisibility_timer = 0.0;
        self.shield.damage(&mut amount, element);
        if element.intersects(E::FIRE) && status_mask & S::Greased as u16 != 0 {
            amount *= 2.0;
        }
        let amount = self.mod_scaled(amount);
        let before = self.hit_points;
        let predicted = before - amount;
        if !ctx.network_client {
            self.hit_points -= amount;
        }
        let result = if predicted <= 0.0 && before > 0.0 {
            DirectResult::Killed
        } else if amount >= 0.0 {
            events.push(HealthEvent::StopHypnotize);
            DirectResult::Damaged
        } else {
            if ctx.avatar {
                events.push(HealthEvent::ProfileHealed(amount));
            }
            DirectResult::Healed
        };
        if ctx.has_last_damager && ctx.gripping_last_damager {
            self.grip_damage += amount;
        }
        if ctx.has_last_damager && !element.intersects(E::LIFE) {
            events.push(HealthEvent::Statistics {
                amount: amount as i32,
                element,
                result,
            });
        }
        match element {
            E::FIRE => self.fire_damage += amount,
            E::POISON => self.poison_damage += amount,
            E::LIFE if predicted < self.max_hit_points => self.healing += amount,
            E::EARTH => self.bleed_damage += amount,
            _ => {}
        }
        if !ctx.network_client {
            if predicted > self.max_hit_points {
                self.hit_points = self.max_hit_points;
            }
            if ctx.entangled && element.intersects(E::FIRE) {
                events.push(HealthEvent::DecreaseEntanglement { amount, element });
            }
        }
        result
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StatusFrame {
    pub panic: f32,
    pub max_panic: f32,
    pub light_present: bool,
    /// Caller supplies the current movement multiplier; updates only cap it.
    pub speed_multiplier: f32,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct StatusUpdateContext {
    /// Original Character.Dead property; not inferred from HP in this method.
    pub dead: bool,
    pub in_cutscene: bool,
    pub cues_exist: u16,
    pub cues_stopping: u16,
    pub buff_tracker: bool,
    pub health: HealthContext,
}
#[derive(Clone, Debug, PartialEq)]
pub enum StatusUpdateEvent {
    Health(HealthEvent),
    StopCue(S),
    RemoveTrackedBuff(S),
    MoveLight,
    StartBurningLight,
    StopLight,
}
const ORDER: [S; 9] = [
    S::Burning,
    S::Wet,
    S::Frozen,
    S::Cold,
    S::Poisoned,
    S::Healing,
    S::Greased,
    S::Steamed,
    S::Bleeding,
];
fn max(a: f32, b: f32) -> f32 {
    if a > b || a.is_nan() {
        a
    } else {
        b
    }
}
fn min(a: f32, b: f32) -> f32 {
    if a < b || a.is_nan() {
        a
    } else {
        b
    }
}

impl CharacterStatuses {
    pub fn update(
        &mut self,
        dt: f32,
        health: &mut CharacterHealth,
        frame: &mut StatusFrame,
        ctx: StatusUpdateContext,
        events: &mut Vec<StatusUpdateEvent>,
    ) {
        self.dry_timer -= dt;
        let old_mask = self.current_mask;
        let mut new_mask = 0;
        for (i, slot_kind) in ORDER.into_iter().enumerate() {
            let effect = &mut self.slots[i];
            if !ctx.dead {
                // Health is sampled afresh after every preceding damage call.
                let tick = effect.update(
                    dt,
                    StatusOwner {
                        is_character: true,
                        in_cutscene: ctx.in_cutscene,
                        hit_points: health.hit_points,
                        max_hit_points: health.max_hit_points,
                    },
                );
                if let Some(speed) = tick.speed_limit {
                    frame.speed_multiplier = min(frame.speed_multiplier, speed);
                }
                if let Some((amount, element)) = tick.damage {
                    let mut health_events = Vec::new();
                    health.damage(
                        amount as f32,
                        element,
                        old_mask,
                        ctx.health,
                        &mut health_events,
                    );
                    events.extend(health_events.into_iter().map(StatusUpdateEvent::Health));
                }
            }
            if ctx.dead || effect.dead() {
                if matches!(effect.kind, S::Burning | S::Wet | S::Frozen)
                    && ctx.cues_exist & effect.kind as u16 != 0
                    && ctx.cues_stopping & effect.kind as u16 == 0
                {
                    events.push(StatusUpdateEvent::StopCue(effect.kind));
                }
                if !ctx.dead && ctx.buff_tracker {
                    events.push(StatusUpdateEvent::RemoveTrackedBuff(slot_kind));
                }
                *effect = StatusEffect::default();
            } else {
                new_mask |= effect.kind as u16;
            }
        }
        if frame.light_present {
            events.push(StatusUpdateEvent::MoveLight);
        }
        // HasStatus still reads the OLD cache until the last assignment.
        if old_mask & S::Steamed as u16 != 0 {
            frame.panic = max(self.slots[7].magnitude - frame.max_panic, 0.0);
        } else if old_mask & S::Burning as u16 != 0 {
            frame.panic = max(self.slots[0].magnitude - frame.max_panic, 0.0);
            if !frame.light_present {
                frame.light_present = true;
                events.push(StatusUpdateEvent::StartBurningLight);
            }
        } else if frame.light_present {
            frame.panic = 0.0;
            frame.light_present = false;
            events.push(StatusUpdateEvent::StopLight);
        }
        self.current_mask = new_mask;
    }
}
