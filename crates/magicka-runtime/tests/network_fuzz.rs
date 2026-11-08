//! Hostile but well-formed requests through the real transport into the real
//! game: whatever a client sends that passes validation must not panic the host
//! or stop it publishing (`research/multiplayer-security.md`).
//!
//! `MAGICKA_FUZZ_TICKS` and `MAGICKA_FUZZ_SEED` lengthen or vary a run.
use magicka_network::{
    control::{Controls, InputStream, PickupRequest, PickupTarget},
    lobby::{DmCommand, DmRequest, LobbySettings, Role, RoleRequest},
    session::{PlayerProfile, ProfileRequest, RespawnRequest},
    transport::LoopbackClient,
};
use magicka_runtime::{network_session::HostedSession, play_session::PlaySession};
use std::path::PathBuf;

struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, one_in: u64) -> bool {
        self.below(one_in) == 0
    }
    fn unit(&mut self) -> f32 {
        (self.below(2001) as f32 - 1000.0) / 1000.0
    }
    /// Usually near the action, sometimes the largest value validation allows.
    fn coordinate(&mut self) -> f32 {
        match self.below(8) {
            0 => 1_000_000.0,
            1 => -1_000_000.0,
            2 => 0.0,
            _ => self.unit() * 40.0,
        }
    }
    fn point(&mut self) -> [f32; 3] {
        [self.coordinate(), self.coordinate(), self.coordinate()]
    }
    fn flag(&mut self) -> bool {
        self.next() & 1 == 0
    }
    fn handle(&mut self, known: &[u16]) -> u16 {
        if !known.is_empty() && self.chance(2) {
            known[self.below(known.len() as u64) as usize]
        } else {
            self.next() as u16
        }
    }
    fn transform(&mut self) -> [f32; 16] {
        let mut m = [0.0; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[15] = 1.0;
        if self.chance(4) {
            // Any finite matrix passes validation: scaled, skewed, degenerate.
            for value in &mut m {
                *value = self.coordinate();
            }
        }
        let at = self.point();
        m[12..15].copy_from_slice(&at);
        m
    }
}

fn controls(r: &mut Random) -> Controls {
    Controls {
        aim: r.flag().then(|| r.point()),
        pad_movement: r.chance(3).then(|| [r.unit(), r.unit()]),
        pad_attack: r.chance(4),
        pad_block: r.chance(4),
        move_down: r.flag(),
        cast_down: r.chance(3),
        cast_self_down: r.chance(6),
        shift_down: r.chance(6),
        boost_down: r.chance(6),
        elements: std::array::from_fn(|_| r.chance(5)),
        magick_prev: r.chance(30),
        magick_next: r.chance(30),
        inventory: r.chance(30),
    }
}

fn dm_command(r: &mut Random, known: &[u16]) -> DmCommand {
    const TEMPLATES: [&str; 6] = [
        "Goblin_pirate",
        "Wizard",
        "no_such_template",
        "Vlad",
        "Cthulhu",
        "x",
    ];
    const SPAWNABLES: [&str; 8] = [
        "npc:Goblin_pirate",
        "npc:nope",
        "prop:bathball",
        "item:nope",
        "book:nope",
        "fx:nope",
        "npc:a/b/c",
        "item:weapon sword",
    ];
    // Removing people is tested elsewhere: here it would end the fuzz early.
    match r.below(10) {
        0 => DmCommand::SpawnNpc {
            template: TEMPLATES[r.below(6) as usize].into(),
            position: r.point(),
        },
        1 => DmCommand::Possess {
            npc: r.handle(known),
        },
        2 => DmCommand::Release,
        3 => DmCommand::Smite {
            handle: r.handle(known),
        },
        4 => DmCommand::Heal {
            handle: r.handle(known),
        },
        5 => DmCommand::SetHotJoin(r.flag()),
        6 => DmCommand::SetPlayerLimit(1 + r.below(256) as u16),
        7 => DmCommand::Spawn {
            spawnable: SPAWNABLES[r.below(8) as usize].into(),
            transform: r.transform(),
            order: r.flag().then(|| "attack".to_string()),
            movable: r.flag(),
        },
        8 => DmCommand::Move {
            handle: r.handle(known),
            transform: r.transform(),
        },
        _ => DmCommand::Remove {
            handle: r.handle(known),
        },
    }
}

