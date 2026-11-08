//! ExecuteTrigger and ClearDelayedActions definitions; lifecycle is runtime-owned.
use crate::{animation_action::ActionReadError, scene_loading::custom_hash};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerControlKind {
    Execute,
    ClearDelayed,
}
#[derive(Clone, Copy, Debug)]
pub struct TriggerControlAction {
    pub owner: i32,
    pub target: i32,
    pub delay: f32,
    pub kind: TriggerControlKind,
}
impl TriggerControlAction {
    pub fn read<'a>(
        tag: &str,
        owner: i32,
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ActionReadError> {
        let kind = match tag.to_ascii_lowercase().as_str() {
            "executetrigger" => TriggerControlKind::Execute,
            "cleardelayedactions" => TriggerControlKind::ClearDelayed,
            _ => return Err(ActionReadError::UnsupportedAction(tag.into())),
        };
        let mut action = Self {
            owner,
            target: 0,
            delay: 0.0,
            kind,
        };
        for (key, value) in attrs {
            match key.to_ascii_lowercase().as_str() {
                "trigger" => {
                    if !value.is_ascii() {
                        return Err(ActionReadError::UnsupportedName(value.into()));
                    }
                    action.target = custom_hash(&value.to_ascii_lowercase());
                }
                "delay" => {
                    action.delay = value
                        .trim()
                        .parse()
                        .map_err(|_| ActionReadError::InvalidAttribute(key.into()))?
                }
                _ => {}
            }
        }
        Ok(action)
    }
    /// Both native QuickExecute overrides are empty.
    pub fn quick_execute(&self) {}
}
