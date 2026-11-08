//! Restart(Checkpoint) on retail Havindr scene 3 (research/checkpoints.md).
use magicka_runtime::play_session::PlaySession;
use magicka_runtime::scene_flow::SpawnPoint;
use magicka_runtime::scene_loading::custom_hash;
use std::path::{Path, PathBuf};

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

fn npc_count(s: &PlaySession) -> usize {
    s.world.npcs.values().filter(|n| !n.dead()).count()
}

fn checkpoint_action() -> magicka_runtime::scene_flow::SceneFlowAction {
    use magicka_runtime::scene_script::{ActionDefinition, SceneScriptLayout};
    let layout = SceneScriptLayout::read(
        br#"<Scene><Trigger id="checkpoint"><Then><TriggerCheckpoint spawnpoint="start" spawnfairy="false" savetodisk="false"/></Then></Trigger></Scene>"#,
        false,
    ).unwrap();
    match layout.actions.into_iter().next().unwrap() {
        ActionDefinition::Flow(action) => *action,
        other => panic!("expected checkpoint, got {other:?}"),
    }
}

#[test]
fn checkpoint_heals_living_players_and_revives_each_dead_player_at_its_locator() {
    use magicka_runtime::avatar::AvatarState;
    use magicka_sim::{damage_delivery::NetworkRole, magicks::AbilityEffect};
    let Some(root) = content() else { return };
    for role in [
        NetworkRole::Offline,
        NetworkRole::Server,
        NetworkRole::Client,
    ] {
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.disable_scripts();
        for slot in 1..4 {
            s.join_player_at_start(slot, "Wizard").unwrap();
        }
        let defaults: Vec<_> = s.world.players[1]
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|(slot, item)| (*slot, item.definition.name.clone()))
            .collect();
        s.world.players[0]
            .as_mut()
            .unwrap()
            .character
            .combat
            .health
            .max_hit_points += 27.0;
        s.world.players[0]
            .as_mut()
            .unwrap()
            .character
            .combat
            .health
            .hit_points = 10.0;
        // Synthetic item identity fixture for the native staff_of_war branch.
        let living = s.world.players[3].as_mut().unwrap();
        living
            .items
            .iter_mut()
            .find(|(slot, _)| *slot == 1)
            .unwrap()
            .1
            .definition
            .name = "staff_of_war".into();
        living.character.combat.health.hit_points = 10.0;
        for slot in [1, 2] {
            s.set_player_loadout(
                slot,
                "Wizard",
                "staff_heirloom_grandfather_clock",
                &defaults[0].1,
            )
            .unwrap();
            let player = s.world.players[slot].as_mut().unwrap();
            player.character.combat.dead = true;
            player.character.combat.health.hit_points = 0.0;
            s.world.dead_age[slot] = if slot == 1 { 10.0 } else { 1.0 };
        }
        s.unlocked_magicks = [1 << 3; 4].into();
        s.world.role = role;
        let mut action = checkpoint_action();
        let matrices = action.initialize(&mut s.world).unwrap().unwrap();
        action.execute(Some(&matrices), &mut s.world).unwrap();
        assert_eq!(s.world.checkpoints.last(), Some(&matrices));
        for (slot, multiplier) in [(0, 1.0), (3, 2.0)] {
            let player = s.world.players[slot].as_ref().unwrap();
            let health = player.character.combat.health.max_hit_points * multiplier;
            assert_eq!(player.character.combat.health.hit_points, health);
            assert_eq!(s.world.physics.bodies[player.body].owner.hit_points, health);
        }
        assert!(s.world.audio_requests.iter().any(|cue| matches!(cue,
            magicka_runtime::audio::AudioRequest::Start { bank: 8, cue, .. }
                if *cue == custom_hash("ui_checkpoint01"))));
        if role == NetworkRole::Client {
            assert!(
                s.world.ability_effects.effects.is_empty(),
                "clients cannot schedule checkpoint spawns"
            );
            continue;
        }
        assert_eq!(s.world.ability_effects.effects.len(), 2);
        for slot in [1, 2] {
            let AbilityEffect::Revive(pending) = s.world.ability_effects.effects[slot - 1] else {
                panic!("expected revive")
            };
            assert_eq!(pending.revivee, slot);
            assert_eq!(pending.health, 1.0);
            let m = matrices[slot];
            let point = [m[12], m[13], m[14]];
            let snapped = s
                .world
                .nav
                .as_ref()
                .and_then(|n| n.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
                .map_or(point, |(_, point, _)| point);
            assert_eq!(pending.position, snapped);
        }
        // A repeated checkpoint must not duplicate an already reserved revive.
        action.execute(Some(&matrices), &mut s.world).unwrap();
        assert_eq!(s.world.ability_effects.effects.len(), 2);
        for _ in 0..17 {
            s.step(1.0 / 60.0);
        }
        for slot in [1, 2] {
            let player = s.world.players[slot].as_ref().unwrap();
            assert!(!player.dead());
            assert_eq!(player.motion.state, AvatarState::Resurrection);
            assert_eq!(
                player.character.combat.health.hit_points,
                player.character.combat.health.max_hit_points
            );
            assert_eq!(
                player
                    .items
                    .iter()
                    .map(|(slot, item)| (*slot, item.definition.name.clone()))
                    .collect::<Vec<_>>(),
                defaults
            );
            assert_eq!(s.unlocked_magicks[slot], 1 << 3);
            assert!(!s.world.ability_effects.ressing(slot));
        }
    }
}

