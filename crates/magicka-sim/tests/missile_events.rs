use magicka_sim::{
    damage::{attack, calculate_damage, CastType, Damage},
    impact_motion::Vector,
    missile_events::*,
    spells::{Elements as E, Spell, SpellType},
};
#[derive(Default)]
struct Host {
    actions: Vec<String>,
    damages: Vec<Damage>,
    presentation: Vec<MissileEvent>,
    kills: usize,
    velocity: f32,
    not_damageable: bool,
}
impl MissileEventHost for Host {
    fn damageable_target(&self) -> bool {
        !self.not_damageable
    }
    fn normalized_velocity(&self) -> f32 {
        self.velocity
    }
    fn damage(&mut self, d: Damage) -> u16 {
        self.actions.push("damage".into());
        self.damages.push(d);
        2
    }
    fn kill(&mut self) {
        self.actions.push("kill".into());
        self.kills += 1;
    }
    fn event(&mut self, e: &MissileEvent, _: &mut Option<Vector>) {
        self.actions.push("presentation".into());
        self.presentation.push(e.clone());
    }
}
fn recipe(es: &[E], splash: f32) -> MissileRecipe {
    MissileRecipe::from_spell(Spell::default_for_list(es), splash, 3).unwrap()
}
fn args(flags: u8) -> EventArgs {
    EventArgs {
        flags,
        ..Default::default()
    }
}
#[test]
fn timer_counts_once_per_call_and_earlier_hit_bit_bypasses_timer() {
    let mut c = EventCondition {
        flags: condition::TIMER,
        time: 10.,
        repeat: true,
        ..Default::default()
    };
    let a = EventArgs {
        flags: condition::TIMER,
        time: 31.,
        ..Default::default()
    };
    assert!(c.is_met(&a));
    assert_eq!(c.count, 1);
    assert!(c.is_met(&a));
    assert_eq!(c.count, 2);
    c.flags |= condition::HIT;
    let mut a = a;
    a.flags |= condition::HIT;
    assert!(c.is_met(&a));
    assert_eq!(c.count, 2);
    c.repeat = false;
    assert!(!c.is_met(&a));
}
#[test]
fn damaged_requires_matching_elements_and_collision_threshold_preserves_nan() {
    let mut c = EventCondition {
        flags: condition::DAMAGED | condition::COLLISION,
        threshold: 5.,
        hitpoints: 20.,
        elements: E::FIRE.0,
        repeat: true,
        ..Default::default()
    };
    let mut a = args(condition::DAMAGED);
    a.hitpoints = 20.;
    a.elements = E::EARTH.0;
    assert!(!c.is_met(&a));
    a.elements = E::FIRE.0;
    assert!(c.is_met(&a));
    a.flags = condition::COLLISION;
    a.threshold = f32::NAN;
    assert!(!c.is_met(&a));
    a.threshold = 5.;
    assert!(c.is_met(&a));
}
#[test]
fn earth_hits_preserve_counter_and_only_scale_first_damage_amount() {
    let mut r = recipe(&[E::EARTH], 0.);
    assert_eq!(r.model, Some(MissileModel::Earth(0)));
    let mut h = Host {
        velocity: 2.,
        ..Default::default()
    };
    let mut a = args(condition::HIT);
    let original = calculate_damage(
        &Spell::default_for_list(&[E::EARTH]),
        SpellType::Projectile,
        CastType::Force,
    )
    .unwrap();
    for _ in 0..25 {
        assert_eq!(r.conditions.execute(&mut a, &mut h), (true, 2));
    }
    assert_eq!(h.kills, 0);
    assert_eq!(h.damages[0].amount, original[0].amount * 2.);
    assert_eq!(h.damages[0].magnitude, original[0].magnitude);
    assert_eq!(
        h.damages.len(),
        125,
        "five single calls including zero slots"
    );
    r.conditions.execute(&mut a, &mut h);
    assert_eq!(h.kills, 1);
}
#[test]
fn ice_effects_precede_four_piercing_damage_calls_without_hit_removal() {
    let mut r = recipe(&[E::ICE, E::FIRE], 0.);
    assert_eq!(r.model, Some(MissileModel::Ice3));
    let mut h = Host {
        velocity: 1.,
        ..Default::default()
    };
    r.conditions.execute(&mut args(condition::HIT), &mut h);
    assert_eq!(h.actions[..2], ["presentation", "presentation"]);
    assert_eq!(h.damages.len(), 4);
    assert_eq!(h.kills, 0);
    let spell = Spell::default_for_list(&[E::ICE, E::FIRE]);
    let ds = calculate_damage(&spell, SpellType::Projectile, CastType::Force).unwrap();
    assert_eq!(h.damages[0].magnitude, ds[0].magnitude / 3.);
    assert!(h.damages.iter().all(|d| d.attack & attack::PIERCING != 0));
    r.conditions
        .execute(&mut args(condition::COLLISION), &mut h);
    assert_eq!(h.kills, 1);
}
#[test]
fn mixed_collision_duplicate_effects_survive_kill_and_condition_order() {
    let mut r = recipe(&[E::EARTH, E::ICE], 0.);
    let mut h = Host::default();
    let mut a = EventArgs {
        flags: condition::HIT | condition::COLLISION | condition::TIMER,
        time: 10.,
        ..Default::default()
    };
    r.conditions.execute(&mut a, &mut h);
    assert_eq!(h.kills, 2, "hit and timer remove both execute");
    let big = h
        .presentation
        .iter()
        .filter(|e| {
            matches!(
                e,
                MissileEvent::Effect {
                    table: EffectTable::HitBig,
                    ..
                }
            )
        })
        .count();
    assert_eq!(big, 4, "two effects for each of Earth/Ice");
    assert_eq!(h.actions[5], "kill");
    assert_eq!(
        r.conditions.slots[2].events[1],
        MissileEvent::Remove { bounces: 0 }
    );
}
#[test]
fn splash_uses_original_spell_damage_and_light_averages_unweighted_colors() {
    let spell = Spell::default_for_list(&[E::EARTH, E::FIRE, E::FIRE, E::ARCANE]);
    let r = MissileRecipe::from_spell(spell, 4., 1).unwrap();
    let blast = r.conditions.slots[1]
        .events
        .iter()
        .find_map(|e| match e {
            MissileEvent::Blast { damages, .. } => Some(damages),
            _ => None,
        })
        .unwrap();
    let expected = calculate_damage(&spell, SpellType::Beam, CastType::Area).unwrap();
    assert_eq!(&blast[..expected.len()], expected.as_slice());
    let light = r.conditions.slots[0].events.last().unwrap();
    if let MissileEvent::Light {
        radius,
        diffuse,
        ambient,
        ..
    } = light
    {
        assert_eq!(*radius, 6.);
        assert!((diffuse[0] - 1.9 * 1.333).abs() < 1e-6);
        assert!((ambient[1] - 0.5 * 0.666).abs() < 1e-6);
    } else {
        panic!("missing native light")
    }
}
#[test]
fn empty_condition_activates_but_returns_false_and_non_damageable_skips_damage() {
    let mut c = MissileConditions::default();
    c.slots[0].condition.flags = condition::DEFAULT;
    c.slots[0].condition.repeat = false;
    assert_eq!(
        c.execute(&mut args(condition::DEFAULT), &mut Host::default()),
        (false, 0)
    );
    assert!(c.slots[0].condition.activated);
    let mut r = recipe(&[E::EARTH], 0.);
    let mut h = Host {
        velocity: f32::NAN,
        not_damageable: true,
        ..Default::default()
    };
    assert_eq!(
        r.conditions.execute(&mut args(condition::HIT), &mut h),
        (true, 0)
    );
    assert!(h.damages.is_empty());
}
#[test]
fn model_selection_repeats_fourth_variant_and_conditions_clone_independently() {
    let mut spell = Spell::default_for_list(&[E::EARTH]);
    spell.mag[0] = 4.;
    let mut a = MissileRecipe::from_spell(spell, 0., 1).unwrap();
    assert_eq!(a.model, Some(MissileModel::Earth(2)));
    let b = a.clone();
    a.conditions
        .execute(&mut args(condition::COLLISION), &mut Host::default());
    assert_eq!(
        b.conditions.slots[2].events[1],
        MissileEvent::Remove { bounces: 1 }
    );
    assert_eq!(
        a.conditions.slots[2].events[1],
        MissileEvent::Remove { bounces: 0 }
    );
}

