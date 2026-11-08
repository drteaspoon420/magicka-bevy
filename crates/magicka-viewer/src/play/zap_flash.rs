//! LightningZapRenderData's flash (research/zap.md): while a character is
//! zapped, ArcaneEffect's Generic technique draws EffectTextures/LightningHit
//! on a camera-facing quad at the render bounding sphere centre, scaled by
//! 0.85·radius and rotated by a fresh random angle every frame, coloured
//! LIGHTNINGCOLOR (centre and edge) at alpha 1, SrcAlpha/One.
//!
//! The original draws it where the body stencil is not 1 (outside the
//! visible zapped body). Bevy has no stencil here: the depth test against the
//! scene keeps the flash behind the body's front surface, which covers the
//! same pixels for a body around the quad's plane (INFERRED stand-in).
use super::Play;
use bevy::prelude::*;
use magicka_runtime::zap_skeleton;

#[derive(Component)]
pub(super) struct ZapFlash(u16);

#[derive(Resource)]
pub(super) struct ZapFlashAssets {
    mesh: Handle<Mesh>,
    material: Handle<crate::beam::BeamMaterial>,
}

/// sFlashVertexBuffer as a mesh. Vertex colour red is the shader's alpha
/// input (Generic's vertex shader passes the Alpha parameter, 1).
fn flash_mesh() -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let quad = zap_skeleton::FLASH_QUAD;
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, quad.map(|(p, _)| p).to_vec())
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, quad.map(|(_, t)| t).to_vec())
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 0.0, -1.0]; 4])
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32, 1.0, 1.0, 1.0]; 4])
    .with_inserted_indices(Indices::U32(zap_skeleton::FLASH_TRIANGLES.to_vec()))
}

pub(super) fn setup_zap_flash(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::beam::BeamMaterial>>,
) {
    use bevy::image::{ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
    // TextureSamplerClamped: linear, clamp.
    let texture = assets
        .load_builder()
        .with_settings(|s: &mut ImageLoaderSettings| {
            s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
        })
        .load("textures/EffectTextures/LightningHit.png");
    let color = magicka_runtime::beam_mesh::LIGHTNING_COLOR;
    let color = Vec4::new(color[0], color[1], color[2], 1.0);
    commands.insert_resource(ZapFlashAssets {
        mesh: meshes.add(flash_mesh()),
        material: materials.add(crate::beam::BeamMaterial {
            color_center: color,
            color_edge: color,
            texture,
        }),
    });
}

/// MagickaMath.RandomBetween(0, 2π) stand-in: a viewer-local xorshift so
/// captures stay reproducible (the shared native random stream is not
/// consumed by presentation here).
fn next_angle(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 8) as f32 / (1u32 << 24) as f32 * std::f32::consts::TAU
}

#[allow(clippy::type_complexity)]
pub(super) fn sync_zap_flashes(
    mut commands: Commands,
    play: Res<Play>,
    flash: Option<Res<ZapFlashAssets>>,
    mut draws: Query<(Entity, &ZapFlash, &mut Transform)>,
    cameras: Query<&GlobalTransform, With<super::PlayCamera>>,
    pilot: Option<Res<super::Autopilot>>,
    mut random: Local<u32>,
) {
    let Some(flash) = flash else { return };
    let hidden = pilot.is_some_and(|p| {
        p.started.is_some_and(|start| {
            super::script_act(p.script, p.frame - start) == Some(super::Act::ZapFlashHidden)
        })
    });
    let Some(camera) = cameras.iter().next() else {
        return;
    };
    if *random == 0 {
        *random = 0x9E37_79B9;
    }
    let eye = camera.translation().to_array();
    let forward = camera.forward().to_array();
    let mut pending: Vec<(u16, zap_skeleton::FlashSphere)> = play
        .session
        .zapped_characters()
        .into_iter()
        .filter_map(|z| Some((z.handle, z.flash?)))
        .filter(|_| !hidden)
        .collect();
    for (entity, draw, mut transform) in &mut draws {
        match pending.iter().position(|(h, _)| *h == draw.0) {
            Some(i) => {
                let (_, sphere) = pending.swap_remove(i);
                let m =
                    zap_skeleton::flash_transform(&sphere, eye, forward, next_angle(&mut random));
                *transform = Transform::from_matrix(Mat4::from_cols_array(&m));
            }
            None => commands.entity(entity).despawn(),
        }
    }
    for (handle, sphere) in pending {
        let m = zap_skeleton::flash_transform(&sphere, eye, forward, next_angle(&mut random));
        commands.spawn((
            ZapFlash(handle),
            Mesh3d(flash.mesh.clone()),
            MeshMaterial3d(flash.material.clone()),
            Transform::from_matrix(Mat4::from_cols_array(&m)),
            bevy::light::NotShadowCaster,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_mesh_is_the_two_triangle_fan() {
        let mesh = flash_mesh();
        assert_eq!(mesh.count_vertices(), 4);
        assert_eq!(mesh.indices().map(|i| i.len()), Some(6));
    }

    #[test]
    fn flash_angles_cover_the_full_turn() {
        let mut state = 0x9E37_79B9;
        let angles: Vec<f32> = (0..256).map(|_| next_angle(&mut state)).collect();
        assert!(angles
            .iter()
            .all(|a| (0.0..std::f32::consts::TAU).contains(a)));
        assert!(angles.iter().any(|&a| a < 1.0) && angles.iter().any(|&a| a > 5.0));
    }
}
