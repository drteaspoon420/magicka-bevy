//! Plan suite `network/faults` (codec half): baselines and deltas never
//! produce a frame the host did not send.
use magicka_network::{
    delta::{decode, encode, Decoder, DeltaError, Encoder, Payload},
    session::*,
    PhysicsFrame, RemoteBody, PROTOCOL_VERSION,
};
use magicka_physics::{BodyKey, BodyState};

fn body(handle: u16, x: f32) -> RemoteBody {
    RemoteBody::new(
        BodyKey {
            handle,
            generation: 0,
        },
        "Models/Items_Wizard/magickbook_major".into(),
        [0.; 3],
        BodyState::at([x, 1.0, 0.0]),
    )
}
fn frame(tick: u64, bodies: Vec<RemoteBody>) -> GameFrame {
    GameFrame {
        physics: PhysicsFrame {
            version: PROTOCOL_VERSION,
            session: 9,
            epoch: 1,
            tick,
            scene: "lab".into(),
            bodies,
        },
        level: "feature-lab.lvl".into(),
        players: vec![],
        npcs: vec![],
        effects: vec![],
        rules: None,
        lobby: Default::default(),
        participants: vec![],
        world: Default::default(),
        mod_ui: vec![],
        time: tick as f64 / 60.0,
        events: vec![],
    }
}

#[test]
fn deltas_reproduce_adds_removes_changes_and_reorders_exactly() {
    let a = frame(1, vec![body(1, 0.), body(2, 0.), body(3, 0.)]);
    let mut b = frame(2, vec![body(3, 0.), body(1, 5.), body(4, 0.)]);
    b.mod_ui = vec![ModUiEntry {
        table: "boosted".into(),
        key: "gold".into(),
        json: "12".into(),
    }];
    b.events = vec![PresentationEventState {
        id: 1,
        sound: false,
        name: "ex:storm_fx".into(),
        position: Some([1.0, 0.0, 2.0]),
        owner: None,
        tick: 2,
    }];
    let d = encode(&a, &b);
    assert_eq!(d.bodies.removed, vec![2]);
    assert_eq!(d.bodies.changed.len(), 2, "moved body 1 and new body 4");
    assert!(d.bodies.order.is_some(), "3 now precedes 1");
    assert_eq!(decode(&a, &d).unwrap(), b);
    // An unchanged frame is an empty delta.
    let mut same = a.clone();
    same.physics.tick = 3;
    assert!(encode(&a, &same).is_empty());
    // Removing a key the base lacks is inconsistent.
    let mut wrong = d.clone();
    wrong.bodies.removed.push(77);
    assert_eq!(decode(&a, &wrong), Err(DeltaError::Inconsistent));
}

#[test]
fn encoder_uses_each_clients_ack_and_decoder_refuses_unknown_bases() {
    let mut enc = Encoder::default();
    let mut dec = Decoder::default();
    let id1 = enc.push(frame(1, vec![body(1, 0.), body(2, 0.)]));
    // No ack yet: full frame.
    let u1 = enc.update_for(None).unwrap();
    assert!(matches!(u1.payload, Payload::Full(_)));
    assert_eq!(dec.apply(u1).unwrap().physics.tick, 1);
    assert_eq!(dec.acknowledged(), Some(id1));
    let id2 = enc.push(frame(2, vec![body(1, 1.), body(2, 0.)]));
    let u2 = enc.update_for(Some(id1)).unwrap();
    assert_eq!(u2.base, Some(id1));
    let Payload::Delta(d) = &u2.payload else {
        panic!("expected a delta")
    };
    assert_eq!(d.bodies.changed.len(), 1);
    // Lost: the client keeps acknowledging id1; the next delta still uses it.
    let id3 = enc.push(frame(3, vec![body(1, 2.), body(2, 0.)]));
    let u3 = enc.update_for(Some(id1)).unwrap();
    assert_eq!(u3.base, Some(id1));
    assert_eq!(
        dec.apply(u3).unwrap(),
        frame(3, vec![body(1, 2.), body(2, 0.)])
    );
    assert_eq!(dec.acknowledged(), Some(id3));
    // A late delta and a delta against a frame the client never held are refused.
    assert_eq!(dec.apply(u2), Err(DeltaError::Inconsistent));
    let mut other = Decoder::default();
    enc.push(frame(4, vec![]));
    assert_eq!(
        other.apply(enc.update_for(Some(id2)).unwrap()),
        Err(DeltaError::MissingBase)
    );
    // An acknowledgement older than the history window falls back to full.
    for t in 5..100 {
        enc.push(frame(t, vec![body(1, t as f32)]));
    }
    assert!(matches!(
        enc.update_for(Some(id1)).unwrap().payload,
        Payload::Full(_)
    ));
    enc.full_only = true;
    let latest = enc.current().unwrap().0;
    assert!(matches!(
        enc.update_for(Some(latest - 1)).unwrap().payload,
        Payload::Full(_)
    ));
}

