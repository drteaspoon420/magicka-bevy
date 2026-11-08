use magicka_content::skinned::IDENTITY;
use magicka_runtime::{
    effect_action::{EffectAction, EffectActionHost},
    play_session::PlaySession,
    scene_loading::custom_hash,
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
#[test]
fn battlefield_initial_trigger_stops_checkpoint_effects_and_named_start_reactivates_them() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Battlefield.lvl", "Wizard").unwrap();
    let id = custom_hash("effect_continous");
    assert!(s.world.effects.effects.contains_key(&id));
    assert!(s.scene_effect_active(id));
    s.step(DT);
    assert!(!s.world.effects.effects[&id].effect.active);
    assert!(!s.scene_effect_active(id));
    s.disable_scripts();
    EffectAction::read("starteffect", [("name", "effect_continous")])
        .unwrap()
        .execute(&mut s.world)
        .unwrap();
    s.step(DT);
    assert!(s.scene_effect_active(id));
    for _ in 0..10 {
        s.step(DT);
    }
    assert!(s.scene_effect_active(id));
}
#[test]
fn script_spawn_stop_restart_and_single_expiry_feed_particle_host() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.stop_level_effects();
    let id = custom_hash("qa_named_effect");
    let hash = custom_hash("ambient_fire_torch");
    s.world.add_effect(id, hash, IDENTITY, 0.0).unwrap();
    s.step(DT);
    assert!(s.scene_effect_active(id));
    assert!(s.effects.active_effects_by_hash(hash) > 0);
    for _ in 0..10 {
        s.step(DT);
    }
    assert!(s.effects.particles.active().iter().any(|p| p.radius > 0.0));
    let original = s.world.effects.effects[&id].effect.instance;
    // An active AddEffect leaves the existing named instance unchanged.
    s.world
        .add_effect(id, custom_hash("magick_nullified"), IDENTITY, 0.0)
        .unwrap();
    assert_eq!(s.world.effects.effects[&id].effect.instance, original);
    assert_eq!(s.world.effects.effects[&id].effect.effect, hash);
    // Same-frame lifecycle calls must reach the host in order.
    s.world.stop_effect(id).unwrap();
    s.world.start_effect(id).unwrap();
    s.step(DT);
    assert!(s.scene_effect_active(id));
    assert_eq!(s.effects.active_effects_by_hash(hash), 1);
    s.world.stop_effect(id).unwrap();
    s.step(DT);
    assert!(!s.scene_effect_active(id));
    s.world.start_effect(id).unwrap();
    s.step(DT);
    assert!(s.scene_effect_active(id));
    assert_eq!(s.world.effects.effects[&id].effect.instance, original);
    s.world.stop_effect(id).unwrap();
    let single = custom_hash("magick_nullified");
    s.world.add_effect(id, single, IDENTITY, 0.0).unwrap();
    s.step(DT);
    assert_ne!(s.world.effects.effects[&id].effect.instance, original);
    for _ in 0..120 {
        s.step(DT);
        assert_eq!(
            s.world.effects.effects[&id].effect.active,
            s.scene_effect_active(id)
        );
    }
    assert!(!s.scene_effect_active(id));
    assert!(!s.world.effects.effects[&id].effect.active);
    s.world.start_effect(id).unwrap();
    s.step(DT);
    assert!(s.scene_effect_active(id));
}

#[test]
fn scripted_scene_effect_range_clips_to_live_shield() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.stop_level_effects();
    let id = custom_hash("qa_ranged_effect");
    let transform = magicka_runtime::particles::create_world(
        [0.0, 100.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0],
    );
    s.world
        .add_effect(id, custom_hash("ambient_fire_torch"), transform, 10.0)
        .unwrap();
    s.world.spawn_shield(
        s.avatar().handle,
        [0.0, 100.0, 3.0],
        1.0,
        [0.0, 0.0, 1.0],
        magicka_sim::shield_entity::ShieldKind::Sphere,
        1000.0,
        magicka_sim::shield_entity::SHIELD_COLOR,
    );
    s.step(DT);
    let t = s.world.effects.effects[&id].effect.transform;
    let length = (t[8] * t[8] + t[9] * t[9] + t[10] * t[10]).sqrt();
    assert!((length - 0.2).abs() < 1e-6);
    assert_eq!([t[12], t[13], t[14]], [0.0, 100.0, 0.0]);
}

#[test]
fn castle_puzzle_effects_stay_off_until_their_callback_starts_them() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..15 {
        s.step(DT);
    }
    for name in [
        "effect_generator_sparks_1",
        "effect_generator_sparks_2",
        "effect_generator_sparks_3",
        "effect_generator_sparks_4",
        "effect_torch_fire_1",
        "effect_torch_fire_2",
    ] {
        let id = custom_hash(name);
        assert!(
            s.world.effects.effects.contains_key(&id),
            "registered {name}"
        );
        assert!(!s.scene_effect_active(id), "Init stops {name}");
    }
    // This is a lifecycle test; combat-input coverage exercises the same
    // callback separately. Keep unrelated tutorial triggers out of the way.
    s.disable_scripts();
    EffectAction::read("starteffect", [("name", "Effect_generator_sparks_1")])
        .unwrap()
        .execute(&mut s.world)
        .unwrap();
    s.step(DT);
    assert!(s.scene_effect_active(custom_hash("effect_generator_sparks_1")));
    assert!(!s.scene_effect_active(custom_hash("effect_generator_sparks_2")));
    assert!(!s.scene_effect_active(custom_hash("effect_torch_fire_1")));
}
