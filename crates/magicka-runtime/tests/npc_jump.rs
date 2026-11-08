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
fn authored_jump_ability_and_action_inventory() {
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
    let (mut abilities, mut actions) = (0, 0);
    for path in paths {
        let name = path.file_stem().unwrap().to_str().unwrap();
        let template = load_character(&root, name).unwrap();
        for a in &template.abilities {
            if matches!(a.kind, AbilityKind::Jump { .. }) {
                abilities += 1;
                assert!(matches!(
                    CombatAbility::from_content(a).unwrap().body,
                    AbilityBody::Jump { .. }
                ));
                eprintln!(
                    "{name} Jump ability: {:?}, cooldown={}, keys={:?}, expression={:?}",
                    a.kind, a.cooldown, a.animation_keys, a.fuzzy_expression
                );
            }
        }
        for clip in template.animation_sets.iter().flatten() {
            for a in &clip.actions {
                if matches!(a.kind, ActionKind::Jump { .. }) {
                    actions += 1;
                    eprintln!(
                        "{name}/{} Jump action: {:?} at {}..{}",
                        clip.animation, a.kind, a.start_time, a.end_time
                    );
                }
            }
        }
    }
    assert_eq!(abilities, 2);
    assert_eq!(actions, 8);
}

#[test]
fn jump_range_uses_three_dimensions_without_capsule_radii() {
    use magicka_runtime::npc_combat::in_range;
    let ability = CombatAbility {
        cooldown: 0.,
        target: 2,
        expression: None,
        animations: vec![],
        body: AbilityBody::Jump {
            min_range: 8.,
            max_range: 18.,
            arc: 0.1,
            elevation: 1.,
        },
    };
    assert!(!in_range(&ability, [0.; 3], 100., [0., 7., 0.], 100.));
    assert!(in_range(&ability, [0.; 3], 100., [0., 10., 0.], 100.));
    assert!(!in_range(&ability, [0.; 3], 100., [18., 1., 0.], 100.));
}

#[test]
fn starspawn_jump_abilities_launch_and_recover_through_air_and_land_states() {
    use magicka_runtime::{character_motion::CharacterState as S, play_session::PlaySession};
    let root = content();
    if !root.is_dir() {
        return;
    }
    for name in ["Starspawn", "Grimnirs_Starspawn"] {
        let mut session = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        session.disable_scripts();
        session.world.flags.cutscene = false;
        session.world.flags.input_limited = false;
        for _ in 0..60 {
            session.step(1. / 60.);
        }
        // Authored Jump desirability peaks at (8+18)/2 while the competing
        // zero-cooldown melee has a constant 0.75 score.
        let handle = session.spawn_npc(name, "attack", 13.).unwrap();
        let npc = session.world.npcs.get_mut(&handle).unwrap();
        for (i, ability) in npc.combat.abilities.iter().enumerate() {
            if !matches!(ability.body, AbilityBody::Jump { .. }) {
                npc.combat.cooldowns[i] = ability.cooldown;
            }
        }
        let (mut airborne, mut landed, mut recovered) = (false, false, false);
        let mut apex = 0.0f32;
        let mut origin = None;
        for _ in 0..720 {
            session.step(1. / 60.);
            let npc = &session.world.npcs[&handle];
            let body = &session.world.physics.bodies[npc.body];
            if npc.motion.state == S::Jump {
                if !airborne {
                    let ability = npc.combat.busy_ability.unwrap();
                    assert!(matches!(
                        npc.combat.ability(ability).body,
                        AbilityBody::Jump { .. }
                    ));
                    assert!(npc.combat.cooldowns[ability] > 0.);
                    origin = Some(body.position);
                }
                airborne = true;
                assert!(!body.allow_move && !body.allow_rotate);
                apex = apex.max(body.position[1] - origin.unwrap()[1]);
            }
            if airborne && npc.motion.state == S::Land {
                landed = true;
            }
            if landed && matches!(npc.motion.state, S::Idle | S::Move) {
                assert!(body.allow_move && body.allow_rotate);
                recovered = true;
                break;
            }
        }
        assert!(
            airborne && landed && recovered,
            "{name}: airborne={airborne}, landed={landed}, recovered={recovered}"
        );
        assert!(apex > 0.5, "{name} Jump had no upward flight");
        eprintln!("{name}: native Jump/Land recovered; apex={apex}");
    }
}

#[test]
fn beastman_authored_melee_jump_action_launches_once_at_its_window() {
    use magicka_runtime::{animation_controller::animation_value, play_session::PlaySession};
    let root = content();
    if !root.is_dir() {
        return;
    }
    let mut session = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    session.disable_scripts();
    session.world.flags.cutscene = false;
    session.world.flags.input_limited = false;
    for _ in 0..60 {
        session.step(1. / 60.);
    }
    let handle = session.spawn_npc("Beastman_raider", "attack", 12.).unwrap();
    let jump_begin = animation_value("move_jump_begin").unwrap();
    let (mut began, mut jumped) = (false, false);
    for _ in 0..480 {
        session.step(1. / 60.);
        let npc = &session.world.npcs[&handle];
        let body = &session.world.physics.bodies[npc.body];
        if npc.motion.controller.current_animation == jump_begin {
            began = true;
        }
        if body.is_jumping {
            assert!(began, "launch preceded the authored jump clip");
            assert!(npc.motion.controller.time / npc.motion.controller.clip_duration() >= 0.9);
            jumped = true;
            break;
        }
    }
    assert!(began && jumped, "authored melee Jump action never launched");
}
