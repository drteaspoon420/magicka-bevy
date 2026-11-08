//! Static-source regression cases, not captures from the running game.
//! Provenance and manual expectations: output/research/damage.md.
use magicka_sim::damage::{attack as A, calculate_damage, CastType as C, Damage, DamageError};
use magicka_sim::spells::{combine, Elements as E, Spell, SpellType as S};

fn d(attack: u16, element: E, amount: f32, magnitude: f32) -> Damage {
    Damage {
        attack,
        element: element.0,
        amount,
        magnitude,
    }
}

fn raw(parts: &[(E, f32)]) -> Spell {
    let mut s = Spell::default();
    for &(e, magnitude) in parts {
        s.element |= e;
        s.mag[e.index() as usize] = magnitude;
    }
    s
}

fn cast(s: &Spell, ct: C) -> Vec<Damage> {
    calculate_damage(s, s.spell_type(), ct).unwrap()
}

#[test]
fn queued_earth_uses_the_shared_spell_type_and_quadratic_damage() {
    let s = combine(&[Spell::default_for(E::EARTH); 3]);
    assert_eq!(
        cast(&s, C::Force),
        vec![d(A::DAMAGE, E::EARTH, 1350.0, 1.0)]
    );
    assert_eq!(
        cast(&s, C::Area),
        vec![
            d(A::DAMAGE, E::EARTH, 0.0, 0.0),
            d(A::KNOCKDOWN, E::EARTH, 450.0, 3.0),
        ]
    );
}

#[test]
fn water_push_depends_on_cast_and_keeps_zero_damage_entry() {
    let s = raw(&[(E::WATER, 2.0)]);
    let status = d(A::STATUS, E::WATER, 0.0, 1.0);
    for ct in [C::Force, C::Area] {
        assert_eq!(
            cast(&s, ct),
            vec![
                status,
                d(A::PUSHED, E::WATER, 140.0, 1.5),
                d(A::DAMAGE, E::WATER, 0.0, 1.0)
            ]
        );
    }
    assert_eq!(
        cast(&s, C::Weapon),
        vec![
            status,
            d(A::PUSHED, E::WATER, 140.0, 2.0),
            d(A::DAMAGE, E::WATER, 0.0, 1.0)
        ]
    );
    assert_eq!(cast(&s, C::SelfCast), vec![status]);
    let fractional = raw(&[(E::WATER, 0.125)]);
    assert_eq!(
        cast(&fractional, C::Force)[1],
        d(A::PUSHED, E::WATER, 8.0, 1.03125)
    );
}

#[test]
fn steam_spray_has_distinct_self_damage() {
    let s = raw(&[(E::STEAM, 4.0)]);
    assert_eq!(
        cast(&s, C::Force),
        vec![
            d(A::STATUS, E::STEAM, 0.0, 1.0),
            d(A::DAMAGE, E::STEAM, 560.0, 1.0)
        ]
    );
    assert_eq!(
        cast(&s, C::SelfCast),
        vec![
            d(A::STATUS, E::STEAM, 0.0, 1.0),
            d(A::DAMAGE, E::STEAM, 160.0, 1.0)
        ]
    );
}

#[test]
fn ice_projectile_preserves_the_two_life_terms() {
    let s = raw(&[(E::ICE, 1.0), (E::LIFE, 1.0)]);
    assert_eq!(
        cast(&s, C::Force),
        vec![
            d(A::DAMAGE, E::ICE | E::LIFE | E::COLD | E::EARTH, -85.0, 1.0),
            d(A::STATUS, E::LIFE, -80.0, 1.0)
        ]
    );
}

#[test]
fn weapon_projectile_keeps_piercing_and_empty_blast() {
    let s = raw(&[(E::ICE, 1.0), (E::EARTH, 1.0), (E::ARCANE, 1.0)]);
    assert_eq!(
        cast(&s, C::Weapon),
        vec![
            d(A::DAMAGE | A::PIERCING, E::EARTH, 700.0, 1.0),
            d(A::KNOCKBACK, E::EARTH | E::ARCANE, 120.0, 2.0),
            d(A::STATUS, E::ARCANE, 0.0, 0.0),
            Damage::default()
        ]
    );
}

