//! LAN advertisement: a host beacon reaches a browser on this machine.
use magicka_network::{discovery::*, lobby::LobbySettings};
use std::time::{Duration, Instant};

#[test]
fn beacons_reach_the_browser_and_expire() {
    let port = 27_000 + (std::process::id() % 2000) as u16;
    let mut browser = LanBrowser::bind(port);
    assert!(browser.listening());
    let mut beacon = Beacon::with_port(port);
    let lobby = LobbySettings {
        max_players: 12,
        ..Default::default()
    };
    let advert = Advert::new(17700, "Alice's game", "Levels/Havindr.lvl", 3, lobby);
    let start = Instant::now();
    let mut games = Vec::new();
    for i in 0..50 {
        // A fresh interval each attempt.
        beacon.tick(start + Duration::from_secs(2 * i), &advert);
        std::thread::sleep(Duration::from_millis(20));
        games = browser.poll(Instant::now());
        if !games.is_empty() {
            break;
        }
    }
    assert_eq!(games.len(), 1, "beacon not heard (broadcast or loopback)");
    let game = &games[0];
    assert_eq!(game.address.port(), 17700);
    assert_eq!(game.advert.name, "Alice's game");
    assert_eq!(game.advert.level, "Havindr.lvl");
    assert_eq!(
        (game.advert.players, game.advert.lobby.max_players),
        (3, 12)
    );
    // Silent games drop off.
    assert!(browser.poll(Instant::now() + ADVERT_TTL * 2).is_empty());
}

#[test]
fn foreign_or_incompatible_beacons_are_ignored() {
    let advert = Advert::new(17700, "Game", "x.lvl", 1, LobbySettings::default());
    assert!(Advert::decode(&advert.encode()).is_some());
    let mut bad: serde_json::Value = serde_json::from_slice(&advert.encode()).unwrap();
    bad["version"] = serde_json::json!(1);
    assert!(Advert::decode(&serde_json::to_vec(&bad).unwrap()).is_none());
    bad["version"] = serde_json::json!(magicka_network::PROTOCOL_VERSION);
    bad["magic"] = serde_json::json!("other-game");
    assert!(Advert::decode(&serde_json::to_vec(&bad).unwrap()).is_none());
    assert!(Advert::decode(b"not json").is_none());
}

#[test]
fn beacons_list_participants_and_open_slots() {
    use magicka_network::discovery::{AdvertSeat, ADVERT_PARTICIPANTS};
    use magicka_network::lobby::Role;
    let lobby = LobbySettings {
        max_players: 8,
        ..LobbySettings::default()
    };
    let seat = |name: &str, role| AdvertSeat {
        name: name.into(),
        role,
        queued: false,
    };
    let advert = Advert::new(17700, "Game", "x.lvl", 2, lobby).with_participants([
        seat("Alice", Role::Player),
        seat("Bob", Role::Player),
        seat(
            "A very long dungeon master name indeed",
            Role::DungeonMaster,
        ),
    ]);
    assert_eq!(advert.open_slots(), 6);
    let back = Advert::decode(&advert.encode()).unwrap();
    assert_eq!(back.participants.len(), 3);
    assert_eq!(back.participants[2].role, Role::DungeonMaster);
    assert_eq!(
        back.participants[2].name.chars().count(),
        24,
        "names are capped"
    );
    // Many participants stay within one datagram.
    let many = Advert::new(17700, "Game", "x.lvl", 256, LobbySettings::default())
        .with_participants((0..300).map(|i| seat(&format!("Player number {i:04}"), Role::Player)));
    assert_eq!(many.participants.len(), ADVERT_PARTICIPANTS);
    assert!(many.encode().len() < 16384);
    // Older beacons without the list still decode.
    let mut json: serde_json::Value = serde_json::from_slice(&advert.encode()).unwrap();
    json.as_object_mut().unwrap().remove("participants");
    let old = Advert::decode(&serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(old.participants.is_empty());
}
