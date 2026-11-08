//! Magick character visuals: the invisibility distortion pass
//! (Character.NormalDistortionRenderData + InvisibilityEffect) and Haste
//! afterimages (Avatar.AfterImageRenderData); research/invisibility-visual.md
//! and research/afterimages.md.
use bevy::{
    asset::embedded_asset,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

/// InvisibilityEffect pixel shader: the scene colour behind the character,
/// offset along its clip-space normal by Distortion·0.02 scaled by the
/// depth gap (saturated over 2 units).
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct InvisibilityMaterial {
    /// x: Distortion (-0.4).
    #[uniform(0)]
    pub params: Vec4,
}
impl Material for InvisibilityMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/invisibility.wgsl".into()
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
}

/// AfterImageRenderData.Draw: additive (SourceAlpha, One), Colorize with
/// Character.ColdColor (diffuse luminance × ColdColor), per-ghost alpha.
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct AfterImageMaterial {
    /// x: alpha.
    #[uniform(0)]
    pub params: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
}
impl Material for AfterImageMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/afterimage.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
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
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// VortexEntity.RenderData post effect: the scene seen through a sphere of
/// the vortex radius, pulled along the screen-space normal
/// (research/vortex.md, VortexEffect disassembly).
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct VortexMaterial {
    /// x: Distortion, y: DistortionPower.
    #[uniform(0)]
    pub params: Vec4,
}
impl Material for VortexMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/vortex.wgsl".into()
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
}

/// The character body primitive a pass mirrors (handle of its owner).
#[derive(Component)]
pub struct BodySource(pub u16);
/// The invisibility twin of a body primitive.
#[derive(Component)]
pub struct InvisibilityPass(pub u16);
/// A mesh hidden because its owner is invisible (restored afterwards).
#[derive(Component)]
pub struct HiddenByInvisibility;
/// Afterimage ghost `n` (1..=4) of the local avatar's body.
#[derive(Component)]
pub struct AfterImagePass(pub usize);

pub struct MagickVisualsPlugin;
impl Plugin for MagickVisualsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "invisibility.wgsl");
        embedded_asset!(app, "afterimage.wgsl");
        embedded_asset!(app, "vortex.wgsl");
        app.add_plugins((
            MaterialPlugin::<InvisibilityMaterial>::default(),
            MaterialPlugin::<AfterImageMaterial>::default(),
            MaterialPlugin::<VortexMaterial>::default(),
        ));
    }
}
