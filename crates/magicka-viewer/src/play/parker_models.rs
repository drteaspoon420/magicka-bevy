//! Parker (research/boss-parker.md): the posed parker glTF, the web
//! strands (SprayEntity.RenderData) and the Entangle_web model wrapped
//! around entangled characters (Entanglement.RenderData).
//!
//! Strands: a camera-facing Spray_web quad (x ±0.5, z 0..length) from the
//! strand toward its child or owner, alpha (1 − decay)^0.2, emissive 1,
//! drawn unlit with alpha blending.
//!
//! Entanglement (PROVEN, EntangleEffect.asm.txt shader 7): the pixel is
//! discarded unless texture alpha + AlphaBias − 0.001 ≥ 0 with AlphaBias =
//! visibility·0.5 − 1, so the web is an alpha-tested opaque mesh whose
//! cutoff falls as the magnitude grows. The root joint is the body
//! orientation scaled by (r·4/3, (r + l)·0.8, r·4/3) at the feet, the hip
//! joint the same basis at the character's hip attach.
use super::*;
use crate::text_box_visual::read_asset;
use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology};
use magicka_runtime::scene_world::boss_live::BossRef;

const MODEL: &str = "gltf/Models/Bosses/parker/parker.glb";
const ENTANGLE: &str = "gltf/Models/Effects/Entangle_web.glb";
/// Entangle_web's hip joint bind height.
const ENTANGLE_HIP_BIND: f32 = 1.168013;

#[derive(Component)]
pub(super) struct ParkerModel;
#[derive(Component)]
pub(super) struct WebStrandDraw(usize);
#[derive(Component)]
pub(super) struct EntangleModel {
    handle: u16,
    threshold: f32,
}
/// A mesh of an entangle model with its own alpha-tested material.
#[derive(Component)]
pub(super) struct EntangleMesh(Handle<StandardMaterial>);

fn host(play: &Play) -> Option<&magicka_runtime::scene_world::parker_live::ParkerHost> {
    play.session
        .world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            BossRef::Parker(p) => Some(p.as_ref()),
            _ => None,
        })
}

pub(super) fn sync_model(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut roots: Query<(Entity, &mut Transform), With<ParkerModel>>,
) {
    let Some(h) = host(&play) else {
        for (e, _) in &roots {
            commands.entity(e).despawn();
        }
        return;
    };
    let parent = Transform::from_matrix(Mat4::from_cols_array(&h.parker.pose.parent));
    if let Some((_, mut t)) = roots.iter_mut().next() {
        *t = parent;
        return;
    }
    commands.spawn((
        ParkerModel,
        parent,
        Visibility::default(),
        WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(MODEL))),
    ));
}

pub(super) fn pose_model(
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<ParkerModel>>,
    mut bones: Query<(Entity, &Name, &mut Transform), Without<ParkerModel>>,
) {
    let Some(h) = host(&play) else { return };
    let p = &h.parker;
    if p.pose.local.is_empty() {
        return;
    }
    for (entity, name, mut transform) in &mut bones {
        if parents.iter_ancestors(entity).any(|a| roots.get(a).is_ok()) {
            super::boss_models::apply_bone_pose(name, &p.skeleton, &p.pose.local, &mut transform);
        }
    }
}

