//! Manually derived static-source regressions; see research/resistance.md.
use magicka_sim::damage::{attack as A, calculate_damage, CastType, Damage};
use magicka_sim::resistance::{MissingAdditionalResistance, Resistance as R};
use magicka_sim::spells::{Elements as E, Spell};

fn resistance(against: E, multiplier: f32, modifier: f32) -> R {
    R {
        against,
        multiplier,
        modifier,
        status_resistance: false,
    }
}
fn damage(element: E, amount: f32) -> Damage {
    Damage {
        attack: A::DAMAGE,
        element: element.0,
        amount,
        magnitude: 1.0,
    }
}
fn table(entries: &[(E, f32, f32)]) -> [R; 11] {
    let mut out = [R::IDENTITY; 11];
    for &(element, multiplier, modifier) in entries {
        out[element.index() as usize] = resistance(element, multiplier, modifier);
    }
    out
}

#[test]
fn generated_fire_damage_passes_through_base_equipment_and_buffs() {
    let spell = Spell::default_for(E::FIRE);
    let mut generated = calculate_damage(&spell, spell.spell_type(), CastType::Force).unwrap();
    let d = &mut generated[1];
    let base = table(&[(E::FIRE, 0.5, -10.0)]);
    let equipment = table(&[(E::FIRE, 0.5, -5.0)]);
    d.apply_resistances(
        Some(&base),
        Some(&equipment),
        &[resistance(E::FIRE, 2.0, 3.0)],
    )
    .unwrap();
    assert_eq!(
        *d,
        Damage {
            amount: 48.0,
            magnitude: 0.5,
            ..damage(E::FIRE, 60.0)
        }
    );
}

#[test]
fn table_slots_match_even_when_stored_masks_do_not() {
    let mut base = table(&[(E::FIRE, 0.25, -10.0)]);
    base[E::FIRE.index() as usize].against = E::WATER;
    base[E::FIRE.index() as usize].status_resistance = true;
    let mut d = damage(E::FIRE, 100.0);
    d.apply_resistances(Some(&base), None, &[]).unwrap();
    assert_eq!(d.amount, 90.0);
    assert_eq!(d.magnitude, 0.25);
}

#[test]
fn modifiers_are_applied_and_clamped_in_element_order() {
    // Fire's -200 reaches zero before Arcane's +20; aggregating modifiers
    // would incorrectly yield zero. Equipment is interleaved per element.
    let base = table(&[(E::FIRE, 1.0, -200.0), (E::ARCANE, 1.0, 20.0)]);
    let equipment = table(&[(E::FIRE, 1.0, 10.0), (E::ARCANE, 1.0, -5.0)]);
    let mut d = damage(E::FIRE | E::ARCANE, 100.0);
    d.apply_resistances(Some(&base), Some(&equipment), &[])
        .unwrap();
    assert_eq!(d.amount, 25.0);
    // Healing can reach zero and then become positive on a subsequent step.
    let base = table(&[(E::EARTH, 1.0, 200.0), (E::LIFE, 1.0, 20.0)]);
    let mut d = damage(E::EARTH | E::LIFE, -100.0);
    d.apply_resistances(Some(&base), None, &[]).unwrap();
    assert_eq!(d.amount, 20.0);
}

#[test]
fn negative_table_multipliers_invert_once_and_life_wins() {
    let base = table(&[
        (E::FIRE, -0.5, 0.0),
        (E::ARCANE, -2.0, 0.0),
        (E::LIFE, -1.0, 0.0),
    ]);
    let mut d = damage(E::FIRE | E::ARCANE, 100.0);
    d.apply_resistances(Some(&base), None, &[]).unwrap();
    assert_eq!(d.amount, -100.0); // two negatives do not cancel here
    assert_eq!(d.magnitude, 1.0);
    let mut d = damage(E::FIRE | E::LIFE, -100.0);
    d.apply_resistances(Some(&base), None, &[]).unwrap();
    assert_eq!(d.amount, 100.0);
    assert_eq!(d.magnitude, 0.5);
}

#[test]
fn negative_buff_sign_and_any_overlap_are_preserved() {
    let mut d = damage(E::FIRE, 100.0);
    d.apply_resistances(None, None, &[resistance(E::FIRE, -0.5, 0.0)])
        .unwrap();
    assert_eq!(d.amount, -100.0);
    assert_eq!(d.magnitude, -0.5); // sign is retained, unlike a base table
    let mut d = damage(E::FIRE, -100.0);
    d.apply_resistances(None, None, &[resistance(E::FIRE | E::LIFE, -0.5, 0.0)])
        .unwrap();
    assert_eq!(d.amount, 100.0); // buff mask's Life matters even for Fire input
    assert_eq!(d.magnitude, -0.5);
}

