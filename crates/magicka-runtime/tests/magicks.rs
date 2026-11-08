//! Magick casting: element combo then Space (Avatar.Boost → CombineMagick),
//! CastState's magick animation, Character.CastSpell → Magick.Effect, and the
//! ported Haste/Invisibility effects (research/magicks.md). Retail tests skip
//! when user-supplied content is absent.
use magicka_runtime::{
    avatar::{AvatarState, PlayerInput},
    play_session::PlaySession,
};
use magicka_sim::magicks::{allowed, animation, is_instant, AbilityEffect, AbilityEffects};
use magicka_sim::spells::MagickType as M;
use std::path::{Path, PathBuf};

#[test]
fn magick_animations_instants_and_unlocks_follow_the_original_tables() {
    assert_eq!(animation(M::Haste), Some("cast_magick_self"));
    assert_eq!(animation(M::Rain), Some("cast_magick_global"));
    assert_eq!(animation(M::ThunderB), Some("cast_magick_direct"));
    assert_eq!(animation(M::Charm), Some("cast_magick_sweep"));
    assert_eq!(animation(M::Portal), Some("cast_area_fireworks"));
    assert_eq!(animation(M::None), None);
    let instants: Vec<_> = (0..35u8)
        .filter_map(|i| M::try_from(i).ok())
        .filter(|&m| is_instant(m))
        .collect();
    assert_eq!(
        instants,
        [
            M::Revive,
            M::Haste,
            M::Invisibility,
            M::Teleport,
            M::Napalm,
            M::ChainLightning
        ]
    );
    assert!(allowed(1 << 3, M::Haste));
    assert!(!allowed(1 << 3, M::Revive));
    assert!(!allowed(u64::MAX, M::None));
}

#[test]
fn haste_refreshes_per_owner_scales_speed_and_expires() {
    let mut e = AbilityEffects::default();
    e.haste(7, None);
    let mut factors = vec![];
    e.update(
        1.0,
        1.0,
        1.0,
        |owner, f| {
            assert_eq!(owner, 7);
            factors.push(f);
            true
        },
        |_| unreachable!(),
        |_| true,
    );
    // TTL 9 after the first update: 1 + (1 - 0.5^9).
    assert!((factors[0] - (2.0 - 0.5f32.powi(9))).abs() < 1e-6);
    e.haste(7, None);
    assert_eq!(e.effects.len(), 1, "same owner refreshes");
    let AbilityEffect::Haste(h) = e.effects[0] else {
        panic!()
    };
    assert_eq!(h.ttl, 10.0);
    // CoffeeBoost-style custom TTL keeps the longer remaining time.
    e.haste(7, Some(3.0));
    let AbilityEffect::Haste(h) = e.effects[0] else {
        panic!()
    };
    assert_eq!(h.ttl, 10.0);
    e.update(10.0, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(e.effects.is_empty(), "expired at TTL <= 0");
    // A disabled body ends the haste at once.
    e.haste(8, None);
    e.update(0.1, 1.0, 1.0, |_, _| false, |_| (), |_| true);
    assert!(e.effects.is_empty());
}

/// Revive.Execute picks the eligible player dead longest (above
/// float.Epsilon, earlier slot on ties); Revive.Update spawns once
/// timer += dt·4 exceeds 1, lingers 3 s, and Ressing lasts until the spawn.
#[test]
fn revive_picks_the_longest_dead_player_spawns_after_a_quarter_second_and_lingers() {
    use magicka_sim::magicks::select_revivee;
    assert_eq!(
        select_revivee([None, Some(2.0), Some(3.0), Some(3.0)]),
        Some(2)
    );
    assert_eq!(
        select_revivee([Some(0.0), None]),
        None,
        "DeadAge must exceed epsilon"
    );
    assert_eq!(select_revivee([None; 4]), None);
    assert_eq!(
        select_revivee([Some(f32::from_bits(1)), Some(f32::NAN)]),
        None
    );
    let mut e = AbilityEffects::default();
    e.revive(2, [1.0, 0.0, 3.0], 0.1);
    assert!(e.ressing(2) && !e.ressing(1));
    let mut spawned = vec![];
    for _ in 0..15 {
        e.update(DT, 1.0, 1.0, |_, _| true, |r| spawned.push(*r), |_| true);
    }
    assert!(spawned.is_empty(), "timer 15·dt·4 = 1.0 does not exceed 1");
    e.update(DT, 1.0, 1.0, |_, _| true, |r| spawned.push(*r), |_| true);
    assert_eq!(spawned.len(), 1);
    assert_eq!((spawned[0].revivee, spawned[0].health), (2, 0.1));
    assert!(!e.ressing(2), "Ressing cleared at the spawn");
    e.update(2.99, 1.0, 1.0, |_, _| true, |_| unreachable!(), |_| true);
    assert_eq!(e.effects.len(), 1, "lingers 3 s");
    e.update(0.02, 1.0, 1.0, |_, _| true, |_| unreachable!(), |_| true);
    assert!(e.effects.is_empty());
}

const DT: f32 = 1.0 / 60.0;
const LEVEL: &str = "Havindr.lvl";
const WATER: usize = 0;
const LIFE: usize = 1;
const SHIELD: usize = 2;
const COLD: usize = 3;
const LIGHTNING: usize = 4;
const ARCANE: usize = 5;
const EARTH: usize = 6;
const FIRE: usize = 7;

#[test]
fn conflagration_combo_starts_a_moving_emitter_and_expires() {
    use magicka_runtime::scene_loading::custom_hash;
    use magicka_sim::spells::Elements as E;
    let Some(root) = content() else { return };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Conflagration as u8;
    // Add Fire before Water for later Steams so Water combines with that
    // new Fire rather than consuming the preceding standalone Fire.
    for keys in [
        &[WATER, FIRE][..],
        &[FIRE][..],
        &[FIRE, WATER][..],
        &[FIRE][..],
        &[FIRE, WATER][..],
    ] {
        press(&mut s, keys);
        for _ in 0..60 {
            frame(&mut s, PlayerInput::default());
        }
    }
    assert_eq!(
        s.avatar()
            .conjure
            .queue
            .iter()
            .map(|s| s.element)
            .collect::<Vec<_>>(),
        [E::STEAM, E::FIRE, E::STEAM, E::FIRE, E::STEAM]
    );
    space(&mut s);
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
        if s.world
            .ability_effects
            .effects
            .iter()
            .any(|e| matches!(e, AbilityEffect::Conflagration { .. }))
        {
            break;
        }
    }
    assert!(cue_started(&s, "magick_conflagration"));
    let hash = custom_hash("magick_conflagration");
    assert_eq!(s.effects.active_effects_by_hash(hash), 1);
    assert!(s.avatar().conjure.queue.is_empty());
    let initial = s
        .world
        .ability_effects
        .effects
        .iter()
        .find_map(|e| match e {
            AbilityEffect::Conflagration { sweep, .. } => Some(sweep.center()),
            _ => None,
        })
        .unwrap();
    for _ in 0..15 {
        frame(&mut s, PlayerInput::default());
    }
    let moved = s
        .world
        .ability_effects
        .effects
        .iter()
        .find_map(|e| match e {
            AbilityEffect::Conflagration { sweep, .. } => Some(sweep.center()),
            _ => None,
        })
        .unwrap();
    assert!(initial.iter().zip(moved).any(|(a, b)| (a - b).abs() > 0.5));
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(!s
        .world
        .ability_effects
        .effects
        .iter()
        .any(|e| matches!(e, AbilityEffect::Conflagration { .. })));
    assert_eq!(s.effects.active_effects_by_hash(hash), 0);
}

#[test]
fn conflagration_burns_solid_targets_once_and_nullify_stops_each_instance() {
    use magicka_sim::status::Status;
    let Some(root) = content() else { return };
    let mut s = session(&root);
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    let ghost = s.spawn_npc("Ghost_Acolyte", "idle", 3.0).unwrap();
    let origin = s.body().position;
    let direction = s.body().direction();
    let position = std::array::from_fn(|i| origin[i] + direction[i] * 3.0);
    for h in [goblin, ghost] {
        let n = s.world.npcs.get_mut(&h).unwrap();
        // spawn_npc enables proximity reactions even for an idle order.
        // Isolate the sweep filter from attack-state corporealization.
        n.react_to = 0;
        n.character.combat.health.hit_points = 5000.0;
        n.character.definition.max_hit_points = 5000.0;
        n.character.combat.health.max_hit_points = 5000.0;
        let b = &mut s.world.physics.bodies[n.body];
        b.position = position;
        b.active = false;
    }
    let wizard = s.avatar().handle;
    assert!(s.world.conflagration_from(wizard));
    frame(&mut s, PlayerInput::default());
    assert!(s.world.npcs[&ghost].character.ethereal);
    let after = s.world.npcs[&goblin].character.combat.health.hit_points;
    assert!(after < 4500.0, "instant Fire damage: {after}");
    frame(&mut s, PlayerInput::default());
    assert!(s.world.npcs[&ghost].character.ethereal);
    assert!(s.world.npcs[&goblin]
        .character
        .combat
        .statuses
        .has(Status::Burning));
    assert_eq!(
        s.world.npcs[&ghost].character.combat.health.hit_points,
        5000.0
    );
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        s.world.npcs[&goblin].character.combat.health.hit_points > after - 100.0,
        "no repeated 600 damage"
    );
    assert!(s.world.conflagration_from(wizard));
    frame(&mut s, PlayerInput::default());
    let hash = magicka_runtime::scene_loading::custom_hash("magick_conflagration");
    assert_eq!(
        s.effects.active_effects_by_hash(hash),
        2,
        "each cast has its own emitter"
    );
    s.world.nullify();
    frame(&mut s, PlayerInput::default());
    assert_eq!(s.effects.active_effects_by_hash(hash), 0);
    assert!(!s
        .world
        .ability_effects
        .effects
        .iter()
        .any(|e| matches!(e, AbilityEffect::Conflagration { .. })));
}

#[test]
fn conflagration_retries_an_ethereal_character_after_corporealize() {
    let Some(root) = content() else { return };
    let mut s = session(&root);
    let ghost = s.spawn_npc("Ghost_Acolyte", "idle", 3.0).unwrap();
    let n = s.world.npcs.get_mut(&ghost).unwrap();
    // spawn_npc enables proximity reactions even for an idle order. Keep
    // the ghost from attacking and becoming corporeal before the sweep.
    n.react_to = 0;
    n.character.combat.health.hit_points = 5000.0;
    n.character.definition.max_hit_points = 5000.0;
    n.character.combat.health.max_hit_points = 5000.0;
    assert!(n.character.ethereal);
    assert!(n.character.definition.looks_ethereal);
    s.world.physics.bodies[n.body].active = false;
    assert!(s.world.conflagration_from(s.avatar().handle));
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
        let c = &s.world.npcs[&ghost].character;
        assert!(c.ethereal);
        assert_eq!(c.combat.health.hit_points, 5000.0);
    }

    // Reuse the same sweep: rejection must not add the ghost to its hit
    // list. LooksEthereal persists and must not act as damage immunity.
    let position = s.world.physics.bodies[s.world.npcs[&ghost].body].position;
    s.world.corporealize(position);
    let c = &s.world.npcs[&ghost].character;
    assert!(!c.ethereal);
    assert!(c.definition.looks_ethereal);
    frame(&mut s, PlayerInput::default());
    let after = s.world.npcs[&ghost].character.combat.health.hit_points;
    assert!(
        after < 4500.0,
        "materialized target takes Fire damage: {after}"
    );
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        s.world.npcs[&ghost].character.combat.health.hit_points > after - 100.0,
        "the sweep damages each target only once"
    );
}

#[test]
fn conflagration_damages_a_ghost_that_materializes_to_attack() {
    let Some(root) = content() else { return };
    let mut s = session(&root);
    // The pirate supplies the hostile neighbour from the original failure;
    // Ghost_Acolyte does not react to the wizard as an enemy.
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    let ghost = s.spawn_npc("Ghost_Acolyte", "idle", 3.0).unwrap();
    let origin = s.body().position;
    let direction = s.body().direction();
    let position = std::array::from_fn(|i| origin[i] + direction[i] * 3.0);
    for handle in [goblin, ghost] {
        let n = s.world.npcs.get_mut(&handle).unwrap();
        // Keep the enemy alive long enough for the ghost's reaction.
        n.character.combat.health.hit_points = 5000.0;
        n.character.definition.max_hit_points = 5000.0;
        n.character.combat.health.max_hit_points = 5000.0;
        let body = n.body;
        s.world.physics.bodies[body].position = position;
        s.world.physics.bodies[body].active = false;
    }
    assert!(s.world.npcs[&ghost].character.ethereal);
    assert!(s.world.conflagration_from(s.avatar().handle));
    let mut hit = false;
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
        let c = &s.world.npcs[&ghost].character;
        if c.combat.health.hit_points < 4500.0 {
            assert!(!c.ethereal, "the attack materializes before the sweep hits");
            assert!(c.definition.looks_ethereal);
            hit = true;
            break;
        }
    }
    assert!(hit, "the proximity reaction makes the ghost vulnerable");
}

#[test]
fn conflagration_shield_rejection_retries_and_clients_do_not_apply_damage() {
    use magicka_sim::shield_entity::{ShieldKind, MAX_HIT_POINTS, SHIELD_COLOR};
    let Some(root) = content() else { return };
    let mut s = session(&root);
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    let owner = s.avatar().handle;
    let origin = s.body().position;
    let direction = s.body().direction();
    let position = std::array::from_fn(|i| origin[i] + direction[i] * 3.0);
    let n = s.world.npcs.get_mut(&goblin).unwrap();
    n.character.combat.health.hit_points = 5000.0;
    n.character.definition.max_hit_points = 5000.0;
    n.character.combat.health.max_hit_points = 5000.0;
    s.world.physics.bodies[n.body].position = position;
    s.world.physics.bodies[n.body].active = false;
    let shield = s.world.spawn_shield(
        owner,
        origin,
        1.5,
        [0.0, 0.0, 1.0],
        ShieldKind::Sphere,
        MAX_HIT_POINTS,
        SHIELD_COLOR,
    );
    s.world.conflagration_from(owner);
    frame(&mut s, PlayerInput::default());
    assert_eq!(
        s.world.npcs[&goblin].character.combat.health.hit_points,
        5000.0
    );
    // Remove the blocking shield, rather than waiting for its dying fade;
    // a fading native shield keeps intersecting until it becomes dead.
    s.world.shields.retain(|s| s.handle != shield);
    frame(&mut s, PlayerInput::default());
    assert!(s.world.npcs[&goblin].character.combat.health.hit_points < 4500.0);
    s.world.nullify();
    frame(&mut s, PlayerInput::default());
    let hp = s.world.npcs[&goblin].character.combat.health.hit_points;
    s.world.role = magicka_sim::damage_delivery::NetworkRole::Client;
    s.world.conflagration_from(owner);
    frame(&mut s, PlayerInput::default());
    assert_eq!(s.world.npcs[&goblin].character.combat.health.hit_points, hp);
    assert_eq!(
        s.effects
            .active_effects_by_hash(magicka_runtime::scene_loading::custom_hash(
                "magick_conflagration"
            )),
        1
    );
}

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}
fn frame(s: &mut PlaySession, input: PlayerInput) {
    s.step(DT);
    s.handle_input(input, 16.0 / 9.0);
    s.events.clear();
}
fn session(root: &Path) -> PlaySession {
    let mut s = PlaySession::load(root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    s
}
fn press(s: &mut PlaySession, keys: &[usize]) {
    for &key in keys {
        let mut elements = [false; 8];
        elements[key] = true;
        frame(
            s,
            PlayerInput {
                elements,
                ..Default::default()
            },
        );
        for _ in 0..6 {
            frame(s, PlayerInput::default());
        }
    }
}
fn space(s: &mut PlaySession) {
    frame(
        s,
        PlayerInput {
            boost_down: true,
            ..Default::default()
        },
    );
    frame(s, PlayerInput::default());
}
fn cue_started(s: &PlaySession, name: &str) -> bool {
    let hash = magicka_runtime::scene_loading::custom_hash(name);
    s.audio.iter().any(|(r, _)| {
        matches!(r, magicka_runtime::audio::AudioRequest::Start { cue, .. } if *cue == hash)
    })
}

/// Real input: Lightning, Arcane, Fire then Space casts Haste only once it
/// is unlocked; the Wizard plays cast_magick_self, the queue is consumed,
/// and the hasted Wizard outruns an unhasted one.
#[test]
fn haste_combo_then_space_casts_only_when_unlocked_and_speeds_up() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut locked = session(&root);
    press(&mut locked, &[LIGHTNING, ARCANE, FIRE]);
    space(&mut locked);
    assert_eq!(
        locked.avatar().conjure.queue.len(),
        3,
        "locked magick is not combined"
    );
    assert!(locked.world.ability_effects.effects.is_empty());

    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Haste as u8;
    press(&mut s, &[LIGHTNING, ARCANE, FIRE]);
    space(&mut s);
    let magick_self = magicka_runtime::character_motion::animation_index("cast_magick_self");
    let (mut cast_state, mut clip) = (false, false);
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        cast_state |= s.avatar().motion.state == AvatarState::Cast;
        clip |= Some(s.avatar().motion.controller.current_animation) == magick_self;
    }
    assert!(cast_state && clip, "CastState with cast_magick_self");
    assert!(s.avatar().conjure.queue.is_empty(), "queue consumed");
    let handle = s.avatar().handle;
    assert!(s.world.ability_effects.hasted(handle));
    assert!(cue_started(&s, "magick_haste"));
    assert!(s
        .world
        .log
        .iter()
        .all(|l| !l.contains("magick not reconstructed")));

    // Walk both Wizards the same way: haste covers more ground.
    let mut plain = session(&root);
    let walk = PlayerInput {
        cursor_ndc: Some([0.0, -0.6]),
        move_down: true,
        ..Default::default()
    };
    let start = (s.body().position, plain.body().position);
    // Short enough that neither Wizard reaches scenery.
    for _ in 0..30 {
        frame(&mut s, walk);
        frame(&mut plain, walk);
    }
    let dist = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    let hasted = dist(s.body().position, start.0);
    let normal = dist(plain.body().position, start.1);
    assert!(hasted > normal * 1.5, "hasted {hasted} vs {normal}");
    // Expires after 10 s.
    for _ in 0..600 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(!s.world.ability_effects.hasted(handle));
}