/// Which part of a frame failed its check, for the report of a failed run.
fn diagnose(host: &HostedSession, session: &PlaySession) {
    let physics = session.physics_frame(host.identity, 1);
    println!(
        "physics frame: {:?}",
        physics.as_ref().map(|f| f.validate())
    );
    if let Ok(frame) = physics {
        for body in &frame.bodies {
            let norm: f32 = body.rotation.iter().map(|v| v * v).sum();
            if !body.position.iter().all(|v| v.is_finite()) || (norm - 1.0).abs() >= 0.01 {
                println!(
                    "bad body {}: {} rotation norm {norm} at {:?}",
                    body.handle, body.asset, body.position
                );
            }
        }
    }
    for (slot, player) in session.world.players.iter().enumerate() {
        if let Some(player) = player {
            let o = &session.world.physics.bodies[player.body].orientation;
            println!(
                "player {slot}: at {:?}, axes {:?} {:?} {:?}, health {}",
                session.world.physics.bodies[player.body].position,
                o.right,
                o.up,
                o.backward,
                player.character.combat.health.hit_points
            );
        }
    }
    println!(
        "last log: {:?}",
        session.world.log.iter().rev().take(5).collect::<Vec<_>>()
    );
    let world = magicka_runtime::network_world::world_presentation(session);
    let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
    for p in &world.pickups {
        if !finite(&p.transform)
            || matches!(&p.kind, magicka_network::world::PickupKind::Item { name } if name.is_empty() || name.contains(':') || name.contains("..") || name.starts_with(['/', '\\']))
        {
            println!("bad pickup {} {:?} {:?}", p.handle, p.kind, p.transform);
        }
    }
    for a in &world.animations {
        if !finite(&[a.time, a.start, a.end, a.speed, a.resting_timer]) {
            println!("bad animation {a:?}");
        }
    }
    let spells = &world.spells;
    println!(
        "counts: pickups {} animations {} beams {} bolts {} shields {} barriers {} mines {} blasts {} missiles {} avatars {} pulses {}",
        world.pickups.len(), world.animations.len(), spells.beams.len(), spells.bolts.len(), spells.shields.len(),
        spells.barriers.len(), spells.mines.len(), spells.arcane_blasts.len(), spells.missiles.len(), world.avatars.len(), world.pulses.len(),
    );
    for m in &spells.missiles {
        if m.model.is_empty()
            || m.model.contains(':')
            || m.model.contains("..")
            || m.model.starts_with(['/', '\\'])
            || !finite(&m.position)
            || !finite(&m.velocity)
        {
            println!(
                "bad missile {:?} {:?} {:?}",
                m.model, m.position, m.velocity
            );
        }
    }
    for (_, shield) in &spells.shields {
        if !finite(&shield.position) || !shield.radius.is_finite() {
            println!(
                "bad shield at {:?} radius {}",
                shield.position, shield.radius
            );
        }
    }
    for b in &spells.beams {
        if !finite(&b.position) || !finite(&b.direction) {
            println!("bad beam {:?} {:?}", b.position, b.direction);
        }
    }
}

