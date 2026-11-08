//! Block ability (Abilities.Block), BlockState, the Block animation action
//! and Character.BlockDamage on live NPCs. See research/npc-block.md.
use magicka_content::character::{AbilityKind, ActionKind};
use magicka_runtime::{npc_combat::CombatAbility, play_content::load_character};
use std::path::PathBuf;

fn content() -> PathBuf {
    magicka_content::retail_content_dir()
}

#[test]
fn authored_block_inventory_has_live_abilities_and_animation_windows() {
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
            if !matches!(ability.kind, AbilityKind::Block { .. }) {
                continue;
            }
            count += 1;
            let live = CombatAbility::from_content(ability).unwrap();
            assert!(matches!(
                live.body,
                magicka_runtime::npc_combat::AbilityBody::Block { .. }
            ));
            eprintln!(
                "{name}: {:?}; target={}; cooldown={}; expression={:?}",
                ability.kind, ability.target, ability.cooldown, ability.fuzzy_expression
            );
            let windows: Vec<_> = template
                .animation_sets
                .iter()
                .flatten()
                .filter(|a| a.animation.eq_ignore_ascii_case("block"))
                .flat_map(|a| &a.actions)
                .filter(|a| matches!(a.kind, ActionKind::Block { .. }))
                .collect();
            eprintln!("block actions: {windows:?}");
        }
    }
    assert_eq!(count, 4);
    eprintln!("authored Block abilities: {count}");
}

#[test]
fn orcs_with_attacks_on_cooldown_select_authored_block_and_expire() {
    use magicka_runtime::{character_motion::CharacterState, play_session::PlaySession};
    let root = content();
    if !root.is_dir() {
        return;
    }
    for name in ["Orc_captain", "Orc_warrior"] {
        let mut session = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        session.disable_scripts();
        session.world.flags.cutscene = false;
        session.world.flags.input_limited = false;
        for _ in 0..60 {
            session.step(1. / 60.);
        }
        let handle = session.spawn_npc(name, "attack", 6.).unwrap();
        // Native melee scores tie Block and are stored first. Model a recovery
        // interval after the competing attacks rather than forcing an animation.
        let npc = session.world.npcs.get_mut(&handle).unwrap();
        for (i, ability) in npc.combat.abilities.iter().enumerate() {
            if !matches!(
                ability.body,
                magicka_runtime::npc_combat::AbilityBody::Block { .. }
            ) {
                npc.combat.cooldowns[i] = ability.cooldown;
            }
        }
        let mut activated = false;
        for _ in 0..360 {
            session.step(1. / 60.);
            let npc = &session.world.npcs[&handle];
            if npc.character.block_item == 1 {
                assert_eq!(npc.motion.state, CharacterState::Block, "{name}");
                assert_eq!(npc.motion.controller.current_animation, 35);
                assert!(npc.motion.blocking && npc.motion.block_timer > 0.);
                assert_eq!(
                    npc.combat.busy_ability, None,
                    "Block action must release busy ability"
                );
                assert!(npc.character.block_values.values().sum::<i32>() > 0);
                eprintln!(
                    "{name}: selected live block, item={}, values={:?}, timer={}",
                    npc.character.block_item, npc.character.block_values, npc.motion.block_timer
                );
                activated = true;
                break;
            }
        }
        assert!(
            activated,
            "{name} never selected its authored block ability"
        );
        // Deliver a frontal Earth hit through the live body adapter and check
        // the recovered effect reaches the decoded particle host.
        {
            use magicka_runtime::avatar::deliver_to_body;
            use magicka_sim::{
                damage::{attack, Damage},
                damage_delivery::DamagePayload,
                resistance::Resistance,
                spells::Elements,
            };
            let npc = session.world.npcs.get_mut(&handle).unwrap();
            let body = &mut session.world.physics.bodies[npc.body];
            let direction = body.direction();
            let impact = std::array::from_fn(|i| body.position[i] + direction[i]);
            let health = npc.character.combat.health.hit_points;
            let result = deliver_to_body(
                &mut npc.character,
                body,
                &[Resistance::IDENTITY; 11],
                false,
                handle,
                DamagePayload::Single(Damage {
                    element: Elements::EARTH.0,
                    attack: attack::DAMAGE,
                    amount: 100.,
                    magnitude: 1.,
                }),
                Some(0),
                impact,
                0.,
            );
            assert_ne!(result & magicka_sim::character_impact::result::DEFLECTED, 0);
            assert_eq!(npc.character.combat.health.hit_points, health);
            assert_eq!(npc.character.block_effects.len(), 1);
            let (effect_position, effect_direction) = npc.character.block_effects[0];
            assert_eq!(effect_position, impact);
            assert_eq!(
                effect_direction,
                std::array::from_fn(|i| impact[i] - body.position[i])
            );
        }
        let effect = magicka_runtime::scene_loading::custom_hash("melee_block");
        session.step(1. / 60.);
        assert!(
            session.effects.active_effects_by_hash(effect) > 0,
            "{name} block effect did not reach the particle host"
        );
        assert!(session.world.npcs[&handle]
            .character
            .block_effects
            .is_empty());
        session
            .world
            .npcs
            .get_mut(&handle)
            .unwrap()
            .set_order(0, 0, 0, 0, None);
        for _ in 0..150 {
            session.step(1. / 60.);
        }
        let npc = &session.world.npcs[&handle];
        assert!(!npc.motion.blocking, "{name} blocking timer never expired");
        assert_eq!(
            npc.character.block_item, -1,
            "{name} animation retained blocking after exit"
        );
        assert_ne!(npc.motion.state, CharacterState::Block);
    }
}

