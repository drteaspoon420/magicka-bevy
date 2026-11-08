//! `TerrainMaterial`: the game's RenderDeferredEffect layer blending
//! (`terrain.wgsl`), applied to glTF primitives whose material carries
//! `extras.magicka_deferred` (written by `level-gltf`).

use bevy::asset::embedded_asset;
use bevy::gltf::GltfMaterialExtras;
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

pub const TEX0: u32 = 1;
pub const A0_OFF: u32 = 2;
pub const MASK0: u32 = 4;
pub const A1_OFF: u32 = 8;
pub const MASK1: u32 = 16;
pub const DUAL: u32 = 32;
pub const VCOLOR: u32 = 64;
pub const UNLIT: u32 = 128;
pub const NO_DISCARD: u32 = 256;
/// MaterialTexture0Enabled / MaterialTexture1Enabled.
pub const MAT0: u32 = 512;
pub const MAT1: u32 = 1024;
/// Static level architecture the optional cutaway may trim (cutaway.rs).
pub const CUTAWAY: u32 = 2048;
/// Only level-owned meshes participate in the outline depth selection.
pub const OUTLINE_OCCLUDER: u32 = 4096;

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct TerrainParams {
    pub color0: Vec4,
    pub color1: Vec4,
    pub flags: UVec4,
    pub misc: Vec4,
    /// SpecAmount, SpecPower, EmissiveAmount per layer.
    pub spec0: Vec4,
    pub spec1: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct TerrainMaterial {
    #[uniform(0)]
    pub params: TerrainParams,
    #[texture(1)]
    #[sampler(2)]
    pub tex0: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub tex1: Option<Handle<Image>>,
    /// MaterialSampler0/1 (raw bytes): R emissive, G spec amount and
    /// B spec power multipliers.
    #[texture(5)]
    #[sampler(6)]
    pub mat0: Option<Handle<Image>>,
    #[texture(7)]
    #[sampler(8)]
    pub mat1: Option<Handle<Image>>,
    #[storage(90, read_only)]
    pub lights: Handle<bevy::render::storage::ShaderBuffer>,
    /// Shared cutaway state ([`crate::cutaway::CutawayBindings`]).
    #[storage(91, read_only)]
    pub cutaway: Handle<bevy::render::storage::ShaderBuffer>,
    #[texture(92, sample_type = "float", filterable = false)]
    pub cutaway_depth: Handle<Image>,
    #[texture(93, sample_type = "float", filterable = false)]
    pub cutaway_heights: Handle<Image>,
}

impl TerrainMaterial {
    /// Material with no textures and the shared light/cutaway bindings.
    pub fn new(
        params: TerrainParams,
        lights: &crate::lighting::MagickaLights,
        cutaway: &crate::cutaway::CutawayBindings,
    ) -> Self {
        Self {
            params,
            tex0: None,
            tex1: None,
            mat0: None,
            mat1: None,
            lights: lights.buffer.clone(),
            cutaway: cutaway.buffer.clone(),
            cutaway_depth: cutaway.depth.clone(),
            cutaway_heights: cutaway.heights.clone(),
        }
    }
}

impl Material for TerrainMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/terrain.wgsl".into()
    }
    /// The outline depth prepass runs the same coverage
    /// discard, so cut-out texels leave no depth behind.
    fn prepass_fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/terrain.wgsl".into()
    }
    /// `Mask` marks the pipeline MAY_DISCARD so the prepass runs the
    /// fragment shader; the cutoff itself is evaluated in `terrain.wgsl`.
    fn alpha_mode(&self) -> AlphaMode {
        // Always run the prepass fragment shader, including geometry-only
        // mode, so non-level models cannot write outline depth.
        AlphaMode::Mask(0.5)
    }
    /// Bevy's prepass pipeline defaults to no culling while the main pass
    /// culls back faces; without this a level wall seen from behind wrote
    /// prepass depth, hid everything past it and drew nothing (ru_s4 black
    /// occluder). The original draws level geometry with back-face culling.
    fn specialize(
        _: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _: &bevy::mesh::MeshVertexBufferLayoutRef,
        _: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = Some(bevy::render::render_resource::Face::Back);
        Ok(())
    }
}

/// Unlit output (exact albedo) instead of the original lighting.
#[derive(Resource, Clone, Copy)]
pub struct TerrainUnlit(pub bool);

