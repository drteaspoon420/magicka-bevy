//! Interact sequencing/lock lifecycle, composed with recovered MessagePlayback.
use crate::{
    dialog_definition::{DialogDefinition, InteractionDefinition, MessageDefinition},
    dialog_state::{DialogCursor, DialogError, DialogPop, InteractionRepetition},
    message_playback::{MessagePlacement, MessagePlayback},
};
#[derive(Clone, Copy, Debug)]
pub enum InteractionLocation {
    Screen([f32; 2]),
    World([f32; 3]),
    Entity(Option<i32>),
}

/// Owns decoded definitions and their live interaction/message state together.
pub struct DialogPlayback {
    definition: DialogDefinition,
    cursor: DialogCursor,
    interactions: Vec<InteractionPlayback>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum DialogPlaybackError<E> {
    Cursor(DialogError),
    Interaction(InteractionError<E>),
    MissingInteraction(usize),
}
impl DialogPlayback {
    pub fn new(definition: DialogDefinition) -> Self {
        let cursor = definition.cursor();
        let interactions = definition
            .interactions
            .iter()
            .map(InteractionPlayback::new)
            .collect();
        Self {
            definition,
            cursor,
            interactions,
        }
    }
    pub fn definition(&self) -> &DialogDefinition {
        &self.definition
    }
    /// Snapshot includes live repetition counts, not the last Pop's cached copy.
    pub fn cursor(&self) -> DialogCursor {
        let mut cursor = self.cursor.clone();
        for (index, interaction) in self.interactions.iter().enumerate() {
            cursor.interactions[index] =
                interaction.repetition(&self.definition.interactions[index]);
        }
        cursor
    }
    pub(crate) fn disk_boundary(&self) -> (i32, i32, Vec<(i32, i32)>) {
        (
            self.cursor.next,
            self.cursor.interacted,
            self.interactions
                .iter()
                .map(|i| (i.current, i.completed))
                .collect(),
        )
    }
    pub(crate) fn restore_disk_boundary(
        &mut self,
        next: i32,
        interacted: i32,
        states: &[(i32, i32)],
    ) -> Result<(), String> {
        let count = self.interactions.len() as i32;
        if !(0..=count).contains(&next)
            || !(0..=count).contains(&interacted)
            || states.len() != self.interactions.len()
        {
            return Err("invalid saved dialog cursor".into());
        }
        for ((current, completed), definition) in states.iter().zip(&self.definition.interactions) {
            if *current < 0 || *current as usize >= definition.messages.len() || *completed < 0 {
                return Err("invalid saved interaction".into());
            }
        }
        self.cursor.next = next;
        self.cursor.interacted = interacted;
        for (interaction, (current, completed)) in self.interactions.iter_mut().zip(states) {
            interaction.current = *current;
            interaction.completed = *completed;
        }
        Ok(())
    }
    pub fn interaction(&self, index: usize) -> Option<&InteractionPlayback> {
        self.interactions.get(index)
    }
    pub fn pop(&mut self) -> Result<Option<DialogPop>, DialogError> {
        for (index, interaction) in self.interactions.iter().enumerate() {
            self.cursor.interactions[index] =
                interaction.repetition(&self.definition.interactions[index]);
        }
        let selected = self.cursor.pop()?;
        if selected.is_some_and(|selected| selected.reset_all) {
            for interaction in &mut self.interactions {
                interaction.reset();
            }
        }
        Ok(selected)
    }
    pub fn begin<H: InteractionHost>(
        &mut self,
        index: usize,
        location: InteractionLocation,
        controller: Option<i32>,
        host: &mut H,
    ) -> Result<bool, DialogPlaybackError<H::Error>> {
        let playback = self
            .interactions
            .get_mut(index)
            .ok_or(DialogPlaybackError::MissingInteraction(index))?;
        playback
            .begin(
                &self.definition.interactions[index],
                location,
                controller,
                host,
            )
            .map_err(DialogPlaybackError::Interaction)
    }
    pub fn advance<H: InteractionHost>(
        &mut self,
        index: usize,
        controller: Option<i32>,
        host: &mut H,
    ) -> Result<bool, DialogPlaybackError<H::Error>> {
        let playback = self
            .interactions
            .get_mut(index)
            .ok_or(DialogPlaybackError::MissingInteraction(index))?;
        playback
            .advance(&self.definition.interactions[index], controller, host)
            .map_err(DialogPlaybackError::Interaction)
    }
    pub fn end<H: InteractionHost>(
        &self,
        index: usize,
        host: &mut H,
    ) -> Result<(), DialogPlaybackError<H::Error>> {
        let playback = self
            .interactions
            .get(index)
            .ok_or(DialogPlaybackError::MissingInteraction(index))?;
        playback
            .end(&self.definition.interactions[index], host)
            .map_err(DialogPlaybackError::Interaction)
    }
    /// Dialog.Reset rewinds its cursor without resetting its interactions.
    pub fn reset(&mut self) {
        self.cursor.reset();
    }
}
pub trait InteractionHost {
    type Error;
    /// Bind a per-message MessageHost and call the supplied MessagePlayback.
    /// Cue ownership must remain separate for each message index.
    fn advance_message(
        &mut self,
        index: usize,
        definition: &MessageDefinition,
        playback: &mut MessagePlayback,
        placement: MessagePlacement,
    ) -> Result<bool, Self::Error>;
    fn lock_input(&mut self, controller: i32) -> Result<(), Self::Error>;
    fn unlock_input(&mut self, controller: i32) -> Result<(), Self::Error>;
    /// Message.StopCue: immediate stop of an existing cue, even if stopping.
    fn end_message(
        &mut self,
        index: usize,
        definition: &MessageDefinition,
    ) -> Result<(), Self::Error>;
}
#[derive(Debug, PartialEq, Eq)]
pub enum InteractionError<E> {
    Host(E),
    MissingMessage(i32),
    MessageCountMismatch,
}
#[derive(Clone, Debug)]
pub struct InteractionPlayback {
    pub current: i32,
    pub completed: i32,
    pub controller: Option<i32>,
    pub last_owner: Option<i32>,
    pub world_position: [f32; 3],
    pub screen_position: bool,
    pub messages: Vec<MessagePlayback>,
}
impl InteractionPlayback {
    pub fn new(definition: &InteractionDefinition) -> Self {
        Self {
            current: 0,
            completed: 0,
            controller: None,
            last_owner: None,
            world_position: [0.0; 3],
            screen_position: false,
            messages: vec![MessagePlayback::default(); definition.messages.len()],
        }
    }
    pub fn repetition(&self, definition: &InteractionDefinition) -> InteractionRepetition {
        InteractionRepetition {
            repeat: definition.repeat,
            completed: self.completed,
        }
    }
    /// These field assignments precede the finite-repeat early-return check.
    pub fn begin<H: InteractionHost>(
        &mut self,
        definition: &InteractionDefinition,
        location: InteractionLocation,
        controller: Option<i32>,
        host: &mut H,
    ) -> Result<bool, InteractionError<H::Error>> {
        match location {
            InteractionLocation::Screen(position) => {
                self.world_position = [position[0], position[1], 0.0];
                self.last_owner = None;
                self.screen_position = true;
            }
            InteractionLocation::World(position) => {
                self.world_position = position;
                self.last_owner = None;
                self.screen_position = false;
            }
            InteractionLocation::Entity(owner) => {
                self.last_owner = owner;
                self.screen_position = false;
            }
        }
        self.advance(definition, controller, host)
    }
    fn placement(&self, definition: &InteractionDefinition) -> MessagePlacement {
        if self.last_owner.is_some() {
            MessagePlacement::Entity {
                default_owner: self.last_owner,
                focus: definition.focus_owner,
            }
        } else if self.screen_position {
            MessagePlacement::Screen([self.world_position[0], self.world_position[1]])
        } else {
            MessagePlacement::World {
                position: self.world_position,
                focus: definition.focus_owner,
            }
        }
    }
    fn advance_current<H: InteractionHost>(
        &mut self,
        definition: &InteractionDefinition,
        host: &mut H,
    ) -> Result<bool, InteractionError<H::Error>> {
        let index = usize::try_from(self.current)
            .map_err(|_| InteractionError::MissingMessage(self.current))?;
        let message = definition
            .messages
            .get(index)
            .ok_or(InteractionError::MissingMessage(self.current))?;
        let placement = self.placement(definition);
        let playback = self
            .messages
            .get_mut(index)
            .ok_or(InteractionError::MessageCountMismatch)?;
        host.advance_message(index, message, playback, placement)
            .map_err(InteractionError::Host)
    }
    pub fn advance<H: InteractionHost>(
        &mut self,
        definition: &InteractionDefinition,
        controller: Option<i32>,
        host: &mut H,
    ) -> Result<bool, InteractionError<H::Error>> {
        if self.completed >= definition.repeat && definition.repeat > 0 {
            return Ok(true);
        }
        if let (None, Some(controller)) = (self.controller, controller) {
            self.controller = Some(controller);
            host.lock_input(controller)
                .map_err(InteractionError::Host)?;
        }
        if self.advance_current(definition, host)? {
            self.current = self.current.wrapping_add(1);
            if self.current >= definition.messages.len() as i32 {
                self.completed = self.completed.wrapping_add(1);
                self.current = 0;
                if let Some(controller) = self.controller {
                    host.unlock_input(controller)
                        .map_err(InteractionError::Host)?;
                    self.controller = None;
                }
                return Ok(true);
            }
            // Native code starts exactly one next message and ignores its result.
            self.advance_current(definition, host)?;
        }
        Ok(false)
    }
    pub fn end<H: InteractionHost>(
        &self,
        definition: &InteractionDefinition,
        host: &mut H,
    ) -> Result<(), InteractionError<H::Error>> {
        if self.current >= 0 && self.current < definition.messages.len() as i32 {
            let index = self.current as usize;
            host.end_message(index, &definition.messages[index])
                .map_err(InteractionError::Host)?;
        }
        if let Some(controller) = self.controller {
            host.unlock_input(controller)
                .map_err(InteractionError::Host)?;
        }
        Ok(())
    }
    /// Does not clear controller/placement or stop audio, matching Interact.Reset.
    pub fn reset(&mut self) {
        self.current = 0;
        self.completed = 0;
        for message in &mut self.messages {
            message.reset();
        }
    }
}
