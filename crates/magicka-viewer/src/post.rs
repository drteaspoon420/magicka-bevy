//! PolygonHead RenderManager post-processing for play mode
//! (research/post-processing.md).
//!
//! The original renders the scene into a buffer that holds `1 − 2^−x`,
//! applies FilterEffect (brightness/contrast/saturation) into the GUI buffer,
//! draws the GUI, then BloomEffect: a 16×16 thresholded downsample, two
//! 13-tap Gaussian blurs at sixteenth size, and a Combine that *decodes* the
//! GUI buffer to `x` and adds the bloom. The filter runs on the 3D camera;
//! the bloom chain on the overlay camera, after the HUD, as in the original.
use bevy::{
    asset::embedded_asset,
    core_pipeline::{
        schedule::{Core2d, Core2dSystems, Core3d, Core3dSystems},
        FullscreenShader,
    },
    prelude::*,
    render::{
        camera::ExtractedCamera,
        extract_component::{
            ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
            UniformComponentPlugin,
        },
        render_resource::{
            binding_types::{sampler, texture_2d, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice, ViewQuery},
        texture::{CachedTexture, TextureCache},
        view::ViewTarget,
        Render, RenderApp, RenderStartup, RenderSystems,
    },
};

/// GameScene Filter/Bloom settings (defaults: 1, 1, 1; 0.8, 1, 2.5).
#[derive(Component, Clone, Copy, Debug, PartialEq, ExtractComponent, ShaderType)]
pub struct MagickaPost {
    /// Brightness, Contrast, Saturation, Flash level.
    pub filter: Vec4,
    /// Threshold, Multiplier, BlurSigma, unused.
    pub bloom: Vec4,
}
impl Default for MagickaPost {
    fn default() -> Self {
        Self {
            filter: Vec4::new(1.0, 1.0, 1.0, 0.0),
            bloom: Vec4::new(0.8, 1.0, 2.5, 0.0),
        }
    }
}

/// FilterEffect runs on this camera (the 3D scene camera).
#[derive(Component, Clone, Copy, Default, ExtractComponent)]
pub struct MagickaFilterPass;

/// The bloom chain and final decode run on this camera (the overlay camera
/// that draws last, after the HUD).
#[derive(Component, Clone, Copy, Default, ExtractComponent)]
pub struct MagickaBloomPass;

/// RenderManager's two sixteenth-size Color targets.
#[derive(Component)]
struct SixteenthBuffers {
    a: CachedTexture,
    b: CachedTexture,
}

#[derive(Resource)]
struct PostPipelines {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    filter: CachedRenderPipelineId,
    downsample: CachedRenderPipelineId,
    blur_h: CachedRenderPipelineId,
    blur_v: CachedRenderPipelineId,
    combine: CachedRenderPipelineId,
}

/// The original's eight-bit Color format for the sixteenth buffers.
const SIXTEENTH_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
/// Non-HDR main textures.
const MAIN_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;

pub struct PostPlugin;
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct FilterSet;
/// The overlay camera's bloom (it swaps the view's main textures).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct BloomSet;
impl Plugin for PostPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "post.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<MagickaPost>::default(),
            ExtractComponentPlugin::<MagickaFilterPass>::default(),
            ExtractComponentPlugin::<MagickaBloomPass>::default(),
            UniformComponentPlugin::<MagickaPost>::default(),
        ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(
                Render,
                prepare_buffers.in_set(RenderSystems::PrepareResources),
            )
            .add_systems(
                Core3d,
                filter.in_set(Core3dSystems::PostProcess).in_set(FilterSet),
            )
            .add_systems(
                Core2d,
                bloom.in_set(Core2dSystems::PostProcess).in_set(BloomSet),
            );
    }
}

fn init_pipelines(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "magicka_post_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<MagickaPost>(true),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let shader = assets.load("embedded://magicka_viewer/post.wgsl");
    let queue = |entry: &'static str, format: TextureFormat| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("magicka_post_{entry}").into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        })
    };
    let pipelines = PostPipelines {
        filter: queue("filter_pass", MAIN_FORMAT),
        downsample: queue("downsample", SIXTEENTH_FORMAT),
        blur_h: queue("blur_h", SIXTEENTH_FORMAT),
        blur_v: queue("blur_v", SIXTEENTH_FORMAT),
        combine: queue("combine", MAIN_FORMAT),
        layout,
        sampler,
    };
    commands.insert_resource(pipelines);
}

/// Sixteenth size, truncated as `screenSize / 16`.
pub fn sixteenth(size: UVec2) -> UVec2 {
    (size / 16).max(UVec2::ONE)
}