#[test]
fn specific_revive_never_falls_back_to_a_different_dead_player() {
    use magicka_sim::magicks::AbilityEffect;
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for slot in [1, 2] {
        s.join_player_at_start(slot, "Wizard").unwrap();
        let player = s.world.players[slot].as_mut().unwrap();
        player.character.combat.dead = true;
        player.character.combat.health.hit_points = 0.0;
    }
    s.world.dead_age = [0.0, 10.0, 1.0, 0.0].into();
    for slot in [0, 3, 4, usize::MAX] {
        assert!(!s.world.revive_specific(slot, [0.0; 3]));
        assert!(s.world.ability_effects.effects.is_empty());
    }
    for age in [0.0, f32::from_bits(1), f32::NAN] {
        s.world.dead_age[2] = age;
        assert!(!s.world.revive_specific(2, [0.0; 3]));
        assert!(s.world.ability_effects.effects.is_empty());
    }
    s.world.dead_age[2] = 1.0;
    assert!(s.world.revive_specific(2, [0.0; 3]));
    let AbilityEffect::Revive(pending) = s.world.ability_effects.effects[0] else {
        panic!("expected revive")
    };
    assert_eq!((pending.revivee, pending.health), (2, 1.0));
    assert!(!s.world.revive_specific(2, [0.0; 3]));
    assert_eq!(s.world.ability_effects.effects.len(), 1);
    assert!(!s.world.ability_effects.ressing(1));
}

#[test]
fn dormant_player_stays_ineligible_across_checkpoint_scene_and_campaign_reloads() {
    let Some(root) = content() else { return };
    let level = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/feature-lab.lvl");
    let mut s = PlaySession::load(&root, level.to_str().unwrap(), "Wizard").unwrap();
    s.step(1.0 / 60.0);
    s.disable_scripts();
    s.join_player_at_start(1, "Wizard").unwrap();
    s.depart_network_player(0).unwrap();
    let check = |s: &mut PlaySession| {
        assert!(!s.avatar().playing);
        assert!(!s.body().active);
        assert!(s
            .world
            .combatants()
            .iter()
            .all(|p| p.handle != s.avatar().handle));
        assert!(!s.world.revive_specific(0, [0.0; 3]));
        assert!(!magicka_runtime::scene_flow::SceneFlowHost::players(&s.world)[0].playing);
        assert!(
            !magicka_runtime::world_condition::WorldConditionFacts::players(&s.world)[0].playing
        );
        assert_eq!(
            magicka_runtime::script_runtime::ScriptWorld::facts(&s.world).player_count,
            1
        );
    };
    check(&mut s);
    assert!(s.restart_at_checkpoint().unwrap());
    check(&mut s);
    let scene = s.scene_name().to_string();
    s.change_scene_named(&scene, "start").unwrap();
    check(&mut s);
    s.change_level("Battlefield.lvl").unwrap();
    check(&mut s);
    s.fresh_network_player_at_start(0).unwrap();
    assert!(s.avatar().playing);
    assert!(s.body().active);
}

