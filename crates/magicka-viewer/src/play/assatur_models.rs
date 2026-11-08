//! Assatur's skinned body (RenderData) and staff (StaffRenderData), posed
//! from the runtime class. The damage-flash and frozen material looks are not
//! reconstructed yet.
use super::*;
use magicka_runtime::{boss_assatur::Assatur, scene_world::boss_live::BossRef};
const BODY: &str = "gltf/Models/Bosses/assatur/assatur.glb";
const STAFF: &str = "gltf/Models/Bosses/assatur/assatur_staff.glb";
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct AssaturModel(u8);
fn actor(play: &Play) -> Option<&Assatur> {
    play.session.world.boss_fight.bosses.iter().find_map(|b| {
        if let BossRef::Assatur(a) = b {
            Some(a.actor.as_ref())
        } else {
            None
        }
    })
}
/// Root transform: the body's parent, or the staff joint (the rail-aimed
/// orientation while the staff beam is cast).
fn root(a: &Assatur, part: u8) -> Mat4 {
    let m = if part == 0 {
        a.pose.parent
    } else {
        a.rail_staff.unwrap_or_else(|| a.staff())
    };
    Mat4::from_cols_array(&m)
}
pub(super) fn sync_models(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut roots: Query<(Entity, &AssaturModel, &mut Transform)>,
) {
    let Some(a) = actor(&play) else {
        for (e, _, _) in &roots {
            commands.entity(e).despawn();
        }
        return;
    };
    for (_, m, mut t) in &mut roots {
        *t = Transform::from_matrix(root(a, m.0));
    }
    for part in 0..2 {
        if roots.iter().any(|(_, m, _)| m.0 == part) {
            continue;
        }
        commands.spawn((
            AssaturModel(part),
            Transform::from_matrix(root(a, part)),
            Visibility::Inherited,
            WorldAssetRoot(
                assets.load(GltfAssetLabel::Scene(0).from_asset(if part == 0 {
                    BODY
                } else {
                    STAFF
                })),
            ),
        ));
    }
}
pub(super) fn pose_models(
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<&AssaturModel>,
    mut bones: Query<(Entity, &Name, &mut Transform), Without<AssaturModel>>,
) {
    let Some(a) = actor(&play) else { return };
    for (e, name, mut t) in &mut bones {
        let Some(part) = parents
            .iter_ancestors(e)
            .find_map(|p| roots.get(p).ok().map(|m| m.0))
        else {
            continue;
        };
        if part != 0 {
            continue;
        }
        super::boss_models::apply_bone_pose(name, &a.skeleton, &a.pose.local, &mut t);
    }
}
