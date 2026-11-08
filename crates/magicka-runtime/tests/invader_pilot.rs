//! Invader pilots drive a possessed NPC's own authored abilities directly
//! (rewrite extension). User request 2026-11-03: "controlling attacks and
//! special actions of npcs as if they were designed for it".
use magicka_network::control::Controls;
use magicka_runtime::npc_combat::PilotAction;
use magicka_runtime::play_session::PlaySession;
use std::path::Path;

fn lab() -> Option<PlaySession> {
    let content = magicka_content::retail_content_dir();
    if !content.is_dir() {
        return None;
    }
    let level = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap();
    let mut s = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
    s.disable_scripts();
    s.step(1. / 60.);
    Some(s)
}

fn possess(s: &mut PlaySession, template: &str) -> u16 {
    let h = s.spawn_npc(template, "idle", 6.0).unwrap();
    for _ in 0..5 {
        s.step(1. / 60.);
    }
    assert!(s.world.possess(h));
    h
}

/// A point three units ahead of the NPC, away from the players.
fn ahead(s: &PlaySession, h: u16) -> [f32; 3] {
    let body = &s.world.physics.bodies[s.world.npcs[&h].body];
    let d = body.direction();
    let p = body.position;
    [p[0] + d[0] * 3.0, p[1], p[2] + d[2] * 3.0]
}

fn pilot(s: &mut PlaySession, h: u16, controls: Controls) {
    s.pilot_npc(h, controls);
    s.step(1. / 60.);
}

#[test]
fn ability_families_follow_the_npcs_authored_abilities() {
    let Some(mut s) = lab() else { return };
    let goblin = s.spawn_npc("Goblin_pirate", "idle", 6.0).unwrap();
    let c = &s.world.npcs[&goblin].combat;
    assert!(!c.pilot_abilities(PilotAction::Primary).is_empty());
    assert!(c.pilot_abilities(PilotAction::Boost).is_empty());
    let cultist = s.spawn_npc("Cultist", "idle", 6.0).unwrap();
    let c = &s.world.npcs[&cultist].combat;
    assert!(
        !c.pilot_abilities(PilotAction::Secondary).is_empty(),
        "ranged"
    );
    // An NPC without melee attacks with its ranged/spell family.
    let druid = s.spawn_npc("Druid", "idle", 6.0).unwrap();
    let c = &s.world.npcs[&druid].combat;
    assert_eq!(
        c.pilot_abilities(PilotAction::Primary),
        c.pilot_abilities(PilotAction::Secondary)
    );
}

/// Primary runs the NPC's melee ability at once, also while walking, and
/// the ability's own cooldown and busy state gate repeats.
#[test]
fn primary_swings_the_npcs_melee_while_moving() {
    let Some(mut s) = lab() else { return };
    let h = possess(&mut s, "Goblin_pirate");
    let aim = ahead(&s, h);
    let mut swung = false;
    for _ in 0..20 {
        pilot(
            &mut s,
            h,
            Controls {
                pad_movement: Some([0.7, 0.0]),
                pad_attack: true,
                aim: Some(aim),
                ..Default::default()
            },
        );
        let npc = &s.world.npcs[&h];
        if npc.combat.busy_ability.is_some() || npc.motion.attacking {
            swung = true;
            break;
        }
    }
    assert!(swung, "the possessed goblin did not attack");
    let npc = &s.world.npcs[&h];
    let used = npc.combat.busy_ability.or(npc.combat.next_ability).unwrap();
    assert!(
        npc.combat
            .pilot_abilities(PilotAction::Primary)
            .contains(&used),
        "a melee ability ran"
    );
}

/// Boost dashes on the press edge; holding it does not chain dashes.
#[test]
fn boost_dashes_once_per_press() {
    let Some(mut s) = lab() else { return };
    let h = possess(&mut s, "Beastman_raider");
    let aim = ahead(&s, h);
    let boost = Controls {
        boost_down: true,
        aim: Some(aim),
        ..Default::default()
    };
    let mut dashes = 0;
    let mut was = false;
    for _ in 0..240 {
        pilot(&mut s, h, boost);
        let dashing = s.world.npcs[&h].motion.dashing;
        if dashing && !was {
            dashes += 1;
        }
        was = dashing;
    }
    assert_eq!(dashes, 1, "one dash per press");
    pilot(&mut s, h, Controls::default());
    for _ in 0..240 {
        pilot(&mut s, h, boost);
        let dashing = s.world.npcs[&h].motion.dashing;
        if dashing && !was {
            dashes += 1;
        }
        was = dashing;
    }
    assert_eq!(dashes, 2, "a new press dashes again");
}

