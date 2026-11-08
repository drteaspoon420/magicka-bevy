//! VortexEntity core (research/vortex.md).
use magicka_sim::vortex::{self as vx, Vortex};

#[test]
fn spawns_ten_metres_ahead_with_quarter_mass() {
    let c = Vortex::spawn_point([1.0, 0.5, 0.0], [0.0, 0.0, -1.0]);
    assert_eq!(c, [1.0, 0.5, -10.0]);
    let v = Vortex::new(Some(3), c);
    assert_eq!((v.mass, v.target_mass), (0.25, 0.25));
}

/// Mass eases toward the target (Lerp with 0.1^dt); the reach is 10 · mass;
/// the target decays 0.05/s, so an unfed vortex dies.
#[test]
fn unfed_vortex_shrinks_and_dies() {
    let mut v = Vortex::new(None, [0.0; 3]);
    assert_eq!(v.begin_update(1.0 / 60.0), Some(2.5));
    v.end_update(1.0 / 60.0);
    let mut frames = 1;
    while !v.dead() && frames < 60 * 60 {
        if v.begin_update(1.0 / 60.0).is_some() {
            v.end_update(1.0 / 60.0);
        }
        frames += 1;
    }
    assert!(v.dead(), "dead after {frames} frames");
    assert!(frames > 5 * 60, "lives past the 5 s target decay: {frames}");
}

/// Absorbing body mass m: target = 1 − 0.75^(log_0.75(1 − target) + m).
#[test]
fn absorption_grows_the_target_mass() {
    let mut v = Vortex::new(None, [0.0; 3]);
    v.absorb(1.0);
    let expected = 1.0 - 0.75f64.powf((0.75f64).ln() / 0.75f64.ln() + 1.0);
    assert!(
        (f64::from(v.target_mass) - expected).abs() < 1e-6,
        "{}",
        v.target_mass
    );
    assert!((v.target_mass - 0.4375).abs() < 1e-5);
}

/// The pull is 15 · (1 − d/r) toward the centre; zero at the rim.
#[test]
fn pull_falls_off_linearly() {
    let mut v = Vortex::new(None, [0.0; 3]);
    v.begin_update(0.0);
    let f = v.pull([1.25, 0.0, 0.0]).unwrap();
    assert!((f[0] + 7.5).abs() < 1e-5, "{f:?}");
    assert!(v.pull([2.5, 0.0, 0.0]).is_none());
    assert!(v.pull([0.0; 3]).is_none());
}

#[test]
fn core_touch_uses_the_bounding_box() {
    let v = Vortex::new(None, [0.0; 3]);
    assert!(v.core_touches([0.4, -1.0, -0.1], [1.0, 1.0, 0.1]));
    assert!(!v.core_touches([0.6, -1.0, -0.1], [1.0, 1.0, 0.1]));
    assert_eq!(vx::POOL_SIZE, 8);
}