/// Arcane, Shield, Steam (Water+Fire), Arcane then Space: 15 s invisibility.
#[test]
fn invisibility_combo_sets_the_original_timer() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Invisibility as u8;
    press(&mut s, &[ARCANE, SHIELD, WATER, FIRE, ARCANE]);
    assert_eq!(
        s.avatar().conjure.queue.len(),
        4,
        "steam combined from water+fire"
    );
    space(&mut s);
    let mut timer = 0.0f32;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        timer = timer.max(s.avatar().character.combat.health.invisibility_timer);
    }
    assert!(timer > 14.0 && timer <= 15.0, "invisibility timer {timer}");
    assert!(cue_started(&s, "magick_invisibility"));
    // The cast state's own SetInvisible(0) must not undo the magick.
    assert!(
        s.avatar().character.is_invisible(),
        "still invisible after the cast: {}",
        s.avatar().character.combat.health.invisibility_timer
    );
}

/// Agent.ChooseTarget, the proximity reaction and AIStateAttack skip an
/// invisible enemy: a hostile goblin ignores the invisible Wizard but
/// targets the same Wizard when visible.
#[test]
fn invisible_wizard_is_not_targeted_by_a_hostile_goblin() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    for invisible in [false, true] {
        let mut s = session(&root);
        if invisible {
            s.unlocked_magicks[0] |= 1 << M::Invisibility as u8;
            press(&mut s, &[ARCANE, SHIELD, WATER, FIRE, ARCANE]);
            space(&mut s);
            for _ in 0..90 {
                frame(&mut s, PlayerInput::default());
            }
            assert!(s.avatar().character.is_invisible());
        }
        let goblin = s.spawn_npc("Goblin_pirate", "attack", 7.0).unwrap();
        let wizard = s.avatar().handle;
        let mut targeted = false;
        for _ in 0..300 {
            frame(&mut s, PlayerInput::default());
            targeted |= s.world.npcs[&goblin].combat.current_target() == Some(wizard);
        }
        assert_eq!(targeted, !invisible, "invisible {invisible}");
        assert_eq!(s.avatar().character.is_invisible(), invisible);
    }
}

/// Lightning, Arcane, Lightning then Space: Teleport moves the Wizard up to
/// 10 m along its facing (navmesh-nearest point, 4 m ground probe) and plays
/// the origin and destination cues.
#[test]
fn teleport_combo_moves_the_wizard_forward_onto_ground() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Teleport as u8;
    let (start, facing) = (s.body().position, s.body().direction());
    press(&mut s, &[LIGHTNING, ARCANE, LIGHTNING]);
    space(&mut s);
    let mut moved = None;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        let p = s.body().position;
        let d = [p[0] - start[0], p[2] - start[2]];
        if (d[0] * d[0] + d[1] * d[1]).sqrt() > 2.0 {
            moved = Some(p);
            break;
        }
    }
    let end = moved.expect("teleport moved the Wizard");
    let d = [end[0] - start[0], end[2] - start[2]];
    let distance = (d[0] * d[0] + d[1] * d[1]).sqrt();
    assert!(
        distance <= 10.5,
        "at most the 10 m teleport distance: {distance}"
    );
    let along = (d[0] * facing[0] + d[1] * facing[2]) / distance;
    assert!(along > 0.7, "moved along the facing direction ({along})");
    assert!(cue_started(&s, "magick_teleporta") && cue_started(&s, "magick_teleportb"));
    assert!(s
        .world
        .log
        .iter()
        .all(|l| !l.contains("magick not reconstructed")));
}

/// Arcane, Shield then Space: Nullify cancels the caster's Haste (SpellManager
/// .ClearMagicks), removes its self shield and stops its Wet status, with the
/// magick_nullify cue.
#[test]
fn nullify_combo_cancels_haste_self_shield_and_statuses() {
    use magicka_sim::status::{Status, StatusEffect};
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Haste as u8 | 1 << M::Nullify as u8;
    press(&mut s, &[LIGHTNING, ARCANE, FIRE]);
    space(&mut s);
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
    }
    let handle = s.avatar().handle;
    assert!(s.world.ability_effects.hasted(handle), "haste first");
    press(&mut s, &[SHIELD]);
    frame(
        &mut s,
        PlayerInput {
            cast_self_down: true,
            ..Default::default()
        },
    );
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        s.avatar().character.combat.health.shield.active(),
        "self shield up"
    );
    let wet = StatusEffect::new(Status::Wet, 0.0, 1.0, false);
    s.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .statuses
        .set_effect(wet)
        .unwrap();
    press(&mut s, &[ARCANE, SHIELD]);
    space(&mut s);
    let mut nullified = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        nullified |= cue_started(&s, "magick_nullify");
        if nullified {
            break;
        }
    }
    assert!(nullified, "magick_nullify cue");
    for _ in 0..5 {
        frame(&mut s, PlayerInput::default());
    }
    let c = &s.avatar().character;
    assert!(!s.world.ability_effects.hasted(handle), "haste cleared");
    assert!(!c.combat.health.shield.active(), "self shield removed");
    assert!(
        c.combat.statuses.effect(Status::Wet).unwrap().dead(),
        "wet stopped"
    );
    assert!(s
        .world
        .log
        .iter()
        .all(|l| !l.contains("magick not reconstructed")));
}

/// Life, Lightning then Space with a second wizard dead: Revive plays the
/// cast cue, reserves the revivee, and a quarter second later rebuilds it
/// 4 m in front of the caster at 0.1 health in RessurectionState (revive
/// clip), with the magick_revive cue. With nobody dead it fails.
#[test]
fn revive_combo_brings_back_a_dead_wizard_in_front_of_the_caster() {
    use magicka_runtime::avatar::AvatarState;
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Revive as u8;
    s.join_player_at_start(1, "Wizard").unwrap();
    let unlocks = s.unlocked_magicks.clone();
    let player = s.world.players[1].as_ref().unwrap();
    let handle = player.handle;
    let equipment: Vec<_> = player
        .items
        .iter()
        .map(|(slot, item)| (*slot, item.definition.name.clone()))
        .collect();
    s.set_player_loadout(
        1,
        "Wizard",
        "staff_heirloom_grandfather_clock",
        &equipment[0].1,
    )
    .unwrap();
    press(&mut s, &[LIFE, LIGHTNING]);
    space(&mut s);
    let mut failed = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        failed |= cue_started(&s, "magick_fail");
    }
    assert!(failed, "nobody to revive: magick_fail");
    assert!(s.world.ability_effects.effects.is_empty());

    s.world.players[1].as_mut().unwrap().kill();
    for _ in 0..600 {
        frame(&mut s, PlayerInput::default());
        if s.world.players[1].as_ref().unwrap().dead() {
            break;
        }
    }
    assert!(
        s.world.players[1].as_ref().unwrap().dead(),
        "second wizard died"
    );
    frame(&mut s, PlayerInput::default());
    assert!(s.world.dead_age[1] > 0.0 && s.world.dead_age[0] == 0.0);

    press(&mut s, &[LIFE, LIGHTNING]);
    space(&mut s);
    let mut cast = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "magick_revive_cast");
        if cast {
            break;
        }
    }
    assert!(cast, "magick_revive_cast");
    assert!(s.world.ability_effects.ressing(1));
    let AbilityEffect::Revive(pending) = s.world.ability_effects.effects[0] else {
        panic!("expected pending revive")
    };
    let caster = s.body();
    let direction = caster.direction();
    let proposed = std::array::from_fn(|i| caster.position[i] + direction[i] * 4.0);
    let (_, expected, _) = s
        .world
        .nav
        .as_ref()
        .unwrap()
        .nearest(proposed, magicka_sim::navmesh::movement::DEFAULT)
        .unwrap();
    for (actual, expected) in pending.position.iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 1e-4,
            "revive point snaps to navmesh"
        );
    }
    assert!(
        !s.world.revive(None, proposed, 1.0),
        "pending revive reserves the only dead player"
    );
    // ClearMagicks cancels the reservation and must not leave a queued spawn.
    s.world.nullify();
    assert!(!s.world.ability_effects.ressing(1));
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.players[1].as_ref().unwrap().dead());
    assert!(s.world.revive(Some(s.body().position), proposed, 0.1));
    let mut revived = false;
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
        revived |= cue_started(&s, "magick_revive");
        if revived {
            break;
        }
    }
    assert!(revived, "magick_revive after the 0.25 s timer");
    let p = s.world.players[1].as_ref().unwrap();
    assert!(!p.dead());
    assert_eq!(p.handle, handle);
    assert_eq!(s.unlocked_magicks, unlocks);
    assert_eq!(
        p.items
            .iter()
            .map(|(slot, item)| (*slot, item.definition.name.clone()))
            .collect::<Vec<_>>(),
        equipment
    );
    assert!(!s.world.ability_effects.ressing(1));
    assert_eq!(p.motion.state, AvatarState::Resurrection);
    let max = p.character.definition.max_hit_points;
    assert!((p.character.combat.health.hit_points - max * 0.1).abs() < 1e-3);
    let at = s.world.physics.bodies[p.body].position;
    for axis in [0, 2] {
        assert!(
            (at[axis] - pending.position[axis]).abs() < 1e-4,
            "spawn uses the scheduled navmesh point"
        );
    }
    for _ in 0..600 {
        frame(&mut s, PlayerInput::default());
    }
    let p = s.world.players[1].as_ref().unwrap();
    assert_eq!(
        p.motion.state,
        AvatarState::Idle,
        "revive clip ends in idle"
    );
    assert!(
        s.world.ability_effects.effects.is_empty(),
        "effect gone after 3 s"
    );
    assert!(s
        .world
        .log
        .iter()
        .all(|l| !l.contains("magick not reconstructed") && !l.contains("revive:")));
}

#[test]
fn scene_change_preserves_dead_age_and_longest_dead_revive_priority() {
    let Some(root) = content() else { return };
    let mut s = session(&root);
    for slot in [1, 2] {
        s.join_player_at_start(slot, "Wizard").unwrap();
    }
    // Slot 2 dies first; slot order must not replace elapsed dead time.
    s.world.players[2].as_mut().unwrap().terminate();
    for _ in 0..30 {
        s.step(DT);
    }
    s.world.players[1].as_mut().unwrap().terminate();
    for _ in 0..10 {
        s.step(DT);
    }
    let ages = s.world.dead_age.clone();
    assert!(ages[2] > ages[1] && ages[1] > 0.0);
    s.change_scene_named("havindr_s3", "start").unwrap();
    assert_eq!(
        s.world.dead_age, ages,
        "scene loading is not a new player lifetime"
    );
    for slot in [1, 2] {
        assert!(s.world.players[slot].as_ref().unwrap().dead());
    }
    assert!(s.world.revive(None, s.body().position, 0.1));
    let AbilityEffect::Revive(pending) = s.world.ability_effects.effects[0] else {
        panic!("expected Revive");
    };
    assert_eq!(pending.revivee, 2);
    for _ in 0..17 {
        s.step(DT);
    }
    assert!(!s.world.players[2].as_ref().unwrap().dead());
    assert!(s.world.players[1].as_ref().unwrap().dead());
    // Native Player.Update clears age when its avatar becomes living.
    s.step(DT);
    assert_eq!(s.world.dead_age[2], 0.0);
    assert!(s.world.dead_age[1] > ages[1]);
    let remaining_age = s.world.dead_age[1];
    let scene = s.scene_name().to_string();
    s.change_scene_named(&scene, "start").unwrap();
    assert_eq!(s.world.dead_age[1], remaining_age);
    assert!(s.world.revive(None, s.body().position, 0.1));
    assert!(s.world.ability_effects.ressing(1));
}

/// Cold, Arcane, Shield then Space with a goblin 3 m ahead: Fear plays its
/// cue, the goblin (another faction) is feared by the caster for 5 s, enters
/// PanicState (move_panic) and runs away from the Wizard; the Wizard itself
/// is not feared. Fear ends after FEAR_TIME and Nullify removes it early.
#[test]
fn fear_combo_makes_a_nearby_goblin_flee_from_the_caster() {
    use magicka_runtime::character_motion::{CharacterState, MOVE_PANIC};
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Fear as u8 | 1 << M::Nullify as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    let wizard = s.avatar().handle;
    let flat = |s: &PlaySession| {
        let a = s.body().position;
        let b = s.world.physics.bodies[s.world.npcs[&goblin].body].position;
        ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    press(&mut s, &[COLD, ARCANE, SHIELD]);
    space(&mut s);
    let mut cast = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "magick_fear");
        if cast {
            break;
        }
    }
    assert!(cast, "magick_fear cue");
    let fear = s.world.npcs[&goblin].character.fear;
    assert_eq!(fear.by, Some(wizard));
    assert!(fear.timer > 4.9 && fear.timer <= magicka_sim::fear::FEAR_TIME);
    assert!(
        !s.avatar().character.fear.is_feared(false),
        "caster unaffected"
    );
    let start = flat(&s);
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let n = &s.world.npcs[&goblin];
    assert_eq!(n.motion.state, CharacterState::Panic);
    assert_eq!(n.motion.controller.current_animation, MOVE_PANIC);
    assert!(s.fear_emitters().contains(&goblin), "magick_feared follows");
    assert!(flat(&s) > start + 1.0, "fled {start} -> {}", flat(&s));
    assert!(s
        .world
        .log
        .iter()
        .all(|l| !l.contains("magick not reconstructed")));

    // Nullify removes the fear (Character.RemoveFear) and the state leaves Panic.
    press(&mut s, &[ARCANE, SHIELD]);
    space(&mut s);
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        if cue_started(&s, "magick_nullify") {
            break;
        }
    }
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    let n = &s.world.npcs[&goblin];
    assert_eq!(n.character.fear.timer, 0.0);
    assert_ne!(n.motion.state, CharacterState::Panic);
    assert!(!s.fear_emitters().contains(&goblin));
}

/// The timer runs out after FEAR_TIME without other interference.
#[test]
fn fear_expires_after_five_seconds() {
    use magicka_runtime::character_motion::CharacterState;
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    let wizard = s.avatar().handle;
    assert!(s.world.fear_from(wizard));
    frame(&mut s, PlayerInput::default());
    assert_eq!(s.world.npcs[&goblin].motion.state, CharacterState::Panic);
    for _ in 0..(5 * 60 + 2) {
        frame(&mut s, PlayerInput::default());
    }
    let n = &s.world.npcs[&goblin];
    assert_eq!((n.character.fear.timer, n.character.fear.by), (0.0, None));
    assert_ne!(n.motion.state, CharacterState::Panic);
}

/// TimeWarp.Update eases TimeMultiplier to 0.5 and saturation to 0.1 per
/// real second, eases back during the last second, and lasts 15 real
/// seconds however slow the game runs. A second cast is refused.
#[test]
fn time_warp_eases_in_real_time_and_refuses_a_second_cast() {
    use magicka_sim::magicks::TIME_WARP_DURATION;
    let mut e = AbilityEffects::default();
    assert!(e.time_warp(Some(1), 0.8));
    assert!(!e.time_warp(Some(2), 0.8), "one TimeWarp at a time");
    let mut scale = 1.0;
    let mut real = 0.0f32;
    let mut at_one = None;
    while e.time_warp_state().is_some() {
        let dt = 1.0 / 60.0;
        // SpellManager receives the scaled delta.
        e.update(dt * scale, scale, 0.8, |_, _| true, |_| (), |_| true);
        real += dt;
        if let Some(w) = e.time_warp_state() {
            scale = w.multiplier;
            if at_one.is_none() && real >= 1.0 {
                at_one = Some((w.multiplier, w.saturation));
            }
        }
    }
    let (m, sat) = at_one.unwrap();
    // Forward Euler of dm/dt = 0.5 - m: about 0.5 + 0.5 e^-1.
    assert!((m - (0.5 + 0.5 * (-1f32).exp())).abs() < 0.01, "{m}");
    assert!((sat - (0.1 + 0.7 * (-1f32).exp())).abs() < 0.01, "{sat}");
    assert!((real - TIME_WARP_DURATION).abs() < 0.05, "{real}");
    assert!(e.time_warp(Some(2), 0.8), "castable again after expiry");
}

/// Cold, Shield then Space: the TimeWarp cue plays, the world clock runs
/// slower than real time, the render desaturates, and after 15 s everything
/// returns to normal and the cue stops.
#[test]
fn time_warp_combo_slows_the_world_then_restores_it() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::TimeWarp as u8;
    press(&mut s, &[COLD, SHIELD]);
    space(&mut s);
    let mut cast = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "magick_timewarp");
        if cast {
            break;
        }
    }
    assert!(cast, "magick_timewarp cue");
    assert!(s.world.ability_effects.time_warp_state().is_some());
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
    }
    let m = s.world.time_multiplier;
    assert!(m > 0.5 && m < 0.6, "multiplier after 2 s: {m}");
    assert!(s.render_saturation() < 0.3 * s.post.saturation + 0.2);
    let before = s.world.play_time;
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let world_second = s.world.play_time - before;
    assert!(world_second < 0.6, "world advanced {world_second} s in 1 s");
    for _ in 0..(13 * 60) {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.ability_effects.time_warp_state().is_none());
    assert_eq!(s.world.time_multiplier, 1.0);
    assert_eq!(s.render_saturation(), s.post.saturation);
    assert!(s
        .world
        .log
        .iter()
        .all(|l| !l.contains("magick not reconstructed")));
}

