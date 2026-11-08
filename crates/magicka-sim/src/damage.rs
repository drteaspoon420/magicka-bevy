//! Spell damage calculation: `Spell.CalculateDamage(SpellType, CastType)`
//! and its helpers (`ScaleAmount`, `CalculateInstantDamage`,
//! `CalculateMiscEffects`, `CalculateStatus`, `CalculateInstantBlast`),
//! with the `Defines` constants as initialized by `Defines..cctor`.
//!
//! Statically reviewed against saved ILSpy output and disassembled CIL;
//! see `output/research/damage.md`. Runtime parity is not verified.
//! Quirks preserved: duplicated life term in the ice projectile, the zeroed
//! `CalculateInstantBlast` entry, and the 5-entry `DamageCollection5` limit
//! (the original throws).

pub use crate::spells::SpellType;
use crate::spells::{Elements as E, Spell};

/// `Magicka.AttackProperties` bits.
pub mod attack {
    pub const DAMAGE: u16 = 1;
    pub const KNOCKDOWN: u16 = 2;
    pub const PUSHED: u16 = 4;
    pub const KNOCKBACK: u16 = 6;
    pub const PIERCING: u16 = 8;
    pub const ARMOUR_PIERCING: u16 = 0x10;
    pub const STATUS: u16 = 0x20;
    pub const ENTANGLEMENT: u16 = 0x40;
    pub const STUN: u16 = 0x80;
    pub const BLEED: u16 = 0x100;
}

/// `Defines` values (static fields, set once by `Defines..cctor`).
pub mod defines {
    pub const SPELL_DAMAGE_BARRIER_LIGHTNING: f32 = 130.0;
    pub const SPELL_DAMAGE_BARRIER_ICE: f32 = 275.0;
    pub const SPELL_DAMAGE_BARRIER_LIFE: f32 = -600.0;
    pub const SPELL_DAMAGE_BARRIER_ARCANE: f32 = 225.0;
    pub const SPELL_DAMAGE_EARTH: f32 = 150.0;
    pub const SPELL_DAMAGE_ICEEARTH: f32 = 275.0;
    pub const SPELL_DAMAGE_LIGHTNING: f32 = 250.0;
    pub const SPELL_DAMAGE_LIFE: f32 = -180.0;
    pub const SPELL_DAMAGE_ARCANE: f32 = 225.0;
    pub const SPELL_DAMAGE_FIRE: f32 = 60.0;
    pub const SPELL_DAMAGE_COLD: f32 = 25.0;
    pub const SPELL_DAMAGE_STEAM: f32 = 280.0;
    pub const SPELL_STRENGTH_WATER: f32 = 70.0;
    pub const STATUS_BURN_DAMAGE: f32 = 60.0;
    pub const STATUS_POISON_DAMAGE: f32 = 50.0;
    pub const STATUS_LIFE_DAMAGE: f32 = -80.0;
    pub const SPELL_DAMAGE_SELF_FIRE: f32 = 60.0;
    pub const SPELL_DAMAGE_SELF_ARCANE: f32 = 200.0;
    pub const SPELL_DAMAGE_SELF_STEAM: f32 = 80.0;
    pub const SPELL_DAMAGE_SELF_LIGHTNING: f32 = 80.0;
}
use defines::*;

/// `Magicka.GameLogic.Damage`.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Damage {
    pub attack: u16,
    pub element: u32,
    pub amount: f32,
    pub magnitude: f32,
}

/// `Magicka.GameLogic.Spells.CastType` (`Blast` == `None` == 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum CastType {
    Blast = 0,
    Force = 1,
    Area = 2,
    SelfCast = 3,
    Weapon = 4,
    Magick = 5,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DamageError {
    /// `DamageCollection5.AddDamage` on a full collection.
    CollectionFull,
    /// The `default:` case (Push / Magick spell types).
    NoDamageFor(SpellType),
}

const STATUS_EFFECTS: u32 = 0x64E;
const INSTANT: u32 = 0x371;
const INSTANT_NON_PHYSICAL: u32 = 0x270;
const PHYSICAL: u32 = 0x101;
const BEAMS: u32 = 0x60;

/// `DamageCollection5`: at most five entries.
#[derive(Default)]
struct Collection(Vec<Damage>);

impl Collection {
    fn add(&mut self, d: Damage) -> Result<(), DamageError> {
        if self.0.len() == 5 {
            return Err(DamageError::CollectionFull);
        }
        self.0.push(d);
        Ok(())
    }
}

