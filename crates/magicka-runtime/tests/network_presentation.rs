use magicka_network::{session::*, transport::LoopbackClient};
use magicka_runtime::{
    network_session::{mirror_player_spell_state, HostedSession, RemoteEffects, RemotePlayers},
    particles::EffectSystem,
    play_session::PlaySession,
    scene_loading::custom_hash,
};
use std::path::PathBuf;

fn session() -> (PlaySession, String) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let level = root
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap();
    let mut session = PlaySession::load(
        &magicka_content::retail_content_dir(),
        level.to_str().unwrap(),
        "Wizard",
    )
    .unwrap();
    session.step(1. / 60.);
    (session, level.to_string_lossy().into_owned())
}

fn charged_items() -> [magicka_sim::weapon_spells::WeaponSpells; 2] {
    use magicka_sim::{
        spells::{Elements as E, Spell},
        weapon_spells::WeaponSpells,
    };
    let mut weapon = WeaponSpells::default();
    weapon.add(
        Spell::default_for_list(&[E::FIRE, E::FIRE, E::ARCANE]),
        false,
        &mut vec![],
    );
    assert_eq!(weapon.queue()[0].get(E::FIRE), 2. / 3.);
    let mut staff = WeaponSpells::default();
    for _ in 0..5 {
        staff.add(Spell::default_for(E::WATER), false, &mut vec![]);
    }
    staff.add(
        Spell::default_for_list(&[E::COLD, E::COLD, E::COLD]),
        false,
        &mut vec![],
    );
    assert!(staff.queue()[4].total_magnitude() > 1.);
    [weapon, staff]
}

