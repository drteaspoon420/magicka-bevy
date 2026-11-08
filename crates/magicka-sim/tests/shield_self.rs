use magicka_sim::{
    shield_self::{plan, ShieldSelf},
    spells::{Elements as E, Spell},
};

fn spell(mask: u32, mags: &[(usize, f32)]) -> Spell {
    let mut s = Spell {
        element: E(mask),
        ..Default::default()
    };
    for &(i, m) in mags {
        s.mag[i] = m;
    }
    s
}

#[test]
fn pure_and_physical_shields_use_the_self_shield() {
    let p = plan(&spell(E::SHIELD.0, &[(7, 1.0)]));
    assert!(p.add_self_shield && p.auras.is_none());
    let p = plan(&spell(E::SHIELD.0 | E::EARTH.0, &[(7, 1.0), (0, 2.0)]));
    assert!(p.add_self_shield && p.auras.is_none());
}

#[test]
fn elemental_shields_add_ordered_immunity_auras() {
    // Shield + Fire×2 + Lightning: num = 2 → TTL 25, radius 3.
    let p = plan(&spell(
        E::SHIELD.0 | E::FIRE.0 | E::LIGHTNING.0,
        &[(7, 1.0), (3, 2.0), (4, 1.0)],
    ));
    assert!(!p.add_self_shield);
    let a = p.auras.unwrap();
    assert_eq!((a.ttl, a.radius, a.remove_self_shield), (25.0, 3.0, true));
    assert_eq!(a.immunities, [E::LIGHTNING, E::FIRE]);
    // Earth + Cold: self shield and a Cold aura that keeps it.
    let p = plan(&spell(
        E::SHIELD.0 | E::EARTH.0 | E::COLD.0,
        &[(0, 1.0), (2, 1.0)],
    ));
    assert!(p.add_self_shield);
    let a = p.auras.unwrap();
    assert!(!a.remove_self_shield);
    assert_eq!(a.immunities, [E::COLD]);
    // Poison alone is not an aura element.
    assert!(plan(&spell(E::SHIELD.0 | E::POISON.0, &[])).auras.is_none());
}

#[test]
fn shield_self_ends_on_animation_end_or_min_ttl() {
    let mut s = ShieldSelf::cast(true, 0.0);
    assert_eq!(s.update(0.3, false), (false, 0.1));
    assert!(s.lifecycle.active);
    s.update(0.3, false);
    assert!(!s.lifecycle.active, "mMinTTL < 0");
    let mut s = ShieldSelf::cast(true, 0.0);
    s.update(0.01, true);
    assert!(!s.lifecycle.active, "animation finished");
}
