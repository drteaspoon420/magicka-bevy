use magicka_content::level::TriMesh;
use magicka_runtime::water_collision::WaterCollision;
use magicka_sim::mesh_octree::Bounds;

#[test]
fn mixed_liquids_use_greatest_depth_but_last_contact_material_for_drowning() {
    use magicka_runtime::level_collision::{SceneCollision, StaticLevelCollision};
    use magicka_sim::character_physics::{BodyEvent, CharacterBody, PhysicsWorld};
    for lava_last in [false, true] {
        let mut liquids = Vec::new();
        for lava in if lava_last {
            [false, true]
        } else {
            [true, false]
        } {
            let y = if lava { 0.3 } else { 1.2 };
            let geometry = TriMesh {
                vertices: vec![[-10.0, y, -10.0], [10.0, y, -10.0], [0.0, y, 10.0]],
                triangles: vec![[0, 2, 1]],
            };
            let mut liquid = WaterCollision::from_geometry(&geometry, true, false).unwrap();
            liquid.lava = lava;
            liquids.push(liquid);
        }
        let scene = SceneCollision {
            water: liquids,
            static_skin: StaticLevelCollision::from_layers(&vec![TriMesh::default(); 10]).unwrap(),
            force_fields: vec![],
            animated_parts: vec![],
        };
        let mut physics = PhysicsWorld::new(-50.0);
        let mut body = CharacterBody::new(0.75, 0.5, 70.0, 5.0);
        body.position = [0.0, 1.0, 0.0];
        body.apply_gravity = false;
        physics.bodies.push(body);
        let events = physics.integrate(1.0 / 60.0, &scene, &|_, _| None);
        let (depth, lava) = events
            .iter()
            .find_map(|(_, e)| match e {
                BodyEvent::Drown { depth, lava } => Some((*depth, *lava)),
                _ => None,
            })
            .expect("deep Water contact must drown");
        assert!((depth - 1.2).abs() < 0.001, "{events:?}");
        assert_eq!(
            lava, lava_last,
            "effect follows last material, not deepest liquid"
        );
        if lava_last {
            assert!(
                events.iter().any(|(_, e)| matches!(e,
                BodyEvent::LiquidStatus { depth, lava: true } if (*depth - 1.2).abs() < 0.001)),
                "shallow Lava status uses previously accumulated deeper Water depth"
            );
        }
    }
}
#[test]
fn frozen_scene_water_supports_character_and_thawed_water_does_not() {
    use magicka_runtime::level_collision::{SceneCollision, StaticLevelCollision};
    use magicka_sim::character_physics::{CharacterBody, PhysicsWorld};
    let geometry = TriMesh {
        vertices: vec![[-10.0, 0.0, -10.0], [10.0, 0.0, -10.0], [0.0, 0.0, 10.0]],
        triangles: vec![[0, 2, 1]],
    };
    let mut heights = Vec::new();
    for frozen in [false, true] {
        let mut water = WaterCollision::from_geometry(&geometry, true, false).unwrap();
        if frozen {
            water.freeze.freeze_all(1.0);
        }
        let scene = SceneCollision {
            water: vec![water],
            static_skin: StaticLevelCollision::from_layers(&vec![TriMesh::default(); 10]).unwrap(),
            force_fields: vec![],
            animated_parts: vec![],
        };
        let mut physics = PhysicsWorld::new(-50.0);
        let mut body = CharacterBody::new(0.75, 0.5, 70.0, 5.0);
        body.position = [0.0, 1.1, 0.0];
        physics.bodies.push(body);
        let mut peak_depth = 0.0f32;
        for _ in 0..60 {
            physics.integrate(1.0 / 60.0, &scene, &|_, _| None);
            peak_depth = peak_depth.max(physics.bodies[0].owner.water_depth);
        }
        if !frozen {
            assert!(peak_depth > 0.0, "missing Water contact depth");
        }
        assert_eq!(physics.bodies[0].is_on_ice(), frozen);
        if frozen {
            let points = physics.contact_points(0);
            assert!(!points.is_empty());
            assert!(points.iter().flatten().all(|value| value.is_finite()));
        }
        heights.push(physics.bodies[0].position[1]);
    }
    assert!(
        heights[0] < 0.0,
        "water unexpectedly supported body: {heights:?}"
    );
    assert!(
        heights[1] > 0.8 && heights[1] < 1.2,
        "ice failed to support body: {heights:?}"
    );
}
#[test]
fn freeze_loop_queries_geometry_and_clips_each_arc_before_selection() {
    let geometry = TriMesh {
        vertices: vec![[0.5, 0.0, 0.0], [1.5, 0.0, 0.0], [0.5, 0.0, 0.1]],
        triangles: vec![[0, 2, 1]],
    };
    let mut water = WaterCollision::from_geometry(&geometry, true, false).unwrap();
    let mut clips = 0;
    water.freeze_local([0.0, 7.0, 0.0], [2.0, 0.0, 0.0], 0.0, 0.75, |segment| {
        clips += 1;
        segment.delta = segment.delta.map(|v| v * 0.5);
    });
    assert_eq!(clips, 1);
    assert_eq!(water.freeze.vertices, [0.75, 0.0, 0.0]);
    water.freeze.freezable = false;
    water.freeze_local([0.0; 3], [2.0, 0.0, 0.0], 1.0, 1.0, |_| {
        panic!("nonfreezable queried shields")
    });
}
#[test]
fn freeze_shield_segments_keep_origin_height_across_multiple_arcs() {
    let geometry = TriMesh {
        vertices: vec![[0.5, 0.0, 0.0], [1.5, 0.0, 0.0], [0.5, 0.0, 0.1]],
        triangles: vec![[0, 2, 1]],
    };
    let mut water = WaterCollision::from_geometry(&geometry, true, false).unwrap();
    let mut origins = Vec::new();
    water.freeze_local([0.0, 7.0, 0.0], [2.0, 0.0, 0.0], 1.0, 0.75, |segment| {
        origins.push(segment.origin);
    });
    assert_eq!(origins, vec![[0.0, 7.0, 0.0]; 3]);
    // The vertex query still ignores height even though the shield segment
    // remains at y=7; it reaches the middle arc's vertex at y=0.
    assert!(water.freeze.vertices[0] >= 0.75);
}

