//! Armour-stage successors: Death spawns the next stage, OverKill (a hit
//! leaving HP <= -(MaxHP/2 + 500), or any kill while Frozen, with a
//! non-pure-Arcane element) runs the OverKill collection instead, which on
//! Orc_warrior skips the armour stage and on that stage has no successor.
//! See research/armour-stages.md.
use magicka_runtime::{avatar::PlayerInput, play_session::PlaySession};
use magicka_sim::{
    damage::{attack, Damage},
    spells::Elements,
    status::{Status, StatusEffect},
};
use std::path::Path;

fn session() -> Option<PlaySession> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    Some(s)
}

fn hit(s: &mut PlaySession, h: u16, amount: f32, element: Elements) {
    let p = s.world.physics.bodies[s.world.npcs[&h].body].position;
    s.world.arc_damage(
        None,
        None,
        p,
        [0., 0., -1.],
        0.1,
        std::f32::consts::PI,
        &[Damage {
            attack: attack::DAMAGE,
            element: element.0,
            amount,
            magnitude: 1.,
        }],
        s.world.play_time,
    );
}

fn freeze(s: &mut PlaySession, h: u16) {
    let statuses = &mut s.world.npcs.get_mut(&h).unwrap().character.combat.statuses;
    statuses
        .set_effect(StatusEffect::new(Status::Frozen, 0., 100., true))
        .unwrap();
    statuses.current_mask |= Status::Frozen as u16;
}

/// The successor templates alive after `act` on a fresh `template`.
fn successors(template: &str, act: impl Fn(&mut PlaySession, u16)) -> Option<Vec<String>> {
    let mut s = session()?;
    let h = s.spawn_npc(template, "idle", 5.).unwrap();
    for _ in 0..10 {
        s.step(1. / 60.);
    }
    act(&mut s, h);
    for _ in 0..180 {
        s.step(1. / 60.);
        s.handle_input(PlayerInput::default(), 16. / 9.);
    }
    assert!(
        s.world
            .entities
            .iter()
            .find(|e| e.handle == h)
            .unwrap()
            .removed,
        "{template} survived"
    );
    let mut ids: Vec<_> = s
        .world
        .npcs
        .iter()
        .filter(|(k, _)| **k != h)
        .map(|(_, n)| n.definition.id.clone())
        .collect();
    ids.sort();
    Some(ids)
}

const WATER_ARCANE: Elements = Elements(Elements::WATER.0 | Elements::ARCANE.0);

#[test]
fn orc_warrior_death_keeps_armour_but_overkill_skips_it() {
    let Some(normal) = successors("Orc_warrior", |s, h| hit(s, h, 300., Elements::ARCANE)) else {
        return;
    };
    assert_eq!(normal, ["orc_warrior_noshield"]);
    // Native InternalDamage fires OverKill only for a non-Arcane element.
    let arcane = successors("Orc_warrior", |s, h| hit(s, h, 2000., Elements::ARCANE)).unwrap();
    assert_eq!(arcane, ["orc_warrior_noshield"]);
    let overkill = successors("Orc_warrior", |s, h| hit(s, h, 2000., WATER_ARCANE)).unwrap();
    assert_eq!(overkill, ["orc_warrior_noarmour"]);
    let shattered = successors("Orc_warrior", |s, h| {
        freeze(s, h);
        hit(s, h, 300., WATER_ARCANE);
    })
    .unwrap();
    assert_eq!(shattered, ["orc_warrior_noarmour"]);
}

#[test]
fn orc_armour_stage_dies_outright_on_overkill() {
    let Some(killed) = successors("Orc_warrior_noshield", |s, h| {
        magicka_runtime::entity_command::EntityCommandHost::kill(&mut s.world, h).unwrap();
    }) else {
        return;
    };
    assert_eq!(killed, ["orc_warrior_noarmour"]);
    let weak = successors("Orc_warrior_noshield", |s, h| {
        s.world
            .npcs
            .get_mut(&h)
            .unwrap()
            .character
            .combat
            .health
            .hit_points = 10.;
        hit(s, h, 50., Elements::WATER);
    })
    .unwrap();
    assert_eq!(weak, ["orc_warrior_noarmour"]);
    // Its OverKill collection is empty: no 2200-HP unarmoured stage.
    let overkill = successors("Orc_warrior_noshield", |s, h| {
        hit(s, h, 6000., Elements::WATER)
    })
    .unwrap();
    assert!(overkill.is_empty(), "{overkill:?}");
    let shattered = successors("Orc_warrior_noshield", |s, h| {
        freeze(s, h);
        hit(s, h, 2500., Elements::WATER);
    })
    .unwrap();
    assert!(shattered.is_empty(), "{shattered:?}");
}
