use magicka_runtime::{
    audio::AudioRequest,
    avatar::{AvatarEvent, PlayerInput},
    play_session::{PlaySession, SessionEvent},
    scene_loading::custom_hash,
};
use std::{collections::BTreeMap, path::Path};

const DT: f32 = 1. / 60.;

fn frame(session: &mut PlaySession, held: bool, area: bool) {
    session.events.clear();
    session.handle_input(
        PlayerInput {
            cursor_ndc: Some([0., 0.5]),
            cast_down: held,
            shift_down: area,
            ..Default::default()
        },
        16. / 9.,
    );
    session.step(DT);
}

#[test]
fn player_force_and_area_push_redirect_an_ordinary_bomber_bomb_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    for (area, charged) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut session = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        session.disable_scripts();
        session.world.flags.cutscene = false;
        session.world.flags.input_limited = false;
        for _ in 0..60 {
            frame(&mut session, false, false);
        }
        // Use open road so the original scene segment test does not find
        // the cart beside the start locator.
        let body = session.avatar().body;
        session.world.physics.bodies[body].position[0] = 0.;
        session.world.physics.bodies[body].position[2] = 0.;
        for _ in 0..60 {
            frame(&mut session, false, false);
        }
        if charged {
            for _ in 0..300 {
                frame(&mut session, true, area);
                if session.avatar().character.spell_power == 1. {
                    break;
                }
            }
            assert_eq!(session.avatar().character.spell_power, 1.);
        }
        let owner = session.spawn_npc("Goblin_bomber", "attack", 2.5).unwrap();
        let mut first = None;
        for _ in 0..180 {
            frame(&mut session, charged, area);
            if let Some(m) = session
                .world
                .missiles
                .iter()
                .find(|m| m.body.owner == owner)
            {
                first = Some((m.body.id, m.handle.unwrap(), m.body.velocity));
                break;
            }
        }
        let (id, handle, initial_velocity) = first.expect("ordinary bomber launch");
        let launch_time = session.play_time;
        let mut cue_missiles = BTreeMap::new();
        for event in &session.events {
            if let SessionEvent::MissileSound { missile, key } = event {
                cue_missiles.insert(*key, *missile);
            }
        }
        if !charged {
            frame(&mut session, true, area);
        }
        let mut hits = 0;
        let mut redirected = None;
        let mut fuse = None;
        let mut moved_after_push = false;
        let mut fuse_stopped = false;
        let mut explosion_age = None;
        for _ in 0..300 {
            frame(&mut session, false, false);
            for event in &session.events {
                if let SessionEvent::MissileSound { missile, key } = event {
                    cue_missiles.insert(*key, *missile);
                }
                if matches!(event, SessionEvent::Avatar(AvatarEvent::PushHit { target }) if *target == handle)
                {
                    hits += 1;
                    let missile = session
                        .world
                        .missiles
                        .iter()
                        .find(|m| m.body.id == id)
                        .expect("push should redirect the bomb, not remove it");
                    let caster = session.body().position;
                    let outward = (missile.body.position[0] - caster[0]) * missile.body.velocity[0]
                        + (missile.body.position[2] - caster[2]) * missile.body.velocity[2];
                    assert!(outward > 0., "bomb still travelling toward caster");
                    assert!(missile.body.velocity[1] > 0., "missing ballistic lift");
                    assert_ne!(missile.body.velocity, initial_velocity);
                    // Independent ballistic range solution for native mass 4,
                    // authored force, distance 1.5, lift angle and
                    // gravity. Incoming velocity must be replaced, not added.
                    let Some(magicka_runtime::avatar::SpellEffect::Push(push)) =
                        &session.avatar().current_spell
                    else {
                        panic!("PushHit without active push");
                    };
                    if charged {
                        assert_eq!(push.force, 550.);
                    } else {
                        assert_eq!(push.force, 50.);
                    }
                    let angle = 0.17453292_f64 + push.elevation as f64 * std::f64::consts::PI / 8.;
                    let range = 1.5 * 2. * (1.25_f64 - 4. / push.force as f64).sqrt();
                    let speed = 1.4 * (19.64 * range / (2. * angle).sin()).sqrt();
                    let horizontal = (missile.body.velocity[0].powi(2)
                        + missile.body.velocity[2].powi(2))
                    .sqrt();
                    assert!((horizontal as f64 - speed * angle.cos()).abs() < 0.001);
                    assert!((missile.body.velocity[1] as f64 - speed * angle.sin()).abs() < 0.001);
                    redirected = Some(missile.body.velocity);
                }
            }
            for (request, _) in std::mem::take(&mut session.audio) {
                match request {
                    AudioRequest::Start {
                        key,
                        bank: 128,
                        cue,
                        ..
                    } if cue_missiles.get(&key) == Some(&id) => {
                        if cue == custom_hash("wep_bomb_fuse") {
                            fuse = Some(key);
                        } else if cue == custom_hash("wep_bomb_explode") {
                            explosion_age = Some(session.play_time - launch_time);
                        }
                    }
                    AudioRequest::Move { key, .. } if Some(key) == fuse && hits > 0 => {
                        moved_after_push = true
                    }
                    AudioRequest::Stop { key, .. } if Some(key) == fuse => fuse_stopped = true,
                    _ => {}
                }
            }
            if explosion_age.is_some() && fuse_stopped {
                break;
            }
        }
        assert_eq!(hits, 1, "one push must not repeatedly accelerate the bomb");
        assert!(redirected.is_some());
        assert!(moved_after_push, "redirected bomb lost its travelling fuse");
        assert!(fuse_stopped, "pushed bomb's fuse never stopped");
        let age = explosion_age.expect("pushed bomb lost its timed explosion sound");
        assert!(
            (3.9..4.1).contains(&age),
            "push reset or bypassed bomb timer: {age}"
        );
        assert!(!session.world.missiles.iter().any(|m| m.body.id == id));
        eprintln!("{} push charged={charged}: bomb={id} incoming={initial_velocity:?} redirected={redirected:?} hits={hits} timed explosion={age}s", if area { "Area" } else { "Force" });
    }
}