#[test]
fn live_damage_reads_animation_window_equipment_sum_and_frontal_cone() {
    use magicka_runtime::{
        avatar::deliver_to_body, character::RuntimeCharacter, CharacterDefinition,
    };
    use magicka_sim::{
        character_impact::result::DEFLECTED,
        character_physics::CharacterBody,
        damage::{attack as A, Damage},
        damage_delivery::DamagePayload,
        resistance::Resistance,
        spells::Elements as E,
    };
    let definition = CharacterDefinition {
        id: "block-test".into(),
        faction: 1,
        ethereal: false,
        looks_ethereal: false,
        can_have_status: true,
        resistances: [Resistance::IDENTITY; 11],
        max_hit_points: 1000.,
        health_bars: 1,
        event_conditions: std::sync::Arc::new(vec![]),
        hit_tolerance: 5000.,
        knockdown_tolerance: 5000.,
        max_panic: 0.,
        fearless: false,
        uncharmable: false,
        nonslippery: false,
        length: 1.,
        radius: 0.5,
        mass: 70.,
        speed: 5.,
        turn_speed: 10.,
        regeneration: 0,
        bleed_rate: 0.,
        stun_time: 0.,
        gib_count: 0,
        gibs: vec![],
        blood: 5,
        attached_sounds: Vec::new(),
        zap_modifier: 1.,
        break_free_strength: 1.,
        undying: false,
        undie_time: 0.0,
        undie_hit_points: 0.0,
        model_tints: Vec::new(),
        score_value: 0,
        display_id: String::new(),
    };
    let hit = |slot: i32, origin: [f32; 3], element: E, amount, magnitude| {
        let mut character = RuntimeCharacter::new(definition.clone());
        character.block_item = slot;
        character.block_values.insert(0, 200);
        character.block_values.insert(1, 300);
        let mut body = CharacterBody::new(0.5, 1., 70., 5.);
        let direction = body.direction();
        let attack_position = [
            direction[0] * origin[0],
            origin[1],
            direction[2] * origin[0],
        ];
        let result = deliver_to_body(
            &mut character,
            &mut body,
            &[Resistance::IDENTITY; 11],
            false,
            1,
            DamagePayload::Single(Damage {
                element: element.0,
                attack: A::DAMAGE,
                amount,
                magnitude,
            }),
            Some(2),
            attack_position,
            0.,
        );
        (result, character.combat.health.hit_points)
    };
    assert_eq!(
        hit(1, [1., 50., 0.], E::EARTH, 200., 2.),
        (DEFLECTED, 1000.)
    );
    assert_eq!(
        hit(1, [1., 0., 0.], E::LIGHTNING, 500., 1.),
        (DEFLECTED, 1000.)
    );
    let inactive = hit(-1, [1., 0., 0.], E::EARTH, 700., 1.);
    let rear = hit(1, [-1., 0., 0.], E::EARTH, 700., 1.);
    assert_eq!(inactive.1, 300.);
    assert_eq!(rear.1, inactive.1);
    assert_eq!(rear.0 & DEFLECTED, 0);
    assert_eq!(
        hit(1, [1., 0., 0.], E::EARTH, 700., 2.).1,
        600.,
        "partial block subtracts before multiplying"
    );
    assert_eq!(
        hit(1, [1., 0., 0.], E::FIRE, 200., 1.).1,
        800.,
        "non-blockable elements bypass the window"
    );
}

