use magicka_runtime::{
    avatar::deliver_to_body,
    boss_action::{BossAction, BossCommand, BossSetup},
    boss_warlord::Phase,
    play_session::PlaySession,
    scene_loading::custom_hash,
    scene_world::boss_live::BossRef,
    script_runtime::ScriptWorld,
};
use magicka_sim::{
    damage::{attack, Damage},
    damage_delivery::DamagePayload,
    resistance::Resistance,
    spells::Elements as E,
};
use std::path::Path;
const DT: f32 = 1. / 60.;
fn session() -> Option<(PlaySession, u16)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "BattleField.lvl", "Wizard").unwrap();
    s.change_scene_named("bf_s4BF", "start").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    for _ in 0..60 {
        s.step(DT);
    }
    s.world
        .boss_action(
            &BossAction {
                delay: 0.,
                command: BossCommand::Spawn {
                    boss: "warlord".into(),
                    prop: false,
                    generic: false,
                    area: custom_hash("boss_spawn"),
                    unique_id: custom_hash("ignored_id"),
                    mesh: -1,
                    on_death: 0,
                    on_damage: 0,
                    setup: BossSetup {
                        delayed: true,
                        ..Default::default()
                    },
                },
            },
            false,
        )
        .unwrap()
        .unwrap();
    let BossRef::Warlord(b) = &s.world.boss_fight.bosses[0] else {
        panic!("specialized Warlord")
    };
    let handle = b.handle;
    Some((s, handle))
}
fn damage(s: &mut PlaySession, handle: u16, element: E) -> u16 {
    let npc = s.world.npcs.get_mut(&handle).unwrap();
    let body = &mut s.world.physics.bodies[npc.body];
    let behind = std::array::from_fn(|i| body.position[i] - body.direction()[i] * 2.);
    deliver_to_body(
        &mut npc.character,
        body,
        &[Resistance::IDENTITY; 11],
        false,
        handle,
        DamagePayload::Single(Damage {
            attack: attack::DAMAGE,
            element: element.0,
            amount: 100.,
            magnitude: 3.,
        }),
        None,
        behind,
        0.,
    )
}
#[test]
fn named_warlord_has_fixed_identity_and_explicit_kill_death_phase() {
    let Some((mut s, h)) = session() else { return };
    let locator =
        magicka_runtime::ai_event::AiEventLevel::locator(&s.world, custom_hash("boss_spawn"))
            .unwrap();
    let ground = s
        .world
        .collision
        .collision()
        .segment_intersect(
            magicka_sim::spray_pulse::Segment {
                origin: [locator[12], locator[13] + 1., locator[14]],
                delta: [0., -6., 0.],
            },
            false,
        )
        .unwrap()
        .hit
        .position;
    let n = &s.world.npcs[&h];
    assert!(
        (s.world.physics.bodies[n.body].position[1] - ground[1] - 5.1).abs() < 1e-5,
        "fresh Warlord placement uses constructor capsule dimensions before ApplyTemplate"
    );
    let e = s.world.entities.iter().find(|e| e.handle == h).unwrap();
    assert_eq!(e.unique_id, custom_hash("#boss_n06"));
    assert!(
        s.world.npcs[&h]
            .character
            .combat
            .cannot_die_without_explicit_kill
    );
    s.world
        .boss_action(
            &BossAction {
                delay: 0.,
                command: BossCommand::Activate,
            },
            false,
        )
        .unwrap()
        .unwrap();
    s.step(DT);
    assert_eq!(s.world.npcs[&h].react_to, 3);
    s.world
        .npcs
        .get_mut(&h)
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 0.;
    s.step(DT);
    let BossRef::Warlord(b) = &s.world.boss_fight.bosses[0] else {
        panic!()
    };
    assert_eq!(b.phase, Phase::Death);
    assert_eq!(s.world.npcs[&h].motion.special_idle, 8);
    assert!(!s.world.boss_fight.dead);
}
#[test]
fn blocking_absorbs_magic_from_behind_and_deflects_physical_without_health_loss() {
    let Some((mut s, h)) = session() else { return };
    s.world.npcs.get_mut(&h).unwrap().character.block_item = 0;
    let before = s.world.npcs[&h].character.combat.health.hit_points;
    let expected = magicka_sim::character_impact::result::DEFLECTED;
    assert_eq!(damage(&mut s, h, E::ARCANE | E::FIRE), expected);
    let queue = s.world.npcs[&h]
        .character
        .combat
        .absorb_shield
        .as_ref()
        .unwrap();
    assert_eq!(queue.queue().len(), 1);
    let spell = queue.peek();
    assert_eq!(spell.element, E::ARCANE | E::FIRE);
    assert_eq!(spell.mag[3], 0.5);
    assert_eq!(spell.mag[5], 0.5);
    assert_eq!(damage(&mut s, h, E::EARTH), expected);
    assert_eq!(damage(&mut s, h, E::EARTH | E::ARCANE), expected);
    assert_eq!(s.world.npcs[&h].character.combat.health.hit_points, before);
    assert_eq!(
        s.world.npcs[&h]
            .character
            .combat
            .absorb_shield
            .as_ref()
            .unwrap()
            .queue()
            .len(),
        1
    );
    s.world.npcs.get_mut(&h).unwrap().character.block_item = -1;
    damage(&mut s, h, E::ARCANE);
    assert!(s.world.npcs[&h].character.combat.health.hit_points < before);
}
#[test]
fn shield_queue_is_bounded_and_bash_releases_through_live_area_damage() {
    let Some((mut s, h)) = session() else { return };
    s.world.npcs.get_mut(&h).unwrap().character.block_item = 0;
    for _ in 0..8 {
        damage(&mut s, h, E::ARCANE);
    }
    assert_eq!(
        s.world.npcs[&h]
            .character
            .combat
            .absorb_shield
            .as_ref()
            .unwrap()
            .queue()
            .len(),
        5
    );
    s.step(DT);
    let arcane_chant = custom_hash("swordelementeffect_arcane");
    assert_eq!(s.effects.active_effects_by_hash(arcane_chant), 1);
    let player_body = s.avatar().body;
    let player_hp = s.avatar().character.combat.health.hit_points;
    let npc = s.world.npcs.get_mut(&h).unwrap();
    let body = &s.world.physics.bodies[npc.body];
    let target = std::array::from_fn(|i| body.position[i] + body.direction()[i]);
    // Use the template's actual Bash animation and its DealDamage window.
    npc.motion
        .attack(npc.combat.abilities[0].animations[0], true);
    s.world.physics.bodies[player_body].position = target;
    s.world.physics.bodies[player_body].velocity = [0.; 3];
    let mut released = false;
    for _ in 0..180 {
        s.step(DT);
        let n = &s.world.npcs[&h];
        if n.character
            .combat
            .absorb_shield
            .as_ref()
            .unwrap()
            .queue()
            .is_empty()
        {
            released = true;
            break;
        }
    }
    assert!(released, "shield melee must release the absorbed queue");
    assert_eq!(
        s.effects.active_effects_by_hash(arcane_chant),
        0,
        "native Bash release must stop its stored-element chant in that frame"
    );
    assert!(s.events.iter().any(|event| matches!(event,
        magicka_runtime::play_session::SessionEvent::NpcSpell { owner, event:
            magicka_runtime::avatar::AvatarEvent::SpellCast { spell, cast_type: magicka_sim::damage::CastType::Area, .. } }
        if *owner == h && spell.element == E::ARCANE)));
    assert!(s.events.iter().any(|event| matches!(event,
        magicka_runtime::play_session::SessionEvent::NpcSpell { owner, event:
            magicka_runtime::avatar::AvatarEvent::BeamArea { .. } } if *owner == h)));
    assert!(s.avatar().character.combat.health.hit_points < player_hp);
}
#[test]
fn warlord_overkill_stops_animation_and_preserves_explicit_kill_gate() {
    let Some((mut s, h)) = session() else { return };
    s.world.overkill(h);
    let n = &s.world.npcs[&h];
    assert_eq!(n.character.combat.health.hit_points, -1.);
    assert!(!n.character.combat.dead && n.character.combat.overkilled);
    assert!(n.character.combat.cannot_die_without_explicit_kill);
    assert!(!n.motion.controller.is_playing());
}

