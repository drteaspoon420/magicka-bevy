//! IceSpikes.RenderData (research/ground-area.md): every drawn spike is
//! `IceBarrier0_mesh` at its root (random Y rotation, uniform scale, ground
//! hit) posed by its IceBarrier_animation clip at the simulation clip time.
use super::{script_act, Act, Autopilot, Play, WorldAssetRoot};
use bevy::prelude::*;
use std::collections::HashMap;

const GLB: &str = "gltf/Models/Effects/iceBarrier0_mesh.glb";
const CLIPS: [&str; 5] = ["emerge0", "emerge1", "emerge2", "emerge3", "emerge4"];

/// One spike: the effect's identity (first spike position bits) and index.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(super) struct SpikeRoot {
    key: [u32; 3],
    index: usize,
}

#[derive(Component)]
pub(super) struct SpikeAnimator(Entity);

fn key(spikes: &magicka_sim::ice_spikes::IceSpikes) -> [u32; 3] {
    spikes.spikes[0].position.map(f32::to_bits)
}

fn transform(spike: &magicka_sim::ice_spikes::Spike) -> Transform {
    Transform::from_translation(Vec3::from(spike.position))
        .with_rotation(Quat::from_rotation_y(spike.yaw))
        .with_scale(Vec3::splat(spike.scale))
}

pub(super) fn sync_ice_spikes(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    pilot: Option<Res<Autopilot>>,
    mut roots: Query<(Entity, &SpikeRoot, &mut Visibility)>,
) {
    let hidden = pilot.is_some_and(|p| {
        p.started
            .is_some_and(|start| script_act(p.script, p.frame - start) == Some(Act::SpikesHidden))
    });
    let live: HashMap<[u32; 3], &magicka_sim::ice_spikes::IceSpikes> = play
        .session
        .ice_spikes
        .iter()
        .map(|s| (key(s), s))
        .collect();
    for (entity, root, mut visibility) in &mut roots {
        if !live.contains_key(&root.key) {
            commands.entity(entity).despawn();
        }
        *visibility = if hidden {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
    for (k, effect) in &live {
        for (index, spike) in effect.spikes.iter().enumerate() {
            let root = SpikeRoot { key: *k, index };
            if roots.iter().any(|(_, r, _)| *r == root) {
                continue;
            }
            commands.spawn((
                root,
                transform(spike),
                Visibility::default(),
                WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(GLB))),
            ));
        }
    }
}

/// Each spike's clip sampled at the simulation time (forward rise, backward
/// sink; ClipSpeed 2 in the simulation).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn animate_ice_spikes(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut gltf: Local<Option<Handle<Gltf>>>,
    mut graphs_by_clip: Local<Vec<(Handle<AnimationGraph>, AnimationNodeIndex)>>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    new_players: Query<Entity, (With<AnimationPlayer>, Without<SpikeAnimator>)>,
    parents: Query<&ChildOf>,
    roots: Query<&SpikeRoot>,
    mut animators: Query<(&SpikeAnimator, &mut AnimationPlayer)>,
) {
    if roots.is_empty() {
        return;
    }
    let handle = gltf.get_or_insert_with(|| assets.load(GLB)).clone();
    if graphs_by_clip.is_empty() {
        let Some(g) = gltfs.get(&handle) else { return };
        for name in CLIPS {
            let Some(clip) = g.named_animations.get(name).cloned() else {
                return;
            };
            let (graph, node) = AnimationGraph::from_clip(clip);
            graphs_by_clip.push((graphs.add(graph), node));
        }
    }
    let spike_of = |root: &SpikeRoot| {
        play.session
            .ice_spikes
            .iter()
            .find(|s| key(s) == root.key)
            .and_then(|s| s.spikes.get(root.index))
    };
    for entity in &new_players {
        let Some(root_entity) = parents.iter_ancestors(entity).find(|a| roots.contains(*a)) else {
            continue;
        };
        let Some(spike) = roots.get(root_entity).ok().and_then(spike_of) else {
            continue;
        };
        if let Some((graph, _)) = graphs_by_clip.get(spike.clip_index) {
            commands.entity(entity).insert((
                SpikeAnimator(root_entity),
                AnimationGraphHandle(graph.clone()),
            ));
        }
    }
    for (animator, mut player) in &mut animators {
        let Some(spike) = roots.get(animator.0).ok().and_then(spike_of) else {
            continue;
        };
        let Some((_, node)) = graphs_by_clip.get(spike.clip_index) else {
            continue;
        };
        if !player.is_playing_animation(*node) {
            player.play(*node);
        }
        if let Some(active) = player.animation_mut(*node) {
            active.seek_to(spike.clip.time).pause();
        }
    }
}
