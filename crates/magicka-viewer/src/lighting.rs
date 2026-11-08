//! PolygonHead deferred lighting shared by the viewer's materials.
//!
//! Every lit material binds the same storage buffer at binding 90 and imports
//! `magicka_viewer::lighting` (`lighting.wgsl`), which evaluates the original
//! light shaders and DeferredShadingEffect composite per fragment. The buffer
//! has a fixed size so per-frame updates are written in place and material
//! bind groups stay valid. See `research/level-lighting.md`.

use bevy::asset::embedded_asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, CompareFunction,
    RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use magicka_content::level::LevelLight;

/// Light capacity; the largest retail level has 134 static lights.
pub const MAX_LIGHTS: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct GpuLight {
    pub position: Vec4,
    pub direction: Vec4,
    pub diffuse: Vec4,
    pub ambient: Vec4,
    pub cone: Vec4,
}

#[derive(Clone, Debug, ShaderType)]
struct GpuLights {
    header: Vec4,
    flags: Vec4,
    fog_color: Vec4,
    fog_range: Vec4,
    lights: [GpuLight; MAX_LIGHTS],
}

impl GpuLight {
    /// CapsuleLight uses its own native effect, represented here by host kind
    /// 3. cone.xyz stores End; Start and Radius use the position field.
    pub fn from_capsule(light: &magicka_runtime::arcane_blade::CapsuleLight) -> Self {
        let v = Vec3::from_array;
        Self {
            position: v(light.start).extend(light.radius),
            direction: Vec4::new(0.0, 0.0, 0.0, 3.0),
            diffuse: (v(light.diffuse) * light.intensity).extend(0.0),
            ambient: (v(light.ambient) * light.intensity).extend(0.0),
            cone: v(light.end).extend(0.0),
        }
    }

    /// A level light at the given intensity (Light.Draw multiplies diffuse,
    /// ambient and specular by `mIntensity`).
    pub fn from_level(l: &LevelLight, intensity: f32) -> Self {
        let v = |a: [f32; 3]| Vec3::from_array(a);
        let direction = v(l.direction).normalize_or_zero();
        Self {
            position: v(l.position).extend(l.range),
            direction: direction.extend(l.light_type as f32),
            diffuse: (v(l.diffuse) * intensity).extend(l.specular * intensity),
            ambient: (v(l.ambient) * intensity).extend(f32::from(u8::from(l.use_attenuation))),
            cone: Vec4::new(l.cutoff_angle, l.sharpness, 0.0, 0.0),
        }
    }
}

/// CPU-side light list; written to the GPU buffer every frame.
#[derive(Resource)]
pub struct MagickaLights {
    pub buffer: Handle<ShaderBuffer>,
    pub lights: Vec<GpuLight>,
    /// A single shadow-casting Bevy DirectionalLight mirrors the level's
    /// shadow-casting directional light (its index must be 0).
    pub sun_shadow: bool,
    pub fog: Option<magicka_runtime::play_content::SceneFog>,
}

impl MagickaLights {
    /// Stand-in for viewers that have no level light list: one white
    /// directional light with a soft ambient term.
    pub fn fallback() -> Vec<GpuLight> {
        vec![GpuLight {
            position: Vec4::ZERO,
            direction: Vec3::new(-0.35, -0.85, -0.4).normalize().extend(1.0),
            diffuse: Vec4::new(0.8, 0.8, 0.8, 1.0),
            ambient: Vec4::new(0.4, 0.4, 0.4, 0.0),
            cone: Vec4::ZERO,
        }]
    }
}

pub struct LightingPlugin;