#[test]
fn lossy_jittery_links_only_ever_present_frames_the_host_sent() {
    use magicka_network::transport::*;
    let mut host = LoopbackHost::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let conditions = NetConditions {
        latency: std::time::Duration::from_millis(30),
        jitter: std::time::Duration::from_millis(20),
        loss: 0.1,
    };
    host.set_conditions(Some(conditions));
    let mut client = LoopbackClient::join(
        host.address(),
        JoinRequest {
            conditions: Some(conditions),
            ..Default::default()
        },
    )
    .unwrap();
    let mut sent = std::collections::BTreeMap::new();
    let mut seen = Vec::new();
    let start = std::time::Instant::now();
    let mut tick = 0;
    while start.elapsed() < std::time::Duration::from_secs(4) {
        client.update();
        host.update();
        tick += 1;
        // Bodies come and go and move, so deltas carry every change kind.
        let bodies: Vec<_> = (0..8u16)
            .filter(|h| (tick / 10 + u64::from(*h)) % 5 != 0)
            .map(|h| body(h, (tick as f32 * 0.1 + f32::from(h)).sin()))
            .collect();
        let f = frame(tick, bodies);
        if host.broadcast_game(&f).unwrap() > 0 {
            sent.insert(tick, f);
        }
        seen.extend(client.take_game_frames());
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(seen.len() > 100, "{} frames arrived", seen.len());
    for f in &seen {
        assert_eq!(
            Some(f),
            sent.get(&f.physics.tick),
            "a frame the host never sent"
        );
    }
    let t = host.traffic();
    assert!(t
        .sent
        .get("GameUpdate.delta")
        .is_some_and(|k| k.messages > 50));
    let ticks: Vec<u64> = seen.iter().map(|f| f.physics.tick).collect();
    assert!(
        ticks.windows(2).all(|w| w[0] < w[1]),
        "monotonic presentation"
    );
}

#[test]
fn interest_culls_far_presentation_but_never_gameplay() {
    use magicka_network::{delta::interest, transport::*};
    let mut f = frame(5, vec![body(1, 0.0), body(2, 500.0)]);
    let mut player = PlayerState {
        slot: 1,
        handle: 40,
        profile: Default::default(),
        position: [0.0; 3],
        right: [1., 0., 0.],
        up: [0., 1., 0.],
        backward: [0., 0., 1.],
        animation: 0,
        animation_time: 0.,
        animation_speed: 1.,
        health: 100.,
        dead: false,
        items: ["sword".into(), "staff".into()],
        attached_slots: vec![],
        unlocked_magicks: 0,
        queue: vec![],
        item_queues: [vec![], vec![]],
        last_input: 0,
        registered_magicks: vec![],
    };
    f.players.push(player.clone());
    player.slot = 2;
    player.handle = 41;
    player.position = [300.0, 0.0, 0.0];
    f.players.push(player);
    let effect = |slot: u16, x: f32| VisualEffectState {
        slot,
        instance: 1,
        definition: 5,
        transform: {
            let mut m = [0.0; 16];
            m[0] = 1.0;
            m[5] = 1.0;
            m[10] = 1.0;
            m[15] = 1.0;
            m[12] = x;
            m
        },
        started_ms: 0,
    };
    f.effects = vec![effect(0, 5.0), effect(1, 300.0)];
    let event = |id: u64, x: f32, owner: Option<u16>| PresentationEventState {
        id,
        sound: true,
        name: "cue".into(),
        position: Some([x, 0.0, 0.0]),
        owner,
        tick: 5,
    };
    f.events = vec![
        event(1, 3.0, None),
        event(2, 300.0, None),
        event(3, 300.0, Some(40)),
    ];
    assert!(f.valid());
    let view = interest(&f, 1, 50.0);
    assert_eq!(view.effects.iter().map(|e| e.slot).collect::<Vec<_>>(), [0]);
    assert_eq!(
        view.events.iter().map(|e| e.id).collect::<Vec<_>>(),
        [1, 3],
        "own events always"
    );
    assert_eq!(view.players, f.players);
    assert_eq!(view.physics, f.physics);
    // Interest views still delta exactly over the transport, and a
    // spectator receives everything.
    let mut host = LoopbackHost::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    host.set_interest_radius(Some(50.0));
    let mut p = LoopbackClient::connect(host.address()).unwrap();
    let mut s =
        LoopbackClient::connect_as(host.address(), magicka_network::lobby::Role::Spectator, "S")
            .unwrap();
    let mut got = (None, None);
    for tick in 0..400 {
        p.update();
        s.update();
        host.update();
        if let Some(slot) = p.player_slot() {
            let mut g = f.clone();
            g.physics.tick = 10 + tick;
            g.time = g.physics.tick as f64 / 60.0;
            g.players[0].slot = slot;
            g.players.sort_by_key(|x| x.slot);
            host.broadcast_game(&g).unwrap();
        }
        if let Some(x) = p.take_game_frames().pop() {
            got.0 = Some(x);
        }
        if let Some(x) = s.take_game_frames().pop() {
            got.1 = Some(x);
        }
        if tick > 100 && got.0.is_some() && got.1.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(got.0.unwrap().effects.len(), 1);
    assert_eq!(got.1.unwrap().effects.len(), 2);
}