#[test]
fn two_clients_receive_host_npc_pose_health_death_and_effect_lifetimes() {
    let (mut session, level) = session();
    let npc = session.spawn_npc("Goblin_pirate", "idle", 7.).unwrap();
    for _ in 0..10 {
        session.step(1. / 60.);
    }
    let mut weighted = magicka_sim::spells::Spell::default_for(magicka_sim::spells::Elements::ICE);
    weighted.mag[8] = 2.5;
    let expected_queue = vec![
        weighted,
        magicka_sim::spells::Spell::magick(magicka_sim::spells::MagickType::Rain),
    ];
    session.world.players[0].as_mut().unwrap().conjure.queue = expected_queue.clone();
    let expected_items = charged_items();
    for (slot, spells) in expected_items.iter().enumerate() {
        session.world.players[0]
            .as_mut()
            .unwrap()
            .items
            .iter_mut()
            .find(|(s, _)| *s == slot as i32)
            .unwrap()
            .1
            .spells = spells.clone();
    }
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    let mut clients = [
        LoopbackClient::connect(host.transport.address()).unwrap(),
        LoopbackClient::connect(host.transport.address()).unwrap(),
    ];
    let torch = custom_hash("ambient_fire_torch");
    let mut effect = session
        .effects
        .start_at(torch, [2., 4., 6.], [0., 0., -1.])
        .unwrap();
    session.effects.update(0.1);
    let expected = host.frame(&session).unwrap();
    let mut bad = expected.clone();
    bad.npcs[0].transform[12] = f32::NAN;
    assert!(!bad.valid(), "nonfinite NPC pose passed the wire boundary");
    let mut bad = expected.clone();
    bad.npcs.push(bad.npcs[0].clone());
    assert!(
        !bad.valid(),
        "duplicate NPC handle passed the wire boundary"
    );
    let mut bad = expected.clone();
    bad.effects.push(bad.effects[0].clone());
    assert!(
        !bad.valid(),
        "duplicate effect slot passed the wire boundary"
    );
    let mut bad = expected.clone();
    bad.effects[0].started_ms = (bad.time * 1000.0) as i64 + 10_000;
    assert!(
        !bad.valid(),
        "an emitter starting in the future passed the wire boundary"
    );
    let state = expected.npcs.iter().find(|n| n.handle == npc).unwrap();
    assert_eq!(state.template.to_ascii_lowercase(), "goblin_pirate");
    assert!(state.clip.is_some());
    let body = &session.world.physics.bodies[session.world.npcs[&npc].body];
    assert_eq!(&state.transform[12..15], &body.feet());
    assert!(expected
        .effects
        .iter()
        .any(|e| e.instance == effect.hash && expected.time - e.started_ms as f64 / 1000.0 >= 0.1));
    let receive =
        |host: &mut HostedSession, clients: &mut [LoopbackClient; 2], session: &mut PlaySession| {
            let expected = host.frame(session).unwrap();
            let mut received: [Option<GameFrame>; 2] = [None, None];
            for _ in 0..400 {
                for (i, client) in clients.iter_mut().enumerate() {
                    client.update();
                    if let Some(frame) = client.take_game_frames().into_iter().last() {
                        let player_storage = |frame: &GameFrame| {
                            frame
                                .players
                                .iter()
                                .find(|p| p.slot == 0)
                                .map(|p| (p.queue.clone(), p.item_queues.clone()))
                        };
                        if frame.npcs == expected.npcs
                            && frame.effects == expected.effects
                            && player_storage(&frame) == player_storage(&expected)
                        {
                            received[i] = Some(frame);
                        }
                    }
                }
                host.poll(session).unwrap();
                host.publish(session).unwrap();
                if received.iter().all(Option::is_some) {
                    return received.map(Option::unwrap);
                }
            }
            panic!("both presentation snapshots did not arrive");
        };
    for frame in receive(&mut host, &mut clients, &mut session) {
        let queue = &frame.players.iter().find(|p| p.slot == 0).unwrap().queue;
        assert_eq!(
            queue
                .iter()
                .map(QueuedSpell::spell)
                .collect::<Option<Vec<_>>>(),
            Some(expected_queue.clone())
        );
        let mut bad = frame.clone();
        bad.players[0].queue[0].magnitudes[8] = f32::NAN;
        assert!(!bad.valid());
        assert_eq!(frame.npcs, expected.npcs);
        assert_eq!(frame.effects, expected.effects);
        let player = frame.players.iter().find(|p| p.slot == 0).unwrap();
        for (queue, expected) in player.item_queues.iter().zip(&expected_items) {
            assert_eq!(
                queue
                    .iter()
                    .map(QueuedSpell::spell)
                    .collect::<Option<Vec<_>>>(),
                Some(expected.queue().to_vec())
            );
        }
        let mut bad = frame.clone();
        bad.players[0].item_queues[0][0].magnitudes[3] = f32::NAN;
        assert!(!bad.valid());
        let mut bad = frame.clone();
        let extra = bad.players[0].item_queues[0][0].clone();
        bad.players[0].item_queues[1].push(extra);
        assert!(
            !bad.valid(),
            "item storage capacity is checked at the wire boundary"
        );
    }
    for (_, item) in &mut session.world.players[0].as_mut().unwrap().items {
        item.spells.retrieve(&mut vec![]);
    }
    for frame in receive(&mut host, &mut clients, &mut session) {
        assert!(frame
            .players
            .iter()
            .find(|p| p.slot == 0)
            .unwrap()
            .item_queues
            .iter()
            .all(Vec::is_empty));
    }
    session
        .world
        .npcs
        .get_mut(&npc)
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 17.;
    session
        .world
        .npcs
        .get_mut(&npc)
        .unwrap()
        .character
        .combat
        .dead = true;
    session.effects.stop(&mut effect);
    let old_instance = expected
        .effects
        .iter()
        .find(|e| e.definition == torch)
        .unwrap()
        .instance;
    let replacement = session
        .effects
        .start_at(torch, [8., 4., 6.], [0., 0., -1.])
        .unwrap();
    assert_ne!(replacement.hash, old_instance);
    // Drain the previous full snapshot; the next set must replace pool lifetimes.
    for client in &mut clients {
        client.update();
        client.take_game_frames();
    }
    for frame in receive(&mut host, &mut clients, &mut session) {
        let state = frame.npcs.iter().find(|n| n.handle == npc).unwrap();
        assert!(state.dead);
        assert_eq!(state.health, 17.);
        assert!(!frame.effects.iter().any(|e| e.instance == old_instance));
        assert!(frame.effects.iter().any(|e| e.instance == replacement.hash));
    }
    session.world.overkilled.insert(npc);
    assert!(!host
        .frame(&session)
        .unwrap()
        .npcs
        .iter()
        .any(|n| n.handle == npc));
}

