use magicka_sim::{damage::Damage, elemental_egg::*, spells::Elements as E};

fn damage(element: E, amount: f32) -> Damage {
    Damage {
        element: element.0,
        amount,
        magnitude: 1.0,
        attack: 0,
    }
}

#[test]
fn memory_overwrites_instead_of_accumulating_and_filters_composite_elements() {
    let mut egg = ElementalEgg::default();
    assert!(!egg.internal_damage(damage(E::EARTH, 1000.0)));
    assert!(egg.internal_damage(damage(E::ARCANE, 100.0)));
    assert!(egg.internal_damage(damage(E::COLD, 50.0)));
    // The weaker latest Arcane hit replaces its earlier strong memory.
    egg.internal_damage(damage(E::ARCANE, 20.0));
    egg.update(0.0, 0.0, false, |_| panic!("no proximity scan"));
    assert_eq!(egg.hatch_element(false), Some(E::COLD));
    assert_eq!(egg.hatch_element(true), None);
    egg.initialize();
    // Accepted bits get the complete amount, with no split or attack gate.
    egg.internal_damage(damage(E(E::EARTH.0 | E::WATER.0 | E::FIRE.0), 30.0));
    egg.update(0.0, 0.0, false, |_| unreachable!());
    assert_eq!(
        egg.spawn_element,
        E::WATER,
        "strict tie keeps first enumerated accepted bit"
    );
    egg.internal_damage(damage(E::FIRE, 31.0));
    egg.update(0.0, 0.0, false, |_| unreachable!());
    assert_eq!(egg.spawn_element, E::FIRE);
}

#[test]
fn proximity_overrides_memory_after_strict_one_second_and_reuse_retains_resting_state() {
    let mut egg = ElementalEgg::default();
    egg.proximity = 5.0;
    egg.internal_damage(damage(E::ARCANE, 100.0));
    egg.update(1.0, 0.0, true, |_| panic!("boundary does not scan"));
    assert!(!egg.resting());
    egg.update(0.01, 0.0, true, |bound| {
        assert_eq!(bound, 8);
        4
    });
    assert!(egg.resting());
    assert_eq!(egg.spawn_element, E::LIFE);
    egg.summoner = Some(4);
    egg.initialize();
    assert_eq!(egg.proximity, 5.0);
    assert!(egg.resting());
    assert_eq!(egg.summoner, None);
    assert!(!egg.dead());
    egg.update(3.0, 1e-5, false, |_| unreachable!());
    assert_eq!(
        egg.proximity_timer, 2.0,
        "only one second is subtracted per update"
    );
    assert_eq!(egg.resting_timer, 1.0);
    egg.kill();
    assert!(egg.dead());
    assert_eq!(
        egg.hatch_element(false),
        None,
        "Shield kill marker does not hatch"
    );
}

#[test]
fn placement_preserves_positive_quadrant_and_radius_only_ground_offset() {
    let crossing = magicka_sim::spray_pulse::Segment {
        origin: [-2.0, 1.0, 0.0],
        delta: [4.0, 0.0, 0.0],
    };
    assert_eq!(
        segment_intersect([0.0; 3], 2.0, 0.5, crossing, 0.0),
        Some([0.0, 1.0, 0.0])
    );
    let direction = random_direction(0.3, 0.4);
    assert!((direction[0] - 0.6).abs() < 1e-6);
    assert!((direction[2] - 0.8).abs() < 1e-6);
    assert_eq!(
        cast_point([1.0, 2.0, 3.0], [0.0, 0.0, -1.0]),
        [1.0, 2.0, 0.0]
    );
    assert_eq!(egg_position([1.0, 2.0, 3.0], None, 0.5), [1.0, 2.5, 3.0]);
    assert_eq!(
        egg_position([1.0, 2.0, 3.0], Some([1.0, 0.0, 3.0]), 0.5),
        [1.0, 0.5, 3.0]
    );
}
