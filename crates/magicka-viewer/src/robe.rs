//! Original detail/fabric albedo blend, shaded with the original deferred
//! lighting (`lighting.wgsl`).
use bevy::{
    asset::embedded_asset,
    gltf::GltfMaterialExtras,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline},
    prelude::*,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, CompareFunction,
        RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct RobeParams {
    pub diffuse: Vec4,
    pub tint: Vec4,
    /// OverlayAlpha, detail/overlay/soft-light flag bits, zapped,
    /// material map (SpecularMap) enabled.
    pub control: Vec4,
    /// SpecularAmount, SpecularPower, EmissiveAmount, ethereal alpha.
    pub spec: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
#[bind_group_data(RobeKey)]
pub struct RobeExtension {
    #[uniform(100)]
    pub params: RobeParams,
    #[texture(101)]
    #[sampler(102)]
    pub detail: Option<Handle<Image>>,
    #[texture(103)]
    #[sampler(104)]
    pub overlay: Option<Handle<Image>>,
    /// SpecularMap / MaterialMapSampler (R emissive, G spec amount, B power).
    #[texture(105)]
    #[sampler(106)]
    pub material: Option<Handle<Image>>,
    #[uniform(107)]
    pub look: crate::status_look::StatusLook,
    #[texture(108, dimension = "cube")]
    #[sampler(109)]
    pub ice: Option<Handle<Image>>,
    #[texture(110, dimension = "cube")]
    #[sampler(111)]
    pub ice_normal: Option<Handle<Image>>,
    #[storage(90, read_only)]
    pub lights: Handle<bevy::render::storage::ShaderBuffer>,
    /// The player's robe (follows RobeColour); mines use the same material.
    pub player: bool,
}
#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub struct RobeKey {
    ethereal: bool,
}
impl From<&RobeExtension> for RobeKey {
    fn from(m: &RobeExtension) -> Self {
        Self {
            ethereal: (m.params.control.y as u32 & 8) != 0,
        }
    }
}
impl MaterialExtension for RobeExtension {
    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if key.bind_group_data.ethereal {
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
        Ok(())
    }
    fn vertex_shader() -> ShaderRef {
        "embedded://magicka_viewer/status_vertex.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/robe.wgsl".into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        Self::fragment_shader()
    }
    /// The player's body stays out of the depth prepass (outline.rs).
    fn enable_prepass() -> bool {
        false
    }
}
pub type RobeMaterial = ExtendedMaterial<StandardMaterial, RobeExtension>;

#[derive(Resource)]
pub struct RobeColour(pub [f32; 3]);
impl Default for RobeColour {
    fn default() -> Self {
        Self([1.0; 3])
    }
}

#[derive(Component)]
pub struct RobeTint(pub [f32; 3]);

pub struct RobePlugin;
impl Plugin for RobePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "robe.wgsl");
        app.init_resource::<RobeColour>()
            .add_plugins(MaterialPlugin::<RobeMaterial>::default())
            .add_systems(
                Update,
                (apply_materials, update_colour, update_remote_colour).chain(),
            );
    }
}

