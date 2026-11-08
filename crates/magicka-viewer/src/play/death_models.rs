//! Death's body, scythe and five afterimages share runtime pose snapshots.
//! The shared deferred lighting and ethereal pipeline shade the decoded assets.
use super::*;

const BODY: &str = "gltf/Models/Bosses/death/death.glb";
const SCYTHE: &str = "gltf/Models/Bosses/death/death_scythe.glb";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Body,
    Scythe,
    Afterimage(usize),
    BossBody(u16, usize),
    BossScythe(u16, usize),
    BossAfterimage(u16, usize),
}
impl Source {
    fn scythe(self) -> bool {
        matches!(self, Self::Scythe | Self::BossScythe(..))
    }
    fn afterimage(self) -> bool {
        matches!(self, Self::Afterimage(_) | Self::BossAfterimage(..))
    }
}

#[derive(Component)]
pub(super) struct DeathModel {
    source: Source,
}

#[derive(Component)]
pub(super) struct DeathMaterial {
    handle: Handle<crate::lighting::PlayerLitMaterial>,
    base_color: Color,
}

struct Sample {
    source: Source,
    matrix: magicka_content::skinned::Mat,
    poses: Option<Vec<magicka_runtime::character_pose::Pose>>,
    alpha: f32,
    additive: bool,
    visible: bool,
}

fn samples(play: &Play) -> Vec<Sample> {
    let mut out = Vec::new();
    for boss in &play.session.world.boss_fight.bosses {
        let magicka_runtime::scene_world::boss_live::BossRef::Death(host) = boss else {
            continue;
        };
        let actor = host.actor.lock().expect("Death actor lock poisoned");
        if !actor.drawn {
            continue;
        }
        for slot in 0..16 {
            if !actor.visible[slot] {
                continue;
            }
            out.push(Sample {
                source: Source::BossBody(host.zone, slot),
                matrix: actor.poses[slot].parent,
                poses: Some(actor.poses[slot].local.clone()),
                alpha: if actor.ethereal {
                    actor.alpha.min(1.)
                } else {
                    1.
                },
                additive: actor.ethereal,
                visible: true,
            });
            if actor.scythe_visible(slot) {
                out.push(Sample {
                    source: Source::BossScythe(host.zone, slot),
                    matrix: actor.scythes[slot],
                    poses: None,
                    alpha: 1.,
                    additive: false,
                    visible: true,
                });
            }
        }
        // Retain saved history roots while motion pauses. Recreating them on
        // each visibility change also perturbs transparent tie ordering.
        {
            for (index, (pose, alpha)) in actor
                .afterimages
                .poses
                .iter()
                .zip(actor.afterimages.draw_alphas())
                .enumerate()
            {
                let Some(pose) = pose else { continue };
                if alpha == 0. {
                    continue;
                }
                out.push(Sample {
                    source: Source::BossAfterimage(host.zone, index),
                    matrix: pose.parent,
                    poses: Some(pose.local.clone()),
                    alpha,
                    additive: actor.ethereal,
                    visible: actor.afterimages.drawn,
                });
            }
        }
    }
    let death = &play.session.world.death;
    let Some(actor) = death.actor.as_ref() else {
        return out;
    };
    let Some(frame) = death.frame.as_ref() else {
        return out;
    };
    out.extend([
        Sample {
            source: Source::Body,
            matrix: frame.model_parent,
            poses: Some(frame.local_poses.clone()),
            alpha: actor.state.alpha,
            additive: frame.additive,
            visible: true,
        },
        // Native ItemRenderData is submitted to the ordinary object queue;
        // the body's ethereal material and alpha are not applied to the scythe.
        Sample {
            source: Source::Scythe,
            matrix: frame.scythe_world,
            poses: None,
            alpha: 1.0,
            additive: false,
            visible: true,
        },
    ]);
    if frame.afterimages_visible {
        for (index, (pose, alpha)) in actor
            .afterimages
            .poses
            .iter()
            .zip(actor.afterimages.draw_alphas())
            .enumerate()
        {
            let Some(pose) = pose else { continue };
            if alpha == 0.0 {
                continue;
            }
            out.push(Sample {
                source: Source::Afterimage(index),
                matrix: pose.model_parent,
                poses: Some(pose.local_poses.clone()),
                alpha,
                additive: actor.state.ethereal,
                visible: true,
            });
        }
    }
    out
}

pub(super) fn sync_models(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut roots: Query<(Entity, &DeathModel, &mut Transform, &mut Visibility)>,
) {
    let samples = samples(&play);
    for (entity, model, mut transform, mut visibility) in &mut roots {
        match samples.iter().find(|sample| sample.source == model.source) {
            Some(sample) => {
                *transform = Transform::from_matrix(Mat4::from_cols_array(&sample.matrix));
                *visibility = if sample.visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            None => {
                commands.entity(entity).despawn();
            }
        }
    }
    for sample in &samples {
        if roots
            .iter()
            .any(|(_, model, _, _)| model.source == sample.source)
        {
            continue;
        }
        let path = if sample.source.scythe() { SCYTHE } else { BODY };
        commands.spawn((
            DeathModel {
                source: sample.source,
            },
            Transform::from_matrix(Mat4::from_cols_array(&sample.matrix)),
            if sample.visible {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path))),
        ));
    }
}

