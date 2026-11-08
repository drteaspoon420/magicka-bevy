use magicka_physics::*;
fn key(handle: u16, generation: u64) -> BodyKey {
    BodyKey { handle, generation }
}
#[test]
fn falling_box_collides_with_triangle_floor_and_can_sleep() {
    let mut world = RigidWorld::new();
    world
        .add_mesh(
            &[
                [-10., 0., -10.],
                [-10., 0., 10.],
                [10., 0., 10.],
                [10., 0., -10.],
            ],
            &[[0, 1, 2], [0, 2, 3]],
        )
        .unwrap();
    let k = key(1, 0);
    world
        .spawn_box(
            k,
            BodyMode::Dynamic,
            [1.; 3],
            50.,
            BodyState::at([0., 5., 0.]),
        )
        .unwrap();
    for _ in 0..360 {
        world.step(1. / 60.).unwrap();
    }
    let state = world.state(k).unwrap();
    assert!((state.position[1] - 0.5).abs() < 0.1, "{state:?}");
    assert!(state.sleeping, "{state:?}");
    assert!(state.linear_velocity.iter().all(|v| v.abs() < 0.01));
}
#[test]
fn keys_generation_snapshots_and_explicit_restore_do_not_use_bevy_ids() {
    let mut world = RigidWorld::new();
    let old = key(10, 0);
    world
        .spawn_box(
            old,
            BodyMode::Kinematic,
            [1.; 3],
            1.,
            BodyState::at([1., 2., 3.]),
        )
        .unwrap();
    assert_eq!(
        world.spawn_box(old, BodyMode::Dynamic, [1.; 3], 1., BodyState::at([0.; 3])),
        Err(PhysicsError::DuplicateKey)
    );
    let mut state = BodyState::at([4., 5., 6.]);
    state.linear_velocity = [2., 0., 0.];
    world.set_state(old, state).unwrap();
    assert_eq!(world.snapshot(), vec![(old, state)]);
    world.step(0.).unwrap();
    assert_eq!(world.ticks, 0);
    world.step(1. / 60.).unwrap();
    assert!(
        world.state(old).unwrap().position[0] > 4.,
        "{:?}",
        world.state(old)
    );
    assert!(world.remove(old));
    assert!(!world.remove(old));
    let new = key(10, 1);
    world
        .spawn_box(new, BodyMode::Static, [1.; 3], 1., BodyState::at([0.; 3]))
        .unwrap();
    assert!(world.state(old).is_none());
    assert_eq!(world.set_state(old, state), Err(PhysicsError::MissingBody));
    assert_eq!(world.snapshot()[0].0, new);
}
#[test]
fn invalid_data_is_rejected_before_entering_the_solver() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<RigidWorld>();
    let mut world = RigidWorld::new();
    assert_eq!(
        world.spawn_box(
            key(1, 0),
            BodyMode::Dynamic,
            [0.; 3],
            1.,
            BodyState::at([0.; 3])
        ),
        Err(PhysicsError::InvalidBody)
    );
    assert_eq!(world.step(f32::NAN), Err(PhysicsError::InvalidDelta));
    assert_eq!(world.step(f32::MAX), Err(PhysicsError::InvalidDelta));
    assert_eq!(
        world.add_mesh(&[[0.; 3]], &[[0, 1, 2]]),
        Err(PhysicsError::InvalidMesh)
    );
    assert_eq!(
        world.add_mesh(&[[0.; 3]], &[[0, 0, 0]]),
        Err(PhysicsError::InvalidMesh)
    );
}

#[test]
fn rotated_boxes_and_sensors_use_their_collision_shape() {
    let mut w = RigidWorld::new();
    w.spawn_box(
        key(1, 0),
        BodyMode::Static,
        [20., 1., 20.],
        1.,
        BodyState::at([0., -0.5, 0.]),
    )
    .unwrap();
    // A 90-degree local rotation swaps the tall box's X/Y extents.
    let half = std::f32::consts::FRAC_1_SQRT_2;
    let shape = BodyShape::Box {
        size: [1., 4., 1.],
        rotation: [0., 0., half, half],
    };
    w.spawn_shape(
        key(2, 0),
        BodyMode::Static,
        &shape,
        1.,
        BodyState::at([0., 3., 0.]),
        true,
    )
    .unwrap();
    w.spawn_box(
        key(3, 0),
        BodyMode::Dynamic,
        [1.; 3],
        1.,
        BodyState::at([0., 7., 0.]),
    )
    .unwrap();
    w.spawn_shape(
        key(4, 0),
        BodyMode::Dynamic,
        &BodyShape::Box {
            size: [1.; 3],
            rotation: [0., 0., 0., 1.],
        },
        1.,
        BodyState::at([5., 7., 0.]),
        false,
    )
    .unwrap();
    for _ in 0..120 {
        w.step(1. / 60.).unwrap();
    }
    assert!((w.state(key(3, 0)).unwrap().position[1] - 4.).abs() < 0.15);
    assert!(w.state(key(4, 0)).unwrap().position[1] < -1.);
}
#[test]
fn mesh_shapes_restore_pose_and_reject_invalid_indices() {
    let mut w = RigidWorld::new();
    let mesh = BodyShape::Mesh {
        vertices: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        triangles: vec![[0, 1, 2], [0, 2, 3], [0, 3, 1], [1, 3, 2]],
    };
    w.spawn_shape(
        key(1, 0),
        BodyMode::Dynamic,
        &mesh,
        3.,
        BodyState::at([1., 5., 2.]),
        true,
    )
    .unwrap();
    w.step(1. / 60.).unwrap();
    assert!(w.state(key(1, 0)).unwrap().position[1] < 5.);
    let bad = BodyShape::Mesh {
        vertices: vec![[0.; 3]],
        triangles: vec![[0, 1, 2]],
    };
    assert_eq!(
        w.spawn_shape(
            key(2, 0),
            BodyMode::Static,
            &bad,
            1.,
            BodyState::at([0.; 3]),
            true
        ),
        Err(PhysicsError::InvalidMesh)
    );
}
