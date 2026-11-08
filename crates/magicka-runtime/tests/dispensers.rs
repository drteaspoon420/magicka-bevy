//! Native finite dispenser behavior and authored Mines action execution.
use magicka_runtime::{
    dispenser::Dispenser, play_session::PlaySession, scene_world::EntityKind,
    script_runtime::ScriptWorld,
};
use magicka_sim::{damage::Damage, spray_pulse::Segment};

fn dispenser(amount: i32, active: bool) -> Dispenser {
    Dispenser::new(
        0,
        magicka_content::skinned::IDENTITY,
        vec!["spider_forest".into()],
        vec![amount],
        2.0,
        active,
        [Some(1.0), Some(0.5), None, Some(1.0)],
    )
}

#[test]
fn spawn_clip_pauses_timer_and_exhaustion_plays_destroyed() {
    let mut d = dispenser(2, true);
    assert_eq!(
        d.update(0.5, |_| panic!("random before spawn clip ends"))
            .unwrap(),
        None
    );
    assert_eq!(d.timer, 0.0);
    assert_eq!(d.update(0.5, |_| 0).unwrap(), Some("spider_forest".into()));
    assert_eq!(d.amounts, vec![1]);
    assert_eq!(
        d.update(0.25, |_| panic!("random during execute")).unwrap(),
        None
    );
    assert_eq!(d.timer, 2.0);
    assert_eq!(d.update(0.25, |_| 0).unwrap(), None);
    assert_eq!(d.timer, 1.75);
    assert_eq!(d.update(1.75, |_| 0).unwrap(), Some("spider_forest".into()));
    assert_eq!(d.amounts, vec![0]);
    assert_eq!(d.update(2.0, |_| 0).unwrap(), None);
    assert_eq!(d.health, 0.0);
    assert_eq!(d.clip, Some(3));
    assert!(!d.removable());
    d.clip_time = 1.0;
    assert!(d.removable());
}

#[test]
fn inactive_and_negative_amounts_do_not_spawn() {
    let mut d = dispenser(14, false);
    assert_eq!(d.update(10.0, |_| panic!("inactive random")).unwrap(), None);
    assert_eq!(d.amounts, vec![14]);
    assert_eq!(d.health, 500.0);
    d.active = true;
    assert!(d.update(0.01, |_| 0).unwrap().is_some());
    let mut negative = dispenser(-1, true);
    assert_eq!(negative.update(1.0, |_| 0).unwrap(), None);
    assert_eq!(negative.health, 0.0, "native >0 guard rejects -1");
}

#[test]
fn damage_is_integer_truncated_and_feature_gated() {
    let mut d = dispenser(1, false);
    let hit = Damage {
        attack: 1,
        element: 2,
        amount: 19.9,
        magnitude: 0.5,
    };
    assert_eq!(d.damage(hit, false), 1);
    assert_eq!(d.health, 500.0);
    assert_eq!(d.damage(hit, true), 1);
    assert_eq!(d.health, 491.0);
    assert_eq!(
        d.damage(
            Damage {
                amount: -5.0,
                ..hit
            },
            true
        ),
        256
    );
    assert_eq!(
        d.health, 493.0,
        "negative amount is also subtracted by native"
    );
}

#[test]
fn capsule_query_retains_native_output_overwrite() {
    let mut d = dispenser(1, false);
    d.transform[12] = 10.0;
    d.transform[13] = 3.0;
    d.transform[14] = 7.0;
    let query = Segment {
        origin: [8.0, 2.5, 7.0],
        delta: [4.0, 0.0, 0.0],
    };
    assert_eq!(d.segment(query, 0.0), Some([20.0, 4.5, 14.0]));
    assert!(d
        .segment(
            Segment {
                origin: [8.0, 8.0, 7.0],
                ..query
            },
            0.0
        )
        .is_none());
}

