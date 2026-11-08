//! Multiplayer security: the secure session, hostile peers and what a client
//! accepts from a host (`research/multiplayer-security.md`).
use magicka_network::{
    discovery::{Advert, LanBrowser},
    lobby::{LobbySettings, Role, RoleRequest},
    security::*,
    session::{ModRequest, PlayerProfile, ProfileRequest, ProfileSubmit},
    transport::*,
    world::{PickupKind, PickupState, WorldPresentation},
};
use std::{
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

fn any_port() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}
fn host(security: HostSecurity) -> LoopbackHost {
    LoopbackHost::bind_secured(any_port(), true, LobbySettings::default(), security).unwrap()
}
fn strict(mut security: HostSecurity) -> LoopbackHost {
    security.enforce_on_loopback = true;
    host(security)
}
fn pump(
    host: &mut LoopbackHost,
    clients: &mut [&mut LoopbackClient],
    mut until: impl FnMut(&mut LoopbackHost, &mut [&mut LoopbackClient]) -> bool,
) -> bool {
    for _ in 0..900 {
        host.update();
        for client in clients.iter_mut() {
            client.update();
        }
        if until(host, clients) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}
fn seat(host: &mut LoopbackHost, client: &mut LoopbackClient) -> bool {
    pump(host, &mut [client], |_, c| {
        matches!(
            c[0].admission(),
            Admission11::Seated(_) | Admission11::Refused(..)
        )
    }) && matches!(client.admission(), Admission11::Seated(_))
}

fn rejected(host: &LoopbackHost, kind: &str) -> u64 {
    host.traffic().received.get(kind).map_or(0, |k| k.rejected)
}

// ---- secure session ----

#[test]
fn an_open_game_joins_without_a_key() {
    let mut host = host(HostSecurity::default());
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut client));
}

