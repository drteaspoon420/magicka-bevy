use magicka_sim::{
    capsule_pair::{detect, CapsuleMotion},
    spray_pulse::Segment,
};

#[test]
fn pair_impulse_conserves_momentum_and_applies_both_inverse_masses() {
    let contact = detect(capsule(0.0, 0.0), capsule(0.8, 0.0), 0.05, || 0).unwrap();
    let mut velocities = [[3.0, 2.0, 4.0], [0.0, -1.0, 5.0]];
    assert!(magicka_sim::capsule_pair::solve_normal(
        contact,
        0.0,
        [0.5, 1.0],
        &mut velocities
    ));
    assert_eq!(velocities, [[2.0, 2.0, 4.0], [2.0, -1.0, 5.0]]);
    assert_eq!(2.0 * velocities[0][0] + velocities[1][0], 6.0);
    assert!(!magicka_sim::capsule_pair::solve_normal(
        contact,
        0.0,
        [0.5, 1.0],
        &mut velocities
    ));
}

#[test]
fn penetration_bias_separates_stationary_bodies_but_keeps_tangential_motion() {
    let contact = detect(capsule(0.0, 0.0), capsule(0.8, 0.0), 0.05, || 0).unwrap();
    let mut velocities = [[0.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
    assert!(magicka_sim::capsule_pair::solve_normal(
        contact,
        2.0,
        [1.0, 1.0],
        &mut velocities
    ));
    assert_eq!(velocities, [[-1.0, 1.0, 0.0], [1.0, 1.0, 0.0]]);
    assert!(!magicka_sim::capsule_pair::solve_normal(
        contact,
        2.0,
        [1.0, 1.0],
        &mut velocities
    ));
}

fn capsule(x: f32, y: f32) -> CapsuleMotion {
    let axis = Segment {
        origin: [x, y, 0.0],
        delta: [0.0, 1.0, 0.0],
    };
    CapsuleMotion {
        old_axis: axis,
        new_axis: axis,
        old_radius: 0.5,
        new_radius: 0.5,
        old_body_position: [x, y + 0.5, 0.0],
    }
}

#[test]
fn physics_pair_contacts_separate_both_bodies_without_marking_ground() {
    use magicka_sim::character_physics::{CharacterBody, CollisionTriangle, PhysicsWorld};
    let mut world = PhysicsWorld::new(-100.0);
    for (x, mass) in [(0.0, 2.0), (0.8, 1.0)] {
        let mut body = CharacterBody::new(0.5, 1.0, mass, 0.0);
        body.position = [x, 10.0, 0.0];
        world.bodies.push(body);
    }
    let before = world.clone();
    let events = world.integrate_with_pairs(
        1.0 / 60.0,
        &Vec::<CollisionTriangle>::new(),
        &|_, _| None,
        &|_, _| true,
    );
    assert!(events.is_empty());
    assert!(world.bodies[0].velocity[0] < 0.0);
    assert!(world.bodies[1].velocity[0] > 0.0);
    assert!((2.0 * world.bodies[0].velocity[0] + world.bodies[1].velocity[0]).abs() < 1e-6);
    assert!(world.bodies[1].position[0] - world.bodies[0].position[0] > 0.8);
    let mut rejected = before;
    rejected.integrate_with_pairs(
        1.0 / 60.0,
        &Vec::<CollisionTriangle>::new(),
        &|_, _| None,
        &|_, _| false,
    );
    assert_eq!(rejected.bodies[0].position[0], 0.0);
    assert_eq!(rejected.bodies[1].position[0], 0.8);
    for i in 0..2 {
        assert_eq!(
            world.bodies[i].ground_contact_normal,
            rejected.bodies[i].ground_contact_normal
        );
        assert_eq!(
            world.bodies[i].is_touching_ground(),
            rejected.bodies[i].is_touching_ground()
        );
    }
}

#[test]
fn overlap_uses_horizontal_normal_and_body_relative_offsets() {
    let hit = detect(capsule(0.0, 0.0), capsule(0.8, 0.0), 0.05, || {
        panic!("no random draw")
    })
    .unwrap();
    assert!((hit.penetration - 0.2).abs() < 1e-6);
    assert_eq!(hit.normal, [-1.0, 0.0, 0.0]);
    assert!((hit.point[0] - 0.4).abs() < 1e-6);
    assert!((hit.offset0[0] - 0.4).abs() < 1e-6);
    assert!((hit.offset1[0] + 0.4).abs() < 1e-6);
}

#[test]
fn predicted_touch_retains_old_negative_penetration() {
    let mut b = capsule(2.0, 0.0);
    b.new_axis.origin[0] = 0.8;
    let hit = detect(capsule(0.0, 0.0), b, 0.05, || panic!("no random draw")).unwrap();
    assert_eq!(hit.penetration, -1.0);
    assert_eq!(hit.normal, [-1.0, 0.0, 0.0]);
    assert_eq!(hit.point[0], 1.0);
}

#[test]
fn strict_boundary_and_coincident_fallback() {
    assert!(
        detect(capsule(0.0, 0.0), capsule(1.05, 0.0), 0.05, || panic!(
            "separated pairs must not draw random"
        ))
        .is_none()
    );
    let hit = detect(capsule(0.0, 0.0), capsule(0.0, 0.0), 0.05, || 90).unwrap();
    assert!((hit.normal[0] - 1.0).abs() < 1e-6);
    assert_eq!(hit.normal[1], 0.0);
    assert!(hit.normal[2].abs() < 1e-6);
    assert_eq!(hit.penetration, 1.0);
}

#[test]
fn vertical_separation_keeps_old_distance_in_horizontal_normal_divisor() {
    let hit = detect(capsule(0.0, 0.0), capsule(0.3, 1.4), 0.05, || {
        panic!("no random draw")
    })
    .unwrap();
    assert!(hit.normal[0].abs() < 0.7);
    assert_eq!(hit.normal[1], 0.0);
    assert!((hit.penetration - 0.5).abs() < 1e-6);
}
