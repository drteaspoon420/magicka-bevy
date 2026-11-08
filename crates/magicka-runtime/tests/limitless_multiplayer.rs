//! Rewrite extensions beyond the original game: more than four players, hot
//! join beside the party, invaders/dungeon masters piloting NPCs, and saves
//! that park players whose profile is absent.
use magicka_network::{
    control::{Controls, InputCommand},
    lobby::*,
    session::*,
    transport::LoopbackClient,
};
use magicka_runtime::{
    network_session::HostedSession, play_session::PlaySession, saves::new_player_id,
};
use std::path::{Path, PathBuf};

fn content() -> Option<PathBuf> {
    let root = magicka_content::retail_content_dir();
    root.is_dir().then_some(root)
}

fn lab(content: &Path) -> (PlaySession, String) {
    let level = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut session = PlaySession::load(content, &level, "Wizard").unwrap();
    session.disable_scripts();
    session.step(1. / 60.);
    (session, level)
}

fn pump(
    host: &mut HostedSession,
    clients: &mut [LoopbackClient],
    session: &mut PlaySession,
    n: usize,
) {
    for _ in 0..n {
        for c in clients.iter_mut() {
            c.update();
        }
        host.poll(session).unwrap();
        session.step(1. / 60.);
        host.publish(session).unwrap();
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn six_players_join_and_late_joiners_spawn_beside_the_party() {
    let Some(content) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let (mut session, level) = lab(&content);
    let mut host = HostedSession::bind_lobby(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &session,
        LobbySettings {
            max_players: 6,
            ..Default::default()
        },
    )
    .unwrap();
    // Walk the host away from the start so hot joiners must follow it.
    let body = session.avatar().body;
    let start = session.world.physics.bodies[body].position;
    let mut clients: Vec<_> = (0..5)
        .map(|_| LoopbackClient::connect(host.transport.address()).unwrap())
        .collect();
    pump(&mut host, &mut clients, &mut session, 120);
    let mut slots: Vec<_> = clients.iter().filter_map(|c| c.player_slot()).collect();
    slots.sort_unstable();
    assert_eq!(slots, vec![1, 2, 3, 4, 5]);
    let frame = host.frame(&session).unwrap();
    assert_eq!(frame.players.len(), 6);
    assert_eq!(frame.lobby.max_players, 6);
    for slot in 4..6 {
        let p = session.world.players[slot]
            .as_ref()
            .expect("slot beyond four");
        assert!(p.playing && !p.dead());
        let position = session.world.physics.bodies[p.body].position;
        assert!(position.iter().all(|v| v.is_finite()));
        assert!(
            distance(position, start) < 12.0,
            "extra player {slot} spawned at {position:?}, far from {start:?}"
        );
    }
    // Every slot gets its own input stream.
    for (i, c) in clients.iter_mut().enumerate() {
        c.send_input(InputCommand {
            session: frame.physics.session,
            epoch: frame.physics.epoch,
            sequence: 1,
            controls: Controls {
                pad_movement: Some([if i % 2 == 0 { 1.0 } else { -1.0 }, 0.0]),
                ..Default::default()
            },
            history: Vec::new(),
        })
        .unwrap();
    }
    pump(&mut host, &mut clients, &mut session, 30);

    // Move the party far away, free a slot, then a hot joiner arrives near it.
    let far = [start[0] + 6.0, start[1], start[2] + 6.0];
    for p in session.world.players.iter().flatten() {
        let b = &mut session.world.physics.bodies[p.body];
        b.position = far;
    }
    clients.truncate(4);
    // Advance the host clock past the lease while the others keep beating.
    let base = std::time::Instant::now();
    for step in 0..=24 {
        for c in clients.iter_mut() {
            c.update();
        }
        host.poll_at(
            &mut session,
            base + std::time::Duration::from_millis(500 * step),
        )
        .unwrap();
    }
    assert_eq!(host.frame(&session).unwrap().players.len(), 5);
    let party = session.party_position().unwrap();
    clients.push(LoopbackClient::connect(host.transport.address()).unwrap());
    pump(&mut host, &mut clients, &mut session, 120);
    let joined = clients
        .last()
        .unwrap()
        .player_slot()
        .expect("hot joiner slot");
    let p = session.world.players[joined].as_ref().unwrap();
    let position = session.world.physics.bodies[p.body].position;
    assert!(
        distance(position, party) < 5.0,
        "hot joiner at {position:?}, party at {party:?}"
    );
}

#[test]
fn locked_hot_join_queues_a_spectator_until_the_next_round() {
    let Some(content) = content() else {
        return;
    };
    let (mut session, level) = lab(&content);
    let mut host = HostedSession::bind_lobby(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &session,
        LobbySettings::versus(),
    )
    .unwrap();
    let mut clients = vec![LoopbackClient::connect(host.transport.address()).unwrap()];
    pump(&mut host, &mut clients, &mut session, 60);
    assert!(clients[0].player_slot().is_none());
    let frame = host.frame(&session).unwrap();
    assert_eq!(frame.players.len(), 1, "only the host plays");
    assert_eq!(frame.participants.len(), 1);
    assert!(frame.participants[0].queued);
    assert_eq!(frame.participants[0].role, Role::Spectator);
    host.restart(&mut session).unwrap();
    pump(&mut host, &mut clients, &mut session, 60);
    assert_eq!(clients[0].player_slot(), Some(1));
    assert_eq!(host.frame(&session).unwrap().players.len(), 2);
}

#[test]
fn invaders_pilot_enemies_and_jump_to_another_after_death_and_dm_commands_apply() {
    let Some(content) = content() else {
        return;
    };
    let (mut session, level) = lab(&content);
    for _ in 0..30 {
        session.step(1. / 60.);
    }
    let first = session.spawn_npc("Goblin_pirate", "idle", 7.0).unwrap();
    let second = session.spawn_npc("Goblin_pirate", "idle", 12.0).unwrap();
    let mut host = HostedSession::bind_lobby(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &session,
        LobbySettings {
            invaders: true,
            dungeon_masters: true,
            ..Default::default()
        },
    )
    .unwrap();
    let address = host.transport.address();
    let mut clients = vec![
        LoopbackClient::connect_as(address, Role::Invader, "Invader").unwrap(),
        LoopbackClient::connect_as(address, Role::DungeonMaster, "DM").unwrap(),
    ];
    pump(&mut host, &mut clients, &mut session, 60);
    let frame = host.frame(&session).unwrap();
    let invader = frame
        .participants
        .iter()
        .find(|p| p.role == Role::Invader)
        .expect("invader seated");
    let body = invader.possessed.expect("invader pilots an NPC");
    assert!(body == first || body == second);
    assert!(session.world.possessed.contains_key(&body));

    // Piloting moves the NPC with ordinary controls.
    let npc_body = session.world.npcs[&body].body;
    let before = session.world.physics.bodies[npc_body].position;
    for sequence in 1..=40 {
        clients[0]
            .send_input(InputCommand {
                session: frame.physics.session,
                epoch: frame.physics.epoch,
                sequence,
                controls: Controls {
                    pad_movement: Some([1.0, 0.0]),
                    ..Default::default()
                },
                history: Vec::new(),
            })
            .unwrap();
        pump(&mut host, &mut clients, &mut session, 1);
    }
    let after = session.world.physics.bodies[npc_body].position;
    assert!(distance(before, after) > 0.5, "possessed NPC did not move");

    // After its death the invader takes the other goblin.
    session.smite(body).unwrap();
    pump(&mut host, &mut clients, &mut session, 30);
    let frame = host.frame(&session).unwrap();
    let invader = frame
        .participants
        .iter()
        .find(|p| p.role == Role::Invader)
        .unwrap();
    let other = if body == first { second } else { first };
    assert_eq!(invader.possessed, Some(other));

    // Dungeon master: spawn and heal.
    let npcs = session.world.npcs.len();
    let at = session.party_position().unwrap();
    clients[1]
        .send_dm(DmRequest {
            sequence: 1,
            command: DmCommand::SpawnNpc {
                template: "Goblin_pirate".into(),
                position: [at[0] + 3.0, at[1], at[2]],
            },
        })
        .unwrap();
    clients[1]
        .send_dm(DmRequest {
            sequence: 2,
            command: DmCommand::SetHotJoin(false),
        })
        .unwrap();
    pump(&mut host, &mut clients, &mut session, 60);
    assert_eq!(session.world.npcs.len(), npcs + 1);
    assert!(!host.lobby().hot_join);
    // Editor commands (protocol 15): spawn a prop, then remove it.
    let mut placement = [0.0f32; 16];
    for i in 0..4 {
        placement[i * 5] = 1.0;
    }
    placement[12..15].copy_from_slice(&[at[0] - 3.0, at[1] + 1.0, at[2]]);
    let bathballs = |s: &PlaySession| -> Vec<u16> {
        s.world
            .entities
            .iter()
            .filter(|e| !e.removed && e.template_name.as_deref() == Some("bathball"))
            .map(|e| e.handle)
            .collect()
    };
    let before = bathballs(&session);
    clients[1]
        .send_dm(DmRequest {
            sequence: 3,
            command: DmCommand::Spawn {
                spawnable: "prop:bathball".into(),
                transform: placement,
                order: None,
                movable: false,
            },
        })
        .unwrap();
    pump(&mut host, &mut clients, &mut session, 30);
    let spawned: Vec<u16> = bathballs(&session)
        .into_iter()
        .filter(|h| !before.contains(h))
        .collect();
    assert_eq!(spawned.len(), 1, "dungeon master prop spawn");
    clients[1]
        .send_dm(DmRequest {
            sequence: 4,
            command: DmCommand::Remove { handle: spawned[0] },
        })
        .unwrap();
    pump(&mut host, &mut clients, &mut session, 30);
    assert!(!bathballs(&session).contains(&spawned[0]));
    // An invader cannot use dungeon master powers.
    clients[0]
        .send_dm(DmRequest {
            sequence: 1,
            command: DmCommand::SetPlayerLimit(32),
        })
        .unwrap();
    pump(&mut host, &mut clients, &mut session, 30);
    assert_eq!(host.lobby().max_players, 4);
    // In-game commands stop at the in-game limit on a default host.
    for (sequence, limit, expected) in [
        (5, IN_GAME_PLAYER_LIMIT + 1, DEFAULT_PLAYER_LIMIT),
        (6, IN_GAME_PLAYER_LIMIT, IN_GAME_PLAYER_LIMIT),
        (7, DEFAULT_PLAYER_LIMIT, DEFAULT_PLAYER_LIMIT),
    ] {
        clients[1]
            .send_dm(DmRequest {
                sequence,
                command: DmCommand::SetPlayerLimit(limit),
            })
            .unwrap();
        pump(&mut host, &mut clients, &mut session, 30);
        assert_eq!(host.lobby().max_players, expected);
    }
}

#[test]
fn saves_park_absent_profiles_and_restore_them_when_they_return() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene_named("havindr_s3", "start").unwrap();
    for _ in 0..5 {
        s.step(1.0 / 60.0);
    }
    s.join_player_at_start(1, "Wizard").unwrap();
    s.set_player_loadout(0, "Wizard", "staff_of_fire", "weapon_stung")
        .unwrap();
    let owner = new_player_id().unwrap();
    let friend = new_player_id().unwrap();
    s.player_ids[0] = Some(owner.clone());
    s.player_ids[1] = Some(friend.clone());
    s.unlocked_magicks = [0x107, 0x801, 0, 0].into();
    let save = s.disk_save().unwrap();

    // A fresh profile plays the save: it starts with robe defaults and the
    // party's magicks; both saved players are parked.
    let fresh = new_player_id().unwrap();
    let mut other = PlaySession::load_disk_save_for(&root, &save, &fresh, "Wizard").unwrap();
    assert_eq!(other.player_ids[0].as_deref(), Some(fresh.as_str()));
    assert_eq!(other.unlocked_magicks[0], 0x107 | 0x801);
    assert!(
        other
            .world
            .players
            .iter()
            .flatten()
            .filter(|p| p.playing)
            .count()
            == 1
    );
    assert_eq!(other.parked_players.len(), 2);
    let staff = |s: &PlaySession, slot: usize| {
        s.world.players[slot]
            .as_ref()
            .unwrap()
            .items
            .iter()
            .find(|(k, _)| *k == 1)
            .map(|(_, i)| i.definition.name.clone())
    };
    assert_ne!(staff(&other, 0).as_deref(), Some("staff_of_fire"));
    // Its save keeps the parked players.
    let resaved = other.disk_save().unwrap();
    let ids: Vec<_> = resaved["players"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["id"].as_str().unwrap().to_string(), r["parked"] == true))
        .collect();
    assert!(ids.contains(&(fresh.clone(), false)));
    assert!(ids.contains(&(owner.clone(), true)));
    assert!(ids.contains(&(friend.clone(), true)));

    // The friend joins this game later and reclaims their player.
    other.join_player_at_start(1, "Wizard").unwrap();
    assert!(other.claim_parked(1, &friend, "Wizard").unwrap());
    assert_eq!(other.unlocked_magicks[1] & 0x801, 0x801);
    assert_eq!(other.parked_players.len(), 1);

    // The original owner starts the resaved game: their staff returns.
    let back = PlaySession::load_disk_save_for(&root, &resaved, &owner, "Wizard").unwrap();
    assert_eq!(staff(&back, 0).as_deref(), Some("staff_of_fire"));
    assert_eq!(back.unlocked_magicks[0], 0x107);
    assert!(back
        .parked_players
        .iter()
        .any(|r| r["id"] == fresh.as_str()));
}

