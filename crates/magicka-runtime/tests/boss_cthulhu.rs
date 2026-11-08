use magicka_runtime::{
    boss_cthulhu::{State, TentacleState, MAX_HIT_POINTS, TENTACLE_HIT_POINTS},
    play_session::PlaySession,
    scene_world::{boss_live::BossRef, cthulhu_live::CthulhuHost},
};
use magicka_sim::{
    damage::{attack, Damage},
    spells::Elements as E,
};
use std::path::Path;
const DT: f32 = 1. / 60.;

fn host(s: &PlaySession) -> &CthulhuHost {
    s.world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            BossRef::Cthulhu(c) => Some(c.as_ref()),
            _ => None,
        })
        .expect("Cthulhu missing")
}
fn session() -> Option<PlaySession> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Tsar/Tsar_Rlyeh.lvl", "Wizard").unwrap();
    s.change_scene_named("tr_s9bf", "move_encounter").unwrap();
    s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
    step(&mut s);
    s.run_trigger("CthulhuStart", true).unwrap();
    for _ in 0..60 {
        step(&mut s);
    }
    Some(s)
}
fn step(s: &mut PlaySession) {
    s.step(DT);
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
}
fn hurt(s: &mut PlaySession, zone: u16, amount: f32) {
    s.world
        .damage_physics(
            zone,
            Damage {
                attack: attack::DAMAGE,
                element: E::ARCANE.0,
                amount,
                magnitude: 1.,
            },
            15,
        )
        .unwrap();
    step(s);
}

#[test]
fn cthulhu_emerges_with_two_tentacles_and_starts_fighting() {
    let Some(mut s) = session() else { return };
    assert!(!s.world.log.iter().any(|l| l.contains("stand-in")));
    let c = &host(&s).cthulhu;
    assert_eq!(c.status.hit_points, MAX_HIT_POINTS);
    let active = c.tentacles.iter().filter(|t| t.active).count();
    assert_eq!(active, 2, "the intro stage raises two tentacles");
    let mut seen = std::collections::BTreeSet::new();
    let mut tentacle_states = std::collections::BTreeSet::new();
    for _ in 0..60 * 60 {
        step(&mut s);
        let c = &host(&s).cthulhu;
        seen.insert(format!("{:?}", c.state));
        for t in c.tentacles.iter().filter(|t| t.active) {
            tentacle_states.insert(format!("{:?}", t.state));
        }
    }
    assert!(seen.contains("Idle"), "never idle: {seen:?}");
    assert!(seen.len() > 2, "no action after StartFight: {seen:?}");
    assert!(
        tentacle_states.contains("Idle"),
        "tentacles never surfaced: {tentacle_states:?}"
    );
}

#[test]
fn cthulhu_damage_stages_and_death_complete_the_fight() {
    let Some(mut s) = session() else { return };
    // Out of Devour's reach (Terminate ignores immortality).
    s.world.players[0].as_mut().unwrap().character.ethereal = true;
    for _ in 0..600 {
        step(&mut s);
    }
    let zone = host(&s).zone;
    hurt(&mut s, zone, 1000.);
    let hp = host(&s).cthulhu.status.hit_points;
    assert!(
        (hp - (MAX_HIT_POINTS - 1000.)).abs() < 1.,
        "arcane is unresisted: {hp}"
    );
    // Water is immune.
    s.world
        .damage_physics(
            zone,
            Damage {
                attack: attack::DAMAGE,
                element: E::WATER.0,
                amount: 500.,
                magnitude: 1.,
            },
            15,
        )
        .unwrap();
    step(&mut s);
    assert_eq!(host(&s).cthulhu.status.hit_points, hp);
    for _ in 0..600 {
        if host(&s).cthulhu.status.hit_points <= 0. {
            break;
        }
        hurt(&mut s, zone, 100000.);
    }
    let mut died = false;
    // Away from the north point it submerges, re-emerges there and dies.
    for _ in 0..60 * 90 {
        step(&mut s);
        if host(&s).cthulhu.dead {
            died = true;
            break;
        }
    }
    let c = &host(&s).cthulhu;
    assert!(
        died,
        "die clip never finished ({:?} {:?} t={} fading={} finished={} playing={})",
        c.state,
        c.rig.controller.clip_name(),
        c.rig.controller.time,
        c.rig.fading(),
        c.rig.finished(),
        c.rig.controller.is_playing()
    );
    assert!(host(&s)
        .cthulhu
        .tentacles
        .iter()
        .filter(|t| t.active)
        .all(|t| t.state == TentacleState::Die));
    // The BossDead trigger's phony LevelComplete froze the entities 7.66 s
    // into the die clip; it clears itself and the clip completes.
    assert_eq!(
        s.end_game.condition,
        magicka_runtime::end_game::EndGameCondition::None
    );
    for _ in 0..30 {
        step(&mut s);
    }
    assert!(s.world.boss_fight.dead);
}

#[test]
fn a_killed_tentacle_submerges_and_regrows() {
    let Some(mut s) = session() else { return };
    for _ in 0..240 {
        step(&mut s);
    }
    let (index, handle) = host(&s)
        .cthulhu
        .tentacles
        .iter()
        .enumerate()
        .find(|(_, t)| t.active && !t.dead)
        .map(|(i, _)| (i, host(&s).tentacles[i]))
        .expect("no live tentacle");
    hurt(&mut s, handle, TENTACLE_HIT_POINTS * 2.);
    let mut submerged = false;
    let mut regrown = false;
    for _ in 0..60 * 30 {
        step(&mut s);
        let t = &host(&s).cthulhu.tentacles[index];
        submerged |= t.state == TentacleState::Submerge;
        if submerged && t.state == TentacleState::Emerge {
            regrown = true;
            break;
        }
    }
    assert!(submerged, "the tentacle never submerged");
    assert!(regrown, "the tentacle never re-emerged");
    assert_ne!(host(&s).cthulhu.state, State::Death);
}
