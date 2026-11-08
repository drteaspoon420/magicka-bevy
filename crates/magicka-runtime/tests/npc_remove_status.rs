use magicka_content::character::{AbilityKind, ActionKind};
use magicka_runtime::play_content::load_character;

#[test]
fn authored_status_removal_inventory() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut count = 0;
    let mut paths: Vec<_> = std::fs::read_dir(root.join("Data/Characters"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort();
    for path in paths {
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("xnb"))
        {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap();
        let t = load_character(&root, name).unwrap();
        for a in &t.abilities {
            if !matches!(a.kind, AbilityKind::RemoveStatus) {
                continue;
            }
            count += 1;
            eprintln!(
                "{name}: target={} cooldown={} animations={:?} expression={:?}",
                a.target, a.cooldown, a.animation_keys, a.fuzzy_expression
            );
            for animation in t.animation_sets.iter().flatten().filter(|an| {
                a.animation_keys
                    .iter()
                    .any(|k| k.eq_ignore_ascii_case(&an.animation))
            }) {
                for action in &animation.actions {
                    if matches!(action.kind, ActionKind::RemoveStatus { .. }) {
                        eprintln!(
                            "action {:?} at {}..{}",
                            action.kind, action.start_time, action.end_time
                        );
                    }
                }
            }
        }
    }
    assert_eq!(count, 7);
}

#[test]
fn wet_orcs_choose_their_authored_removal_animation_and_clear_only_wet() {
    use magicka_runtime::{
        animation_controller::animation_value, npc_combat::AbilityBody, play_session::PlaySession,
    };
    use magicka_sim::status::{Status, StatusEffect};
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let names = [
        "Orc_captain_noarmour",
        "Orc_infested",
        "Orc_light_armored",
        "Orc_regular",
        "Orc_regular_axe",
        "Orc_vietcong_rpd",
        "Orc_warrior_noarmour",
    ];
    let animation = animation_value("removestatus0").unwrap();
    for name in names {
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.disable_scripts();
        s.world.flags.cutscene = false;
        s.world.flags.input_limited = false;
        for _ in 0..60 {
            s.step(1. / 60.);
        }
        // The infested orc's expression subtracts proximity desirability.
        // At this distance its authored wet-removal score is positive.
        let owner = s.spawn_npc(name, "attack", 25.).unwrap();
        let n = s.world.npcs.get_mut(&owner).unwrap();
        assert!(n
            .combat
            .abilities
            .iter()
            .any(|a| matches!(a.body, AbilityBody::RemoveStatus)));
        n.character
            .combat
            .statuses
            .set_effect(StatusEffect::new(Status::Wet, 0., 100., true))
            .unwrap();
        n.character
            .combat
            .statuses
            .set_effect(StatusEffect::new(Status::Greased, 0., 100., true))
            .unwrap();
        n.character.combat.statuses.current_mask = Status::Wet as u16 | Status::Greased as u16;
        n.character.status_cues.exists |= Status::Wet as u16;
        n.character.status_cues.playing |= Status::Wet as u16;
        let mut began = false;
        let mut cleared = false;
        for _ in 0..180 {
            s.step(1. / 60.);
            let n = &s.world.npcs[&owner];
            if n.motion.controller.current_animation == animation {
                began = true;
                assert!(!n.motion.allow_attack_rotate, "{name} removal rotates");
            }
            if n.character
                .combat
                .statuses
                .effect(Status::Wet)
                .unwrap()
                .dead()
            {
                assert!(began, "{name} wet cleared without its animation");
                assert!(
                    n.motion.controller.time / n.motion.controller.clip_duration() >= 0.5,
                    "{name} cleared Wet before authored action time"
                );
                assert!(
                    !n.character
                        .combat
                        .statuses
                        .effect(Status::Greased)
                        .unwrap()
                        .dead(),
                    "{name} unrelated status cleared"
                );
                assert_eq!(
                    n.character.status_cues.playing & Status::Wet as u16,
                    0,
                    "{name} wet cue not stopped"
                );
                cleared = true;
                break;
            }
        }
        assert!(
            began && cleared,
            "{name} did not execute authored wet removal"
        );
        eprintln!("{name}: normal attack selected removestatus0, cleared Wet, preserved Greased and stopped Wet cue");
    }
}

#[test]
fn status_flags_match_native_enum_names_and_signed_underlying_type() {
    use magicka_runtime::character_motion::status_mask;
    assert_eq!(status_mask(" wEt, COLD, Life ").unwrap(), 2 | 8 | 32);
    assert_eq!(status_mask("-1").unwrap(), u16::MAX);
    assert_eq!(status_mask("NumberOfTypes").unwrap(), 512);
    assert!(status_mask("wet | cold").is_err());
    assert!(status_mask("65535").is_err());
}

#[test]
fn removal_selection_respects_status_expression_animation_and_cooldown() {
    use magicka_runtime::{
        animation_controller::animation_value,
        npc_combat::{AbilityBody, CombatAgent, CombatView},
        play_session::PlaySession,
    };
    use magicka_sim::ai_expression::ExpressionArgs;
    use std::sync::Arc;
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    let handle = s.spawn_npc("Orc_regular", "attack", 25.).unwrap();
    let ability = s.world.npcs[&handle]
        .combat
        .abilities
        .iter()
        .find(|a| matches!(a.body, AbilityBody::RemoveStatus))
        .unwrap()
        .clone();
    let mut characters = s.world.combatants();
    let owner = characters.iter().position(|c| c.handle == handle).unwrap();
    let target = characters.iter().position(|c| c.avatar).unwrap();
    let args = ExpressionArgs::new(
        characters[owner].position,
        characters[owner].direction,
        characters[target].position,
        characters[target].direction,
    );
    let mut agent = CombatAgent::new(Arc::new(vec![ability]), Default::default(), 1);
    let select = |agent: &mut CombatAgent, characters: &[_]| {
        agent
            .choose_ability(
                &CombatView::new(characters, None).unwrap(),
                owner,
                target,
                &args,
                false,
            )
            .1
    };
    assert_eq!(
        select(&mut agent, &characters),
        None,
        "dry orc should not remove Wet"
    );
    characters[owner].status_mask = 2;
    assert_eq!(select(&mut agent, &characters), Some(0));
    characters[owner].current_animation = animation_value("removestatus0").unwrap();
    assert_eq!(
        select(&mut agent, &characters),
        None,
        "removal animation must not restart"
    );
    characters[owner].current_animation = animation_value("idle").unwrap();
    agent.cooldowns[0] = 1.;
    assert_eq!(select(&mut agent, &characters), None);
    agent.cooldowns[0] = 0.;
    let mut missing_expression = agent.abilities[0].clone();
    missing_expression.expression = None;
    agent.abilities = Arc::new(vec![missing_expression]);
    assert_eq!(
        select(&mut agent, &characters),
        None,
        "do not invent missing native desirability"
    );
}
