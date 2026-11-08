//! A client decides what reaches the host and when, so nothing it does may stop
//! the host for everyone (`research/multiplayer-security.md`).
use magicka_network::{
    control::{InputStream, PickupRequest, PickupTarget},
    security::{HostSecurity, NetworkKey},
    session::{PlayerProfile, ProfileRequest},
    transport::{JoinRequest, LoopbackClient},
};
use magicka_runtime::{network_session::HostedSession, play_session::PlaySession};
use std::path::PathBuf;

fn session() -> PlaySession {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut session = PlaySession::load(
        &root.join("../../../../target/Magicka/Content"),
        "Havindr.lvl",
        "Wizard",
    )
    .unwrap();
    session.disable_scripts();
    session.step(1. / 60.);
    session
}
fn host(session: &PlaySession, security: HostSecurity) -> HostedSession {
    HostedSession::bind_secured(
        "127.0.0.1:0".parse().unwrap(),
        "Havindr.lvl",
        false,
        PlayerProfile::default(),
        session,
        Default::default(),
        security,
    )
    .unwrap()
}
fn pump(host: &mut HostedSession, session: &mut PlaySession, client: &mut LoopbackClient) {
    host.poll(session).unwrap();
    host.publish(session).unwrap();
    session.step(1. / 60.);
    client.update();
    std::thread::sleep(std::time::Duration::from_millis(1));
}
fn seated(
    host: &mut HostedSession,
    session: &mut PlaySession,
    client: &mut LoopbackClient,
) -> usize {
    for _ in 0..600 {
        pump(host, session, client);
        if let Some(slot) = client.player_slot() {
            if session.world.players[slot].is_some() {
                return slot;
            }
        }
    }
    panic!("not seated: {:?}", client.admission());
}