#[test]
fn client_revive_timer_cannot_create_an_authoritative_avatar() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.join_player_at_start(1, "Wizard").unwrap();
    let player = s.world.players[1].as_mut().unwrap();
    let handle = player.handle;
    player.character.combat.dead = true;
    player.character.combat.health.hit_points = 0.0;
    s.world.dead_age[1] = 1.0;
    s.world.role = magicka_sim::damage_delivery::NetworkRole::Client;
    assert!(s.world.revive_specific(1, [50.0, 0.0, 50.0]));
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    let player = s.world.players[1].as_ref().unwrap();
    assert_eq!(player.handle, handle);
    assert!(player.dead());
    assert_eq!(player.character.combat.health.hit_points, 0.0);
    assert!(s.world.revive_spawns.is_empty());
    assert_ne!(
        player.motion.state,
        magicka_runtime::avatar::AvatarState::Resurrection
    );
}

#[test]
fn explicit_respawn_cancels_pending_and_already_queued_old_player_spawns() {
    use magicka_sim::magicks::AbilityEffect;
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.join_player_at_start(1, "Wizard").unwrap();
    let player = s.world.players[1].as_mut().unwrap();
    let handle = player.handle;
    player.character.combat.dead = true;
    player.character.combat.health.hit_points = 0.0;
    s.world.dead_age[1] = 1.0;
    assert!(s.world.revive_specific(1, [50.0, 0.0, 50.0]));
    let AbilityEffect::Revive(pending) = s.world.ability_effects.effects[0] else {
        panic!("expected revive")
    };
    s.world.revive_spawns.push(pending);
    s.world.ability_effects.haste(handle, None);
    let other = s.avatar().handle;
    s.world.ability_effects.haste(other, None);
    s.respawn_player(1, "Wizard").unwrap();
    assert!(!s.world.ability_effects.ressing(1));
    assert!(s.world.revive_spawns.is_empty());
    assert_eq!(s.world.ability_effects.effects.len(), 1);
    assert!(
        matches!(s.world.ability_effects.effects[0], AbilityEffect::Haste(h) if h.owner == other)
    );
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    let player = s.world.players[1].as_ref().unwrap();
    assert!(player.playing);
    assert!(!player.dead());
    assert_ne!(
        player.motion.state,
        magicka_runtime::avatar::AvatarState::Resurrection
    );
    assert_eq!(
        player.character.combat.health.hit_points,
        player.character.combat.health.max_hit_points
    );
}