impl Plugin for LightingPlugin {
    fn build(&self, app: &mut App) {
        // An imported module only resolves once its shader asset is loaded.
        bevy::shader::load_shader_library!(app, "lighting.wgsl");
        embedded_asset!(app, "lit.wgsl");
        let buffer =
            app.world_mut()
                .resource_mut::<Assets<ShaderBuffer>>()
                .add(ShaderBuffer::from(GpuLights {
                    header: Vec4::ZERO,
                    flags: Vec4::ZERO,
                    fog_color: Vec4::ZERO,
                    fog_range: Vec4::ZERO,
                    lights: [GpuLight::default(); MAX_LIGHTS],
                }));
        app.insert_resource(MagickaLights {
            buffer,
            lights: MagickaLights::fallback(),
            sun_shadow: false,
            fog: None,
        })
        .add_plugins(MaterialPlugin::<LitMaterial>::default())
        .add_plugins(MaterialPlugin::<PlayerLitMaterial>::default())
        .add_plugins(MaterialPlugin::<NonOccludingMaterial>::default())
        .add_systems(PostUpdate, upload_lights)
        .add_systems(
            PostUpdate,
            (exclude_standard_occluders, sync_standard_sources).chain(),
        )
        .add_systems(Update, apply_lit_materials);
        if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render.add_systems(bevy::render::Render, report_pipeline_errors);
        }
    }
}

/// Bevy retries some pipeline failures (e.g. an unresolved shader import)
/// without logging them; report each distinct failure once.
fn report_pipeline_errors(
    mut seen: Local<std::collections::HashSet<String>>,
    cache: Res<bevy::render::render_resource::PipelineCache>,
) {
    for pipeline in cache.pipelines() {
        if let bevy::render::render_resource::CachedPipelineState::Err(e) = &pipeline.state {
            let message = format!("{e:?}");
            if seen.insert(message.clone()) {
                warn!("render pipeline failed: {message}");
            }
        }
    }
}

