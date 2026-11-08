//! TutorialManager hint lifecycle, independent of presentation and scripts.
//! Native evidence: research/tutorial-hints.md.
use crate::{
    ai_event::clr_enum,
    script_attributes::{boolean, identifier, number, ScriptReadError},
};

// TutorialManager enum constants decoded from the target's Constant table.
pub const ANIMATIONS: &[(&str, i32)] = &[
    ("none", 0),
    ("interact", 1),
    ("navmagick", 2),
    ("conjure", 4),
    ("conjurewater", 8),
    ("conjurelife", 16),
    ("conjureshield", 32),
    ("conjurecold", 64),
    ("conjurelightning", 128),
    ("conjurearcane", 256),
    ("conjureearth", 512),
    ("conjurefire", 1024),
    ("castforce", 2048),
    ("castarea", 4096),
    ("castself", 8192),
    ("castmagick", 16384),
    ("block", 32768),
    ("attack", 65536),
];
pub const POSITIONS: &[(&str, i32)] = &[
    ("topleft", 0),
    ("top", 1),
    ("topright", 2),
    ("centerleft", 3),
    ("center", 4),
    ("centerright", 5),
    ("bottomleft", 6),
    ("bottom", 7),
    ("bottomright", 8),
];

#[derive(Clone, Debug)]
pub enum TutorialCommand {
    Hint {
        id: String,
        append_controller: bool,
        animation: i32,
        position: i32,
    },
    Dialog {
        id: i32,
        element: u32,
        magick: i32,
        trigger: i32,
        scale: Option<f32>,
        size: Option<[f32; 2]>,
    },
    RemoveDialog,
    /// EnableDark/DisableDark: TutorialManager's dark-screen fade alone.
    Dark(bool),
}
#[derive(Clone, Debug)]
pub struct TutorialAction {
    pub delay: f32,
    pub command: TutorialCommand,
}
impl TutorialAction {
    pub fn read<'a>(
        tag: &str,
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        let mut action = Self {
            delay: 0.0,
            command: match tag.to_ascii_lowercase().as_str() {
                "sethint" => TutorialCommand::Hint {
                    id: String::new(),
                    append_controller: false,
                    animation: 0,
                    position: 8,
                },
                "setdialoghint" => TutorialCommand::Dialog {
                    id: 0,
                    element: 0,
                    magick: 0,
                    trigger: 0,
                    scale: None,
                    size: None,
                },
                "removedialoghint" => TutorialCommand::RemoveDialog,
                "enabledark" => TutorialCommand::Dark(true),
                "disabledark" => TutorialCommand::Dark(false),
                _ => return Err(ScriptReadError::UnsupportedTag(tag.into())),
            },
        };
        for (key, value) in attrs {
            let lower = key.to_ascii_lowercase();
            if lower == "delay" {
                action.delay = number(value, key)?;
                continue;
            }
            match (&mut action.command, lower.as_str()) {
                (TutorialCommand::Hint { id, .. }, "id") => *id = value.to_ascii_lowercase(),
                (
                    TutorialCommand::Hint {
                        append_controller, ..
                    },
                    "appendcontroller",
                ) => *append_controller = boolean(value, key)?,
                (TutorialCommand::Hint { animation, .. }, "animation") => {
                    *animation = clr_enum(value, ANIMATIONS, key, i32::MAX)?
                }
                (TutorialCommand::Hint { position, .. }, "position") => {
                    *position = clr_enum(value, POSITIONS, key, i32::MAX)?
                }
                (TutorialCommand::Dialog { id, .. }, "id") => *id = identifier(value)?,
                (TutorialCommand::Dialog { trigger, .. }, "trigger") => {
                    *trigger = identifier(value)?
                }
                (TutorialCommand::Dialog { element, .. }, "element") => {
                    *element =
                        clr_enum(value, crate::toggle_action::ELEMENTS, key, i32::MAX)? as u32
                }
                // Enum.Parse(MagickType): names, NrOfMagicks or any integer.
                (TutorialCommand::Dialog { magick, .. }, "magick") => {
                    let v = value.trim();
                    *magick = crate::magick_books::magick_id(v)
                        .map(i32::from)
                        .or_else(|| {
                            v.eq_ignore_ascii_case("nrofmagicks")
                                .then_some(crate::magick_books::SENTINEL as i32)
                        })
                        .or_else(|| v.parse().ok())
                        .ok_or_else(|| ScriptReadError::InvalidAttribute(key.into()))?
                }
                // Custom is the int view of the same MagickType field
                // (an unchecked cast); the later attribute wins.
                (TutorialCommand::Dialog { magick, .. }, "custom") => *magick = number(value, key)?,
                (TutorialCommand::Dialog { scale, .. }, "scale") => {
                    let v = number(value, key)?;
                    *scale = (v > 0.0).then_some(v);
                }
                (TutorialCommand::Dialog { size, .. }, "size") => {
                    let parts: Vec<_> = value.split(',').collect();
                    if parts.len() < 2 {
                        return Err(ScriptReadError::InvalidAttribute(key.into()));
                    }
                    let v = [number(parts[0], key)?, number(parts[1], key)?];
                    *size = (v[0] > 0.0 && v[1] > 0.0).then_some(v);
                }
                _ => {}
            }
        }
        Ok(action)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub id: i32,
    pub text: String,
    /// Original controller-specific localization, wrapped with the same font.
    pub controller_text: Option<String>,
    pub animation: i32,
    pub position: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DialogHint {
    pub text: String,
    pub element: u32,
    pub magick: i32,
    pub trigger: i32,
    pub scale: Option<f32>,
    pub size: Option<[f32; 2]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dismissed {
    /// ExecuteTrigger receives no sender, and runs only for a nonzero ID.
    pub trigger: Option<i32>,
}

#[derive(Clone, Debug, Default)]
pub struct TutorialHints {
    pub hint: Option<Hint>,
    pub dialog: Option<DialogHint>,
    pub hint_alpha: f32,
    pub dialog_alpha: f32,
    pub dark_alpha: f32,
    pub time: f32,
    pub holdoff: f32,
    /// Counts SetDialogHint calls: network clients dismiss each dialog once.
    pub dialog_serial: u32,
    pending_hint: Option<Hint>,
    darkening: bool,
    fade_in: bool,
}

impl TutorialHints {
    pub fn set_hint(&mut self, hint: Hint) {
        self.time = 0.0;
        // Native animation changes immediately, before old text fades out.
        if let Some(current) = self.hint.as_mut() {
            current.animation = hint.animation;
        }
        self.pending_hint = Some(hint);
    }

    pub fn remove_hint(&mut self) {
        self.pending_hint = None;
        self.fade_in = false;
    }

    pub fn set_dialog(&mut self, dialog: DialogHint) {
        self.dialog_serial = self.dialog_serial.wrapping_add(1);
        self.dialog = Some(dialog);
        self.enable_dark();
        self.holdoff = 1.0;
    }

    /// TutorialManager.EnableDark: start (or resume) fading the screen in.
    pub fn enable_dark(&mut self) {
        self.darkening = true;
        self.fade_in = true;
    }

    /// TutorialManager.DisableDark: fade out; drawing stops at alpha 0.
    pub fn disable_dark(&mut self) {
        self.fade_in = false;
    }

    /// The hint being shown or faded in (network presentation target).
    pub fn target_hint(&self) -> Option<&Hint> {
        self.pending_hint.as_ref()
    }
    /// EnableDark is in effect (the dark screen fades in or stays).
    pub fn dark_target(&self) -> bool {
        self.darkening && self.fade_in
    }
    /// Network client: follow the host's hint and dark-screen targets. The
    /// fades still run locally in `update`; dialogs are mirrored separately
    /// because each machine dismisses its own.
    pub fn mirror(&mut self, hint: Option<Hint>, dark: bool) {
        if self.pending_hint != hint {
            match hint {
                Some(hint) => self.set_hint(hint),
                None => self.remove_hint(),
            }
        }
        if dark != self.dark_target() && self.dialog.is_none() {
            if dark {
                self.enable_dark();
            } else {
                self.disable_dark();
            }
        }
    }

    pub fn dialog_done(&self) -> bool {
        self.dialog.is_none()
    }

    /// Native RemoveDialogHint ignores dismissal during the one-second
    /// holdoff; a successful removal clears the active flag before its trigger.
    pub fn dismiss_dialog(&mut self) -> Option<Dismissed> {
        if self.holdoff > 0.0 {
            return None;
        }
        let dialog = self.dialog.take()?;
        self.fade_in = false;
        Some(Dismissed {
            trigger: (dialog.trigger != 0).then_some(dialog.trigger),
        })
    }

    /// Update's hint replacement and dark-screen fade use rate 4 per second.
    /// The caller supplies the same presentation delta as TutorialManager.
    pub fn update(&mut self, dt: f32) {
        self.holdoff -= dt;
        self.time += dt;
        if self.hint.is_some() || self.pending_hint.is_some() {
            if self.hint.as_ref().map(|h| h.id) != self.pending_hint.as_ref().map(|h| h.id) {
                self.hint_alpha = (self.hint_alpha - dt * 4.0).max(0.0);
                if self.hint_alpha <= 0.0 {
                    self.hint = self.pending_hint.clone();
                    self.hint_alpha = 0.0;
                }
            } else {
                self.hint_alpha = (self.hint_alpha + dt * 4.0).min(1.0);
            }
        }
        if self.dialog.is_some() {
            self.dialog_alpha = (self.dialog_alpha + dt * 4.0).min(1.0);
        }
        if self.darkening {
            if self.fade_in {
                self.dark_alpha = (self.dark_alpha + dt * 4.0).min(1.0);
            } else {
                self.dark_alpha = (self.dark_alpha - dt * 4.0).max(0.0);
                self.darkening = self.dark_alpha > 0.0;
            }
        }
    }
}
