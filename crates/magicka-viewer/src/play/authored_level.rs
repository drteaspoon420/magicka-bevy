//! Render exactly the triangles used by authored scene collision.
use super::*;
use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology};
pub(super) fn is_authored(play: &Play) -> bool {
    play.session
        .model_path
        .extension()
        .is_some_and(|e| e == "arena")
}
pub(super) fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    play: &Play,
    assets: &AssetServer,
) {
    let level = &play.session.world.level;
    let visuals = std::fs::read(&play.session.model_path)
        .map_err(|e| e.to_string())
        .and_then(|b| magicka_runtime::authored_arena::presentation(&b));
    let visuals = match visuals {
        Ok(v) => v,
        Err(e) => {
            error!("authored presentation: {e}");
            return;
        }
    };
    for (layer, geometry) in level.collision_layers.iter().enumerate() {
        if geometry.triangles.is_empty() {
            continue;
        }
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uv = Vec::new();
        for (ti, t) in geometry.triangles.iter().enumerate() {
            if layer == 1 && visuals.hidden_boxes.contains(&(ti / 12)) {
                continue;
            }
            let p = t.map(|i| Vec3::from_array(geometry.vertices[i as usize]));
            let n = (p[1] - p[0])
                .cross(p[2] - p[0])
                .normalize_or_zero()
                .to_array();
            for v in p {
                positions.push(v.to_array());
                normals.push(n);
                uv.push([v.x, v.z]);
            }
        }
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        let material = StandardMaterial {
            base_color: if layer == 0 {
                Color::srgb(0.22, 0.28, 0.34)
            } else {
                Color::srgb(0.5, 0.35, 0.2)
            },
            perceptual_roughness: 0.9,
            ..default()
        };
        commands.spawn((
            LevelRoot,
            crate::lighting::OutlineOccluder,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(material)),
            Transform::IDENTITY,
        ));
    }
    let mut floor_layers = vec![magicka_content::level::TriMesh::default(); 10];
    floor_layers[0] = level.collision_layers[0].clone();
    let floor =
        magicka_runtime::level_collision::StaticLevelCollision::from_layers(&floor_layers).ok();
    for s in visuals.surfaces {
        use bevy::image::{
            ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor,
        };
        let texture = s.texture.map(|p| {
            assets
                .load_builder()
                .with_settings(|settings: &mut ImageLoaderSettings| {
                    settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                        address_mode_u: ImageAddressMode::Repeat,
                        address_mode_v: ImageAddressMode::Repeat,
                        ..ImageSamplerDescriptor::linear()
                    });
                })
                .load(p)
        });
        let [w, d] = s.size;
        let nx = (w / 2.).ceil() as usize;
        let nz = (d / 2.).ceil() as usize;
        let rotation = Quat::from_rotation_y(s.yaw);
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uv = Vec::new();
        // Drape every visual vertex onto the authoritative collision floor.
        for z in 0..nz {
            for x in 0..nx {
                let point = |i: usize, j: usize| {
                    let local = Vec3::new(
                        -w / 2. + w * i as f32 / nx as f32,
                        0.,
                        -d / 2. + d * j as f32 / nz as f32,
                    );
                    let mut local = local;
                    let wave = (local.z * 0.21).sin() * 0.65 + (local.z * 0.49).sin() * 0.35;
                    local.x *= 1. + s.edge * wave * 2. / w;
                    let world = Vec3::from_array(s.center) + rotation * local;
                    let height = floor
                        .as_ref()
                        .and_then(|f| {
                            f.segment_intersect(
                                magicka_sim::spray_pulse::Segment {
                                    origin: [world.x, 50., world.z],
                                    delta: [0., -100., 0.],
                                },
                                false,
                            )
                        })
                        .map_or(0., |hit| hit.hit.position[1]);
                    (
                        Vec3::new(world.x, height + s.center[1], world.z),
                        [(local.x + w / 2.) / s.tile, (local.z + d / 2.) / s.tile],
                    )
                };
                let p = [
                    point(x, z),
                    point(x, z + 1),
                    point(x + 1, z + 1),
                    point(x + 1, z),
                ];
                for tri in [[0, 1, 2], [0, 2, 3]] {
                    let n = (p[tri[1]].0 - p[tri[0]].0)
                        .cross(p[tri[2]].0 - p[tri[0]].0)
                        .normalize_or_zero()
                        .to_array();
                    for i in tri {
                        positions.push(p[i].0.to_array());
                        normals.push(n);
                        uv.push(p[i].1);
                    }
                }
            }
        }
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        commands.spawn((
            LevelRoot,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(s.color[0], s.color[1], s.color[2]),
                base_color_texture: texture,
                perceptual_roughness: 0.95,
                ..default()
            })),
            Transform::IDENTITY,
        ));
    }
    for liquid in &level.liquids {
        let magicka_content::level::LiquidMaterial::Water(effect) = &liquid.material else {
            continue;
        };
        let Ok(geometry) = liquid.collision_geometry() else {
            continue;
        };
        let mut positions = Vec::new();
        let mut colors = Vec::new();
        let mut uv = Vec::new();
        for t in &geometry.triangles {
            for i in t {
                let p = geometry.vertices[*i as usize];
                positions.push(p);
                uv.push([p[0] / 6., p[2] / 6.]);
                let deep = if visuals.terrain.deep(p[0], p[2]) {
                    1.
                } else {
                    0.
                };
                colors.push([deep, deep, deep, 1.]);
            }
        }
        let count = positions.len();
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 1., 0.]; count])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0., 0.]; count])
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        commands.spawn((
            LevelRoot,
            Mesh3d(meshes.add(mesh)),
            AuthoredWater((**effect).clone()),
            Transform::IDENTITY,
        ));
    }
    for s in visuals.scenery {
        commands.spawn((
            LevelRoot,
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(s.asset))),
            Transform::from_translation(Vec3::from_array(s.position))
                .with_scale(Vec3::from_array(s.scale))
                .with_rotation(Quat::from_rotation_y(s.yaw)),
        ));
    }
}

#[derive(Component)]
pub(super) struct AuthoredWater(magicka_content::xna::LiquidEffect);
/// Use the ordinary water shader and freeze-stream synchronization for authored water.
pub(super) fn water_materials(
    mut commands: Commands,
    assets: Res<AssetServer>,
    lights: Res<crate::lighting::MagickaLights>,
    mut materials: ResMut<Assets<crate::liquid::LiquidMaterial>>,
    pending: Query<(Entity, &AuthoredWater), Added<AuthoredWater>>,
) {
    for (entity, AuthoredWater(e)) in &pending {
        let d = serde_json::json!({
            "bottom_color":e.bottom_color,"deep_bottom_color":e.deep_bottom_color,
            "wave_height":e.wave_height,"wave_speed0":e.wave_speed0,"wave_speed1":e.wave_speed1,
            "reflectiveness":e.water_reflectiveness,"emissive":e.water_emissive,
            "spec_amount":e.water_spec_amount,"spec_power":e.water_spec_power,
            "ice_color":e.ice_color,"bottom_texture":e.bottom_texture,
            "normal_map":e.water_normal_map,"ice_diffuse_map":e.ice_diffuse_map,
        });
        let material = crate::liquid::from_extras(&d, &assets, lights.buffer.clone(), |p| {
            format!("textures/{p}.png")
        });
        commands.entity(entity).insert((
            MeshMaterial3d(materials.add(material)),
            crate::liquid::LiquidSurface,
        ));
    }
}
