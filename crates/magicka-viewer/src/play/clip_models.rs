//! Models that show one named glTF clip sampled at a simulation time:
//! dispensers and SpawnAnimProp's animated physics props.
use super::*;

const DISPENSER_MODELS: [&str; 3] = [
    "gltf/Models/Effects/SpiderHole.glb",
    "gltf/Models/Effects/GoblinTent.glb",
    "gltf/Models/Effects/spiderEggs.glb",
];

/// (source, handle): source 0 = dispenser, 1 = animated prop.
type Key = (u8, u16);

struct ClipModel {
    path: String,
    transform: [f32; 16],
    clip: Option<String>,
    time: f32,
}

fn desired(play: &Play) -> HashMap<Key, ClipModel> {
    let world = &play.session.world;
    let mut out = HashMap::new();
    for (&handle, d) in &world.dispensers {
        let Some(path) = DISPENSER_MODELS.get(d.model) else {
            continue;
        };
        out.insert(
            (0, handle),
            ClipModel {
                path: (*path).to_string(),
                transform: d.render_transform(),
                clip: d
                    .clip
                    .map(|c| magicka_runtime::dispenser::CLIPS[c].to_string()),
                time: d.clip_time,
            },
        );
    }
    for e in &world.entities {
        let Some(prop) = world.anim_props.get(&e.handle) else {
            continue;
        };
        if !e.added || e.removed || e.dead {
            continue;
        }
        let Some(name) = e
            .template_name
            .as_deref()
            .and_then(|n| n.strip_prefix(magicka_runtime::play_content::ANIMATED_PROPS))
        else {
            continue;
        };
        out.insert(
            (1, e.handle),
            ClipModel {
                path: format!("gltf/Data/AnimatedProps/{name}.glb"),
                transform: e.orientation,
                clip: prop.clip.clone(),
                time: prop.time,
            },
        );
    }
    out
}

#[derive(Component)]
pub(super) struct ClipModelRoot {
    key: Key,
    path: String,
}
#[derive(Component)]
pub(super) struct ClipAnimator {
    root: Entity,
    clip: Option<String>,
}
type Graph = (Handle<AnimationGraph>, HashMap<String, AnimationNodeIndex>);
#[derive(Default)]
pub(super) struct Graphs {
    models: HashMap<String, (Handle<Gltf>, Option<Graph>)>,
}

pub(super) fn sync_models(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut roots: Query<(Entity, &ClipModelRoot, &mut Transform)>,
) {
    let desired = desired(&play);
    for (entity, root, mut transform) in &mut roots {
        match desired.get(&root.key) {
            Some(m) if m.path == root.path => {
                *transform = Transform::from_matrix(Mat4::from_cols_array(&m.transform));
            }
            _ => commands.entity(entity).despawn(),
        }
    }
    for (key, m) in desired {
        if roots
            .iter()
            .any(|(_, r, _)| r.key == key && r.path == m.path)
        {
            continue;
        }
        commands.spawn((
            ClipModelRoot {
                key,
                path: m.path.clone(),
            },
            Transform::from_matrix(Mat4::from_cols_array(&m.transform)),
            Visibility::default(),
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(m.path))),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn animate_models(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut cache: Local<Graphs>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    new_players: Query<Entity, (With<AnimationPlayer>, Without<ClipAnimator>)>,
    parents: Query<&ChildOf>,
    roots: Query<&ClipModelRoot>,
    mut animators: Query<(&mut ClipAnimator, &mut AnimationPlayer)>,
) {
    let desired = desired(&play);
    for m in desired.values() {
        let entry = cache
            .models
            .entry(m.path.clone())
            .or_insert_with(|| (assets.load(m.path.clone()), None));
        if entry.1.is_some() {
            continue;
        }
        let Some(gltf) = gltfs.get(&entry.0) else {
            continue;
        };
        let mut graph = AnimationGraph::new();
        let nodes = gltf
            .named_animations
            .iter()
            .map(|(name, clip)| {
                (
                    name.to_string(),
                    graph.add_clip(clip.clone(), 1.0, graph.root),
                )
            })
            .collect();
        entry.1 = Some((graphs.add(graph), nodes));
    }
    for entity in &new_players {
        let Some(root) = parents.iter_ancestors(entity).find(|a| roots.contains(*a)) else {
            continue;
        };
        let Ok(r) = roots.get(root) else {
            continue;
        };
        let Some((_, Some((graph, _)))) = cache.models.get(&r.path) else {
            continue;
        };
        commands.entity(entity).insert((
            ClipAnimator { root, clip: None },
            AnimationGraphHandle(graph.clone()),
        ));
    }
    for (mut animator, mut player) in &mut animators {
        let Ok(root) = roots.get(animator.root) else {
            continue;
        };
        let Some(m) = desired.get(&root.key) else {
            continue;
        };
        let Some((_, Some((_, nodes)))) = cache.models.get(&root.path) else {
            continue;
        };
        if animator.clip != m.clip {
            player.stop_all();
            animator.clip = m.clip.clone();
        }
        let Some(node) = m.clip.as_ref().and_then(|c| nodes.get(c)).copied() else {
            continue;
        };
        if !player.is_playing_animation(node) {
            player.play(node);
        }
        if let Some(animation) = player.animation_mut(node) {
            animation.seek_to(m.time).pause();
        }
    }
}
