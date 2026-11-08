//! Scene collision as character-physics static geometry. See
//! research/character-physics.md.
//!
//! Every level skin (LevelModel static layers, ForceField, AnimatedLevelPart) is
//! registered at scene initialisation and tagged with the LevelModel. Candidate
//! triangles come from each mesh's octree query for the world box, transformed
//! meshes querying the box spanned by the eight inverse-transformed corners.
use crate::level_collision::{AnimatedCollisionPart, SceneCollision, TransformedCollisionMesh};
use magicka_sim::{
    character_physics::{CollisionTriangle, StaticGeometry},
    impact_motion::Vector,
    mesh_octree::{Bounds, MeshOctree},
};

/// DetectFunctor.IntStackAlloc capacity passed to GetTrianglesIntersectingtAABox.
const MAX_CANDIDATES: usize = 2048;
/// Primitive keys: static layers 0..10, force fields and animated parts after.
const FORCE_FIELD_KEY: u32 = 1 << 16;
pub(crate) const ANIMATED_PART_KEY: u32 = 1 << 24;
const WATER_ICE_KEY: u32 = 1 << 28;

fn push_static(mesh: &MeshOctree, world: Bounds, layer: usize, out: &mut Vec<CollisionTriangle>) {
    for i in mesh.candidates(world, MAX_CANDIDATES) {
        let [a, b, c] = mesh.triangle(i);
        // Static layer index doubles as the reported ground material.
        out.push(CollisionTriangle::new(a, b, c, layer as u32, layer as u8));
    }
}

/// Single-primitive skins report primitive index 0 (Generic) as ground
/// material, whatever material byte the part stores.
fn push_transformed(
    mesh: &TransformedCollisionMesh,
    world: Bounds,
    key: u32,
    out: &mut Vec<CollisionTriangle>,
) {
    let local = mesh.transform.local_bounds(world);
    for i in mesh.mesh.candidates(local, MAX_CANDIDATES) {
        let [a, b, c] = mesh.mesh.triangle(i).map(|p| mesh.transform.point(p));
        out.push(CollisionTriangle::new(a, b, c, key, 0));
    }
}

fn push_part(
    part: &AnimatedCollisionPart,
    world: Bounds,
    next: &mut u32,
    out: &mut Vec<CollisionTriangle>,
) {
    if let Some(mesh) = &part.collision {
        push_transformed(mesh, world, ANIMATED_PART_KEY + *next, out);
    }
    *next += 1;
    for child in &part.children {
        push_part(child, world, next, out);
    }
}

impl StaticGeometry for SceneCollision {
    fn triangles_in_box(&self, min: Vector, max: Vector, out: &mut Vec<CollisionTriangle>) {
        let world = Bounds { min, max };
        for (layer, mesh) in self.static_skin.layers().iter().enumerate() {
            push_static(mesh, world, layer, out);
        }
        for (i, field) in self.force_fields.iter().enumerate() {
            push_transformed(field, world, FORCE_FIELD_KEY + i as u32, out);
        }
        let mut next = 0;
        for part in &self.animated_parts {
            push_part(part, world, &mut next, out);
        }
        for (index, water) in self.water.iter().enumerate() {
            for triangle in water.candidates(world, MAX_CANDIDATES, true) {
                let [a, b, c] = water.mesh.triangle(triangle);
                let mut contact = CollisionTriangle::new(a, b, c, WATER_ICE_KEY + index as u32, 0);
                // CharacterBody.HandleCollisions: mIsOnIce needs an IceMesh
                // primitive and a Water tag; frozen Lava is ordinary rock.
                contact.water_ice = !water.lava;
                out.push(contact);
            }
            for triangle in water.candidates(world, MAX_CANDIDATES, false) {
                let [a, b, c] = water.mesh.triangle(triangle);
                let mut contact = CollisionTriangle::new(
                    a,
                    b,
                    c,
                    WATER_ICE_KEY + (1 << 20) + index as u32,
                    if water.lava { 9 } else { 8 },
                );
                contact.water_surface = true;
                out.push(contact);
            }
        }
    }
}

/// Primitive keys for solid static physics-entity boxes, after water.
const PROP_BOX_KEY: u32 = 1 << 30;

/// Level collision plus the boxes of solid, static physics entities.
/// PhysicsEntity.OnCollision returns the template Solid flag for entity
/// contacts and static entities ignore their own skin in PostCollision, so a
/// character meets an immovable box. Each box is presented as twelve outward
/// triangles sharing one primitive key (single-primitive skin, Generic
/// ground material); this replaces JigLibX's box detector in the modern host.
pub struct WithPropBoxes<'a> {
    pub scene: &'a SceneCollision,
    pub boxes: &'a [magicka_sim::box_intersection::OrientedBox],
}

/// Outward-facing triangles of an oriented box.
pub fn box_triangles(
    b: &magicka_sim::box_intersection::OrientedBox,
    key: u32,
) -> [CollisionTriangle; 12] {
    let half: [Vector; 3] =
        std::array::from_fn(|a| std::array::from_fn(|i| b.axes[a][i] * b.side_lengths[a] * 0.5));
    let add = |p: Vector, q: Vector, s: f32| -> Vector { std::array::from_fn(|i| p[i] + q[i] * s) };
    let mut out = [CollisionTriangle::new([0.; 3], [0.; 3], [0.; 3], key, 0); 12];
    let mut n = 0;
    for a in 0..3 {
        let (u, v) = ((a + 1) % 3, (a + 2) % 3);
        for s in [-1.0f32, 1.0] {
            let face = add(b.center, half[a], s);
            let corner = |su: f32, sv: f32| add(add(face, half[u], su), half[v], sv);
            let quad = [
                corner(-1., -1.),
                corner(1., -1.),
                corner(1., 1.),
                corner(-1., 1.),
            ];
            for [i, j, k] in [[0, 1, 2], [0, 2, 3]] {
                let mut t = CollisionTriangle::new(quad[i], quad[j], quad[k], key, 0);
                let outward: f32 = (0..3).map(|c| t.normal[c] * half[a][c] * s).sum();
                if outward < 0.0 {
                    t = CollisionTriangle::new(quad[i], quad[k], quad[j], key, 0);
                }
                out[n] = t;
                n += 1;
            }
        }
    }
    out
}

impl StaticGeometry for WithPropBoxes<'_> {
    fn triangles_in_box(&self, min: Vector, max: Vector, out: &mut Vec<CollisionTriangle>) {
        self.scene.triangles_in_box(min, max, out);
        for (index, b) in self.boxes.iter().enumerate() {
            let extent: Vector = std::array::from_fn(|i| {
                (0..3)
                    .map(|a| (b.axes[a][i] * b.side_lengths[a] * 0.5).abs())
                    .sum::<f32>()
            });
            if (0..3).any(|i| b.center[i] + extent[i] < min[i] || b.center[i] - extent[i] > max[i])
            {
                continue;
            }
            out.extend(box_triangles(b, PROP_BOX_KEY + index as u32));
        }
    }
}
