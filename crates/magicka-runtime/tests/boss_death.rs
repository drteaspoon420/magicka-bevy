//! Actual nh_s3BF script and live Death boss integration.
use magicka_runtime::{
    boss_death::State,
    play_session::PlaySession,
    scene_loading::custom_hash,
    scene_world::{boss_death_live::DeathBossHost, boss_live::BossRef},
};
use magicka_sim::{
    character_impact::result,
    damage::{attack, Damage},
    spells::Elements,
};
use std::path::Path;
const DT: f32 = 1. / 60.;
fn host(s: &PlaySession) -> &DeathBossHost {
    s.world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| {
            if let BossRef::Death(d) = b {
                Some(d.as_ref())
            } else {
                None
            }
        })
        .expect("no live Death boss")
}
fn step(s: &mut PlaySession) {
    s.step(DT);
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
}
fn battle() -> Option<PlaySession> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Niflheim.lvl", "Wizard").unwrap();
    s.change_scene_named("nh_s3BF", "start").unwrap();
    s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
    // Startup must install its AvatarMove before the throne trigger replaces it.
    for _ in 0..60 {
        step(&mut s);
    }
    assert!(s.run_trigger("Move to the throne", true).unwrap());
    for tick in 0..3600 {
        if tick % 30 == 0 {
            s.world.advance_dialog(0);
        }
        step(&mut s);
        if s.world.boss_fight.running && s.checkpoint.is_some() && !s.input_limited() {
            {
                let d = host(&s).actor.lock().unwrap();
                eprintln!(
                    "Death intro ready after {} ticks: avatar {:?}, boss {:?}, actor {:?}",
                    tick + 60,
                    s.body().position,
                    d.orientation,
                    d.state
                );
            }
            return Some(s);
        }
    }
    panic!(
        "Death intro failed: state={:?}, position={:?}, start_dialog={}, log={:?}",
        host(&s).actor.lock().unwrap().state,
        s.body().position,
        s.runtime
            .trigger(custom_hash("Start Dialog"))
            .unwrap()
            .has_triggered,
        s.world.log.iter().rev().take(15).collect::<Vec<_>>()
    );
}
fn life(amount: f32) -> Damage {
    Damage {
        attack: attack::DAMAGE,
        element: Elements::LIFE.0,
        amount,
        magnitude: 0.,
    }
}

#[test]
fn death_script_intro_checkpoint_and_summons_use_live_actor_instead_of_timer() {
    let Some(mut s) = battle() else {
        return;
    };
    assert!(s
        .world
        .dialogs
        .as_ref()
        .unwrap()
        .is_done(custom_hash("deathintro"), -1)
        .unwrap());
    assert!(!s
        .world
        .log
        .iter()
        .any(|l| l.contains("boss death: stand-in")));
    assert!(s.world.physics_damage[&host(&s).zone]
        .collision_box
        .is_none());
    for _ in 0..2400 {
        step(&mut s);
    }
    assert_eq!(host(&s).actor.lock().unwrap().combat.hit_points(), 10000.);
    assert!(
        !host(&s).skeletons.is_empty(),
        "RaiseDead commands must create real NPCs"
    );
    assert!(!s.world.boss_fight.dead);
    assert_eq!(s.world.boss_fight.normalized_health, 1.);
    assert!(s.restart_at_checkpoint().unwrap());
    step(&mut s);
    assert_eq!(
        s.world
            .boss_fight
            .bosses
            .iter()
            .filter(|b| matches!(b, BossRef::Death(_)))
            .count(),
        1
    );
    assert_eq!(host(&s).actor.lock().unwrap().combat.hit_points(), 10000.);
    assert!(s.world.player_input_locked.iter().all(|b| !*b));
}

#[test]
fn live_life_damage_is_synchronous_and_outro_waits_for_defeated_clip() {
    let Some(mut s) = battle() else {
        return;
    };
    for _ in 0..1800 {
        {
            let d = host(&s).actor.lock().unwrap();
            if d.drawn && !d.ethereal {
                break;
            }
        }
        step(&mut s);
    }
    let zone = host(&s).zone;
    assert_eq!(
        s.world.damage_physics(zone, life(-1000.), 0).unwrap(),
        result::DEFLECTED
    );
    assert_eq!(
        host(&s).actor.lock().unwrap().combat.hit_points(),
        9000.,
        "damage cannot wait for UpdateBoss"
    );
    assert!(host(&s).actor.lock().unwrap().combat.hit_pending);
    s.world.damage_physics(zone, life(20000.), 15).unwrap();
    assert_eq!(host(&s).actor.lock().unwrap().combat.hit_points(), 1.);
    step(&mut s);
    assert_eq!(host(&s).actor.lock().unwrap().state, State::DefeatPreSit);
    assert!(
        !s.runtime
            .trigger(custom_hash("dialogoutro"))
            .unwrap()
            .has_triggered
    );
    for _ in 0..900 {
        step(&mut s);
        if s.runtime
            .trigger(custom_hash("dialogoutro"))
            .unwrap()
            .has_triggered
        {
            break;
        }
    }
    assert!(
        s.runtime
            .trigger(custom_hash("dialogoutro"))
            .unwrap()
            .has_triggered
    );
    assert_eq!(host(&s).actor.lock().unwrap().combat.hit_points(), 0.);
    assert!(
        !s.world.boss_fight.dead,
        "native outro reads bosshealth, not Dead"
    );
    assert!(host(&s).skeletons.is_empty());
}