/// Rain's singleton: a second cast refreshes the TTL in place, and the wet
/// pulse fires on the first update, then every 0.25 s.
#[test]
fn rain_refreshes_one_effect_and_pulses_every_quarter_second() {
    use magicka_sim::magicks::{RAIN_TTL, RAIN_WET_PERIOD};
    let mut e = AbilityEffects::default();
    e.rain(Some(1), 2.0);
    e.update(0.1, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert_eq!(e.rain_pulse.take(), Some((Some(1), 2.0)));
    e.update(0.1, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert_eq!(e.rain_pulse.take(), None);
    e.update(0.15, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(e.rain_pulse.take().is_some(), "{RAIN_WET_PERIOD} s later");
    e.rain(Some(2), 3.0);
    assert_eq!(e.effects.len(), 1);
    assert_eq!(e.rain_state().unwrap().ttl, RAIN_TTL);
    e.update(RAIN_TTL, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(e.rain_state().is_none());
}

/// Water, Water, Fire (Water then Steam) and Space outdoors on Havindr: the
/// ambience plays, the Wizard and a goblin get Wet within a quarter second,
/// the directional light target drops to 0.333 and the intensity follows;
/// after 17.5 s the rain ends, the light comes back and the cue stops.
#[test]
fn rain_combo_wets_everyone_outdoors_and_dims_the_light() {
    use magicka_sim::status::Status;
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    assert!(!s.world.scene_indoors, "Havindr is outdoors");
    s.unlocked_magicks[0] |= 1 << M::Rain as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 8.0).unwrap();
    // A repeated element waits out its ElementManager cooldown.
    press(&mut s, &[WATER]);
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[WATER, FIRE]);
    space(&mut s);
    let mut cast = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "magick_rain");
        if cast {
            break;
        }
    }
    assert!(cast, "magick_rain ambience");
    assert_eq!(s.world.light_target_intensity, 0.333);
    for _ in 0..20 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        s.avatar().character.combat.statuses.has(Status::Wet),
        "wizard wet"
    );
    assert!(
        s.world.npcs[&goblin]
            .character
            .combat
            .statuses
            .has(Status::Wet),
        "goblin wet"
    );
    assert!(s.rain_effect_active(), "magick_rain follows the camera");
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    assert!((s.world.light_intensity - 0.333).abs() < 0.02);
    let mut stopped = false;
    for _ in 0..(17 * 60) {
        frame(&mut s, PlayerInput::default());
        stopped |= s.world.ability_effects.rain_state().is_none()
            && s.audio
                .iter()
                .any(|(r, _)| matches!(r, magicka_runtime::audio::AudioRequest::Stop { .. }));
    }
    assert!(s.world.ability_effects.rain_state().is_none());
    assert!(stopped, "ambience stopped by OnRemove");
    assert_eq!(s.world.light_target_intensity, 1.0);
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(s.world.light_intensity, 1.0);
    assert!(!s.rain_effect_active());
}

/// GameScene `Indoor` (Rain refuses indoors): every retail scene with the
/// element parses to the flag its text says, in either tag case.
#[test]
fn retail_scene_indoor_flags_parse() {
    let Some(root) = content() else {
        return;
    };
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")) {
                out.push(p);
            }
        }
    }
    let mut files = vec![];
    walk(&root.join("Levels"), &mut files);
    let (mut indoor, mut checked) = (0, 0);
    for path in files {
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).to_ascii_lowercase();
        let Some(start) = text.find("<indoor>") else {
            continue;
        };
        let expected = text[start + 8..].trim_start().starts_with("true");
        let scene = magicka_runtime::play_content::read_scene(&path)
            .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
        assert_eq!(scene.indoor, expected, "{}", path.display());
        indoor += usize::from(expected);
        checked += 1;
    }
    assert_eq!((indoor, checked), (48, 103));
}

/// Life, Shield, Earth then Space facing an attacking goblin: Charm picks
/// it, it reads FRIENDLY while charmed, stops harming the Wizard, and the
/// charm effect follows it; Nullify ends the charm. Without an enemy in
/// range the magick fails with the magick_fail cue.
#[test]
fn charm_combo_turns_a_goblin_friendly_until_nullified() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Charm as u8 | 1 << M::Nullify as u8;
    // No enemy around: magick_fail.
    press(&mut s, &[LIFE, SHIELD, EARTH]);
    space(&mut s);
    let mut failed = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        failed |= cue_started(&s, "magick_fail");
    }
    assert!(failed, "magick_fail without a target");

    let goblin = s.spawn_npc("Goblin_pirate", "attack", 3.0).unwrap();
    let wizard = s.avatar().handle;
    press(&mut s, &[LIFE, SHIELD, EARTH]);
    space(&mut s);
    let mut cast = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "magick_charm");
        if cast {
            break;
        }
    }
    assert!(cast, "magick_charm cue");
    let charm = s.world.npcs[&goblin].character.charm.unwrap();
    assert_eq!(charm.owner, wizard);
    assert!(charm.timer > 14.9 && charm.timer <= 15.0);
    let entity = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == goblin)
        .unwrap()
        .clone();
    assert_eq!(s.world.faction(&entity), 4, "charmed NPCs read FRIENDLY");
    let hp = s.avatar().character.combat.health.hit_points;
    for _ in 0..(4 * 60) {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(
        s.avatar().character.combat.health.hit_points,
        hp,
        "a charmed goblin does not attack its charmer"
    );
    assert!(s.charm_emitters().contains(&goblin), "magick_charm follows");

    press(&mut s, &[ARCANE, SHIELD]);
    space(&mut s);
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        if cue_started(&s, "magick_nullify") {
            break;
        }
    }
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(!s.world.npcs[&goblin].character.is_charmed());
    assert_ne!(s.world.faction(&entity), 4);
    assert!(!s.charm_emitters().contains(&goblin));
}

/// Control for the Charm test: the same attacking goblin, uncharmed, hurts
/// the Wizard within the same 4 s.
#[test]
fn uncharmed_attacking_goblin_hurts_the_wizard() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.spawn_npc("Goblin_pirate", "attack", 3.0).unwrap();
    let hp = s.avatar().character.combat.health.hit_points;
    // Same delay as the charm cast took (three element presses and Space).
    for _ in 0..(4 * 60 + 30) {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.avatar().character.combat.health.hit_points < hp);
}

/// Steam (Water+Fire), Lightning, Arcane, Lightning and Space with a goblin
/// 8 m ahead: Thunderbolt flashes the screen, strikes the goblin from 40 m
/// with 5000 Lightning damage (it dies), draws a bolt ending on it, scorches
/// the ground and plays its cue.
#[test]
fn thunderbolt_combo_strikes_the_tallest_enemy_ahead() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::ThunderB as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 8.0).unwrap();
    let hp = s.world.npcs[&goblin].character.combat.health.hit_points;
    // The idle goblin stays put; the strike may remove it (overkill).
    let g = s.world.physics.bodies[s.world.npcs[&goblin].body].position;
    press(&mut s, &[WATER, FIRE, LIGHTNING, ARCANE]);
    for _ in 0..40 {
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[LIGHTNING]);
    space(&mut s);
    let mut cast = false;
    let mut flashed = 0.0f32;
    let mut bolt = None;
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "magick_thunderbolt");
        flashed = flashed.max(s.world.flash_level());
        if bolt.is_none() {
            bolt = s
                .bolts
                .iter()
                .find(|b| b.visual.ttl == 1.0)
                .map(|b| b.visual);
        }
        if cast {
            break;
        }
    }
    assert!(cast, "magick_thunderbolt cue");
    frame(&mut s, PlayerInput::default());
    flashed = flashed.max(s.world.flash_level());
    assert!(flashed > 0.0 && flashed <= 0.75, "flash {flashed}");
    let bolt = bolt
        .or_else(|| {
            s.bolts
                .iter()
                .find(|b| b.visual.ttl == 1.0)
                .map(|b| b.visual)
        })
        .expect("thunderbolt bolt");
    assert!(bolt.from[1] > 35.0, "falls from 40 m: {:?}", bolt.from);
    assert!((bolt.to[0] - g[0]).abs() < 0.5 && (bolt.to[2] - g[2]).abs() < 0.5);
    // 5000 Lightning kills it: dead, or already removed after an overkill.
    if let Some(n) = s.world.npcs.get(&goblin) {
        assert!(
            n.dead() || n.character.combat.health.hit_points < hp - 100.0,
            "struck: {} of {hp}",
            n.character.combat.health.hit_points
        );
    }
}

#[test]
fn confuse_refreshes_per_target_and_reports_its_end() {
    use magicka_sim::magicks::CONFUSE_TTL;
    let mut e = AbilityEffects::default();
    assert!(e.confuse(5), "new target");
    e.update(10.0, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(!e.confuse(5), "same target refreshes");
    assert_eq!(
        e.effects,
        [AbilityEffect::Confuse(magicka_sim::magicks::Confuse {
            target: 5,
            ttl: CONFUSE_TTL
        })]
    );
    e.update(CONFUSE_TTL - 0.01, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(e.confused(5) && e.confuse_ended.is_empty());
    e.update(0.02, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(!e.confused(5));
    assert_eq!(std::mem::take(&mut e.confuse_ended), [5]);
    // A dead target ends it on the next update; ClearMagicks reports too.
    e.confuse(6);
    e.confuse(7);
    e.update(0.1, 1.0, 1.0, |_, _| true, |_| (), |t| t != 6);
    assert_eq!(std::mem::take(&mut e.confuse_ended), [6]);
    e.clear_magicks();
    assert_eq!(e.confuse_ended, [7]);
    assert!(e.effects.is_empty());
}

fn confuse_cast(s: &mut PlaySession, keys: &[usize]) -> bool {
    s.unlocked_magicks[0] |= 1 << M::Confuse as u8 | 1 << M::Nullify as u8;
    press(s, keys);
    space(s);
    for _ in 0..90 {
        frame(s, PlayerInput::default());
        if cue_started(s, "boss_fafnir_confuse") {
            return true;
        }
    }
    false
}

/// Arcane, Shield, Lightning and Space with a goblin ahead: the goblin reads
/// faction NONE with the `confused` effect on it for 15 s, then its template
/// faction comes back.
#[test]
fn confuse_combo_makes_a_goblin_factionless_for_fifteen_seconds() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Confuse as u8;
    press(&mut s, &[ARCANE, SHIELD, LIGHTNING]);
    space(&mut s);
    let mut failed = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        failed |= cue_started(&s, "magick_fail");
    }
    assert!(failed, "magick_fail without a target");
    let goblin = s.spawn_npc("Goblin_pirate", "attack", 3.0).unwrap();
    assert!(confuse_cast(&mut s, &[ARCANE, SHIELD, LIGHTNING]));
    let entity = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == goblin)
        .unwrap()
        .clone();
    assert_eq!(s.world.npcs[&goblin].faction_override, Some(0));
    assert_eq!(s.world.faction(&entity), 0, "Factions.NONE");
    assert!(!s.avatar().controls_inverted);
    frame(&mut s, PlayerInput::default());
    assert_eq!(s.confuse_emitters(), [goblin], "confused effect follows");
    let mut frames = 0;
    while s.world.ability_effects.confused(goblin) && frames < 16 * 60 {
        frame(&mut s, PlayerInput::default());
        frames += 1;
    }
    assert!(
        (14 * 60..=15 * 60).contains(&frames),
        "15 s from the cast, ended after {frames} frames"
    );
    assert_eq!(s.world.npcs[&goblin].faction_override, None);
    assert_eq!(s.world.faction(&entity), entity.faction, "template faction");
    assert!(s.confuse_emitters().is_empty());
}

/// A confused goblin shares no faction bit with a second goblin, which
/// attacks it.
#[test]
fn a_goblin_attacks_its_confused_companion() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let confused = s.spawn_npc("Goblin_pirate", "attack", 3.0).unwrap();
    assert!(confuse_cast(&mut s, &[ARCANE, SHIELD, LIGHTNING]));
    let other = s.spawn_npc("Goblin_pirate", "attack", 5.0).unwrap();
    let hp = |s: &PlaySession, h: u16| {
        s.world
            .npcs
            .get(&h)
            .map(|n| n.character.combat.health.hit_points)
    };
    let (a, b) = (hp(&s, confused), hp(&s, other));
    for _ in 0..(6 * 60) {
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        hp(&s, confused) < a || hp(&s, other) < b,
        "the goblins fight: {a:?}->{:?}, {b:?}->{:?}",
        hp(&s, confused),
        hp(&s, other)
    );
}

/// A confused player: KeyboardMouseController.Invert swaps the element keys
/// in pairs (Fire's key conjures Water), so Nullify (Arcane, Shield) is cast
/// with the Earth and Cold keys and restores the controls.
#[test]
fn a_confused_wizard_has_swapped_element_keys_until_nullified() {
    use magicka_runtime::avatar::INVERTED_KEYS;
    use magicka_sim::spells::Elements as E;
    for (i, &k) in INVERTED_KEYS.iter().enumerate() {
        assert_eq!(INVERTED_KEYS[k], i, "pair swaps");
    }
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Nullify as u8;
    let wizard = s.avatar().handle;
    let at = s.body().position;
    assert!(s.world.confuse_character(at, wizard));
    assert!(s.avatar().controls_inverted);
    press(&mut s, &[FIRE]);
    assert_eq!(s.avatar().conjure.queue[0].element, E::WATER);
    s.world.players[0].as_mut().unwrap().conjure.queue.clear();
    press(&mut s, &[EARTH, COLD]);
    space(&mut s);
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        if cue_started(&s, "magick_nullify") {
            break;
        }
    }
    assert!(!s.avatar().controls_inverted, "ClearMagicks runs OnRemove");
    assert!(!s.world.ability_effects.confused(wizard));
}

/// Control for the companion test: the same two goblins, neither confused,
/// do not hurt each other in the same 6 s (+ the cast time).
#[test]
fn unconfused_goblins_leave_each_other_alone() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let first = s.spawn_npc("Goblin_pirate", "attack", 3.0).unwrap();
    let second = s.spawn_npc("Goblin_pirate", "attack", 5.0).unwrap();
    let hp = |s: &PlaySession, h: u16| {
        s.world
            .npcs
            .get(&h)
            .map(|n| n.character.combat.health.hit_points)
    };
    let (a, b) = (hp(&s, first), hp(&s, second));
    for _ in 0..(6 * 60 + 4 * 30) {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!((hp(&s, first), hp(&s, second)), (a, b));
}

#[test]
fn levitation_hovers_on_a_hit_sinks_faster_without_and_ends_at_thirty_seconds() {
    use magicka_sim::magicks::{Levitation, LEVITATE_TTL};
    let mut l = Levitation::default();
    assert!(!l.set(0.0) && !l.active(false));
    assert!(l.set(LEVITATE_TTL) && l.active(false) && !l.active(true));
    // A hit at fraction 0.6 pushes up by (1 - 0.6) * 0.75 and resets free fall.
    let (dv, ended) = l.tick(0.5, Some(0.6));
    assert!((dv - 0.3).abs() < 1e-6 && !ended);
    // Nothing below: -dt * (0.5 + 1.5 * free_fall), free fall += dt / 2 (max 1).
    let (a, _) = l.tick(1.0, None);
    let (b, _) = l.tick(1.0, None);
    let (c, _) = l.tick(2.0, None);
    let (d, _) = l.tick(1.0, None);
    assert_eq!([a, b, c, d], [-0.5, -1.25, -4.0, -2.0]);
    assert_eq!(l.free_fall, 1.0);
    let (_, ended) = l.tick(LEVITATE_TTL, Some(1.0));
    assert!(ended && l.timer == 0.0);
    let (origin, delta) = Levitation::probe([0.0, 1.0, 0.0], -0.8);
    assert_eq!((origin, delta), ([0.0, 0.6, 0.0], [0.0, -1.15, 0.0]));
}

/// Steam (Water+Fire), Arcane, Steam and Space: Levitate plays the haste
/// cue, turns the Wizard's gravity off with `magick_levitate` at its feet,
/// lifts it to hover 0.75 m up, and Nullify drops it.
#[test]
fn levitate_combo_lifts_the_wizard_until_nullified() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Levitate as u8 | 1 << M::Nullify as u8;
    let ground = s.body().position[1];
    press(&mut s, &[WATER, FIRE, ARCANE, WATER, FIRE]);
    space(&mut s);
    let mut cast = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "magick_haste");
        if cast {
            break;
        }
    }
    assert!(cast, "Levitate uses the haste cue");
    let timer = s.avatar().character.levitation.timer;
    assert!(timer > 29.9 && timer <= 30.0, "SetLevitate(30): {timer}");
    assert!(!s.body().apply_gravity && s.body().owner.levitating);
    frame(&mut s, PlayerInput::default());
    let wizard = s.avatar().handle;
    assert_eq!(s.levitate_emitters(), [wizard]);
    for _ in 0..(5 * 60) {
        frame(&mut s, PlayerInput::default());
    }
    // The probe reaches 0.75 m below the feet: the body settles there.
    let hover = s.body().position[1] - ground;
    assert!((hover - 0.75).abs() < 0.02, "hovers 0.75 m up: {hover}");

    press(&mut s, &[ARCANE, SHIELD]);
    space(&mut s);
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        if cue_started(&s, "magick_nullify") {
            break;
        }
    }
    assert_eq!(s.avatar().character.levitation.timer, 0.0);
    assert!(s.body().apply_gravity && !s.body().owner.levitating);
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.levitate_emitters().is_empty());
    assert!((s.body().position[1] - ground).abs() < 0.05, "back down");
}

/// Arcane, Steam (Water+Fire), Lightning, Shield, Arcane and Space with a
/// Ghost_Acolyte (an IsEthereal template) 3 m ahead and another 40 m away: Corporealize plays its cue and a
/// 0.2 s flash, and only the near Ghost stops being ethereal (the
/// combatant snapshot that spells read follows).
#[test]
fn corporealize_combo_makes_ghosts_within_thirty_metres_solid() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Corporealize as u8;
    let near = s.spawn_npc("Ghost_Acolyte", "attack", 3.0).unwrap();
    let far = s.spawn_npc("Ghost_Acolyte", "attack", 40.0).unwrap();
    let ethereal = |s: &PlaySession, h: u16| {
        let c = s
            .world
            .combatants()
            .into_iter()
            .find(|c| c.handle == h)
            .unwrap();
        (s.world.npcs[&h].character.ethereal, c.ethereal)
    };
    assert!(
        s.world.npcs[&near].character.definition.ethereal,
        "IsEthereal template"
    );
    assert!(
        s.world.npcs[&near].character.definition.looks_ethereal,
        "LooksEthereal decoded independently"
    );
    assert_eq!(ethereal(&s, near), (true, true));
    press(&mut s, &[ARCANE, WATER, FIRE, LIGHTNING, SHIELD, ARCANE]);
    space(&mut s);
    let mut cast = false;
    let mut flash = 0.0f32;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "magick_corporealize");
        flash = flash.max(s.world.flash_level());
        if cast {
            break;
        }
    }
    assert!(cast, "magick_corporealize cue");
    assert!(flash > 0.0, "flash");
    assert!(
        s.world.npcs[&near].character.definition.looks_ethereal,
        "Corporealize preserves the template appearance"
    );
    assert_eq!(ethereal(&s, near), (false, false));
    assert_eq!(ethereal(&s, far), (true, true), "beyond 30 m");
}

