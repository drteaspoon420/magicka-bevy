use magicka_sim::arc_intersection::*;
fn q() -> ArcQuery {
    ArcQuery {
        origin: [0.0; 3],
        direction: [0.0, 0.0, 1.0],
        range: 4.0,
        angle: 0.2,
        height_difference: 2.0,
    }
}
#[test]
fn range_and_height_are_inclusive_and_return_surface_position() {
    assert_eq!(entity_arc([0.0, 2.0, 5.0], 1.0, q()), Some([0.0, 2.0, 4.0]));
    assert!(entity_arc([0.0, 2.01, 5.0], 1.0, q()).is_none());
    assert!(entity_arc([0.0, 0.0, 5.01], 1.0, q()).is_none());
}
#[test]
fn radius_expands_angle_but_angle_boundary_is_strict() {
    let mut a = q();
    a.angle = 0.0;
    assert!(entity_arc([0.0, 0.0, 2.0], 0.0, a).is_none());
    assert!(entity_arc([0.0, 0.0, 2.0], 0.5, a).is_some());
    assert!(entity_arc([1.0, 0.0, 3.0], 0.0, q()).is_none());
    assert!(entity_arc([1.0, 0.0, 3.0], 1.0, q()).is_some());
}
#[test]
fn direction_is_not_renormalized_and_vertical_component_is_discarded() {
    let mut a = q();
    a.direction = [0.0, 100.0, 1.0];
    assert!(entity_arc([0.0, 0.0, 2.0], 0.5, a).is_some());
    a.direction = [0.0, 0.0, 2.0];
    assert!(entity_arc([0.0, 0.0, 2.0], 0.5, a).is_none());
}
#[test]
fn ethereal_and_degenerate_positions_are_rejected() {
    assert!(character_arc([0.0, 0.0, 2.0], 0.5, true, q()).is_none());
    assert!(entity_arc([0.0; 3], 0.5, q()).is_none());
    assert!(entity_arc([0.0, 0.0, 0.1], 1.0, q()).is_none());
}
