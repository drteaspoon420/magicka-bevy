use magicka_runtime::{
    boss_grimnir2::{State, HEAL_HP, MAX_HP},
    play_session::PlaySession,
    scene_loading::custom_hash,
    scene_world::{boss_live::BossRef, grimnir2_live::Grimnir2Host},
};
use magicka_sim::{
    damage::{attack, Damage},
    spells::Elements as E,
};
const DT: f32 = 1. / 60.;
fn host(s: &PlaySession) -> &Grimnir2Host {
    s.world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| {
            if let BossRef::Grimnir2(g) = b {
                Some(g.as_ref())
            } else {
                None
            }
        })
        .expect("physical Grimnir2 missing")
}
fn session() -> Option<PlaySession> {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "EndofWorld.lvl", "Wizard").unwrap();
    s.change_scene_named("ew_s5", "endspawn").unwrap();
    s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
    step(&mut s);
    Some(s)
}
fn step(s: &mut PlaySession) {
    s.step(DT);
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
}
fn activate(s: &mut PlaySession) {
    s.run_trigger("Start Grimnir Fight", true).unwrap();
    for _ in 0..190 {
        step(s)
    }
    assert!(s.world.boss_fight.running);
}
fn hurt(s: &mut PlaySession) {
    let h = host(s).zones[0];
    s.world
        .damage_physics(
            h,
            Damage {
                attack: attack::DAMAGE,
                element: E::ARCANE.0,
                amount: 100000.,
                magnitude: 1.,
            },
            15,
        )
        .unwrap();
    step(s);
}
#[test]
fn physical_actor_survives_timer_casts_and_has_real_boss_health() {
    let Some(mut s) = session() else { return };
    assert_eq!(host(&s).actor.status.hit_points, MAX_HP);
    assert_eq!(host(&s).actor.state, State::Intro);
    assert!(!s.world.boss_fight.running);
    for h in host(&s).zones {
        assert!(!s.world.npcs.contains_key(&h));
        assert!(s.world.physics_damage[&h].boss_capsule.is_some());
        assert!(s.world.physics_damage[&h].collision_box.is_none());
    }
    activate(&mut s);
    let mut spirits = false;
    for _ in 0..2400 {
        step(&mut s);
        spirits |= host(&s).actor.spirits[0].drawn;
    }
    assert_eq!(host(&s).actor.status.hit_points, MAX_HP);
    assert!(!s.world.boss_fight.dead);
    assert_eq!(s.world.boss_fight.normalized_health, 1.);
    assert!(spirits, "no spectral summon");
    assert!(
        s.world
            .log
            .iter()
            .any(|l| l.starts_with("boss spell zone=")),
        "no live spell casts"
    );
    assert!(!s.world.log.iter().any(|l| l.contains("grimnir2: stand-in")));
    eprintln!(
        "Grimnir2 lives after40seconds, spectral summon and {} actual spell casts",
        s.world
            .log
            .iter()
            .filter(|l| l.starts_with("boss spell zone="))
            .count()
    );
}