/// Live-session Block regressions (BlockState, BlockItem and the frontal block sum).
mod live_session {
    use magicka_content::character::AbilityKind;
    use magicka_runtime::{
        character_motion::CharacterState,
        npc_combat::{AbilityBody, CombatAbility},
        play_content::load_character,
        play_session::PlaySession,
    };
    use magicka_sim::{
        damage::{attack, Damage},
        spells::Elements,
    };
    use std::path::{Path, PathBuf};

    const ANIMATION_BLOCK: u16 = 35;

    fn content() -> Option<PathBuf> {
        let root = magicka_content::retail_content_dir();
        root.is_dir().then_some(root)
    }

    fn session(root: &Path) -> PlaySession {
        let mut s = PlaySession::load(root, "Havindr.lvl", "Wizard").unwrap();
        s.disable_scripts();
        s.world.flags.cutscene = false;
        s.world.flags.input_limited = false;
        for _ in 0..60 {
            s.step(1. / 60.);
        }
        s
    }

    /// Put every ability except Block on a long cooldown.
    fn only_block(s: &mut PlaySession, h: u16) {
        let n = s.world.npcs.get_mut(&h).unwrap();
        for i in 0..n.combat.abilities.len() {
            if !matches!(n.combat.abilities[i].body, AbilityBody::Block { .. }) {
                n.combat.cooldowns[i] = 1000.;
            }
        }
    }

    #[test]
    fn authored_block_abilities_load_with_expressions() {
        let Some(root) = content() else {
            return;
        };
        let mut templates = Vec::new();
        for name in ["Orc_warrior", "Orc_captain", "Wizard_Tank", "Boss_Warlord"] {
            let t = load_character(&root, name).unwrap();
            for a in t
                .abilities
                .iter()
                .filter(|a| matches!(a.kind, AbilityKind::Block { .. }))
            {
                let c = CombatAbility::from_content(a).unwrap();
                assert!(matches!(c.body, AbilityBody::Block { .. }), "{name}");
                // Block.Desirability throws without one.
                assert!(c.expression.is_some(), "{name} block without expression");
                templates.push(name);
            }
        }
        assert_eq!(templates.len(), 4, "{templates:?}");
    }

    #[test]
    fn orc_warrior_selects_block_sets_block_item_and_releases_busy() {
        let Some(root) = content() else {
            return;
        };
        let mut s = session(&root);
        let h = s.spawn_npc("Orc_warrior", "attack", 4.).unwrap();
        // With every other ability on cooldown ChooseTarget finds nothing
        // better, NextAbility stays Block and (as natively: Ability.Execute
        // checks no cooldown) it is re-executed after the busy release, so
        // this test stops once the Block action has run.
        only_block(&mut s, h);
        let mut entered = false;
        for _ in 0..600 {
            s.step(1. / 60.);
            let n = &s.world.npcs[&h];
            if n.motion.state != CharacterState::Block {
                assert!(!entered, "left BlockState early for {:?}", n.motion.state);
                continue;
            }
            entered = true;
            assert!(n.motion.blocking);
            if n.character.block_item == 1 {
                assert_eq!(n.motion.controller.current_animation, ANIMATION_BLOCK);
                assert!(
                    n.combat.busy_ability.is_none(),
                    "Block action did not release the busy Block"
                );
                return;
            }
        }
        panic!("orc warrior never ran its Block action (entered BlockState: {entered})");
    }

