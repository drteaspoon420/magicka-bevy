//! Avatar.OutlineRenderData: the player's coloured silhouette outline where
//! the body is hidden behind level geometry. See `research/avatar-outline.md`.
//!
//! The original draws Model.Meshes[0].MeshParts[0] twice with
//! SkinnedModelDeferredEffect. Pass 1 (colour writes off) draws the body
//! shrunk by Bloat −0.0333 with depth Greater and increments stencil where it
//! is hidden; pass 2 draws the full body (DiffuseColor = TintColor,
//! EmissiveAmount 1, DepthBias −0.0025) only where the stencil was not
//! incremented, leaving a hidden rim. Bevy's main depth buffer has no stencil:
//! pass 1 here writes the shrunk body's (farther) depth instead, so pass 2's
//! "behind" test fails over the interior. Both passes sort after every other
//! transparent item, so the substituted depth reaches nothing drawn later.
use bevy::{
    asset::embedded_asset,
    camera::visibility::NoFrustumCulling,
    mesh::{skinning::SkinnedMesh, MeshVertexBufferLayoutRef},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, ColorWrites, CompareFunction, RenderPipelineDescriptor, ShaderType,
        SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

/// SkinnedModelDeferredEffect.Bloat of the first pass (static CIL, Draw IL_0041).
pub const PASS1_BLOAT: f32 = -0.0333;
/// RenderState.DepthBias of the visible pass when not Forced (IL_020a).
pub const OUTLINE_DEPTH_BIAS: f32 = -0.0025;
/// RenderState.DepthBias of the Forced first pass (IL_00ca).
pub const FORCED_DEPTH_BIAS: f32 = -0.1;
/// MagickCamera projection planes (`play-session.md`).
pub const CAMERA_NEAR: f32 = 105.0;
pub const CAMERA_FAR: f32 = 500.0;

/// Reverse-Z clip-depth scale equivalent to a D3D9 DepthBias on XNA's
/// conventional `d = f/(f−n)·(1 − n/z)` depth at view distance `z`.
///
/// With `d' = d + bias`, the biased distance is `z' = n / (n/z − bias(f−n)/f)`;
/// an infinite reverse-Z depth `near/z` scales by `z/z' = 1 − k·z` with
/// `k = bias(f−n)/(f·n)` (this function; the shader applies `1 − k·clip.w`).
pub fn depth_bias_coefficient(bias: f32) -> f32 {
    bias * (CAMERA_FAR - CAMERA_NEAR) / (CAMERA_FAR * CAMERA_NEAR)
}

/// Whether Avatar.Update submits the outline this frame and with which mode.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AvatarOutline {
    pub submit: bool,
    /// OutlineRenderData.Forced = Character.IsInvisibile.
    pub forced: bool,
}