fn tint(material: &mut StandardMaterial, base: Color, sample: &Sample) {
    // Additive and alpha-blended samples share the blend mode here.
    material.alpha_mode = if sample.source.scythe() {
        AlphaMode::Opaque
    } else {
        AlphaMode::Blend
    };
    let base = base.to_linear();
    let cold = if sample.source.afterimage() && !sample.additive {
        [1.0, 1.6, 2.0]
    } else {
        [1.0; 3]
    };
    material.base_color = Color::linear_rgba(
        base.red * cold[0],
        base.green * cold[1],
        base.blue * cold[2],
        base.alpha * if sample.additive { 1. } else { sample.alpha },
    );
}

/// Clone mesh materials so Death's fade never changes another model's assets.
#[allow(clippy::type_complexity)]
pub(super) fn tint_models(
    mut commands: Commands,
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<&DeathModel>,
    fresh: Query<
        (
            Entity,
            &MeshMaterial3d<StandardMaterial>,
            Option<&bevy::gltf::GltfMaterialExtras>,
        ),
        Without<DeathMaterial>,
    >,
    tinted: Query<(Entity, &DeathMaterial)>,
    materials: Res<Assets<StandardMaterial>>,
    mut lit: ResMut<Assets<crate::lighting::PlayerLitMaterial>>,
    lights: Res<crate::lighting::MagickaLights>,
) {
    let samples = samples(&play);
    let source = |entity| {
        parents
            .iter_ancestors(entity)
            .find_map(|root| roots.get(root).ok().map(|model| model.source))
    };
    for (entity, mesh, extras) in &fresh {
        let Some(source) = source(entity) else {
            continue;
        };
        let Some(sample) = samples.iter().find(|sample| sample.source == source) else {
            continue;
        };
        let Some(mut material) = materials.get(&mesh.0).cloned() else {
            continue;
        };
        let base_color = material.base_color;
        tint(&mut material, base_color, sample);
        // Death shares the original deferred lighting with other skinned
        // bodies; retaining StandardMaterial made the seated intro fullbright.
        material.unlit = false;
        let json = extras.and_then(|e| serde_json::from_str::<serde_json::Value>(&e.value).ok());
        let mut params =
            crate::lighting::material_params(json.as_ref().and_then(|j| j.get("magicka_skinned")));
        params.maps.y = sample.alpha;
        params.maps.z = if sample.additive { 1. } else { 0. };
        let handle = lit.add(crate::lighting::PlayerLitMaterial {
            base: material,
            extension: crate::lighting::PlayerLitExtension {
                params,
                lights: lights.buffer.clone(),
                material_map: None,
                look: default(),
                ice: None,
                ice_normal: None,
            },
        });
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert((
                MeshMaterial3d(handle.clone()),
                DeathMaterial { handle, base_color },
            ));
        if source.afterimage() {
            // Native AfterImageRenderData.DrawShadow is empty.
            commands.entity(entity).insert(bevy::light::NotShadowCaster);
        }
    }
    for (entity, material) in &tinted {
        let Some(source) = source(entity) else {
            continue;
        };
        let Some(sample) = samples.iter().find(|sample| sample.source == source) else {
            continue;
        };
        if let Some(mut target) = lit.get_mut(&material.handle) {
            tint(&mut target.base, material.base_color, sample);
            target.base.unlit = false;
            target.extension.params.maps.y = sample.alpha;
            target.extension.params.maps.z = if sample.additive { 1. } else { 0. };
        }
    }
}

/// Apply each body's saved split-bone pose after animation, before propagation.
pub(super) fn pose_body(
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<&DeathModel>,
    mut bones: Query<(Entity, &Name, &mut Transform)>,
) {
    let names: Vec<String> = if let Some(actor) = play.session.world.death.actor.as_ref() {
        actor
            .skeleton
            .skeleton
            .bones
            .iter()
            .map(|b| b.name.clone())
            .collect()
    } else {
        play.session
            .world
            .boss_fight
            .bosses
            .iter()
            .find_map(|b| {
                if let magicka_runtime::scene_world::boss_live::BossRef::Death(h) = b {
                    Some(
                        h.actor
                            .lock()
                            .unwrap()
                            .skeleton
                            .skeleton
                            .bones
                            .iter()
                            .map(|b| b.name.clone())
                            .collect(),
                    )
                } else {
                    None
                }
            })
            .unwrap_or_default()
    };
    let samples = samples(&play);
    for (entity, name, mut transform) in &mut bones {
        let Some(source) = parents
            .iter_ancestors(entity)
            .find_map(|root| roots.get(root).ok().map(|model| model.source))
        else {
            continue;
        };
        let Some(poses) = samples
            .iter()
            .find(|sample| sample.source == source)
            .and_then(|sample| sample.poses.as_deref())
        else {
            continue;
        };
        let (bone, ts) = name
            .as_str()
            .strip_suffix(".ts")
            .map_or((name.as_str(), false), |bone| (bone, true));
        let Some(index) = names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(bone))
        else {
            continue;
        };
        let Some(&(translation, rotation, scale)) = poses.get(index) else {
            continue;
        };
        if ts {
            transform.translation = Vec3::from_array(translation);
            transform.scale = Vec3::from_array(scale);
        } else {
            transform.rotation = Quat::from_array(rotation);
        }
    }
}
