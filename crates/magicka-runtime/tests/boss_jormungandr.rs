use magicka_runtime::{
    boss_action::{BossAction, BossCommand},
    boss_health::BossHealth,
    jormungandr::{Jormungandr, State, MODEL},
    play_content::load_skinned_model,
    play_session::PlaySession,
    script_runtime::ScriptWorld,
};
use magicka_sim::{
    damage::{attack, Damage},
    spells::Elements as E,
    status::Status as S,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
const DT: f32 = 1. / 60.;
fn root() -> PathBuf {
    magicka_content::retail_content_dir()
}
fn actor() -> Option<Jormungandr> {
    if !root().is_dir() {
        return None;
    }
    Some(
        Jormungandr::new(
            load_skinned_model(&root(), &Path::new(MODEL).with_extension("xnb")).unwrap(),
            magicka_content::skinned::IDENTITY,
            42,
        )
        .unwrap(),
    )
}
fn d(element: u32, amount: f32, magnitude: f32) -> Damage {
    Damage {
        attack: attack::DAMAGE,
        element,
        amount,
        magnitude,
    }
}
#[test]
fn boss_damage_uses_resistance_average_wet_lightning_and_frozen_physical_rules() {
    let mut h = BossHealth::new(10000.);
    h.resistances[0].multiplier = 0.75;
    h.resistances[8].multiplier = 0.75;
    h.damage(d(E::EARTH.0 | E::ARCANE.0, 100., 2.));
    assert_eq!(h.hp, 9825.);
    h.damage(Damage {
        attack: attack::STATUS,
        element: E::WATER.0,
        amount: 0.,
        magnitude: 2.,
    });
    h.update(0., false);
    assert!(h.has(S::Wet));
    h.damage(d(E::LIGHTNING.0, 100., 1.));
    assert_eq!(h.hp, 9625.);
    h.damage(Damage {
        attack: attack::STATUS,
        element: E::COLD.0,
        amount: 0.,
        magnitude: 2.,
    });
    h.update(0., false);
    assert!(h.has(S::Frozen));
    h.damage(d(E::EARTH.0, 300., 2.));
    assert_eq!(h.hp, 9175.);
    h.scalar_damage(-50000., E::LIFE.0);
    assert_eq!(h.hp, 10000.);
    h.update(0., true);
    assert_eq!(h.mask, 0);
}

#[test]
fn status_ticks_deliver_synchronously_before_later_slots_read_health() {
    let mut h = BossHealth::new(10000.);
    h.damage(Damage {
        attack: attack::STATUS,
        element: E::FIRE.0,
        amount: 10.,
        magnitude: 1.,
    });
    h.damage(Damage {
        attack: attack::STATUS,
        element: E::LIFE.0,
        amount: -10.,
        magnitude: 1.,
    });
    h.update(0.1, false);
    assert_eq!(h.hp, 10000.);
    assert!(
        h.has(S::Healing),
        "Burning lowers health before Healing checks whether the boss is full"
    );
}
#[test]
fn retail_rig_drives_head_spine_and_attack_cycle_without_timed_defeat() {
    let Some(mut b) = actor() else { return };
    assert_eq!(b.state, State::Sleep);
    assert!(!b.update_animation);
    let mut states = BTreeSet::new();
    let mut max_spine = 0;
    let mut saw_bite_damage = false;
    for _ in 0..2400 {
        let target = [3., 1., 0.];
        b.update(DT, true, false, Some(target), false, |p, a| {
            Some([p[0] + 14. * a.cos(), 0., p[2] + 14. * a.sin()])
        });
        states.insert(format!("{:?}", b.state));
        max_spine = max_spine.max(b.frame.spine.len());
        if b.state == State::Bite {
            saw_bite_damage |= b
                .collision_damage()
                .iter()
                .any(|d| d.element == E::POISON.0 && d.amount == 30.);
        }
        assert!(b.frame.bones.iter().flatten().all(|v| v.is_finite()));
        assert!(!b.dead);
    }
    eprintln!("states={states:?}, spine={max_spine}");
    assert_eq!(b.health.hp, 10000.);
    assert_eq!(max_spine, 25);
    assert!(states.contains("Underground"));
    assert!(states.contains("Risen"));
    assert!(saw_bite_damage);
    assert_eq!(b.warning_time(), 2.5);
    b.health.hp = 0.;
    assert_eq!(b.warning_time(), 0.75);
}
#[test]
fn head_damage_gates_recoil_and_death_on_authored_animation() {
    let Some(mut b) = actor() else { return };
    let damage = d(E::ARCANE.0, 300., 1.);
    b.visible = false;
    assert_eq!(b.damage(&[damage], true), 0);
    b.visible = true;
    b.state = State::Risen;
    b.update_animation = true;
    b.controller.start_named_clip("idle", 1., true);
    assert_eq!(b.damage(&[damage], false), 0);
    assert_eq!(b.health.hp, 10000.);
    b.damage(&[damage], true);
    assert_eq!(b.health.hp, 9700.);
    assert!(b.hit);
    b.update(DT, true, false, Some([3., 0., 0.]), false, |_, _| None);
    assert_eq!(b.state, State::HitRisen);
    for _ in 0..600 {
        b.update(DT, true, false, Some([3., 0., 0.]), false, |_, _| None);
        if b.controller.clip_name() == Some("submerge") {
            break;
        }
    }
    assert_eq!(b.controller.clip_name(), Some("submerge"));
    assert_eq!(b.damage(&[damage], true), 0);
    b.health.hp = 0.;
    b.hit = false;
    b.state = State::Underground;
    b.update(DT, true, false, None, false, |_, _| None);
    assert_eq!(b.state, State::Dead);
    assert!(!b.dead);
    assert_eq!(b.controller.clip_name(), Some("die"));
    for _ in 0..600 {
        b.update(DT, true, false, None, false, |_, _| None);
        if b.dead {
            break;
        }
    }
    assert!(b.dead);
}
#[test]
fn mountaindale_spawns_real_delayed_boss_and_routes_arc_damage() {
    if !root().is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root(), "Mountaindale.lvl", "Wizard").unwrap();
    s.change_scene_named("md_s6BF", "start").unwrap();
    let locator = magicka_runtime::ai_event::AiEventLevel::locator(
        &s.world,
        magicka_runtime::scene_loading::custom_hash("boss0"),
    )
    .unwrap();
    let player = s.world.players[0].as_mut().unwrap();
    player.ai.set_order(0, None);
    let body = &mut s.world.physics.bodies[player.body];
    body.position = [
        locator[12],
        locator[13] + body.radius + body.length * 0.5 + 0.1,
        locator[14],
    ];
    body.velocity = [0.; 3];
    for _ in 0..5 {
        s.step(DT);
    }
    assert_eq!(
        s.world.jormungandr.len(),
        1,
        "campaign SpawnBoss must create its actor"
    );
    let handle = *s.world.jormungandr.keys().next().unwrap();
    assert_eq!(s.world.jormungandr[&handle].state, State::Sleep);
    assert!(!s.world.boss_fight.running);
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
    s.world
        .boss_action(
            &BossAction {
                delay: 0.,
                command: BossCommand::Activate,
            },
            false,
        )
        .unwrap()
        .unwrap();
    let mut exposed = false;
    for _ in 0..1200 {
        s.step(DT);
        if s.world.jormungandr[&handle].update_animation
            && s.world.jormungandr[&handle].state == State::Risen
        {
            exposed = true;
            break;
        }
    }
    assert!(
        exposed,
        "the authored intro must lead to an exposed damage zone"
    );
    let p = s.world.jormungandr[&handle].frame.head;
    let hp = s.world.jormungandr[&handle].health.hp;
    let origin = [p[0] - 2., p[1], p[2]];
    assert!(s.world.arc_damage(
        Some(s.avatar().handle),
        None,
        origin,
        [1., 0., 0.],
        4.,
        std::f32::consts::PI,
        &[d(E::ARCANE.0, 100., 1.)],
        s.world.play_time
    ));
    // CircleDamage falloff: (range - (centre distance - radius))/range.
    assert_eq!(s.world.jormungandr[&handle].health.hp, hp - 77.5);
    s.step(DT);
    assert!((s.world.boss_fight.normalized_health - (hp - 77.5) / 10000.).abs() < 1e-5);
    assert!(!s
        .world
        .log
        .iter()
        .any(|l| l.contains("boss jormungandr: stand-in")));
    assert!(
        !s.avatar().dead(),
        "the fixture must stay on the authored arena floor"
    );
    s.world.arc_damage(
        Some(s.avatar().handle),
        None,
        origin,
        [1., 0., 0.],
        4.,
        std::f32::consts::PI,
        &[d(E::ARCANE.0, 20000., 1.)],
        s.world.play_time,
    );
    for _ in 0..900 {
        s.step(DT);
        if s.world.boss_fight.dead {
            break;
        }
    }
    assert!(
        s.world.boss_fight.dead,
        "damage and the native death animation must end the fight"
    );
    assert_eq!(s.world.boss_fight.normalized_health, 0.);
}

