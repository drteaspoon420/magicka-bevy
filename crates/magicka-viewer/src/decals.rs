//! HardwareInstancedProjectionEffect shader 3, with Bevy's depth prepass.
use bevy::{
    asset::{embedded_asset, RenderAssetUsages},
    camera::visibility::NoFrustumCulling,
    image::ImageLoaderSettings,
    mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, CompareFunction, Face, RenderPipelineDescriptor, SpecializedMeshPipelineError,
        VertexFormat,
    },
    shader::ShaderRef,
};
use magicka_runtime::decals::Projection;

const INV_X: MeshVertexAttribute =
    MeshVertexAttribute::new("DecalInverseX", 0x6d64_0001, VertexFormat::Float32x4);
const INV_Y: MeshVertexAttribute =
    MeshVertexAttribute::new("DecalInverseY", 0x6d64_0002, VertexFormat::Float32x4);
const INV_Z: MeshVertexAttribute =
    MeshVertexAttribute::new("DecalInverseZ", 0x6d64_0003, VertexFormat::Float32x4);
const ARGS: MeshVertexAttribute =
    MeshVertexAttribute::new("DecalArgs", 0x6d64_0004, VertexFormat::Float32x3);

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct DecalMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub atlas: Handle<Image>,
    #[storage(90, read_only)]
    pub lights: Handle<bevy::render::storage::ShaderBuffer>,
}
impl Material for DecalMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://magicka_viewer/decals.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        // Transparent3d sorts ascending: ground projections precede all
        // finite-distance particles, regardless of either mesh origin.
        f32::NEG_INFINITY
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _: &MaterialPipeline,
        d: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        d.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            INV_X.at_shader_location(1),
            INV_Y.at_shader_location(2),
            INV_Z.at_shader_location(3),
            ARGS.at_shader_location(4),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(5),
        ])?];
        if let Some(depth) = &mut d.depth_stencil {
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
            depth.depth_write_enabled = Some(false);
        }
        // Draw exit faces so projection also works with the camera inside.
        d.primitive.cull_mode = Some(Face::Front);
        Ok(())
    }
}
#[derive(Resource, Default)]
pub struct DecalFrame(pub Vec<Projection>);
#[derive(Resource)]
struct Renderer(Handle<Mesh>);
pub struct DecalPlugin;
impl Plugin for DecalPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "decals.wgsl");
        app.add_plugins(MaterialPlugin::<DecalMaterial>::default())
            .init_resource::<DecalFrame>()
            .add_systems(Startup, setup)
            .add_systems(PostUpdate, upload);
    }
}
fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DecalMaterial>>,
    lights: Res<crate::lighting::MagickaLights>,
) {
    let atlas = assets
        .load_builder()
        .with_settings(|s: &mut ImageLoaderSettings| {
            s.is_srgb = false;
        })
        .load("textures/EffectTextures/decals.png");
    let mesh = meshes.add(build_mesh(&[]));
    commands.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(materials.add(DecalMaterial {
            atlas,
            lights: lights.buffer.clone(),
        })),
        Transform::IDENTITY,
        NoFrustumCulling,
        crate::lighting::NoDepthPrepass,
    ));
    commands.insert_resource(Renderer(mesh));
}
fn upload(frame: Res<DecalFrame>, renderer: Res<Renderer>, mut meshes: ResMut<Assets<Mesh>>) {
    if let Some(mut mesh) = meshes.get_mut(&renderer.0) {
        *mesh = build_mesh(&frame.0);
    }
}
pub fn build_mesh(list: &[Projection]) -> Mesh {
    let mut positions = vec![];
    let mut ix = vec![];
    let mut iy = vec![];
    let mut iz = vec![];
    let mut args = vec![];
    let mut colors = vec![];
    let mut indices = vec![];
    // Expired holes remain in the simulation snapshot, but produce no pixels.
    for p in list
        .iter()
        .filter(|p| p.args[2] > 0.0 && p.inverse.iter().all(|x| x.is_finite()))
    {
        let base = positions.len() as u32;
        let transform = Mat4::from_cols_array(&p.transform);
        for j in 0..8 {
            let v = Vec3::new(
                if j % 2 < 1 { -0.5 } else { 0.5 },
                if j % 4 < 2 { -0.5 } else { 0.5 },
                if j % 8 < 4 { -0.5 } else { 0.5 },
            );
            positions.push(transform.transform_point3(v).to_array());
            let column = |c| {
                [
                    p.inverse[c],
                    p.inverse[c + 4],
                    p.inverse[c + 8],
                    p.inverse[c + 12],
                ]
            };
            ix.push(column(0));
            iy.push(column(1));
            iz.push(column(2));
            args.push(p.args);
            colors.push(p.color);
        }
        indices.extend(
            [
                0, 2, 1, 1, 2, 3, 1, 0, 4, 4, 5, 1, 0, 2, 4, 4, 2, 6, 4, 6, 5, 5, 6, 7, 2, 3, 6, 6,
                3, 7, 3, 1, 5, 5, 7, 3,
            ]
            .map(|i| base + i),
        );
    }
    if positions.is_empty() {
        positions.push([0.0; 3]);
        ix.push([0.0; 4]);
        iy.push([0.0; 4]);
        iz.push([0.0; 4]);
        args.push([0.0; 3]);
        colors.push([0.0; 4]);
        indices.extend([0, 0, 0]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(INV_X, ix)
    .with_inserted_attribute(INV_Y, iy)
    .with_inserted_attribute(INV_Z, iz)
    .with_inserted_attribute(ARGS, args)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}
