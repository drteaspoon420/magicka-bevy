//! A real server process and a Lightyear client; no GPU or retail executable.
use magicka_network::{control::*, session::*, transport::LoopbackClient};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
struct Server(Child, PathBuf);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let _ = fs::remove_file(&self.1);
    }
}
#[test]
fn standalone_headless_server_assigns_and_moves_a_named_player() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let log = std::env::temp_dir().join(format!("magicka-server-qa-{}.log", std::process::id()));
    let child = Command::new(env!("CARGO_BIN_EXE_magicka-server"))
        .args([
            "--content",
            magicka_content::retail_content_dir().to_str().unwrap(),
            "--level",
            root.join("../../scenarios/feature-lab.lvl")
                .canonicalize()
                .unwrap()
                .to_str()
                .unwrap(),
            "--bind",
            "127.0.0.1:0",
            "--max-ticks",
            "600",
        ])
        .stdout(Stdio::from(fs::File::create(&log).unwrap()))
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut server = Server(child, log);
    let deadline = Instant::now() + Duration::from_secs(10);
    let address = loop {
        assert!(Instant::now() < deadline, "server startup timed out");
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "server exited before listening"
        );
        let text = fs::read_to_string(&server.1).unwrap();
        if let Some(address) = text
            .lines()
            .find_map(|line| line.strip_prefix("Headless Magicka rewrite server listening on "))
        {
            // "<address> (players N, hot join ...)".
            break address.split(' ').next().unwrap().parse().unwrap();
        }
        thread::sleep(Duration::from_millis(5));
    };
    let mut client = LoopbackClient::connect(address).unwrap();
    let mut sequence = 0;
    let mut profile_sent = false;
    let mut initial = None;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        assert!(Instant::now() < deadline, "player setup/movement timed out");
        client.update();
        for frame in client.take_game_frames() {
            assert!(frame.valid());
            assert_eq!(client.player_slot(), Some(0));
            let state = &frame.players[0];
            let start = *initial.get_or_insert(state.position);
            if !profile_sent {
                client
                    .send_profile(ProfileRequest {
                        profile_id: None,
                        sequence: 1,
                        profile: PlayerProfile {
                            name: "Process client".into(),
                            colour: [0.2, 0.3, 0.8],
                            ..Default::default()
                        },
                    })
                    .unwrap();
                profile_sent = true;
            }
            sequence += 1;
            client
                .send_input(InputCommand {
                    session: frame.physics.session,
                    epoch: frame.physics.epoch,
                    sequence,
                    controls: Controls {
                        aim: Some([start[0] + 4., start[1], start[2]]),
                        move_down: true,
                        ..Default::default()
                    },
                    history: Vec::new(),
                })
                .unwrap();
            if state.profile.name == "Process client" && (state.position[0] - start[0]).abs() > 0.3
            {
                println!(
                    "server PID {}, client PID {}, assigned slot {}, profile {}, movement {:.3}",
                    server.0.id(),
                    std::process::id(),
                    state.slot,
                    state.profile.name,
                    state.position[0] - start[0]
                );
                return;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
}