#[test]
fn venom_launches_four_ballistic_missiles_with_native_hit_and_splash_recipes() {
    let Some(mut b) = actor() else { return };
    b.state = State::Risen;
    b.last_state = State::Bite;
    b.update_animation = true;
    b.visible = true;
    b.attack_count = 2;
    b.idle_timer = 4.;
    b.controller.start_named_clip("idle", 1., true);
    let target = [30., -1., 0.];
    let mut missiles = Vec::new();
    for _ in 0..600 {
        missiles.extend(
            b.update(DT, true, false, Some(target), false, |_, _| None)
                .spit,
        );
        if b.last_state == State::Spit && b.state == State::Risen {
            break;
        }
    }
    assert_eq!(missiles.len(), 4);
    for (origin, velocity) in missiles {
        assert!(velocity.iter().all(|v| v.is_finite()));
        let t =
            ((origin[1] - target[1]) / (-0.5 * magicka_sim::character_physics::GRAVITY[1])).sqrt();
        for i in 0..3 {
            let landed = origin[i]
                + velocity[i] * t
                + magicka_sim::character_physics::GRAVITY[i] * t * t * 0.5;
            assert!((landed - target[i]).abs() < 1e-3);
        }
    }
    use magicka_sim::missile_events::MissileEvent as V;
    let c = magicka_runtime::jormungandr::spit_conditions();
    assert!(c.slots[0].events.iter().any(|v| matches!(
        v,
        V::NamedEffect {
            name: "jormungandr_spit_projectile",
            follow: true,
            ..
        }
    )));
    assert!(c.slots[1].events.iter().any(|v|matches!(v,V::Damage{damage,..} if damage.amount==75.&&damage.magnitude==4.&&damage.element==E::POISON.0)));
    assert_eq!(
        c.slots[2]
            .events
            .iter()
            .filter(|v| matches!(v, V::Splash { radius: 2., .. }))
            .count(),
        2
    );
}

