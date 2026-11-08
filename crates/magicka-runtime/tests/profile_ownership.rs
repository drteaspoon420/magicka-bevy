//! Profile ownership and failed restore must not destroy campaign player data.
use magicka_network::{session::*, transport::*};
use magicka_runtime::{network_session::HostedSession, play_session::PlaySession};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn session() -> PlaySession {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let level = root
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap();
    let mut s = PlaySession::load(
        &magicka_content::retail_content_dir(),
        level.to_str().unwrap(),
        "Wizard",
    )
    .unwrap();
    s.disable_scripts();
    s.step(1. / 60.);
    s
}
fn request(id: &str) -> JoinRequest {
    JoinRequest {
        profile: Some(ProfileSubmit {
            profile: PlayerProfile::default(),
            profile_id: Some(id.into()),
        }),
        ..Default::default()
    }
}
fn pump(
    host: &mut HostedSession,
    s: &mut PlaySession,
    c: &mut LoopbackClient,
    until: impl Fn(&LoopbackClient) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        c.update();
        host.poll(s).unwrap();
        if until(c) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("profile admission did not finish");
}

#[test]
fn duplicate_claim_and_failed_equipment_restore_keep_ids_and_parked_rows_unchanged() {
    let mut s = session();
    let owner = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let parked = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    s.player_ids[0] = Some(owner.into());
    s.join_player_at_start(1, "Wizard").unwrap();
    assert!(s.claim_parked(1, owner, "Wizard").is_err());
    assert_eq!(s.profile_slot(owner), Some(0));
    assert!(s.player_ids[1].is_none());
    let row = serde_json::json!({"id":parked,"robe":"Wizard", "magicks":128,
        "equipment":[[1,"staff_missing_profile_admission_fixture"],[0,"weapon_stung"]]});
    s.parked_players.push(row.clone());
    let items: Vec<_> = s.world.players[1]
        .as_ref()
        .unwrap()
        .items
        .iter()
        .map(|(slot, i)| (*slot, i.definition.name.clone()))
        .collect();
    assert!(s.claim_parked(1, parked, "Wizard").is_err());
    assert!(s.player_ids[1].is_none());
    assert_eq!(s.parked_players, vec![row]);
    assert_eq!(s.unlocked_magicks[1], 0);
    assert_eq!(
        s.world.players[1]
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|(slot, i)| (*slot, i.definition.name.clone()))
            .collect::<Vec<_>>(),
        items
    );
}

#[test]
fn duplicate_profile_is_refused_before_an_avatar_exists_and_owner_keeps_its_inventory() {
    let mut s = session();
    let id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    s.player_ids[0] = Some(id.into());
    s.set_player_loadout(0, "Wizard", "staff_of_fire", "weapon_stung")
        .unwrap();
    let level = s.level_file().to_owned();
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &s,
    )
    .unwrap();
    let mut duplicate = LoopbackClient::join(host.transport.address(), request(id)).unwrap();
    pump(&mut host, &mut s, &mut duplicate, |c| c.refusal().is_some());
    assert!(duplicate.refusal().unwrap().contains("already playing"));
    assert_eq!(host.frame(&s).unwrap().players.len(), 1);
    assert!(s.world.players.get(1).is_none_or(Option::is_none));
    assert_eq!(s.profile_slot(id), Some(0));
    assert!(s
        .avatar()
        .items
        .iter()
        .any(|(_, i)| i.definition.name == "staff_of_fire"));
    assert!(host.transport.player_slots().is_empty());
}

#[test]
fn changing_persistent_profile_requires_departure_and_does_not_change_the_wizard() {
    let mut s = session();
    let level = s.level_file().to_owned();
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &s,
    )
    .unwrap();
    let id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let mut client = LoopbackClient::join(host.transport.address(), request(id)).unwrap();
    pump(&mut host, &mut s, &mut client, |c| {
        c.player_slot().is_some()
    });
    let slot = client.player_slot().unwrap();
    client
        .send_profile(ProfileRequest {
            profile_id: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            sequence: 1,
            profile: PlayerProfile {
                name: "Replacement identity".into(),
                robe: "Wizard_Samurai".into(),
                ..Default::default()
            },
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut reply = None;
    while Instant::now() < deadline {
        client.update();
        host.poll(&mut s).unwrap();
        reply = client.take_profile_replies().into_iter().last();
        if reply.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(reply.unwrap().error.unwrap().contains("leave the game"));
    assert_eq!(s.player_ids[slot].as_deref(), Some(id));
    assert_eq!(s.player_robe(slot), Some("Wizard"));
    assert_ne!(
        host.frame(&s)
            .unwrap()
            .players
            .iter()
            .find(|p| p.slot == slot)
            .unwrap()
            .profile
            .name,
        "Replacement identity"
    );
}

#[test]
fn unavailable_saved_equipment_refuses_join_and_remains_in_the_campaign_save() {
    let mut s = session();
    let id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    s.player_ids[0] = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
    let row = serde_json::json!({"id":id,"robe":"Wizard", "magicks":128,
        "equipment":[[1,"staff_missing_profile_admission_fixture"],[0,"weapon_stung"]]});
    s.parked_players.push(row.clone());
    let level = s.level_file().to_owned();
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        &level,
        false,
        PlayerProfile::default(),
        &s,
    )
    .unwrap();
    let mut client = LoopbackClient::join(host.transport.address(), request(id)).unwrap();
    pump(&mut host, &mut s, &mut client, |c| c.refusal().is_some());
    assert!(client.refusal().unwrap().contains("could not be restored"));
    assert_eq!(s.parked_players, vec![row]);
    assert!(s.profile_slot(id).is_none());
    assert_eq!(host.frame(&s).unwrap().players.len(), 1);
    let save = s.disk_save().unwrap();
    assert!(save["players"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["id"] == id && p["parked"] == true));
}
