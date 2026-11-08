//! Cutscene, input-limit, fade and camera trigger actions. Each forwards to one
//! PlayState/RenderManager/camera operation through `SceneCommandHost`.
use crate::{
    ai_event::clr_enum,
    script_attributes::{boolean, identifier, number, ScriptReadError},
};

/// XNA 3.1 PackUtils.PackUNorm(255, v): single multiply, NaN to zero, infinite
/// and out-of-range values clamped, Math.Round (ties to even).
pub fn pack_unorm8(value: f32) -> u8 {
    let scaled = value * 255.0;
    if scaled.is_nan() || scaled < 0.0 {
        0
    } else if scaled > 255.0 {
        255
    } else {
        (scaled as f64).round_ties_even() as u8
    }
}

/// Action.Read's Color conversion: four invariant floats into Color(Vector4).
/// Extra components are ignored; fewer than four fail like the native indexer.
pub fn xna_color(value: &str, key: &str) -> Result<[u8; 4], ScriptReadError> {
    let parts: Vec<_> = value.split(',').collect();
    if parts.len() < 4 {
        return Err(ScriptReadError::InvalidAttribute(key.into()));
    }
    let mut out = [0; 4];
    for (o, p) in out.iter_mut().zip(&parts) {
        *o = pack_unorm8(number(p, key)?);
    }
    Ok(out)
}

