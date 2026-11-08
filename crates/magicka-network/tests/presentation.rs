use bevy::math::{Mat3, Mat4, Quat, Vec3};
use magicka_network::{presentation::*, session::*, transport::*, *};
use magicka_physics::{BodyKey, BodyState};
use std::time::{Duration, Instant};

fn frame(tick: u64, x: f32) -> GameFrame {
    GameFrame {
        lobby: Default::default(),
        participants: Vec::new(),
        world: Default::default(),
        physics: PhysicsFrame {
            version: PROTOCOL_VERSION,
            session: 1,
            epoch: 1,
            tick,
            scene: "lab".into(),
            bodies: vec![RemoteBody::new(
                BodyKey {
                    handle: 5,
                    generation: 0,
                },
                "Models/Items_Wizard/magickbook_major".into(),
                [0.; 3],
                BodyState::at([x, 1., 0.]),
            )],
        },
        level: "lab.lvl".into(),
        players: vec![PlayerState {
            slot: 0,
            handle: 10,
            profile: PlayerProfile::default(),
            position: [x, 2., 0.],
            right: [1., 0., 0.],
            up: [0., 1., 0.],
            backward: [0., 0., 1.],
            animation: 1,
            animation_time: x,
            animation_speed: 1.,
            health: 100.,
            dead: false,
            items: ["sword".into(), "staff".into()],
            attached_slots: vec![0, 1],
            unlocked_magicks: 0,
            queue: vec![],
            item_queues: [vec![], vec![]],
            last_input: 0,
            registered_magicks: vec![],
        }],
        npcs: vec![],
        effects: vec![],
        rules: None,
        mod_ui: vec![],
        time: tick as f64 / 60.0,
        events: vec![],
    }
}
fn at(start: Instant, ms: u64) -> Instant {
    start + Duration::from_millis(ms)
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.0001, "{a} != {b}");
}

#[test]
fn poses_and_rotation_blend_while_gameplay_state_is_immediate() {
    let start = Instant::now();
    let mut view = GamePresentation::default();
    view.apply(frame(1, 0.), start).unwrap();
    assert_eq!(view.sample(start).unwrap(), frame(1, 0.));
    let mut latest = frame(4, 10.);
    let rotation = Mat3::from_quat(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
    latest.players[0].right = rotation.x_axis.to_array();
    latest.players[0].up = rotation.y_axis.to_array();
    latest.players[0].backward = rotation.z_axis.to_array();
    latest.physics.bodies[0].rotation =
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2).to_array();
    latest.players[0].health = 37.;
    latest.players[0].items[0] = "new_sword".into();
    latest.players[0].unlocked_magicks = 8;
    latest.players[0]
        .queue
        .push(QueuedSpell::from(&magicka_sim::spells::Spell::default_for(
            magicka_sim::spells::Elements::FIRE,
        )));
    latest.players[0].item_queues[0] = latest.players[0].queue.clone();
    latest.players[0].attached_slots.clear();
    view.apply(latest.clone(), at(start, 50)).unwrap();
    let sample = view.sample(at(start, 75)).unwrap();
    near(sample.players[0].position[0], 5.);
    near(sample.physics.bodies[0].position[0], 5.);
    near(sample.players[0].animation_time, 5.);
    near(sample.players[0].right[0], std::f32::consts::FRAC_1_SQRT_2);
    near(sample.players[0].right[2], -std::f32::consts::FRAC_1_SQRT_2);
    assert!(sample.valid());
    assert_eq!(sample.players[0].health, 37.);
    assert_eq!(sample.players[0].items, latest.players[0].items);
    assert_eq!(sample.players[0].queue, latest.players[0].queue);
    assert_eq!(sample.players[0].item_queues, latest.players[0].item_queues);
    assert_eq!(
        sample.players[0].attached_slots,
        latest.players[0].attached_slots
    );
    assert_eq!(sample.players[0].unlocked_magicks, 8);
    assert_eq!(view.current(), Some(&latest));
    assert_eq!(
        view.sample(at(start, 5000)).unwrap(),
        latest,
        "loss must hold confirmed state"
    );
}

#[test]
fn attachment_slots_require_nonnegative_unique_canonical_order() {
    for slots in [vec![], vec![0], vec![0, 1, 2]] {
        let mut f = frame(1, 0.);
        f.players[0].attached_slots = slots;
        assert!(f.valid());
    }
    for slots in [vec![-1], vec![0, 0], vec![1, 0]] {
        let mut f = frame(1, 0.);
        f.players[0].attached_slots = slots;
        assert!(!f.valid());
    }
}

