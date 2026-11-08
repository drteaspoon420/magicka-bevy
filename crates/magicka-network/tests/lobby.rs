//! Lobby policy over real loopback UDP: player limits beyond the original
//! four, hot-join locking, queued promotion, role changes and refusal.
use magicka_network::{
    lobby::*,
    transport::{LoopbackClient, LoopbackHost},
};

fn pump(host: &mut LoopbackHost, clients: &mut [LoopbackClient], frames: usize) {
    for _ in 0..frames {
        for client in clients.iter_mut() {
            client.update();
        }
        host.update();
    }
}

fn dedicated(lobby: LobbySettings) -> LoopbackHost {
    LoopbackHost::bind_lobby("127.0.0.1:0".parse().unwrap(), true, lobby).unwrap()
}

#[test]
fn admission_policy_is_pure_and_defaults_to_the_original_four() {
    let lobby = LobbySettings::default();
    assert_eq!(lobby.max_players, 4);
    assert!(lobby.hot_join && lobby.spectators);
    assert!(!lobby.dungeon_masters && !lobby.invaders);
    assert!(!LobbySettings::versus().hot_join);
    assert_eq!(lobby.admit(Role::Player, 3, true), Admission::Player);
    assert_eq!(
        lobby.admit(Role::Player, 4, true),
        Admission::Observer {
            role: Role::Spectator,
            queued: true
        }
    );
    let locked = LobbySettings::versus();
    assert_eq!(
        locked.admit(Role::Player, 1, true),
        Admission::Observer {
            role: Role::Spectator,
            queued: true
        }
    );
    assert_eq!(locked.admit(Role::Player, 1, false), Admission::Player);
    assert!(matches!(
        lobby.admit(Role::DungeonMaster, 0, true),
        Admission::Rejected(_)
    ));
    let no_watchers = LobbySettings {
        spectators: false,
        ..LobbySettings::versus()
    };
    assert!(matches!(
        no_watchers.admit(Role::Player, 1, true),
        Admission::Rejected(_)
    ));
    let big = LobbySettings::default()
        .parse_onto("players=12,hotjoin=off,dm,invaders=on")
        .unwrap();
    assert_eq!(big.max_players, 12);
    assert!(!big.hot_join && big.dungeon_masters && big.invaders);
    assert!(LobbySettings::default().parse_onto("players=0").is_err());
    assert!(LobbySettings::default()
        .parse_onto(&format!("players={}", PLAYER_SLOT_CAP + 1))
        .is_err());
    assert!(LobbySettings::default().parse_onto("bogus=1").is_err());
}

#[test]
fn eight_player_lobby_seats_slots_beyond_four_and_queues_the_ninth() {
    let mut host = dedicated(LobbySettings {
        max_players: 8,
        ..Default::default()
    });
    let mut clients: Vec<_> = (0..9)
        .map(|i| {
            LoopbackClient::connect_as(host.address(), Role::Player, &format!("P{i}")).unwrap()
        })
        .collect();
    pump(&mut host, &mut clients, 300);
    let mut slots: Vec<_> = clients.iter().filter_map(|c| c.player_slot()).collect();
    slots.sort_unstable();
    assert_eq!(slots, (0..8).collect::<Vec<_>>());
    assert_eq!(host.player_slots(), (0..8).collect::<Vec<_>>());
    let queued: Vec<_> = clients
        .iter()
        .filter_map(|c| c.welcome())
        .filter(|w| w.queued)
        .collect();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].role, Role::Spectator);
    assert!(queued[0].seat >= OBSERVER_SEAT_BASE);
    let seats = host.seats();
    assert_eq!(seats.len(), 9);
    assert!(seats.iter().any(|s| s.name == "P0"));

    // Raising the limit promotes the queued player immediately (hot join on).
    host.set_lobby(LobbySettings {
        max_players: 9,
        ..Default::default()
    })
    .unwrap();
    pump(&mut host, &mut clients, 200);
    let mut slots: Vec<_> = clients.iter().filter_map(|c| c.player_slot()).collect();
    slots.sort_unstable();
    assert_eq!(slots, (0..9).collect::<Vec<_>>());
    assert!(host.take_departed_observers().len() == 1);
}

