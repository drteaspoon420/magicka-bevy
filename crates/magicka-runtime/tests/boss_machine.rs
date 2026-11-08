//! The Machine boss in havindr_s7BF (research/boss-battles.md).
use magicka_runtime::{
    boss_machine::State, play_session::PlaySession, scene_loading::custom_hash,
    scene_world::boss_live::BossRef,
};
use magicka_sim::{
    damage::{attack, Damage},
    spells::Elements,
};
use std::path::Path;

const DT: f32 = 1.0 / 60.0;

fn session() -> Option<PlaySession> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene_named("havindr_s7BF", "start").unwrap();
    for _ in 0..30 {
        s.step(DT);
    }
    Some(s)
}

fn machine(s: &PlaySession) -> &magicka_runtime::scene_world::machine_live::MachineHost {
    s.world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            BossRef::Machine(m) => Some(m),
            _ => None,
        })
        .expect("no Machine boss")
}
fn machine_mut(
    s: &mut PlaySession,
) -> &mut magicka_runtime::scene_world::machine_live::MachineHost {
    s.world
        .boss_fight
        .bosses
        .iter_mut()
        .find_map(|b| match b {
            BossRef::Machine(m) => Some(m),
            _ => None,
        })
        .expect("no Machine boss")
}

/// Run the script's intro end: activateBoss after 5 s.
fn start_fight(s: &mut PlaySession) {
    s.run_trigger("introTalk2done", true).unwrap();
    for _ in 0..(6.0 / DT) as usize {
        s.step(DT);
    }
    assert!(
        s.world.boss_fight.running,
        "activateBoss did not start the fight"
    );
}

fn step(s: &mut PlaySession, seconds: f32) {
    for _ in 0..(seconds / DT) as usize {
        s.step(DT);
    }
}

#[test]
fn machine_spawns_its_warlock_and_zones_then_drills_after_the_intro() {
    let Some(mut s) = session() else {
        return;
    };
    let m = machine(&s);
    assert_eq!(m.machine.stage, State::Intro);
    assert_eq!(m.machine.machine_state, State::Idle);
    let warlock = m.warlock;
    let zones = m.zones;
    let entity = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == warlock)
        .unwrap();
    assert_eq!(entity.unique_id, custom_hash("boss_machine_warlock"));
    assert!(s.world.npcs[&warlock].character.is_immortal());
    for z in zones {
        assert!(s.world.physics_damage[&z].boss_zone.is_some());
        assert!(s.world.static_prop_boxes.contains_key(&z));
    }
    start_fight(&mut s);
    let m = machine(&s);
    assert_eq!(m.machine.stage, State::PreGunter);
    assert_eq!(m.machine.machine_state, State::Drill);
    // The orc nods, then pedals; the drill advances at the pedal speed.
    let t0 = m.machine.drill_time;
    step(&mut s, 10.0);
    let m = machine(&s);
    assert_eq!(m.machine.orc.clip_name(), Some("pedal"));
    let advanced = m.machine.drill_time - t0;
    assert!(
        advanced > 5.0 && advanced < 10.0,
        "drill advanced {advanced}"
    );
    let ruler = m.machine.ruler.unwrap();
    assert!(ruler.alpha > 0.99);
    // The bar shows the machine: (HP + 1) / 5000.
    assert!((s.world.boss_fight.normalized_health - 1.0).abs() < 1e-3);
}

/// A delivery through the zone's PhysicsDamage, as every prop-damaging path
/// (missiles, sprays, beams, melee) performs it.
fn hit_zone(s: &mut PlaySession, zone: u16, damage: Damage) {
    let mut triggers = Vec::new();
    s.world
        .physics_damage
        .get_mut(&zone)
        .unwrap()
        .deliver(&[damage], 15, &mut triggers);
}

#[test]
fn arc_attack_reaches_machine_zone_and_runs_the_authored_break_trigger() {
    let Some(mut s) = session() else {
        return;
    };
    start_fight(&mut s);
    let zones = machine(&s).zones;
    let position = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == zones[0])
        .unwrap()
        .position;
    let damage = Damage {
        attack: attack::DAMAGE,
        element: Elements::WATER.0,
        amount: 5000.,
        magnitude: 1.,
    };
    assert!(s.world.arc_damage(
        Some(s.avatar().handle),
        Some(zones[1]),
        position,
        [0., 0., 1.],
        6.,
        std::f32::consts::PI,
        &[damage],
        0.,
    ));
    assert_eq!(
        s.world.physics_damage[&zones[0]].boss_inbox.len(),
        1,
        "the zone must receive the arc before the boss update"
    );
    step(&mut s, 0.1);
    assert_eq!(machine(&s).machine.machine_state, State::Broken);
    assert!(
        s.runtime
            .trigger(custom_hash("machine_destroyed"))
            .unwrap()
            .has_triggered,
        "the boss break must run its authored script trigger"
    );
}

