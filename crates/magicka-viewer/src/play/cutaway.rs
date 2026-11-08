//! Play mode: the optional wall cutaway (`crate::cutaway`). Off by default and
//! only switched on from the F1 debug menu (Diagnostics → Cutaway walls).
//! While off, no helper camera or floor proxy exists and the terrain shader
//! takes its unchanged path.
use super::PlayAppExt;
use super::{
    level_parts::LevelPartNode, script_act, Act, Autopilot, InspectionCamera, LevelRoot, Play,
    PlayCamera, Step,
};
use crate::cutaway::{
    floor_triangles, height_image, proxy_mesh, CutawayBindings, CutawayDepthCamera, GpuCutaway,
    HeightGrid, LAYER,
};
use crate::terrain::{TerrainMaterial, CUTAWAY};
use bevy::{
    camera::{visibility::RenderLayers, CameraUpdateSystems, ImageRenderTarget, RenderTarget},
    core_pipeline::tonemapping::Tonemapping,
    light::NotShadowCaster,
    prelude::*,
    render::{
        render_resource::{TextureFormat, TextureUsages},
        storage::ShaderBuffer,
    },
};

/// Session-local cutaway settings; never persisted or enabled implicitly.
#[derive(Resource, Clone, Debug, PartialEq)]
pub(super) struct CutawaySettings {
    pub(super) enabled: bool,
    /// Validation view: tint occluders (red/orange) and navmesh floor depth
    /// (blue) instead of cutting.
    pub(super) debug_view: bool,
    /// Wall kept above the floor, in world units (≈ metres: the gameplay
    /// camera's ~20-unit-tall view frames characters about 2 units tall).
    pub(super) knee_height: f32,
    /// Cut-edge width in physical pixels.
    pub(super) outline_width_px: f32,
    /// Cut-edge darkening (0 none, 1 black).
    pub(super) outline_strength: f32,
    /// Minimum view-distance gap, in world units, for a wall to count as in
    /// front of the floor.
    pub(super) depth_epsilon: f32,
    /// How far outside the navmesh, in world units, a wall column still finds
    /// the floor height next to it.
    pub(super) footprint_padding: f32,
    /// Height-grid cell size in world units.
    pub(super) grid_cell: f32,
}

impl Default for CutawaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            debug_view: false,
            knee_height: 0.75,
            outline_width_px: 1.5,
            outline_strength: 0.2,
            depth_epsilon: 0.1,
            footprint_padding: 2.0,
            grid_cell: 0.5,
        }
    }
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CutawayStatus {
    #[default]
    Off,
    NoNavmesh,
    DepthNotReady,
    Active,
}

#[cfg_attr(not(feature = "debug-ui"), allow(dead_code))]
impl CutawayStatus {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::NoNavmesh => "no navmesh (rendering normally)",
            Self::DepthNotReady => "depth not ready",
            Self::Active => "active",
        }
    }
}

/// Autopilot QA (`--autopilot-script cutaway`): paired off/on captures from
/// several inspection views (plus the debug tint), ending switched off.
pub(super) const SCRIPT: &[Step] = &[
    (120, [0.0, 0.0], Act::Idle, "idle"),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 0, view: 0 },
        "off_default",
    ),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 1, view: 0 },
        "on_default",
    ),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 2, view: 0 },
        "debug_default",
    ),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 0, view: 1 },
        "off_yaw90",
    ),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 1, view: 1 },
        "on_yaw90",
    ),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 2, view: 1 },
        "debug_yaw90",
    ),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 0, view: 2 },
        "off_yaw180",
    ),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 1, view: 2 },
        "on_yaw180",
    ),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 2, view: 2 },
        "debug_yaw180",
    ),
    (20, [0.0, 0.0], Act::Cutaway { mode: 0, view: 4 }, "off_low"),
    (20, [0.0, 0.0], Act::Cutaway { mode: 1, view: 4 }, "on_low"),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 2, view: 4 },
        "debug_low",
    ),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 0, view: 5 },
        "off_low_yaw90",
    ),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 1, view: 5 },
        "on_low_yaw90",
    ),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 2, view: 5 },
        "debug_low_yaw90",
    ),
    (20, [0.0, 0.0], Act::Cutaway { mode: 0, view: 3 }, "off_top"),
    (20, [0.0, 0.0], Act::Cutaway { mode: 1, view: 3 }, "on_top"),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 2, view: 3 },
        "debug_top",
    ),
    (
        20,
        [0.0, 0.0],
        Act::Cutaway { mode: 0, view: 0 },
        "restored",
    ),
];

