//! Character.LightningZapRenderData (research/zap.md): while a character's
//! zap timer runs, its body renders black and SkinnedModelSkeletonEffect
//! draws the generated skeleton as white line strips inside the body's
//! stencil. Bevy has no stencil here: lines draw without a depth test and
//! discard where prepass geometry is nearer than the line by more than the
//! character radius (an INFERRED stand-in for "inside the visible body").
use bevy::{
    asset::{embedded_asset, RenderAssetUsages},
    mesh::{MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct ZapLineMaterial {
    /// x: occlusion margin (character radius).
    #[uniform(0)]
    pub params: Vec4,
}
impl Material for ZapLineMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/zap.wgsl".into()
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
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_compare = Some(CompareFunction::Always);
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// The zap skeleton lines of one character.
#[derive(Component)]
pub struct ZapLines(pub u16);

pub struct ZapPlugin;
impl Plugin for ZapPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "zap.wgsl");
        app.add_plugins(MaterialPlugin::<ZapLineMaterial>::default());
    }
}

/// A LineList mesh of world-space segments.
pub fn line_mesh(lines: &[[[f32; 3]; 2]]) -> Mesh {
    let positions: Vec<[f32; 3]> = lines.iter().flatten().copied().collect();
    Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_mesh_has_two_vertices_per_segment() {
        let mesh = line_mesh(&[[[0.0; 3], [1.0, 0.0, 0.0]], [[0.0; 3], [0.0, 1.0, 0.0]]]);
        assert_eq!(mesh.count_vertices(), 4);
        assert_eq!(mesh.primitive_topology(), PrimitiveTopology::LineList);
    }
}
