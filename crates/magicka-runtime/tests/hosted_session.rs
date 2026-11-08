use magicka_network::{session::*, transport::LoopbackClient};
use magicka_runtime::{network_session::HostedSession, play_session::PlaySession};
use std::path::PathBuf;
fn session() -> (PlaySession, String) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let level = root
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut session = PlaySession::load(
        &root.join("../../../../target/Magicka/Content"),
        &level,
        "Wizard",
    )
    .unwrap();
    session.step(1. / 60.);
    (session, level)
}
fn pump(
    host: &mut HostedSession,
    client: &mut LoopbackClient,
    session: &mut PlaySession,
) -> GameFrame {
    for _ in 0..400 {
        client.update();
        host.poll(session).unwrap();
        host.publish(session).unwrap();
        if let Some(frame) = client.take_game_frames().into_iter().last() {
            return frame;
        }
    }
    panic!("no authoritative frame");
}

#[test]
fn defeat_before_first_checkpoint_reloads_world_once_and_keeps_udp_owners() {
    use magicka_network::control::{Controls, InputCommand};
    for dedicated in [false, true] {
        let content =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
        let mut session = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
        session.disable_scripts();
        let initial_scene = session.scene_name().to_string();
        let mut host = HostedSession::bind(
            "127.0.0.1:0".parse().unwrap(),
            "Havindr.lvl",
            dedicated,
            PlayerProfile::default(),
            &session,
        )
        .unwrap();
        let mut first = LoopbackClient::connect(host.transport.address()).unwrap();
        pump(&mut host, &mut first, &mut session);
        let own = first.player_slot().unwrap();
        let mut second = LoopbackClient::connect(host.transport.address()).unwrap();
        pump(&mut host, &mut second, &mut session);
        let second_slot = second.player_slot().unwrap();
        first
            .send_profile(ProfileRequest {
                profile_id: None,
                sequence: 1,
                profile: PlayerProfile {
                    name: "Before checkpoint".into(),
                    robe: "Wizard_Samurai".into(),
                    ..Default::default()
                },
            })
            .unwrap();
        for _ in 0..12 {
            pump(&mut host, &mut first, &mut session);
            second.update();
        }
        let defaults = host.frame(&session).unwrap();
        let robe_defaults: std::collections::BTreeMap<_, Vec<_>> = defaults
            .players
            .iter()
            .map(|player| {
                let fresh =
                    PlaySession::load(&content, "Havindr.lvl", &player.profile.robe).unwrap();
                (
                    player.slot,
                    fresh
                        .avatar()
                        .items
                        .iter()
                        .filter(|(slot, _)| matches!(slot, 0 | 1))
                        .map(|(_, item)| item.definition.name.clone())
                        .collect(),
                )
            })
            .collect();
        session.change_scene_named("havindr_s3", "start").unwrap();
        assert_ne!(session.scene_name(), initial_scene);
        assert!(session.checkpoint.is_none());
        session.spawn_npc("Goblin_pirate", "idle", 7.0).unwrap();
        assert!(!session.world.npcs.is_empty());
        for player in &defaults.players {
            session
                .set_player_loadout(
                    player.slot,
                    &player.profile.robe,
                    "staff_heirloom_grandfather_clock",
                    &player.items[0],
                )
                .unwrap();
            session.unlocked_magicks[player.slot] |= 1 << 3;
            let p = session.world.players[player.slot].as_mut().unwrap();
            p.character.combat.dead = true;
            p.character.combat.health.hit_points = 0.0;
        }
        let before = host.frame(&session).unwrap();
        for _ in 0..400 {
            session.end_game.update(1.0 / 60.0, true);
        }
        assert!(session.end_game.restart);
        // Liveness is heartbeat-paced (protocol 11): the second client is
        // alive across the host's long restart work, so one is in flight.
        second.heartbeat_now();
        second.update();
        host.restart(&mut session).unwrap();
        let after = pump(&mut host, &mut first, &mut session);
        let observed = pump(&mut host, &mut second, &mut session);
        assert!(
            !session.end_game.restart,
            "restart request must be consumed"
        );
        assert_eq!(session.scene_name(), initial_scene);
        assert!(
            session.world.npcs.is_empty(),
            "discard the defeated scene's NPCs"
        );
        assert_eq!(session.script_error.as_deref(), Some("scripts disabled"));
        assert_eq!(after.level, before.level);
        assert_eq!(after.physics.session, before.physics.session);
        assert!(after.physics.epoch > before.physics.epoch);
        assert_eq!(observed.physics.epoch, after.physics.epoch);
        assert_eq!(first.player_slot(), Some(own));
        assert_eq!(second.player_slot(), Some(second_slot));
        assert_eq!(after.players.len(), defaults.players.len());
        for player in &after.players {
            let default = defaults
                .players
                .iter()
                .find(|p| p.slot == player.slot)
                .unwrap();
            assert_eq!(player.profile, default.profile);
            assert_eq!(player.items.as_slice(), robe_defaults[&player.slot]);
            assert_eq!(player.unlocked_magicks, default.unlocked_magicks | (1 << 3));
            assert!(!player.dead);
            assert_eq!(
                player.health,
                session.world.players[player.slot]
                    .as_ref()
                    .unwrap()
                    .character
                    .combat
                    .health
                    .max_hit_points
            );
        }
        let command = |epoch| {
            let mut controls = Controls::default();
            controls.elements[0] = true;
            InputCommand {
                session: after.physics.session,
                epoch,
                sequence: 100,
                controls,
                history: Vec::new(),
            }
        };
        first.send_input(command(before.physics.epoch)).unwrap();
        for _ in 0..12 {
            pump(&mut host, &mut first, &mut session);
            second.update();
        }
        assert!(session.world.players[own]
            .as_ref()
            .unwrap()
            .conjure
            .queue
            .is_empty());
        first.send_input(command(after.physics.epoch)).unwrap();
        for _ in 0..12 {
            pump(&mut host, &mut first, &mut session);
            second.update();
        }
        assert_eq!(
            session.world.players[own]
                .as_ref()
                .unwrap()
                .conjure
                .queue
                .len(),
            1
        );
        for _ in 0..30 {
            session.step(1.0 / 60.0);
            pump(&mut host, &mut first, &mut session);
            second.update();
        }
        assert!(!session.end_game.restart);
        assert_eq!(
            host.frame(&session).unwrap().physics.epoch,
            after.physics.epoch,
            "a living restored world cannot repeatedly restart"
        );
    }
}

