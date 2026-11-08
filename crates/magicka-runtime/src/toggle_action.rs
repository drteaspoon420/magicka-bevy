//! Light, POI, tutorial element/cast-type, hint, achievement and debug trigger
//! actions: single host operations with their own QuickExecute rules.
use crate::{
    ai_event::clr_enum,
    script_attributes::{boolean, identifier, number, ScriptReadError},
};

pub const ELEMENTS: &[(&str, i32)] = &[
    ("none", 0),
    ("earth", 1),
    ("physical", 1),
    ("water", 2),
    ("cold", 4),
    ("fire", 8),
    ("lightning", 0x10),
    ("arcane", 0x20),
    ("life", 0x40),
    ("shield", 0x80),
    ("ice", 0x100),
    ("steam", 0x200),
    ("poison", 0x400),
    ("offensive", 0xff3f),
    ("defensive", 0xb0),
    ("all", 0xffff),
    ("magick", 0xffff),
    ("basic", 0xff),
    ("instant", 0x371),
    ("instantphysical", 0x171),
    ("instantnonphysical", 0x270),
    ("statuseffects", 0x64e),
    ("shieldelements", 0xe0),
    ("physicalelements", 0x101),
    ("beams", 0x60),
];
pub const CAST_TYPES: &[(&str, i32)] = &[
    ("none", 0),
    ("blast", 0),
    ("force", 1),
    ("area", 2),
    ("self", 3),
    ("weapon", 4),
    ("magick", 5),
];
pub const LIGHT_TRANSITIONS: &[(&str, i32)] = &[("none", 0), ("linear", 1), ("flicker", 2)];

#[derive(Clone, Debug, PartialEq)]
pub enum ToggleKind {
    /// `ids` is `None` when Id was never set (Execute dereferences null).
    Light {
        enable: bool,
        ids: Option<Vec<i32>>,
        effect: i32,
        time: f32,
    },
    Poi {
        enable: bool,
        id: i32,
    },
    Element {
        enable: bool,
        elements: i32,
    },
    CastType {
        enable: bool,
        cast_type: i32,
    },
    Push {
        enable: bool,
    },
    UnlockMagick {
        magick: u8,
    },
    RemoveHint,
    /// Lowercased achievement name; `None` passes null.
    UnlockAchievement {
        name: Option<String>,
    },
    /// Native Execute is empty.
    PrintDebug,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToggleAction {
    pub delay: f32,
    pub kind: ToggleKind,
}

pub const TAGS: &[&str] = &[
    "disablelight",
    "enablelight",
    "disablepoi",
    "enablepoi",
    "enableelement",
    "disableelement",
    "enablecasttype",
    "disablecasttype",
    "enablepush",
    "disablepush",
    "unlockmagick",
    "removehint",
    "unlockachievement",
    "printdebug",
];

pub trait ToggleHost {
    type Error;
    /// LevelModel.Lights[id].Enable(scene, effect, time) / Disable(effect, time);
    /// `false` when the light key is missing (the original throws).
    fn set_light(
        &mut self,
        id: i32,
        enable: bool,
        effect: i32,
        time: f32,
    ) -> Result<bool, Self::Error>;
    /// (GameScene.Triggers[id] as Interactable).Enabled; `false` when the key is
    /// missing or not an Interactable (KeyNotFound / NullReference natively).
    fn set_interactable_enabled(&mut self, id: i32, enabled: bool) -> Result<bool, Self::Error>;
    fn set_tutorial_element(&mut self, elements: i32, enabled: bool) -> Result<(), Self::Error>;
    fn set_tutorial_cast_type(&mut self, cast_type: i32, enabled: bool) -> Result<(), Self::Error>;
    fn set_tutorial_push(&mut self, enabled: bool) -> Result<(), Self::Error>;
    fn unlock_magick(&mut self, magick: u8) -> Result<(), Self::Error>;
    fn remove_hint(&mut self) -> Result<(), Self::Error>;
    fn award_achievement(&mut self, name: Option<&str>) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum ToggleError<E> {
    MissingLightIds,
    MissingLight(i32),
    MissingInteractable(i32),
    Host(E),
}
impl<E> From<E> for ToggleError<E> {
    fn from(e: E) -> Self {
        Self::Host(e)
    }
}

impl ToggleAction {
    pub fn read<'a>(
        tag: &str,
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        use ToggleKind::*;
        let lower = tag.to_ascii_lowercase();
        let mut kind = match lower.as_str() {
            "disablelight" | "enablelight" => Light {
                enable: lower == "enablelight",
                ids: None,
                effect: 0,
                time: 0.0,
            },
            "disablepoi" | "enablepoi" => Poi {
                enable: lower == "enablepoi",
                id: 0,
            },
            "enableelement" | "disableelement" => Element {
                enable: lower == "enableelement",
                elements: 0,
            },
            "enablecasttype" | "disablecasttype" => CastType {
                enable: lower == "enablecasttype",
                cast_type: 0,
            },
            "enablepush" | "disablepush" => Push {
                enable: lower == "enablepush",
            },
            "unlockmagick" => UnlockMagick { magick: 0 },
            "removehint" => RemoveHint,
            "unlockachievement" => UnlockAchievement { name: None },
            "printdebug" => PrintDebug,
            _ => return Err(ScriptReadError::UnsupportedTag(tag.into())),
        };
        let mut delay = 0.0;
        for (key, value) in attrs {
            match (&mut kind, key.to_ascii_lowercase().as_str()) {
                (_, "delay") => delay = number(value, key)?,
                (Light { ids, .. }, "id") => {
                    // The setter lowercases and splits on commas without trimming.
                    *ids = Some(value.split(',').map(identifier).collect::<Result<_, _>>()?)
                }
                (Light { effect, .. }, "effect") => {
                    *effect = clr_enum(value, LIGHT_TRANSITIONS, key, i32::MAX)?
                }
                (Light { time, .. }, "time") => *time = number(value, key)?,
                (Poi { id, .. }, "id") => *id = identifier(value)?,
                (Element { elements, .. }, "element") => {
                    *elements = clr_enum(value, ELEMENTS, key, i32::MAX)?
                }
                (CastType { cast_type, .. }, "type") => {
                    *cast_type = clr_enum(value, CAST_TYPES, key, i32::MAX)?
                }
                (UnlockMagick { magick }, "magicktype") => {
                    *magick = crate::magick_books::magick_id(value)
                        .ok_or_else(|| ScriptReadError::InvalidAttribute(key.into()))?;
                }
                (UnlockAchievement { name }, "achievement") => {
                    if !value.is_ascii() {
                        return Err(ScriptReadError::UnsupportedName(value.into()));
                    }
                    *name = Some(value.to_ascii_lowercase())
                }
                // PrintDebug's writable Message/Break/CounterName/EntityPos are unused.
                (PrintDebug, "break") => {
                    boolean(value, key)?;
                }
                _ => {}
            }
        }
        Ok(Self { delay, kind })
    }