#[test]
fn host_camera_follows_only_local_players_and_local_coop_shares_it() {
    let Some(content) = content() else {
        return;
    };
    let (mut session, level) = lab(&content);
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    let mut clients = vec![LoopbackClient::connect(host.transport.address()).unwrap()];
    pump(&mut host, &mut clients, &mut session, 60);
    let remote = clients[0].player_slot().unwrap();
    let local = host
        .add_local_player(
            PlayerProfile {
                name: "Couch".into(),
                ..Default::default()
            },
            &mut session,
        )
        .unwrap();
    assert_ne!(local, remote);
    pump(&mut host, &mut clients, &mut session, 5);
    assert_eq!(
        session.camera_slots,
        Some([0, local].into_iter().collect()),
        "remote players never steer the host camera"
    );
    // Teleport the remote player far away: the host camera stays with the
    // local pair.
    let body = session.world.players[remote].as_ref().unwrap().body;
    let home = session.world.physics.bodies[session.avatar().body].position;
    session.world.physics.bodies[body].position = [home[0] + 200.0, home[1], home[2] + 200.0];
    pump(&mut host, &mut clients, &mut session, 120);
    let target = session.world.camera.ground_position();
    assert!(
        distance(target, home) < 20.0,
        "camera {target:?} drifted toward the remote player"
    );
    let frame = host.frame(&session).unwrap();
    assert!(frame.players.iter().any(|p| p.slot == local));
    host.remove_local_player(local, &mut session).unwrap();
    pump(&mut host, &mut clients, &mut session, 5);
    assert_eq!(session.camera_slots, Some([0].into_iter().collect()));
    assert!(host
        .add_local_player(PlayerProfile::default(), &mut session)
        .is_ok());
}