/// Disable the coverage discard (geometry-only silhouette checks).
#[derive(Resource, Clone, Copy)]
pub struct TerrainNoDiscard(pub bool);

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "terrain.wgsl");
        embedded_asset!(app, "additive.wgsl");
        app.add_plugins(MaterialPlugin::<crate::additive::AdditiveMaterial>::default());
        embedded_asset!(app, "shadow_only.wgsl");
        app.add_plugins(MaterialPlugin::<crate::shadow_only::ShadowOnlyMaterial>::default());
        embedded_asset!(app, "beam.wgsl");
        app.add_plugins(MaterialPlugin::<crate::beam::BeamMaterial>::default());
        embedded_asset!(app, "liquid.wgsl");
        embedded_asset!(app, "lava.wgsl");
        app.add_plugins(MaterialPlugin::<crate::lava::LavaMaterial>::default())
            .add_systems(Update, crate::lava::advance_time);
        app.add_plugins(MaterialPlugin::<crate::liquid::LiquidMaterial>::default())
            .add_systems(
                Update,
                (
                    crate::liquid::advance_time,
                    crate::liquid::add_freeze_stream,
                ),
            );
        app.add_plugins(crate::lighting::LightingPlugin)
            .init_resource::<crate::cutaway::CutawayBindings>()
            .add_plugins(MaterialPlugin::<TerrainMaterial>::default())
            .add_plugins(crate::status_look::StatusLookPlugin)
            .add_plugins(crate::robe::RobePlugin)
            .add_systems(Update, apply_terrain_materials);
    }
}