#[test]
fn irregular_timing_and_packet_bursts_do_not_assume_a_host_tick_rate() {
    let start = Instant::now();
    let mut view = GamePresentation::default();
    view.apply(frame(u64::MAX - 10, 0.), start).unwrap();
    view.apply(frame(u64::MAX - 9, 20.), at(start, 100))
        .unwrap();
    near(
        view.sample(at(start, 100)).unwrap().players[0].position[0],
        10.,
    );
    view.apply(frame(u64::MAX - 5, 30.), at(start, 100))
        .unwrap();
    near(
        view.sample(at(start, 100)).unwrap().players[0].position[0],
        15.,
    );
    view.apply(frame(u64::MAX, 40.), at(start, 120)).unwrap();
    near(
        view.sample(at(start, 160)).unwrap().players[0].position[0],
        35.,
    );
    for i in 0..100 {
        let mut f = frame(i, i as f32);
        f.physics.epoch = 2;
        view.apply(f, at(start, 200 + i)).unwrap();
    }
    assert!(view.sample(at(start, 270)).unwrap().valid());
}

#[test]
fn removal_replacement_and_scene_restart_never_blend_old_lifetimes() {
    let start = Instant::now();
    let mut view = GamePresentation::default();
    view.apply(frame(1, 0.), start).unwrap();
    let mut removed = frame(2, 10.);
    removed.players.clear();
    removed.physics.bodies.clear();
    view.apply(removed, at(start, 50)).unwrap();
    let sample = view.sample(at(start, 50)).unwrap();
    assert!(sample.players.is_empty() && sample.physics.bodies.is_empty());
    let mut rejoined = frame(3, 20.);
    rejoined.players[0].handle = 11;
    rejoined.physics.bodies[0].generation = 1;
    view.apply(rejoined.clone(), at(start, 60)).unwrap();
    assert_eq!(view.sample(at(start, 60)).unwrap(), rejoined);
    let mut restart = frame(0, 100.);
    restart.physics.epoch = 2;
    view.apply(restart.clone(), at(start, 70)).unwrap();
    assert_eq!(view.sample(at(start, 70)).unwrap(), restart);
    assert_eq!(
        view.apply(frame(100, 0.), at(start, 80)),
        Err(FrameError::Stale)
    );
    assert_eq!(view.current(), Some(&restart));
}

#[test]
fn death_and_respawn_clear_all_buffered_player_poses() {
    let start = Instant::now();
    let mut view = GamePresentation::default();
    for i in 0..3 {
        view.apply(frame(i, i as f32), at(start, i * 10)).unwrap();
    }
    let mut dead = frame(3, 30.);
    dead.players[0].dead = true;
    view.apply(dead, at(start, 30)).unwrap();
    assert_eq!(
        view.sample(at(start, 30)).unwrap().players[0].position[0],
        30.
    );
    let alive = frame(4, 100.);
    view.apply(alive.clone(), at(start, 40)).unwrap();
    assert_eq!(view.sample(at(start, 40)).unwrap().players, alive.players);
}

#[test]
fn animation_changes_and_loops_do_not_blend_incompatible_clip_times() {
    let start = Instant::now();
    let mut view = GamePresentation::default();
    view.apply(frame(1, 10.), start).unwrap();
    let mut next = frame(2, 20.);
    next.players[0].animation = 2;
    next.players[0].animation_time = 0.1;
    view.apply(next, at(start, 50)).unwrap();
    assert_eq!(
        view.sample(at(start, 75)).unwrap().players[0].animation_time,
        0.1
    );
    let mut looped = frame(3, 30.);
    looped.players[0].animation = 2;
    looped.players[0].animation_time = 0.01;
    view.apply(looped, at(start, 100)).unwrap();
    assert_eq!(
        view.sample(at(start, 125)).unwrap().players[0].animation_time,
        0.01
    );
}

