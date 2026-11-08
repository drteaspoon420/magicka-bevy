//! Dash ability (Abilities.Dash) and DashState. See research/npc-dash.md.
use magicka_content::character::{AbilityKind, ActionKind};
use magicka_runtime::{
    npc_combat::{AbilityBody, CombatAbility},
    play_content::load_character,
};
use std::path::{Path, PathBuf};

fn content() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
}

#[test]
fn all_authored_dash_abilities_have_a_live_representation() {
    let root = content();
    if !root.is_dir() {
        return;
    }
    let mut paths: Vec<_> = std::fs::read_dir(root.join("Data/Characters"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xnb")))
        .collect();
    paths.sort();
    let mut count = 0;
    for path in paths {
        let name = path.file_stem().unwrap().to_str().unwrap();
        let template = load_character(&root, name).unwrap();
        for ability in &template.abilities {
            let AbilityKind::Dash { vector, .. } = &ability.kind else {
                continue;
            };
            count += 1;
            let live = CombatAbility::from_content(ability).unwrap();
            assert!(matches!(live.body, AbilityBody::Dash { vector: v, .. } if v == *vector));
            eprintln!(
                "{name}: {:?}, cooldown={}, animations={:?}, expression={:?}",
                ability.kind, ability.cooldown, ability.animation_keys, ability.fuzzy_expression
            );
            for clip in template.animation_sets.iter().flatten().filter(|clip| {
                ability
                    .animation_keys
                    .iter()
                    .any(|k| clip.animation.eq_ignore_ascii_case(k))
            }) {
                for action in &clip.actions {
                    if matches!(
                        action.kind,
                        ActionKind::Move { .. } | ActionKind::Jump { .. }
                    ) {
                        eprintln!(
                            "{name}/{} movement: {:?} at {}..{}",
                            clip.animation, action.kind, action.start_time, action.end_time
                        );
                    }
                }
            }
        }
    }
    assert_eq!(count, 28);
}

#[test]
fn dash_writes_attack_key_and_stop_attacking_clears_only_native_fields() {
    use magicka_runtime::{
        animation_controller::AnimationSet,
        character_motion::{CharacterMotion, CharacterState},
    };
    let mut motion = CharacterMotion::new(std::sync::Arc::new(AnimationSet::new(
        &[],
        Default::default(),
    )));
    motion.next_dash_animation = 10;
    motion.dash(20, false);
    assert!(motion.dashing);
    assert!(!motion.attacking);
    assert_eq!(motion.state, CharacterState::Idle);
    assert_eq!(motion.next_attack_animation, 20);
    assert_eq!(
        motion.next_dash_animation, 10,
        "Character.Dash must not overwrite the separate dash key"
    );
    motion.stop_attacking();
    assert!(!motion.dashing && !motion.attacking);
    assert_eq!(
        motion.next_attack_animation,
        magicka_runtime::animation_controller::ANIMATION_NONE
    );
    assert_eq!(
        motion.next_dash_animation, 10,
        "native StopAttacking retains the dash key"
    );
}

#[test]
fn dash_fallback_expression_cooldown_and_grip_gates_match_native_order() {
    use magicka_runtime::{
        npc_combat::{CombatAgent, CombatView},
        play_session::PlaySession,
    };
    use magicka_sim::ai_expression::{fuzzy_distance_exponential, Expr, ExpressionArgs};
    use std::sync::Arc;
    let root = content();
    if !root.is_dir() {
        return;
    }
    let mut session = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    let handle = session.spawn_npc("Zombie", "attack", 4.).unwrap();
    let ability = session.world.npcs[&handle]
        .combat
        .abilities
        .iter()
        .find(|a| matches!(a.body, AbilityBody::Dash { .. }))
        .unwrap()
        .clone();
    assert!(ability.expression.is_none());
    let characters = session.world.combatants();
    let owner = characters.iter().position(|c| c.handle == handle).unwrap();
    let target = characters.iter().position(|c| c.avatar).unwrap();
    let args = ExpressionArgs::new(
        characters[owner].position,
        characters[owner].direction,
        characters[target].position,
        characters[target].direction,
    );
    let view = CombatView::new(&characters, None).unwrap();
    let mut agent = CombatAgent::new(Arc::new(vec![ability]), Default::default(), 1);
    let (score, selection) = agent.choose_ability(&view, owner, target, &args, false);
    let AbilityBody::Dash {
        min_range,
        max_range,
        ..
    } = agent.abilities[0].body
    else {
        unreachable!()
    };
    assert_eq!(selection, Some(0));
    assert_eq!(
        score,
        fuzzy_distance_exponential(args.distance, min_range, max_range)
    );
    assert_eq!(
        agent.choose_ability(&view, owner, target, &args, true).1,
        None
    );
    agent.cooldowns[0] = 1.;
    assert_eq!(
        agent.choose_ability(&view, owner, target, &args, false).1,
        None
    );
    agent.cooldowns[0] = 0.;
    let mut authored = agent.abilities[0].clone();
    authored.expression = Some(Expr::read("1").unwrap());
    agent.abilities = Arc::new(vec![authored]);
    assert_eq!(
        agent.choose_ability(&view, owner, target, &args, false).0,
        1.
    );
    assert_eq!(
        agent.choose_ability(&view, owner, target, &args, true).1,
        None,
        "grip rejection must precede an authored expression"
    );
}

#[test]
fn authored_forward_and_reactive_side_dashes_move_through_normal_ai() {
    use magicka_runtime::{
        avatar::PlayerInput, character_motion::CharacterState, play_session::PlaySession,
    };
    let root = content();
    if !root.is_dir() {
        return;
    }
    for (name, distance, incoming_attack) in [
        ("Zombie", 4., false),
        ("Orc_regular", 4., false),
        ("Beastman_raider", 3., true),
    ] {
        let mut session = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        session.disable_scripts();
        session.world.flags.cutscene = false;
        session.world.flags.input_limited = false;
        for _ in 0..60 {
            session.step(1. / 60.);
        }
        if incoming_attack {
            session.handle_input(
                PlayerInput {
                    shift_down: true,
                    move_down: true,
                    ..Default::default()
                },
                1.,
            );
            session.step(1. / 60.);
        }
        let handle = session.spawn_npc(name, "attack", distance).unwrap();
        let mut entry = None;
        let mut displacement = 0.0f32;
        let mut left = 0.0f32;
        let mut forward = 0.0f32;
        let mut finished = false;
        for frame in 0..360 {
            if incoming_attack {
                session.handle_input(
                    PlayerInput {
                        shift_down: true,
                        move_down: frame < 30,
                        ..Default::default()
                    },
                    1.,
                );
            }
            session.step(1. / 60.);
            let npc = &session.world.npcs[&handle];
            let body = &session.world.physics.bodies[npc.body];
            if npc.motion.state == CharacterState::Dash {
                if entry.is_none() {
                    let ability = npc.combat.busy_ability.expect("dash has busy ability");
                    assert!(matches!(
                        npc.combat.ability(ability).body,
                        AbilityBody::Dash { .. }
                    ));
                    assert!(npc.combat.cooldowns[ability] > 0.);
                    assert!(npc.motion.dashing && !npc.motion.attacking);
                    assert!(!npc.motion.allow_attack_rotate);
                    entry = Some((body.position, body.orientation.right, body.direction()));
                    eprintln!(
                        "{name}: entered native dash clip {}",
                        npc.motion.controller.current_animation
                    );
                }
                let (start, right, direction) = entry.unwrap();
                let delta: [f32; 3] = std::array::from_fn(|i| body.position[i] - start[i]);
                displacement = displacement.max((delta[0] * delta[0] + delta[2] * delta[2]).sqrt());
                left = left.max(-delta[0] * right[0] - delta[2] * right[2]);
                forward = forward.max(delta[0] * direction[0] + delta[2] * direction[2]);
            } else if entry.is_some() {
                assert!(!npc.motion.dashing, "DashState exit retained Dashing");
                finished = true;
                break;
            }
        }
        assert!(
            entry.is_some() && finished,
            "{name} never completed an authored dash"
        );
        assert!(displacement > 0.1, "{name} dash was animation-only");
        if incoming_attack {
            assert!(
                left > 0.1,
                "{name} did not follow its authored left-side Move action"
            );
        } else {
            assert!(
                forward > 0.1,
                "{name} did not move forward during its authored dash"
            );
        }
        eprintln!("{name}: dash displacement={displacement}, left={left}, forward={forward}");
    }
}

/// Live-session Dash regressions (selection, DashState and clip motion).
mod live_session {
    use magicka_content::character::AbilityKind;
    use magicka_runtime::{
        animation_controller::animation_value,
        character_motion::CharacterState,
        npc_combat::{AbilityBody, CombatAgent, CombatView},
        play_content::load_character,
        play_session::PlaySession,
    };
    use magicka_sim::ai_expression::ExpressionArgs;
    use std::{path::Path, sync::Arc};

    fn content() -> Option<std::path::PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
        root.is_dir().then_some(root)
    }

    #[test]
    fn authored_dash_abilities_load_as_executable_bodies() {
        let Some(root) = content() else {
            return;
        };
        let mut paths: Vec<_> = std::fs::read_dir(root.join("Data/Characters"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xnb")))
            .collect();
        paths.sort();
        let mut templates = 0;
        for path in paths {
            let name = path.file_stem().unwrap().to_str().unwrap();
            let t = load_character(&root, name).unwrap();
            let mut found = false;
            for a in t
                .abilities
                .iter()
                .filter(|a| matches!(a.kind, AbilityKind::Dash { .. }))
            {
                let c = magicka_runtime::npc_combat::CombatAbility::from_content(a).unwrap();
                assert!(matches!(c.body, AbilityBody::Dash { .. }), "{name}");
                assert!(!c.animations.is_empty(), "{name} dash without animations");
                found = true;
            }
            if found {
                eprintln!("{name}");
                templates += 1;
            }
        }
        assert_eq!(templates, 15);
    }

    #[test]
    fn dash_selection_uses_distance_and_refuses_while_gripping() {
        let Some(root) = content() else {
            return;
        };
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        let handle = s.spawn_npc("Orc_regular", "attack", 3.).unwrap();
        let mut ability = s.world.npcs[&handle]
            .combat
            .abilities
            .iter()
            .find(|a| matches!(a.body, AbilityBody::Dash { .. }))
            .unwrap()
            .clone();
        // Dash.Desirability without an authored expression.
        ability.expression = None;
        let characters = s.world.combatants();
        let owner = characters.iter().position(|c| c.handle == handle).unwrap();
        let target = characters.iter().position(|c| c.avatar).unwrap();
        let args = ExpressionArgs::new(
            characters[owner].position,
            characters[owner].direction,
            characters[target].position,
            characters[target].direction,
        );
        let mut agent = CombatAgent::new(Arc::new(vec![ability]), Default::default(), 1);
        let view = CombatView::new(&characters, None).unwrap();
        let (score, chosen) = agent.choose_ability(&view, owner, target, &args, false);
        assert_eq!(chosen, Some(0), "score {score}");
        let expected =
            magicka_sim::ai_expression::fuzzy_distance_exponential(args.distance, 0., 5.);
        assert_eq!(score, expected);
        let (_, chosen) = agent.choose_ability(&view, owner, target, &args, true);
        assert_eq!(chosen, None, "gripping owner chose Dash");
    }

    #[test]
    fn orc_dash_plays_its_clip_translates_and_leaves_dash_state() {
        let Some(root) = content() else {
            return;
        };
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.disable_scripts();
        s.world.flags.cutscene = false;
        s.world.flags.input_limited = false;
        for _ in 0..60 {
            s.step(1. / 60.);
        }
        let h = s.spawn_npc("Orc_regular", "attack", 4.).unwrap();
        // Leave only the Dash ability on cooldown-free terms: the other
        // abilities are put on a long cooldown so ordinary selection picks it.
        {
            let n = s.world.npcs.get_mut(&h).unwrap();
            let dash = n
                .combat
                .abilities
                .iter()
                .position(|a| matches!(a.body, AbilityBody::Dash { .. }))
                .unwrap();
            for i in 0..n.combat.abilities.len() {
                if i != dash {
                    n.combat.cooldowns[i] = 1000.;
                }
            }
        }
        let body = s.world.npcs[&h].body;
        let clip = animation_value("attack_melee2").unwrap();
        let mut entered = None;
        let mut left = None;
        for frame in 0..600 {
            s.step(1. / 60.);
            let n = &s.world.npcs[&h];
            let p = s.world.physics.bodies[body].position;
            match (entered, n.motion.state) {
                (None, CharacterState::Dash) => {
                    assert!(n.motion.dashing);
                    assert_eq!(n.motion.next_dash_animation, clip);
                    entered = Some((frame, p));
                }
                (Some(_), CharacterState::Dash) => {
                    assert_eq!(n.motion.controller.current_animation, clip);
                }
                (Some((_, start)), state) => {
                    // The orc's dash clip carries a Hold Grip action: a caught
                    // player sends it to GrippingState instead (npc_grip.rs).
                    assert!(
                        matches!(state, CharacterState::Idle | CharacterState::Move)
                            || (state == CharacterState::Gripping
                                && n.character.grip.gripped.is_some()),
                        "left DashState for {state:?}"
                    );
                    assert!(!n.motion.dashing, "Dashing survived DashState.OnExit");
                    left = Some((frame, (p[0] - start[0]).hypot(p[2] - start[2])));
                    break;
                }
                _ => {}
            }
        }
        let (start, _) = entered.expect("orc never entered DashState");
        let (end, displacement) = left.expect("orc never left DashState");
        eprintln!("dash frames {start}..{end}, displacement {displacement}");
        assert!(
            displacement > s.world.physics.bodies[body].radius,
            "authored dash Move did not translate: {displacement}"
        );
        assert!(s.world.npcs[&h].combat.cooldowns.iter().any(|&c| c > 10.));
    }
}
