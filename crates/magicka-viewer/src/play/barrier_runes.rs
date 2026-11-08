//! Barrier rune discs (research/barrier.md).
use bevy::prelude::*;

/// A Solid Arcane/Life barrier's rune disc (RenderableAdditiveObject):
/// runesArcane/runesLife tinted by the spell colour without Earth, Ice and
/// Shield, spinning with mRuneRotation below the wall (research/barrier.md).
#[derive(Component)]
pub(super) struct BarrierRune {
    slot: usize,
    key: [u32; 3],
}

pub(super) fn sync_barrier_runes(
    mut commands: Commands,
    play: Res<super::Play>,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<crate::additive::AdditiveMaterial>>,
    mut runes: Query<(Entity, &BarrierRune, &mut Transform)>,
) {
    use magicka_sim::barrier::Rune;
    let barriers = &play.session.world.barriers;
    let key = |b: &magicka_sim::barrier::Barrier| b.position.map(f32::to_bits);
    let place = |b: &magicka_sim::barrier::Barrier| {
        let (yaw, at) = b.rune_placement();
        Transform::from_translation(Vec3::from(at)).with_rotation(Quat::from_rotation_y(yaw))
    };
    for (entity, rune, mut transform) in &mut runes {
        match barriers.get(rune.slot).filter(|b| key(b) == rune.key) {
            Some(b) => *transform = place(b),
            None => commands.entity(entity).despawn(),
        }
    }
    for (slot, b) in barriers.iter() {
        if !b.solid()
            || runes
                .iter()
                .any(|(_, r, _)| r.slot == slot && r.key == key(b))
        {
            continue;
        }
        let Some((rune, color)) = b.rune() else {
            continue;
        };
        let glb = match rune {
            Rune::Arcane => "gltf/Models/Effects/runesArcane.glb",
            Rune::Life => "gltf/Models/Effects/runesLife.glb",
        };
        let material = materials.add(crate::additive::AdditiveMaterial {
            color: Vec4::new(color[0], color[1], color[2], 1.0),
            flags: UVec4::new(1, 0, 0, 0),
            texture: Some(assets.load("textures/Models/Effects/runes00_0.png")),
        });
        commands.spawn((
            BarrierRune { slot, key: key(b) },
            Mesh3d(
                assets.load(
                    GltfAssetLabel::Primitive {
                        mesh: 0,
                        primitive: 0,
                    }
                    .from_asset(glb),
                ),
            ),
            MeshMaterial3d(material),
            place(b),
            bevy::light::NotShadowCaster,
        ));
    }
}
