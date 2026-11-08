//! Original dialog definitions; localized text/assets are resolved by the host.
use crate::{
    dialog_enums::*,
    dialog_state::{DialogCursor, InteractionRepetition},
    scene_script::{xml, LayoutError, XmlElement},
    script_attributes::{boolean, identifier, number, ScriptReadError},
};
use std::collections::HashSet;
#[derive(Clone, Debug)]
pub struct DialogDefinition {
    pub id: i32,
    pub icon: i32,
    pub repeat_from: i32,
    pub interactions: Vec<InteractionDefinition>,
}
#[derive(Clone, Debug)]
pub struct InteractionDefinition {
    pub lock_game: bool,
    pub focus_owner: bool,
    pub repeat: i32,
    pub icon: i32,
    pub messages: Vec<MessageDefinition>,
}
#[derive(Clone, Debug)]
pub struct MessageDefinition {
    pub owner: i32,
    pub message_type: i32,
    pub force_on_screen: bool,
    pub facing: bool,
    pub width: i32,
    pub texts: Vec<TextDefinition>,
}
#[derive(Clone, Debug)]
pub struct TextDefinition {
    pub ttl: f32,
    pub animation: u16,
    pub sound_bank: u16,
    pub sound: i32,
    pub localization: i32,
    pub arguments: Option<Vec<i32>>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum DialogReadError {
    Xml(LayoutError),
    Attribute(ScriptReadError),
    UnexpectedElement(String),
    UnknownAttribute(String),
    InvalidSound,
    DuplicateDialog(i32),
}
impl From<ScriptReadError> for DialogReadError {
    fn from(e: ScriptReadError) -> Self {
        Self::Attribute(e)
    }
}
fn enum_value(
    value: &str,
    table: &[(&str, i32)],
    key: &str,
    max: Option<i32>,
) -> Result<i32, DialogReadError> {
    let parsed = if let Ok(value) = value.trim().parse::<i32>() {
        value
    } else {
        let mut combined = 0;
        for name in value.split(',') {
            let value = table
                .iter()
                .find(|(n, _)| name.trim().eq_ignore_ascii_case(n))
                .map(|(_, v)| *v)
                .ok_or_else(|| ScriptReadError::InvalidAttribute(key.into()))?;
            combined |= value;
        }
        combined
    };
    if max.is_some_and(|max| parsed < 0 || parsed > max) {
        return Err(ScriptReadError::InvalidAttribute(key.into()).into());
    }
    Ok(parsed)
}
impl DialogDefinition {
    pub fn read_collection(inputs: &[&[u8]]) -> Result<Vec<Self>, DialogReadError> {
        let mut dialogs = vec![];
        let mut ids = HashSet::new();
        for input in inputs {
            let root = xml(input).map_err(DialogReadError::Xml)?;
            if !root.name.eq_ignore_ascii_case("dialogs") {
                return Err(DialogReadError::UnexpectedElement(root.name));
            }
            for node in &root.children {
                if !node.name.eq_ignore_ascii_case("dialog") {
                    return Err(DialogReadError::UnexpectedElement(node.name.clone()));
                }
                let dialog = Self::read(node)?;
                if !ids.insert(dialog.id) {
                    return Err(DialogReadError::DuplicateDialog(dialog.id));
                }
                dialogs.push(dialog);
            }
        }
        Ok(dialogs)
    }
    fn read(node: &XmlElement) -> Result<Self, DialogReadError> {
        let mut definition = Self {
            id: 0,
            icon: 8,
            repeat_from: 0,
            interactions: vec![],
        };
        for (key, value) in &node.attributes {
            match key.to_ascii_lowercase().as_str() {
                "id" => definition.id = identifier(value)?,
                "icontext" => definition.icon = enum_value(value, ICONS, key, None)?,
                "repeatfrom" => definition.repeat_from = number(value, key)?,
                _ => return Err(DialogReadError::UnknownAttribute(key.clone())),
            }
        }
        for child in &node.children {
            if !child.name.eq_ignore_ascii_case("interact") {
                return Err(DialogReadError::UnexpectedElement(child.name.clone()));
            }
            definition
                .interactions
                .push(InteractionDefinition::read(child, definition.icon)?);
        }
        Ok(definition)
    }
    pub fn cursor(&self) -> DialogCursor {
        DialogCursor::new(
            self.interactions
                .iter()
                .map(|i| InteractionRepetition {
                    repeat: i.repeat,
                    completed: 0,
                })
                .collect(),
            self.repeat_from,
        )
    }
}
impl InteractionDefinition {
    fn read(node: &XmlElement, icon: i32) -> Result<Self, DialogReadError> {
        let mut definition = Self {
            lock_game: false,
            focus_owner: true,
            repeat: 1,
            icon,
            messages: vec![],
        };
        for (key, value) in &node.attributes {
            match key.to_ascii_lowercase().as_str() {
                "lock" => definition.lock_game = boolean(value, key)?,
                "focusowner" => definition.focus_owner = boolean(value, key)?,
                "count" => definition.repeat = number(value, key)?,
                _ => return Err(DialogReadError::UnknownAttribute(key.clone())),
            }
        }
        for child in &node.children {
            if !child.name.eq_ignore_ascii_case("message") {
                return Err(DialogReadError::UnexpectedElement(child.name.clone()));
            }
            definition.messages.push(MessageDefinition::read(child)?);
        }
        Ok(definition)
    }
}
impl MessageDefinition {
    fn read(node: &XmlElement) -> Result<Self, DialogReadError> {
        let mut definition = Self {
            owner: 0,
            message_type: 0,
            force_on_screen: true,
            facing: true,
            width: 600,
            texts: vec![],
        };
        for (key, value) in &node.attributes {
            match key.to_ascii_lowercase().as_str() {
                "character" => definition.owner = identifier(value)?,
                "type" => definition.message_type = enum_value(value, MESSAGE_TYPES, key, None)?,
                "forceonscreen" => {
                    definition.force_on_screen = boolean(value, key).unwrap_or(false)
                }
                "facing" => definition.facing = boolean(value, key).unwrap_or(false),
                "width" => {
                    if let Ok(width) = number(value, key) {
                        definition.width = width;
                    }
                }
                _ => return Err(DialogReadError::UnknownAttribute(key.clone())),
            }
        }
        for child in &node.children {
            let mut text = TextDefinition {
                ttl: -1.0,
                animation: 151,
                sound_bank: 32,
                sound: 0,
                localization: 0,
                arguments: None,
            };
            for (key, value) in &child.attributes {
                match key.to_ascii_lowercase().as_str() {
                    "ttl" => text.ttl = number(value, key)?,
                    "anim" => {
                        text.animation = enum_value(value, ANIMATIONS, key, Some(65535))? as u16
                    }
                    "sound" => {
                        let path: Vec<_> = value.split('/').collect();
                        match path.as_slice() {
                            [sound] => text.sound = identifier(sound)?,
                            [bank, sound] => {
                                text.sound_bank = enum_value(bank, BANKS, key, Some(65535))? as u16;
                                text.sound = identifier(sound)?;
                            }
                            _ => return Err(DialogReadError::InvalidSound),
                        }
                    }
                    _ => {}
                }
            }
            let pieces: Vec<_> = child.inner_text.split(',').collect();
            text.localization = identifier(pieces[0])?;
            if pieces.len() > 1 {
                text.arguments = Some(
                    pieces[1..]
                        .iter()
                        .map(|v| identifier(v))
                        .collect::<Result<_, _>>()?,
                );
            }
            definition.texts.push(text);
        }
        Ok(definition)
    }
}
