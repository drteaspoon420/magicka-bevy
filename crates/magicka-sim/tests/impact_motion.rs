use magicka_sim::damage::{attack as A, Damage};
use magicka_sim::impact_motion::*;
use magicka_sim::spells::Elements as E;
fn context() -> MotionContext {
    MotionContext {
        mass: 70.0,
        gravity_y: -19.64,
        ..Default::default()
    }
}
fn damage(attack: u16, element: E, amount: f32, magnitude: f32) -> Damage {
    Damage {
        attack,
        element: element.0,
        amount,
        magnitude,
    }
}
#[test]
fn ballistic_range_and_direction_match_independent_solution() {
    let c = context();
    let v = c.impulse([1.0, 42.0, 0.0], std::f32::consts::FRAC_PI_4, 70.0, 2.0);
    // mass==power gives scale 1. Range 2 at 45 degrees, gravity 19.64,
    // speed multiplier 1.4: each component = 1.4*sqrt(19.64).
    let expected = 1.4 * 19.64f32.sqrt();
    assert!((v[0] - expected).abs() < 1e-5);
    assert!((v[1] - expected).abs() < 1e-5);
    assert_eq!(v[2], 0.0);
    assert_eq!(c.impulse([0.0; 3], 0.7, 70.0, 2.0), [0.0; 3]);
    assert_eq!(c.impulse([1.0, 0.0, 0.0], 0.7, 0.0, 2.0), [0.0; 3]);
}
#[test]
fn heavy_mass_and_character_guards_suppress_impulses() {
    for c in [
        MotionContext {
            mass: 1001.0,
            ..context()
        },
        MotionContext {
            mass: 100.0,
            ..context()
        },
        MotionContext {
            immortal: true,
            ..context()
        },
        MotionContext {
            ethereal: true,
            ..context()
        },
        MotionContext {
            solid_shield: true,
            ..context()
        },
        MotionContext {
            entangled: true,
            ..context()
        },
        MotionContext {
            dead: true,
            ..context()
        },
    ] {
        assert_eq!(c.impulse([1.0, 0.0, 0.0], 0.7, 70.0, 2.0), [0.0; 3]);
    }
}
#[test]
fn disabled_knockback_still_sets_result_and_knockdown() {
    let mut state = ImpactMotion {
        knockdown_tolerance: 2.0,
        velocity: [3.0, 4.0, 5.0],
        ..Default::default()
    };
    let result = state.apply_impact(
        damage(A::KNOCKBACK, E::EARTH, 100.0, 1.0),
        [-1.0, 0.0, 0.0],
        context(),
        false,
    );
    assert_eq!(result, 8);
    assert!(state.knocked_down);
    assert!(!state.is_pushed);
    assert_eq!(state.impulse, [0.0; 3]);
    assert_eq!(state.velocity, [3.0, 4.0, 5.0]);
}
#[test]
fn knockdown_tolerance_sentinel_and_overloads_differ() {
    let mut state = ImpactMotion {
        knockdown_tolerance: 2.0,
        velocity: [3.0, 4.0, 5.0],
        grip_damage: 6.0,
        ..Default::default()
    };
    state.knock_down(Some(1.0));
    assert!(!state.knocked_down);
    state.knock_down(Some(2.0));
    assert!(state.knocked_down);
    assert_eq!(state.velocity, [0.0, 4.0, 0.0]);
    assert_eq!(state.grip_damage, 6.0);
    state.knock_down(None);
    assert_eq!(state.grip_damage, 0.0);
    state.knocked_down = false;
    state.knockdown_tolerance = 5000.0;
    state.knock_down(Some(9000.0));
    state.knock_down(None);
    assert!(!state.knocked_down);
}
#[test]
fn impulse_accumulates_before_leaving_ground_threshold() {
    let mut state = ImpactMotion::default();
    state.add_impulse([1.0, 2.0, 3.0], false);
    assert!(state.is_pushed);
    assert!(!state.leaving_ground);
    state.add_impulse([1.0, 1.0, 3.0], false);
    assert_eq!(state.impulse, [2.0, 3.0, 6.0]);
    assert!(state.leaving_ground);
    state.add_impulse([100.0; 3], true);
    assert_eq!(state.impulse, [2.0, 3.0, 6.0]);
}
#[test]
fn frontal_cone_ignores_height_and_rejects_coincident_or_side_hits() {
    let p = [0.0; 3];
    let forward = [0.0, 1.0, -2.0];
    assert_eq!(
        frontal_block(p, forward, [0.0, 99.0, -1.0], &[10, 20]),
        Some(30.0)
    );
    assert_eq!(frontal_block(p, forward, [1.0, 0.0, 0.0], &[30]), None);
    assert_eq!(frontal_block(p, forward, [0.0, 0.0, 1.0], &[30]), None);
    assert_eq!(frontal_block(p, forward, p, &[30]), None);
    for (degrees, blocked) in [(62.0f64, true), (64.0, false)] {
        let a = degrees.to_radians();
        assert_eq!(
            frontal_block(p, forward, [a.sin() as f32, 0.0, -a.cos() as f32], &[30]).is_some(),
            blocked
        );
    }
}
#[test]
fn partial_block_subtracts_before_magnitude_full_block_keeps_entry() {
    let mut d = damage(A::DAMAGE | A::STATUS, E::EARTH, 100.0, 2.0);
    let outcome = block_impact(
        &mut d,
        0,
        [0.0; 3],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        &[30],
    );
    assert_eq!(
        outcome,
        BlockOutcome {
            effect: true,
            deflected: false
        }
    );
    assert_eq!(d.amount, 70.0);
    assert_eq!(d.magnitude, 2.0);
    let original = d;
    assert!(
        block_impact(
            &mut d,
            0,
            [0.0; 3],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            &[140]
        )
        .deflected
    );
    assert_eq!(d, original);
    d.element = E::ICE.0;
    assert_eq!(
        block_impact(
            &mut d,
            0,
            [0.0; 3],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            &[140]
        ),
        BlockOutcome::default()
    );
}
