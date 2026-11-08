//! Native missile render basis follows velocity, including gravity pitch.
use magicka_runtime::missiles::MissileBody;
fn body(velocity: [f32; 3]) -> MissileBody {
    MissileBody {
        id: 1,
        owner: 1,
        position: [0.; 3],
        old_position: [0.; 3],
        velocity,
        radius: 0.075,
        model: None,
        item_model: None,
        no_gravity: false,
        dead: false,
        homing: None,
    }
}
fn near(a: [f32; 3], b: [f32; 3]) {
    for i in 0..3 {
        assert!((a[i] - b[i]).abs() < 1e-6, "{a:?} != {b:?}");
    }
}
#[test]
fn missile_forward_tracks_all_headings_and_gravity_pitch() {
    for velocity in [
        [0., 0., -10.],
        [10., 0., 0.],
        [0., 0., 10.],
        [-10., 0., 0.],
        [3., 4., 0.],
        [3., -4., 0.],
    ] {
        let orientation = body(velocity).render_orientation();
        let speed = velocity.iter().map(|v| v * v).sum::<f32>().sqrt();
        near(orientation.forward(), velocity.map(|v| v / speed));
        for axis in [orientation.right, orientation.up, orientation.backward] {
            assert!((axis.iter().map(|v| v * v).sum::<f32>() - 1.).abs() < 1e-6);
        }
        assert!(orientation.up[1] > 0.);
        assert_eq!(orientation.right[1], 0.);
        let q = orientation.to_quaternion();
        // Quaternion rotating model -Z must equal the native Forward column.
        let [x, y, z, w] = q;
        near(
            [
                -2. * (x * z + w * y),
                -2. * (y * z - w * x),
                -(1. - 2. * (x * x + y * y)),
            ],
            orientation.forward(),
        );
    }
}
#[test]
fn stationary_and_vertical_missiles_keep_finite_fallback() {
    for v in [[0., 0., 0.], [0., -10., 0.]] {
        assert_eq!(
            body(v).render_orientation(),
            magicka_sim::character_physics::Orientation::IDENTITY
        );
    }
}