#[test]
fn authored_mines_eggs_and_hole_actions_have_live_hosts() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        eprintln!("retail content absent; skipping");
        return;
    }
    let mut s = PlaySession::load(&root, "Mines.lvl", "Wizard").unwrap();
    s.change_scene_named("mi_s4H", "start").unwrap();
    for _ in 0..5 {
        s.step(1.0 / 60.0);
    }
    assert!(
        s.events
            .iter()
            .all(|e| !format!("{e:?}").contains("ScriptError")),
        "{:?}",
        s.events
    );
    assert_eq!(s.world.dispensers.len(), 17);
    for d in s.world.dispensers.values() {
        assert_eq!(d.model, 2);
        assert!(!d.active);
        assert_eq!(d.amounts, [14]);
    }
    assert!(s
        .world
        .entities
        .iter()
        .filter(|e| matches!(e.kind, EntityKind::Dispenser))
        .all(|e| e.unique_id == 0));
    // Execute an actual authored active SpiderHole action at its authored
    // locator, without depending on traversal/combat of the full Mines map.
    let xml = std::fs::read_to_string(root.join("Levels/Mines/mi_s4H.xml")).unwrap();
    let layout =
        magicka_runtime::scene_script::SceneScriptLayout::read(xml.as_bytes(), false).unwrap();
    let hole = layout
        .actions
        .iter()
        .find_map(|a| match a {
            magicka_runtime::scene_script::ActionDefinition::Dispenser(a)
                if matches!(
                    a.command,
                    magicka_runtime::dispenser::DispenserCommand::Spawn { model: 0, .. }
                ) =>
            {
                Some(a.clone())
            }
            _ => None,
        })
        .unwrap();
    s.world.dispenser_action(&hole).unwrap().unwrap();
    let handle = *s.world.dispensers.keys().last().unwrap();
    let before = s.world.npcs.len();
    for _ in 0..300 {
        s.step(1.0 / 60.0);
    }
    assert!(
        s.world.npcs.len() > before,
        "finite dispenser creates real NPCs"
    );
    if let Some(d) = s.world.dispensers.get(&handle) {
        assert!(d.amounts[0] < 2);
    }
    assert!(s
        .world
        .dispensers
        .values()
        .filter(|d| d.model == 2)
        .all(|d| !d.active));
    let activation = layout
        .actions
        .iter()
        .find_map(|a| match a {
            magicka_runtime::scene_script::ActionDefinition::Dispenser(a)
                if matches!(
                    a.command,
                    magicka_runtime::dispenser::DispenserCommand::Activate { .. }
                ) =>
            {
                Some(a)
            }
            _ => None,
        })
        .unwrap();
    s.world.dispenser_action(activation).unwrap().unwrap();
    assert!(
        s.world
            .dispensers
            .values()
            .filter(|d| d.model == 2)
            .all(|d| !d.active),
        "native area lookup must not select unnamed eggs"
    );
    let position = s.world.dispensers[&handle].position();
    let damage = Damage {
        attack: 1,
        element: 2,
        amount: 100.0,
        magnitude: 1.0,
    };
    let owner = s.avatar().handle;
    let hits = s.world.rail_weapon(
        owner,
        [position[0], position[1], position[2] - 4.0],
        [0.0, 0.0, 1.0],
        5.0,
        [
            damage,
            Damage::default(),
            Damage::default(),
            Damage::default(),
            Damage::default(),
        ],
        0.0,
    );
    assert!(
        hits.contains(&handle),
        "ordinary damage query includes dispenser"
    );
    assert_eq!(s.world.dispensers[&handle].health, 400.0);
    assert_eq!(
        s.world
            .entities
            .iter()
            .find(|e| e.handle == handle)
            .unwrap()
            .hit_points,
        400.0
    );
    magicka_runtime::entity_command::EntityCommandHost::kill(&mut s.world, handle).unwrap();
    s.step(1.0 / 60.0);
    assert!(!s.world.dispensers.contains_key(&handle));
}
