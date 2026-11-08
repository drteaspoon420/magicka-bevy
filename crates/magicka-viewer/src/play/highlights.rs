//! Timed PhysicsEntity / AnimatedLevelPart AdditiveFresnel draws.
//! Native evidence and modern framebuffer limitations: castle-tutorial-feedback.md.
use super::{equipment::HeldItem, level_parts::LevelPartNode, Play};
use bevy::{
    asset::embedded_asset,
    light::{NotShadowCaster, NotShadowReceiver},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, CompareFunction,
        RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};
use magicka_runtime::{scene_loading::custom_hash, scene_world::campaign_live::Highlight};
use std::collections::HashMap;

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub(super) struct HighlightMaterial {
    #[uniform(0)]
    pub power: f32,
}
impl Material for HighlightMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/play/highlight.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
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
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
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
        Ok(())
    }
}
pub(super) fn plugin(app: &mut App) {
    embedded_asset!(app, "highlight.wgsl");
    app.add_plugins(MaterialPlugin::<HighlightMaterial>::default());
}

#[derive(Component)]
pub(super) struct HighlightDraw {
    source: Entity,
}
#[derive(Default)]
pub(super) struct Timers {
    scene: u32,
    active: Vec<(Highlight, f64)>,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn sync(
    mut commands: Commands,
    mut play: ResMut<Play>,
    mut timers: Local<Timers>,
    mut handles: Local<Option<[Handle<HighlightMaterial>; 2]>>,
    mut materials: ResMut<Assets<HighlightMaterial>>,
    meshes: Query<(Entity, &Mesh3d), Without<HighlightDraw>>,
    parents: Query<&ChildOf>,
    parts: Query<&LevelPartNode>,
    props: Query<&HeldItem>,
    mut draws: Query<(
        Entity,
        &HighlightDraw,
        &mut Visibility,
        &mut MeshMaterial3d<HighlightMaterial>,
    )>,
) {
    let now = play.session.world.play_time;
    if timers.scene != play.session.scene_changes {
        timers.active.clear();
        timers.scene = play.session.scene_changes;
    }
    timers.active.retain(|(_, end)| *end >= now);
    for request in std::mem::take(&mut play.session.world.highlights) {
        let time = match &request {
            Highlight::Phys { time, .. } | Highlight::Anim { time, .. } => *time,
        };
        timers.active.retain(|(old, _)| match (old, &request) {
            (Highlight::Phys { id: a, .. }, Highlight::Phys { id: b, .. }) => a != b,
            (Highlight::Anim { path: a, .. }, Highlight::Anim { path: b, .. }) => a != b,
            _ => true,
        });
        if time >= 0.0 {
            timers.active.push((request, now + f64::from(time)));
        }
    }
    let handles = handles.get_or_insert_with(|| {
        [
            materials.add(HighlightMaterial { power: 1.0 }),
            materials.add(HighlightMaterial { power: 2.0 }),
        ]
    });
    let mut wanted = HashMap::new();
    for (mesh, _) in &meshes {
        let mut node = mesh;
        loop {
            if let Ok(prop) = props.get(node) {
                if prop.slot == -302 {
                    if let Some(entity) = play
                        .session
                        .world
                        .entities
                        .iter()
                        .find(|e| e.handle == prop.owner)
                    {
                        if timers.active.iter().any(|(h, _)| matches!(h, Highlight::Phys { id, .. } if *id == entity.unique_id)) { wanted.insert(mesh, 1usize); }
                    }
                }
                break;
            }
            if let Ok(part) = parts.get(node) {
                if let Some(path) = &part.0 {
                    let path: Vec<_> = path
                        .iter()
                        .map(|s| custom_hash(&s.to_lowercase()))
                        .collect();
                    if timers
                        .active
                        .iter()
                        .any(|(h, _)| matches!(h, Highlight::Anim { path: p, .. } if *p == path))
                    {
                        wanted.insert(mesh, 0usize);
                    }
                }
                break;
            }
            let Ok(parent) = parents.get(node) else {
                break;
            };
            node = parent.parent();
        }
    }
    for (_, draw, mut visibility, mut material) in &mut draws {
        if let Some(power) = wanted.remove(&draw.source) {
            *visibility = Visibility::Inherited;
            material.0 = handles[power].clone();
        } else {
            *visibility = Visibility::Hidden;
        }
    }
    for (source, power) in wanted {
        let Ok((_, mesh)) = meshes.get(source) else {
            continue;
        };
        commands.spawn((
            HighlightDraw { source },
            mesh.clone(),
            MeshMaterial3d(handles[power].clone()),
            Transform::IDENTITY,
            Visibility::Inherited,
            NotShadowCaster,
            NotShadowReceiver,
            ChildOf(source),
        ));
    }
}
