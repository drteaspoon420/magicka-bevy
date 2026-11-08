//! Explicit seat release over real UDP, independent of lease expiry.
use magicka_network::{
    control::{Controls, InputStream},
    lobby::{LobbySettings, Role},
    transport::*,
};
use std::time::{Duration, Instant};

fn pump(
    host: &mut LoopbackHost,
    clients: &mut [&mut LoopbackClient],
    until: impl Fn(&mut LoopbackHost, &[&mut LoopbackClient]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        for client in clients.iter_mut() {
            client.update();
        }
        host.update();
        if until(host, clients) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("departure/admission did not finish before the lease deadline");
}

fn host(spectators: bool) -> LoopbackHost {
    LoopbackHost::bind_lobby(
        "127.0.0.1:0".parse().unwrap(),
        true,
        LobbySettings {
            max_players: 1,
            spectators,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn leave_releases_full_lobby_without_spectators_and_old_socket_cannot_reclaim_it() {
    let mut host = host(false);
    let mut old = LoopbackClient::connect(host.address()).unwrap();
    pump(&mut host, &mut [&mut old], |_, c| {
        c[0].player_slot() == Some(0)
    });
    let mut inputs = InputStream::default();
    let input = inputs.next(
        1,
        1,
        Controls {
            pad_movement: Some([1., 0.]),
            ..Default::default()
        },
    );
    old.send_input(input.clone()).unwrap();
    let start = Instant::now();
    old.leave().unwrap();
    old.leave().unwrap(); // idempotent even while the acknowledgement is pending
    assert_eq!(old.admission(), Admission11::Leaving);
    assert!(old.player_slot().is_none());
    assert!(old.send_input(input).is_err());
    pump(&mut host, &mut [&mut old], |h, c| {
        h.player_slots().is_empty() && c[0].leave_finished()
    });
    assert!(start.elapsed() < PLAYER_LEASE);
    assert_eq!(old.admission(), Admission11::Left);
    assert_eq!(host.take_departed(), vec![0]);
    assert!(
        host.take_inputs().is_empty(),
        "departed input must be purged"
    );
    let mut replacement = LoopbackClient::connect(host.address()).unwrap();
    pump(&mut host, &mut [&mut old, &mut replacement], |_, c| {
        c[1].player_slot() == Some(0)
    });
    old.leave().unwrap();
    for _ in 0..40 {
        old.update();
        replacement.update();
        host.update();
    }
    assert_eq!(host.player_slots(), vec![0]);
    assert_eq!(host.seats().len(), 1);
    assert!(host.take_departed().is_empty());
}

#[test]
fn leave_promotes_a_queued_player_without_waiting_for_expiry() {
    let mut host = host(true);
    let mut first = LoopbackClient::connect(host.address()).unwrap();
    pump(&mut host, &mut [&mut first], |_, c| {
        c[0].player_slot().is_some()
    });
    let mut queued = LoopbackClient::connect(host.address()).unwrap();
    pump(&mut host, &mut [&mut first, &mut queued], |_, c| {
        c[1].welcome().is_some_and(|w| w.queued)
    });
    first.leave().unwrap();
    pump(&mut host, &mut [&mut first, &mut queued], |_, c| {
        c[0].leave_finished() && c[1].player_slot() == Some(0)
    });
    assert_eq!(host.take_departed(), vec![0]);
    assert_eq!(host.seats().len(), 1);
    assert_eq!(host.seats()[0].role, Role::Player);
}

#[test]
fn cancellation_before_hello_is_acknowledged_without_seating_or_retrying_hello() {
    let mut host = host(false);
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    client.leave().unwrap();
    pump(&mut host, &mut [&mut client], |_, c| c[0].leave_finished());
    assert_eq!(client.admission(), Admission11::Left);
    assert!(host.seats().is_empty());
    assert!(host.take_departed().is_empty());
    assert!(!client.traffic().sent.contains_key("Hello"));
    assert!(!client.traffic().sent.contains_key("Heartbeat"));
}

#[test]
fn reliable_departure_survives_latency_jitter_and_packet_loss() {
    let mut host = host(false);
    let conditions = NetConditions {
        latency: Duration::from_millis(15),
        jitter: Duration::from_millis(5),
        loss: 0.25,
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
    pump(&mut host, &mut [&mut client], |_, c| {
        c[0].player_slot().is_some()
    });
    client.leave().unwrap();
    pump(&mut host, &mut [&mut client], |h, c| {
        h.player_slots().is_empty() && c[0].admission() == Admission11::Left
    });
    assert_eq!(host.take_departed(), vec![0]);
}
