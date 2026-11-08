//! Fafnir (research/boss-battles.md): the posed fafnir_mesh glTF and the
//! fire lance (Fafnir.FireRenderData): two additive EffectTextures/FireLance02
//! layers on a quad from the mouth along the clipped lance, turned to face
//! the camera around the beam axis, scrolling at (0, t·10) and (0.5, t·4)
//! with the texture repeated along the length. INFERRED: the beam length is
//! the clipped segment (the native world matrix's ×16 length factor is not
//! applied); the damage texture blend and flash are not drawn.
use super::*;
use crate::text_box_visual::read_asset;
use bevy::{asset::RenderAssetUsages, math::Affine2, mesh::PrimitiveTopology};
use magicka_runtime::scene_world::boss_live::BossRef;

const MODEL: &str = "gltf/Models/Bosses/fafnir/fafnir_mesh.glb";

#[derive(Component)]
pub(super) struct FafnirModel;
#[derive(Component)]
pub(super) struct FafnirLance(usize);

fn host(play: &Play) -> Option<&magicka_runtime::scene_world::fafnir_live::FafnirHost> {
    play.session
        .world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            BossRef::Fafnir(f) => Some(f.as_ref()),
            _ => None,
        })
}

pub(super) fn sync_model(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut roots: Query<(Entity, &mut Transform), With<FafnirModel>>,
) {
    let Some(h) = host(&play) else {
        for (e, _) in &roots {
            commands.entity(e).despawn();
        }
        return;
    };
    let parent = Transform::from_matrix(Mat4::from_cols_array(&h.fafnir.pose.parent));
    if let Some((_, mut t)) = roots.iter_mut().next() {
        *t = parent;
        return;
    }
    commands.spawn((
        FafnirModel,
        parent,
        Visibility::default(),
        WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(MODEL))),
    ));
}

pub(super) fn pose_model(
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<FafnirModel>>,
    mut bones: Query<(Entity, &Name, &mut Transform), Without<FafnirModel>>,
) {
    let Some(h) = host(&play) else { return };
    let f = &h.fafnir;
    if f.pose.local.is_empty() {
        return;
    }
    for (entity, name, mut transform) in &mut bones {
        if !parents.iter_ancestors(entity).any(|p| roots.get(p).is_ok()) {
            continue;
        }
        super::boss_models::apply_bone_pose(name, &f.skeleton, &f.pose.local, &mut transform);
    }
}

pub(super) fn setup_lance(
    mut commands: Commands,
    play: Res<Play>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let result = (|| -> Result<(), String> {
        let magicka_content::assets::Asset::Texture2D(t) =
            read_asset(&play.content.join("EffectTextures/FireLance02.xnb"))?
        else {
            return Err("expected EffectTextures/FireLance02".into());
        };
        let mut texture = crate::play_hud::upload_texture(&t, &mut images)?;
        if let Some(mut image) = images.get_mut(&texture) {
            image.sampler =
                bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
                    address_mode_u: bevy::image::ImageAddressMode::Repeat,
                    address_mode_v: bevy::image::ImageAddressMode::Repeat,
                    ..bevy::image::ImageSamplerDescriptor::linear()
                });
        } else {
            texture = Handle::default();
        }
        // Unit quad: x ±0.5, z 0..1 (u 0..0.5, v 0..1).
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [-0.5, 0.0, 1.0],
                [-0.5, 0.0, 0.0],
                [0.5, 0.0, 0.0],
                [0.5, 0.0, 0.0],
                [0.5, 0.0, 1.0],
                [-0.5, 0.0, 1.0],
            ],
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![
                [0.0, 1.0],
                [0.0, 0.0],
                [0.5, 0.0],
                [0.5, 0.0],
                [0.5, 1.0],
                [0.0, 1.0],
            ],
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 6]);
        let mesh = meshes.add(mesh);
        for layer in 0..2 {
            commands.spawn((
                FafnirLance(layer),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color_texture: Some(texture.clone()),
                    unlit: true,
                    alpha_mode: AlphaMode::Add,
                    cull_mode: None,
                    ..default()
                })),
                Transform::default(),
                Visibility::Hidden,
                bevy::light::NotShadowCaster,
            ));
        }
        Ok(())
    })();
    if let Err(e) = result {
        error!("fafnir lance: {e}");
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn present_lance(
    play: Res<Play>,
    camera: Query<&GlobalTransform, With<super::PlayCamera>>,
    mut lances: Query<(
        &FafnirLance,
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let beam = host(&play)
        .filter(|h| h.fafnir.draw_lance)
        .map(|h| h.fafnir.lance);
    let eye = camera.iter().next().map(|c| c.translation());
    for (layer, mut transform, mut visibility, material) in &mut lances {
        let (Some((origin, delta)), Some(eye)) = (beam, eye) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let origin = Vec3::from_array(origin);
        let delta = Vec3::from_array(delta);
        let length = delta.length();
        if length < 1e-3 {
            *visibility = Visibility::Hidden;
            continue;
        }
        *visibility = Visibility::Visible;
        let forward = delta / length;
        let right = forward.cross(eye - origin).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        // Local x → right (width 2), y → up, z → the beam.
        let basis = Mat3::from_cols(right * 2.0, up, forward * length);
        *transform = Transform::from_matrix(Mat4::from_mat3(basis).with_translation_vec(origin));
        if let Some(mut m) = materials.get_mut(&material.0) {
            let time = play.session.world.play_time as f32;
            let offset = if layer.0 == 0 {
                Vec2::new(0.0, time * 10.0)
            } else {
                Vec2::new(0.5, time * 4.0)
            };
            m.uv_transform =
                Affine2::from_scale_angle_translation(Vec2::new(1.0, length), 0.0, offset);
        }
    }
}

trait WithTranslation {
    fn with_translation_vec(self, t: Vec3) -> Mat4;
}
impl WithTranslation for Mat4 {
    fn with_translation_vec(mut self, t: Vec3) -> Mat4 {
        self.w_axis = t.extend(1.0);
        self
    }
}
