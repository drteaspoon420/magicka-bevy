//! Live conjuring and Force casting against user-supplied retail content
//! (skips when absent): element keys fill the queue through Avatar.HandleCombo,
//! the Cast button combines it, CastState plays the cast clip whose CastSpell
//! action starts SpraySpell, and the spray damages a script-spawned NPC
//! through the public damage route. See research/character-casting.md.
use magicka_runtime::{
    avatar::{AvatarEvent, AvatarState, PlayerInput},
    play_content::load_start_scene,
    play_session::{PlaySession, SessionEvent},
    scene_script::{ActionDefinition, SceneScriptLayout},
    spawn_action::SpawnState,
};
use magicka_sim::spells::Elements as E;
use std::path::{Path, PathBuf};

const DT: f32 = 1.0 / 60.0;
const LEVEL: &str = "Havindr.lvl";
/// `ELEMENT_KEYS` index of Fire (default key F).
const FIRE: usize = 7;

fn equip_healer_staff(s: &mut PlaySession, root: &Path) {
    equip_staff(s, root, "staff_healers_burden");
}

fn equip_staff(s: &mut PlaySession, root: &Path, name: &str) {
    let bytes = std::fs::read(root.join(format!("Data/items/Wizard/{name}.xnb"))).unwrap();
    let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
    let magicka_content::Content::Item(item) = magicka_content::parse_primary(&xnb)
        .unwrap()
        .unwrap()
        .content
    else {
        panic!("not an item");
    };
    let avatar = s.world.players[0].as_mut().unwrap();
    avatar.items.retain(|(slot, _)| *slot != 1);
    avatar
        .items
        .push((1, magicka_runtime::avatar::AvatarItem::new(*item)));
}

/// The lone campaign avatar's revival fairy keeps its `special_fairy` orbit
/// effect alive for the whole session (revival-fairy.md); spell effect counts
/// exclude it.
fn fairy_effects(s: &PlaySession) -> usize {
    s.effects
        .active_effects_by_hash(magicka_runtime::scene_loading::custom_hash("special_fairy"))
}

#[test]
fn debug_player_loadout_rebuilds_equipment_rig_without_restarting_scene() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let npc = spawn_in_front(&mut s, &root, "Goblin_miner", 5.0);
    let position = s.body().position;
    let scene = s.scene_name().to_string();
    let weapon = s
        .avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1
        .definition
        .name
        .clone();
    s.debug_player_loadout("Wizard_Crusader", "staff_of_invisibility", &weapon)
        .unwrap();
    assert_eq!(s.body().position, position);
    assert_eq!(s.scene_name(), scene);
    assert!(s.world.npcs.contains_key(&npc));
    let player = s.avatar();
    let staff = &player.items.iter().find(|(slot, _)| *slot == 1).unwrap().1;
    assert_eq!(staff.definition.name, "staff_of_invisibility");
    assert_eq!(staff.definition.special_ability.as_ref().unwrap().0, 20.0);
    let rig = player.rig.as_ref().unwrap();
    assert_eq!(rig.attachments[rig.spell_source.unwrap()].weapon_class, 18);
    assert!(s
        .debug_player_loadout("Wizard", "missing_staff", &weapon)
        .is_err());
    assert_eq!(
        s.avatar()
            .items
            .iter()
            .find(|(slot, _)| *slot == 1)
            .unwrap()
            .1
            .definition
            .name,
        "staff_of_invisibility"
    );
}

#[test]
fn invisibility_staff_preserves_special_animation_then_melee_clears_visibility() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    equip_staff(&mut s, &root, "staff_of_invisibility");
    let player = s.world.players[0].as_mut().unwrap();
    player.character.combat.health.shield = magicka_sim::character_health::SelfShield::from_spell(
        &magicka_sim::spells::Spell::default_for(E::SHIELD),
        &mut Vec::new(),
    );
    let mass = player.character.definition.mass;
    s.world.physics.bodies[player.body].mass = mass * 2.0;
    let mut specials = 0;
    let mut cues = 0;
    for i in 0..120 {
        for event in frame(
            &mut s,
            PlayerInput {
                cast_self_down: i < 30 || i == 60,
                ..Default::default()
            },
        ) {
            if let SessionEvent::Avatar(AvatarEvent::ItemSpecial { name, .. }) = event {
                assert_eq!(name, "invisibility");
                specials += 1;
            }
        }
        for (request, _) in std::mem::take(&mut s.audio) {
            if let magicka_runtime::audio::AudioRequest::Start {
                bank: 16,
                cue,
                source,
                ..
            } = request
            {
                if cue == magicka_runtime::scene_loading::custom_hash("magick_invisibility") {
                    assert!(matches!(
                        source,
                        magicka_runtime::audio::SoundSource::Emitter(_)
                    ));
                    cues += 1;
                }
            }
        }
    }
    assert_eq!(specials, 1);
    assert_eq!(cues, 1);
    assert!(!s.avatar().character.combat.health.shield.active());
    assert_eq!(s.world.physics.bodies[s.avatar().body].mass, mass);
    assert_eq!(s.avatar().character.combat.health.invisibility_timer, 15.0);
    assert!(
        !s.avatar().character.just_cast_invisible,
        "Attack entry consumes the preserve flag"
    );
    assert!(
        s.avatar()
            .items
            .iter()
            .find(|(slot, _)| *slot == 1)
            .unwrap()
            .1
            .special_cooldown
            > 17.0
    );
    for i in 0..60 {
        frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                move_down: i < 5,
                ..Default::default()
            },
        );
    }
    assert_eq!(s.avatar().character.combat.health.invisibility_timer, 0.0);
}

#[test]
fn healer_staff_special_casts_two_life_and_obeys_original_recharge() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    equip_healer_staff(&mut s, &root);
    s.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 200.0;
    let mut specials = 0;
    let mut spells = Vec::new();
    let mut unsupported = Vec::new();
    for i in 0..120 {
        let events = frame(
            &mut s,
            PlayerInput {
                cast_self_down: i < 30 || i == 60,
                ..Default::default()
            },
        );
        for event in events {
            match event {
                SessionEvent::Avatar(AvatarEvent::ItemSpecial { name, .. })
                    if name == "HealFriendlies" =>
                {
                    specials += 1
                }
                SessionEvent::Avatar(AvatarEvent::SpellCast {
                    spell, cast_type, ..
                }) => {
                    if cast_type == magicka_sim::damage::CastType::Blast {
                        // The item's cast_self animation can still call CastSpell.
                        // Native Spell.Cast returns immediately for CastType.None.
                        assert_eq!(spell.element, E::NONE);
                    } else {
                        assert_eq!(cast_type, magicka_sim::damage::CastType::SelfCast);
                        spells.push(spell);
                    }
                }
                SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(reason)) => {
                    unsupported.push(reason)
                }
                _ => {}
            }
        }
    }
    assert_eq!(
        specials, 1,
        "held input and a recharge-time press must not retrigger"
    );
    assert_eq!(spells.len(), 1, "the ability casts synchronously, once");
    assert_eq!(spells[0].get(E::LIFE), 2.0);
    assert_eq!(
        s.avatar().character.combat.health.hit_points,
        s.avatar().character.combat.health.max_hit_points
    );
    assert!(
        s.avatar()
            .items
            .iter()
            .find(|(slot, _)| *slot == 1)
            .unwrap()
            .1
            .special_cooldown
            > 7.0
    );
    assert!(unsupported.is_empty(), "{unsupported:?}");
    // Recharge comes from the original item (10 seconds), rather than an
    // animation duration or an input edge timeout.
    for _ in 0..510 {
        frame(&mut s, PlayerInput::default());
    }
    let events = frame(
        &mut s,
        PlayerInput {
            cast_self_down: true,
            ..Default::default()
        },
    );
    // Input dispatch runs on the following simulation frame.
    let events = events
        .into_iter()
        .chain(frame(&mut s, PlayerInput::default()));
    assert_eq!(events.filter(|event| matches!(event,
        SessionEvent::Avatar(AvatarEvent::ItemSpecial { name, .. }) if name == "HealFriendlies"
    )).count(), 1);
}

#[test]
fn healer_staff_unordered_cooldown_blocks_special_but_queued_spell_takes_priority() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    equip_healer_staff(&mut s, &root);
    s.world.players[0]
        .as_mut()
        .unwrap()
        .items
        .iter_mut()
        .find(|(slot, _)| *slot == 1)
        .unwrap()
        .1
        .special_cooldown = f32::NAN;
    let mut specials = 0;
    for i in 0..30 {
        let events = frame(
            &mut s,
            PlayerInput {
                cast_self_down: i < 5,
                ..Default::default()
            },
        );
        specials += events
            .iter()
            .filter(|event| matches!(event, SessionEvent::Avatar(AvatarEvent::ItemSpecial { .. })))
            .count();
    }
    assert_eq!(specials, 0, "native bgt.un rejects NaN cooldown");
    s.world.players[0]
        .as_mut()
        .unwrap()
        .conjure
        .queue
        .push(magicka_sim::spells::Spell::default_for(E::LIFE));
    let mut casts = Vec::new();
    for i in 0..90 {
        for event in frame(
            &mut s,
            PlayerInput {
                cast_self_down: i < 5,
                ..Default::default()
            },
        ) {
            match event {
                SessionEvent::Avatar(AvatarEvent::ItemSpecial { .. }) => specials += 1,
                SessionEvent::Avatar(AvatarEvent::SpellCast {
                    spell,
                    cast_type: magicka_sim::damage::CastType::SelfCast,
                    ..
                }) => casts.push(spell),
                _ => {}
            }
        }
    }
    assert_eq!(specials, 0);
    assert_eq!(casts.len(), 1, "a queued spell uses normal self casting");
    assert_eq!(casts[0].get(E::LIFE), 1.0);
}

#[test]
fn heal_friendlies_excludes_exact_factions_instead_of_faction_bits() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let mut targets = Vec::new();
    for faction in [1, 2, 8, 16, 17] {
        let target = spawn_in_front(&mut s, &root, "Goblin_miner", 20.0);
        s.world
            .entities
            .iter_mut()
            .find(|e| e.handle == target)
            .unwrap()
            .faction = faction;
        s.world
            .npcs
            .get_mut(&target)
            .unwrap()
            .character
            .combat
            .health
            .hit_points = 100.0;
        targets.push((target, faction));
    }
    s.world.heal_friendlies();
    for (target, faction) in targets {
        assert_eq!(
            npc_hp(&s, target),
            if faction == 17 { 600.0 } else { 100.0 },
            "faction {faction}"
        );
    }
}

#[test]
fn wizard_shift_click_melee_uses_equipped_weapon_and_animation_hit_window() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let target = spawn_in_front(&mut s, &root, "Goblin_miner", 1.0);
    let hp = npc_hp(&s, target);
    let mut attacks = 0;
    let mut states = Vec::new();
    let mut hit_changes = 0;
    let mut last_hp = hp;
    for i in 0..90 {
        let events = frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                move_down: i < 30,
                ..Default::default()
            },
        );
        attacks += events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    SessionEvent::Avatar(AvatarEvent::Attack(
                        magicka_sim::avatar_attack::AttackEvent::Attack { .. }
                    ))
                )
            })
            .count();
        states.push(s.avatar().motion.state);
        let new_hp = npc_hp(&s, target);
        if new_hp != last_hp {
            hit_changes += 1;
            last_hp = new_hp;
        }
    }
    assert_eq!(
        attacks, 1,
        "holding an ordinary weapon must not repeat attacks"
    );
    assert!(states.contains(&AvatarState::Attack));
    let weapon = &s
        .avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1;
    eprintln!(
        "wizard melee class {} range {} hitlist {:?}; hp {hp} -> {last_hp}; changes {hit_changes}",
        weapon.definition.weapon_class, weapon.melee.range, weapon.melee.hitlist
    );
    assert!(last_hp < hp, "animation must deliver original item damage");
    assert_eq!(
        hit_changes, 1,
        "single-hit weapon must not redamage during its hit window"
    );
    assert!(s.avatar().conjure.queue.is_empty());
    for i in 0..90 {
        frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                move_down: i == 0,
                ..Default::default()
            },
        );
    }
    assert!(
        npc_hp(&s, target) < last_hp,
        "a new swing must clear the prior hit list"
    );
}

#[test]
fn wizard_weapon_enchant_keeps_avatar_queue_until_cast_action() {
    use magicka_sim::{damage::CastType, weapon_spells::WeaponSpellEvent};
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, FIRE);
    s.handle_input(
        PlayerInput {
            shift_down: true,
            move_down: true,
            ..Default::default()
        },
        16.0 / 9.0,
    );
    assert_eq!(s.avatar().cast.cast_type, CastType::Weapon);
    assert_eq!(s.avatar().conjure.queue.len(), 1);
    let mut awarded = 0;
    for _ in 0..90 {
        let events = frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                ..Default::default()
            },
        );
        awarded += events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    SessionEvent::Avatar(AvatarEvent::Weapon(WeaponSpellEvent::AwardEnchanter))
                )
            })
            .count();
    }
    assert_eq!(awarded, 1);
    assert!(s.avatar().conjure.queue.is_empty());
    assert_eq!(s.avatar().conjure.chanted_spells, 0);
    let item = &s
        .avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1;
    assert!(item.spells.charged());
    assert_eq!(item.spells.peek().element, E::FIRE);
    assert_eq!(item.spells.peek().get(E::FIRE), 1.0);
    assert_eq!(s.avatar().cast.cast_type, CastType::Blast);
}

#[test]
fn wizard_weapon_cast_permission_preserves_queue_and_blocks_enchantment() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, FIRE);
    s.world
        .flags
        .tutorial_cast_types_disabled
        .insert(magicka_sim::damage::CastType::Weapon as i32);
    let mut weapon_events = 0;
    for i in 0..60 {
        let events = frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                move_down: i < 20,
                ..Default::default()
            },
        );
        weapon_events += events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    SessionEvent::Avatar(AvatarEvent::Weapon(_))
                        | SessionEvent::Avatar(AvatarEvent::Attack(_))
                )
            })
            .count();
    }
    assert_eq!(weapon_events, 0);
    assert_eq!(s.avatar().conjure.queue.len(), 1);
    assert!(!s
        .avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1
        .spells
        .charged());
}

/// RailGunSpell.CastWeapon / LightningSpell.CastWeapon: Arcane and
/// Lightning sword enchantments are released by a swing (feedback
/// 2026-10-08: only spray enchantments worked).
#[test]
fn arcane_and_lightning_weapon_enchantments_release_on_a_swing() {
    let Some(root) = content() else {
        return;
    };
    let lightning = magicka_runtime::avatar::ELEMENT_KEYS
        .iter()
        .position(|&e| e == E::LIGHTNING)
        .unwrap();
    for (element, name) in [(ARCANE, "arcane"), (lightning, "lightning")] {
        let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
        s.disable_scripts();
        for _ in 0..60 {
            frame(&mut s, PlayerInput::default());
        }
        conjure(&mut s, element);
        for i in 0..90 {
            frame(
                &mut s,
                PlayerInput {
                    shift_down: true,
                    move_down: i == 0,
                    ..Default::default()
                },
            );
        }
        let front = spawn_in_front(&mut s, &root, "Goblin_miner", 2.0);
        let hp = npc_hp(&s, front);
        let mut unsupported = Vec::new();
        let mut rail_weapons = 0;
        let mut bolts = 0;
        for i in 0..120 {
            for event in frame(
                &mut s,
                PlayerInput {
                    shift_down: true,
                    move_down: i < 30,
                    ..Default::default()
                },
            ) {
                match event {
                    SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(reason)) => {
                        unsupported.push(reason)
                    }
                    SessionEvent::Avatar(AvatarEvent::RailWeapon { .. }) => rail_weapons += 1,
                    _ => {}
                }
            }
            bolts = bolts.max(s.bolts.len());
        }
        assert!(unsupported.is_empty(), "{name}: {unsupported:?}");
        if element == lightning {
            assert!(bolts > 0, "lightning weapon fired no bolts");
        }
        if element == ARCANE {
            assert_eq!(rail_weapons, 1, "{name}");
        }
        assert!(
            npc_hp(&s, front) < hp,
            "{name}: goblin hp {hp} -> {}",
            npc_hp(&s, front)
        );
    }
}