#[test]
fn life_beam_reaches_live_death_capsule_and_enters_hit_state() {
    let Some(mut s) = battle() else { return };
    for _ in 0..3600 {
        let res = {
            let d = host(&s).actor.lock().unwrap();
            d.state == State::Attack && d.drawn && !d.ethereal
        };
        if res {
            break;
        }
        step(&mut s);
    }
    let actor = host(&s).actor.clone();
    assert_eq!(actor.lock().unwrap().state, State::Attack);
    let p = actor.lock().unwrap().body_position;
    let before = actor.lock().unwrap().combat.hit_points();
    let owner = s.avatar().handle;
    let id = s.world.rails.start(magicka_sim::railgun::RailStart {
        owner,
        owner_position: s.body().position,
        owner_direction: [0., 0., -1.],
        color: [0., 1., 0.],
        damages: [life(-100.); 5],
        element: Elements::LIFE,
        timestamp: s.play_time,
    });
    s.world
        .rails
        .steer(id, [p[0], p[1] + 0.6, p[2] + 3.], [0., 0., -1.]);
    let mut stunned = false;
    for _ in 0..30 {
        step(&mut s);
        stunned |= actor.lock().unwrap().state == State::Hit;
    }
    eprintln!(
        "beam {:?} death {:?}",
        s.world.rails.rail(id),
        actor.lock().unwrap().state
    );
    assert!(
        actor.lock().unwrap().combat.hit_points() < before,
        "beam did not hit Death"
    );
    assert!(stunned, "Life beam failed to interrupt Death");
}

#[test]
fn life_projectile_reaches_death_capsule_and_interrupts_attack() {
    use magicka_runtime::missiles::LiveMissile;
    use magicka_sim::missile_events::{condition, EventCondition, MissileConditions, MissileEvent};
    let Some(mut s) = battle() else { return };
    for _ in 0..3600 {
        if host(&s).actor.lock().unwrap().state == State::Attack {
            break;
        }
        step(&mut s);
    }
    let actor = host(&s).actor.clone();
    assert_eq!(actor.lock().unwrap().state, State::Attack);
    let p = actor.lock().unwrap().body_position;
    let before = actor.lock().unwrap().combat.hit_points();
    let mut conditions = MissileConditions::default();
    conditions.slots[0].condition = EventCondition {
        flags: condition::HIT,
        ..Default::default()
    };
    conditions.slots[0].events = vec![MissileEvent::Damage {
        damage: life(-100.),
        velocity_based: false,
    }];
    let mut missile = LiveMissile::spawn_custom(
        923,
        s.avatar().handle,
        [p[0], p[1] + 0.6, p[2] + 1.],
        [0., 0., -30.],
        0.1,
        false,
        conditions,
    );
    missile.body.no_gravity = true;
    s.world.missiles.push(missile);
    let mut stunned = false;
    for _ in 0..30 {
        step(&mut s);
        stunned |= actor.lock().unwrap().state == State::Hit;
    }
    assert!(
        actor.lock().unwrap().combat.hit_points() < before,
        "Life projectile failed to damage Death"
    );
    assert!(stunned, "Life projectile failed to stun Death");
}

#[test]
fn shield_ice_life_area_pillars_damage_and_stun_death() {
    let Some(mut s) = battle() else { return };
    for _ in 0..3600 {
        if host(&s).actor.lock().unwrap().state == State::Attack {
            break;
        }
        step(&mut s);
    }
    let actor = host(&s).actor.clone();
    assert_eq!(actor.lock().unwrap().state, State::Attack);
    let target = actor.lock().unwrap().body_position;
    let before = actor.lock().unwrap().combat.hit_points();
    let spell = magicka_sim::spells::Spell::default_for(Elements(
        Elements::SHIELD.0 | Elements::ICE.0 | Elements::LIFE.0,
    ));
    let direction = [0., 0., -1.];
    let first = magicka_sim::barrier::cast_placements(true, target, direction, &spell)
        .unwrap()
        .placements[0]
        .probe_origin;
    let source = [
        target[0] + target[0] - first[0],
        target[1],
        target[2] + target[2] - first[2],
    ];
    let orientation = s.body().orientation;
    let body = s.avatar().body;
    s.world.physics.bodies[body].move_to(source, orientation);
    let cast = magicka_sim::barrier::cast_placements(true, source, direction, &spell).unwrap();
    s.world.spawn_barriers(s.avatar().handle, &cast, spell);
    assert!(
        s.world.barriers.iter().next().is_some(),
        "no area pillars spawned"
    );
    assert!(
        actor.lock().unwrap().combat.hit_points() < before,
        "Shield/Ice/Life initial damage omitted Death"
    );
    let after_initial = actor.lock().unwrap().combat.hit_points();
    step(&mut s);
    assert_eq!(
        actor.lock().unwrap().state,
        State::Hit,
        "Life pillars did not interrupt Death"
    );
    for _ in 0..30 {
        step(&mut s);
    }
    assert!(
        actor.lock().unwrap().combat.hit_points() < after_initial,
        "standing in Life pillars did not damage Death again"
    );
}