/// Resolve a URI relative to the directory of `base` (an asset path).
pub(crate) fn resolve(base: &str, uri: &str) -> String {
    let mut parts: Vec<&str> = base.split('/').collect();
    parts.pop();
    for seg in uri.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[derive(Resource, Clone)]
pub struct ScenePath(pub String);

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn apply_terrain_materials(
    mut commands: Commands,
    assets: Res<AssetServer>,
    scene: Option<Res<ScenePath>>,
    hierarchy: (
        Query<&bevy::world_serialization::WorldAssetRoot>,
        Query<&ChildOf>,
        Query<(), With<crate::lighting::OutlineOccluder>>,
    ),
    unlit: Option<Res<TerrainUnlit>>,
    no_discard: Option<Res<TerrainNoDiscard>>,
    std_materials: Res<Assets<StandardMaterial>>,
    mut terrain: ResMut<Assets<TerrainMaterial>>,
    mut additive: ResMut<Assets<crate::additive::AdditiveMaterial>>,
    mut shadow_only: ResMut<Assets<crate::shadow_only::ShadowOnlyMaterial>>,
    mut liquids: ResMut<Assets<crate::liquid::LiquidMaterial>>,
    mut lava: ResMut<Assets<crate::lava::LavaMaterial>>,
    lights: Res<crate::lighting::MagickaLights>,
    cutaway: Res<crate::cutaway::CutawayBindings>,
    q: Query<
        (
            Entity,
            &GltfMaterialExtras,
            &MeshMaterial3d<StandardMaterial>,
        ),
        (
            Added<GltfMaterialExtras>,
            Without<crate::lighting::EditorGhost>,
        ),
    >,
) {
    let (roots, parents, occluders) = hierarchy;
    let Some(scene) = scene else { return };
    for (entity, extras, std_handle) in &q {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&extras.value) else {
            continue;
        };
        let asset_path = parents
            .iter_ancestors(entity)
            .find_map(|parent| roots.get(parent).ok())
            .and_then(|root| assets.get_path(&root.0))
            .map(|p| p.path().to_string_lossy().into_owned())
            .unwrap_or_else(|| scene.0.clone());
        if let Some(d) = json.get("magicka_additive") {
            let c = d["color"].as_array();
            let channel = |i: usize| {
                c.and_then(|a| a.get(i))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.) as f32
            };
            let texture = std_materials
                .get(&std_handle.0)
                .and_then(|m| m.base_color_texture.clone());
            let material = additive.add(crate::additive::AdditiveMaterial {
                color: Vec4::new(channel(0), channel(1), channel(2), 1.),
                flags: UVec4::new(
                    u32::from(d["texture_enabled"].as_bool().unwrap_or(false) && texture.is_some()),
                    u32::from(d["vertex_color"].as_bool().unwrap_or(false)),
                    0,
                    0,
                ),
                texture,
            });
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(material));
            continue;
        }
        if json.get("magicka_shadow_only").is_some() {
            let material = shadow_only.add(crate::shadow_only::ShadowOnlyMaterial::default());
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(material));
            continue;
        }
        if let Some(d) = json.get("magicka_lava") {
            let material = lava.add(crate::lava::from_extras(
                d,
                &assets,
                lights.buffer.clone(),
                |u| resolve(&asset_path, u),
            ));
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert((MeshMaterial3d(material), crate::liquid::LiquidSurface));
            continue;
        }
        if let Some(d) = json.get("magicka_liquid") {
            let material = liquids.add(crate::liquid::from_extras(
                d,
                &assets,
                lights.buffer.clone(),
                |u| resolve(&asset_path, u),
            ));
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert((MeshMaterial3d(material), crate::liquid::LiquidSurface));
            continue;
        }
        let Some(d) = json.get("magicka_deferred") else {
            continue;
        };
        let b = |k: &str| d.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
        let c = |k: &str| {
            let a = d
                .get(k)
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let f = |i: usize| a.get(i).and_then(|x| x.as_f64()).unwrap_or(1.0) as f32;
            Vec4::new(f(0), f(1), f(2), 1.0)
        };
        let tex0 = std_materials
            .get(&std_handle.0)
            .and_then(|m| m.base_color_texture.clone());
        let tex1 = d.get("tex1").and_then(|v| v.as_str()).map(|u| {
            assets
                .load_builder()
                .with_settings(|settings: &mut ImageLoaderSettings| {
                    // Original DiffuseSampler1 wraps tiled terrain UVs. Direct
                    // PNG loading otherwise clamps, unlike glTF's sampler.
                    settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                        address_mode_u: ImageAddressMode::Repeat,
                        address_mode_v: ImageAddressMode::Repeat,
                        ..ImageSamplerDescriptor::linear()
                    });
                })
                .load(resolve(&asset_path, u))
        });
        let raw = |k: &str| {
            d.get(k).and_then(|v| v.as_str()).map(|u| {
                assets
                    .load_builder()
                    .with_settings(|settings: &mut ImageLoaderSettings| {
                        settings.is_srgb = false;
                        settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                            address_mode_u: ImageAddressMode::Repeat,
                            address_mode_v: ImageAddressMode::Repeat,
                            ..ImageSamplerDescriptor::linear()
                        });
                    })
                    .load(resolve(&asset_path, u))
            })
        };
        let mat0 = raw("mat0");
        let mat1 = raw("mat1");
        let mut flags = if occluders.contains(entity)
            || parents
                .iter_ancestors(entity)
                .any(|p| occluders.contains(p))
        {
            OUTLINE_OCCLUDER
        } else {
            0
        };
        for (bit, on) in [
            (TEX0, tex0.is_some()),
            (A0_OFF, b("tex0_alpha_disabled")),
            (MASK0, b("alpha_mask0")),
            (A1_OFF, b("tex1_alpha_disabled")),
            (MASK1, b("alpha_mask1")),
            (DUAL, b("dual_layer") && tex1.is_some()),
            (VCOLOR, b("vertex_color")),
            (UNLIT, unlit.as_ref().is_some_and(|u| u.0)),
            (NO_DISCARD, no_discard.as_ref().is_some_and(|u| u.0)),
            (MAT0, mat0.is_some()),
            (MAT1, mat1.is_some()),
        ] {
            if on {
                flags |= bit;
            }
        }
        let sharpness = d.get("sharpness").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let spec = |k: &str| {
            let a = d.get(k).and_then(|v| v.as_array());
            let f = |i: usize| {
                a.and_then(|a| a.get(i))
                    .and_then(|x| x.as_f64())
                    .unwrap_or(0.0) as f32
            };
            Vec4::new(f(0), f(1), f(2), 0.0)
        };
        let mat = terrain.add(TerrainMaterial {
            tex0,
            tex1,
            mat0,
            mat1,
            ..TerrainMaterial::new(
                TerrainParams {
                    color0: c("color0"),
                    color1: c("color1"),
                    flags: UVec4::new(flags, 0, 0, 0),
                    misc: Vec4::new(sharpness, 0.0, 0.0, 0.0),
                    spec0: spec("spec0"),
                    spec1: spec("spec1"),
                },
                &lights,
                &cutaway,
            )
        });
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(mat));
    }
}

/// Deterministic test texture: the same generator is used by
/// `tools/terrain_check.py` to compute the expected image.
pub fn test_texture_bytes(seed: u32, size: u32) -> Vec<u8> {
    (0..size * size * 4)
        .map(|i| {
            let x = (i.wrapping_add(seed.wrapping_mul(7919))).wrapping_mul(2654435761);
            ((x >> 13) & 0xFF) as u8
        })
        .collect()
}
