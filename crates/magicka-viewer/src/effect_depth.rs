//! Opaque scene depth for soft particles. The outline's prepass deliberately
//! excludes the player; effects need the depth after *all* opaque meshes.
use bevy::{
    asset::embedded_asset,
    camera::CameraUpdateSystems,
    core_pipeline::{
        core_3d::{main_opaque_pass_3d, main_transparent_pass_3d},
        schedule::{Core3d, Core3dSystems},
        FullscreenShader,
    },
    prelude::*,
    render::{
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderContext, ViewQuery},
        texture::GpuImage,
        view::ViewDepthTexture,
        RenderApp, RenderStartup,
    },
};

/// Only the gameplay camera produces this snapshot, before transparent effects.
#[derive(Component, Clone, Copy, ExtractComponent)]
pub struct EffectDepthCamera;

#[derive(Resource, Clone, ExtractResource)]
pub struct EffectDepth {
    pub image: Handle<Image>,
}

pub(crate) fn depth_image(size: UVec2) -> Image {
    let mut image = Image::new_target_texture(size.x, size.y, TextureFormat::R32Float, None);
    image.texture_descriptor.usage =
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
    image
}

impl FromWorld for EffectDepth {
    fn from_world(world: &mut World) -> Self {
        Self {
            image: world
                .resource_mut::<Assets<Image>>()
                .add(depth_image(UVec2::ONE)),
        }
    }
}

#[derive(Resource)]
pub(crate) struct Pipelines {
    layouts: [BindGroupLayoutDescriptor; 2],
    pipelines: [CachedRenderPipelineId; 2],
}

pub struct EffectDepthPlugin;
impl Plugin for EffectDepthPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "effect_depth.wgsl");
        app.init_resource::<EffectDepth>()
            .add_plugins((
                ExtractResourcePlugin::<EffectDepth>::default(),
                ExtractComponentPlugin::<EffectDepthCamera>::default(),
            ))
            .add_systems(PostUpdate, resize.after(CameraUpdateSystems));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(
                Core3d,
                snapshot
                    .in_set(Core3dSystems::MainPass)
                    .after(main_opaque_pass_3d)
                    .before(main_transparent_pass_3d),
            );
    }
}

fn resize(
    mut cameras: Query<(&Camera, &mut Camera3d), With<EffectDepthCamera>>,
    depth: Res<EffectDepth>,
    mut images: ResMut<Assets<Image>>,
) {
    let Ok((camera, mut camera_3d)) = cameras.single_mut() else {
        return;
    };
    let usage = TextureUsages::from(camera_3d.depth_texture_usages);
    if !usage.contains(TextureUsages::TEXTURE_BINDING) {
        camera_3d.depth_texture_usages = (usage | TextureUsages::TEXTURE_BINDING).into();
    }
    let Some(size) = camera
        .physical_target_size()
        .filter(|size| size.x > 0 && size.y > 0)
    else {
        return;
    };
    if let Some(mut image) = images.get_mut(&depth.image) {
        if image.width() != size.x || image.height() != size.y {
            *image = depth_image(size);
        }
    }
}

fn init_pipelines(
    mut commands: Commands,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    let shader = assets.load("embedded://magicka_viewer/effect_depth.wgsl");
    let layouts = [false, true].map(|multisampled| {
        BindGroupLayoutDescriptor::new(
            "effect_depth_layout",
            &BindGroupLayoutEntries::single(
                ShaderStages::FRAGMENT,
                BindingType::Texture {
                    sample_type: TextureSampleType::Depth,
                    view_dimension: TextureViewDimension::D2,
                    multisampled,
                },
            ),
        )
    });
    let pipelines = [0, 1].map(|i| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("effect_depth_snapshot".into()),
            layout: vec![layouts[i].clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                shader_defs: if i == 1 {
                    vec!["MULTISAMPLED".into()]
                } else {
                    vec![]
                },
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::R32Float,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            ..default()
        })
    });
    commands.insert_resource(Pipelines { layouts, pipelines });
}

fn snapshot(
    view: ViewQuery<(&ViewDepthTexture, &Msaa), With<EffectDepthCamera>>,
    depth: Res<EffectDepth>,
    images: Res<RenderAssets<GpuImage>>,
    pipelines: Res<Pipelines>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (source, msaa) = view.into_inner();
    if let Some(target) = images.get(&depth.image) {
        copy_depth(source, msaa, target, &pipelines, &cache, &mut ctx);
    }
}

/// Copy a view's depth attachment into an `R32Float` image of the same size.
/// Also used for the cutaway's navmesh floor depth (cutaway.rs).
pub(crate) fn copy_depth(
    source: &ViewDepthTexture,
    msaa: &Msaa,
    target: &GpuImage,
    pipelines: &Pipelines,
    cache: &PipelineCache,
    ctx: &mut RenderContext,
) {
    let i = usize::from(msaa.samples() > 1);
    let Some(pipeline) = cache.get_render_pipeline(pipelines.pipelines[i]) else {
        return;
    };
    let group = ctx.render_device().create_bind_group(
        "effect_depth_bind_group",
        &cache.get_bind_group_layout(&pipelines.layouts[i]),
        &BindGroupEntries::single(source.view()),
    );
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("effect_depth_snapshot"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &target.texture_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &group, &[]);
    pass.draw(0..3, 0..1);
}