/// Action.Read's Vector3 conversion; extra components are ignored.
pub fn xna_vector3(value: &str, key: &str) -> Result<[f32; 3], ScriptReadError> {
    let parts: Vec<_> = value.split(',').collect();
    if parts.len() < 3 {
        return Err(ScriptReadError::InvalidAttribute(key.into()));
    }
    Ok([
        number(parts[0], key)?,
        number(parts[1], key)?,
        number(parts[2], key)?,
    ])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraInterpolation {
    Linear,
    Interpolated,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SceneCommandKind {
    Cutscene {
        on_skip: i32,
        skip_bar_move: bool,
        kill_dialogs: bool,
    },
    EndCutscene {
        skip_bar_move: bool,
    },
    DisableInput,
    EnableInput,
    Fade {
        fade_in: bool,
        /// RGBA bytes.
        color: [u8; 4],
        time: f32,
        override_transition: bool,
    },
    CameraRelease {
        time: f32,
    },
    CameraMove {
        lock_input: bool,
        time: f32,
        target: i32,
        offset: [f32; 3],
        magnification: f32,
        interpolation: CameraInterpolation,
    },
    CameraShake {
        magnitude: f32,
        time: f32,
        /// `None` when Area is null or empty (string.IsNullOrEmpty).
        area: Option<i32>,
    },
    CameraMagnify {
        magnification: f32,
        time: f32,
    },
    CameraLock {
        time: f32,
        area: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneCommand {
    pub delay: f32,
    pub kind: SceneCommandKind,
}

pub const TAGS: &[&str] = &[
    "cutscene",
    "endcutscene",
    "disableinput",
    "enableinput",
    "fade",
    "camerarelease",
    "cameramove",
    "camerashake",
    "cameramagnify",
    "cameralock",
];

pub trait SceneCommandHost {
    type Error;
    fn begin_cutscene(
        &mut self,
        on_skip: i32,
        skip_bar_move: bool,
        kill_dialogs: bool,
    ) -> Result<(), Self::Error>;
    fn end_cutscene(&mut self, skip_bar_move: bool) -> Result<(), Self::Error>;
    /// ControlManager.LimitInput / UnlimitInput(GameScene.Level).
    fn limit_input(&mut self, limited: bool) -> Result<(), Self::Error>;
    /// RenderManager.BeginTransition (fade in) or EndTransition, Transitions.Fade.
    fn fade(&mut self, fade_in: bool, color: [u8; 4], time: f32) -> Result<(), Self::Error>;
    /// Level.ClearTransition.
    fn clear_transition(&mut self) -> Result<(), Self::Error>;
    fn camera_release(&mut self, time: f32) -> Result<(), Self::Error>;
    /// GameScene.TryGetLocator: static locators then animated parts.
    fn try_locator(&mut self, id: i32) -> Result<Option<[f32; 16]>, Self::Error>;
    fn camera_ground_position(&mut self) -> Result<[f32; 3], Self::Error>;
    fn camera_move_to(&mut self, target: [f32; 3], time: f32) -> Result<(), Self::Error>;
    fn set_camera_interpolation(&mut self, value: CameraInterpolation) -> Result<(), Self::Error>;
    fn set_camera_magnification(&mut self, value: f32) -> Result<(), Self::Error>;
    fn set_camera_time(&mut self, value: f32) -> Result<(), Self::Error>;
    fn set_camera_lock_input(&mut self, value: bool) -> Result<(), Self::Error>;
    /// GameScene.GetLocator(int, out Matrix), including its exception.
    fn locator(&mut self, id: i32) -> Result<[f32; 16], Self::Error>;
    fn camera_shake(
        &mut self,
        at: Option<[f32; 3]>,
        magnitude: f32,
        time: f32,
    ) -> Result<(), Self::Error>;
    /// LockOn(TriggerAreas[area] box, time); `false` when the area key is missing.
    fn camera_lock_on(&mut self, area: i32, time: f32) -> Result<bool, Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum SceneCommandError<E> {
    MissingArea(i32),
    Host(E),
}
impl<E> From<E> for SceneCommandError<E> {
    fn from(e: E) -> Self {
        Self::Host(e)
    }
}

impl SceneCommand {
    pub fn read<'a>(
        tag: &str,
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        use SceneCommandKind::*;
        let mut kind = match tag.to_ascii_lowercase().as_str() {
            "cutscene" => Cutscene {
                on_skip: 0,
                skip_bar_move: false,
                kill_dialogs: true,
            },
            "endcutscene" => EndCutscene {
                skip_bar_move: false,
            },
            "disableinput" => DisableInput,
            "enableinput" => EnableInput,
            "fade" => Fade {
                fade_in: false,
                color: [0; 4],
                time: 0.0,
                override_transition: false,
            },
            "camerarelease" => CameraRelease { time: 1.0 },
            "cameramove" => CameraMove {
                lock_input: false,
                time: 1.0,
                target: 0,
                offset: [0.0; 3],
                magnification: -1.0,
                interpolation: CameraInterpolation::Interpolated,
            },
            "camerashake" => CameraShake {
                magnitude: 0.0,
                time: 0.0,
                area: None,
            },
            "cameramagnify" => CameraMagnify {
                magnification: 1.0,
                time: 1.0,
            },
            "cameralock" => CameraLock { time: 1.0, area: 0 },
            _ => return Err(ScriptReadError::UnsupportedTag(tag.into())),
        };
        let mut delay = 0.0;
        for (key, value) in attrs {
            let lower = key.to_ascii_lowercase();
            if lower == "delay" {
                delay = number(value, key)?;
                continue;
            }
            match (&mut kind, lower.as_str()) {
                (Cutscene { on_skip, .. }, "onskip") => *on_skip = identifier(value)?,
                (Cutscene { skip_bar_move, .. }, "skipbarmove")
                | (EndCutscene { skip_bar_move }, "skipbarmove") => {
                    *skip_bar_move = boolean(value, key)?
                }
                (Cutscene { kill_dialogs, .. }, "killdialogs") => {
                    *kill_dialogs = boolean(value, key)?
                }
                (Fade { fade_in, .. }, "fadein") => *fade_in = boolean(value, key)?,
                (Fade { color, .. }, "color") => *color = xna_color(value, key)?,
                (Fade { time, .. }, "time")
                | (CameraRelease { time }, "time")
                | (CameraMove { time, .. }, "time")
                | (CameraShake { time, .. }, "time")
                | (CameraMagnify { time, .. }, "time")
                | (CameraLock { time, .. }, "time") => *time = number(value, key)?,
                (
                    Fade {
                        override_transition,
                        ..
                    },
                    "override",
                ) => *override_transition = boolean(value, key)?,
                (CameraMove { lock_input, .. }, "lockinput") => *lock_input = boolean(value, key)?,
                (CameraMove { target, .. }, "target") => *target = identifier(value)?,
                (CameraMove { offset, .. }, "offset") => *offset = xna_vector3(value, key)?,
                (CameraMove { magnification, .. }, "magnification")
                | (CameraMagnify { magnification, .. }, "magnification") => {
                    *magnification = number(value, key)?
                }
                (CameraMove { interpolation, .. }, "interpolation") => {
                    *interpolation =
                        match clr_enum(value, &[("linear", 0), ("interpolated", 1)], key, 255)? {
                            0 => CameraInterpolation::Linear,
                            1 => CameraInterpolation::Interpolated,
                            // Undefined byte values are not modeled.
                            _ => return Err(ScriptReadError::InvalidAttribute(key.into())),
                        }
                }
                (CameraShake { magnitude, .. }, "magnitude") => *magnitude = number(value, key)?,
                (CameraShake { area, .. }, "area") => {
                    *area = if value.is_empty() {
                        None
                    } else {
                        Some(identifier(value)?)
                    }
                }
                (CameraLock { area, .. }, "area") => {
                    // The setter hashes, then throws for an empty string.
                    if value.is_empty() {
                        return Err(ScriptReadError::InvalidAttribute(key.into()));
                    }
                    *area = identifier(value)?
                }
                _ => {}
            }
        }
        Ok(Self { delay, kind })
    }

    pub fn execute<H: SceneCommandHost + ?Sized>(
        &self,
        host: &mut H,
    ) -> Result<(), SceneCommandError<H::Error>> {
        use SceneCommandKind::*;
        match self.kind {
            Cutscene {
                on_skip,
                skip_bar_move,
                kill_dialogs,
            } => host.begin_cutscene(on_skip, skip_bar_move, kill_dialogs)?,
            EndCutscene { skip_bar_move } => host.end_cutscene(skip_bar_move)?,
            DisableInput => host.limit_input(true)?,
            EnableInput => host.limit_input(false)?,
            Fade {
                fade_in,
                color,
                time,
                override_transition,
            } => {
                host.fade(fade_in, color, time)?;
                if override_transition {
                    host.clear_transition()?;
                }
            }
            CameraRelease { time } => host.camera_release(time)?,
            CameraMove {
                lock_input,
                time,
                target,
                offset,
                magnification,
                interpolation,
            } => {
                let base = match host.try_locator(target)? {
                    Some(m) => [m[12], m[13], m[14]],
                    None => host.camera_ground_position()?,
                };
                let target = [
                    base[0] + offset[0],
                    base[1] + offset[1],
                    base[2] + offset[2],
                ];
                host.camera_move_to(target, time)?;
                host.set_camera_interpolation(interpolation)?;
                if magnification > 0.0 {
                    host.set_camera_magnification(magnification)?;
                }
                host.set_camera_lock_input(lock_input)?;
            }
            CameraShake {
                magnitude,
                time,
                area,
            } => {
                let at = match area {
                    Some(id) => {
                        let m = host.locator(id)?;
                        Some([m[12], m[13], m[14]])
                    }
                    None => None,
                };
                host.camera_shake(at, magnitude, time)?;
            }
            CameraMagnify {
                magnification,
                time,
            } => {
                host.set_camera_magnification(magnification)?;
                host.set_camera_time(time)?;
            }
            CameraLock { time, area } => {
                if !host.camera_lock_on(area, time)? {
                    return Err(SceneCommandError::MissingArea(area));
                }
            }
        }
        Ok(())
    }

    /// QuickExecute: cutscene transitions force SkipBarMove, input limits repeat
    /// Execute, and fade/camera commands do nothing.
    pub fn quick_execute<H: SceneCommandHost + ?Sized>(
        &self,
        host: &mut H,
    ) -> Result<(), SceneCommandError<H::Error>> {
        use SceneCommandKind::*;
        match self.kind {
            Cutscene {
                on_skip,
                kill_dialogs,
                ..
            } => host.begin_cutscene(on_skip, true, kill_dialogs)?,
            EndCutscene { .. } => host.end_cutscene(true)?,
            DisableInput | EnableInput => self.execute(host)?,
            _ => {}
        }
        Ok(())
    }
}
