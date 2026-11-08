//! Bodies and held models consume the same native channel poses.
use super::*;
use magicka_content::{mesh::resolve_reference, Content};
use magicka_runtime::{character_pose::CharacterSkeleton, play_content};

const ICE_BLADE: &str = "Models/Effects/Ice_Blade";
/// Pseudo equipment slot for the blade's held-model entry.
const ICE_BLADE_SLOT: i32 = -1;

/// Test harness: the paired capture's hidden-blade window.
fn ice_blade_hidden(pilot: Option<&Autopilot>) -> bool {
    pilot.is_some_and(|p| {
        p.started
            .is_some_and(|start| script_act(p.script, p.frame - start) == Some(Act::IceBladeHidden))
    })
}

/// IceBlade.TrailRenderData: the trail pairs as a strip (both windings,
/// CullMode None), u = (i/2)/15, v = i mod 2, AdditiveEffect with the
/// EffectTextures/IceBlade texture and tint 1.
fn trail_mesh(trail: &[[f32; 3]; magicka_runtime::avatar::ICE_BLADE_TRAIL]) -> Mesh {
    trail_mesh_from(trail)
}

fn trail_mesh_from(trail: &[[f32; 3]]) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let n = trail.len() as u32;
    let uvs: Vec<[f32; 2]> = (0..n)
        .map(|i| [(i / 2) as f32 / 15.0, (i % 2) as f32])
        .collect();
    let mut indices = Vec::new();
    for i in 0..n - 2 {
        indices.extend([i, i + 1, i + 2, i + 2, i + 1, i]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, trail.to_vec())
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![[0.0f32, 1.0, 0.0]; trail.len()],
    )
    .with_inserted_indices(Indices::U32(indices))
}

/// One wizard's IceBlade trail (owner handle): local or mirrored.
#[derive(Component)]
pub(super) struct IceBladeTrail(u16);

pub(super) fn sync_ice_blade_trail(
    mut commands: Commands,
    play: Res<Play>,
    server: Res<AssetServer>,
    pilot: Option<Res<Autopilot>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::additive::AdditiveMaterial>>,
    mut trails: Query<(&IceBladeTrail, &Mesh3d, &mut Visibility)>,
) {
    // Every wizard shown here: the local simulation's and the host's
    // records for mirrored wizards (PlaySession::ice_blade_trails).
    let mut pending = if ice_blade_hidden(pilot.as_deref()) {
        Vec::new()
    } else {
        play.session.ice_blade_trails()
    };
    for (draw, mesh, mut visibility) in &mut trails {
        match pending.iter().position(|(owner, _)| *owner == draw.0) {
            Some(i) => {
                let (_, trail) = pending.swap_remove(i);
                meshes.insert(&mesh.0, trail_mesh(&trail)).ok();
                *visibility = Visibility::Visible;
            }
            None => *visibility = Visibility::Hidden,
        }
    }
    for (owner, trail) in pending {
        commands.spawn((
            IceBladeTrail(owner),
            Mesh3d(meshes.add(trail_mesh(&trail))),
            MeshMaterial3d(materials.add(crate::additive::AdditiveMaterial {
                color: Vec4::ONE,
                flags: UVec4::new(1, 0, 0, 0),
                texture: Some(server.load("textures/EffectTextures/IceBlade.png")),
            })),
            Transform::IDENTITY,
            Visibility::Visible,
            bevy::light::NotShadowCaster,
            bevy::camera::visibility::NoFrustumCulling,
        ));
    }
}

/// ArcaneBlade.RenderData: ArcaneEffect technique 0 over the 32-vertex strip
/// (both windings for CullMode None), u = (i/2)/15, v = i mod 2, colour
/// centre = colour + 1, edge = colour, alpha 1, EffectTextures/ArcaneBlade.
#[derive(Component)]
pub(super) struct ArcaneBladeDraw {
    owner: u16,
    id: u32,
    material: Handle<crate::beam::BeamMaterial>,
}

fn arcane_blade_mesh(vertices: &[[f32; 3]]) -> Mesh {
    let mut mesh = trail_mesh_from(vertices);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vec![[1.0f32, 1.0, 1.0, 1.0]; vertices.len()],
    );
    mesh
}