#[test]
fn checkpoint_restart_restores_scene_spawns_and_avatar() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene(SpawnPoint {
        scene: custom_hash("havindr_s3"),
        locations: std::array::from_fn(|i| custom_hash(&format!("start{i}"))),
        spawn_players: true,
    })
    .unwrap();
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    let cp = s.checkpoint.clone().expect("StartOfLevel1 checkpoint");
    assert_eq!(cp.scene, custom_hash("havindr_s3"));
    let spawned = npc_count(&s);
    assert!(spawned > 0, "StartOfLevel2 spawns");
    let home = s.body().position;
    // Wander off, lose health, then die.
    let index = s.avatar().body;
    let body = &mut s.world.physics.bodies[index];
    let orientation = body.orientation;
    body.move_to([home[0] + 15.0, home[1], home[2]], orientation);
    // Single player: the checkpoint's revival fairy revives the first death
    // (research/revival-fairy.md); only the next death reaches Defeat.
    assert!(
        s.world.revival_fairies[0].active,
        "StartOfLevel1 arms the fairy"
    );
    s.world.players[0].as_mut().unwrap().terminate();
    for _ in 0..600 {
        s.step(1.0 / 60.0);
        assert!(!s.end_game.restart, "fairy revival must not restart");
        if !s.dead() {
            break;
        }
    }
    assert!(!s.dead(), "the fairy revived the avatar");
    for _ in 0..60 {
        s.step(1.0 / 60.0);
    }
    // WaitingForAvatar deactivated it on the update after the revival.
    assert!(!s.world.revival_fairies[0].active, "one use per checkpoint");
    s.world.players[0].as_mut().unwrap().terminate();
    let mut restarted = false;
    for _ in 0..600 {
        s.step(1.0 / 60.0);
        if s.end_game.restart {
            assert!(s.restart_at_checkpoint().unwrap());
            restarted = true;
            break;
        }
    }
    assert!(restarted, "defeat reached Restart(Checkpoint)");
    assert_eq!(s.scene_name(), "havindr_s3");
    assert!(!s.dead());
    let p = s.body().position;
    let m = cp.matrices[0];
    let d = ((p[0] - m[12]).powi(2) + (p[2] - m[14]).powi(2)).sqrt();
    assert!(d < 0.05, "avatar {d} m from the checkpoint matrix");
    assert_eq!(npc_count(&s), spawned, "spawns replayed once");
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    assert_eq!(npc_count(&s), spawned, "no StartOfLevel re-run");
    assert!(s.checkpoint.is_some());
    // State.ApplyState re-armed the fairy captured by the checkpoint.
    assert!(s.world.revival_fairies[0].active);
}