#[test]
fn remote_effects_emit_once_per_instance_retain_tails_and_reset_on_epoch() {
    let (mut session, _) = session();
    let mut remote = RemoteEffects::default();
    let mut effects = EffectSystem::new(session.effects.library.clone());
    let hash = custom_hash("ambient_fire_torch");
    let mut host_ref = session
        .effects
        .start_at(hash, [1., 2., 3.], [0., 0., -1.])
        .unwrap();
    let states = || {
        vec![VisualEffectState {
            slot: host_ref.id as u16,
            instance: host_ref.hash,
            definition: hash,
            transform: magicka_content::skinned::IDENTITY,
            started_ms: 0,
        }]
    };
    let first = states();
    remote.reconcile(1, &first, 5.0, &mut effects).unwrap();
    assert!(
        effects.particles.active().is_empty(),
        "late join must not replay five seconds of births"
    );
    effects.update(0.1);
    effects.update_particles(0.1);
    assert!(!effects.particles.active().is_empty());
    remote.reconcile(1, &first, 5.1, &mut effects).unwrap();
    assert_eq!(
        effects.active_effects_by_hash(hash),
        1,
        "snapshot duplicated emitter"
    );
    remote.reconcile(1, &[], 5.1, &mut effects).unwrap();
    assert_eq!(effects.active_effects_by_hash(hash), 0);
    assert!(
        !effects.particles.active().is_empty(),
        "removal erased particle tails"
    );
    session.effects.stop(&mut host_ref);
    let new_ref = session
        .effects
        .start_at(hash, [1., 2., 3.], [0., 0., -1.])
        .unwrap();
    let mut reused = first;
    reused[0].instance = new_ref.hash;
    remote.reconcile(1, &reused, 5.1, &mut effects).unwrap();
    assert_eq!(effects.active_effects_by_hash(hash), 1);
    remote.reconcile(2, &[], 5.1, &mut effects).unwrap();
    assert_eq!(effects.active_effects(), 0);
    assert!(
        effects.particles.active().is_empty(),
        "old scene particles survived epoch reset"
    );
}

#[test]
fn remote_mirrors_follow_avatar_lifetimes_and_remove_omitted_equipment_owners() {
    for own_slot in [0, 1] {
        let (mut session, level) = session();
        session.disable_scripts();
        let host = HostedSession::bind(
            "127.0.0.1:0".parse().unwrap(),
            &level,
            false,
            PlayerProfile::default(),
            &session,
        )
        .unwrap();
        let mut frame = host.frame(&session).unwrap();
        let mut remote = frame.players[0].clone();
        remote.slot = 1;
        remote.handle = 5001;
        frame.players.push(remote);
        let remote_slot = 1 - own_slot;
        let view_slot = 1;
        let own_handle = session.avatar().handle;
        let own_body = session.avatar().body;
        let mut mirrors = RemotePlayers::default();
        mirrors
            .reconcile(&frame, Some(own_slot), &mut session)
            .unwrap();
        let first = session.world.players[view_slot].as_ref().unwrap().handle;
        let first_body = session.world.players[view_slot].as_ref().unwrap().body;
        session.world.players[view_slot]
            .as_mut()
            .unwrap()
            .character
            .combat
            .dead = true;
        session.unlocked_magicks[view_slot] = 1 << 7;
        // Duplicate snapshots keep an existing mirror and its controller.
        mirrors
            .reconcile(&frame, Some(own_slot), &mut session)
            .unwrap();
        assert_eq!(
            session.world.players[view_slot].as_ref().unwrap().handle,
            first
        );
        assert!(session.world.players[view_slot].as_ref().unwrap().dead());
        // Same slot and same robe, new host handle: recreate instead of retaining stale state.
        frame
            .players
            .iter_mut()
            .find(|p| p.slot == remote_slot)
            .unwrap()
            .handle = 5003;
        mirrors
            .reconcile(&frame, Some(own_slot), &mut session)
            .unwrap();
        let second = session.world.players[view_slot].as_ref().unwrap().handle;
        assert_ne!(first, second);
        assert!(!session.world.players[view_slot].as_ref().unwrap().dead());
        assert_eq!(session.unlocked_magicks[view_slot], 0);
        assert!(!session.world.physics.bodies[first_body].active);
        assert!(
            session
                .world
                .entities
                .iter()
                .find(|e| e.handle == first)
                .unwrap()
                .removed
        );
        // A scene restart establishes new lifetimes even if host handles happen to match.
        frame.physics.epoch += 1;
        mirrors
            .reconcile(&frame, Some(own_slot), &mut session)
            .unwrap();
        let third = session.world.players[view_slot].as_ref().unwrap().handle;
        assert_ne!(second, third);
        let third_body = session.world.players[view_slot].as_ref().unwrap().body;
        // Full-snapshot omission removes the mirror, so equipment traversal has no owner.
        frame.players.retain(|p| p.slot == own_slot);
        mirrors
            .reconcile(&frame, Some(own_slot), &mut session)
            .unwrap();
        assert!(session.world.players[view_slot].is_none());
        assert!(!session.world.physics.bodies[third_body].active);
        // Invalid snapshots must leave the local player intact.
        let mut bad = frame.clone();
        bad.players.clear();
        assert!(mirrors
            .reconcile(&bad, Some(own_slot), &mut session)
            .is_err());
        assert_eq!(session.avatar().handle, own_handle);
        assert_eq!(session.avatar().body, own_body);
    }
}