#[test]
fn observer_clients_mirror_the_hosts_first_player_beside_their_dormant_avatar() {
    use magicka_runtime::network_session::{RemotePlayers, OBSERVER_VIEW_SLOT};
    let Some(content) = content() else {
        return;
    };
    let (mut session, level) = lab(&content);
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    let mut clients = vec![LoopbackClient::connect(host.transport.address()).unwrap()];
    pump(&mut host, &mut clients, &mut session, 60);
    let frame = host.frame(&session).unwrap();
    assert_eq!(frame.players.len(), 2);
    let (mut mirror, _) = lab(&content);
    mirror.depart_network_player(0).unwrap();
    let mut remotes = RemotePlayers::default();
    remotes.reconcile(&frame, None, &mut mirror).unwrap();
    let shown = mirror.world.players[OBSERVER_VIEW_SLOT]
        .as_ref()
        .expect("host slot 0 mirror");
    assert!(shown.playing);
    let joined = clients[0].player_slot().unwrap();
    assert!(mirror.world.players[joined].is_some());
    assert!(
        !mirror.world.players[0].as_ref().unwrap().playing,
        "own avatar stays dormant"
    );
    // Becoming a player remaps every mirror; the observer slot empties.
    remotes
        .reconcile(&frame, Some(joined), &mut mirror)
        .unwrap();
    assert!(mirror
        .world
        .players
        .get(OBSERVER_VIEW_SLOT)
        .is_none_or(Option::is_none));
}

