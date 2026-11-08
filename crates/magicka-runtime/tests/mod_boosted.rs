//! Plan suite `mod/boosted` (milestone M10), run against the shipped
//! `mods/boosted` package (data + Lua only) on the real simulation.
use magicka_definitions::schema::{self, Change, Contribution, Layer};
use magicka_runtime::{
    item_definitions::number, mod_runtime::ModRuntime, play_session::PlaySession,
};
use serde_json::{json, Value};
use std::path::PathBuf;

fn mods_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}
fn content() -> PathBuf {
    magicka_content::retail_content_dir()
}
fn two_players() -> PlaySession {
    let mut s = PlaySession::load(&content(), "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.join_player_at_start(1, "Wizard").unwrap();
    s
}
fn boosted(session: &mut PlaySession, seed: u64) -> ModRuntime {
    let mut mods = ModRuntime::load(None, &mods_root(), Some(&["boosted".into()]), seed)
        .unwrap_or_else(|e| panic!("{e}"));
    mods.attach(session);
    for _ in 0..3 {
        tick(session, &mut mods);
    }
    mods
}
fn tick(session: &mut PlaySession, mods: &mut ModRuntime) {
    mods.step(session, 1.0 / 60.0);
    session.step(1.0 / 60.0);
}
fn table(mods: &ModRuntime, key: &str) -> Value {
    mods.scripts
        .net_tables()
        .get("boosted")
        .and_then(|t| t.get(key).cloned())
        .unwrap_or(Value::Null)
}
fn offers(mods: &ModRuntime, p: usize) -> Vec<Value> {
    table(mods, &format!("offers.{p}"))
        .as_array()
        .cloned()
        .unwrap_or_default()
}
fn gold(mods: &ModRuntime, p: usize) -> i64 {
    table(mods, &format!("gold.{p}")).as_i64().unwrap()
}
fn value(session: &PlaySession, p: usize, target: &str, key: &str) -> f64 {
    let item = &session.world.players[p]
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|(_, i)| i.definition.name.eq_ignore_ascii_case(target))
        .unwrap()
        .1;
    number(&item.definition, key).unwrap()
}
fn base(session: &PlaySession, p: usize, target: &str, key: &str) -> f64 {
    let item = &session.world.players[p]
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|(_, i)| i.definition.name.eq_ignore_ascii_case(target))
        .unwrap()
        .1;
    number(
        item.base_definition.as_deref().unwrap_or(&item.definition),
        key,
    )
    .unwrap()
}
fn expected(base: f64, key: &str, percent: f64) -> f64 {
    let s = schema::find(magicka_definitions::ITEM_FIELDS, key).unwrap();
    schema::resolve(
        s,
        base,
        &[Contribution {
            layer: Layer::Upgrade,
            change: Change::Percent(percent),
            source: "test".into(),
        }],
    )
}
fn earn(session: &mut PlaySession, mods: &mut ModRuntime, kills: usize) {
    let at = session.world.physics.bodies[session.avatar().body].position;
    let handles: Vec<u16> = (0..kills)
        .map(|i| {
            session
                .spawn_npc_at(
                    "Goblin_pirate",
                    [at[0] + 6.0 + i as f32, at[1], at[2] + 6.0],
                )
                .unwrap()
        })
        .collect();
    tick(session, mods);
    for h in &handles {
        session.smite(*h).unwrap();
    }
    for _ in 0..400 {
        tick(session, mods);
        if handles
            .iter()
            .all(|h| session.world.npcs.get(h).is_none_or(|n| n.dead()))
        {
            break;
        }
    }
    for _ in 0..3 {
        tick(session, mods);
    }
}

