use magicka_sim::{mesh_octree::*, spray_pulse::Segment};
fn broad() -> Bounds {
    Bounds {
        min: [-10.0; 3],
        max: [10.0; 3],
    }
}
#[test]
fn breadth_first_order_places_root_triangle_before_earlier_child_triangle() {
    let v = vec![
        [-3.0, -3.0, 0.0],
        [-2.0, -3.0, 0.0],
        [-3.0, -2.0, 0.0],
        [-4.0, -4.0, 0.0],
        [4.0, -4.0, 0.0],
        [0.0, 4.0, 0.0],
    ];
    let tree = MeshOctree::build(v, vec![[0, 1, 2], [3, 4, 5]]).unwrap();
    assert_eq!(tree.candidates(broad(), 2048), [1, 0]);
    assert_eq!(tree.candidates(broad(), 1), [1]);
    let hit = tree
        .segment_intersect(
            Segment {
                origin: [-2.75, -2.75, 1.0],
                delta: [0.0, 0.0, -2.0],
            },
            false,
        )
        .unwrap();
    assert_eq!(hit.triangle, 1);
    assert_eq!(hit.hit.fraction, 0.5);
}
#[test]
fn nearest_fraction_and_backfaces() {
    let tree = MeshOctree::build(
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
        ],
        vec![[0, 1, 2], [3, 4, 5]],
    )
    .unwrap();
    let hit = tree
        .segment_intersect(
            Segment {
                origin: [0.25, 0.25, 2.0],
                delta: [0.0, 0.0, -4.0],
            },
            true,
        )
        .unwrap();
    assert_eq!(hit.triangle, 1);
    assert_eq!(hit.position, [0.25, 0.25, 1.0]);
    assert!(tree
        .segment_intersect(
            Segment {
                origin: [0.25, 0.25, -1.0],
                delta: [0.0, 0.0, 4.0]
            },
            true
        )
        .is_none());
}
#[test]
fn candidate_cap_and_invalid_input() {
    let tree = MeshOctree::build(
        vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        vec![[0, 1, 2]; 2050],
    )
    .unwrap();
    assert_eq!(tree.candidates(broad(), 2048).len(), 2048);
    assert!(matches!(
        MeshOctree::build(vec![], vec![[0, 1, 2]]),
        Err(MeshError::InvalidIndex)
    ));
    assert!(matches!(
        MeshOctree::build(vec![[f32::NAN; 3]], vec![]),
        Err(MeshError::NonFiniteVertex)
    ));
    assert!(matches!(
        MeshOctree::build(vec![[0.0; 3]], vec![[0, 0, 0]]),
        Err(MeshError::NonShrinkingCell)
    ));
    assert!(MeshOctree::build(vec![], vec![])
        .unwrap()
        .candidates(broad(), 2048)
        .is_empty());
}

#[test]
fn transformed_mesh_returns_world_position_and_rotated_normal() {
    let mesh = MeshOctree::build(
        vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        vec![[0, 1, 2]],
    )
    .unwrap();
    // Local x->world -z, local z->world x; translation (3,4,5).
    let transform = MeshTransform {
        forward: [
            0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 3.0, 4.0, 5.0, 1.0,
        ],
        inverse: [
            0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 5.0, -4.0, -3.0, 1.0,
        ],
    };
    let hit = mesh
        .segment_intersect_transformed(
            Segment {
                origin: [4.0, 4.25, 4.75],
                delta: [-2.0, 0.0, 0.0],
            },
            true,
            transform,
        )
        .unwrap();
    assert_eq!(hit.hit.fraction, 0.5);
    assert_eq!(hit.position, [3.0, 4.25, 4.75]);
    assert_eq!(hit.normal, [1.0, 0.0, 0.0]);
}
#[test]
fn normal_preserves_forward_scale_without_normalization() {
    let mesh = MeshOctree::build(
        vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        vec![[0, 1, 2]],
    )
    .unwrap();
    let transform = MeshTransform {
        forward: [
            2.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
        inverse: [
            0.5,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0 / 3.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.25,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ],
    };
    let hit = mesh
        .segment_intersect_transformed(
            Segment {
                origin: [0.5, 0.75, 4.0],
                delta: [0.0, 0.0, -8.0],
            },
            false,
            transform,
        )
        .unwrap();
    assert_eq!(hit.normal, [0.0, 0.0, 4.0]);
    assert_eq!(hit.position, [0.5, 0.75, 0.0]);
}
