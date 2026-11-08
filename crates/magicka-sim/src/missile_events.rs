//! Ordered native missile condition/event recipes. See research/missile-events.md.
use crate::{
    damage::{attack, calculate_damage, CastType, Damage, DamageError},
    impact_motion::Vector,
    spells::{Elements as E, Spell, SpellType},
};
pub mod condition {
    pub const DEFAULT: u8 = 1;
    pub const HIT: u8 = 2;
    pub const COLLISION: u8 = 4;
    pub const DAMAGED: u8 = 8;
    pub const TIMER: u8 = 16;
    pub const DEATH: u8 = 32;
    pub const OVERKILL: u8 = 64;
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EventArgs {
    pub flags: u8,
    pub hitpoints: f32,
    pub elements: u32,
    pub time: f32,
    pub threshold: f32,
    pub position: Option<Vector>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EventCondition {
    pub flags: u8,
    pub hitpoints: f32,
    pub elements: u32,
    pub time: f32,
    pub threshold: f32,
    pub count: i32,
    pub activated: bool,
    pub repeat: bool,
}
impl EventCondition {
    pub fn is_met(&mut self, args: &EventArgs) -> bool {
        let flags = args.flags & self.flags;
        if flags == 0 || (self.activated && !self.repeat) {
            return false;
        }
        for bit in 0..7 {
            let flag = (1 << bit) & flags;
            let met = match flag {
                condition::DEFAULT | condition::HIT | condition::DEATH | condition::OVERKILL => {
                    true
                }
                condition::COLLISION => args.threshold >= self.threshold,
                condition::DAMAGED => {
                    args.hitpoints >= self.hitpoints && args.elements & self.elements != 0
                }
                condition::TIMER if args.time >= self.time * self.count.wrapping_add(1) as f32 => {
                    self.count = self.count.wrapping_add(1);
                    true
                }
                _ => false,
            };
            if met {
                self.activated = true;
                return true;
            }
        }
        false
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectTable {
    TrailBig,
    TrailSmall,
    HitBig,
    HitSmall,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundTable {
    Projectile,
    Blast,
    Hit,
}
#[derive(Clone, Debug, PartialEq)]
pub enum MissileEvent {
    Damage {
        damage: Damage,
        velocity_based: bool,
    },
    Remove {
        bounces: i32,
    },
    Crater {
        scale: f32,
    },
    Blast {
        radius: f32,
        damages: [Damage; 5],
    },
    /// SplashEvent uses CircleDamage with one damage slot.
    Splash {
        radius: f32,
        damage: Damage,
    },
    Effect {
        table: EffectTable,
        element: E,
        follow: bool,
    },
    Sound {
        table: SoundTable,
        element: E,
        magnitude: f32,
        stop_on_remove: bool,
    },
    Light {
        radius: f32,
        diffuse: Vector,
        ambient: Vector,
        intensity: f32,
    },
    /// PlayEffectEvent with a named effect (ProppMagick, JudgementSpray):
    /// world aligned (Forward) or along the missile's facing.
    NamedEffect {
        name: &'static str,
        follow: bool,
        world_align: bool,
    },
    /// PlaySoundEvent with a named cue and its authored Magnitude variable.
    NamedSound {
        bank: u16,
        name: &'static str,
        magnitude: f32,
        stop_on_remove: bool,
    },
}

/// ProppMagick.sDamage: Arcane 6000 and Fire 2000 Damage, Earth 200
/// Knockback (magnitude 3).
pub fn propp_damage() -> [Damage; 5] {
    let mut d = [Damage::default(); 5];
    d[0] = Damage {
        attack: attack::DAMAGE,
        element: E::ARCANE.0,
        amount: 6000.0,
        magnitude: 1.0,
    };
    d[1] = Damage {
        attack: attack::DAMAGE,
        element: E::FIRE.0,
        amount: 2000.0,
        magnitude: 1.0,
    };
    d[2] = Damage {
        attack: attack::KNOCKBACK,
        element: E::EARTH.0,
        amount: 200.0,
        magnitude: 3.0,
    };
    d
}

/// ProppMagick.Spawn's condition collection after Clear (every slot
/// repeatable): Collision and Hit play the explosion, its sound, Remove and
/// a 4 m blast; the 8 s Timer only explodes and removes, because the
/// original adds its BlastEvent to the Hit slot (which therefore blasts
/// twice); Default loops the plasma sound.
pub fn propp_conditions() -> MissileConditions {
    const BANK_ADDITIONAL: u16 = 0x200;
    let mut c = MissileConditions::default();
    c.clear();
    let explode = || {
        vec![
            MissileEvent::NamedEffect {
                name: "magick_propp_explosion",
                follow: false,
                world_align: true,
            },
            MissileEvent::NamedSound {
                bank: BANK_ADDITIONAL,
                name: "magick_propps_plasma_explosion",
                magnitude: 1.,
                stop_on_remove: false,
            },
            MissileEvent::Remove { bounces: 0 },
        ]
    };
    let blast = MissileEvent::Blast {
        radius: 4.0,
        damages: propp_damage(),
    };
    c.slots[0].condition.flags = condition::COLLISION;
    c.slots[0].events = explode();
    c.slots[0].events.push(blast.clone());
    c.slots[1].condition.flags = condition::HIT;
    c.slots[1].events = explode();
    c.slots[1].events.push(blast.clone());
    c.slots[2].condition.flags = condition::TIMER;
    c.slots[2].condition.time = 8.0;
    c.slots[2].events = explode();
    c.slots[1].events.push(blast);
    c.slots[3].condition.flags = condition::DEFAULT;
    c.slots[3].condition.repeat = true;
    c.slots[3].events = vec![MissileEvent::NamedSound {
        bank: BANK_ADDITIONAL,
        name: "magick_propps_plasma_loop",
        magnitude: 1.,
        stop_on_remove: true,
    }];
    c
}
/// The three thrown staff grenades (Grenade, HolyGrenade, Fireflask).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrenadeKind {
    Liberty,
    Holy,
    Fireflask,
}

/// Grenade/HolyGrenade/Fireflask.SpawnGrenade's conditions after Clear:
/// Damaged (Fire|Lightning|Arcane, 20 HP) and a 3 s Timer explode
/// (effect, sound, two blasts, Remove); Fireflask also explodes on a 20+
/// Collision; Default (repeating) starts the followed trail effect.
pub fn grenade_conditions(kind: GrenadeKind) -> MissileConditions {
    const BANK_ADDITIONAL: u16 = 0x200;
    const BANK_SPELLS: u16 = 16;
    let one = |d: Damage| {
        let mut damages = [Damage::default(); 5];
        damages[0] = d;
        damages
    };
    let (effect, bank, sound, radius, first, second) = match kind {
        GrenadeKind::Fireflask => (
            "fireflask_explosion",
            BANK_SPELLS,
            "spell_fire_force",
            2.0,
            Damage {
                attack: attack::STATUS,
                element: E::FIRE.0,
                amount: 30.0,
                magnitude: 1.0,
            },
            Damage {
                attack: attack::DAMAGE,
                element: E::FIRE.0,
                amount: 470.0,
                magnitude: 1.0,
            },
        ),
        _ => (
            if kind == GrenadeKind::Holy {
                "holy_explosion"
            } else {
                "liberty_explosion"
            },
            BANK_ADDITIONAL,
            "wep_handgrenade_explode",
            6.0,
            Damage {
                attack: attack::KNOCKBACK,
                element: E::EARTH.0,
                amount: 750.0,
                magnitude: 2.0,
            },
            Damage {
                attack: attack::DAMAGE,
                element: E(E::EARTH.0 | E::FIRE.0).0,
                amount: 800.0,
                magnitude: 2.0,
            },
        ),
    };
    let explode = |world_align: bool| {
        vec![
            MissileEvent::NamedEffect {
                name: effect,
                follow: false,
                world_align,
            },
            MissileEvent::NamedSound {
                bank,
                name: sound,
                magnitude: 1.0,
                stop_on_remove: false,
            },
            MissileEvent::Blast {
                radius,
                damages: one(first),
            },
            MissileEvent::Blast {
                radius,
                damages: one(second),
            },
            MissileEvent::Remove { bounces: 0 },
        ]
    };
    let mut c = MissileConditions::default();
    c.clear();
    c.slots[0].condition.flags = condition::DAMAGED;
    c.slots[0].condition.elements = E::FIRE.0 | E::LIGHTNING.0 | E::ARCANE.0;
    c.slots[0].condition.hitpoints = 20.0;
    c.slots[0].events = explode(true);
    c.slots[1].condition.flags = condition::TIMER;
    c.slots[1].condition.time = 3.0;
    c.slots[1].events = explode(true);
    c.slots[2].condition.flags = condition::DEFAULT;
    c.slots[2].condition.repeat = true;
    c.slots[2].events = vec![MissileEvent::NamedEffect {
        name: "vietnam_grenade_trail",
        follow: true,
        world_align: false,
    }];
    if kind == GrenadeKind::Fireflask {
        c.slots[3].condition.flags = condition::COLLISION;
        c.slots[3].condition.threshold = 20.0;
        c.slots[3].events = explode(false);
    }
    c
}

pub trait MissileEventHost {
    fn damageable_target(&self) -> bool;
    fn normalized_velocity(&self) -> f32;
    fn damage(&mut self, damage: Damage) -> u16;
    fn kill(&mut self);
    fn event(&mut self, event: &MissileEvent, position: &mut Option<Vector>);
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventCollection {
    pub condition: EventCondition,
    pub events: Vec<MissileEvent>,
}
impl EventCollection {
    pub fn execute(
        &mut self,
        args: &mut EventArgs,
        host: &mut impl MissileEventHost,
    ) -> (bool, u16) {
        if !self.condition.is_met(args) {
            return (false, 0);
        }
        let mut result = 0;
        for event in &mut self.events {
            match event {
                MissileEvent::Damage {
                    damage,
                    velocity_based,
                } => {
                    if !host.damageable_target() {
                        continue;
                    }
                    let mut d = *damage;
                    if *velocity_based {
                        d.amount *= host.normalized_velocity();
                    }
                    result |= host.damage(d);
                }
                MissileEvent::Remove { bounces } => {
                    if *bounces <= 0 {
                        host.kill();
                    } else {
                        *bounces -= 1;
                    }
                }
                _ => host.event(event, &mut args.position),
            }
        }
        (!self.events.is_empty(), result)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct MissileConditions {
    pub slots: [EventCollection; 5],
}
impl Default for MissileConditions {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| EventCollection {
                condition: EventCondition {
                    flags: condition::DEFAULT,
                    ..Default::default()
                },
                events: vec![],
            }),
        }
    }
}
impl MissileConditions {
    /// Native Clear resets activation/count and marks every slot repeatable.
    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            slot.condition = EventCondition {
                repeat: true,
                ..Default::default()
            };
            slot.events.clear();
        }
    }
    pub fn execute(
        &mut self,
        args: &mut EventArgs,
        host: &mut impl MissileEventHost,
    ) -> (bool, u16) {
        let (mut any, mut result) = (false, 0);
        for slot in &mut self.slots {
            let (a, r) = slot.execute(args, host);
            any |= a;
            result |= r;
        }
        (any, result)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissileModel {
    Earth(u8),
    IceEarth(u8),
    Ice3,
}
#[derive(Clone, Debug, PartialEq)]
pub enum RecipeError {
    Damage(DamageError),
    InvalidModelMagnitude,
}
impl From<DamageError> for RecipeError {
    fn from(error: DamageError) -> Self {
        Self::Damage(error)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct MissileRecipe {
    pub model: Option<MissileModel>,
    pub radius: f32,
    pub can_hit_owner: bool,
    pub conditions: MissileConditions,
}
fn damage_slots(
    spell: &Spell,
    kind: SpellType,
    cast: CastType,
) -> Result<[Damage; 5], DamageError> {
    let mut slots = [Damage::default(); 5];
    for (s, d) in slots.iter_mut().zip(calculate_damage(spell, kind, cast)?) {
        *s = d;
    }
    Ok(slots)
}
fn sound(table: SoundTable, element: E, magnitude: f32, stop_on_remove: bool) -> MissileEvent {
    MissileEvent::Sound {
        table,
        element,
        magnitude,
        stop_on_remove,
    }
}
fn effect(table: EffectTable, element: E, follow: bool) -> MissileEvent {
    MissileEvent::Effect {
        table,
        element,
        follow,
    }
}
impl MissileRecipe {
    pub fn from_spell(
        spell: Spell,
        splash: f32,
        total_projectiles: i32,
    ) -> Result<Self, RecipeError> {
        let mut damages = damage_slots(&spell, SpellType::Projectile, CastType::Force)?;
        let earth = spell.get(E::EARTH);
        let ice = spell.get(E::ICE);
        for magnitude in [earth, ice] {
            if !magnitude.is_finite()
                || magnitude as f64 >= 2147483648.
                || (magnitude as f64) < -2147483648.
            {
                return Err(RecipeError::InvalidModelMagnitude);
            }
        }
        let both = spell.element.has(E::PHYSICAL_ELEMENTS);
        let earth_mask = spell.element.has(E::EARTH);
        let ice_only = !both && !earth_mask && spell.element.has(E::ICE);
        let model = if both {
            let index = ((ice as i32).wrapping_add(earth as i32).wrapping_sub(1)).clamp(0, 4);
            Some(MissileModel::IceEarth([0, 1, 2, 2, 3][index as usize]))
        } else if earth_mask {
            let index = (earth as i32).wrapping_sub(1).clamp(0, 4);
            Some(MissileModel::Earth([0, 1, 2, 2, 3][index as usize]))
        } else if ice_only {
            Some(MissileModel::Ice3)
        } else {
            None
        };
        let mut c = MissileConditions::default();
        c.clear();
        c.slots[0].condition.flags = condition::DEFAULT;
        c.slots[1].condition.flags = condition::HIT;
        c.slots[2].condition.flags = condition::COLLISION;
        c.slots[3].condition.flags = condition::TIMER;
        c.slots[3].condition.time = 10.;
        c.slots[3].events.push(MissileEvent::Remove { bounces: 0 });
        if ice_only {
            for d in &mut damages {
                d.attack |= attack::PIERCING;
                d.magnitude *= 1. / total_projectiles as f32;
            }
            for e in E::BY_INDEX {
                if spell.get(e) > 0. {
                    c.slots[0]
                        .events
                        .push(effect(EffectTable::TrailSmall, e, true));
                    c.slots[1]
                        .events
                        .push(effect(EffectTable::HitSmall, e, false));
                    c.slots[2]
                        .events
                        .push(effect(EffectTable::HitSmall, e, false));
                }
            }
            for (i, d) in damages[..4].iter().enumerate() {
                c.slots[1].events.push(MissileEvent::Damage {
                    damage: *d,
                    velocity_based: i == 0,
                });
            }
            c.slots[1]
                .events
                .push(sound(SoundTable::Hit, E::ICE, 1., false));
            c.slots[2]
                .events
                .push(sound(SoundTable::Hit, E::ICE, 1., false));
            c.slots[2].events.push(MissileEvent::Remove { bounces: 0 });
        } else {
            for (i, d) in damages.iter().enumerate() {
                c.slots[1].events.push(MissileEvent::Damage {
                    damage: *d,
                    velocity_based: i == 0,
                });
            }
            c.slots[2].events.push(MissileEvent::Crater {
                scale: if both {
                    earth + ice - 1.
                } else {
                    1. + (earth - 1.) * 0.5
                },
            });
            if both {
                c.slots[1].events.push(MissileEvent::Remove { bounces: 0 });
                c.slots[2].events.push(MissileEvent::Remove { bounces: 1 });
            } else if splash > f32::from_bits(1) {
                let mut stripped = spell;
                stripped.mag[0] = 0.;
                stripped.element = E(stripped.element.0 & !E::EARTH.0);
                if stripped.element != E::NONE {
                    let blast = MissileEvent::Blast {
                        radius: splash,
                        damages: damage_slots(&spell, stripped.spell_type(), CastType::Area)?,
                    };
                    c.slots[1].events.push(blast.clone());
                    c.slots[2].events.push(blast);
                }
                c.slots[1].events.push(MissileEvent::Remove { bounces: 0 });
                c.slots[2].events.push(MissileEvent::Remove { bounces: 0 });
            } else {
                c.slots[1].events.push(MissileEvent::Remove { bounces: 25 });
                c.slots[2].events.push(MissileEvent::Remove { bounces: 1 });
            }
            for e in E::BY_INDEX {
                let magnitude = spell.get(e);
                if magnitude > 0. {
                    c.slots[0].events.push(sound(
                        SoundTable::Projectile,
                        e,
                        if both { magnitude } else { 1. },
                        false,
                    ));
                    c.slots[0]
                        .events
                        .push(effect(EffectTable::TrailBig, e, true));
                    if both {
                        c.slots[1]
                            .events
                            .push(sound(SoundTable::Blast, e, magnitude, false));
                        c.slots[2].events.extend([
                            effect(EffectTable::HitBig, e, false),
                            effect(EffectTable::HitBig, e, false),
                            sound(SoundTable::Blast, e, magnitude, false),
                        ]);
                    } else if splash > 0. {
                        c.slots[1]
                            .events
                            .push(sound(SoundTable::Blast, e, magnitude, false));
                        c.slots[2]
                            .events
                            .push(sound(SoundTable::Blast, e, magnitude, false));
                    } else {
                        c.slots[2].events.extend([
                            effect(EffectTable::HitBig, e, false),
                            sound(SoundTable::Hit, e, magnitude, false),
                        ]);
                    }
                }
            }
        }
        let mut color = [0.; 3];
        let mut count = 0.;
        for (e, rgb) in [
            (E::FIRE, [1.8, 0.6, 0.4]),
            (E::ARCANE, [2., 0.4, 0.6]),
            (E::LIFE, [0.2, 1.6, 0.2]),
            (E::LIGHTNING, [0.75, 0.5, 1.]),
            (E::COLD, [1., 1., 1.4]),
        ] {
            if spell.get(e) > f32::from_bits(1) {
                for i in 0..3 {
                    color[i] += rgb[i];
                }
                count += 1.;
            }
        }
        if count > f32::from_bits(1) {
            for v in &mut color {
                *v /= count;
            }
            c.slots[0].events.push(MissileEvent::Light {
                radius: 6.,
                diffuse: color.map(|v| v * 1.333),
                ambient: color.map(|v| v * 0.666),
                intensity: ((color.iter().map(|v| v * v).sum::<f32>()) as f64).sqrt() as f32,
            });
        }
        Ok(Self {
            model,
            radius: 0.075 + 0.075 * earth,
            can_hit_owner: true,
            conditions: c,
        })
    }
}

/// JudgementSpray.SpawnProjectile's conditions after Clear: Damaged (20 hit
/// points of Fire/Lightning/Arcane), Hit, Collision (threshold 0) and the
/// 2 s Timer each play the hit effect and sound, deal Cold 273 Damage and
/// Cold 100 Status to the struck target, and remove the missile; Default
/// repeats the trail and missile effects (followed).
pub fn judgement_conditions() -> MissileConditions {
    const BANK_ADDITIONAL: u16 = 0x200;
    let mut c = MissileConditions::default();
    c.clear();
    let hit = |world_align: bool| {
        vec![
            MissileEvent::NamedEffect {
                name: "magick_judgementspray_hit",
                follow: false,
                world_align,
            },
            MissileEvent::NamedSound {
                bank: BANK_ADDITIONAL,
                name: "wep_handgrenade_explode",
                magnitude: 1.,
                stop_on_remove: false,
            },
            MissileEvent::Damage {
                damage: Damage {
                    attack: attack::DAMAGE,
                    element: E::COLD.0,
                    amount: 273.0,
                    magnitude: 1.0,
                },
                velocity_based: false,
            },
            MissileEvent::Damage {
                damage: Damage {
                    attack: attack::STATUS,
                    element: E::COLD.0,
                    amount: 100.0,
                    magnitude: 1.0,
                },
                velocity_based: false,
            },
            MissileEvent::Remove { bounces: 0 },
        ]
    };
    c.slots[0].condition.flags = condition::DAMAGED;
    c.slots[0].condition.elements = E::FIRE.0 | E::LIGHTNING.0 | E::ARCANE.0;
    c.slots[0].condition.hitpoints = 20.0;
    c.slots[0].events = hit(true);
    c.slots[1].condition.flags = condition::HIT;
    c.slots[1].events = hit(true);
    c.slots[2].condition.flags = condition::DEFAULT;
    c.slots[2].condition.repeat = true;
    c.slots[2].events = vec![
        MissileEvent::NamedEffect {
            name: "magick_judgementspray_trail",
            follow: true,
            world_align: false,
        },
        MissileEvent::NamedEffect {
            name: "magick_judgementspray_missile",
            follow: true,
            world_align: false,
        },
    ];
    c.slots[3].condition.flags = condition::COLLISION;
    c.slots[3].condition.threshold = 0.0;
    c.slots[3].events = hit(false);
    c.slots[4].condition.flags = condition::TIMER;
    c.slots[4].condition.time = 2.0;
    c.slots[4].events = hit(true);
    c
}
