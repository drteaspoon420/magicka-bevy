use magicka_sim::{
    box_intersection::OrientedBox,
    hollow_sphere_box::{detect, PrimitiveMotion, ShellBoxContact},
    shield_intersection::HollowSphere,
};
use std::f32::consts::{FRAC_PI_4, PI};

fn sphere() -> PrimitiveMotion<HollowSphere> {
    let geometry = HollowSphere {
        center: [0.0; 3],
        radius: 5.0,
        forward: [1.0, 0.0, 0.0],
        max_angle: PI,
    };
    PrimitiveMotion {
        old: geometry,
        new: geometry,
        owner_old_position: [0.0; 3],
    }
}
fn box_motion(old: f32, new: f32) -> PrimitiveMotion<OrientedBox> {
    let geometry = OrientedBox {
        center: [old, 0.0, 0.0],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        // Half diagonal = 1.5 exactly.
        side_lengths: [2.0, 2.0, 1.0],
    };
    PrimitiveMotion {
        old: geometry,
        new: OrientedBox {
            center: [new, 0.0, 0.0],
            ..geometry
        },
        owner_old_position: [old, 0.0, 0.0],
    }
}
fn contact(old: f32, new: f32) -> Option<ShellBoxContact> {
    detect(sphere(), box_motion(old, new), 0.0, || {
        panic!("unexpected RNG")
    })
}

#[test]
fn inside_and_outside_contacts_have_opposite_normals_and_old_frame_offsets() {
    let inside = contact(4.0, 4.5).unwrap();
    assert_eq!(inside.normal, [1.0, 0.0, 0.0]);
    assert_eq!(inside.penetration, 0.5);
    assert_eq!(inside.point, [2.75, 0.0, 0.0]);
    assert_eq!(inside.sphere_offset, inside.point);
    assert_eq!(inside.box_offset, [-1.25, 0.0, 0.0]);
    let outside = contact(6.0, 5.5).unwrap();
    assert_eq!(outside.normal, [-1.0, 0.0, 0.0]);
    assert_eq!(outside.penetration, 0.5);
    assert_eq!(outside.point, [4.75, 0.0, 0.0]);
}

#[test]
fn hollow_interior_and_far_exterior_do_not_contact_and_equal_thresholds_are_strict() {
    for (old, new) in [(2.0, 3.0), (8.0, 7.0), (5.0, 5.0), (4.0, 3.5), (6.0, 6.5)] {
        assert!(contact(old, new).is_none(), "{old} -> {new}");
    }
    // Tolerance changes both the side classification and new-center limit.
    assert!(detect(sphere(), box_motion(5.0, 5.0), 0.05, || panic!()).is_some());
}

#[test]
fn disc_uses_new_direction_and_accepts_the_angular_boundary() {
    let mut disc = sphere();
    disc.new.max_angle = FRAC_PI_4;
    let mut motion = box_motion(-6.0, 5.0);
    assert!(detect(disc, motion, 0.0, || panic!()).is_some());
    motion.new.center = [-5.0, 0.0, 0.0];
    assert!(detect(disc, motion, 0.0, || panic!()).is_none());
    motion.new.center = [3.0, 0.0, 3.0];
    assert!(detect(disc, motion, 0.0, || panic!()).is_some());
}

#[test]
fn old_and_new_box_sizes_and_owner_positions_remain_distinct() {
    let mut s = sphere();
    s.owner_old_position = [10.0, 20.0, 30.0];
    let mut b = box_motion(4.0, 4.5);
    b.old.side_lengths = [4.0, 4.0, 2.0]; // Old radius = 3.
    b.owner_old_position = [1.0, 2.0, 3.0];
    let c = detect(s, b, 0.0, || panic!()).unwrap();
    assert_eq!(c.penetration, 0.5); // Uses NEW radius = 1.5.
    assert_eq!(c.point, [1.25, 0.0, 0.0]); // Uses OLD radius = 3.
    assert_eq!(c.sphere_offset, [-8.75, -20.0, -30.0]);
    assert_eq!(c.box_offset, [0.25, -2.0, -3.0]);
}

#[test]
fn moving_sphere_uses_each_frame_center_but_only_the_new_radius() {
    let mut s = sphere();
    s.old.center = [30.0, 0.0, 0.0];
    s.old.radius = 100.0;
    s.new.center = [10.0, 0.0, 0.0];
    s.owner_old_position = s.old.center;
    let c = detect(s, box_motion(34.0, 14.5), 0.0, || panic!()).unwrap();
    assert_eq!(c.penetration, 0.5);
    assert_eq!(c.normal, [1.0, 0.0, 0.0]);
    assert_eq!(c.point, [32.75, 0.0, 0.0]);
    assert_eq!(c.sphere_offset, [2.75, 0.0, 0.0]);
}

#[test]
fn degenerate_old_center_draws_once_and_keeps_negative_penetration() {
    let mut draws = 0;
    let c = detect(sphere(), box_motion(0.0, 4.0), 0.0, || {
        draws += 1;
        0
    })
    .unwrap();
    assert_eq!(draws, 1);
    assert_eq!(c.penetration, -3.5);
    assert_eq!(c.normal, [0.0, 0.0, -1.0]);
    assert_eq!(c.point, [0.0, 0.0, 3.25]);
}

#[test]
fn coincident_new_center_nan_angle_does_not_reject_the_contact() {
    let mut s = sphere();
    s.new.max_angle = 0.0;
    assert!(detect(s, box_motion(6.0, 0.0), 0.0, || panic!()).is_some());
}