#[test]
fn campaign_completion_keeps_udp_owners_and_gear_and_loads_client_worlds() {
    use magicka_network::control::{Controls, InputCommand};
    use magicka_runtime::{
        end_game::EndGameCondition,
        network_session::{mirror_player_spell_state, mirror_world, RemotePlayers},
    };
    for dedicated in [false, true] {
        let content =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
        let mut session = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
        session.disable_scripts();
        let mut host = HostedSession::bind(
            "127.0.0.1:0".parse().unwrap(),
            "Havindr.lvl",
            dedicated,
            PlayerProfile::default(),
            &session,
        )
        .unwrap();
        let mut first = LoopbackClient::connect(host.transport.address()).unwrap();
        pump(&mut host, &mut first, &mut session);
        let own = first.player_slot().unwrap();
        let mut second = LoopbackClient::connect(host.transport.address()).unwrap();
        pump(&mut host, &mut second, &mut session);
        let profile = PlayerProfile {
            name: "Campaign wizard".into(),
            ..Default::default()
        };
        first
            .send_profile(ProfileRequest {
                profile_id: None,
                sequence: 1,
                profile: profile.clone(),
            })
            .unwrap();
        for _ in 0..12 {
            pump(&mut host, &mut first, &mut session);
            second.update();
        }
        let initial = host.frame(&session).unwrap();
        for player in &initial.players {
            session
                .set_player_loadout(
                    player.slot,
                    &player.profile.robe,
                    "staff_heirloom_grandfather_clock",
                    &player.items[0],
                )
                .unwrap();
            session.unlocked_magicks[player.slot] |= 1 << 3;
            let staff = &mut session.world.players[player.slot]
                .as_mut()
                .unwrap()
                .items
                .iter_mut()
                .find(|(s, _)| *s == 1)
                .unwrap()
                .1;
            staff.special_cooldown = 7.0;
            staff.spells.add(
                magicka_sim::spells::Spell::default_for(magicka_sim::spells::Elements::WATER),
                false,
                &mut vec![],
            );
        }
        let before = host.frame(&session).unwrap();
        assert_eq!(
            before
                .players
                .iter()
                .find(|p| p.slot == own)
                .unwrap()
                .profile,
            profile
        );
        let mut mirror = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
        mirror.disable_scripts();
        let mut remotes = RemotePlayers::default();
        remotes.reconcile(&before, Some(own), &mut mirror).unwrap();
        // Exercise the campaign completion timer, rather than loading directly.
        session
            .end_game
            .begin(EndGameCondition::LevelComplete, true, false, 0.0);
        for _ in 0..240 {
            session.end_game.update(1.0 / 60.0, false);
        }
        assert_eq!(session.next_level(), Some("Battlefield.lvl"));
        let after = pump(&mut host, &mut first, &mut session);
        let observed = pump(&mut host, &mut second, &mut session);
        assert_eq!(after.level, "Battlefield.lvl");
        assert_eq!(observed.level, after.level);
        assert_eq!(after.physics.session, before.physics.session);
        assert!(after.physics.epoch > before.physics.epoch);
        assert_eq!(first.player_slot(), Some(own));
        assert_eq!(after.players.len(), before.players.len());
        for player in &after.players {
            let prior = before
                .players
                .iter()
                .find(|p| p.slot == player.slot)
                .unwrap();
            assert_eq!(player.profile, prior.profile);
            assert_eq!(player.items, prior.items);
            assert_eq!(player.item_queues, prior.item_queues);
            assert_eq!(player.unlocked_magicks, prior.unlocked_magicks);
            assert_eq!(player.attached_slots, prior.attached_slots);
            let staff = &session.world.players[player.slot]
                .as_ref()
                .unwrap()
                .items
                .iter()
                .find(|(s, _)| *s == 1)
                .unwrap()
                .1;
            assert_eq!(staff.special_cooldown, 7.0);
        }
        let command = |epoch| {
            let mut controls = Controls::default();
            controls.elements[0] = true;
            InputCommand {
                session: after.physics.session,
                epoch,
                sequence: 100,
                controls,
                history: Vec::new(),
            }
        };
        first.send_input(command(before.physics.epoch)).unwrap();
        for _ in 0..12 {
            pump(&mut host, &mut first, &mut session);
            second.update();
        }
        assert!(
            session.world.players[own]
                .as_ref()
                .unwrap()
                .conjure
                .queue
                .is_empty(),
            "old-level input cannot affect the new campaign level"
        );
        first.send_input(command(after.physics.epoch)).unwrap();
        for _ in 0..12 {
            pump(&mut host, &mut first, &mut session);
            second.update();
        }
        assert_eq!(
            session.world.players[own]
                .as_ref()
                .unwrap()
                .conjure
                .queue
                .len(),
            1,
            "the retained owner can control its player in the new epoch"
        );
        assert!(mirror_world(&mut mirror, &after).unwrap());
        assert_eq!(mirror.level_file(), after.level);
        assert_eq!(mirror.scene_name(), after.physics.scene);
        assert_eq!(mirror.script_error.as_deref(), Some("scripts disabled"));
        remotes.reconcile(&after, Some(own), &mut mirror).unwrap();
        let player = after.players.iter().find(|p| p.slot == own).unwrap();
        mirror
            .set_player_loadout(0, &player.profile.robe, &player.items[1], &player.items[0])
            .unwrap();
        mirror_player_spell_state(&mut mirror, 0, player).unwrap();
        assert_eq!(mirror.unlocked_magicks[0], player.unlocked_magicks);
        assert!(
            !mirror_world(&mut mirror, &after).unwrap(),
            "same snapshot cannot reload twice"
        );
        assert_eq!(
            host.frame(&session).unwrap().physics.epoch,
            after.physics.epoch
        );
    }
}

