use magicka_sim::spells::Elements as E;
use magicka_sim::status::{DifferentStatusTypes, Status as S, StatusEffect as Effect, StatusOwner};

fn owner() -> StatusOwner {
    StatusOwner {
        is_character: true,
        in_cutscene: false,
        hit_points: 50.0,
        max_hit_points: 100.0,
    }
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.00001, "{a} != {b}");
}

#[test]
fn constructors_transform_healing_steam_and_persistent_bleeding() {
    assert_eq!(Effect::new(S::Healing, -80.0, 4.0, false).magnitude, 2.5);
    assert_eq!(Effect::new(S::Steamed, 0.0, 8.0, false).magnitude, 1.0);
    assert_eq!(Effect::new(S::Bleeding, 20.0, 8.0, false).magnitude, 1.0);
    assert_eq!(Effect::new(S::Bleeding, 20.0, 8.0, true).magnitude, 8.0);
    assert_eq!(Effect::new(S::Cold, 0.0, 8.0, false).magnitude, 8.0);
}

#[test]
fn burning_grows_damage_and_caps_after_decaying_magnitude() {
    let mut effect = Effect::new(S::Burning, 100.0, 5.0, false);
    assert_eq!(effect.update(0.5, owner()).damage, Some((56, E::FIRE)));
    close(effect.magnitude, 2.9);
    assert_eq!(effect.dps, 112.5);
    assert_eq!(effect.damage_rest, 0.25);
    let mut effect = Effect::new(S::Burning, 2000.0, 1.0, false);
    let tick = effect.update(0.5, owner());
    assert_eq!(effect.dps, 450.0);
    assert_eq!(tick.damage, Some((225, E::FIRE)));
}

#[test]
fn cold_uses_pre_decay_slowdown_and_caps_after_decay() {
    let mut effect = Effect::new(S::Cold, 0.0, 1.0, false);
    close(effect.update(0.5, owner()).speed_limit.unwrap(), 0.333667);
    close(effect.magnitude, 0.95);
    let mut cold = Effect::new(S::Cold, 0.0, 4.0, false);
    let mut frozen = Effect::new(S::Frozen, 0.0, 4.0, false);
    cold.update(1.0, owner());
    frozen.update(1.0, owner());
    assert_eq!(cold.magnitude, 3.0);
    assert_eq!(frozen.magnitude, 2.75); // Frozen caps before decay
}

#[test]
fn wet_greased_and_ordinary_poison_do_not_decay_here() {
    for (kind, expected) in [(S::Wet, 1.0), (S::Greased, 5.0), (S::Poisoned, 1.0)] {
        let mut effect = Effect::new(kind, 0.0, 5.0, false);
        effect.update(20.0, owner());
        assert_eq!(effect.magnitude, expected);
    }
}

#[test]
fn poison_damage_uses_post_decay_magnitude_and_cutscene_gates_slowdown() {
    let mut effect = Effect::new(S::Poisoned, 20.0, 2.0, true);
    let tick = effect.update(0.5, owner());
    assert_eq!(effect.magnitude, 1.5);
    assert_eq!(tick.damage, Some((15, E::POISON)));
    assert_eq!(tick.speed_limit, Some(0.5));
    let in_cutscene = StatusOwner {
        in_cutscene: true,
        ..owner()
    };
    assert_eq!(effect.update(0.25, in_cutscene).speed_limit, None);
    let mut cold = Effect::new(S::Cold, 0.0, 1.0, false);
    assert!(cold.update(0.25, in_cutscene).speed_limit.is_some());
    let non_character = StatusOwner {
        is_character: false,
        ..owner()
    };
    assert_eq!(cold.update(0.25, non_character).speed_limit, None);
}