/// ProppMagick: a hit blasts twice (the timer's BlastEvent was added to the
/// Hit slot), a collision once, the 8 s timer never; each removes the ball.
#[test]
fn propp_magick_hit_blasts_twice_collision_once_timer_never() {
    let blasts = |h: &Host| {
        h.presentation
            .iter()
            .filter(|e| matches!(e, MissileEvent::Blast { radius, .. } if *radius == 4.0))
            .count()
    };
    for (flags, time, expected) in [
        (condition::HIT, 0.0, 2),
        (condition::COLLISION, 0.0, 1),
        (condition::TIMER, 8.0, 0),
    ] {
        let mut c = propp_conditions();
        let mut h = Host::default();
        let mut a = EventArgs {
            flags,
            time,
            ..Default::default()
        };
        c.execute(&mut a, &mut h);
        assert_eq!(blasts(&h), expected, "flags {flags}");
        assert_eq!(h.kills, 1, "removed");
        assert!(h.presentation.iter().any(|e| matches!(
            e,
            MissileEvent::NamedEffect {
                name: "magick_propp_explosion",
                world_align: true,
                ..
            }
        )));
    }
    let d = propp_damage();
    assert_eq!(
        (d[0].amount, d[1].amount, d[2].amount, d[2].magnitude),
        (6000.0, 2000.0, 200.0, 3.0)
    );
    // Before 8 s the timer slot stays quiet.
    let mut c = propp_conditions();
    let mut h = Host::default();
    c.execute(
        &mut EventArgs {
            flags: condition::TIMER,
            time: 7.9,
            ..Default::default()
        },
        &mut h,
    );
    assert_eq!(h.kills, 0);
}

