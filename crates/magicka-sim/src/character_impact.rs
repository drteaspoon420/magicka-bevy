//! Numerical Character.InternalDamage pipeline. World effects are events;
//! see research/character-impact.md for the remaining lifecycle integrations.
use crate::character_health::CharacterHealth;
use crate::character_status::{CharacterStatuses, StatusContext, StatusError, StatusEvent};
use crate::damage::{attack as A, Damage};
use crate::impact_motion::{block_impact, ImpactMotion, MotionContext, Vector};
use crate::self_shield::{ShieldEvent, ShieldImpactContext};
use crate::spells::Elements as E;
use crate::status::{Status as S, StatusEffect};

pub mod result {
    pub const DAMAGED: u16 = 1;
    pub const HIT: u16 = 2;
    pub const HEALED: u16 = 0x80;
    pub const DEFLECTED: u16 = 0x100;
    pub const KILLED: u16 = 0x200;
    pub const PIERCED: u16 = 0x400;
    pub const OVERKILLED: u16 = 0x800;
}
pub mod feature {
    pub const DAMAGE: u8 = 1;
    pub const KNOCKBACK: u8 = 2;
    pub const NOTIFY: u8 = 4;
    pub const EFFECTS: u8 = 8;
}
#[derive(Clone, Debug, PartialEq)]
pub enum ImpactEvent {
    Shield(ShieldEvent),
    Status(StatusEvent),
    Entangle(f32),
    DecreaseEntanglement {
        magnitude: f32,
        element: E,
    },
    BlockEffect,
    StopHypnotize,
    ProfileHealed(f32),
    OverNineThousand,
    ChangeToGripping,
    Gore {
        amount: f32,
        magnitude: f32,
        element: E,
    },
    Notify {
        amount: f32,
        element: E,
    },
    OverkillLifecycle,
    DamageTrigger,
    Statistics {
        damage: Damage,
        result: u16,
    },
}
#[derive(Clone, Debug, Default)]
pub struct ImpactCharacter {
    /// WarlordCharacter.InternalDamage's specialized equipment-zero queue.
    pub absorb_shield: Option<crate::weapon_spells::WeaponSpells>,
    pub health: CharacterHealth,
    pub statuses: CharacterStatuses,
    pub motion: ImpactMotion,
    pub dead: bool,
    pub overkilled: bool,
    pub cannot_die_without_explicit_kill: bool,
    pub death_status_mask: u16,
    pub zap_timer: f32,
    pub hit_by_lightning: f32,
    pub stun_timer: f32,
    pub is_hit: bool,
    pub last_attacker: Option<u64>,
    pub last_attacker_timer: f32,
    pub last_damage: u16,
    pub last_accumulation_damager: Option<u64>,
    pub time_since_last_damage: f32,
}
impl ImpactCharacter {
    /// Character.Update: with no status effects the last attacker is
    /// forgotten once its 8 s timer has run out.
    pub fn tick_last_attacker(&mut self, dt: f32) {
        if self.last_attacker.is_some() && self.statuses.current_mask == 0 {
            if self.last_attacker_timer <= 0.0 {
                self.last_attacker = None;
            }
            self.last_attacker_timer -= dt;
        }
    }
}
#[derive(Clone)]
pub struct ImpactContext<'a> {
    pub status: StatusContext<'a>,
    pub motion: MotionContext,
    pub attack_position: Vector,
    pub forward: Vector,
    pub block_values: &'a [i32],
    pub block_item: i32,
    pub features: u8,
    pub network_client: bool,
    pub attacker: Option<u64>,
    pub attacker_is_self: bool,
    pub attacker_avatar: bool,
    pub attacker_local_avatar: bool,
    pub attacker_is_gripped: bool,
    pub grip_attack_state: bool,
    pub hit_tolerance: f32,
    pub resurrection_state: bool,
    pub stun_time: f32,
    pub bleed_rate: f32,
    pub blood_and_gore: bool,
    pub has_blood: bool,
    pub bloat_killed: bool,
    pub has_damage_trigger: bool,
}
impl ImpactCharacter {
    fn add_impact_status(
        &mut self,
        kind: S,
        dps: f32,
        magnitude: f32,
        ctx: &mut ImpactContext<'_>,
        events: &mut Vec<ImpactEvent>,
    ) -> Result<u16, StatusError> {
        let mut status_events = Vec::new();
        let r = self.statuses.add(
            StatusEffect::new(kind, dps, magnitude, false),
            &ctx.status,
            &mut status_events,
        )?;
        for event in &status_events {
            ctx.status.cues.observe(*event);
        }
        events.extend(status_events.into_iter().map(ImpactEvent::Status));
        Ok(r as u16)
    }
    pub fn impact(
        &mut self,
        mut damage: Damage,
        ctx: &ImpactContext<'_>,
        events: &mut Vec<ImpactEvent>,
    ) -> Result<u16, StatusError> {
        let mut context = ctx.clone();
        let ctx = &mut context;
        use result::*;
        if ctx.block_item >= 0 && damage.element != 0 {
            if let Some(queue) = &mut self.absorb_shield {
                if damage.element & E::PHYSICAL_ELEMENTS.0 == 0 {
                    let mut spell = crate::spells::Spell {
                        element: E(damage.element),
                        ..Default::default()
                    };
                    for (i, element) in E::BY_INDEX.iter().enumerate() {
                        if damage.element & element.0 != 0 {
                            spell.mag[i] += damage.magnitude;
                        }
                    }
                    queue.add(spell, false, &mut Vec::new());
                } else {
                    let block = block_impact(
                        &mut damage,
                        ctx.block_item,
                        ctx.motion.position,
                        ctx.forward,
                        ctx.attack_position,
                        ctx.block_values,
                    );
                    if block.effect {
                        events.push(ImpactEvent::BlockEffect);
                    }
                }
                return Ok(DEFLECTED);
            }
        }
        if self.dead || ctx.status.immortal {
            return Ok(0);
        }
        if ctx.status.avatar && ctx.status.equipment.is_none() {
            return Err(StatusError::MissingAvatarEquipment);
        }
        let enabled = ctx.features & feature::DAMAGE != 0;
        let before = self.health.hit_points;
        self.health.invisibility_timer = 0.0;
        if damage.attack & A::ENTANGLEMENT != 0 && !self.statuses.has(S::Burning) {
            if !ctx.attacker_is_self {
                events.push(ImpactEvent::Entangle(damage.magnitude));
                return Ok(HIT);
            }
            return Ok(0);
        }
        if ctx.motion.entangled
            && (damage.attack & A::DAMAGE != 0
                || (damage.attack & A::STATUS != 0 && damage.element & E::FIRE.0 != 0))
        {
            events.push(ImpactEvent::DecreaseEntanglement {
                magnitude: damage.magnitude,
                element: E(damage.element),
            });
        }
        let mut shield_events = Vec::new();
        let mut outcome = self.health.shield.filter_impact(
            &mut damage,
            ShieldImpactContext {
                damage_enabled: enabled,
                network_client: ctx.network_client,
            },
            &mut shield_events,
        ) as u16;
        events.extend(shield_events.into_iter().map(ImpactEvent::Shield));
        let mut motion = ctx.motion;
        motion.dead = self.dead;
        motion.immortal = ctx.status.immortal;
        motion.ethereal = ctx.status.ethereal;
        motion.solid_shield = self.health.shield.solid();
        outcome |= self.motion.apply_impact(
            damage,
            ctx.attack_position,
            motion,
            ctx.features & feature::KNOCKBACK != 0,
        );
        let block = block_impact(
            &mut damage,
            ctx.block_item,
            ctx.motion.position,
            ctx.forward,
            ctx.attack_position,
            ctx.block_values,
        );
        if block.effect {
            events.push(ImpactEvent::BlockEffect);
        }
        if block.deflected {
            outcome |= DEFLECTED;
        }
        if enabled && damage.attack & A::STATUS != 0 {
            for (element, kind) in [
                (E::FIRE, S::Burning),
                (E::COLD, S::Cold),
                (E::WATER, S::Wet),
                (E::POISON, S::Poisoned),
                (E::LIFE, S::Healing),
                (E::STEAM, S::Wet),
            ] {
                if damage.element & element.0 != 0 {
                    outcome |=
                        self.add_impact_status(kind, damage.amount, damage.magnitude, ctx, events)?;
                }
            }
        }
        damage
            .apply_resistances(
                Some(ctx.status.base),
                if ctx.status.avatar {
                    ctx.status.equipment.map(|x| x.as_slice())
                } else {
                    None
                },
                ctx.status.buffs,
            )
            .expect("full element resistance tables validated by context");
        if damage.magnitude.abs() <= f32::from_bits(1) {
            outcome |= DEFLECTED;
        }
        if outcome & DEFLECTED != 0 {
            return Ok(outcome);
        }
        if damage.attack & A::STUN != 0 && damage.amount > 0.0 {
            self.stun_timer = ctx.stun_time;
        }
        if damage.attack & A::BLEED != 0 && damage.amount > 0.0 {
            outcome |= self.add_impact_status(S::Bleeding, ctx.bleed_rate, 1.0, ctx, events)?;
        }
        if damage.attack & A::DAMAGE != 0 {
            if damage.element & E::LIGHTNING.0 != 0 {
                if self.zap_timer <= 0.0 {
                    self.zap_timer = 0.1;
                }
                if self.statuses.has(S::Wet) {
                    self.hit_by_lightning = 0.1;
                    damage.amount *= 2.0;
                }
            }
            if damage.element & E::LIFE.0 != 0 {
                let poison = &mut self.statuses.slots[4];
                poison.magnitude -= damage.magnitude;
                if poison.magnitude <= 0.0 {
                    poison.stop();
                }
            }
            if damage.element & (E::EARTH | E::ICE | E::ARCANE).0 != 0 {
                if self.statuses.has(S::Frozen) {
                    damage.amount *= 3.0;
                } else if ctx.has_blood
                    && ctx.blood_and_gore
                    && damage.amount > 0.0
                    && ctx.features & feature::EFFECTS != 0
                {
                    events.push(ImpactEvent::Gore {
                        amount: damage.amount,
                        magnitude: damage.magnitude,
                        element: E(damage.element),
                    });
                }
            }
            damage.amount *= damage.magnitude;
            damage.amount = self.health.mod_scaled(damage.amount);
            if enabled && !ctx.network_client {
                self.health.hit_points -= damage.amount;
            }
            if ctx.attacker_local_avatar {
                if enabled && damage.element == E::LIFE.0 {
                    events.push(ImpactEvent::ProfileHealed(damage.amount));
                }
                if damage.amount > 9000.0 {
                    events.push(ImpactEvent::OverNineThousand);
                }
            }
            if ctx.attacker_is_gripped {
                self.health.grip_damage += damage.amount;
                if ctx.grip_attack_state {
                    events.push(ImpactEvent::ChangeToGripping);
                }
            }
            if damage.attack & A::PIERCING != 0 && damage.magnitude > 0.0 && damage.amount > 0.0 {
                outcome |= PIERCED;
            }
            if damage.amount > 0.0 {
                outcome |= DAMAGED;
            }
            if damage.amount == 0.0 {
                outcome |= DEFLECTED;
            }
            if damage.amount < 0.0 {
                outcome |= HEALED;
            }
            if damage.amount > ctx.hit_tolerance || ctx.resurrection_state {
                outcome |= HIT;
                self.is_hit = true;
            }
            if ctx.features & feature::NOTIFY != 0 {
                if damage.amount != 0.0 {
                    self.time_since_last_damage = 0.0;
                }
                events.push(ImpactEvent::Notify {
                    amount: damage.amount,
                    element: E(damage.element),
                });
            }
        }
        if self.health.hit_points > self.health.max_hit_points {
            self.health.hit_points = self.health.max_hit_points;
        }
        let predicted = if damage.attack & A::DAMAGE != 0 {
            before - damage.amount
        } else {
            before
        };
        if damage.amount == 0.0 {
            outcome |= DEFLECTED;
        } else if damage.amount > 0.0 {
            events.push(ImpactEvent::StopHypnotize);
        }
        if predicted <= 0.0 {
            if self.statuses.has(S::Frozen)
                || ((predicted <= self.health.max_hit_points * -0.5 - 500.0 || ctx.bloat_killed)
                    && ctx.blood_and_gore)
            {
                if enabled
                    && damage.element & !E::ARCANE.0 != 0
                    && !self.overkilled
                    && !self.cannot_die_without_explicit_kill
                {
                    self.dead = true;
                    self.overkilled = true;
                    self.death_status_mask = self.statuses.current_mask;
                    self.health.invisibility_timer = 0.0;
                    self.health.hit_points = -self.health.max_hit_points - 1000.0;
                    events.push(ImpactEvent::OverkillLifecycle);
                }
                outcome |= OVERKILLED;
            }
            outcome |= KILLED;
        }
        if ctx.attacker_avatar && outcome & 0x20 != 0 && damage.element & E::FIRE.0 != 0 {
            self.last_accumulation_damager = ctx.attacker;
        }
        let statistics_result = if before <= 0.0 {
            outcome & !KILLED
        } else {
            outcome
        };
        if outcome & DAMAGED != 0 {
            if ctx.has_damage_trigger {
                events.push(ImpactEvent::DamageTrigger);
            }
            events.push(ImpactEvent::Statistics {
                damage,
                result: statistics_result,
            });
        }
        if before > 0.0 {
            self.last_attacker = ctx.attacker;
        }
        self.last_attacker_timer = 8.0;
        self.last_damage = outcome;
        Ok(outcome)
    }
    /// Original wrapper always calls all five slots, including zero entries.
    pub fn impact_collection(
        &mut self,
        damages: [Damage; 5],
        ctx: &ImpactContext<'_>,
        events: &mut Vec<ImpactEvent>,
    ) -> Result<u16, StatusError> {
        let mut context = ctx.clone();
        let mut union = 0;
        let mut intersection = u16::MAX;
        for damage in damages {
            let start = events.len();
            let r = self.impact(damage, &context, events)?;
            for event in &events[start..] {
                if let ImpactEvent::Status(event) = event {
                    context.status.cues.observe(*event);
                }
            }
            union |= r;
            intersection &= r;
        }
        Ok(union & !(intersection ^ result::DEFLECTED))
    }
}
