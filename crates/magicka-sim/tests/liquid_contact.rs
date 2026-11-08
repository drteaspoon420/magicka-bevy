use magicka_sim::liquid_contact::*;
#[test]
fn drowning_requires_live_contact_and_inclusive_height_threshold() {
    assert!(!should_drown(100.0, true, 0.99, -1.0));
    assert!(should_drown(100.0, true, 1.0, -1.0));
    assert!(!should_drown(0.0, true, 2.0, -1.0));
    assert!(!should_drown(100.0, false, 2.0, -1.0));
}
#[test]
fn depth_uses_literal_contact_offset_and_damping_has_native_floor() {
    assert_eq!(contact_depth(3.0, -1.0, 0.5), 1.5);
    assert_eq!(contact_depth(0.0, -1.0, 0.5), 1.5);
    assert_eq!(velocity_multiplier(0.0, 1.0), 1.0);
    assert_eq!(velocity_multiplier(1.0, 1.0), 0.1);
    assert_eq!(velocity_multiplier(10.0, 0.0), 1.0);
    assert!((velocity_multiplier(1.0, 0.5) - 0.1f32.sqrt()).abs() < 1e-6);
}
