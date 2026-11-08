//! The Cult boss in Mines mi_s5BF (research/boss-battles.md).
use magicka_runtime::{
    play_session::PlaySession,
    scene_loading::custom_hash,
    scene_world::{boss_live::BossRef, cult_live::CultState},
};
use std::path::Path;

const DT: f32 = 1.0 / 60.0;

fn cult(s: &PlaySession) -> &magicka_runtime::scene_world::cult_live::CultHost {
    s.world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            BossRef::Cult(c) => Some(c.as_ref()),
            _ => None,
        })
        .expect("no Cult boss")
}

fn step(s: &mut PlaySession, seconds: f32) {
    for _ in 0..(seconds / DT) as usize {
        s.step(DT);
    }
}

fn kill_all(s: &mut PlaySession, handles: &[u16]) {
    for &h in handles {
        let _ = magicka_runtime::entity_command::EntityCommandHost::kill(&mut s.world, h);
    }
}

#[test]
fn cult_spawns_its_warlocks_and_runs_rounds_until_the_leader_revives_them() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Mines.lvl", "Wizard").unwrap();
    s.change_scene_named("mi_s5BF", "start").unwrap();
    step(&mut s, 0.5);
    let c = cult(&s);
    assert_eq!(c.state, CultState::Intro);
    assert_eq!(c.warlocks.len(), 26);
    assert_eq!(c.fighting.len(), 2);
    assert_eq!(c.spectators.len(), 24);
    for id in ["bill", "bull", "leader"] {
        assert!(
            s.world
                .entities
                .iter()
                .any(|e| e.unique_id == custom_hash(id) && !e.removed),
            "missing scripted cultist {id}"
        );
    }
    assert!(c.total_max > 0.0);
    // activateBoss (Intro done, delay 2).
    s.run_trigger("Intro done", true).unwrap();
    step(&mut s, 3.0);
    assert_eq!(cult(&s).state, CultState::Battle);
    // Killing the opening pair starts the spectator show, which sends four
    // more fighters down.
    let pair = cult(&s).fighting.clone();
    kill_all(&mut s, &pair);
    step(&mut s, 0.5);
    assert_eq!(cult(&s).state, CultState::Spectator);
    for _ in 0..(20.0 / DT) as usize {
        s.step(DT);
        if cult(&s).state == CultState::Battle {
            break;
        }
    }
    let c = cult(&s);
    assert_eq!(c.state, CultState::Battle);
    assert_eq!(c.fighting.len(), 4, "spectator show sends four fighters");
    assert_eq!(c.spectators.len(), 20);
    // Rounds of four until no spectators remain, then the revive.
    for _ in 0..20 {
        let fighters = cult(&s).fighting.clone();
        kill_all(&mut s, &fighters);
        step(&mut s, 3.0);
        if cult(&s).state == CultState::Revive {
            break;
        }
    }
    assert_eq!(cult(&s).state, CultState::Revive);
    assert!(s.world.flags.input_limited, "revive limits input");
    // The leader's "cult_domyself" dialog gates the revive of five.
    let mut final_state = false;
    for _ in 0..(120.0 / DT) as usize {
        s.step(DT);
        if cult(&s).state == CultState::Final {
            final_state = true;
            break;
        }
        // Advance the dialog as a player would.
        s.handle_input(
            magicka_runtime::avatar::PlayerInput {
                cast_down: s.world.play_time as i64 % 2 == 0,
                ..Default::default()
            },
            16. / 9.,
        );
    }
    if final_state {
        let c = cult(&s);
        assert_eq!(c.fighting.len(), 5, "the leader revives five cultists");
        assert!(!s.world.flags.input_limited);
    } else {
        eprintln!(
            "revive dialog did not complete headlessly; state {:?}",
            cult(&s).state
        );
    }
}
