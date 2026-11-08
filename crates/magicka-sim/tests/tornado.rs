//! TornadoEntity core (research/tornado.md).
use magicka_sim::tornado::{self as tn, Tornado};

fn flat_ground(_: [f32; 3]) -> Option<f32> {
    Some(0.0)
}

#[test]
fn spawn_keeps_the_scaled_forward() {
    let (point, forward) = tn::spawn([1.0, 0.0, 2.0], [0.0, 0.0, -1.0]);
    assert_eq!(point, [1.0, 0.0, -2.0]);
    assert_eq!(forward, [0.0, 0.0, -4.0]);
}

/// Alpha ramps in over 0.25 s and out over the last second; the entity is
/// Dead once TTL drops below zero, and Kill leaves one second.
#[test]
fn lifetime_and_alpha() {
    let mut t = Tornado::new([0.0; 3], [0.0, 0.0, -1.0], None, 0.0);
    let dt = 0.125;
    t.update(dt, || 0.5, |_, _| None, flat_ground);
    assert_eq!(t.alpha, 0.0, "alpha uses the TTL before the decrement");
    t.update(dt, || 0.5, |_, _| None, flat_ground);
    assert!((t.alpha - 0.25).abs() < 1e-6);
    for _ in 0..200 {
        t.update(dt, || 0.5, |_, _| None, flat_ground);
    }
    assert!(t.dead());
    t.kill();
    assert!(!t.dead());
    assert_eq!(t.ttl, 1.0);
}

/// With no path, a 0.5/0.5 roll aims straight along the heading at
/// 5 + 0.5 × 10 = 10 m; the speed is |sin(TTL) · 4| in the XZ plane, and the
/// height eases toward the ground probe by dt per frame.
#[test]
fn wanders_along_the_heading_at_the_sine_speed() {
    let mut t = Tornado::new([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], None, 0.0);
    let dt = 0.1;
    t.update(dt, || 0.5, |_, _| None, flat_ground);
    let speed = ((tn::TTL - dt) as f64).sin().abs() as f32 * 4.0;
    assert!(
        (t.position[0] - speed * dt).abs() < 1e-5,
        "{:?}",
        t.position
    );
    assert!(t.position[2].abs() < 1e-5);
    assert!((t.position[1] - 0.9).abs() < 1e-6, "y eases toward 0");
}

/// A found path drops its start node and walks the following nodes,
/// skipping those within 1 m.
#[test]
fn follows_navmesh_paths() {
    let mut t = Tornado::new([0.0; 3], [1.0, 0.0, 0.0], None, 0.0);
    let path = vec![[0.0; 3], [0.0, 0.0, 5.0], [5.0, 0.0, 5.0]];
    t.update(0.1, || 0.5, |_, _| Some(path.clone()), flat_ground);
    assert!(
        t.position[2] > 0.0 && t.position[0].abs() < 1e-5,
        "toward (0,0,5)"
    );
    assert!(t.heading[2] > 0.99);
}

/// Each damageable is hit at most once per second.
#[test]
fn hit_cooldown_is_one_second() {
    let mut t = Tornado::new([0.0; 3], [1.0, 0.0, 0.0], None, 0.0);
    let point = t.hit(7, [4.0, 0.0, 0.0]).unwrap();
    assert!((point[0] - tn::RADIUS).abs() < 1e-6);
    assert!(t.hit(7, [4.0, 0.0, 0.0]).is_none());
    for _ in 0..9 {
        t.update(0.1, || 0.5, |_, _| None, flat_ground);
    }
    assert!(t.hit(7, [4.0, 0.0, 0.0]).is_none(), "0.9 s");
    t.update(0.11, || 0.5, |_, _| None, flat_ground);
    assert!(t.hit(7, [4.0, 0.0, 0.0]).is_some(), "after 1 s");
}

/// The capsule runs from the body position up 6 m with radius 2.5.
#[test]
fn capsule_contact() {
    let t = Tornado::new([0.0; 3], [1.0, 0.0, 0.0], None, 0.0);
    assert!(tn::touches(&t, [3.0, 0.0, 0.0], [3.0, 1.0, 0.0], 0.6));
    assert!(!tn::touches(&t, [3.2, 0.0, 0.0], [3.2, 1.0, 0.0], 0.6));
    assert!(
        tn::touches(&t, [0.0, 8.9, 0.0], [0.0, 9.5, 0.0], 0.5),
        "3 m above the top"
    );
    assert!(!tn::touches(&t, [0.0, 9.1, 0.0], [0.0, 9.5, 0.0], 0.5));
    assert_eq!(tn::damage()[0].amount, 800.0);
}

/// The authored enhancement must carry a free body to the top under the
/// campaign's gravity, rather than losing the lift to gravity or ejecting
/// the body sideways. Exercise the complete trajectory at several rates.
#[test]
fn enhanced_field_lifts_and_flings_under_campaign_gravity() {
    use tn::field::{self, Effect};
    for hz in [30, 60, 120] {
        let dt = 1.0 / hz as f32;
        let mut t = Tornado::new([0.0; 3], [0.0, 0.0, -1.0], None, 0.0);
        let mut position = [2.0, 0.0, 0.0];
        let mut velocity = [0.0; 3];
        let mut flung = false;
        for _ in 0..hz * 2 {
            let effect = field::effect(&t, 7, position, 0.3, dt);
            let impulse = match effect {
                Some(Effect::Lift(v)) => v,
                Some(Effect::Fling(v)) => {
                    assert!(position[1] >= tn::LENGTH * 0.85);
                    assert!(position[0] * v[0] + position[2] * v[2] > 0.0);
                    assert!(v[1] > 0.0);
                    t.flung.add_for(7, field::FLING_COOLDOWN);
                    assert_eq!(field::effect(&t, 7, position, 0.3, dt), None);
                    flung = true;
                    break;
                }
                _ => panic!("left the funnel before reaching its top at {hz} Hz: {position:?}"),
            };
            for i in 0..3 {
                velocity[i] += impulse[i] + magicka_sim::character_physics::GRAVITY[i] * dt;
                position[i] += velocity[i] * dt;
            }
        }
        assert!(flung, "never reached the top at {hz} Hz: {position:?}");
    }
}
