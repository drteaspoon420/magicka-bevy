use magicka_content::character::{AbilityKind, ActionKind};
use magicka_runtime::play_content::{load_character, melee_weapons};
use std::path::{Path, PathBuf};
fn content() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
}

#[test]
fn authored_special_ability_and_action_inventory() {
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
    let mut types = std::collections::BTreeMap::new();
    for path in paths {
        let name = path.file_stem().unwrap().to_str().unwrap();
        let template = load_character(&root, name).unwrap();
        let weapons = melee_weapons(&root, &template);
        for a in &template.abilities {
            if let AbilityKind::SpecialAbilityAbility { weapon, .. } = a.kind {
                count += 1;
                let item = weapons
                    .iter()
                    .find(|(slot, _)| *slot == weapon)
                    .and_then(|(_, w)| w.special.as_ref());
                eprintln!(
                    "{name}: {:?}, cooldown={}, keys={:?}, expression={:?}; item_special={:?}",
                    a.kind, a.cooldown, a.animation_keys, a.fuzzy_expression, item
                );
            }
        }
        for clip in template.animation_sets.iter().flatten() {
            for action in &clip.actions {
                if let ActionKind::SpecialAbility { weapon, ability } = &action.kind {
                    if let Some(a) = ability {
                        *types.entry(a.type_name.to_ascii_lowercase()).or_insert(0) += 1;
                    }
                    eprintln!(
                        "{name}/{} special at {}..{} slot={weapon} ability={ability:?}",
                        clip.animation, action.start_time, action.end_time
                    );
                }
            }
        }
    }
    assert_eq!(count, 11);
    eprintln!("direct action classes: {types:?}");
}

