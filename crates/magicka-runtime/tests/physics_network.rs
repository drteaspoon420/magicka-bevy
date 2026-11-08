use magicka_network::{transport::*, PhysicsFrame, PhysicsReplica};
use magicka_runtime::play_session::PlaySession;
use std::path::PathBuf;
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

struct Probe {
    child: Child,
    frames: Receiver<Result<PhysicsFrame, String>>,
}
impl Probe {
    fn start(binary: &std::path::Path, address: std::net::SocketAddr) -> Self {
        let mut child = Command::new(binary)
            .args([address.to_string(), "3".into(), "1".into()])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start independent physics client process");
        let stdout = child.stdout.take().unwrap();
        let (sender, frames) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let result = line
                    .map_err(|e| e.to_string())
                    .and_then(|line| serde_json::from_str(&line).map_err(|e| e.to_string()));
                if sender.send(result).is_err() {
                    break;
                }
            }
        });
        Self { child, frames }
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        // Also terminate children when a test assertion unwinds.
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "run with ./bin/viewer-qa network-physics, which builds the client probe"]
fn gameplay_physics_reaches_independent_client_processes() {
    let binary = PathBuf::from(
        std::env::var_os("MAGICKA_PHYSICS_PROBE").expect("QA wrapper sets the probe path"),
    );
    let mut host = LoopbackHost::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let mut probes = [
        Probe::start(&binary, host.address()),
        Probe::start(&binary, host.address()),
    ];
    assert_ne!(probes[0].child.id(), probes[1].child.id());
    assert!(probes.iter().all(|p| p.child.id() != std::process::id()));
    let mut replicas = [PhysicsReplica::default(), PhysicsReplica::default()];
    exercise_gameplay(|frame| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            host.update();
            host.broadcast(frame).unwrap();
            for (probe, replica) in probes.iter_mut().zip(&mut replicas) {
                while let Ok(received) = probe.frames.try_recv() {
                    replica
                        .apply(received.expect("client JSON snapshot"))
                        .unwrap();
                }
            }
            if replicas.iter().all(|r| r.current() == Some(frame)) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "both processes did not receive epoch {} tick {}",
                frame.epoch,
                frame.tick
            );
            thread::sleep(Duration::from_millis(1));
        }
        println!(
            "process clients confirmed epoch {} tick {} bodies {}",
            frame.epoch,
            frame.tick,
            frame.bodies.len()
        );
    });
    for probe in &mut probes {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = probe.child.try_wait().unwrap() {
                assert!(status.success(), "probe exit: {status}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "probe did not exit after final frame"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    println!(
        "independent client PIDs {}, {}; host PID {}",
        probes[0].child.id(),
        probes[1].child.id(),
        std::process::id()
    );
}

fn deliver(
    host: &mut LoopbackHost,
    clients: &mut [LoopbackClient; 2],
    replicas: &mut [PhysicsReplica; 2],
    frame: &PhysicsFrame,
) {
    for _ in 0..400 {
        host.update();
        host.broadcast(frame).unwrap();
        for (client, replica) in clients.iter_mut().zip(replicas.iter_mut()) {
            for received in client.update() {
                if replica
                    .current()
                    .is_none_or(|old| (received.epoch, received.tick) > (old.epoch, old.tick))
                {
                    replica.apply(received).unwrap();
                }
            }
        }
        if replicas.iter().all(|r| r.current() == Some(frame)) {
            return;
        }
    }
    panic!(
        "both clients did not receive epoch {} tick {}",
        frame.epoch, frame.tick
    );
}

fn exercise_gameplay(mut send: impl FnMut(&PhysicsFrame)) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let content = magicka_content::retail_content_dir();
    assert!(
        content.is_dir(),
        "this integration test requires user-supplied content"
    );
    let level = root
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap();
    let mut session = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
    for _ in 0..30 {
        session.step(1. / 60.);
    }
    let initial = session.physics_frame(1, 30).unwrap();
    assert_eq!(initial.bodies.len(), 3);
    assert_eq!(initial.epoch, 1);
    send(&initial);
    for _ in 0..30 {
        session.step(1. / 60.);
    }
    let moved = session.physics_frame(1, 60).unwrap();
    assert_ne!(initial.bodies, moved.bodies);
    send(&moved);
    let handle = *session.world.book_pickups.keys().next().unwrap();
    let position = session
        .world
        .entities
        .iter()
        .find(|e| e.handle == handle)
        .unwrap()
        .position;
    let avatar = session.avatar().body;
    session.world.physics.bodies[avatar].position = position;
    session.pickup_book(handle).unwrap();
    let collected = session.physics_frame(1, 61).unwrap();
    assert_eq!(collected.bodies.len(), 2);
    assert!(collected.bodies.iter().all(|b| b.handle != handle));
    send(&collected);
    session.change_scene_named("corridor", "start").unwrap();
    let changed = session.physics_frame(1, 0).unwrap();
    assert_eq!(changed.scene, "corridor");
    assert_eq!(changed.epoch, 2);
    assert!(changed.bodies.is_empty());
    send(&changed);
    session.step(1. / 60.);
    assert!(session.restart_at_checkpoint().unwrap());
    let restarted = session.physics_frame(1, 0).unwrap();
    assert_eq!(restarted.epoch, 3);
    send(&restarted);
    // Failed content lookup must not advance the network lifetime.
    assert!(session
        .change_scene_named("missing_scene", "start")
        .is_err());
    assert_eq!(session.physics_frame(1, 0).unwrap(), restarted);
    let mut pose = magicka_content::skinned::IDENTITY;
    pose[12..15].copy_from_slice(&[-4., 1., 0.]);
    session.spawn_magick_book(1, pose).unwrap();
    session.step(1. / 60.);
    let respawned = session.physics_frame(1, 1).unwrap();
    assert_eq!(respawned.epoch, 3);
    assert_eq!(respawned.bodies.len(), 1);
    send(&respawned);
    println!("two UDP clients: three live bodies, motion, collected book removed, corridor teardown, same-scene restart, new book after restart");
}

#[test]
fn gameplay_physics_collection_scene_change_and_restart_reach_two_udp_clients() {
    let mut host = LoopbackHost::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let mut clients = [
        LoopbackClient::connect(host.address()).unwrap(),
        LoopbackClient::connect(host.address()).unwrap(),
    ];
    let mut replicas = [PhysicsReplica::default(), PhysicsReplica::default()];
    exercise_gameplay(|frame| {
        deliver(&mut host, &mut clients, &mut replicas, frame);
        if frame.epoch > 1 {
            let stale = PhysicsFrame {
                epoch: 1,
                tick: 1000,
                ..frame.clone()
            };
            for replica in &mut replicas {
                assert!(replica.apply(stale.clone()).is_err());
            }
        }
    });
}
