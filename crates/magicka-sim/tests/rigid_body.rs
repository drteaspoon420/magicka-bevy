use magicka_sim::rigid_body::{
    BodyTransform, GravityAxis, Matrix, RigidBody, TransformRate, IDENTITY,
};

fn diagonal(x: f32, y: f32, z: f32) -> Matrix {
    [
        [x, 0., 0., 0.],
        [0., y, 0., 0.],
        [0., 0., z, 0.],
        [0., 0., 0., 1.],
    ]
}
fn body() -> RigidBody {
    RigidBody {
        current: BodyTransform::default(),
        old: BodyTransform::default(),
        rate: TransformRate::default(),
        auxiliary: TransformRate::default(),
        inverse_mass: 0.25,
        mass: 4.,
        apply_gravity: true,
        force: [0.; 3],
        torque: [0.; 3],
        body_inertia: IDENTITY,
        body_inverse_inertia: IDENTITY,
        world_inertia: IDENTITY,
        world_inverse_inertia: IDENTITY,
        active: true,
        immovable: false,
    }
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}

#[test]
fn force_and_world_torque_integrate_before_contact_angular_drag() {
    let mut b = body();
    b.rate.velocity = [1., 2., 3.];
    b.rate.angular_velocity = [1., 2., 3.];
    b.force = [8., -16., 4.];
    b.torque = [2., 4., 6.];
    b.world_inverse_inertia = diagonal(0.5, 0.25, 2.);
    b.update_velocity(0.5, true);
    assert_eq!(b.rate.velocity, [2., 0., 3.5]);
    for (a, e) in b
        .rate
        .angular_velocity
        .into_iter()
        .zip([1.5 * 0.99, 2.5 * 0.99, 9. * 0.99])
    {
        near(a, e);
    }
    assert_eq!(b.force, [8., -16., 4.]); // Integration does not clear external forces.
}

#[test]
fn activity_gates_main_updates_but_auxiliary_update_clears_corrections() {
    for immovable in [false, true] {
        let mut b = body();
        b.active = immovable;
        b.immovable = immovable;
        b.rate.velocity = [1.; 3];
        b.force = [10.; 3];
        b.auxiliary.velocity = [2.; 3];
        b.auxiliary.angular_velocity = [3.; 3];
        b.update_velocity(1., true);
        assert_eq!(b.rate.velocity, [1.; 3]);
        assert_eq!(b.update_position(1.), None);
        assert_eq!(b.auxiliary.velocity, [2.; 3]);
        assert_eq!(b.update_position_with_aux(1., GravityAxis::Y), None);
        assert_eq!(b.auxiliary, TransformRate::default());
        assert_eq!(b.current, BodyTransform::default());
    }
}

#[test]
fn auxiliary_translation_retains_gravity_component_and_is_consumed_once() {
    for (axis, expected) in [
        (GravityAxis::None, [10., 20., 30.]),
        (GravityAxis::X, [10., 2., 3.]),
        (GravityAxis::Y, [1., 20., 3.]),
        (GravityAxis::Z, [1., 2., 30.]),
    ] {
        let mut b = body();
        b.auxiliary.velocity = [10., 20., 30.];
        b.update_position_with_aux(1., axis);
        assert_eq!(b.current.position, expected);
        assert_eq!(b.auxiliary, TransformRate::default());
        assert_eq!(b.rate, TransformRate::default());
        b.update_position_with_aux(1., axis);
        assert_eq!(b.current.position, expected);
    }
}

#[test]
fn auxiliary_rotation_updates_tensor_without_retaining_auxiliary_momentum() {
    let mut b = body();
    b.body_inertia = diagonal(2., 4., 8.);
    b.body_inverse_inertia = diagonal(0.5, 0.25, 0.125);
    b.world_inertia = b.body_inertia;
    b.world_inverse_inertia = b.body_inverse_inertia;
    b.auxiliary.angular_velocity = [0., 0., std::f32::consts::FRAC_PI_2];
    b.update_position_with_aux(1., GravityAxis::None);
    near(b.current.orientation[0][1], 1.);
    near(b.current.orientation[1][0], -1.);
    near(b.world_inertia[0][0], 4.);
    near(b.world_inertia[1][1], 2.);
    near(b.world_inverse_inertia[0][0], 0.25);
    near(b.world_inverse_inertia[1][1], 0.5);
    assert_eq!(b.rate.angular_velocity, [0.; 3]);
}

#[test]
fn main_angular_momentum_survives_nonprincipal_rotation() {
    let mut b = body();
    b.body_inertia = diagonal(2., 4., 8.);
    b.body_inverse_inertia = diagonal(0.5, 0.25, 0.125);
    b.world_inertia = b.body_inertia;
    b.world_inverse_inertia = b.body_inverse_inertia;
    b.rate.angular_velocity = [1., 2., 3.];
    b.update_position(0.25);
    let w = b.rate.angular_velocity;
    for (i, expected) in [2., 8., 24.].into_iter().enumerate() {
        near(
            w[0] * b.world_inertia[0][i]
                + w[1] * b.world_inertia[1][i]
                + w[2] * b.world_inertia[2][i],
            expected,
        );
    }
}

#[test]
fn transform_publication_preserves_old_state_and_postmultiplies_orientation() {
    let mut b = body();
    b.old.position = [-1., -2., -3.];
    b.current.orientation = diagonal(2., 3., 4.); // No implicit orthonormalization.
    b.rate.velocity = [2., 4., 6.];
    b.rate.angular_velocity = [0., 0., std::f32::consts::FRAC_PI_2];
    let (old, current) = b.update_position(1.).unwrap();
    assert_eq!(old.position, [-1., -2., -3.]);
    assert_eq!(b.old, old);
    assert_eq!(current.position, [2., 4., 6.]);
    near(current.orientation[0][1], 2.);
    near(current.orientation[1][0], -3.);
    near(current.orientation[2][2], 4.);
}