#[test]
fn boss_arc_distinguishes_inside_radius_from_co_centred_query() {
    use magicka_sim::arc_intersection::ArcQuery;
    let q = ArcQuery {
        origin: [0., 2., 0.],
        direction: [1., 0., 0.],
        range: 4.,
        angle: 0.1,
        height_difference: 2.,
    };
    assert_eq!(Jormungandr::arc([0., 2., 0.], 1.1, q), None);
    assert_eq!(Jormungandr::arc([0.5, 2., 0.], 1.1, q), Some([0.; 3]));
    assert_eq!(Jormungandr::arc([0.5, 5., 0.], 1.1, q), None);
}

#[test]
fn barrier_recoil_costs_real_boss_health_once_per_native_contact_window() {
    let Some(mut b) = actor() else { return };
    b.state = State::Bite;
    b.update_animation = true;
    assert_eq!(b.hit_barrier(500, 1000., true).unwrap().amount, 750.);
    assert_eq!(b.health.hp, 9250.);
    assert!(b.hit);
    assert_eq!(b.hit_barrier(500, 1000., true), None);
    assert_eq!(b.health.hp, 9250.);
    b.hit = false;
    assert_eq!(b.hit_barrier(501, 1000., false), None);
    assert_eq!(b.health.hp, 9250.);
    assert!(!b.hit);
    // Native records a non-solid barrier before returning without damage.
    assert_eq!(b.hit_barrier(501, 1000., true), None);
    b.hits.clear();
    b.state = State::Spit;
    assert_eq!(b.hit_barrier(502, 1000., true), None);
}