/// Write the light list when it changes. Touching the buffer asset makes
/// Bevy re-prepare every material bound to it, so unchanged frames must not
/// call `get_mut`.
fn upload_lights(
    lights: Res<MagickaLights>,
    mut uploaded: Local<
        Option<(
            Vec<GpuLight>,
            bool,
            Option<magicka_runtime::play_content::SceneFog>,
        )>,
    >,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let count = lights.lights.len().min(MAX_LIGHTS);
    let current = (
        lights.lights[..count].to_vec(),
        lights.sun_shadow,
        lights.fog,
    );
    if uploaded.as_ref() == Some(&current) {
        return;
    }
    let mut data = GpuLights {
        header: Vec4::new(count as f32, 0.0, 0.0, 0.0),
        flags: Vec4::new(f32::from(u8::from(lights.sun_shadow)), 0.0, 0.0, 0.0),
        fog_color: lights.fog.map_or(Vec4::ZERO, |f| Vec4::from_array(f.color)),
        fog_range: lights
            .fog
            .map_or(Vec4::ZERO, |f| Vec4::new(f.start, f.end, 1., 0.)),
        lights: [GpuLight::default(); MAX_LIGHTS],
    };
    data.lights[..count].copy_from_slice(&current.0);
    if let Some(mut buffer) = buffers.get_mut(&lights.buffer) {
        buffer.set_data(data);
        *uploaded = Some(current);
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct LitParams {
    /// SpecAmount, SpecPower, EmissiveAmount, zapped flag.
    pub material: Vec4,
    /// x: skinned SpecularMap enabled, y: ethereal alpha, z: additive body.
    pub maps: Vec4,
}

/// AnimatedModelAdditive is selected independently of ordinary alpha modes.
#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub struct LitKey {
    ethereal: bool,
}

fn specialize_ethereal(descriptor: &mut RenderPipelineDescriptor, ethereal: bool) {
    if !ethereal {
        return;
    }
    // PolygonHead.Scene.DrawAdditive: source InverseDestinationColor,
    // destination One; native LessEqual becomes GreaterEqual in reverse Z.
    let blend = BlendComponent {
        src_factor: BlendFactor::OneMinusDst,
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
    if let Some(depth) = &mut descriptor.depth_stencil {
        depth.depth_compare = Some(CompareFunction::GreaterEqual);
        depth.depth_write_enabled = Some(false);
    }
}

/// Original lighting on top of a StandardMaterial's albedo, for meshes that
/// have no dedicated reconstructed material.
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
#[bind_group_data(LitKey)]
pub struct LitExtension {
    #[uniform(100)]
    pub params: LitParams,
    #[storage(90, read_only)]
    pub lights: Handle<ShaderBuffer>,
    /// Skinned SpecularMap (R emissive, G spec amount, B spec power).
    #[texture(101)]
    #[sampler(102)]
    pub material_map: Option<Handle<Image>>,
    /// Character status look (status_look.rs).
    #[uniform(107)]
    pub look: crate::status_look::StatusLook,
    #[texture(108, dimension = "cube")]
    #[sampler(109)]
    pub ice: Option<Handle<Image>>,
    #[texture(110, dimension = "cube")]
    #[sampler(111)]
    pub ice_normal: Option<Handle<Image>>,
}

impl From<&LitExtension> for LitKey {
    fn from(m: &LitExtension) -> Self {
        Self {
            ethereal: m.params.maps.z > 0.5,
        }
    }
}
impl MaterialExtension for LitExtension {
    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_ethereal(descriptor, key.bind_group_data.ethereal);
        Ok(())
    }
    fn vertex_shader() -> ShaderRef {
        "embedded://magicka_viewer/status_vertex.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/lit.wgsl".into()
    }
}

pub type LitMaterial = ExtendedMaterial<StandardMaterial, LitExtension>;

/// Meshes below an entity with this marker stay out of the depth prepass, so
/// the prepass depth holds only what may hide the player's outline
/// (outline.rs). The main pass still writes their depth, which is included in
/// the separate soft-particle snapshot (effect_depth.rs).
#[derive(Component)]
pub struct NoDepthPrepass;

/// Retail level renderers write stencil 1; entity renderers use other values.
/// Explicit provenance, rather than material appearance, selects occluders.
#[derive(Component)]
pub struct OutlineOccluder;

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct NonOccludingExtension {}
impl MaterialExtension for NonOccludingExtension {
    fn enable_prepass() -> bool {
        false
    }
}
#[derive(Component)]
struct StandardSource(Handle<StandardMaterial>);

pub type NonOccludingMaterial = ExtendedMaterial<StandardMaterial, NonOccludingExtension>;

pub(crate) fn exclude_standard_occluders(
    mut commands: Commands,
    parents: Query<&ChildOf>,
    occluders: Query<(), With<OutlineOccluder>>,
    // Native materials may arrive after Update. Leave them for their owners
    // next frame; taking their StandardMaterial here strands them in Bevy's
    // PBR shader (and its bright ambient lighting) permanently.
    meshes: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>),
        (With<KeepStandardMaterial>, Without<EditorGhost>),
    >,
    standard: Res<Assets<StandardMaterial>>,
    mut materials: ResMut<Assets<NonOccludingMaterial>>,
) {
    for (entity, handle) in &meshes {
        if occluders.contains(entity)
            || parents
                .iter_ancestors(entity)
                .any(|p| occluders.contains(p))
        {
            continue;
        }
        if let Some(base) = standard.get(&handle.0) {
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert((
                    StandardSource(handle.0.clone()),
                    MeshMaterial3d(materials.add(NonOccludingMaterial {
                        base: base.clone(),
                        extension: NonOccludingExtension {},
                    })),
                ));
        }
    }
}

