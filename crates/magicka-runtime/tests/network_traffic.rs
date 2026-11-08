//! Plan milestone B0: measured server-to-client traffic for reproducible QA.
//! `NETWORK_TRAFFIC_OUT=<dir>` writes the JSON/CSV exports.
use magicka_network::{
    session::{PlayerProfile, ProfileSubmit},
    telemetry::TrafficStats,
    transport::{JoinRequest, LoopbackClient},
};
use magicka_runtime::{network_session::HostedSession, play_session::PlaySession};
use std::{path::PathBuf, time::Duration};

#[derive(Clone)]
pub struct Measurement {
    pub host: TrafficStats,
    pub clients: Vec<TrafficStats>,
    pub ticks: u64,
}

/// Run a dedicated host with `clients` named joiners for `ticks` 60 Hz steps.
/// `act` drives the world between steps (idle when it does nothing).
pub fn measure(
    level: &str,
    clients: usize,
    ticks: u64,
    deltas: bool,
    mut act: impl FnMut(u64, &mut PlaySession),
) -> Measurement {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut session = PlaySession::load(
        &root.join("../../../../target/Magicka/Content"),
        level,
        "Wizard",
    )
    .unwrap();
    session.disable_scripts();
    session.step(1. / 60.);
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        level,
        true,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    host.transport.set_deltas(deltas);
    let mut joined: Vec<_> = (0..clients)
        .map(|i| {
            LoopbackClient::join(
                host.transport.address(),
                JoinRequest {
                    name: format!("Meter {i}"),
                    profile: Some(ProfileSubmit {
                        profile: PlayerProfile {
                            name: format!("Meter {i}"),
                            ..Default::default()
                        },
                        profile_id: None,
                    }),
                    ..Default::default()
                },
            )
            .unwrap()
        })
        .collect();
    // Admission and the first frames are not part of the steady state.
    for _ in 0..240 {
        for c in &mut joined {
            c.update();
            c.take_game_frames();
        }
        host.poll(&mut session).unwrap();
        session.step(1. / 60.);
        host.publish(&session).unwrap();
        if joined.iter().all(|c| c.player_slot().is_some()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(joined.iter().all(|c| c.player_slot().is_some()));
    host.transport.traffic_mut().reset();
    for c in &mut joined {
        c.traffic_mut().reset();
    }
    for tick in 0..ticks {
        act(tick, &mut session);
        for c in &mut joined {
            c.update();
            c.take_game_frames();
        }
        host.poll(&mut session).unwrap();
        session.step(1. / 60.);
        host.publish(&session).unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    // Drain what is in flight.
    for _ in 0..20 {
        for c in &mut joined {
            c.update();
            c.take_game_frames();
        }
        host.poll(&mut session).unwrap();
    }
    let out = Measurement {
        host: host.transport.traffic().clone(),
        clients: joined.iter().map(|c| c.traffic().clone()).collect(),
        ticks,
    };
    if let Ok(dir) = std::env::var("NETWORK_TRAFFIC_OUT") {
        let mut host = out.host.clone();
        std::fs::create_dir_all(&dir).unwrap();
        let name = format!(
            "{}-{}c-{}t-{}",
            level.replace(['/', '.'], "_"),
            clients,
            ticks,
            if deltas { "delta" } else { "full" }
        );
        std::fs::write(format!("{dir}/{name}.json"), host.to_json()).unwrap();
        std::fs::write(format!("{dir}/{name}.csv"), host.to_csv()).unwrap();
    }
    out
}

/// Server-to-client game-state bytes per client per simulated second.
pub fn state_rate(m: &Measurement) -> f64 {
    let bytes: u64 = m
        .host
        .sent
        .iter()
        .filter(|(kind, _)| kind.starts_with("Game") || kind.starts_with("World"))
        .map(|(_, k)| k.bytes)
        .sum();
    bytes as f64 / m.clients.len() as f64 / (m.ticks as f64 / 60.0)
}

fn moving(tick: u64, session: &mut PlaySession) {
    if let Some(p) = session.world.players[0].as_ref() {
        let body = p.body;
        session.world.physics.bodies[body].position[0] += 0.02 * (tick as f32).sin();
    }
}

#[test]
fn idle_and_moving_traffic_is_measured_and_deltas_halve_idle_state() {
    let full = measure("Havindr.lvl", 2, 180, false, |_, _| {});
    let idle = measure("Havindr.lvl", 2, 180, true, |_, _| {});
    let moving_full = measure("Havindr.lvl", 2, 180, false, moving);
    let moving = measure("Havindr.lvl", 2, 180, true, moving);
    let rates = [full.clone(), idle.clone(), moving_full, moving.clone()].map(|m| state_rate(&m));
    eprintln!(
        "state bytes/s/client: idle full {:.0}, idle delta {:.0}, moving full {:.0}, moving delta {:.0}",
        rates[0], rates[1], rates[2], rates[3]
    );
    eprintln!("{}", idle.host.to_csv());
    // Plan B0 provisional goal: at least 50% lower idle server-to-client
    // payload with no loss of required state (deltas are proven exact).
    assert!(rates[1] <= rates[0] * 0.5, "{rates:?}");
    assert!(rates[3] < rates[2], "{rates:?}");
    // Liveness is a few bytes a few times per second, not a full hello.
    for c in &idle.clients {
        assert!(!c.sent.contains_key("Hello"));
        let beats = c.sent["Heartbeat"];
        assert!(beats.bytes <= beats.messages * 8);
        assert!(c.received.get("GameUpdate.full").map_or(0, |k| k.rejected) == 0);
        assert!(c.received.get("GameUpdate.delta").map_or(0, |k| k.rejected) == 0);
    }
}
