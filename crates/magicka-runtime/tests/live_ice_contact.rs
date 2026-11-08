use magicka_runtime::{
    character_contact::{self, ContactActor},
    play_content::load_start_scene,
    play_session::PlaySession,
    scene_script::{ActionDefinition, SceneScriptLayout},
    spawn_action::SpawnState,
};
use magicka_sim::{
    character_health::SelfShield,
    resistance::Resistance,
    spells::{Elements as E, Spell},
};
use std::path::PathBuf;

fn scene() -> Option<(PlaySession, u16)> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    if !root.is_dir() {
        return None;
    }
    let mut session = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    session.disable_scripts();
    let area = load_start_scene(&root, "Havindr.lvl")
        .unwrap()
        .start
        .area
        .unwrap();
    let xml = format!(
        r#"<Scene><trigger id="t"><then><spawn type="Goblin_miner" area="{area}1" order="idle"/></then></trigger></Scene>"#
    );
    let ActionDefinition::Spawn(action) = SceneScriptLayout::read(xml.as_bytes(), false)
        .unwrap()
        .actions
        .remove(0)
    else {
        panic!()
    };
    let mut state = SpawnState::default();
    action.initialize(&mut state, &mut session.world).unwrap();
    action.execute(&mut state, &mut session.world).unwrap();
    Some((session, state.spawned[0].handle.unwrap()))
}
fn shield(ice: f32) -> SelfShield {
    let mut spell = Spell::default_for(E::SHIELD | E::ICE);
    spell.mag[E::ICE.index() as usize] = ice;
    SelfShield::from_spell(&spell, &mut vec![])
}

fn arcane_hit() -> magicka_sim::damage::Damage {
    magicka_sim::damage::Damage {
        attack: magicka_sim::damage::attack::DAMAGE,
        element: E::ARCANE.0,
        amount: 100.0,
        magnitude: 1.0,
    }
}

#[test]
fn avatar_event_immunity_tracks_null_empty_pending_and_completed_lists() {
    use magicka_runtime::ai_event::{AiEvent, FaceEvent};
    let Some((mut s, handle)) = scene() else {
        return;
    };
    let body_index = s.avatar().body;
    let player = s.world.players[0].as_mut().unwrap();
    let body = &mut s.world.physics.bodies[body_index];
    player.character.definition.resistances = [Resistance::IDENTITY; 11];
    for (events, current, immune) in [
        (None, 0, false),
        (Some(vec![]), 0, false),
        (Some(vec![AiEvent::Face(FaceEvent::default())]), 0, true),
        (Some(vec![AiEvent::Face(FaceEvent::default())]), 1, false),
    ] {
        player.set_events(body, events);
        player.ai.current_event = current;
        player.ai.event_delay = f32::NAN;
        player.character.combat.health.hit_points = 1000.0;
        player.receive_damage(body, arcane_hit(), handle, [0.0; 3], 0.0);
        assert_eq!(player.character.is_immortal(), immune);
        assert_eq!(
            player.character.combat.health.hit_points,
            if immune { 1000.0 } else { 900.0 }
        );
    }
    player.set_events(body, Some(vec![AiEvent::Face(FaceEvent::default())]));
    player.character.combat.health.hit_points = 1000.0;
    player.apply_fall_damage(body, 100.0, false, 0.0);
    assert_eq!(player.character.combat.health.hit_points, 1000.0);
    player
        .character
        .combat
        .statuses
        .set_effect(magicka_sim::status::StatusEffect::new(
            magicka_sim::status::Status::Bleeding,
            100.0,
            1.0,
            false,
        ))
        .unwrap();
    player.character.update_damage_and_status(
        0.01,
        true,
        magicka_sim::character_health::StatusUpdateContext::default(),
        &mut vec![],
    );
    assert_eq!(player.character.combat.health.hit_points, 1000.0);
}

