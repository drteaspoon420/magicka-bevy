use magicka_sim::{box_box::overlap, box_intersection::OrientedBox};

fn cube(center: [f32; 3], sides: [f32; 3]) -> OrientedBox {
    OrientedBox {
        center,
        side_lengths: sides,
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    }
}

#[test]
fn clearance_tolerance_and_normal_orientation() {
    let a = cube([0.0; 3], [2.0; 3]);
    let b = cube([2.02, 0.0, 0.0], [2.0; 3]);
    assert!(overlap(a, b, 0.01).is_none());
    let hit = overlap(a, b, 0.05).unwrap();
    assert_eq!(hit.normal, [-1.0, 0.0, 0.0]);
    assert!((hit.depth + 0.02).abs() < 1e-5);
    assert_eq!(overlap(b, a, 0.05).unwrap().normal, [1.0, 0.0, 0.0]);
}

#[test]
fn nested_boxes_use_exit_distance() {
    let hit = overlap(
        cube([0.0; 3], [2.0; 3]),
        cube([0.25, 0.0, 0.0], [1.0; 3]),
        0.0,
    )
    .unwrap();
    assert_eq!(hit.depth, 1.25);
    assert_eq!(hit.normal, [-1.0, 0.0, 0.0]);
}

#[test]
fn skew_thin_boxes_require_cross_axes() {
    let q = std::f32::consts::FRAC_1_SQRT_2;
    let a = OrientedBox {
        center: [0.0; 3],
        axes: [[q, q, 0.0], [-q, q, 0.0], [0.0, 0.0, 1.0]],
        side_lengths: [4.0, 0.1, 0.1],
    };
    let b = OrientedBox {
        center: [0.0, 0.0, 0.5],
        axes: [[q, 0.0, q], [0.0, 1.0, 0.0], [-q, 0.0, q]],
        side_lengths: [4.0, 0.1, 0.1],
    };
    // Their face-axis projections overlap, but their skew centre lines do
    // not touch. A face-only or bounding-sphere test invents a collision.
    assert!(overlap(a, b, 0.0).is_none());
    assert!(overlap(
        a,
        OrientedBox {
            center: [0.0; 3],
            ..b
        },
        0.0
    )
    .is_some());
}
