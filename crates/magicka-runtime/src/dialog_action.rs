//! StartDialog/KillDialogs decoding and dispatch into recovered manager ownership.
use crate::{
    dialog_manager::{DialogManager, DialogManagerError, DialogManagerHost},
    interaction_playback::InteractionLocation,
    scene_loading::custom_hash,
    script_attributes::{boolean, identifier, number, ScriptReadError},
};
#[derive(Clone, Copy, Debug)]
pub struct StartDialogAction {
    pub dialog: i32,
    pub owner: i32,
    pub position: [f32; 3],
    pub is_2d: bool,
    pub is_3d: bool,
    /// Original writable property is not read by Execute.
    pub force_on_screen: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum DialogActionKind {
    Start(StartDialogAction),
    Kill,
}
#[derive(Clone, Copy, Debug)]
pub struct DialogAction {
    pub delay: f32,
    pub kind: DialogActionKind,
}
#[derive(Clone, Copy, Debug)]
pub struct DialogActionCharacter {
    pub owner: i32,
    pub avatar: bool,
    pub controller: Option<i32>,
}
pub trait DialogActionHost: DialogManagerHost {
    fn resolve_character(&mut self, id: i32) -> Result<Option<DialogActionCharacter>, Self::Error>;
    fn resolve_sender(&mut self, sender: u16)
        -> Result<Option<DialogActionCharacter>, Self::Error>;
    fn locator(&mut self, id: i32) -> Result<[f32; 3], Self::Error>;
    fn cutscene(&self) -> bool;
}
#[derive(Debug, PartialEq, Eq)]
pub enum DialogActionError<E> {
    Host(E),
    Manager(DialogManagerError<E>),
}
impl DialogAction {
    pub fn read<'a>(
        tag: &str,
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        let kind = match tag.to_ascii_lowercase().as_str() {
            "startdialog" => DialogActionKind::Start(StartDialogAction {
                dialog: 0,
                owner: custom_hash("interactor"),
                position: [0.0; 3],
                is_2d: false,
                is_3d: false,
                force_on_screen: false,
            }),
            "killdialogs" => DialogActionKind::Kill,
            _ => return Err(ScriptReadError::UnsupportedTag(tag.into())),
        };
        let mut action = Self { delay: 0.0, kind };
        for (key, value) in attrs {
            let key_lower = key.to_ascii_lowercase();
            if key_lower == "delay" {
                action.delay = number(value, key)?;
                continue;
            }
            let DialogActionKind::Start(start) = &mut action.kind else {
                continue;
            };
            match key_lower.as_str() {
                "dialog" => start.dialog = identifier(value)?,
                "forceonscreen" => start.force_on_screen = boolean(value, key)?,
                "position" => {
                    let parts: Vec<_> = value.split(',').collect();
                    match parts.as_slice() {
                        [x, y] => {
                            start.position[0] = number(x, key)?;
                            start.position[1] = number(y, key)?;
                            start.is_2d = true;
                            start.is_3d = false;
                        }
                        [x, y, z] => {
                            start.position[0] = number(x, key)?;
                            start.position[1] = number(y, key)?;
                            // Literal native setter overwrites Y with the third value.
                            start.position[1] = number(z, key)?;
                            start.is_2d = false;
                            start.is_3d = true;
                        }
                        _ => {
                            start.owner = identifier(value)?;
                            start.is_2d = false;
                            start.is_3d = false;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(action)
    }
    pub fn execute<H: DialogActionHost>(
        &self,
        sender: Option<u16>,
        manager: &mut DialogManager,
        host: &mut H,
    ) -> Result<Option<usize>, DialogActionError<H::Error>> {
        let DialogActionKind::Start(start) = self.kind else {
            manager.end_all(host).map_err(DialogActionError::Manager)?;
            return Ok(None);
        };
        let (location, controller) = if start.is_2d {
            (
                InteractionLocation::Screen([start.position[0], start.position[1]]),
                None,
            )
        } else if start.is_3d {
            (InteractionLocation::World(start.position), None)
        } else {
            let mut position = [0.0; 3];
            let character = if start.owner == custom_hash("interactor") {
                match sender {
                    Some(sender) => host
                        .resolve_sender(sender)
                        .map_err(DialogActionError::Host)?,
                    None => None,
                }
            } else {
                let character = host
                    .resolve_character(start.owner)
                    .map_err(DialogActionError::Host)?;
                if character.is_none() {
                    position = host.locator(start.owner).map_err(DialogActionError::Host)?;
                }
                character
            };
            match character {
                Some(character) => (
                    InteractionLocation::Entity(Some(character.owner)),
                    if character.avatar {
                        character.controller
                    } else {
                        None
                    },
                ),
                None => (InteractionLocation::World(position), None),
            }
        };
        let cutscene = host.cutscene();
        manager
            .start(start.dialog, location, controller, cutscene, host)
            .map_err(DialogActionError::Manager)
    }
    pub fn quick_execute<H: DialogActionHost>(
        &self,
        manager: &mut DialogManager,
        host: &mut H,
    ) -> Result<(), DialogActionError<H::Error>> {
        if matches!(self.kind, DialogActionKind::Kill) {
            self.execute(None, manager, host)?;
        }
        Ok(())
    }
}
