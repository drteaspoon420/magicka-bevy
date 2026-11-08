use magicka_content::level::TriMesh;
use magicka_runtime::level_collision::*;
use magicka_sim::{mesh_octree::MeshTransform, spray_pulse::Segment};
fn plane(z: f32) -> TriMesh {
    TriMesh {
        vertices: vec![[-1.0, -1.0, z], [1.0, -1.0, z], [0.0, 1.0, z]],
        triangles: vec![[0, 1, 2]],
    }
}
fn dynamic(z: f32) -> TransformedCollisionMesh {
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    TransformedCollisionMesh {
        mesh: collision_mesh(&plane(z)).unwrap(),
        transform: MeshTransform {
            forward: identity,
            inverse: identity,
        },
    }
}
fn segment() -> Segment {
    Segment {
        origin: [0.0; 3],
        delta: [0.0, 0.0, 8.0],
    }
}
fn scene(z: f32) -> SceneCollision {
    let mut layers = vec![TriMesh::default(); 10];
    layers[4] = plane(z);
    SceneCollision {
        water: Vec::new(),
        static_skin: StaticLevelCollision::from_layers(&layers).unwrap(),
        force_fields: vec![],
        animated_parts: vec![],
    }
}
fn part(id: i32, z: f32, children: Vec<AnimatedCollisionPart>) -> AnimatedCollisionPart {
    AnimatedCollisionPart {
        id,
        owner: id as u32,
        material: 7,
        collision: Some(dynamic(z)),
        children,
    }
}
#[test]
fn nearest_dynamic_hit_sets_material_and_descendant_identity() {
    let mut s = scene(6.0);
    s.force_fields.push(dynamic(4.0));
    s.animated_parts
        .push(part(10, 3.0, vec![part(20, 2.0, vec![])]));
    let hit = s.segment_intersect(segment(), false).unwrap();
    assert_eq!(hit.animated_part, Some(20));
    assert_eq!(hit.primitive, 7);
    assert_eq!(hit.hit.fraction, 0.25);
}
#[test]
fn equal_hits_preserve_static_then_force_then_parent_order() {
    let mut s = scene(4.0);
    s.force_fields.push(dynamic(4.0));
    s.animated_parts
        .push(part(10, 4.0, vec![part(20, 4.0, vec![])]));
    let hit = s.segment_intersect(segment(), false).unwrap();
    assert_eq!(hit.primitive, 4);
    assert_eq!(hit.animated_part, None);
    s.force_fields[0] = dynamic(2.0);
    s.animated_parts[0] = part(10, 2.0, vec![]);
    let hit = s.segment_intersect(segment(), false).unwrap();
    assert_eq!(hit.primitive, 0);
    assert_eq!(hit.animated_part, None);
    assert_eq!(
        part(10, 2.0, vec![part(20, 2.0, vec![])])
            .segment_intersect(segment())
            .unwrap()
            .animated_part,
        Some(10)
    );
}
#[test]
fn static_backface_flag_does_not_cull_dynamic_parts() {
    let mut s = scene(4.0);
    assert!(s.segment_intersect(segment(), true).is_none());
    s.force_fields.push(dynamic(4.0));
    assert!(s.segment_intersect(segment(), true).is_some());
    s.force_fields.clear();
    s.animated_parts.push(AnimatedCollisionPart {
        id: 10,
        owner: 10,
        material: 1,
        collision: None,
        children: vec![part(20, 2.0, vec![])],
    });
    assert_eq!(
        s.segment_intersect(segment(), true).unwrap().animated_part,
        Some(20)
    );
}