/// `Spell.ScaleAmount`: `base * (float)Math.Sqrt(magnitude)`.
pub fn scale_amount(base: f32, magnitude: f32) -> f32 {
    base * (magnitude as f64).sqrt() as f32
}

// System.Math.Min propagates NaN; f32::min would discard it.
fn cap_one(value: f32) -> f32 {
    if value.is_nan() || value < 1.0 {
        value
    } else {
        1.0
    }
}

/// `(int)(x)` then back to float, as the original's `Amount = (int)(...)`.
fn trunc(x: f32) -> f32 {
    (x as i32) as f32
}

struct Ctx<'a> {
    s: &'a Spell,
    el: u32,
}

impl Ctx<'_> {
    fn m(&self, e: E) -> f32 {
        self.s.get(e)
    }
    fn has(&self, mask: u32) -> bool {
        self.el & mask != 0
    }

    fn instant_damage(&self) -> Damage {
        let mut d = Damage {
            attack: attack::DAMAGE,
            ..Default::default()
        };
        if self.m(E::ICE) > 0.0 {
            d.attack |= attack::PIERCING;
        }
        d.element = E::EARTH.0;
        d.amount += scale_amount(SPELL_DAMAGE_ICEEARTH, self.m(E::ICE)) * 2.0;
        d.amount += scale_amount(SPELL_DAMAGE_EARTH, self.m(E::EARTH));
        d.magnitude = 1.0;
        d
    }

    fn misc_effects(&self, ct: CastType) -> Damage {
        let mut d = Damage::default();
        if self.has(E::WATER.0 | E::ARCANE.0) {
            d.attack = attack::PUSHED;
        }
        d.element |= self.el & E::WATER.0;
        d.magnitude += self.m(E::WATER);
        d.amount = trunc(70.0 * self.m(E::WATER));
        if ct == CastType::Weapon || ct == CastType::Area {
            d.element |= self.el & (E::EARTH.0 | E::ARCANE.0);
            d.amount += trunc(120.0 * self.m(E::ARCANE));
            d.magnitude += self.m(E::EARTH) + self.m(E::ARCANE);
            if self.m(E::EARTH) > 0.0 {
                d.attack |= attack::KNOCKDOWN;
            }
        }
        d
    }

    fn status(&self) -> Damage {
        let mut d = Damage {
            attack: attack::STATUS,
            element: self.el & (STATUS_EFFECTS | E::ARCANE.0),
            ..Default::default()
        };
        if self.m(E::STEAM) > 0.0 {
            d.element |= E::WATER.0;
        }
        d.amount += scale_amount(SPELL_DAMAGE_FIRE, self.m(E::FIRE));
        d.magnitude =
            cap_one(self.m(E::WATER) + self.m(E::FIRE) + self.m(E::COLD) + self.m(E::STEAM) * 0.5);
        d
    }

    /// Status entry shared by spray/beam/lightning (water/cold/fire/steam/poison).
    fn status_poison_burn(&self, attack_bits: u16) -> Damage {
        Damage {
            attack: attack_bits,
            element: self.el & STATUS_EFFECTS,
            amount: STATUS_POISON_DAMAGE * cap_one(self.m(E::POISON))
                + STATUS_BURN_DAMAGE * cap_one(self.m(E::FIRE)),
            magnitude: cap_one(
                self.m(E::WATER)
                    + self.m(E::FIRE)
                    + self.m(E::COLD)
                    + self.m(E::POISON)
                    + self.m(E::STEAM),
            ),
        }
    }

    fn life_status(&self, ct: CastType) -> Damage {
        let mut d = Damage {
            attack: attack::STATUS,
            element: self.el & E::LIFE.0,
            ..Default::default()
        };
        d.amount += scale_amount(STATUS_LIFE_DAMAGE, self.m(E::LIFE));
        d.magnitude = 1.0;
        if ct == CastType::SelfCast {
            d.magnitude = self.m(E::LIFE);
        }
        d
    }
}

