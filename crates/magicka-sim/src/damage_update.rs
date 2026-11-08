//! Character.UpdateDamage: regeneration and floating-number accumulation.
//! See research/damage-update.md. Timers retain the source's single-step behavior.
use crate::character_impact::ImpactCharacter;
use crate::status::Status;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberKind {
    Regeneration,
    Healing,
    Fire,
    Poison,
    Bleeding,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DamageUpdateEvent {
    Number {
        kind: NumberKind,
        amount: f32,
        /// AddNumber iTTL.
        ttl: f32,
    },
    ReleaseNumber(i32),
    MoveNumber(i32),
}
#[derive(Clone, Copy, Debug)]
pub struct DamageUpdateContext {
    pub regeneration: i32,
    pub is_play_state: bool,
    pub in_cutscene: bool,
    /// Evaluated Character.Dead, including explicit-kill-only suppression.
    pub dead: bool,
}
/// Constructor/Initialize timer values for a newly allocated character only.
/// Pooled character reuse must preserve/reset fields at the original call sites.
#[derive(Clone, Debug)]
pub struct DamageBookkeeping {
    pub regen_timer: f32,
    pub total_regen: f32,
    /// Healing, fire, poison, bleed in original processing order.
    pub accumulation_timers: [f32; 4],
    pub last_damage_index: i32,
    pub time_since_status_damage: f32,
}
impl Default for DamageBookkeeping {
    fn default() -> Self {
        Self {
            regen_timer: 1.0,
            total_regen: 0.0,
            accumulation_timers: [0.25, 0.0, 0.25, 0.0],
            last_damage_index: -1,
            time_since_status_damage: f32::MAX,
        }
    }
}
impl DamageBookkeeping {
    pub fn update(
        &mut self,
        character: &mut ImpactCharacter,
        dt: f32,
        ctx: DamageUpdateContext,
        events: &mut Vec<DamageUpdateEvent>,
    ) {
        let h = &mut character.health;
        if h.hit_points > 0.0
            && h.hit_points <= h.max_hit_points
            && !character.statuses.has(Status::Burning)
            && !character.statuses.has(Status::Poisoned)
        {
            let amount = dt * ctx.regeneration as f32;
            self.total_regen += amount;
            h.hit_points += amount;
            self.regen_timer -= dt;
            if self.regen_timer <= 0.0 {
                self.regen_timer = 1.0;
                self.total_regen -= ctx.regeneration as f32;
                if ctx.regeneration != 0
                    && ctx.is_play_state
                    && !ctx.in_cutscene
                    && h.hit_points <= h.max_hit_points
                {
                    events.push(DamageUpdateEvent::Number {
                        kind: NumberKind::Regeneration,
                        amount: ctx.regeneration.wrapping_neg() as f32,
                        ttl: 0.7,
                    });
                }
            }
            if h.hit_points > h.max_hit_points {
                h.hit_points = h.max_hit_points;
            }
        }
        character.time_since_last_damage += dt;
        self.time_since_status_damage += dt;
        if self.last_damage_index >= 0 {
            if character.time_since_last_damage > 0.333 || ctx.dead {
                events.push(DamageUpdateEvent::ReleaseNumber(self.last_damage_index));
                self.last_damage_index = -1;
            } else {
                events.push(DamageUpdateEvent::MoveNumber(self.last_damage_index));
            }
        }
        for ((timer, value), kind) in self
            .accumulation_timers
            .iter_mut()
            .zip([
                &mut h.healing,
                &mut h.fire_damage,
                &mut h.poison_damage,
                &mut h.bleed_damage,
            ])
            .zip([
                NumberKind::Healing,
                NumberKind::Fire,
                NumberKind::Poison,
                NumberKind::Bleeding,
            ])
        {
            *timer -= dt;
            if *timer <= 0.0 {
                *timer += 1.0;
                if *value != 0.0 {
                    events.push(DamageUpdateEvent::Number {
                        kind,
                        amount: *value,
                        ttl: 0.7,
                    });
                    *value = 0.0;
                    self.time_since_status_damage = 0.0;
                }
            }
        }
    }
}