#[test]
fn cutscene_hooks_protect_avatar_immediately_and_leave_npc_damage_enabled() {
    use magicka_runtime::scene_command::SceneCommandHost;
    use magicka_sim::damage_delivery::DamagePayload;
    let Some((mut s, handle)) = scene() else {
        return;
    };
    s.world.begin_cutscene(0, false, false).unwrap();
    assert!(s.avatar().character.is_immortal());
    let body_index = s.avatar().body;
    let player = s.world.players[0].as_mut().unwrap();
    player.character.definition.resistances = [Resistance::IDENTITY; 11];
    player.character.combat.health.hit_points = 1000.0;
    player.receive_payload(
        &mut s.world.physics.bodies[body_index],
        DamagePayload::Single(arcane_hit()),
        Some(handle),
        [0.0; 3],
        0.0,
    );
    assert_eq!(player.character.combat.health.hit_points, 1000.0);
    let npc = s.world.npcs.get_mut(&handle).unwrap();
    npc.character.definition.resistances = [Resistance::IDENTITY; 11];
    npc.character.combat.health.hit_points = 1000.0;
    npc.character.definition.max_hit_points = 1000.0;
    npc.character.combat.health.max_hit_points = 1000.0;
    assert!(!npc.character.is_immortal());
    magicka_runtime::avatar::deliver_to_body(
        &mut npc.character,
        &mut s.world.physics.bodies[npc.body],
        &[Resistance::IDENTITY; 11],
        false,
        handle,
        DamagePayload::Single(arcane_hit()),
        None,
        [0.0; 3],
        0.0,
    );
    assert_eq!(npc.character.combat.health.hit_points, 900.0);
    s.world.end_cutscene(false).unwrap();
    assert!(!s.avatar().character.is_immortal());
    let player = s.world.players[0].as_mut().unwrap();
    player.receive_damage(
        &mut s.world.physics.bodies[body_index],
        arcane_hit(),
        handle,
        [0.0; 3],
        0.0,
    );
    assert_eq!(player.character.combat.health.hit_points, 900.0);
}

#[test]
fn live_cutscene_ice_contact_records_hit_without_hurting_avatar() {
    let Some((mut s, handle)) = scene() else {
        return;
    };
    let player_handle = s.avatar().handle;
    let player = s.world.players[0].as_mut().unwrap();
    player.character.combat.health.hit_points = 1000.0;
    player.character.definition.regeneration = 0;
    s.world
        .npcs
        .get_mut(&handle)
        .unwrap()
        .character
        .combat
        .health
        .shield = shield(1.0);
    reset_bodies(&mut s, handle);
    // A raw frame flag change must be reflected before physics delivery.
    s.world.flags.cutscene = true;
    s.world.step_live(None, 1.0 / 60.0).unwrap();
    assert_eq!(s.avatar().character.combat.health.hit_points, 1000.0);
    assert!(s.avatar().character.is_immortal());
    assert!(s.world.npcs[&handle]
        .character
        .collision_hits
        .contains(player_handle));
    assert!(!s.world.physics.contact_points(s.avatar().body).is_empty());
}
fn reset_bodies(s: &mut PlaySession, npc: u16) {
    let indices = [s.avatar().body, s.world.npcs[&npc].body];
    for (i, index) in indices.into_iter().enumerate() {
        let b = &mut s.world.physics.bodies[index];
        b.apply_gravity = false;
        b.add_external_forces(0.3, None);
        b.position = [i as f32 * 0.4, 100.0, 0.0];
        b.velocity = [10.0, 0.0, 0.0];
    }
}

#[test]
fn live_ice_hits_through_public_damage_then_suppresses_repeat_even_if_ethereal() {
    for (ethereal, ignore_time) in [(false, 0.0), (true, 0.0), (false, 0.05)] {
        let Some((mut s, handle)) = scene() else {
            return;
        };
        let player_handle = s.avatar().handle;
        let player = s.world.players[0].as_mut().unwrap();
        player.character.combat.health.shield = shield(1.0);
        player.character.ethereal = ethereal;
        player.character.collision_ignore_time = ignore_time;
        let npc = s.world.npcs.get_mut(&handle).unwrap();
        npc.character.definition.resistances = [Resistance::IDENTITY; 11];
        npc.character.definition.regeneration = 0;
        npc.character.definition.bleed_rate = 0.0;
        npc.character.combat.health.hit_points = 10000.0;
        npc.character.definition.max_hit_points = 10000.0;
        npc.character.combat.health.max_hit_points = 10000.0;
        reset_bodies(&mut s, handle);
        s.world.step_live(None, 1.0 / 60.0).unwrap();
        assert_eq!(
            s.world.npcs[&handle].character.combat.health.hit_points,
            9725.0
        );
        assert!(s.avatar().character.collision_hits.contains(handle));
        assert!(
            (s.avatar()
                .character
                .collision_hits
                .remaining(handle)
                .unwrap()
                - (0.25 - 1.0 / 60.0))
                .abs()
                < 1e-6
        );
        assert!(s.world.npcs[&handle]
            .combat
            .attacked_by
            .iter()
            .any(|a| a.0 == player_handle && a.1 > 0.0));
        assert_eq!(
            !s.world.physics.contact_points(s.avatar().body).is_empty(),
            !ethereal && ignore_time <= 0.0
        );
        reset_bodies(&mut s, handle);
        s.world.step_live(None, 1.0 / 60.0).unwrap();
        assert_eq!(
            s.world.npcs[&handle].character.combat.health.hit_points,
            9725.0
        );
        assert!(
            (s.avatar()
                .character
                .collision_hits
                .remaining(handle)
                .unwrap()
                - (0.25 - 2.0 / 60.0))
                .abs()
                < 1e-6
        );
        s.world.players[0]
            .as_mut()
            .unwrap()
            .character
            .collision_hits
            .update(0.25);
        reset_bodies(&mut s, handle);
        s.world.step_live(None, 1.0 / 60.0).unwrap();
        assert_eq!(
            s.world.npcs[&handle].character.combat.health.hit_points,
            9450.0
        );
    }
}