#[test]
fn low_health_leap_warns_whips_and_recoils_back_underground() {
    if !root().is_dir() {
        return;
    }
    let asset = load_skinned_model(&root(), &Path::new(MODEL).with_extension("xnb")).unwrap();
    let mut b = Jormungandr::new(
        asset,
        magicka_content::skinned::IDENTITY,
        0x2545_F491_4F6C_DD1D,
    )
    .unwrap();
    b.state = State::Underground;
    b.health.hp = 1000.;
    let target = [0., 1., 0.];
    for _ in 0..90 {
        b.update(DT, true, false, Some(target), false, |_, _| {
            Some([14., 0., 0.])
        });
        if b.state == State::Leap {
            break;
        }
    }
    assert_eq!(b.state, State::Leap);
    assert!(!b.update_animation);
    assert!(!b.visible);
    for _ in 0..90 {
        b.update(DT, true, false, Some(target), false, |_, _| None);
        if b.update_animation {
            break;
        }
    }
    assert!(b.update_animation && b.visible);
    assert_eq!(b.frame.spine.len(), 25);
    assert_eq!(
        b.collision_damage()[1].element,
        0,
        "retail's Leap status slot is unassigned; preserve the constructor typo"
    );
    b.damage(&[d(E::ARCANE.0, 300., 1.)], true);
    b.update(DT, true, false, Some(target), false, |_, _| None);
    assert_eq!(b.state, State::HitLeap);
    for _ in 0..600 {
        b.update(DT, true, false, Some(target), false, |_, _| None);
        if b.state == State::Underground {
            break;
        }
    }
    assert_eq!(b.state, State::Underground);
    assert!(!b.visible);
}

#[test]
fn viewer_spawn_sequence_and_idle_input_advance_emergence() {
    if !root().is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root(), "Mountaindale.lvl", "Wizard").unwrap();
    s.change_scene_named("md_s6BF", "start").unwrap();
    s.run_trigger("StartOfLevel", true).unwrap();
    let locator = magicka_runtime::ai_event::AiEventLevel::locator(
        &s.world,
        magicka_runtime::scene_loading::custom_hash("boss0"),
    )
    .unwrap();
    let p = s.world.players[0].as_mut().unwrap();
    p.character.immortal_time = f32::MAX;
    p.ai.set_order(0, None);
    let body = &mut s.world.physics.bodies[p.body];
    body.position = [
        locator[12],
        locator[13] + body.radius + body.length * 0.5 + 0.1,
        locator[14],
    ];
    body.velocity = [0.; 3];
    s.step(0.);
    s.disable_scripts();
    s.world.players[0].as_mut().unwrap().ai.set_order(0, None);
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    s.world
        .boss_action(
            &BossAction {
                delay: 0.,
                command: BossCommand::Activate,
            },
            false,
        )
        .unwrap()
        .unwrap();
    let handle = *s.world.jormungandr.keys().next().unwrap();
    let mut advanced = false;
    let b = &s.world.jormungandr[&handle];
    let clips = b.skeleton.clip_durations();
    let frames =
        ((clips["intro"] + 2. * b.warning_time() + clips["emerge"] + 2.) / DT).ceil() as usize;
    for frame in 0..frames {
        s.step(DT);
        s.handle_input(magicka_runtime::avatar::PlayerInput::default(), 16. / 9.);
        let b = &s.world.jormungandr[&handle];
        if frame % 120 == 0 {
            eprintln!("viewer-like frame={frame} state={:?} time={} idle={} running={} active={} scale={}/{} end-freeze={} hp={}",b.state,b.controller.time,b.idle_timer,s.world.boss_fight.running,s.world.boss_fight.active,s.world.time_multiplier,s.world.time_modifier,s.end_game.freezes_entities(),s.avatar().character.combat.health.hit_points);
        }
        advanced |= b.state == State::Risen && b.update_animation && b.controller.time > 0.5;
    }
    assert!(
        advanced,
        "the render fixture's spawn/input sequence must advance the real emergence clip"
    );
}

