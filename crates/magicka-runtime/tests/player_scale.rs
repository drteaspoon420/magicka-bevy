//! Sessions at the largest player limit a dedicated server can be configured
//! with (rewrite extension, not retail behaviour).
use magicka_network::{
    lobby::{LobbySettings, PLAYER_SLOT_CAP},
    session::PlayerProfile,
    telemetry::payload_size,
    transport::LoopbackClient,
};
use magicka_runtime::{network_session::HostedSession, play_session::PlaySession};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn lab() -> Option<(PlaySession, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let content: PathBuf = magicka_content::retail_content_dir();
    if !content.is_dir() {
        eprintln!("retail content absent; skipping");
        return None;
    }
    let level = root
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut session = PlaySession::load(&content, &level, "Wizard").unwrap();
    session.disable_scripts();
    session.step(1. / 60.);
    Some((session, level))
}

#[test]
fn a_session_runs_with_every_player_slot_filled() {
    let Some((mut session, _)) = lab() else {
        return;
    };
    for slot in 1..PLAYER_SLOT_CAP {
        session.join_player_at_start(slot, "Wizard").unwrap();
    }
    for _ in 0..10 {
        session.step(1. / 60.);
    }
    for slot in 0..PLAYER_SLOT_CAP {
        let player = session.world.players[slot].as_ref().expect("filled slot");
        assert!(player.playing && !player.dead(), "slot {slot}");
        let position = session.world.physics.bodies[player.body].position;
        assert!(position.iter().all(|v| v.is_finite()), "slot {slot}");
    }
}

#[test]
fn boss_damage_scales_with_the_players_in_play() {
    let Some((mut session, _)) = lab() else {
        return;
    };
    session.step(1. / 60.);
    assert_eq!(
        session.world.boss_damage_divisor, 1.0,
        "one player is native"
    );
    for slot in 1..4 {
        session.join_player_at_start(slot, "Wizard").unwrap();
    }
    session.step(1. / 60.);
    assert_eq!(session.world.boss_damage_divisor, 1.0, "four are native");
    for slot in 4..16 {
        session.join_player_at_start(slot, "Wizard").unwrap();
    }
    session.step(1. / 60.);
    assert_eq!(session.world.boss_damage_divisor, 4.0);
    session.world.boss_scale = false;
    session.step(1. / 60.);
    assert_eq!(session.world.boss_damage_divisor, 1.0, "option off");
}

/// Real UDP clients joining a dedicated host. Run in release:
/// Clients and the host's lease share simulated time: 256 client apps in one
/// process update far slower than real clients, whose heartbeats would then
/// miss the wall-clock lease.
/// `SCALE_PLAYERS=64 cargo test --release --locked -p magicka-runtime --test all
/// player_scale::real_clients -- --ignored --nocapture`
#[test]
#[ignore = "measurement: many client transports; see the comment for the command"]
fn real_clients_fill_a_configured_server() {
    let Some((mut session, level)) = lab() else {
        return;
    };
    let players: usize = std::env::var("SCALE_PLAYERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);
    let mut host = HostedSession::bind_lobby(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        true,
        PlayerProfile::default(),
        &session,
        LobbySettings {
            max_players: players as u16,
            ..Default::default()
        },
    )
    .unwrap();
    let address = host.transport.address();
    let start = Instant::now();
    let mut clock = start;
    let mut clients = Vec::new();
    while clients.len() < players
        || clients
            .iter()
            .any(|c: &LoopbackClient| c.player_slot().is_none())
    {
        assert!(
            start.elapsed() < Duration::from_secs(150),
            "{} of {} connected clients seated; host sees {}; refusals: {:?}",
            clients.iter().filter(|c| c.player_slot().is_some()).count(),
            clients.len(),
            host.transport.player_slots().len(),
            clients
                .iter()
                .filter_map(|c| c.refusal())
                .collect::<std::collections::BTreeSet<_>>(),
        );
        if clients.len() < players {
            clients.push(LoopbackClient::connect(address).unwrap());
        }
        for client in &mut clients {
            client.update();
        }
        clock += Duration::from_secs_f64(1. / 60.);
        host.poll_at(&mut session, clock).unwrap();
        session.step(1. / 60.);
        host.publish(&session).unwrap();
    }
    let joined = start.elapsed();
    let mut slots: Vec<_> = clients.iter().filter_map(|c| c.player_slot()).collect();
    slots.sort_unstable();
    slots.dedup();
    assert_eq!(slots.len(), players, "every client has its own slot");
    let frame = host.frame(&session).unwrap();
    assert_eq!(frame.players.len(), players);
    assert_eq!(usize::from(frame.lobby.max_players), players);
    assert!(frame.valid());
    // Host cost only: clients run here too but are not timed.
    let mut times = Vec::new();
    for _ in 0..120 {
        for client in &mut clients {
            client.update();
        }
        let tick = Instant::now();
        clock += Duration::from_secs_f64(1. / 60.);
        host.poll_at(&mut session, clock).unwrap();
        session.step(1. / 60.);
        host.publish(&session).unwrap();
        times.push(tick.elapsed().as_secs_f64() * 1000.);
    }
    times.sort_by(f64::total_cmp);
    let mut received = 0;
    for client in &mut clients {
        let frames = client.take_game_frames();
        if frames.last().is_some_and(|f| f.players.len() == players) {
            received += 1;
        }
    }
    println!(
        "players={players} joined_in={:.1}s frame_bytes={} host_tick p50={:.2}ms p95={:.2}ms max={:.2}ms clients_with_full_roster={received}",
        joined.as_secs_f64(),
        payload_size(&frame),
        times[60],
        times[114],
        times[119],
    );
}

/// A revive or a join rebuilds an avatar during a tick: it must not re-read the model.
#[test]
fn rebuilding_an_avatar_shares_the_parsed_model() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let template = magicka_runtime::play_content::load_character(&root, "Wizard").unwrap();
    let first = magicka_runtime::play_content::load_skeleton(&root, &template)
        .unwrap()
        .unwrap();
    let second = magicka_runtime::play_content::load_skeleton(&root, &template)
        .unwrap()
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &second));
}