/// Fire, Earth and Shift+cast (a ground area blast), once on an ethereal
/// Ghost_Acolyte 2 m ahead and once after Corporealize: only the solid one
/// is hurt.
#[test]
fn corporealized_ghosts_take_area_blast_damage() {
    let Some(root) = content() else {
        return;
    };
    let mut hurt = vec![];
    for corporealize in [false, true] {
        let mut s = session(&root);
        s.unlocked_magicks[0] |= 1 << M::Corporealize as u8;
        let ghost = s.spawn_npc("Ghost_Acolyte", "attack", 2.0).unwrap();
        if corporealize {
            let at = s.body().position;
            s.world.corporealize(at);
        }
        let hp = s.world.npcs[&ghost].character.combat.health.hit_points;
        press(&mut s, &[FIRE, EARTH]);
        for _ in 0..60 {
            frame(
                &mut s,
                PlayerInput {
                    cast_down: true,
                    shift_down: true,
                    ..Default::default()
                },
            );
        }
        let after = s
            .world
            .npcs
            .get(&ghost)
            .map_or(0.0, |n| n.character.combat.health.hit_points);
        hurt.push(after < hp);
    }
    assert_eq!(hurt, [false, true]);
}

#[test]
fn thunderstorm_waits_two_seconds_reports_bolts_and_dies_with_its_rain() {
    use magicka_sim::magicks::{RAIN_TTL, THUNDERSTORM_FIRST_BOLT};
    let mut e = AbilityEffects::default();
    e.rain(Some(1), 3.0);
    e.thunderstorm(Some(1), [1.0, 2.0, 3.0], 3.0);
    e.update_thunderstorm(THUNDERSTORM_FIRST_BOLT - 0.01);
    e.update(
        THUNDERSTORM_FIRST_BOLT - 0.01,
        1.0,
        1.0,
        |_, _| true,
        |_| (),
        |_| true,
    );
    assert!(e.storm_bolts_due.is_none());
    e.update_thunderstorm(0.02);
    e.update(0.02, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    let due = e.storm_bolts_due.take().expect("first bolt after 2 s");
    assert_eq!(
        (due.owner, due.position, due.timestamp),
        (Some(1), [1.0, 2.0, 3.0], 3.0)
    );
    // The host rolls the next timer; without it the bolt stays due.
    e.thunderstorm_mut().unwrap().bolt_ttl = 0.75;
    e.update_thunderstorm(0.5);
    e.update(0.5, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(e.storm_bolts_due.is_none());
    // The Rain's last frame still reports a due bolt, then both are gone.
    e.thunderstorm_mut().unwrap().bolt_ttl = 0.1;
    e.update_thunderstorm(RAIN_TTL);
    e.update(RAIN_TTL, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(e.storm_bolts_due.is_some());
    assert!(e.rain_state().is_none() && e.thunderstorm_state().is_none());
    // Without a Rain the storm is dead at once.
    e.thunderstorm(None, [0.0; 3], 0.0);
    e.update(0.01, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(e.thunderstorm_state().is_none());
}

#[test]
fn thunderstorm_recast_refreshes_singleton_and_clear_cancels_pending_bolts() {
    let mut e = AbilityEffects::default();
    e.rain(Some(1), 0.0);
    e.thunderstorm(Some(1), [0.0; 3], 0.0);
    e.update_thunderstorm(2.1);
    assert!(e.storm_bolts_due.is_some());
    e.clear_magicks();
    assert!(e.storm_bolts_due.is_none());
    e.rain(Some(2), 4.0);
    e.thunderstorm(Some(2), [1.0; 3], 4.0);
    e.update_thunderstorm(1.0);
    e.thunderstorm(Some(3), [2.0; 3], 5.0);
    let t = e.thunderstorm_state().unwrap();
    assert_eq!(
        (t.owner, t.position, t.timestamp, t.bolt_ttl),
        (Some(3), [2.0; 3], 5.0, 2.0)
    );
    assert_eq!(e.effects.len(), 2);
}

#[test]
fn thunderstorm_refuses_indoors_and_clients_do_not_advance_bolts() {
    use magicka_sim::damage_delivery::NetworkRole;
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.world.scene_indoors = true;
    s.world.cast_magick(0, M::ThunderS);
    assert!(s.world.ability_effects.effects.is_empty());
    s.world.scene_indoors = false;
    s.world.role = NetworkRole::Client;
    s.world.cast_magick(0, M::ThunderS);
    for _ in 0..180 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(
        s.world
            .ability_effects
            .thunderstorm_state()
            .unwrap()
            .bolt_ttl,
        2.0
    );
    assert!(s.bolts.is_empty());
    assert!(s.world.ability_effects.storm_bolts_due.is_none());
    s.world.ability_effects.clear_magicks();
    frame(&mut s, PlayerInput::default());
    assert!(s.world.ability_effects.thunderstorm_state().is_none());
    assert!(s.world.ability_effects.rain_state().is_none());
    assert_eq!(s.world.light_target_intensity, 1.0);
}

/// Steam, Steam, Lightning, Arcane, Lightning and Space outdoors (Havindr):
/// Thunderstorm starts its Rain, the first bolts fall 2 s later from 40 m
/// above points within 20 m of the Wizard, one or two every 0.5-1 s, and
/// the storm ends with the Rain after 17.5 s.
#[test]
fn thunderstorm_combo_rains_bolts_around_the_caster_until_the_rain_ends() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::ThunderS as u8;
    press(&mut s, &[WATER, FIRE, WATER, FIRE, LIGHTNING, ARCANE]);
    for _ in 0..40 {
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[LIGHTNING]);
    space(&mut s);
    let mut cast_frame = None;
    for i in 0..120 {
        frame(&mut s, PlayerInput::default());
        if s.world.ability_effects.thunderstorm_state().is_some() {
            cast_frame = Some(i);
            break;
        }
    }
    assert!(cast_frame.is_some(), "storm cast");
    assert!(
        s.world.ability_effects.rain_state().is_some(),
        "with its rain"
    );
    let wizard = s.body().position;
    let mut first = None;
    let mut bolts = 0;
    let mut frames = 0;
    let mut last_serial = 0;
    while s.world.ability_effects.thunderstorm_state().is_some() && frames < 20 * 60 {
        frame(&mut s, PlayerInput::default());
        frames += 1;
        for b in s.bolts.iter().filter(|b| b.serial > last_serial) {
            first.get_or_insert(frames);
            bolts += 1;
            let to = b.visual.to;
            let flat = ((to[0] - wizard[0]).powi(2) + (to[2] - wizard[2]).powi(2)).sqrt();
            assert!(flat <= 25.5, "within 20 m (+5 m target reach): {flat}");
            assert!(
                (b.visual.from[1] - to[1] - 40.0).abs() < 1e-3,
                "from 40 m above"
            );
        }
        if let Some(serial) = s.bolts.iter().map(|b| b.serial).max() {
            last_serial = serial;
        }
    }
    let first = first.expect("bolts");
    assert!(
        (119..=121).contains(&first),
        "first bolt after 2 s: frame {first}"
    );
    // (17.5 - 2) s of bolts: 1-2 every 0.5-1 s.
    assert!((15..=64).contains(&bolts), "{bolts} bolts");
    assert!(
        (17 * 60..=18 * 60).contains(&frames),
        "ends with the rain: {frames}"
    );
    assert!(s.world.ability_effects.rain_state().is_none());
}

#[test]
fn blizzard_combo_chills_characters_and_fades_before_stopping_ambience() {
    use magicka_sim::status::Status;
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Blizzard as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 8.0).unwrap();
    press(&mut s, &[COLD]);
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[COLD, WATER]);
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[COLD]);
    space(&mut s);
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
        if s.world.ability_effects.blizzard_state().is_some() {
            break;
        }
    }
    assert!(
        s.world.ability_effects.blizzard_state().is_some(),
        "Cold/Ice/Cold cast"
    );
    assert!(cue_started(&s, "magick_blizzard"));
    for _ in 0..20 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.avatar().character.combat.statuses.has(Status::Cold));
    assert!(s.world.npcs[&goblin]
        .character
        .combat
        .statuses
        .has(Status::Cold));
    assert_eq!(
        s.world.light_target_intensity, 1.0,
        "Blizzard does not dim light like Rain"
    );
    let mut fading = false;
    for _ in 0..12 * 60 {
        frame(&mut s, PlayerInput::default());
        let Some(b) = s.world.ability_effects.blizzard_state() else {
            break;
        };
        if b.ttl <= 0.0 {
            fading = true;
            assert!(b.alpha > 0.0);
        }
    }
    assert!(fading, "visible tail beyond TTL");
    assert!(s.world.ability_effects.blizzard_state().is_none());
    assert!(
        s.audio
            .iter()
            .any(|(r, _)| matches!(r, magicka_runtime::audio::AudioRequest::Stop { .. })),
        "ambience stops at removal"
    );
}

#[test]
fn blizzard_respects_spheres_and_cold_immunity_and_nullify_removes_it() {
    use magicka_sim::{
        shield_entity::{ShieldKind, MAX_HIT_POINTS, SHIELD_COLOR},
        status::Status,
    };
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 8.0).unwrap();
    let immune = s.spawn_npc("Goblin_pirate", "idle", 16.0).unwrap();
    s.world
        .npcs
        .get_mut(&immune)
        .unwrap()
        .character
        .definition
        .resistances[magicka_sim::spells::Elements::COLD.index() as usize]
        .multiplier = 0.0;
    let shield = s.world.spawn_shield(
        s.avatar().handle,
        s.body().position,
        3.0,
        [0.0, 0.0, 1.0],
        ShieldKind::Sphere,
        MAX_HIT_POINTS,
        SHIELD_COLOR,
    );
    s.world.scene_indoors = true;
    s.world.cast_magick(0, M::Blizzard);
    assert!(s.world.ability_effects.blizzard_state().is_none());
    assert!(!cue_started(&s, "magick_blizzard"));
    s.world.scene_indoors = false;
    s.world.cast_magick(0, M::Blizzard);
    for _ in 0..20 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        !s.avatar().character.combat.statuses.has(Status::Cold),
        "sphere protects"
    );
    assert!(s.world.npcs[&goblin]
        .character
        .combat
        .statuses
        .has(Status::Cold));
    assert!(!s.world.npcs[&immune]
        .character
        .combat
        .statuses
        .has(Status::Cold));
    s.world
        .shields
        .iter_mut()
        .find(|s| s.handle == shield)
        .unwrap()
        .shield
        .dead = true;
    for _ in 0..20 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.avatar().character.combat.statuses.has(Status::Cold));
    s.world.nullify();
    assert!(s.world.ability_effects.blizzard_state().is_none());
    frame(&mut s, PlayerInput::default());
    assert!(!s.avatar().character.combat.statuses.has(Status::Cold));
}

#[test]
fn blizzard_freezes_freezable_water_and_lava_even_on_clients() {
    use magicka_content::level::TriMesh;
    use magicka_runtime::water_collision::WaterCollision;
    use magicka_sim::{damage_delivery::NetworkRole, status::Status};
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let mesh = TriMesh {
        vertices: vec![
            [1000.0, 0.0, 1000.0],
            [1001.0, 0.0, 1000.0],
            [1000.0, 0.0, 1001.0],
        ],
        triangles: vec![[0, 2, 1]],
    };
    let mut liquids = vec![];
    for (lava, freezable) in [(false, true), (true, true), (false, false)] {
        let mut liquid = WaterCollision::from_geometry(&mesh, freezable, false).unwrap();
        liquid.lava = lava;
        liquids.push(liquid);
    }
    s.world.collision.collision_mut().water = liquids;
    s.world.role = NetworkRole::Client;
    s.world.cast_magick(0, M::Blizzard);
    frame(&mut s, PlayerInput::default());
    let liquids = &s.world.collision.collision().water;
    assert!(liquids[0].freeze.vertices.iter().all(|&v| v >= 0.24));
    assert!(liquids[1].freeze.vertices.iter().all(|&v| v >= 0.24));
    assert!(liquids[2].freeze.vertices.iter().all(|&v| v == 0.0));
    assert!(
        !s.avatar().character.combat.statuses.has(Status::Cold),
        "damage remains authoritative"
    );
}

/// The custom Etherealize combo (Arcane, Lightning, Shield, Arcane) and
/// Space: the Wizard turns ethereal with no timer (the combatant snapshot
/// follows) until a Corporealize makes it solid again.
#[test]
fn etherealize_combo_lasts_until_corporealize() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Etherealize as u8;
    let wizard = s.avatar().handle;
    let snapshot = |s: &PlaySession| {
        s.world
            .combatants()
            .into_iter()
            .find(|c| c.handle == wizard)
            .unwrap()
            .ethereal
    };
    assert!(!s.avatar().character.ethereal && !snapshot(&s));
    press(&mut s, &[ARCANE, LIGHTNING, SHIELD, ARCANE]);
    space(&mut s);
    let mut cast = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "chr_daemon_etherealize3");
        if cast {
            break;
        }
    }
    assert!(cast, "etherealize cue");
    assert!(s.avatar().character.ethereal && snapshot(&s));
    for _ in 0..(60 * 60) {
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        s.avatar().character.ethereal,
        "no timer: still ethereal after 60 s"
    );
    let at = s.body().position;
    s.world.corporealize(at);
    assert!(!s.avatar().character.ethereal && !snapshot(&s));
}

fn grease_cast(s: &mut PlaySession) {
    let spray_hash = magicka_runtime::scene_loading::custom_hash("magick_grease_spray");
    s.unlocked_magicks[0] |= 1 << M::Grease as u8;
    press(s, &[WATER, EARTH, LIFE]);
    space(s);
    for _ in 0..90 {
        frame(s, PlayerInput::default());
        if !s.world.ability_effects.grease_sprays().is_empty() {
            break;
        }
    }
    assert!(
        !s.world.ability_effects.grease_sprays().is_empty(),
        "grease cast"
    );
    assert_eq!(
        s.effects.active_effects_by_hash(spray_hash),
        1,
        "spray started"
    );
    for _ in 0..40 {
        frame(s, PlayerInput::default());
    }
    assert!(
        s.world.ability_effects.grease_sprays().is_empty(),
        "0.5 s spray"
    );
    assert_eq!(
        s.effects.active_effects_by_hash(spray_hash),
        0,
        "expired spray stopped"
    );
    // Let cast_magick_sweep finish so the next cast press is not swallowed.
    for _ in 0..240 {
        if s.avatar().motion.state == AvatarState::Idle {
            break;
        }
        frame(s, PlayerInput::default());
    }
}

/// Water, Earth, Life and Space: the 0.5 s spray lays six fields in an arc
/// in front of the Wizard; the goblin standing in them is on grease (its
/// Greased status is natively rejected, see the assertion).
#[test]
fn grease_combo_lays_six_fields_a_goblin_slips_on() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    grease_cast(&mut s);
    let wizard = s.body().position;
    let fields: Vec<_> = s.world.grease.iter().map(|(_, f)| f.position).collect();
    assert_eq!(fields.len(), 6, "{fields:?}");
    for f in &fields {
        let d = ((f[0] - wizard[0]).powi(2) + (f[2] - wizard[2]).powi(2)).sqrt();
        assert!(d <= 6.0, "within the 6 m spray: {d}");
    }
    // Native quirk: the spray's Greased has DPS 0, which AddStatusEffect
    // rejects (|DPS| < float.Epsilon) for a goblin without a Water
    // modifier, so the ray greases nobody here (research/grease.md).
    assert!(!s.world.npcs[&goblin]
        .character
        .combat
        .statuses
        .has(magicka_sim::status::Status::Greased));
    assert!(!s
        .avatar()
        .character
        .combat
        .statuses
        .has(magicka_sim::status::Status::Greased));
    let goblin_body = s.world.npcs[&goblin].body;
    assert!(
        s.world.grease_touching(goblin_body),
        "goblin stands in grease"
    );
}

/// Fire ignites a field, the fire spreads to the touching fields and burns a
/// goblin standing in them; burning fields burn out three times as fast.
/// Nullify then removes every field.
#[test]
fn fire_ignites_grease_which_spreads_burns_and_nullify_clears() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Nullify as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    grease_cast(&mut s);
    let hp = s.world.npcs[&goblin].character.combat.health.hit_points;
    // A Fire spray at the fields.
    press(&mut s, &[FIRE]);
    for _ in 0..60 {
        frame(
            &mut s,
            PlayerInput {
                cast_down: true,
                ..Default::default()
            },
        );
    }
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let burning = s
        .world
        .grease
        .iter()
        .filter(|(_, f)| f.field.burning)
        .count();
    let total = s.world.grease.len();
    assert!(burning >= 2, "ignited and spread: {burning} of {total}");
    assert!(!s.grease_burning_emitters().is_empty(), "burning effect");
    let after = s
        .world
        .npcs
        .get(&goblin)
        .map_or(0.0, |n| n.character.combat.health.hit_points);
    assert!(after < hp, "the goblin burns: {hp} -> {after}");
    let lit_ttl = s
        .world
        .grease
        .iter()
        .filter(|(_, f)| f.field.burning)
        .map(|(_, f)| f.field.ttl)
        .fold(f32::MAX, f32::min);
    assert!(lit_ttl < 56.0, "burning triples the decay: {lit_ttl}");
    press(&mut s, &[ARCANE, SHIELD]);
    space(&mut s);
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        if s.world.grease.is_empty() {
            break;
        }
    }
    assert!(s.world.grease.is_empty(), "Nullify kills grease fields");
}

