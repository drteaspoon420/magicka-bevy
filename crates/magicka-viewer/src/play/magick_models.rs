//! Skinned magick models drawn by their entities: the Tornado funnel and
//! debris (RenderableAdditiveObject with mAlpha, Models/Magicks/tornado and
//! tornado_debri sharing the `tornado` clip at speed 4) and the Summon
//! Phoenix descent (Models/Magicks/Phoenix, `descend`, RotationY(3π/2)).
//! research/tornado.md, research/phoenix.md.
use super::*;

const TORNADO: &str = "gltf/Models/Magicks/tornado.glb";
const DEBRIS: &str = "gltf/Models/Magicks/tornado_debri.glb";
const PHOENIX: &str = "gltf/Models/Magicks/Phoenix.glb";
/// The funnel's greyscale streak texture drawn additively. The native
/// additive deferred technique's colour maths is not recovered; this dusty
/// tint and strength approximate the user's retail reference screenshot
/// (2026-10-11) instead of blowing out to white (INFERRED, visual only).
const FUNNEL_TINT: Color = Color::srgba(0.8, 0.62, 0.38, 1.0);
const FUNNEL_STRENGTH: f32 = 0.45;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    /// A tornado serial; `debris` selects the second model.
    Tornado {
        serial: u64,
        debris: bool,
    },
    Phoenix,
}

#[derive(Component)]
pub(crate) struct MagickModel {
    source: Source,
    glb: &'static str,
    clip: &'static str,
}

#[derive(Component)]
pub(super) struct MagickModelAnimator(Entity);

/// The additive copy of a Tornado mesh material, faded by the entity alpha.
#[derive(Component)]
pub(super) struct TornadoTint(Handle<StandardMaterial>);

struct Wanted {
    source: Source,
    glb: &'static str,
    clip: &'static str,
    transform: Transform,
    /// Clip time, and whether it loops.
    time: f32,
    alpha: f32,
}

fn wanted(play: &Play) -> Vec<Wanted> {
    let world = &play.session.world;
    let mut out = Vec::new();
    for live in &world.tornadoes {
        let t = &live.tornado;
        // Update: iM = Orientation (identity after the first MoveTo) with
        // the position, UniformMatrixScale 0.75; controller speed 4.
        let transform =
            Transform::from_translation(Vec3::from(t.position)).with_scale(Vec3::splat(0.75));
        let time = (magicka_sim::tornado::TTL - t.ttl) * 4.0;
        for (debris, glb) in [(false, TORNADO), (true, DEBRIS)] {
            out.push(Wanted {
                source: Source::Tornado {
                    serial: live.serial,
                    debris,
                },
                glb,
                clip: "tornado",
                transform,
                time,
                alpha: t.alpha,
            });
        }
    }
    let p = &world.phoenix;
    if p.registered && !p.cast_fail {
        out.push(Wanted {
            source: Source::Phoenix,
            glb: PHOENIX,
            clip: magicka_sim::phoenix::CLIP,
            transform: Transform::from_translation(Vec3::from(p.point))
                .with_rotation(Quat::from_rotation_y(4.712389)),
            time: p.clip_time,
            alpha: 1.0,
        });
    }
    out
}

/// Spawn, move and despawn one root per wanted model.
pub(super) fn sync_magick_models(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut roots: Query<(Entity, &MagickModel, &mut Transform)>,
) {
    let wanted = wanted(&play);
    for (entity, model, mut transform) in &mut roots {
        match wanted.iter().find(|w| w.source == model.source) {
            Some(w) => *transform = w.transform,
            None => commands.entity(entity).despawn(),
        }
    }
    for w in &wanted {
        if roots.iter().any(|(_, m, _)| m.source == w.source) {
            continue;
        }
        commands.spawn((
            MagickModel {
                source: w.source,
                glb: w.glb,
                clip: w.clip,
            },
            w.transform,
            Visibility::default(),
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(w.glb))),
        ));
    }
}

/// Tornado meshes become unlit additive copies (kept out of the lit
/// material conversion) whose alpha follows mAlpha each frame.
#[allow(clippy::type_complexity)]
pub(super) fn tint_magick_models(
    mut commands: Commands,
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<&MagickModel>,
    fresh: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>),
        (
            Without<TornadoTint>,
            Without<crate::lighting::KeepStandardMaterial>,
        ),
    >,
    tinted: Query<(Entity, &TornadoTint)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let wanted = wanted(&play);
    let root_of = |entity: Entity| {
        parents
            .iter_ancestors(entity)
            .find_map(|a| roots.get(a).ok())
    };
    for (entity, handle) in &fresh {
        let Some(model) = root_of(entity) else {
            continue;
        };
        if !matches!(model.source, Source::Tornado { .. }) {
            continue;
        }
        let Some(base) = materials.get(&handle.0).cloned() else {
            continue;
        };
        let copy = materials.add(StandardMaterial {
            alpha_mode: AlphaMode::Add,
            unlit: true,
            base_color: FUNNEL_TINT,
            ..base
        });
        // RenderableAdditiveObject: no shadow pass.
        commands.entity(entity).insert((
            MeshMaterial3d(copy.clone()),
            TornadoTint(copy),
            crate::lighting::KeepStandardMaterial,
            bevy::light::NotShadowCaster,
        ));
    }
    for (entity, tint) in &tinted {
        let Some(model) = root_of(entity) else {
            continue;
        };
        let Some(w) = wanted.iter().find(|w| w.source == model.source) else {
            continue;
        };
        if let Some(mut m) = materials.get_mut(&tint.0) {
            m.base_color
                .set_alpha(w.alpha.clamp(0.0, 1.0) * FUNNEL_STRENGTH);
        }
    }
}

