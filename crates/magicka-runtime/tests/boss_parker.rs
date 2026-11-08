use magicka_runtime::{
    boss_parker::{Stage, State, MAX_HIT_POINTS},
    play_session::PlaySession,
    scene_world::{boss_live::BossRef, parker_live::ParkerHost},
};
use magicka_sim::{
    damage::{attack, Damage},
    spells::Elements as E,
};
use std::path::Path;
const DT: f32 = 1. / 60.;

fn host(s: &PlaySession) -> &ParkerHost {
    s.world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            BossRef::Parker(p) => Some(p.as_ref()),
            _ => None,
        })
        .expect("Parker missing")
}
fn session() -> Option<PlaySession> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Tsar/Tsar_WizardCastle.lvl", "Wizard").unwrap();
    s.change_scene_named("twc_s3bf", "respawn_pre_boss")
        .unwrap();
    s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
    step(&mut s);
    s.run_trigger("Bossfight start", true).unwrap();
    for _ in 0..120 {
        step(&mut s);
    }
    Some(s)
}
fn step(s: &mut PlaySession) {
    s.step(DT);
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
}
fn hurt(s: &mut PlaySession, amount: f32) {
    let h = host(s).abdomen;
    s.world
        .damage_physics(
            h,
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
fn parker_intro_reaches_battle_and_stages_follow_health() {
    let Some(mut s) = session() else { return };
    assert!(!s.world.log.iter().any(|l| l.contains("parker: stand-in")));
    assert_eq!(host(&s).parker.status.hit_points, MAX_HIT_POINTS);
    let mut battle = false;
    for _ in 0..1200 {
        step(&mut s);
        battle |= host(&s).parker.state != State::Intro;
    }
    assert!(battle, "Parker never left the intro");
    // Arcane on the abdomen: multiplier 1.25.
    hurt(&mut s, 4000.);
    let hp = host(&s).parker.status.hit_points;
    assert!(hp < MAX_HIT_POINTS, "abdomen took no damage");
    for _ in 0..2 {
        step(&mut s);
    }
    assert_eq!(host(&s).parker.stage, Stage::Battle);
    assert!(s.world.boss_fight.normalized_health < 1.0);
}

#[test]
fn parker_dies_after_the_die_clip_and_the_scene_completes() {
    let Some(mut s) = session() else { return };
    for _ in 0..600 {
        step(&mut s);
    }
    // Damage is refused in the cave states; retry until it lands.
    for _ in 0..600 {
        if host(&s).parker.status.hit_points <= 0. {
            break;
        }
        hurt(&mut s, 100000.);
    }
    assert!(host(&s).parker.status.hit_points <= 0.);
    let mut dead = false;
    for _ in 0..600 {
        step(&mut s);
        if host(&s).parker.dead {
            dead = true;
            break;
        }
    }
    assert!(dead, "die clip never finished");
    assert_eq!(host(&s).parker.state, State::Dead);
    for _ in 0..30 {
        step(&mut s);
    }
    assert!(s.world.boss_fight.dead);
}

/// With a mortal avatar in front: the web entangles it, Parker rushes and
/// bites, backs up, and retreats to the cave once every player is down.
#[test]
fn parker_webs_entangle_then_rushes_and_bites() {
    let Some(mut s) = session() else { return };
    s.world.players[0].as_mut().unwrap().character.immortal_time = 0.;
    let mut seen = std::collections::BTreeSet::new();
    let mut entangled = 0;
    let mut webs = 0;
    for _ in 0..60 * 60 {
        step(&mut s);
        seen.insert(format!("{:?}", host(&s).parker.state));
        webs = webs.max(s.world.web_strands.slots.iter().filter(|w| w.added).count());
        let a = s.world.players[0].as_ref().unwrap();
        entangled += s.world.physics.bodies[a.body].owner.entangled as usize;
    }
    assert_eq!(webs, 10, "one web is ten strands");
    assert!(entangled > 0, "the web never entangled the avatar");
    for state in ["Web", "Rush", "Bite", "Backup"] {
        assert!(seen.contains(state), "{state} missing from {seen:?}");
    }
}
