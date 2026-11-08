use magicka_sim::{
    arc_intersection::ArcQuery, shield_intersection::HollowSphere, spray_pulse::Segment,
};
fn sphere() -> HollowSphere {
    HollowSphere {
        center: [0.0; 3],
        radius: 1.0,
        forward: [0.0, 0.0, 1.0],
        max_angle: std::f32::consts::PI,
    }
}
#[test]
fn closest_root_and_inside_exit() {
    let s = sphere();
    assert_eq!(
        s.segment_fraction(Segment {
            origin: [0.0, 0.0, -2.0],
            delta: [0.0, 0.0, 4.0]
        }),
        Some(0.25)
    );
    assert_eq!(
        s.segment_fraction(Segment {
            origin: [0.0; 3],
            delta: [0.0, 0.0, 2.0]
        }),
        Some(0.5)
    );
}
#[test]
fn angular_opening_can_select_far_root() {
    let mut s = sphere();
    s.max_angle = 0.1;
    assert_eq!(
        s.segment_fraction(Segment {
            origin: [0.0, 0.0, -2.0],
            delta: [0.0, 0.0, 4.0]
        }),
        Some(0.75)
    );
    s.forward = [1.0, 0.0, 0.0];
    assert!(s
        .segment_fraction(Segment {
            origin: [0.0, 0.0, -2.0],
            delta: [0.0, 0.0, 4.0]
        })
        .is_none());
}
#[test]
fn endpoints_tangents_and_zero_segments_are_excluded() {
    let s = sphere();
    for segment in [
        Segment {
            origin: [0.0, 0.0, -2.0],
            delta: [0.0, 0.0, 1.0],
        },
        Segment {
            origin: [0.0, 0.0, -1.0],
            delta: [0.0, 0.0, 2.0],
        },
        Segment {
            origin: [1.0, 0.0, -2.0],
            delta: [0.0, 0.0, 4.0],
        },
        Segment {
            origin: [0.0; 3],
            delta: [0.0; 3],
        },
    ] {
        assert!(s.segment_fraction(segment).is_none());
    }
}
#[test]
fn shield_arc_ignores_angle_and_height_and_keeps_vertical_direction() {
    let q = ArcQuery {
        origin: [0.0, -2.0, 0.0],
        direction: [0.0, 1.0, 0.0],
        range: 4.0,
        angle: -1.0,
        height_difference: -1.0,
    };
    assert_eq!(sphere().arc_intersect(q), Some([0.0, -1.0, 0.0]));
}