#[test]
fn armour_piercing_bypasses_protection_but_not_vulnerability() {
    let base = table(&[(E::FIRE, 0.0, -500.0), (E::ARCANE, 2.0, 10.0)]);
    let mut d = damage(E::FIRE | E::ARCANE, 100.0);
    d.attack |= A::ARMOUR_PIERCING;
    d.apply_resistances(
        Some(&base),
        None,
        &[
            resistance(E::FIRE, -2.0, -50.0),
            resistance(E::ARCANE, 3.0, 5.0),
        ],
    )
    .unwrap();
    assert_eq!(d.amount, 115.0);
    assert_eq!(d.magnitude, 6.0);
    let mut d = damage(E::FIRE, 100.0);
    d.attack |= A::PIERCING; // a different flag; does not bypass resistance
    d.apply_resistances(Some(&base), None, &[]).unwrap();
    assert_eq!(d.amount, 0.0);
    assert_eq!(d.magnitude, 0.0);
}

#[test]
fn null_base_ignores_equipment_but_still_applies_buffs() {
    let equipment = table(&[(E::FIRE, 0.0, -500.0)]);
    let mut d = damage(E::FIRE, 100.0);
    d.apply_resistances(None, Some(&equipment), &[resistance(E::FIRE, 2.0, 10.0)])
        .unwrap();
    assert_eq!(d.amount, 110.0);
    assert_eq!(d.magnitude, 2.0);
}

#[test]
fn short_additional_table_fails_only_on_a_matching_slot() {
    let base = table(&[(E::FIRE, 0.5, -10.0)]);
    let mut d = damage(E::FIRE, 100.0);
    assert_eq!(
        d.apply_resistances(Some(&base), Some(&[]), &[]),
        Err(MissingAdditionalResistance { index: 3 })
    );
    assert_eq!(d.amount, 90.0); // base modifier already applied
    assert_eq!(d.magnitude, 1.0); // accumulated multiplier not committed
    let mut d = damage(E::NONE, 100.0);
    assert!(d.apply_resistances(Some(&base), Some(&[]), &[]).is_ok());
}

#[test]
fn inclusive_uses_maximum_ignores_buff_modifiers_and_piercing() {
    let base = table(&[(E::FIRE, 0.25, -10.0), (E::ARCANE, 0.5, -5.0)]);
    let equipment = table(&[(E::FIRE, 0.75, -2.0), (E::ARCANE, 0.25, 0.0)]);
    let mut d = damage(E::FIRE | E::ARCANE, 100.0);
    d.attack |= A::ARMOUR_PIERCING;
    d.apply_resistances_inclusive(
        &base,
        Some(&equipment),
        &[
            resistance(E::FIRE, 0.9, -999.0),
            resistance(E::WATER, 100.0, 100.0),
        ],
    )
    .unwrap();
    assert_eq!(d.amount, 83.0);
    assert_eq!(d.magnitude, 0.9);
}

#[test]
fn inclusive_no_match_and_negative_multipliers_are_not_repaired() {
    let mut d = damage(E::NONE, 100.0);
    d.apply_resistances_inclusive(&[], None, &[]).unwrap();
    assert_eq!(d.magnitude, f32::NEG_INFINITY);
    let mut d = damage(E::FIRE, 100.0);
    d.apply_resistances_inclusive(&table(&[(E::FIRE, -2.0, 0.0)]), None, &[])
        .unwrap();
    assert_eq!(d.amount, 100.0);
    assert_eq!(d.magnitude, -2.0);
}

#[test]
fn weighted_damage_addition_retains_flags_and_zero_division() {
    let a = Damage {
        magnitude: 2.0,
        ..damage(E::FIRE, 10.0)
    };
    let b = Damage {
        attack: A::STATUS,
        magnitude: 3.0,
        ..damage(E::COLD, 20.0)
    };
    assert_eq!(
        a.combined(b),
        Damage {
            attack: A::DAMAGE | A::STATUS,
            element: (E::FIRE | E::COLD).0,
            amount: 16.0,
            magnitude: 5.0,
        }
    );
    assert!(Damage::default()
        .combined(Damage::default())
        .amount
        .is_nan());
}