impl AvatarOutline {
    /// Avatar.Update IL_0508–IL_0551: alive, rendered and outside cutscenes,
    /// and either visible (`!(timer >un 0)`) or not a Versus game (type 4).
    pub fn from_state(
        dead: bool,
        do_not_render: bool,
        in_cutscene: bool,
        invisibility_timer: f32,
        game_type: u8,
    ) -> Self {
        let visible = !(invisibility_timer > 0.0 || invisibility_timer.is_nan());
        Self {
            submit: !dead && !do_not_render && !in_cutscene && (visible || game_type != 4),
            // Character.get_IsInvisibile uses an ordered `cgt`.
            forced: invisibility_timer > 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct OutlineParams {
    /// TintColor (raw gamma-space RGB).
    pub color: Vec4,
    /// Bloat, depth_bias_coefficient(DepthBias), EmissiveAmount,
    /// prepass-occlusion test flag.
    pub control: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
#[bind_group_data(OutlineKey)]
pub struct OutlineMaterial {
    #[uniform(100)]
    pub params: OutlineParams,
    #[storage(90, read_only)]
    pub lights: Handle<bevy::render::storage::ShaderBuffer>,
    /// Nearest opaque surface: rejects props covering the selected level depth.
    #[texture(101, sample_type = "float", filterable = false)]
    pub opaque_depth: Handle<Image>,
    pub colour_pass: bool,
    pub forced: bool,
}

#[repr(C)]
#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub struct OutlineKey {
    colour_pass: bool,
    forced: bool,
}
impl From<&OutlineMaterial> for OutlineKey {
    fn from(m: &OutlineMaterial) -> Self {
        Self {
            colour_pass: m.colour_pass,
            forced: m.forced,
        }
    }
}

/// Native compare functions translated to Bevy's reverse-Z depth.
fn pass_depth(colour_pass: bool, forced: bool) -> (CompareFunction, bool) {
    match (forced, colour_pass) {
        // Pass 1 tests "behind" in the shader against the prepass depth
        // (only level geometry) and
        // writes its depth as the stencil stand-in; pass 2 keeps Greater
        // ("behind") → Less against that.
        (false, false) => (CompareFunction::Always, true),
        (false, true) => (CompareFunction::Less, false),
        // Forced: Always + write, then LessEqual → GreaterEqual.
        (true, false) => (CompareFunction::Always, true),
        (true, true) => (CompareFunction::GreaterEqual, false),
    }
}

impl Material for OutlineMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://magicka_viewer/outline.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/outline.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        // Transparent items sort ascending; draw pass 1, then pass 2, last.
        if self.colour_pass {
            2.0e6
        } else {
            1.0e6
        }
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
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let OutlineKey {
            colour_pass,
            forced,
        } = key.bind_group_data;
        if let Some(fragment) = &mut descriptor.fragment {
            for target in fragment.targets.iter_mut().flatten() {
                // AlphaBlendEnable false; pass 1 has ColorWriteChannels 0.
                target.blend = None;
                target.write_mask = if colour_pass {
                    ColorWrites::ALL
                } else {
                    ColorWrites::empty()
                };
            }
        }
        if let Some(depth) = &mut descriptor.depth_stencil {
            let (compare, write) = pass_depth(colour_pass, forced);
            depth.depth_compare = Some(compare);
            depth.depth_write_enabled = Some(write);
        }
        Ok(())
    }
}

#[derive(Component)]
struct OutlinePass {
    colour_pass: bool,
}

/// The player's held items: outlined like the body, and (with
/// `NoDepthPrepass`) never occluders of the outline. INFERRED from the user's
/// observation of the original; OutlineRenderData itself draws only
/// Meshes[0].MeshParts[0] of the body.
#[derive(Component)]
pub struct OutlinedEquipment;

/// Marks a body part that already has outline passes.
#[derive(Component)]
struct OutlineSource;

#[derive(Resource)]
struct OutlineMaterials {
    /// [pass 1, pass 2] for normal and Forced submission.
    normal: [Handle<OutlineMaterial>; 2],
    forced: [Handle<OutlineMaterial>; 2],
}

pub struct OutlinePlugin;
impl Plugin for OutlinePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "outline.wgsl");
        if !app.is_plugin_added::<crate::effect_depth::EffectDepthPlugin>() {
            app.add_plugins(crate::effect_depth::EffectDepthPlugin);
        }
        app.init_resource::<AvatarOutline>()
            .add_plugins(MaterialPlugin::<OutlineMaterial>::default())
            .add_systems(
                Update,
                (create_materials, spawn_passes, update_passes).chain(),
            );
    }
}

fn material(
    lights: &crate::lighting::MagickaLights,
    colour: [f32; 3],
    opaque_depth: Handle<Image>,
    colour_pass: bool,
    forced: bool,
) -> OutlineMaterial {
    let (bloat, bias) = match (colour_pass, forced) {
        (false, false) => (PASS1_BLOAT, 0.0),
        (false, true) => (PASS1_BLOAT, FORCED_DEPTH_BIAS),
        (true, false) => (0.0, OUTLINE_DEPTH_BIAS),
        (true, true) => (0.0, 0.0),
    };
    OutlineMaterial {
        params: OutlineParams {
            color: Vec3::from_array(colour).extend(1.0),
            control: Vec4::new(
                bloat,
                depth_bias_coefficient(bias),
                1.0,
                // Non-forced passes require occlusion by prepass geometry.
                if forced { 0.0 } else { 1.0 },
            ),
        },
        lights: lights.buffer.clone(),
        opaque_depth,
        colour_pass,
        forced,
    }
}

fn create_materials(
    mut commands: Commands,
    existing: Option<Res<OutlineMaterials>>,
    lights: Option<Res<crate::lighting::MagickaLights>>,
    colour: Res<crate::robe::RobeColour>,
    depth: Res<crate::effect_depth::EffectDepth>,
    mut materials: ResMut<Assets<OutlineMaterial>>,
) {
    let Some(lights) = lights else { return };
    if let Some(existing) = existing {
        if colour.is_changed() {
            let rgb = Vec3::from_array(colour.0).extend(1.0);
            for handle in existing.normal.iter().chain(&existing.forced) {
                if let Some(mut m) = materials.get_mut(handle) {
                    m.params.color = rgb;
                }
            }
        }
        return;
    }
    let mut make = |pass, forced| {
        materials.add(material(
            &lights,
            colour.0,
            depth.image.clone(),
            pass,
            forced,
        ))
    };
    commands.insert_resource(OutlineMaterials {
        normal: [make(false, false), make(true, false)],
        forced: [make(false, true), make(true, true)],
    });
}

