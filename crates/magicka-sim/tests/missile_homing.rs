use magicka_sim::missile_homing::{transform, Homing};

#[test]
fn homing_turns_towards_the_target_and_gives_up_past_the_tolerance() {
    // Flying +X, target straight up the +Z side: the velocity turns to +Z.
    let mut h = Homing::new(Some(1), 0.55);
    let v = h.steer(
        Some(([0.0, 0.0, 10.0], 0.0)),
        [0.0; 3],
        [10.0, 0.0, 0.0],
        0.1,
    );
    assert!(v[2] > 0.0 && v[0] < 10.0, "{v:?}");
    assert_eq!(h.last_known, [0.0, 0.0, 10.0]);
    // A target behind (dot <= -0.75) switches homing off for good.
    let mut h = Homing::new(Some(1), 0.55);
    h.steer(
        Some(([-10.0, 0.0, 0.1], 0.5)),
        [0.0; 3],
        [10.0, 0.0, 0.0],
        0.1,
    );
    assert_eq!(h.power, 0.0);
    let v = h.steer(
        Some(([0.0, 0.0, 10.0], 0.0)),
        [0.0; 3],
        [10.0, 0.0, 0.0],
        0.1,
    );
    assert_eq!(v, [10.0, 0.0, 0.0], "off");
    // No target: the zeroed last known position (the world origin).
    let mut h = Homing::new(None, 0.55);
    let v = h.steer(None, [0.0, 0.0, -10.0], [10.0, 0.0, 0.0], 0.1);
    assert!(v[2] > 0.0, "towards the origin: {v:?}");
    // A unit quaternion rotates without scaling.
    let q = [0.0, (0.5f32).sin(), 0.0, (0.5f32).cos()];
    let r = transform([1.0, 0.0, 0.0], q);
    assert!(((r[0] * r[0] + r[2] * r[2]).sqrt() - 1.0).abs() < 1e-6);
}
