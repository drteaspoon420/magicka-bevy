//! AnimatedModelAdditive for ordinary NPC bodies using native lit materials.
use super::*;
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn sync(
    play: Res<Play>,
    roots: Query<(Entity, &NpcRoot)>,
    children: Query<&Children>,
    meshes: Query<
        (
            Option<&MeshMaterial3d<crate::lighting::LitMaterial>>,
            Option<&MeshMaterial3d<crate::lighting::PlayerLitMaterial>>,
        ),
        (
            With<bevy::mesh::skinning::SkinnedMesh>,
            Without<crate::self_shield::SolidShieldPass>,
        ),
    >,
    mut lit: ResMut<Assets<crate::lighting::LitMaterial>>,
    mut entities: ResMut<Assets<crate::lighting::PlayerLitMaterial>>,
    mut original_lit: Local<HashMap<AssetId<crate::lighting::LitMaterial>, AlphaMode>>,
    mut original_entity: Local<HashMap<AssetId<crate::lighting::PlayerLitMaterial>, AlphaMode>>,
) {
    for (root, npc) in &roots {
        let Some(n) = play.session.world.npcs.get(&npc.handle) else {
            continue;
        };
        let ethereal = n.character.ethereal || n.character.definition.looks_ethereal;
        let alpha = n.character.ethereal_alpha;
        for entity in children.iter_descendants(root) {
            let Ok((a, b)) = meshes.get(entity) else {
                continue;
            };
            if let Some(handle) = a {
                let active = ethereal
                    && lit
                        .get(&handle.0)
                        .is_some_and(|m| m.extension.params.material.w < 0.5);
                if lit.get(&handle.0).is_some_and(|m| {
                    m.extension.params.maps.y != alpha
                        || m.extension.params.maps.z != f32::from(u8::from(active))
                }) {
                    let mut m = lit.get_mut(&handle.0).unwrap();
                    m.extension.params.maps.y = alpha;
                    m.extension.params.maps.z = if active { 1. } else { 0. };
                    if active {
                        original_lit
                            .entry(handle.0.id())
                            .or_insert(m.base.alpha_mode);
                        m.base.alpha_mode = AlphaMode::Blend;
                    } else if let Some(mode) = original_lit.remove(&handle.0.id()) {
                        m.base.alpha_mode = mode;
                    }
                }
            }
            if let Some(handle) = b {
                let active = ethereal
                    && entities
                        .get(&handle.0)
                        .is_some_and(|m| m.extension.params.material.w < 0.5);
                if entities.get(&handle.0).is_some_and(|m| {
                    m.extension.params.maps.y != alpha
                        || m.extension.params.maps.z != f32::from(u8::from(active))
                }) {
                    let mut m = entities.get_mut(&handle.0).unwrap();
                    m.extension.params.maps.y = alpha;
                    m.extension.params.maps.z = if active { 1. } else { 0. };
                    if active {
                        original_entity
                            .entry(handle.0.id())
                            .or_insert(m.base.alpha_mode);
                        m.base.alpha_mode = AlphaMode::Blend;
                    } else if let Some(mode) = original_entity.remove(&handle.0.id()) {
                        m.base.alpha_mode = mode;
                    }
                }
            }
        }
    }
    original_lit.retain(|id, _| lit.contains(*id));
    original_entity.retain(|id, _| entities.contains(*id));
}
