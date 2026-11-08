//! Clients without a wizard (spectators, dungeon masters, invaders) mirror
//! the host's level and scene like players do.
use magicka_network::{lobby::*, session::*, transport::LoopbackClient};
use magicka_runtime::{
    network_session::{mirror_world, HostedSession, RemotePlayers},
    play_session::PlaySession,
};
use std::path::{Path, PathBuf};

fn content() -> Option<PathBuf> {
    let root = magicka_content::retail_content_dir();
    root.is_dir().then_some(root)
}

fn corridor() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenarios/corridor.lvl")
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn follow(level: &str, scene: &str, scripts: bool) {
    let Some(content) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut host_session = PlaySession::load(&content, level, "Wizard").unwrap();
    if !scripts {
        host_session.disable_scripts();
    }
    host_session.step(1. / 60.);
    let mut host = HostedSession::bind_lobby(
        "127.0.0.1:0".parse().unwrap(),
        level,
        false,
        PlayerProfile::default(),
        &host_session,
        LobbySettings {
            invaders: true,
            ..Default::default()
        },
    )
    .unwrap();
    let mut client = LoopbackClient::connect_as(host.transport.address(), Role::Invader, "Inv")
        .expect("invader connection");
    let mut client_session = PlaySession::load(&content, level, "Wizard").unwrap();
    client_session.disable_scripts();
    let mut players = RemotePlayers::default();
    let mut frame = None;
    let mut observer_ready = false;
    for i in 0..240 {
        if i == 60 {
            host_session.change_scene_named(scene, "start").unwrap();
        }
        client.update();
        host.poll(&mut host_session).unwrap();
        host_session.step(1. / 60.);
        host.publish(&host_session).unwrap();
        for f in client.take_game_frames() {
            frame = Some(f);
        }
        let Some(frame) = &frame else { continue };
        if !observer_ready {
            // The viewer's dormant slot-0 avatar while observing.
            client_session.depart_network_player(0).unwrap();
            observer_ready = true;
        }
        mirror_world(&mut client_session, frame).unwrap();
        players.reconcile(frame, None, &mut client_session).unwrap();
    }
    assert_eq!(host_session.scene_name(), scene);
    assert_eq!(client_session.scene_name(), scene);
}

#[test]
fn observers_follow_host_scene_changes() {
    follow(&corridor(), "lab", false);
}

#[test]
fn observers_follow_scripted_campaign_scenes() {
    follow("Wizard_Castle.lvl", "wc_s3", true);
}

#[test]
fn scene_changes_keep_players_beyond_the_fourth_slot() {
    let Some(content) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut session = PlaySession::load(&content, "Wizard_Castle.lvl", "Wizard").unwrap();
    session.disable_scripts();
    for slot in [1, 6] {
        session.join_player_at_start(slot, "Wizard").unwrap();
    }
    session.change_scene_named("wc_s2", "start").unwrap();
    assert!(session.world.players[1].is_some());
    assert!(
        session.world.players.get(6).is_some_and(Option::is_some),
        "slot 6 was dropped by the scene change"
    );
}
