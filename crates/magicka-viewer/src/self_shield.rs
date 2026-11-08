//! Pure Character.SelfShield: additive, bloated skinned-body projection.
//! See research/shield-self.md and shield-self/shader.asm.txt.
use bevy::{
    asset::embedded_asset,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, CompareFunction,
        RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct ShieldParams {
    pub projection0: Mat4,
    pub projection1: Mat4,
    pub projection2: Mat4,
    pub color: Vec4,
    pub offset0: Vec4,
    pub offset1: Vec4,
    pub offset2: Vec4,
}
impl ShieldParams {
    pub fn from_state(
        position: Vec3,
        direction: Vec3,
        shield: &magicka_sim::character_health::SelfShield,
    ) -> Self {
        let projection0 = Mat4::look_at_rh(position, position + direction, Vec3::Y);
        let rotation = Mat4::from_rotation_y(std::f32::consts::TAU / 3.0);
        let projection1 = rotation * projection0;
        let offset = |i: usize| {
            Vec4::new(
                shield.noise_offsets[i][0],
                shield.noise_offsets[i][1],
                0.0,
                0.0,
            )
        };
        Self {
            projection0,
            projection1,
            projection2: rotation * projection1,
            color: Vec3::from_array(magicka_sim::shield_entity::SHIELD_COLOR)
                .extend(4.0 + (0.333 - shield.time_since_damage).max(0.0) * 60.0),
            offset0: offset(0),
            offset1: offset(1),
            offset2: offset(2),
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct SelfShieldMaterial {
    #[uniform(0)]
    pub params: ShieldParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
}
impl Material for SelfShieldMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://magicka_viewer/self_shield.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
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
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // ShieldSkinRenderData disables culling. Scene.DrawAdditive uses
        // InverseDestinationColor/One, conventional LessEqual, no depth write.
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
            depth.depth_write_enabled = Some(false);
        }
        let blend = BlendComponent {
            src_factor: BlendFactor::OneMinusDst,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        if let Some(fragment) = &mut descriptor.fragment {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: blend,
                    alpha: blend,
                });
            }
        }
        Ok(())
    }
}

#[derive(Component)]
pub struct ShieldSource;
#[derive(Component)]
pub struct ShieldPass(pub u16);
#[derive(Resource)]
pub struct ShieldTexture(pub Handle<Image>);
/// The two original combined_armor primitives; their joints use the caster's
/// own skin matrices, as BarrierSkinRenderData shares mRenderData.mSkeleton.
#[derive(Resource)]
pub struct SolidShieldAssets(pub Handle<bevy::gltf::Gltf>);
#[derive(Component)]
pub struct SolidShieldSource;
#[derive(Component)]
pub struct SolidShieldPass {
    pub owner: u16,
    pub ice: bool,
}
pub fn solid_overlay(magnitude: f32) -> f32 {
    ((magnitude - 1.0) * 0.333333).clamp(0.0, 1.0)
}
pub struct SelfShieldPlugin;
impl Plugin for SelfShieldPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "self_shield.wgsl");
        app.add_plugins(MaterialPlugin::<SelfShieldMaterial>::default())
            .add_systems(Startup, load_texture);
    }
}
fn load_texture(mut commands: Commands, server: Res<AssetServer>) {
    let texture = server
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| {
            settings.is_srgb = false;
            settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                ..ImageSamplerDescriptor::linear()
            });
        })
        .load("textures/EffectTextures/shield.png");
    commands.insert_resource(ShieldTexture(texture));
    commands.insert_resource(SolidShieldAssets(
        server.load("gltf/Models/Effects/combined_armor.glb"),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projections_follow_owner_and_damage_flash() {
        let mut shield = magicka_sim::character_health::SelfShield {
            time_since_damage: 1.0,
            noise_offsets: [[0.1, 0.2]; 3],
            ..default()
        };
        let pos = Vec3::new(7.0, 2.0, -4.0);
        let a = ShieldParams::from_state(pos, Vec3::NEG_Z, &shield);
        for projection in [a.projection0, a.projection1, a.projection2] {
            assert!((projection * pos.extend(1.0)).truncate().length() < 1e-5);
        }
        assert_eq!(a.color, Vec4::new(2.0, 1.5, 1.0, 4.0));
        assert_eq!(a.offset0, Vec4::new(0.1, 0.2, 0.0, 0.0));
        shield.time_since_damage = 0.0;
        let b = ShieldParams::from_state(pos, Vec3::X, &shield);
        assert!((b.color.w - 23.98).abs() < 1e-4);
        assert!((b.projection0.transform_vector3(Vec3::X) - Vec3::NEG_Z).length() < 1e-5);
    }
}
