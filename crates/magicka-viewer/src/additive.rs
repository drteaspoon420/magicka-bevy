//! Original AdditiveEffect geometry, used by static castle window rays.
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
pub struct AdditiveMaterial {
    #[uniform(0)]
    pub color: Vec4,
    #[uniform(1)]
    pub flags: UVec4,
    #[texture(2)]
    #[sampler(3)]
    pub texture: Option<Handle<Image>>,
}
impl Material for AdditiveMaterial {
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
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Scene.DrawAdditive: src InverseDestinationColor, dst One,
        // depth LessEqual, no depth write. Modern framebuffer color space
        // differs from native D3D9; full blend/lighting parity remains open.
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
        if let Some(depth) = &mut descriptor.depth_stencil {
            // Native LessEqual on conventional depth; Bevy's projection is
            // reverse-Z, where the same test is GreaterEqual.
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}