/// The custom GreaseSplash (Water, Earth, Life, Shield, Earth): seven fields
/// at 1-4 m around the Wizard, at Y = 0 as the original places them.
#[test]
fn grease_splash_drops_seven_fields_around_the_caster() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::GreaseSplash as u8;
    let wizard = s.body().position;
    press(&mut s, &[WATER, EARTH, LIFE, SHIELD, EARTH]);
    space(&mut s);
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        if !s.world.grease.is_empty() {
            break;
        }
    }
    let fields: Vec<_> = s.world.grease.iter().map(|(_, f)| f.position).collect();
    assert_eq!(fields.len(), 7);
    for f in fields {
        let d = ((f[0] - wizard[0]).powi(2) + (f[2] - wizard[2]).powi(2)).sqrt();
        assert!((1.0..=4.0 + 1e-3).contains(&d), "{d}");
        assert_eq!(f[1], 0.0);
    }
}

/// One ignited field sets its touching neighbour alight (240 Fire status
/// damage per hit-list period) and burns a goblin standing on it; an unlit
/// field 10 m away stays unlit.
#[test]
fn a_burning_field_ignites_its_neighbour_and_burns_whoever_stands_there() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 6.0).unwrap();
    let g = s.world.physics.bodies[s.world.npcs[&goblin].body].position;
    let feet = [g[0], g[1] - 0.8, g[2]];
    let lit = s.world.spawn_grease_field(
        None,
        [feet[0] + 2.0, feet[1], feet[2]],
        [0.0, 1.0, 0.0],
        None,
    );
    let near = s
        .world
        .spawn_grease_field(None, feet, [0.0, 1.0, 0.0], None);
    let far = s.world.spawn_grease_field(
        None,
        [feet[0] + 12.0, feet[1], feet[2]],
        [0.0, 1.0, 0.0],
        None,
    );
    s.world.grease.get_mut(lit).unwrap().field.burn(1.0);
    let hp = s.world.npcs[&goblin].character.combat.health.hit_points;
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.grease.get(near).unwrap().field.burning, "spread");
    assert!(!s.world.grease.get(far).unwrap().field.burning, "too far");
    let after = s
        .world
        .npcs
        .get(&goblin)
        .map_or(0.0, |n| n.character.combat.health.hit_points);
    assert!(after < hp, "burnt: {hp} -> {after}");
}

/// Steam, Earth, Life, Fire, Fire and Space outdoors: Napalm marks a smoke
/// target 15 m ahead, the plane passes 5 s later and six hits across the
/// target lay burning grease fields; a second strike on a goblin blasts it,
/// and a recast during its airstrike fails.
#[test]
fn napalm_combo_airstrikes_the_target_fifteen_metres_ahead() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Napalm as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    let hp = s.world.npcs[&goblin].character.combat.health.hit_points;
    press(&mut s, &[WATER, FIRE, EARTH, LIFE, FIRE]);
    for _ in 0..40 {
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[FIRE]);
    space(&mut s);
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        if s.world.ability_effects.napalm_state().is_some() {
            break;
        }
    }
    let n = *s.world.ability_effects.napalm_state().expect("napalm cast");
    let wizard = s.body().position;
    let d = ((n.target[0] - wizard[0]).powi(2) + (n.target[2] - wizard[2]).powi(2)).sqrt();
    assert!((d - 15.0).abs() < 0.5, "target 15 m ahead: {d}");
    frame(&mut s, PlayerInput::default());
    assert!(s.napalm_smoke_active(), "smoke target");
    let mut frames = 0;
    while !s.world.ability_effects.napalm_state().unwrap().airstrike {
        frame(&mut s, PlayerInput::default());
        frames += 1;
        assert!(frames < 6 * 60);
    }
    assert!(cue_started(&s, "magick_plane_passby"), "plane passes");
    assert!(s.world.grease.is_empty(), "no hits before the strike");
    while s.world.ability_effects.napalm_state().is_some() && frames < 9 * 60 {
        frame(&mut s, PlayerInput::default());
        frames += 1;
    }
    assert!(
        s.world.ability_effects.napalm_state().is_none(),
        "done after 2 s"
    );
    let fields = s.world.grease.len();
    assert!((3..=6).contains(&fields), "hits lay grease: {fields}");
    assert!(s.world.grease.iter().all(|(_, f)| f.field.burning), "lit");
    assert!(!s.napalm_smoke_active(), "smoke stops with the strike");
    // The combo's target lies off the goblin's navmesh, so strike the
    // goblin itself: a caster 15 m behind it along +X.
    let wizard_handle = s.avatar().handle;
    let g = s.world.physics.bodies[s.world.npcs[&goblin].body].position;
    assert!(s
        .world
        .napalm(wizard_handle, [g[0] - 15.0, g[1], g[2]], [1.0, 0.0, 0.0]));
    // A recast during the airstrike fails.
    for _ in 0..(5 * 60 + 50) {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.ability_effects.napalm_state().unwrap().airstrike);
    assert!(!s
        .world
        .napalm(wizard_handle, [g[0] - 15.0, g[1], g[2]], [1.0, 0.0, 0.0]));
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
    }
    let after = s
        .world
        .npcs
        .get(&goblin)
        .map_or(0.0, |n| n.character.combat.health.hit_points);
    assert!(after < hp, "the goblin is blasted: {hp} -> {after}");
}

#[test]
fn meteor_shower_refreshes_spawns_while_over_two_and_a_half_seconds() {
    use magicka_sim::magicks::{METEOR_LAST_SPAWN, METEOR_TTL};
    let mut e = AbilityEffects::default();
    assert!(e.meteor_shower(Some(1), [0.0; 3]));
    // The singleton timer starts at 0: the first meteors fall at once.
    e.update(0.01, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(e.meteors_due.take().is_some());
    e.meteor_timer = 0.5;
    e.update(0.4, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(e.meteors_due.is_none());
    assert!(!e.meteor_shower(Some(2), [5.0; 3]), "refresh");
    let m = *e.meteor_shower_state().unwrap();
    assert_eq!(
        (m.owner, m.position, m.ttl),
        (Some(1), [0.0; 3], METEOR_TTL)
    );
    // Below 2.5 s nothing spawns, however late the timer.
    e.update(
        METEOR_TTL - METEOR_LAST_SPAWN,
        1.0,
        1.0,
        |_, _| true,
        |_| (),
        |_| true,
    );
    e.meteors_due = None;
    e.meteor_timer = -1.0;
    e.update(1.0, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(e.meteors_due.is_none());
    e.update(2.0, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert!(e.meteor_shower_state().is_none());
}

/// Meteor Shower outdoors (Havindr): the light target dims to 0.4,
/// Earth/Fire meteors fall from 146 m around the Wizard, the shower lasts
/// 17.5 s and the light target returns to 1.
#[test]
fn meteor_shower_drops_meteors_and_dims_the_light() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    let wizard = s.avatar().handle;
    let at = s.body().position;
    assert!(s.world.meteor_shower(Some(wizard), at));
    assert!((s.world.light_target_intensity - 0.4).abs() < 1e-6);
    let mut seen = 0;
    let mut highest = f32::MIN;
    let mut frames = 0;
    while s.world.ability_effects.meteor_shower_state().is_some() && frames < 19 * 60 {
        frame(&mut s, PlayerInput::default());
        frames += 1;
        for m in &s.world.missiles {
            highest = highest.max(m.body.position[1] - at[1]);
        }
        seen = seen.max(s.world.missiles.len());
    }
    assert!((17 * 60..=18 * 60).contains(&frames), "17.5 s: {frames}");
    assert!(highest > 140.0, "falls from 146 m: {highest}");
    assert!(seen >= 1, "meteors in flight");
    assert_eq!(s.world.light_target_intensity, 1.0);
}

fn summon_undead_cast(s: &mut PlaySession) -> Option<u16> {
    let before: Vec<u16> = s.world.npcs.keys().copied().collect();
    press(s, &[WATER, COLD, EARTH, ARCANE]);
    for _ in 0..40 {
        frame(s, PlayerInput::default());
    }
    press(s, &[COLD]);
    space(s);
    for _ in 0..120 {
        frame(s, PlayerInput::default());
        if let Some(h) = s.world.npcs.keys().find(|h| !before.contains(h)) {
            return Some(*h);
        }
    }
    None
}

/// Ice, Earth, Arcane, Cold and Space: Summon Undead raises one of the five
/// undead templates about 4 m ahead as the Wizard's summon with its faction
/// (so it leaves the Wizard alone); a second cast overkills the first
/// (one undead per master), and Nullify kills the summon.
#[test]
fn summon_undead_raises_one_faithful_undead_per_master() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::SUndead as u8 | 1 << M::Nullify as u8;
    let wizard = s.avatar().handle;
    let at = s.body().position;
    let first = summon_undead_cast(&mut s).expect("an undead rises");
    let n = &s.world.npcs[&first];
    assert!(
        [
            "ghoul",
            "skeleton_wight_friendly",
            "skeleton_darksoul_arcane",
            "skeleton_darksoul_frost",
            "skeleton_darksoul_lightning"
        ]
        .contains(&n.definition.id.to_ascii_lowercase().as_str()),
        "{}",
        n.definition.id
    );
    assert_eq!(
        n.summon,
        Some((wizard, magicka_sim::summons::SummonKind::Undead))
    );
    let p = s.world.physics.bodies[n.body].position;
    let d = ((p[0] - at[0]).powi(2) + (p[2] - at[2]).powi(2)).sqrt();
    assert!((2.0..=6.0).contains(&d), "about 4 m ahead: {d}");
    let e = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == first)
        .unwrap()
        .clone();
    let w = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == wizard)
        .unwrap()
        .clone();
    assert_eq!(
        s.world.faction(&e),
        s.world.faction(&w),
        "the master's faction"
    );
    let hp = s.avatar().character.combat.health.hit_points;
    for _ in 0..(3 * 60) {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(
        s.avatar().character.combat.health.hit_points,
        hp,
        "faithful"
    );
    let second = summon_undead_cast(&mut s).expect("a second undead");
    assert!(s
        .world
        .npcs
        .get(&first)
        .is_none_or(|n| n.character.combat.overkilled));
    assert_eq!(
        s.world.summons[&wizard]
            .summons()
            .map(|(h, _)| h)
            .collect::<Vec<_>>(),
        [second]
    );
    for _ in 0..240 {
        if s.avatar().motion.state == AvatarState::Idle {
            break;
        }
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[ARCANE, SHIELD]);
    space(&mut s);
    let mut nullified = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        nullified |= cue_started(&s, "magick_nullify");
        if nullified {
            break;
        }
    }
    assert!(nullified, "nullify cast");
    // Character.Kill zeroes the hit points; DeadState then Die() once the
    // death clip has played.
    assert_eq!(
        s.world
            .npcs
            .get(&second)
            .map(|n| n.character.combat.health.hit_points),
        Some(0.0),
        "Nullify kills summons"
    );
    for _ in 0..(5 * 60) {
        if s.world.npcs.get(&second).is_none_or(|n| n.dead()) {
            break;
        }
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.npcs.get(&second).is_none_or(|n| n.dead()));
}

/// Lightning, Lightning, Fire, Life and Space with a goblin nearby: CTD
/// terminates one random character within 20 m (the caster can be the
/// victim): it is dead at once, with no death clip, and an NPC is removed
/// on its next update.
#[test]
fn ctd_terminates_one_random_character_within_twenty_metres() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Ctd as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    press(&mut s, &[LIGHTNING]);
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[LIGHTNING, FIRE, LIFE]);
    space(&mut s);
    let mut cast = false;
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "magick_ctd");
        if cast {
            break;
        }
    }
    assert!(cast, "magick_ctd cue");
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    let goblin_gone = s.world.npcs.get(&goblin).is_none_or(|n| n.dead());
    let wizard_gone = s.avatar().dead();
    assert!(
        goblin_gone ^ wizard_gone,
        "exactly one: goblin {goblin_gone} wizard {wizard_gone}"
    );
    if goblin_gone {
        assert!(s
            .world
            .entities
            .iter()
            .any(|e| e.handle == goblin && e.removed));
    }
}

/// Earth, Arcane and Space: Tractor Pull drags a goblin within 15 m towards
/// the Wizard (a Pushed Earth hit from its far side).
#[test]
fn tractor_pull_drags_a_goblin_towards_the_caster() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::TractorPull as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 6.0).unwrap();
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    let flat = |s: &PlaySession| {
        let g = s.world.physics.bodies[s.world.npcs[&goblin].body].position;
        let w = s.body().position;
        ((g[0] - w[0]).powi(2) + (g[2] - w[2]).powi(2)).sqrt()
    };
    let before = flat(&s);
    press(&mut s, &[EARTH, ARCANE]);
    space(&mut s);
    let mut cast = false;
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "spell_push_area");
        if cast {
            break;
        }
    }
    assert!(cast, "spell_push_area cue");
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    let after = flat(&s);
    assert!(after < before - 0.5, "pulled closer: {before} -> {after}");
}

/// Fire, Steam, Arcane and Space: ProppMagick lobs a 0.6 m plasma ball
/// upwards (18 m/s) that explodes within its 8 s timer.
#[test]
fn propp_magick_lobs_a_plasma_ball_that_explodes() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::ProppMagick as u8;
    // Fire, then Fire + Water (Steam with the last Fire), then Arcane.
    // (The second Fire waits out the 0.125 s element cooldown.)
    press(&mut s, &[FIRE]);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[FIRE, WATER, ARCANE]);
    space(&mut s);
    let mut ball = None;
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
        if let Some(m) = s.world.missiles.iter().find(|m| m.body.radius == 0.6) {
            ball = Some(m.body.id);
            break;
        }
    }
    let id = ball.expect("plasma ball");
    let mut frames = 0;
    while s
        .world
        .missiles
        .iter()
        .any(|m| m.body.id == id && !m.body.dead)
        && frames < 9 * 60
    {
        frame(&mut s, PlayerInput::default());
        frames += 1;
    }
    assert!(
        frames < 8 * 60 + 5,
        "explodes or times out within 8 s: {frames}"
    );
}

/// Lightning ×3 and Space: Chain Lightning (Zap) strikes the goblin ahead
/// with 850 Lightning and hops on to the next one within 90° of the bolt.
#[test]
fn chain_lightning_hops_between_goblins() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::ChainLightning as u8;
    let a = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    let b = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    let hp = |s: &PlaySession, h: u16| {
        s.world
            .npcs
            .get(&h)
            .map_or(0.0, |n| n.character.combat.health.hit_points)
    };
    let (ha, hb) = (hp(&s, a), hp(&s, b));
    for i in 0..3 {
        press(&mut s, &[LIGHTNING]);
        if i < 2 {
            for _ in 0..10 {
                frame(&mut s, PlayerInput::default());
            }
        }
    }
    // Stand the second goblin 2 m beyond the first, along the line from
    // the Wizard (the chain hops within 90° of the previous bolt).
    let pa = s.world.physics.bodies[s.world.npcs[&a].body].position;
    let w = s.body().position;
    let len = ((pa[0] - w[0]).powi(2) + (pa[2] - w[2]).powi(2)).sqrt();
    let along = [(pa[0] - w[0]) / len, 0.0, (pa[2] - w[2]) / len];
    let bb = s.world.npcs[&b].body;
    s.world.physics.bodies[bb].position = [pa[0] + along[0] * 2.0, pa[1], pa[2] + along[2] * 2.0];
    space(&mut s);
    let mut cast = false;
    for _ in 0..90 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "magick_thunderbolt");
        if cast {
            break;
        }
    }
    assert!(cast, "thunderbolt cue");
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(hp(&s, a) < ha, "first goblin struck: {ha} -> {}", hp(&s, a));
    assert!(
        hp(&s, b) < hb,
        "chained to the second: {hb} -> {}",
        hp(&s, b)
    );
}

#[test]
fn performance_boosts_for_six_seconds_then_withdraws_for_four() {
    use magicka_sim::magicks::PERFORMANCE_TTL;
    let mut e = AbilityEffects::default();
    assert!(e.performance(3));
    let mut factors = vec![];
    e.update(
        1.0,
        1.0,
        1.0,
        |owner, f| {
            assert_eq!(owner, 3);
            factors.push(f);
            true
        },
        |_| (),
        |_| true,
    );
    // TTL 9: 1 + (1 - 0.5^5).
    assert!((factors[0] - (2.0 - 0.5f32.powi(5))).abs() < 1e-6);
    assert!(e.performance_withdrawn.is_empty());
    e.update(
        5.5,
        1.0,
        1.0,
        |_, f| {
            // TTL 3.5: withdrawal 0.5 + (0.5 - 0.75^3.5).
            assert!((f - (1.0 - 0.75f32.powf(3.5))).abs() < 1e-5);
            true
        },
        |_| (),
        |_| true,
    );
    assert_eq!(std::mem::take(&mut e.performance_withdrawn), [3]);
    assert!(!e.performance(3), "refresh");
    let p = *e.performance_state(3).unwrap();
    assert_eq!((p.ttl, p.boosted), (PERFORMANCE_TTL, true));
    e.update(PERFORMANCE_TTL, 1.0, 1.0, |_, _| true, |_| (), |_| true);
    assert_eq!(e.performance_ended, [3]);
    assert!(e.performance_state(3).is_none());
}

/// Life, Fire, Lightning, Fire, Life and Space: Performance Enchantment
/// gives the Wizard a self aura halving all damage (10 s), the boost effect
/// and the buff cue; the withdrawal cue plays 6 s in, and at 10 s ClearAura
/// ends its self auras.
#[test]
fn performance_enchantment_buffs_then_withdraws_the_wizard() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::PerformanceEnchantment as u8;
    press(&mut s, &[LIFE, FIRE, LIGHTNING]);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[FIRE]);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[LIFE]);
    space(&mut s);
    let wizard = s.avatar().handle;
    let mut cast = false;
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "woot_magick_enchantment_buff");
        if cast {
            break;
        }
    }
    assert!(cast, "buff cue");
    let aura = s
        .avatar()
        .character
        .auras
        .iter()
        .find(|a| a.resistance.multiplier == 0.5)
        .copied()
        .expect("resistance aura");
    assert_eq!(aura.resistance.against, magicka_sim::spells::Elements::ALL);
    frame(&mut s, PlayerInput::default());
    assert_eq!(s.performance_emitters(), [wizard]);
    let mut withdrawn = false;
    let mut frames = 0;
    while s.world.ability_effects.performance_state(wizard).is_some() && frames < 11 * 60 {
        frame(&mut s, PlayerInput::default());
        frames += 1;
        if !withdrawn && cue_started(&s, "woot_magick_enchantment_debuff") {
            withdrawn = true;
            let t = s
                .world
                .ability_effects
                .performance_state(wizard)
                .unwrap()
                .ttl;
            assert!(t < 4.0 && t > 3.9, "withdrawal at 4 s left: {t}");
            frame(&mut s, PlayerInput::default());
            assert!(s.performance_emitters().is_empty(), "effect stops");
        }
    }
    assert!(withdrawn, "debuff cue");
    assert!(
        s.avatar()
            .character
            .auras
            .iter()
            .filter(|a| a.self_casted)
            .all(|a| a.ttl <= 0.0),
        "ClearAura"
    );
}