#[test]
fn remote_queue_mirror_retains_magnitudes_magicks_and_authoritative_clears() {
    use magicka_sim::spells::{Elements as E, MagickType, Spell};
    let (mut session, level) = session();
    let host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    let mut frame = host.frame(&session).unwrap();
    let mut remote = frame.players[0].clone();
    remote.slot = 1;
    remote.handle = 5001;
    let mut spell = Spell::default_for(E::FIRE | E::STEAM);
    spell.mag[3] = 1.25;
    spell.mag[9] = 3.75;
    let spells = vec![spell, Spell::magick(MagickType::Rain)];
    remote.queue = spells.iter().map(QueuedSpell::from).collect();
    remote.unlocked_magicks = 1 << 7;
    frame.players.push(remote);
    let mut mirrors = RemotePlayers::default();
    mirrors.reconcile(&frame, Some(0), &mut session).unwrap();
    assert_eq!(
        session.world.players[1].as_ref().unwrap().conjure.queue,
        spells
    );
    assert_eq!(session.unlocked_magicks[1], 1 << 7);
    assert!(session.avatar().conjure.queue.is_empty());
    let handle = session.world.players[1].as_ref().unwrap().handle;
    frame.players[1].queue.clear();
    frame.players[1].unlocked_magicks = 0;
    mirrors.reconcile(&frame, Some(0), &mut session).unwrap();
    assert_eq!(session.world.players[1].as_ref().unwrap().handle, handle);
    assert!(session.world.players[1]
        .as_ref()
        .unwrap()
        .conjure
        .queue
        .is_empty());
    assert_eq!(session.unlocked_magicks[1], 0);
}

#[test]
fn owned_and_remote_item_storage_preserves_values_and_authoritative_clears() {
    let (mut session, level) = session();
    let host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    let mut frame = host.frame(&session).unwrap();
    let stored = charged_items();
    frame.players[0].item_queues = stored
        .each_ref()
        .map(|s| s.queue().iter().map(QueuedSpell::from).collect());
    let mut remote = frame.players[0].clone();
    remote.slot = 1;
    remote.handle = 5001;
    frame.players.push(remote);
    let mut mirrors = RemotePlayers::default();
    mirrors.reconcile(&frame, Some(0), &mut session).unwrap();
    let effect_count = session.effects.active_effects();
    let events = session.events.len();
    let avatar_events = session.avatar().events.len();
    let health = session.avatar().character.combat.health.hit_points;
    mirror_player_spell_state(&mut session, 0, &frame.players[0]).unwrap();
    let assert_storage = |session: &PlaySession, slot: usize| {
        let avatar = session.world.players[slot].as_ref().unwrap();
        for (item_slot, expected) in stored.iter().enumerate() {
            let item = &avatar
                .items
                .iter()
                .find(|(s, _)| *s == item_slot as i32)
                .unwrap()
                .1;
            assert_eq!(item.spells.queue(), expected.queue());
            assert_eq!(item.spells.peek(), expected.peek());
            assert!(item.spells.charged());
        }
        assert!(avatar.current_spell.is_none());
    };
    assert_storage(&session, 0);
    assert_storage(&session, 1);
    assert_eq!(session.effects.active_effects(), effect_count);
    assert_eq!(session.events.len(), events);
    assert_eq!(session.avatar().events.len(), avatar_events);
    assert_eq!(session.avatar().character.combat.health.hit_points, health);
    // Invalid staff data must not clear either previously accepted item queue.
    let mut bad = frame.players[0].clone();
    bad.item_queues[0].clear();
    bad.item_queues[1][0].magnitudes[0] = f32::INFINITY;
    assert!(mirror_player_spell_state(&mut session, 0, &bad).is_err());
    assert_storage(&session, 0);
    // Rebuilding the owned loadout must be followed by exact snapshot restoration.
    let state = &frame.players[0];
    session
        .set_player_loadout(0, &state.profile.robe, &state.items[1], &state.items[0])
        .unwrap();
    mirror_player_spell_state(&mut session, 0, state).unwrap();
    assert_storage(&session, 0);
    for player in &mut frame.players {
        player.item_queues.iter_mut().for_each(Vec::clear);
    }
    mirrors.reconcile(&frame, Some(0), &mut session).unwrap();
    mirror_player_spell_state(&mut session, 0, &frame.players[0]).unwrap();
    for avatar in session.world.players.iter().flatten() {
        assert!(avatar.items.iter().all(|(_, item)| !item.spells.charged()));
        assert!(avatar.current_spell.is_none());
    }
}

