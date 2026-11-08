use magicka_runtime::{play_session::PlaySession, scene_loading::custom_hash};
use std::path::PathBuf;
const DT: f32 = 1. / 60.;
fn session(scene: &str) -> Option<PlaySession> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Swamp.lvl", "Wizard").unwrap();
    s.change_scene_named(scene, "start").unwrap();
    s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
    Some(s)
}
fn tick(s: &mut PlaySession, n: usize) {
    for _ in 0..n {
        s.step(DT);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
}
fn fired(s: &PlaySession, name: &str) -> bool {
    s.runtime
        .trigger(custom_hash(&name.to_lowercase()))
        .unwrap()
        .has_triggered
}
#[test]
fn swamp_monastery_necromancer_reaches_exit_before_door_boss() {
    let Some(mut s) = session("sw_s2") else {
        return;
    };
    tick(&mut s, 60);
    s.run_trigger("Graveyard Cutscene", true).unwrap();
    tick(&mut s, 1800);
    assert!(fired(&s, "Remove Necromancer and animate door"));
}
#[test]
fn healed_swamp_villagers_complete_the_mob_sequence() {
    let Some(mut s) = session("sw_s5") else {
        return;
    };
    tick(&mut s, 120);
    for name in ["#ViMN9", "#ViFN9", "#ViMN6", "#ViMN4", "#ViFN2", "#ViMN2"] {
        let h = s
            .world
            .entities
            .iter()
            .find(|e| e.unique_id == custom_hash(&name.to_lowercase()))
            .unwrap()
            .handle;
        let c = &mut s.world.npcs.get_mut(&h).unwrap().character.combat.health;
        c.hit_points = c.max_hit_points;
    }
    tick(&mut s, 60 * 120);
    assert!(fired(&s, "Mob shouts outside castle"));
    assert!(!s.world.flags.cutscene);
}
#[test]
fn destroyed_explosive_crate_emits_its_authored_death_effect_once() {
    use magicka_sim::{
        damage::{attack, Damage},
        spells::Elements,
    };
    let Some(mut s) = session("sw_s1") else {
        return;
    };
    s.disable_scripts();
    let mut transform = magicka_content::skinned::IDENTITY;
    let p = s.body().position;
    transform[12..15].copy_from_slice(&[p[0], p[1] + 20., p[2]]);
    let h = s.spawn_physics_prop("crate_explosive", transform).unwrap();
    let mut nearby = transform;
    nearby[12] += 1.;
    let victim = s.spawn_physics_prop("dungeon_wall", nearby).unwrap();
    s.world
        .physics_damage
        .get_mut(&victim)
        .unwrap()
        .resistances
        .fill(magicka_sim::resistance::Resistance::IDENTITY);
    let before = s.world.physics_damage[&victim].health;
    s.world
        .damage_physics(
            h,
            Damage {
                attack: attack::DAMAGE,
                element: Elements::FIRE.0,
                amount: 1_000_000.,
                magnitude: 1.,
            },
            1,
        )
        .unwrap();
    let authored = s.world.physics_damage[&h].conditions.clone();
    let effect = authored
        .iter()
        .flat_map(|c| &c.events)
        .find_map(|e| {
            if let magicka_content::common::Event::Effect { effect, .. } = e {
                Some(effect.to_ascii_lowercase())
            } else {
                None
            }
        })
        .unwrap();
    s.world.step_live(None, DT).unwrap();
    let emitted = |s: &PlaySession| {
        s.world
            .item_event_effects
            .iter()
            .filter(|e| {
                matches!(e,
        magicka_runtime::npc_combat::ItemEffectRequest::Start { name, .. } if name == &effect)
            })
            .count()
    };
    assert_eq!(emitted(&s), 1, "authored explosion must be dispatched");
    assert!(
        s.world.physics_damage[&victim].health < before,
        "explosion failed to damage a nearby prop"
    );
    s.world.step_live(None, DT).unwrap();
    assert_eq!(emitted(&s), 1, "destroyed crate exploded twice");
}

#[test]
fn swamp_necromancer_releases_camera_after_local_enemies_die() {
    use magicka_runtime::entity_command::EntityCommandHost;
    let Some(mut s) = session("sw_s1") else {
        return;
    };
    tick(&mut s, 60);
    s.run_trigger("Ghoul Filler", true).unwrap();
    tick(&mut s, 60);
    s.run_trigger("Necromancer", true).unwrap();
    tick(&mut s, 600);
    // Clear the authored encounter area, preserving earlier living zombies
    // elsewhere in the scene. The release condition uses local evil presence.
    for _ in 0..3 {
        s.world.refresh_presence();
        let inside = s
            .world
            .presence
            .get(custom_hash("checkbox_necromancer"))
            .unwrap()
            .characters
            .clone();
        let handles: Vec<_> = inside
            .iter()
            .filter_map(|&i| {
                let e = s.world.presence_entities[i];
                (e.character.unwrap().original_faction & 1 != 0 && !e.dead).then_some(e.handle)
            })
            .collect();
        for h in handles {
            s.world.overkill(h);
        }
        tick(&mut s, 360);
    }
    assert!(fired(&s, "Release camera"));
    assert!(s
        .world
        .entities
        .iter()
        .any(|e| e.template_name.as_deref() == Some("zombie") && !e.dead && !e.removed));
    // Explicit removal must not leave an alive presence entry behind.
    let h = s
        .world
        .entities
        .iter()
        .find(|e| e.template_name.as_deref() == Some("zombie") && !e.dead && !e.removed)
        .unwrap()
        .handle;
    s.world.terminate(h).unwrap();
    s.world.refresh_presence();
    assert!(!s.world.presence_entities.iter().any(|e| e.handle == h));
}

#[test]
fn skeleton_archer_attacks_without_a_connected_navmesh() {
    let Some(mut s) = session("sw_s1") else {
        return;
    };
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    for _ in 0..120 {
        s.step(DT);
    }
    let h = s.spawn_npc("skeleton_archer", "attack", 8.).unwrap();
    s.world.nav = None;
    let mut shots = 0;
    for _ in 0..600 {
        s.world.nav = None;
        s.step(DT);
        shots = shots.max(s.world.missiles.len());
    }
    assert!(shots > 0, "archer did not fire: {:?}", s.world.npcs[&h].ai);
}

#[test]
fn mines_ledge_archer_reacts_with_defend_and_fires() {
    use magicka_runtime::entity_command::EntityCommandHost;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Mines.lvl", "Wizard").unwrap();
    s.change_scene_named("mi_s2", "start").unwrap();
    tick(&mut s, 2);
    s.run_trigger("langley", true).unwrap();
    tick(&mut s, 2);
    let reported = [63.7, 6.0, -87.6];
    let h = s
        .world
        .entities
        .iter()
        .filter(|e| e.template_name.as_deref() == Some("goblin_warrior_archer"))
        .min_by(|a, b| {
            let distance = |e: &magicka_runtime::scene_world::SceneEntity| {
                let n = &s.world.npcs[&e.handle];
                s.world.physics.bodies[n.body]
                    .position
                    .iter()
                    .zip(reported)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f32>()
            };
            distance(a).total_cmp(&distance(b))
        })
        .unwrap()
        .handle;
    let p = s.world.physics.bodies[s.world.npcs[&h].body].position;
    assert!((p[0] - reported[0]).abs() < 3. && (p[2] - reported[2]).abs() < 1.);
    let others: Vec<_> = s.world.npcs.keys().copied().filter(|&x| x != h).collect();
    for other in others {
        s.world.terminate(other).unwrap();
    }
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    let player = s.world.players[0].as_mut().unwrap();
    player.character.immortal_time = f32::MAX;
    let body = &mut s.world.physics.bodies[player.body];
    body.move_to([55., 2.5, -87.6], body.orientation);
    let mut fired = false;
    for _ in 0..600 {
        s.step(DT);
        fired |= !s.world.missiles.is_empty();
    }
    assert_eq!(
        s.world.npcs[&h].ai.order,
        magicka_runtime::avatar_ai::ORDER_DEFEND
    );
    assert!(
        fired,
        "ledge archer failed to fire: {:?}",
        s.world.npcs[&h].ai
    );
}

/// Campaign bot 2026-11-05: Battlefield stalled in bf_s1 11 m short of
/// `trigger_exit`, pushing with zero velocity 0.7 m from a navmesh vertex on
/// a 0.45 m, ~49° rise its capsule could not climb (research/campaign-bot.md).
/// Walk the bot's way (navmesh path, pad input, 1 m waypoint reach, bot rules)
/// from the start to the exit and require arrival; a stop reports where and
/// what the collision says is there.
#[test]
fn battlefield_exit_walk_like_the_bot() {
    use magicka_runtime::avatar::PlayerInput;
    use magicka_sim::{navmesh::movement, spray_pulse::Segment};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Battlefield.lvl", "Wizard").unwrap();
    s.change_scene_named("bf_s1", "start").unwrap();
    s.headless_dialogs = Some([1280.0, 720.0]);
    let exit = s
        .world
        .area_center("trigger_exit")
        .expect("trigger_exit area");
    let body = s.avatar().body;
    let flat = |a: [f32; 3], b: [f32; 3]| (a[0] - b[0]).hypot(a[2] - b[2]);
    // Let the intro and its AvatarMove finish, as a player waits them out.
    let mut waited = 0;
    let mut locked_seen = false;
    loop {
        let held = s.input_limited() || s.avatar().ai.controls_avatar();
        locked_seen |= held;
        if (locked_seen && !held) || waited >= 60 * 120 {
            break;
        }
        // As the bot: skip cutscenes every 30 frames instead of reading.
        if waited % 30 == 0 {
            s.skip_cutscene();
        }
        s.step(1.0 / 60.0);
        waited += 1;
    }
    eprintln!(
        "free after {} s at {:?}; exit {:?}",
        waited as f32 / 60.0,
        s.world.physics.bodies[body].position,
        exit
    );
    let mut path: Vec<[f32; 3]> = Vec::new();
    let mut still = 0.0f32;
    let mut last = s.world.physics.bodies[body].position;
    let mut outcome = "budget";
    for tick in 0..(180.0 * 60.0) as usize {
        let position = s.world.physics.bodies[body].position;
        // The bot's rules: health locked, hostiles nearby removed.
        {
            let health = &mut s.world.players[0].as_mut().unwrap().character.combat.health;
            health.hit_points = health.max_hit_points;
        }
        let hostile: Vec<u16> = s
            .world
            .entities
            .iter()
            .filter(|e| {
                e.added
                    && !e.removed
                    && !e.dead
                    && matches!(e.kind, magicka_runtime::scene_world::EntityKind::Npc { .. })
                    && e.faction & magicka_runtime::camera::FACTION_FRIENDLY
                        != magicka_runtime::camera::FACTION_FRIENDLY
                    && flat(e.position, position) < 12.0
            })
            .map(|e| e.handle)
            .collect();
        for handle in hostile {
            s.world.terminate_character(handle);
        }
        if s.world.area_contains("trigger_exit", position) || flat(position, exit) < 0.5 {
            outcome = "arrived";
            break;
        }
        if path.is_empty() || tick % 600 == 0 {
            path = s
                .world
                .nav
                .as_ref()
                .and_then(|nav| {
                    nav.find_shortest_path(position, exit, movement::DEFAULT, &|_| false)
                        .ok()
                })
                .map(|(_, nodes)| nodes.iter().map(|n| n.position).collect())
                .unwrap_or_default();
            if flat(*path.last().unwrap_or(&position), exit) < 6.0 {
                path.push(exit);
            }
        }
        while path.len() > 1 && flat(path[0], position) < 1.0 {
            path.remove(0);
        }
        let Some(next) = path.first().copied() else {
            outcome = "no route";
            break;
        };
        let (dx, dz) = (next[0] - position[0], next[2] - position[2]);
        let len = dx.hypot(dz).max(1e-3);
        s.handle_input_with_dialog_select(
            PlayerInput {
                pad_movement: Some([dx / len, dz / len]),
                move_down: tick % 30 == 0,
                pad_attack: still > 1.0 && still < 1.1,
                ..Default::default()
            },
            16.0 / 9.0,
            tick % 45 == 0,
        );
        if tick % 30 == 0 {
            s.skip_cutscene();
        }
        s.step(1.0 / 60.0);
        let now = s.world.physics.bodies[body].position;
        if s.input_limited() || s.avatar().ai.controls_avatar() {
            still = 0.0;
        } else if flat(now, last) < 0.01 {
            still += 1.0 / 60.0;
        } else {
            still = 0.0;
            last = now;
        }
        if still > 3.0 {
            outcome = "stuck";
            break;
        }
    }
    let b = &s.world.physics.bodies[body];
    let a = s.avatar();
    eprintln!(
        "{outcome}: at {:?} velocity {:?} movement {:?} motion {:?} ai {} limited {} next waypoint {:?} (path {})",
        b.position,
        b.velocity,
        b.movement(),
        a.motion.state,
        a.ai.controls_avatar(),
        s.input_limited(),
        path.first(),
        path.len()
    );
    if outcome == "stuck" {
        let m = b.movement();
        for height in [0.1f32, 0.5, 1.0, 1.6] {
            let origin = [b.position[0], b.position[1] + height, b.position[2]];
            let delta = [m[0] * 2.0, 0.0, m[2] * 2.0];
            let hit = s
                .world
                .collision
                .collision()
                .segment_intersect(Segment { origin, delta }, false);
            eprintln!(
                "  2 m ahead at +{height}: {:?}",
                hit.map(|h| (h.hit.fraction, h.hit.position, h.hit.normal, h.primitive))
            );
        }
        let near: Vec<String> = s
            .world
            .entities
            .iter()
            .filter(|e| e.added && !e.removed && flat(e.position, b.position) < 4.0 + e.radius)
            .map(|e| {
                format!(
                    "{:?}@{:?} r{:.1} dead {}",
                    e.kind, e.position, e.radius, e.dead
                )
            })
            .collect();
        eprintln!("  near: {near:?}");
    }
    assert_eq!(outcome, "arrived", "the exit must be walkable");
}

/// Campaign bot 2026-11-05: in bf_s2 the navmesh path to `trigger_exit`
/// ended near the shore. This probe deliberately excludes ice: water lies
/// between the shore and the exit with no ground under it. It does not prove
/// the river is unfrozen. The campaign-bot binary regression verifies the
/// existing ice connects the route (research/campaign-bot.md).
#[test]
fn battlefield_s2_exit_lies_across_water() {
    use magicka_sim::navmesh::movement;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Battlefield.lvl", "Wizard").unwrap();
    s.change_scene_named("bf_s2", "start").unwrap();
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    let start = s.world.physics.bodies[s.avatar().body].position;
    let exit = s.world.area_center("trigger_exit").expect("trigger_exit");
    let stuck = [39.0, -0.3, -75.1];
    let mut complete = false;
    for (label, from) in [("start", start), ("bot stuck pose", stuck)] {
        let result = s
            .world
            .nav
            .as_ref()
            .map(|nav| nav.find_shortest_path(from, exit, movement::DEFAULT, &|_| false));
        match result {
            Some(Ok((found, nodes))) => {
                complete |= found;
                eprintln!(
                    "{label} {from:?} -> exit {exit:?}: complete {found}, {} nodes, last {:?}",
                    nodes.len(),
                    nodes.last().map(|n| n.position)
                )
            }
            Some(Err(e)) => eprintln!("{label} {from:?} -> exit {exit:?}: error {e:?}"),
            None => eprintln!("no navmesh"),
        }
    }
    assert!(
        !complete,
        "the navmesh must not connect the shore to the exit"
    );
    // What lies between the shore and the exit: liquid and ground, every 10 m.
    let steps = 12;
    let mut water_without_ground = 0;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = stuck[0] + (exit[0] - stuck[0]) * t;
        let z = stuck[2] + (exit[2] - stuck[2]) * t;
        let segment = magicka_sim::spray_pulse::Segment {
            origin: [x, 10.0, z],
            delta: [0.0, -30.0, 0.0],
        };
        let c = s.world.collision.collision();
        let liquid = c.liquid_segment_intersect(segment, false, false, false);
        let ground = c.segment_intersect(segment, false);
        eprintln!(
            "{:5.1} m: liquid {:?} ground y {:?}",
            t * (exit[0] - stuck[0]).hypot(exit[2] - stuck[2]),
            liquid.map(|h| h.position[1]),
            ground.map(|h| h.hit.position[1])
        );
        if liquid.is_some() && ground.is_none() {
            water_without_ground += 1;
        }
    }
    assert!(
        water_without_ground >= 2,
        "open water must lie on the way to the exit"
    );
}
