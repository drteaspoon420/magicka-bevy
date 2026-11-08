//! The reference mod example.lua_magicks replaces all 49 built-in Magicks
//! with Lua written on the world operations. Every one is cast through the
//! native entry point (as any caster would): no script or argument errors,
//! no native effect ran in its place, and a sample of effects are checked.
use magicka_runtime::{mod_runtime::ModRuntime, play_session::PlaySession};
use magicka_sim::spells::MagickType as M;
use std::path::PathBuf;

/// Outcomes the native Magick also has (no target, refused, already on).
fn expected_refusal(line: &str) -> bool {
    [
        "did not apply",
        "did not start",
        "refused",
        "already active",
        "no target",
    ]
    .iter()
    .any(|r| line.contains(r))
}

#[test]
fn every_builtin_magick_runs_from_lua() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut s = PlaySession::load(
        &root.join("../../../../target/Magicka/Content"),
        "Havindr.lvl",
        "Wizard",
    )
    .unwrap();
    s.disable_scripts();
    let mut mods = ModRuntime::load(
        None,
        &root.join("../../mods"),
        Some(&["example.lua_magicks".into()]),
        7,
    )
    .unwrap();
    mods.attach(&mut s);
    assert_eq!(s.world.scripted_magicks.count_ones(), 49);
    let tick = |s: &mut PlaySession, m: &mut ModRuntime| {
        m.step(s, 1.0 / 60.0);
        s.step(1.0 / 60.0);
    };
    tick(&mut s, &mut mods);
    // A sturdy caster (above CTD's 100000 limit) and goblins to target.
    {
        let health = &mut s.world.players[0].as_mut().unwrap().character.combat.health;
        health.max_hit_points = 1.0e6;
        health.hit_points = 1.0e6;
    }
    let me = s.avatar().handle;
    let at = s.world.physics.bodies[s.avatar().body].position;
    let facing = s.world.physics.bodies[s.avatar().body].direction();
    for i in 0..4 {
        let d = 4.0 + i as f32 * 1.5;
        s.spawn_npc_at(
            "Goblin_shaman",
            [at[0] + facing[0] * d, at[1], at[2] + facing[2] * d],
        )
        .unwrap();
    }
    let legacies: Vec<u8> = mods
        .definitions
        .magicks
        .iter()
        .filter_map(|m| m.legacy)
        .collect();
    assert_eq!(legacies.len(), 49);
    let mut failures = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    // CTD and Polymorph change the cast; cast them last.
    let mut order: Vec<u8> = legacies
        .iter()
        .copied()
        .filter(|l| ![M::Ctd as u8, M::Polymorph as u8].contains(l))
        .collect();
    order.extend([M::Polymorph as u8, M::Ctd as u8]);
    for legacy in order {
        let magick = M::try_from(legacy).unwrap();
        let log_start = s.world.log.len();
        // A goblin touching the caster breaks invisibility at once
        // (character_contact), and panicking goblins now stumble into it
        // within the two ticks (npc-agents.md, PanicState). Hold them 15 m
        // away for this cast and put them back afterwards.
        let parked: Option<Vec<(usize, [f32; 3])>> = (magick == M::Invisibility).then(|| {
            let bodies: Vec<usize> = s.world.npcs.values().map(|n| n.body).collect();
            bodies
                .into_iter()
                .map(|b| {
                    let body = &mut s.world.physics.bodies[b];
                    let p = body.position;
                    body.position[0] += 15.0;
                    (b, p)
                })
                .collect()
        });
        s.world.cast_magick_from(me, magick);
        assert_eq!(
            s.world.scripted_magick_casts.len(),
            1,
            "{magick:?} queued for Lua instead of running natively"
        );
        // Effects are checked right after the handler ran (the goblins may
        // break invisibility and the like a moment later).
        for _ in 0..2 {
            tick(&mut s, &mut mods);
        }
        if let Some(parked) = parked {
            for (b, p) in parked {
                s.world.physics.bodies[b].position = p;
            }
        }
        match magick {
            M::Haste => assert!(
                s.world.ability_effects.hasted(me),
                "haste: {:?}",
                &s.world.log[log_start..]
            ),
            M::Invisibility => {
                assert!(
                    s.avatar().character.is_invisible(),
                    "invisibility: {:?}",
                    &s.world.log[log_start..]
                )
            }
            M::Rain => assert!(s.world.ability_effects.rain_state().is_some(), "rain"),
            M::Blizzard => assert!(s.world.ability_effects.blizzard_state().is_some()),
            M::TimeWarp => assert!(s.world.ability_effects.time_warp_state().is_some()),
            M::Charm => assert!(
                s.world.npcs.values().any(|n| n.character.is_charmed()),
                "a goblin was charmed"
            ),
            M::MeteorS => {
                assert!(
                    s.world.missiles.iter().any(|m| m.body.position[1] > 20.0),
                    "meteors falling"
                );
                assert_eq!(s.world.light_target_intensity, 0.4);
            }
            M::SUndead => {
                assert!(
                    s.world.entities.iter().any(|e| e
                        .template_name
                        .as_deref()
                        .is_some_and(|t| t.contains("skeleton") || t.contains("ghoul"))),
                    "an undead was summoned"
                )
            }
            _ => {}
        }
        for _ in 0..18 {
            tick(&mut s, &mut mods);
        }
        for line in &s.world.log[log_start..] {
            let script = line.starts_with("mod ") || line.starts_with("magick ");
            if script && !expected_refusal(line) {
                failures.push(format!("{magick:?}: {line}"));
            } else if script {
                eprintln!("{magick:?} refused: {line}");
            }
        }
        seen.insert(legacy);
    }
    assert_eq!(seen.len(), 49);
    assert!(failures.is_empty(), "{failures:#?}");
}
