//! Runtime cleanup and saved-profile recovery on explicit connection departure.
use magicka_network::{session::*, transport::*};
use magicka_runtime::{network_session::HostedSession, play_session::PlaySession};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn setup(dedicated: bool) -> (PlaySession, HostedSession) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let level = root
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap();
    let mut session = PlaySession::load(
        &root.join("../../../../target/Magicka/Content"),
        level.to_str().unwrap(),
        "Wizard",
    )
    .unwrap();
    session.disable_scripts();
    session.step(1. / 60.);
    let host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        level.to_str().unwrap(),
        dedicated,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    (session, host)
}

#[test]
fn explicit_departure_parks_equipment_and_rejoin_restores_it_before_lease_expiry() {
    for dedicated in [false, true] {
        let (mut session, mut host) = setup(dedicated);
        let id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let request = JoinRequest {
            profile: Some(ProfileSubmit {
                profile: PlayerProfile {
                    name: "Returning player".into(),
                    ..Default::default()
                },
                profile_id: Some(id.into()),
            }),
            ..Default::default()
        };
        let mut first = LoopbackClient::join(host.transport.address(), request.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while first.player_slot().is_none() && Instant::now() < deadline {
            first.update();
            host.poll(&mut session).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
        let slot = first.player_slot().unwrap();
        host.poll(&mut session).unwrap();
        let before = host.frame(&session).unwrap();
        let before = before.players.iter().find(|p| p.slot == slot).unwrap();
        session
            .set_player_loadout(
                slot,
                "Wizard",
                "staff_heirloom_grandfather_clock",
                &before.items[0],
            )
            .unwrap();
        session.unlocked_magicks[slot] = 1 << 7;
        let old_body = session.world.players[slot].as_ref().unwrap().body;
        let start = Instant::now();
        first.leave().unwrap();
        while first.admission() != Admission11::Left && start.elapsed() < Duration::from_secs(3) {
            first.update();
            host.poll(&mut session).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(first.admission(), Admission11::Left);
        assert!(start.elapsed() < PLAYER_LEASE);
        assert!(!host
            .frame(&session)
            .unwrap()
            .players
            .iter()
            .any(|p| p.slot == slot));
        assert!(!session.world.physics.bodies[old_body].active);
        assert!(session.parked_players.iter().any(|p| p["id"] == id));
        let mut replacement = LoopbackClient::join(host.transport.address(), request).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while replacement.player_slot().is_none() && Instant::now() < deadline {
            first.update();
            replacement.update();
            host.poll(&mut session).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(replacement.player_slot(), Some(slot));
        host.poll(&mut session).unwrap();
        assert_eq!(session.player_ids[slot].as_deref(), Some(id));
        assert_eq!(session.unlocked_magicks[slot], 1 << 7);
        let frame = host.frame(&session).unwrap();
        assert!(frame
            .players
            .iter()
            .find(|p| p.slot == slot)
            .unwrap()
            .items
            .iter()
            .any(|i| i == "staff_heirloom_grandfather_clock"));
        assert!(!session.parked_players.iter().any(|p| p["id"] == id));
    }
}

#[test]
fn cancelling_transport_admission_before_runtime_spawn_does_not_remove_an_uncreated_avatar() {
    let (mut session, mut host) = setup(false);
    let mut client = LoopbackClient::connect(host.transport.address()).unwrap();
    let start = Instant::now();
    while client.player_slot().is_none() && start.elapsed() < Duration::from_secs(3) {
        client.update();
        host.transport.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(client.player_slot(), Some(1));
    assert!(session.world.players.get(1).is_none_or(Option::is_none));
    client.leave().unwrap();
    // Deliver the leave entirely to the transport before its first runtime poll.
    while client.admission() != Admission11::Left && start.elapsed() < Duration::from_secs(3) {
        client.update();
        host.transport.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(client.admission(), Admission11::Left);
    host.poll(&mut session).unwrap();
    assert_eq!(host.frame(&session).unwrap().players.len(), 1);
}
