//! ElementalEgg.RenderData (research/summon-elemental.md): each registered
//! egg draws `elemental_mesh` with its own material at the runtime model
//! matrix, posed from the actor's sampled `spawn` clip (the mesh skeleton,
//! bound by bone name like Death's body).
use super::*;

const MESH: &str = "gltf/Models/Characters/Elemental/elemental_mesh.glb";

#[derive(Component)]
pub(super) struct EggModel {
    handle: u16,
}

fn frames(play: &Play) -> Vec<(u16, &magicka_runtime::elemental_egg_actor::EggFrame)> {
    let Some(cache) = play.session.world.elemental_eggs.as_ref() else {
        return Vec::new();
    };
    cache
        .slots
        .iter()
        .filter(|slot| slot.actor.registered)
        .filter_map(|slot| slot.frame.as_ref().map(|frame| (slot.handle, frame)))
        .collect()
}

pub(super) fn sync_eggs(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut roots: Query<(Entity, &EggModel, &mut Transform)>,
) {
    let frames = frames(&play);
    for (entity, model, mut transform) in &mut roots {
        match frames.iter().find(|(handle, _)| *handle == model.handle) {
            Some((_, frame)) => {
                *transform = Transform::from_matrix(Mat4::from_cols_array(&frame.model_parent));
            }
            None => commands.entity(entity).despawn(),
        }
    }
    for (handle, frame) in &frames {
        if roots.iter().any(|(_, model, _)| model.handle == *handle) {
            continue;
        }
        commands.spawn((
            EggModel { handle: *handle },
            Transform::from_matrix(Mat4::from_cols_array(&frame.model_parent)),
            Visibility::default(),
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(MESH))),
        ));
    }
}

/// Apply each egg's sampled split-bone pose after animation, before
/// transform propagation.
pub(super) fn pose_eggs(
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<&EggModel>,
    mut bones: Query<(Entity, &Name, &mut Transform)>,
) {
    let Some(cache) = play.session.world.elemental_eggs.as_ref() else {
        return;
    };
    if roots.is_empty() {
        return;
    }
    // Every slot shares one skeleton (Arc of the mesh skeleton).
    let skeleton = &cache.slots[0].actor.skeleton;
    let frames = frames(&play);
    for (entity, name, mut transform) in &mut bones {
        let Some(handle) = parents
            .iter_ancestors(entity)
            .find_map(|root| roots.get(root).ok().map(|model| model.handle))
        else {
            continue;
        };
        let Some((_, frame)) = frames.iter().find(|(h, _)| *h == handle) else {
            continue;
        };
        super::boss_models::apply_bone_pose(name, skeleton, &frame.local_poses, &mut transform);
    }
}