#[test]
fn judgement_spray_fires_twenty_missiles_in_one_second() {
    let mut e = AbilityEffects::default();
    e.judgement_spray(4);
    let mut due = 0;
    let mut frames = 0;
    while e
        .effects
        .iter()
        .any(|x| matches!(x, AbilityEffect::Judgement { .. }))
    {
        e.update(1.0 / 60.0, 1.0, 1.0, |_, _| true, |_| (), |_| true);
        due += std::mem::take(&mut e.judgement_due).len();
        frames += 1;
    }
    // The interval starts at 0, so the first missile fires at once; then
    // every 1/20 s (3 frames at 60 fps).
    assert!((59..=61).contains(&frames), "{frames}");
    assert!((19..=21).contains(&due), "{due} missiles");
}

/// Ice, Ice, Arcane, Shield and Space: Judgement Spray fires about 20
/// missiles alternating left and right, each homing on the goblin standing
/// 8 m ahead (held in place): their horizontal heading turns towards it.
/// (In this scene they fall short from staff height; whether the original
/// reaches 8 m targets is open, see research/judgement-spray.md.)
#[test]
fn judgement_spray_missiles_home_on_the_goblin_ahead() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::JudgementSpray as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 6.0).unwrap();
    press(&mut s, &[WATER, COLD]);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[WATER, COLD, ARCANE, SHIELD]);
    let (w, f) = (s.body().position, s.body().direction());
    let spot = [w[0] + f[0] * 8.0, w[1], w[2] + f[2] * 8.0];
    let gb = s.world.npcs[&goblin].body;
    let hold = |s: &mut PlaySession| {
        s.world.physics.bodies[gb].position = spot;
        s.world.physics.bodies[gb].velocity = [0.0; 3];
    };
    hold(&mut s);
    space(&mut s);
    let mut cast = false;
    for _ in 0..120 {
        hold(&mut s);
        frame(&mut s, PlayerInput::default());
        cast |= cue_started(&s, "woot_magick_judgment_fire");
        if cast {
            break;
        }
    }
    assert!(cast, "fire cue");
    // Horizontal angle between each missile's velocity and its line to the
    // goblin, at first sight and later.
    let heading = |p: [f32; 3], v: [f32; 3]| {
        let to = [spot[0] - p[0], spot[2] - p[2]];
        let (lt, lv) = (
            (to[0] * to[0] + to[1] * to[1]).sqrt(),
            (v[0] * v[0] + v[2] * v[2]).sqrt(),
        );
        ((to[0] * v[0] + to[1] * v[2]) / (lt * lv))
            .clamp(-1.0, 1.0)
            .acos()
    };
    let mut first = std::collections::BTreeMap::new();
    let mut last = std::collections::BTreeMap::new();
    for _ in 0..(2 * 60) {
        hold(&mut s);
        frame(&mut s, PlayerInput::default());
        for m in s
            .world
            .missiles
            .iter()
            .filter(|m| m.body.homing.is_some() && !m.body.dead)
        {
            assert_eq!(
                m.body.homing.unwrap().target,
                Some(goblin),
                "targets the goblin"
            );
            let a = heading(m.body.position, m.body.velocity);
            first.entry(m.body.id).or_insert(a);
            last.insert(m.body.id, a);
        }
    }
    assert!(first.len() >= 15, "about 20 missiles: {}", first.len());
    let turned = first.iter().filter(|(id, a)| last[id] < **a - 0.2).count();
    assert!(
        turned * 2 > first.len(),
        "most turn towards it: {turned} of {}",
        first.len()
    );
}

/// Amalgameddon's registered combo (Arcane, Water, Lightning) cannot be
/// typed: Water and Lightning cancel in the queue (native TryAddToQueue has
/// no magick exception). Executed directly with two pirate goblins, it
/// lifts both (gravity off) until the first is above Y 5, pushes them
/// apart and smashes them together: one is overkilled, the other absorbs
/// its health (capped at the maximum) and falls again.
#[test]
fn amalgameddon_merges_two_goblins_of_one_type() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Amalgameddon as u8;
    press(&mut s, &[ARCANE, WATER, LIGHTNING]);
    assert_eq!(
        s.avatar()
            .conjure
            .queue
            .iter()
            .map(|q| q.element)
            .collect::<Vec<_>>(),
        [magicka_sim::spells::Elements::ARCANE],
        "Water and Lightning cancel"
    );
    let a = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    let b = s.spawn_npc("Goblin_pirate", "idle", 5.0).unwrap();
    s.world
        .npcs
        .get_mut(&a)
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 300.0;
    let wizard = s.avatar().handle;
    assert!(s.world.amalgameddon(wizard));
    assert!(!s.world.amalgameddon(wizard), "one at a time");
    let body = |s: &PlaySession, h: u16| {
        s.world
            .npcs
            .get(&h)
            .map(|n| s.world.physics.bodies[n.body].clone())
    };
    assert!(!body(&s, a).unwrap().apply_gravity && !body(&s, b).unwrap().apply_gravity);
    let active = |s: &PlaySession| {
        s.world
            .ability_effects
            .effects
            .iter()
            .any(|e| matches!(e, AbilityEffect::Amalgameddon(_)))
    };
    let mut highest = f32::MIN;
    let mut frames = 0;
    while active(&s) && frames < 11 * 60 {
        frame(&mut s, PlayerInput::default());
        frames += 1;
        for h in [a, b] {
            if let Some(b) = body(&s, h) {
                highest = highest.max(b.position[1]);
            }
        }
    }
    assert!(highest > 5.0, "lifted above Y 5: {highest}");
    let dead = [a, b]
        .iter()
        .filter(|h| {
            s.world
                .npcs
                .get(h)
                .is_none_or(|n| n.character.combat.overkilled)
        })
        .count();
    assert_eq!(dead, 1, "one absorbed");
    let survivor = if s
        .world
        .npcs
        .get(&a)
        .is_some_and(|n| !n.character.combat.overkilled)
    {
        a
    } else {
        b
    };
    for _ in 0..2 {
        frame(&mut s, PlayerInput::default());
    }
    let n = &s.world.npcs[&survivor];
    assert_eq!(
        n.character.combat.health.hit_points, n.definition.max_hit_points,
        "healed to the maximum"
    );
    assert!(body(&s, survivor).unwrap().apply_gravity, "falls again");
}

/// Cursor NDC for a world point through the session camera.
fn cursor_at(s: &PlaySession, point: [f32; 3]) -> [f32; 2] {
    let vp = magicka_runtime::camera::multiply(
        &s.world.camera.view(),
        &s.world.camera.projection(16.0 / 9.0),
    );
    let v = [point[0], point[1], point[2], 1.0];
    let p: [f32; 4] = std::array::from_fn(|i| (0..4).map(|j| v[j] * vp[j * 4 + i]).sum());
    [p[0] / p[3], p[1] / p[3]]
}

/// Types `keys`, presses Space and lets the cast finish, holding the cursor
/// over `aim` (re-projected every frame) or with no cursor at all.
fn cast_aimed(s: &mut PlaySession, keys: &[usize], aim: Option<u16>) {
    let input = |s: &PlaySession| PlayerInput {
        cursor_ndc: aim.map(|h| {
            let body = s
                .world
                .players
                .iter()
                .flatten()
                .find(|p| p.handle == h)
                .map(|p| p.body)
                .or_else(|| s.world.npcs.get(&h).map(|n| n.body))
                .unwrap();
            cursor_at(s, s.world.physics.bodies[body].position)
        }),
        ..Default::default()
    };
    for &key in keys {
        let mut i = input(s);
        i.elements[key] = true;
        frame(s, i);
        for _ in 0..6 {
            let i = input(s);
            frame(s, i);
        }
    }
    let mut i = input(s);
    i.boost_down = true;
    frame(s, i);
    for _ in 0..240 {
        let i = input(s);
        frame(s, i);
        if s.avatar().motion.state == AvatarState::Idle {
            break;
        }
    }
}

/// Target-only Inflict Poison (Life, Water, Life, Earth, Shield) poisons the
/// character under the cursor (rewrite rule; natively a self-cast does
/// nothing). The 100 DPS, magnitude 3 payload is the native one.
#[test]
fn inflict_poison_targets_the_character_under_the_cursor() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::InflictPoisonDamage as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    cast_aimed(&mut s, &[LIFE, WATER, LIFE, EARTH, SHIELD], Some(goblin));
    use magicka_sim::status::Status;
    assert!(s.world.npcs[&goblin]
        .character
        .combat
        .statuses
        .has(Status::Poisoned));
    assert!(!s.avatar().character.combat.statuses.has(Status::Poisoned));
}

/// The caster is a candidate too: the cursor on the Wizard, or no cursor at
/// all, makes Inflict Bleeding (Arcane, Earth, Cold, Shield, Earth) bleed
/// the caster rather than a nearby goblin.
#[test]
fn inflict_bleeding_falls_back_to_the_caster() {
    let Some(root) = content() else {
        return;
    };
    use magicka_sim::status::Status;
    for aim_self in [true, false] {
        let mut s = session(&root);
        s.unlocked_magicks[0] |= 1 << M::InflictBleedDamage as u8;
        let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
        for _ in 0..10 {
            frame(&mut s, PlayerInput::default());
        }
        let me = s.avatar().handle;
        cast_aimed(
            &mut s,
            &[ARCANE, EARTH, COLD, SHIELD, EARTH],
            aim_self.then_some(me),
        );
        assert!(
            s.avatar().character.combat.statuses.has(Status::Bleeding),
            "aim_self {aim_self}"
        );
        assert!(!s.world.npcs[&goblin]
            .character
            .combat
            .statuses
            .has(Status::Bleeding));
    }
}

/// Knockdown Chance (Earth, Cold, Shield, Earth, Cold) delivers its 10
/// Status damage to the goblin under the cursor, which records the Wizard as
/// its (decaying) attacker and then fights back. Element None adds no status and no health damage, so only
/// the 1-in-4 Knockdown variant has a visible effect (native semantics).
#[test]
fn knockdown_chance_hits_the_cursor_target() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::KnockDownChance as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let goblin_hp = s.world.npcs[&goblin].character.combat.health.hit_points;
    cast_aimed(&mut s, &[EARTH, COLD, SHIELD, EARTH, COLD], Some(goblin));
    let me = s.avatar().handle;
    assert!(s.world.npcs[&goblin]
        .combat
        .attacked_by
        .iter()
        .any(|a| a.0 == me && a.1 > 0.0));
    assert_eq!(
        s.world.npcs[&goblin].character.combat.health.hit_points,
        goblin_hp
    );
}

/// The live NPC carrying `unique_id` (Polymorph keeps the UniqueID across
/// replacements) and its template name.
fn npc_by_unique(s: &PlaySession, unique_id: i32) -> Option<(u16, String)> {
    s.world
        .entities
        .iter()
        .find(|e| {
            e.unique_id == unique_id
                && !e.removed
                && s.world.npcs.get(&e.handle).is_some_and(|n| !n.dead())
        })
        .map(|e| (e.handle, e.template_name.clone().unwrap_or_default()))
}

/// Gives a test-spawned NPC (UniqueID 0) a distinctive UniqueID.
fn mark_unique(s: &mut PlaySession, handle: u16) -> i32 {
    const ID: i32 = 0x5150_0001;
    s.world
        .entities
        .iter_mut()
        .find(|e| e.handle == handle)
        .unwrap()
        .unique_id = ID;
    ID
}

fn polymorph_cast(s: &mut PlaySession) {
    s.unlocked_magicks[0] |= 1 << M::Polymorph as u8;
    cast_aimed(s, &[LIFE, WATER, EARTH, SHIELD, LIFE], None);
}

/// Polymorph (Life, Water, Earth, Shield, Life) replaces the goblin 3 m in
/// front with one of the four NPC forms. The replacement keeps the UniqueID,
/// the health fraction and statuses. After 20 s the goblin returns, still
/// carrying the damage taken while transformed.
#[test]
fn polymorph_turns_a_goblin_into_a_form_and_back() {
    let Some(root) = content() else {
        return;
    };
    use magicka_sim::status::{Status, StatusEffect};
    let mut s = session(&root);
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let unique = mark_unique(&mut s, goblin);
    {
        let c = &mut s.world.npcs.get_mut(&goblin).unwrap().character;
        let max = c.combat.health.max_hit_points;
        c.combat.health.hit_points = max * 0.5;
        let mut events = Vec::new();
        c.add_status_effect(
            StatusEffect::new(Status::Wet, 0.0, 1.0, false),
            None,
            &mut events,
        )
        .unwrap();
    }
    polymorph_cast(&mut s);
    let (form, name) = npc_by_unique(&s, unique).expect("replacement carries the UniqueID");
    assert_ne!(form, goblin, "a new actor");
    assert!(
        ["horse", "imp", "goblin_bomber", "troll_forest"]
            .iter()
            .any(|f| name.eq_ignore_ascii_case(f)),
        "{name}"
    );
    assert!(
        s.world.npcs.get(&goblin).is_none_or(|n| n.dead()),
        "old actor terminated"
    );
    let c = &s.world.npcs[&form].character.combat;
    let fraction = c.health.hit_points / c.health.max_hit_points;
    assert!((fraction - 0.5).abs() < 0.02, "{fraction}");
    assert!(c.statuses.has(Status::Wet), "statuses carried");
    assert_eq!(s.world.polymorphs.active().len(), 1);
    // Damage while transformed survives the reversal.
    {
        let h = &mut s.world.npcs.get_mut(&form).unwrap().character.combat.health;
        h.hit_points = h.max_hit_points * 0.25;
    }
    for _ in 0..(21 * 60) {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.polymorphs.active().is_empty());
    let (back, name) = npc_by_unique(&s, unique).expect("restored");
    assert!(name.eq_ignore_ascii_case("Goblin_pirate"), "{name}");
    assert_ne!(back, form);
    let h = &s.world.npcs[&back].character.combat.health;
    let fraction = h.hit_points / h.max_hit_points;
    assert!(
        fraction < 0.3,
        "current health, not the cast-time snapshot: {fraction}"
    );
}

/// Recasting at the transformed character cancels and restores at once; a
/// cast with nothing ahead fails with magick_fail.
#[test]
fn polymorph_recast_cancels_and_a_miss_fails() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Polymorph as u8;
    cast_aimed(&mut s, &[LIFE, WATER, EARTH, SHIELD, LIFE], None);
    assert!(s.world.polymorphs.active().is_empty(), "no target");
    assert!(
        s.world.log.iter().all(|l| !l.contains("polymorph")),
        "{:?}",
        s.world.log
    );
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let unique = mark_unique(&mut s, goblin);
    polymorph_cast(&mut s);
    let (form, _) = npc_by_unique(&s, unique).unwrap();
    assert_ne!(form, goblin);
    polymorph_cast(&mut s);
    assert!(s.world.polymorphs.active().is_empty(), "recast cancels");
    let (back, name) = npc_by_unique(&s, unique).unwrap();
    assert!(name.eq_ignore_ascii_case("Goblin_pirate"), "{name}");
    assert_ne!(back, form);
}

/// Nullify (SpellManager.ClearMagicks) runs Polymorph.OnRemove: the goblin
/// comes back at once.
#[test]
fn nullify_restores_a_polymorphed_goblin() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let unique = mark_unique(&mut s, goblin);
    polymorph_cast(&mut s);
    assert_eq!(s.world.polymorphs.active().len(), 1);
    s.world.nullify();
    assert!(s.world.polymorphs.active().is_empty());
    let (_, name) = npc_by_unique(&s, unique).unwrap();
    assert!(name.eq_ignore_ascii_case("Goblin_pirate"), "{name}");
}

/// Polymorph at a second Wizard 3 m ahead (PolymorphAvatar): the same
/// avatar becomes one of the four Polymorph forms. It keeps its handle and
/// health fraction, cannot conjure elements, and returns as a Wizard with
/// its own staff and weapon after 20 s.
#[test]
fn polymorph_turns_another_wizard_into_a_form_and_back() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let other = s.join_player(1, "Wizard", "start").unwrap();
    let ahead = {
        let b = s.body();
        let d = b.direction();
        [
            b.position[0] + d[0] * 3.0,
            b.position[1],
            b.position[2] + d[2] * 3.0,
        ]
    };
    let body = s.world.players[1].as_ref().unwrap().body;
    s.world.physics.bodies[body].position = ahead;
    let wizard_type = s.world.players[1].as_ref().unwrap().template_type;
    let names = |s: &PlaySession| -> Vec<String> {
        let mut v: Vec<_> = s.world.players[1]
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|(slot, i)| format!("{slot}:{}", i.definition.name))
            .collect();
        v.sort();
        v
    };
    let wizard_items = names(&s);
    {
        let h = &mut s.world.players[1].as_mut().unwrap().character.combat.health;
        h.hit_points = h.max_hit_points * 0.5;
    }
    polymorph_cast(&mut s);
    let p = s.world.players[1].as_ref().unwrap();
    assert_eq!(p.handle, other);
    assert!(p.polymorphed);
    let forms =
        magicka_sim::polymorph::AVATAR_FORMS.map(magicka_runtime::scene_loading::custom_hash);
    assert!(forms.contains(&p.template_type), "{}", p.template_type);
    let h = &p.character.combat.health;
    assert!((h.hit_points / h.max_hit_points - 0.5).abs() < 0.02);
    assert_eq!(s.world.polymorphs.active().len(), 1);
    // Element keys are ignored in the form.
    let mut controls = magicka_network::control::Controls::default();
    controls.elements[FIRE] = true;
    s.handle_remote_input(1, controls).unwrap();
    assert!(s.world.players[1]
        .as_ref()
        .unwrap()
        .conjure
        .queue
        .is_empty());
    for _ in 0..(21 * 60) {
        frame(&mut s, PlayerInput::default());
    }
    let p = s.world.players[1].as_ref().unwrap();
    assert!(s.world.polymorphs.active().is_empty());
    assert!(!p.polymorphed);
    assert_eq!(p.template_type, wizard_type);
    assert_eq!(names(&s), wizard_items);
}

