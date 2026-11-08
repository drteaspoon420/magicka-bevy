use magicka_sim::{
    character_health::{SelfShield, SelfShieldType},
    damage::{attack, Damage},
    spells::{Elements as E, Spell},
    spray_pulse::HitList,
};

fn ice() -> SelfShield {
    SelfShield::from_spell(&Spell::default_for(E::SHIELD | E::ICE), &mut vec![])
}

#[test]
fn absolute_speed_gate_includes_boundary_and_preserves_unordered_comparisons() {
    for (source, target, expected) in [
        ([9.0, 0.0, 0.0], [-9.0, 0.0, 0.0], false),
        ([10.0, 0.0, 0.0], [10.0, 0.0, 0.0], true),
        ([0.0; 3], [0.0, -10.0, 0.0], true),
        ([f32::NAN, 0.0, 0.0], [0.0; 3], false),
        ([0.0; 3], [f32::NAN, 0.0, 0.0], false),
        ([10.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0], true),
        ([f32::NAN, 0.0, 0.0], [10.0, 0.0, 0.0], true),
    ] {
        let mut hits = HitList::default();
        let mut delivered = false;
        assert_eq!(
            ice().ice_contact(7, source, target, &mut hits, |_| delivered = true),
            Ok(expected)
        );
        assert_eq!(delivered, expected);
        assert_eq!(hits.contains(7), expected);
    }
}

#[test]
fn damage_uses_retained_spell_even_without_shield_health() {
    for (earth, amount) in [(0.0, 275.0), (1.0, 1250.0)] {
        let mut spell = Spell::default_for(E::SHIELD | E::ICE);
        if earth > 0.0 {
            spell.element |= E::EARTH;
            spell.mag[E::EARTH.index() as usize] = earth;
            spell.mag[E::ICE.index() as usize] = 2.0;
        }
        let mut shield = SelfShield::from_spell(&spell, &mut vec![]);
        let retained = spell;
        spell.mag.fill(0.0);
        assert_eq!(shield.spell, retained);
        shield.health = 0.0; // Native tests the stored type, not Active/health.
        let mut delivered = vec![];
        assert_eq!(
            shield.ice_contact(
                9,
                [10.0, 0.0, 0.0],
                [0.0; 3],
                &mut HitList::default(),
                |d| {
                    delivered.extend_from_slice(d);
                }
            ),
            Ok(true)
        );
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].attack, attack::DAMAGE);
        assert_eq!(delivered[0].amount, amount);
        assert_eq!(delivered[0].magnitude, 1.0);
        assert_eq!(
            delivered[0].element,
            if earth > 0.0 {
                (E::EARTH | E::ICE | E::COLD).0
            } else {
                (E::SHIELD | E::EARTH | E::ICE | E::COLD).0
            }
        );
    }
}

#[test]
fn per_target_cooldown_is_not_refreshed_by_rejected_contact() {
    let mut shield = ice();
    let mut hits = HitList::default();
    let mut deliveries: Vec<Vec<Damage>> = vec![];
    for target in [7, 8] {
        assert_eq!(
            shield.ice_contact(target, [10.0, 0.0, 0.0], [0.0; 3], &mut hits, |d| {
                deliveries.push(d.to_vec());
            }),
            Ok(true)
        );
    }
    hits.update(0.125);
    assert_eq!(
        shield.ice_contact(7, [10.0, 0.0, 0.0], [0.0; 3], &mut hits, |_| panic!(
            "cooling target"
        )),
        Ok(false)
    );
    assert_eq!(hits.remaining(7), Some(0.125));
    hits.update(0.125);
    assert_eq!(
        shield.ice_contact(7, [10.0, 0.0, 0.0], [0.0; 3], &mut hits, |d| deliveries
            .push(d.to_vec())),
        Ok(true)
    );
    assert_eq!(deliveries.len(), 3);
    for kind in [
        SelfShieldType::None,
        SelfShieldType::Earth,
        SelfShieldType::Shield,
    ] {
        shield.kind = kind;
        assert_eq!(
            shield.ice_contact(10, [10.0, 0.0, 0.0], [0.0; 3], &mut hits, |_| panic!(
                "wrong type"
            )),
            Ok(false)
        );
        assert!(!hits.contains(10));
    }
}
