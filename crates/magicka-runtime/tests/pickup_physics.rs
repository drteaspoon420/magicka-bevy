use magicka_content::{
    level::{LevelModel, NavMesh, TriMesh},
    skinned::IDENTITY,
};
use magicka_physics::BodyKey;
use magicka_runtime::{
    pickup_physics::{PickupBody, PickupPhysics},
    pickups::PickupBounds,
    play_session::PlaySession,
};
fn floor() -> LevelModel {
    let mut layers = vec![TriMesh::default(); 10];
    layers[0] = TriMesh {
        vertices: vec![
            [-20., 0., -20.],
            [-20., 0., 20.],
            [20., 0., 20.],
            [20., 0., -20.],
        ],
        triangles: vec![[0, 1, 2], [0, 2, 3], [0, 0, 0]],
    };
    LevelModel {
        model: None,
        animated_parts: vec![],
        lights: vec![],
        effects: vec![],
        physics_entities: vec![],
        liquids: vec![],
        force_fields: vec![],
        collision_layers: layers,
        camera_mesh: None,
        trigger_areas: vec![],
        locators: vec![],
        nav_mesh: NavMesh::default(),
    }
}
fn body(immovable: bool) -> PickupBody {
    let mut transform = IDENTITY;
    transform[13] = 5.;
    PickupBody {
        key: BodyKey {
            handle: 1,
            generation: 0,
        },
        model: "test cube".into(),
        transform,
        bounds: PickupBounds {
            min: [-0.5; 3],
            max: [0.5; 3],
        },
        book: false,
        immovable,
        mass: 50.,
        solid: true,
        shape: None,
    }
}
#[test]
fn adapter_advances_motion_preserves_static_and_cleans_up_generations() {
    let mut physics = PickupPhysics::new(&floor()).unwrap();
    let mut moving = body(false);
    let mut fixed = body(true);
    fixed.key.handle = 2;
    fixed.transform[12] = 3.;
    for _ in 0..300 {
        let poses = physics
            .step(vec![moving.clone(), fixed.clone()], 1. / 60.)
            .unwrap();
        for (handle, pose, _) in poses {
            if handle == 1 {
                moving.transform = pose;
            } else {
                fixed.transform = pose;
            }
        }
    }
    assert!(
        (moving.transform[13] - 0.5).abs() < 0.1,
        "{:?}",
        moving.transform
    );
    assert_eq!(fixed.transform[13], 5.);
    assert!(physics.state(1).unwrap().sleeping);
    moving.key.generation = 1;
    moving.transform[13] = 8.;
    physics.step(vec![moving], 0.).unwrap();
    assert_eq!(physics.state(1).unwrap().position[1], 8.);
    assert!(physics.state(2).is_none());
    physics.step(vec![], 1. / 60.).unwrap();
    assert!(physics.state(1).is_none());
}
fn session() -> Option<PlaySession> {
    let content = magicka_content::retail_content_dir();
    if !content.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    Some(s)
}

#[test]
fn release_velocity_precedes_integration_and_cannot_target_a_reused_handle() {
    let mut physics = PickupPhysics::new(&floor()).unwrap();
    let mut moving = body(false);
    let old_key = moving.key;
    let poses = physics
        .step_with_release_velocities(vec![moving.clone()], 1. / 60., &[(old_key, [6., 0., 0.])])
        .unwrap();
    assert!(poses[0].1[12] > 0.05, "release moves on the first frame");
    moving.key.generation += 1;
    physics
        .step_with_release_velocities(vec![moving], 0., &[(old_key, [6., 0., 0.])])
        .unwrap();
    assert_eq!(
        physics.state(old_key.handle).unwrap().linear_velocity,
        [0.; 3]
    );
}
#[test]
fn live_dynamic_book_falls_and_registry_tracks_it_then_collection_removes_body() {
    let Some(mut s) = session() else {
        return;
    };
    let mut t = IDENTITY;
    t[12] = s.body().position[0] - 3.;
    t[13] = s.body().position[1] + 5.;
    t[14] = s.body().position[2];
    let h = s.spawn_magick_book(3, t).unwrap();
    let book = s.world.book_pickups.get_mut(&h).unwrap();
    book.immovable = false;
    book.body_enabled = true;
    for _ in 0..120 {
        s.step(1. / 60.);
    }
    assert_eq!(s.script_error.as_deref(), Some("scripts disabled"));
    let book = &s.world.book_pickups[&h];
    assert!(book.transform[13] < t[13] - 1., "{:?}", book.transform);
    let e = s.world.entities.iter().find(|e| e.handle == h).unwrap();
    assert_eq!(
        e.position,
        [book.transform[12], book.transform[13], book.transform[14]]
    );
    assert_eq!(e.orientation, book.transform);
    assert!(s.pickup_physics.as_ref().unwrap().state(h).is_some());
    assert!(
        !s.events.iter().any(|e| matches!(
            e,
            magicka_runtime::play_session::SessionEvent::PhysicsFailed(_)
        )),
        "{:?}",
        s.events
    );
    let player = s.avatar().body;
    let orientation = s.body().orientation;
    let position = e.position;
    s.world.physics.bodies[player].move_to(position, orientation);
    s.pickup_book(h).unwrap();
    assert!(s.pickup_physics.as_ref().unwrap().state(h).is_none());
    s.change_scene_named("havindr_s2", "start").unwrap();
    assert!(s.pickup_physics.is_none());
}
#[test]
fn swapped_equipment_starts_a_new_dynamic_body_and_removed_items_leave_physics() {
    let Some(mut s) = session() else {
        return;
    };
    let mut t = IDENTITY;
    t[12..15].copy_from_slice(&s.body().position);
    let h = s.spawn_item_pickup("weapon_kris", t).unwrap();
    s.step(1. / 60.);
    assert!(s.pickup_physics.as_ref().unwrap().state(h).is_some());
    s.pickup_item(h).unwrap();
    assert!(!s.world.item_pickups[&h].immovable);
    s.step(1. / 60.);
    assert!(
        !s.pickup_physics
            .as_ref()
            .unwrap()
            .state(h)
            .unwrap()
            .sleeping
    );
    s.world
        .entities
        .iter_mut()
        .find(|e| e.handle == h)
        .unwrap()
        .removed = true;
    s.step(1. / 60.);
    assert!(s.pickup_physics.as_ref().unwrap().state(h).is_none());
}
