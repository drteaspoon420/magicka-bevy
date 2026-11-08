use magicka_content::character::ActionKind;
use magicka_runtime::{play_content::load_character, play_session::PlaySession};

#[test]
fn authored_ethereal_windows_inventory() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut count = 0;
    for name in ["Daemon_lesser", "Daemon_greater", "Ghost_Acolyte"] {
        let t = load_character(&root, name).unwrap();
        eprintln!("{name} abilities: {:?}", t.abilities);
        for a in t.animation_sets.iter().flatten() {
            for action in &a.actions {
                if let ActionKind::Ethereal { .. } = action.kind {
                    count += 1;
                    eprintln!(
                        "{name}/{}: {}..{} {:?}",
                        a.animation, action.start_time, action.end_time, action.kind
                    );
                }
            }
        }
    }
    assert!(count > 0);
}

#[test]
fn daemon_ai_has_corporeal_windows_and_returns_to_ethereal() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    for name in ["Daemon_lesser", "Daemon_greater"] {
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.disable_scripts();
        s.world.flags.cutscene = false;
        s.world.flags.input_limited = false;
        for _ in 0..60 {
            s.step(1. / 60.);
        }
        let h = s.spawn_npc(name, "attack", 5.).unwrap();
        // Isolate the two authored transition abilities from damaging attacks,
        // retaining their native range, score expressions and cooldowns.
        let n = s.world.npcs.get_mut(&h).unwrap();
        n.combat.abilities = std::sync::Arc::new(
            n.combat
                .abilities
                .iter()
                .filter(|a| {
                    a.animations.iter().any(|&key| {
                        ["spec_materialize", "spec_etherealize"].iter().any(|name| {
                            magicka_runtime::animation_controller::animation_value(name)
                                == Some(key)
                        })
                    })
                })
                .cloned()
                .collect(),
        );
        let mut corporeal = false;
        let mut restored = false;
        for frame in 0..1200 {
            s.step(1. / 60.);
            let n = &s.world.npcs[&h];
            if !n.character.ethereal && !corporeal {
                corporeal = true;
                let position = s.world.physics.bodies[n.body].position;
                let player = s.avatar().body;
                s.world.physics.bodies[player].position =
                    [position[0] + 20., position[1], position[2]];
                // Greater daemon's authored score includes !s.Health.
                let n = s.world.npcs.get_mut(&h).unwrap();
                n.character.combat.health.hit_points *= 0.5;
            }
            let n = &s.world.npcs[&h];
            if corporeal && n.character.ethereal {
                restored = true;
            }
            if frame % 30 == 0 {
                eprintln!(
                    "{name} frame={frame} clip={:?} time={} ethereal={} alpha={}",
                    n.motion.controller.clip_name(),
                    n.motion.controller.time,
                    n.character.ethereal,
                    n.character.ethereal_alpha
                );
            }
            if restored {
                break;
            }
        }
        assert!(
            corporeal && restored,
            "{name}: corporeal={corporeal} restored={restored}"
        );
    }
}
