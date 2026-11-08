//! Protocol 11 admission: profiles are validated in the hello, before any
//! avatar exists; hellos stop after admission; heartbeats time the link.
use magicka_network::{
    control::{Controls, InputAuthority, InputStream},
    lobby::{LobbySettings, Role},
    session::{PlayerProfile, ProfileSubmit},
    transport::*,
};
use std::time::Duration;

fn submit(name: &str, robe: &str) -> ProfileSubmit {
    ProfileSubmit {
        profile: PlayerProfile {
            name: name.into(),
            robe: robe.into(),
            colour: [0.5, 1.0, 1.4],
        },
        profile_id: Some("00000000000000000000000000000042".into()),
    }
}
fn join(host: &LoopbackHost, profile: Option<ProfileSubmit>) -> LoopbackClient {
    LoopbackClient::join(
        host.address(),
        JoinRequest {
            role: Role::Player,
            name: profile
                .as_ref()
                .map_or("x".into(), |p| p.profile.name.clone()),
            profile,
            ..Default::default()
        },
    )
    .unwrap()
}
fn pump(
    host: &mut LoopbackHost,
    clients: &mut [&mut LoopbackClient],
    until: impl Fn(&[&mut LoopbackClient]) -> bool,
) {
    for _ in 0..600 {
        for c in clients.iter_mut() {
            c.update();
        }
        host.update();
        if until(clients) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn invalid_profiles_and_foreign_content_are_refused_with_reasons() {
    let mut host = LoopbackHost::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let mut blank = join(&host, Some(submit("   ", "Wizard")));
    let mut robe = join(&host, Some(submit("Ok", "Goblin")));
    let mut modded = LoopbackClient::join(
        host.address(),
        JoinRequest {
            profile: Some(submit("Modder", "Wizard")),
            rules_hash: Some("abc".into()),
            ..Default::default()
        },
    )
    .unwrap();
    pump(&mut host, &mut [&mut blank, &mut robe, &mut modded], |c| {
        c.iter().all(|c| c.refusal().is_some())
    });
    assert_eq!(blank.refusal_code(), Some(RefusalCode::InvalidProfile));
    assert!(blank.refusal().unwrap().contains("name"));
    assert_eq!(robe.refusal_code(), Some(RefusalCode::InvalidProfile));
    assert!(robe.refusal().unwrap().contains("robe"));
    assert_eq!(modded.refusal_code(), Some(RefusalCode::ContentMismatch));
    assert!(host.player_slots().is_empty(), "refused joins took no slot");
}

#[test]
fn admitted_profile_is_known_before_any_avatar_and_hellos_stop() {
    let mut host = LoopbackHost::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    host.set_server_info(ServerInfo {
        name: "Test host".into(),
        level: "feature-lab.lvl".into(),
        ..Default::default()
    });
    let mut client = join(&host, Some(submit("Named", "Wizard_Samurai")));
    pump(&mut host, &mut [&mut client], |c| {
        c[0].welcome().is_some() && c[0].server_info().is_some()
    });
    let slot = client.player_slot().expect("seated");
    assert!(matches!(client.admission(), Admission11::Seated(_)));
    assert_eq!(
        host.admitted_profile(slot),
        Some(submit("Named", "Wizard_Samurai"))
    );
    let info = client.server_info().unwrap();
    assert_eq!(info.name, "Test host");
    assert_eq!(info.level, "feature-lab.lvl");
    let hellos = client.traffic().sent["Hello"].messages;
    assert!(hellos <= u64::from(HELLO_BURST));
    // Keep the connection alive long enough for several heartbeats.
    let start = std::time::Instant::now();
    let before = client
        .traffic()
        .sent
        .get("Heartbeat")
        .map_or(0, |k| k.messages);
    let mut updates = 0;
    while start.elapsed() < Duration::from_millis(1200) {
        client.update();
        host.update();
        updates += 1;
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        client.traffic().sent["Hello"].messages,
        hellos,
        "no hello after admission"
    );
    // Every 250 ms or every HEARTBEAT_UPDATES updates, whichever is first.
    let beats = client.traffic().sent["Heartbeat"].messages - before;
    let bound = (updates / u64::from(HEARTBEAT_UPDATES)).max(5) + 1;
    assert!(
        (3..=bound).contains(&beats),
        "{beats} heartbeats in {updates} updates"
    );
    assert!(client.traffic().timing.rtt_ms.is_some_and(|r| r < 500.0));
    assert!(host.traffic().received["Heartbeat"].messages >= 2);
    assert!(client.traffic().to_csv().contains("sent,Heartbeat,"));
    assert!(client.traffic_mut().to_json().contains("\"rtt_ms\""));
}

#[test]
fn queued_players_keep_their_profile_through_promotion() {
    let lobby = LobbySettings {
        max_players: 1,
        ..Default::default()
    };
    let mut host = LoopbackHost::bind_lobby("127.0.0.1:0".parse().unwrap(), true, lobby).unwrap();
    let mut first = join(&host, Some(submit("First", "Wizard")));
    pump(&mut host, &mut [&mut first], |c| {
        c[0].player_slot().is_some()
    });
    let mut second = join(&host, Some(submit("Second", "Wizard")));
    pump(&mut host, &mut [&mut first, &mut second], |c| {
        c[1].welcome().is_some()
    });
    assert!(second.welcome().unwrap().queued);
    assert_eq!(second.welcome().unwrap().role, Role::Spectator);
    // The seated player leaves; its lease expires and the queue promotes.
    drop(first);
    let later = std::time::Instant::now() + PLAYER_LEASE + Duration::from_millis(10);
    for _ in 0..300 {
        second.update();
        host.update_at(later);
        if second.player_slot().is_some() {
            break;
        }
    }
    let slot = second.player_slot().expect("promoted");
    assert_eq!(host.admitted_profile(slot).unwrap().profile.name, "Second");
}

#[test]
fn redundant_input_history_replays_samples_lost_in_transit() {
    let mut authority = InputAuthority::dedicated(1, 1).unwrap();
    let mut stream = InputStream::default();
    let press = |cast| Controls {
        cast_down: cast,
        ..Default::default()
    };
    let first = stream.next(1, 1, press(false));
    let tap = stream.next(1, 1, press(true)); // lost
    let release = stream.next(1, 1, press(false));
    assert_eq!(tap.history.len(), 1);
    assert_eq!(
        authority.accept_with_history(0, &first).unwrap(),
        vec![press(false)]
    );
    // The press edge survives the lost packet, in order.
    assert_eq!(
        authority.accept_with_history(0, &release).unwrap(),
        vec![press(true), press(false)]
    );
    // A late duplicate is stale and replays nothing.
    assert!(authority.accept_with_history(0, &tap).is_err());
    // Beyond the redundancy window only the carried samples return.
    for _ in 0..6 {
        stream.next(1, 1, press(true));
    }
    let last = stream.next(1, 1, press(false));
    assert_eq!(authority.accept_with_history(0, &last).unwrap().len(), 4);
}

#[test]
fn conditioned_links_still_admit_and_deliver_under_latency_and_loss() {
    let mut host = LoopbackHost::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let conditions = NetConditions {
        latency: Duration::from_millis(40),
        jitter: Duration::from_millis(10),
        loss: 0.1,
    };
    host.set_conditions(Some(conditions));
    let mut client = LoopbackClient::join(
        host.address(),
        JoinRequest {
            profile: Some(submit("Laggy", "Wizard")),
            name: "Laggy".into(),
            conditions: Some(conditions),
            ..Default::default()
        },
    )
    .unwrap();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(3) && client.traffic().timing.rtt_ms.is_none() {
        client.update();
        host.update();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(client.player_slot().is_some(), "admitted through loss");
    let rtt = client.traffic().timing.rtt_ms.expect("heartbeat echoed");
    assert!(rtt >= 60.0, "conditioned rtt {rtt}");
}
