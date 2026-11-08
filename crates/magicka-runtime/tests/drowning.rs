//! Character.Drown through live water contacts against user-supplied retail
//! content (skips when absent). See research/drowning.md.
use magicka_content::level::TriMesh;
use magicka_runtime::{
    avatar::PlayerInput, play_session::PlaySession, water_collision::WaterCollision,
};
use std::path::{Path, PathBuf};

const DT: f32 = 1.0 / 60.0;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    root.is_dir().then_some(root)
}

/// A square water surface `height` above the avatar's feet.
fn water_over_avatar(s: &mut PlaySession, height: f32) {
    let feet = s.body().feet();
    water_over_feet(s, feet, height);
}

fn water_over_feet(s: &mut PlaySession, feet: [f32; 3], height: f32) {
    let y = feet[1] + height;
    let (x, z, r) = (feet[0], feet[2], 6.0);
    let geometry = TriMesh {
        vertices: vec![
            [x - r, y, z - r],
            [x + r, y, z - r],
            [x + r, y, z + r],
            [x - r, y, z + r],
        ],
        triangles: vec![[0, 2, 1], [0, 3, 2]],
    };
    s.world
        .collision
        .collision_mut()
        .water
        .push(WaterCollision::from_geometry(&geometry, true, false).unwrap());
}

#[test]
fn npc_liquid_contacts_reach_the_status_owner() {
    use magicka_sim::{character_status::StatusEvent, status::Status};
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..30 {
        s.step(DT);
    }
    let handle = s.spawn_npc("Goblin_pirate", "attack", 7.0).unwrap();
    let body = s.world.npcs[&handle].body;
    let feet = s.world.physics.bodies[body].feet();
    water_over_feet(&mut s, feet, 0.3);
    s.world
        .collision
        .collision_mut()
        .water
        .last_mut()
        .unwrap()
        .lava = true;
    s.step(DT);
    assert!(s.world.npcs[&handle]
        .character
        .combat
        .statuses
        .has(Status::Burning));
    assert!(s
        .world
        .liquid_status_events
        .iter()
        .any(|(owner, event)| *owner == handle
            && *event == StatusEvent::PlayStatusCue(Status::Burning)));
    assert!(!s
        .world
        .liquid_status_events
        .iter()
        .any(|(owner, event)| *owner == handle && matches!(event, StatusEvent::Hud { .. })));
    s.world
        .collision
        .collision_mut()
        .water
        .last_mut()
        .unwrap()
        .lava = false;
    s.step(DT);
    assert!(!s.world.npcs[&handle]
        .character
        .combat
        .statuses
        .has(Status::Burning));
}

#[test]
fn deep_water_drowns_avatar_and_shallow_water_does_not() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    // Shallow: well below the body centre, the avatar only wades.
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..30 {
        s.step(DT);
    }
    water_over_avatar(&mut s, 0.3);
    for _ in 0..60 {
        s.step(DT);
        s.handle_input(PlayerInput::default(), 16.0 / 9.0);
    }
    assert!(s.body().is_in_water(), "wading contact");
    assert!(s
        .avatar()
        .character
        .combat
        .statuses
        .has(magicka_sim::status::Status::Wet));
    assert!(!s.avatar().motion.drowning);
    assert!(s.avatar().character.combat.health.hit_points > 0.0);

    // Deep: the surface crosses the capsule above its centre, so the
    // contact depth (feet to surface) is at least -HeightOffset.
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..30 {
        s.step(DT);
    }
    let before = s.body().position;
    let centre = -s.body().height_offset();
    s.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .set_invisible(60.0);
    water_over_avatar(&mut s, centre + 0.3);
    s.step(DT);
    assert!(s.avatar().motion.drowning);
    assert_eq!(
        s.avatar().character.combat.health.invisibility_timer,
        0.0,
        "Drown clears invisibility immediately, before natural expiration"
    );
    let mut died = false;
    // The die_drown clip plays out before Character.Die (~4 s).
    for _ in 0..600 {
        s.step(DT);
        s.handle_input(PlayerInput::default(), 16.0 / 9.0);
        died |= s.dead();
    }
    let p = s.avatar();
    assert!(p.motion.drowning, "drowned");
    assert_eq!(p.character.combat.health.hit_points, 0.0);
    assert!(died, "avatar entered death");
    assert!(s.world.log.iter().any(|l| l.starts_with("drown ")));
    let body = s.body();
    assert!(!body.active, "DisableBody");
    assert!(body.position[1] > before[1], "lifted by WaterDepth");
    let animation = magicka_runtime::animation_controller::animation_name(
        p.motion.controller.current_animation,
    );
    assert!(animation.starts_with("die_drown"), "{animation}");
}

