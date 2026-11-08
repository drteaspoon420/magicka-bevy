//! The cursor ray and where it lands. [`cast`] computes the ray from the
//! [`ToolCamera`] and a fallback hit on a horizontal plane; games with real
//! collision overwrite [`Reticle`] in [`crate::ToolModeSet::Pick`].
use crate::{gizmo::ToolGizmos, tools, ToolCamera, ToolMode, ToolModeSettings};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

#[derive(Resource, Default, Debug)]
pub struct ToolRay {
    /// Cursor in logical window pixels (None when outside the window).
    pub cursor: Option<Vec2>,
    pub ray: Option<Ray3d>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReticleHit {
    pub point: Vec3,
    pub normal: Vec3,
}

#[derive(Resource, Default, Debug)]
pub struct Reticle {
    pub hit: Option<ReticleHit>,
}

pub(crate) fn cast(
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &Transform), With<ToolCamera>>,
    settings: Res<ToolModeSettings>,
    mut out: ResMut<ToolRay>,
    mut reticle: ResMut<Reticle>,
) {
    out.cursor = windows.iter().next().and_then(Window::cursor_position);
    // The camera transform is used directly (cameras are root entities), so
    // a pose written earlier this frame is already in effect.
    out.ray = cameras
        .iter()
        .next()
        .zip(out.cursor)
        .and_then(|((camera, t), cursor)| {
            camera
                .viewport_to_world(&GlobalTransform::from(*t), cursor)
                .ok()
        });
    let plane = Vec3::Y * settings.ground_plane_y;
    reticle.hit = out.ray.and_then(|r| {
        let d = r.intersect_plane(plane, InfinitePlane3d::new(Vec3::Y))?;
        Some(ReticleHit {
            point: r.get_point(d),
            normal: Vec3::Y,
        })
    });
}

pub(crate) fn draw(
    mode: Res<ToolMode>,
    reticle: Res<Reticle>,
    cameras: Query<(&Camera, &Transform), With<ToolCamera>>,
    mut gizmos: Gizmos<ToolGizmos>,
) {
    if !mode.active {
        return;
    }
    let (Some(hit), Some((camera, cam))) = (reticle.hit, cameras.iter().next()) else {
        return;
    };
    // About 12 px per unit of `size`.
    let size = crate::gizmo::world_size(camera, &GlobalTransform::from(*cam), hit.point, 12.0);
    let colour = match mode.tool.as_str() {
        tools::REMOVE => Color::srgb(1.0, 0.3, 0.25),
        tools::SPAWN if mode.stage_spawns => Color::srgb(0.3, 0.9, 1.0),
        tools::SPAWN => Color::srgb(0.4, 1.0, 0.4),
        _ => Color::srgb(1.0, 1.0, 1.0),
    };
    let normal = Dir3::new(hit.normal).unwrap_or(Dir3::Y);
    let rotation = Quat::from_rotation_arc(Vec3::Z, *normal);
    let iso = Isometry3d::new(hit.point + *normal * 0.02, rotation);
    gizmos.circle(iso, size * 2.0, colour);
    gizmos.circle(iso, size * 0.4, colour);
    let (u, v) = normal.any_orthonormal_pair();
    for dir in [u, -u, v, -v] {
        gizmos.line(
            hit.point + dir * size * 1.2,
            hit.point + dir * size * 2.8,
            colour,
        );
    }
    gizmos.line(hit.point, hit.point + *normal * size * 3.0, colour);
    if mode.tool == tools::SPAWN {
        // Facing of the next spawn.
        let facing = Quat::from_rotation_y(mode.spawn_yaw) * Vec3::NEG_Z;
        gizmos
            .arrow(hit.point, hit.point + facing * size * 5.0, colour)
            .with_tip_length(size * 1.2);
    }
}
