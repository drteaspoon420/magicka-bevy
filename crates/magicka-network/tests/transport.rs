use magicka_network::{transport::*, *};
use magicka_physics::{BodyKey, BodyState};
#[test]
fn lightyear_udp_two_clients_receive_a_fragmented_full_snapshot() {
    let mut host = LoopbackHost::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let mut a = LoopbackClient::connect(host.address()).unwrap();
    let mut b = LoopbackClient::connect(host.address()).unwrap();
    let frame = PhysicsFrame {
        version: PROTOCOL_VERSION,
        session: 1,
        epoch: 1,
        tick: 1,
        scene: "lab".into(),
        bodies: (0..128)
            .map(|handle| {
                RemoteBody::new(
                    BodyKey {
                        handle,
                        generation: 0,
                    },
                    "Models/Items_Wizard/magickbook_major".into(),
                    [0.; 3],
                    BodyState::at([handle as f32, 2., 0.]),
                )
            })
            .collect(),
    };
    let mut got_a = None;
    let mut got_b = None;
    for _ in 0..400 {
        for f in a.update() {
            got_a = Some(f);
        }
        for f in b.update() {
            got_b = Some(f);
        }
        host.update();
        host.broadcast(&frame).unwrap();
        if got_a.is_some() && got_b.is_some() {
            break;
        }
    }
    assert!(got_a.is_some(), "client A received no frame");
    assert_eq!(got_a, Some(frame.clone()));
    assert_eq!(got_b, Some(frame));
}

#[test]
fn ownership_reserves_local_slot_and_rejects_a_fourth_remote_client() {
    let mut host = LoopbackHost::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let mut clients: [_; 4] =
        std::array::from_fn(|_| LoopbackClient::connect(host.address()).unwrap());
    for _ in 0..300 {
        for client in &mut clients {
            client.update();
        }
        host.update();
    }
    let assigned: std::collections::BTreeSet<_> =
        clients.iter().filter_map(|c| c.player_slot()).collect();
    assert_eq!(assigned, [1, 2, 3].into_iter().collect());
    assert_eq!(host.player_slots(), vec![1, 2, 3]);
    let mut pending = 0;
    for client in &mut clients {
        let command = magicka_network::control::InputCommand {
            session: 1,
            epoch: 1,
            sequence: 1,
            controls: Default::default(),
            history: Vec::new(),
        };
        if client.player_slot().is_some() {
            client.send_input(command).unwrap();
        } else {
            assert!(client.send_input(command).is_err());
            pending += 1;
        }
    }
    assert_eq!(pending, 1);
    let mut received = std::collections::BTreeSet::new();
    for _ in 0..300 {
        for client in &mut clients {
            client.update();
        }
        host.update();
        received.extend(host.take_inputs().into_iter().map(|input| input.slot));
        if received.len() == 3 {
            break;
        }
    }
    assert_eq!(received, assigned);
}
