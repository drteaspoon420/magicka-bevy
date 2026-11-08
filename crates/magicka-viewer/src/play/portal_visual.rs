//! PortalEntity.RenderData (research/portal.md): an additive camera-facing
//! quad, 2 m wide and 4 m tall, at each live portal. It shows the blue
//! (left) or orange (right) half of EffectTextures/portal at alpha
//! mEffectAlpha. The particle effects come from the runtime's effect
//! commands.
use super::Play;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use magicka_sim::portal::PortalType;

#[derive(Component)]
pub(super) struct PortalDraw {
    kind: PortalType,
    material: Handle<crate::additive::AdditiveMaterial>,
}

/// RenderData.QUAD with TextureScale (0.5, 1) and the type's offset baked
/// into the UVs. It faces +Z with counter-clockwise winding.
fn quad(kind: PortalType) -> Mesh {
    let u0 = kind.texture_offset();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-0.5, -0.5, 0.0],
            [-0.5, 0.5, 0.0],
            [0.5, 0.5, 0.0],
            [0.5, -0.5, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4])
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[u0, 1.0], [u0, 0.0], [u0 + 0.5, 0.0], [u0 + 0.5, 1.0]],
    )
    .with_inserted_indices(Indices::U16(vec![0, 2, 1, 0, 3, 2]))
}

/// PreRenderUpdate: forward = toward the camera, right = Up × forward,
/// up = forward × right, with right scaled by 2 and up by 4. The native
/// matrix stores −forward in its third row, a mirror. For a flat quad that
/// differs only in which face shows, so the rewrite keeps +forward and a
/// proper rotation.
fn billboard(position: Vec3, eye: Vec3) -> Transform {
    let forward = (eye - position).normalize_or(Vec3::Z);
    let right = Vec3::Y.cross(forward).normalize_or(Vec3::X);
    let up = forward.cross(right);
    Transform::from_matrix(Mat4::from_cols(
        (right * 2.0).extend(0.0),
        (up * 4.0).extend(0.0),
        forward.extend(0.0),
        position.extend(1.0),
    ))
}

#[allow(clippy::type_complexity)]
pub(super) fn sync_portals(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::additive::AdditiveMaterial>>,
    mut draws: Query<(Entity, &PortalDraw, &mut Transform)>,
    cameras: Query<&GlobalTransform, With<super::PlayCamera>>,
) {
    let Some(eye) = cameras.iter().next().map(|t| t.translation()) else {
        return;
    };
    let portal = &play.session.world.portals.portal;
    let live: Vec<_> = [portal.a, portal.b]
        .into_iter()
        .filter(|p| !p.dead)
        .collect();
    for (entity, draw, mut transform) in &mut draws {
        match live.iter().find(|p| p.kind == draw.kind) {
            Some(p) => {
                *transform = billboard(Vec3::from(p.position), eye);
                if let Some(mut m) = materials.get_mut(&draw.material) {
                    m.color = Vec4::new(1.0, 1.0, 1.0, p.effect_alpha);
                }
            }
            None => commands.entity(entity).despawn(),
        }
    }
    for p in &live {
        if draws.iter().any(|(_, d, _)| d.kind == p.kind) {
            continue;
        }
        let material = materials.add(crate::additive::AdditiveMaterial {
            color: Vec4::new(1.0, 1.0, 1.0, p.effect_alpha),
            flags: UVec4::new(1, 0, 0, 0),
            texture: Some(assets.load("textures/EffectTextures/portal.png")),
        });
        commands.spawn((
            PortalDraw {
                kind: p.kind,
                material: material.clone(),
            },
            Mesh3d(meshes.add(quad(p.kind))),
            MeshMaterial3d(material),
            billboard(Vec3::from(p.position), eye),
            Visibility::default(),
            bevy::light::NotShadowCaster,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billboard_faces_the_camera_upright() {
        let t = billboard(Vec3::ZERO, Vec3::new(0.0, 0.0, 10.0));
        let m = t.to_matrix();
        assert!(m.x_axis.truncate().abs_diff_eq(Vec3::X * 2.0, 1e-5));
        assert!(m.y_axis.truncate().abs_diff_eq(Vec3::Y * 4.0, 1e-5));
        assert!(m.z_axis.truncate().abs_diff_eq(Vec3::Z, 1e-5));
    }
}
