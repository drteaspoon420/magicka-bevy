//! Message.Advance overload ordering; world/UI/audio operations are synchronous.
use crate::{dialog_definition::MessageDefinition, scene_loading::custom_hash};
#[derive(Clone, Copy, Debug)]
pub enum MessagePlacement {
    Screen([f32; 2]),
    World {
        position: [f32; 3],
        focus: bool,
    },
    Entity {
        default_owner: Option<i32>,
        focus: bool,
    },
}
#[derive(Clone, Copy, Debug)]
pub struct MessageOwner {
    pub id: i32,
    pub dead: bool,
    pub character: bool,
    pub dead_state: bool,
    /// Entity overload: NPC has no remaining AI event, or Avatar.Events is null.
    pub animation_allowed: bool,
    pub special_idle: u16,
}
#[derive(Clone, Copy, Debug)]
pub enum MessageEffect {
    StopCue,
    PlayCue {
        bank: u16,
        sound: i32,
        emitter: Option<i32>,
    },
    Animate {
        owner: i32,
        animation: u16,
        blend: f32,
    },
    MoveCamera {
        owner: i32,
        time: f32,
    },
    Magnification(f32),
    ReleaseCamera(f32),
    Show {
        text: usize,
        placement: MessagePlacement,
        owner: Option<i32>,
    },
    Hide,
}
pub trait MessageHost {
    type Error;
    fn resolve_owner(&mut self, id: i32) -> Result<Option<MessageOwner>, Self::Error>;
    /// None means no cue; true means the previous cue is already stopping.
    fn cue_stopping(&self) -> Option<bool>;
    fn effect(
        &mut self,
        definition: &MessageDefinition,
        effect: MessageEffect,
    ) -> Result<(), Self::Error>;
}
#[derive(Debug, PartialEq, Eq)]
pub enum MessagePlaybackError<E> {
    Host(E),
    MissingText(usize),
    InvalidIndex(i32),
}
#[derive(Default, Clone, Copy, Debug)]
pub struct MessagePlayback {
    pub current: i32,
}
impl MessagePlayback {
    pub fn reset(&mut self) {
        self.current = 0;
    }
    pub fn advance<H: MessageHost>(
        &mut self,
        definition: &MessageDefinition,
        placement: MessagePlacement,
        host: &mut H,
    ) -> Result<bool, MessagePlaybackError<H::Error>> {
        let mut owner;
        loop {
            owner = match placement {
                MessagePlacement::Screen(_) => None,
                MessagePlacement::World { .. } => host
                    .resolve_owner(definition.owner)
                    .map_err(MessagePlaybackError::Host)?
                    .filter(|o| o.character),
                MessagePlacement::Entity { default_owner, .. } => {
                    let id = if definition.owner == custom_hash("owner") {
                        default_owner
                    } else {
                        Some(definition.owner)
                    };
                    match id {
                        Some(id) => host.resolve_owner(id).map_err(MessagePlaybackError::Host)?,
                        None => None,
                    }
                }
            };
            if matches!(placement, MessagePlacement::Entity { .. }) && owner.is_none_or(|o| o.dead)
            {
                self.current = self.current.wrapping_add(1);
                if self.current >= definition.texts.len() as i32 {
                    return Ok(true);
                }
                continue;
            }
            break;
        }
        let emit = |host: &mut H, effect| {
            host.effect(definition, effect)
                .map_err(MessagePlaybackError::Host)
        };
        let screen = matches!(placement, MessagePlacement::Screen(_));
        if !screen && host.cue_stopping() == Some(false) {
            emit(host, MessageEffect::StopCue)?;
        }
        if self.current >= definition.texts.len() as i32 {
            self.current = 0;
            emit(host, MessageEffect::Hide)?;
            if let Some(owner) = owner {
                let focus = match placement {
                    MessagePlacement::World { focus, .. }
                    | MessagePlacement::Entity { focus, .. } => focus,
                    _ => false,
                };
                if focus {
                    emit(host, MessageEffect::ReleaseCamera(2.0))?;
                }
                if owner.character
                    && !(matches!(placement, MessagePlacement::Entity { .. }) && owner.dead_state)
                {
                    let index = definition.texts.len().saturating_sub(1);
                    let last = definition
                        .texts
                        .get(index)
                        .ok_or(MessagePlaybackError::MissingText(index))?;
                    if last.animation != 0 {
                        emit(
                            host,
                            MessageEffect::Animate {
                                owner: owner.id,
                                animation: if owner.special_idle != 0 {
                                    owner.special_idle
                                } else {
                                    1
                                },
                                blend: 0.3,
                            },
                        )?;
                    }
                }
            }
            return Ok(true);
        }
        let index = usize::try_from(self.current)
            .map_err(|_| MessagePlaybackError::InvalidIndex(self.current))?;
        let text = definition
            .texts
            .get(index)
            .ok_or(MessagePlaybackError::MissingText(index))?;
        if screen && host.cue_stopping() == Some(false) {
            emit(host, MessageEffect::StopCue)?;
        }
        if let Some(owner) = owner {
            let allowed = matches!(placement, MessagePlacement::World { .. })
                || owner.character && owner.animation_allowed;
            if text.animation != 0 && allowed {
                emit(
                    host,
                    MessageEffect::Animate {
                        owner: owner.id,
                        animation: text.animation,
                        blend: 0.3,
                    },
                )?;
            }
        }
        // World fallback plays even sound=0. Other paths skip a zero sound ID.
        if text.sound != 0 || matches!(placement, MessagePlacement::World { .. }) && owner.is_none()
        {
            emit(
                host,
                MessageEffect::PlayCue {
                    bank: text.sound_bank,
                    sound: text.sound,
                    emitter: owner.map(|o| o.id),
                },
            )?;
        }
        if let Some(owner) = owner {
            let focus = match placement {
                MessagePlacement::World { focus, .. } | MessagePlacement::Entity { focus, .. } => {
                    focus
                }
                _ => false,
            };
            if focus {
                emit(
                    host,
                    MessageEffect::MoveCamera {
                        owner: owner.id,
                        time: 2.0,
                    },
                )?;
                emit(host, MessageEffect::Magnification(1.5))?;
            }
        }
        emit(
            host,
            MessageEffect::Show {
                text: index,
                placement,
                owner: owner.map(|o| o.id),
            },
        )?;
        self.current = self.current.wrapping_add(1);
        Ok(false)
    }
}