#[test]
fn retained_scene_water_thaws_only_with_present_data_channel() {
    use magicka_runtime::level_collision::{SceneCollision, StaticLevelCollision};
    use magicka_sim::spray_pulse::Segment;
    let geometry = TriMesh {
        vertices: vec![[-1.0, 0.0, -1.0], [1.0, 0.0, -1.0], [0.0, 0.0, 1.0]],
        triangles: vec![[0, 2, 1]],
    };
    let mut water = WaterCollision::from_geometry(&geometry, true, false).unwrap();
    water.freeze.freeze_all(0.5);
    let mut scene = SceneCollision {
        water: vec![water],
        static_skin: StaticLevelCollision::from_layers(&vec![TriMesh::default(); 10]).unwrap(),
        force_fields: vec![],
        animated_parts: vec![],
    };
    let segment = Segment {
        origin: [0.0, 1.0, 0.0],
        delta: [0.0, -2.0, 0.0],
    };
    assert!(scene
        .liquid_segment_intersect(segment, true, true, false)
        .is_some());
    scene.update_water(1.0, false);
    assert_eq!(scene.water[0].freeze.vertices, [0.5; 3]);
    scene.update_water(1.0, true);
    assert!(scene
        .liquid_segment_intersect(segment, true, true, false)
        .is_none());
    assert!(scene
        .liquid_segment_intersect(segment, true, false, true)
        .is_some());
    scene.freeze_water(
        [0.0; 3],
        [3.0, 0.0, 0.0],
        std::f32::consts::TAU,
        1.0,
        &[magicka_sim::damage::Damage {
            element: magicka_sim::spells::Elements::COLD.0,
            magnitude: 1.0,
            ..Default::default()
        }],
        |_| {},
    );
    assert!(scene
        .liquid_segment_intersect(segment, true, true, false)
        .is_some());
}
#[test]
fn segment_filters_and_native_shortening_fraction_are_preserved() {
    use magicka_sim::spray_pulse::Segment;
    let geometry = TriMesh {
        vertices: vec![
            [-1.0, 0.0, -1.0],
            [1.0, 0.0, -1.0],
            [0.0, 0.0, 1.0],
            [-1.0, 0.5, -1.0],
            [1.0, 0.5, -1.0],
            [0.0, 0.5, 1.0],
        ],
        triangles: vec![[0, 2, 1], [3, 5, 4]],
    };
    let mut water = WaterCollision::from_geometry(&geometry, true, false).unwrap();
    let segment = Segment {
        origin: [0.0, 2.0, 0.0],
        delta: [0.0, -4.0, 0.0],
    };
    let hit = water
        .segment_intersect(segment, true, false, false)
        .unwrap();
    // First surface shortens the segment to y=0 with fraction .5. The
    // nearer second surface has relative fraction .75, which fails the
    // native comparison against .5 and is ignored.
    assert_eq!(hit.triangle, 0);
    assert_eq!(hit.fraction, 0.5);
    assert_eq!(hit.position, [0.0, 0.0, 0.0]);
    assert_eq!(hit.normal, [0.0, 1.0, 0.0]);
    let inverse = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -10.0, 0.0, 0.0, 1.0,
    ];
    let translated = Segment {
        origin: [10.0, 2.0, 0.0],
        delta: segment.delta,
    };
    assert_eq!(
        water.segment_intersect_transformed(translated, &inverse, true, false, false),
        Some(hit)
    );
    assert!(water
        .segment_intersect(segment, true, true, false)
        .is_none());
    water.freeze.freeze_all(1.0);
    assert!(water
        .segment_intersect(segment, true, false, true)
        .is_none());
    assert!(water.segment_intersect(segment, true, true, true).is_none());
}