#[test]
fn full_leap_and_above_ground_death_preserve_native_root_orientation() {
    if !root().is_dir() {
        return;
    }
    // Deterministic seed selects the native low-health leap branch.
    let asset = load_skinned_model(&root(), &Path::new(MODEL).with_extension("xnb")).unwrap();
    let mut b = Jormungandr::new(
        asset,
        magicka_content::skinned::IDENTITY,
        0x2545_F491_4F6C_DD1D,
    )
    .unwrap();
    b.state = State::Underground;
    b.health.hp = 1000.;
    let target = [0., 1., 0.];
    for _ in 0..90 {
        b.update(DT, true, false, Some(target), false, |_, _| {
            Some([14., 0., 0.])
        });
        if b.state != State::Underground {
            break;
        }
    }
    assert_eq!(b.state, State::Leap);
    b.update(DT, true, false, Some(target), false, |_, _| None);
    let locked = b.orientation;
    assert!(
        b.direction[1] > 0.,
        "native target lock includes target height"
    );
    assert!((locked[12] - (14. + b.direction[0] * 8.)).abs() < 1e-5);
    assert!((locked[13] + 9.5).abs() < 1e-5);
    let mut max_head_y = f32::NEG_INFINITY;
    for _ in 0..600 {
        b.update(DT, true, false, Some(target), false, |_, _| None);
        if b.visible {
            max_head_y = max_head_y.max(b.frame.head[1]);
        }
        if b.state == State::Underground {
            break;
        }
    }
    assert_eq!(b.state, State::Underground);
    assert!(
        max_head_y > target[1],
        "head must complete the above-ground leap, got {max_head_y}"
    );
    b.state = State::Risen;
    b.update_animation = true;
    b.visible = true;
    b.controller
        .start_named_clip("idle", b.skeleton.clip_durations()["idle"], true);
    b.health.hp = 0.;
    b.update(DT, true, false, Some(target), false, |_, _| None);
    assert_eq!(b.state, State::Dead);
    assert_eq!(b.controller.clip_name(), Some("die_above"));
    b.update(DT, true, false, Some(target), false, |_, _| None);
    assert!(b.orientation[5] > 0., "death must not invert the corpse");
    for _ in 0..600 {
        b.update(DT, true, false, Some(target), false, |_, _| None);
        if b.dead {
            break;
        }
    }
    assert!(b.dead && b.visible);
}