/// Tornado (Earth, Steam, Water, Steam) spawns 4 m ahead on the navmesh:
/// the goblin standing there is hit by the Knockback Earth collection (the
/// Wizard is recorded as attacker), the dust effect loops, and the tornado
/// is gone with its effect after 15 s.
#[test]
fn tornado_hits_a_goblin_and_expires() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Tornado as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 4.0).unwrap();
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let hp = s.world.npcs[&goblin].character.combat.health.hit_points;
    let dust = magicka_runtime::scene_loading::custom_hash("magick_tornado");
    // Same-element presses need the 0.125 s ElementManager cooldown apart.
    press(&mut s, &[EARTH, WATER, FIRE, WATER]);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    cast_aimed(&mut s, &[WATER, FIRE], None);
    assert_eq!(
        s.world.tornadoes.len(),
        1,
        "{:?}",
        &s.world.log[s.world.log.len().saturating_sub(5)..]
    );
    assert_eq!(s.effects.active_effects_by_hash(dust), 1);
    let me = s.avatar().handle;
    let goblin_hit = s.world.npcs.get(&goblin).is_some_and(|n| {
        n.character.combat.health.hit_points < hp || n.combat.attacked_by.iter().any(|a| a.0 == me)
    });
    assert!(goblin_hit, "the goblin in the funnel was hit");
    for _ in 0..(16 * 60) {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.tornadoes.is_empty());
    assert_eq!(s.effects.active_effects_by_hash(dust), 0);
}

/// Vortex (Ice, Arcane, Ice, Shield, Ice) opens 10 m ahead: a goblin 1.5 m
/// from it is pulled in and absorbed (terminated, or gibbed when it has gibs) and feeds
/// the mass; the unfed vortex then shrinks away with its particle effect.
#[test]
fn vortex_absorbs_a_goblin_and_collapses() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Vortex as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 10.0).unwrap();
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let effect = magicka_runtime::scene_loading::custom_hash("magick_vortex");
    cast_aimed(
        &mut s,
        &[WATER, COLD, ARCANE, WATER, COLD, SHIELD, WATER, COLD],
        None,
    );
    assert_eq!(s.world.vortices.len(), 1);
    assert_eq!(s.effects.active_effects_by_hash(effect), 1);
    // The Wizard turns while casting; put the goblin 1.5 m from the centre,
    // inside the 2.5 m reach of a fresh vortex.
    let c = s.world.vortices[0].vortex.center;
    let body = s.world.npcs[&goblin].body;
    s.world.physics.bodies[body].position = [c[0] + 1.5, c[1], c[2]];
    let mut absorbed = false;
    for _ in 0..(3 * 60) {
        frame(&mut s, PlayerInput::default());
        absorbed |= s.world.npcs.get(&goblin).is_none_or(|n| n.dead());
        if absorbed {
            break;
        }
    }
    assert!(absorbed, "the goblin was pulled in and absorbed");
    let v = &s.world.vortices[0].vortex;
    assert!(v.target_mass > 0.25, "fed: {}", v.target_mass);
    for _ in 0..(60 * 60) {
        frame(&mut s, PlayerInput::default());
        if s.world.vortices.is_empty() {
            break;
        }
    }
    assert!(s.world.vortices.is_empty(), "collapsed");
    assert_eq!(s.effects.active_effects_by_hash(effect), 0);
}

/// Summon Phoenix (Life, Lightning, Fire) descends near the caster; when the descend clip
/// ends the fire blast hits a goblin within 6 m and a dead second Wizard is
/// revived.
#[test]
fn phoenix_blasts_and_revives_a_dead_wizard() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::SPhoenix as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 2.0).unwrap();
    let other = s.join_player(1, "Wizard", "start").unwrap();
    s.world.terminate_character(other);
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        s.world.players[1].as_ref().unwrap().dead(),
        "second wizard dead"
    );
    let hp = s.world.npcs[&goblin].character.combat.health.hit_points;
    s.world.scene_indoors = false;
    cast_aimed(&mut s, &[LIFE, LIGHTNING, FIRE], None);
    assert!(
        s.world.phoenix.registered || s.world.phoenix.finished,
        "{:?}",
        s.world.phoenix
    );
    let mut blasted = false;
    for _ in 0..(20 * 60) {
        frame(&mut s, PlayerInput::default());
        if s.world.phoenix.finished {
            blasted = true;
            break;
        }
    }
    assert!(blasted, "the descent ended");
    let goblin_hurt = s
        .world
        .npcs
        .get(&goblin)
        .is_none_or(|n| n.character.combat.health.hit_points < hp || n.dead());
    assert!(goblin_hurt, "the blast hit the goblin");
    let mut revived = false;
    for _ in 0..(10 * 60) {
        frame(&mut s, PlayerInput::default());
        if !s.world.players[1].as_ref().unwrap().dead() {
            revived = true;
            break;
        }
    }
    assert!(revived, "the dead wizard came back");
}

/// An attacking goblin 1.4 m from the vortex is dragged in and absorbed
/// despite running at the Wizard. (A vortex has already shrunk to about a
/// 2 m reach when the cast animation ends: native 10·mass.)
#[test]
fn vortex_pulls_an_attacking_goblin() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Vortex as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 25.0).unwrap();
    cast_aimed(
        &mut s,
        &[WATER, COLD, ARCANE, WATER, COLD, SHIELD, WATER, COLD],
        None,
    );
    let c = s.world.vortices[0].vortex.center;
    let body = s.world.npcs[&goblin].body;
    s.world.physics.bodies[body].position = [c[0] + 1.4, c[1], c[2]];
    // Agent order Attack, reacting to nothing: it runs at the Wizard.
    s.world.npcs.get_mut(&goblin).unwrap().set_order(
        magicka_runtime::npc::ORDER_ATTACK,
        0,
        0,
        0,
        None,
    );
    let mut closest = f32::MAX;
    let mut absorbed = false;
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
        match s.world.npcs.get(&goblin) {
            Some(n) if !n.dead() => {
                let p = s.world.physics.bodies[n.body].position;
                closest = closest.min(((p[0] - c[0]).powi(2) + (p[2] - c[2]).powi(2)).sqrt());
            }
            _ => {
                absorbed = true;
                break;
            }
        }
    }
    assert!(absorbed, "closest approach {closest}");
}

/// Enhanced Tornado (user request, not native): a goblin 6 m away is sucked
/// in, carried up the funnel and flung out from the top.
#[test]
fn tornado_sucks_lifts_and_flings() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 3.0).unwrap();
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let start = s.world.physics.bodies[s.world.npcs[&goblin].body].position;
    s.world.tornado_at([start[0] + 6.0, start[1], start[2]]);
    let (mut nearest, mut highest, mut flung_far) = (f32::MAX, f32::MIN, false);
    for _ in 0..(8 * 60) {
        frame(&mut s, PlayerInput::default());
        let (Some(t), Some(n)) = (s.world.tornadoes.first(), s.world.npcs.get(&goblin)) else {
            break;
        };
        let a = t.tornado.position;
        let p = s.world.physics.bodies[n.body].position;
        let d = ((p[0] - a[0]).powi(2) + (p[2] - a[2]).powi(2)).sqrt();
        nearest = nearest.min(d);
        highest = highest.max(p[1] - a[1]);
        flung_far |= highest > 3.0 && d > 6.0;
    }
    assert!(nearest < 2.5, "sucked in: nearest {nearest}");
    assert!(highest > 3.0, "lifted: highest {highest}");
    assert!(
        flung_far,
        "flung out (nearest {nearest}, highest {highest})"
    );
}

/// Earthquake's custom combo reaches the live singleton. Nullify stops its
/// retained particle/cue instances without losing the cached quake center.
#[test]
fn earthquake_keyboard_cast_and_nullify() {
    let Some(root) = content() else { return };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1u64 << M::Earthquake as u8;
    // Four Earth presses need more than the 0.125-second element cooldown.
    for key in [EARTH, EARTH, SHIELD, EARTH, EARTH] {
        press(&mut s, &[key]);
        for _ in 0..10 {
            frame(&mut s, PlayerInput::default());
        }
    }
    space(&mut s);
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
        if s.world.earthquake.registered {
            break;
        }
    }
    assert!(s.world.earthquake.registered, "{:?}", s.world.log);
    assert_eq!(s.world.earthquake.owner, Some(s.avatar().handle));
    assert_eq!(s.world.earthquake.timestamp, 0.0);
    s.world.receive_earthquake_center(s.body().position, 2.0);
    s.world.earthquake.hit_timer = 0.0;
    frame(&mut s, PlayerInput::default());
    let effect = magicka_runtime::scene_loading::custom_hash("magick_earthquake");
    assert!(s.effects.active_effects_by_hash(effect) > 0);
    let center = s.world.earthquake.quake_position;
    s.world.nullify();
    frame(&mut s, PlayerInput::default());
    assert!(!s.world.earthquake.registered);
    assert_eq!(s.effects.active_effects_by_hash(effect), 0);
    assert_eq!(s.world.earthquake.quake_position, center);
}

/// A forced pulse at a settled loose body reaches the separate Avian solver.
#[test]
fn earthquake_pulse_lifts_a_loose_prop() {
    let Some(root) = content() else { return };
    let mut s = session(&root);
    let p = s.body().position;
    let prop = prop_at(&mut s, [p[0] + 4.0, p[1], p[2]]);
    // Bathball supplies a supported collision body but is damageable in the
    // retail template. This authored fixture exercises the non-damageable
    // velocity branch rather than claiming Bathball follows that branch.
    let entity = s
        .world
        .entities
        .iter_mut()
        .find(|e| e.handle == prop)
        .unwrap();
    entity.kind = magicka_runtime::scene_world::EntityKind::Physics { damageable: false };
    entity.hit_points = 0.0;
    entity.max_hit_points = 0.0;
    assert!(matches!(
        s.world
            .entities
            .iter()
            .find(|e| e.handle == prop)
            .unwrap()
            .kind,
        magicka_runtime::scene_world::EntityKind::Physics { damageable: false }
    ));
    let before = s.pickup_physics.as_ref().unwrap().state(prop).unwrap();
    assert!(s.world.earthquake_from(s.avatar().handle));
    s.world.receive_earthquake_center(before.position, 2.0);
    s.world.earthquake.hit_timer = 0.0;
    frame(&mut s, PlayerInput::default());
    let after = s.pickup_physics.as_ref().unwrap().state(prop).unwrap();
    assert!(s.world.earthquake.hits.contains(prop));
    assert!(
        after.linear_velocity[1] > before.linear_velocity[1] + 3.0,
        "native +5 velocity must reach the prop before gravity: {before:?} -> {after:?}"
    );
    assert!(
        s.world.earthquake_velocities.is_empty(),
        "requests consumed this step"
    );
}

/// Death is a non-character IDamageable with mass500. Earthquake consumes
/// its hit/RNG path but its pure Earth damage cannot trigger Death's Life hit.
#[test]
fn earthquake_reaches_asset_backed_death_without_character_impulse() {
    use magicka_sim::earthquake::EarthquakeHost;
    let Some(root) = content() else { return };
    let mut s = session(&root);
    let owner = s.avatar().handle;
    assert!(s.world.summon_death_from(owner));
    for _ in 0..60 {
        if s.world.death.actor.is_some() {
            break;
        }
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.death.actor.is_some(), "{:?}", s.world.log);
    assert!(
        !s.world.summon_death_from(owner),
        "active singleton refuses recast"
    );
    let handle = s.world.death.handle.unwrap();
    let target = EarthquakeHost::target(&s.world, handle).unwrap();
    assert_eq!(target.mass, 500.0);
    assert!(target.damageable && target.character.is_none());
    assert!(EarthquakeHost::targets(&mut s.world, target.position, 5.0).contains(&handle));
    // Isolate the damage routing from Death's autonomous phase transitions.
    s.world.death.actor.as_mut().unwrap().state.ethereal = false;
    assert!(s.world.earthquake_from(owner));
    s.world.receive_earthquake_center(target.position, 2.0);
    s.world.earthquake.hit_timer = 0.0;
    frame(&mut s, PlayerInput::default());
    assert!(s.world.earthquake.hits.contains(handle));
    assert!(!s.world.death.actor.as_ref().unwrap().state.is_hit);
    assert!(s
        .world
        .receive_death_damage(magicka_sim::spells::Elements::LIFE, true));
    s.world.nullify();
    assert!(s.world.death.actor.as_ref().unwrap().state.dead);
    assert_eq!(s.world.time_modifier, 1.0);
    assert!(EarthquakeHost::target(&s.world, handle).is_none());
    assert!(
        s.world.death.frame.is_none(),
        "explicit removal clears presentation"
    );
    // Reinitialization uses the cached native actor/handle. Its final body
    // and scythe draw is queued before the missing/dead-target early return.
    assert!(s.world.summon_death_from(owner));
    for _ in 0..60 {
        if s.world.death.actor.as_ref().is_some_and(|a| !a.state.dead) {
            break;
        }
        frame(&mut s, PlayerInput::default());
    }
    let actor = s.world.death.actor.as_mut().unwrap();
    assert!(!actor.state.dead);
    actor.state.target = owner;
    s.world.players[0].as_mut().unwrap().terminate();
    frame(&mut s, PlayerInput::default());
    assert!(s.world.death.actor.as_ref().unwrap().state.dead);
    assert!(
        s.world.death.frame.is_some(),
        "terminal draw survives deinitialization"
    );
    assert_eq!(s.world.time_modifier, 1.0);
    frame(&mut s, PlayerInput::default());
    assert!(
        s.world.death.frame.is_none(),
        "terminal draw expires on next entity update"
    );
}

#[test]
fn elemental_cast_egg_hatches_selected_template_and_links_summoner() {
    use magicka_sim::{damage::Damage, spells::Elements as E};
    let Some(root) = content() else { return };
    let mut s = session(&root);
    let owner = s.avatar().handle;
    let handle = s.world.summon_elemental_from(owner).unwrap().unwrap();
    let cache = s.world.elemental_eggs.as_ref().unwrap();
    assert_eq!(cache.slots.len(), 128);
    assert_eq!(cache.slots[0].handle, handle);
    assert_eq!(cache.slots[0].actor.state.summoner, Some(owner));
    assert!(cache.slots[0].actor.registered);
    assert!(s.world.damage_elemental_egg(
        handle,
        &[Damage {
            element: E::FIRE.0,
            amount: 50.0,
            magnitude: 1.0,
            ..Default::default()
        }]
    ));
    frame(&mut s, PlayerInput::default());
    let cache = s.world.elemental_eggs.as_ref().unwrap();
    assert!(!cache.slots[0].actor.registered);
    assert!(cache.slots[0].frame.is_some(), "final egg draw retained");
    let child = s
        .world
        .npcs
        .values()
        .find(|npc| {
            npc.summon.is_some_and(|summon| summon.0 == owner)
                && npc
                    .character
                    .definition
                    .id
                    .eq_ignore_ascii_case("Elemental_Fire")
        })
        .expect("selected elemental spawned");
    assert_eq!(
        child.summon.unwrap().1,
        magicka_sim::summons::SummonKind::Normal
    );
    assert!(s.world.summons[&owner]
        .summons()
        .any(|(handle, _)| handle == child.handle));
    frame(&mut s, PlayerInput::default());
    assert!(s.world.elemental_eggs.as_ref().unwrap().slots[0]
        .frame
        .is_none());
    s.world.role = magicka_sim::damage_delivery::NetworkRole::Client;
    assert_eq!(s.world.summon_elemental_from(owner).unwrap(), None);
    assert!(
        !s.world.elemental_eggs.as_ref().unwrap().slots[1]
            .actor
            .registered
    );
}

#[test]
fn missile_blast_selects_and_hatches_an_elemental_egg() {
    use magicka_runtime::missiles::LiveMissile;
    use magicka_sim::{
        damage::Damage,
        missile_events::{condition, EventCondition, MissileConditions, MissileEvent},
        spells::Elements as E,
    };
    let Some(root) = content() else { return };
    let mut s = session(&root);
    let owner = s.avatar().handle;
    let handle = s.world.summon_elemental_from(owner).unwrap().unwrap();
    // Isolate event delivery above the scene; this is an authored recipe,
    // not a claim about the timing or damage values of a retail projectile.
    let actor = &mut s.world.elemental_eggs.as_mut().unwrap().slots[0].actor;
    actor.initialize([0., 100., 0.], [0., 0., 1.], None);
    actor.state.summoner = Some(owner);
    let position = actor.position;
    let mut damages = [Damage::default(); 5];
    damages[0] = Damage {
        element: E::LIGHTNING.0,
        amount: 50.,
        magnitude: 1.,
        ..Default::default()
    };
    let mut conditions = MissileConditions::default();
    conditions.slots[0].condition = EventCondition {
        flags: condition::TIMER,
        time: 0.01,
        ..Default::default()
    };
    conditions.slots[0].events = vec![
        MissileEvent::Blast {
            radius: 5.,
            damages,
        },
        MissileEvent::Remove { bounces: 0 },
    ];
    s.world.missiles.push(LiveMissile::spawn_custom(
        900, owner, position, [0.; 3], 0.05, false, conditions,
    ));
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        !s.world.elemental_eggs.as_ref().unwrap().slots[0]
            .actor
            .registered
    );
    assert_eq!(
        s.world.elemental_eggs.as_ref().unwrap().slots[0].handle,
        handle
    );
    assert!(
        s.world.npcs.values().any(|npc| {
            npc.summon.is_some_and(|summon| summon.0 == owner)
                && npc
                    .character
                    .definition
                    .id
                    .eq_ignore_ascii_case("Elemental_Lightning")
        }),
        "normal missile blast selected and hatched Lightning"
    );
}