/// JudgementSpray missiles: a hit (or a 20 HP Fire/Lightning/Arcane damage,
/// a collision, the 2 s timer) deals Cold 273 Damage and Cold 100 Status to
/// the struck target and removes the missile.
#[test]
fn judgement_missile_hits_deal_cold_damage_and_status() {
    for flags in [condition::HIT, condition::COLLISION] {
        let mut c = judgement_conditions();
        let mut h = Host::default();
        c.execute(
            &mut EventArgs {
                flags,
                ..Default::default()
            },
            &mut h,
        );
        let cold: Vec<_> = h
            .damages
            .iter()
            .map(|d| (d.attack, d.element, d.amount))
            .collect();
        assert_eq!(
            cold,
            [
                (attack::DAMAGE, E::COLD.0, 273.0),
                (attack::STATUS, E::COLD.0, 100.0)
            ],
            "flags {flags}"
        );
        assert_eq!(h.kills, 1);
    }
    // Damaged only by 20+ HP of Fire/Lightning/Arcane.
    let mut c = judgement_conditions();
    let mut h = Host::default();
    c.execute(
        &mut EventArgs {
            flags: condition::DAMAGED,
            hitpoints: 30.0,
            elements: E::COLD.0,
            ..Default::default()
        },
        &mut h,
    );
    assert_eq!(h.kills, 0);
    c.execute(
        &mut EventArgs {
            flags: condition::DAMAGED,
            hitpoints: 30.0,
            elements: E::FIRE.0,
            ..Default::default()
        },
        &mut h,
    );
    assert_eq!(h.kills, 1);
}