#[test]
fn invalid_and_delayed_snapshots_leave_state_and_clock_unchanged() {
    let start = Instant::now();
    let mut view = GamePresentation::default();
    let valid = frame(1, 0.);
    view.apply(valid.clone(), start).unwrap();
    let mut invalid = frame(2, 10.);
    invalid.players[0].up = invalid.players[0].right;
    assert_eq!(
        view.apply(invalid, at(start, 50)),
        Err(FrameError::InvalidBody)
    );
    let mut invalid = frame(2, 10.);
    let mut duplicate = invalid.players[0].clone();
    duplicate.slot = 1;
    invalid.players.push(duplicate);
    assert_eq!(
        view.apply(invalid, at(start, 50)),
        Err(FrameError::InvalidBody)
    );
    let mut invalid = frame(2, 10.);
    invalid.level = "other.lvl".into();
    assert_eq!(view.apply(invalid, at(start, 50)), Err(FrameError::Scene));
    let mut invalid = frame(2, 10.);
    invalid.physics.session = 2;
    assert_eq!(view.apply(invalid, at(start, 50)), Err(FrameError::Session));
    assert_eq!(
        view.apply(frame(1, 20.), at(start, 50)),
        Err(FrameError::Stale)
    );
    assert_eq!(view.current(), Some(&valid));
    assert_eq!(view.sample(at(start, 75)).unwrap(), valid);
    view.apply(frame(2, 10.), at(start, 50)).unwrap();
    assert_eq!(view.apply(frame(3, 20.), start), Err(FrameError::Stale));
    near(
        view.sample(at(start, 75)).unwrap().players[0].position[0],
        5.,
    );
}