#[test]
fn absorbed_elements_chant_at_the_shield_and_stop_when_released_or_removed() {
    let Some((mut s, h)) = session() else { return };
    s.world.npcs.get_mut(&h).unwrap().character.block_item = 0;
    damage(&mut s, h, E::ARCANE | E::FIRE);
    s.step(DT);
    let fire = custom_hash("swordelementeffect_fire");
    let arcane = custom_hash("swordelementeffect_arcane");
    for hash in [fire, arcane] {
        let effects = s.effects.presentation();
        let matching: Vec<_> = effects.iter().filter(|e| e.definition == hash).collect();
        assert_eq!(
            matching.len(),
            1,
            "each absorbed element must have one chant"
        );
        let frame = s.world.npcs[&h]
            .held_effects
            .iter()
            .find(|item| item.slot == 0)
            .unwrap()
            .frame
            .unwrap();
        assert_eq!(matching[0].transform, frame.chant_transform());
    }
    let refs: Vec<_> = s
        .effects
        .presentation()
        .into_iter()
        .filter(|e| [fire, arcane].contains(&e.definition))
        .map(|e| e.reference)
        .collect();
    for _ in 0..20 {
        s.step(DT);
    }
    assert!(
        refs.iter().all(|reference| s.effects.is_active(reference)),
        "ordinary updates must retain references rather than accumulating emitters"
    );
    for hash in [fire, arcane] {
        assert_eq!(s.effects.active_effects_by_hash(hash), 1);
    }
    // HideEffects / invisibility gate passive effects, not attached queue chants.
    {
        let n = s.world.npcs.get_mut(&h).unwrap();
        n.character.combat.health.invisibility_timer = 10.;
        n.held_effects
            .iter_mut()
            .find(|item| item.slot == 0)
            .unwrap()
            .hide = true;
    }
    s.step(DT);
    for hash in [fire, arcane] {
        assert_eq!(s.effects.active_effects_by_hash(hash), 1);
    }
    let npc = s.world.npcs.get_mut(&h).unwrap();
    npc.character
        .combat
        .absorb_shield
        .as_mut()
        .unwrap()
        .retrieve(&mut Vec::new());
    s.step(DT);
    for hash in [fire, arcane] {
        assert_eq!(s.effects.active_effects_by_hash(hash), 0);
    }
    s.world.npcs.get_mut(&h).unwrap().character.block_item = 0;
    damage(&mut s, h, E::POISON);
    s.step(DT);
    let cold = custom_hash("swordelementeffect_cold");
    assert_eq!(
        s.effects.active_effects_by_hash(cold),
        1,
        "Poison uses native cold chant entry"
    );
    s.world.npcs.remove(&h);
    s.step(DT);
    assert_eq!(s.effects.active_effects_by_hash(cold), 0);
}