fn prepare_buffers(
    mut commands: Commands,
    mut textures: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ExtractedCamera), With<MagickaBloomPass>>,
) {
    for (entity, camera) in &views {
        let Some(size) = camera.physical_viewport_size else {
            continue;
        };
        let size = sixteenth(size);
        let mut make = |label: &'static str| {
            textures.get(
                &device,
                TextureDescriptor {
                    label: Some(label),
                    size: Extent3d {
                        width: size.x,
                        height: size.y,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: SIXTEENTH_FORMAT,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        let buffers = SixteenthBuffers {
            a: make("magicka_sixteenth_a"),
            b: make("magicka_sixteenth_b"),
        };
        commands.entity(entity).insert(buffers);
    }
}

#[allow(clippy::too_many_arguments)]
fn pass(
    ctx: &mut RenderContext,
    cache: &PipelineCache,
    pipelines: &PostPipelines,
    pipeline: CachedRenderPipelineId,
    t0: &TextureView,
    t1: &TextureView,
    uniform: BindingResource,
    index: u32,
    target: &TextureView,
) -> bool {
    let Some(pipeline) = cache.get_render_pipeline(pipeline) else {
        return false;
    };
    let bind_group = ctx.render_device().create_bind_group(
        "magicka_post_bind_group",
        &cache.get_bind_group_layout(&pipelines.layout),
        &BindGroupEntries::sequential((t0, t1, &pipelines.sampler, uniform)),
    );
    let mut render_pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("magicka_post_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    render_pass.set_pipeline(pipeline);
    render_pass.set_bind_group(0, &bind_group, &[index]);
    render_pass.draw(0..3, 0..1);
    true
}

fn filter(
    view: ViewQuery<(&ViewTarget, &DynamicUniformIndex<MagickaPost>), With<MagickaFilterPass>>,
    pipelines: Option<Res<PostPipelines>>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<MagickaPost>>,
    mut ctx: RenderContext,
) {
    let (target, index) = view.into_inner();
    let (Some(pipelines), Some(uniform)) = (pipelines, uniforms.uniforms().binding()) else {
        return;
    };
    if target.main_texture_format() != MAIN_FORMAT
        || cache.get_render_pipeline(pipelines.filter).is_none()
    {
        return;
    }
    let post = target.post_process_write();
    pass(
        &mut ctx,
        &cache,
        &pipelines,
        pipelines.filter,
        post.source,
        post.source,
        uniform,
        index.index(),
        post.destination,
    );
}

fn bloom(
    view: ViewQuery<
        (
            &ViewTarget,
            &SixteenthBuffers,
            &DynamicUniformIndex<MagickaPost>,
        ),
        With<MagickaBloomPass>,
    >,
    pipelines: Option<Res<PostPipelines>>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<MagickaPost>>,
    mut ctx: RenderContext,
) {
    let (target, buffers, index) = view.into_inner();
    let (Some(pipelines), Some(uniform)) = (pipelines, uniforms.uniforms().binding()) else {
        return;
    };
    let ready = [
        pipelines.downsample,
        pipelines.blur_h,
        pipelines.blur_v,
        pipelines.combine,
    ]
    .iter()
    .all(|&id| cache.get_render_pipeline(id).is_some());
    if target.main_texture_format() != MAIN_FORMAT || !ready {
        return;
    }
    let i = index.index();
    let (a, b) = (&buffers.a.default_view, &buffers.b.default_view);
    let post = target.post_process_write();
    let p = &pipelines;
    pass(
        &mut ctx,
        &cache,
        p,
        p.downsample,
        post.source,
        post.source,
        uniform.clone(),
        i,
        a,
    );
    pass(&mut ctx, &cache, p, p.blur_h, a, a, uniform.clone(), i, b);
    pass(&mut ctx, &cache, p, p.blur_v, b, b, uniform.clone(), i, a);
    pass(
        &mut ctx,
        &cache,
        p,
        p.combine,
        post.source,
        a,
        uniform,
        i,
        post.destination,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteenth_buffers_truncate_like_the_original() {
        assert_eq!(sixteenth(UVec2::new(1280, 720)), UVec2::new(80, 45));
        assert_eq!(sixteenth(UVec2::new(1366, 768)), UVec2::new(85, 48));
        assert_eq!(sixteenth(UVec2::new(8, 8)), UVec2::ONE);
    }

    #[test]
    fn defaults_match_render_manager() {
        let d = MagickaPost::default();
        assert_eq!(d.filter.truncate(), Vec3::ONE);
        assert_eq!(d.bloom.truncate(), Vec3::new(0.8, 1.0, 2.5));
    }
}