#[test]
fn players_buy_distinct_upgrades_on_the_same_item_and_requests_charge_once() {
    let mut s = two_players();
    let mut mods = boosted(&mut s, 5);
    assert_eq!((gold(&mods, 0), gold(&mods, 1)), (100, 100));
    let o0 = offers(&mods, 0)
        .into_iter()
        .find(|o| o["price"].as_i64().unwrap() <= 100)
        .expect("an affordable offer");
    let key = |o: &Value| {
        (
            o["target"].as_str().unwrap().to_string(),
            o["key"].as_str().unwrap().to_string(),
        )
    };
    let o1 = offers(&mods, 1)
        .into_iter()
        .find(|o| key(o) != key(&o0))
        .expect("a different offer");
    let price0 = o0["price"].as_i64().unwrap();
    // The same request delivered twice charges once and returns one result.
    let r1 = mods.request(&mut s, 0, 77, "boosted.buy", &json!({ "item": o0 }));
    let r2 = mods.request(&mut s, 0, 77, "boosted.buy", &json!({ "item": o0 }));
    assert_eq!(r1, r2);
    if price0 <= 100 {
        assert_eq!(r1["ok"], json!(true), "{r1}");
        assert_eq!(gold(&mods, 0), 100 - price0);
    } else {
        assert_eq!(r1["error"], json!("not enough gold"));
    }
    let r = mods.request(&mut s, 1, 77, "boosted.buy", &json!({ "item": o1 }));
    let bought1 = r["ok"] == json!(true);
    tick(&mut s, &mut mods);
    // Each player's own purchase shows on their item only.
    let (t0, k0) = key(&o0);
    let (t1, k1) = key(&o1);
    if r1["ok"] == json!(true) {
        let b = base(&s, 0, &t0, &k0);
        assert_eq!(
            value(&s, 0, &t0, &k0),
            expected(b, &k0, o0["percent"].as_f64().unwrap())
        );
        if (t0.clone(), k0.clone()) != (t1.clone(), k1.clone()) || !bought1 {
            assert_eq!(
                value(&s, 1, &t0, &k0),
                base(&s, 1, &t0, &k0),
                "player 1 unaffected"
            );
        }
    }
    if bought1 {
        let b = base(&s, 1, &t1, &k1);
        assert_eq!(
            value(&s, 1, &t1, &k1),
            expected(b, &k1, o1["percent"].as_f64().unwrap())
        );
    }
    // A stale offer is refused without charging.
    let g = gold(&mods, 0);
    let r = mods.request(&mut s, 0, 78, "boosted.buy", &json!({ "item": o0 }));
    assert_eq!(r["error"], json!("that offer is no longer available"));
    assert_eq!(gold(&mods, 0), g);
}

#[test]
fn offers_respect_blocked_and_linked_keys_and_linked_upgrades_apply_once() {
    let mut s = two_players();
    let mut mods = boosted(&mut s, 9);
    earn(&mut s, &mut mods, 40);
    assert!(gold(&mods, 0) >= 1000, "kills pay: {}", gold(&mods, 0));
    let mut request = 1000;
    let mut linked_bought = None;
    for _ in 0..40 {
        let list = offers(&mods, 0);
        for o in &list {
            let k = o["key"].as_str().unwrap();
            assert!(
                !["GunAccuracy", "RangedElevation"].contains(&k),
                "blocked {k} offered"
            );
        }
        // Linked partners never appear side by side for one item.
        for o in &list {
            if let Some(partner) = o["linked"].as_str() {
                assert!(!list
                    .iter()
                    .any(|x| x["target"] == o["target"] && x["key"] == partner));
            }
        }
        let pick = list
            .iter()
            .find(|o| o["linked"].is_string())
            .or(list.first())
            .cloned()
            .unwrap();
        request += 1;
        let r = mods.request(&mut s, 0, request, "boosted.buy", &json!({ "item": pick }));
        if r["ok"] == json!(true) && pick["linked"].is_string() {
            linked_bought = Some(pick);
            break;
        }
    }
    let pick = linked_bought.expect("a linked offer was bought");
    let target = pick["target"].as_str().unwrap();
    let mine: Vec<_> = mods
        .upgrades
        .values()
        .filter(|u| u.player == 0 && u.target == target)
        .map(|u| u.key.clone())
        .collect();
    let key = pick["key"].as_str().unwrap();
    let partner = pick["linked"].as_str().unwrap();
    assert_eq!(mine.iter().filter(|k| *k == key).count(), 1, "{mine:?}");
    assert_eq!(mine.iter().filter(|k| *k == partner).count(), 1, "{mine:?}");
}

