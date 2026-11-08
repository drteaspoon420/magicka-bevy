//! Level meshes whose MeshSettings are (visible = false, castShadows = true):
//! DeferredRenderData draws them only into shadow maps (e.g. the wc_s3
//! opening's overhead planks, `board_anims`). Opaque, so the shadow pass is
//! depth-only; the main-pass fragment discards and no prepass depth is
//! written (an invisible caster must not occlude or outline anything).
use bevy::{prelude::*, render::render_resource::AsBindGroup, shader::ShaderRef};

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug, Default)]
pub struct ShadowOnlyMaterial {}

impl Material for ShadowOnlyMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/shadow_only.wgsl".into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        true
    }
}
