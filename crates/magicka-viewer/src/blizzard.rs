//! Blizzard's Linear FogEffect, before FilterEffect (research/blizzard.md).
use bevy::{
    asset::{embedded_asset, RenderAssetUsages},
    core_pipeline::{
        schedule::{Core3d, Core3dSystems},
        FullscreenShader,
    },
    prelude::*,
    render::{
        extract_component::{
            ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
            UniformComponentPlugin,
        },
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{
            binding_types::{sampler, texture_2d, uniform_buffer},
            *,
        },
        renderer::{RenderContext, ViewQuery},
        texture::GpuImage,
        view::ViewTarget,
        RenderApp, RenderStartup,
    },
};
use std::path::{Path, PathBuf};

#[derive(Component, Clone, Copy, Debug, ExtractComponent, ShaderType)]
pub struct BlizzardPost {
    pub offsets: Vec4,
    /// Alpha, FogStart, FogEnd, unused.
    pub control: Vec4,
    pub view_from_clip: Mat4,
}
impl Default for BlizzardPost {
    fn default() -> Self {
        Self {
            offsets: Vec4::ZERO,
            control: Vec4::new(0.0, 180.0, 240.0, 0.0),
            view_from_clip: Mat4::IDENTITY,
        }
    }
}

#[derive(Resource, Clone, ExtractResource)]
struct BlizzardTexture(Handle<Image>);

#[derive(Resource)]
struct Pipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    id: CachedRenderPipelineId,
}

pub struct BlizzardPlugin(pub PathBuf);
impl Plugin for BlizzardPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "blizzard.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<BlizzardPost>::default(),
            UniformComponentPlugin::<BlizzardPost>::default(),
            ExtractResourcePlugin::<BlizzardTexture>::default(),
        ));
        let content = self.0.clone();
        app.add_systems(
            Startup,
            move |mut commands: Commands, mut images: ResMut<Assets<Image>>| match load_texture(
                &content,
                &mut images,
            ) {
                Ok(texture) => {
                    commands.insert_resource(BlizzardTexture(texture));
                }
                Err(error) => error!("Blizzard texture: {error}"),
            },
        );
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .add_systems(RenderStartup, init_pipeline)
                .add_systems(
                    Core3d,
                    draw.in_set(Core3dSystems::PostProcess)
                        .before(crate::post::FilterSet),
                );
        }
    }
}

fn load_texture(content: &Path, images: &mut Assets<Image>) -> Result<Handle<Image>, String> {
    let magicka_content::assets::Asset::Texture2D(t) =
        crate::text_box_visual::read_asset(&content.join("EffectTextures/Blizzard.xnb"))?
    else {
        return Err("expected Texture2D".into());
    };
    let mip = t.mips.first().ok_or("missing mip")?;
    let rgba =
        magicka_content::texture::decode_rgba(t.format, t.width as usize, t.height as usize, mip)
            .ok_or("unsupported Blizzard texture format")?;
    // Fog shader samples the original bytes as raw values, not sRGB.
    let image = Image::new(
        Extent3d {
            width: t.width as u32,
            height: t.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    Ok(images.add(image))
}

fn init_pipeline(
    mut commands: Commands,
    device: Res<bevy::render::renderer::RenderDevice>,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "blizzard_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: false }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<BlizzardPost>(true),
            ),
        ),
    );
    let sampler = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("blizzard_linear_fog".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: assets.load("embedded://magicka_viewer/blizzard.wgsl"),
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8UnormSrgb,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(Pipeline {
        layout,
        sampler,
        id,
    });
}

#[allow(clippy::too_many_arguments)]
fn draw(
    view: ViewQuery<(
        &ViewTarget,
        &DynamicUniformIndex<BlizzardPost>,
        &BlizzardPost,
    )>,
    pipeline: Option<Res<Pipeline>>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<BlizzardPost>>,
    texture: Option<Res<BlizzardTexture>>,
    depth: Option<Res<crate::effect_depth::EffectDepth>>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, index, params) = view.into_inner();
    if params.control.x <= 0.0 || target.main_texture_format() != TextureFormat::Rgba8UnormSrgb {
        return;
    }
    let (Some(p), Some(texture), Some(depth), Some(uniform)) =
        (pipeline, texture, depth, uniforms.uniforms().binding())
    else {
        return;
    };
    let (Some(render_pipeline), Some(noise), Some(depth_image)) = (
        cache.get_render_pipeline(p.id),
        images.get(&texture.0),
        images.get(&depth.image),
    ) else {
        return;
    };
    let post = target.post_process_write();
    let group = ctx.render_device().create_bind_group(
        "blizzard_bind_group",
        &cache.get_bind_group_layout(&p.layout),
        &BindGroupEntries::sequential((
            post.source,
            &noise.texture_view,
            &depth_image.texture_view,
            &p.sampler,
            uniform,
        )),
    );
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("blizzard_linear_fog"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(render_pipeline);
    pass.set_bind_group(0, &group, &[index.index()]);
    pass.draw(0..3, 0..1);
}