/// ProjectileSpell.CastWeapon + AnimationEnd: an Earth-enchanted swing
/// launches an UnderGroundAttack that reaches a goblin beyond sword range.
#[test]
fn earth_weapon_enchantment_launches_an_underground_attack() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, EARTH);
    // CastState.OnEnter: enchanting (queued elements) plays cast_sword; a
    // charged weapon with an empty queue plays the spell-type swing,
    // cast_sword_projectile for Earth (feedback 2026-10-09).
    let mut clips = Vec::new();
    let note = |s: &PlaySession, clips: &mut Vec<u16>| {
        let a = s.avatar().motion.controller.current_animation;
        if (59..=64).contains(&a) && clips.last() != Some(&a) {
            clips.push(a);
        }
    };
    for i in 0..90 {
        frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                move_down: i == 0,
                ..Default::default()
            },
        );
        note(&s, &mut clips);
    }
    let target = spawn_in_front(&mut s, &root, "Goblin_miner", 5.0);
    let hp = npc_hp(&s, target);
    let (mut launched, mut peak) = (0, 0);
    let (mut knocked, mut reached) = (false, false);
    let mut trail_sprites = std::collections::BTreeSet::new();
    let mut trail_positions = Vec::new();
    let mut trail_active = false;
    let mut wave_positions = Vec::new();
    let mut unsupported = Vec::new();
    for i in 0..120 {
        for event in frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                move_down: i < 30,
                ..Default::default()
            },
        ) {
            match event {
                SessionEvent::Avatar(AvatarEvent::UndergroundAttack { .. }) => launched += 1,
                SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(r)) => unsupported.push(r),
                _ => {}
            }
        }
        note(&s, &mut clips);
        peak = peak.max(s.world.underground.len());
        wave_positions.extend(s.world.underground.iter().map(|(_, a)| a.position));
        trail_active |=
            s.effects
                .active_effects_by_hash(magicka_runtime::scene_loading::custom_hash(
                    "underground_earth",
                ))
                > 0;
        for p in s.effects.particles.active() {
            if p.normalized_lifetime < 1.0 && [161, 7, 197].contains(&p.sprite) {
                trail_sprites.insert(p.sprite);
                if p.sprite == 161 {
                    trail_positions.push(p.position);
                }
            }
        }
        reached |= s
            .world
            .underground
            .iter()
            .any(|(_, a)| a.hits.contains(&target));
        knocked |= s
            .world
            .npcs
            .get(&target)
            .is_some_and(|n| n.character.combat.motion.knocked_down);
    }
    assert_eq!(clips, [59, 61], "cast_sword then cast_sword_projectile");
    assert!(unsupported.is_empty(), "{unsupported:?}");
    assert_eq!(launched, 1);
    assert_eq!(peak, 1, "attack never followed the ground");
    assert!(s.world.underground.is_empty(), "attack never ended");
    assert!(trail_active, "underground_earth never started");
    assert_eq!(trail_sprites, [7, 161, 197].into_iter().collect());
    let spread = (0..3)
        .map(|axis| {
            let min = trail_positions
                .iter()
                .map(|p| p[axis])
                .fold(f32::INFINITY, f32::min);
            let max = trail_positions
                .iter()
                .map(|p| p[axis])
                .fold(f32::NEG_INFINITY, f32::max);
            max - min
        })
        .fold(0.0, f32::max);
    let wave_span = (0..3)
        .map(|axis| {
            let min = wave_positions
                .iter()
                .map(|p| p[axis])
                .fold(f32::INFINITY, f32::min);
            let max = wave_positions
                .iter()
                .map(|p| p[axis])
                .fold(f32::NEG_INFINITY, f32::max);
            max - min
        })
        .fold(0.0, f32::max);
    // Havindr geometry can stop the wave before its nominal 15 m range.
    // The native dust emitter has horizontal offsets of ±0.2 m. Compare
    // against actual travel, allowing those offsets at both ends.
    assert!(
        wave_span > 0.4,
        "fixture wave never moved beyond its emitter offsets"
    );
    assert!(
        spread >= wave_span - 0.4,
        "dust span {spread}, wave span {wave_span}"
    );
    eprintln!(
        "Earth weapon trail: sprites {trail_sprites:?}, dust span {spread}, wave span {wave_span}"
    );
    assert_eq!(
        s.effects
            .active_effects_by_hash(magicka_runtime::scene_loading::custom_hash(
                "underground_earth"
            ),),
        0,
        "underground effect kept emitting after removal",
    );
    // Projectile/Weapon damage for pure Earth is a Knockdown (amount 0):
    // the wave knocks the goblin over without hurting it.
    assert!(reached, "the wave never reached the goblin");
    assert!(knocked, "goblin was not knocked down");
    assert_eq!(npc_hp(&s, target), hp);
}

#[test]
fn wizard_fire_weapon_release_consumes_enchantment_and_hits_forward_arc_once() {
    use magicka_sim::damage::CastType;
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, FIRE);
    for i in 0..90 {
        frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                move_down: i == 0,
                ..Default::default()
            },
        );
    }
    assert!(s
        .avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1
        .spells
        .charged());
    let mut releases = Vec::new();
    let front = spawn_in_front(&mut s, &root, "Goblin_miner", 2.0);
    let behind = spawn_in_front(&mut s, &root, "Goblin_miner", -2.0);
    let front_hp = npc_hp(&s, front);
    let behind_hp = npc_hp(&s, behind);
    let mut arcs = 0;
    let mut starts = 0;
    let mut stops = 0;
    let mut unsupported = Vec::new();
    for i in 0..90 {
        let events = frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                move_down: i < 30,
                ..Default::default()
            },
        );
        for event in events {
            match &event {
                SessionEvent::Avatar(AvatarEvent::SprayWeapon { .. }) => arcs += 1,
                SessionEvent::Avatar(AvatarEvent::SelfEffectStarted {
                    name: "weapon_spray_fire",
                    ..
                }) => starts += 1,
                SessionEvent::Avatar(AvatarEvent::SelfEffectsStopped) => stops += 1,
                SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(reason)) => {
                    unsupported.push(reason.clone())
                }
                _ => {}
            }
            if let SessionEvent::Avatar(AvatarEvent::SpellCast {
                spell,
                cast_type: CastType::Weapon,
                ..
            }) = event
            {
                releases.push(spell);
            }
        }
    }
    assert_eq!(releases.len(), 1);
    assert_eq!(arcs, 1);
    assert_eq!(starts, 1);
    assert_eq!(stops, 1);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    assert!(
        npc_hp(&s, front) < front_hp,
        "weapon arc must damage the forward target"
    );
    assert_eq!(
        npc_hp(&s, behind),
        behind_hp,
        "weapon arc must exclude the rear target"
    );
    assert_eq!(releases[0].element, E::FIRE);
    assert_eq!(releases[0].get(E::FIRE), 1.0);
    assert!(!s
        .avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1
        .spells
        .charged());
}

#[test]
fn weapon_spray_arc_reaches_spell_mines_and_excludes_the_rear() {
    use magicka_sim::{spell_mine::SpellMine, spells::Spell, spray_weapon::SprayWeaponPlan};
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let owner = s.avatar().handle;
    let origin = s.body().position;
    let direction = s.body().direction();
    let mine_spell = Spell::default_for(E::SHIELD) + Spell::default_for(E::ARCANE);
    let mut slots = Vec::new();
    for distance in [2.0, -2.0] {
        let hit = std::array::from_fn(|i| origin[i] + direction[i] * distance);
        slots.push(
            s.world.mines.add(
                SpellMine::initialize(
                    owner,
                    hit,
                    direction,
                    1.0,
                    0.0,
                    direction,
                    0.0,
                    1.0,
                    mine_spell,
                    s.play_time,
                    1.0,
                    0.0,
                )
                .unwrap(),
            ),
        );
    }
    let plan = SprayWeaponPlan::new(&Spell::default_for(E::FIRE), false).unwrap();
    assert!(s
        .world
        .spray_weapon(owner, origin, direction, origin, s.play_time, &plan));
    assert_eq!(s.world.mines.get(slots[0]).unwrap().detonation_timer, 0.1);
    assert_eq!(s.world.mines.get(slots[1]).unwrap().detonation_timer, 0.0);
}

#[test]
fn shift_cold_area_freezes_retained_water_through_casting_input() {
    use magicka_content::level::TriMesh;
    use magicka_runtime::water_collision::WaterCollision;
    let Some(root) = content() else { return };
    let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    session.disable_scripts();
    for _ in 0..60 {
        frame(&mut session, PlayerInput::default());
    }
    let origin = session.world.physics.bodies[session.avatar().body].position;
    let geometry = TriMesh {
        vertices: vec![
            [origin[0] + 0.5, origin[1] - 0.5, origin[2]],
            [origin[0] + 1.0, origin[1] - 0.5, origin[2]],
            [origin[0] + 0.5, origin[1] - 0.5, origin[2] + 0.25],
        ],
        triangles: vec![[0, 2, 1]],
    };
    session
        .world
        .collision
        .collision_mut()
        .water
        .push(WaterCollision::from_geometry(&geometry, true, false).unwrap());
    let index = session.world.collision.collision().water.len() - 1;
    let cold_key = magicka_runtime::avatar::ELEMENT_KEYS
        .iter()
        .position(|&e| e == E::COLD)
        .unwrap();
    conjure(&mut session, cold_key);
    let mut casts = 0;
    for i in 0..60 {
        casts += frame(
            &mut session,
            PlayerInput {
                cast_down: i < 20,
                shift_down: i < 20,
                ..PlayerInput::default()
            },
        )
        .iter()
        .filter(|e| matches!(e, SessionEvent::Avatar(AvatarEvent::SprayArea { .. })))
        .count();
    }
    assert_eq!(casts, 1);
    let water = &session.world.collision.collision().water[index];
    assert!(water.freeze.vertices.iter().any(|&value| value >= 0.5));
    assert!(water.freeze.triangle_is_ice([0, 2, 1]));
}

/// GroundBlast → BlastDamage ends in Liquid.Freeze(position, Right·radius,
/// 2π, 1): an Earth+Cold area freezes nearby water (feedback 2026-10-09:
/// every Cold AoE should freeze water and lava).
#[test]
fn earth_cold_ground_area_freezes_nearby_water() {
    use magicka_content::level::TriMesh;
    use magicka_runtime::water_collision::WaterCollision;
    let Some(root) = content() else { return };
    let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    session.disable_scripts();
    for _ in 0..60 {
        frame(&mut session, PlayerInput::default());
    }
    let origin = session.world.physics.bodies[session.avatar().body].position;
    let geometry = TriMesh {
        vertices: vec![
            [origin[0] + 0.5, origin[1] - 0.5, origin[2]],
            [origin[0] + 1.0, origin[1] - 0.5, origin[2]],
            [origin[0] + 0.5, origin[1] - 0.5, origin[2] + 0.25],
        ],
        triangles: vec![[0, 2, 1]],
    };
    session
        .world
        .collision
        .collision_mut()
        .water
        .push(WaterCollision::from_geometry(&geometry, true, false).unwrap());
    let index = session.world.collision.collision().water.len() - 1;
    let key = |e| {
        magicka_runtime::avatar::ELEMENT_KEYS
            .iter()
            .position(|&k| k == e)
            .unwrap()
    };
    conjure(&mut session, key(E::EARTH));
    conjure(&mut session, key(E::COLD));
    let mut areas = 0;
    for i in 0..90 {
        areas += frame(
            &mut session,
            PlayerInput {
                cast_down: i < 20,
                shift_down: i < 20,
                ..PlayerInput::default()
            },
        )
        .iter()
        .filter(|e| matches!(e, SessionEvent::Avatar(AvatarEvent::GroundArea { .. })))
        .count();
    }
    assert_eq!(areas, 1);
    let water = &session.world.collision.collision().water[index];
    assert!(water.freeze.vertices.iter().any(|&value| value > 0.0));
}

/// SpraySpell.CastUpdate (Force): Liquid.Freeze(position, direction·range,
/// π/10, dt) every update, so a held Cold spray freezes water ahead
/// (feedback 2026-10-08: only Area freezing worked).
#[test]
fn cold_force_spray_freezes_water_ahead() {
    use magicka_content::level::TriMesh;
    use magicka_runtime::water_collision::WaterCollision;
    let Some(root) = content() else { return };
    let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    session.disable_scripts();
    for _ in 0..60 {
        frame(&mut session, PlayerInput::default());
    }
    let body = &session.world.physics.bodies[session.avatar().body];
    let (p, f) = (body.position, body.direction());
    let at = |d: f32, side: f32| {
        [
            p[0] + f[0] * d - f[2] * side,
            p[1] - 0.5,
            p[2] + f[2] * d + f[0] * side,
        ]
    };
    let geometry = TriMesh {
        vertices: vec![at(2.0, -0.2), at(2.0, 0.2), at(2.5, 0.0)],
        triangles: vec![[0, 1, 2]],
    };
    session
        .world
        .collision
        .collision_mut()
        .water
        .push(WaterCollision::from_geometry(&geometry, true, false).unwrap());
    let index = session.world.collision.collision().water.len() - 1;
    let cold_key = magicka_runtime::avatar::ELEMENT_KEYS
        .iter()
        .position(|&e| e == E::COLD)
        .unwrap();
    conjure(&mut session, cold_key);
    for i in 0..90 {
        frame(
            &mut session,
            PlayerInput {
                cast_down: i < 60,
                ..PlayerInput::default()
            },
        );
    }
    let water = &session.world.collision.collision().water[index];
    assert!(
        water.freeze.vertices.iter().any(|&v| v >= 0.5),
        "{:?}",
        water.freeze.vertices
    );
    assert!(water.freeze.triangle_is_ice([0, 1, 2]));
}

#[test]
fn live_shields_clip_liquid_freezing_before_vertex_selection() {
    use magicka_content::level::TriMesh;
    use magicka_runtime::water_collision::WaterCollision;
    use magicka_sim::shield_entity::{ShieldKind, SHIELD_COLOR};
    let Some(root) = content() else { return };
    let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    session.disable_scripts();
    let geometry = TriMesh {
        vertices: vec![[0.5, 0.0, 0.0], [3.0, 0.0, 0.0], [0.5, 0.0, 0.1]],
        triangles: vec![[0, 2, 1]],
    };
    let world = &mut session.world;
    world.collision.collision_mut().water =
        vec![WaterCollision::from_geometry(&geometry, true, false).unwrap()];
    let damages = [magicka_sim::damage::Damage {
        element: E::COLD.0,
        magnitude: 1.0,
        ..Default::default()
    }];
    for kind in [ShieldKind::Sphere, ShieldKind::Disc, ShieldKind::Wall] {
        world.shields.clear();
        world.spawn_shield(
            0,
            [1.5, 7.0, 0.0],
            0.8,
            if kind == ShieldKind::Wall {
                [0.0, 0.0, -1.0]
            } else {
                [-1.0, 0.0, 0.0]
            },
            kind,
            5000.0,
            SHIELD_COLOR,
        );
        // A dying shield remains in EntityManager.Shields until faded away.
        world.shields[0].shield.kill();
        world.collision.collision_mut().water[0]
            .freeze
            .vertices
            .fill(0.0);
        world.freeze_water([0.0, 7.0, 0.0], [4.0, 0.0, 0.0], 1.0, 1.0, &damages);
        let values = &world.collision.collision().water[0].freeze.vertices;
        assert!(values[0] > 0.0, "near vertex behind {kind:?}: {values:?}");
        assert_eq!(values[1], 0.0, "far vertex behind {kind:?}: {values:?}");
    }
    world.shields.clear();
    world.freeze_water([0.0, 7.0, 0.0], [4.0, 0.0, 0.0], 1.0, 1.0, &damages);
    assert!(world.collision.collision().water[0].freeze.vertices[1] > 0.0);
}

#[test]
fn shift_fire_area_cast_reaches_live_scene_once() {
    let Some(root) = content() else { return };
    let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    session.disable_scripts();
    for _ in 0..60 {
        frame(&mut session, PlayerInput::default());
    }
    let target = spawn_in_front(&mut session, &root, "Goblin_miner", 2.0);
    let hp = npc_hp(&session, target);
    let owner_hp = session.avatar().character.combat.health.hit_points;
    conjure(&mut session, FIRE);
    let mut casts = 0;
    let mut peak_particles_after_cast = 0;
    let mut peak_effects_after_cast = 0;
    for i in 0..90 {
        let events = frame(
            &mut session,
            PlayerInput {
                cast_down: i < 20,
                shift_down: i < 20,
                ..PlayerInput::default()
            },
        );
        casts += events
            .iter()
            .filter(|e| matches!(e, SessionEvent::Avatar(AvatarEvent::SprayArea { .. })))
            .count();
        if casts > 0 {
            peak_particles_after_cast =
                peak_particles_after_cast.max(session.effects.particles.active().len());
            peak_effects_after_cast = peak_effects_after_cast.max(session.effects.active_effects());
        }
        assert!(!events.iter().any(|e| matches!(
            e,
            SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(_))
        )));
    }
    assert_eq!(casts, 1);
    assert!(peak_effects_after_cast > 0, "Area effect never started");
    assert!(
        peak_particles_after_cast > 0,
        "Area effect emitted no particles"
    );
    assert!(npc_hp(&session, target) < hp);
    assert_eq!(
        session.avatar().character.combat.health.hit_points,
        owner_hp
    );
    use magicka_runtime::audio::{AudioRequest, SoundSource};
    let keys = cue_keys(&session, "spell_fire_area");
    assert_eq!(keys.len(), 1, "one Area cue for one Fire element");
    let key = keys[0];
    let start = session
        .audio
        .iter()
        .position(|(request, _)| {
            matches!(request, AudioRequest::Start {
            key: k, magnitude: Some(1.0), source: SoundSource::Emitter(_), ..
        } if *k == key)
        })
        .expect("Area cue must carry Magnitude and the owner's emitter");
    let stops: Vec<_> = session
        .audio
        .iter()
        .enumerate()
        .filter_map(|(i, (request, _))| {
            matches!(request, AudioRequest::Stop { key: k, instant: false } if *k == key)
                .then_some(i)
        })
        .collect();
    assert_eq!(stops.len(), 1, "deinitialization releases the cue once");
    assert!(start < stops[0]);
    assert!(cue_keys(&session, "spell_fire_spray").is_empty());
}

/// Supply an explicit nonzero pooled private TTL to test cue retention across
/// frames. This is a lifecycle fixture, not a retail runtime capture.
#[test]
fn spray_area_audio_keeps_fractional_magnitudes_and_releases_owned_cues() {
    use magicka_runtime::audio::{AudioRequest, SoundSource};
    use magicka_runtime::avatar::SpellEffect;
    use magicka_runtime::scene_loading::custom_hash;
    use magicka_sim::{damage::CastType, spells::Spell, spray_area};
    let Some(root) = content() else { return };
    let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    session.disable_scripts();
    for _ in 0..60 {
        frame(&mut session, PlayerInput::default());
    }
    session.audio.clear();
    let mut spell = Spell::default_for(E::FIRE);
    spell.element |= E::POISON;
    spell.mag[E::FIRE.index() as usize] = 0.75;
    spell.mag[E::POISON.index() as usize] = 0.25;
    let position = session.world.physics.bodies[session.avatar().body].position;
    let cast = AvatarEvent::SprayArea {
        spell,
        position,
        cast_source: position,
        timestamp: session.play_time,
        plan: spray_area::plan(&spell, false).unwrap(),
    };
    let player = session.world.players[0].as_mut().unwrap();
    player.cast.cast_type = CastType::Area;
    player.cast.pressed = CastType::Area;
    player.current_spell = Some(SpellEffect::SprayArea(spray_area::SprayArea::begin(
        false,
        session.play_time,
        1.0,
    )));
    player.events.push(cast.clone());
    session.step(DT);
    let starts: Vec<_> = session
        .audio
        .iter()
        .filter_map(|(request, _)| match request {
            AudioRequest::Start {
                key,
                cue,
                magnitude,
                source,
                ..
            } => Some((*key, *cue, *magnitude, *source)),
            _ => None,
        })
        .collect();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[0].1, custom_hash("spell_fire_area"));
    assert_eq!(starts[0].2, Some(0.75));
    assert_eq!(starts[1].1, custom_hash("spell_poison_area"));
    assert_eq!(starts[1].2, Some(0.25));
    assert!(starts.iter().all(|s| s.3 == SoundSource::Emitter(position)));
    for _ in 0..3 {
        session.step(DT);
    }
    assert!(session.avatar().current_spell.as_ref().unwrap().active());
    assert!(!session
        .audio
        .iter()
        .any(|(r, _)| matches!(r, AudioRequest::Stop { .. })));
    assert_eq!(cue_keys(&session, "spell_fire_area").len(), 1);

    // Starting again stops the old effect-owned cue list before new cues.
    let boundary = session.audio.len();
    session.world.players[0].as_mut().unwrap().events.push(cast);
    session.step(DT);
    let replacement: Vec<_> = session.audio[boundary..].iter().map(|(r, _)| r).collect();
    assert!(matches!(replacement[0], AudioRequest::Stop { key, .. } if *key == starts[0].0));
    assert!(matches!(replacement[1], AudioRequest::Stop { key, .. } if *key == starts[1].0));
    assert!(matches!(replacement[2], AudioRequest::Start { .. }));
    assert!(matches!(replacement[3], AudioRequest::Start { .. }));
    let new_keys = cue_keys(&session, "spell_fire_area");
    assert_eq!(new_keys.len(), 2);

    // Expiration must release cues even while current_spell is still Some.
    let player = session.world.players[0].as_mut().unwrap();
    let Some(SpellEffect::SprayArea(area)) = player.current_spell.as_mut() else {
        panic!("Area effect unexpectedly ended");
    };
    area.private_ttl = 0.0;
    session.step(DT);
    assert!(session.avatar().current_spell.is_some());
    assert!(!session.avatar().current_spell.as_ref().unwrap().active());
    for key in new_keys {
        assert_eq!(
            session
                .audio
                .iter()
                .filter(|(r, _)| {
                    matches!(r, AudioRequest::Stop { key: k, instant: false } if *k == key)
                })
                .count(),
            1
        );
    }
}