/// Model.Meshes[0].MeshParts[0] is the first primitive of the character
/// glTF's single skinned mesh (`character_scene::export_skinned`).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn spawn_passes(
    mut commands: Commands,
    server: Res<AssetServer>,
    materials: Option<Res<OutlineMaterials>>,
    roots: Query<&WorldAssetRoot, With<crate::play::AvatarRoot>>,
    parents: Query<&ChildOf>,
    parts: Query<
        (Entity, &Mesh3d, &SkinnedMesh, &Transform, &ChildOf),
        (Without<OutlineSource>, Without<OutlinePass>),
    >,
    equipment: Query<(), With<OutlinedEquipment>>,
    item_parts: Query<
        (Entity, &Mesh3d, &Transform, &ChildOf),
        (
            Without<SkinnedMesh>,
            Without<OutlineSource>,
            Without<OutlinePass>,
        ),
    >,
) {
    let Some(materials) = materials else { return };
    for (entity, mesh, transform, parent) in &item_parts {
        if !parents
            .iter_ancestors(entity)
            .any(|a| equipment.contains(a))
        {
            continue;
        }
        commands.entity(entity).insert(OutlineSource);
        for (i, colour_pass) in [false, true].into_iter().enumerate() {
            commands.spawn((
                OutlinePass { colour_pass },
                Mesh3d(mesh.0.clone()),
                *transform,
                MeshMaterial3d(materials.normal[i].clone()),
                Visibility::Hidden,
                ChildOf(parent.parent()),
            ));
        }
    }
    for (entity, mesh, skin, transform, parent) in &parts {
        let Some(root) = parents
            .iter_ancestors(entity)
            .find_map(|p| roots.get(p).ok())
        else {
            continue;
        };
        let (Some(mesh_path), Some(root_path)) =
            (server.get_path(&mesh.0), server.get_path(&root.0))
        else {
            continue;
        };
        if mesh_path.path() != root_path.path() || mesh_path.label() != Some("Mesh0/Primitive0") {
            continue;
        }
        commands.entity(entity).insert(OutlineSource);
        for (i, colour_pass) in [false, true].into_iter().enumerate() {
            commands.spawn((
                OutlinePass { colour_pass },
                Mesh3d(mesh.0.clone()),
                skin.clone(),
                *transform,
                MeshMaterial3d(materials.normal[i].clone()),
                NoFrustumCulling,
                Visibility::Hidden,
                ChildOf(parent.parent()),
            ));
        }
    }
}

fn update_passes(
    state: Res<AvatarOutline>,
    materials: Option<Res<OutlineMaterials>>,
    mut passes: Query<(
        &OutlinePass,
        &mut Visibility,
        &mut MeshMaterial3d<OutlineMaterial>,
    )>,
) {
    let Some(materials) = materials else { return };
    for (pass, mut visibility, mut handle) in &mut passes {
        let set = if state.forced {
            &materials.forced
        } else {
            &materials.normal
        };
        let wanted = &set[usize::from(pass.colour_pass)];
        if handle.0 != *wanted {
            handle.0 = wanted.clone();
        }
        visibility.set_if_neq(if state.submit {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_follows_avatar_update_gate() {
        let s =
            |dead, dnr, cut, timer, game| AvatarOutline::from_state(dead, dnr, cut, timer, game);
        assert_eq!(
            s(false, false, false, 0.0, 1),
            AvatarOutline {
                submit: true,
                forced: false
            }
        );
        assert!(!s(true, false, false, 0.0, 1).submit);
        assert!(!s(false, true, false, 0.0, 1).submit);
        assert!(!s(false, false, true, 0.0, 1).submit);
        // Invisible: Forced outline in Campaign, nothing in Versus.
        assert_eq!(
            s(false, false, false, 3.0, 1),
            AvatarOutline {
                submit: true,
                forced: true
            }
        );
        assert!(!s(false, false, false, 3.0, 4).submit);
        // NaN: `cgt.un` treats it as invisible for the gate, `cgt` as not Forced.
        assert_eq!(
            s(false, false, false, f32::NAN, 1),
            AvatarOutline {
                submit: true,
                forced: false
            }
        );
        assert!(!s(false, false, false, f32::NAN, 4).submit);
    }

    #[test]
    fn depth_bias_moves_toward_camera_by_conventional_depth() {
        let (n, f, z) = (CAMERA_NEAR, CAMERA_FAR, 225.0_f32);
        let d = |z: f32| f / (f - n) * (1.0 - n / z);
        let scale = 1.0 - depth_bias_coefficient(OUTLINE_DEPTH_BIAS) * z;
        // Reverse-Z depth near/z scaled by z/z' gives distance z'.
        let biased = z / scale;
        assert!((d(biased) - (d(z) + OUTLINE_DEPTH_BIAS)).abs() < 1e-6);
        assert!(biased < z);
        assert_eq!(depth_bias_coefficient(0.0), 0.0);
    }

    #[test]
    fn passes_translate_native_compare_functions_to_reverse_z() {
        assert_eq!(pass_depth(false, false), (CompareFunction::Always, true));
        assert_eq!(pass_depth(true, false), (CompareFunction::Less, false));
        assert_eq!(pass_depth(false, true), (CompareFunction::Always, true));
        assert_eq!(
            pass_depth(true, true),
            (CompareFunction::GreaterEqual, false)
        );
    }
}
