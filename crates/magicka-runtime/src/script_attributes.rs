//! Attribute decoding for recovered condition/variable-action types. A scene
//! loader supplies XML order; unsupported script types remain explicit errors.
use crate::{
    level_variables::VariableAction,
    scene_loading::custom_hash,
    script_condition::{ConditionKind, ScriptCondition},
};
#[derive(Debug, PartialEq, Eq)]
pub enum ScriptReadError {
    UnsupportedTag(String),
    UnsupportedName(String),
    InvalidAttribute(String),
}
pub(crate) fn number<T: std::str::FromStr>(value: &str, key: &str) -> Result<T, ScriptReadError> {
    value
        .trim()
        .parse()
        .map_err(|_| ScriptReadError::InvalidAttribute(key.into()))
}
pub(crate) fn boolean(value: &str, key: &str) -> Result<bool, ScriptReadError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ScriptReadError::InvalidAttribute(key.into())),
    }
}
pub(crate) fn identifier(value: &str) -> Result<i32, ScriptReadError> {
    if !value.is_ascii() {
        return Err(ScriptReadError::UnsupportedName(value.into()));
    }
    Ok(custom_hash(&value.to_ascii_lowercase()))
}
pub(crate) fn comparison(value: &str, key: &str) -> Result<i8, ScriptReadError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "less" => Ok(-1),
        "equal" => Ok(0),
        "greater" => Ok(1),
        _ => number(value, key),
    }
}
fn counter_operation(value: &str, key: &str) -> Result<i8, ScriptReadError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "inc" => Ok(0),
        "dec" => Ok(1),
        "set" => Ok(2),
        _ => number(value, key),
    }
}
impl ScriptCondition {
    pub fn read<'a>(
        tag: &str,
        attributes: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        let kind = match tag.to_ascii_lowercase().as_str() {
            "counter" => ConditionKind::Counter {
                id: 0,
                compare: 0,
                value: 0,
            },
            "timer" => ConditionKind::Timer {
                id: 0,
                compare: 0,
                value: 0.0,
            },
            "triggered" => ConditionKind::Triggered { id: 0 },
            "nocutsceneactive" => ConditionKind::NoCutsceneActive,
            "playercount" => ConditionKind::PlayerCount {
                compare: 0,
                value: 0,
            },
            _ => return Err(ScriptReadError::UnsupportedTag(tag.into())),
        };
        let mut result = Self {
            invert: false,
            kind,
        };
        for (key, value) in attributes {
            let attribute = key.to_ascii_lowercase();
            if attribute == "invert" {
                result.invert = boolean(value, key)?;
                continue;
            }
            match &mut result.kind {
                ConditionKind::Counter {
                    id,
                    compare,
                    value: expected,
                } => match attribute.as_str() {
                    "name" => *id = identifier(value)?,
                    "comparemethod" => *compare = comparison(value, key)?,
                    "value" => *expected = number(value, key)?,
                    _ => {}
                },
                ConditionKind::Timer {
                    id,
                    compare,
                    value: expected,
                } => match attribute.as_str() {
                    "name" => *id = identifier(value)?,
                    "comparemethod" => *compare = comparison(value, key)?,
                    "value" => *expected = number(value, key)?,
                    _ => {}
                },
                ConditionKind::Triggered { id } => {
                    if attribute == "trigger" {
                        *id = identifier(value)?;
                    }
                }
                ConditionKind::PlayerCount {
                    compare,
                    value: expected,
                } => match attribute.as_str() {
                    "comparemethod" => *compare = comparison(value, key)?,
                    "nr" => *expected = number(value, key)?,
                    _ => {}
                },
                ConditionKind::NoCutsceneActive => {}
            }
        }
        Ok(result)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct DecodedVariableAction {
    pub action: VariableAction,
    pub delay: f32,
}
impl DecodedVariableAction {
    pub fn read<'a>(
        tag: &str,
        attributes: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        let action = match tag.to_ascii_lowercase().as_str() {
            "counter" => VariableAction::Counter {
                id: 0,
                operation: 0,
                value: 0,
            },
            "timer" => VariableAction::Timer {
                id: 0,
                value: None,
                paused: None,
            },
            "addtimer" => VariableAction::AddTimer {
                id: 0,
                played: false,
                value: 0.0,
            },
            _ => return Err(ScriptReadError::UnsupportedTag(tag.into())),
        };
        let mut result = Self { action, delay: 0.0 };
        for (key, value) in attributes {
            let attribute = key.to_ascii_lowercase();
            if attribute == "delay" {
                result.delay = number(value, key)?;
                continue;
            }
            match &mut result.action {
                VariableAction::Counter {
                    id,
                    operation,
                    value: expected,
                } => match attribute.as_str() {
                    "name" => *id = identifier(value)?,
                    "action" => *operation = counter_operation(value, key)?,
                    "value" => *expected = number(value, key)?,
                    _ => {}
                },
                VariableAction::Timer {
                    id,
                    value: expected,
                    paused,
                } => match attribute.as_str() {
                    "name" => *id = identifier(value)?,
                    "value" => *expected = Some(number(value, key)?),
                    "paused" => *paused = Some(boolean(value, key)?),
                    _ => {}
                },
                VariableAction::AddTimer {
                    id,
                    played,
                    value: expected,
                } => match attribute.as_str() {
                    "name" => *id = identifier(value)?,
                    "value" => *expected = number(value, key)?,
                    "playedtimer" => *played = boolean(value, key)?,
                    _ => {}
                },
            }
        }
        Ok(result)
    }
}
