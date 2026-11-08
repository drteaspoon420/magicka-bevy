use magicka_sim::{
    character_physics::{CharacterBody, CollisionTriangle, PhysicsWorld, ShieldContacts},
    hollow_sphere_box::PrimitiveMotion,
    shield_intersection::{HollowSphere, ShieldGeometry},
};
use std::{cell::RefCell, f32::consts::PI};

fn world(count: usize) -> PhysicsWorld {
    let mut world = PhysicsWorld::new(-1000.0);
    for i in 0..count {
        let mut body = CharacterBody::new(0.75, 0.5, 70.0, 5.0);
        body.position = [4.5 + i as f32 * 0.2, 100.0, 0.0];
        body.apply_gravity = false;
        // Let the native ground-contact grace period expire in empty space.
        body.add_external_forces(0.3, None);
        body.velocity = [1.0, 0.0, 0.0];
        world.bodies.push(body);
    }
    world
}
fn geometry() -> [PrimitiveMotion<ShieldGeometry>; 1] {
    let sphere = HollowSphere {
        center: [0.0, 100.0, 0.0],
        radius: 5.0,
        forward: [1.0, 0.0, 0.0],
        max_angle: PI,
    };
    [PrimitiveMotion {
        old: ShieldGeometry::Sphere(sphere),
        new: ShieldGeometry::Sphere(sphere),
        owner_old_position: sphere.center,
    }]
}

#[test]
fn pair_and_shield_callbacks_share_one_prediction_then_restore_transform_rate() {
    let mut world = world(3);
    let seen = RefCell::new(vec![]);
    let mut pair = |i, j, a: &mut CharacterBody, _: &mut CharacterBody| {
        if i == 0 && j == 1 {
            assert!((a.position[0] - 4.6).abs() < 1e-5);
            a.position[0] += 0.05;
            a.velocity[0] = 77.0;
            a.mass = 222.0;
            a.add_impulse_velocity([3.0, 0.0, 0.0]);
        } else if i == 0 && j == 2 {
            seen.borrow_mut()
                .push(("pair", a.position[0], a.velocity[0], a.mass));
        }
        false
    };
    let mut shield = |i, _, body: &mut CharacterBody| {
        if i == 0 {
            seen.borrow_mut()
                .push(("shield", body.position[0], body.velocity[0], body.mass));
        }
        false
    };
    world.integrate_with_shields(
        0.1,
        &Vec::<CollisionTriangle>::new(),
        &|_, _| None,
        &|_, _| true,
        Some(ShieldContacts {
            obstacles: &[],
            accept_obstacle: None,
            geometry: &geometry(),
            accept: &mut shield,
            pair: Some(&mut pair),
        }),
    );
    let seen = seen.into_inner();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].0, "pair");
    assert_eq!(seen[1].0, "shield");
    for (_, position, velocity, mass) in seen {
        assert!((position - 4.65).abs() < 1e-5);
        assert_eq!(velocity, 77.0);
        assert_eq!(mass, 222.0);
    }
    let body = &world.bodies[0];
    assert!((body.position[0] - 4.6).abs() < 1e-5);
    assert_eq!(body.velocity, [1.0, 0.0, 0.0]);
    assert_eq!(body.mass, 222.0);
    assert!(body.is_pushed());
    assert!(world.contact_points(0).is_empty());
    // The callback's CharacterBody impulse survives RestoreState and is
    // consumed once by the next external-force pass.
    world.integrate(0.1, &Vec::<CollisionTriangle>::new(), &|_, _| None);
    assert_eq!(world.bodies[0].velocity, [4.0, 0.0, 0.0]);
    world.integrate(0.1, &Vec::<CollisionTriangle>::new(), &|_, _| None);
    assert_eq!(world.bodies[0].velocity, [4.0, 0.0, 0.0]);
}

#[test]
fn initially_active_body_is_restored_even_if_callback_disables_it() {
    let mut world = world(2);
    let original = world.bodies[1].position;
    let mut pair = |_, _, _: &mut CharacterBody, b: &mut CharacterBody| {
        b.position = [99.0; 3];
        b.velocity = [88.0; 3];
        b.active = false;
        b.mass = 333.0;
        false
    };
    let mut shield = |_, _, _: &mut CharacterBody| false;
    world.integrate_with_shields(
        0.1,
        &Vec::<CollisionTriangle>::new(),
        &|_, _| None,
        &|_, _| true,
        Some(ShieldContacts {
            obstacles: &[],
            accept_obstacle: None,
            geometry: &[],
            accept: &mut shield,
            pair: Some(&mut pair),
        }),
    );
    let body = &world.bodies[1];
    assert_eq!(body.position, original);
    assert_eq!(body.velocity, [1.0, 0.0, 0.0]);
    assert!(!body.active);
    assert_eq!(body.mass, 333.0);
}