#[test]
fn spray_area_audio_skips_missing_sound_list_entries() {
    use magicka_runtime::audio::AudioRequest;
    use magicka_runtime::avatar::SpellEffect;
    use magicka_runtime::play_content::FsSceneContent;
    use magicka_runtime::scene_world::SceneWorld;
    use magicka_sim::{damage::CastType, spells::Spell, spray_area};
    let Some(root) = content() else { return };
    let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    session.disable_scripts();
    for _ in 0..60 {
        frame(&mut session, PlayerInput::default());
    }
    // Preserve the decoded world/avatar fixture but supply an empty catalog.
    // Nothing is written into the original Content directory.
    let content = FsSceneContent::new(&root.join("__synthetic_missing_audio_catalog__"));
    let mut world = SceneWorld::new(session.world.level.clone(), content, 123).unwrap();
    world.physics.bodies = session.world.physics.bodies.clone();
    let mut player = session.world.players[0].take().unwrap();
    let position = world.physics.bodies[player.body].position;
    let spell = Spell::default_for(E::FIRE);
    player.cast.cast_type = CastType::Area;
    player.cast.pressed = CastType::Area;
    player.current_spell = Some(SpellEffect::SprayArea(spray_area::SprayArea::begin(
        false,
        session.play_time,
        1.0,
    )));
    player.events.push(AvatarEvent::SprayArea {
        spell,
        position,
        cast_source: position,
        timestamp: session.play_time,
        plan: spray_area::plan(&spell, false).unwrap(),
    });
    world.players[0] = Some(player);
    world.update_players(0.0);
    assert!(!world
        .audio_requests
        .iter()
        .any(|r| matches!(r, AudioRequest::Start { .. } | AudioRequest::Stop { .. })));
}

#[test]
fn scene_spray_area_greased_owner_damage_matches_single_payload_route() {
    use magicka_sim::{damage_delivery::DamagePayload, spells::Spell, spray_area};
    let Some(root) = content() else { return };
    let plan = spray_area::plan(&Spell::default_for_list(&[E::FIRE]), true).unwrap();
    let fire = plan.greased_owner_damage.unwrap();
    let mut outcomes = Vec::new();
    for via_area in [false, true] {
        let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
        session.disable_scripts();
        for _ in 0..60 {
            frame(&mut session, PlayerInput::default());
        }
        let owner = session.avatar().handle;
        let body_index = session.avatar().body;
        let position = session.world.physics.bodies[body_index].position;
        let source = [position[0] + 0.5, position[1], position[2]];
        let before = session.avatar().character.combat.health.hit_points;
        if via_area {
            assert!(session
                .world
                .spray_area(owner, position, source, 1.0, &plan));
        } else {
            let player = session.world.players[0].as_mut().unwrap();
            player.receive_payload(
                &mut session.world.physics.bodies[body_index],
                DamagePayload::Single(fire),
                Some(owner),
                source,
                1.0,
            );
        }
        let after = session.avatar().character.combat.health.hit_points;
        assert_eq!(after, before);
        assert!(session
            .avatar()
            .character
            .combat
            .statuses
            .has(magicka_sim::status::Status::Burning));
        assert_eq!(
            session.world.physics.bodies[body_index].owner.hit_points,
            after
        );
        outcomes.push((
            after,
            session.avatar().character.combat.statuses.current_mask,
        ));
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn scene_spray_area_damages_npc_excludes_owner_and_queues_radial_visual() {
    let Some(root) = content() else { return };
    let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    session.disable_scripts();
    for _ in 0..60 {
        frame(&mut session, PlayerInput::default());
    }
    let target = spawn_in_front(&mut session, &root, "Goblin_miner", 2.0);
    let hp = npc_hp(&session, target);
    let owner = session.avatar().handle;
    let owner_hp = session.avatar().character.combat.health.hit_points;
    let position = session.world.physics.bodies[session.avatar().body].position;
    let plan = magicka_sim::spray_area::plan(
        &magicka_sim::spells::Spell::default_for_list(&[E::FIRE]),
        false,
    )
    .unwrap();
    assert!(session
        .world
        .spray_area(owner, position, position, 1.0, &plan));
    assert!(npc_hp(&session, target) < hp);
    assert_eq!(
        session.avatar().character.combat.health.hit_points,
        owner_hp
    );
    assert!(
        matches!(session.world.missile_visuals.last(), Some(magicka_runtime::missiles::MissileVisual::Blast { damages, .. }) if *damages == plan.damages)
    );
    assert!(session.world.beam_blasts.is_empty());
}

#[test]
fn earth_fire_area_blast_damages_nearby_npc_but_excludes_caster_and_levitation() {
    let Some(root) = content() else { return };
    for levitating in [false, true] {
        let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
        session.disable_scripts();
        for _ in 0..60 {
            frame(&mut session, PlayerInput::default());
        }
        let target = spawn_in_front(&mut session, &root, "Goblin_miner", 2.0);
        if levitating {
            // Levitate.CastLevitate on the goblin: IsLevitating for 30 s.
            assert!(session.world.levitate(target));
        }
        let npc_hp = session.world.npcs[&target]
            .character
            .combat
            .health
            .hit_points;
        let caster_hp = session.avatar().character.combat.health.hit_points;
        let mut fire = PlayerInput::default();
        fire.elements[FIRE] = true;
        frame(&mut session, fire);
        frame(&mut session, PlayerInput::default());
        let mut input = PlayerInput::default();
        input.elements[6] = true;
        frame(&mut session, input);
        frame(&mut session, PlayerInput::default());
        let mut cast = false;
        for _ in 0..90 {
            cast |= frame(
                &mut session,
                PlayerInput {
                    cast_down: true,
                    shift_down: true,
                    ..PlayerInput::default()
                },
            )
            .iter()
            .any(|event| matches!(event, SessionEvent::Avatar(AvatarEvent::GroundArea { .. })));
            if cast {
                break;
            }
        }
        assert!(cast);
        let after = session.world.npcs[&target]
            .character
            .combat
            .health
            .hit_points;
        if levitating {
            assert_eq!(after, npc_hp);
        } else {
            assert!(after < npc_hp);
        }
        assert_eq!(
            session.avatar().character.combat.health.hit_points,
            caster_hp
        );
        assert!(session.world.missiles.is_empty());
    }
}

#[test]
fn earth_self_cast_launches_one_falling_live_rock() {
    let Some(root) = content() else { return };
    let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    session.disable_scripts();
    for _ in 0..60 {
        frame(&mut session, PlayerInput::default());
    }
    let mut input = PlayerInput::default();
    input.elements[6] = true;
    frame(&mut session, input);
    frame(&mut session, PlayerInput::default());
    let mut launched = 0;
    let mut falling = false;
    for _ in 0..120 {
        for event in frame(
            &mut session,
            PlayerInput {
                cast_self_down: true,
                ..PlayerInput::default()
            },
        ) {
            if let SessionEvent::Avatar(AvatarEvent::MissileLaunched(launch)) = event {
                launched += 1;
                assert!(launch.velocity[1] < 0.0);
                assert!(launch.position[1] > session.body().position[1] + 7.0);
            }
        }
        falling |= session
            .world
            .missiles
            .iter()
            .any(|m| m.body.velocity[1] < 0.0);
    }
    assert_eq!(launched, 1);
    assert!(falling);
}

#[test]
fn live_character_pairs_separate_and_ethereal_npcs_pass_through() {
    let Some(root) = content() else { return };
    for ethereal in [false, true] {
        let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
        session.disable_scripts();
        let handle = spawn_in_front(&mut session, &root, "Goblin_miner", 5.0);
        let player_body = session.avatar().body;
        let npc = session.world.npcs.get_mut(&handle).unwrap();
        npc.character.ethereal = ethereal;
        let npc_body = npc.body;
        for (index, x) in [(player_body, 0.0), (npc_body, 0.4)] {
            let body = &mut session.world.physics.bodies[index];
            body.position = [x, 100.0, 0.0];
            body.velocity = [0.0; 3];
            body.apply_gravity = false;
        }
        session.step(DT);
        let a = &session.world.physics.bodies[player_body];
        let b = &session.world.physics.bodies[npc_body];
        if ethereal {
            assert!((b.position[0] - a.position[0] - 0.4).abs() < 1e-6);
            assert_eq!(a.velocity[0], 0.0);
            assert_eq!(b.velocity[0], 0.0);
        } else {
            assert!(b.position[0] - a.position[0] > 0.4);
            assert!(a.velocity[0] < 0.0 && b.velocity[0] > 0.0);
            assert!((a.mass * a.velocity[0] + b.mass * b.velocity[0]).abs() < 0.001);
        }
    }
}

#[test]
fn dialog_select_hurries_then_advances_once_and_respects_cutscenes() {
    use magicka_runtime::{
        dialog_definition::DialogDefinition, dialog_manager::DialogManager,
        interaction_playback::InteractionLocation, scene_loading::custom_hash,
    };
    let Some(root) = content() else { return };
    let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    session.disable_scripts();
    let definitions = DialogDefinition::read_collection(&[br##"<Dialogs>
      <Dialog id="inputtest" repeatFrom="-1"><Interact>
        <Message><Text>#inputone</Text></Message>
        <Message><Text>#inputtwo</Text></Message>
      </Interact></Dialog></Dialogs>"##])
    .unwrap();
    let host = session.world.dialog_text.as_mut().unwrap();
    host.strings
        .strings
        .insert(custom_hash("#inputone"), "First".into());
    host.strings
        .strings
        .insert(custom_hash("#inputtwo"), "Second".into());
    host.initialize_layouts(&definitions).unwrap();
    let mut manager = DialogManager::new(definitions).unwrap();
    manager
        .start(
            custom_hash("inputtest"),
            InteractionLocation::Screen([0.0; 2]),
            Some(0),
            false,
            host,
        )
        .unwrap();
    manager.tick_input(0.3);
    session.world.dialogs = Some(manager);
    session.world.flags.cutscene = true;
    session.handle_input_with_dialog_select(PlayerInput::default(), 1.5, true);
    assert!(session.world.dialog_text.as_ref().unwrap().boxes[0]
        .state
        .animating());
    session.world.flags.cutscene = false;
    session.handle_input_with_dialog_select(PlayerInput::default(), 1.5, true);
    let state = &session.world.dialog_text.as_ref().unwrap().boxes[0].state;
    assert!(!state.animating());
    assert!(!state.automatic_advance);
    assert_eq!(state.text.as_deref(), Some("First"));
    session.handle_input_with_dialog_select(
        PlayerInput {
            move_down: true,
            ..PlayerInput::default()
        },
        1.5,
        true,
    );
    let state = &session.world.dialog_text.as_ref().unwrap().boxes[0].state;
    assert_eq!(state.text.as_deref(), Some("Second"));
    assert!(
        state.animating(),
        "simultaneous click and Select must not hurry the next message"
    );
    session.handle_input(
        PlayerInput {
            move_down: true,
            ..PlayerInput::default()
        },
        1.5,
    );
    assert!(
        session.world.dialog_text.as_ref().unwrap().boxes[0]
            .state
            .animating(),
        "holding left mouse must not advance again"
    );
    assert!(
        session.world.dialog_failure.is_none(),
        "{:?}",
        session.world.dialog_failure
    );
}

#[test]
fn missile_blast_request_shakes_camera_on_next_world_update() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut shaken = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    let mut plain = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    for session in [&mut shaken, &mut plain] {
        session.disable_scripts();
        for _ in 0..60 {
            session.step(DT);
        }
    }
    shaken
        .world
        .missile_visuals
        .push(magicka_runtime::missiles::MissileVisual::Blast {
            position: shaken.body().position,
            radius: 6.0,
            damages: [magicka_sim::damage::Damage::default(); 5],
        });
    shaken.step(DT);
    plain.step(DT);
    assert_eq!(
        shaken.camera().ground_position(),
        plain.camera().ground_position()
    );
    shaken.step(DT);
    plain.step(DT);
    let delta = shaken.camera().ground_position()[1] - plain.camera().ground_position()[1];
    let expected = (0.3f32 * 60.0).sin() * (6.0 * 0.333) * 0.0333;
    assert!((delta - expected).abs() < 0.00001, "{delta} != {expected}");
    assert!(shaken.world.missile_visuals.is_empty());
}

#[test]
fn keyboard_hud_tracks_accepted_input_before_the_next_hud_update() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let mut input = PlayerInput::default();
    input.elements[FIRE] = true;
    frame(&mut s, input);
    // Input comes after the HUD's Update: the newly set cooldown is still
    // .125 and its rendering fields remain unchanged until the next frame.
    assert_eq!(s.keyboard_hud.icons[3].cooldown, 0.125);
    assert_eq!(s.keyboard_hud.icons[3].saturation, 1.);
    assert_eq!(s.avatar().conjure.queue.len(), 1);
    frame(&mut s, input);
    assert_eq!(s.keyboard_hud.icons[3].saturation, 0.5);
    assert!(s.keyboard_hud.icons[3].cooldown < 0.125);
    assert_eq!(
        s.avatar().conjure.queue.len(),
        1,
        "held key must not retrigger"
    );
    assert_eq!(s.keyboard_hud.icons[1].saturation, 1.);
}

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    root.is_dir().then_some(root)
}

fn frame(s: &mut PlaySession, input: PlayerInput) -> Vec<SessionEvent> {
    s.step(DT);
    s.handle_input(input, 16.0 / 9.0);
    std::mem::take(&mut s.events)
}

/// Spawn one idle NPC through the reconstructed SpawnAction at the second
/// player locator, then place it `distance` m in front of the avatar.
fn spawn_in_front(s: &mut PlaySession, root: &Path, template: &str, distance: f32) -> u16 {
    let area = load_start_scene(root, LEVEL).unwrap().start.area.unwrap();
    let xml = format!(
        r#"<Scene><trigger id="t"><then><spawn type="{template}" area="{area}1" order="idle"/></then></trigger></Scene>"#
    );
    let ActionDefinition::Spawn(action) = SceneScriptLayout::read(xml.as_bytes(), false)
        .unwrap()
        .actions
        .remove(0)
    else {
        panic!("not a spawn action");
    };
    let mut state = SpawnState::default();
    action.initialize(&mut state, &mut s.world).unwrap();
    action.execute(&mut state, &mut s.world).unwrap();
    let handle = state.spawned[0].handle.unwrap();
    let (position, forward) = (s.body().position, s.body().direction());
    let npc = &s.world.npcs[&handle];
    let body = &mut s.world.physics.bodies[npc.body];
    let target = [
        position[0] + forward[0] * distance,
        body.position[1].max(position[1]),
        position[2] + forward[2] * distance,
    ];
    let orientation = body.orientation;
    body.move_to(target, orientation);
    handle
}

#[test]
fn fire_spray_conjured_by_keys_damages_a_spawned_npc() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    // Ambient level effects share the effect/particle systems.
    s.stop_level_effects();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let npc = spawn_in_front(&mut s, &root, "Goblin_miner", 2.5);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let hp0 = s.world.npcs[&npc].character.combat.health.hit_points;
    assert!(hp0 > 0.0);

    // F, release, F: two queued Fire elements (MagickaElement 0.125 s
    // cooldown elapses between the presses).
    let fire = |down: bool| {
        let mut elements = [false; 8];
        elements[FIRE] = down;
        PlayerInput {
            elements,
            ..PlayerInput::default()
        }
    };
    for down in [
        true, false, false, false, false, false, false, false, false, true, false,
    ] {
        frame(&mut s, fire(down));
    }
    let queue = &s.avatar().conjure.queue;
    assert_eq!(queue.len(), 2, "{queue:?}");
    assert!(queue.iter().all(|spell| spell.element == E::FIRE));

    // Hold Cast (right mouse) for 1.5 s with no cursor: the avatar keeps its
    // facing towards the NPC.
    let cast = PlayerInput {
        cast_down: true,
        ..PlayerInput::default()
    };
    let mut events = Vec::new();
    let mut casting = false;
    let mut max_particles = 0;
    for _ in 0..90 {
        events.extend(frame(&mut s, cast));
        casting |= s.avatar().motion.state == AvatarState::Cast;
        max_particles = max_particles.max(s.effects.particles.active().len());
    }
    // spray_fire1 emits from the staff while the spray is active.
    assert!(max_particles > 50, "{max_particles} particles");
    assert!(casting, "never entered CastState");
    assert!(s.avatar().conjure.queue.is_empty(), "queue not combined");
    let avatar_events: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Avatar(a) => Some(a.clone()),
            _ => None,
        })
        .collect();
    let cast_spell = avatar_events.iter().find_map(|e| match e {
        AvatarEvent::SpellCast { spell, .. } => Some(*spell),
        _ => None,
    });
    let spell = cast_spell.expect("CastSpell action never fired");
    assert_eq!(spell.element, E::FIRE);
    assert!(
        avatar_events
            .iter()
            .any(|e| matches!(e, AvatarEvent::SprayStarted { .. })),
        "{avatar_events:?}"
    );
    assert!(
        avatar_events
            .iter()
            .any(|e| matches!(e, AvatarEvent::SprayHit { target } if *target == npc)),
        "spray never hit the NPC: {avatar_events:?}"
    );
    let scales: Vec<f32> = avatar_events
        .iter()
        .filter_map(|e| match e {
            AvatarEvent::SprayOrientation(m) => {
                Some((m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt())
            }
            _ => None,
        })
        .collect();
    eprintln!(
        "spray visual scales: first {:?} last {:?} (n={})",
        scales.first(),
        scales.last(),
        scales.len()
    );
    if let Some(AvatarEvent::SprayStarted {
        range,
        position,
        direction,
        ..
    }) = avatar_events
        .iter()
        .find(|e| matches!(e, AvatarEvent::SprayStarted { .. }))
    {
        eprintln!("spray range {range} from {position:?} towards {direction:?}");
    }
    let hp1 = s.world.npcs[&npc].character.combat.health.hit_points;
    assert!(hp1 < hp0, "NPC hit points {hp0} -> {hp1}");
    let hits = avatar_events
        .iter()
        .filter(|e| matches!(e, AvatarEvent::SprayHit { .. }))
        .count();
    let status = s.world.npcs[&npc].character.combat.statuses.current_mask;
    eprintln!("spell {spell:?}; {hits} spray hits; NPC hp {hp0} -> {hp1}; status mask {status:#x}; peak particles {max_particles}");

    // Release: the spray stops and CastState hands back to Idle.
    let mut stopped = false;
    let mut idle = false;
    for _ in 0..120 {
        for e in frame(&mut s, PlayerInput::default()) {
            stopped |= matches!(e, SessionEvent::Avatar(AvatarEvent::SprayStopped));
        }
        idle |= s.avatar().motion.state == AvatarState::Idle;
    }
    assert!(stopped, "spray never stopped");
    // DeInitialize releases scourge references without stopping the authored
    // ground-fire effect. Its native duration outlives the released spray.
    let scourges = s
        .effects
        .active_effects_by_hash(magicka_runtime::scene_loading::custom_hash("fire_scourge"));
    assert_eq!(scourges, 2);
    assert_eq!(
        s.effects.active_effects(),
        s.active_status_emitters() + scourges + fairy_effects(&s)
    );
    assert!(
        s.world
            .decals
            .frame
            .iter()
            .any(|p| p.args[0] == 4.0 && p.args[1] == 2.0 && p.color[3] > 0.0),
        "fire spray left scorch marks"
    );
    assert!(idle, "never returned to Idle");
    assert!(s.avatar().current_spell.is_none());
    let scourge_hash = magicka_runtime::scene_loading::custom_hash("fire_scourge");
    let duration = s.effects.library.effects[&scourge_hash].duration;
    assert_eq!(duration, 3.0, "decoded ground fire duration");
    for _ in 0..((duration / DT).ceil() as usize + 1) {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(
        s.effects.active_effects_by_hash(scourge_hash),
        0,
        "released scourges finish without retained references"
    );
    assert!(
        s.world
            .decals
            .frame
            .iter()
            .any(|p| p.args[0] == 4.0 && p.args[1] == 2.0 && p.args[2] > 0.0),
        "scorch outlives ground fire"
    );
}

#[test]
fn repeated_fire_sprays_kill_the_npc() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let npc = spawn_in_front(&mut s, &root, "Goblin_miner", 2.5);
    let fire = |down: bool| {
        let mut elements = [false; 8];
        elements[FIRE] = down;
        PlayerInput {
            elements,
            ..PlayerInput::default()
        }
    };
    let mut casts = 0;
    while !s.world.npcs.get(&npc).is_none_or(|n| n.dead()) && casts < 12 {
        for down in [
            true, false, false, false, false, false, false, false, false, true, false,
        ] {
            frame(&mut s, fire(down));
        }
        for _ in 0..120 {
            frame(
                &mut s,
                PlayerInput {
                    cast_down: true,
                    ..PlayerInput::default()
                },
            );
        }
        for _ in 0..30 {
            frame(&mut s, PlayerInput::default());
        }
        casts += 1;
        let n = s.world.npcs.get(&npc);
        eprintln!(
            "cast {casts}: hp {:?} dead {:?} state {:?}",
            n.map(|n| n.character.combat.health.hit_points),
            n.map(|n| n.dead()),
            n.map(|n| n.motion.state)
        );
    }
    let entity = s.world.entities.iter().find(|e| e.handle == npc).unwrap();
    assert!(
        entity.dead || s.world.npcs.get(&npc).is_none_or(|n| n.dead()),
        "goblin survived {casts} sprays"
    );
}