/// SkinnedModelDeferredEffect material from `magicka_skinned` extras with
/// TintColor `tint` (raw RGB); None when an enabled texture is missing.
pub fn skinned_extension(
    data: &serde_json::Value,
    base: &StandardMaterial,
    base_path: &str,
    server: &AssetServer,
    tint: [f32; 3],
    lights: &crate::lighting::MagickaLights,
    cubes: &crate::status_look::StatusCubes,
) -> Option<RobeExtension> {
    let overlay = data["overlay"]
        .as_str()
        .map(|uri| server.load(crate::terrain::resolve(base_path, uri)));
    let material = data["material"]
        .as_str()
        .map(|uri| server.load(crate::terrain::resolve(base_path, uri)));
    let material_enabled =
        data["material_enabled"].as_bool().unwrap_or(false) && material.is_some();
    let detail_enabled = data["detail_enabled"].as_bool().unwrap_or(false);
    let overlay_enabled = data["overlay_enabled"].as_bool().unwrap_or(false);
    // Do not silently supply white for a missing enabled original texture.
    if (detail_enabled && base.base_color_texture.is_none())
        || (overlay_enabled && overlay.is_none())
    {
        return None;
    }
    let flags = u32::from(detail_enabled)
        | (u32::from(overlay_enabled) << 1)
        | (u32::from(data["soft_light"].as_bool().unwrap_or(false)) << 2);
    let channel = |k| data["diffuse"][k].as_f64().unwrap_or(1.0) as f32;
    Some(RobeExtension {
        params: RobeParams {
            diffuse: Vec4::new(channel(0), channel(1), channel(2), 1.0),
            tint: Vec4::new(tint[0], tint[1], tint[2], 1.0),
            control: Vec4::new(
                data["overlay_alpha"].as_f64().unwrap_or(1.0) as f32,
                flags as f32,
                0.0,
                f32::from(u8::from(material_enabled)),
            ),
            spec: crate::lighting::material_params(Some(data)).material,
        },
        detail: base.base_color_texture.clone(),
        overlay,
        material,
        look: default(),
        ice: cubes.ice.clone(),
        ice_normal: cubes.ice_normal.clone(),
        lights: lights.buffer.clone(),
        player: false,
    })
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn apply_materials(
    mut commands: Commands,
    server: Res<AssetServer>,
    colour: Res<RobeColour>,
    roots: Query<
        (&WorldAssetRoot, Option<&RobeTint>),
        Or<(With<crate::play::AvatarRoot>, With<RobeTint>)>,
    >,
    parents: Query<&ChildOf>,
    meshes: Query<(
        Entity,
        &GltfMaterialExtras,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    standard: Res<Assets<StandardMaterial>>,
    mut robes: ResMut<Assets<RobeMaterial>>,
    lights: Res<crate::lighting::MagickaLights>,
    cubes: Res<crate::status_look::StatusCubes>,
) {
    for (entity, extras, handle) in &meshes {
        let Some((root, tint)) = parents
            .iter_ancestors(entity)
            .find_map(|p| roots.get(p).ok())
        else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&extras.value) else {
            continue;
        };
        let Some(data) = json.get("magicka_skinned") else {
            continue;
        };
        let Some(base) = standard.get(&handle.0) else {
            continue;
        };
        let Some(path) = server.get_path(&root.0) else {
            continue;
        };
        let Some(extension) = skinned_extension(
            data,
            base,
            &path.path().to_string_lossy(),
            &server,
            tint.map_or(colour.0, |t| t.0),
            &lights,
            &cubes,
        ) else {
            continue;
        };
        let extension = RobeExtension {
            player: tint.is_none(),
            ..extension
        };
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(robes.add(RobeMaterial {
                base: base.clone(),
                extension,
            })));
    }
}

fn update_colour(colour: Res<RobeColour>, mut materials: ResMut<Assets<RobeMaterial>>) {
    if !colour.is_changed() {
        return;
    }
    let tint = Vec4::new(colour.0[0], colour.0[1], colour.0[2], 1.0);
    let ids: Vec<_> = materials
        .iter()
        .filter(|(_, m)| m.extension.player)
        .map(|(id, _)| id)
        .collect();
    for id in ids {
        if let Some(mut material) = materials.get_mut(id) {
            material.extension.params.tint = tint;
        }
    }
}

fn update_remote_colour(
    roots: Query<&RobeTint>,
    parents: Query<&ChildOf>,
    meshes: Query<(Entity, &MeshMaterial3d<RobeMaterial>)>,
    mut materials: ResMut<Assets<RobeMaterial>>,
) {
    for (entity, handle) in &meshes {
        if let Some(tint) = parents
            .iter_ancestors(entity)
            .find_map(|p| roots.get(p).ok())
        {
            if let Some(mut material) = materials.get_mut(&handle.0) {
                material.extension.params.tint = Vec4::new(tint.0[0], tint.0[1], tint.0[2], 1.);
            }
        }
    }
}
