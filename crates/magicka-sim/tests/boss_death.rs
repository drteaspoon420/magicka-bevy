use magicka_sim::{
    boss_death::*,
    character_impact::result,
    damage::{attack, Damage},
    spells::Elements as E,
};

const CORPOREAL: Vulnerability = Vulnerability {
    drawn: true,
    ethereal: false,
    fake_multi_attack: false,
};
fn life(amount: f32, magnitude: f32) -> Damage {
    Damage {
        attack: attack::DAMAGE,
        element: E::LIFE.0,
        amount,
        magnitude,
    }
}

#[test]
fn life_uses_absolute_amount_ignores_magnitude_features_and_always_deflects() {
    let mut death = DeathCombat::default();
    for (amount, magnitude, features, first) in [
        (-200., 0., 0, true),
        (100., 100., 15, false),
        (0., -1., 0, false),
    ] {
        let (result, hit) = death.damage(life(amount, magnitude), CORPOREAL, features);
        assert_eq!(result, result::DEFLECTED);
        assert_eq!(
            hit,
            Some(Hit {
                amount: amount.abs(),
                first_hit: first
            })
        );
    }
    assert_eq!(death.hit_points(), 9700.);
    assert!(death.hit_pending);
    assert_eq!(death.flash_timer, 0.1);
    death.age_flash(0.025);
    assert!((death.flash_timer - 0.075).abs() < 1e-6);
    death.exit_hit();
    assert!(
        death
            .damage(life(1., 1.), CORPOREAL, 15)
            .1
            .unwrap()
            .first_hit
    );
}

#[test]
fn hidden_ethereal_fake_and_nonlife_damage_do_not_open_a_hit_window() {
    let mut death = DeathCombat::default();
    for visibility in [
        Vulnerability {
            drawn: false,
            ..CORPOREAL
        },
        Vulnerability {
            ethereal: true,
            ..CORPOREAL
        },
        Vulnerability {
            fake_multi_attack: true,
            ..CORPOREAL
        },
    ] {
        assert_eq!(
            death.damage(life(20000., 1.), visibility, 15),
            (result::DEFLECTED, None)
        );
    }
    for damage in [
        Damage {
            element: E::FIRE.0,
            ..life(20000., 1.)
        },
        Damage {
            attack: attack::STATUS,
            ..life(20000., 1.)
        },
        life(f32::NAN, 1.),
    ] {
        assert_eq!(
            death.damage(damage, CORPOREAL, 15),
            (result::DEFLECTED, None)
        );
    }
    assert_eq!(death.hit_points(), MAX_HIT_POINTS);
    assert!(!death.hit_pending);
}

#[test]
fn lethal_life_waits_at_one_until_defeat_animation_enters_seated_state() {
    let mut death = DeathCombat::default();
    let damage = Damage {
        element: E::LIFE.0 | E::FIRE.0,
        ..life(f32::INFINITY, 0.)
    };
    assert!(death.damage(damage, CORPOREAL, 0).1.is_some());
    assert_eq!(death.hit_points(), 1.);
    assert!(!death.defeated());
    assert!(death.begin_defeat());
    assert!(death.defeated());
    assert!(!death.begin_defeat());
    death.age_flash(100.);
    assert_eq!(
        death.hit_points(),
        1.,
        "elapsed time cannot finish the fight"
    );
    assert!(!death.dead());
    death.enter_defeat();
    assert_eq!(death.hit_points(), 0.);
    assert!(!death.dead(), "native outro uses boss health, not Dead");
}

#[test]
fn prep_branch_order_reuses_low_rolls_when_a_previous_ability_is_excluded() {
    use Ability::*;
    assert_eq!(choose_ability(0, 10000., None, -99.), RaiseDead);
    assert_eq!(choose_ability(0, 7999., Some(RaiseDead), -99.), MultiDeath);
    assert_eq!(choose_ability(4, 4999., Some(MultiDeath), -99.), WarpTime);
    assert_eq!(choose_ability(4, 4999., Some(MultiDeath), -4.999), Move);
    assert_eq!(choose_ability(8, 4999., None, -5.), WarpTime);
    assert_eq!(choose_ability(8, 4999., Some(WarpTime), -99.), Move);
    assert_eq!(choose_ability(9, 1., None, -99.), Move);
    assert_eq!(choose_ability(4, 8000., None, -99.), Move);
    assert_eq!(choose_ability(8, 5000., None, -99.), Move);
    for roll in 0..10 {
        for hp in [1., 4999., 5000., 7999., 8000., 10000.] {
            for previous in [Some(RaiseDead), Some(MultiDeath), Some(WarpTime)] {
                assert_ne!(choose_ability(roll, hp, previous, -99.), previous.unwrap());
            }
        }
    }
}

