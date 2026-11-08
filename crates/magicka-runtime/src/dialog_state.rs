//! Dialog cursor/finished state and predicates; message/UI advancement is separate.
use crate::script_attributes::{boolean, identifier, number, ScriptReadError};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug)]
pub struct InteractionRepetition {
    pub repeat: i32,
    pub completed: i32,
}
impl Default for InteractionRepetition {
    fn default() -> Self {
        Self {
            repeat: 1,
            completed: 0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct DialogCursor {
    pub next: i32,
    pub interacted: i32,
    pub repeat_from: i32,
    pub interactions: Vec<InteractionRepetition>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogPop {
    pub interaction: usize,
    /// Host must also reset every interaction's message state when true.
    pub reset_all: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogError {
    MissingFinishedFlag(i32),
    MissingDialog(i32),
    InvalidCursor(i32),
    MissingWorldFacts,
}
impl DialogCursor {
    pub fn new(interactions: Vec<InteractionRepetition>, repeat_from: i32) -> Self {
        Self {
            next: 0,
            interacted: 0,
            repeat_from,
            interactions,
        }
    }
    pub fn pop(&mut self) -> Result<Option<DialogPop>, DialogError> {
        let len = self.interactions.len() as i32;
        if self.next >= len {
            return Ok(None);
        }
        let index =
            usize::try_from(self.next).map_err(|_| DialogError::InvalidCursor(self.next))?;
        let interaction = self.interactions[index];
        if interaction.repeat > 0 && interaction.repeat.wrapping_sub(interaction.completed) <= 1 {
            self.next = self.next.wrapping_add(1);
            self.interacted = self.interacted.wrapping_add(1).min(len);
        }
        let reset_all = self.next >= len && self.repeat_from >= 0;
        if reset_all {
            self.next = self.repeat_from;
            for i in &mut self.interactions {
                i.completed = 0;
            }
        }
        Ok(Some(DialogPop {
            interaction: index,
            reset_all,
        }))
    }
    /// Native Dialog.Reset only resets Next, preserving Interacted and repetitions.
    pub fn reset(&mut self) {
        self.next = 0;
    }
}
#[derive(Default)]
pub struct DialogLedger {
    pub dialogs: BTreeMap<i32, DialogCursor>,
    pub finished: BTreeMap<i32, bool>,
}
impl DialogLedger {
    pub fn insert(&mut self, id: i32, cursor: DialogCursor) {
        self.dialogs.insert(id, cursor);
        self.finished.insert(id, false);
    }
    pub fn mark_finished(&mut self, id: i32) {
        self.finished.insert(id, true);
    }
    pub fn is_done(&self, id: i32, interact_index: i32) -> Result<bool, DialogError> {
        let finished = *self
            .finished
            .get(&id)
            .ok_or(DialogError::MissingFinishedFlag(id))?;
        if !finished {
            return Ok(false);
        }
        Ok(self
            .dialogs
            .get(&id)
            .ok_or(DialogError::MissingDialog(id))?
            .interacted
            > interact_index)
    }
    pub fn reset(&mut self) {
        for (&id, cursor) in &mut self.dialogs {
            cursor.reset();
            self.finished.insert(id, false);
        }
    }
}
#[derive(Clone, Copy)]
pub struct DialogFacts<'a> {
    pub ledger: &'a DialogLedger,
    pub active_dialogs: &'a [Option<i32>; 8],
    pub message_box_count: usize,
}
#[derive(Clone, Copy, Debug)]
pub enum DialogConditionKind {
    Done { id: i32, interact_index: i32 },
    Active { id: Option<i32> },
}
#[derive(Clone, Copy, Debug)]
pub struct DialogCondition {
    pub invert: bool,
    pub kind: DialogConditionKind,
}
impl DialogCondition {
    pub fn read<'a>(
        tag: &str,
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        let kind = match tag.to_ascii_lowercase().as_str() {
            "dialogdone" => DialogConditionKind::Done {
                id: 0,
                interact_index: -1,
            },
            "dialogactive" => DialogConditionKind::Active { id: None },
            _ => return Err(ScriptReadError::UnsupportedTag(tag.into())),
        };
        let mut condition = Self {
            invert: false,
            kind,
        };
        for (key, value) in attrs {
            match key.to_ascii_lowercase().as_str() {
                "invert" => condition.invert = boolean(value, key)?,
                "dialog" => match &mut condition.kind {
                    DialogConditionKind::Done { id, .. } => *id = identifier(value)?,
                    DialogConditionKind::Active { id } => {
                        // CLR loader lowercases but does not trim string values.
                        let hash = identifier(value)?;
                        *id = if value.is_empty() || value.eq_ignore_ascii_case("any") {
                            None
                        } else {
                            Some(hash)
                        };
                    }
                },
                "interactindex" => {
                    if let DialogConditionKind::Done { interact_index, .. } = &mut condition.kind {
                        *interact_index = number(value, key)?;
                    }
                }
                _ => {}
            }
        }
        Ok(condition)
    }
    pub fn is_met(self, facts: DialogFacts<'_>) -> Result<bool, DialogError> {
        let met = match self.kind {
            DialogConditionKind::Done { id, interact_index } => {
                facts.ledger.is_done(id, interact_index)?
            }
            DialogConditionKind::Active { id: None } => facts.message_box_count > 0,
            DialogConditionKind::Active { id: Some(id) } => {
                facts.active_dialogs.contains(&Some(id))
            }
        };
        Ok(self.invert ^ met)
    }
}