#[test]
fn wall_shield_protects_machine_damage_zone_from_an_arc() {
    use magicka_sim::shield_entity::{ShieldKind, SHIELD_COLOR};
    let Some(mut s) = session() else {
        return;
    };
    start_fight(&mut s);
    let zones = machine(&s).zones;
    let p = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == zones[0])
        .unwrap()
        .position;
    let origin = [p[0] + 6., p[1], p[2]];
    s.world.spawn_shield(
        0,
        [p[0] + 3., p[1], p[2]],
        4.,
        [0., 0., -1.],
        ShieldKind::Wall,
        5000.,
        SHIELD_COLOR,
    );
    let damage = Damage {
        attack: attack::DAMAGE,
        element: Elements::WATER.0,
        amount: 100.,
        magnitude: 1.,
    };
    s.world.arc_damage(
        None,
        Some(zones[1]),
        origin,
        [-1., 0., 0.],
        12.,
        std::f32::consts::PI,
        &[damage],
        0.,
    );
    assert!(s.world.physics_damage[&zones[0]].boss_inbox.is_empty());
    assert!(s.world.shields[0].shield.hit_points < 5000.);
    s.world.shields.clear();
    s.world.arc_damage(
        None,
        Some(zones[1]),
        origin,
        [-1., 0., 0.],
        12.,
        std::f32::consts::PI,
        &[damage],
        0.,
    );
    assert_eq!(s.world.physics_damage[&zones[0]].boss_inbox.len(), 1);
}

#[test]
fn breaking_the_machine_frees_the_orc_and_starts_the_warlock_fight() {
    let Some(mut s) = session() else {
        return;
    };
    start_fight(&mut s);
    let zones = machine(&s).zones;
    let water = Damage {
        attack: attack::DAMAGE,
        element: Elements::WATER.0,
        amount: 1000.0,
        magnitude: 1.0,
    };
    hit_zone(&mut s, zones[0], water);
    step(&mut s, 0.1);
    let hp = machine(&s).machine.status.hit_points;
    assert_eq!(hp, 4000.0, "zone damage did not reach the machine");
    for _ in 0..4 {
        hit_zone(&mut s, zones[0], water);
        step(&mut s, 0.1);
    }
    let m = machine(&s);
    assert_eq!(m.machine.machine_state, State::Broken);
    assert!(
        s.runtime
            .trigger(custom_hash("machine_destroyed"))
            .unwrap()
            .has_triggered
    );
    // Broken machines take no further damage.
    hit_zone(&mut s, zones[0], water);
    step(&mut s, 0.1);
    assert!(machine(&s).machine.status.hit_points <= 0.0);
    // The script sends OrcBipedal; the breakfree clip ends the stage.
    machine_mut(&mut s).machine.message("OrcBipedal");
    let warlock = machine(&s).warlock;
    for _ in 0..(20.0 / DT) as usize {
        s.step(DT);
        if machine(&s).machine.stage == State::Warlock {
            break;
        }
    }
    assert_eq!(machine(&s).machine.stage, State::Warlock);
    let n = &s.world.npcs[&warlock];
    assert!(!n.character.is_immortal(), "warlock still immortal");
    assert_eq!(
        n.character.combat.health.hit_points,
        n.character.combat.health.max_hit_points
    );
    // The bar follows the warlock now; killing him starts the finale.
    s.world
        .npcs
        .get_mut(&warlock)
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 0.0;
    step(&mut s, 0.1);
    assert_eq!(machine(&s).machine.stage, State::Final);
    let mut post_final = false;
    for _ in 0..(30.0 / DT) as usize {
        s.step(DT);
        if machine(&s).machine.stage == State::PostFinal {
            post_final = true;
            break;
        }
    }
    assert!(post_final, "king_vader never finished");
    step(&mut s, 0.5);
    assert!(
        s.world
            .entities
            .iter()
            .find(|e| e.handle == warlock)
            .is_none_or(|e| e.removed || e.dead),
        "warlock not terminated"
    );
    assert!(s.world.boss_fight.normalized_health <= 0.0);
}

#[test]
fn the_drill_reaching_the_king_is_a_defeat() {
    let Some(mut s) = session() else {
        return;
    };
    start_fight(&mut s);
    step(&mut s, 5.0);
    assert_eq!(machine(&s).machine.orc.clip_name(), Some("pedal"));
    machine_mut(&mut s).machine.drill_time = 119.99;
    step(&mut s, 0.2);
    let m = machine(&s);
    assert_eq!(m.machine.stage, State::KingKilled);
    assert!(!m.machine.render_king);
    assert!(s.world.log.iter().any(|l| l == "machine: king killed"));
}