#[test]
fn a_private_game_needs_the_passphrase() {
    let security = HostSecurity::private("correct horse battery").unwrap();
    assert!(security.key.is_private());
    let mut host = host(security);
    let address = host.address();
    let join = |key| {
        LoopbackClient::join(
            address,
            JoinRequest {
                key,
                ..Default::default()
            },
        )
        .unwrap()
    };
    let mut right = join(NetworkKey::from_passphrase("correct horse battery").unwrap());
    assert!(seat(&mut host, &mut right), "the right passphrase joins");

    for key in [
        NetworkKey::default(),
        NetworkKey::from_passphrase("wrong horse battery").unwrap(),
    ] {
        let mut wrong = join(key);
        for _ in 0..240 {
            host.update();
            wrong.update();
            right.update();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(wrong.welcome().is_none(), "no seat without the passphrase");
        assert_eq!(
            host.seats().len(),
            1,
            "nothing but the real player is seated"
        );
    }
}

#[test]
fn passphrases_are_derived_deterministically_and_never_printed() {
    let a = NetworkKey::from_passphrase("  correct horse battery ").unwrap();
    let b = NetworkKey::from_passphrase("correct horse battery").unwrap();
    assert_eq!(a, b, "surrounding whitespace is not part of it");
    assert_ne!(
        a,
        NetworkKey::from_passphrase("correct horse batterz").unwrap()
    );
    assert_ne!(a, NetworkKey::default());
    assert_eq!(format!("{a:?}"), "NetworkKey(private)");
    assert!(NetworkKey::from_passphrase("short").is_err());
}

#[test]
fn a_silent_or_forged_peer_gets_no_seat_and_no_game_state() {
    let mut host = host(HostSecurity::default());
    let attacker = UdpSocket::bind("127.0.0.1:0").unwrap();
    attacker.set_nonblocking(true).unwrap();
    // Without a connect token no datagram reaches the game: not random bytes,
    // and not what a raw hello would have looked like.
    for i in 0..2000u32 {
        let junk: Vec<u8> = (0..(i % 1400)).map(|b| (b * 31 + i) as u8).collect();
        attacker.send_to(&junk, host.address()).unwrap();
    }
    let mut buffer = [0; 2048];
    for _ in 0..120 {
        host.update();
        std::thread::sleep(Duration::from_millis(1));
        assert!(
            attacker.recv_from(&mut buffer).is_err(),
            "nothing is sent to an address that never finished the handshake"
        );
    }
    assert!(host.seats().is_empty());
}

#[test]
fn links_that_never_finish_the_handshake_are_reclaimed() {
    let mut host = host(HostSecurity::default());
    let attacker = UdpSocket::bind("127.0.0.1:0").unwrap();
    attacker.send_to(&[0xff; 64], host.address()).unwrap();
    let start = Instant::now();
    for _ in 0..30 {
        host.update_at(start);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(host.link_count(), 1, "the datagram made a link");
    host.update_at(start + HANDSHAKE_DEADLINE + Duration::from_secs(1));
    host.update_at(start + HANDSHAKE_DEADLINE + Duration::from_secs(1));
    assert_eq!(host.link_count(), 0, "and it did not stay");
}

#[test]
fn a_session_that_never_says_hello_is_reclaimed() {
    // Finished handshake, no hello: `leave` stops this client's hellos.
    let mut host = host(HostSecurity::default());
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    client.leave().unwrap();
    let start = Instant::now();
    for _ in 0..120 {
        host.update_at(start);
        client.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(host.link_count() >= 1);
    let later = start + HELLO_DEADLINE + Duration::from_secs(1);
    host.update_at(later);
    let gone = later + RETIRE_GRACE + Duration::from_secs(1);
    host.update_at(gone);
    host.update_at(gone);
    assert_eq!(host.link_count(), 0);
}

#[test]
fn a_removed_connection_frees_its_link_after_the_refusal_went_out() {
    let mut host = host(HostSecurity::default());
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut client));
    let seat_number = host.player_slots()[0];
    assert!(host.refuse_seat(seat_number, "testing"));
    assert!(pump(&mut host, &mut [&mut client], |_, c| c[0]
        .refusal()
        .is_some()));
    assert_eq!(client.refusal(), Some("testing"));
    let later = Instant::now() + RETIRE_GRACE + Duration::from_secs(1);
    host.update_at(later);
    host.update_at(later);
    assert_eq!(host.link_count(), 0);
}

#[test]
fn a_client_that_disconnects_mid_game_frees_its_seat() {
    let mut host = host(HostSecurity::default());
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut client));
    assert_eq!(host.player_slots().len(), 1);
    host.despawn_link_for_test(0);
    host.update();
    host.update();
    assert!(host.player_slots().is_empty());
    assert_eq!(host.take_departed(), vec![0], "the session hears of it");
    let mut next = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut next), "and the slot is free again");
}

// ---- the single-packet kill (MS-1) ----

fn varint(value: u64) -> Vec<u8> {
    match value {
        0..=63 => vec![value as u8],
        64..=16383 => ((value as u16) | 0x4000).to_be_bytes().to_vec(),
        16384..=1_073_741_823 => ((value as u32) | 0x8000_0000).to_be_bytes().to_vec(),
        _ => (value | 0xC000_0000_0000_0000).to_be_bytes().to_vec(),
    }
}
fn fragment_packet(index: u64, count: u64) -> Vec<u8> {
    let mut packet = vec![1u8]; // PacketType::DataFragment
    packet.extend([0; 12]);
    packet.extend(varint(0)); // channel
    packet.extend(7u16.to_be_bytes()); // message id
    packet.extend(varint(index));
    packet.extend(varint(count));
    packet.push(0); // compression, first fragment only
    packet.extend(varint(4)); // fragment bytes
    packet.extend([1, 2, 3, 4]);
    packet
}

