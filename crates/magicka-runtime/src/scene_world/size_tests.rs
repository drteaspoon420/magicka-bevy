use super::*;
use magicka_sim::{size_magicks::SizeKind as K, spells::MagickType as M};

#[test]
fn live_size_body_buffs_and_opposite_cast_follow_the_recovered_order() {
    let Some((mut s, npc)) = damage_boost_tests::session() else {
        return;
    };
    let handle = s.avatar().handle;
    let index = s.avatar().body;
    let template = s.avatar().character.definition.clone();
    let feet = s.world.physics.bodies[index].feet();
    s.world.cast_magick(0, M::Grow);
    assert_eq!(s.world.size_effects.active(handle).unwrap().ttl, 20.0);
    s.world.update_ability_effects(1.0);
    let body = &s.world.physics.bodies[index];
    assert_eq!(body.feet(), feet);
    assert!((body.length - template.length * 2.0).abs() < 1e-5);
    assert!((body.radius - template.radius * 2.0).abs() < 1e-5);
    assert!((body.mass - template.mass - 750.0).abs() < 1e-4);
    assert_eq!(body.velocity, [0.0; 3]);
    assert!((body.speed_multiplier() - 0.5).abs() < 1e-5);
    assert_eq!(s.avatar().character.combat.health.hit_points, 1000.0);
    assert_eq!(s.avatar().character.buffs.damage_boosts()[0].magnitude, 3.0);
    s.world.update_auras_and_buffs(0.01, false);
    assert_eq!(s.avatar().character.combat.health.max_hit_points, 2000.0);
    // Same-kind recast preserves animation, IDs and the retained aura clocks.
    let starts = s.world.effect_commands.len();
    let audio = s.world.audio_requests.len();
    let ids = s.avatar().character.buffs.nonvisual.clone();
    s.world.cast_magick(0, M::Grow);
    assert_eq!(s.world.effect_commands.len(), starts);
    assert_eq!(s.world.audio_requests.len(), audio);
    assert_eq!(
        s.world.size_effects.active(handle).unwrap().animation_time,
        1.0
    );
    s.world.cast_magick(0, M::Shrink);
    assert_eq!(s.world.effect_commands.len(), starts);
    assert_eq!(s.world.audio_requests.len(), audio + 1);
    assert_eq!(s.world.size_effects.active(handle).unwrap().kind, K::Grow);
    s.world.update_ability_effects(1.0);
    assert!(s.world.size_effects.active(handle).is_none());
    assert_eq!(s.world.physics.bodies[index].length, template.length);
    assert_eq!(s.avatar().character.model_scale, 1.0);
    assert_eq!(s.avatar().character.buffs.nonvisual[0].id, ids[0].id);
    assert_eq!(
        s.avatar().character.buffs.nonvisual[0].ttl,
        0.5,
        "final update refreshes auras"
    );
    // OnRemove leaves the buffs to expire on their own Character.Update.
    s.world.update_auras_and_buffs(0.5, false);
    assert_eq!(s.avatar().character.combat.health.max_hit_points, 2000.0);
    s.world.update_auras_and_buffs(0.0, false);
    assert_eq!(s.avatar().character.combat.health.max_hit_points, 1000.0);
    assert!(s.avatar().character.buffs.damage_boosts().is_empty());
    assert!(
        s.world.npcs[&npc].character.buffs.nonvisual.is_empty(),
        "self auras never reach neighbours"
    );
}

#[test]
fn npc_health_insertion_and_cache_reuse_keep_ids_and_quarter_second_period() {
    let Some((mut s, npc)) = damage_boost_tests::session() else {
        return;
    };
    s.world
        .npcs
        .get_mut(&npc)
        .unwrap()
        .character
        .set_ethereal(true);
    assert!(s.world.size_magick(npc, K::Shrink));
    s.world.update_ability_effects(0.1);
    let c = &s.world.npcs[&npc].character;
    assert_eq!(
        c.combat.health.hit_points, 500.0,
        "self aura accepts ethereal owner"
    );
    let ids: Vec<_> = c.buffs.nonvisual.iter().map(|b| b.id).collect();
    s.world.size_magick(npc, K::Shrink);
    s.world.update_ability_effects(0.1);
    assert_eq!(
        s.world.npcs[&npc].character.combat.health.hit_points, 500.0,
        "refresh does not halve again"
    );
    s.world.nullify();
    assert!(s.world.size_effects.active(npc).is_none());
    s.world.update_auras_and_buffs(0.5, false);
    s.world.update_auras_and_buffs(0.0, false);
    assert!(s.world.npcs[&npc].character.buffs.nonvisual.is_empty());
    s.world.size_magick(npc, K::Shrink);
    s.world.update_ability_effects(0.0);
    assert!(
        s.world.npcs[&npc].character.buffs.nonvisual.is_empty(),
        "cached period is not reset at Execute"
    );
    s.world.update_ability_effects(0.06);
    let c = &s.world.npcs[&npc].character;
    assert_eq!(
        c.buffs.nonvisual.iter().map(|b| b.id).collect::<Vec<_>>(),
        ids
    );
    assert_eq!(
        c.combat.health.hit_points, 250.0,
        "new insertion after expiry halves once"
    );
}

