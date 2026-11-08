//! Character material ownership and actual scene-light response controls.
use super::*;
pub(super) const SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::LightingSpawn, "spawn"),
    (90, [0., 0.], Act::Idle, "settled"),
    (30, [0., 0.], Act::LightingBright, "lit"),
    (30, [0., 0.], Act::LightingDark, "unlit"),
    (30, [0., 0.], Act::LightingBright, "restored"),
];
pub(super) const ETHEREAL_SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::EtherealNpcSpawn, "spawn"),
    (90, [0., 0.], Act::Idle, "settled"),
    (30, [0., 0.], Act::LightingBright, "lit"),
    (30, [0., 0.], Act::LightingDark, "dark"),
    (30, [0., 0.], Act::LightingBright, "restored"),
];
pub(super) const ETHEREAL_BLEND_SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::EtherealNpcSpawn, "spawn"),
    (90, [0., 0.], Act::Idle, "settled"),
    (30, [0., 0.], Act::EtherealNpcAlpha(0), "ethereal"),
    (30, [0., 0.], Act::EtherealNpcAlpha(1), "zero_alpha"),
    (30, [0., 0.], Act::EtherealNpcAlpha(2), "hidden"),
    (30, [0., 0.], Act::EtherealNpcAlpha(3), "restored"),
];