/// Spawn an NPC with an explicit order (and reaction) via SpawnAction.
fn spawn_ordered(
    s: &mut PlaySession,
    root: &Path,
    template: &str,
    order: &str,
    distance: f32,
) -> u16 {
    let area = load_start_scene(root, LEVEL).unwrap().start.area.unwrap();
    let xml = format!(
        r#"<Scene><trigger id="t"><then><spawn type="{template}" area="{area}1" order="{order}" reactTo="proximity"/></then></trigger></Scene>"#
    );
    let ActionDefinition::Spawn(action) = SceneScriptLayout::read(xml.as_bytes(), false)
        .unwrap()
        .actions
        .remove(0)
    else {
        panic!("not a spawn action");
    };
    let mut state = SpawnState::default();
    action.initialize(&mut state, &mut s.world).unwrap();
    action.execute(&mut state, &mut s.world).unwrap();
    let handle = state.spawned[0].handle.unwrap();
    let (position, forward) = (s.body().position, s.body().direction());
    let npc = &s.world.npcs[&handle];
    let body = &mut s.world.physics.bodies[npc.body];
    let target = [
        position[0] + forward[0] * distance,
        body.position[1].max(position[1]),
        position[2] + forward[2] * distance,
    ];
    let orientation = body.orientation;
    body.move_to(target, orientation);
    handle
}

#[test]
// Goblin_pirate: its attack_dagger DealDamage window (0.50-0.60) covers the
// stab's reach peak (~0.55). Goblin_miner's window for the same clip is
// 0.09-0.14 in the retail data, so its stab misses at range (faithful).
fn attacking_goblin_closes_in_and_hits_the_wizard() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let npc = spawn_ordered(&mut s, &root, "Goblin_pirate", "attack", 6.0);
    let n = &s.world.npcs[&npc];
    eprintln!(
        "abilities {:?}; weapons {}",
        n.combat
            .abilities
            .iter()
            .map(|a| format!("{:?}", a.body).chars().take(40).collect::<String>())
            .collect::<Vec<_>>(),
        n.weapons.len()
    );
    let hp0 = s.avatar().character.combat.health.hit_points;
    let start = s.world.physics.bodies[s.world.npcs[&npc].body].position;
    let (mut attacked, mut targeted, mut closest) = (false, false, f32::MAX);
    for _ in 0..600 {
        frame(&mut s, PlayerInput::default());
        let n = &s.world.npcs[&npc];
        targeted |= n.combat.current_target() == Some(s.avatar().handle);
        attacked |= n.motion.state == magicka_runtime::character_motion::CharacterState::Attack;
        let p = s.world.physics.bodies[n.body].position;
        let a = s.body().position;
        closest = closest.min(((p[0] - a[0]).powi(2) + (p[2] - a[2]).powi(2)).sqrt());
    }
    let hp1 = s.avatar().character.combat.health.hit_points;
    eprintln!("goblin from {start:?}: closest {closest:.2} m; wizard hp {hp0} -> {hp1}");
    assert!(targeted, "never targeted the wizard");
    assert!(closest < 2.5, "never closed in ({closest})");
    assert!(attacked, "never entered AttackState");
    assert!(hp1 < hp0, "wizard took no melee damage");
    let shown = s.avatar().character.normalized_hit_points;
    assert!(
        shown < 0.99,
        "displayed health never followed melee damage: {shown}"
    );
    assert!(
        shown >= hp1 / hp0,
        "displayed damage should ease toward the new health"
    );
}

#[test]
fn water_spray_pushes_the_live_body_away_and_consumes_each_impulse() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let npc = spawn_in_front(&mut s, &root, "Goblin_miner", 2.5);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let index = s.world.npcs[&npc].body;
    let start = s.world.physics.bodies[index].position;
    let forward = s.body().direction();
    // Q conjures Water. Let the native element cooldown elapse, then Q again.
    for down in [
        true, false, false, false, false, false, false, false, false, true, false,
    ] {
        let mut input = PlayerInput::default();
        input.elements[0] = down;
        frame(&mut s, input);
    }
    assert!(s
        .avatar()
        .conjure
        .queue
        .iter()
        .all(|spell| spell.element == E::WATER));
    assert_eq!(s.avatar().conjure.queue.len(), 2);
    let mut hit = false;
    let mut pushed = false;
    let mut peak_speed = 0.0_f32;
    for _ in 0..90 {
        let events = frame(
            &mut s,
            PlayerInput {
                cast_down: true,
                ..PlayerInput::default()
            },
        );
        hit |= events.iter().any(|e| matches!(e, SessionEvent::Avatar(AvatarEvent::SprayHit { target }) if *target == npc));
        let b = &s.world.physics.bodies[index];
        pushed |= b.is_pushed();
        peak_speed = peak_speed.max(b.velocity[0] * forward[0] + b.velocity[2] * forward[2]);
        assert_eq!(
            s.world.npcs[&npc].character.combat.motion.impulse, [0.; 3],
            "numerical impulse must be consumed once"
        );
    }
    let end = s.world.physics.bodies[index].position;
    let away = (end[0] - start[0]) * forward[0] + (end[2] - start[2]) * forward[2];
    eprintln!("water spray: hit={hit} pushed={pushed}, peak outward speed {peak_speed}, displacement {away} m ({start:?} -> {end:?})");
    assert!(hit, "water spray missed target");
    assert!(pushed, "live physics body never became pushed");
    assert!(peak_speed > 0.0, "no outward velocity");
    assert!(away > 0.0, "no outward displacement");
}

#[test]
fn life_self_cast_does_not_release_old_enemy_knockback() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let index = s.avatar().body;
    let position = s.body().position;
    let result = s.world.players[0].as_mut().unwrap().receive_damage(
        &mut s.world.physics.bodies[index],
        magicka_sim::damage::Damage {
            attack: magicka_sim::damage::attack::DAMAGE | magicka_sim::damage::attack::PUSHED,
            element: E::EARTH.0,
            amount: 400.,
            magnitude: 0.1,
        },
        123,
        [position[0] - 1., position[1], position[2]],
        s.play_time,
    );
    assert_ne!(result & 16, 0, "the preceding hit must push");
    assert!(
        s.body().is_pushed(),
        "knockback must reach the body on the hit"
    );
    assert_eq!(s.avatar().character.combat.motion.impulse, [0.; 3]);
    assert!(!s.avatar().character.combat.motion.is_pushed);
    // Let the original knockback dissipate before healing through real input.
    for _ in 0..240 {
        frame(&mut s, PlayerInput::default());
    }
    let mut input = PlayerInput::default();
    input.elements[1] = true;
    frame(&mut s, input);
    frame(&mut s, PlayerInput::default());
    let mut healed = false;
    for _ in 0..120 {
        let events = frame(
            &mut s,
            PlayerInput {
                cast_self_down: true,
                ..Default::default()
            },
        );
        healed |= events.iter().any(|e| matches!(e,
            SessionEvent::Avatar(AvatarEvent::SelfCastApplied { hit_points_before, hit_points_after, .. })
                if hit_points_after > hit_points_before));
        let v = s.body().velocity;
        assert!(
            v[0].abs() < 1. && v[2].abs() < 1.,
            "healing released lateral velocity: {v:?}"
        );
    }
    assert!(healed, "Life must still heal");
}

#[test]
fn life_self_cast_heals_once_and_stops_native_effects() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    // Ambient level effects share the effect/particle systems.
    s.stop_level_effects();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    // A real public damage call, rather than altering HP directly.
    let index = s.avatar().body;
    let position = s.body().position;
    s.world.players[0].as_mut().unwrap().receive_damage(
        &mut s.world.physics.bodies[index],
        magicka_sim::damage::Damage {
            attack: magicka_sim::damage::attack::DAMAGE,
            element: E::EARTH.0,
            amount: 400.,
            magnitude: 1.,
        },
        123,
        position,
        s.play_time,
    );
    let hp0 = s.avatar().character.combat.health.hit_points;
    assert_eq!(hp0, 600.);
    let mut input = PlayerInput::default();
    input.elements[1] = true; // W: Life
    frame(&mut s, input);
    frame(&mut s, PlayerInput::default());
    let mut deliveries = 0;
    let mut started = false;
    let mut stopped = false;
    let mut peak_particles = 0;
    for _ in 0..120 {
        for e in frame(
            &mut s,
            PlayerInput {
                cast_self_down: true,
                ..PlayerInput::default()
            },
        ) {
            match e {
                SessionEvent::Avatar(AvatarEvent::SelfCastApplied {
                    hit_points_before,
                    hit_points_after,
                    ..
                }) => {
                    deliveries += 1;
                    assert_eq!(
                        hit_points_after - hit_points_before,
                        180.,
                        "native instantaneous Life term"
                    );
                }
                SessionEvent::Avatar(AvatarEvent::SelfEffectStarted {
                    name: "self_life", ..
                }) => started = true,
                SessionEvent::Avatar(AvatarEvent::SelfEffectsStopped) => stopped = true,
                SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(reason)) => {
                    panic!("{reason}")
                }
                _ => {}
            }
        }
        peak_particles = peak_particles.max(s.effects.particles.active().len());
    }
    let hp1 = s.avatar().character.combat.health.hit_points;
    eprintln!("Life self-cast: {hp0} -> {hp1} HP, {deliveries} deliveries, {peak_particles} peak particles");
    // Beam Self also applies Healing status (-80 DPS for one second).
    // Its existing integer tick rounding can overshoot the nominal 80 total.
    assert!(
        (860. ..1000.).contains(&hp1),
        "instant plus status healing: {hp1}"
    );
    assert_eq!(deliveries, 1, "held middle mouse must not repeat heal");
    assert!(started && stopped);
    assert!(
        peak_particles > 0,
        "native self_life must produce particles"
    );
    assert!(s.avatar().current_spell.is_none());
    assert_eq!(s.effects.active_effects(), fairy_effects(&s));
}

#[test]
fn water_self_cast_applies_wet_once_and_cleans_up() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    // Ambient level effects share the effect/particle systems.
    s.stop_level_effects();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let mut input = PlayerInput::default();
    input.elements[0] = true;
    frame(&mut s, input);
    frame(&mut s, PlayerInput::default());
    let mut wet = false;
    let mut deliveries = 0;
    let mut started = false;
    let mut stopped = false;
    for _ in 0..90 {
        for e in frame(
            &mut s,
            PlayerInput {
                cast_self_down: true,
                ..PlayerInput::default()
            },
        ) {
            match e {
                SessionEvent::Avatar(AvatarEvent::SelfCastApplied { .. }) => deliveries += 1,
                SessionEvent::Avatar(AvatarEvent::SelfEffectStarted {
                    name: "self_water", ..
                }) => started = true,
                SessionEvent::Avatar(AvatarEvent::SelfEffectsStopped) => stopped = true,
                SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(reason)) => {
                    panic!("{reason}")
                }
                _ => {}
            }
        }
        wet |= s
            .avatar()
            .character
            .combat
            .statuses
            .has(magicka_sim::status::Status::Wet);
    }
    assert!(wet, "Water self-cast must apply Wet");
    assert_eq!(deliveries, 1);
    assert!(started && stopped);
    assert_eq!(s.avatar().character.combat.health.hit_points, 1000.);
    assert!(s.avatar().current_spell.is_none());
    // The Wet StatusEffect emitter persists with the status.
    assert_eq!(
        s.effects.active_effects(),
        s.active_status_emitters() + fairy_effects(&s)
    );
    assert_eq!(s.status_emitters(s.avatar().handle), ["statuseffect_wet"]);
    assert_eq!(
        s.audio
            .iter()
            .filter(|(request, _)| matches!(request,
        magicka_runtime::audio::AudioRequest::Start { bank: 16, cue, .. }
        if *cue == magicka_runtime::scene_loading::custom_hash("spell_water_drip")))
            .count(),
        1,
        "Water self cast starts one owned drip cue"
    );
}

#[test]
fn water_self_cast_extinguishes_burning_with_audio_and_drying_effect() {
    use magicka_sim::{
        damage::{attack, Damage},
        status::Status,
    };
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let index = s.avatar().body;
    let position = s.body().position;
    s.world.players[0].as_mut().unwrap().receive_damage(
        &mut s.world.physics.bodies[index],
        Damage {
            attack: attack::STATUS,
            element: E::FIRE.0,
            amount: 60.,
            magnitude: 1.,
        },
        123,
        position,
        s.play_time,
    );
    assert!(s.avatar().character.combat.statuses.has(Status::Burning));
    let mut input = PlayerInput::default();
    input.elements[0] = true;
    frame(&mut s, input);
    frame(&mut s, PlayerInput::default());
    let mut delivered = false;
    for _ in 0..45 {
        let events = frame(
            &mut s,
            PlayerInput {
                cast_self_down: true,
                ..Default::default()
            },
        );
        delivered |= events
            .iter()
            .any(|e| matches!(e, SessionEvent::Avatar(AvatarEvent::SelfCastApplied { .. })));
        if delivered {
            break;
        }
    }
    assert!(delivered, "Self cast must reach public damage delivery");
    assert!(
        s.effects
            .active_effects_by_hash(magicka_runtime::scene_loading::custom_hash("drying_steam"))
            > 0
    );
    let fire_keys: Vec<_> = s
        .audio
        .iter()
        .filter_map(|(request, _)| match request {
            magicka_runtime::audio::AudioRequest::Start {
                key, bank: 16, cue, ..
            } if *cue == magicka_runtime::scene_loading::custom_hash("spell_fire_onfire") => {
                Some(*key)
            }
            _ => None,
        })
        .collect();
    assert_eq!(fire_keys.len(), 1);
    assert!(s.audio.iter().any(|(request, _)| matches!(request,
        magicka_runtime::audio::AudioRequest::Stop { key, instant: false } if *key == fire_keys[0])));
    assert!(s.audio.iter().any(|(request, _)| matches!(request,
        magicka_runtime::audio::AudioRequest::Start { bank: 16, cue, .. }
        if *cue == magicka_runtime::scene_loading::custom_hash("spell_steam_hit"))));
    let statuses = &s.avatar().character.combat.statuses;
    assert!(!statuses.has(Status::Burning));
    assert!(
        !statuses.has(Status::Wet),
        "extinguishing consumes the first Water application"
    );
    assert!(statuses.effect(Status::Burning).unwrap().dead());
    let hp = s.avatar().character.combat.health.hit_points;
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(
        s.avatar().character.combat.health.hit_points,
        hp,
        "no residual burning damage"
    );
}