    #[test]
    fn npc_block_lasts_one_second_then_returns_to_idle() {
        let Some(root) = content() else {
            return;
        };
        let mut s = session(&root);
        let h = s.spawn_npc("Orc_warrior", "idle", 6.).unwrap();
        // No ability may win, so the orc settles into IdleState.
        s.world
            .npcs
            .get_mut(&h)
            .unwrap()
            .combat
            .cooldowns
            .fill(1000.);
        for _ in 0..600 {
            s.step(1. / 60.);
            if s.world.npcs[&h].motion.state == CharacterState::Idle {
                break;
            }
        }
        assert_eq!(s.world.npcs[&h].motion.state, CharacterState::Idle);
        s.world.npcs.get_mut(&h).unwrap().motion.block();
        let mut frames = 0;
        for frame in 1..=240 {
            s.step(1. / 60.);
            let n = &s.world.npcs[&h];
            if n.motion.state == CharacterState::Block {
                frames = frame;
            } else if frames > 0 {
                assert!(
                    matches!(n.motion.state, CharacterState::Idle | CharacterState::Move),
                    "left BlockState for {:?}",
                    n.motion.state
                );
                assert!(!n.motion.blocking);
                break;
            }
        }
        eprintln!("BlockState through frame {frames}");
        // NonPlayerCharacter.Block: a one second timer, ticked before the
        // state machine, so the 60th update clears IsBlocking.
        assert!((58..=60).contains(&frames), "{frames}");
    }

    fn earth_hit(s: &mut PlaySession, h: u16, front: bool) -> (f32, usize) {
        let npc = s.world.npcs.get_mut(&h).unwrap();
        let body = &mut s.world.physics.bodies[npc.body];
        let side = if front { 1.0 } else { -1.0 };
        let attack_position =
            std::array::from_fn(|i| body.position[i] + body.direction()[i] * side);
        let before = npc.character.combat.health.hit_points;
        npc.character.block_effects.clear();
        magicka_runtime::avatar::deliver_to_body(
            &mut npc.character,
            body,
            &[magicka_sim::resistance::Resistance::IDENTITY; 11],
            false,
            h,
            magicka_sim::damage_delivery::DamagePayload::Single(Damage {
                attack: attack::DAMAGE,
                element: Elements::EARTH.0,
                // Large enough to pass Orc_warrior's Earth resistance.
                amount: 5000.,
                magnitude: 1.,
            }),
            None,
            attack_position,
            s.world.play_time,
        );
        let npc = &s.world.npcs[&h];
        (
            before - npc.character.combat.health.hit_points,
            npc.character.block_effects.len(),
        )
    }

    #[test]
    fn blocking_orc_reduces_frontal_earth_damage_by_its_block_values() {
        let Some(root) = content() else {
            return;
        };
        let mut s = session(&root);
        let h = s.spawn_npc("Orc_warrior", "attack", 4.).unwrap();
        let blocked: i32 = s.world.npcs[&h].character.block_values.values().sum();
        eprintln!(
            "Orc_warrior block values {:?}",
            s.world.npcs[&h].character.block_values
        );
        assert!(blocked > 0);
        {
            // Buffs re-apply the template maximum, so raise both.
            let c = &mut s.world.npcs.get_mut(&h).unwrap().character;
            c.definition.max_hit_points = 10_000.;
            c.combat.health.max_hit_points = 10_000.;
            c.combat.health.hit_points = 10_000.;
        }
        for _ in 0..120 {
            s.step(1. / 60.);
        }
        s.world.npcs.get_mut(&h).unwrap().character.block_item = -1;
        // Not blocking: BlockItem is -1, the full hit lands.
        let (open, open_effects) = earth_hit(&mut s, h, true);
        // Separate hits: one Character.Update between them (which resets
        // BlockItem, so it is set again right before each blocked hit).
        let hit = |s: &mut PlaySession, item: i32, front: bool| {
            for _ in 0..30 {
                s.step(1. / 60.);
            }
            let n = s.world.npcs.get_mut(&h).unwrap();
            n.character.combat.health.hit_points = 10_000.;
            n.character.block_item = item;
            earth_hit(s, h, front)
        };
        let (front, front_effects) = hit(&mut s, 1, true);
        let (back, back_effects) = hit(&mut s, 1, false);
        eprintln!(
            "open {open}/{open_effects} front {front}/{front_effects} back {back}/{back_effects}"
        );
        assert_eq!(open_effects, 0);
        assert_eq!(front_effects, 1, "no melee_block effect");
        assert_eq!(back_effects, 0, "hit from behind was blocked");
        // Character.BlockDamage subtracts the summed block values first.
        // Earth resistance (x0.5) applies after the block subtraction.
        assert!(open > 0.);
        assert_eq!(open - front, blocked as f32 * 0.5);
        assert_eq!(back, open);
    }
}
