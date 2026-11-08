use magicka_sim::{
    capsule_pair::CapsuleMotion, hollow_sphere_box::PrimitiveMotion, hollow_sphere_capsule::detect,
    shield_intersection::HollowSphere, spray_pulse::Segment,
};
use std::f32::consts::{FRAC_PI_4, PI};

fn sphere() -> PrimitiveMotion<HollowSphere> {
    let s = HollowSphere {
        center: [0.0; 3],
        radius: 5.0,
        forward: [1.0, 0.0, 0.0],
        max_angle: PI,
    };
    PrimitiveMotion {
        old: s,
        new: s,
        owner_old_position: [0.0; 3],
    }
}
fn capsule(x: f32) -> CapsuleMotion {
    let axis = Segment {
        origin: [x, -2.0, 0.0],
        delta: [0.0, 4.0, 0.0],
    };
    CapsuleMotion {
        old_axis: axis,
        new_axis: axis,
        old_radius: 0.5,
        new_radius: 0.5,
        old_body_position: [x, 0.0, 0.0],
    }
}

#[test]
fn inside_outside_contacts_include_thickness_and_use_old_axis_point() {
    let inside = detect(sphere(), capsule(4.5), 0.0, || panic!()).unwrap();
    assert_eq!(inside.normal, [1.0, 0.0, 0.0]);
    assert_eq!(inside.penetration, 0.5);
    assert_eq!(inside.point, [4.75, 0.0, 0.0]);
    assert_eq!(inside.offset0, inside.point);
    assert_eq!(inside.offset1, [0.25, 0.0, 0.0]);
    let outside = detect(sphere(), capsule(5.5), 0.0, || panic!()).unwrap();
    assert_eq!(outside.normal, [-1.0, 0.0, 0.0]);
    assert_eq!(outside.penetration, 0.5);
    assert_eq!(outside.point, [5.25, 0.0, 0.0]);
}

#[test]
fn translation_is_used_for_angle_but_not_new_axis_distance() {
    let mut c = capsule(6.5);
    c.new_axis.origin = [4.5, -2.0, 0.0];
    assert!(detect(sphere(), c, 0.0, || panic!()).is_none());
    let mut c = capsule(4.5);
    c.new_axis.origin = [6.5, -2.0, 0.0];
    assert!(detect(sphere(), c, 0.0, || panic!()).is_some());
}

#[test]
fn new_axis_orientation_is_used_at_old_origin() {
    let mut c = capsule(6.0);
    c.old_axis.origin = [6.0, 0.0, 0.0];
    c.new_axis = Segment {
        origin: [100.0, 0.0, 0.0],
        delta: [-2.0, 0.0, 0.0],
    };
    let contact = detect(sphere(), c, 0.0, || panic!()).unwrap();
    assert_eq!(contact.penetration, 0.0);
    assert_eq!(contact.point, [5.5, 0.0, 0.0]);
}

#[test]
fn strict_radial_thresholds_and_tolerance() {
    for x in [2.0, 4.0, 5.0, 6.0, 7.0] {
        assert!(
            detect(sphere(), capsule(x), 0.0, || panic!()).is_none(),
            "x={x}"
        );
    }
    assert!(detect(sphere(), capsule(5.0), 0.05, || panic!()).is_some());
}

#[test]
fn disc_filter_reads_new_capsule_origin_rather_than_closest_axis_point() {
    let mut s = sphere();
    s.new.max_angle = FRAC_PI_4;
    let mut c = capsule(4.5);
    c.new_axis.origin = [-4.5, 0.0, 0.0];
    assert!(detect(s, c, 0.0, || panic!()).is_none());
    c.new_axis.origin = [3.0, 0.0, 3.0];
    assert!(detect(s, c, 0.0, || panic!()).is_some());
}

#[test]
fn old_radius_and_owner_offsets_remain_distinct_from_new_geometry() {
    let mut s = sphere();
    s.old.radius = 100.0; // Detector thresholds use the new sphere radius.
    s.owner_old_position = [10.0, 20.0, 30.0];
    let mut c = capsule(4.5);
    c.old_radius = 0.25;
    c.new_radius = 0.75;
    c.old_body_position = [1.0, 2.0, 3.0];
    let contact = detect(s, c, 0.0, || panic!()).unwrap();
    assert_eq!(contact.penetration, 0.75);
    assert_eq!(contact.point, [4.375, 0.0, 0.0]);
    assert_eq!(contact.offset0, [-5.625, -20.0, -30.0]);
    assert_eq!(contact.offset1, [3.375, -2.0, -3.0]);
}

#[test]
fn moving_sphere_can_trigger_degenerate_rng_with_negative_penetration() {
    let mut s = sphere();
    s.new.center = [5.5, 0.0, 0.0];
    let mut draws = 0;
    let contact = detect(s, capsule(0.0), 0.0, || {
        draws += 1;
        0
    })
    .unwrap();
    assert_eq!(draws, 1);
    assert_eq!(contact.penetration, -4.0);
    assert_eq!(contact.normal, [0.0, 0.0, 1.0]);
    assert_eq!(contact.point, [0.0, 0.0, 2.5]);
}

#[test]
fn coincident_new_origin_nan_angle_is_not_rejected() {
    let mut s = sphere();
    s.new.max_angle = 0.0;
    let mut c = capsule(5.5);
    c.new_axis.origin = [0.0; 3];
    assert!(detect(s, c, 0.0, || panic!()).is_some());
}
