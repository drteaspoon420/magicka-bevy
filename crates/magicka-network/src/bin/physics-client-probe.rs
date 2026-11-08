//! Headless remote physics probe used by the networking QA wrapper.
//! Prints each accepted full snapshot as JSON; never loads retail assets.
use magicka_network::{transport::LoopbackClient, FrameError, PhysicsReplica};
use std::{
    io::{self, Write},
    net::SocketAddr,
    thread,
    time::{Duration, Instant},
};

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    // `--join <address>`: print one line when a real client gets game state.
    if args.len() == 2 && args[0] == "--join" {
        let address: SocketAddr = args[1].parse().map_err(|e| format!("address: {e}"))?;
        let key = match magicka_network::security::secret_from(None, "MAGICKA_PASSPHRASE")? {
            Some(passphrase) => {
                magicka_network::security::NetworkKey::from_passphrase(&passphrase)?
            }
            None => Default::default(),
        };
        let mut client = LoopbackClient::join(
            address,
            magicka_network::transport::JoinRequest {
                key,
                ..Default::default()
            },
        )?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(20) {
            client.update();
            if let Some(reason) = client.refusal() {
                return Err(format!("refused: {reason}"));
            }
            if !client.take_game_frames().is_empty() {
                println!("joined");
                return Ok(());
            }
            thread::sleep(Duration::from_millis(1));
        }
        return Err("no game state".into());
    }
    // `--handshake-flood <address> <seconds>`: start real handshakes and walk
    // away from them, as an attacker holding an open game's public key can.
    if args.len() == 3 && args[0] == "--handshake-flood" {
        let address: SocketAddr = args[1].parse().map_err(|e| format!("address: {e}"))?;
        let seconds: u64 = args[2].parse().map_err(|e| format!("seconds: {e}"))?;
        let started = Instant::now();
        let mut count = 0u64;
        while started.elapsed() < Duration::from_secs(seconds) {
            let mut client = LoopbackClient::connect(address)?;
            client.update();
            client.update();
            count += 1;
        }
        println!("{count}");
        return Ok(());
    }
    if args.len() != 3 {
        return Err(
            "usage: physics-client-probe <loopback-address> <final-epoch> <final-tick>".into(),
        );
    }
    let address: SocketAddr = args[0].parse().map_err(|e| format!("address: {e}"))?;
    let final_epoch: u64 = args[1].parse().map_err(|e| format!("epoch: {e}"))?;
    let final_tick: u64 = args[2].parse().map_err(|e| format!("tick: {e}"))?;
    let mut client = LoopbackClient::connect(address)?;
    let mut replica = PhysicsReplica::default();
    let started = Instant::now();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    while started.elapsed() < Duration::from_secs(20) {
        for frame in client.update() {
            match replica.apply(frame) {
                Ok(()) => {
                    let current = replica.current().unwrap();
                    serde_json::to_writer(&mut output, current).map_err(|e| e.to_string())?;
                    writeln!(output).map_err(|e| e.to_string())?;
                    output.flush().map_err(|e| e.to_string())?;
                    if (current.epoch, current.tick) >= (final_epoch, final_tick) {
                        return Ok(());
                    }
                }
                Err(FrameError::Stale) => {}
                Err(e) => return Err(format!("invalid host snapshot: {e:?}")),
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    Err("timed out waiting for the final host snapshot".into())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("physics-client-probe: {error}");
        std::process::exit(1);
    }
}