#[test]
fn fractional_damage_and_healing_truncate_towards_zero_and_carry_remainder() {
    for (kind, dps, element, sign) in [
        (S::Bleeding, 3.0, E::EARTH, 1),
        (S::Healing, -3.0, E::LIFE, -1),
    ] {
        let mut effect = Effect::new(kind, dps, 4.0, false);
        assert_eq!(effect.update(0.25, owner()).damage, None);
        assert_eq!(effect.update(0.25, owner()).damage, Some((sign, element)));
        assert_eq!(effect.damage_rest, sign as f32 * 0.5);
        assert_eq!(effect.update(0.25, owner()).damage, Some((sign, element)));
        assert_eq!(effect.damage_rest, sign as f32 * 0.25);
    }
}

#[test]
fn healing_emits_its_final_tick_even_at_full_health() {
    let mut effect = Effect::new(S::Healing, -80.0, 1.0, false);
    let tick = effect.update(
        0.25,
        StatusOwner {
            hit_points: 100.0,
            ..owner()
        },
    );
    assert_eq!(tick.damage, Some((-20, E::LIFE)));
    assert!(effect.dead());
    assert_eq!(effect.update(0.25, owner()).damage, None);
}

#[test]
fn expiration_is_not_clamped_before_final_damage() {
    let mut poison = Effect::new(S::Poisoned, 20.0, 0.25, true);
    assert_eq!(poison.update(0.5, owner()).damage, Some((-2, E::POISON)));
    assert_eq!(poison.magnitude, -0.25);
    assert_eq!(poison.damage_rest, -0.5);
    let mut bleeding = Effect::new(S::Bleeding, 20.0, 0.25, true);
    assert_eq!(bleeding.update(0.5, owner()).damage, Some((10, E::EARTH)));
    assert!(bleeding.dead());
    let mut steam = Effect::new(S::Steamed, 0.0, 100.0, false);
    steam.update(0.75, owner());
    assert_eq!(steam.magnitude, 0.25);
}

#[test]
fn merge_uses_burn_maximum_poison_average_and_healing_max_duration() {
    let burning = Effect::new(S::Burning, 10.0, 1.0, false)
        .merged(Effect::new(S::Burning, 20.0, 3.0, false))
        .unwrap();
    assert_eq!((burning.magnitude, burning.dps), (4.0, 20.0));
    let poison = Effect::new(S::Poisoned, 10.0, 1.0, false)
        .merged(Effect::new(S::Poisoned, 20.0, 3.0, false))
        .unwrap();
    assert_eq!((poison.magnitude, poison.dps), (4.0, 17.5));
    let healing = Effect::new(S::Healing, -80.0, 1.0, false)
        .merged(Effect::new(S::Healing, -80.0, 3.0, false))
        .unwrap();
    assert_eq!((healing.magnitude, healing.dps), (2.0, -120.0));
}

#[test]
fn merge_timed_effects_replace_but_preserve_accumulated_fractional_damage() {
    let a = Effect {
        damage_rest: 0.25,
        ..Effect::new(S::Poisoned, 10.0, 2.0, true)
    };
    let b = Effect {
        damage_rest: 0.5,
        ..Effect::new(S::Poisoned, 20.0, 3.0, true)
    };
    let out = a.merged(b).unwrap();
    assert_eq!(
        (out.magnitude, out.dps, out.damage_rest, out.effect_stops),
        (3.0, 20.0, 0.75, true)
    );
    let mixed = a
        .merged(Effect::new(S::Poisoned, 20.0, 3.0, false))
        .unwrap();
    assert!(!mixed.effect_stops);
    assert_eq!(mixed.magnitude, 5.0);
}

#[test]
fn dead_shortcuts_precede_type_check_and_stop_keeps_other_state() {
    let a = Effect::new(S::Burning, 60.0, 1.0, false);
    let b = Effect::new(S::Wet, 0.0, 1.0, false);
    assert_eq!(a.merged(b), Err(DifferentStatusTypes(S::Burning, S::Wet)));
    let mut stopped = a;
    stopped.stop();
    assert_eq!(stopped.dps, 60.0);
    assert_eq!(stopped.merged(b), Ok(b));
    assert!(a.same_kind(&stopped));
    assert_eq!(Effect::default().merged(a), Ok(a));
}