#[allow(clippy::too_many_arguments)]
pub(super) fn sync_arcane_blades(
    mut commands: Commands,
    play: Res<Play>,
    server: Res<AssetServer>,
    mut texture: Local<Option<Handle<Image>>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::beam::BeamMaterial>>,
    mut draws: Query<(Entity, &ArcaneBladeDraw, &Mesh3d, &mut Visibility)>,
    pilot: Option<Res<Autopilot>>,
) {
    let hidden = ice_blade_hidden(pilot.as_deref());
    use bevy::image::{ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
    let texture = texture
        .get_or_insert_with(|| {
            // TextureSamplerClamped: linear, clamp.
            server
                .load_builder()
                .with_settings(|s: &mut ImageLoaderSettings| {
                    s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
                })
                .load("textures/EffectTextures/ArcaneBlade.png")
        })
        .clone();
    // Every wizard shown here: the local simulation's blades and the
    // host's records for mirrored wizards (PlaySession::arcane_blade_draws).
    let blades = play.session.arcane_blade_draws();
    let color = |c: [f32; 3]| Vec4::new(c[0], c[1], c[2], 1.0);
    for (entity, draw, mesh, mut visibility) in &mut draws {
        *visibility = if hidden {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        match blades
            .iter()
            .find(|b| b.owner == draw.owner && b.id == draw.id)
        {
            Some(blade) => {
                meshes
                    .insert(&mesh.0, arcane_blade_mesh(&blade.vertices))
                    .ok();
                if let Some(mut m) = materials.get_mut(&draw.material) {
                    m.color_center = color(blade.center);
                    m.color_edge = color(blade.edge);
                }
            }
            None => commands.entity(entity).despawn(),
        }
    }
    for blade in &blades {
        if draws
            .iter()
            .any(|(_, d, _, _)| d.owner == blade.owner && d.id == blade.id)
        {
            continue;
        }
        let material = materials.add(crate::beam::BeamMaterial {
            color_center: color(blade.center),
            color_edge: color(blade.edge),
            texture: texture.clone(),
        });
        commands.spawn((
            ArcaneBladeDraw {
                owner: blade.owner,
                id: blade.id,
                material: material.clone(),
            },
            Mesh3d(meshes.add(arcane_blade_mesh(&blade.vertices))),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::Visible,
            bevy::light::NotShadowCaster,
            bevy::camera::visibility::NoFrustumCulling,
        ));
    }
}

/// Item.Update: EmissiveMultiplyer = min(0.05^cooldown, 1) while an avatar
/// item's special ability recharges (research/staff-items.md). The original
/// EmissiveAmount of each lit material is kept to scale from.
pub(super) fn dim_recharging_items(
    play: Res<Play>,
    items: Query<(Entity, &HeldItem)>,
    children: Query<&Children>,
    lit: Query<&MeshMaterial3d<crate::lighting::LitMaterial>>,
    mut materials: ResMut<Assets<crate::lighting::LitMaterial>>,
    mut original: Local<HashMap<AssetId<crate::lighting::LitMaterial>, f32>>,
) {
    for (entity, held) in &items {
        let cooldown = play
            .session
            .world
            .players
            .iter()
            .flatten()
            .find(|p| p.handle == held.owner)
            .and_then(|p| p.items.iter().find(|(slot, _)| *slot == held.slot))
            .filter(|(_, item)| item.definition.special_ability.is_some())
            .map_or(0.0, |(_, item)| item.special_cooldown);
        let multiplier = if cooldown > 0.0 {
            0.05f32.powf(cooldown).min(1.0)
        } else {
            1.0
        };
        for child in children.iter_descendants(entity) {
            let Ok(handle) = lit.get(child) else {
                continue;
            };
            let id = handle.id();
            let Some(material) = materials.get(id) else {
                continue;
            };
            let base = *original
                .entry(id)
                .or_insert(material.extension.params.material.z);
            let value = base * multiplier;
            if material.extension.params.material.z != value {
                if let Some(mut material) = materials.get_mut(id) {
                    material.extension.params.material.z = value;
                }
            }
        }
    }
}

#[derive(Component)]
pub(super) struct HeldItem {
    pub(super) owner: u16,
    pub(super) slot: i32,
    path: String,
}

pub(super) fn exported(
    dir: &std::path::Path,
    root: &std::path::Path,
    files: &mut HashMap<String, String>,
) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            exported(&path, root, files);
        } else if path.extension().is_some_and(|e| e == "glb") {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(rel.to_ascii_lowercase(), rel);
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn sync(
    mut commands: Commands,
    play: Res<Play>,
    server: Res<AssetServer>,
    mut items: Query<(Entity, &HeldItem, &mut Transform)>,
    mut files: Local<Option<HashMap<String, String>>>,
    mut templates: Local<HashMap<String, Vec<(i32, String)>>>,
    pilot: Option<Res<Autopilot>>,
    network: Option<NonSend<super::network::NetworkPlay>>,
) {
    let blade_hidden = ice_blade_hidden(pilot.as_deref());
    let files = files.get_or_insert_with(|| {
        let mut files = HashMap::new();
        exported(&play.cache.join("gltf"), &play.cache, &mut files);
        files
    });
    let mut desired = HashMap::new();
    // A dormant avatar (an observer client's slot 0) shows no equipment.
    for avatar in play
        .session
        .world
        .players
        .iter()
        .flatten()
        .filter(|a| a.playing)
    {
        if let Some(rig) = &avatar.rig {
            let body = &play.session.world.physics.bodies[avatar.body];
            let parent = magicka_runtime::character_pose::character_orientation(
                &avatar
                    .character
                    .scaled_static_transform(&rig.static_transform),
                &body.orientation,
                body.position,
                -body.radius - 0.5 * body.length,
            );
            let pose = rig.skeleton.skinned_for(&avatar.motion.controller, &parent);
            for (slot, item) in &avatar.items {
                if !item.attached {
                    continue;
                }
                if let Some(model) = &item.definition.model {
                    let model = resolve_reference(
                        &format!("Data/items/Wizard/{}", item.definition.name),
                        model,
                    );
                    if let (Some(path), Some(a)) = (
                        files.get(&format!("gltf/{model}.glb").to_ascii_lowercase()),
                        rig.attachments.iter().find(|a| a.slot == *slot),
                    ) {
                        desired.insert(
                            (avatar.handle, *slot),
                            (path.clone(), a.item_transform(&pose)),
                        );
                    }
                }
            }
            // IceBlade.Update: the Ice_Blade model at Equipment[0]'s orientation
            // with its 3×3 part scaled by range / attach0.Y.
            if let (Some(range), Some(path), Some(a)) = (
                avatar.ice_blade_range().filter(|_| !blade_hidden),
                files.get(&format!("gltf/{ICE_BLADE}.glb").to_ascii_lowercase()),
                rig.attachments.iter().find(|a| a.slot == 0),
            ) {
                let mut m = a.item_transform(&pose);
                let scale = range / magicka_runtime::avatar::ICE_BLADE_ATTACH_Y;
                for i in [0, 1, 2, 4, 5, 6, 8, 9, 10] {
                    m[i] *= scale;
                }
                desired.insert((avatar.handle, ICE_BLADE_SLOT), (path.clone(), m));
            }
        }
    }
    for entity in &play.session.world.entities {
        if !entity.added || entity.removed || play.session.world.overkilled.contains(&entity.handle)
        {
            continue;
        }
        let Some(npc) = play.session.world.npcs.get(&entity.handle) else {
            continue;
        };
        let (Some(rig), Some(name)) = (&npc.rig, &entity.template_name) else {
            continue;
        };
        let paths = templates.entry(name.clone()).or_insert_with(|| {
            let Ok(template) = play_content::load_character(&play.content, name) else {
                return Vec::new();
            };
            template
                .equipment
                .iter()
                .filter_map(|(slot, reference)| {
                    let key = resolve_reference(
                        &format!("Data/Characters/{name}"),
                        reference.item.as_ref()?,
                    );
                    let path = play_content::resolve(
                        &play.content,
                        &std::path::PathBuf::from(format!("{key}.xnb")),
                    )?;
                    let bytes = std::fs::read(path).ok()?;
                    let xnb = magicka_xnb::xnb::parse(&bytes).ok()?;
                    let Content::Item(item) = magicka_content::parse_primary(&xnb)?.ok()?.content
                    else {
                        return None;
                    };
                    let model = resolve_reference(&key, item.model.as_ref()?);
                    Some((
                        *slot,
                        files
                            .get(&format!("gltf/{model}.glb").to_ascii_lowercase())?
                            .clone(),
                    ))
                })
                .collect()
        });
        let body = &play.session.world.physics.bodies[npc.body];
        let parent = magicka_runtime::character_pose::character_orientation(
            &npc.character.scaled_static_transform(&rig.static_transform),
            &body.orientation,
            body.position,
            -body.radius - 0.5 * body.length,
        );
        let pose = rig.skeleton.skinned_for(&npc.motion.controller, &parent);
        for (slot, path) in paths.iter() {
            if let Some(a) = rig.attachments.iter().find(|a| a.slot == *slot) {
                desired.insert(
                    (entity.handle, *slot),
                    (path.clone(), a.item_transform(&pose)),
                );
            }
        }
    }
    for entity in &play.session.world.entities {
        if !entity.added || entity.removed || entity.dead {
            continue;
        }
        let Some(pickup) = play.session.world.item_pickups.get(&entity.handle) else {
            continue;
        };
        let Some(item) = &pickup.item else {
            continue;
        };
        if item.definition.hide_model {
            continue;
        }
        let Some(model) = &item.definition.model else {
            continue;
        };
        let model = resolve_reference(
            &format!("Data/items/Wizard/{}", item.definition.name),
            model,
        );
        if let Some(path) = files.get(&format!("gltf/{model}.glb").to_ascii_lowercase()) {
            desired.insert((entity.handle, -300), (path.clone(), pickup.transform));
        }
    }
    for entity in &play.session.world.entities {
        if !entity.added || entity.removed || entity.dead {
            continue;
        }
        let Some(book) = play.session.world.book_pickups.get(&entity.handle) else {
            continue;
        };
        if book.dead {
            continue;
        }
        if let Some(path) = files.get(
            &format!("gltf/{}.glb", magicka_runtime::magick_books::BOOK_MODEL).to_ascii_lowercase(),
        ) {
            desired.insert((entity.handle, -301), (path.clone(), book.transform));
        }
    }
    for entity in &play.session.world.entities {
        if !entity.added
            || entity.removed
            || entity.dead
            || play.session.world.item_pickups.contains_key(&entity.handle)
            || play.session.world.book_pickups.contains_key(&entity.handle)
        {
            continue;
        }
        if !matches!(
            entity.kind,
            magicka_runtime::scene_world::EntityKind::Physics { .. }
        ) {
            continue;
        }
        let Some(name) = &entity.template_name else {
            continue;
        };
        if let Some(path) =
            files.get(&format!("gltf/Data/PhysicsEntities/{name}.glb").to_ascii_lowercase())
        {
            desired.insert((entity.handle, -302), (path.clone(), entity.orientation));
        }
    }
    if matches!(
        network.as_deref(),
        Some(super::network::NetworkPlay::Client(_))
    ) {
        desired.retain(|(_, slot), _| !(-302..=-300).contains(slot));
    }
    for (entity, item, mut transform) in &mut items {
        if let Some((path, matrix)) = desired.get(&(item.owner, item.slot)) {
            if *path == item.path {
                *transform = Transform::from_matrix(Mat4::from_cols_array(matrix));
                desired.remove(&(item.owner, item.slot));
                continue;
            }
        }
        commands.entity(entity).despawn();
    }
    let player = play.session.avatar().handle;
    for ((owner, slot), (path, matrix)) in desired {
        let mut item = commands.spawn((
            HeldItem {
                owner,
                slot,
                path: path.clone(),
            },
            Transform::from_matrix(Mat4::from_cols_array(&matrix)),
            Visibility::default(),
            WorldAssetRoot(server.load(GltfAssetLabel::Scene(0).from_asset(path))),
        ));
        if owner == player {
            // The player's own models never occlude its outline (outline.rs).
            item.insert((
                crate::lighting::NoDepthPrepass,
                crate::outline::OutlinedEquipment,
            ));
        }
    }
}

/// Apply XNA's TRS ordering and crossfade directly to the exported split bones,
/// after Bevy animation and before hierarchy propagation. No second pose clock.
#[allow(clippy::type_complexity)]
pub(super) fn pose_bodies(
    play: Res<Play>,
    parents: Query<&ChildOf>,
    avatars: Query<(), With<AvatarRoot>>,
    npcs: Query<&NpcRoot>,
    remotes: Query<&super::network::NetworkAvatar>,
    mut bones: Query<(Entity, &Name, &mut Transform)>,
) {
    let mut poses: HashMap<
        Entity,
        (
            &CharacterSkeleton,
            Vec<magicka_runtime::character_pose::Pose>,
        ),
    > = HashMap::new();
    for (entity, name, mut transform) in &mut bones {
        let Some(root) = parents
            .iter_ancestors(entity)
            .find(|p| avatars.contains(*p) || npcs.contains(*p) || remotes.contains(*p))
        else {
            continue;
        };
        if let std::collections::hash_map::Entry::Vacant(entry) = poses.entry(root) {
            let data = if avatars.contains(root) {
                let avatar = play.session.avatar();
                avatar
                    .rig
                    .as_ref()
                    .map(|r| (&r.skeleton, &avatar.motion.controller))
            } else if let Ok(remote) = remotes.get(root) {
                play.session
                    .world
                    .players
                    .get(remote.view_slot)
                    .and_then(Option::as_ref)
                    .as_ref()
                    .and_then(|a| a.rig.as_ref().map(|r| (&r.skeleton, &a.motion.controller)))
            } else {
                npcs.get(root)
                    .ok()
                    .and_then(|r| play.session.world.npcs.get(&r.handle))
                    .and_then(|n| n.rig.as_ref().map(|r| (&r.skeleton, &n.motion.controller)))
            };
            let Some((skeleton, controller)) = data else {
                continue;
            };
            entry.insert((skeleton, skeleton.local_poses(controller.pose_sample())));
        }
        let (skeleton, poses) = &poses[&root];
        let (bone_name, ts) = name
            .as_str()
            .strip_suffix(".ts")
            .map_or((name.as_str(), false), |n| (n, true));
        if let Some(index) = skeleton.bone(bone_name) {
            let (translation, rotation, scale) = poses[index];
            if ts {
                transform.translation = Vec3::from_array(translation);
                transform.scale = Vec3::from_array(scale);
            } else {
                transform.rotation = Quat::from_array(rotation);
            }
        }
    }
}

/// Headless regression measurement: compare held-model transforms against the
/// rendered hand hierarchy, rather than another simulation pose calculation.
#[allow(clippy::too_many_arguments)]
pub(super) fn verify_pose(
    pilot: Option<Res<Autopilot>>,
    play: Res<Play>,
    parents: Query<&ChildOf>,
    avatars: Query<(), With<AvatarRoot>>,
    npcs: Query<&NpcRoot>,
    bones: Query<(Entity, &Name, &GlobalTransform)>,
    items: Query<(&HeldItem, &GlobalTransform)>,
    mut last: Local<u32>,
) {
    let Some(pilot) = pilot else {
        return;
    };
    if pilot.frame == *last || pilot.frame % 60 != 0 {
        return;
    }
    *last = pilot.frame;
    let mut count = 0;
    let mut max_error = 0.0_f32;
    for (item, actual) in &items {
        let rig = if item.owner == play.session.avatar().handle {
            play.session.avatar().rig.as_ref()
        } else {
            play.session
                .world
                .npcs
                .get(&item.owner)
                .and_then(|n| n.rig.as_ref())
        };
        let Some(rig) = rig else {
            continue;
        };
        let Some(attachment) = rig.attachments.iter().find(|a| a.slot == item.slot) else {
            continue;
        };
        let bone = &rig.skeleton.skeleton.bones[attachment.bone];
        for (entity, name, global) in &bones {
            if name.as_str() != bone.name {
                continue;
            }
            let matches = parents
                .iter_ancestors(entity)
                .find(|p| avatars.contains(*p) || npcs.contains(*p))
                .is_some_and(|r| {
                    if avatars.contains(r) {
                        item.owner == play.session.avatar().handle
                    } else {
                        npcs.get(r).is_ok_and(|n| n.handle == item.owner)
                    }
                });
            if !matches {
                continue;
            }
            let expected = global.to_matrix()
                * Mat4::from_cols_array(&bone.inverse_bind)
                * Mat4::from_cols_array(&attachment.bind);
            for (a, b) in actual
                .to_matrix()
                .to_cols_array()
                .iter()
                .zip(expected.to_cols_array())
            {
                max_error = max_error.max((a - b).abs());
            }
            count += 1;
            break;
        }
    }
    info!(
        "equipment pose check: frame={} items={} max_matrix_error={}",
        pilot.frame, count, max_error
    );
}
