//! GUIBasicEffect 2D material; CPU placement supplies native screen transforms.
use bevy::{
    asset::embedded_asset,
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dPlugin},
};
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct GuiBasicMaterial {
    #[uniform(0)]
    pub color: Vec4,
    #[uniform(0)]
    pub overlay_tint: Vec4,
    #[uniform(0)]
    pub uv_transform: Vec4,
    #[uniform(0)]
    pub settings: Vec4,
    #[uniform(0)]
    pub flags: UVec4,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub overlay: Handle<Image>,
}
impl GuiBasicMaterial {
    pub fn textured(texture: Handle<Image>, color: Vec4) -> Self {
        Self {
            overlay: texture.clone(),
            texture,
            color,
            overlay_tint: Vec4::ZERO,
            uv_transform: Vec4::new(1., 1., 0., 0.),
            settings: Vec4::new(1., 0., 0., 0.),
            flags: UVec4::new(1, 0, 0, 1),
        }
    }
}
impl Material2d for GuiBasicMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/gui_basic.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}
pub struct GuiBasicPlugin;
impl Plugin for GuiBasicPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "gui_basic.wgsl");
        app.add_plugins(Material2dPlugin::<GuiBasicMaterial>::default());
    }
}