/// User report 2026-11-05: the fountain warlock never returns after dying
/// following his dialog. Exercise the actual kill-gated wave chain twice.
#[test]
fn havindr_fountain_warlock_returns_after_checkpoint_defeat() {
    use magicka_runtime::give_order::GiveOrderHost;
    for (skip_intro, disk_reload) in [(false, 0), (true, 0), (false, 1), (false, 2)] {
        eprintln!("fountain case: skipped intro={skip_intro}, disk reload={disk_reload}");
        let Some(root) = content() else { return };
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.change_scene_named("havindr_s3", "start").unwrap();
        s.player_ids[0] = Some(magicka_runtime::saves::new_player_id().unwrap());
        s.headless_dialogs = Some([1280.0, 720.0]);
        let warlock = |s: &PlaySession| {
            s.world
                .entities
                .iter()
                .find(|e| {
                    e.added
                        && !e.removed
                        && !e.dead
                        && e.template_name
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case("warlock"))
                })
                .map(|e| e.handle)
        };
        let clear_wave = |s: &mut PlaySession| {
            let area = custom_hash("checkbox_encounter");
            s.world.update_present(area).unwrap();
            let targets = s.world.present_characters(area).unwrap().unwrap();
            let time = s.world.play_time;
            for target in targets.into_iter().filter(|c| c.original_faction & 1 != 0) {
                let Some(npc) = s.world.npcs.get_mut(&target.handle) else {
                    continue;
                };
                let body = &mut s.world.physics.bodies[npc.body];
                let position = body.position;
                magicka_runtime::avatar::deliver_to_body(
                    &mut npc.character,
                    body,
                    &[magicka_sim::resistance::Resistance::IDENTITY; 11],
                    false,
                    target.handle,
                    magicka_sim::damage_delivery::DamagePayload::Single(
                        magicka_sim::damage::Damage {
                            attack: magicka_sim::damage::attack::DAMAGE,
                            element: magicka_sim::spells::Elements::EARTH.0,
                            amount: 100000.0,
                            magnitude: 1.0,
                        },
                    ),
                    None,
                    position,
                    time,
                );
            }
        };
        for _ in 0..30 {
            s.step(1.0 / 60.0);
        }
        s.run_trigger(
            if skip_intro {
                "skip_encounterintro"
            } else {
                "encounter intro"
            },
            true,
        )
        .unwrap();
        for tick in 0..720 {
            s.step(1.0 / 60.0);
            if tick % 30 == 0 {
                s.world.advance_dialog(0);
            }
        }
        let checkpoint = s.checkpoint.clone().expect("market checkpoint");
        if disk_reload == 1 {
            s = PlaySession::load_disk_save(&root, &s.disk_save().unwrap()).unwrap();
            s.headless_dialogs = Some([1280.0, 720.0]);
        }
        for _ in 0..1800 {
            clear_wave(&mut s);
            s.step(1.0 / 60.0);
            if warlock(&s).is_some() {
                break;
            }
        }
        assert!(
            warlock(&s).is_some(),
            "warlock spawned after clearing waves"
        );
        for tick in 0..2400 {
            s.step(1.0 / 60.0);
            if tick % 30 == 0 {
                s.world.advance_dialog(0);
            }
            if has_fired(&s, custom_hash("end warlock cutscene"))
                && !s.world.flags.cutscene
                && !s.input_limited()
            {
                break;
            }
        }
        assert!(
            has_fired(&s, custom_hash("end warlock cutscene")),
            "warlock dialog completed"
        );
        if disk_reload == 2 {
            s = PlaySession::load_disk_save(&root, &s.disk_save().unwrap()).unwrap();
            s.headless_dialogs = Some([1280.0, 720.0]);
        }
        let mut restarted = false;
        for _ in 0..2 {
            s.world.players[0].as_mut().unwrap().terminate();
            for _ in 0..900 {
                s.step(1.0 / 60.0);
                if s.end_game.restart {
                    assert!(s.restart_at_checkpoint().unwrap());
                    restarted = true;
                    break;
                }
                if !s.dead() {
                    break;
                }
            }
            if restarted {
                break;
            }
            for _ in 0..60 {
                s.step(1.0 / 60.0);
            }
        }
        assert!(
            restarted,
            "deaths exhausted the fairy and restored the checkpoint"
        );
        assert_eq!(s.checkpoint.as_ref().unwrap().matrices, checkpoint.matrices);
        assert!(
            warlock(&s).is_none(),
            "checkpoint precedes the warlock spawn"
        );
        for _ in 0..1800 {
            clear_wave(&mut s);
            s.step(1.0 / 60.0);
            if warlock(&s).is_some() {
                break;
            }
        }
        assert_eq!(s.script_error, None);
        assert!(warlock(&s).is_some(), "warlock must return after clearing the replayed waves; counters market={} wave1={} wave2={} warlock={}, triggers {:?}",
        s.runtime.variables.counter(custom_hash("encounter_market")),
        s.runtime.variables.counter(custom_hash("wave_one")),
        s.runtime.variables.counter(custom_hash("wave_two")),
        s.runtime.variables.counter(custom_hash("warlock")),
        s.runtime.layout().triggers.iter().filter(|t| t.has_triggered).map(|t| t.name.as_str()).collect::<Vec<_>>()
    );
        eprintln!("warlock returned after actual defeat; skipped intro={skip_intro}, disk reload={disk_reload}");
    }
}