#[test]
fn immortal_animation_refreshes_timers_before_kill_for_player_and_npc() {
    use magicka_content::character::{ActionKind, AnimationAction, AnimationClipAction};
    use magicka_runtime::{animation_controller::AnimationSet, character_motion::CharacterMotion};
    use std::{collections::HashMap, sync::Arc};
    let Some((mut s, handle)) = scene() else {
        return;
    };
    let animations = Arc::new(AnimationSet::new(
        &[AnimationClipAction {
            animation: "idle".into(),
            clip: "fixture".into(),
            speed: 1.0,
            blend_time: 0.0,
            looping: false,
            actions: vec![AnimationAction {
                start_time: 0.0,
                end_time: 0.05,
                kind: ActionKind::Immortal { collide: false },
            }],
        }],
        HashMap::from([("fixture".into(), 1.0)]),
    ));
    s.world.players[0].as_mut().unwrap().motion = CharacterMotion::new(animations.clone());
    s.world.npcs.get_mut(&handle).unwrap().motion = CharacterMotion::new(animations);
    for _ in 0..2 {
        s.world.update_players(0.01);
        s.world.update_npcs(0.01);
        for character in [&s.avatar().character, &s.world.npcs[&handle].character] {
            assert_eq!(character.immortal_time, 3.0);
            assert_eq!(character.collision_ignore_time, 0.15);
        }
    }
    let body = s.avatar().body;
    let player = s.world.players[0].as_mut().unwrap();
    let before = player.character.combat.health.hit_points;
    player.receive_damage(
        &mut s.world.physics.bodies[body],
        magicka_sim::damage::Damage {
            attack: magicka_sim::damage::attack::DAMAGE,
            element: E::EARTH.0,
            amount: 100.0,
            magnitude: 1.0,
        },
        handle,
        [0.0; 3],
        0.0,
    );
    assert_eq!(player.character.combat.health.hit_points, before);
    player
        .character
        .combat
        .statuses
        .set_effect(magicka_sim::status::StatusEffect::new(
            magicka_sim::status::Status::Bleeding,
            100.0,
            1.0,
            false,
        ))
        .unwrap();
    player.character.update_damage_and_status(
        0.01,
        true,
        magicka_sim::character_health::StatusUpdateContext::default(),
        &mut vec![],
    );
    assert_eq!(player.character.combat.health.hit_points, before);
    s.world.update_players(0.1);
    s.world.update_npcs(0.1);
    for character in [&s.avatar().character, &s.world.npcs[&handle].character] {
        assert_eq!(character.immortal_time, 0.0);
        assert!((character.collision_ignore_time - 0.05).abs() < 1e-6);
    }
}

#[test]
fn ignore_timer_boundaries_and_nullification_keep_native_order() {
    let Some((s, _)) = scene() else {
        return;
    };
    for (timer, accepted) in [(0.0, true), (-1.0, true), (0.001, false), (f32::NAN, true)] {
        let mut a = s.avatar().character.clone();
        let mut b = a.clone();
        a.collision_ignore_time = timer;
        let mut body_a = s.body().clone();
        let mut body_b = body_a.clone();
        body_b.position[0] += 1.0;
        assert_eq!(a.accepts_owned_contact(), accepted);
        let pure = || SelfShield::from_spell(&Spell::default_for(E::SHIELD), &mut vec![]);
        a.combat.health.shield = pure();
        b.combat.health.shield = pure();
        let (physical, effect) = magicka_runtime::character_shield::pair(
            [&mut a, &mut b],
            [&mut body_a, &mut body_b],
            [false; 2],
        );
        assert_eq!(physical, accepted);
        assert!(effect.is_some());
        assert!(!a.combat.health.shield.active());
        assert!(!b.combat.health.shield.active());
    }
    let mut a = s.avatar().character.clone();
    a.execute_immortal_action(false);
    a.execute_immortal_action(true);
    assert_eq!(a.collision_ignore_time, 0.15);
    a.kill_immortal_action();
    assert_eq!(a.collision_ignore_time, 0.15);
    a.update_contact_timers(0.25);
    assert!(a.collision_ignore_time < 0.0);
    assert_eq!(a.immortal_time, -0.25);
}

