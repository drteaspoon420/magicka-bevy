//! Assatur in EndofWorld2 ew2_s2 (research/boss-assatur.md). User report:
//! "final boss is completely missing" (it was a 20 s stand-in).
use magicka_runtime::{
    boss_assatur::State, play_session::PlaySession, scene_world::boss_live::BossRef,
};
use magicka_sim::{
    damage::{attack, Damage},
    spells::Elements,
};

const DT: f32 = 1.0 / 60.0;

fn host(s: &PlaySession) -> &magicka_runtime::scene_world::assatur_live::AssaturHost {
    s.world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            BossRef::Assatur(a) => Some(a.as_ref()),
            _ => None,
        })
        .expect("no Assatur boss")
}

fn hit(s: &mut PlaySession, element: Elements, amount: f32) {
    let zone = host(s).zone;
    let mut triggers = Vec::new();
    s.world.physics_damage.get_mut(&zone).unwrap().deliver(
        &[Damage {
            attack: attack::DAMAGE,
            element: element.0,
            amount,
            magnitude: 1.0,
        }],
        15,
        &mut triggers,
    );
}

#[test]
fn assatur_descends_casts_hurts_and_dies_with_its_triggers() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "EndofWorld2.lvl", "Wizard").unwrap();
    s.change_scene_named("ew2_s2", "start").unwrap();
    for _ in 0..12 {
        s.step(DT);
    }
    let start = host(&s).actor.position()[1];
    for _ in 0..(8.0 / DT) as usize {
        s.step(DT);
    }
    assert!(s.world.boss_fight.running, "activateBoss after 6 s");
    let a = &host(&s).actor;
    assert!(a.position()[1] < start - 30.0, "descends from 40 m above");
    assert!(
        s.world.log.iter().all(|l| !l.contains("stand-in")),
        "no stand-in"
    );
    // Fight: several cast states, and they hurt the Wizard.
    let mut states = std::collections::BTreeSet::new();
    let mut hurt = 0.0;
    for _ in 0..(90.0 / DT) as usize {
        let before = s.avatar().character.combat.health.hit_points;
        s.step(DT);
        states.insert(host(&s).actor.state);
        let after = s.avatar().character.combat.health.hit_points;
        hurt += (before - after).max(0.0);
        if after < 500.0 {
            let p = s.world.players[0].as_mut().unwrap();
            p.character.combat.health.hit_points = 1000.0;
            p.character.combat.dead = false;
        }
    }
    eprintln!("states {states:?} hurt {hurt}");
    assert!(states.len() >= 5, "too few casts: {states:?}");
    assert!(hurt > 0.0, "Assatur's attacks never hurt the Wizard");
    // Water does not hurt him (multiplier 0); Arcane does.
    let hp = host(&s).actor.status.hit_points;
    hit(&mut s, Elements::WATER, 1000.0);
    s.step(DT);
    assert_eq!(
        host(&s).actor.status.hit_points,
        hp.max(host(&s).actor.status.hit_points)
    );
    hit(&mut s, Elements::ARCANE, 1000.0);
    s.step(DT);
    assert!(
        host(&s).actor.status.hit_points < hp,
        "arcane damages Assatur"
    );
    // Death: assatur_defeated at once, assatur_killed after the die clip.
    hit(&mut s, Elements::ARCANE, 100_000.0);
    let mut defeated = false;
    let mut killed = false;
    for _ in 0..(20.0 / DT) as usize {
        s.step(DT);
        defeated |= host_state(&s) == Some(State::Dead);
        killed |= !s.world.boss_fight.active;
        if killed {
            break;
        }
    }
    assert!(defeated, "Assatur must enter its death state");
    assert!(killed, "assatur_killed must end the boss fight");
}

fn host_state(s: &PlaySession) -> Option<State> {
    s.world.boss_fight.bosses.iter().find_map(|b| match b {
        BossRef::Assatur(a) => Some(a.actor.state),
        _ => None,
    })
}