#[test]
fn prep_delay_placement_clone_counts_and_unclamped_summon_speed() {
    assert_eq!(preparation_delay(10000., 0.), 2.5);
    assert_eq!(preparation_delay(5000., 0.5), 2.125);
    assert_eq!(placement_distance(5000., Ability::Move), 7.5);
    assert_eq!(placement_distance(5000., Ability::RaiseDead), 10.);
    assert_eq!(movement_speed(5000.), 7.5);
    assert_eq!(clone_count(7000.), 8);
    assert_eq!(clone_count(5000.), 12);
    assert_eq!(clone_count(3000.), 16);
    assert_eq!(raise_animation_speed(10000.), 2.);
    assert!((raise_animation_speed(5000.) - 2.675).abs() < 1e-6);
    assert!((raise_animation_speed(0.) - 3.35).abs() < 1e-6);
}

#[test]
fn target_scan_wraps_sparse_slots_and_keeps_the_old_target_if_none_qualifies() {
    let mut players = [PlayerSlot {
        playing: false,
        living_avatar: None,
    }; 4];
    players[0] = PlayerSlot {
        playing: true,
        living_avatar: Some(10),
    };
    players[3] = PlayerSlot {
        playing: true,
        living_avatar: Some(13),
    };
    assert_eq!(select_target(&players, 1, Some(99)), Some(13));
    assert_eq!(select_target(&players, 0, Some(99)), Some(10));
    players[3].living_avatar = None;
    assert_eq!(select_target(&players, 3, None), Some(10));
    players[0].playing = false;
    assert_eq!(select_target(&players, 0, Some(99)), Some(99));
}

#[test]
fn skeleton_placement_retries_squared_distance_and_rejects_tenth_sample() {
    let mut draws = 0;
    assert_eq!(
        skeleton_position(&[[0.; 3]], || {
            draws += 1;
            if draws == 2 {
                [2., 0., 0.]
            } else {
                [1., 0., 0.]
            }
        }),
        Some([2., 0., 0.])
    );
    assert_eq!(
        draws, 2,
        "two metres is clear: threshold is squared distance 3"
    );
    draws = 0;
    assert_eq!(
        skeleton_position(&[[0.; 3]], || {
            draws += 1;
            if draws == 10 {
                [10., 0., 0.]
            } else {
                [0.; 3]
            }
        }),
        None
    );
    assert_eq!(draws, 10);
    assert_eq!(skeleton_position(&[], || [0.; 3]), Some([0.; 3]));
}

#[test]
fn rise_intro_hold_waits_for_clip_completion_and_crossfade_end() {
    let mut gate = RiseIntroGate::default();
    assert!(!gate.update(100., false, false));
    assert!(!gate.update(100., true, true));
    assert!(!gate.update(0.1, true, false));
    assert!(!gate.update(100., false, false));
    assert!(gate.update(0.051, true, false));
}

#[test]
fn summon_pulse_is_once_per_frame_and_resets_even_while_at_capacity() {
    let mut pulse = RaiseDeadPulse::default();
    assert!(!pulse.update(0.1, 0));
    assert!(pulse.update(0.1, 0));
    assert!(!pulse.update(0., 0));
    assert!(pulse.update(5., 0));
    assert!(
        !pulse.update(0., 0),
        "large dt must not cause catch-up spawns"
    );
    assert!(!pulse.update(0.2, 6));
    assert!(
        !pulse.update(0., 5),
        "a full cap still resets the pulse timer"
    );
    assert!(pulse.update(0.2, 5));
}
