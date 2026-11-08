//! CharacterBody control law + capsule/level contact behaviour.
//! Wizard template values: Length .5, Radius .75, Mass 70, Speed 5, TurnSpeed 7.
use magicka_sim::character_physics::*;

const DT: f32 = 1.0 / 60.0;
type V = [f32; 3];

fn wizard() -> CharacterBody {
    let mut body = CharacterBody::new(0.75, 0.5, 70.0, 5.0);
    body.owner.turn_speed = 7.0;
    body
}

/// Body centre for feet at `feet` (HeightOffset = -R - L/2 = -1).
fn at_feet(feet: V) -> V {
    [feet[0], feet[1] + 1.0, feet[2]]
}

/// Quad split as (p0,p1,p2),(p0,p2,p3), wound so its normal matches `out`.
fn quad(p: [V; 4], out: V, primitive: u32, material: u8) -> Vec<CollisionTriangle> {
    let mut t = vec![
        CollisionTriangle::new(p[0], p[1], p[2], primitive, material),
        CollisionTriangle::new(p[0], p[2], p[3], primitive, material),
    ];
    for tri in &mut t {
        let n = tri.normal;
        if n[0] * out[0] + n[1] * out[1] + n[2] * out[2] < 0.0 {
            *tri = CollisionTriangle::new(
                tri.vertices[0],
                tri.vertices[2],
                tri.vertices[1],
                primitive,
                material,
            );
        }
        let n = tri.normal;
        assert!(n[0] * out[0] + n[1] * out[1] + n[2] * out[2] > 0.99);
    }
    t
}