#[test]
fn shallow_lava_burns_through_live_contact_and_water_extinguishes_it() {
    use magicka_runtime::{audio::AudioRequest, scene_loading::custom_hash};
    use magicka_sim::status::Status;
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..30 {
        s.step(DT);
    }
    let body = s.avatar().body;
    let character = &mut s.world.players[0].as_mut().unwrap().character;
    character.definition.regeneration = 0;
    let hp = character.combat.health.hit_points;
    // Hold the wading depth constant: this tests actual contact callbacks,
    // while sinking/drowning is covered separately.
    s.world.physics.bodies[body].apply_gravity = false;
    s.world.physics.bodies[body].velocity = [0.0; 3];
    water_over_avatar(&mut s, 0.3);
    s.audio.clear();
    s.world
        .collision
        .collision_mut()
        .water
        .last_mut()
        .unwrap()
        .lava = true;
    for _ in 0..60 {
        s.step(DT);
    }
    assert!(!s.avatar().motion.drowning);
    assert!(s.avatar().character.combat.statuses.has(Status::Burning));
    assert!(s.avatar().character.combat.health.hit_points < hp - 20.0);
    assert!(s
        .status_emitters(s.avatar().handle)
        .iter()
        .any(|name| name.contains("burning")));
    let fire_keys: Vec<_> = s
        .audio
        .iter()
        .filter_map(|(request, _)| match request {
            AudioRequest::Start {
                key, bank: 16, cue, ..
            } if *cue == custom_hash("spell_fire_onfire") => Some(*key),
            _ => None,
        })
        .collect();
    assert_eq!(
        fire_keys.len(),
        1,
        "repeated contacts must retain one owned burning cue"
    );
    assert!(s.audio.iter().any(|(request, _)| matches!(request,
        AudioRequest::Move {key, ..} if *key == fire_keys[0])));
    s.audio.clear();
    s.world
        .collision
        .collision_mut()
        .water
        .last_mut()
        .unwrap()
        .lava = false;
    s.step(DT);
    assert!(
        s.effects
            .active_effects_by_hash(custom_hash("drying_steam"))
            > 0,
        "extinguishing a playing burning cue starts drying steam"
    );
    for _ in 0..59 {
        s.step(DT);
    }
    assert_eq!(
        s.audio
            .iter()
            .filter(|(request, _)| matches!(request,
        AudioRequest::Stop { key, instant: false } if *key == fire_keys[0]))
            .count(),
        1
    );
    assert!(s.audio.iter().any(|(request, _)| matches!(request,
        AudioRequest::Start { bank: 16, cue, .. } if *cue == custom_hash("spell_steam_hit"))));
    assert!(!s.avatar().character.combat.statuses.has(Status::Burning));
    assert!(s.avatar().character.combat.statuses.has(Status::Wet));
    let wet_keys: Vec<_> = s
        .audio
        .iter()
        .filter_map(|(request, _)| match request {
            AudioRequest::Start {
                key, bank: 16, cue, ..
            } if *cue == custom_hash("spell_water_drip") => Some(*key),
            _ => None,
        })
        .collect();
    assert_eq!(wet_keys.len(), 1);
    s.world.collision.collision_mut().water.pop();
    s.audio.clear();
    for _ in 0..600 {
        s.step(DT);
    }
    assert!(
        s.avatar().character.combat.statuses.has(Status::Wet),
        "native Wet has no time decay"
    );
    assert!(!s.audio.iter().any(|(request, _)| matches!(request,
        AudioRequest::Stop { key, .. } if *key == wet_keys[0])));
    water_over_avatar(&mut s, 0.3);
    s.world
        .collision
        .collision_mut()
        .water
        .last_mut()
        .unwrap()
        .lava = true;
    s.step(DT);
    assert!(!s.avatar().character.combat.statuses.has(Status::Wet));
    assert_eq!(
        s.audio
            .iter()
            .filter(|(request, _)| matches!(request,
        AudioRequest::Stop { key, instant: false } if *key == wet_keys[0]))
            .count(),
        1,
        "removed Wet status releases its owned cue once"
    );
}