#[test]
fn remote_revive_input_restores_another_player_and_publishes_attachment_lifetimes() {
    use magicka_network::control::{Controls, InputCommand};
    use magicka_runtime::avatar::AvatarState;
    use magicka_sim::spells::MagickType;
    let (mut session, level) = session();
    session.disable_scripts();
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    let mut client = LoopbackClient::connect(host.transport.address()).unwrap();
    let initial = pump(&mut host, &mut client, &mut session);
    let caster = client.player_slot().unwrap();
    assert_eq!(caster, 1);
    let original = initial
        .players
        .iter()
        .find(|p| p.slot == 0)
        .unwrap()
        .clone();
    session
        .set_player_loadout(
            0,
            "Wizard",
            "staff_heirloom_grandfather_clock",
            &original.items[0],
        )
        .unwrap();
    session.unlocked_magicks[0] |= 1 << MagickType::Revive as u8;
    let unlocks = session.unlocked_magicks[0];
    session.world.players[0].as_mut().unwrap().kill();
    for _ in 0..600 {
        session.step(1.0 / 60.0);
        pump(&mut host, &mut client, &mut session);
        if session.dead() {
            break;
        }
    }
    assert!(session.dead());
    let dead = pump(&mut host, &mut client, &mut session);
    let victim = dead.players.iter().find(|p| p.slot == 0).unwrap();
    assert!(victim.attached_slots.is_empty());
    assert_eq!(victim.items[1], "staff_heirloom_grandfather_clock");
    let dropped_staff = *session
        .world
        .item_pickups
        .iter()
        .find(|(_, pickup)| {
            pickup
                .item
                .as_ref()
                .is_some_and(|item| item.definition.name == "staff_heirloom_grandfather_clock")
        })
        .expect("the dead wizard's staff becomes a pickup")
        .0;
    assert!(
        dead.physics
            .bodies
            .iter()
            .any(|body| body.handle == dropped_staff),
        "released equipment must also appear in the pickup physics snapshot"
    );
    let mut sequence = 0;
    for element in [1, 4] {
        sequence += 1;
        let mut controls = Controls::default();
        controls.elements[element] = true;
        client
            .send_input(InputCommand {
                session: host.identity,
                epoch: dead.physics.epoch,
                sequence,
                controls,
                history: Vec::new(),
            })
            .unwrap();
        for _ in 0..12 {
            pump(&mut host, &mut client, &mut session);
            session.step(1.0 / 60.0);
        }
        sequence += 1;
        client
            .send_input(InputCommand {
                session: host.identity,
                epoch: dead.physics.epoch,
                sequence,
                controls: Controls::default(),
                history: Vec::new(),
            })
            .unwrap();
        for _ in 0..12 {
            pump(&mut host, &mut client, &mut session);
            session.step(1.0 / 60.0);
        }
    }
    assert_eq!(
        session.world.players[caster]
            .as_ref()
            .unwrap()
            .conjure
            .queue
            .len(),
        2
    );
    sequence += 1;
    client
        .send_input(InputCommand {
            session: host.identity,
            epoch: dead.physics.epoch,
            sequence,
            controls: Controls {
                boost_down: true,
                ..Default::default()
            },
            history: Vec::new(),
        })
        .unwrap();
    for _ in 0..90 {
        pump(&mut host, &mut client, &mut session);
        session.step(1.0 / 60.0);
        if session.world.ability_effects.ressing(0) {
            break;
        }
    }
    assert!(session.world.ability_effects.ressing(0));
    assert!(
        session.dead(),
        "accepted input reserves a player before the timed spawn"
    );
    for _ in 0..30 {
        pump(&mut host, &mut client, &mut session);
        session.step(1.0 / 60.0);
        if !session.dead() {
            break;
        }
    }
    assert!(!session.dead());
    assert_eq!(session.avatar().motion.state, AvatarState::Resurrection);
    let mut revived = None;
    for _ in 0..30 {
        let frame = pump(&mut host, &mut client, &mut session);
        let victim = frame.players.iter().find(|p| p.slot == 0).unwrap();
        if !victim.dead {
            revived = Some(victim.clone());
            break;
        }
    }
    let victim = revived.expect("revival must reach the client over UDP");
    assert_eq!(victim.handle, original.handle);
    assert_eq!(
        victim.items, original.items,
        "server restores robe starting equipment"
    );
    assert_eq!(victim.attached_slots, original.attached_slots);
    assert_eq!(victim.unlocked_magicks, unlocks);
    assert_eq!(
        victim.animation,
        magicka_runtime::character_motion::animation_index("revive").unwrap()
    );
    let max = session.avatar().character.definition.max_hit_points;
    assert!((victim.health - max * 0.1).abs() < 1e-3);
}
#[test]
fn dedicated_respawn_restores_robe_equipment_and_retains_unlocks_on_server() {
    let (mut session, level) = session();
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        true,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    assert!(host.frame(&session).unwrap().players.is_empty());
    session.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .dead = true;
    session.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 0.;
    let mut client = LoopbackClient::connect(host.transport.address()).unwrap();
    pump(&mut host, &mut client, &mut session);
    assert_eq!(client.player_slot(), Some(0));
    assert!(
        !session.avatar().dead(),
        "the first client must receive a fresh server-spawned player"
    );
    let profile = PlayerProfile {
        name: "Named wizard".into(),
        colour: [0.2, 0.4, 0.8],
        ..Default::default()
    };
    session.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 123.;
    client
        .send_profile(ProfileRequest {
            profile_id: None,
            sequence: 1,
            profile: profile.clone(),
        })
        .unwrap();
    for _ in 0..40 {
        pump(&mut host, &mut client, &mut session);
    }
    assert_eq!(host.frame(&session).unwrap().players[0].profile, profile);
    assert_eq!(
        session.avatar().character.combat.health.hit_points,
        123.,
        "renaming cannot heal"
    );
    let starting_items = host.frame(&session).unwrap().players[0].items.clone();
    let weapon = starting_items[0].clone();
    session
        .set_player_loadout(0, "Wizard", "staff_heirloom_grandfather_clock", &weapon)
        .unwrap();
    session.unlocked_magicks[0] |= 1 << 3;
    host.capture(&session);
    let before = host.frame(&session).unwrap();
    // The debug respawn is the host's choice (it is off by default).
    host.set_lobby(magicka_network::lobby::LobbySettings {
        debug_respawn: true,
        ..host.lobby()
    })
    .unwrap();
    let request = |sequence| RespawnRequest {
        session: before.physics.session,
        epoch: before.physics.epoch,
        sequence,
    };
    // A living client cannot request healing/teleportation.
    session.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 123.;
    client.send_respawn(request(1)).unwrap();
    for _ in 0..20 {
        pump(&mut host, &mut client, &mut session);
    }
    assert_eq!(session.avatar().character.combat.health.hit_points, 123.);
    session.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .dead = true;
    session.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 0.;
    client.send_respawn(request(2)).unwrap();
    for _ in 0..20 {
        pump(&mut host, &mut client, &mut session);
    }
    assert!(!session.avatar().dead());
    let after = host.frame(&session).unwrap();
    assert_eq!(after.players[0].items, starting_items);
    assert_eq!(after.players[0].profile, profile);
    assert_eq!(
        after.players[0].unlocked_magicks,
        before.players[0].unlocked_magicks
    );
    assert!(session.avatar().character.combat.health.hit_points > 123.);
    // Restart restores checkpoint geometry, robe defaults and the server's unlocks.
    host.restart(&mut session).unwrap();
    let restarted = host.frame(&session).unwrap();
    assert_eq!(restarted.players[0].items, starting_items);
    assert_eq!(restarted.players[0].profile, profile);
    assert_eq!(
        restarted.players[0].unlocked_magicks,
        before.players[0].unlocked_magicks
    );
    assert!(restarted.physics.epoch > before.physics.epoch);
    session.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .dead = true;
    client.send_respawn(request(3)).unwrap();
    for _ in 0..20 {
        pump(&mut host, &mut client, &mut session);
    }
    assert!(
        session.avatar().dead(),
        "old epoch must not respawn a player"
    );
}
#[test]
fn playable_host_keeps_local_slot_and_each_remote_inventory() {
    let (mut session, level) = session();
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    let mut client = LoopbackClient::connect(host.transport.address()).unwrap();
    pump(&mut host, &mut client, &mut session);
    assert_eq!(client.player_slot(), Some(1));
    let starting_items = host.frame(&session).unwrap().players[1].items.clone();
    let weapon = starting_items[0].clone();
    session
        .set_player_loadout(1, "Wizard", "staff_heirloom_grandfather_clock", &weapon)
        .unwrap();
    session.unlocked_magicks[1] = 1 << 7;
    host.capture(&session);
    let before = host.frame(&session).unwrap();
    host.respawn_player(1, &mut session).unwrap();
    let after = host.frame(&session).unwrap();
    assert_eq!(after.players[0].items, before.players[0].items);
    assert_eq!(after.players[1].items, starting_items);
    assert_eq!(after.players[1].unlocked_magicks, 1 << 7);
}