#[test]
fn packet_guard_rejects_what_would_exhaust_memory() {
    assert!(packet_allowed(&fragment_packet(0, 16), HOST_FRAGMENT_LIMIT));
    assert!(packet_allowed(
        &fragment_packet(31, 32),
        HOST_FRAGMENT_LIMIT
    ));
    assert!(!packet_allowed(
        &fragment_packet(0, 33),
        HOST_FRAGMENT_LIMIT
    ));
    assert!(!packet_allowed(
        &fragment_packet(0, 1 << 40),
        HOST_FRAGMENT_LIMIT
    ));
    assert!(!packet_allowed(
        &fragment_packet(0, 1 << 40),
        CLIENT_FRAGMENT_LIMIT
    ));
    assert!(!packet_allowed(&fragment_packet(0, 0), HOST_FRAGMENT_LIMIT));
    assert!(!packet_allowed(&fragment_packet(5, 5), HOST_FRAGMENT_LIMIT));
    // Not a packet at all, truncations, and types this build never sends.
    assert!(!packet_allowed(&[], HOST_FRAGMENT_LIMIT));
    assert!(!packet_allowed(&[0; 12], HOST_FRAGMENT_LIMIT));
    assert!(!packet_allowed(&[1; 13], HOST_FRAGMENT_LIMIT));
    for kind in [2u8, 3, 5, 255] {
        let mut packet = vec![kind];
        packet.extend([0; 40]);
        assert!(!packet_allowed(&packet, HOST_FRAGMENT_LIMIT));
    }
    let mut data = vec![0u8];
    data.extend([0; 40]);
    assert!(packet_allowed(&data, HOST_FRAGMENT_LIMIT));
    let full = fragment_packet(0, 16);
    for cut in 13..17 {
        assert!(!packet_allowed(&full[..cut], HOST_FRAGMENT_LIMIT));
    }
}

#[test]
fn an_authenticated_peer_cannot_kill_the_host_with_one_packet() {
    let mut host = host(HostSecurity::default());
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut client));
    // Would abort the process if it reached lightyear's fragment receiver.
    for _ in 0..8 {
        client.inject_packet_for_test(fragment_packet(0, 1 << 40));
    }
    for _ in 0..60 {
        client.update();
        host.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(host.seats().len(), 1, "the host is still running the game");
}

#[test]
fn incomplete_fragmented_messages_are_limited_per_connection() {
    let mut host = strict(HostSecurity::default());
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut client));
    // Each first fragment would allocate a whole message that, on a reliable
    // channel, is kept until it completes: never, here.
    let reliable_start = |message: u16| {
        let mut packet = fragment_packet(0, 32);
        packet[14..16].copy_from_slice(&message.to_be_bytes());
        packet
    };
    let mut message = 0;
    assert!(
        pump(&mut host, &mut [&mut client], |h, c| {
            for _ in 0..4 {
                c[0].inject_packet_for_test(reliable_start(message));
                message += 1;
            }
            h.player_slots().is_empty()
        }),
        "a connection that keeps starting messages is removed"
    );
    assert_eq!(rejected(&host, "abuse"), 1);
}

#[test]
fn a_legitimate_large_request_still_passes_the_guard() {
    let mut host = host(HostSecurity::default());
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut client));
    let request = ModRequest {
        request_id: 1,
        channel: "test.big".into(),
        json: "x".repeat(16 << 10),
    };
    client.send_mod_request(request.clone()).unwrap();
    let mut got = Vec::new();
    assert!(pump(&mut host, &mut [&mut client], |h, _| {
        got.extend(h.take_mod_requests());
        !got.is_empty()
    }));
    assert_eq!(got[0].1, request);
}

