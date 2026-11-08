//! Dedicated boss models consume the same poses used for damage zones.
use super::*;
#[derive(Component)]
pub(super) struct BossModel(u16);
pub(super) fn sync_models(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut roots: Query<(Entity, &BossModel, &mut Transform, &mut Visibility)>,
) {
    let bosses = &play.session.world.jormungandr;
    for (entity, model, mut transform, mut visibility) in &mut roots {
        if let Some(boss) = bosses.get(&model.0) {
            *transform = Transform::from_matrix(Mat4::from_cols_array(&boss.frame.parent));
            *visibility = if boss.frame.visible {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        } else {
            commands.entity(entity).despawn();
        }
    }
    for (&handle, boss) in bosses {
        if roots.iter().any(|(_, root, _, _)| root.0 == handle) {
            continue;
        }
        commands.spawn((
            BossModel(handle),
            Transform::from_matrix(Mat4::from_cols_array(&boss.frame.parent)),
            if boss.frame.visible {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
            WorldAssetRoot(
                assets.load(
                    GltfAssetLabel::Scene(0)
                        .from_asset("gltf/Models/Bosses/jormungandr/jormungandr.glb"),
                ),
            ),
        ));
    }
}
pub(super) fn pose_models(
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<&BossModel>,
    mut bones: Query<(Entity, &Name, &mut Transform)>,
) {
    for (entity, name, mut transform) in &mut bones {
        let Some(boss) = parents
            .iter_ancestors(entity)
            .find_map(|root| roots.get(root).ok())
            .and_then(|root| play.session.world.jormungandr.get(&root.0))
        else {
            continue;
        };
        apply_bone_pose(
            name,
            &boss.skeleton,
            &boss.frame.local_poses,
            &mut transform,
        );
    }
}

/// Write a sampled local pose onto an exported glTF bone: `<bone>.ts`
/// nodes carry translation and scale, `<bone>` nodes the rotation.
pub(super) fn apply_bone_pose(
    name: &Name,
    skeleton: &magicka_runtime::character_pose::CharacterSkeleton,
    local: &[magicka_runtime::character_pose::Pose],
    transform: &mut Transform,
) {
    let (bone, ts) = name
        .as_str()
        .strip_suffix(".ts")
        .map_or((name.as_str(), false), |bone| (bone, true));
    let Some(&(translation, rotation, scale)) = skeleton.bone(bone).and_then(|i| local.get(i))
    else {
        return;
    };
    if ts {
        transform.translation = Vec3::from_array(translation);
        transform.scale = Vec3::from_array(scale);
    } else {
        transform.rotation = Quat::from_array(rotation);
    }
}
