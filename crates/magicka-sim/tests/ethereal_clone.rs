use magicka_sim::ethereal_clone::*;

#[test]
fn native_parent_gate_is_exact_and_clone_template_is_fixed() {
    assert!(native_eligible("dungeons_human_adventurer"));
    for parent in [
        "wizard",
        "dungeons_human_adventurer_clone",
        "Dungeons_Human_Adventurer",
        "",
    ] {
        assert!(!native_eligible(parent));
    }
    assert_eq!(TEMPLATE, "dungeons_human_adventurer_clone");
}

#[test]
fn square_offset_preserves_native_double_height_before_navmesh_snap() {
    assert_eq!(position([4.0, 7.0, 9.0], 0.5, 0.5), [4.0, 14.0, 9.0]);
    assert_eq!(position([4.0, -7.0, 9.0], 0.0, 1.0), [-2.0, -14.0, 15.0]);
    assert_eq!(position([4.0, 0.0, 9.0], 0.25, 0.75), [1.0, 0.0, 12.0]);
}