#[test]
fn two_real_udp_clients_sample_host_poses_and_remove_consumed_props() {
    let mut host = LoopbackHost::bind_mode("127.0.0.1:0".parse().unwrap(), true).unwrap();
    let mut clients: [_; 2] =
        std::array::from_fn(|_| LoopbackClient::connect(host.address()).unwrap());
    let mut views: [_; 2] = std::array::from_fn(|_| GamePresentation::default());
    let start = Instant::now();
    for (stage, latest) in [npc_frame(1, 0.), npc_frame(4, 10.)]
        .into_iter()
        .enumerate()
    {
        let deadline = Instant::now() + Duration::from_secs(5);
        while views.iter().any(|v| {
            v.current()
                .is_none_or(|f| f.physics.tick != latest.physics.tick)
        }) {
            assert!(
                Instant::now() < deadline,
                "clients did not receive host stage {stage}"
            );
            for (client, view) in clients.iter_mut().zip(&mut views) {
                client.update();
                for received in client.take_game_frames() {
                    let _ = view.apply(received, at(start, stage as u64 * 50));
                }
            }
            host.update();
            host.broadcast_game(&latest).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    for view in &views {
        let sampled = view.sample(at(start, 75)).unwrap();
        near(sampled.players[0].position[0], 5.);
        near(sampled.physics.bodies[0].position[0], 5.);
        near(sampled.npcs[0].transform[12], 5.);
        near(sampled.npcs[0].clip_time, 5.);
        assert_eq!(view.current().unwrap(), &npc_frame(4, 10.));
    }
    let mut removed = npc_frame(7, 20.);
    removed.npcs.clear();
    removed.physics.bodies.clear();
    let deadline = Instant::now() + Duration::from_secs(5);
    while views.iter().any(|v| v.current().unwrap().physics.tick != 7) {
        assert!(Instant::now() < deadline, "clients did not receive removal");
        for (client, view) in clients.iter_mut().zip(&mut views) {
            client.update();
            for received in client.take_game_frames() {
                let _ = view.apply(received, at(start, 100));
            }
        }
        host.update();
        host.broadcast_game(&removed).unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(views.iter().all(|v| {
        let sample = v.sample(at(start, 100)).unwrap();
        sample.physics.bodies.is_empty() && sample.npcs.is_empty()
    }));
}

fn npc_frame(tick: u64, x: f32) -> GameFrame {
    let mut f = frame(tick, x);
    f.npcs.push(NpcState {
        handle: 20,
        template: "Goblin".into(),
        transform: Mat4::from_translation(Vec3::new(x, 0., 0.)).to_cols_array(),
        clip: Some("walk".into()),
        clip_time: x,
        looping: true,
        health: 100.,
        dead: false,
        max_health: 100.,
        health_bars: 1,
        radius: 0.5,
    });
    f
}

#[test]
fn npc_pose_blends_without_delaying_health_or_effects() {
    let start = Instant::now();
    let mut view = GamePresentation::default();
    view.apply(npc_frame(1, 0.), start).unwrap();
    let mut latest = npc_frame(2, 10.);
    latest.npcs[0].transform = Mat4::from_scale_rotation_translation(
        Vec3::new(3., 5., 7.),
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        Vec3::new(10., 0., 0.),
    )
    .to_cols_array();
    latest.npcs[0].health = 23.;
    latest.effects.push(VisualEffectState {
        slot: 0,
        instance: 1,
        definition: 5,
        transform: Mat4::IDENTITY.to_cols_array(),
        started_ms: 0,
    });
    view.apply(latest.clone(), at(start, 50)).unwrap();
    let sampled = view.sample(at(start, 75)).unwrap();
    let (scale, rotation, position) =
        Mat4::from_cols_array(&sampled.npcs[0].transform).to_scale_rotation_translation();
    near(position.x, 5.);
    near(scale.x, 2.);
    near(scale.y, 3.);
    near(scale.z, 4.);
    let right = rotation * Vec3::X;
    near(right.x, std::f32::consts::FRAC_1_SQRT_2);
    near(right.z, -std::f32::consts::FRAC_1_SQRT_2);
    near(sampled.npcs[0].clip_time, 5.);
    assert_eq!(sampled.npcs[0].health, 23.);
    assert_eq!(sampled.effects, latest.effects);
    assert!(sampled.valid());
    assert_eq!(view.sample(at(start, 5000)).unwrap(), latest);
}

#[test]
fn npc_omission_template_change_death_and_respawn_clear_history() {
    let start = Instant::now();
    let mut view = GamePresentation::default();
    view.apply(npc_frame(1, 0.), start).unwrap();
    view.apply(npc_frame(2, 10.), at(start, 10)).unwrap();
    let mut removed = npc_frame(3, 20.);
    removed.npcs.clear();
    view.apply(removed, at(start, 20)).unwrap();
    assert!(view.sample(at(start, 20)).unwrap().npcs.is_empty());
    // Even reuse of the same handle/template cannot revive an omitted pose.
    let rejoined = npc_frame(4, 100.);
    view.apply(rejoined.clone(), at(start, 30)).unwrap();
    assert_eq!(view.sample(at(start, 30)).unwrap().npcs, rejoined.npcs);
    let mut replaced = npc_frame(5, 200.);
    replaced.npcs[0].template = "Troll".into();
    view.apply(replaced.clone(), at(start, 40)).unwrap();
    assert_eq!(view.sample(at(start, 40)).unwrap().npcs, replaced.npcs);
    let mut dead = replaced.clone();
    dead.physics.tick = 6;
    dead.npcs[0].dead = true;
    dead.npcs[0].transform[12] = 300.;
    view.apply(dead.clone(), at(start, 50)).unwrap();
    assert_eq!(view.sample(at(start, 50)).unwrap().npcs, dead.npcs);
    let mut respawn = replaced;
    respawn.physics.tick = 7;
    respawn.npcs[0].transform[12] = 400.;
    view.apply(respawn.clone(), at(start, 60)).unwrap();
    assert_eq!(view.sample(at(start, 60)).unwrap().npcs, respawn.npcs);
    let mut restart = npc_frame(0, 1000.);
    restart.physics.epoch = 2;
    view.apply(restart.clone(), at(start, 70)).unwrap();
    assert_eq!(view.sample(at(start, 70)).unwrap(), restart);
}

#[test]
fn npc_incompatible_clip_times_and_loop_modes_snap() {
    let start = Instant::now();
    for mode in 0..3 {
        let mut view = GamePresentation::default();
        view.apply(npc_frame(1, 10.), start).unwrap();
        let mut latest = npc_frame(2, 20.);
        match mode {
            0 => latest.npcs[0].clip = Some("attack".into()),
            1 => latest.npcs[0].clip_time = 0.01,
            _ => latest.npcs[0].looping = false,
        }
        view.apply(latest.clone(), at(start, 50)).unwrap();
        let sampled = view.sample(at(start, 75)).unwrap();
        near(sampled.npcs[0].transform[12], 15.);
        assert_eq!(sampled.npcs[0].clip_time, latest.npcs[0].clip_time);
        assert_eq!(sampled.npcs[0].clip, latest.npcs[0].clip);
        assert_eq!(sampled.npcs[0].looping, latest.npcs[0].looping);
    }
}

#[test]
fn unsupported_npc_matrices_hold_latest_pose_without_nan() {
    let start = Instant::now();
    let mut transforms = vec![Mat4::ZERO.to_cols_array()];
    let mut shear = Mat4::IDENTITY.to_cols_array();
    shear[4] = 0.5;
    transforms.push(shear);
    let mut projective = Mat4::IDENTITY.to_cols_array();
    projective[3] = 0.5;
    transforms.push(projective);
    transforms.push(Mat4::from_scale(Vec3::new(-1., 1., 1.)).to_cols_array());
    for transform in transforms {
        let mut view = GamePresentation::default();
        view.apply(npc_frame(1, 0.), start).unwrap();
        let mut latest = npc_frame(2, 10.);
        latest.npcs[0].transform = transform;
        assert!(latest.valid());
        view.apply(latest.clone(), at(start, 50)).unwrap();
        let sampled = view.sample(at(start, 75)).unwrap();
        assert_eq!(sampled.npcs[0].transform, latest.npcs[0].transform);
        assert!(sampled.valid());
    }
}
