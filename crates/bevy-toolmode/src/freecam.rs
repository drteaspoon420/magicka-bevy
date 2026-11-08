//! Free camera decoupled from the game's own camera. While tool mode is on,
//! WASD/QE only fly while the secondary button is held (those keys are the
//! hotbar otherwise); arrow keys and PageUp/PageDown always fly.
use crate::{ToolButtons, ToolCamera, ToolMode, ToolModeSettings};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

/// Radians of look per pixel of mouse travel.
const LOOK_SENSITIVITY: f32 = 0.003;
const PITCH_LIMIT: f32 = 1.54;

#[derive(Resource, Clone, Debug)]
pub struct FreeCam {
    pub position: Vec3,
    /// Rotation about +Y; 0 looks down -Z.
    pub yaw: f32,
    /// Rotation about the camera's right axis; negative looks down.
    pub pitch: f32,
    /// Flight speed in world units per second.
    pub speed: f32,
    /// Vertical field of view (radians).
    pub fov: f32,
    /// The pose was taken from the camera when the freecam was switched on.
    pub initialised: bool,
}

impl Default for FreeCam {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 10.0, 10.0),
            yaw: 0.0,
            pitch: -0.6,
            speed: 12.0,
            fov: std::f32::consts::FRAC_PI_4,
            initialised: false,
        }
    }
}

/// One frame of freecam input in camera-local axes.
#[derive(Clone, Copy, Debug, Default)]
pub struct FreeCamInput {
    /// x right, y up, z forward; each in -1..=1.
    pub movement: Vec3,
    /// Mouse travel in pixels.
    pub look: Vec2,
    /// Multiply speed by 4.
    pub boost: bool,
    /// Scroll steps; each changes speed by 20%.
    pub speed_steps: f32,
}

impl FreeCam {
    pub fn rotation(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0)
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation() * Vec3::NEG_Z
    }

    pub fn transform(&self) -> Transform {
        Transform::from_translation(self.position).with_rotation(self.rotation())
    }

    /// Take position and look direction from an existing camera transform.
    pub fn look_from(&mut self, transform: &Transform) {
        self.position = transform.translation;
        let forward = transform.forward();
        self.yaw = (-forward.x).atan2(-forward.z);
        self.pitch = forward.y.clamp(-1.0, 1.0).asin();
        self.initialised = true;
    }

    pub fn step(&mut self, input: FreeCamInput, dt: f32) {
        self.yaw -= input.look.x * LOOK_SENSITIVITY;
        self.pitch =
            (self.pitch - input.look.y * LOOK_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        self.speed = (self.speed * 1.2f32.powf(input.speed_steps)).clamp(0.5, 500.0);
        let rotation = Quat::from_rotation_y(self.yaw);
        let local = input.movement.clamp_length_max(1.0);
        // Horizontal movement follows yaw only so looking down does not dive.
        let delta = rotation * Vec3::new(local.x, 0.0, -local.z) + Vec3::Y * local.y;
        let speed = self.speed * if input.boost { 4.0 } else { 1.0 };
        self.position += delta * speed * dt;
    }
}

pub(crate) fn drive(
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time<Real>>,
    mode: Res<ToolMode>,
    buttons: Res<ToolButtons>,
    mut cam: ResMut<FreeCam>,
    cameras: Query<&Transform, With<ToolCamera>>,
) {
    if !mode.freecam {
        cam.initialised = false;
        return;
    }
    if !cam.initialised {
        if let Some(t) = cameras.iter().next() {
            cam.look_from(t);
        }
        cam.initialised = true;
    }
    let axis = |a: KeyCode, b: KeyCode| f32::from(keys.pressed(a)) - f32::from(keys.pressed(b));
    let mut movement = Vec3::new(
        axis(KeyCode::ArrowRight, KeyCode::ArrowLeft),
        axis(KeyCode::PageUp, KeyCode::PageDown),
        axis(KeyCode::ArrowUp, KeyCode::ArrowDown),
    );
    let letters = buttons.looking || !mode.active;
    if letters {
        movement += Vec3::new(
            axis(KeyCode::KeyD, KeyCode::KeyA),
            axis(KeyCode::KeyE, KeyCode::KeyQ),
            axis(KeyCode::KeyW, KeyCode::KeyS),
        );
    }
    let input = FreeCamInput {
        movement,
        look: if buttons.looking {
            motion.delta
        } else {
            Vec2::ZERO
        },
        boost: keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
        // One speed step per wheel event direction while looking.
        speed_steps: if buttons.looking && scroll.delta.y != 0.0 {
            scroll.delta.y.signum()
        } else {
            0.0
        },
    };
    cam.step(input, time.delta_secs().min(0.1));
}

pub(crate) fn apply_to_camera(
    mode: Res<ToolMode>,
    settings: Res<ToolModeSettings>,
    cam: Res<FreeCam>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<ToolCamera>>,
) {
    if !mode.freecam || !settings.apply_freecam_to_camera {
        return;
    }
    for (mut transform, mut projection) in &mut cameras {
        *transform = cam.transform();
        if let Projection::Perspective(p) = &mut *projection {
            p.fov = cam.fov;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_from_round_trips_a_camera_direction() {
        let t = Transform::from_xyz(1.0, 8.0, 3.0).looking_at(Vec3::new(4.0, 0.0, -2.0), Vec3::Y);
        let mut cam = FreeCam::default();
        cam.look_from(&t);
        assert!(cam.forward().distance(*t.forward()) < 1e-4);
        assert_eq!(cam.position, t.translation);
    }

    #[test]
    fn forward_flight_stays_level_when_looking_down() {
        let mut cam = FreeCam {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: -1.0,
            speed: 10.0,
            ..default()
        };
        cam.step(
            FreeCamInput {
                movement: Vec3::Z,
                ..default()
            },
            0.5,
        );
        assert!((cam.position - Vec3::new(0.0, 0.0, -5.0)).length() < 1e-5);
    }

    #[test]
    fn look_clamps_pitch_and_scroll_scales_speed() {
        let mut cam = FreeCam::default();
        cam.step(
            FreeCamInput {
                look: Vec2::new(0.0, -10_000.0),
                speed_steps: 1.0,
                ..default()
            },
            0.0,
        );
        assert_eq!(cam.pitch, PITCH_LIMIT);
        assert!((cam.speed - 14.4).abs() < 1e-4);
    }
}
