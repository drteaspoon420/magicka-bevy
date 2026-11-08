//! Original `Damage.ApplyResistances` and `ApplyResistancesInclusive`.
//! Static evidence and the deliberately preserved asymmetries are documented
//! in `output/research/resistance.md`. These do not apply damage to health.

use crate::damage::{attack, Damage};
use crate::spells::Elements;

/// `Magicka.GameLogic.Entities.Resistance`.
/// Indexed base/equipment tables use the slot number, not `against`, to match
/// elements. Buff entries use `against`. `status_resistance` belongs to the
/// separate Character.AddStatusEffect path and is unused by these methods.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Resistance {
    pub against: Elements,
    pub multiplier: f32,
    pub modifier: f32,
    pub status_resistance: bool,
}

impl Resistance {
    /// Identity resistance for building a full element-indexed table.
    pub const IDENTITY: Self = Self {
        against: Elements::NONE,
        multiplier: 1.0,
        modifier: 0.0,
        status_resistance: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissingAdditionalResistance {
    pub index: usize,
}

// Match System.Math's propagation of NaN and choice of the second equal value.
fn min(a: f32, b: f32) -> f32 {
    if a < b || a.is_nan() {
        a
    } else {
        b
    }
}
fn max(a: f32, b: f32) -> f32 {
    if a > b || a.is_nan() {
        a
    } else {
        b
    }
}

fn modify_amount(amount: &mut f32, modifier: f32) {
    *amount = if *amount >= 0.0 {
        max(0.0, *amount + modifier)
    } else {
        min(0.0, *amount + modifier)
    };
}

#[derive(Default)]
struct Inversion {
    life: bool,
    any: bool,
}

impl Inversion {
    fn include(&mut self, multiplier: f32, life: bool) {
        let negative = multiplier < 0.0;
        self.life |= negative && life;
        self.any |= negative;
    }
}

impl Damage {
    /// `Damage.Add`: combine flags and compute a magnitude-weighted amount.
    /// Zero total magnitude is not special-cased in the original.
    pub fn combined(self, other: Self) -> Self {
        Self {
            attack: self.attack | other.attack,
            element: self.element | other.element,
            amount: (self.amount * self.magnitude + other.amount * other.magnitude)
                / (self.magnitude + other.magnitude),
            magnitude: self.magnitude + other.magnitude,
        }
    }

    /// Multiplicative resistance path used by Character.InternalDamage.
    ///
    /// `buffs` must contain only Resistance buffs, in their original order.
    /// The original status-effects argument is unused, so it is omitted.
    /// Additional equipment is ignored when `base` is None. Missing matching
    /// equipment slots return an error at the original bounds-check point,
    /// preserving any amount changes already made (original throws).
    pub fn apply_resistances(
        &mut self,
        base: Option<&[Resistance]>,
        additional: Option<&[Resistance]>,
        buffs: &[Resistance],
    ) -> Result<(), MissingAdditionalResistance> {
        let piercing = self.attack & attack::ARMOUR_PIERCING != 0;
        let mut inversion = Inversion::default();
        let mut factor = 1.0;
        if let Some(base) = base {
            for (index, resistance) in base.iter().enumerate() {
                let element = 1u32.wrapping_shl(index as u32);
                if self.element & element == 0 {
                    continue;
                }
                // Equipment is applied immediately after its base slot.
                self.apply_indexed_resistance(
                    *resistance,
                    element,
                    piercing,
                    &mut inversion,
                    &mut factor,
                );
                if let Some(additional) = additional {
                    let resistance = additional
                        .get(index)
                        .ok_or(MissingAdditionalResistance { index })?;
                    self.apply_indexed_resistance(
                        *resistance,
                        element,
                        piercing,
                        &mut inversion,
                        &mut factor,
                    );
                }
            }
        }
        for resistance in buffs {
            if resistance.against.0 & self.element == 0 {
                continue;
            }
            if !piercing || resistance.multiplier > 1.0 {
                inversion.include(
                    resistance.multiplier,
                    resistance.against.intersects(Elements::LIFE),
                );
                // Unlike indexed resistances, buffs retain the multiplier sign.
                factor *= resistance.multiplier;
            }
            if !piercing || resistance.modifier > 0.0 {
                modify_amount(&mut self.amount, resistance.modifier);
            }
        }
        if inversion.life {
            self.amount = self.amount.abs();
        } else if inversion.any {
            self.amount = 0.0 - self.amount;
        }
        self.magnitude *= factor;
        Ok(())
    }

    fn apply_indexed_resistance(
        &mut self,
        resistance: Resistance,
        element: u32,
        piercing: bool,
        inversion: &mut Inversion,
        factor: &mut f32,
    ) {
        if !piercing || resistance.multiplier > 1.0 {
            inversion.include(resistance.multiplier, element == Elements::LIFE.0);
            *factor *= resistance.multiplier.abs();
        }
        if !piercing || resistance.modifier > 0.0 {
            modify_amount(&mut self.amount, resistance.modifier);
        }
    }

    /// `Damage.ApplyResistancesInclusive`: largest matching multiplier wins.
    /// Unlike the normal method, this ignores armour piercing and buff modifiers,
    /// does no sign correction, and starts at negative infinity. Thus no matches
    /// produce an infinite (or NaN for zero) magnitude; this is not normalized.
    pub fn apply_resistances_inclusive(
        &mut self,
        base: &[Resistance],
        additional: Option<&[Resistance]>,
        buffs: &[Resistance],
    ) -> Result<(), MissingAdditionalResistance> {
        let mut factor = f32::NEG_INFINITY;
        for (index, resistance) in base.iter().enumerate() {
            if self.element & 1u32.wrapping_shl(index as u32) == 0 {
                continue;
            }
            factor = max(factor, resistance.multiplier);
            modify_amount(&mut self.amount, resistance.modifier);
            if let Some(additional) = additional {
                let resistance = additional
                    .get(index)
                    .ok_or(MissingAdditionalResistance { index })?;
                factor = max(factor, resistance.multiplier);
                modify_amount(&mut self.amount, resistance.modifier);
            }
        }
        for resistance in buffs {
            if resistance.against.0 & self.element != 0 {
                factor = max(factor, resistance.multiplier);
            }
        }
        self.magnitude *= factor;
        Ok(())
    }
}
