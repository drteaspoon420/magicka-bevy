//! Decoded prop collision data for the modern Avian solver.
use crate::{pickup_physics::PickupBody, pickups::PickupBounds};
use magicka_content::physics::PhysicsEntityTemplate;
use magicka_physics::{BodyKey, BodyShape};

/// JigLibX boxes store their corner; the modern backend stores the centre.
/// Rotate the half sides before composing the entity's world transform.
pub fn collision_box(
    b: &magicka_content::physics::CollisionBox,
    transform: &[f32; 16],
) -> magicka_sim::box_intersection::OrientedBox {
    let local = magicka_content::skinned::pose_matrix(b.position, b.orientation, [1.0; 3]);
    let world = magicka_content::skinned::mul(&local, transform);
    let axes = [
        [world[0], world[1], world[2]],
        [world[4], world[5], world[6]],
        [world[8], world[9], world[10]],
    ];
    magicka_sim::box_intersection::OrientedBox {
        center: std::array::from_fn(|i| {
            world[12 + i] + (0..3).map(|a| axes[a][i] * b.sides[a] * 0.5).sum::<f32>()
        }),
        axes,
        side_lengths: b.sides,
    }
}

pub fn body(
    handle: u16,
    generation: u64,
    name: &str,
    transform: [f32; 16],
    template: &PhysicsEntityTemplate,
) -> Option<PickupBody> {
    if !template.mass.is_finite() || (template.movable && template.mass <= 0.) {
        return None;
    }
    // The first decoded box is used by the original reader. Mesh-only dynamic
    // props use a convex hull in Avian, an explicit modern backend policy.
    // AnimatedPhysicsEntity.CreateBody replaces the template box.
    let anim_box = crate::anim_prop::collision_box(template);
    let (bounds, shape) = if let Some(b) = anim_box.as_ref().or(template.boxes.first()) {
        if b.sides.iter().any(|v| !v.is_finite() || *v <= 0.)
            || b.position.iter().any(|v| !v.is_finite())
            || b.orientation.iter().any(|v| !v.is_finite())
            || b.orientation.iter().map(|v| v * v).sum::<f32>() <= f32::EPSILON
        {
            return None;
        }
        let local_center = collision_box(b, &magicka_content::skinned::IDENTITY).center;
        (
            PickupBounds {
                min: std::array::from_fn(|i| local_center[i] - b.sides[i] * 0.5),
                max: std::array::from_fn(|i| local_center[i] + b.sides[i] * 0.5),
            },
            BodyShape::Box {
                size: b.sides,
                rotation: b.orientation,
            },
        )
    } else if let Some(mesh) = &template.collision_mesh {
        let triangles = mesh
            .triangles
            .iter()
            .map(|t| {
                t.iter()
                    .map(|i| u32::try_from(*i).ok())
                    .collect::<Option<Vec<_>>>()
                    .map(|v| [v[0], v[1], v[2]])
            })
            .collect::<Option<Vec<_>>>()?;
        (
            PickupBounds {
                min: [0.; 3],
                max: [0.; 3],
            },
            BodyShape::Mesh {
                vertices: mesh.vertices.clone(),
                triangles,
            },
        )
    } else {
        return None;
    };
    Some(PickupBody {
        key: BodyKey { handle, generation },
        model: format!("Data/PhysicsEntities/{name}"),
        transform,
        bounds,
        book: false,
        immovable: if anim_box.is_some() {
            !template.pushable
        } else {
            !template.movable
        },
        // Static body mass is unused by the solver; zero is valid content data.
        mass: if !template.movable && template.mass == 0. {
            1.
        } else {
            template.mass
        },
        solid: template.solid,
        shape: Some(shape),
    })
}