#[test]
fn emerge_warning_starts_once_and_stops_before_death() {
    if !root().is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root(), "Mountaindale.lvl", "Wizard").unwrap();
    s.change_scene_named("md_s6BF", "start").unwrap();
    s.run_trigger("StartOfLevel", true).unwrap();
    let locator = magicka_runtime::ai_event::AiEventLevel::locator(
        &s.world,
        magicka_runtime::scene_loading::custom_hash("boss0"),
    )
    .unwrap();
    let player = s.world.players[0].as_mut().unwrap();
    player.character.immortal_time = f32::MAX;
    player.ai.set_order(0, None);
    let body = &mut s.world.physics.bodies[player.body];
    body.position = [
        locator[12],
        locator[13] + body.radius + body.length * 0.5 + 0.1,
        locator[14],
    ];
    body.velocity = [0.; 3];
    s.step(0.);
    s.disable_scripts();
    s.world
        .boss_action(
            &BossAction {
                delay: 0.,
                command: BossCommand::Activate,
            },
            false,
        )
        .unwrap()
        .unwrap();
    let effect = magicka_runtime::scene_loading::custom_hash("jormungandr_dirt_tremor");
    let mut seen = None;
    for _ in 0..1800 {
        s.step(DT);
        if let Some((&handle, boss)) = s
            .world
            .jormungandr
            .iter()
            .find(|(_, b)| b.state == State::Risen && !b.update_animation)
        {
            if let Some(instance) = boss.ground_effects[0] {
                assert!(
                    s.effects.active_effects_by_hash(effect) > 0,
                    "warning asset must be loaded"
                );
                seen = Some((handle, instance));
                break;
            }
        }
    }
    let (handle, instance) = seen.expect("native emerge must send a ground warning");
    s.world.effect_commands.clear();
    s.step(DT);
    assert_eq!(
        s.world.jormungandr[&handle].ground_effects[0],
        Some(instance)
    );
    {
        let b = s.world.jormungandr.get_mut(&handle).unwrap();
        b.state = State::Underground;
        b.health.hp = 0.;
    }
    s.world.effect_commands.clear();
    s.step(DT);
    assert_eq!(s.world.jormungandr[&handle].state, State::Dead);
    assert_eq!(s.world.jormungandr[&handle].ground_effects, [None; 3]);
}

#[test]
fn warning_uses_native_health_formula_and_retains_paused_capsule_origins() {
    let Some(mut b) = actor() else { return };
    assert!((b.warning_time() - 2.5).abs() < 1e-6);
    b.health.hp = 1000.;
    assert!((b.warning_time() - 0.925).abs() < 1e-6);
    b.state = State::Underground;
    let before = b.frame.body_points.clone();
    b.orientation[12] += 40.;
    b.update(DT, true, false, Some([0., 1., 0.]), false, |_, _| None);
    assert!(!b.update_animation);
    assert_eq!(
        b.frame.body_points, before,
        "native hidden updates retain primitive locals"
    );
}

#[test]
fn campaign_leap_warning_retains_last_animated_capsule_origins() {
    if !root().is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root(), "Mountaindale.lvl", "Wizard").unwrap();
    s.change_scene_named("md_s6BF", "start").unwrap();
    s.run_trigger("StartOfLevel", true).unwrap();
    let locator = magicka_runtime::ai_event::AiEventLevel::locator(
        &s.world,
        magicka_runtime::scene_loading::custom_hash("boss0"),
    )
    .unwrap();
    let player = s.world.players[0].as_mut().unwrap();
    player.character.immortal_time = f32::MAX;
    player.ai.set_order(0, None);
    let body = &mut s.world.physics.bodies[player.body];
    body.position = [
        locator[12],
        locator[13] + body.radius + body.length * 0.5 + 0.1,
        locator[14],
    ];
    body.velocity = [0.; 3];
    s.step(0.);
    s.disable_scripts();
    s.world
        .boss_action(
            &BossAction {
                delay: 0.,
                command: BossCommand::Activate,
            },
            false,
        )
        .unwrap()
        .unwrap();
    let handle = *s.world.jormungandr.keys().next().unwrap();
    let mut last_animated_points = Vec::new();
    for _ in 0..6000 {
        let boss = s.world.jormungandr.get_mut(&handle).unwrap();
        boss.health.hp = 1000.;
        if boss.state == State::Leap && !boss.update_animation && boss.idle_timer > 0.15 {
            eprintln!(
                "campaign leap warning effects={:?} points={:?}",
                boss.ground_effects, boss.frame.body_points
            );
            assert_eq!(boss.frame.body_points, last_animated_points);
            return;
        }
        if boss.update_animation {
            last_animated_points = boss.frame.body_points.clone();
        }
        s.step(DT);
        s.handle_input(magicka_runtime::avatar::PlayerInput::default(), 16. / 9.);
    }
    panic!("campaign fixture did not enter Leap");
}