/// Optional private player-save reproduction; the source save is read only.
#[test]
fn private_havindr_fountain_save_replays_the_warlock_wave() {
    use magicka_runtime::entity_command::EntityCommand;
    let Some(path) = std::env::var_os("MAGICKA_CHECKPOINT_REPRO_SAVE") else {
        return;
    };
    let Some(root) = content() else { return };
    let saved = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut s = PlaySession::load_disk_save(&root, &saved).unwrap();
    assert_eq!(s.scene_name(), "havindr_s3");
    assert!(s.restart_at_checkpoint().unwrap());
    s.headless_dialogs = Some([1280.0, 720.0]);
    let kill = EntityCommand::read(
        "kill",
        [("factions", "evil"), ("area", "checkbox_encounter")],
    )
    .unwrap();
    let mut found = false;
    for tick in 0..1800 {
        kill.execute(&mut s.world).unwrap();
        s.step(1.0 / 60.0);
        if tick % 30 == 0 {
            s.world.advance_dialog(0);
        }
        found = s.world.entities.iter().any(|e| {
            e.added
                && !e.removed
                && !e.dead
                && e.template_name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case("warlock"))
        });
        if found {
            break;
        }
    }
    assert!(found, "saved checkpoint warlock missing; counters market={} wave1={} wave2={} warlock={}, fired {:?}, remaining {:?}",
        s.runtime.variables.counter(custom_hash("encounter_market")),
        s.runtime.variables.counter(custom_hash("wave_one")),
        s.runtime.variables.counter(custom_hash("wave_two")),
        s.runtime.variables.counter(custom_hash("warlock")),
        s.runtime.layout().triggers.iter().filter(|t| t.has_triggered).map(|t| t.name.as_str()).collect::<Vec<_>>(),
        s.world.combatants().into_iter().filter(|c| !c.avatar && !c.dead).map(|c| (c.handle,c.faction,c.position)).collect::<Vec<_>>()
    );
}

/// User report 2026-11-04: dying does not reset the triggers for the checkpoint respawn.
#[test]
fn triggers_fired_after_a_checkpoint_are_reset_by_the_restart() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene(SpawnPoint {
        scene: custom_hash("havindr_s3"),
        locations: std::array::from_fn(|i| custom_hash(&format!("start{i}"))),
        spawn_players: true,
    })
    .unwrap();
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    let fired = |s: &PlaySession| -> Vec<String> {
        s.runtime
            .layout()
            .triggers
            .iter()
            .filter(|t| t.has_triggered)
            .map(|t| t.name.clone())
            .collect()
    };
    let at_checkpoint = fired(&s);
    let later: Vec<String> = s
        .runtime
        .layout()
        .triggers
        .iter()
        .filter(|t| !t.has_triggered && t.autorun && !t.conditions.is_empty())
        .map(|t| t.name.clone())
        .take(3)
        .collect();
    assert!(!later.is_empty());
    for name in &later {
        assert!(s.run_trigger(name, true).unwrap(), "{name}");
    }
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    let before_death = fired(&s);
    for name in &later {
        assert!(before_death.contains(name), "{name} should have fired");
    }
    assert!(s.restart_at_checkpoint().unwrap());
    let after = fired(&s);
    assert_eq!(after, at_checkpoint, "exactly the checkpoint's triggers");
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    for name in &later {
        assert!(s.run_trigger(name, false).is_ok(), "{name} can run again");
    }
}