/// Mutated and random packets inside a real session must not panic the host.
#[test]
fn hostile_session_packets_do_not_panic_the_host() {
    let mut host = host(HostSecurity::default());
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut client));
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut random = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for round in 0..400 {
        for _ in 0..4 {
            let kind = [0u8, 1, 4][(random() % 3) as usize];
            let mut packet = vec![kind];
            packet.extend(random().to_le_bytes());
            packet.extend(random().to_le_bytes());
            if round % 2 == 0 {
                // A plausible fragment whose count passes the guard.
                packet.truncate(13);
                packet.extend(varint(random() % 6));
                packet.extend(((random() % 8) as u16).to_be_bytes());
                let count = 1 + random() % 64;
                packet.extend(varint(random() % count));
                packet.extend(varint(count));
                packet.push(0);
                let len = random() % 2000;
                packet.extend(varint(len));
                packet.extend((0..len.min(1100)).map(|_| random() as u8));
                packet[0] = 1;
            } else {
                packet.extend((0..random() % 1100).map(|_| random() as u8));
            }
            client.inject_packet_for_test(packet);
        }
        client.update();
        host.update();
    }
    for _ in 0..30 {
        client.update();
        host.update();
    }
    // The peer may have been ignored; the host must simply still work.
    let mut other = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut other));
}

// ---- forged datagrams before any session (MS-7) ----

const DEADLY: [&[u8]; 3] = [
    // Sequence length 15: netcode slices an 8-byte array with it.
    &[0xff; 64],
    &[0x95; 64],
    // A sequence so large the replay check's addition overflows.
    &[
        0x84, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0,
    ],
];

#[test]
fn datagram_guard_lets_real_packet_shapes_through() {
    for deadly in DEADLY {
        assert!(!datagram_allowed(deadly));
    }
    assert!(!datagram_allowed(&[]));
    assert!(!datagram_allowed(&[0x87]), "kind out of range");
    assert!(!datagram_allowed(&[0x14]), "shorter than its sequence");
    assert!(!datagram_allowed(&[0x05]), "no sequence at all");
    assert!(!datagram_allowed(&[0x10; 8]), "a request is exactly zero");
    assert!(datagram_allowed(&[0; 1078]), "connection request");
    assert!(datagram_allowed(&[0x14, 1, 0, 0]), "client keep-alive");
    let mut host_payload = vec![0x85];
    host_payload.extend((1u64 << 62).to_le_bytes());
    assert!(
        datagram_allowed(&host_payload),
        "a host's starting sequence"
    );
}