pub(super) fn blend_visibility(
    pilot: Option<Res<Autopilot>>,
    mut roots: Query<&mut Visibility, With<NpcRoot>>,
) {
    let Some(pilot) = pilot else { return };
    if pilot.script != ETHEREAL_BLEND_SCRIPT {
        return;
    }
    let Some(start) = pilot.started else { return };
    let Some(Act::EtherealNpcAlpha(phase)) = script_act(pilot.script, pilot.frame - start) else {
        return;
    };
    for mut visibility in &mut roots {
        *visibility = if phase == 2 {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
}

pub(super) const ETHEREAL_ATTACK_SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::EtherealNpcAttackSpawn(3), "spawn"),
    (30, [0., 0.], Act::Idle, "pre_motion"),
    (150, [0., 0.], Act::Idle, "combat"),
    (30, [0., 0.], Act::LightingBright, "settled"),
];
pub(super) const GHOST_ATTACK_SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::EtherealNpcAttackSpawn(1), "spawn"),
    (180, [0., 0.], Act::Idle, "combat"),
    (30, [0., 0.], Act::LightingBright, "settled"),
];
pub(super) const PROJECTION_ATTACK_SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::EtherealNpcAttackSpawn(2), "spawn"),
    (180, [0., 0.], Act::Idle, "combat"),
    (30, [0., 0.], Act::LightingBright, "settled"),
];
pub(super) const ETHEREAL_ATTACK_DIRECT_SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::EtherealNpcAttackSpawn(3), "spawn_direct"),
    (180, [0., 0.], Act::Idle, "combat_direct"),
    (30, [0., 0.], Act::LightingBright, "settled_direct"),
];
pub(super) fn audit(
    pilot: Option<Res<Autopilot>>,
    play: Res<Play>,
    parents: Query<&ChildOf>,
    roots: Query<(), Or<(With<AvatarRoot>, With<NpcRoot>)>>,
    npc_roots: Query<(), With<NpcRoot>>,
    lit_materials: Res<Assets<crate::lighting::PlayerLitMaterial>>,
    robe_materials: Res<Assets<crate::robe::RobeMaterial>>,
    meshes: Query<
        (
            Entity,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&MeshMaterial3d<crate::lighting::NonOccludingMaterial>>,
            Option<&MeshMaterial3d<crate::lighting::PlayerLitMaterial>>,
            Option<&MeshMaterial3d<crate::robe::RobeMaterial>>,
        ),
        With<bevy::mesh::skinning::SkinnedMesh>,
    >,
    mut checked: Local<bool>,
    skins: Query<(
        Entity,
        &Mesh3d,
        &bevy::mesh::skinning::SkinnedMesh,
        &bevy::camera::visibility::InheritedVisibility,
        &bevy::camera::visibility::ViewVisibility,
    )>,
    mesh_assets: Res<Assets<Mesh>>,
    transforms: Query<&GlobalTransform>,
    bindposes: Res<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
) {
    let Some(pilot) = pilot else { return };
    let Some(start) = pilot.started else { return };
    let combat_scripts = [
        ETHEREAL_ATTACK_SCRIPT,
        GHOST_ATTACK_SCRIPT,
        PROJECTION_ATTACK_SCRIPT,
        ETHEREAL_ATTACK_DIRECT_SCRIPT,
    ];
    if combat_scripts.contains(&pilot.script) && (pilot.frame - start) % 10 == 0 {
        for (&handle, npc) in &play.session.world.npcs {
            eprintln!(
                "NPC attack audit frame={} handle={} busy={:?} clip={:?} time={} spell={:?}",
                pilot.frame - start,
                handle,
                npc.combat.busy_ability,
                npc.motion.controller.clip_name(),
                npc.motion.controller.time,
                npc.current_spell
            );
        }
        for (entity, mesh_handle, skin, inherited, visible) in &skins {
            if !parents
                .iter_ancestors(entity)
                .any(|p| npc_roots.contains(p))
            {
                continue;
            }
            eprintln!(
                "NPC visibility audit mesh={entity:?} inherited={} view={}",
                inherited.get(),
                visible.get()
            );
            let Some(bind) = bindposes.get(&skin.inverse_bindposes) else {
                continue;
            };
            let mut min_det = f32::INFINITY;
            let mut palette = Vec::with_capacity(skin.joints.len());
            for (i, joint) in skin.joints.iter().enumerate() {
                let Ok(transform) = transforms.get(*joint) else {
                    continue;
                };
                let matrix = transform.to_matrix() * bind[i];
                assert!(matrix.is_finite(), "NPC render palette must be finite");
                min_det = min_det.min(matrix.determinant().abs());
                palette.push(matrix);
            }
            eprintln!(
                "NPC palette audit mesh={entity:?} joints={} min_abs_det={min_det}",
                skin.joints.len()
            );
            if let Ok((_, _, _, lit, robe)) = meshes.get(entity) {
                eprintln!(
                    "NPC material audit mesh={entity:?} lit={:?} robe={:?}",
                    lit.and_then(|h| lit_materials.get(&h.0))
                        .map(|m| m.extension.params.maps),
                    robe.and_then(|h| robe_materials.get(&h.0))
                        .map(|m| m.extension.params.control)
                );
            }
            let Some(mesh) = mesh_assets.get(&mesh_handle.0) else {
                continue;
            };
            use bevy::mesh::VertexAttributeValues;
            let Some(VertexAttributeValues::Uint16x4(indices)) =
                mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
            else {
                panic!("NPC skin needs joint indices")
            };
            let Some(VertexAttributeValues::Float32x4(weights)) =
                mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)
            else {
                panic!("NPC skin needs joint weights")
            };
            let Some(VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!("NPC skin needs positions")
            };
            assert_eq!(indices.len(), weights.len());
            assert_eq!(indices.len(), positions.len());
            assert_eq!(
                palette.len(),
                skin.joints.len(),
                "all skin joints must exist"
            );
            let mut min_weighted_det = f32::INFINITY;
            let mut max_weight_error = 0f32;
            for ((indices, weights), position) in indices.iter().zip(weights).zip(positions) {
                // Bevy reads all four matrices even for zero-weight lanes.
                let mut matrix = Mat4::ZERO;
                for (&index, &weight) in indices.iter().zip(weights) {
                    assert!(weight.is_finite() && weight >= 0., "invalid skin weight");
                    matrix += palette[usize::from(index)] * weight;
                }
                assert!(matrix.is_finite(), "weighted skin matrix must be finite");
                assert!(matrix
                    .transform_point3(Vec3::from_array(*position))
                    .is_finite());
                min_weighted_det = min_weighted_det.min(matrix.determinant().abs());
                max_weight_error = max_weight_error.max((weights.iter().sum::<f32>() - 1.).abs());
            }
            eprintln!("NPC weighted skin audit mesh={entity:?} vertices={} min_abs_det={min_weighted_det} max_weight_error={max_weight_error}", positions.len());
        }
        let effects: std::collections::BTreeSet<_> = play
            .session
            .effects
            .presentation()
            .iter()
            .filter_map(|e| play.session.effects.library.effects.get(&e.definition))
            .map(|e| e.name.clone())
            .collect();
        eprintln!("NPC attack audit effects={effects:?}");
    }
    if ![SCRIPT, ETHEREAL_SCRIPT].contains(&pilot.script) || pilot.frame - start < 100 || *checked {
        return;
    }
    let mut native = 0;
    let mut fallback = 0;
    let mut ethereal = 0;
    for (entity, standard, other, lit, robe) in &meshes {
        if !parents.iter_ancestors(entity).any(|p| roots.contains(p)) {
            continue;
        }
        native += usize::from(lit.is_some() || robe.is_some());
        fallback += usize::from(standard.is_some() || other.is_some());
        if pilot.script == ETHEREAL_SCRIPT
            && parents
                .iter_ancestors(entity)
                .any(|p| npc_roots.contains(p))
        {
            let enabled = lit
                .and_then(|handle| lit_materials.get(&handle.0))
                .is_some_and(|m| m.extension.params.maps.z > 0.5)
                || robe
                    .and_then(|handle| robe_materials.get(&handle.0))
                    .is_some_and(|m| m.extension.params.control.y as u32 & 8 != 0);
            assert!(
                enabled,
                "ghost/projection body must use its ethereal material path"
            );
            ethereal += 1;
        }
    }
    assert!(
        native >= 2,
        "player and NPC must both have native lit body materials"
    );
    assert_eq!(
        fallback, 0,
        "character bodies must not be stranded on fallback PBR"
    );
    if pilot.script == ETHEREAL_SCRIPT {
        assert!(
            native >= 3 && ethereal >= 2,
            "both ethereal NPC models must load"
        );
    }
    eprintln!("character lighting materials: native={native} fallback={fallback}");
    *checked = true;
}

