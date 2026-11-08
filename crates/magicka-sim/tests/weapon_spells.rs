use magicka_sim::{
    avatar_input::AvatarInput,
    damage::CastType,
    spells::{Elements as E, Spell},
    weapon_spells::*,
};
fn s(e: E) -> Spell {
    Spell::default_for(e)
}
#[test]
fn normalization_preserves_mask_and_ieee_zero_behavior() {
    let a = Spell::default_for_list(&[E::FIRE, E::FIRE, E::ARCANE]).normalized();
    assert_eq!(a.element, E::FIRE | E::ARCANE);
    assert_eq!(a.get(E::FIRE), 2.0 / 3.0);
    assert_eq!(a.get(E::ARCANE), 1.0 / 3.0);
    assert!(Spell::default().normalized().mag.iter().all(|x| x.is_nan()));
}
#[test]
fn growth_starts_effects_cancellation_only_stops_incoming_effect() {
    let mut w = WeaponSpells::default();
    let mut e = vec![];
    assert!(w.add(s(E::FIRE), false, &mut e));
    assert_eq!(
        e,
        [
            WeaponSpellEvent::StopEffect(3),
            WeaponSpellEvent::StartEffect(3),
            WeaponSpellEvent::RefreshColor(s(E::FIRE))
        ]
    );
    e.clear();
    assert!(!w.add(s(E::COLD), false, &mut e));
    assert!(!w.charged());
    assert!(matches!(e[0], WeaponSpellEvent::ChantMerge { .. }));
    assert!(!e
        .iter()
        .any(|x| matches!(x, WeaponSpellEvent::StartEffect(_))));
}
#[test]
fn full_queue_combination_keeps_incoming_magnitude() {
    let mut w = WeaponSpells::default();
    for _ in 0..5 {
        w.add(s(E::WATER), false, &mut vec![]);
    }
    let cold = Spell::default_for_list(&[E::COLD, E::COLD, E::COLD]);
    assert!(!w.add(cold, false, &mut vec![]));
    assert_eq!(w.queue()[4], cold + s(E::WATER));
    assert_eq!(w.queue().len(), 5);
}
#[test]
fn lightning_precedence_replaces_last_different_slot_only_when_no_match() {
    let mut w = WeaponSpells::default();
    for _ in 0..5 {
        w.add(s(E::FIRE), false, &mut vec![]);
    }
    assert!(!w.add(s(E::LIGHTNING), false, &mut vec![]));
    assert_eq!(w.queue()[4].element, E::FIRE);
    assert!(!w.add(s(E::LIGHTNING), true, &mut vec![]));
    assert_eq!(w.queue()[4], s(E::LIGHTNING));
    let spell = w.retrieve(&mut vec![]);
    assert!(spell.element.intersects(E::LIGHTNING));
    assert!(!w.charged());
}
#[test]
fn avatar_enchants_then_releases_without_resetting_prepared_spell() {
    let mut a = AvatarInput {
        queue: vec![s(E::FIRE), s(E::FIRE)],
        chanted_spells: 2,
        ..Default::default()
    };
    let mut w = WeaponSpells::default();
    let mut e = vec![];
    assert_eq!(
        a.dispatch_weapon_cast(CastType::Weapon, false, &mut w, &mut e),
        AvatarCastDispatch::Enchanted
    );
    assert!(a.queue.is_empty());
    assert_eq!(a.chanted_spells, 0);
    assert_eq!(e.last(), Some(&WeaponSpellEvent::AwardEnchanter));
    let expected = w.peek();
    e.clear();
    assert_eq!(
        a.dispatch_weapon_cast(CastType::Weapon, false, &mut w, &mut e),
        AvatarCastDispatch::Release(expected)
    );
    assert!(!w.charged());
    assert_eq!(e.len(), 12);
    assert_eq!(a.prepared, Spell::default());
}
#[test]
fn all_marker_delegates_to_base_and_remote_enchantment_skips_award() {
    let mut a = AvatarInput {
        prepared: Spell {
            element: E::ALL,
            ..Default::default()
        },
        queue: vec![s(E::FIRE)],
        ..Default::default()
    };
    let mut w = WeaponSpells::default();
    let mut e = vec![];
    assert_eq!(
        a.dispatch_weapon_cast(CastType::Weapon, false, &mut w, &mut e),
        AvatarCastDispatch::Base
    );
    assert!(e.is_empty());
    assert_eq!(a.queue.len(), 1);
    a.prepared = Spell::default();
    a.dispatch_weapon_cast(CastType::Weapon, true, &mut w, &mut e);
    assert!(!e.contains(&WeaponSpellEvent::AwardEnchanter));
}