fn run(seed: u64, ticks: u64) -> (u64, u64) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut session = PlaySession::load(
        &root.join("../../../../target/Magicka/Content"),
        "Havindr.lvl",
        "Wizard",
    )
    .unwrap();
    session.step(1. / 60.);
    let mut host = HostedSession::bind_lobby(
        "127.0.0.1:0".parse().unwrap(),
        "Havindr.lvl",
        false,
        PlayerProfile::default(),
        &session,
        LobbySettings {
            max_players: 8,
            dungeon_masters: true,
            invaders: true,
            ..Default::default()
        },
    )
    .unwrap();
    let address = host.transport.address();
    let mut players = vec![
        LoopbackClient::connect(address).unwrap(),
        LoopbackClient::connect(address).unwrap(),
    ];
    let mut gm = LoopbackClient::connect_as(address, Role::DungeonMaster, "GM").unwrap();
    let mut streams: Vec<InputStream> = vec![Default::default(), Default::default()];
    let mut invader_stream = InputStream::default();
    let mut invader = LoopbackClient::connect_as(address, Role::Invader, "Goblin").unwrap();
    let mut r = Random(seed | 1);
    let mut sequence = 0;
    let mut publish_errors = 0;
    let mut frames_seen = 0;
    let mut known: Vec<u16> = Vec::new();
    let mut identity = (0, 0);
    let mut last = None;
    for tick in 0..ticks {
        sequence += 1;
        if let Err(e) = host.poll(&mut session) {
            panic!("seed {seed} tick {tick}: the host stopped: {e}");
        }
        session.step(1. / 60.);
        if session.end_game.restart {
            host.restart(&mut session).unwrap();
        }
        host.capture(&session);
        if let Err(e) = host.publish(&session) {
            if publish_errors == 0 {
                println!("seed {seed} tick {tick}: first unpublishable frame: {e}; last request: {last:?}");
                diagnose(&host, &session);
            }
            publish_errors += 1;
        }
        for client in players.iter_mut().chain([&mut gm, &mut invader]) {
            client.update();
            for frame in client.take_game_frames() {
                frames_seen += 1;
                identity = (frame.physics.session, frame.physics.epoch);
                known = frame
                    .npcs
                    .iter()
                    .map(|n| n.handle)
                    .chain(frame.players.iter().map(|p| p.handle))
                    .chain(frame.physics.bodies.iter().map(|b| b.handle))
                    .collect();
            }
        }
        if identity.0 == 0 {
            continue;
        }
        for (client, stream) in players.iter_mut().zip(&mut streams) {
            let input = stream.next(identity.0, identity.1, controls(&mut r));
            client.send_input(input).ok();
            if r.chance(8) {
                client
                    .send_pickup(PickupRequest {
                        session: identity.0,
                        epoch: identity.1,
                        sequence,
                        target: PickupTarget {
                            handle: r.handle(&known),
                            generation: r.below(4),
                        },
                    })
                    .ok();
            }
            if r.chance(40) {
                client
                    .send_respawn(RespawnRequest {
                        session: identity.0,
                        epoch: identity.1,
                        sequence,
                    })
                    .ok();
            }
            if r.chance(120) {
                const ROBES: [&str; 3] = ["Wizard", "Wizard_Samurai", "wizard_no_such_robe"];
                client
                    .send_profile(ProfileRequest {
                        sequence,
                        profile: PlayerProfile {
                            robe: ROBES[r.below(3) as usize].into(),
                            ..Default::default()
                        },
                        profile_id: None,
                    })
                    .ok();
            }
            if r.chance(400) {
                let role = Role::ALL[r.below(4) as usize];
                client
                    .request_role(RoleRequest {
                        sequence,
                        role,
                        name: "Changed".into(),
                    })
                    .ok();
            }
        }
        invader
            .send_input(invader_stream.next(identity.0, identity.1, controls(&mut r)))
            .ok();
        if r.chance(3) {
            let command = dm_command(&mut r, &known);
            last = Some(command.clone());
            gm.send_dm(DmRequest { sequence, command }).ok();
        }
    }
    // What the run reached, so a pass can be told from a run that did nothing.
    let count = |text: &str| {
        session
            .world
            .log
            .iter()
            .filter(|l| l.contains(text))
            .count()
    };
    println!(
        "seed {seed}: {frames_seen} frames, {} NPCs, {} DM results logged, {} DM spawns, \
         {} seats, inputs accepted for slots {:?}",
        session.world.npcs.len(),
        count("dungeon master"),
        count("dungeon master: spawned"),
        host.transport.seats().len(),
        host.transport.player_slots(),
    );
    (frames_seen, publish_errors)
}

/// Found by this fuzz: one Move with a skewed matrix made every frame invalid.
#[test]
fn a_skewed_or_degenerate_move_leaves_a_character_publishable() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut session = PlaySession::load(
        &root.join("../../../../target/Magicka/Content"),
        "Havindr.lvl",
        "Wizard",
    )
    .unwrap();
    session.disable_scripts();
    session.step(1. / 60.);
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        "Havindr.lvl",
        false,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    let handle = session.world.players[0].as_ref().unwrap().handle;
    let mut skewed = [0.0; 16];
    skewed[..15].copy_from_slice(&[
        0.0, -13.0, -25.0, -1e6, -26.0, 0.0, -12.0, -11.0, -1e6, -1e6, 29.0, -23.0, -23.0, 20.0,
        34.0,
    ]);
    for transform in [skewed, [0.0; 16], [1e6; 16]] {
        session.editor_move(handle, transform).unwrap();
        session.step(1. / 60.);
        host.publish(&session).unwrap();
        assert!(host.frame(&session).unwrap().valid());
    }
    // An ordinary rotation is kept exactly.
    let mut turned = [0.0; 16];
    turned[0] = 0.0;
    turned[2] = -1.0;
    turned[5] = 1.0;
    turned[8] = 1.0;
    turned[15] = 1.0;
    session.editor_move(handle, turned).unwrap();
    let player = session.world.players[0].as_ref().unwrap();
    let o = &session.world.physics.bodies[player.body].orientation;
    assert_eq!(
        (o.right, o.up, o.backward),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0],)
    );
}

