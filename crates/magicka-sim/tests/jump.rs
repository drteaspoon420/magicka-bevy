use magicka_sim::{
    character_physics::{CharacterBody, GRAVITY},
    jump::*,
};

#[test]
fn ability_and_animation_prediction_preserve_distinct_distance_rules() {
    let target = JumpTarget {
        position: [4., 0., 0.],
        velocity: [2., 0., 0.],
    };
    assert_eq!(
        ability_delta([0.; 3], target, 0., 100.),
        Some([5.5, 0., 0.])
    );
    assert_eq!(
        animation_delta([0.; 3], [1., 0., 0.], Some(target), None, None),
        Some([27.5, 0., 0.])
    );
    assert_eq!(
        animation_delta([0.; 3], [1., 0., 0.], Some(target), Some(30.), Some(20.)),
        Some([20., 0., 0.])
    );
    assert!(ability_delta(
        target.position,
        JumpTarget {
            velocity: [0.; 3],
            ..target
        },
        0.,
        100.
    )
    .is_none());
    assert!(animation_delta(
        [0.; 3],
        [1., 0., 0.],
        Some(JumpTarget {
            position: [f32::NAN, 0., 0.],
            velocity: [0.; 3]
        }),
        None,
        None
    )
    .is_none());
}

#[test]
fn no_target_action_preserves_absolute_world_point_fallback() {
    assert_eq!(
        animation_delta([10., 0., 0.], [1., 0., 0.], None, Some(2.), Some(6.)),
        Some([-6., 0., 0.])
    );
}

#[test]
fn ballistic_velocity_hits_requested_height_and_rejects_invalid_speed() {
    let v = velocity([8., 2., 0.], 0.7, GRAVITY[1]).unwrap();
    let flight = 8. / v[0];
    let height = v[1] * flight + 0.5 * GRAVITY[1] * flight * flight;
    assert!((height - 2.).abs() < 1e-5);
    assert_eq!(v[2], 0.);
    assert!(velocity([4., 100., 0.], 0.7, GRAVITY[1]).is_none());
    assert!(velocity([0.; 3], 0.7, GRAVITY[1]).is_none());
}

#[test]
fn launch_keeps_body_ground_movement_repeat_and_owner_guards() {
    let mut body = CharacterBody::new(0.5, 1., 70., 5.);
    body.add_external_forces(1., None);
    launch(&mut body, false, [4., 0., 0.], 0.7, GRAVITY[1]);
    assert!(!body.is_jumping, "airborne body must reject AddJump");
    body.enable();
    body.allow_move = false;
    launch(&mut body, false, [4., 0., 0.], 0.7, GRAVITY[1]);
    assert!(!body.is_jumping);
    body.allow_move = true;
    body.owner.entangled = true;
    launch(&mut body, false, [4., 0., 0.], 0.7, GRAVITY[1]);
    assert!(!body.is_jumping);
    body.owner.entangled = false;
    launch(&mut body, true, [4., 0., 0.], 0.7, GRAVITY[1]);
    assert!(!body.is_jumping);
    launch(&mut body, false, [4., 0., 0.], 0.7, GRAVITY[1]);
    assert!(body.is_jumping && body.is_leaving_ground());
    launch(&mut body, false, [4., 0., 0.], 0.7, GRAVITY[1]);
    body.add_external_forces(0., None);
    assert_eq!(
        body.velocity,
        velocity([4., 0., 0.], 0.7, GRAVITY[1]).unwrap(),
        "repeated launch must not accumulate while leaving ground"
    );
}