#[test]
fn requests_queued_for_a_refused_seat_do_not_stop_the_host() {
    let mut session = session();
    let mut host = host(&session, HostSecurity::default());
    let mut client = LoopbackClient::connect(host.transport.address()).unwrap();
    let slot = seated(&mut host, &mut session, &mut client);
    client
        .send_profile(ProfileRequest {
            sequence: 1,
            profile: PlayerProfile::default(),
            profile_id: None,
        })
        .unwrap();
    client
        .send_pickup(PickupRequest {
            session: 1,
            epoch: 1,
            sequence: 1,
            target: PickupTarget {
                handle: 1,
                generation: 1,
            },
        })
        .unwrap();
    // The requests reach the transport's queues, then the seat goes away
    // before the session reads them, as when its robe fails to load.
    for _ in 0..40 {
        client.update();
        host.transport.update();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(host.transport.refuse_seat(slot, "testing"));
    host.poll(&mut session)
        .expect("the host keeps running after refusing a seat with queued requests");
}

#[test]
fn input_for_a_wizard_that_is_gone_does_not_stop_the_host() {
    let mut session = session();
    let mut host = host(&session, HostSecurity::default());
    let mut client = LoopbackClient::connect(host.transport.address()).unwrap();
    let slot = seated(&mut host, &mut session, &mut client);
    let frame = host.frame(&session).unwrap();
    let mut stream = InputStream::default();
    session.depart_network_player(slot).unwrap();
    for _ in 0..30 {
        client
            .send_input(stream.next(
                frame.physics.session,
                frame.physics.epoch,
                Default::default(),
            ))
            .unwrap();
        pump(&mut host, &mut session, &mut client);
    }
}

#[test]
fn a_private_game_plays_end_to_end() {
    let mut session = session();
    let security = HostSecurity::private("correct horse battery").unwrap();
    let key = security.key.clone();
    let mut host = host(&session, security);
    let mut client = LoopbackClient::join(
        host.transport.address(),
        JoinRequest {
            key,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(NetworkKey::default() != NetworkKey::from_passphrase("correct horse battery").unwrap());
    let slot = seated(&mut host, &mut session, &mut client);
    let mut frames = 0;
    for _ in 0..120 {
        pump(&mut host, &mut session, &mut client);
        frames += client.take_game_frames().len();
    }
    assert!(frames > 0, "game state arrives over the encrypted session");
    assert!(session.world.players[slot].is_some());
}

#[test]
fn the_debug_respawn_is_refused_unless_the_host_opted_in() {
    use magicka_network::session::RespawnRequest;
    let mut session = session();
    let mut host = host(&session, HostSecurity::default());
    let mut client = LoopbackClient::connect(host.transport.address()).unwrap();
    let slot = seated(&mut host, &mut session, &mut client);
    let frame = host.frame(&session).unwrap();
    let kill = |session: &mut PlaySession| {
        let player = session.world.players[slot].as_mut().unwrap();
        player.character.combat.dead = true;
        player.character.combat.health.hit_points = 0.0;
    };
    let dead = |session: &PlaySession| session.world.players[slot].as_ref().unwrap().dead();
    let ask = |client: &mut LoopbackClient, sequence| {
        client
            .send_respawn(RespawnRequest {
                session: frame.physics.session,
                epoch: frame.physics.epoch,
                sequence,
            })
            .unwrap();
    };
    kill(&mut session);
    ask(&mut client, 1);
    for _ in 0..40 {
        host.poll(&mut session).unwrap();
        client.update();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(dead(&session), "off by default");
    host.set_lobby(magicka_network::lobby::LobbySettings {
        debug_respawn: true,
        ..host.lobby()
    })
    .unwrap();
    ask(&mut client, 2);
    for _ in 0..40 {
        host.poll(&mut session).unwrap();
        client.update();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!dead(&session), "works once the host allows it");
}

#[test]
fn a_kicked_player_leaves_the_running_game_and_a_dungeon_master_can_do_it() {
    use magicka_network::{
        lobby::{DmCommand, DmRequest, LobbySettings, Role},
        transport::RefusalCode,
    };
    let mut session = session();
    let mut host = HostedSession::bind_lobby(
        "127.0.0.1:0".parse().unwrap(),
        "Havindr.lvl",
        false,
        PlayerProfile::default(),
        &session,
        LobbySettings {
            dungeon_masters: true,
            ..Default::default()
        },
    )
    .unwrap();
    let address = host.transport.address();
    let mut player = LoopbackClient::connect(address).unwrap();
    let slot = seated(&mut host, &mut session, &mut player);
    let mut gm = LoopbackClient::connect_as(address, Role::DungeonMaster, "GM").unwrap();
    for _ in 0..300 {
        pump(&mut host, &mut session, &mut player);
        gm.update();
        if gm.welcome().is_some() {
            break;
        }
    }
    assert!(gm.welcome().is_some());
    let own = gm.welcome().unwrap().seat;
    // Listed for the host; a dungeon master cannot remove itself.
    assert!(host.kickable().iter().any(|(seat, _)| *seat == slot));
    let send = |gm: &mut LoopbackClient, sequence, seat| {
        gm.send_dm(DmRequest {
            sequence,
            command: DmCommand::Kick { seat },
        })
        .unwrap();
    };
    send(&mut gm, 1, own);
    for _ in 0..30 {
        pump(&mut host, &mut session, &mut player);
        gm.update();
    }
    assert!(gm.refusal().is_none(), "still here");
    send(&mut gm, 2, slot);
    for _ in 0..200 {
        pump(&mut host, &mut session, &mut player);
        gm.update();
        if player.refusal().is_some() && session.world.players[slot].is_none() {
            break;
        }
    }
    assert_eq!(player.refusal_code(), Some(RefusalCode::Kicked));
    assert!(session.world.players[slot].is_none(), "the wizard is gone");
    // The host itself cannot be listed: it holds no connection.
    assert!(host.kickable().iter().all(|(seat, _)| *seat != 0));
}