#[test]
fn death_fades_without_expiring_positive_ttl_and_owner_replacement_cancels() {
    let Some((mut s, _)) = damage_boost_tests::session() else {
        return;
    };
    let handle = s.avatar().handle;
    s.world.cast_magick(0, M::Grow);
    s.world.update_ability_effects(1.0);
    s.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 0.0;
    s.world.update_ability_effects(1.0);
    let effect = s.world.size_effects.active(handle).unwrap();
    assert_eq!(effect.animation_time, 0.0);
    assert_eq!(effect.ttl, 18.0);
    s.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 100.0;
    s.world.update_ability_effects(1.0);
    assert_eq!(
        s.world.size_effects.active(handle).unwrap().animation_time,
        1.0
    );
    s.world.cancel_player_abilities(0, handle);
    assert!(s.world.size_effects.active(handle).is_none());
    assert_eq!(s.avatar().character.model_scale, 1.0);
}

#[test]
fn static_scale_precedes_template_transform_without_scaling_its_translation() {
    let Some((mut s, _)) = damage_boost_tests::session() else {
        return;
    };
    let c = &mut s.world.players[0].as_mut().unwrap().character;
    c.model_scale = 2.0;
    let mut template = magicka_content::skinned::IDENTITY;
    template[12] = 7.0;
    let scaled = c.scaled_static_transform(&template);
    assert_eq!(scaled[0], 2.0);
    assert_eq!(scaled[12], 7.0);
}

#[test]
fn scene_change_keeps_player_size_timer_and_retained_aura_identity() {
    let Some((mut s, _)) = damage_boost_tests::session() else {
        return;
    };
    s.world.cast_magick(0, M::Grow);
    s.world.update_ability_effects(1.0);
    let effect = s.world.size_effects.active(s.avatar().handle).unwrap();
    let ids: Vec<_> = s
        .avatar()
        .character
        .buffs
        .nonvisual
        .iter()
        .map(|b| b.id)
        .collect();
    s.change_scene_named("havindr_s3", "start").unwrap();
    assert_eq!(
        s.world.size_effects.active(s.avatar().handle).unwrap(),
        effect
    );
    assert_eq!(s.avatar().character.model_scale, 2.0);
    assert_eq!(
        s.body().radius,
        2.0 * s.avatar().character.definition.radius
    );
    s.world.update_ability_effects(0.25);
    assert_eq!(
        s.avatar()
            .character
            .buffs
            .nonvisual
            .iter()
            .map(|b| b.id)
            .collect::<Vec<_>>(),
        ids
    );
}

#[test]
fn real_keyboard_combos_cast_both_sizes_through_cast_spell0() {
    for (magick, kind, keys) in [
        (M::Grow, K::Grow, [1, 6, 2, 6, 1]),
        (M::Shrink, K::Shrink, [5, 3, 2, 3, 5]),
    ] {
        let Some((mut s, _)) = damage_boost_tests::session() else {
            return;
        };
        let frame = |s: &mut crate::play_session::PlaySession, input| {
            s.step(1.0 / 60.0);
            s.handle_input(input, 16.0 / 9.0);
        };
        for _ in 0..60 {
            frame(&mut s, crate::avatar::PlayerInput::default());
        }
        s.unlocked_magicks[0] |= 1 << magick as u8;
        for key in keys {
            let mut elements = [false; 8];
            elements[key] = true;
            frame(
                &mut s,
                crate::avatar::PlayerInput {
                    elements,
                    ..Default::default()
                },
            );
            for _ in 0..6 {
                frame(&mut s, crate::avatar::PlayerInput::default());
            }
        }
        frame(
            &mut s,
            crate::avatar::PlayerInput {
                boost_down: true,
                ..Default::default()
            },
        );
        let mut cast_clip = false;
        for _ in 0..120 {
            frame(&mut s, crate::avatar::PlayerInput::default());
            cast_clip |= Some(s.avatar().motion.controller.current_animation)
                == crate::character_motion::animation_index("cast_spell0");
        }
        let effect = s
            .world
            .size_effects
            .active(s.avatar().handle)
            .expect("keyboard cast reaches size effect");
        assert_eq!(effect.kind, kind);
        assert!(cast_clip);
        assert!(s.avatar().conjure.queue.is_empty());
        assert_eq!(
            s.avatar().character.model_scale,
            if kind == K::Grow { 2.0 } else { 0.5 }
        );
    }
}
