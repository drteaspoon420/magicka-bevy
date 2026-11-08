use magicka_sim::entity_query::*;
fn entity(x: f32, y: f32, z: f32, radius: f32) -> QueryEntity {
    QueryEntity {
        position: [x, y, z],
        radius,
        dead: false,
        shield: false,
        damageable: true,
    }
}
#[test]
fn grid_traversal_orders_before_insertion_and_deduplicates_wrapped_cells() {
    let entities = [
        entity(9.0, 0.0, 0.0, 1.0),
        entity(-1.0, 0.0, 0.0, 1.0),
        entity(0.0, 0.0, 0.0, 130.0),
    ];
    let grid = EntityQueryGrid::build(&entities).unwrap();
    let found = grid
        .query(&entities, [0.0; 3], 10.0, false, false, |_, _| panic!())
        .unwrap();
    // Large-radius entity occupies the first visited cell before the entity at
    // negative X; neither handle nor insertion sorting produces this order.
    assert_eq!(found, [2, 1, 0]);
    let all = grid
        .query(&entities, [0.0; 3], 1000.0, false, false, |_, _| panic!())
        .unwrap();
    assert_eq!(all.len(), 3);
    for i in 0..3 {
        assert_eq!(all.iter().filter(|&&j| j == i).count(), 1);
    }
}
#[test]
fn spherical_range_includes_radius_and_exact_boundary_dead_and_y_filters() {
    let mut entities = [
        entity(3.0, 0.0, 0.0, 1.0),
        entity(0.0, 10.0, 0.0, 1.0),
        entity(0.0, 0.0, 0.0, 1.0),
    ];
    entities[2].dead = true;
    let grid = EntityQueryGrid::build(&entities).unwrap();
    assert_eq!(
        grid.query(&entities, [0.0; 3], 2.0, false, false, |_, _| panic!())
            .unwrap(),
        [0]
    );
    let flat = grid
        .query(
            &entities,
            [0.0, 20.0, 0.0],
            2.0,
            false,
            true,
            |_, _| panic!(),
        )
        .unwrap();
    assert!(flat.contains(&0) && flat.contains(&1) && !flat.contains(&2));
}
#[test]
fn shield_removal_before_outer_index_preserves_later_target_processing() {
    let mut entities = [entity(0.0, 0.0, 0.0, 0.0); 4];
    entities[1].shield = true;
    entities[0].position[0] = 1.0;
    entities[2].position[0] = 2.0;
    entities[3].position[0] = 3.0;
    let grid = EntityQueryGrid::build(&entities).unwrap();
    let mut calls = vec![];
    let found = grid
        .query(&entities, [0.0; 3], 4.0, true, false, |shield, seg| {
            calls.push((shield, seg.delta[0]));
            if seg.delta[0] < 3.0 {
                (true, [0.5, 0.0, 0.0])
            } else {
                (false, [0.0; 3])
            }
        })
        .unwrap();
    assert_eq!(found, [1, 3]);
    assert_eq!(calls, [(1, 1.0), (1, 2.0), (1, 3.0)]);
}
#[test]
fn protected_filter_always_evaluates_segment_and_uses_miss_output_for_non_damageable() {
    let mut entities = [entity(1.0, 0.0, 0.0, 0.0), entity(2.0, 0.0, 0.0, 0.0)];
    entities[0].shield = true;
    entities[1].damageable = false;
    let grid = EntityQueryGrid::build(&entities).unwrap();
    let mut calls = 0;
    let found = grid
        .query(&entities, [1.0, 0.0, 0.0], 3.0, true, false, |_, _| {
            calls += 1;
            (false, [0.0; 3])
        })
        .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(found, [0]);
    // An intersection at the query origin is too close to remove the target.
    assert_eq!(
        grid.query(&entities, [1.0, 0.0, 0.0], 3.0, true, false, |_, _| (
            true,
            [1.0, 0.0, 0.0]
        ))
        .unwrap(),
        [0, 1]
    );
}
#[test]
fn invalid_geometry_is_reported_instead_of_invented_cells() {
    assert!(matches!(
        EntityQueryGrid::build(&[entity(f32::NAN, 0.0, 0.0, 1.0)]),
        Err(QueryError::InvalidGeometry)
    ));
    assert!(matches!(
        EntityQueryGrid::build(&[entity(f32::MAX, 0.0, 0.0, 1.0)]),
        Err(QueryError::CellOverflow)
    ));
    let entities = [entity(0.0, 0.0, 0.0, 1.0)];
    let grid = EntityQueryGrid::build(&entities).unwrap();
    assert_eq!(
        grid.query(&entities, [0.0; 3], -1.0, false, false, |_, _| panic!()),
        Err(QueryError::InvalidGeometry)
    );
}
