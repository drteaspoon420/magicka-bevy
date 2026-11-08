use magicka_network::{control::*, transport::*};
use magicka_runtime::play_session::PlaySession;
use magicka_sim::spells::Elements;
use std::{collections::BTreeSet, path::PathBuf};

fn exchange(
    host: &mut LoopbackHost,
    clients: &mut [LoopbackClient; 2],
    authority: &mut InputAuthority,
    session: &mut PlaySession,
    epoch: u64,
    sequence: u64,
    controls: [Controls; 2],
) {
    for (client, controls) in clients.iter_mut().zip(controls) {
        client
            .send_input(InputCommand {
                session: 1,
                epoch,
                sequence,
                controls,
                history: Vec::new(),
            })
            .unwrap();
    }
    let mut accepted = BTreeSet::new();
    for _ in 0..400 {
        for client in clients.iter_mut() {
            client.update();
        }
        host.update();
        for owned in host.take_inputs() {
            let controls = authority.accept(owned.slot, &owned.command).unwrap();
            session.handle_remote_input(owned.slot, controls).unwrap();
            accepted.insert(owned.slot);
        }
        if accepted.len() == 2 {
            return;
        }
    }
    panic!("two inputs did not arrive");
}
#[test]
fn two_owned_wizards_move_conjure_cast_and_survive_scene_rebuilds() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let content = root.join("../../../../target/Magicka/Content");
    let level = root
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap();
    let mut session = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
    session.step(1. / 60.);
    let mut host = LoopbackHost::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let mut clients = [
        LoopbackClient::connect(host.address()).unwrap(),
        LoopbackClient::connect(host.address()).unwrap(),
    ];
    for _ in 0..400 {
        for client in &mut clients {
            client.update();
        }
        host.update();
        if clients.iter().all(|c| c.player_slot().is_some()) {
            break;
        }
    }
    let slots = clients.each_ref().map(|c| c.player_slot().unwrap());
    assert_ne!(slots[0], slots[1]);
    assert_eq!(host.player_slots().len(), 2);
    for slot in slots {
        session.join_player(slot, "Wizard", "start").unwrap();
    }
    let count = session.world.physics.bodies.len();
    assert!(session.join_player(slots[0], "Wizard", "start").is_err());
    assert!(session.join_player(0, "Wizard", "start").is_err());
    assert_eq!(session.world.physics.bodies.len(), count);
    let local_before = session.body().position;
    let before = slots.map(|slot| {
        session.world.physics.bodies[session.world.players[slot].as_ref().unwrap().body].position
    });
    // Keep each wizard in its own lane regardless of UDP arrival order.
    let controls = slots.map(|slot| Controls {
        aim: Some(if slot == 1 {
            [-14., 1., 5.]
        } else {
            [-3., 1., 7.]
        }),
        move_down: true,
        ..Default::default()
    });
    let mut authority = InputAuthority::new(1, 1).unwrap();
    for sequence in 1..=90 {
        exchange(
            &mut host,
            &mut clients,
            &mut authority,
            &mut session,
            1,
            sequence,
            controls,
        );
        session.step(1. / 60.);
    }
    let after = slots.map(|slot| {
        session.world.physics.bodies[session.world.players[slot].as_ref().unwrap().body].position
    });
    for i in 0..2 {
        let delta = after[i][0] - before[i][0];
        assert!(
            if slots[i] == 1 {
                delta < -0.5
            } else {
                delta > 0.5
            },
            "slots {slots:?}: {before:?} -> {after:?}"
        );
    }
    assert!(
        (session.body().position[0] - local_before[0]).abs() < 0.1,
        "local position {:?} -> {:?}",
        local_before,
        session.body().position
    );
    assert_eq!(session.body().movement(), [0.; 3]);
    let mut spells = [Controls::default(); 2];
    spells[0].elements[7] = true; // Fire
    spells[1].elements[3] = true; // Cold
    exchange(
        &mut host,
        &mut clients,
        &mut authority,
        &mut session,
        1,
        91,
        spells,
    );
    session.step(1. / 60.);
    assert!(session.world.players[slots[0]]
        .as_ref()
        .unwrap()
        .conjure
        .queue
        .iter()
        .any(|s| s.element == Elements::FIRE));
    assert!(session.world.players[slots[1]]
        .as_ref()
        .unwrap()
        .conjure
        .queue
        .iter()
        .any(|s| s.element == Elements::COLD));
    for input in &mut spells {
        input.elements = [false; 8];
        input.cast_down = true;
        input.aim = Some([-8., 1., -8.]);
    }
    let mut cast = [false; 2];
    for sequence in 92..=240 {
        exchange(
            &mut host,
            &mut clients,
            &mut authority,
            &mut session,
            1,
            sequence,
            spells,
        );
        session.step(1. / 60.);
        for (i, slot) in slots.into_iter().enumerate() {
            if let Some(magicka_runtime::avatar::SpellEffect::ForceSpray(spray)) =
                &session.world.players[slot].as_ref().unwrap().current_spell
            {
                assert_eq!(
                    spray.spell.element,
                    if i == 0 {
                        Elements::FIRE
                    } else {
                        Elements::COLD
                    }
                );
                cast[i] = true;
            }
        }
        if cast == [true, true] {
            break;
        }
    }
    assert_eq!(
        cast,
        [true, true],
        "both owned avatars must create live spells"
    );
    assert!(session.avatar().current_spell.is_none());
    assert!(session.avatar().conjure.queue.is_empty());
    exchange(
        &mut host,
        &mut clients,
        &mut authority,
        &mut session,
        1,
        241,
        [Controls::default(); 2],
    );
    session.change_scene_named("corridor", "start").unwrap();
    assert_eq!(session.physics_frame(1, 0).unwrap().epoch, 2);
    for slot in slots {
        assert!(session.world.players[slot].is_some());
    }
    authority.advance_epoch(2).unwrap();
    exchange(
        &mut host,
        &mut clients,
        &mut authority,
        &mut session,
        2,
        1,
        [Controls::default(); 2],
    );
    session.step(1. / 60.);
    assert!(session.restart_at_checkpoint().unwrap());
    assert_eq!(session.physics_frame(1, 0).unwrap().epoch, 3);
    for slot in slots {
        assert!(session.world.players[slot].is_some());
    }
    authority.advance_epoch(3).unwrap();
    exchange(
        &mut host,
        &mut clients,
        &mut authority,
        &mut session,
        3,
        1,
        [Controls::default(); 2],
    );
    println!("two assigned remote slots: independent movement, Fire/Cold queues, live casting, unchanged local ownership, scene change/restart persistence");
}