/// The same report as a player meets it: walk in, die, respawn, walk in again.
#[test]
fn an_area_trigger_fires_again_naturally_after_the_checkpoint_restart() {
    use magicka_runtime::scene_script::ConditionDefinition;
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene(SpawnPoint {
        scene: custom_hash("havindr_s3"),
        locations: std::array::from_fn(|i| custom_hash(&format!("start{i}"))),
        spawn_players: true,
    })
    .unwrap();
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    assert!(s.checkpoint.is_some());
    // Unfired area triggers whose only condition is one presence check.
    let names = s.world.area_names();
    let layout = s.runtime.layout();
    let area_of = |c: usize| match &layout.conditions[c] {
        ConditionDefinition::Presence(p) if !p.invert => names
            .iter()
            .find(|n| custom_hash(&n.to_ascii_lowercase()) == p.area)
            .cloned(),
        _ => None,
    };
    let candidates: Vec<(i32, String, String)> = layout
        .triggers
        .iter()
        .filter(|t| !t.has_triggered && t.autorun && t.repeat < 0.0)
        .filter(|t| t.conditions.len() == 1 && t.conditions[0].len() == 1)
        .filter_map(|t| Some((t.id, t.name.clone(), area_of(t.conditions[0][0])?)))
        .collect();
    assert!(!candidates.is_empty());
    let fired = |s: &PlaySession, id: i32| {
        s.runtime
            .layout()
            .triggers
            .iter()
            .any(|t| t.id == id && t.has_triggered)
    };
    let walk_in = |s: &mut PlaySession, area: &str| {
        let at = s.world.area_center(area).unwrap();
        let index = s.avatar().body;
        let body = &mut s.world.physics.bodies[index];
        let orientation = body.orientation;
        body.move_to(at, orientation);
        for _ in 0..10 {
            s.step(1.0 / 60.0);
        }
    };
    let mut checked = 0;
    for (id, name, area) in candidates.iter().take(4) {
        walk_in(&mut s, area);
        if !fired(&s, *id) {
            eprintln!("{name}: did not fire on entry, not usable here");
            continue;
        }
        assert!(s.restart_at_checkpoint().unwrap());
        assert!(!fired(&s, *id), "{name} still marked fired after restart");
        walk_in(&mut s, area);
        assert!(fired(&s, *id), "{name} did not fire again after restart");
        checked += 1;
        assert!(s.restart_at_checkpoint().unwrap());
        for _ in 0..30 {
            s.step(1.0 / 60.0);
        }
    }
    eprintln!("checked {checked} of {} area triggers", candidates.len());
    assert!(checked > 0);
}

/// User report 2026-11-04: the solo revival fairy does not follow the player.
#[test]
fn the_revival_fairy_stays_with_its_owner() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let walk_and_check = |s: &mut PlaySession, when: &str| {
        // Wherever the wizard goes, the effect stays within its orbit.
        for tick in 0..240 {
            if tick % 60 == 0 {
                let index = s.avatar().body;
                let body = &mut s.world.physics.bodies[index];
                let p = body.position;
                let orientation = body.orientation;
                body.move_to([p[0] + 2.0, p[1], p[2] + 1.0], orientation);
            }
            s.step(1.0 / 60.0);
        }
        let at = s.fairy_effect_position(0);
        let owner = s.body().position;
        eprintln!("{when}: fairy {at:?}, owner {owner:?}");
        let at = at.unwrap_or_else(|| panic!("{when}: no fairy effect"));
        let d = ((at[0] - owner[0]).powi(2) + (at[2] - owner[2]).powi(2)).sqrt();
        assert!(d < 3.0, "{when}: fairy {d} m from its owner");
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene(SpawnPoint {
        scene: custom_hash("havindr_s3"),
        locations: std::array::from_fn(|i| custom_hash(&format!("start{i}"))),
        spawn_players: true,
    })
    .unwrap();
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    assert!(s.world.revival_fairies[0].active);
    walk_and_check(&mut s, "after the checkpoint");
    assert!(s.restart_at_checkpoint().unwrap());
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    walk_and_check(&mut s, "after a checkpoint restart");
    s.change_scene(SpawnPoint {
        scene: custom_hash("havindr_s4"),
        locations: std::array::from_fn(|i| custom_hash(&format!("start{i}"))),
        spawn_players: true,
    })
    .unwrap();
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    walk_and_check(&mut s, "after a scene change");
}

/// Unfired single-condition area triggers of the current scene: (id, name, area).
fn area_triggers(s: &PlaySession) -> Vec<(i32, String, String)> {
    use magicka_runtime::scene_script::ConditionDefinition;
    let names = s.world.area_names();
    let layout = s.runtime.layout();
    layout
        .triggers
        .iter()
        .filter(|t| !t.has_triggered && t.autorun && t.repeat < 0.0)
        .filter(|t| t.conditions.len() == 1 && t.conditions[0].len() == 1)
        .filter_map(|t| {
            let ConditionDefinition::Presence(p) = &layout.conditions[t.conditions[0][0]] else {
                return None;
            };
            let area = names
                .iter()
                .find(|n| !p.invert && custom_hash(&n.to_ascii_lowercase()) == p.area)?;
            Some((t.id, t.name.clone(), area.clone()))
        })
        .collect()
}