#[test]
fn charged_force_and_area_push_move_live_bodies_once() {
    let Some(root) = content() else {
        return;
    };
    for area in [false, true] {
        let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
        s.disable_scripts();
        for _ in 0..60 {
            frame(&mut s, PlayerInput::default());
        }
        let npc = spawn_in_front(&mut s, &root, "Goblin_miner", 2.5);
        for _ in 0..10 {
            frame(&mut s, PlayerInput::default());
        }
        let index = s.world.npcs[&npc].body;
        let hp = s.world.npcs[&npc].character.combat.health.hit_points;
        let start = s.world.physics.bodies[index].position;
        let forward = s.body().direction();
        let mut charged = false;
        for _ in 0..600 {
            frame(
                &mut s,
                PlayerInput {
                    cast_down: true,
                    shift_down: area,
                    ..Default::default()
                },
            );
            charged |= s.avatar().motion.state == AvatarState::Charge;
            if s.avatar().character.spell_power == 1. {
                break;
            }
        }
        assert!(charged, "push must enter native ChargeState");
        assert_eq!(s.avatar().character.spell_power, 1.);
        let mut hits = 0;
        let mut pushed = false;
        for _ in 0..90 {
            for e in frame(&mut s, PlayerInput::default()) {
                match e {
                    SessionEvent::Avatar(AvatarEvent::PushHit { target }) if target == npc => {
                        hits += 1
                    }
                    SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(reason)) => {
                        panic!("{reason}")
                    }
                    _ => {}
                }
            }
            pushed |= s.world.physics.bodies[index].is_pushed();
        }
        let end = s.world.physics.bodies[index].position;
        let away = (end[0] - start[0]) * forward[0] + (end[2] - start[2]) * forward[2];
        eprintln!(
            "{} push: hits={hits} pushed={pushed} outward={away} m",
            if area { "Area" } else { "Force" }
        );
        assert_eq!(hits, 1);
        assert!(pushed);
        assert!(away > 0.);
        assert_eq!(s.world.npcs[&npc].character.combat.health.hit_points, hp);
        assert_eq!(s.avatar().character.spell_power, 0.);
        assert!(s.avatar().current_spell.is_none());
    }
}

/// `ELEMENT_KEYS` index of Earth (default key D).
const EARTH: usize = 6;

/// BlastEvent → FullBlast with an Arcane element: an Earth+Arcane rock's
/// explosion draws the ArcaneBlast disc (feedback 2026-10-09).
#[test]
fn earth_arcane_rock_explosion_draws_an_arcane_blast() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    spawn_in_front(&mut s, &root, "Goblin_miner", 5.0);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, EARTH);
    conjure(&mut s, ARCANE);
    let (mut launched, mut discs) = (0, 0);
    for i in 0..300 {
        for e in frame(
            &mut s,
            PlayerInput {
                cast_down: i < 45,
                ..PlayerInput::default()
            },
        ) {
            if matches!(e, SessionEvent::Avatar(AvatarEvent::MissileLaunched(_))) {
                launched += 1;
            }
        }
        discs = discs.max(s.arcane_blasts.len());
    }
    assert_eq!(launched, 1);
    assert!(
        discs > 0,
        "no ArcaneBlast for the Earth+Arcane rock explosion"
    );
}

#[test]
fn charged_earth_rock_flies_and_hits_a_goblin() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let npc = spawn_in_front(&mut s, &root, "Goblin_miner", 5.0);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let hp0 = s.world.npcs[&npc].character.combat.health.hit_points;
    let mut earth = [false; 8];
    earth[EARTH] = true;
    frame(
        &mut s,
        PlayerInput {
            elements: earth,
            ..PlayerInput::default()
        },
    );
    frame(&mut s, PlayerInput::default());
    assert_eq!(s.avatar().conjure.queue.len(), 1);
    let (mut launched, mut peak_missiles, mut charged) = (0, 0, false);
    let mut events = Vec::new();
    // Hold to charge for 0.75 s, then release.
    for i in 0..300 {
        let input = PlayerInput {
            cast_down: i < 45,
            ..PlayerInput::default()
        };
        for e in frame(&mut s, input) {
            if matches!(e, SessionEvent::Avatar(AvatarEvent::MissileLaunched(_))) {
                launched += 1;
            }
            events.push(e);
        }
        charged |=
            s.avatar().motion.state == magicka_runtime::character_motion::CharacterState::Charge;
        peak_missiles = peak_missiles.max(s.world.missiles.len());
    }
    let hp1 = s
        .world
        .npcs
        .get(&npc)
        .map_or(0.0, |n| n.character.combat.health.hit_points);
    eprintln!(
        "charged {charged}; launched {launched}; peak missiles {peak_missiles}; goblin hp {hp0} -> {hp1}; remaining {}; not reconstructed {:?}",
        s.world.missiles.len(),
        events.iter().filter_map(|e| match e {
            SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(x)) => Some(x.clone()),
            _ => None,
        }).collect::<Vec<_>>()
    );
    assert!(charged, "never charged");
    assert_eq!(launched, 1, "an Earth cast launches one rock");
    assert!(hp1 < hp0, "the rock did not hurt the goblin");
    assert!(s.world.missiles.is_empty(), "the rock was not removed");
}

#[test]
fn earth_rock_flies_forward_under_gravity_and_is_removed() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let mut earth = [false; 8];
    earth[EARTH] = true;
    frame(
        &mut s,
        PlayerInput {
            elements: earth,
            ..PlayerInput::default()
        },
    );
    frame(&mut s, PlayerInput::default());
    let start = s.body().position;
    let mut path = Vec::new();
    for i in 0..240 {
        frame(
            &mut s,
            PlayerInput {
                cast_down: i < 45,
                ..PlayerInput::default()
            },
        );
        if let Some(m) = s.world.missiles.first() {
            path.push((i, m.body.position, m.body.velocity, m.state.has_collided));
        }
    }
    for p in path.iter().step_by(4).take(20) {
        eprintln!("{p:?}");
    }
    let far = path
        .iter()
        .map(|(_, p, _, _)| ((p[0] - start[0]).powi(2) + (p[2] - start[2]).powi(2)).sqrt())
        .fold(0.0f32, f32::max);
    eprintln!(
        "rock path {} frames, farthest {far:.2} m, remaining {}",
        path.len(),
        s.world.missiles.len()
    );
    assert!(far > 4.0, "the rock barely moved ({far})");
}

#[test]
fn water_and_cold_make_ice_shards_that_hit_a_goblin() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let npc = spawn_in_front(&mut s, &root, "Goblin_miner", 4.0);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let hp0 = s.world.npcs[&npc].character.combat.health.hit_points;
    // Q (Water, index 0), then R (Cold, index 3): Water + Cold combine to Ice.
    for key in [0usize, 3] {
        let mut e = [false; 8];
        e[key] = true;
        frame(
            &mut s,
            PlayerInput {
                elements: e,
                ..PlayerInput::default()
            },
        );
        for _ in 0..10 {
            frame(&mut s, PlayerInput::default());
        }
    }
    let queue = s.avatar().conjure.queue.clone();
    eprintln!(
        "queue {:?}",
        queue.iter().map(|q| q.element).collect::<Vec<_>>()
    );
    let mut launched = 0;
    for i in 0..240 {
        for e in frame(
            &mut s,
            PlayerInput {
                cast_down: i < 30,
                ..PlayerInput::default()
            },
        ) {
            if matches!(e, SessionEvent::Avatar(AvatarEvent::MissileLaunched(_))) {
                launched += 1;
            }
        }
    }
    let hp1 = s
        .world
        .npcs
        .get(&npc)
        .map_or(0.0, |n| n.character.combat.health.hit_points);
    eprintln!("ice shards launched {launched}; goblin hp {hp0} -> {hp1}");
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].element, E::ICE);
    assert_eq!(launched, 3, "3 × Ice magnitude 1 shards");
    assert!(hp1 < hp0);
}

#[test]
fn lightning_bolts_chain_into_goblins() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let near = spawn_in_front(&mut s, &root, "Goblin_miner", 3.0);
    let far = spawn_in_front(&mut s, &root, "Goblin_miner", 5.5);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let hp = |s: &PlaySession, h| {
        s.world
            .npcs
            .get(&h)
            .map_or(0.0, |n| n.character.combat.health.hit_points)
    };
    let (near0, far0) = (hp(&s, near), hp(&s, far));
    // A A: Lightning (index 4) twice.
    for _ in 0..2 {
        let mut e = [false; 8];
        e[4] = true;
        frame(
            &mut s,
            PlayerInput {
                elements: e,
                ..PlayerInput::default()
            },
        );
        for _ in 0..10 {
            frame(&mut s, PlayerInput::default());
        }
    }
    let (mut bolts, mut peak_drawn, mut peak_effects) = (Vec::new(), 0, 0);
    for i in 0..120 {
        for e in frame(
            &mut s,
            PlayerInput {
                cast_down: i < 75,
                ..PlayerInput::default()
            },
        ) {
            if let SessionEvent::Avatar(AvatarEvent::LightningBolt(b)) = e {
                bolts.push(b);
            }
        }
        peak_drawn = peak_drawn.max(s.bolts.len());
        peak_effects = peak_effects.max(s.effects.active_effects());
    }
    let (near1, far1) = (hp(&s, near), hp(&s, far));
    eprintln!(
        "lightning: {} bolt segments, peak drawn {peak_drawn}, peak effects {peak_effects}; \
         near {near0} -> {near1}, far {far0} -> {far1}",
        bolts.len()
    );
    // Eight casts, each hitting the near goblin and chaining to the far one,
    // then a cut bolt beyond it.
    assert_eq!(bolts.len(), 24);
    assert!(near1 < near0 && far1 < far0);
    assert!(peak_effects > 0 && peak_drawn > 0);
    assert!(s.bolts.is_empty(), "bolts expire");
}

/// `ELEMENT_KEYS` index of Arcane (default key S).
const ARCANE: usize = 5;

fn conjure(s: &mut PlaySession, element: usize) {
    let mut e = [false; 8];
    e[element] = true;
    frame(
        s,
        PlayerInput {
            elements: e,
            ..PlayerInput::default()
        },
    );
    for _ in 0..10 {
        frame(s, PlayerInput::default());
    }
}

fn npc_hp(s: &PlaySession, h: u16) -> f32 {
    s.world
        .npcs
        .get(&h)
        .map_or(0.0, |n| n.character.combat.health.hit_points)
}

/// Railgun: a beam hitting a Shield entity obliquely spawns a reflected
/// child rail (feedback 2026-10-08: shields did not bounce rays).
#[test]
fn arcane_beam_reflects_off_a_shield_entity() {
    use magicka_sim::shield_entity::{ShieldKind, SHIELD_COLOR};
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let (p, f) = (s.body().position, s.body().direction());
    let side = [-f[2], 0.0, f[0]];
    let center = [
        p[0] + f[0] * 5.0 + side[0] * 1.2,
        p[1],
        p[2] + f[2] * 5.0 + side[2] * 1.2,
    ];
    let owner = s.avatar().handle;
    s.world.spawn_shield(
        owner,
        center,
        2.0,
        [-f[0], 0.0, -f[2]],
        ShieldKind::Sphere,
        100000.0,
        SHIELD_COLOR,
    );
    conjure(&mut s, ARCANE);
    let mut reflected = None;
    for i in 0..60 {
        frame(
            &mut s,
            PlayerInput {
                cast_down: i < 50,
                ..PlayerInput::default()
            },
        );
        let rails = &s.world.rails;
        for &id in rails.active() {
            let Some(r) = rails.rail(id) else { continue };
            if !r.parents.is_empty() {
                reflected = Some(r.direction);
            }
        }
    }
    let d = reflected.expect("a reflected child rail");
    let along = d[0] * f[0] + d[2] * f[2];
    assert!(along < 0.99, "child continues straight: {d:?}");
}

#[test]
fn arcane_beam_grows_hits_a_goblin_every_quarter_second_and_ends() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let goblin = spawn_in_front(&mut s, &root, "Goblin_miner", 4.0);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let hp0 = npc_hp(&s, goblin);
    conjure(&mut s, ARCANE);
    let (mut peak_rails, mut peak_length, mut frames_alive) = (0, 0.0f32, 0);
    let mut hp_steps = Vec::new();
    let mut last = hp0;
    for i in 0..150 {
        frame(
            &mut s,
            PlayerInput {
                cast_down: i < 90,
                ..PlayerInput::default()
            },
        );
        let rails = &s.world.rails;
        peak_rails = peak_rails.max(rails.active().len());
        if let Some(r) = rails.active().first().and_then(|&id| rails.rail(id)) {
            peak_length = peak_length.max(r.length);
            frames_alive += 1;
        }
        let hp = npc_hp(&s, goblin);
        if hp != last {
            hp_steps.push((i, last - hp));
            last = hp;
        }
    }
    eprintln!(
        "beam: {frames_alive} frames, peak {peak_rails} rails, peak length {peak_length}, \
         hp {hp0} -> {last}, steps {hp_steps:?}"
    );
    // One root rail, stopped at the goblin (about 4 m minus the staff
    // offset), never reaching the 75 m cap.
    assert_eq!(peak_rails, 1);
    assert!(peak_length > 2.0 && peak_length < 5.0, "{peak_length}");
    // Damage arrives in separate ticks 0.25 s (15 frames) apart.
    assert!(hp_steps.len() >= 3, "{hp_steps:?}");
    for w in hp_steps.windows(2) {
        assert_eq!(w[1].0 - w[0].0, 15, "{hp_steps:?}");
    }
    assert!(last < hp0);
    // Releasing the button ends the spell and the rail.
    assert!(s.world.rails.is_empty());
    assert!(
        s.avatar().current_spell.is_none() || !s.avatar().current_spell.as_ref().unwrap().active()
    );
}

#[test]
fn freezing_an_active_beam_stops_the_effect_without_ending_cast_input() {
    use magicka_runtime::avatar::SpellEffect;
    use magicka_sim::{
        damage::{attack, Damage},
        damage_delivery::DamagePayload,
        status::Status,
    };
    let Some(root) = content() else {
        return;
    };
    for (collection, immune) in [(false, false), (true, false), (false, true)] {
        let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
        s.disable_scripts();
        for _ in 0..60 {
            frame(&mut s, PlayerInput::default());
        }
        conjure(&mut s, ARCANE);
        for _ in 0..60 {
            frame(
                &mut s,
                PlayerInput {
                    cast_down: true,
                    ..Default::default()
                },
            );
            if !s.world.rails.is_empty() {
                break;
            }
        }
        assert!(
            !s.world.rails.is_empty(),
            "ordinary casting must start a beam"
        );
        let body = s.avatar().body;
        let position = s.body().position;
        let before = s.avatar().cast.cast_type;
        let player = s.world.players[0].as_mut().unwrap();
        player.character.definition.resistances[E::ICE.index() as usize].status_resistance = immune;
        let water = Damage {
            attack: attack::STATUS,
            element: E::WATER.0,
            amount: 0.0,
            magnitude: 1.0,
        };
        let cold = Damage {
            element: E::COLD.0,
            ..water
        };
        player.receive_damage(
            &mut s.world.physics.bodies[body],
            water,
            123,
            position,
            s.play_time,
        );
        if collection {
            player.receive_payload(
                &mut s.world.physics.bodies[body],
                DamagePayload::Collection([
                    cold,
                    Damage::default(),
                    Damage::default(),
                    Damage::default(),
                    Damage::default(),
                ]),
                Some(123),
                position,
                s.play_time,
            );
        } else {
            player.receive_damage(
                &mut s.world.physics.bodies[body],
                cold,
                123,
                position,
                s.play_time,
            );
        }
        if immune {
            assert!(!player.character.combat.statuses.has(Status::Frozen));
            assert!(player.rail_kills.is_empty());
            assert!(
                player.current_spell.as_ref().unwrap().active(),
                "Ice status immunity preserves the beam"
            );
            continue;
        }
        assert!(player.character.combat.statuses.has(Status::Frozen));
        assert_eq!(
            player.cast.cast_type, before,
            "status Stop is distinct from EndCast"
        );
        assert!(
            !player.rail_kills.is_empty(),
            "beam deinitialization queues its owned rail kill"
        );
        assert!(
            matches!(&player.current_spell, Some(SpellEffect::RailGun(effect))
            if !effect.lifecycle.active && effect.rail.is_none()),
            "native current spell pointer remains inactive"
        );
        for _ in 0..90 {
            frame(
                &mut s,
                PlayerInput {
                    cast_down: true,
                    ..Default::default()
                },
            );
        }
        assert!(
            s.world.rails.is_empty(),
            "held input must not recreate the stopped beam"
        );
    }
}

