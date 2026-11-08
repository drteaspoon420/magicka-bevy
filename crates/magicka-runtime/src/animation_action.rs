//! Typed Play/Stop/ResumeAnimation attributes. Trigger scheduling is separate.
use crate::{animated_hierarchy::PlayCommand, scene_loading::custom_hash};
#[derive(Clone, Copy, Debug)]
pub enum AnimationOperation {
    Play(PlayCommand),
    Stop,
    Resume {
        length: f32,
        speed: f32,
        looping: Option<bool>,
    },
}
#[derive(Clone, Debug)]
pub struct AnimationAction {
    pub path: Vec<i32>,
    pub children: bool,
    pub delay: f32,
    pub operation: AnimationOperation,
}
#[derive(Debug, PartialEq, Eq)]
pub enum ActionReadError {
    UnsupportedAction(String),
    MissingName,
    UnsupportedName(String),
    InvalidAttribute(String),
}
fn boolean(value: &str, attribute: &str) -> Result<bool, ActionReadError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ActionReadError::InvalidAttribute(attribute.into())),
    }
}
fn number(value: &str, attribute: &str) -> Result<f32, ActionReadError> {
    // Ordinary invariant-culture game numeric syntax. Thousands separators and
    // special CLR spellings remain explicit unsupported inputs.
    value
        .trim()
        .parse()
        .map_err(|_| ActionReadError::InvalidAttribute(attribute.into()))
}
impl AnimationAction {
    /// Attributes in XML order. Unknown attributes are ignored like Action.Read.
    /// ASCII string lowercasing models the original invariant loader restriction.
    pub fn read<'a>(
        tag: &str,
        attributes: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ActionReadError> {
        let operation = match tag.to_ascii_lowercase().as_str() {
            "playanimation" => AnimationOperation::Play(PlayCommand {
                start: -1.0,
                end: -1.0,
                speed: 1.0,
                looping: false,
                resume: false,
            }),
            "stopanimation" => AnimationOperation::Stop,
            "resumeanimation" => AnimationOperation::Resume {
                length: 0.0,
                speed: 1.0,
                looping: None,
            },
            _ => return Err(ActionReadError::UnsupportedAction(tag.into())),
        };
        let mut action = Self {
            path: vec![],
            children: true,
            delay: 0.0,
            operation,
        };
        for (attribute, value) in attributes {
            let key = attribute.to_ascii_lowercase();
            match key.as_str() {
                "name" => {
                    if !value.is_ascii() {
                        return Err(ActionReadError::UnsupportedName(value.into()));
                    }
                    action.path = value
                        .to_ascii_lowercase()
                        .split('/')
                        .map(custom_hash)
                        .collect();
                }
                "children" => action.children = boolean(value, attribute)?,
                "delay" => action.delay = number(value, attribute)?,
                _ => match &mut action.operation {
                    AnimationOperation::Play(command) => match key.as_str() {
                        "start" => command.start = number(value, attribute)?,
                        "end" => command.end = number(value, attribute)?,
                        "speed" => command.speed = number(value, attribute)?,
                        "loop" => command.looping = boolean(value, attribute)?,
                        "resume" => command.resume = boolean(value, attribute)?,
                        _ => {}
                    },
                    AnimationOperation::Resume {
                        length,
                        speed,
                        looping,
                    } => match key.as_str() {
                        "length" => *length = number(value, attribute)?,
                        "speed" => *speed = number(value, attribute)?,
                        "loop" => *looping = Some(boolean(value, attribute)?),
                        _ => {}
                    },
                    AnimationOperation::Stop => {}
                },
            }
        }
        if action.path.is_empty() {
            return Err(ActionReadError::MissingName);
        }
        Ok(action)
    }
    /// Original QuickExecute overrides are empty for all three action types.
    pub fn quick_execute(&self) {}
}

/// Animation action plus the original base delay queue. Trigger conditions and
/// GameScene's action-history storage remain world responsibilities.
pub struct ScheduledAnimationAction {
    pub action: AnimationAction,
    pub queue: crate::action_queue::ActionQueue,
}
impl ScheduledAnimationAction {
    pub fn new(action: AnimationAction) -> Self {
        let queue = crate::action_queue::ActionQueue::new(action.delay);
        Self { action, queue }
    }
    pub fn trigger(&mut self) {
        self.queue.trigger();
    }
    pub fn update(
        &mut self,
        dt: f32,
        scene: &mut crate::live_collision::LiveCollisionScene,
        mut executed: impl FnMut(&AnimationAction),
    ) -> Result<usize, crate::live_collision::LiveCollisionError> {
        self.queue.update(
            dt,
            |_| scene.execute_animation_action(&self.action),
            |_| {
                executed(&self.action);
                Ok(())
            },
        )
    }
}