#[test]
fn ordinary_lethal_hit_still_allows_second_callback_before_death_state_update() {
    let Some((mut s, handle)) = scene() else {
        return;
    };
    reset_bodies(&mut s, handle);
    let player_handle = s.avatar().handle;
    let player_body = s.avatar().body;
    let npc_body = s.world.npcs[&handle].body;
    let mut combatants = s.world.combatants();
    combatants.sort_by_key(|c| c.handle != player_handle);
    let player = s.world.players[0].as_mut().unwrap();
    player.character.combat.health.shield = shield(1.0);
    player.character.combat.health.shield.health = 0.0;
    player.character.combat.health.hit_points = 1000.0;
    player.character.definition.max_hit_points = 1000.0;
    player.character.combat.health.max_hit_points = 1000.0;
    player.character.definition.resistances = [Resistance::IDENTITY; 11];
    let npc = s.world.npcs.get_mut(&handle).unwrap();
    npc.character.combat.health.shield = shield(1.0);
    npc.character.combat.health.shield.health = 0.0;
    npc.character.combat.health.hit_points = 100.0;
    npc.character.definition.max_hit_points = 100.0;
    npc.character.combat.health.max_hit_points = 100.0;
    npc.character.definition.resistances = [Resistance::IDENTITY; 11];
    let (before, after) = s.world.physics.bodies.split_at_mut(npc_body);
    let mut a = ContactActor::Player(player);
    let mut b = ContactActor::Npc(npc);
    let (accepted, _) = character_contact::pair(
        [&mut a, &mut b],
        [&mut before[player_body], &mut after[0]],
        [&combatants[0], &combatants[1]],
        |_, _, _| {},
        &mut vec![],
    )
    .unwrap();
    assert!(accepted);
    assert!(b.character().combat.health.hit_points <= 0.0);
    assert!(!b.dead());
    assert!(b.character().collision_hits.contains(player_handle));
    assert!(a.character().collision_hits.contains(handle));
    assert!(a.character().combat.health.hit_points < 1000.0);
}

#[test]
fn overkill_in_first_callback_prevents_second_ice_attack_and_clears_victim_hits() {
    let Some((mut s, handle)) = scene() else {
        return;
    };
    reset_bodies(&mut s, handle);
    let attacker_handle = s.avatar().handle;
    let player_body = s.avatar().body;
    let npc_body = s.world.npcs[&handle].body;
    let mut combatants = s.world.combatants();
    combatants.sort_by_key(|c| c.handle != attacker_handle);
    let player = s.world.players[0].as_mut().unwrap();
    player.character.combat.health.shield = shield(4.0);
    let npc = s.world.npcs.get_mut(&handle).unwrap();
    npc.character.combat.health.shield = shield(1.0);
    npc.character.combat.health.shield.health = -10000.0;
    npc.character.combat.health.hit_points = 1.0;
    npc.character.definition.max_hit_points = 1.0;
    npc.character.combat.health.max_hit_points = 1.0;
    npc.character.definition.resistances = [Resistance::IDENTITY; 11];
    npc.character.collision_hits.add(999);
    npc.on_damage = 123;
    let initial_player_health = player.character.combat.health.hit_points;
    let (before, after) = s.world.physics.bodies.split_at_mut(npc_body);
    let mut a = ContactActor::Player(player);
    let mut b = ContactActor::Npc(npc);
    let mut triggers = vec![];
    let mut notifications = 0;
    let (accepted, _) = character_contact::pair(
        [&mut a, &mut b],
        [&mut before[player_body], &mut after[0]],
        [&combatants[0], &combatants[1]],
        |_, _, _| notifications += 1,
        &mut triggers,
    )
    .unwrap();
    assert!(!accepted);
    assert!(b.dead());
    assert!(!b.character().collision_hits.contains(attacker_handle));
    assert!(!b.character().collision_hits.contains(999));
    assert!(a.character().collision_hits.contains(handle));
    assert_eq!(
        a.character().combat.health.hit_points,
        initial_player_health
    );
    assert_eq!(notifications, 1);
    assert_eq!(triggers, [(123, Some(handle))]);
}
