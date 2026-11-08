//! HaloAuraRenderData (research/aura-buffs.md): while a character has
//! auras, armor_aura's halo part is drawn additively as a camera-facing
//! billboard at the hip joint (skeleton bone 1) raised by −HeightOffset/2,
//! tinted with the cycled aura colour at alpha 0.666.
use super::{AvatarRoot, NpcRoot, Play};
use bevy::prelude::*;

#[derive(Component)]
pub(super) struct HaloDraw {
    handle: u16,
    material: Handle<crate::additive::AdditiveMaterial>,
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn sync_halos(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<crate::additive::AdditiveMaterial>>,
    mut draws: Query<(Entity, &HaloDraw, &mut Transform, &mut Visibility)>,
    roots: Query<
        (
            Entity,
            Option<&NpcRoot>,
            Option<&super::network::NetworkAvatar>,
            &GlobalTransform,
        ),
        Or<(
            With<NpcRoot>,
            With<AvatarRoot>,
            With<super::network::NetworkAvatar>,
        )>,
    >,
    children: Query<&Children>,
    skins: Query<&bevy::mesh::skinning::SkinnedMesh>,
    globals: Query<&GlobalTransform>,
    cameras: Query<&GlobalTransform, With<super::PlayCamera>>,
) {
    let world = &play.session.world;
    let avatar = play.session.avatar().handle;
    // Every wizard shown here (local, mirrored) and the NPCs.
    let character = |handle: u16| {
        world
            .players
            .iter()
            .flatten()
            .find(|p| p.handle == handle)
            .map(|p| &p.character)
            .or_else(|| world.npcs.get(&handle).map(|n| &n.character))
    };
    let Some(eye) = cameras.iter().next().map(|t| t.translation()) else {
        return;
    };
    let mut anchors = Vec::new();
    for (root, npc, remote, root_transform) in &roots {
        let handle = match (npc, remote) {
            (Some(n), _) => n.handle,
            (None, Some(r)) => match r.mirror_handle(&play) {
                Some(h) => h,
                None => continue,
            },
            (None, None) => avatar,
        };
        let Some(c) = character(handle) else {
            continue;
        };
        let Some(tint) = c.halo_draw else {
            continue;
        };
        // GetHipAttachOrientation: skeleton bone 1 (identity without one).
        let hip = children
            .iter_descendants(root)
            .find_map(|e| skins.get(e).ok())
            .and_then(|s| s.joints.get(1))
            .and_then(|j| globals.get(*j).ok())
            .map_or(root_transform.translation(), |g| g.translation());
        let d = &c.definition;
        let height_offset = -d.radius - 0.5 * d.length;
        anchors.push((handle, hip + Vec3::Y * (-height_offset * 0.5), tint));
    }
    for (entity, draw, mut transform, mut visibility) in &mut draws {
        match anchors.iter().find(|a| a.0 == draw.handle) {
            Some(&(_, at, tint)) => {
                *transform = Transform::from_translation(at).looking_at(eye, Vec3::Y);
                if let Some(mut m) = materials.get_mut(&draw.material) {
                    m.color = Vec4::from_array(tint);
                }
                *visibility = Visibility::Visible;
            }
            None if character(draw.handle).is_none() => commands.entity(entity).despawn(),
            None => *visibility = Visibility::Hidden,
        }
    }
    for &(handle, at, tint) in &anchors {
        if draws.iter().any(|(_, d, ..)| d.handle == handle) {
            continue;
        }
        let material = materials.add(crate::additive::AdditiveMaterial {
            color: Vec4::from_array(tint),
            flags: UVec4::new(1, 0, 0, 0),
            texture: Some(assets.load("textures/Models/Effects/armor_aura_0.png")),
        });
        commands.spawn((
            HaloDraw {
                handle,
                material: material.clone(),
            },
            Mesh3d(
                assets.load(
                    GltfAssetLabel::Primitive {
                        mesh: 0,
                        primitive: 1,
                    }
                    .from_asset("gltf/Models/Effects/armor_aura.glb"),
                ),
            ),
            MeshMaterial3d(material),
            Transform::from_translation(at).looking_at(eye, Vec3::Y),
            bevy::light::NotShadowCaster,
        ));
    }
}