    pub fn execute<H: ToggleHost + ?Sized>(
        &self,
        host: &mut H,
    ) -> Result<(), ToggleError<H::Error>> {
        self.run(host, false)
    }

    /// Lights repeat with LightTransitionType.None and time 0; achievements do
    /// nothing; every other toggle repeats Execute.
    pub fn quick_execute<H: ToggleHost + ?Sized>(
        &self,
        host: &mut H,
    ) -> Result<(), ToggleError<H::Error>> {
        self.run(host, true)
    }

    fn run<H: ToggleHost + ?Sized>(
        &self,
        host: &mut H,
        quick: bool,
    ) -> Result<(), ToggleError<H::Error>> {
        match &self.kind {
            ToggleKind::Light {
                enable,
                ids,
                effect,
                time,
            } => {
                let ids = ids.as_ref().ok_or(ToggleError::MissingLightIds)?;
                let (effect, time) = if quick { (0, 0.0) } else { (*effect, *time) };
                for &id in ids {
                    if !host.set_light(id, *enable, effect, time)? {
                        return Err(ToggleError::MissingLight(id));
                    }
                }
            }
            ToggleKind::Poi { enable, id } => {
                if !host.set_interactable_enabled(*id, *enable)? {
                    return Err(ToggleError::MissingInteractable(*id));
                }
            }
            ToggleKind::Element { enable, elements } => {
                host.set_tutorial_element(*elements, *enable)?
            }
            ToggleKind::CastType { enable, cast_type } => {
                host.set_tutorial_cast_type(*cast_type, *enable)?
            }
            ToggleKind::Push { enable } => host.set_tutorial_push(*enable)?,
            ToggleKind::UnlockMagick { magick } => host.unlock_magick(*magick)?,
            ToggleKind::RemoveHint => host.remove_hint()?,
            ToggleKind::UnlockAchievement { name } => {
                if !quick {
                    host.award_achievement(name.as_deref())?;
                }
            }
            ToggleKind::PrintDebug => {}
        }
        Ok(())
    }
}
