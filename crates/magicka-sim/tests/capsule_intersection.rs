use magicka_sim::{capsule_intersection::segment_fraction, spray_pulse::Segment};
fn fraction(origin: [f32; 3], delta: [f32; 3]) -> Option<f32> {
    segment_fraction(
        Segment { origin, delta },
        [0.0; 3],
        [0.0, 1.0, 0.0],
        1.0,
        0.5,
    )
}
#[test]
fn side_hit_returns_near_surface_and_inside_returns_zero() {
    assert_eq!(fraction([2.0, 0.5, 0.0], [-4.0, 0.0, 0.0]), Some(0.375));
    assert_eq!(fraction([0.0, 0.5, 0.0], [4.0, 0.0, 0.0]), Some(0.0));
}
#[test]
fn cap_hit_and_parallel_outside_miss() {
    assert_eq!(fraction([0.0, 3.0, 0.0], [0.0, -3.0, 0.0]), Some(0.5));
    assert_eq!(fraction([1.0, 3.0, 0.0], [0.0, -3.0, 0.0]), None);
}
#[test]
fn finite_segment_does_not_reach_cylinder() {
    assert_eq!(fraction([2.0, 0.5, 0.0], [-0.5, 0.0, 0.0]), None);
}
#[test]
fn short_segment_retains_native_sphere_output_even_when_boolean_is_false() {
    assert_eq!(fraction([0.0, 10.0, 0.0], [0.0, 0.1, 0.0]), Some(0.0));
}