#[test]
fn attachment_mirrors_follow_server_slots_independently_of_health() {
    use magicka_sim::spells::{Elements, Spell};
    let (mut session, level) = session();
    let host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    let mut frame = host.frame(&session).unwrap();
    frame.players[0].health = 0.0;
    frame.players[0].dead = true;
    let mut remote = frame.players[0].clone();
    remote.slot = 1;
    remote.handle = 5001;
    remote.health = 100.0;
    remote.dead = false;
    remote.attached_slots = vec![0];
    frame.players.push(remote);
    let mut mirrors = RemotePlayers::default();
    mirrors.reconcile(&frame, Some(0), &mut session).unwrap();
    mirror_player_spell_state(&mut session, 0, &frame.players[0]).unwrap();
    assert!(
        session.avatar().items.iter().all(|(_, item)| item.attached),
        "health/death cannot override the transmitted attachment state"
    );
    let avatar = session.world.players[1].as_ref().unwrap();
    assert!(
        avatar
            .items
            .iter()
            .find(|(slot, _)| *slot == 0)
            .unwrap()
            .1
            .attached
    );
    assert!(
        !avatar
            .items
            .iter()
            .find(|(slot, _)| *slot == 1)
            .unwrap()
            .1
            .attached
    );
    // Invalid/unknown slots must not partially change accepted storage or flags.
    for slots in [vec![-1], vec![0, 0], vec![1, 0], vec![99]] {
        let mut bad = frame.players[0].clone();
        bad.attached_slots = slots;
        bad.queue
            .push(QueuedSpell::from(&Spell::default_for(Elements::FIRE)));
        assert!(mirror_player_spell_state(&mut session, 0, &bad).is_err());
        assert!(session.avatar().conjure.queue.is_empty());
        assert!(session.avatar().items.iter().all(|(_, item)| item.attached));
    }
    frame.players[0].attached_slots.clear();
    frame.players[0].health = 100.0;
    frame.players[0].dead = false;
    mirror_player_spell_state(&mut session, 0, &frame.players[0]).unwrap();
    assert!(session
        .avatar()
        .items
        .iter()
        .all(|(_, item)| !item.attached));
    frame.players[1].profile.robe = "Wizard_Samurai".into();
    let content = magicka_content::retail_content_dir();
    let template =
        magicka_runtime::play_content::load_character(&content, "Wizard_Samurai").unwrap();
    frame.players[1].attached_slots =
        magicka_runtime::play_content::avatar_items(&content, &template)
            .iter()
            .map(|(slot, _)| *slot)
            .collect();
    frame.players[1].attached_slots.sort_unstable();
    mirrors.reconcile(&frame, Some(0), &mut session).unwrap();
    let avatar = session.world.players[1].as_ref().unwrap();
    assert_eq!(
        avatar.items.len(),
        3,
        "Samurai fixture has a robe accessory"
    );
    assert!(avatar.items.iter().all(|(_, item)| item.attached));
    frame.players[1].attached_slots.clear();
    mirrors.reconcile(&frame, Some(0), &mut session).unwrap();
    assert!(
        session.world.players[1]
            .as_ref()
            .unwrap()
            .items
            .iter()
            .all(|(_, item)| !item.attached),
        "death release also removes the robe accessory on the client"
    );
}
