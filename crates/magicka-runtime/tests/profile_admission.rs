//! Protocol 11: a joining player's submitted profile builds its wizard; the
//! host never publishes a placeholder, and an unloadable robe is refused.
use magicka_network::{
    lobby::Role,
    session::{GameFrame, PlayerProfile, ProfileSubmit},
    transport::{JoinRequest, LoopbackClient, RefusalCode},
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
fn join(host: &HostedSession, name: &str, robe: &str) -> LoopbackClient {
    LoopbackClient::join(
        host.transport.address(),
        JoinRequest {
            role: Role::Player,
            name: name.into(),
            profile: Some(ProfileSubmit {
                profile: PlayerProfile {
                    name: name.into(),
                    robe: robe.into(),
                    colour: [0.2, 0.4, 1.3],
                },
                profile_id: None,
            }),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn in_progress_join_publishes_the_submitted_profile_from_the_first_frame() {
    let mut session = session();
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        "Havindr.lvl",
        false,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    // The game is under way before the joiner arrives (hot join).
    for _ in 0..5 {
        host.poll(&mut session).unwrap();
        host.publish(&session).unwrap();
        session.step(1. / 60.);
    }
    let mut client = join(&host, "Late Lena", "Wizard_Samurai");
    let mut frames: Vec<GameFrame> = Vec::new();
    for _ in 0..600 {
        client.update();
        host.poll(&mut session).unwrap();
        host.publish(&session).unwrap();
        frames.extend(client.take_game_frames());
        if client
            .player_slot()
            .is_some_and(|s| frames.iter().any(|f| f.players.iter().any(|p| p.slot == s)))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let slot = client.player_slot().expect("seated");
    let mut seen = false;
    for frame in &frames {
        if let Some(player) = frame.players.iter().find(|p| p.slot == slot) {
            seen = true;
            // Never a "Player N" wizard before the profile arrives.
            assert_eq!(player.profile.name, "Late Lena");
            assert_eq!(player.profile.robe, "Wizard_Samurai");
            assert_eq!(player.profile.colour, [0.2, 0.4, 1.3]);
        }
    }
    assert!(seen, "the joiner's wizard was published");
    assert_eq!(
        session.player_profiles[slot].as_ref().unwrap().0,
        "Late Lena"
    );
}

#[test]
fn unavailable_robe_is_refused_without_spawning_a_substitute() {
    let mut session = session();
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        "Havindr.lvl",
        true,
        PlayerProfile::default(),
        &session,
    )
    .unwrap();
    // Valid by name, but no such robe exists in this content.
    let mut client = join(&host, "Ghost", "Wizard_DoesNotExist");
    for _ in 0..600 {
        client.update();
        host.poll(&mut session).unwrap();
        host.publish(&session).unwrap();
        if client.refusal().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(client.refusal_code(), Some(RefusalCode::Denied));
    assert!(client.refusal().unwrap().contains("robe"));
    assert!(!host.has_players());
    assert!(host.frame(&session).unwrap().players.is_empty());
}
