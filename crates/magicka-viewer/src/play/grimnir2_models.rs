//! The physical Grimnir body, independently posed spirits, and Assatur.
use super::*;
use magicka_runtime::{boss_grimnir2::Grimnir2, scene_world::boss_live::BossRef};
const GRIMNIR: &str = "gltf/Models/Bosses/grimnir/grimnir_mesh.glb";
const ASSATUR: &str = "gltf/Models/Bosses/assatur/assatur.glb";
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Grimnir2Model(u8);
fn actor(play: &Play) -> Option<&Grimnir2> {
    play.session.world.boss_fight.bosses.iter().find_map(|b| {
        if let BossRef::Grimnir2(g) = b {
            Some(g.actor.as_ref())
        } else {
            None
        }
    })
}
fn sample(
    g: &Grimnir2,
    part: u8,
) -> (
    &magicka_runtime::boss_fafnir::ModelPose,
    &magicka_runtime::character_pose::CharacterSkeleton,
    bool,
    f32,
) {
    match part {
        0 => (&g.pose, &g.skeleton, false, 1.),
        1 | 2 => (
            &g.spirits[part as usize - 1].pose,
            &g.skeleton,
            true,
            g.spectral_alpha,
        ),
        _ => (
            &g.assatur_pose,
            &g.assatur_skeleton,
            !g.corporeal,
            if g.corporeal {
                1.
            } else {
                g.assatur_alpha * 0.25
            },
        ),
    }
}
pub(super) fn sync_models(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    audit: Res<boss_qa::Audit>,
    mut roots: Query<(Entity, &Grimnir2Model, &mut Transform, &mut Visibility)>,
) {
    let Some(g) = actor(&play) else {
        for (e, _, _, _) in &roots {
            commands.entity(e).despawn();
        }
        return;
    };
    let visible = |part: u8| {
        !audit.grimnir2_hidden
            && match part {
                0 => true,
                1 | 2 => g.spirits[part as usize - 1].drawn,
                _ => g.assatur_alpha > 0. || g.corporeal,
            }
    };
    for (_, m, mut t, mut v) in &mut roots {
        let (pose, _, _, _) = sample(g, m.0);
        *t = Transform::from_matrix(Mat4::from_cols_array(&pose.parent));
        *v = if visible(m.0) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for part in 0..4 {
        if roots.iter().any(|(_, m, _, _)| m.0 == part) {
            continue;
        }
        let (pose, _, _, _) = sample(g, part);
        commands.spawn((
            Grimnir2Model(part),
            Transform::from_matrix(Mat4::from_cols_array(&pose.parent)),
            if visible(part) {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
            WorldAssetRoot(
                assets.load(GltfAssetLabel::Scene(0).from_asset(if part == 3 {
                    ASSATUR
                } else {
                    GRIMNIR
                })),
            ),
        ));
    }
}
pub(super) fn materials(
    mut commands: Commands,
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<&Grimnir2Model>,
    meshes: Query<(Entity, &MeshMaterial3d<crate::lighting::PlayerLitMaterial>)>,
    mut materials: ResMut<Assets<crate::lighting::PlayerLitMaterial>>,
    cubes: Res<crate::status_look::StatusCubes>,
) {
    let Some(g) = actor(&play) else { return };
    for (e, h) in &meshes {
        let Some(part) = parents
            .iter_ancestors(e)
            .find_map(|p| roots.get(p).ok().map(|m| m.0))
        else {
            continue;
        };
        let Some(m) = materials.get(&h.0) else {
            continue;
        };
        let (_, _, ethereal, alpha) = sample(g, part);
        let frozen = part == 0 && g.status.has(magicka_sim::status::Status::Frozen);
        let look = crate::status_look::StatusLook {
            look: if frozen {
                Vec4::new(0., 0., 0.1, 1.)
            } else {
                Vec4::ZERO
            },
        };
        let mode = if ethereal {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        };
        if m.extension.params.maps.y != alpha
            || m.extension.params.maps.z != f32::from(u8::from(ethereal))
            || m.extension.look != look
            || m.base.alpha_mode != mode
            || (frozen && m.extension.ice.is_none() && cubes.ice.is_some())
        {
            let mut m = materials.get_mut(&h.0).unwrap();
            m.extension.params.maps.y = alpha;
            m.extension.params.maps.z = f32::from(u8::from(ethereal));
            m.extension.look = look;
            m.base.alpha_mode = mode;
            if frozen {
                m.extension.ice = cubes.ice.clone();
                m.extension.ice_normal = cubes.ice_normal.clone();
            }
        }
        if ethereal {
            commands.entity(e).insert(bevy::light::NotShadowCaster);
        } else {
            commands.entity(e).remove::<bevy::light::NotShadowCaster>();
        }
    }
}
pub(super) fn pose_models(
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<&Grimnir2Model>,
    mut bones: Query<(Entity, &Name, &mut Transform), Without<Grimnir2Model>>,
) {
    let Some(g) = actor(&play) else { return };
    for (e, name, mut t) in &mut bones {
        let Some(part) = parents
            .iter_ancestors(e)
            .find_map(|p| roots.get(p).ok().map(|m| m.0))
        else {
            continue;
        };
        let (pose, skeleton, _, _) = sample(g, part);
        super::boss_models::apply_bone_pose(name, skeleton, &pose.local, &mut t);
    }
}
