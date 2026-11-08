//! Wave.RenderData: the NormalDistortionEffect arc sweeping forward for the
//! effect's second (research/wave.md). One material per live effect carries
//! its Distortion and Time.
use super::*;
use bevy::{
    asset::{embedded_asset, RenderAssetUsages},
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

/// Wave.RenderData.Draw: TextureScale 4.
const TEXTURE_SCALE: f32 = 4.0;

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub(super) struct WaveMaterial {
    /// x: Distortion, y: Time, z: 1 / TextureScale.
    #[uniform(0)]
    params: Vec4,
    #[texture(1)]
    #[sampler(2)]
    normal: Handle<Image>,
}
impl Material for WaveMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/play/wave.wgsl".into()
    }
    fn reads_view_transmission_texture(&self) -> bool {
        true
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The post effect draws both faces and writes no depth.
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

pub(super) struct WaveVisualPlugin;
impl Plugin for WaveVisualPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "wave.wgsl");
        app.add_plugins(MaterialPlugin::<WaveMaterial>::default());
    }
}

/// The static Wave vertex/index buffers: 8 arc steps from −3π/4 by π/14,
/// four rings each (X radii 4/6/8/10, Z radii 2/3/4/5 around 3.5),
/// TexCoord = (X, Z), ring alphas 0, 0.5|1, 1, 0 (0 on the end steps).
pub(super) fn arc_mesh() -> Mesh {
    let mut positions = Vec::with_capacity(32);
    let mut uvs = Vec::with_capacity(32);
    let mut colors = Vec::with_capacity(32);
    let mut angle = std::f32::consts::PI * -3.0 / 4.0;
    for i in 0..8 {
        let (sin, cos) = angle.sin_cos();
        let end = i == 0 || i == 7;
        let alphas = [
            0.0,
            if end {
                0.0
            } else if i == 1 || i == 6 {
                0.5
            } else {
                1.0
            },
            if end { 0.0 } else { 1.0 },
            0.0,
        ];
        for (ring, alpha) in alphas.into_iter().enumerate() {
            let k = ring as f32;
            let (x, z) = (cos * (4.0 + 2.0 * k), sin * (2.0 + k) + 3.5);
            positions.push([x, 0.0, z]);
            uvs.push([x, z]);
            colors.push([1.0, 1.0, 1.0, alpha]);
        }
        angle += std::f32::consts::PI / 14.0;
    }
    let mut indices = Vec::with_capacity(126);
    for j in 0..7u16 {
        let b = j * 4;
        for ring in 0..3 {
            let v = b + ring;
            indices.extend_from_slice(&[v, v + 1, v + 4, v + 1, v + 5, v + 4]);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U16(indices))
}

/// XNA Matrix.CreateWorld(position, forward, Up) with row 3 (Backward)
/// scaled by `backward_scale`, as a Bevy matrix.
fn world(position: Vec3, forward: Vec3, backward_scale: f32) -> Mat4 {
    let back = (-forward).normalize_or(Vec3::Z);
    let right = Vec3::Y.cross(back).normalize_or(Vec3::X);
    let up = back.cross(right);
    Mat4::from_cols(
        right.extend(0.0),
        up.extend(0.0),
        (back * backward_scale).extend(0.0),
        position.extend(1.0),
    )
}

#[derive(Component)]
pub(super) struct WaveArc {
    serial: u64,
    material: Handle<WaveMaterial>,
}

pub(super) fn sync_wave_arcs(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WaveMaterial>>,
    mut arcs: Query<(Entity, &WaveArc, &mut Transform)>,
    mut shared: Local<Option<(Handle<Mesh>, Handle<Image>)>>,
) {
    let live: Vec<_> = play
        .session
        .world
        .waves
        .iter()
        .filter(|w| !w.removed)
        .collect();
    let params = |w: &magicka_sim::wave::WaveEffect| {
        let d = w.distortion();
        let m = world(
            Vec3::from(d.center),
            Vec3::from(w.direction),
            d.forward_scale,
        );
        (
            Transform::from_matrix(m),
            Vec4::new(d.strength, d.time, 1.0 / TEXTURE_SCALE, 0.0),
        )
    };
    for (entity, arc, mut transform) in &mut arcs {
        match live.iter().find(|w| w.serial == arc.serial) {
            Some(w) => {
                let (t, p) = params(&w.effect);
                *transform = t;
                if let Some(mut m) = materials.get_mut(&arc.material) {
                    m.params = p;
                }
            }
            None => commands.entity(entity).despawn(),
        }
    }
    for w in live {
        if arcs.iter().any(|(_, a, _)| a.serial == w.serial) {
            continue;
        }
        let (mesh, normal) = shared
            .get_or_insert_with(|| {
                let normal = assets
                    .load_builder()
                    .with_settings(|s: &mut ImageLoaderSettings| {
                        s.is_srgb = false;
                        s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                            address_mode_u: ImageAddressMode::Repeat,
                            address_mode_v: ImageAddressMode::Repeat,
                            ..ImageSamplerDescriptor::linear()
                        });
                    })
                    .load("textures/EffectTextures/normalDistortion.png");
                (meshes.add(arc_mesh()), normal)
            })
            .clone();
        let (transform, p) = params(&w.effect);
        let material = materials.add(WaveMaterial { params: p, normal });
        commands.spawn((
            WaveArc {
                serial: w.serial,
                material: material.clone(),
            },
            Mesh3d(mesh),
            MeshMaterial3d(material),
            transform,
            Visibility::default(),
            bevy::light::NotShadowCaster,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_matches_the_native_buffers() {
        let mesh = arc_mesh();
        assert_eq!(mesh.count_vertices(), 32);
        assert_eq!(mesh.indices().unwrap().len(), 126);
    }

    #[test]
    fn create_world_backward_is_minus_forward() {
        let m = world(Vec3::new(1.0, 2.0, 3.0), Vec3::X, 2.0);
        assert!(m.z_axis.truncate().abs_diff_eq(Vec3::NEG_X * 2.0, 1e-6));
        assert!(m.y_axis.truncate().abs_diff_eq(Vec3::Y, 1e-6));
        // Right = Up × Backward = Y × (−X) = +Z.
        assert!(m.x_axis.truncate().abs_diff_eq(Vec3::Z, 1e-6));
        assert_eq!(m.w_axis, Vec4::new(1.0, 2.0, 3.0, 1.0));
    }
}