#[test]
fn locked_hot_join_waits_for_the_next_round_and_roles_can_change() {
    let mut host = dedicated(LobbySettings {
        max_players: 6,
        ..LobbySettings::versus()
    });
    let mut clients =
        vec![LoopbackClient::connect_as(host.address(), Role::Player, "Late").unwrap()];
    pump(&mut host, &mut clients, 200);
    let welcome = clients[0].welcome().expect("seated");
    assert_eq!((welcome.role, welcome.queued), (Role::Spectator, true));
    assert!(host.player_slots().is_empty());
    // Still locked: no promotion during the round.
    pump(&mut host, &mut clients, 50);
    assert!(clients[0].player_slot().is_none());
    // The next round is a join point.
    assert_eq!(host.begin_round(), vec![0]);
    pump(&mut host, &mut clients, 100);
    assert_eq!(clients[0].player_slot(), Some(0));

    // A player can step out to spectate, freeing the slot.
    clients[0]
        .request_role(RoleRequest {
            sequence: 1,
            role: Role::Spectator,
            name: "Watcher".into(),
        })
        .unwrap();
    pump(&mut host, &mut clients, 100);
    let w = clients[0].welcome().unwrap();
    assert_eq!((w.role, w.queued), (Role::Spectator, false));
    assert_eq!(host.take_departed(), vec![0]);
    assert!(host.player_slots().is_empty());
    assert_eq!(host.seats()[0].name, "Watcher");
}

#[test]
fn disallowed_roles_are_refused_and_dungeon_master_commands_reach_the_host() {
    let mut host = dedicated(LobbySettings {
        dungeon_masters: true,
        ..Default::default()
    });
    let mut clients = vec![
        LoopbackClient::connect_as(host.address(), Role::DungeonMaster, "DM").unwrap(),
        LoopbackClient::connect_as(host.address(), Role::Invader, "Nope").unwrap(),
    ];
    pump(&mut host, &mut clients, 200);
    let dm = clients[0].welcome().expect("dm seated");
    assert_eq!(dm.role, Role::DungeonMaster);
    assert!(clients[1].welcome().is_none());
    assert!(clients[1].refusal().is_some());
    clients[0]
        .send_dm(DmRequest {
            sequence: 1,
            command: DmCommand::SetPlayerLimit(16),
        })
        .unwrap();
    assert!(clients[0]
        .send_dm(DmRequest {
            sequence: 2,
            command: DmCommand::SpawnNpc {
                template: "../escape".into(),
                position: [0.0; 3],
            },
        })
        .is_err());
    let mut got = Vec::new();
    for _ in 0..200 {
        pump(&mut host, &mut clients, 1);
        got.extend(host.take_dm_requests());
        if !got.is_empty() {
            break;
        }
    }
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].role, Role::DungeonMaster);
    assert_eq!(got[0].seat, dm.seat);
    assert_eq!(got[0].request.command, DmCommand::SetPlayerLimit(16));
}

#[test]
fn hostnames_resolve_to_ipv4_and_join_a_server_on_all_interfaces() {
    let resolved = magicka_network::transport::resolve("localhost:17700").unwrap();
    assert!(
        resolved.is_ipv4(),
        "localhost must prefer IPv4, got {resolved}"
    );
    let mut host = dedicated(LobbySettings::default());
    drop(host);
    host = LoopbackHost::bind_lobby("0.0.0.0:0".parse().unwrap(), true, LobbySettings::default())
        .unwrap();
    let port = host.address().port();
    let address = magicka_network::transport::resolve(&format!("localhost:{port}")).unwrap();
    let mut clients = vec![LoopbackClient::connect(address).unwrap()];
    pump(&mut host, &mut clients, 200);
    assert_eq!(clients[0].player_slot(), Some(0));
}

#[test]
fn editor_dm_commands_are_validated() {
    use magicka_network::lobby::spawnable_id;
    assert!(spawnable_id("npc:Goblin_pirate"));
    assert!(spawnable_id("prop:Barrels/barrel_01"));
    assert!(!spawnable_id("prop:../secret"));
    assert!(!spawnable_id("deco:tree.glb"));
    assert!(!spawnable_id("npc:"));
    let mut transform = [0.0; 16];
    transform[0] = 1.0;
    let spawn = |spawnable: &str, transform: [f32; 16], order: Option<&str>| DmRequest {
        sequence: 1,
        command: DmCommand::Spawn {
            spawnable: spawnable.into(),
            transform,
            order: order.map(String::from),
            movable: false,
        },
    };
    assert!(spawn("npc:Zombie", transform, Some("idle")).valid());
    assert!(!spawn("npc:Zombie", transform, Some("../x")).valid());
    let mut bad = transform;
    bad[12] = f32::NAN;
    assert!(!spawn("npc:Zombie", bad, None).valid());
    assert!(!DmRequest {
        sequence: 2,
        command: DmCommand::Move {
            handle: 3,
            transform: bad
        }
    }
    .valid());
}