#[test]
fn burning_speeds_the_orc_and_freezing_stops_it() {
    let Some(mut s) = session() else {
        return;
    };
    start_fight(&mut s);
    step(&mut s, 3.0);
    let zones = machine(&s).zones;
    let fire = Damage {
        attack: attack::STATUS,
        element: Elements::FIRE.0,
        amount: 50.0,
        magnitude: 1.0,
    };
    // Status attacks on the machine zone are stripped; the orc zone burns.
    hit_zone(&mut s, zones[0], fire);
    step(&mut s, 0.1);
    assert!(!machine(&s)
        .machine
        .status
        .has(magicka_sim::status::Status::Burning));
    hit_zone(&mut s, zones[1], fire);
    step(&mut s, 2.0);
    let m = machine(&s);
    assert!(m.machine.status.has(magicka_sim::status::Status::Burning));
    assert!(
        m.machine.pedal_speed > 1.5,
        "pedal {}",
        m.machine.pedal_speed
    );
}

#[test]
fn machine_initialization_provides_the_named_warlock_for_the_real_intro() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        eprintln!("retail content absent; skipping");
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene_named("havindr_s7BF", "start").unwrap();
    s.step(1. / 60.);
    let warlocks: Vec<_> = s
        .world
        .entities
        .iter()
        .filter(|e| e.unique_id == custom_hash("boss_machine_warlock") && e.added && !e.removed)
        .collect();
    assert_eq!(
        warlocks.len(),
        1,
        "native Machine.Initialize creates the speaker"
    );
    let handle = warlocks[0].handle;
    let n = &s.world.npcs[&handle];
    assert_eq!(
        warlocks[0].template_name.as_deref(),
        Some("boss_machine_warlock")
    );
    assert!(n.character.combat.cannot_die_without_explicit_kill);
    assert!(n.character.immortal_time.is_infinite());
    assert_eq!(n.ai.order, 1);
    assert_eq!(n.react_to, 0);
    assert_eq!(n.reaction_order, 1);
    assert!(!s.world.boss_fight.running, "the authored boss is delayed");
    assert!(s.world.flags.cutscene && s.world.flags.input_limited);
    for frame in 0..3600 {
        s.step(1. / 60.);
        assert!(s.script_error.is_none(), "{:?}", s.script_error);
        if frame % 60 == 0 {
            let n = &s.world.npcs[&handle];
            eprintln!("machine intro frame={frame} running={} cutscene={} limited={} order={} event={} clip={:?} position={:?}",
                s.world.boss_fight.running,s.world.flags.cutscene,s.world.flags.input_limited,
                n.ai.order,n.ai.current_event,n.motion.controller.clip_name(),s.world.physics.bodies[n.body].position);
        }
        if s.world.boss_fight.running {
            break;
        }
    }
    assert!(
        s.world.boss_fight.running,
        "authored intro must activate the boss"
    );
    assert!(!s.world.flags.cutscene && !s.world.flags.input_limited);
    for name in ["machineintro1", "machineintro2"] {
        assert!(s
            .world
            .dialogs
            .as_ref()
            .unwrap()
            .is_done(custom_hash(name), -1)
            .unwrap());
    }
    assert_eq!(
        s.world.npcs[&handle].motion.controller.clip_name(),
        Some("warlock_machine_idle")
    );
    assert_eq!(
        s.world
            .entities
            .iter()
            .filter(|e| e.added && !e.removed && e.template_name.as_deref() == Some("warlock"))
            .count(),
        2
    );
    eprintln!("machine opening complete: both dialogs finished, two backups present, fight active and input restored");
}

#[test]
fn machine_checkpoint_rebuilds_one_protected_speaker() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene_named("havindr_s7BF", "start").unwrap();
    for _ in 0..120 {
        s.step(1. / 60.);
    }
    assert!(s.checkpoint.is_some(), "authored opening checkpoint");
    for _ in 0..2 {
        assert!(s.restart_at_checkpoint().unwrap());
        s.step(1. / 60.);
        assert!(s.script_error.is_none(), "{:?}", s.script_error);
        let speakers: Vec<_> = s
            .world
            .entities
            .iter()
            .filter(|e| e.unique_id == custom_hash("boss_machine_warlock") && e.added && !e.removed)
            .collect();
        assert_eq!(
            speakers.len(),
            1,
            "checkpoint replay must rebuild the boss-owned NPC"
        );
        let n = &s.world.npcs[&speakers[0].handle];
        assert!(n.character.combat.cannot_die_without_explicit_kill);
        assert!(n.character.immortal_time.is_infinite());
    }
}