#[test]
fn beam_caps_life_damage_but_scales_healing_status() {
    let s = raw(&[(E::LIFE, 4.0)]);
    assert_eq!(
        cast(&s, C::Force),
        vec![
            d(A::STATUS, E::LIFE, -160.0, 1.0),
            d(A::DAMAGE, E::LIFE, -180.0, 1.0)
        ]
    );
    assert_eq!(
        cast(&s, C::SelfCast),
        vec![
            d(A::STATUS, E::LIFE, -160.0, 4.0),
            d(A::DAMAGE, E::LIFE, -180.0, 1.0)
        ]
    );
    let s = raw(&[(E::ARCANE, 4.0), (E::LIGHTNING, 4.0), (E::STEAM, 1.0)]);
    assert_eq!(
        cast(&s, C::Force),
        vec![
            d(A::STATUS, E::STEAM | E::WATER, 0.0, 1.0),
            d(A::DAMAGE, s.element, 755.0, 1.0)
        ]
    );
    assert_eq!(cast(&s, C::SelfCast)[1].amount, 440.0);
}

#[test]
fn shield_uses_fixed_beam_damage_and_conditional_push() {
    let s = raw(&[(E::SHIELD, 1.0), (E::ARCANE, 4.0)]);
    assert_eq!(
        cast(&s, C::Force),
        vec![
            d(A::DAMAGE, E::ARCANE, 225.0, 1.0),
            d(A::KNOCKBACK, E::NONE, 500.0, 2.0)
        ]
    );
    let s = raw(&[
        (E::SHIELD, 1.0),
        (E::LIFE, 4.0),
        (E::EARTH, 1.0),
        (E::WATER, 4.0),
        (E::LIGHTNING, 4.0),
    ]);
    assert_eq!(
        cast(&s, C::Area),
        vec![
            d(A::DAMAGE, E::LIFE, -600.0, 1.0),
            d(A::STATUS, E::WATER | E::LIFE, 0.0, 1.0),
            d(A::PUSHED, E::WATER, 150.0, 2.0),
            d(A::DAMAGE, E::LIGHTNING, 260.0, 1.0)
        ]
    );
}

#[test]
fn lightning_keeps_all_five_entries_in_source_order() {
    let s = raw(&[(E::LIGHTNING, 1.0), (E::FIRE, 1.0)]);
    assert_eq!(
        cast(&s, C::Force),
        vec![
            d(A::DAMAGE, s.element, 250.0, 1.0),
            d(A::STATUS, E::FIRE, 60.0, 1.0),
            Damage::default(),
            Damage::default(),
            d(A::STATUS, E::FIRE, 60.0, 1.0)
        ]
    );
}

#[test]
fn every_spell_and_cast_dispatch_branch() {
    let casts = [
        C::Blast,
        C::Force,
        C::Area,
        C::SelfCast,
        C::Weapon,
        C::Magick,
    ];
    // Entry counts for a single unit element in each spell family, read from
    // the original switch branches. Not all cast values are gameplay-reachable.
    for (st, element, lengths) in [
        (S::Spray, E::FIRE, [0, 2, 2, 2, 2, 0]),
        (S::Projectile, E::EARTH, [0, 1, 2, 0, 3, 0]),
        (S::Shield, E::SHIELD, [0; 6]),
        (S::Beam, E::ARCANE, [1; 6]),
        (S::Lightning, E::LIGHTNING, [3; 6]),
    ] {
        let s = Spell::default_for(element);
        for (ct, count) in casts.into_iter().zip(lengths) {
            assert_eq!(
                calculate_damage(&s, st, ct).unwrap().len(),
                count,
                "{st:?}/{ct:?}"
            );
        }
    }
    for st in [S::Push, S::Magick] {
        for ct in casts {
            assert_eq!(
                calculate_damage(&Spell::default(), st, ct),
                Err(DamageError::NoDamageFor(st))
            );
        }
    }
}

#[test]
fn status_minimum_propagates_nan_and_does_not_clamp_negative_values() {
    let s = raw(&[(E::POISON, f32::NAN)]);
    let damages = cast(&s, C::Force);
    assert!(damages[0].amount.is_nan());
    assert!(damages[0].magnitude.is_nan());
    let s = raw(&[(E::POISON, -0.5)]);
    assert_eq!(
        cast(&s, C::Force),
        vec![d(A::STATUS, E::POISON, -25.0, -0.5)]
    );
}