/// `Spell.CalculateDamage(spellType, castType)`.
pub fn calculate_damage(
    s: &Spell,
    st: SpellType,
    ct: CastType,
) -> Result<Vec<Damage>, DamageError> {
    let c = Ctx { s, el: s.element.0 };
    let mut out = Collection::default();
    let status_mask = E::WATER.0 | E::COLD.0 | E::FIRE.0 | E::STEAM.0 | E::POISON.0;
    let hot = E::COLD.0 | E::FIRE.0 | E::LIGHTNING.0 | E::STEAM.0;
    match st {
        SpellType::Spray => match ct {
            CastType::Force | CastType::Area | CastType::SelfCast | CastType::Weapon => {
                let self_cast = ct == CastType::SelfCast;
                if c.has(STATUS_EFFECTS) {
                    if c.has(status_mask) {
                        out.add(c.status_poison_burn(attack::STATUS))?;
                    }
                    if c.has(E::LIFE.0) {
                        out.add(c.life_status(ct))?;
                    }
                }
                if c.has(hot) {
                    let (fire, steam) = if self_cast {
                        (SPELL_DAMAGE_SELF_FIRE, SPELL_DAMAGE_SELF_STEAM)
                    } else {
                        (SPELL_DAMAGE_FIRE, SPELL_DAMAGE_STEAM)
                    };
                    let mut d = Damage {
                        attack: attack::DAMAGE,
                        element: c.el & hot,
                        ..Default::default()
                    };
                    d.amount += scale_amount(fire, c.m(E::FIRE));
                    d.amount += scale_amount(SPELL_DAMAGE_COLD, c.m(E::COLD));
                    d.amount += scale_amount(steam, c.m(E::STEAM));
                    d.magnitude =
                        cap_one(c.m(E::FIRE) + c.m(E::COLD) + c.m(E::STEAM) + c.m(E::LIGHTNING));
                    out.add(d)?;
                }
                if !self_cast && c.m(E::WATER) > 0.0 {
                    let w = c.m(E::WATER);
                    let magnitude = if ct == CastType::Weapon {
                        w
                    } else {
                        1.0 + w / 4.0
                    };
                    out.add(Damage {
                        attack: attack::PUSHED,
                        element: E::WATER.0,
                        magnitude,
                        amount: trunc(SPELL_STRENGTH_WATER * w),
                    })?;
                    out.add(Damage {
                        attack: attack::DAMAGE,
                        element: E::WATER.0,
                        magnitude: 1.0,
                        amount: 0.0,
                    })?;
                }
            }
            CastType::Blast | CastType::Magick => {}
        },
        SpellType::Projectile => match ct {
            CastType::Force => {
                if c.m(E::EARTH) > 0.0 {
                    let mut d = Damage {
                        attack: attack::DAMAGE,
                        element: c.el & PHYSICAL,
                        ..Default::default()
                    };
                    if c.m(E::ICE) > 0.0 {
                        d.element |= E::COLD.0;
                    }
                    d.amount += SPELL_DAMAGE_EARTH * c.m(E::EARTH) * c.m(E::EARTH);
                    d.amount += SPELL_DAMAGE_ICEEARTH * c.m(E::ICE) * c.m(E::ICE);
                    d.magnitude = 1.0;
                    out.add(d)?;
                } else if c.m(E::ICE) > 0.0 {
                    let mut d = Damage {
                        attack: attack::DAMAGE,
                        element: c.el | E::COLD.0 | E::EARTH.0,
                        ..Default::default()
                    };
                    d.amount += scale_amount(SPELL_DAMAGE_ICEEARTH, c.m(E::ICE));
                    d.amount += scale_amount(SPELL_DAMAGE_LIGHTNING, c.m(E::LIGHTNING));
                    d.amount += scale_amount(SPELL_DAMAGE_ARCANE, c.m(E::ARCANE));
                    d.amount += scale_amount(SPELL_DAMAGE_LIFE, c.m(E::LIFE));
                    d.amount += scale_amount(SPELL_DAMAGE_LIFE, c.m(E::LIFE)); // duplicated in the original
                    d.magnitude = 1.0;
                    out.add(d)?;
                }
                if c.m(E::ICE) > 0.0 && c.has(STATUS_EFFECTS) {
                    let mut d = Damage {
                        attack: attack::STATUS,
                        element: c.el & STATUS_EFFECTS,
                        amount: STATUS_POISON_DAMAGE * cap_one(c.m(E::POISON)),
                        ..Default::default()
                    };
                    d.amount += STATUS_LIFE_DAMAGE * cap_one(c.m(E::LIFE));
                    d.magnitude =
                        cap_one(c.m(E::WATER) + c.m(E::COLD) + c.m(E::POISON) + c.m(E::LIFE));
                    out.add(d)?;
                }
            }
            CastType::Area => {
                if c.has(STATUS_EFFECTS) {
                    if c.has(E::WATER.0 | E::COLD.0 | E::FIRE.0 | E::STEAM.0) {
                        out.add(Damage {
                            attack: attack::STATUS,
                            element: c.el & STATUS_EFFECTS,
                            amount: STATUS_BURN_DAMAGE * cap_one(c.m(E::FIRE)),
                            magnitude: cap_one(
                                c.m(E::WATER) + c.m(E::FIRE) + c.m(E::COLD) + c.m(E::STEAM),
                            ),
                        })?;
                    }
                    if c.has(E::LIFE.0) {
                        out.add(c.life_status(ct))?;
                    }
                }
                let mut d = Damage {
                    attack: attack::DAMAGE,
                    element: (c.el & INSTANT) | E::EARTH.0,
                    ..Default::default()
                };
                d.amount += scale_amount(SPELL_DAMAGE_FIRE, c.m(E::FIRE));
                d.amount += scale_amount(SPELL_DAMAGE_COLD, c.m(E::COLD));
                d.amount += scale_amount(SPELL_DAMAGE_STEAM, c.m(E::STEAM));
                d.amount += scale_amount(SPELL_DAMAGE_LIGHTNING, c.m(E::LIGHTNING));
                d.amount += scale_amount(SPELL_DAMAGE_BARRIER_ICE, c.m(E::ICE));
                d.amount += scale_amount(SPELL_DAMAGE_LIFE, c.m(E::LIFE));
                d.amount += scale_amount(SPELL_DAMAGE_ARCANE, c.m(E::ARCANE));
                d.magnitude = cap_one(
                    c.m(E::FIRE)
                        + c.m(E::COLD)
                        + c.m(E::STEAM)
                        + c.m(E::LIGHTNING)
                        + c.m(E::ICE)
                        + c.m(E::ARCANE),
                );
                out.add(d)?;
                if c.m(E::WATER) > 0.0 {
                    let w = c.m(E::WATER);
                    out.add(Damage {
                        attack: attack::PUSHED,
                        element: E::WATER.0,
                        magnitude: w,
                        amount: trunc(SPELL_STRENGTH_WATER * w),
                    })?;
                }
                if c.m(E::EARTH) > 0.0 {
                    let e = c.m(E::EARTH);
                    out.add(Damage {
                        attack: attack::KNOCKDOWN,
                        element: E::EARTH.0,
                        magnitude: e,
                        amount: trunc(SPELL_DAMAGE_EARTH * e),
                    })?;
                }
            }
            CastType::Weapon => {
                if c.el & E::ICE.0 == E::ICE.0 {
                    out.add(c.instant_damage())?;
                }
                out.add(c.misc_effects(ct))?;
                out.add(c.status())?;
                out.add(Damage::default())?; // CalculateInstantBlast()
            }
            CastType::SelfCast | CastType::Blast | CastType::Magick => {}
        },
        SpellType::Shield => {
            let not_shield = c.el & !E::SHIELD.0;
            if not_shield & E::LIFE.0 == E::LIFE.0 {
                out.add(Damage {
                    attack: attack::DAMAGE,
                    element: c.el & BEAMS,
                    amount: SPELL_DAMAGE_BARRIER_LIFE,
                    magnitude: 1.0,
                })?;
            } else if not_shield & E::ARCANE.0 == E::ARCANE.0 {
                out.add(Damage {
                    attack: attack::DAMAGE,
                    element: E::ARCANE.0,
                    amount: SPELL_DAMAGE_BARRIER_ARCANE,
                    magnitude: 1.0,
                })?;
            }
            if c.has(STATUS_EFFECTS) {
                out.add(c.status())?;
            }
            let (water, earth, ice) = (c.m(E::WATER), c.m(E::EARTH), c.m(E::ICE));
            if water > 0.0 && (earth > 0.0 || ice > 0.0) {
                let num = (water as f64).powf(0.5) as f32;
                out.add(Damage {
                    attack: attack::PUSHED,
                    element: E::WATER.0,
                    amount: trunc(75.0 * num),
                    magnitude: num,
                })?;
            }
            if c.m(E::LIGHTNING) > 0.0 {
                out.add(Damage {
                    attack: attack::DAMAGE,
                    element: E::LIGHTNING.0,
                    magnitude: 1.0,
                    amount: scale_amount(SPELL_DAMAGE_BARRIER_LIGHTNING, c.m(E::LIGHTNING)),
                })?;
            }
            if earth + ice <= 0.0 && c.m(E::ARCANE) > 0.0 {
                out.add(Damage {
                    attack: attack::KNOCKBACK,
                    element: 0,
                    magnitude: 2.0,
                    amount: 500.0,
                })?;
            }
        }
        SpellType::Beam => {
            let self_cast = ct == CastType::SelfCast;
            if c.has(STATUS_EFFECTS) {
                if c.has(status_mask) {
                    let mut bits = attack::STATUS;
                    if c.m(E::WATER) > 0.0 {
                        bits |= attack::DAMAGE;
                    }
                    let mut d = c.status_poison_burn(bits);
                    if c.m(E::STEAM) > 0.0 {
                        d.element |= E::WATER.0;
                    }
                    out.add(d)?;
                }
                if c.has(E::LIFE.0) {
                    out.add(c.life_status(ct))?;
                }
            }
            let mut d = Damage {
                attack: attack::DAMAGE,
                element: c.el & (INSTANT_NON_PHYSICAL | E::COLD.0 | E::FIRE.0),
                ..Default::default()
            };
            if !self_cast {
                d.amount += scale_amount(SPELL_DAMAGE_FIRE, c.m(E::FIRE));
                d.amount += scale_amount(SPELL_DAMAGE_COLD, c.m(E::COLD));
                d.amount += scale_amount(SPELL_DAMAGE_LIGHTNING * 0.5, c.m(E::LIGHTNING));
                d.amount += scale_amount(SPELL_DAMAGE_STEAM, c.m(E::STEAM));
                d.amount += SPELL_DAMAGE_LIFE * cap_one(c.m(E::LIFE));
                d.amount += SPELL_DAMAGE_ARCANE * cap_one(c.m(E::ARCANE));
            } else {
                d.amount += scale_amount(SPELL_DAMAGE_SELF_FIRE, c.m(E::FIRE));
                d.amount += scale_amount(SPELL_DAMAGE_COLD, c.m(E::COLD));
                d.amount += scale_amount(SPELL_DAMAGE_SELF_LIGHTNING, c.m(E::LIGHTNING));
                d.amount += scale_amount(SPELL_DAMAGE_SELF_STEAM, c.m(E::STEAM));
                d.amount += SPELL_DAMAGE_LIFE * cap_one(c.m(E::LIFE));
                d.amount += SPELL_DAMAGE_SELF_ARCANE * cap_one(c.m(E::ARCANE));
            }
            d.magnitude = 1.0;
            out.add(d)?;
            if c.m(E::WATER) > 0.0 {
                let w = c.m(E::WATER);
                out.add(Damage {
                    attack: attack::PUSHED,
                    element: E::WATER.0,
                    magnitude: w,
                    amount: trunc(SPELL_STRENGTH_WATER * w),
                })?;
            }
        }
        SpellType::Lightning => {
            let mut d = Damage {
                attack: attack::DAMAGE,
                element: c.el,
                ..Default::default()
            };
            d.amount += scale_amount(SPELL_DAMAGE_LIGHTNING, c.m(E::LIGHTNING));
            d.amount += scale_amount(SPELL_DAMAGE_ARCANE, c.m(E::ARCANE));
            d.amount += scale_amount(SPELL_DAMAGE_LIFE, c.m(E::LIFE));
            d.magnitude = 1.0;
            out.add(d)?;
            if c.has(STATUS_EFFECTS) && c.has(status_mask) {
                let mut bits = attack::STATUS;
                if c.m(E::WATER) > 0.0 {
                    bits |= attack::DAMAGE;
                }
                out.add(c.status_poison_burn(bits))?;
            }
            out.add(Damage::default())?; // CalculateInstantBlast()
            out.add(c.misc_effects(CastType::Area))?;
            if c.has(STATUS_EFFECTS) || c.m(E::STEAM) > 0.0 {
                out.add(c.status())?;
            }
        }
        SpellType::Push | SpellType::Magick => return Err(DamageError::NoDamageFor(st)),
    }
    Ok(out.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spell(e: E, m: f32) -> Spell {
        let mut s = Spell::default_for(e);
        for x in &mut s.mag {
            *x *= m;
        }
        s
    }

    #[test]
    fn sqrt_scaling_and_earth_square() {
        let d = calculate_damage(&spell(E::FIRE, 2.0), SpellType::Spray, CastType::Force).unwrap();
        assert_eq!(d.len(), 2);
        assert!((d[1].amount - 60.0 * 2f32.sqrt()).abs() < 1e-4);
        let d = calculate_damage(
            &spell(E::EARTH, 3.0),
            SpellType::Projectile,
            CastType::Force,
        )
        .unwrap();
        assert_eq!(d[0].amount, 1350.0);
    }

    #[test]
    fn push_has_no_damage_table() {
        assert_eq!(
            calculate_damage(&Spell::default(), SpellType::Push, CastType::Force),
            Err(DamageError::NoDamageFor(SpellType::Push))
        );
    }
}