#[test]
fn same_mesh_changes_from_water_to_ice_and_back_after_thaw() {
    let geometry = TriMesh {
        vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        triangles: vec![[0, 2, 1]],
    };
    let bounds = Bounds {
        min: [-2.0; 3],
        max: [2.0; 3],
    };
    let mut water = WaterCollision::from_geometry(&geometry, true, false).unwrap();
    assert_eq!(water.candidates(bounds, 2048, false), [0]);
    assert!(water.candidates(bounds, 2048, true).is_empty());
    water.freeze.freeze_arc_batches(&[vec![1]], 0.5);
    assert_eq!(water.candidates(bounds, 2048, true), [0]);
    assert!(water.candidates(bounds, 2048, false).is_empty());
    water.freeze.update(1.0, true);
    assert_eq!(water.candidates(bounds, 2048, false), [0]);
    assert!(water.candidates(bounds, 2048, true).is_empty());
    assert_eq!(
        water.mesh.triangle(0),
        [
            geometry.vertices[0],
            geometry.vertices[2],
            geometry.vertices[1]
        ]
    );
}

#[test]
fn molten_lava_drowns_with_lava_flag_and_cooled_rock_supports_character() {
    use magicka_runtime::level_collision::{SceneCollision, StaticLevelCollision};
    use magicka_sim::character_physics::{BodyEvent, CharacterBody, PhysicsWorld};
    let geometry = TriMesh {
        vertices: vec![[-10.0, 0.0, -10.0], [10.0, 0.0, -10.0], [0.0, 0.0, 10.0]],
        triangles: vec![[0, 2, 1]],
    };
    for cooled in [false, true] {
        let mut lava = WaterCollision::from_geometry(&geometry, true, false).unwrap();
        lava.lava = true;
        if cooled {
            lava.freeze.freeze_all(1.0);
        }
        let scene = SceneCollision {
            water: vec![lava],
            static_skin: StaticLevelCollision::from_layers(&vec![TriMesh::default(); 10]).unwrap(),
            force_fields: vec![],
            animated_parts: vec![],
        };
        let mut physics = PhysicsWorld::new(-50.0);
        let mut body = CharacterBody::new(0.75, 0.5, 70.0, 5.0);
        body.position = [0.0, 1.1, 0.0];
        physics.bodies.push(body);
        let mut drown = false;
        let mut liquid_status = false;
        for _ in 0..60 {
            let events = physics.integrate(1.0 / 60.0, &scene, &|_, _| None);
            let status = events
                .iter()
                .position(|(_, e)| matches!(e, BodyEvent::LiquidStatus { lava: true, .. }));
            if let Some(drown_index) = events
                .iter()
                .position(|(_, e)| matches!(e, BodyEvent::Drown { lava: true, .. }))
            {
                assert!(
                    status.is_some_and(|i| i < drown_index),
                    "contact status must precede drowning"
                );
            }
            liquid_status |= status.is_some();
            drown |= events
                .iter()
                .any(|(_, e)| matches!(e, BodyEvent::Drown { lava: true, .. }));
        }
        assert_eq!(drown, !cooled);
        assert_eq!(liquid_status, !cooled);
        // Native ice requires a Water tag; cooled lava is not slippery.
        assert!(!physics.bodies[0].is_on_ice());
        if cooled {
            assert!(physics.bodies[0].position[1] > 0.8);
        } else {
            assert_eq!(physics.bodies[0].ground_material, 9);
        }
    }
}

#[test]
fn lava_repeated_arc_vertices_accumulate_and_render_only_surface_has_no_contacts() {
    let geometry = TriMesh {
        vertices: vec![[0.5, 0.0, 0.0], [1.5, 0.0, 0.0], [0.5, 0.0, 0.1]],
        triangles: vec![[0, 2, 1]],
    };
    let mut water = WaterCollision::from_geometry(&geometry, true, false).unwrap();
    let mut lava = water.clone();
    lava.lava = true;
    for liquid in [&mut water, &mut lava] {
        liquid.freeze_local(
            [0.0; 3],
            [2.0, 0.0, 0.0],
            std::f32::consts::PI / 2.0,
            0.1,
            |_| {},
        );
    }
    assert!(lava
        .freeze
        .vertices
        .iter()
        .zip(&water.freeze.vertices)
        .any(|(a, b)| a > b));
    lava.has_collision = false;
    lava.freeze.freeze_all(2.0);
    assert!(lava
        .candidates(
            Bounds {
                min: [-10.0; 3],
                max: [10.0; 3]
            },
            2048,
            true
        )
        .is_empty());
    assert!(lava
        .segment_intersect(
            magicka_sim::spray_pulse::Segment {
                origin: [0.5, 1.0, 0.05],
                delta: [0.0, -2.0, 0.0]
            },
            false,
            false,
            false
        )
        .is_none());
}