/// Railgun BloatKill: an Arcane beam tick that leaves a gibbed goblin at
/// HitPoints ≤ 0 bloats it; 0.333 s later BloatBlast's Arcane FullBlast
/// (ArcaneBlast visual) damages a neighbour and the goblin is overkilled.
#[test]
fn arcane_beam_kill_bloats_and_explodes_the_goblin() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let goblin = spawn_in_front(&mut s, &root, "Goblin_miner", 4.0);
    let neighbour = spawn_in_front(&mut s, &root, "Goblin_miner", 5.5);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        s.world.npcs[&goblin].definition.gib_count > 0,
        "goblin has gibs"
    );
    // One beam tick kills it.
    s.world
        .npcs
        .get_mut(&goblin)
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 1.0;
    let neighbour_hp0 = npc_hp(&s, neighbour);
    conjure(&mut s, ARCANE);
    let (mut bloat_frame, mut blast_frame, mut arcane_blast) = (None, None, false);
    let mut overkill_hp = 0.0;
    for i in 0..120 {
        frame(
            &mut s,
            PlayerInput {
                cast_down: i < 60,
                ..PlayerInput::default()
            },
        );
        if bloat_frame.is_none() && s.world.bloats.contains_key(&goblin) {
            bloat_frame = Some(i);
        }
        if blast_frame.is_none()
            && s.world
                .npcs
                .get(&goblin)
                .is_some_and(|n| n.character.combat.overkilled)
        {
            blast_frame = Some(i);
            overkill_hp = npc_hp(&s, goblin);
        }
        arcane_blast |= s
            .beam_blasts
            .iter()
            .any(|&(_, _, e)| e & magicka_sim::spells::Elements::ARCANE.0 != 0);
    }
    let (b, x) = (
        bloat_frame.expect("bloated"),
        blast_frame.expect("exploded"),
    );
    // mBloatTimer >= 0.333 s: the 20th or 21st 1/60 s update.
    assert!((19..=21).contains(&(x - b)), "bloat {b} blast {x}");
    assert!(arcane_blast, "ArcaneBlast visual requested");
    assert!(overkill_hp < -1000.0, "OverKill hit points {overkill_hp}");
    assert!(
        npc_hp(&s, neighbour) < neighbour_hp0,
        "BloatBlast damaged the neighbour"
    );
    // DeadState: SpawnGibs (every Goblin_miner gib model loads), the body is
    // hidden and mDeadTimer = -100 removes the character.
    assert!(s.world.overkilled.contains(&goblin));
    assert!(!s.world.npcs.contains_key(&goblin), "gibbed goblin removed");
    let gib_count = s.world.gibs.live.len();
    assert!(gib_count > 0, "gibs spawned");
    assert!(
        s.world
            .decals
            .frame
            .iter()
            .any(|p| p.args[1] < 2.0 && p.args[2] > 0.0),
        "damage/gib blood retained after removing the goblin"
    );
    let ground = s.body().feet()[1];
    for _ in 0..240 {
        frame(&mut s, PlayerInput::default());
    }
    // Four seconds later the gibs rest on the floor, not below it.
    assert_eq!(s.world.gibs.live.len(), gib_count);
    for g in &s.world.gibs.live {
        let speed = g.velocity.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!(speed < 0.5, "gib {} still moving at {speed}", g.id);
        assert!(
            g.position[1] > ground - 0.25 && g.position[1] < ground + 2.0,
            "gib {} at {:?}, floor {ground}",
            g.id,
            g.position
        );
    }
    // TTL is 10-20 s; all gibs sink away and return to the cache.
    for _ in 0..(17 * 60) {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.gibs.live.is_empty(), "gibs expired");
    assert_eq!(s.world.gibs.cached(), magicka_runtime::gibs::CACHE_SIZE);
}

/// LightningSpell.CastSelf: the self-cast effect, then Lightning/Self damage
/// to the caster at once (no bolt).
#[test]
fn lightning_self_cast_damages_the_caster() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let hp0 = s.avatar().character.combat.health.hit_points;
    let lightning = magicka_runtime::avatar::ELEMENT_KEYS
        .iter()
        .position(|&e| e == E::LIGHTNING)
        .unwrap();
    conjure(&mut s, lightning);
    let mut unsupported = Vec::new();
    for i in 0..90 {
        for e in frame(
            &mut s,
            PlayerInput {
                cast_self_down: i < 20,
                ..PlayerInput::default()
            },
        ) {
            if let SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(n)) = e {
                unsupported.push(n);
            }
        }
    }
    assert!(unsupported.is_empty(), "{unsupported:?}");
    assert!(s.bolts.is_empty(), "no bolt for Self");
    let hp1 = s.avatar().character.combat.health.hit_points;
    assert!(hp1 < hp0, "caster hp {hp0} -> {hp1}");
}

fn self_cast(s: &mut PlaySession, keys: &[usize]) -> Vec<String> {
    for &k in keys {
        conjure(s, k);
    }
    let mut unsupported = Vec::new();
    for i in 0..40 {
        for e in frame(
            s,
            PlayerInput {
                cast_self_down: i < 20,
                ..PlayerInput::default()
            },
        ) {
            if let SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(n)) = e {
                unsupported.push(n);
            }
        }
    }
    unsupported
}

/// ShieldSpell.CastSelf → AddSelfShield: pure Shield (500 HP decaying
/// 100/s) and Earth (1500 HP, mass 1001, SpeedMultiplier 0.825).
#[test]
fn shield_self_casts_create_self_shields() {
    use magicka_sim::character_health::SelfShieldType as T;
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let key = |e: E| {
        magicka_runtime::avatar::ELEMENT_KEYS
            .iter()
            .position(|&k| k == e)
            .unwrap()
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let unsupported = self_cast(&mut s, &[key(E::SHIELD)]);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    let shield = s.avatar().character.combat.health.shield.clone();
    assert_eq!(shield.kind, T::Shield);
    assert!(
        shield.health > 400.0 && shield.health < 500.0,
        "{}",
        shield.health
    );
    for _ in 0..360 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(
        s.avatar().character.combat.health.shield.kind,
        T::None,
        "decayed"
    );

    let unsupported = self_cast(&mut s, &[key(E::SHIELD), key(E::EARTH)]);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    let shield = s.avatar().character.combat.health.shield.clone();
    assert_eq!(shield.kind, T::Earth);
    assert_eq!(shield.max_health, 1500.0);
    assert_eq!(s.body().mass, 1001.0);
    assert!((s.body().speed_multiplier() - 0.825).abs() < 1e-4);
}

/// Shield + Fire Self: a self-casted Fire immunity aura (resistance ×0)
/// that blocks the fire of a following Fire self-cast.
#[test]
fn shield_fire_self_cast_grants_fire_immunity() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let key = |e: E| {
        magicka_runtime::avatar::ELEMENT_KEYS
            .iter()
            .position(|&k| k == e)
            .unwrap()
    };
    // Reference: Fire self-cast without the aura.
    let mut plain = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    plain.disable_scripts();
    for _ in 0..60 {
        frame(&mut plain, PlayerInput::default());
    }
    let hp0 = plain.avatar().character.combat.health.hit_points;
    self_cast(&mut plain, &[FIRE]);
    let burned = hp0 - plain.avatar().character.combat.health.hit_points;
    assert!(burned > 0.0);

    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let unsupported = self_cast(&mut s, &[key(E::SHIELD), FIRE]);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    let auras = &s.avatar().character.auras;
    assert_eq!(auras.len(), 1);
    assert_eq!(auras[0].resistance.against, E::FIRE);
    assert!(
        auras[0].ttl > 19.0 && auras[0].ttl <= 20.0,
        "{}",
        auras[0].ttl
    );
    let hp1 = s.avatar().character.combat.health.hit_points;
    self_cast(&mut s, &[FIRE]);
    let hp2 = s.avatar().character.combat.health.hit_points;
    assert!(
        hp1 - hp2 < burned * 0.5,
        "immune: lost {} vs {burned}",
        hp1 - hp2
    );
}

fn cast_held(s: &mut PlaySession, frames: usize, shift: bool) -> Vec<String> {
    let mut unsupported = Vec::new();
    for i in 0..frames {
        for e in frame(
            s,
            PlayerInput {
                cast_down: i < 30,
                shift_down: shift && i < 30,
                ..PlayerInput::default()
            },
        ) {
            if let SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(n)) = e {
                unsupported.push(n);
            }
        }
    }
    unsupported
}

/// ShieldSpell pure Shield: Force places an 800 HP disc 3.5 m behind the
/// caster, Area a 1000 HP sphere at the caster. The recovered
/// CircleCircleIntersect does not count this pair as overlapping (the circle
/// intersections lie ~71° off the disc axis, outside its 45° opening), so
/// both remain until they decay.
#[test]
fn projectile_launch_checks_live_shields_but_self_cast_bypasses_them() {
    use magicka_sim::shield_entity::{ShieldKind, SHIELD_COLOR};
    let Some(root) = content() else { return };
    for (blocked, self_cast) in [(false, false), (true, false), (true, true)] {
        let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
        s.disable_scripts();
        for _ in 0..60 {
            frame(&mut s, PlayerInput::default());
        }
        if blocked {
            // Controlled geometry fixture: the staff source lies outside a
            // small sphere around the body, so the launch segment exits it.
            s.world.spawn_shield(
                s.avatar().handle,
                s.body().position,
                0.25,
                [0.0, 0.0, 1.0],
                ShieldKind::Sphere,
                1000.0,
                SHIELD_COLOR,
            );
        }
        conjure(&mut s, EARTH);
        let mut launches = 0;
        let mut hits = 0;
        let mut had_missile = false;
        for i in 0..120 {
            for event in frame(
                &mut s,
                PlayerInput {
                    cast_down: !self_cast && i < 30,
                    cast_self_down: self_cast && i < 30,
                    ..PlayerInput::default()
                },
            ) {
                match event {
                    SessionEvent::Avatar(AvatarEvent::MissileLaunched(_)) => launches += 1,
                    SessionEvent::Avatar(AvatarEvent::ProjectileBlocked {
                        element,
                        position,
                        direction,
                    }) => {
                        hits += 1;
                        assert_eq!(element, E::EARTH);
                        let center = s.world.shields[0].shield.position;
                        let distance = position
                            .iter()
                            .zip(center)
                            .map(|(a, b)| (a - b).powi(2))
                            .sum::<f32>()
                            .sqrt();
                        assert!((distance - 0.25).abs() < 0.001);
                        assert!((direction.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 0.001);
                    }
                    _ => {}
                }
            }
            had_missile |= !s.world.missiles.is_empty();
        }
        let suppressed = blocked && !self_cast;
        assert_eq!(launches, usize::from(!suppressed));
        assert_eq!(hits, usize::from(suppressed));
        assert_eq!(had_missile, !suppressed);
    }
}

#[test]
fn shield_force_and_area_spawn_shield_entities() {
    use magicka_sim::shield_entity::ShieldKind;
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let shield_key = magicka_runtime::avatar::ELEMENT_KEYS
        .iter()
        .position(|&k| k == E::SHIELD)
        .unwrap();
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, shield_key);
    let unsupported = cast_held(&mut s, 40, false);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    assert_eq!(s.world.shields.len(), 1);
    let disc = s.world.shields[0].shield.clone();
    assert_eq!(disc.kind, ShieldKind::Disc);
    assert!(
        disc.hit_points > 700.0 && disc.hit_points < 800.0,
        "{}",
        disc.hit_points
    );
    let p = s.body().position;
    let d = ((disc.position[0] - p[0]).powi(2) + (disc.position[2] - p[2]).powi(2)).sqrt();
    assert!((d - 3.5).abs() < 0.6, "disc {d} m from the caster");

    conjure(&mut s, shield_key);
    cast_held(&mut s, 40, true);
    let sphere = s.world.shields.last().unwrap().shield.clone();
    assert_eq!(sphere.kind, ShieldKind::Sphere);
    assert_eq!(s.world.shields.len(), 2);
    // 1000 HP at 100/s: gone after ~10 s plus the fade.
    for _ in 0..720 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.shields.is_empty());
}

/// A Shield Force disc in front of the caster stops its own Fire spray from
/// reaching a goblin behind it and takes the damage itself.
#[test]
fn shield_disc_blocks_spray_and_takes_damage() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let shield_key = magicka_runtime::avatar::ELEMENT_KEYS
        .iter()
        .position(|&k| k == E::SHIELD)
        .unwrap();
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let goblin = spawn_in_front(&mut s, &root, "Goblin_miner", 4.0);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, shield_key);
    cast_held(&mut s, 40, false);
    assert_eq!(s.world.shields.len(), 1);
    let hp_goblin = npc_hp(&s, goblin);
    let shield_hp = s.world.shields[0].shield.hit_points;
    conjure(&mut s, FIRE);
    conjure(&mut s, FIRE);
    cast_held(&mut s, 50, false);
    assert_eq!(npc_hp(&s, goblin), hp_goblin, "goblin protected");
    let after = s.world.shields.first().map_or(0.0, |s| s.shield.hit_points);
    // Decay alone over these 72 frames (two conjures, 50 cast frames) is
    // 120 HP; the blocked spray costs more.
    assert!(shield_hp - after > 150.0, "shield {shield_hp} -> {after}");
}

#[test]
fn shift_arcane_area_blasts_a_nearby_goblin_once() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let goblin = spawn_in_front(&mut s, &root, "Goblin_miner", 3.0);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let hp0 = npc_hp(&s, goblin);
    let owner_hp = s.world.players[0]
        .as_ref()
        .unwrap()
        .character
        .combat
        .health
        .hit_points;
    conjure(&mut s, ARCANE);
    let mut steps = 0;
    let mut last = hp0;
    let mut blasts = Vec::new();
    for i in 0..90 {
        frame(
            &mut s,
            PlayerInput {
                cast_down: i < 20,
                shift_down: i < 20,
                ..PlayerInput::default()
            },
        );
        blasts.extend(s.beam_blasts.iter().copied());
        let hp = npc_hp(&s, goblin);
        if hp != last {
            steps += 1;
            last = hp;
        }
    }
    eprintln!("area beam: hp {hp0} -> {last}, blasts {blasts:?}");
    assert_eq!(blasts.len(), 1);
    // BlastSize = sqrt(1/5): radius 4.47 m around the caster.
    assert!((blasts[0].1 - (0.2f64.sqrt() * 10.0) as f32).abs() < 1e-4);
    assert_eq!(blasts[0].2 & E::ARCANE.0, E::ARCANE.0);
    assert_eq!(steps, 1);
    assert!(last < hp0);
    assert!(s.world.rails.is_empty());
    assert_eq!(
        s.world.players[0]
            .as_ref()
            .unwrap()
            .character
            .combat
            .health
            .hit_points,
        owner_hp
    );
}

fn mine_positions(s: &PlaySession) -> Vec<[f32; 3]> {
    s.world.mines.iter().map(|(_, m)| m.position).collect()
}

/// Shift + Shield + Arcane rings the caster with twelve chained mines on a
/// 4 m circle (two seeds, then one mine per side every 0.1 s).
#[test]
fn shield_arcane_area_rings_the_caster_with_chained_mines() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let shield_key = magicka_runtime::avatar::ELEMENT_KEYS
        .iter()
        .position(|&k| k == E::SHIELD)
        .unwrap();
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    assert!(s
        .world
        .mine_clip_duration
        .is_some_and(|d| (d - 2.167).abs() < 1e-3));
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, shield_key);
    conjure(&mut s, ARCANE);
    let center = s.body().position;
    let unsupported = cast_held(&mut s, 40, true);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let mines = mine_positions(&s);
    assert_eq!(mines.len(), 12);
    for m in &mines {
        let r = ((m[0] - center[0]).powi(2) + (m[2] - center[2]).powi(2)).sqrt();
        assert!((r - 4.0).abs() < 0.3, "mine {r} m from the caster");
    }
    assert!(s
        .world
        .mines
        .iter()
        .all(|(_, m)| !m.dead && m.clip_finished));
}

/// Shield + Arcane Force lays four mines ahead. A goblin standing on one
/// arms it; its blast hurts the goblin and chains through the other mines.
#[test]
fn shield_arcane_force_mines_detonate_under_a_goblin_and_chain() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let shield_key = magicka_runtime::avatar::ELEMENT_KEYS
        .iter()
        .position(|&k| k == E::SHIELD)
        .unwrap();
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, shield_key);
    conjure(&mut s, ARCANE);
    cast_held(&mut s, 30, false);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(mine_positions(&s).len(), 4);
    let goblin = spawn_in_front(&mut s, &root, "Goblin_miner", 3.0);
    let hp = npc_hp(&s, goblin);
    let mut activated = false;
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
        activated |= s.world.mines.iter().any(|(_, m)| m.activation_started);
    }
    assert!(activated);
    assert!(
        npc_hp(&s, goblin) < hp,
        "goblin {hp} -> {}",
        npc_hp(&s, goblin)
    );
    assert!(
        s.world.mines.iter().next().is_none(),
        "chain left {:?}",
        mine_positions(&s)
    );
}

fn shield_key() -> usize {
    magicka_runtime::avatar::ELEMENT_KEYS
        .iter()
        .position(|&k| k == E::SHIELD)
        .unwrap()
}

/// Distance walked along the facing direction in `frames` while holding
/// Move toward `ndc`.
fn walk_toward(s: &mut PlaySession, ndc: [f32; 2], frames: usize) -> f32 {
    let start = s.body().position;
    let input = PlayerInput {
        cursor_ndc: Some(ndc),
        move_down: true,
        ..Default::default()
    };
    for _ in 0..frames {
        frame(s, input);
    }
    let (p, forward) = (s.body().position, s.body().direction());
    (p[0] - start[0]) * forward[0] + (p[2] - start[2]) * forward[2]
}

/// Shield + Earth Force raises four Solid walls ahead; walking into them
/// stops the Wizard, while the same walk without walls covers several metres.
/// The Wizard first turns toward the open road.
#[test]
fn earth_force_walls_block_the_caster() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    const OPEN: [f32; 2] = [0.0, 0.6];
    let mut free = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    free.disable_scripts();
    for _ in 0..60 {
        frame(&mut free, PlayerInput::default());
    }
    walk_toward(&mut free, OPEN, 20);
    for _ in 0..40 {
        frame(&mut free, PlayerInput::default());
    }
    let open = walk_toward(&mut free, OPEN, 120);
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    assert!(s.world.barrier_clips.is_some());
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    walk_toward(&mut s, OPEN, 20);
    conjure(&mut s, shield_key());
    conjure(&mut s, EARTH);
    let unsupported = cast_held(&mut s, 30, false);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let walls: Vec<_> = s.world.barriers.iter().map(|(_, b)| b.clone()).collect();
    assert_eq!(walls.len(), 4);
    assert!(walls.iter().all(|b| b.solid() && b.hit_points > 900.0));
    let blocked = walk_toward(&mut s, OPEN, 120);
    assert!(open > 3.0, "free walk {open}");
    assert!(blocked < 1.0, "walked {blocked} m into the walls");
}

