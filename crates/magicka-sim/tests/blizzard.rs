use magicka_sim::magicks::{AbilityEffects, BLIZZARD_SCROLL};

fn step(e: &mut AbilityEffects, dt: f32) {
    e.update(dt, 1.0, 1.0, |_, _| true, |_| (), |_| true);
}

#[test]
fn blizzard_pulses_through_fade_and_recasts_preserve_singleton_state() {
    let mut e = AbilityEffects::default();
    e.blizzard(Some(7), 12.0);
    assert_eq!(
        e.blizzard_state().unwrap().offsets,
        [[0.0, 0.5], [0.5, 0.0]]
    );
    step(&mut e, 0.125);
    assert_eq!(e.blizzard_pulse.take(), Some((Some(7), 12.0)));
    let b = *e.blizzard_state().unwrap();
    assert_eq!((b.ttl, b.alpha), (9.875, 0.125));
    for i in 0..2 {
        for j in 0..2 {
            let initial = [[0.0, 0.5], [0.5, 0.0]][i][j];
            assert!((b.offsets[i][j] - (initial - 0.125 * BLIZZARD_SCROLL[i][j])).abs() < 1e-6);
        }
    }
    e.blizzard(Some(8), 13.0);
    assert_eq!(e.effects.len(), 1);
    let new = e.blizzard_state().unwrap();
    assert_eq!((new.caster, new.timestamp, new.ttl), (Some(8), 13.0, 10.0));
    assert_eq!((new.alpha, new.offsets), (b.alpha, b.offsets));
    step(&mut e, 0.125);
    assert!(
        e.blizzard_pulse.take().is_none(),
        "recast does not reset cold timer"
    );
    step(&mut e, 0.125);
    assert_eq!(e.blizzard_pulse.take(), Some((Some(8), 13.0)));

    // Reset in a new singleton for an exactly representable 0.25 s clock.
    let mut e = AbilityEffects::default();
    e.blizzard(None, 0.0);
    for _ in 0..39 {
        step(&mut e, 0.25);
        e.blizzard_pulse.take();
    }
    assert_eq!(
        (
            e.blizzard_state().unwrap().ttl,
            e.blizzard_state().unwrap().alpha
        ),
        (0.25, 1.0)
    );
    for alpha in [0.75, 0.5, 0.25] {
        step(&mut e, 0.25);
        assert_eq!(e.blizzard_state().unwrap().alpha, alpha);
        assert_eq!(
            e.blizzard_pulse.take(),
            Some((None, 0.0)),
            "fade still freezes and chills"
        );
    }
    step(&mut e, 0.25);
    assert!(e.blizzard_state().is_none());
    assert!(
        e.blizzard_pulse.take().is_some(),
        "final frame still pulses"
    );
    e.blizzard(None, 0.0);
    assert_eq!(
        e.blizzard_state().unwrap().offsets,
        [[0.0, 0.5], [0.5, 0.0]]
    );
    assert_eq!(e.blizzard_state().unwrap().alpha, 0.0);
}

#[test]
fn clearing_blizzard_cancels_pending_pulse_but_recast_keeps_cached_alpha() {
    let mut e = AbilityEffects::default();
    e.blizzard(Some(1), 1.0);
    step(&mut e, 0.5);
    let old = *e.blizzard_state().unwrap();
    e.clear_magicks();
    assert!(e.blizzard_state().is_none() && e.blizzard_pulse.is_none());
    e.blizzard(Some(2), 2.0);
    let new = e.blizzard_state().unwrap();
    assert_eq!((new.alpha, new.offsets), (old.alpha, old.offsets));
    assert_eq!(new.ttl, 10.0);
}