/// Axis-aligned box with outward faces.
fn block(min: V, max: V, primitive: u32) -> Vec<CollisionTriangle> {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let mut t = Vec::new();
    t.extend(quad(
        [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
        [0.0, 1.0, 0.0],
        primitive,
        5,
    ));
    t.extend(quad(
        [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
        [0.0, -1.0, 0.0],
        primitive,
        5,
    ));
    t.extend(quad(
        [[x0, y0, z0], [x0, y1, z0], [x0, y1, z1], [x0, y0, z1]],
        [-1.0, 0.0, 0.0],
        primitive,
        5,
    ));
    t.extend(quad(
        [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
        [1.0, 0.0, 0.0],
        primitive,
        5,
    ));
    t.extend(quad(
        [[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0]],
        [0.0, 0.0, -1.0],
        primitive,
        5,
    ));
    t.extend(quad(
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        [0.0, 0.0, 1.0],
        primitive,
        5,
    ));
    t
}

fn floor(x0: f32, x1: f32, primitive: u32, material: u8) -> Vec<CollisionTriangle> {
    quad(
        [
            [x0, 0.0, -20.0],
            [x1, 0.0, -20.0],
            [x1, 0.0, 20.0],
            [x0, 0.0, 20.0],
        ],
        [0.0, 1.0, 0.0],
        primitive,
        material,
    )
}

fn world_with(body: CharacterBody) -> PhysicsWorld {
    let mut world = PhysicsWorld::new(-50.0);
    world.bodies.push(body);
    world
}

fn run(
    world: &mut PhysicsWorld,
    level: &dyn StaticGeometry,
    steps: usize,
) -> Vec<(usize, BodyEvent)> {
    let mut events = Vec::new();
    for _ in 0..steps {
        events.extend(world.integrate(DT, level, &|_, _| None));
    }
    events
}

#[test]
fn rests_on_flat_ground_with_bounded_penetration() {
    let level = floor(-20.0, 20.0, 2, 2);
    let mut body = wizard();
    body.move_to(at_feet([0.0, 0.0, 0.0]), Orientation::IDENTITY);
    let mut world = world_with(body);
    let events = run(&mut world, &level, 180);
    let b = &world.bodies[0];
    assert!(events.is_empty(), "{events:?}");
    assert!(b.feet()[1].abs() < 0.01, "feet {:?}", b.feet());
    assert!(b.velocity[1].abs() < 0.2, "velocity {:?}", b.velocity);
    assert!(b.is_touching_ground());
    assert!(!b.moving());
    // Static layer primitive index is reported as the ground material.
    assert_eq!(b.ground_material, 2);
}

#[test]
fn drops_onto_ground_and_settles() {
    let level = floor(-20.0, 20.0, 0, 0);
    let mut body = wizard();
    body.move_to(at_feet([0.0, 0.5, 0.0]), Orientation::IDENTITY);
    let mut world = world_with(body);
    run(&mut world, &level, 120);
    let b = &world.bodies[0];
    assert!(b.feet()[1].abs() < 0.01, "feet {:?}", b.feet());
    assert!(b.is_touching_ground());
}

#[test]
fn walks_at_template_speed_without_ground_friction() {
    let level = floor(-20.0, 20.0, 0, 0);
    let mut body = wizard();
    body.move_to(at_feet([-10.0, 0.0, 0.0]), Orientation::IDENTITY);
    let mut world = world_with(body);
    run(&mut world, &level, 30);
    world.bodies[0].set_movement([1.0, 0.0, 0.0]);
    run(&mut world, &level, 30);
    let x0 = world.bodies[0].position[0];
    run(&mut world, &level, 60);
    let b = &world.bodies[0];
    assert!(
        (b.position[0] - x0 - 5.0).abs() < 1e-3,
        "{}",
        b.position[0] - x0
    );
    assert_eq!(b.velocity[0], 5.0);
    assert!(b.moving());
    assert!(b.feet()[1].abs() < 0.01);
    let f = b.direction();
    assert!((f[0] - 1.0).abs() < 1e-3 && f[2].abs() < 0.05, "{f:?}");
}

#[test]
fn movement_setter_normalises_long_input_and_keeps_direction_on_zero() {
    let mut body = wizard();
    body.set_movement([0.3, 7.0, 0.4]);
    assert_eq!(body.movement(), [0.3, 0.0, 0.4]);
    let d = body.desired_direction();
    assert!((d[0] - 0.6).abs() < 1e-6 && (d[2] - 0.8).abs() < 1e-6);
    body.set_movement([3.0, 0.0, 4.0]);
    let m = body.movement();
    assert!((m[0] - 0.6).abs() < 1e-6 && (m[2] - 0.8).abs() < 1e-6);
    body.set_movement([0.0; 3]);
    assert_eq!(body.movement(), [0.0; 3]);
    let d = body.desired_direction();
    assert!((d[0] - 0.6).abs() < 1e-6 && (d[2] - 0.8).abs() < 1e-6);
}

#[test]
fn turning_is_quaternion_lerp_by_dt_times_turn_speed() {
    let level = floor(-20.0, 20.0, 0, 0);
    let mut body = wizard();
    body.move_to(at_feet([0.0, 0.0, 0.0]), Orientation::IDENTITY);
    body.set_desired_direction([1.0, 0.0, 0.0]);
    let mut world = world_with(body);
    run(&mut world, &level, 1);
    // One step: nlerp(identity, yaw -90°, 7/60).
    let target = Orientation::from_forward_up([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).to_quaternion();
    let expected =
        Orientation::from_quaternion(quaternion_lerp([0.0, 0.0, 0.0, 1.0], target, 7.0 * DT))
            .forward();
    let got = world.bodies[0].direction();
    for i in 0..3 {
        assert!((got[i] - expected[i]).abs() < 1e-6, "{got:?} {expected:?}");
    }
    assert!(got[0] > 0.0 && got[0] < 0.5);
    run(&mut world, &level, 120);
    let f = world.bodies[0].direction();
    assert!((f[0] - 1.0).abs() < 1e-4, "{f:?}");
}

#[test]
fn climbs_step_within_step_size() {
    let mut level = floor(-20.0, 2.0, 0, 0);
    level.extend(block([2.0, 0.0, -20.0], [40.0, 0.4, 20.0], 1));
    let mut body = wizard();
    body.move_to(at_feet([0.0, 0.0, 0.0]), Orientation::IDENTITY);
    let mut world = world_with(body);
    world.bodies[0].set_movement([1.0, 0.0, 0.0]);
    run(&mut world, &level, 120);
    let b = &world.bodies[0];
    assert!(b.position[0] > 8.0, "{:?}", b.position);
    assert!((b.feet()[1] - 0.4).abs() < 0.02, "feet {:?}", b.feet());
}

#[test]
fn wall_above_step_size_blocks() {
    let mut level = floor(-20.0, 20.0, 0, 0);
    level.extend(block([2.0, 0.0, -20.0], [3.0, 0.8, 20.0], 1));
    let mut body = wizard();
    body.move_to(at_feet([0.0, 0.0, 0.0]), Orientation::IDENTITY);
    let mut world = world_with(body);
    world.bodies[0].set_movement([1.0, 0.0, 0.0]);
    run(&mut world, &level, 120);
    let b = &world.bodies[0];
    assert!((b.position[0] - 1.25).abs() < 0.02, "{:?}", b.position);
    assert!(b.feet()[1].abs() < 0.02, "feet {:?}", b.feet());
}

fn ramp(angle_degrees: f32) -> Vec<CollisionTriangle> {
    let rise = angle_degrees.to_radians().tan();
    let mut level = floor(-20.0, 2.0, 0, 0);
    level.extend(quad(
        [
            [2.0, 0.0, -20.0],
            [12.0, 10.0 * rise, -20.0],
            [12.0, 10.0 * rise, 20.0],
            [2.0, 0.0, 20.0],
        ],
        [-rise, 1.0, 0.0],
        1,
        0,
    ));
    level
}

#[test]
fn walkable_ramp_is_climbed_and_steep_ramp_blocks() {
    let mut body = wizard();
    body.move_to(at_feet([0.0, 0.0, 0.0]), Orientation::IDENTITY);
    let mut world = world_with(body.clone());
    world.bodies[0].set_movement([1.0, 0.0, 0.0]);
    let level = ramp(30.0);
    run(&mut world, &level, 90);
    let b = &world.bodies[0];
    let x = b.position[0];
    assert!(x > 5.0, "{:?}", b.position);
    // Feet follow the surface: y = (x - 2) * tan 30°.
    let surface = (x - 2.0) * 30f32.to_radians().tan();
    assert!(
        (b.feet()[1] - surface).abs() < 0.1,
        "{:?} {surface}",
        b.feet()
    );

    let mut world = world_with(body);
    world.bodies[0].set_movement([1.0, 0.0, 0.0]);
    let level = ramp(60.0);
    run(&mut world, &level, 90);
    let b = &world.bodies[0];
    assert!(b.position[0] < 2.5, "{:?}", b.position);
    assert!(b.feet()[1] < 0.6, "{:?}", b.feet());
}

#[test]
fn short_drop_is_harmless_long_drop_reports_one_impact() {
    let level = floor(-20.0, 20.0, 0, 0);
    let mut body = wizard();
    body.move_to(at_feet([0.0, 1.0, 0.0]), Orientation::IDENTITY);
    let mut world = world_with(body.clone());
    assert!(run(&mut world, &level, 120).is_empty());

    body.move_to(at_feet([0.0, 3.0, 0.0]), Orientation::IDENTITY);
    let mut world = world_with(body);
    let events = run(&mut world, &level, 120);
    let impacts: Vec<_> = events
        .iter()
        .filter_map(|(_, e)| match e {
            BodyEvent::ImpactDamage { amount, knockdown } => Some((*amount, *knockdown)),
            _ => None,
        })
        .collect();
    // v = sqrt(2 * 19.64 * 3) ≈ 10.9 m/s → amount = v^2.7 * 100 / 10^2.7.
    assert_eq!(impacts.len(), 1, "{events:?}");
    assert!(impacts[0].0 > 100.0 && impacts[0].0 < 135.0, "{impacts:?}");
    assert!(impacts[0].1);
    assert!(world.bodies[0].feet()[1].abs() < 0.02);
}

#[test]
fn fast_fall_uses_sweep_and_does_not_tunnel() {
    let level = floor(-20.0, 20.0, 0, 0);
    let mut body = wizard();
    body.move_to(at_feet([0.0, 4.0, 0.0]), Orientation::IDENTITY);
    body.velocity = [0.0, -45.0, 0.0];
    let mut world = world_with(body);
    for _ in 0..240 {
        world.integrate(DT, &level, &|_, _| None);
        assert!(
            world.bodies[0].feet()[1] > -0.3,
            "{:?}",
            world.bodies[0].feet()
        );
    }
    assert!(world.bodies[0].feet()[1].abs() < 0.02);
}

#[test]
fn kill_plane_reports_when_capsule_reaches_it() {
    let mut body = wizard();
    body.move_to([0.0, -45.0, 0.0], Orientation::IDENTITY);
    let mut world = world_with(body);
    let empty: Vec<CollisionTriangle> = Vec::new();
    let mut first = None;
    for step in 0..240 {
        let events = world.integrate(DT, &empty, &|_, _| None);
        if events.iter().any(|(_, e)| *e == BodyEvent::KillPlane) {
            first = Some((step, world.bodies[0].position));
            break;
        }
    }
    let (_, position) = first.expect("kill plane");
    // Bottom sphere centre (y - L/2) came within tolerance + radius of y=-50.
    assert!(position[1] - 0.25 < -50.0 + 0.8 + 0.5, "{position:?}");
}

#[test]
fn deterministic_corner_contacts() {
    let mut level = floor(-20.0, 20.0, 0, 0);
    level.extend(block([2.0, 0.0, -20.0], [3.0, 3.0, 20.0], 1));
    level.extend(block([-20.0, 0.0, 2.0], [20.0, 3.0, 3.0], 2));
    let make = || {
        let mut body = wizard();
        body.move_to(at_feet([0.0, 0.0, 0.0]), Orientation::IDENTITY);
        body.set_movement([1.0, 0.0, 1.0]);
        world_with(body)
    };
    let mut a = make();
    let mut b = make();
    for _ in 0..300 {
        a.integrate(DT, &level, &|_, _| None);
        b.integrate(DT, &level, &|_, _| None);
    }
    let (pa, pb) = (a.bodies[0].position, b.bodies[0].position);
    assert_eq!(pa.map(f32::to_bits), pb.map(f32::to_bits));
    // Wedged in the corner, not through either wall.
    assert!(
        (pa[0] - 1.25).abs() < 0.03 && (pa[2] - 1.25).abs() < 0.03,
        "{pa:?}"
    );
}

#[test]
fn closest_point_queries() {
    let tri = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    let (d, s, t) = point_triangle_distance_sq([0.25, 2.0, 0.25], &tri);
    assert!((d - 4.0).abs() < 1e-6 && (s - 0.25).abs() < 1e-6 && (t - 0.25).abs() < 1e-6);
    let (d, s, t) = point_triangle_distance_sq([2.0, 0.0, 2.0], &tri);
    assert!((d - 4.5).abs() < 1e-5 && (s - 0.5).abs() < 1e-6 && (t - 0.5).abs() < 1e-6);
    // Vertical segment crossing the triangle: distance zero at the crossing.
    let (d, seg_t, ..) = segment_triangle_distance_sq([0.2, -1.0, 0.2], [0.0, 2.0, 0.0], &tri);
    assert_eq!(d, 0.0);
    assert!((seg_t - 0.5).abs() < 1e-6);
    // Parallel segment above an edge region.
    let (d, ..) = segment_triangle_distance_sq([-1.0, 0.5, -0.5], [3.0, 0.0, 0.0], &tri);
    assert!((d - 0.5).abs() < 1e-5, "{d}");
}