/// Inspection (yaw, tilt, zoom) presets for [`SCRIPT`].
const VIEWS: [(f32, f32, f32); 6] = [
    (0.0, 0.0, 0.0),
    (std::f32::consts::FRAC_PI_2, 0.0, 0.0),
    (std::f32::consts::PI, 0.0, 0.0),
    (-0.8, -0.45, 0.5),
    (0.0, 0.6, 0.0),
    (std::f32::consts::FRAC_PI_2, 0.6, 0.0),
];

pub(super) fn fixture(
    pilot: Option<Res<Autopilot>>,
    mut settings: ResMut<CutawaySettings>,
    mut inspection: ResMut<InspectionCamera>,
) {
    let Some(pilot) = pilot else {
        return;
    };
    let Some(start) = pilot.started else {
        return;
    };
    if pilot.script != SCRIPT {
        return;
    }
    if let Some(Act::Cutaway { mode, view }) = script_act(pilot.script, pilot.frame - start) {
        settings.enabled = mode > 0;
        settings.debug_view = mode == 2;
        let (yaw, tilt, zoom) = VIEWS[usize::from(view)];
        inspection.yaw = yaw;
        inspection.tilt = tilt;
        inspection.zoom = zoom;
    }
}

/// The render-only navmesh floor drawn by the helper camera.
#[derive(Component)]
struct FloorProxy;

/// Scene identity the proxy was built for.
type FloorKey = (u32, u32, std::path::PathBuf);

struct CutawayState {
    key: Option<FloorKey>,
    grid: Option<HeightGrid>,
    size: UVec2,
    target: Option<Handle<Image>>,
    /// Frames rendered since the helper camera started or resized.
    frames: u32,
    /// Buffer contents; `None` forces the next write.
    uploaded: Option<GpuCutaway>,
    /// Frames left in which to re-prepare the cuttable materials.
    rebind: u8,
}