/// Each model's clip sampled at the simulation time (Tornado loops).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn animate_magick_models(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut cache: Local<
        HashMap<
            &'static str,
            (
                Handle<Gltf>,
                Option<(Handle<AnimationGraph>, AnimationNodeIndex, f32)>,
            ),
        >,
    >,
    gltfs: Res<Assets<Gltf>>,
    clips: Res<Assets<AnimationClip>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    new_players: Query<Entity, (With<AnimationPlayer>, Without<MagickModelAnimator>)>,
    parents: Query<&ChildOf>,
    roots: Query<(Entity, &MagickModel)>,
    mut animators: Query<(&MagickModelAnimator, &mut AnimationPlayer)>,
) {
    if roots.is_empty() {
        return;
    }
    for (_, model) in &roots {
        let entry = cache
            .entry(model.glb)
            .or_insert_with(|| (assets.load(model.glb), None));
        if entry.1.is_none() {
            if let Some(clip) = gltfs
                .get(&entry.0)
                .and_then(|g| g.named_animations.get(model.clip).cloned())
            {
                if let Some(duration) = clips.get(&clip).map(AnimationClip::duration) {
                    let (graph, node) = AnimationGraph::from_clip(clip);
                    entry.1 = Some((graphs.add(graph), node, duration));
                }
            }
        }
    }
    for entity in &new_players {
        let Some(root) = parents.iter_ancestors(entity).find(|a| roots.contains(*a)) else {
            continue;
        };
        let Ok((_, model)) = roots.get(root) else {
            continue;
        };
        if let Some((_, Some((graph, _, _)))) = cache.get(model.glb) {
            commands.entity(entity).insert((
                MagickModelAnimator(root),
                AnimationGraphHandle(graph.clone()),
            ));
        }
    }
    let wanted = wanted(&play);
    for (animator, mut player) in &mut animators {
        let Ok((_, model)) = roots.get(animator.0) else {
            continue;
        };
        let (Some(w), Some((_, Some((_, node, duration))))) = (
            wanted.iter().find(|w| w.source == model.source),
            cache.get(model.glb),
        ) else {
            continue;
        };
        let time = match model.source {
            Source::Tornado { .. } if *duration > 0.0 => w.time.rem_euclid(*duration),
            _ => w.time.min(*duration),
        };
        if !player.is_playing_animation(*node) {
            player.play(*node);
        }
        if let Some(active) = player.animation_mut(*node) {
            active.seek_to(time).pause();
        }
    }
}

/// One VortexEffect sphere per live vortex.
#[derive(Component)]
pub(super) struct VortexSphere(u64);

/// VortexEntity.RenderData: Transform = CreateScale(mRadius) at the centre,
/// drawn with Distortion 0.025 and DistortionPower 2.
pub(super) fn sync_vortex_spheres(
    mut commands: Commands,
    play: Res<Play>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::magick_visuals::VortexMaterial>>,
    mut spheres: Query<(Entity, &VortexSphere, &mut Transform, &mut Visibility)>,
    mut shared: Local<Option<(Handle<Mesh>, Handle<crate::magick_visuals::VortexMaterial>)>>,
) {
    let vortices = &play.session.world.vortices;
    for (entity, sphere, mut transform, mut visibility) in &mut spheres {
        match vortices.iter().find(|v| v.serial == sphere.0) {
            Some(v) => {
                transform.translation = Vec3::from(v.vortex.center);
                transform.scale = Vec3::splat(v.vortex.radius.max(0.0));
                *visibility = if v.vortex.radius > 1e-3 {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            None => commands.entity(entity).despawn(),
        }
    }
    for v in vortices {
        if spheres.iter().any(|(_, s, _, _)| s.0 == v.serial) {
            continue;
        }
        let (mesh, material) = shared
            .get_or_insert_with(|| {
                (
                    meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap()),
                    materials.add(crate::magick_visuals::VortexMaterial {
                        params: Vec4::new(0.025, 2.0, 0.0, 0.0),
                    }),
                )
            })
            .clone();
        commands.spawn((
            VortexSphere(v.serial),
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_translation(Vec3::from(v.vortex.center))
                .with_scale(Vec3::splat(v.vortex.radius.max(0.0))),
            Visibility::default(),
            bevy::light::NotShadowCaster,
        ));
    }
}
