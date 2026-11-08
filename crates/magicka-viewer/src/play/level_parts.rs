//! Play mode: animated level parts (gates, doors, carts…) follow the live
//! scene's AnimatedLevelPart playback driven by scene scripts
//! (`playAnimation`/`stopAnimation`/`resumeAnimation`). The exported glTF
//! mirrors the part hierarchy with one node per part, named after it and
//! tagged with `magicka_level_animation` extras.
use super::Play;
use bevy::{gltf::GltfExtras, prelude::*};
use magicka_runtime::animated_hierarchy::AnimatedPartState;
use magicka_runtime::animated_pose::local_pose;

/// A level-part node and its name path from the outermost part.
#[derive(Component)]
pub(super) struct LevelPartNode(pub(super) Option<Vec<String>>);

fn is_part(extras: &GltfExtras) -> bool {
    extras.value.contains("\"magicka_level_animation\"")
}

pub(super) fn tag_parts(
    mut commands: Commands,
    nodes: Query<(Entity, &GltfExtras), Added<GltfExtras>>,
) {
    for (entity, extras) in &nodes {
        if is_part(extras) {
            commands.entity(entity).insert(LevelPartNode(None));
        }
    }
}

fn find<'a>(parts: &'a [AnimatedPartState], path: &[String]) -> Option<&'a AnimatedPartState> {
    let (name, rest) = path.split_first()?;
    let part = parts.iter().find(|p| p.name == *name)?;
    if rest.is_empty() {
        Some(part)
    } else {
        find(&part.children, rest)
    }
}

pub(super) fn sync_parts(
    play: Res<Play>,
    mut nodes: Query<(Entity, &mut LevelPartNode, &mut Transform)>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    tagged: Query<(), With<LevelPartNode>>,
) {
    let parts = play.session.world.collision.parts();
    for (entity, mut node, mut transform) in &mut nodes {
        if node.0.is_none() {
            // Walk up through enclosing part nodes to build the name path.
            let mut path = Vec::new();
            let mut current = entity;
            while let Ok(name) = names.get(current) {
                path.push(name.as_str().to_string());
                match parents.get(current) {
                    Ok(parent) if tagged.contains(parent.parent()) => current = parent.parent(),
                    _ => break,
                }
            }
            path.reverse();
            node.0 = Some(path);
        }
        let Some(part) = node.0.as_deref().and_then(|p| find(parts, p)) else {
            continue;
        };
        if let Ok(matrix) = local_pose(&part.animation, part.playback.time) {
            let pose = Transform::from_matrix(Mat4::from_cols_array(&matrix));
            if *transform != pose {
                *transform = pose;
            }
        }
    }
}
