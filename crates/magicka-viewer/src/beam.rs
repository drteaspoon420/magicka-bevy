//! Railgun beams: ArcaneEffect's Beam technique. Vertices come from the CPU
//! port of its vertex shader (magicka_runtime::beam_mesh); this material is
//! the pixel stage with the pass's SrcAlpha/One blend (alpha is 1).
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
pub struct BeamMaterial {
    #[uniform(0)]
    pub color_center: Vec4,
    #[uniform(1)]
    pub color_edge: Vec4,
    #[texture(2)]
    #[sampler(3)]
    pub texture: Handle<Image>,
}
impl Material for BeamMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/beam.wgsl".into()
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
        // Pass1: AlphaBlendEnable, SrcBlend SrcAlpha, DestBlend One,
        // ZWriteEnable 0; Scene.DrawAdditive's depth test LessEqual
        // (GreaterEqual under Bevy's reverse-Z). The strip is drawn without
        // culling (its camera-facing winding is not re-derived here).
        let blend = BlendComponent {
            src_factor: BlendFactor::SrcAlpha,
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
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}
