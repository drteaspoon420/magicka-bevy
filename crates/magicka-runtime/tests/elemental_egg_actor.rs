use magicka_runtime::{
    elemental_egg_actor::{EggGeometry, ElementalEggActor, ElementalEggCache, NATIVE_CACHE_SIZE},
    play_content,
};
use magicka_sim::{damage::Damage, elemental_egg as egg, spells::Elements as E};
use std::path::{Path, PathBuf};

#[test]
fn retail_assets_supply_egg_geometry_pose_and_cached_reinitialization() {
    let content = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    if !content.is_dir() {
        return;
    }
    let fire = play_content::load_character(&content, "Elemental_Fire").unwrap();
    let mesh =
        play_content::load_skinned_model(&content, &Path::new(egg::MODEL).with_extension("xnb"))
            .unwrap();
    let animation = play_content::load_skinned_model(
        &content,
        &Path::new(egg::ANIMATION_MODEL).with_extension("xnb"),
    )
    .unwrap();
    let clip_duration = animation
        .clips
        .iter()
        .find(|clip| clip.name == egg::CLIP)
        .unwrap()
        .duration;
    let geometry = EggGeometry::from_fire_template(&fire).unwrap();
    assert_eq!(geometry.radius, fire.radius);
    assert_eq!(geometry.length, fire.length.max(0.01));
    assert_eq!(geometry.scale, fire.models[0].scale);
    let bone_count = mesh.skeleton.bones.len();
    let mut cache = ElementalEggCache::new(
        mesh.clone(),
        animation.clone(),
        geometry,
        std::array::from_fn(|index| index as u16 + 1),
    )
    .unwrap();
    assert_eq!(cache.slots.len(), 128);
    assert!(std::sync::Arc::ptr_eq(
        &cache.slots[0].actor.mesh,
        &cache.slots[1].actor.mesh
    ));
    assert!(std::sync::Arc::ptr_eq(
        &cache.slots[0].actor.skeleton,
        &cache.slots[1].actor.skeleton
    ));
    let first = cache.acquire();
    assert_eq!(first.handle, 1);
    first.actor.initialize([0.0; 3], [0.0, 0.0, -1.0], None);
    first.actor.state.resting_timer = -2.0;
    for index in 1..NATIVE_CACHE_SIZE {
        let slot = cache.acquire();
        assert_eq!(slot.handle, index as u16 + 1);
        assert!(
            !slot.actor.registered,
            "controllers/lifecycles remain independent"
        );
    }
    let reused = cache.acquire();
    assert_eq!(reused.handle, 1);
    assert!(
        reused.actor.registered,
        "native rotation does not skip active slots"
    );
    assert_eq!(
        reused.actor.state.resting_timer, -2.0,
        "checkout alone does not initialize"
    );
    let mut actor = ElementalEggActor::new(mesh, animation, geometry).unwrap();
    assert!(actor.update(0.1, false, |_| unreachable!()).is_none());
    actor.state.proximity = 3.0;
    actor.state.resting_timer = -0.25;
    actor.velocity = [2.0, 1.0, 3.0];
    actor.initialize([3.0, 5.0, 4.0], [0.0, 0.0, -2.0], Some([3.0, 0.0, 4.0]));
    assert_eq!(actor.position, [3.0, geometry.radius, 4.0]);
    assert_eq!(actor.velocity, [0.0; 3]);
    assert_eq!(actor.state.proximity, 3.0);
    assert_eq!(actor.state.resting_timer, -0.25);
    assert_eq!(
        actor.orientation[10], 2.0,
        "direction magnitude is retained"
    );
    let frame = actor
        .update(clip_duration + 0.1, false, |_| unreachable!())
        .unwrap();
    assert_eq!(frame.bones.len(), bone_count);
    assert_eq!(frame.local_poses.len(), bone_count);
    assert!(frame.bones.iter().flatten().all(|v| v.is_finite()));
    assert!(actor.controller.time < clip_duration, "spawn clip loops");
    assert_eq!(frame.model_parent[12..15], actor.position);
    actor.state.summoner = Some(7);
    actor.state.internal_damage(Damage {
        element: E::FIRE.0,
        amount: 5.0,
        magnitude: 1.0,
        ..Default::default()
    });
    assert!(
        actor.update(0.1, false, |_| unreachable!()).is_some(),
        "final hatch frame is sampled"
    );
    assert!(actor.state.dead());
    let hatch = actor.deinitialize(false).unwrap();
    assert_eq!(hatch.element, E::FIRE);
    assert_eq!(hatch.summoner, Some(7));
    assert_eq!(hatch.orientation, actor.orientation);
    assert!(
        actor.deinitialize(false).is_none(),
        "only one hatch per registration"
    );
    actor.initialize([1.0; 3], [1.0, 0.0, 0.0], None);
    assert_eq!(actor.controller.time, 0.0);
    assert_eq!(actor.state.summoner, None);
    actor.state.kill();
    assert!(
        actor.deinitialize(false).is_none(),
        "Shield removal does not hatch"
    );
}
