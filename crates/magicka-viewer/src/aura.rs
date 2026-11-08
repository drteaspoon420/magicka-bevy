//! Character.HaloAuraRenderData: original halo primitive, hip billboard and
//! additive tint. Ray primitives are assigned but never drawn by native Draw.
use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, CompareFunction,
        RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct HaloMaterial {
    #[uniform(0)]
    pub color: Vec4,
    #[uniform(1)]
    pub flags: UVec4,
    #[texture(2)]
    #[sampler(3)]
    pub texture: Option<Handle<Image>>,
    #[uniform(4)]
    pub depth: Vec4,
}
impl Material for HaloMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://magicka_viewer/aura.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/additive.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
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
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let blend = BlendComponent {
            src_factor: BlendFactor::OneMinusDst,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        if let Some(fragment) = &mut d.fragment {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: blend,
                    alpha: blend,
                });
            }
        }
        if let Some(depth) = &mut d.depth_stencil {
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}
#[derive(Resource)]
pub struct HaloAsset(pub Handle<bevy::gltf::Gltf>);
#[derive(Component)]
pub struct HaloPass(pub u16);
pub struct AuraPlugin;
impl Plugin for AuraPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "aura.wgsl");
        app.add_plugins(MaterialPlugin::<HaloMaterial>::default())
            .add_systems(Startup, load);
    }
}
fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(HaloAsset(server.load("gltf/Models/Effects/armor_aura.glb")));
}
pub fn billboard(position: Vec3, camera: Vec3) -> Transform {
    let forward = (camera - position).normalize();
    let right = Vec3::Y.cross(forward).normalize();
    let up = forward.cross(right);
    // XNA Matrix.Forward stores the negative third row.
    Transform::from_matrix(Mat4::from_cols(
        right.extend(0.0),
        up.extend(0.0),
        (-forward).extend(0.0),
        position.extend(1.0),
    ))
}
