//! Barrier.Initialize's magnitude-dependent detail/overlay material blend.
use super::*;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn apply_barrier_materials(
    mut commands: Commands,
    play: Res<Play>,
    server: Res<AssetServer>,
    parents: Query<&ChildOf>,
    parts: Query<&BarrierPart>,
    roots: Query<&BarrierRoot>,
    meshes: Query<(
        Entity,
        &MeshMaterial3d<StandardMaterial>,
        &bevy::gltf::GltfMaterialExtras,
    )>,
    standard: Res<Assets<StandardMaterial>>,
    mut robes: ResMut<Assets<crate::robe::RobeMaterial>>,
    lights: Res<crate::lighting::MagickaLights>,
    cubes: Res<crate::status_look::StatusCubes>,
    autopilot: Option<Res<Autopilot>>,
    converted: Query<(Entity, &MeshMaterial3d<crate::robe::RobeMaterial>)>,
) {
    let weak = autopilot.as_ref().is_some_and(|p| {
        p.started
            .is_some_and(|start| script_act(p.script, p.frame - start) == Some(Act::BarrierWeak))
    });
    for (entity, handle) in &converted {
        let Some(part) = parents
            .iter_ancestors(entity)
            .find_map(|a| parts.get(a).ok())
        else {
            continue;
        };
        let Ok(root) = roots.get(part.root) else {
            continue;
        };
        let Some(barrier) = play.session.world.barriers.get(root.slot) else {
            continue;
        };
        if let Some(mut material) = robes.get_mut(&handle.0) {
            material.extension.params.control.x = if weak {
                0.0
            } else {
                barrier.overlay_alpha(part.ice)
            };
        }
    }
    for (entity, handle, extras) in &meshes {
        let Some(part) = parents
            .iter_ancestors(entity)
            .find_map(|a| parts.get(a).ok())
        else {
            continue;
        };
        let Ok(root) = roots.get(part.root) else {
            continue;
        };
        let Some(barrier) = play.session.world.barriers.get(root.slot) else {
            continue;
        };
        let Some(base) = standard.get(&handle.0) else {
            continue;
        };
        let Ok(data) = serde_json::from_str::<serde_json::Value>(&extras.value) else {
            continue;
        };
        let Some(skinned) = data.get("magicka_skinned") else {
            continue;
        };
        let Some(mut extension) = crate::robe::skinned_extension(
            skinned, base, part.glb, &server, [1.0; 3], &lights, &cubes,
        ) else {
            continue;
        };
        extension.params.control.x = barrier.overlay_alpha(part.ice);
        // Each spawned mesh owns its material: different strengths coexist.
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(robes.add(crate::robe::RobeMaterial {
                base: base.clone(),
                extension,
            })));
    }
}