pub(super) fn setup_strands(
    mut commands: Commands,
    play: Res<Play>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let texture = (|| -> Result<Handle<Image>, String> {
        let magicka_content::assets::Asset::Texture2D(t) = read_asset(
            &play
                .content
                .join(format!("{}.xnb", magicka_runtime::boss_parker::WEB_TEXTURE)),
        )?
        else {
            return Err("expected a Texture2D".into());
        };
        crate::play_hud::upload_texture(&t, &mut images)
    })()
    .map_err(|e| error!("web strands: {e}"))
    .ok();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    // SprayEntity's strip (1,1) (0,1) (0,0) (1,0) as two triangles.
    let corners = [
        ([0.5, 0.0, 0.0], [1.0, 1.0]),
        ([-0.5, 0.0, 0.0], [0.0, 1.0]),
        ([-0.5, 0.0, 1.0], [0.0, 0.0]),
        ([0.5, 0.0, 1.0], [1.0, 0.0]),
    ];
    let order = [0, 1, 2, 0, 2, 3];
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        order.iter().map(|&i| corners[i].0).collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        order.iter().map(|&i| corners[i].1).collect::<Vec<_>>(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 6]);
    let mesh = meshes.add(mesh);
    for slot in 0..magicka_runtime::web_strand::CACHE {
        commands.spawn((
            WebStrandDraw(slot),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color_texture: texture.clone(),
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                cull_mode: None,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
            bevy::light::NotShadowCaster,
        ));
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn present_strands(
    play: Res<Play>,
    camera: Query<&GlobalTransform, With<super::PlayCamera>>,
    mut strands: Query<(
        &WebStrandDraw,
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let pool = &play.session.world.web_strands;
    let eye = camera.iter().next().map(|c| c.translation());
    for (draw, mut transform, mut visibility, material) in &mut strands {
        let s = &pool.slots[draw.0];
        let (Some(tail), Some(eye), true) = (s.tail, eye, s.added && !s.dead) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let position = Vec3::from_array(s.position);
        let back = Vec3::from_array(tail) - position;
        let length = back.length();
        if length < 1e-4 {
            *visibility = Visibility::Hidden;
            continue;
        }
        // Forward = position − tail, so local +Z runs toward the tail.
        let forward = -back / length;
        let right = forward.cross(eye - position).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let mut m = Mat4::from_mat3(Mat3::from_cols(right, up, -forward * length));
        m.w_axis = position.extend(1.0);
        *transform = Transform::from_matrix(m);
        *visibility = Visibility::Visible;
        if let Some(mut mat) = materials.get_mut(&material.0) {
            mat.base_color = Color::srgba(1.0, 1.0, 1.0, s.alpha());
        }
    }
}

/// (handle, feet, yaw rotation, (r·4/3, (r + l)·0.8), visibility) of each
/// entangled or fading character.
fn entangled(play: &Play) -> Vec<(u16, Vec3, Quat, Vec2, f32)> {
    let world = &play.session.world;
    let players = world
        .players
        .iter()
        .flatten()
        .map(|p| (p.handle, p.body, &p.character));
    let npcs = world.npcs.iter().map(|(h, n)| (*h, n.body, &n.character));
    players
        .chain(npcs)
        .filter_map(|(handle, body, c)| {
            let visibility = c.entangle_visibility?;
            let b = world.physics.bodies.get(body)?;
            let (r, l) = (c.definition.radius, c.definition.length);
            let mut feet = Vec3::from_array(b.position);
            feet.y -= r + l * 0.5;
            let d = b.direction();
            let yaw = Quat::from_rotation_arc(
                Vec3::NEG_Z,
                Vec3::new(d[0], 0.0, d[2]).normalize_or(Vec3::NEG_Z),
            );
            Some((
                handle,
                feet,
                yaw,
                Vec2::new(r * 4.0 / 3.0, (r + l) * 0.8),
                visibility,
            ))
        })
        .collect()
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn sync_entangle(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut models: Query<(Entity, &mut EntangleModel, &mut Transform)>,
    characters: Query<(Entity, Option<&NpcRoot>), Or<(With<NpcRoot>, With<AvatarRoot>)>>,
    children: Query<&Children>,
    skins: Query<&bevy::mesh::skinning::SkinnedMesh>,
    globals: Query<&GlobalTransform>,
    mut bones: Query<(&Name, &mut Transform), Without<EntangleModel>>,
    model_children: Query<&Children, With<EntangleModel>>,
) {
    let avatar = play.session.avatar().handle;
    let hips: Vec<(u16, GlobalTransform)> = characters
        .iter()
        .filter_map(|(root, npc)| {
            let joint = children
                .iter_descendants(root)
                .find_map(|e| skins.get(e).ok())
                .and_then(|s| s.joints.get(1).copied())?;
            Some((npc.map_or(avatar, |n| n.handle), *globals.get(joint).ok()?))
        })
        .collect();
    let list = entangled(&play);
    for (entity, mut model, mut transform) in &mut models {
        let Some(&(_, feet, yaw, scale, visibility)) = list.iter().find(|e| e.0 == model.handle)
        else {
            commands.entity(entity).despawn();
            continue;
        };
        *transform = Transform::IDENTITY;
        model.threshold = 1.0 - 0.5 * visibility + 0.001;
        let basis =
            Mat3::from_quat(yaw) * Mat3::from_diagonal(Vec3::new(scale.x, scale.y, scale.x));
        // GetHipAttachOrientation × the web's hip inverse bind: translation
        // hip − 1.168·hip.up; the joint sits the bind height above it.
        let hip_attach = hips
            .iter()
            .find(|h| h.0 == model.handle)
            .map(|(_, g)| {
                let (_, rotation, translation) = g.to_scale_rotation_translation();
                translation - rotation * Vec3::Y * ENTANGLE_HIP_BIND
            })
            .unwrap_or(feet);
        let hip_world = hip_attach + basis * Vec3::Y * ENTANGLE_HIP_BIND;
        let hip_local = basis.inverse() * (hip_world - feet);
        let Ok(kids) = model_children.get(entity) else {
            continue;
        };
        for e in kids
            .iter()
            .flat_map(|k| std::iter::once(k).chain(children.iter_descendants(k)))
        {
            let Ok((name, mut t)) = bones.get_mut(e) else {
                continue;
            };
            match name.as_str() {
                "root.ts" => {
                    t.translation = feet;
                    t.scale = Vec3::new(scale.x, scale.y, scale.x);
                }
                "root" => t.rotation = yaw,
                "hips.ts" => {
                    t.translation = hip_local;
                    t.scale = Vec3::ONE;
                }
                "hips" => t.rotation = Quat::IDENTITY,
                _ => {}
            }
        }
    }
    for &(handle, _, _, _, visibility) in &list {
        if models.iter().any(|(_, m, _)| m.handle == handle) {
            continue;
        }
        commands.spawn((
            EntangleModel {
                handle,
                threshold: 1.0 - 0.5 * visibility + 0.001,
            },
            Transform::IDENTITY,
            Visibility::default(),
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(ENTANGLE))),
        ));
    }
}

/// Give each entangle mesh its own alpha-tested material (kept out of the
/// lit-material conversion) and track the model's cutoff.
#[allow(clippy::type_complexity)]
pub(super) fn entangle_materials(
    mut commands: Commands,
    parents: Query<&ChildOf>,
    models: Query<&EntangleModel>,
    fresh: Query<(Entity, &MeshMaterial3d<StandardMaterial>), Without<EntangleMesh>>,
    tracked: Query<(Entity, &EntangleMesh)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let model_of = |e: Entity| parents.iter_ancestors(e).find_map(|a| models.get(a).ok());
    for (entity, handle) in &fresh {
        if model_of(entity).is_none() {
            continue;
        }
        let Some(mut base) = materials.get(&handle.0).cloned() else {
            continue;
        };
        base.alpha_mode = AlphaMode::Mask(1.0);
        let own = materials.add(base);
        commands.entity(entity).insert((
            MeshMaterial3d(own.clone()),
            EntangleMesh(own),
            crate::lighting::KeepStandardMaterial,
        ));
    }
    for (entity, mesh) in &tracked {
        let Some(model) = model_of(entity) else {
            continue;
        };
        if let Some(m) = materials.get(&mesh.0) {
            if m.alpha_mode == AlphaMode::Mask(model.threshold) {
                continue;
            }
        }
        if let Some(mut m) = materials.get_mut(&mesh.0) {
            m.alpha_mode = AlphaMode::Mask(model.threshold);
        }
    }
}
