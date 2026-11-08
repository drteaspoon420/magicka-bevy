use magicka_sim::water_freeze::WaterFreeze;
#[test]
fn arc_bounds_preserves_vertical_extent_and_reused_vector_quirk() {
    use magicka_sim::water_freeze::arc_bounds;
    let bounds = arc_bounds([10.0, 50.0, 3.0], [2.0, 80.0, 0.0], 0.0);
    assert_eq!(bounds.min, [10.0, -10000.0, 1.0]);
    assert_eq!(bounds.max, [12.0, 10000.0, 3.0]);
    let untranslated = arc_bounds([-2.0, 0.0, 0.0], [2.0, 0.0, 0.0], 0.0);
    assert_eq!(untranslated.min[2], 0.0);
}
#[test]
fn arc_vertex_selection_flattens_y_deduplicates_and_obeys_capacity() {
    use magicka_sim::water_freeze::arc_vertices;
    let vertices = [
        [1.0, 100.0, 0.0],
        [0.0, -100.0, 1.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0],
    ];
    let triangles = [[0, 1, 2], [1, 0, 3]];
    let query = |capacity| {
        arc_vertices(
            &vertices,
            &triangles,
            &[1, 0],
            [0.0, 5.0, 0.0],
            [1.0, 8.0, 0.0],
            std::f32::consts::FRAC_PI_2,
            capacity,
        )
    };
    assert_eq!(query(10), [1, 0]);
    assert_eq!(query(1), [1]);
    assert!(query(0).is_empty());
}
#[test]
fn collision_filters_preserve_order_and_select_threshold_crossing_triangles() {
    let mut water = WaterFreeze::new(6, true, false);
    water.vertices = vec![0.0, 0.49, 0.5, 0.0, 0.0, 0.0];
    let triangles = [[0, 1, 3], [1, 2, 3], [3, 4, 5]];
    let mut ice = vec![2, 1, 0];
    let mut liquid = ice.clone();
    water.filter_collision_candidates(&mut ice, &triangles, true);
    water.filter_collision_candidates(&mut liquid, &triangles, false);
    assert_eq!(ice, [1]);
    assert_eq!(liquid, [2, 0]);
    water.freeze_all(-1.0);
    water.filter_collision_candidates(&mut ice, &triangles, true);
    assert!(ice.is_empty());
}
#[test]
fn overlap_skips_first_new_vertex_but_removes_later_duplicates() {
    let mut water = WaterFreeze::new(4, true, false);
    water.freeze_arc_batches(&[vec![0, 1], vec![0, 1, 2]], 0.25);
    assert_eq!(water.vertices, [0.5, 0.25, 0.25, 0.0]);
}
#[test]
fn removed_duplicates_still_consume_query_capacity() {
    let mut water = WaterFreeze::new(2050, true, false);
    let initial: Vec<_> = (0..1024).collect();
    // Second batch consumes 1024 slots despite 1023 being removed. The
    // third batch cannot reuse those marked entries.
    water.freeze_arc_batches(&[initial.clone(), initial, vec![2048]], 1.0);
    assert_eq!(water.vertices[0], 2.0);
    assert_eq!(water.vertices[2047], 0.0);
    assert_eq!(water.vertices[2048], 0.0);
}
#[test]
fn spells_accumulate_without_clamp_but_updates_thaw_and_clamp() {
    let mut water = WaterFreeze::new(3, true, false);
    water.freeze_all(3.0);
    assert_eq!(water.vertices, [3.0; 3]);
    water.update(1.0, false);
    assert_eq!(water.vertices, [3.0; 3]);
    water.update(1.0, true);
    assert_eq!(water.vertices, [2.0; 3]);
    water.update(10.0, true);
    assert_eq!(water.vertices, [1.0; 3]);
    water.freeze_all(-4.0);
    water.update(0.0, true);
    assert_eq!(water.vertices, [0.0; 3]);
}
#[test]
fn auto_freeze_initialization_and_nonfreezable_gate_are_distinct() {
    let mut water = WaterFreeze::new(3, false, true);
    water.freeze_all(-3.0);
    assert_eq!(water.vertices, [1.0; 3]);
    water.vertices[0] = -2.0;
    water.update(1.0, true);
    assert_eq!(water.vertices[0], -1.0);
    water.vertices = vec![0.49, 0.5, 0.0];
    assert!(water.triangle_is_ice([0, 1, 2]));
    water.vertices[1] = 0.49;
    assert!(!water.triangle_is_ice([0, 1, 2]));
}
