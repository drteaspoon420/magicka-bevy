use magicka_content::skinned::IDENTITY;
use magicka_runtime::{
    play_content::load_physics_template, play_session::PlaySession, spawn_phys::SpawnPhysHost,
};
use std::path::PathBuf;
fn content() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
}
#[test]
fn retail_prop_catalogue_decodes_collision_descriptors() {
    let content = content();
    if !content.is_dir() {
        return;
    }
    let mut total = 0;
    let mut boxes = 0;
    let mut meshes = 0;
    let mut unsupported = 0;
    let mut entries: Vec<_> = std::fs::read_dir(content.join("Data/PhysicsEntities"))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "xnb") {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap();
        let t = load_physics_template(&content, name).unwrap();
        total += 1;
        if !t.boxes.is_empty() {
            boxes += 1;
        } else if t.collision_mesh.is_some() {
            meshes += 1;
        } else {
            unsupported += 1;
        }
        println!(
            "{name}: movable={} solid={} mass={} boxes={} mesh={} model={} animated={}",
            t.movable,
            t.solid,
            t.mass,
            t.boxes.len(),
            t.collision_mesh.is_some(),
            t.model.is_some(),
            t.animated.is_some()
        );
    }
    println!("physics catalogue: total={total} boxes={boxes} mesh_only={meshes} no_collision={unsupported}");
    assert!(total > 0);
    assert!(boxes > 0);
}
#[test]
fn debug_and_script_props_share_bodies_and_reinitialization_resets_generation() {
    let content = content();
    if !content.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let mut t = IDENTITY;
    let p = s.body().position;
    t[12..15].copy_from_slice(&[p[0] - 3., p[1] + 5., p[2]]);
    let definition = load_physics_template(&content, "bathball").unwrap();
    assert!(definition.movable);
    assert!(!definition.boxes.is_empty());
    let count = s.world.entities.len();
    assert!(s.spawn_physics_prop("nonexistent_prop", t).is_err());
    assert_eq!(s.world.entities.len(), count);
    let mut static_pose = t;
    static_pose[12] += 6.;
    let fixed = s
        .spawn_physics_prop("barrel_explosive", static_pose)
        .unwrap();
    let h = s.spawn_physics_prop("bathball", t).unwrap();
    s.step(1. / 60.);
    let initial = s
        .pickup_physics
        .as_ref()
        .unwrap()
        .state(h)
        .unwrap()
        .position[1];
    for _ in 0..120 {
        s.step(1. / 60.);
    }
    let static_entity = s.world.entities.iter().find(|e| e.handle == fixed).unwrap();
    assert_eq!(
        static_entity.position,
        [static_pose[12], static_pose[13], static_pose[14]]
    );
    let body = s.pickup_physics.as_ref().unwrap().state(h).unwrap();
    assert!(body.position[1] < initial - 1., "{body:?}");
    let e = s.world.entities.iter().find(|e| e.handle == h).unwrap();
    assert_eq!(
        e.position,
        [e.orientation[12], e.orientation[13], e.orientation[14]]
    );
    let g = s.world.physics_generations[&h];
    t[13] += 3.;
    s.world
        .initialize_entity(h, Some("bathball"), t, 42)
        .unwrap();
    s.step(0.);
    assert_eq!(s.world.physics_generations[&h], g + 1);
    assert!(
        s.pickup_physics
            .as_ref()
            .unwrap()
            .state(h)
            .unwrap()
            .position[1]
            > body.position[1] + 3.
    );
    s.world
        .entities
        .iter_mut()
        .find(|e| e.handle == h)
        .unwrap()
        .removed = true;
    s.step(1. / 60.);
    assert!(s.pickup_physics.as_ref().unwrap().state(h).is_none());
    assert!(!s.events.iter().any(|e| matches!(
        e,
        magicka_runtime::play_session::SessionEvent::PhysicsFailed(_)
    )));
}

use magicka_content::physics::{CollisionBox, PhysicsEntityTemplate};
use magicka_runtime::physics_props::{body, collision_box};

#[test]
fn rotated_corner_maps_to_the_same_centre_in_combat_and_physics() {
    let q = std::f32::consts::FRAC_1_SQRT_2;
    let b = CollisionBox {
        name: String::new(),
        position: [1.0, 2.0, 3.0],
        sides: [2.0, 4.0, 6.0],
        orientation: [0.0, 0.0, q, q],
    };
    let template = PhysicsEntityTemplate {
        movable: false,
        pushable: false,
        solid: true,
        mass: 0.0,
        max_hitpoints: 100,
        can_have_status: false,
        resistances: vec![],
        gibs: vec![],
        hit_effect: String::new(),
        hit_sound: String::new(),
        gib_trail_effect: String::new(),
        model: None,
        collision_mesh: None,
        boxes: vec![b.clone()],
        effects: vec![],
        conditions: Default::default(),
        id: String::new(),
        animated: None,
    };
    let mut transform = magicka_content::skinned::IDENTITY;
    transform[12..15].copy_from_slice(&[10.0, 20.0, 30.0]);
    let bounds = collision_box(&b, &transform);
    // Rotating (half X, half Y, half Z) by 90 degrees around Z gives
    // (-2, 1, 3), added to the corner and entity translation.
    let expected = [9.0, 23.0, 36.0];
    for i in 0..3 {
        assert!((bounds.center[i] - expected[i]).abs() < 1e-5);
    }
    let descriptor = body(1, 1, "authored", transform, &template).unwrap();
    let local_center = std::array::from_fn::<_, 3, _>(|i| {
        (descriptor.bounds.min[i] + descriptor.bounds.max[i]) * 0.5
    });
    let state = magicka_physics::BodyState::from_transform(transform, local_center);
    for i in 0..3 {
        assert!((state.position[i] - bounds.center[i]).abs() < 1e-5);
    }
    assert_eq!(descriptor.bounds.max[2] - descriptor.bounds.min[2], 6.0);
}