/// Shield + Fire Force makes six Elemental fire walls on a 4 m arc. A goblin
/// standing in them is damaged; the walls decay away (200 HP at 50/s).
#[test]
fn fire_force_walls_burn_a_goblin_and_decay() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let goblin = spawn_in_front(&mut s, &root, "Goblin_miner", 4.0);
    let hp = npc_hp(&s, goblin);
    conjure(&mut s, shield_key());
    conjure(&mut s, FIRE);
    cast_held(&mut s, 30, false);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let walls: Vec<_> = s.world.barriers.iter().map(|(_, b)| b.clone()).collect();
    assert_eq!(walls.len(), 6);
    assert!(walls.iter().all(|b| !b.solid()));
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        npc_hp(&s, goblin) < hp,
        "goblin {hp} -> {}",
        npc_hp(&s, goblin)
    );
    for _ in 0..240 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.barriers.iter().next().is_none());
}

/// Shift + Shield + Earth rings the caster with ten walls (radius Shield+3).
#[test]
fn earth_area_rings_the_caster_with_walls() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let center = s.body().position;
    conjure(&mut s, shield_key());
    conjure(&mut s, EARTH);
    cast_held(&mut s, 30, true);
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    let walls: Vec<_> = s.world.barriers.iter().map(|(_, b)| b.base()).collect();
    assert_eq!(walls.len(), 10, "{walls:?}");
    for w in &walls {
        let r = ((w[0] - center[0]).powi(2) + (w[2] - center[2]).powi(2)).sqrt();
        assert!((r - 4.0).abs() < 0.3, "wall {r} m from the caster");
    }
}

/// A Fire spray reaches mines (InternalDamage arms a 0.1 s fuse) and the
/// resulting blasts chain through the remaining mines.
#[test]
fn fire_spray_detonates_arcane_mines() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, shield_key());
    conjure(&mut s, ARCANE);
    cast_held(&mut s, 30, false);
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(mine_positions(&s).len(), 4);
    conjure(&mut s, FIRE);
    cast_held(&mut s, 40, false);
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        s.world.mines.iter().next().is_none(),
        "left {:?}",
        mine_positions(&s)
    );
}

/// A Fire spray damages Solid Earth walls beyond their self-decay.
#[test]
fn fire_spray_damages_earth_walls() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, shield_key());
    conjure(&mut s, EARTH);
    cast_held(&mut s, 30, false);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let before: f32 = s.world.barriers.iter().map(|(_, b)| b.hit_points).sum();
    let frames = 2 + 40;
    conjure(&mut s, FIRE);
    cast_held(&mut s, 40, false);
    let after: f32 = s.world.barriers.iter().map(|(_, b)| b.hit_points).sum();
    // Decay alone: 4 walls × 50 HP/s.
    let decay = 4.0 * 50.0 * frames as f32 / 60.0;
    assert!(before - after > decay + 50.0, "{before} -> {after}");
}

/// ProjectileSpell.CastForce captures SpellPower * 3 before clearing it.
/// Check charged input through casting, the event boundary and scene audio,
/// for both force cues and two different charge lengths.
#[test]
fn force_projectile_audio_retains_charge_after_spell_power_reset() {
    use magicka_runtime::audio::{banks::SPELLS, AudioRequest, SoundSource};
    use magicka_runtime::scene_loading::custom_hash;
    use magicka_sim::damage::CastType;
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut powers = Vec::new();
    for (elements, cue) in [
        (vec![E::EARTH], "spell_earth_force"),
        (vec![E::WATER, E::COLD], "spell_ice_force"),
    ] {
        for hold_frames in [12, 45] {
            let mut session = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
            session.disable_scripts();
            for _ in 0..60 {
                frame(&mut session, PlayerInput::default());
            }
            for element in &elements {
                let key = magicka_runtime::avatar::ELEMENT_KEYS
                    .iter()
                    .position(|e| e == element)
                    .unwrap();
                conjure(&mut session, key);
            }
            session.audio.clear();
            let mut captured = None;
            for i in 0..120 {
                for event in frame(
                    &mut session,
                    PlayerInput {
                        cast_down: i < hold_frames,
                        ..PlayerInput::default()
                    },
                ) {
                    if let SessionEvent::Avatar(AvatarEvent::SpellCast {
                        cast_type: CastType::Force,
                        spell_power,
                        ..
                    }) = event
                    {
                        assert!(captured.replace(spell_power).is_none(), "duplicate cast");
                        assert_eq!(session.avatar().character.spell_power, 0.0);
                    }
                }
            }
            let power = captured.expect("force cast did not occur");
            assert!(power > 0.0 && power <= 1.0, "power={power}");
            let matching: Vec<_> = session
                .audio
                .iter()
                .filter_map(|(request, _)| match request {
                    AudioRequest::Start {
                        bank,
                        cue: hash,
                        source,
                        magnitude,
                        ..
                    } if *hash == custom_hash(cue) => Some((*bank, *source, *magnitude)),
                    _ => None,
                })
                .collect();
            assert_eq!(
                matching,
                vec![(SPELLS, SoundSource::Global, Some(power * 3.0))]
            );
            powers.push(power);
        }
    }
    assert!(powers[0] < powers[1], "Earth charge must affect the cue");
    assert!(powers[2] < powers[3], "Ice charge must affect the cue");
}

fn cue_keys(s: &PlaySession, name: &str) -> Vec<u32> {
    let hash = magicka_runtime::scene_loading::custom_hash(name);
    s.audio
        .iter()
        .filter_map(|(r, _)| match r {
            magicka_runtime::audio::AudioRequest::Start { key, cue, .. } if *cue == hash => {
                Some(*key)
            }
            _ => None,
        })
        .collect()
}

/// Mines play spell_mine01 per mine; an Earth wall starts its loop cue per
/// wall and stops each one when the wall is removed.
#[test]
fn mine_and_barrier_casts_emit_their_cues() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, shield_key());
    conjure(&mut s, ARCANE);
    cast_held(&mut s, 30, false);
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(cue_keys(&s, "spell_mine01").len(), 4);
    s.audio.clear();
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, shield_key());
    conjure(&mut s, EARTH);
    cast_held(&mut s, 30, false);
    let loops = cue_keys(&s, "spell_earth_barrier");
    assert_eq!(loops.len(), 4);
    for _ in 0..1500 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.barriers.iter().next().is_none());
    let stopped: Vec<u32> = s
        .audio
        .iter()
        .filter_map(|(r, _)| match r {
            magicka_runtime::audio::AudioRequest::Stop { key, .. } => Some(*key),
            _ => None,
        })
        .collect();
    assert!(
        loops.iter().all(|k| stopped.contains(k)),
        "{loops:?} {stopped:?}"
    );
    assert!(!cue_keys(&s, "spell_earth_barrier_break").is_empty());
}

/// A Fire spray starts spell_fire_spray (Magnitude 1) on the caster; the
/// emitter follows the caster and the cue stops with the spray.
#[test]
fn spray_loop_cue_follows_and_stops() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, FIRE);
    cast_held(&mut s, 40, false);
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    let keys = cue_keys(&s, "spell_fire_spray");
    assert_eq!(keys.len(), 1, "one loop for one Fire element");
    let key = keys[0];
    let moved = s.audio.iter().any(|(r, _)| {
        matches!(r, magicka_runtime::audio::AudioRequest::Move { key: k, .. } if *k == key)
    });
    let stopped = s.audio.iter().any(|(r, _)| {
        matches!(r, magicka_runtime::audio::AudioRequest::Stop { key: k, .. } if *k == key)
    });
    assert!(moved && stopped);
    let magnitude = s.audio.iter().find_map(|(r, _)| match r {
        magicka_runtime::audio::AudioRequest::Start {
            key: k, magnitude, ..
        } if *k == key => *magnitude,
        _ => None,
    });
    assert_eq!(magnitude, Some(1.0));
}

/// S + Shift cast (RailGunSpell.CastArea): the Arcane FullBlast requests the
/// ArcaneBlast visual and starts its particle effect at the caster.
#[test]
fn arcane_area_cast_starts_the_arcane_blast_effect() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, ARCANE);
    let (mut requested, mut particles) = (false, 0usize);
    let mut arcane_particles = 0usize;
    let (mut discs, mut light) = (0usize, 0.0f32);
    let before = s
        .effects
        .particles
        .active()
        .iter()
        .filter(|g| g.radius > 0.0)
        .count();
    for i in 0..70 {
        frame(
            &mut s,
            PlayerInput {
                cast_down: i < 3,
                shift_down: i < 3,
                ..PlayerInput::default()
            },
        );
        requested |= s
            .beam_blasts
            .iter()
            .any(|&(_, _, e)| e & magicka_sim::spells::Elements::ARCANE.0 != 0);
        particles = particles.max(s.effects.active_effects());
        let live = s
            .effects
            .particles
            .active()
            .iter()
            .filter(|g| g.radius > 0.0)
            .count();
        arcane_particles = arcane_particles.max(live.saturating_sub(before));
        discs = discs.max(s.arcane_blasts.iter().map(|b| b.draws().len()).sum());
        light = s
            .dynamic_lights
            .iter()
            .map(|l| l.draw_intensity())
            .fold(light, f32::max);
    }
    assert!(requested, "ArcaneBlast visual requested");
    assert!(particles > 0, "beam_arcane_explosion effect started");
    assert!(
        arcane_particles >= 50,
        "beam_arcane_explosion particles emitted"
    );
    // ArcaneBlast RenderData: two discs for the single Arcane element.
    assert_eq!(discs, 2, "ArcaneBlast discs drawn");
    assert!(
        light > 1.0,
        "ArcaneBlast DynamicLight flashes (intensity {light})"
    );
    assert!(
        s.arcane_blasts.is_empty(),
        "ArcaneBlast dies after 0.4 s: {:?}",
        s.arcane_blasts
    );
}

/// GameScene.Initialize starts LevelModel.GetAllEffects: wc_s1 places 30
/// candle and 19 torch flames (plus dust, fireplace and checkpoint effects),
/// all of which emit particles.
#[test]
fn wizard_castle_level_effects_start_and_emit() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    let infinite = s.active_level_effects();
    assert!(infinite >= 50, "level effects active: {infinite}");
    let live = s
        .effects
        .particles
        .active()
        .iter()
        .filter(|g| g.radius > 0.0)
        .count();
    assert!(live > 200, "flame particles alive: {live}");
}

/// Wet then Cold freezes a goblin; Frozen stops movement, turning and
/// animation, and a Cold-only goblin's speed is capped (feedback 2026-10-08).
#[test]
fn wet_then_cold_freezes_and_cold_slows() {
    use magicka_sim::status::Status;
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let run = |elements: &[usize]| {
        let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
        s.disable_scripts();
        for _ in 0..60 {
            frame(&mut s, PlayerInput::default());
        }
        let goblin = spawn_in_front(&mut s, &root, "Goblin_miner", 4.0);
        for _ in 0..10 {
            frame(&mut s, PlayerInput::default());
        }
        for &element in elements {
            conjure(&mut s, element);
            for i in 0..75 {
                frame(
                    &mut s,
                    PlayerInput {
                        cast_down: i < 60,
                        ..PlayerInput::default()
                    },
                );
            }
        }
        (s, goblin)
    };
    let (mut s, goblin) = run(&[0, 3]);
    let n = &s.world.npcs[&goblin];
    assert!(n.character.combat.statuses.has(Status::Frozen), "frozen");
    assert!(
        !n.character.combat.statuses.has(Status::Wet),
        "wet consumed"
    );
    assert_eq!(s.status_emitters(goblin), ["statuseffect_frozen"]);
    let wet_keys: Vec<_> = s
        .audio
        .iter()
        .filter_map(|(request, _)| match request {
            magicka_runtime::audio::AudioRequest::Start {
                key, bank: 16, cue, ..
            } if *cue == magicka_runtime::scene_loading::custom_hash("spell_water_drip") => {
                Some(*key)
            }
            _ => None,
        })
        .collect();
    assert!(!wet_keys.is_empty());
    assert!(wet_keys
        .iter()
        .all(|wet| s.audio.iter().any(|(request, _)| matches!(request,
        magicka_runtime::audio::AudioRequest::Stop { key, instant: false } if key == wet))));
    assert!(s.audio.iter().any(|(request, _)| matches!(request,
        magicka_runtime::audio::AudioRequest::Start { bank: 16, cue, .. }
        if *cue == magicka_runtime::scene_loading::custom_hash("spell_ice_frozen"))));
    let before = s.world.physics.bodies[n.body].position;
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    let n = &s.world.npcs[&goblin];
    let body = &s.world.physics.bodies[n.body];
    assert_eq!(body.speed_multiplier(), 0.0);
    assert_eq!(n.motion.controller.speed, 0.0);
    let moved =
        ((body.position[0] - before[0]).powi(2) + (body.position[2] - before[2]).powi(2)).sqrt();
    assert!(moved < 1e-3, "frozen goblin moved {moved}");

    let (s, goblin) = run(&[3]);
    let n = &s.world.npcs[&goblin];
    let cold = n
        .character
        .combat
        .statuses
        .effect(Status::Cold)
        .expect("cold");
    let speed = s.world.physics.bodies[n.body].speed_multiplier();
    assert!(cold.magnitude > 0.0);
    assert!(
        s.status_emitters(goblin)
            .iter()
            .any(|n| n.starts_with("statuseffect_cold")),
        "{:?}",
        s.status_emitters(goblin)
    );
    assert!(
        (speed - cold.slowdown()).abs() < 1e-3 && speed < 1.0,
        "speed {speed} slowdown {}",
        cold.slowdown()
    );
}

/// LightningZapRenderData: a zapped goblin yields its generated skeleton
/// lines (balls and bone pyramids) inside its body bounds.
#[test]
fn zapped_goblin_skeleton_lines_stay_inside_the_body() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    let goblin = spawn_in_front(&mut s, &root, "Goblin_pirate", 4.0);
    frame(&mut s, PlayerInput::default());
    assert!(s.zapped_characters().iter().all(|z| z.handle != goblin));
    s.world
        .npcs
        .get_mut(&goblin)
        .unwrap()
        .character
        .combat
        .zap_timer = 0.05;
    let zapped = s.zapped_characters();
    let zap = zapped.iter().find(|z| z.handle == goblin).expect("zapped");
    let (radius, lines) = (&zap.radius, &zap.lines);
    let n = &s.world.npcs[&goblin];
    let body = &s.world.physics.bodies[n.body];
    let bones = n.rig.as_ref().unwrap().skeleton.skeleton.bones.len();
    // Per bone after the root: a 27-vertex ball (24 white segments); per bone
    // after the first: a 20-vertex pyramid (8 visible segments).
    assert_eq!(lines.len(), (bones - 1) * 24 + (bones - 2) * 8);
    for l in lines {
        for v in l {
            let dx = v[0] - body.position[0];
            let dz = v[2] - body.position[2];
            assert!((dx * dx + dz * dz).sqrt() < radius + 0.5, "{v:?}");
            assert!((v[1] - body.position[1]).abs() < 0.5 * body.length + body.radius + 0.5);
        }
    }
    // LightningHit flash: centred on Position, sized from the model's
    // Meshes[0] bounding sphere · 1.333 · scale, so the 0.85·radius quad
    // covers the body's collision extent.
    let flash = zap.flash.expect("goblin model bounding sphere");
    assert_eq!(flash.center, body.position);
    let half_height = 0.5 * body.length + body.radius;
    assert!(
        flash.radius * magicka_runtime::zap_skeleton::FLASH_SCALE > half_height.max(*radius)
            && flash.radius < 4.0 * half_height,
        "flash radius {} body half height {half_height}",
        flash.radius
    );
}

#[test]
fn earth_arcane_area_has_ground_burst_and_colored_flash_without_beam_discs() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, ARCANE);
    conjure(&mut s, EARTH);
    let mut radius = None;
    for _ in 0..90 {
        for event in frame(
            &mut s,
            PlayerInput {
                cast_down: true,
                shift_down: true,
                ..Default::default()
            },
        ) {
            if let SessionEvent::Avatar(AvatarEvent::GroundArea { radius: r, .. }) = event {
                radius = Some(r);
            }
        }
        if radius.is_some() {
            break;
        }
    }
    let radius = radius.expect("GroundArea did not cast");
    assert_eq!(
        s.effects
            .active_effects_by_hash(magicka_runtime::scene_loading::custom_hash(
                "area_ground_earth"
            )),
        // Combined Earth|Arcane damage plus Earth knockdown: eight each.
        16
    );
    assert!(
        s.arcane_blasts.is_empty(),
        "GroundBlast must not become a FullBlast disc"
    );
    let light = s.dynamic_lights.last().expect("missing GroundBlast light");
    assert_eq!(light.radius, radius * 2.0);
    assert_eq!(
        (light.dynamic_intensity, light.speed, light.specular),
        (6.0, 4.0, 1.0)
    );
    assert_eq!(
        light.diffuse,
        magicka_sim::lightning::spell_color(&magicka_sim::spells::Spell::default_for(E::ARCANE))
    );
    for _ in 0..20 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.dynamic_lights.iter().any(|l| l.draw_intensity() > 0.0));
    assert!(
        s.effects
            .particles
            .active()
            .iter()
            .filter(|p| p.sprite == 7 && p.normalized_lifetime < 1.0)
            .count()
            >= 48,
        "missing decoded ground rock particles"
    );
    for _ in 0..180 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.dynamic_lights.is_empty());
    assert_eq!(
        s.effects
            .active_effects_by_hash(magicka_runtime::scene_loading::custom_hash(
                "area_ground_earth"
            )),
        0
    );
}

/// ProjectileSpell.CastSelf: the Earth rock falls on its caster, which is
/// in the missile's hit list only until it expires (iCanHitOwner), and
/// hits once (feedback 2026-10-09: the self-cast rock missed the caster).
#[test]
fn earth_self_cast_rock_hits_its_caster_once() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, EARTH);
    let hp0 = s.avatar().character.combat.health.hit_points;
    let mut drops = Vec::new();
    let mut last = hp0;
    for i in 0..150 {
        frame(
            &mut s,
            PlayerInput {
                cast_self_down: i < 5,
                ..PlayerInput::default()
            },
        );
        let hp = s.avatar().character.combat.health.hit_points;
        if hp < last {
            drops.push(last - hp);
        }
        last = hp;
    }
    assert_eq!(drops.len(), 1, "{drops:?}");
    assert!(drops[0] > 0.0 && last > 0.0, "{drops:?}");
}