const DT: f32 = 1. / 60.;
fn session(name: &str) -> Option<(magicka_runtime::play_session::PlaySession, u16)> {
    use magicka_runtime::npc_combat::AbilityBody;
    let root = content();
    if !root.is_dir() {
        return None;
    }
    let mut s =
        magicka_runtime::play_session::PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    for _ in 0..60 {
        s.step(DT);
    }
    let h = s.spawn_npc(name, "attack", 4.).unwrap();
    let npc = s.world.npcs.get_mut(&h).unwrap();
    let ability = npc
        .combat
        .abilities
        .iter()
        .find(|a| matches!(a.body, AbilityBody::Special { .. }))
        .unwrap()
        .clone();
    npc.combat.abilities = std::sync::Arc::new(vec![ability]);
    npc.combat.cooldowns.fill(0.);
    Some((s, h))
}
#[test]
fn dwarf_epic_buff_refreshes_without_stacking_then_expires() {
    for name in ["Dwarf_diplomat", "Dwarf_engineer", "Dwarf_rifleman"] {
        let Some((mut s, h)) = session(name) else {
            return;
        };
        let mut first = None;
        let mut last = None;
        for tick in 0..650 {
            s.step(DT);
            let n = &s.world.npcs[&h];
            let boosts = n.character.buffs.damage_boosts();
            if !boosts.is_empty() {
                first.get_or_insert(tick);
                last = Some(tick);
                assert_eq!(
                    boosts.len(),
                    1,
                    "refresh uses the ability's stable buff identity"
                );
                assert_eq!(boosts[0].amount, 1.);
                assert_eq!(boosts[0].magnitude, 1.1);
                assert_eq!(boosts[0].element, 0xffff);
            }
        }
        eprintln!("{name}: buff first={first:?}, last={last:?}");
        let first = first.expect("ordinary AI must execute the authored EpicBuff");
        let last = last.unwrap();
        assert!((8.0..=8.6).contains(&((last - first) as f32 * DT)));
        let n = &s.world.npcs[&h];
        assert!(n.character.nonvisual_auras.is_empty());
        assert!(n.character.buffs.damage_boosts().is_empty());
        let special = n
            .weapons
            .iter()
            .find_map(|(_, w)| w.special.as_ref())
            .unwrap();
        assert!(
            !special.ready(),
            "item recharge still blocks recast after AI cooldown expires"
        );
        assert!(n.combat.cooldowns[0] <= 0.);
    }
}
#[test]
fn druid_animation_summons_owned_spirit_and_item_recharge_blocks_early_recast() {
    use magicka_runtime::character_motion::CharacterState;
    for name in ["Druid", "Dungeons_Druid"] {
        let Some((mut s, h)) = session(name) else {
            return;
        };
        let mut first = None;
        for tick in 0..1600 {
            s.step(DT);
            let spirits: Vec<_> = s
                .world
                .npcs
                .iter()
                .filter(|(_, n)| n.summon.is_some_and(|(owner, _)| owner == h))
                .map(|(&child, _)| child)
                .collect();
            if let Some(&child) = spirits.first() {
                if first.is_none() {
                    first = Some(tick);
                    let n = &s.world.npcs[&child];
                    assert!(n.character.definition.id.eq_ignore_ascii_case("treespirit"));
                    assert_eq!(n.motion.state, CharacterState::Resurrection);
                    assert_eq!(n.weights.alert_radius, 12.);
                    assert_eq!(
                        n.faction_override,
                        Some(
                            s.world
                                .combatants()
                                .iter()
                                .find(|c| c.handle == h)
                                .unwrap()
                                .faction
                        )
                    );
                }
                assert_eq!(
                    spirits.len(),
                    1,
                    "30 s item recharge must block the 25 s AI recast"
                );
                for child in spirits {
                    s.world.npcs.get_mut(&child).unwrap().combat.abilities =
                        std::sync::Arc::new(Vec::new());
                }
            }
        }
        eprintln!("{name}: spirit first={first:?}");
        assert!(
            first.is_some(),
            "cast_spell6 must execute its authored item-special action"
        );
    }
}
#[test]
fn special_readiness_rejects_missing_items_positive_cooldown_and_nan() {
    use magicka_runtime::{
        item_special::ItemSpecial,
        npc_combat::{CombatAgent, CombatView},
    };
    use magicka_sim::ai_expression::ExpressionArgs;
    let Some((s, h)) = session("Dwarf_engineer") else {
        return;
    };
    let mut characters = s.world.combatants();
    let owner = characters.iter().position(|c| c.handle == h).unwrap();
    let target = owner;
    let args = ExpressionArgs::new(
        characters[owner].position,
        characters[owner].direction,
        characters[target].position,
        characters[target].direction,
    );
    let mut agent = CombatAgent::new(
        s.world.npcs[&h].combat.abilities.clone(),
        Default::default(),
        1,
    );
    let view = CombatView::new(&characters, None).unwrap();
    assert_eq!(
        agent.choose_ability(&view, owner, target, &args, false).1,
        Some(0)
    );
    assert_eq!(
        agent.choose_ability(&view, owner, target, &args, true).1,
        Some(0),
        "SpecialAbilityAbility does not reject gripping"
    );
    characters[owner].special_ready.fill(false);
    let view = CombatView::new(&characters, None).unwrap();
    assert_eq!(
        agent.choose_ability(&view, owner, target, &args, false).1,
        None
    );
    let original = s.world.npcs[&h]
        .weapons
        .iter()
        .find_map(|(_, w)| w.special.as_ref())
        .unwrap();
    let mut special: ItemSpecial = original.clone();
    special.cooldown = f32::NAN;
    assert!(!special.ready() && special.execute().is_none());
    special.cooldown = 0.;
    let first = special.execute().unwrap();
    assert_eq!(special.cooldown, special.recharge);
    assert!(special.execute().is_none());
    special.tick(special.recharge);
    let second = special.execute().unwrap();
    assert_eq!(first.1.buff_id, second.1.buff_id);
}

