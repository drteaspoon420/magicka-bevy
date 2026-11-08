//! Frozen live-avatar regression for the shared particle renderer. Fixture
//! sizes/colours are audit inputs, not reconstructed emitter constants.
use super::{script_act, Act, Autopilot, Play, Step};
use crate::particles::{ParticleDepthSource, ParticleFrame};
use bevy::prelude::*;
use magicka_runtime::particles::ParticleGraphics;

pub(super) const SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (20, [0.0, 0.5], Act::ParticleDepthBaseline, "baseline"),
    (20, [0.0, 0.5], Act::ParticleDepthNoItems, "no_items"),
    (20, [0.0, 0.5], Act::ParticleDepthBehind, "behind"),
    (20, [0.0, 0.5], Act::ParticleDepthOldSource, "old_source"),
    (20, [0.0, 0.5], Act::ParticleDepthFront, "front"),
    (20, [0.0, 0.5], Act::ParticleDepthBaseline, "restored"),
];

pub(super) fn fixture(
    pilot: Option<Res<Autopilot>>,
    play: Res<Play>,
    mut particles: ResMut<ParticleFrame>,
    mut source: ResMut<ParticleDepthSource>,
    mut post: Query<&mut crate::post::MagickaPost>,
    mut items: Query<&mut Visibility, With<crate::outline::OutlinedEquipment>>,
) {
    let Some(pilot) = pilot else {
        return;
    };
    let Some(start) = pilot.started else {
        return;
    };
    let act = script_act(pilot.script, pilot.frame - start);
    if !matches!(
        act,
        Some(
            Act::ParticleDepthBaseline
                | Act::ParticleDepthBehind
                | Act::ParticleDepthOldSource
                | Act::ParticleDepthFront
                | Act::ParticleDepthNoItems
        )
    ) {
        return;
    }
    particles.0.clear();
    // Bloom from correctly visible surrounding particles may brighten opaque
    // robe pixels. Disable it for this direct-occlusion measurement.
    for mut settings in &mut post {
        settings.bloom.y = 0.0;
    }
    for mut visibility in &mut items {
        *visibility = if act == Some(Act::ParticleDepthNoItems) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
    *source = if act == Some(Act::ParticleDepthOldSource) {
        ParticleDepthSource::OutlinePrepass
    } else {
        ParticleDepthSource::FullScene
    };
    if matches!(
        act,
        Some(Act::ParticleDepthBaseline | Act::ParticleDepthNoItems)
    ) {
        return;
    }
    let forward = Vec3::from(play.session.camera().direction()).normalize();
    let up = Vec3::from(play.session.camera().up()).normalize();
    let right = forward.cross(up).normalize();
    let center = Vec3::from(play.session.avatar_transform().0) + Vec3::Y * 1.2;
    // Separate footprints keep the body fixture away from the staff's
    // farther horn; otherwise its native spherical soft fade overlaps it.
    for (center, distance, radius) in [
        (center, 1.4, 0.35),
        (center - right * 0.45 + up * 0.5, 2.4, 0.3),
    ] {
        let distance = if act == Some(Act::ParticleDepthFront) {
            -distance
        } else {
            distance
        };
        for sprite in [0, 7, 64, 128] {
            for _ in 0..16 {
                particles.0.push(ParticleGraphics {
                    position: (center + forward * distance).to_array(),
                    rotation: 0.0,
                    radius,
                    normalized_lifetime: 0.5,
                    color: [4.0; 4],
                    sprite,
                    alpha_blended: 0,
                    hsv_colorize: 0,
                    rotation_aligned: 0,
                    velocity: [0.0; 3],
                });
            }
        }
    }
}