#[test]
fn a_forged_datagram_from_a_players_address_does_not_kill_the_host() {
    // A relay stands in for an attacker who forges a player's source address:
    // to the host, its socket is that player.
    let mut host = host(HostSecurity::private("correct horse battery").unwrap());
    let front = UdpSocket::bind("127.0.0.1:0").unwrap();
    let back = UdpSocket::bind("127.0.0.1:0").unwrap();
    front.set_nonblocking(true).unwrap();
    back.set_nonblocking(true).unwrap();
    let mut client = LoopbackClient::join(
        front.local_addr().unwrap(),
        JoinRequest {
            key: NetworkKey::from_passphrase("correct horse battery").unwrap(),
            ..Default::default()
        },
    )
    .unwrap();
    let mut buffer = [0; 2048];
    let mut player = None;
    let mut relay = |host: &LoopbackHost, player: &mut Option<SocketAddr>| {
        while let Ok((n, from)) = front.recv_from(&mut buffer) {
            *player = Some(from);
            back.send_to(&buffer[..n], host.address()).unwrap();
        }
        while let Ok((n, _)) = back.recv_from(&mut buffer) {
            if let Some(to) = *player {
                front.send_to(&buffer[..n], to).unwrap();
            }
        }
    };
    for _ in 0..900 {
        client.update();
        relay(&host, &mut player);
        host.update();
        relay(&host, &mut player);
        if client.welcome().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(client.welcome().is_some(), "seated through the relay");
    for deadly in DEADLY {
        back.send_to(deadly, host.address()).unwrap();
    }
    for _ in 0..60 {
        client.update();
        relay(&host, &mut player);
        host.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(host.seats().len(), 1, "the host and the player carry on");
}

#[test]
fn a_forged_datagram_does_not_kill_a_client() {
    let hostile = UdpSocket::bind("127.0.0.1:0").unwrap();
    hostile.set_nonblocking(true).unwrap();
    let mut client = LoopbackClient::connect(hostile.local_addr().unwrap()).unwrap();
    let mut buffer = [0; 2048];
    let mut answered = false;
    for _ in 0..240 {
        client.update();
        if let Ok((_, from)) = hostile.recv_from(&mut buffer) {
            for deadly in DEADLY {
                hostile.send_to(deadly, from).unwrap();
            }
            answered = true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(answered, "the client did try to connect");
    assert!(client.welcome().is_none());
}

// ---- budgets and bans ----

#[test]
fn a_flood_of_requests_is_dropped_and_then_removed() {
    let mut host = strict(HostSecurity::default());
    let mut abuser = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut abuser));
    let flood = |client: &mut LoopbackClient, from: u64| {
        for sequence in from..from + 200 {
            client
                .send_profile(ProfileRequest {
                    sequence,
                    profile: PlayerProfile::default(),
                    profile_id: None,
                })
                .ok();
        }
    };
    flood(&mut abuser, 1);
    let mut delivered = 0;
    let mut sequence = 201;
    assert!(
        pump(&mut host, &mut [&mut abuser], |h, c| {
            delivered += h.take_profiles().len();
            flood(c[0], sequence);
            sequence += 200;
            h.player_slots().is_empty()
        }),
        "the flooding connection is removed"
    );
    assert!(delivered <= 16, "only its budget got through: {delivered}");
    assert_eq!(host.take_departed(), vec![0]);
    assert_eq!(rejected(&host, "abuse"), 1);
    // Its address gets no answer for a while, which costs the host nothing.
    let mut again = LoopbackClient::connect(host.address()).unwrap();
    for _ in 0..120 {
        host.update();
        again.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(again.welcome().is_none());
    assert!(host.seats().is_empty());
    // After the ban it can come back.
    let later = Instant::now() + ABUSE_BAN + Duration::from_secs(1);
    let mut back = LoopbackClient::connect(host.address()).unwrap();
    for _ in 0..300 {
        host.update_at(later);
        back.update();
        if back.welcome().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(back.welcome().is_some());
}

#[test]
fn one_address_cannot_flood_connection_requests() {
    let mut host = strict(HostSecurity::default());
    let attacker = UdpSocket::bind("127.0.0.1:0").unwrap();
    for _ in 0..30 {
        for _ in 0..50 {
            attacker.send_to(&[0; 1078], host.address()).unwrap();
        }
        std::thread::sleep(Duration::from_millis(1));
        host.update();
    }
    assert!(rejected(&host, "ConnectionRequest") > 500);
    // Once the budget has refilled, the same address can join normally.
    let later = Instant::now() + Duration::from_secs(10);
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    for _ in 0..300 {
        host.update_at(later);
        client.update();
        if client.welcome().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(client.welcome().is_some());
}

#[test]
fn ordinary_play_stays_within_every_budget() {
    let mut host = strict(HostSecurity::default());
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut client));
    let mut stream = magicka_network::control::InputStream::default();
    for _ in 0..600 {
        client
            .send_input(stream.next(1, 1, Default::default()))
            .unwrap();
        client.update();
        host.update();
        host.take_inputs();
    }
    assert_eq!(host.player_slots().len(), 1);
    assert_eq!(rejected(&host, "abuse"), 0);
    assert_eq!(rejected(&host, "InputCommand"), 0);
}

#[test]
fn connections_per_address_are_limited() {
    let mut host = strict(HostSecurity {
        connections_per_address: 2,
        ..Default::default()
    });
    let lobby = LobbySettings {
        max_players: 8,
        ..Default::default()
    };
    host.set_lobby(lobby).unwrap();
    let mut a = LoopbackClient::connect(host.address()).unwrap();
    let mut b = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut a));
    assert!(seat(&mut host, &mut b));
    let mut c = LoopbackClient::connect(host.address()).unwrap();
    assert!(pump(&mut host, &mut [&mut c], |_, c| c[0]
        .refusal_code()
        .is_some()));
    assert_eq!(c.refusal_code(), Some(RefusalCode::TooManyConnections));
}

#[test]
fn banned_addresses_are_refused() {
    let mut host = strict(HostSecurity {
        banned: vec!["127.0.0.1".parse().unwrap()],
        ..Default::default()
    });
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    for _ in 0..120 {
        host.update();
        client.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(client.welcome().is_none());
    assert!(host.seats().is_empty());
}

// ---- removing a player ----

#[test]
fn a_kicked_player_is_told_why_and_cannot_come_back() {
    let mut host = strict(HostSecurity {
        connections_per_address: 4,
        ..Default::default()
    });
    let mut client = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut client));
    let slot = host.player_slots()[0];
    assert!(!host.kick(slot + 100, "nobody there"), "unknown seat");
    assert!(host.kick(slot, "Removed from the game by the host."));
    assert!(host.player_slots().is_empty(), "the seat is free at once");
    assert_eq!(host.take_departed(), vec![slot], "the session hears of it");
    assert!(pump(&mut host, &mut [&mut client], |_, c| c[0]
        .refusal()
        .is_some()));
    assert_eq!(client.refusal_code(), Some(RefusalCode::Kicked));
    // The same address does not get back in.
    let mut again = LoopbackClient::connect(host.address()).unwrap();
    for _ in 0..120 {
        host.update();
        again.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(again.welcome().is_none());
    assert!(host.seats().is_empty());
}

// ---- the dungeon master credential ----

fn dm_lobby() -> LobbySettings {
    LobbySettings {
        dungeon_masters: true,
        ..Default::default()
    }
}
fn dm_host() -> LoopbackHost {
    LoopbackHost::bind_secured(
        any_port(),
        true,
        dm_lobby(),
        HostSecurity {
            dm_credential: Some("the keys to the dungeon".into()),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn the_dungeon_master_role_needs_the_credential() {
    let mut host = dm_host();
    let join = |credential: Option<&str>, address| {
        LoopbackClient::join(
            address,
            JoinRequest {
                role: Role::DungeonMaster,
                name: "GM".into(),
                credential: credential.map(str::to_string),
                ..Default::default()
            },
        )
        .unwrap()
    };
    let address = host.address();
    for bad in [None, Some("guess")] {
        let mut wrong = join(bad, address);
        assert!(pump(&mut host, &mut [&mut wrong], |_, c| c[0]
            .refusal_code()
            .is_some()));
        assert_eq!(wrong.refusal_code(), Some(RefusalCode::Credentials));
    }
    let mut right = join(Some("the keys to the dungeon"), address);
    assert!(seat(&mut host, &mut right));
    assert_eq!(right.welcome().unwrap().role, Role::DungeonMaster);
}

#[test]
fn a_player_cannot_step_up_to_dungeon_master_without_the_credential() {
    let mut host = dm_host();
    let mut player = LoopbackClient::connect(host.address()).unwrap();
    assert!(seat(&mut host, &mut player));
    player
        .request_role(RoleRequest {
            sequence: 1,
            role: Role::DungeonMaster,
            name: "Sneaky".into(),
        })
        .unwrap();
    for _ in 0..120 {
        host.update();
        player.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(host.seats()[0].role, Role::Player);
}

#[test]
fn without_a_credential_an_enabled_role_stays_open_and_the_host_is_warned() {
    let security = HostSecurity::default();
    assert_eq!(security.warnings(true, true).len(), 2);
    assert_eq!(security.warnings(false, true).len(), 0);
    let mut host = LoopbackHost::bind_secured(any_port(), true, dm_lobby(), security).unwrap();
    let mut gm = LoopbackClient::connect_as(host.address(), Role::DungeonMaster, "GM").unwrap();
    assert!(seat(&mut host, &mut gm));
}

// ---- what is accepted from the other computer ----

#[test]
fn names_cannot_imitate_other_players() {
    let profile = |name: &str| PlayerProfile {
        name: name.into(),
        ..Default::default()
    };
    assert!(profile("Alice").valid());
    assert!(profile("Ålice 魔法").valid());
    for name in [
        "Al\u{202E}ice",
        "Alice\u{200B}",
        "\u{FEFF}Alice",
        "Al\nice",
        "\u{00AD}Alice",
    ] {
        assert!(!profile(name).valid(), "{name:?}");
        let submit = ProfileSubmit {
            profile: profile(name),
            profile_id: None,
        };
        assert!(submit.problem().is_some(), "{name:?}");
    }
    assert_eq!(display_text("A\u{202E}b\u{200B}c\n", 32), "Abc");
    assert_eq!(display_text("abcdef", 3), "abc");
}

#[test]
fn profile_ids_and_colours_have_their_real_shape() {
    let submit = |id: &str, colour| ProfileSubmit {
        profile: PlayerProfile {
            colour,
            ..Default::default()
        },
        profile_id: Some(id.into()),
    };
    let id = "0123456789abcdef0123456789abcdef";
    assert_eq!(submit(id, [1.42, 0.0, 16.0]).problem(), None);
    for bad in ["", "x", "../../etc/passwd", &"a".repeat(33)] {
        assert!(submit(bad, [1.0; 3]).problem().is_some(), "{bad:?}");
    }
    assert!(submit(id, [1e30, 1.0, 1.0]).problem().is_some());
    assert!(submit(id, [f32::INFINITY, 1.0, 1.0]).problem().is_some());
}

#[test]
fn content_names_from_a_host_stay_inside_the_content_folder() {
    let world = |name: &str| WorldPresentation {
        pickups: vec![PickupState {
            kind: PickupKind::Item { name: name.into() },
            handle: 1,
            generation: 1,
            transform: [0.0; 16],
        }],
        ..Default::default()
    };
    assert!(world("weapons/sword").valid());
    for bad in [
        "/etc/passwd",
        "\\\\server\\share",
        "C:/Windows/win.ini",
        "../../secret",
        "a/../../b",
        "",
    ] {
        assert!(!world(bad).valid(), "{bad:?}");
    }
}

#[test]
fn beacons_are_reduced_to_what_may_be_shown() {
    let mut advert = Advert::new(
        17700,
        "Game",
        "Levels/Havindr.lvl",
        1,
        LobbySettings::default(),
    );
    advert.name = format!("Evil\u{202E}{}", "x".repeat(4000));
    advert.level = "y\n".repeat(4000);
    let decoded = Advert::decode(&advert.encode()).unwrap();
    assert_eq!(decoded.name.chars().count(), 32);
    assert!(decoded.name.chars().all(displayable));
    assert_eq!(decoded.level.chars().count(), 64);

    let private = Advert::new(17700, "Game", "L.lvl", 1, LobbySettings::default())
        .with_participants([magicka_network::discovery::AdvertSeat {
            name: "Alice".into(),
            role: Role::Player,
            queued: false,
        }])
        .private();
    let decoded = Advert::decode(&private.encode()).unwrap();
    assert!(decoded.private && decoded.participants.is_empty());
}

#[test]
fn a_beacon_flood_cannot_grow_the_game_list_without_bound() {
    let port = 29_000 + (std::process::id() % 2000) as u16;
    let mut browser = LanBrowser::bind(port);
    assert!(browser.listening());
    let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut listed = 0;
    for game in 0..400u16 {
        let advert = Advert::new(10_000 + game, "Fake", "L.lvl", 1, LobbySettings::default());
        sender
            .send_to(&advert.encode(), ("127.0.0.1", port))
            .unwrap();
        if game % 32 == 31 {
            std::thread::sleep(Duration::from_millis(2));
            listed = browser.poll(Instant::now()).len();
        }
    }
    std::thread::sleep(Duration::from_millis(5));
    listed = listed.max(browser.poll(Instant::now()).len());
    assert!(
        listed > 0 && listed <= magicka_network::discovery::MAX_LAN_GAMES,
        "{listed}"
    );
}