/// Both custom slime magicks use the native wizard fallback: one small poison
/// child, at full health, sharing faction without the capped summon linkage.
#[test]
fn slime_magicks_spawn_faction_children_that_survive_nullify() {
    let Some(root) = content() else { return };
    let mut s = session(&root);
    for magick in [M::SpawnSlime, M::SpawnSlimeOverkill] {
        let existing: Vec<_> = s.world.npcs.keys().copied().collect();
        s.world.cast_magick(0, magick);
        let new: Vec<_> = s
            .world
            .npcs
            .keys()
            .copied()
            .filter(|h| !existing.contains(h))
            .collect();
        assert_eq!(new.len(), 1, "{:?}", s.world.log);
        let h = new[0];
        let npc = &s.world.npcs[&h];
        assert_eq!(
            npc.character.definition.id,
            "dungeons_slimecube_poison_small"
        );
        assert_eq!(
            npc.character.combat.health.hit_points,
            npc.character.combat.health.max_hit_points
        );
        assert!(npc.summon.is_none());
        assert_eq!(npc.ai.order, magicka_runtime::npc::ORDER_ATTACK);
        let me = s
            .world
            .entities
            .iter()
            .find(|e| e.handle == s.avatar().handle)
            .unwrap();
        let child = s.world.entities.iter().find(|e| e.handle == h).unwrap();
        assert_eq!(s.world.faction(child), s.world.faction(me));
        s.world.nullify();
        assert!(!s.world.npcs[&h].character.combat.dead);
    }
    assert!(!s.world.summons.contains_key(&s.avatar().handle));
}

#[test]
fn ethereal_clone_player_extension_spawns_native_child_without_summon_link() {
    use magicka_runtime::scene_world::clone_live::CloneCast;
    let Some(root) = content() else { return };
    let mut s = session(&root);
    let owner = s.avatar().handle;
    // The native owner overload still refuses an ordinary wizard.
    assert_eq!(
        s.world
            .ethereal_clone_from(owner, CloneCast::default())
            .unwrap(),
        None
    );
    let before: Vec<_> = s.world.npcs.keys().copied().collect();
    s.world.cast_magick(0, M::EtherealClone);
    let new: Vec<_> = s
        .world
        .npcs
        .keys()
        .copied()
        .filter(|h| !before.contains(h))
        .collect();
    assert_eq!(new.len(), 1, "{:?}", s.world.log);
    let h = new[0];
    let npc = &s.world.npcs[&h];
    assert_eq!(
        npc.character.definition.id,
        magicka_sim::ethereal_clone::TEMPLATE
    );
    assert!(npc.summon.is_none());
    assert_eq!(
        npc.character.combat.health.hit_points,
        npc.character.combat.health.max_hit_points
    );
    assert_eq!(npc.ai.order, magicka_runtime::npc::ORDER_ATTACK);
    let me = s.world.entities.iter().find(|e| e.handle == owner).unwrap();
    let child = s.world.entities.iter().find(|e| e.handle == h).unwrap();
    assert_eq!(s.world.faction(child), s.world.faction(me));
    // The player extension does not turn the native NPC entry point into a
    // general clone producer for an arbitrary non-adventurer NPC.
    assert_eq!(
        s.world
            .ethereal_clone_from(h, CloneCast::default())
            .unwrap(),
        None
    );
    s.world.nullify();
    assert!(!s.world.npcs[&h].character.combat.dead);
}

/// Cross survives Nullify; summoned death clears the caster's self auras
/// without resetting the native bubble flag. Authored, validation pending.
#[test]
fn summon_cross_preserves_bubble_on_nullify_and_clears_it_on_summoned_death() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let owner = s.avatar().handle;
    let h = s.world.summon_cross(owner).expect("cross template");
    let cross = &s.world.npcs[&h];
    assert_eq!(cross.character.definition.id, "cross");
    assert_eq!(cross.summon.unwrap().0, owner);
    assert_eq!(cross.weights.alert_radius, 0.0);
    let aura = s.avatar().character.auras.last().unwrap();
    assert!(aura.self_only && aura.self_casted);
    assert_eq!(aura.resistance.against, magicka_sim::spells::Elements::ALL);
    assert_eq!(aura.resistance.multiplier, 0.0);
    assert_eq!(aura.visual.color, [2.0, 1.5, 1.0]);
    assert_eq!(aura.visual.decal_cell, 31);
    let bubble = magicka_runtime::scene_loading::custom_hash("paladin_bubble");
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(s.effects.active_effects_by_hash(bubble), 1);
    s.world.nullify();
    assert!(!s.world.npcs[&h].character.combat.dead);
    assert!(s.avatar().character.auras.last().unwrap().ttl > 0.0);
    s.world.overkill(h);
    assert!(s.avatar().character.bubble);
    assert!(s
        .avatar()
        .character
        .auras
        .iter()
        .filter(|a| a.self_casted)
        .all(|a| a.ttl == 0.0));
    // The aura particle instance stops with the aura.
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(s.effects.active_effects_by_hash(bubble), 0);
}

/// Ordinary NPC templates are not native VladCharacter instances. Only
/// explicitly supplied boss participants enter the native ring query.
#[test]
fn vlad_zap_native_empty_ring_and_player_cursor_extension_are_distinct() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let owner = s.avatar().handle;
    let target = s.spawn_npc("Goblin_pirate", "idle", 4.0).unwrap();
    s.world.barrier_visuals.clear();
    assert!(s.world.vlad_zap_native(owner, &[]));
    assert!(s.world.barrier_visuals.is_empty());
    let hp = s.world.npcs[&target].character.combat.health.hit_points;
    assert!(s.world.vlad_zap_player(owner, target));
    assert!(!s.world.barrier_visuals.is_empty());
    assert!(s.world.npcs[&target].character.combat.health.hit_points < hp);
}

fn prop_at(s: &mut PlaySession, position: [f32; 3]) -> u16 {
    let mut t = magicka_content::skinned::IDENTITY;
    t[12..15].copy_from_slice(&position);
    let h = s.spawn_physics_prop("bathball", t).unwrap();
    for _ in 0..30 {
        frame(s, PlayerInput::default());
    }
    h
}

/// A loose prop near a vortex gains velocity += pull each frame (native)
/// and is swallowed at the core, feeding the mass.
#[test]
fn vortex_swallows_a_prop() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let p = s.body().position;
    let prop = prop_at(&mut s, [p[0] + 4.0, p[1], p[2]]);
    let at = s
        .pickup_physics
        .as_ref()
        .unwrap()
        .state(prop)
        .unwrap()
        .position;
    // A fresh vortex centred 1.5 m beside the resting prop.
    let body = s.avatar().body;
    s.world.physics.bodies[body].position = [at[0] - 11.5, at[1] + 0.3, at[2]];
    s.world.physics.bodies[body].set_desired_direction([1.0, 0.0, 0.0]);
    let mut gone = false;
    let mut cast = false;
    for _ in 0..(6 * 60) {
        if !cast && !s.world.vortices.is_empty() {
            cast = true;
            let v = &mut s.world.vortices[0].vortex;
            v.center = [at[0] - 1.5, at[1], at[2]];
        }
        if s.world.vortices.is_empty() && !cast {
            s.world.vortex_from(s.avatar().handle);
            continue;
        }
        frame(&mut s, PlayerInput::default());
        if s.world
            .entities
            .iter()
            .any(|e| e.handle == prop && e.removed)
        {
            gone = true;
            break;
        }
    }
    assert!(gone, "the prop was pulled in and swallowed");
}

/// Enhanced Tornado on props: a bathball is carried to the top and flung,
/// rather than only moving upward briefly before escaping the funnel.
#[test]
fn tornado_tosses_a_prop() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let p = s.body().position;
    let prop = prop_at(&mut s, [p[0] + 4.0, p[1], p[2]]);
    let at = s
        .pickup_physics
        .as_ref()
        .unwrap()
        .state(prop)
        .unwrap()
        .position;
    s.world.tornado_at([at[0] + 2.0, at[1], at[2]]);
    let mut highest = f32::MIN;
    let mut flung = false;
    for _ in 0..(6 * 60) {
        frame(&mut s, PlayerInput::default());
        let (Some(t), Some(st)) = (
            s.world.tornadoes.first(),
            s.pickup_physics.as_ref().and_then(|ph| ph.state(prop)),
        ) else {
            break;
        };
        highest = highest.max(st.position[1] - t.tornado.position[1]);
        flung |= t.tornado.flung.contains(prop);
    }
    assert!(
        highest >= magicka_sim::tornado::LENGTH * 0.85,
        "carried to the fling height: {highest}"
    );
    assert!(
        flung,
        "the prop reached the top and received the fling impulse"
    );
}

/// Wave (Earth, Steam, Earth, Steam, Earth) from the keyboard raises six
/// rock pillars ahead of the Wizard; a goblin standing in the line is lifted
/// (the pillar damage has amount 0, so its health stays unchanged).
#[test]
fn wave_combo_raises_pillars_into_a_goblin() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Wave as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 6.0).unwrap();
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    press(&mut s, &[EARTH, WATER, FIRE, EARTH, WATER, FIRE, EARTH]);
    frame(
        &mut s,
        PlayerInput {
            boost_down: true,
            ..Default::default()
        },
    );
    let mut lifted = false;
    for _ in 0..240 {
        frame(&mut s, PlayerInput::default());
        lifted |= s
            .world
            .npcs
            .get(&goblin)
            .is_some_and(|n| s.world.physics.bodies[n.body].velocity[1] > 4.0);
    }
    let pillars: Vec<_> = s
        .world
        .log
        .iter()
        .filter(|l| l.starts_with("wave pillar"))
        .collect();
    assert_eq!(pillars.len(), 6, "{pillars:?}");
    assert!(lifted, "the goblin in the line was lifted");
}

/// Wave.Execute directly: six pillars 1.5 m apart, 0.1 s after each other,
/// one lift per cast, immune to Nullify's Kill, all gone after their
/// 0.82 s clip timer.
#[test]
fn wave_chains_six_pillars_and_lifts_once() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 20.0).unwrap();
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    let me = s.avatar().handle;
    let body = s.world.players[0].as_ref().unwrap().body;
    let (p, d) = (
        s.world.physics.bodies[body].position,
        s.world.physics.bodies[body].direction(),
    );
    // Halfway along the line (pillars 3 m beyond the probe hit, every 1.5 m).
    let gb = s.world.npcs[&goblin].body;
    s.world.physics.bodies[gb].position = [p[0] + d[0] * 6.5, p[1], p[2] + d[2] * 6.5];
    for _ in 0..5 {
        frame(&mut s, PlayerInput::default());
    }
    let before = s.world.log.len();
    assert!(s.world.wave_from(me));
    assert_eq!(s.world.waves.len(), 1);
    let (mut max_live, mut lifted, mut nullified) = (0, false, false);
    for i in 0..(3 * 60) {
        frame(&mut s, PlayerInput::default());
        let live = s
            .world
            .barriers
            .iter()
            .filter(|(_, b)| b.wave.is_some())
            .count();
        max_live = max_live.max(live);
        lifted |= s
            .world
            .npcs
            .get(&goblin)
            .is_some_and(|n| s.world.physics.bodies[n.body].velocity[1] > 4.0);
        if let Some(w) = s.world.waves.first() {
            assert!(w.effect.hit_list.iter().filter(|h| **h == goblin).count() <= 1);
        }
        if i == 20 && live > 0 {
            s.world.nullify();
            nullified = true;
            let after = s
                .world
                .barriers
                .iter()
                .filter(|(_, b)| b.wave.is_some())
                .count();
            assert_eq!(after, live, "WaveEntity.Kill is empty");
        }
    }
    assert!(nullified);
    let pillars = s.world.log[before..]
        .iter()
        .filter(|l| l.starts_with("wave pillar"))
        .count();
    assert_eq!(pillars, 6, "{:?}", &s.world.log[before..]);
    assert!(max_live >= 2, "pillars overlap in time: {max_live}");
    assert!(lifted, "the grounded goblin was lifted");
    assert!(s.world.waves.is_empty(), "effect and parent entry ended");
    assert_eq!(
        s.world
            .barriers
            .iter()
            .filter(|(_, b)| b.wave.is_some())
            .count(),
        0,
        "pillars removed when their timers ran out"
    );
}

/// Portal (Steam, Lightning, Shield): two casts place the orange then the
/// blue portal; once connected a goblin stepping into one comes out of the
/// other (0.8 × its offset) without bouncing back; Nullify kills both.
#[test]
fn portal_pair_teleports_a_goblin_and_nullify_closes_it() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    s.unlocked_magicks[0] |= 1 << M::Portal as u8;
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 25.0).unwrap();
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    cast_aimed(&mut s, &[WATER, FIRE, LIGHTNING, SHIELD], None);
    let first = s.world.portals.portal.a;
    assert!(
        !first.dead,
        "{:?}",
        &s.world.log[s.world.log.len().saturating_sub(5)..]
    );
    // Walk the Wizard back 8 m and cast directly for the second portal.
    let me = s.avatar().handle;
    let body = s.world.players[0].as_ref().unwrap().body;
    let d = s.world.physics.bodies[body].direction();
    let p = s.world.physics.bodies[body].position;
    s.world.physics.bodies[body].position = [p[0] - d[0] * 8.0, p[1], p[2] - d[2] * 8.0];
    for _ in 0..5 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.portal_from(me));
    let (a, b) = (s.world.portals.portal.a, s.world.portals.portal.b);
    assert_ne!(a.kind, b.kind);
    assert!(!a.dead && !b.dead);
    frame(&mut s, PlayerInput::default());
    assert!(s.world.portals.portal.connected);
    let blue = magicka_runtime::scene_loading::custom_hash("magick_portal_blue");
    let orange = magicka_runtime::scene_loading::custom_hash("magick_portal_orange");
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(s.effects.active_effects_by_hash(blue), 1);
    assert_eq!(s.effects.active_effects_by_hash(orange), 1);
    // Step the goblin into portal B.
    let gb = s.world.npcs[&goblin].body;
    let entry = b.position;
    s.world.physics.bodies[gb].position = [entry[0] + 0.3, entry[1], entry[2]];
    let mut arrived = false;
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
        let g = s.world.physics.bodies[gb].position;
        let near = |q: [f32; 3]| ((g[0] - q[0]).powi(2) + (g[2] - q[2]).powi(2)).sqrt();
        if near(a.position) < 1.5 {
            arrived = true;
        }
        if arrived {
            assert!(near(b.position) > 3.0, "no bounce back");
        }
    }
    assert!(arrived, "the goblin came out of the other portal");
    s.world.nullify();
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.portals.portal.a.dead && s.world.portals.portal.b.dead);
    assert!(!s.world.portals.portal.connected);
    assert_eq!(s.effects.active_effects_by_hash(blue), 0);
    assert_eq!(s.effects.active_effects_by_hash(orange), 0);
}

/// NullifyArea kills TornadoEntity (one more second) and VortexEntity
/// (target mass 0).
#[test]
fn nullify_winds_down_tornado_and_vortex() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let me = s.avatar().handle;
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.tornado_from(me));
    assert!(s.world.vortex_from(me));
    frame(&mut s, PlayerInput::default());
    assert!(s.world.tornadoes[0].tornado.ttl > 5.0);
    s.world.nullify();
    assert_eq!(s.world.tornadoes[0].tornado.ttl, 1.0);
    assert_eq!(s.world.vortices[0].vortex.target_mass, 0.0);
    for _ in 0..(3 * 60) {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.tornadoes.is_empty());
    assert!(s.world.vortices.is_empty());
}

/// Character.Stun: the character enters StunState (no movement or
/// rotation), shows `stunned` while the timer runs and returns to Idle.
#[test]
fn stunned_goblin_stands_still_then_recovers() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 6.0).unwrap();
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let stars = magicka_runtime::scene_loading::custom_hash("stunned");
    s.world
        .npcs
        .get_mut(&goblin)
        .unwrap()
        .character
        .combat
        .stun_timer = 1.0;
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    let n = &s.world.npcs[&goblin];
    assert_eq!(n.motion.state, magicka_runtime::avatar::AvatarState::Stun);
    let body = &s.world.physics.bodies[n.body];
    assert!(!body.allow_move && !body.allow_rotate);
    assert_eq!(s.effects.active_effects_by_hash(stars), 1);
    for _ in 0..70 {
        frame(&mut s, PlayerInput::default());
    }
    let n = &s.world.npcs[&goblin];
    assert_ne!(n.motion.state, magicka_runtime::avatar::AvatarState::Stun);
    assert_eq!(s.effects.active_effects_by_hash(stars), 0);
}

/// User report: rocks ignored Vortex. Missiles are IDamageable bodies of
/// mass 4: the vortex pulls them (velocity) and overkills them at the core,
/// feeding its target mass.
#[test]
fn vortex_pulls_and_swallows_a_rock() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let me = s.avatar().handle;
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.vortex_from(me));
    let c = s.world.vortices[0].vortex.center;
    // A slow rock hovering 1.5 m from the centre, inside the 2.5 m reach.
    let rock = magicka_runtime::missiles::LiveMissile::spawn(
        900,
        me,
        false,
        &magicka_sim::projectile_launch::ProjectileLaunch {
            position: [c[0] + 1.5, c[1], c[2]],
            velocity: [0.0; 3],
            spell: magicka_sim::spells::Spell::default_for_list(&[
                magicka_sim::spells::Elements::EARTH,
            ]),
            splash: 0.0,
            total_projectiles: 1,
        },
    )
    .unwrap();
    s.world.missiles.push(rock);
    let before = s.world.vortices[0].vortex.target_mass;
    let mut gone = false;
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
        if !s.world.missiles.iter().any(|m| m.body.id == 900) {
            gone = true;
            break;
        }
    }
    assert!(gone, "the rock was swallowed");
    assert!(
        s.world.vortices[0].vortex.target_mass > before,
        "the rock fed the vortex"
    );
}
