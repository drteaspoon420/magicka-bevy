use magicka_sim::{
    box_intersection::OrientedBox,
    capsule_box::{detect, segment_distance},
    capsule_pair::CapsuleMotion,
    hollow_sphere_box::PrimitiveMotion,
    spray_pulse::Segment,
};
fn wall() -> OrientedBox {
    OrientedBox {
        center: [0.0; 3],
        axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        side_lengths: [0.4, 6., 12.],
    }
}
fn capsule(x: f32, new_x: f32) -> CapsuleMotion {
    CapsuleMotion {
        old_axis: Segment {
            origin: [x, -0.25, 0.],
            delta: [0., 0.5, 0.],
        },
        new_axis: Segment {
            origin: [new_x, -0.25, 0.],
            delta: [0., 0.5, 0.],
        },
        old_radius: 0.75,
        new_radius: 0.75,
        old_body_position: [x, 0., 0.],
    }
}
#[test]
fn interpreted_original_distance_fixtures() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../evidence/shield-wall/distance-cases.tsv");
    let Ok(data) = std::fs::read_to_string(path) else {
        eprintln!("private static-IL fixture absent");
        return;
    };
    let mut max_error = 0.0f32;
    let mut count = 0;
    for (i, row) in data.lines().enumerate() {
        let v: Vec<f32> = row.split('\t').map(|v| v.parse().unwrap()).collect();
        let s = Segment {
            origin: v[0..3].try_into().unwrap(),
            delta: v[3..6].try_into().unwrap(),
        };
        let b = OrientedBox {
            side_lengths: v[6..9].try_into().unwrap(),
            center: v[9..12].try_into().unwrap(),
            axes: [
                v[12..15].try_into().unwrap(),
                v[15..18].try_into().unwrap(),
                v[18..21].try_into().unwrap(),
            ],
        };
        let (sq, t, q) = segment_distance(s, b);
        for (actual, expected) in [sq, t, q[0], q[1], q[2]].into_iter().zip(&v[21..26]) {
            let error = (actual - expected).abs();
            max_error = max_error.max(error);
            assert!(
                error < 0.00002,
                "case {i}: got {actual} expected {expected}; {v:?}"
            );
        }
        count += 1;
    }
    assert_eq!(count, 1024);
    eprintln!("Original IL distance: {count} cases; max absolute error {max_error}");
}
#[test]
fn wall_detects_both_faces_and_uses_old_geometry_when_prediction_touches() {
    let b = wall();
    let motion = PrimitiveMotion {
        old: b,
        new: b,
        owner_old_position: b.center,
    };
    for sign in [-1., 1.] {
        let c = capsule(sign * 1.0, sign * 0.8);
        let hit = detect(c, motion, 0.1, || panic!("unexpected random normal")).unwrap();
        assert_eq!(hit.normal, [sign, 0., 0.]);
        assert!((hit.penetration - (-0.05)).abs() < 1e-6);
        assert!((hit.point[0] - sign * 0.2).abs() < 1e-6);
        assert_eq!(
            hit.offset0,
            [
                hit.point[0] - c.old_body_position[0],
                hit.point[1],
                hit.point[2]
            ]
        );
    }
    assert!(detect(capsule(1.1, 1.1), motion, 0.1, || 0).is_none());
}
#[test]
fn intersection_retains_native_normalized_corner_parameters() {
    let (sq, t, q) = segment_distance(
        Segment {
            origin: [-1., 0., 0.],
            delta: [2., 0., 0.],
        },
        wall(),
    );
    assert_eq!(sq, 0.0);
    assert!((t - 0.4).abs() < 1e-6);
    assert!(q[0].abs() < 1e-6);
    assert_eq!(q[1..], [0.5, 0.5]);
}
