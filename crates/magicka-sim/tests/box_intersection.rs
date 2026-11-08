use magicka_sim::{
    arc_intersection::ArcQuery, box_intersection::*, shield_intersection::ShieldGeometry,
    spray_pulse::Segment,
};
fn cube() -> OrientedBox {
    OrientedBox {
        center: [0.0; 3],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        side_lengths: [2.0; 3],
    }
}
#[test]
fn entry_exit_and_endpoint_normals() {
    let b = cube();
    let hit = b
        .segment_intersect(Segment {
            origin: [-2.0, 0.0, 0.0],
            delta: [1.0, 0.0, 0.0],
        })
        .unwrap();
    assert_eq!(hit.fraction, 1.0);
    assert_eq!(hit.normal, [-1.0, 0.0, 0.0]);
    let hit = b
        .segment_intersect(Segment {
            origin: [0.0; 3],
            delta: [2.0, 0.0, 0.0],
        })
        .unwrap();
    assert_eq!(hit.fraction, 0.5);
    assert_eq!(hit.normal, [-1.0, 0.0, 0.0]);
    assert_eq!(
        b.segment_intersect(Segment {
            origin: [-1.0, 0.0, 0.0],
            delta: [2.0, 0.0, 0.0]
        })
        .unwrap()
        .fraction,
        0.0
    );
}
#[test]
fn parallel_outside_and_stationary_inside_reject() {
    assert!(cube()
        .segment_intersect(Segment {
            origin: [2.0, 0.0, 0.0],
            delta: [0.0, 2.0, 0.0]
        })
        .is_none());
    assert!(cube()
        .segment_intersect(Segment {
            origin: [0.0; 3],
            delta: [0.0; 3]
        })
        .is_none());
}
#[test]
fn oriented_wall_uses_world_axes_and_arc_adapter() {
    let b = OrientedBox {
        center: [0.0; 3],
        axes: [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]],
        side_lengths: [0.4, 6.0, 12.0],
    };
    let q = ArcQuery {
        origin: [0.0, 0.0, -2.0],
        direction: [0.0, 0.0, 1.0],
        range: 4.0,
        angle: 0.0,
        height_difference: 0.0,
    };
    let p = ShieldGeometry::Wall(b).arc_intersect(q).unwrap();
    assert!((p[2] + 0.2).abs() < 1e-6);
}