/// ShieldSpell.CastSelf (Shield + Fire): a self-casted Fire immunity aura
/// whose AuraBuff reaches the caster and a goblin in range, each drawn as a
/// BuffDefensive ground decal, with the AuraDefensive ring around the
/// caster (feedback 2026-10-09: no resistance projection rendered).
#[test]
fn shield_fire_self_cast_buffs_nearby_characters_with_decals() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let goblin = spawn_in_front(&mut s, &root, "Goblin_miner", 1.0);
    let far = spawn_in_front(&mut s, &root, "Goblin_miner", 8.0);
    for _ in 0..5 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, 2); // Shield
    conjure(&mut s, FIRE);
    for i in 0..40 {
        frame(
            &mut s,
            PlayerInput {
                cast_self_down: i < 5,
                ..PlayerInput::default()
            },
        );
    }
    let fire = |c: &magicka_runtime::character::RuntimeCharacter| {
        c.buffs
            .list
            .iter()
            .any(|b| b.resistance.against == magicka_sim::spells::Elements::FIRE)
    };
    assert_eq!(s.avatar().character.auras.len(), 1);
    assert!(fire(&s.avatar().character), "caster has no Fire buff");
    assert!(
        fire(&s.world.npcs[&goblin].character),
        "goblin in range unbuffed"
    );
    assert!(
        !fire(&s.world.npcs[&far].character),
        "goblin out of range buffed"
    );
    let cell = |c: u8| [(c % 8) as f32, (c / 8) as f32];
    let count = |c: u8| {
        s.world
            .decals
            .frame
            .iter()
            .filter(|p| p.args[..2] == cell(c) && p.args[2] > 0.0 && p.color[3] > 0.0)
            .count()
    };
    assert_eq!(count(magicka_runtime::buffs::BUFF_DEFENSIVE), 2);
    assert_eq!(count(magicka_runtime::buffs::AURA_DEFENSIVE), 6);

    // HaloAuraRenderData belongs to the aura caster, not every receiver of
    // its projected buff. The animated hip follows the owner's world pose.
    let halos = s.aura_halos();
    assert_eq!(halos.len(), 1);
    assert_eq!(halos[0].0, s.avatar().handle);
    assert_eq!(halos[0].2[3], 0.666);
    let original = halos[0].1;
    let body = s.avatar().body;
    let delta = [7.0, 2.0, -4.0];
    for (position, offset) in s.world.physics.bodies[body].position.iter_mut().zip(delta) {
        *position += offset;
    }
    let moved = s.aura_halos()[0].1;
    for i in 0..3 {
        assert!((moved[i] - original[i] - delta[i]).abs() < 1e-4);
    }
    for (position, offset) in s.world.physics.bodies[body].position.iter_mut().zip(delta) {
        *position -= offset;
    }
    // Template.Read's named-joint branches preempt hip index1. Its empty
    // constructor leaves the hip bind matrix zero in that case.
    let player = s.world.players[0].as_mut().unwrap();
    let original_rig = player.rig.clone();
    let rig = player.rig.as_mut().unwrap();
    std::sync::Arc::make_mut(&mut rig.skeleton).skeleton.bones[1].name = "RightAttach".into();
    let unassigned = s.aura_halos()[0].1;
    assert_eq!(
        unassigned,
        [0.0, 0.25 * s.body().length + 0.5 * s.body().radius, 0.0]
    );
    s.world.players[0].as_mut().unwrap().rig = original_rig;
    s.world.players[0].as_mut().unwrap().character.auras.clear();
    for _ in 0..120 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.aura_halos().is_empty());
}

/// Charge the sword with the conjured elements (Shift + cast press while
/// enchanting), then swing it (Shift + cast again). Returns the
/// SpellNotReconstructed reports and the number of ShieldWeapon events.
fn charge_and_swing_shield_weapon(s: &mut PlaySession, frames: usize) -> (Vec<String>, usize) {
    for i in 0..90 {
        frame(
            s,
            PlayerInput {
                shift_down: true,
                move_down: i == 0,
                ..Default::default()
            },
        );
    }
    let mut unsupported = Vec::new();
    let mut placed = 0;
    for i in 0..frames {
        for event in frame(
            s,
            PlayerInput {
                shift_down: true,
                move_down: i < 30,
                ..Default::default()
            },
        ) {
            match event {
                SessionEvent::Avatar(AvatarEvent::ShieldWeapon { .. }) => placed += 1,
                SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(r)) => unsupported.push(r),
                _ => {}
            }
        }
    }
    (unsupported, placed)
}

/// ShieldSpell.AnimationEnd (Weapon) with pure Shield: one WALL shield
/// (radius 5.5, 800 HP) 7.5 m past the caster's front, facing forward.
#[test]
fn shield_weapon_enchantment_raises_a_wall_ahead() {
    use magicka_sim::shield_entity::ShieldKind;
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, shield_key());
    let (unsupported, placed) = charge_and_swing_shield_weapon(&mut s, 60);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    assert_eq!(placed, 1);
    assert_eq!(s.world.shields.len(), 1);
    let wall = &s.world.shields[0].shield;
    assert_eq!(wall.kind, ShieldKind::Wall);
    assert_eq!(wall.radius, 5.5);
    let body = s.body();
    let d = body.direction();
    let ahead = (0..3)
        .map(|k| (wall.position[k] - body.position[k]) * d[k])
        .sum::<f32>();
    eprintln!("shield wall {ahead} m ahead");
    assert!(
        (ahead - (body.radius + 7.5)).abs() < 0.5,
        "wall {ahead} m ahead"
    );
}

/// Shield + Arcane weapon: a mine line starting 2.5 m past the caster's
/// front and stepping 2.5 m forward (3 slots for TotalMagnitude 2).
#[test]
fn shield_arcane_weapon_enchantment_lays_a_forward_mine_line() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, shield_key());
    conjure(&mut s, ARCANE);
    let (unsupported, placed) = charge_and_swing_shield_weapon(&mut s, 90);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    assert_eq!(placed, 1);
    let body = s.body();
    let d = body.direction();
    let mut ahead: Vec<f32> = mine_positions(&s)
        .iter()
        .map(|m| (m[0] - body.position[0]) * d[0] + (m[2] - body.position[2]) * d[2])
        .collect();
    ahead.sort_by(f32::total_cmp);
    eprintln!("mine line {ahead:?}");
    assert!(ahead.len() >= 2, "mines {ahead:?}");
    assert!(
        (ahead[0] - (body.radius + 2.5)).abs() < 0.3,
        "first mine {ahead:?}"
    );
    for pair in ahead.windows(2) {
        assert!((pair[1] - pair[0] - 2.5).abs() < 0.3, "spacing {ahead:?}");
    }
    // Every mine sits on the forward line.
    for m in mine_positions(&s) {
        let side = (m[0] - body.position[0]) * d[2] - (m[2] - body.position[2]) * d[0];
        assert!(side.abs() < 0.3, "off-line mine {m:?}");
    }
}

/// Shield + Earth weapon: a forward line of Earth barriers.
#[test]
fn shield_earth_weapon_enchantment_raises_a_forward_wall_line() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, shield_key());
    conjure(&mut s, EARTH);
    let (unsupported, placed) = charge_and_swing_shield_weapon(&mut s, 90);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    assert_eq!(placed, 1);
    let body = s.body();
    let d = body.direction();
    let walls: Vec<_> = s.world.barriers.iter().map(|(_, b)| b.clone()).collect();
    assert!(walls.len() >= 2, "{} walls", walls.len());
    assert!(walls.iter().all(|b| b.solid()));
    let mut ahead: Vec<f32> = walls
        .iter()
        .map(|b| {
            (b.position[0] - body.position[0]) * d[0] + (b.position[2] - body.position[2]) * d[2]
        })
        .collect();
    ahead.sort_by(f32::total_cmp);
    eprintln!("wall line {ahead:?}");
    assert!(
        (ahead[0] - (body.radius + 2.5)).abs() < 0.3,
        "first wall {ahead:?}"
    );
}

fn element_key(e: E) -> usize {
    magicka_runtime::avatar::ELEMENT_KEYS
        .iter()
        .position(|&k| k == e)
        .unwrap()
}

/// ProjectileSpell.CastWeapon with Ice grows an IceBlade (range
/// 1.25·Ice + 3.75 = 5 m for one Ice); AnimationEnd kills it and OnRemove
/// damages damageables crossing the forward 5 m segment (1 m wide).
#[test]
fn ice_weapon_enchantment_blade_cuts_along_its_range_once() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, element_key(E::WATER));
    conjure(&mut s, element_key(E::COLD));
    for i in 0..90 {
        frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                move_down: i == 0,
                ..Default::default()
            },
        );
    }
    let near = spawn_in_front(&mut s, &root, "Goblin_miner", 3.5);
    let far = spawn_in_front(&mut s, &root, "Goblin_miner", 9.0);
    let (near_hp, far_hp) = (npc_hp(&s, near), npc_hp(&s, far));
    let mut strikes = Vec::new();
    let mut unsupported = Vec::new();
    let hash = magicka_runtime::scene_loading::custom_hash;
    let (mut spawn_effects, mut death_effects) = (0, 0);
    for i in 0..90 {
        for event in frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                move_down: i < 30,
                ..Default::default()
            },
        ) {
            match event {
                SessionEvent::Avatar(AvatarEvent::IceBladeSpawned { .. }) => {
                    spawn_effects = s.effects.active_effects_by_hash(hash("weapon_ice_spawn"));
                }
                SessionEvent::Avatar(AvatarEvent::IceBladeStrike { range, .. }) => {
                    death_effects = s.effects.active_effects_by_hash(hash("weapon_ice_death"));
                    strikes.push(range)
                }
                SessionEvent::Avatar(AvatarEvent::SpellNotReconstructed(r)) => unsupported.push(r),
                _ => {}
            }
        }
    }
    assert!(unsupported.is_empty(), "{unsupported:?}");
    assert_eq!(strikes, [5.0]);
    // IceBlade.Initialize / OnRemove effects at the item orientation.
    assert_eq!((spawn_effects, death_effects), (1, 1));
    assert!(npc_hp(&s, near) < near_hp, "blade missed the near goblin");
    assert_eq!(npc_hp(&s, far), far_hp, "blade reached past its range");
}

/// Blast.GroundBlast with an Ice damage starts IceSpikes: spikes on the
/// ground of two rings (2·1.666·s and 1.666·s, s = radius/4.332) rise and
/// sink at ClipSpeed 2, then the effect is removed.
#[test]
fn ice_area_raises_ice_spikes_that_rise_and_sink() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, element_key(E::WATER));
    conjure(&mut s, element_key(E::COLD));
    let mut blast = None;
    for _ in 0..90 {
        for event in frame(
            &mut s,
            PlayerInput {
                cast_down: true,
                shift_down: true,
                ..Default::default()
            },
        ) {
            if let SessionEvent::Avatar(AvatarEvent::GroundArea {
                position, radius, ..
            }) = event
            {
                blast = Some((position, radius));
            }
        }
        if blast.is_some() {
            break;
        }
    }
    let (center, radius) = blast.expect("ice area ground blast");
    frame(&mut s, PlayerInput::default());
    assert_eq!(s.ice_spikes.len(), 1, "one Ice damage → one IceSpikes");
    let spikes = &s.ice_spikes[0].spikes;
    let scale = radius / magicka_sim::ice_spikes::SCALE_DIVISOR;
    assert!(spikes.len() >= 12, "{} spikes on the ground", spikes.len());
    for spike in spikes {
        let r = ((spike.position[0] - center[0]).powi(2) + (spike.position[2] - center[2]).powi(2))
            .sqrt();
        let outer = (r - 2.0 * 1.666 * scale).abs() < 1e-3;
        let inner = (r - 1.666 * scale).abs() < 1e-3;
        assert!(outer || inner, "spike at {r} m");
        assert!(spike.position[1] <= center[1] + 1.5);
    }
    let mut frames = 0;
    while !s.ice_spikes.is_empty() {
        frame(&mut s, PlayerInput::default());
        frames += 1;
        assert!(frames < 60, "spikes never sank");
    }
    // 0.25 s up + 0.25 s down at 60 Hz.
    assert!((27..=33).contains(&frames), "{frames} frames");
}

/// RailGunSpell.CastWeapon (Arcane + Fire sword): an ArcaneBlade of range
/// TotalMagnitude·0.5 + 1.5 = 2.5 rides the sword with its
/// `weapon_arcane_fire` effect, is killed when the weapon spell ends, fades
/// and is removed (stopping the effect).
#[test]
fn arcane_weapon_swing_draws_a_fading_arcane_blade() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, ARCANE);
    conjure(&mut s, FIRE);
    for i in 0..90 {
        frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                move_down: i == 0,
                ..Default::default()
            },
        );
    }
    let fire = magicka_runtime::scene_loading::custom_hash("weapon_arcane_fire");
    let (mut seen, mut faded, mut fire_peak, mut lit, mut light_faded) =
        (None, false, 0, false, false);
    for i in 0..150 {
        frame(
            &mut s,
            PlayerInput {
                shift_down: i < 40,
                move_down: i < 30,
                ..Default::default()
            },
        );
        let blades = &s.avatar().arcane_blades;
        assert!(blades.len() <= 1);
        if let Some(b) = blades.first() {
            seen.get_or_insert(b.max_range);
            faded |= b.dead && b.range < b.max_range;
            fire_peak = fire_peak.max(s.effects.active_effects_by_hash(fire));
            if let Some(light) = b.capsule_light() {
                lit = true;
                light_faded |= b.dead && light.intensity < 1.0;
                assert_eq!(light.start, b.vertices[0]);
                assert_eq!(light.end, b.vertices[1]);
                assert_eq!(light.radius, 4.0);
                assert_eq!(light.diffuse, b.color);
                assert_eq!(light.ambient, [0.0; 3]);
                assert!(light.intensity > 0.0 && light.intensity <= 1.0);
            }
        }
    }
    assert_eq!(seen, Some(2.5), "no ArcaneBlade drawn");
    assert!(faded, "blade never faded");
    assert!(lit && light_faded, "capsule light never lit and faded");
    assert!(s.avatar().arcane_blades.is_empty(), "blade never removed");
    assert_eq!(fire_peak, 1);
    assert_eq!(
        s.effects.active_effects_by_hash(fire),
        0,
        "effect not stopped"
    );
}

/// Item.Update (attached): a held item's definition effects run while it
/// is visible (Blade of Surt `flaming_sword_idle`, Healer's Burden
/// `epic__green_trail`), and every element of a charged weapon spell shows
/// its `swordelementeffect_<element>` until the charge is released.
#[test]
fn held_items_show_passive_and_enchantment_effects() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    s.debug_player_loadout("Wizard", "staff_healers_burden", "weapon_blade_of_surt")
        .unwrap();
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let hash = magicka_runtime::scene_loading::custom_hash;
    let count = |s: &PlaySession, name: &str| s.effects.active_effects_by_hash(hash(name));
    assert_eq!(count(&s, "flaming_sword_idle"), 1);
    assert_eq!(count(&s, "epic__green_trail"), 1);
    assert_eq!(count(&s, "swordelementeffect_fire"), 0);
    conjure(&mut s, FIRE);
    let mut charged = 0;
    for i in 0..90 {
        frame(
            &mut s,
            PlayerInput {
                shift_down: true,
                move_down: i == 0,
                ..Default::default()
            },
        );
        charged = charged.max(count(&s, "swordelementeffect_fire"));
    }
    assert_eq!(charged, 1, "no enchantment effect on the charged sword");
    for i in 0..90 {
        frame(
            &mut s,
            PlayerInput {
                shift_down: i < 40,
                move_down: i < 30,
                ..Default::default()
            },
        );
    }
    assert_eq!(
        count(&s, "swordelementeffect_fire"),
        0,
        "enchantment effect not stopped"
    );
    // Passive effects persist (restarted when they end).
    assert_eq!(count(&s, "flaming_sword_idle"), 1);
}

/// NPC-held items run their definition effects too: the goblin captain's
/// cleaver keeps `frostcleaver_idle` at its blade; the effect stops when the
/// goblin is removed.
#[test]
fn npc_held_items_show_their_passive_effects() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    let hash = magicka_runtime::scene_loading::custom_hash("frostcleaver_idle");
    let captain = spawn_in_front(&mut s, &root, "Goblin_warrior_captain", 6.0);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    let held = &s.world.npcs[&captain].held_effects;
    assert!(held
        .iter()
        .any(|h| h.effects.iter().any(|e| e == "frostcleaver_idle")));
    assert!(held.iter().all(|h| h.frame.is_some()));
    assert_eq!(s.effects.active_effects_by_hash(hash), 1);
    s.world.npcs.remove(&captain);
    frame(&mut s, PlayerInput::default());
    assert_eq!(s.effects.active_effects_by_hash(hash), 0);
}

/// User report: rocks flew through earth walls. A Solid barrier's
/// primitive-0 capsule is a missile collision target: the rock damages
/// the wall (Barrier.InternalDamage) and does not reach the goblin behind.
#[test]
fn earth_rock_damages_earth_walls_it_flies_through() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, shield_key());
    conjure(&mut s, EARTH);
    cast_held(&mut s, 30, false);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.barriers.iter().any(|(_, b)| b.solid()));
    conjure(&mut s, EARTH);
    let frames = 120;
    for i in 0..frames {
        frame(
            &mut s,
            PlayerInput {
                cast_down: i < 30,
                ..PlayerInput::default()
            },
        );
    }
    // Self-decay removes 10 HP steps; a rock hit leaves a fractional total.
    let hit = s
        .world
        .barriers
        .iter()
        .any(|(_, b)| (b.hit_points / 10.0).fract().abs() > 1e-3);
    assert!(hit, "the rock damaged a wall");
}

/// User report: Earth AoE did not hit enemies. Pure Earth Area delivers a
/// Knockdown slot; the goblin enters KnockDownState (hit_stun_begin, then
/// hit_stun_end) and gets back up.
#[test]
fn earth_area_knocks_a_goblin_down_and_it_gets_up() {
    use magicka_runtime::character_motion::CharacterState;
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let npc = spawn_in_front(&mut s, &root, "Goblin_miner", 3.0);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    conjure(&mut s, EARTH);
    let (mut down, mut up_after) = (false, false);
    for i in 0..360 {
        frame(
            &mut s,
            PlayerInput {
                cast_down: i < 30,
                shift_down: i < 30,
                ..PlayerInput::default()
            },
        );
        let state = s.world.npcs[&npc].motion.state;
        if state == CharacterState::KnockDown {
            down = true;
        } else if down {
            up_after = true;
        }
    }
    assert!(down, "the goblin was knocked down");
    assert!(up_after, "the goblin got back up");
}