#[test]
fn editor_spawns_moves_and_removes_catalogue_entries() {
    use magicka_runtime::play_session::EditorSpawnOptions;
    let Some(content) = content() else {
        return;
    };
    let (mut s, _) = lab(&content);
    let at = s.party_position().unwrap();
    let mut m = [0.0f32; 16];
    for i in 0..4 {
        m[i * 5] = 1.0;
    }
    m[12..15].copy_from_slice(&[at[0] + 3.0, at[1] + 1.0, at[2]]);
    let idle = EditorSpawnOptions {
        order: Some("idle".into()),
        movable: false,
    };
    let npc = s
        .editor_spawn("npc:Goblin_pirate", m, &idle)
        .unwrap()
        .unwrap();
    let body = s.world.npcs[&npc].body;
    let feet = s.world.physics.bodies[body].feet();
    assert!(distance(feet, [at[0] + 3.0, 0.0, at[2]]) < 1e-3, "{feet:?}");
    // The gizmo moves the model (feet) transform.
    let mut moved = m;
    moved[12] += 2.0;
    moved[13] = feet[1];
    s.editor_move(npc, moved).unwrap();
    let feet = s.world.physics.bodies[body].feet();
    assert!((feet[0] - (at[0] + 5.0)).abs() < 1e-3 && (feet[1] - moved[13]).abs() < 1e-3);
    let prop = s
        .editor_spawn("prop:bathball", m, &EditorSpawnOptions::default())
        .unwrap()
        .unwrap();
    s.editor_remove(prop).unwrap();
    s.editor_remove(npc).unwrap();
    for handle in [prop, npc] {
        assert!(s
            .world
            .entities
            .iter()
            .any(|e| e.handle == handle && e.removed));
    }
    let avatar = s.avatar().handle;
    assert!(s.editor_remove(avatar).is_err());
    assert!(s.editor_spawn("deco:tree.glb", m, &idle).is_err());
    assert!(s
        .editor_spawn(
            "npc:Goblin_pirate",
            m,
            &EditorSpawnOptions {
                order: Some("dance".into()),
                movable: false
            }
        )
        .is_err());
}