#[test]
fn every_attacking_spirit_is_visible_before_and_after_the_vlad_sequence() {
    use magicka_runtime::boss_grimnir2::{Command, Inputs};
    let Some(mut s) = session() else { return };
    activate(&mut s);
    let mut actor = host(&s).actor.clone();
    assert!(!actor.both_spirits);
    let inputs = Inputs {
        fight_started: true,
        nearest_distance_squared: 1000.0,
        target: [0.0, 1.0, 20.0],
        ..Default::default()
    };
    for later_phase in [false, true] {
        if later_phase {
            actor.message("FutureVladDone");
            assert!(actor.both_spirits);
        }
        actor.spirit_timer = 0.0;
        let mut casts = [0; 2];
        for _ in 0..1200 {
            for command in actor.update(DT, inputs) {
                if let Command::Cast { part: 1..=2, .. } = command {
                    let Command::Cast { part, .. } = command else {
                        unreachable!()
                    };
                    casts[part - 1] += 1;
                    assert!(
                        actor.spirits[part - 1].drawn,
                        "spirit {part} attacks invisibly (later phase {later_phase})"
                    );
                    assert!(
                        actor.spectral_alpha > 0.0,
                        "attacking spirit has zero alpha"
                    );
                }
            }
        }
        assert!(
            casts.iter().all(|n| *n > 0),
            "both native casters stay active: {casts:?}"
        );
        eprintln!("later phase {later_phase}: visible spirit casts {casts:?}");
        let a = &actor.spirits[0].pose.parent;
        let b = &actor.spirits[1].pose.parent;
        assert!(
            (a[12] - b[12]).hypot(a[14] - b[14]) > 1.0,
            "spirits must have separate positions"
        );
    }
}
#[test]
fn health_threshold_drives_assatur_once_and_healing_rejects_more_hits() {
    let Some(mut s) = session() else { return };
    activate(&mut s);
    hurt(&mut s);
    assert!(host(&s).actor.healing);
    assert_eq!(host(&s).actor.status.hit_points, HEAL_HP);
    assert_eq!(s.world.boss_fight.normalized_health, 0.1);
    hurt(&mut s);
    assert_eq!(host(&s).actor.status.hit_points, HEAL_HP);
    assert!(
        !s.runtime
            .trigger(custom_hash("assatur_appears"))
            .unwrap()
            .has_triggered
    );
    for _ in 0..260 {
        step(&mut s)
    }
    assert!(
        s.runtime
            .trigger(custom_hash("assatur_appears"))
            .unwrap()
            .has_triggered
    );
    assert_eq!(host(&s).actor.assatur_alpha, 1.);
    assert!(!s.world.boss_fight.dead);
    let p = s.body().position;
    s.world.corporealize(p);
    assert!(host(&s).actor.corporeal);
    assert_eq!(
        host(&s).actor.status.hit_points,
        HEAL_HP,
        "first encounter has no assatur_talks trigger"
    );
}
#[test]
fn checkpoint_recreates_one_physical_boss_and_full_health() {
    let Some(mut s) = session() else { return };
    activate(&mut s);
    assert!(s.checkpoint.is_some());
    for _ in 0..2 {
        hurt(&mut s);
        assert!(s.restart_at_checkpoint().unwrap());
        for _ in 0..5 {
            step(&mut s)
        }
        assert_eq!(
            s.world
                .boss_fight
                .bosses
                .iter()
                .filter(|b| matches!(b, BossRef::Grimnir2(_)))
                .count(),
            1
        );
        assert_eq!(host(&s).actor.status.hit_points, MAX_HP);
        for h in host(&s).zones {
            assert!(
                s.world
                    .entities
                    .iter()
                    .find(|e| e.handle == h)
                    .unwrap()
                    .added
            );
        }
    }
}
#[test]
fn final_encounter_corporealization_uses_native_script_gate() {
    let Some(mut s) = session() else { return };
    // Same class is reused in ew2_s1; trigger availability, rather than a scene
    // name or timer, determines whether Corporealize enters the final outro.
    s.world
        .scene_trigger_ids
        .insert(custom_hash("assatur_talks"));
    hurt(&mut s);
    let p = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == host(&s).zones[0])
        .unwrap()
        .position;
    s.world.corporealize(p);
    assert_eq!(host(&s).actor.state, State::Die);
    assert_eq!(host(&s).actor.status.hit_points, 0.);
    assert!(!host(&s).actor.healing);
    assert!(s
        .world
        .pending_triggers
        .iter()
        .any(|t| t.0 == custom_hash("assatur_talks")));
    let BossRef::Grimnir2(g) = s
        .world
        .boss_fight
        .bosses
        .iter_mut()
        .find(|b| matches!(b, BossRef::Grimnir2(_)))
        .unwrap()
    else {
        unreachable!()
    };
    for _ in 0..120 {
        g.actor.update(DT, Default::default());
    }
    assert_eq!(g.actor.status.hit_points, 0.);
    assert_eq!(g.actor.state, State::Die);
}