#[test]
fn liquid_status_respects_shield_kind_immunity_and_status_depth() {
    use magicka_sim::{
        character_health::SelfShieldType as Shield, character_status::StatusResult,
        spells::Elements as E, status::Status,
    };
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let player = s.world.players[0].as_mut().unwrap();
    for shield in [
        Shield::None,
        Shield::Shield,
        Shield::Earth,
        Shield::Ice,
        Shield::IcedEarth,
    ] {
        player.character.combat.statuses = Default::default();
        player.character.combat.health.shield.kind = shield;
        let mut events = Vec::new();
        let result = player.liquid_status(0.3, DT, true, &mut events).unwrap();
        assert_eq!(result == StatusResult::Added, shield != Shield::Shield);
        assert_eq!(
            player
                .character
                .combat
                .statuses
                .effect(Status::Burning)
                .unwrap()
                .dps,
            if shield == Shield::Shield { 0.0 } else { 100.0 }
        );
    }
    player.character.combat.statuses = Default::default();
    player.character.combat.health.shield.kind = Shield::None;
    let mut events = Vec::new();
    assert_eq!(
        player.liquid_status(0.199, DT, false, &mut events).unwrap(),
        StatusResult::None
    );
    assert_eq!(
        player.liquid_status(0.2, DT, false, &mut events).unwrap(),
        StatusResult::Added
    );
    player.character.combat.statuses = Default::default();
    player
        .character
        .buffs
        .list
        .push(magicka_runtime::buffs::ActiveBuff {
            id: 0,
            resistance: magicka_sim::resistance::Resistance {
                against: E::FIRE,
                multiplier: 0.5,
                ..magicka_sim::resistance::Resistance::IDENTITY
            },
            ttl: 10.0,
            color: [1.0; 3],
            self_casted: true,
            decal: None,
        });
    assert_eq!(
        player.liquid_status(0.3, DT, true, &mut events).unwrap(),
        StatusResult::Added
    );
    assert_eq!(
        player
            .character
            .combat
            .statuses
            .effect(Status::Burning)
            .unwrap()
            .dps,
        50.0
    );
    player.character.combat.statuses = Default::default();
    player.character.definition.resistances[E::FIRE.index() as usize].status_resistance = true;
    assert_eq!(
        player.liquid_status(0.3, DT, true, &mut events).unwrap(),
        StatusResult::None
    );
    assert!(player
        .character
        .combat
        .statuses
        .effect(Status::Burning)
        .unwrap()
        .dead());
}

#[test]
fn burning_expiration_stops_the_owned_contact_cue_once() {
    use magicka_runtime::{audio::AudioRequest, scene_loading::custom_hash};
    use magicka_sim::status::Status;
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..30 {
        s.step(DT);
    }
    let body = s.avatar().body;
    let character = &mut s.world.players[0].as_mut().unwrap().character;
    character.definition.regeneration = 0;
    character.combat.health.hit_points = 100_000.0;
    character.definition.max_hit_points = 100_000.0;
    character.combat.health.max_hit_points = 100_000.0;
    s.world.physics.bodies[body].apply_gravity = false;
    s.world.physics.bodies[body].velocity = [0.0; 3];
    water_over_avatar(&mut s, 0.3);
    s.world
        .collision
        .collision_mut()
        .water
        .last_mut()
        .unwrap()
        .lava = true;
    s.audio.clear();
    for _ in 0..60 {
        s.step(DT);
    }
    let key = s
        .audio
        .iter()
        .find_map(|(request, _)| match request {
            AudioRequest::Start {
                key, bank: 16, cue, ..
            } if *cue == custom_hash("spell_fire_onfire") => Some(*key),
            _ => None,
        })
        .unwrap();
    s.world.collision.collision_mut().water.pop();
    s.audio.clear();
    for _ in 0..1200 {
        s.step(DT);
    }
    assert!(s.avatar().character.combat.health.hit_points > 0.0);
    assert!(!s.avatar().character.combat.statuses.has(Status::Burning));
    assert_eq!(
        s.audio
            .iter()
            .filter(|(request, _)| matches!(request,
        AudioRequest::Stop { key: stopped, instant: false } if *stopped == key))
            .count(),
        1
    );
}