#[test]
fn hostile_requests_cannot_stop_the_host() {
    let env = |name: &str| std::env::var(name).ok().and_then(|v| v.parse().ok());
    let ticks = env("MAGICKA_FUZZ_TICKS").unwrap_or(800);
    let seeds = match env("MAGICKA_FUZZ_SEED") {
        Some(seed) => vec![seed],
        // The second one found more Arcane blasts than a frame carries.
        None => vec![0x2545_F491_4F6C_DD1D, 3_141_592_653_589_793],
    };
    for seed in seeds {
        let (frames, publish_errors) = run(seed, ticks);
        assert!(frames > 0, "seed {seed}: game state kept flowing");
        assert_eq!(publish_errors, 0, "seed {seed}: frames stayed publishable");
    }
}

/// The other direction: a hostile host's frames that pass validation must not
/// panic a client's mirror. Errors are fine; clients report and carry on.
#[test]
fn hostile_frames_cannot_panic_a_client_mirror() {
    use magicka_runtime::network_session::{mirror_world, RemoteEffects, RemotePlayers};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let content = root.join("../../../../target/Magicka/Content");
    let mut hosted = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    hosted.disable_scripts();
    hosted.step(1. / 60.);
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        "Havindr.lvl",
        false,
        PlayerProfile::default(),
        &hosted,
    )
    .unwrap();
    host.poll(&mut hosted).unwrap();
    let base = host.frame(&hosted).unwrap();
    assert!(base.valid());
    let mut mirror = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    mirror.disable_scripts();
    let (mut players, mut effects) = (RemotePlayers::default(), RemoteEffects::default());
    let mut r = Random(0x0DDB_1A5E_5BAD_5EED);
    let (mut applied, mut refused) = (0, 0);
    for round in 0..400 {
        let started = std::time::Instant::now();
        let mut frame = base.clone();
        let template = frame.players[0].clone();
        match r.below(9) {
            0 => frame.physics.epoch = 1 + r.below(5),
            1 => {
                frame.physics.scene = ["nope", "s", "../../x", "start"][r.below(4) as usize].into()
            }
            2 => frame.level = ["nope.lvl", "a/b.lvl", "Havindr.lvl"][r.below(3) as usize].into(),
            3 => frame.players.clear(),
            4 => {
                frame.players = (0..1 + r.below(8))
                    .map(|i| {
                        let mut p = template.clone();
                        p.slot = (i * (1 + r.below(36))) as usize % 256;
                        p.handle = r.next() as u16;
                        p
                    })
                    .collect();
                let mut seen = std::collections::BTreeSet::new();
                frame.players.retain(|p| seen.insert(p.slot));
            }
            5 => {
                frame.players[0].profile.robe =
                    ["Wizard_nope", "Wizard_Samurai", "wizard"][r.below(3) as usize].into();
                frame.players[0].items[r.below(2) as usize] =
                    ["nope", "", "weapon_none"][r.below(3) as usize].into();
            }
            6 => {
                frame.players[0].attached_slots = vec![r.below(12) as i32];
                frame.players[0].health = r.coordinate();
                frame.players[0].position = r.point();
                frame.players[0].unlocked_magicks = r.next();
                frame.players[0].registered_magicks = vec!["no:such".into()];
            }
            7 => {
                frame.effects = vec![magicka_network::session::VisualEffectState {
                    slot: r.below(256) as u16,
                    instance: 1 + r.below(100) as i32,
                    definition: r.next() as i32,
                    transform: r.transform(),
                    started_ms: 0,
                }];
            }
            _ => frame.time = f64::from(r.coordinate()).abs(),
        }
        if !frame.valid() {
            refused += 1;
            continue;
        }
        applied += 1;
        let own = (r.chance(2) && !frame.players.is_empty()).then(|| frame.players[0].slot);
        mirror_world(&mut mirror, &frame).ok();
        players.reconcile(&frame, own, &mut mirror).ok();
        effects
            .reconcile(
                frame.physics.epoch,
                &frame.effects,
                frame.time,
                &mut mirror.effects,
            )
            .ok();
        mirror.step(1. / 60.);
        if started.elapsed().as_secs() >= 2 {
            println!("round {round}: {:?}", started.elapsed());
        }
    }
    println!("{applied} hostile frames applied, {refused} refused by validation");
    assert!(applied > 100, "the frames reached the mirror");
}