#[test]
fn expired_owner_is_removed_and_replacement_starts_fresh() {
    use magicka_network::{
        control::{Controls, InputCommand},
        transport::PLAYER_LEASE,
    };
    use std::time::{Duration, Instant};
    for dedicated in [false, true] {
        let (mut session, level) = session();
        session.disable_scripts();
        session.unlocked_magicks = [1 << 4; 4].into();
        let mut host = HostedSession::bind(
            "127.0.0.1:0".parse().unwrap(),
            &level,
            dedicated,
            PlayerProfile::default(),
            &session,
        )
        .unwrap();
        let mut departed = LoopbackClient::connect(host.transport.address()).unwrap();
        pump(&mut host, &mut departed, &mut session);
        let slot = departed.player_slot().unwrap();
        let mut observer = LoopbackClient::connect(host.transport.address()).unwrap();
        pump(&mut host, &mut observer, &mut session);
        departed
            .send_profile(ProfileRequest {
                profile_id: None,
                sequence: 90,
                profile: PlayerProfile {
                    name: "Departed owner".into(),
                    ..Default::default()
                },
            })
            .unwrap();
        let before = host.frame(&session).unwrap();
        let input = |sequence, index| {
            let mut controls = Controls::default();
            controls.elements[index] = true;
            InputCommand {
                session: before.physics.session,
                epoch: before.physics.epoch,
                sequence,
                controls,
                history: Vec::new(),
            }
        };
        departed.send_input(input(90, 0)).unwrap();
        for _ in 0..40 {
            pump(&mut host, &mut departed, &mut session);
        }
        let default_items = before
            .players
            .iter()
            .find(|p| p.slot == slot)
            .unwrap()
            .items
            .clone();
        session
            .set_player_loadout(
                slot,
                "Wizard",
                "staff_heirloom_grandfather_clock",
                &default_items[0],
            )
            .unwrap();
        session.unlocked_magicks[slot] = 1 << 7;
        let old = session.world.players[slot].as_ref().unwrap();
        let old_handle = old.handle;
        let old_body = old.body;
        let observer_slot = observer.player_slot().unwrap();
        let observer_handle = session.world.players[observer_slot]
            .as_ref()
            .unwrap()
            .handle;
        session.world.players[slot]
            .as_mut()
            .unwrap()
            .character
            .combat
            .dead = true;
        session.world.players[slot]
            .as_mut()
            .unwrap()
            .character
            .combat
            .health
            .hit_points = 0.0;
        session.world.dead_age[slot] = 1.0;
        assert!(session.world.revive_specific(slot, [50.0, 0.0, 50.0]));
        session.world.ability_effects.haste(old_handle, None);
        session.world.ability_effects.haste(observer_handle, None);
        // Drain departed packets, then advance only the host clock: no wall-clock sleep.
        for _ in 0..40 {
            pump(&mut host, &mut observer, &mut session);
        }
        let now = Instant::now() + PLAYER_LEASE + Duration::from_secs(1);
        // The live observer's heartbeat arrives at the advanced host clock.
        observer.heartbeat_now();
        observer.update();
        host.poll_at(&mut session, now).unwrap();
        assert!(!host.transport.player_slots().contains(&slot));
        assert!(host
            .transport
            .player_slots()
            .contains(&observer.player_slot().unwrap()));
        assert!(host
            .frame(&session)
            .unwrap()
            .players
            .iter()
            .all(|p| p.slot != slot));
        assert!(!session.world.physics.bodies[old_body].active);
        assert!(!session.world.ability_effects.ressing(slot));
        assert!(session
            .world
            .ability_effects
            .effects
            .iter()
            .all(|effect| match effect {
                magicka_sim::magicks::AbilityEffect::Revive(r) => r.revivee != slot,
                magicka_sim::magicks::AbilityEffect::Haste(h) => h.owner != old_handle,
                magicka_sim::magicks::AbilityEffect::Grease(g) => g.owner != old_handle,
                magicka_sim::magicks::AbilityEffect::Performance(p) => p.owner != old_handle,
                magicka_sim::magicks::AbilityEffect::Judgement { owner, .. } => {
                    *owner != old_handle
                }
                magicka_sim::magicks::AbilityEffect::Napalm(_)
                | magicka_sim::magicks::AbilityEffect::MeteorShower(_)
                | magicka_sim::magicks::AbilityEffect::Amalgameddon(_) => true,
                magicka_sim::magicks::AbilityEffect::TimeWarp(_)
                | magicka_sim::magicks::AbilityEffect::Rain(_)
                | magicka_sim::magicks::AbilityEffect::Confuse(_)
                | magicka_sim::magicks::AbilityEffect::Thunderstorm(_)
                | magicka_sim::magicks::AbilityEffect::Blizzard(_)
                | magicka_sim::magicks::AbilityEffect::Conflagration { .. } => true,
            }));
        assert!(
            session
                .world
                .ability_effects
                .effects
                .iter()
                .any(|effect| matches!(effect,
            magicka_sim::magicks::AbilityEffect::Haste(h) if h.owner == observer_handle)),
            "departure cannot cancel another owner's effect"
        );
        assert!(
            session
                .world
                .entities
                .iter()
                .find(|e| e.handle == old_handle)
                .unwrap()
                .removed
        );
        // The remaining observer keeps its ownership; expiring it leaves only a playable host.
        assert!(host.has_players());
        host.poll_at(&mut session, now + PLAYER_LEASE).unwrap();
        assert_eq!(host.has_players(), !dedicated);
        if dedicated {
            assert!(host.frame(&session).unwrap().players.is_empty());
        }
        // Camera/checkpoint internals still have a dormant slot-zero avatar.
        session.step(1. / 60.);
        if dedicated {
            assert!(!session.avatar().playing);
            assert_eq!(session.world.dead_age[0], 0.0);
            assert!(!session.world.revive_specific(0, [0.0; 3]));
            assert!(
                !magicka_runtime::scene_flow::SceneFlowHost::players(&session.world)[0].playing
            );
            assert!(
                !magicka_runtime::world_condition::WorldConditionFacts::players(&session.world)[0]
                    .playing
            );
        }
        let mut replacement = LoopbackClient::connect(host.transport.address()).unwrap();
        pump(&mut host, &mut replacement, &mut session);
        assert_eq!(replacement.player_slot(), Some(slot));
        assert_ne!(
            session.world.players[slot].as_ref().unwrap().handle,
            old_handle
        );
        assert!(session.world.players[slot].as_ref().unwrap().playing);
        for _ in 0..30 {
            session.step(1.0 / 60.0);
            pump(&mut host, &mut replacement, &mut session);
        }
        let player = session.world.players[slot].as_ref().unwrap();
        assert!(!player.dead());
        assert_ne!(
            player.motion.state,
            magicka_runtime::avatar::AvatarState::Resurrection,
            "old Revive cannot spawn the replacement owner"
        );
        assert_eq!(
            player.character.combat.health.hit_points,
            player.character.combat.health.max_hit_points
        );
        assert_eq!(session.unlocked_magicks[slot], 1 << 4);
        assert_eq!(
            host.frame(&session)
                .unwrap()
                .players
                .iter()
                .find(|p| p.slot == slot)
                .unwrap()
                .items,
            default_items
        );
        replacement
            .send_profile(ProfileRequest {
                profile_id: None,
                sequence: 1,
                profile: PlayerProfile {
                    name: "Replacement".into(),
                    ..Default::default()
                },
            })
            .unwrap();
        replacement.send_input(input(1, 1)).unwrap();
        for _ in 0..40 {
            pump(&mut host, &mut replacement, &mut session);
        }
        assert_eq!(
            host.frame(&session)
                .unwrap()
                .players
                .iter()
                .find(|p| p.slot == slot)
                .unwrap()
                .profile
                .name,
            "Replacement"
        );
        assert_eq!(
            session.world.players[slot]
                .as_ref()
                .unwrap()
                .conjure
                .queue
                .len(),
            1
        );
        // A resumed old socket cannot rejoin or control the replacement's slot.
        departed
            .send_profile(ProfileRequest {
                profile_id: None,
                sequence: 91,
                profile: PlayerProfile {
                    name: "Stale socket".into(),
                    ..Default::default()
                },
            })
            .unwrap();
        departed.send_input(input(91, 2)).unwrap();
        for _ in 0..40 {
            departed.update();
            pump(&mut host, &mut replacement, &mut session);
        }
        assert_eq!(
            host.frame(&session)
                .unwrap()
                .players
                .iter()
                .find(|p| p.slot == slot)
                .unwrap()
                .profile
                .name,
            "Replacement"
        );
        assert_eq!(
            session.world.players[slot]
                .as_ref()
                .unwrap()
                .conjure
                .queue
                .len(),
            1
        );
    }
}
