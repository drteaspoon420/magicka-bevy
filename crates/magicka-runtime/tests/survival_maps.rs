//! Actual challenge content, driven through the scene host. Combat is excluded
//! from the broad sweep so assertions measure wave dispatch and progression.
use magicka_runtime::{
    play_session::PlaySession, ruleset::RulesetKind, scene_loading::custom_hash,
    scene_script::ActionDefinition,
};
use std::path::PathBuf;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

#[test]
fn every_survival_map_dispatches_its_first_wave_and_keeps_future_waves_idle() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let mut levels: Vec<_> = std::fs::read_dir(root.join("Levels/Challenges"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lvl")))
        .collect();
    levels.sort();
    let mut checked = 0;
    for path in levels {
        let level = format!("Challenges/{}", path.file_name().unwrap().to_string_lossy());
        let mut s = PlaySession::load(&root, &level, "Wizard").unwrap();
        let Some(layout) = s
            .runtime
            .layout()
            .ruleset
            .clone()
            .filter(|r| r.kind == RulesetKind::Survival)
        else {
            continue;
        };
        checked += 1;
        assert!(s.script_error.is_none(), "{level}: {:?}", s.script_error);
        assert!(s.world.survival().is_some(), "{level}: ruleset unattached");
        let first: Vec<_> = layout.waves[0]
            .iter()
            .flat_map(|a| {
                s.runtime
                    .trigger(a.trigger)
                    .unwrap()
                    .actions
                    .iter()
                    .flatten()
                    .copied()
            })
            .collect();
        let future: Vec<_> = layout
            .waves
            .iter()
            .skip(1)
            .flatten()
            .flat_map(|a| {
                s.runtime
                    .trigger(a.trigger)
                    .unwrap()
                    .actions
                    .iter()
                    .flatten()
                    .copied()
            })
            .collect();
        let expected: usize = first
            .iter()
            .filter_map(|&i| match &s.runtime.layout().actions[i] {
                ActionDefinition::Spawn(a) => Some(a.amount as usize),
                _ => None,
            })
            .sum();
        let delay = layout.waves[0]
            .iter()
            .map(|a| {
                let action_delay = s
                    .runtime
                    .trigger(a.trigger)
                    .unwrap()
                    .actions
                    .iter()
                    .flatten()
                    .filter_map(|&i| s.runtime.queue(i).map(|q| q.delay))
                    .fold(0.0, f32::max);
                a.delay + action_delay
            })
            .fold(0.0, f32::max);
        for _ in 0..((delay + 1.0) * 20.0).ceil() as usize {
            s.world
                .step(&mut s.runtime, 0.05)
                .unwrap_or_else(|e| panic!("{level}: {e:?}"));
        }
        let actual: usize = first
            .iter()
            .filter_map(|&i| s.runtime.spawn_state(i).map(|state| state.spawned.len()))
            .sum();
        assert_eq!(actual, expected, "{level}: first-wave spawn count");
        assert!(expected > 0, "{level}: first wave must exercise spawning");
        for &i in &future {
            let q = s.runtime.queue(i).unwrap();
            assert_eq!(
                q.state,
                Default::default(),
                "{level}: future action {i} ticked"
            );
            assert!(
                !s.runtime.history().contains(&i),
                "{level}: future action fired"
            );
        }
        for h in first
            .iter()
            .filter_map(|&i| s.runtime.spawn_state(i))
            .flat_map(|state| &state.spawned)
            .filter_map(|spawn| spawn.handle)
        {
            let e = s.world.entities.iter().find(|e| e.handle == h).unwrap();
            assert_eq!(e.faction, 1, "{level}: survival Spawn forces EVIL");
            assert_eq!(e.original_faction, 1, "{level}: uncharmed faction count");
            assert!(e.position.iter().all(|v| v.is_finite()));
        }
        assert_eq!(
            s.world.survival().unwrap().wave_number,
            1,
            "{level}: living enemies must hold the wave"
        );
        // Real input-independent GetLocator(any) now uses the ruleset areas.
        use magicka_runtime::spawn_action::SpawnHost;
        assert!(s
            .world
            .locator(custom_hash("any"))
            .unwrap()
            .iter()
            .all(|v| v.is_finite()));
        eprintln!(
            "{level}: wave 1 spawned {actual}, {} later actions idle",
            future.len()
        );
    }
    assert_eq!(checked, 17, "installed survival map census changed");
}

#[test]
fn live_session_runs_challenge_waves_without_forcing_a_scene_trigger() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, "Challenges/ch_glade.lvl", "Wizard").unwrap();
    assert!(s.world.npcs.is_empty());
    s.step(1.0 / 60.0);
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert_eq!(s.world.npcs.len(), 6);
    let ratio = s.world.survival().unwrap().hit_point_percentage(|h| {
        s.world
            .entities
            .iter()
            .find(|e| e.handle == h)
            .map(|e| e.hit_points)
    });
    assert!(
        (ratio - 1.0).abs() < 1e-6,
        "wave health replaced pre-read share: {ratio}"
    );
}

/// A script progression sweep, not a combat or native parity test. The harness
/// removes hostile actors after each script frame to exercise every authored
/// wave, its delays, ordinary triggers called from waves and final victory.
#[test]
#[ignore = "full 316-wave content sweep"]
fn every_survival_wave_can_execute_and_finish() {
    use magicka_runtime::entity_command::EntityCommandHost;
    let Some(root) = content() else {
        return;
    };
    let mut paths: Vec<_> = std::fs::read_dir(root.join("Levels/Challenges"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lvl")))
        .collect();
    paths.sort();
    let mut total = 0;
    let mut failures = Vec::new();
    for path in paths {
        let level = format!("Challenges/{}", path.file_name().unwrap().to_string_lossy());
        let mut s = PlaySession::load(&root, &level, "Wizard").unwrap();
        let Some(layout) = s
            .runtime
            .layout()
            .ruleset
            .clone()
            .filter(|r| r.kind == RulesetKind::Survival)
        else {
            continue;
        };
        let result = (|| -> Result<(), String> {
            if let Some(e) = &s.script_error {
                return Err(e.clone());
            }
            for _ in 0..(layout.waves.len() * 2400) {
                s.world
                    .step(&mut s.runtime, 0.05)
                    .map_err(|e| format!("{e:?}"))?;
                let hostiles: Vec<_> = s
                    .world
                    .entities
                    .iter()
                    .filter(|e| e.added && !e.removed && s.world.faction(e) & 11 != 0)
                    .map(|e| e.handle)
                    .collect();
                for h in hostiles {
                    s.world.terminate(h)?;
                }
                if s.world.survival().unwrap().victory {
                    for a in layout.waves.iter().flatten() {
                        assert!(s.runtime.wave_actions_finished(a.trigger));
                        for &i in s
                            .runtime
                            .trigger(a.trigger)
                            .unwrap()
                            .actions
                            .iter()
                            .flatten()
                        {
                            if !s.runtime.history().contains(&i) {
                                return Err(format!("action {i} never executed"));
                            }
                        }
                    }
                    return Ok(());
                }
            }
            Err(format!(
                "stalled at wave {}",
                s.world.survival().unwrap().wave_number
            ))
        })();
        match result {
            Ok(()) => {
                total += layout.waves.len();
                eprintln!("{level}: {} waves finished", layout.waves.len());
            }
            Err(e) => {
                let error = format!(
                    "{level}: wave {}: {e}",
                    s.world.survival().unwrap().wave_number
                );
                eprintln!("{error}");
                failures.push(error);
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(total, 316, "installed survival wave census changed");
}