#[test]
fn rogue_smoke_bomb_teleports_back_and_tank_special_animation_executes() {
    use magicka_runtime::character_motion::CharacterState;
    let Some((mut s, h)) = session("Wizard_Rogue") else {
        return;
    };
    let mut moved = false;
    for _ in 0..240 {
        let body = s.world.npcs[&h].body;
        let before = s.world.physics.bodies[body].position;
        let facing = s.world.physics.bodies[body].direction();
        s.step(DT);
        let after = s.world.physics.bodies[body].position;
        let delta: [f32; 3] = std::array::from_fn(|i| after[i] - before[i]);
        if delta.iter().map(|v| v * v).sum::<f32>() > 4. {
            assert!(
                delta[0] * facing[0] + delta[2] * facing[2] < 0.,
                "SmokeBomb retreats along a perturbed facing"
            );
            moved = true;
            break;
        }
    }
    assert!(
        moved,
        "Rogue's ordinary AI must execute its staff SmokeBomb"
    );
    assert!(s.world.npcs[&h]
        .weapons
        .iter()
        .filter_map(|(_, w)| w.special.as_ref())
        .any(|i| i.cooldown > 0.));
    let Some((mut s, h)) = session("Wizard_Tank") else {
        return;
    };
    let enemy = s.spawn_npc("Orc_regular", "idle", 8.).unwrap();
    s.world.npcs.get_mut(&enemy).unwrap().combat.abilities = std::sync::Arc::new(Vec::new());
    let special = magicka_runtime::animation_controller::animation_value("special1").unwrap();
    let mut saw = false;
    for _ in 0..360 {
        s.step(DT);
        let n = &s.world.npcs[&h];
        saw |= n.motion.state == CharacterState::Attack
            && n.motion.controller.current_animation == special;
    }
    assert!(
        saw,
        "animation-only item special must enter its authored animation"
    );
}
#[test]
fn goblin_authored_teleport_action_consumes_item_cooldown_and_moves() {
    let Some((mut s, h)) = session("Goblin_shaman") else {
        return;
    };
    // Start the exact attack selected by the authored SpecialAbilityAbility;
    // its incoming-spell danger desirability is a separate encounter requirement.
    // A wide procedural floor provides a valid landing for every possible
    // 20 m random destination; retail landing failures still consume cooldown.
    let level = magicka_runtime::authored_arena::decode(br#"<Arena version="1"><Floor center="0,0,0" size="64,64" cell="4" clearance="0.5" height="2"/></Arena>"#).unwrap();
    s.world.collision = magicka_runtime::live_collision::LiveCollisionScene::load(
        &level,
        &magicka_content::skinned::IDENTITY,
    )
    .unwrap();
    s.world.nav = Some(magicka_runtime::navigation::nav_mesh(&level.nav_mesh).unwrap());
    let npc_body = s.world.npcs[&h].body;
    let player_body = s.avatar().body;
    s.world.physics.bodies[npc_body].position = [0., 2., 4.];
    s.world.physics.bodies[player_body].position = [0., 2., 0.];
    let animation = s.world.npcs[&h].combat.abilities[0].animations[0];
    s.world.npcs.get_mut(&h).unwrap().combat.abilities = std::sync::Arc::new(Vec::new());
    s.world
        .npcs
        .get_mut(&h)
        .unwrap()
        .motion
        .attack(animation, false);
    let body = s.world.npcs[&h].body;
    let mut moved = false;
    let mut consumed = false;
    for _ in 0..180 {
        let before = s.world.physics.bodies[body].position;
        s.world.update_npcs(DT);
        let after = s.world.physics.bodies[body].position;
        moved |= after[0] != before[0] || after[2] != before[2];
        consumed |= s.world.npcs[&h]
            .weapons
            .iter()
            .filter_map(|(_, w)| w.special.as_ref())
            .any(|i| i.cooldown > 0.);
    }
    eprintln!("goblin: moved={moved}, effects={:?}", s.world.mine_effects);
    assert!(s
        .world
        .mine_effects
        .iter()
        .any(|(name, ..)| *name == "magick_teleport_disappear"));
    assert!(s
        .world
        .mine_effects
        .iter()
        .any(|(name, ..)| *name == "magick_teleport_appear"));
    assert!(consumed, "authored action must execute the item's special");
    assert!(
        moved,
        "RandomTeleport must reach the existing live teleport host"
    );
}

#[test]
fn nullify_removes_the_aura_after_its_final_refresh_then_the_buff_expires() {
    let Some((mut s, h)) = session("Dwarf_engineer") else {
        return;
    };
    for _ in 0..5 {
        s.step(DT);
    }
    assert_eq!(s.world.npcs[&h].character.nonvisual_auras.len(), 1);
    s.world.npcs.get_mut(&h).unwrap().character.nonvisual_auras[0].period = 0.;
    s.world.nullify();
    assert_eq!(s.world.npcs[&h].character.nonvisual_auras[0].ttl, 0.);
    s.world.update_players(DT);
    assert!(s.world.npcs[&h].character.nonvisual_auras.is_empty());
    assert_eq!(
        s.world.npcs[&h].character.buffs.damage_boosts().len(),
        1,
        "native Execute runs once before expired aura removal"
    );
    for _ in 0..45 {
        s.step(DT);
    }
    assert!(s.world.npcs[&h].character.buffs.damage_boosts().is_empty());
}

#[test]
fn client_specials_consume_recharge_without_authoritative_teleport_or_summon() {
    for name in ["Druid", "Goblin_shaman", "Wizard_Rogue"] {
        let Some((mut s, h)) = session(name) else {
            return;
        };
        let npc = s.world.npcs.get_mut(&h).unwrap();
        npc.combat.abilities = std::sync::Arc::new(Vec::new());
        let item = npc
            .weapons
            .iter_mut()
            .find_map(|(_, w)| w.special.as_mut())
            .unwrap();
        let recharge = item.recharge;
        npc.special_requests.push(item.execute().unwrap().1);
        let body = npc.body;
        let before = s.world.physics.bodies[body].position;
        s.world.role = magicka_sim::damage_delivery::NetworkRole::Client;
        s.world.update_npcs(0.);
        assert_eq!(s.world.physics.bodies[body].position, before);
        assert!(s
            .world
            .npcs
            .iter()
            .all(|(_, n)| !n.summon.is_some_and(|(owner, _)| owner == h)));
        assert!(s.world.mine_effects.is_empty());
        let cooldown = s.world.npcs[&h]
            .weapons
            .iter()
            .find_map(|(_, w)| w.special.as_ref())
            .unwrap()
            .cooldown;
        assert_eq!(
            cooldown, recharge,
            "client Execute returns success and the item still consumes recharge"
        );
    }
}
#[test]
fn unsupported_authored_action_reports_once_instead_of_growing_each_cast() {
    let Some((mut s, h)) = session("Dwarf_engineer") else {
        return;
    };
    let template = load_character(&content(), "Elder_thing").unwrap();
    let ability = template
        .animation_sets
        .iter()
        .flatten()
        .flat_map(|clip| &clip.actions)
        .find_map(|action| match &action.kind {
            ActionKind::SpecialAbility {
                ability: Some(a), ..
            } if a.type_name.eq_ignore_ascii_case("stargaze") => Some(a.clone()),
            _ => None,
        })
        .unwrap();
    // Every authored ability is reconstructed now; an unknown class name
    // stands in for an unsupported one.
    let mut ability = ability;
    ability.type_name = "NotAnAbility".into();
    s.world.npcs.get_mut(&h).unwrap().combat.abilities = std::sync::Arc::new(Vec::new());
    for _ in 0..100 {
        s.world.npcs.get_mut(&h).unwrap().special_requests.push(
            magicka_runtime::item_special::SpecialInvocation {
                ability: ability.clone(),
                buff_id: None,
            },
        );
        s.world.update_npcs(0.);
    }
    assert_eq!(
        s.world
            .log
            .iter()
            .filter(|line| line.as_str() == "special ability not reconstructed: NotAnAbility")
            .count(),
        1
    );
}
