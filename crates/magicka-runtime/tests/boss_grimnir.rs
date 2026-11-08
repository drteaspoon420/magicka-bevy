//! Real ew_s4 dream waves and the authored transition into ew_s5.
use magicka_runtime::{
    boss_grimnir::{State, COUNTS, MINIONS, VENUES},
    entity_command::EntityCommandHost,
    play_session::PlaySession,
    scene_loading::custom_hash,
    scene_world::{boss_live::BossRef, grimnir_live::GrimnirHost},
};
use std::path::Path;
const DT: f32 = 1. / 60.;
fn host(s: &PlaySession) -> &GrimnirHost {
    s.world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| {
            if let BossRef::Grimnir(g) = b {
                Some(g.as_ref())
            } else {
                None
            }
        })
        .expect("no live Grimnir")
}
fn step(s: &mut PlaySession) {
    s.step(DT);
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    for b in &s.world.boss_fight.bosses {
        if let BossRef::Grimnir(g) = b {
            assert!(g.failed.is_none(), "{:?}", g.failed);
        }
    }
}
fn session() -> Option<PlaySession> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "EndofWorld.lvl", "Wizard").unwrap();
    s.change_scene_named("ew_s4", "starta").unwrap();
    s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
    step(&mut s);
    Some(s)
}
fn wait_wave(s: &mut PlaySession, venue: usize) {
    for _ in 0..600 {
        if host(s).dreams.venue == venue && host(s).minions.len() == COUNTS[venue] {
            return;
        }
        step(s);
    }
    panic!("venue {venue} did not spawn: {:?}", host(s).dreams);
}
#[test]
fn six_real_waves_drive_health_teleports_and_the_authored_next_scene() {
    let Some(mut s) = session() else { return };
    wait_wave(&mut s, 0);
    assert!(s.checkpoint.is_some());
    assert!(!s
        .world
        .log
        .iter()
        .any(|l| l.contains("boss grimnir: stand-in")));
    // A living wave cannot be replaced by elapsed time. Goblin shamans'
    // element-less Area push hits allies too (PushSpell.CastUpdate only
    // removes the owner), so a minion may be thrown off the island while
    // the wave fights the immortal player; the survivors still hold it.
    for _ in 0..1500 {
        step(&mut s);
    }
    assert!(
        !host(&s).minions.is_empty(),
        "the first wave died unattended"
    );
    assert_eq!(host(&s).dreams.health, 6.);
    assert_eq!(host(&s).dreams.venue, 0);
    for venue in 0..6 {
        if venue > 0 {
            wait_wave(&mut s, venue);
        }
        let handles = host(&s).minions.clone();
        for &h in &handles {
            let n = &s.world.npcs[&h];
            assert_eq!(n.ai.order, magicka_runtime::npc::ORDER_ATTACK);
            assert!(s.world.entities.iter().any(|e| e.handle == h
                && e.added
                && e.template_name.as_deref() == Some(MINIONS[venue])));
        }
        assert_eq!(
            s.world.boss_fight.normalized_health,
            (6 - venue) as f32 / 6.
        );
        if venue > 0 {
            let m = magicka_runtime::ai_event::AiEventLevel::locator(
                &s.world,
                custom_hash(&format!("start{}0", VENUES[venue])),
            )
            .unwrap();
            let p = s.body().position;
            assert!(
                (p[0] - m[12]).abs() < 0.25 && (p[2] - m[14]).abs() < 0.25,
                "teleport to venue {venue}: player {p:?}, locator {:?}",
                &m[12..15]
            );
        }
        for h in handles {
            EntityCommandHost::kill(&mut s.world, h).unwrap();
        }
        for _ in 0..600 {
            step(&mut s);
            if host(&s).dreams.health == (5 - venue) as f32 {
                break;
            }
        }
        assert_eq!(
            host(&s).dreams.health,
            (5 - venue) as f32,
            "remaining minions {:?}",
            host(&s)
                .minions
                .iter()
                .map(|h| {
                    let n = &s.world.npcs[h];
                    (
                        h,
                        n.character.combat.health.hit_points,
                        n.dead(),
                        n.motion.controller.clip_name(),
                    )
                })
                .collect::<Vec<_>>()
        );
        assert!(
            !s.world.boss_fight.dead,
            "native Grimnir Dead remains false"
        );
    }
    assert_eq!(host(&s).dreams.state, State::OutroFade);
    assert_eq!(s.world.boss_fight.normalized_health, 0.);
    assert!(
        s.runtime
            .trigger(custom_hash("nextscene"))
            .unwrap()
            .has_triggered
    );
    for _ in 0..300 {
        step(&mut s);
        if s.scene_name() == "ew_s5" {
            break;
        }
    }
    assert_eq!(s.scene_name(), "ew_s5");
}
#[test]
fn checkpoint_recreates_first_dream_and_the_original_idle_rig() {
    let Some(mut s) = session() else { return };
    wait_wave(&mut s, 0);
    let local = host(&s).pose.local.clone();
    let parent = host(&s).pose.parent;
    assert!(!local.is_empty());
    assert!(parent.iter().all(|v| v.is_finite()));
    assert!((parent[0].abs() - 7.).abs() < 1e-5);
    for _ in 0..30 {
        step(&mut s);
    }
    assert_ne!(host(&s).pose.parent, parent);
    assert_ne!(host(&s).pose.local, local);
    // BossFight updates before the camera advances later in this world step.
    let ground = s.world.camera.ground_position();
    s.world
        .camera
        .set_inspection_view(Some(magicka_runtime::camera::CameraView {
            position: [10000.; 3],
            direction: [0., 0., -1.],
            up: [0., 1., 0.],
            fov: 1.,
        }));
    step(&mut s);
    let pose = host(&s).pose.parent;
    assert!((pose[12] - ground[0]).abs() <= 0.126);
    assert!((pose[13] - ground[1] + 28.).abs() <= 0.126);
    assert!((pose[14] - ground[2] + 30.).abs() < 1e-4);
    s.world.camera.set_inspection_view(None);
    for h in host(&s).minions.clone() {
        EntityCommandHost::kill(&mut s.world, h).unwrap();
    }
    wait_wave(&mut s, 1);
    assert!(s.restart_at_checkpoint().unwrap());
    step(&mut s);
    assert_eq!(host(&s).dreams.health, 6.);
    assert_eq!(host(&s).dreams.venue, 0);
    assert_eq!(s.world.boss_fight.normalized_health, 1.);
    wait_wave(&mut s, 0);
}