/// Block runs the NPC's own Block ability as designed: Orc_warrior's
/// blocks for the native one-second timer, then its 6 s cooldown applies
/// before a still-held button blocks again.
#[test]
fn block_uses_the_npcs_block_ability_and_cooldown() {
    let Some(mut s) = lab() else { return };
    let h = possess(&mut s, "Orc_warrior");
    let aim = ahead(&s, h);
    let block = Controls {
        pad_block: true,
        aim: Some(aim),
        ..Default::default()
    };
    let mut blocking = Vec::new();
    for _ in 0..(60 * 9) {
        pilot(&mut s, h, block);
        blocking.push(s.world.npcs[&h].motion.blocking);
    }
    let starts: Vec<usize> = (0..blocking.len())
        .filter(|&i| blocking[i] && (i == 0 || !blocking[i - 1]))
        .collect();
    assert!(
        starts.len() >= 2,
        "blocked again after the cooldown: {starts:?}"
    );
    let gap = (starts[1] - starts[0]) as f32 / 60.0;
    assert!(gap >= 6.0, "the 6 s Block cooldown applies ({gap} s)");
    let first_len = blocking[starts[0]..].iter().take_while(|b| **b).count() as f32 / 60.0;
    assert!(first_len <= 1.2, "native one-second block ({first_len} s)");
}

/// Secondary chants a CastSpell ability's elements while held, then casts.
#[test]
fn secondary_chants_then_casts_the_npcs_spell() {
    let Some(mut s) = lab() else { return };
    let h = possess(&mut s, "Goblin_shaman");
    let aim = ahead(&s, h);
    let mut chanted = false;
    let mut cast = false;
    for _ in 0..600 {
        pilot(
            &mut s,
            h,
            Controls {
                shift_down: true,
                aim: Some(aim),
                ..Default::default()
            },
        );
        let npc = &s.world.npcs[&h];
        chanted |= !npc.combat.spellcasting.queue.is_empty();
        cast |= chanted && (npc.motion.attacking || npc.current_spell.is_some());
        if cast {
            break;
        }
    }
    assert!(chanted, "elements were conjured");
    assert!(cast, "the spell was cast");
}

/// Self cast runs the NPC's Target=Self abilities on itself.
#[test]
fn self_cast_uses_the_npcs_self_targeted_spells() {
    let Some(mut s) = lab() else { return };
    let h = possess(&mut s, "Goblin_shaman");
    let selfish = s.world.npcs[&h]
        .combat
        .pilot_abilities(PilotAction::SelfCast);
    assert!(!selfish.is_empty());
    assert!(s.world.npcs[&h]
        .combat
        .pilot_abilities(PilotAction::Secondary)
        .iter()
        .all(|i| !selfish.contains(i)));
    let aim = ahead(&s, h);
    let mut used = None;
    for _ in 0..600 {
        pilot(
            &mut s,
            h,
            Controls {
                cast_self_down: true,
                aim: Some(aim),
                ..Default::default()
            },
        );
        let c = &s.world.npcs[&h].combat;
        if let Some(n) = c.next_ability {
            used = Some(n);
        }
        if c.busy_ability.is_some() {
            break;
        }
    }
    let used = used.expect("a self-targeted ability was chosen");
    assert!(selfish.contains(&used), "ability {used} is self-targeted");
    assert!(s.world.npcs[&h].combat.busy_ability.is_some(), "and it ran");
}

/// User report 2026-11-04: in Havindr's last cutscene the lightning's root does not
/// follow the warlock. Machine.cs takes it from his animated hands.
#[test]
fn the_warlocks_final_lightning_starts_at_his_hands() {
    let Some(mut s) = lab() else { return };
    let h = s.spawn_npc("boss_machine_warlock", "idle", 6.0).unwrap();
    let mut hand_tracks = Vec::new();
    for _ in 0..90 {
        s.step(1. / 60.);
        s.world.magick_bolts.clear();
        s.world.machine_emperor_lightning(h);
        let npc = &s.world.npcs[&h];
        let body = s.world.physics.bodies[npc.body].position;
        let [right, left] = [&s.world.magick_bolts[0], &s.world.magick_bolts[1]];
        let at = |m: &[f32; 16]| [m[12], m[13], m[14]];
        assert_eq!(right.from, at(&npc.weapon_transform));
        assert_eq!(left.from, at(&npc.staff_orb));
        for bolt in [right, left] {
            let d = (0..3)
                .map(|i| (bolt.from[i] - body[i]).powi(2))
                .sum::<f32>()
                .sqrt();
            assert!(d < 3.0, "bolt {d} m from the warlock's body");
        }
        assert_ne!(at(&npc.hip_transform), [0.0; 3], "no hip bone");
        hand_tracks.push(right.from);
    }
    // The idle animation moves the hand, which a fixed body offset would not.
    let moved = hand_tracks
        .windows(2)
        .any(|w| (0..3).any(|i| (w[0][i] - w[1][i]).abs() > 1e-4));
    assert!(moved, "the root should follow the animated hand");
    // And it follows the body.
    let index = s.world.npcs[&h].body;
    let before = hand_tracks[hand_tracks.len() - 1];
    let body = &mut s.world.physics.bodies[index];
    let (p, o) = (body.position, body.orientation);
    body.move_to([p[0] + 5.0, p[1], p[2]], o);
    s.step(1. / 60.);
    s.world.magick_bolts.clear();
    s.world.machine_emperor_lightning(h);
    assert!((s.world.magick_bolts[0].from[0] - before[0]).abs() > 3.0);
}
