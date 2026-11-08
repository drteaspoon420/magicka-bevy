//! Camera-relative Grimnir apparition; native deferred shading is approximated.
use super::*;
use magicka_runtime::scene_world::{boss_live::BossRef, grimnir_live::GrimnirHost};
const MODEL: &str = "gltf/Models/Bosses/grimnir/grimnir_mesh.glb";
#[derive(Component)]
pub(super) struct GrimnirModel;
#[derive(Component)]
pub(super) struct GrimnirMaterial;
fn host(play: &Play) -> Option<&GrimnirHost> {
    play.session.world.boss_fight.bosses.iter().find_map(|b| {
        if let BossRef::Grimnir(g) = b {
            Some(g.as_ref())
        } else {
            None
        }
    })
}
pub(super) fn sync_models(
    mut commands: Commands,
    play: Res<Play>,
    audit: Res<boss_qa::Audit>,
    assets: Res<AssetServer>,
    mut roots: Query<(Entity, &mut Transform, &mut Visibility), With<GrimnirModel>>,
) {
    let Some(h) = host(&play) else {
        for (entity, _, _) in &roots {
            commands.entity(entity).despawn();
        }
        return;
    };
    let transform = Transform::from_matrix(Mat4::from_cols_array(&h.pose.parent));
    let visibility = if audit.grimnir_hidden {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    if let Some((_, mut t, mut v)) = roots.iter_mut().next() {
        *t = transform;
        *v = visibility;
    } else {
        commands.spawn((
            GrimnirModel,
            transform,
            visibility,
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(MODEL))),
        ));
    }
}
pub(super) fn tint_models(
    mut commands: Commands,
    parents: Query<&ChildOf>,
    roots: Query<(), With<GrimnirModel>>,
    meshes: Query<(Entity, &MeshMaterial3d<StandardMaterial>), Without<GrimnirMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, mesh) in &meshes {
        if !parents
            .iter_ancestors(entity)
            .any(|root| roots.contains(root))
        {
            continue;
        }
        let Some(mut material) = materials.get(&mesh.0).cloned() else {
            continue;
        };
        let base = material.base_color.to_linear();
        material.base_color = Color::linear_rgba(
            base.red,
            base.green * 1.6,
            base.blue * 2.,
            base.alpha * 0.65,
        );
        material.alpha_mode = AlphaMode::Add;
        commands.entity(entity).insert((
            MeshMaterial3d(materials.add(material)),
            GrimnirMaterial,
            crate::lighting::KeepStandardMaterial,
            bevy::light::NotShadowCaster,
        ));
    }
}
pub(super) fn pose_models(
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<GrimnirModel>>,
    mut bones: Query<(Entity, &Name, &mut Transform)>,
) {
    let Some(h) = host(&play) else { return };
    for (entity, name, mut transform) in &mut bones {
        if !parents
            .iter_ancestors(entity)
            .any(|root| roots.contains(root))
        {
            continue;
        }
        super::boss_models::apply_bone_pose(name, &h.skeleton, &h.pose.local, &mut transform);
    }
}