/// Diagnostic visibility controls preserve simulation, materials and animation.
/// They apply only to the combined attack autopilot, never ordinary play.
pub(super) fn isolate(
    pilot: Option<Res<Autopilot>>,
    roots: Query<(Entity, &NpcRoot)>,
    children: Query<&Children>,
    skins: Query<(), With<bevy::mesh::skinning::SkinnedMesh>>,
    items: Query<(Entity, &equipment::HeldItem)>,
    mut commands: Commands,
    mut hidden: Local<std::collections::HashSet<Entity>>,
) {
    if !pilot.is_some_and(|p| p.script == ETHEREAL_ATTACK_SCRIPT) {
        return;
    }
    let Ok(mode) = std::env::var("MAGICKA_QA_HIDE_NPC") else {
        return;
    };
    assert!(
        ["all", "ghost", "projection", "bodies", "equipment"].contains(&mode.as_str()),
        "unknown NPC visibility control"
    );
    for (root, npc) in &roots {
        let hide_root = mode == "all"
            || (mode == "ghost" && npc.template == "ghost_acolyte")
            || (mode == "projection" && npc.template == "boss_vlad_spirit");
        if hide_root {
            commands.entity(root).insert(Visibility::Hidden);
            for entity in children.iter_descendants(root) {
                commands.entity(entity).insert(Visibility::Hidden);
            }
            if hidden.insert(root) {
                eprintln!(
                    "NPC visibility control mode={mode} template={} root={root:?}",
                    npc.template
                );
            }
        } else if mode == "bodies" {
            for entity in children.iter_descendants(root) {
                if skins.contains(entity) {
                    commands.entity(entity).insert(Visibility::Hidden);
                    for child in children.iter_descendants(entity) {
                        commands.entity(child).insert(Visibility::Hidden);
                    }
                    if !hidden.insert(entity) {
                        continue;
                    }
                    eprintln!(
                        "NPC visibility control mode={mode} template={} mesh={entity:?}",
                        npc.template
                    );
                }
            }
        }
        // Held models are independent world roots, not body descendants.
        if hide_root || mode == "equipment" {
            for (entity, item) in &items {
                if item.owner == npc.handle {
                    commands.entity(entity).insert(Visibility::Hidden);
                    for child in children.iter_descendants(entity) {
                        commands.entity(child).insert(Visibility::Hidden);
                    }
                    if !hidden.insert(entity) {
                        continue;
                    }
                    eprintln!(
                        "NPC visibility control mode={mode} template={} item={entity:?} slot={}",
                        npc.template, item.slot
                    );
                }
            }
        }
    }
}