impl Default for CutawayState {
    fn default() -> Self {
        Self {
            key: None,
            grid: None,
            size: UVec2::ZERO,
            target: None,
            frames: 0,
            // `CutawayBindings` starts with the default (disabled) contents.
            uploaded: Some(GpuCutaway::default()),
            rebind: 0,
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_plugins(crate::cutaway::CutawayRenderPlugin)
        .init_play_resource::<CutawaySettings>()
        .init_play_resource::<CutawayStatus>()
        .add_play_systems(
            PostUpdate,
            (
                tag_architecture,
                update
                    .before(CameraUpdateSystems)
                    .before(bevy::transform::TransformSystems::Propagate),
            ),
        );
}

/// Static level architecture: terrain-material meshes of the level glTF,
/// excluding animated level parts (doors, gates, carts).
fn tag_architecture(
    added: Query<
        (Entity, &MeshMaterial3d<TerrainMaterial>),
        Added<MeshMaterial3d<TerrainMaterial>>,
    >,
    parents: Query<&ChildOf>,
    levels: Query<(), With<LevelRoot>>,
    parts: Query<(), With<LevelPartNode>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    for (entity, material) in &added {
        let chain = || std::iter::once(entity).chain(parents.iter_ancestors(entity));
        if !chain().any(|e| levels.contains(e)) || chain().any(|e| parts.contains(e)) {
            continue;
        }
        if let Some(mut m) = materials.get_mut(&material.0) {
            m.params.flags.x |= CUTAWAY;
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update(
    mut commands: Commands,
    settings: Res<CutawaySettings>,
    mut status: ResMut<CutawayStatus>,
    play: Res<Play>,
    bindings: Res<CutawayBindings>,
    mut state: Local<CutawayState>,
    mut images: ResMut<Assets<Image>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut floor_materials: ResMut<Assets<StandardMaterial>>,
    mut terrain: ResMut<Assets<TerrainMaterial>>,
    main: Query<
        (&Camera, &Transform, &Projection),
        (With<PlayCamera>, Without<CutawayDepthCamera>),
    >,
    mut helpers: Query<(Entity, &mut Transform, &mut Projection), With<CutawayDepthCamera>>,
    proxies: Query<Entity, With<FloorProxy>>,
) {
    if !settings.enabled {
        for e in helpers.iter().map(|h| h.0).chain(&proxies) {
            commands.entity(e).despawn();
        }
        state.key = None;
        state.grid = None;
        state.frames = 0;
        *status = CutawayStatus::Off;
        upload(&mut state, &mut buffers, &bindings, GpuCutaway::default());
        return;
    }
    let key = (
        play.reloads,
        play.session.scene_changes,
        play.session.model_path.clone(),
    );
    if state.key.as_ref() != Some(&key) {
        for e in &proxies {
            commands.entity(e).despawn();
        }
        let triangles = if play.hide_level {
            Vec::new()
        } else {
            floor_triangles(&play.session.world.level.nav_mesh)
        };
        state.grid = HeightGrid::build(&triangles, settings.grid_cell, settings.footprint_padding);
        info!(
            "cutaway floor: {} triangles, grid {:?}",
            triangles.len(),
            state.grid.as_ref().map(|g| g.size)
        );
        let heights = height_image(state.grid.as_ref().unwrap_or(&HeightGrid::empty()));
        if let Some(mut image) = images.get_mut(&bindings.heights) {
            *image = heights;
        }
        if state.grid.is_some() {
            commands.spawn((
                FloorProxy,
                Mesh3d(meshes.add(proxy_mesh(&triangles))),
                MeshMaterial3d(floor_materials.add(StandardMaterial {
                    unlit: true,
                    cull_mode: None,
                    double_sided: true,
                    ..default()
                })),
                crate::lighting::KeepStandardMaterial,
                NotShadowCaster,
                RenderLayers::layer(LAYER),
                Transform::IDENTITY,
            ));
        }
        state.key = Some(key);
        state.rebind = 2;
    }

    let Ok((camera, transform, projection)) = main.single() else {
        return;
    };
    let size = camera
        .physical_target_size()
        .filter(|s| s.x > 0 && s.y > 0)
        .unwrap_or(state.size.max(UVec2::ONE));
    if state.size != size || state.target.is_none() {
        let mut target =
            Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
        target.texture_descriptor.usage =
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
        match &state.target {
            Some(handle) => {
                if let Some(mut image) = images.get_mut(handle) {
                    *image = target;
                }
            }
            None => state.target = Some(images.add(target)),
        }
        if let Some(mut image) = images.get_mut(&bindings.depth) {
            *image = crate::effect_depth::depth_image(size);
        }
        state.size = size;
        state.frames = 0;
        state.rebind = 2;
    }

    // Same pose and projection as the gameplay camera this frame; both
    // propagate and update after this system.
    match helpers.single_mut() {
        Ok((_, mut t, mut p)) => {
            *t = *transform;
            *p = projection.clone();
            state.frames = state.frames.saturating_add(1);
        }
        Err(_) => {
            commands.spawn((
                CutawayDepthCamera,
                Camera3d {
                    depth_texture_usages: (TextureUsages::RENDER_ATTACHMENT
                        | TextureUsages::TEXTURE_BINDING)
                        .into(),
                    ..default()
                },
                Camera {
                    order: -1,
                    clear_color: ClearColorConfig::Custom(Color::NONE),
                    ..default()
                },
                RenderTarget::Image(ImageRenderTarget {
                    handle: state.target.clone().expect("helper target"),
                    scale_factor: 1.0,
                }),
                Msaa::Off,
                Tonemapping::None,
                RenderLayers::layer(LAYER),
                *transform,
                projection.clone(),
            ));
            state.frames = 0;
        }
    }

    *status = match (&state.grid, state.frames) {
        (None, _) => CutawayStatus::NoNavmesh,
        (Some(_), 0..=1) => CutawayStatus::DepthNotReady,
        _ => CutawayStatus::Active,
    };
    // Bind groups keep the texture views they were built with, and touching
    // the shared buffer does not rebuild them: re-prepare the cuttable
    // materials after the depth or height image is replaced. Twice, in case
    // the first runs before the new image reaches the GPU.
    if state.rebind > 0 {
        state.rebind -= 1;
        let ids: Vec<_> = terrain
            .iter()
            .filter(|(_, m)| m.params.flags.x & CUTAWAY != 0)
            .map(|(id, _)| id)
            .collect();
        for id in ids {
            // `AssetMut` only marks the asset modified on mutable access.
            if let Some(material) = terrain.get_mut(id) {
                material.into_inner();
            }
        }
    }
    let gpu = state.grid.as_ref().map_or(GpuCutaway::default(), |grid| {
        gpu_cutaway(&settings, grid, size)
    });
    upload(&mut state, &mut buffers, &bindings, gpu);
}

fn gpu_cutaway(settings: &CutawaySettings, grid: &HeightGrid, size: UVec2) -> GpuCutaway {
    GpuCutaway {
        params: Vec4::new(
            match (settings.enabled, settings.debug_view) {
                (false, _) => 0.0,
                (true, false) => 1.0,
                (true, true) => 2.0,
            },
            settings.knee_height,
            settings.depth_epsilon,
            settings.outline_width_px,
        ),
        target: Vec4::new(
            settings.outline_strength,
            size.x as f32,
            size.y as f32,
            grid.cell,
        ),
        grid: Vec4::new(
            grid.origin.x,
            grid.origin.y,
            grid.size.x as f32,
            grid.size.y as f32,
        ),
    }
}

/// Only write when the contents change (toggle, level, resize): touching the
/// buffer asset re-uploads it.
fn upload(
    state: &mut CutawayState,
    buffers: &mut Assets<ShaderBuffer>,
    bindings: &CutawayBindings,
    gpu: GpuCutaway,
) {
    if state.uploaded == Some(gpu) {
        return;
    }
    if let Some(mut buffer) = buffers.get_mut(&bindings.buffer) {
        buffer.set_data(gpu);
        state.uploaded = Some(gpu);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutaway_is_off_by_default() {
        assert!(!CutawaySettings::default().enabled);
        assert_eq!(CutawayStatus::default(), CutawayStatus::Off);
        // The disabled shader state is all zero: `params.x` gates every cut.
        assert_eq!(GpuCutaway::default().params.x, 0.0);
    }

    #[test]
    fn gpu_parameters_carry_grid_and_target() {
        let settings = CutawaySettings {
            enabled: true,
            ..default()
        };
        let grid = HeightGrid {
            origin: Vec2::new(-3.0, 5.0),
            cell: 0.5,
            size: UVec2::new(40, 20),
            heights: vec![0.0; 800],
        };
        let gpu = gpu_cutaway(&settings, &grid, UVec2::new(1280, 720));
        assert_eq!(gpu.params, Vec4::new(1.0, 0.75, 0.1, 1.5));
        assert_eq!(gpu.target, Vec4::new(0.2, 1280.0, 720.0, 0.5));
        assert_eq!(gpu.grid, Vec4::new(-3.0, 5.0, 40.0, 20.0));
    }
}