#[test]
fn saved_upgrades_and_gold_restore_and_disabling_restores_base_values() {
    let mut s = two_players();
    let mut mods = boosted(&mut s, 3);
    let offer = offers(&mods, 0)
        .into_iter()
        .find(|o| o["price"].as_i64().unwrap() <= 100)
        .expect("an affordable offer");
    let r = mods.request(&mut s, 0, 5, "boosted.buy", &json!({ "item": offer }));
    assert_eq!(r["ok"], json!(true));
    for _ in 0..40 {
        tick(&mut s, &mut mods);
    }
    let (target, key) = (
        offer["target"].as_str().unwrap(),
        offer["key"].as_str().unwrap(),
    );
    let upgraded = value(&s, 0, target, key);
    let left = gold(&mods, 0);
    s.player_ids[0] = Some("00000000000000000000000000000001".into());
    s.player_ids[1] = Some("00000000000000000000000000000002".into());
    let save = s.disk_save().unwrap();
    // Reload into a fresh match: same gold, same upgrade, and the stored
    // transaction still answers a repeated request without charging.
    let mut restored = PlaySession::load_disk_save(&content(), &save).unwrap();
    restored.disable_scripts();
    let mut again = ModRuntime::load(None, &mods_root(), Some(&["boosted".into()]), 3).unwrap();
    again.attach(&mut restored);
    for _ in 0..3 {
        tick(&mut restored, &mut again);
    }
    assert_eq!(gold(&again, 0), left);
    assert_eq!(value(&restored, 0, target, key), upgraded);
    let repeat = again.request(
        &mut restored,
        0,
        5,
        "boosted.buy",
        &json!({ "item": offer }),
    );
    assert_eq!(repeat, r);
    assert_eq!(gold(&again, 0), left);
    // Disabled Boosted leaves no live effect: base values and no UI state.
    let baseline = value(&restored, 1, target, key);
    again.disable_mod(&mut restored, "boosted");
    tick(&mut restored, &mut again);
    assert_eq!(value(&restored, 0, target, key), baseline);
    assert!(again.upgrades.is_empty());
    assert!(again.scripts.net_tables().values().all(|t| t.is_empty()));
}

#[test]
fn late_joiners_see_current_offers_and_gold_without_rerunning_rewards() {
    use magicka_network::{
        session::{PlayerProfile, ProfileSubmit},
        transport::{JoinRequest, LoopbackClient},
    };
    use magicka_runtime::network_session::HostedSession;
    let mut s = PlaySession::load(&content(), "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let mut mods = boosted(&mut s, 1);
    let mut host = HostedSession::bind(
        "127.0.0.1:0".parse().unwrap(),
        "Havindr.lvl",
        false,
        PlayerProfile::default(),
        &s,
    )
    .unwrap();
    host.set_content(mods.content.rules_hash.clone(), mods.content.mods.clone());
    let offer = offers(&mods, 0)[0].clone();
    mods.request(&mut s, 0, 1, "boosted.buy", &json!({ "item": offer }));
    let host_gold = gold(&mods, 0);
    let mut client = LoopbackClient::join(
        host.transport.address(),
        JoinRequest {
            name: "Late".into(),
            profile: Some(ProfileSubmit {
                profile: PlayerProfile {
                    name: "Late".into(),
                    ..Default::default()
                },
                profile_id: None,
            }),
            rules_hash: mods.content.rules_hash.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    let mut frame = None;
    for _ in 0..400 {
        client.update();
        host.poll(&mut s).unwrap();
        host.serve_mod_requests(&mut mods, &mut s);
        mods.step(&mut s, 1.0 / 60.0);
        s.step(1.0 / 60.0);
        host.publish_mods(&mut mods);
        host.publish(&s).unwrap();
        if let Some(f) = client.take_game_frames().pop() {
            let slot = client.player_slot();
            if slot.is_some_and(|sl| f.mod_ui.iter().any(|e| e.key == format!("offers.{sl}"))) {
                frame = Some(f);
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let frame = frame.expect("the joiner's offers replicated");
    let slot = client.player_slot().unwrap();
    // The joiner's panel renders from replicated state only.
    let panel = &mods.definitions.panels[0];
    let view =
        magicka_runtime::mod_ui::render(panel, &frame.mod_ui, slot, &mods.definitions.localization);
    assert_eq!(view.title, "Boosted");
    assert_eq!(view.lines[0], "Gold: 100");
    assert!(!view.choices.is_empty());
    assert_eq!(view.action.as_deref(), Some("boosted.buy"));
    // The host player's state is untouched by the join.
    assert_eq!(gold(&mods, 0), host_gold);
    // A client purchase travels as a request; the reply comes back.
    client
        .send_mod_request(magicka_network::session::ModRequest {
            request_id: 9,
            channel: "boosted.buy".into(),
            json: json!({ "item": view.choices[0].1 }).to_string(),
        })
        .unwrap();
    let mut reply = None;
    for _ in 0..400 {
        client.update();
        host.poll(&mut s).unwrap();
        host.serve_mod_requests(&mut mods, &mut s);
        mods.step(&mut s, 1.0 / 60.0);
        s.step(1.0 / 60.0);
        host.publish_mods(&mut mods);
        host.publish(&s).unwrap();
        if let Some(r) = client.take_mod_replies().pop() {
            reply = Some(r);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let reply: Value = serde_json::from_str(&reply.expect("reply").json).unwrap();
    assert!(
        reply["ok"] == json!(true) || reply["error"] == json!("not enough gold"),
        "{reply}"
    );
}