fn has_fired(s: &PlaySession, id: i32) -> bool {
    s.runtime
        .layout()
        .triggers
        .iter()
        .any(|t| t.id == id && t.has_triggered)
}

fn walk_in(s: &mut PlaySession, area: &str) {
    let at = s.world.area_center(area).unwrap();
    let index = s.avatar().body;
    let body = &mut s.world.physics.bodies[index];
    let orientation = body.orientation;
    body.move_to(at, orientation);
    for _ in 0..10 {
        s.step(1.0 / 60.0);
    }
}

/// Before any checkpoint a defeat restarts the level (`restart_level`).
#[test]
fn area_triggers_fire_again_after_a_restart_without_a_checkpoint() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Mountaindale.lvl", "Wizard").unwrap();
    for _ in 0..60 {
        s.step(1.0 / 60.0);
    }
    assert!(
        s.checkpoint.is_none(),
        "this scene should have no checkpoint yet"
    );
    let mut checked = 0;
    for (id, name, area) in area_triggers(&s).into_iter().take(3) {
        walk_in(&mut s, &area);
        if !has_fired(&s, id) {
            continue;
        }
        s.restart_level().unwrap();
        for _ in 0..60 {
            s.step(1.0 / 60.0);
        }
        assert!(!has_fired(&s, id), "{name} still marked fired");
        walk_in(&mut s, &area);
        assert!(has_fired(&s, id), "{name} did not fire again");
        checked += 1;
        s.restart_level().unwrap();
        for _ in 0..60 {
            s.step(1.0 / 60.0);
        }
    }
    eprintln!("checked {checked}");
    assert!(checked > 0);
}

/// Dying in the next scene returns to the checkpoint's scene, and that next scene must
/// start over when reached again.
#[test]
fn a_later_scene_starts_over_after_dying_there() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let to = |s: &mut PlaySession, scene: &str| {
        s.change_scene(SpawnPoint {
            scene: custom_hash(scene),
            locations: std::array::from_fn(|i| custom_hash(&format!("start{i}"))),
            spawn_players: true,
        })
        .unwrap();
        for _ in 0..30 {
            s.step(1.0 / 60.0);
        }
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    to(&mut s, "havindr_s3");
    assert!(s.checkpoint.is_some());
    let checkpoint_scene = s.checkpoint.as_ref().unwrap().scene;
    to(&mut s, "havindr_s4");
    // A later scene's own checkpoint would make this a different case.
    if s.checkpoint.as_ref().unwrap().scene != checkpoint_scene {
        eprintln!("havindr_s4 took its own checkpoint");
    }
    let mut checked = 0;
    for (id, name, area) in area_triggers(&s).into_iter().take(3) {
        walk_in(&mut s, &area);
        if !has_fired(&s, id) {
            continue;
        }
        let in_later = s.checkpoint.as_ref().unwrap().scene == checkpoint_scene;
        assert!(s.restart_at_checkpoint().unwrap());
        for _ in 0..30 {
            s.step(1.0 / 60.0);
        }
        if in_later {
            assert_eq!(s.scene_name(), "havindr_s3");
            to(&mut s, "havindr_s4");
        }
        assert!(!has_fired(&s, id), "{name} still marked fired");
        walk_in(&mut s, &area);
        assert!(has_fired(&s, id), "{name} did not fire again");
        checked += 1;
        assert!(s.restart_at_checkpoint().unwrap());
        for _ in 0..30 {
            s.step(1.0 / 60.0);
        }
        if in_later {
            to(&mut s, "havindr_s4");
        }
    }
    eprintln!("checked {checked}");
    assert!(checked > 0);
}
