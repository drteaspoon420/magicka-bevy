//! Cthulhu (research/boss-cthulhu.md): the posed cthulhu_mesh glTF (drawn
//! at Scale(1.25)·mTransform) and its four tentacles, each posed from its
//! own BossRig. Tentacles are drawn once activated (they stay in the
//! entity manager, submerged or not). The bolts, mist and water effects are
//! tracked scene effects.
use super::*;
use magicka_runtime::{boss_rig::BossRig, scene_world::boss_live::BossRef};

const BODY: &str = "gltf/Models/Bosses/cthulhu/cthulhu_mesh.glb";
const TENTACLE: &str = "gltf/Models/Bosses/cthulhu/tentacle_mesh.glb";

/// 0: the body; 1..=4: tentacle index + 1.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(super) struct CthulhuModel(u8);

fn host(play: &Play) -> Option<&magicka_runtime::scene_world::cthulhu_live::CthulhuHost> {
    play.session
        .world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            BossRef::Cthulhu(c) => Some(c.as_ref()),
            _ => None,
        })
}

/// The rig drawn for a part, if it is drawn.
fn rig(play: &Play, part: u8) -> Option<&BossRig> {
    let c = &host(play)?.cthulhu;
    match part {
        0 => Some(&c.rig),
        i => c
            .tentacles
            .get(i as usize - 1)
            .filter(|t| t.active)
            .map(|t| &t.rig),
    }
}

pub(super) fn sync_models(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut roots: Query<(Entity, &CthulhuModel, &mut Transform, &mut Visibility)>,
) {
    if host(&play).is_none() {
        for (e, ..) in &roots {
            commands.entity(e).despawn();
        }
        return;
    }
    for (_, part, mut t, mut v) in &mut roots {
        match rig(&play, part.0).filter(|r| !r.pose.local.is_empty()) {
            Some(r) => {
                *t = Transform::from_matrix(Mat4::from_cols_array(&r.pose.parent));
                *v = Visibility::Inherited;
            }
            None => *v = Visibility::Hidden,
        }
    }
    for part in 0..5u8 {
        if roots.iter().any(|(_, p, ..)| p.0 == part) {
            continue;
        }
        let Some(r) = rig(&play, part).filter(|r| !r.pose.local.is_empty()) else {
            continue;
        };
        commands.spawn((
            CthulhuModel(part),
            Transform::from_matrix(Mat4::from_cols_array(&r.pose.parent)),
            Visibility::default(),
            WorldAssetRoot(
                assets.load(GltfAssetLabel::Scene(0).from_asset(if part == 0 {
                    BODY
                } else {
                    TENTACLE
                })),
            ),
        ));
    }
}

pub(super) fn pose_models(
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<&CthulhuModel>,
    mut bones: Query<(Entity, &Name, &mut Transform), Without<CthulhuModel>>,
) {
    if host(&play).is_none() {
        return;
    }
    for (entity, name, mut transform) in &mut bones {
        let Some(part) = parents
            .iter_ancestors(entity)
            .find_map(|a| roots.get(a).ok().copied())
        else {
            continue;
        };
        if let Some(r) = rig(&play, part.0) {
            super::boss_models::apply_bone_pose(name, &r.skeleton, &r.pose.local, &mut transform);
        }
    }
}
