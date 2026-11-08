//! The Machine boss (research/boss-battles.md): the machine mesh, the orc
//! mesh (same skeleton, its own controller) and the king, each posed from
//! the runtime's local bone poses. themachine.glb holds both native meshes as
//! one glTF mesh: primitives 0-1 are native Meshes[0] (the orc), primitive 2
//! is Meshes[1] (the machine). StandardMaterial shading approximates the
//! native deferred effect; the damage flash and frozen orc material are not
//! drawn.
use super::*;
use magicka_runtime::scene_world::boss_live::BossRef;

const MACHINE: &str = "gltf/Models/Bosses/themachine/themachine.glb";
const KING: &str = "gltf/Models/Characters/Human/king_animation.glb";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Machine,
    Orc,
    King,
}

#[derive(Component)]
pub(super) struct MachineModel {
    part: Part,
}

struct Sample<'a> {
    part: Part,
    parent: magicka_content::skinned::Mat,
    poses: &'a [magicka_runtime::character_pose::Pose],
    skeleton: &'a magicka_runtime::character_pose::CharacterSkeleton,
}

fn samples(play: &Play) -> Vec<Sample<'_>> {
    let Some(host) = play
        .session
        .world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            BossRef::Machine(m) => Some(m),
            _ => None,
        })
    else {
        return Vec::new();
    };
    let m = &host.machine;
    let mut out = Vec::new();
    if !m.machine_pose.local.is_empty() {
        out.push(Sample {
            part: Part::Machine,
            parent: m.machine_pose.parent,
            poses: &m.machine_pose.local,
            skeleton: &m.machine_skeleton,
        });
        out.push(Sample {
            part: Part::Orc,
            parent: m.orc_pose.parent,
            poses: &m.orc_pose.local,
            skeleton: &m.machine_skeleton,
        });
    }
    if m.render_king && !m.king_pose.local.is_empty() {
        out.push(Sample {
            part: Part::King,
            parent: m.king_pose.parent,
            poses: &m.king_pose.local,
            skeleton: &m.king_skeleton,
        });
    }
    out
}

pub(super) fn sync_models(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut roots: Query<(Entity, &MachineModel, &mut Transform)>,
) {
    let samples = samples(&play);
    for (entity, model, mut transform) in &mut roots {
        match samples.iter().find(|s| s.part == model.part) {
            Some(s) => *transform = Transform::from_matrix(Mat4::from_cols_array(&s.parent)),
            None => commands.entity(entity).despawn(),
        }
    }
    for s in &samples {
        if roots.iter().any(|(_, m, _)| m.part == s.part) {
            continue;
        }
        let path = if s.part == Part::King { KING } else { MACHINE };
        commands.spawn((
            MachineModel { part: s.part },
            Transform::from_matrix(Mat4::from_cols_array(&s.parent)),
            Visibility::default(),
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path))),
        ));
    }
}

/// Each machine instance draws only its own controller's primitives
/// using stable glTF mesh labels; entity names contain material names.
pub(super) fn split_meshes(
    assets: Res<AssetServer>,
    parents: Query<&ChildOf>,
    roots: Query<&MachineModel>,
    mut meshes: Query<(Entity, &Mesh3d, &mut Visibility)>,
) {
    for (entity, mesh, mut visibility) in &mut meshes {
        let Some(part) = parents
            .iter_ancestors(entity)
            .find_map(|root| roots.get(root).ok().map(|m| m.part))
        else {
            continue;
        };
        let path = assets.get_path(&mesh.0);
        let show = draws_primitive(part, path.as_ref().and_then(|p| p.label()));
        let wanted = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Apply the runtime local poses after animation, before propagation.
pub(super) fn pose_models(
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<&MachineModel>,
    mut bones: Query<(Entity, &Name, &mut Transform), Without<MachineModel>>,
) {
    let samples = samples(&play);
    if samples.is_empty() {
        return;
    }
    for (entity, name, mut transform) in &mut bones {
        let Some(part) = parents
            .iter_ancestors(entity)
            .find_map(|root| roots.get(root).ok().map(|m| m.part))
        else {
            continue;
        };
        let Some(s) = samples.iter().find(|s| s.part == part) else {
            continue;
        };
        super::boss_models::apply_bone_pose(name, s.skeleton, s.poses, &mut transform);
    }
}

fn draws_primitive(part: Part, label: Option<&str>) -> bool {
    match part {
        Part::King => true,
        Part::Machine => label == Some("Mesh0/Primitive2"),
        Part::Orc => matches!(label, Some("Mesh0/Primitive0" | "Mesh0/Primitive1")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_controllers_draw_disjoint_native_meshes() {
        for (label, machine, orc) in [
            (Some("Mesh0/Primitive0"), false, true),
            (Some("Mesh0/Primitive1"), false, true),
            (Some("Mesh0/Primitive2"), true, false),
            (Some("Mesh0/Primitive3"), false, false),
            (None, false, false),
        ] {
            assert_eq!(draws_primitive(Part::Machine, label), machine);
            assert_eq!(draws_primitive(Part::Orc, label), orc);
            assert!(draws_primitive(Part::King, label));
        }
    }
}

/// Capture-time evidence of the actual loaded mesh ownership, including the
/// two independently timed breakfree animations.
pub(super) fn audit(
    pilot: Option<Res<Autopilot>>,
    play: Res<Play>,
    assets: Res<AssetServer>,
    parents: Query<&ChildOf>,
    roots: Query<&MachineModel>,
    meshes: Query<(Entity, &Mesh3d, &Visibility)>,
) {
    let Some(pilot) = pilot else { return };
    if pilot.script != MACHINE_BREAK_SCRIPT {
        return;
    }
    let Some(start) = pilot.started else { return };
    if (pilot.frame - start) % 30 != 0 {
        return;
    }
    let mut counts = [0; 3];
    for (entity, mesh, visibility) in &meshes {
        let Some(part) = parents
            .iter_ancestors(entity)
            .find_map(|p| roots.get(p).ok().map(|m| m.part))
        else {
            continue;
        };
        if *visibility == Visibility::Hidden {
            continue;
        }
        let path = assets.get_path(&mesh.0).expect("loaded Machine mesh path");
        assert!(draws_primitive(part, path.label()));
        counts[match part {
            Part::Machine => 0,
            Part::Orc => 1,
            Part::King => 2,
        }] += 1;
    }
    if counts[0] > 0 || counts[1] > 0 {
        assert_eq!(
            &counts[..2],
            &[1, 2],
            "one machine and one rider/bike mesh set"
        );
    }
    for b in &play.session.world.boss_fight.bosses {
        if let BossRef::Machine(m) = b {
            eprintln!(
                "Machine mesh audit frame={} counts={counts:?} machine={:?} orc={:?}",
                pilot.frame - start,
                m.machine.machine_state,
                m.machine.stage
            );
        }
    }
}