// Retain updates by systems which own StandardMaterial handles (e.g. Death).
fn sync_standard_sources(
    mut events: MessageReader<AssetEvent<StandardMaterial>>,
    sources: Query<(&StandardSource, &MeshMaterial3d<NonOccludingMaterial>)>,
    standard: Res<Assets<StandardMaterial>>,
    mut materials: ResMut<Assets<NonOccludingMaterial>>,
) {
    let changed: std::collections::HashSet<_> = events
        .read()
        .filter_map(|event| match event {
            AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    for (source, handle) in &sources {
        if changed.contains(&source.0.id()) {
            if let (Some(base), Some(mut material)) =
                (standard.get(&source.0), materials.get_mut(&handle.0))
            {
                material.base = base.clone();
            }
        }
    }
}

/// [`LitExtension`] without a prepass (player equipment).
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
#[bind_group_data(LitKey)]
pub struct PlayerLitExtension {
    #[uniform(100)]
    pub params: LitParams,
    #[storage(90, read_only)]
    pub lights: Handle<ShaderBuffer>,
    /// Skinned SpecularMap (R emissive, G spec amount, B spec power).
    #[texture(101)]
    #[sampler(102)]
    pub material_map: Option<Handle<Image>>,
    /// Character status look (status_look.rs).
    #[uniform(107)]
    pub look: crate::status_look::StatusLook,
    #[texture(108, dimension = "cube")]
    #[sampler(109)]
    pub ice: Option<Handle<Image>>,
    #[texture(110, dimension = "cube")]
    #[sampler(111)]
    pub ice_normal: Option<Handle<Image>>,
}
impl From<&PlayerLitExtension> for LitKey {
    fn from(m: &PlayerLitExtension) -> Self {
        Self {
            ethereal: m.params.maps.z > 0.5,
        }
    }
}
impl MaterialExtension for PlayerLitExtension {
    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_ethereal(descriptor, key.bind_group_data.ethereal);
        Ok(())
    }
    fn vertex_shader() -> ShaderRef {
        "embedded://magicka_viewer/status_vertex.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/lit.wgsl".into()
    }
    fn enable_prepass() -> bool {
        false
    }
}
pub type PlayerLitMaterial = ExtendedMaterial<StandardMaterial, PlayerLitExtension>;

/// Marks meshes that keep Bevy's StandardMaterial on purpose.
#[derive(Component)]
pub struct KeepStandardMaterial;

/// Editor ghost meshes (bevy-toolmode): their translucent StandardMaterial
/// must not be swapped for the game's lit/terrain materials.
#[derive(Component)]
pub struct EditorGhost;

/// Material parameters from `magicka_skinned`/`magicka_deferred` extras'
/// `spec` array (SpecAmount, SpecPower, EmissiveAmount); zero otherwise.
pub fn material_params(extras: Option<&serde_json::Value>) -> LitParams {
    let spec = extras
        .and_then(|d| d.get("spec"))
        .and_then(|s| s.as_array())
        .map(|a| {
            let f = |i: usize| a.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            Vec4::new(f(0), f(1), f(2), 0.0)
        })
        .unwrap_or(Vec4::ZERO);
    LitParams {
        material: spec,
        maps: Vec4::ZERO,
    }
}

/// Replace remaining lit StandardMaterials with [`LitMaterial`]. Terrain,
/// additive, liquid and the player's robe have dedicated materials whose
/// systems convert them; this skips their extras.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn apply_lit_materials(
    mut commands: Commands,
    lights: Res<MagickaLights>,
    standard: Res<Assets<StandardMaterial>>,
    mut lit: ResMut<Assets<LitMaterial>>,
    mut player_lit: ResMut<Assets<PlayerLitMaterial>>,
    server: Res<AssetServer>,
    roots: Query<&bevy::world_serialization::WorldAssetRoot>,
    parents: Query<&ChildOf>,
    avatars: Query<(), Or<(With<crate::play::AvatarRoot>, With<crate::robe::RobeTint>)>>,
    barriers: Query<(), With<crate::play::BarrierPart>>,
    no_prepass: Query<(), With<NoDepthPrepass>>,
    occluders: Query<(), With<OutlineOccluder>>,
    q: Query<
        (
            Entity,
            &MeshMaterial3d<StandardMaterial>,
            Option<&bevy::gltf::GltfMaterialExtras>,
        ),
        (Without<KeepStandardMaterial>, Without<EditorGhost>),
    >,
) {
    for (entity, handle, extras) in &q {
        let json = extras.and_then(|e| serde_json::from_str::<serde_json::Value>(&e.value).ok());
        if json.as_ref().is_some_and(|j| {
            [
                "magicka_deferred",
                "magicka_additive",
                "magicka_liquid",
                "magicka_lava",
                "magicka_shadow_only",
            ]
            .iter()
            .any(|k| j.get(*k).is_some())
        }) {
            continue;
        }
        let skinned = json.as_ref().and_then(|j| j.get("magicka_skinned"));
        if skinned.is_some()
            && parents
                .iter_ancestors(entity)
                .any(|a| avatars.contains(a) || barriers.contains(a))
        {
            continue;
        }
        let Some(base) = standard.get(&handle.0) else {
            continue;
        };
        if base.unlit || matches!(base.alpha_mode, AlphaMode::Add | AlphaMode::Multiply) {
            commands.entity(entity).insert(KeepStandardMaterial);
            continue;
        }
        let mut params = material_params(skinned);
        let material_map = skinned
            .filter(|s| s["material_enabled"].as_bool().unwrap_or(false))
            .and_then(|s| s["material"].as_str())
            .and_then(|uri| {
                let root = parents
                    .iter_ancestors(entity)
                    .find_map(|a| roots.get(a).ok())?;
                let path = server.get_path(&root.0)?;
                Some(server.load(crate::terrain::resolve(&path.path().to_string_lossy(), uri)))
            });
        params.maps.x = f32::from(u8::from(material_map.is_some()));
        let mut e = commands.entity(entity);
        e.remove::<MeshMaterial3d<StandardMaterial>>();
        if !(occluders.contains(entity)
            || parents
                .iter_ancestors(entity)
                .any(|a| occluders.contains(a)))
            || parents
                .iter_ancestors(entity)
                .any(|a| no_prepass.contains(a))
        {
            e.insert(MeshMaterial3d(player_lit.add(PlayerLitMaterial {
                base: base.clone(),
                extension: PlayerLitExtension {
                    params,
                    lights: lights.buffer.clone(),
                    material_map: material_map.clone(),
                    look: default(),
                    ice: None,
                    ice_normal: None,
                },
            })));
        } else {
            e.insert(MeshMaterial3d(lit.add(LitMaterial {
                base: base.clone(),
                extension: LitExtension {
                    params,
                    lights: lights.buffer.clone(),
                    material_map,
                    look: default(),
                    ice: None,
                    ice_normal: None,
                },
            })));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level_light(light_type: i32) -> LevelLight {
        LevelLight {
            name: "light_directional".into(),
            position: [1.0, 2.0, 3.0],
            direction: [0.0, -2.0, 0.0],
            light_type,
            variation_type: 0,
            range: 5.0,
            use_attenuation: true,
            cutoff_angle: 0.5,
            sharpness: 2.0,
            diffuse: [0.8, 0.8, 0.56],
            ambient: [0.35, 0.35, 0.5],
            specular: 1.0,
            variation_speed: 0.0,
            variation_amount: 0.0,
            shadow_map_size: 1024,
            cast_shadows: true,
        }
    }

    #[test]
    fn level_light_packs_intensity_scaled_colours() {
        let gpu = GpuLight::from_level(&level_light(1), 0.5);
        assert_eq!(gpu.position, Vec4::new(1.0, 2.0, 3.0, 5.0));
        // Direction is normalized; w carries LightType.
        assert_eq!(gpu.direction, Vec4::new(0.0, -1.0, 0.0, 1.0));
        // Light.Draw multiplies diffuse, ambient and specular by intensity.
        assert_eq!(gpu.diffuse, Vec4::new(0.4, 0.4, 0.28, 0.5));
        assert_eq!(gpu.ambient, Vec4::new(0.175, 0.175, 0.25, 1.0));
        assert_eq!(gpu.cone, Vec4::new(0.5, 2.0, 0.0, 0.0));
    }

    #[test]
    fn spot_without_attenuation_is_flagged() {
        let mut light = level_light(2);
        light.use_attenuation = false;
        assert_eq!(GpuLight::from_level(&light, 1.0).ambient.w, 0.0);
    }

    #[test]
    fn material_params_read_spec_extras() {
        let extras: serde_json::Value = serde_json::from_str(r#"{"spec":[0.5,20,2]}"#).unwrap();
        assert_eq!(
            material_params(Some(&extras)).material,
            Vec4::new(0.5, 20.0, 2.0, 0.0)
        );
        assert_eq!(material_params(None).material, Vec4::ZERO);
    }
}
