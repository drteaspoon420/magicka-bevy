//! Live-scene facts for DialogTextHost: owner positions/capsules/death from
//! the SceneWorld, cutscene state, and queued world effects (camera,
//! animation, input/freeze) that the scene applies after each dialog call.
//! See research/play-session.md.
use crate::{
    dialog_definition::MessageDefinition,
    dialog_frame::DialogFrame,
    dialog_manager::DialogManagerEffect,
    dialog_text_host::{DialogMessageKey, DialogTextFrameWorld, DialogTextWorld},
    message_playback::{MessageEffect, MessageHost, MessageOwner},
    message_textbox::MessageTextBoxWorld,
    text_box::TextBoxOwner,
    text_box_initialize::TextBoxInitHost,
};
use magicka_sim::impact_motion::Vector;
use std::collections::BTreeMap;

/// Scene ID reported for entities of the live scene.
pub const LIVE_SCENE: i32 = 1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialogOwnerFacts {
    pub position: Vector,
    /// (Capsule.Length, Capsule.Radius) for characters.
    pub capsule: Option<(f32, f32)>,
    pub dead: bool,
    pub character: bool,
    /// NPC has no remaining AI event / Avatar.Events is null.
    pub animation_allowed: bool,
    pub special_idle: u16,
}

#[derive(Clone, Debug)]
pub enum DialogWorldCommand {
    PlayCue {
        message: DialogMessageKey,
        bank: u16,
        sound: i32,
        emitter: Option<i32>,
    },
    StopCue {
        message: DialogMessageKey,
        instant: bool,
    },
    /// Camera/animation message effects (Show/Hide stay in the text host).
    Message(MessageEffect),
    Manager(DialogManagerEffect),
    LockController(i32),
    UnlockController(i32),
}

#[derive(Clone, Debug, Default)]
pub struct SceneDialogWorld {
    pub owners: BTreeMap<i32, DialogOwnerFacts>,
    pub cutscene: bool,
    pub commands: Vec<DialogWorldCommand>,
    /// Presentation request keys use the scene's shared audio allocator.
    pub(crate) cue_keys: BTreeMap<DialogMessageKey, u32>,
    bound_message: Option<DialogMessageKey>,
    cue_stopping: BTreeMap<DialogMessageKey, bool>,
}

impl SceneDialogWorld {
    fn stop_bound_cue(&mut self, instant: bool) {
        if let Some(message) = self.bound_message {
            if let Some(stopping) = self.cue_stopping.get_mut(&message) {
                *stopping = true;
                self.commands
                    .push(DialogWorldCommand::StopCue { message, instant });
            }
        }
    }
}

impl MessageHost for SceneDialogWorld {
    type Error = String;
    fn resolve_owner(&mut self, id: i32) -> Result<Option<MessageOwner>, String> {
        Ok(self.owners.get(&id).map(|o| MessageOwner {
            id,
            dead: o.dead,
            character: o.character,
            dead_state: o.dead,
            animation_allowed: o.animation_allowed,
            special_idle: o.special_idle,
        }))
    }
    fn cue_stopping(&self) -> Option<bool> {
        self.bound_message
            .and_then(|key| self.cue_stopping.get(&key).copied())
    }
    fn effect(&mut self, _: &MessageDefinition, effect: MessageEffect) -> Result<(), String> {
        match effect {
            MessageEffect::PlayCue {
                bank,
                sound,
                emitter,
            } => {
                let message = self
                    .bound_message
                    .ok_or("dialog cue without message context")?;
                self.cue_stopping.insert(message, false);
                self.commands.push(DialogWorldCommand::PlayCue {
                    message,
                    bank,
                    sound,
                    emitter,
                });
            }
            MessageEffect::StopCue => self.stop_bound_cue(false),
            _ => self.commands.push(DialogWorldCommand::Message(effect)),
        }
        Ok(())
    }
}
impl TextBoxInitHost for SceneDialogWorld {
    type Error = String;
    fn load_background(&mut self, _: &str) -> Result<(), String> {
        // The presentation owns the background texture (UI/HUD/Dialog_Say).
        Ok(())
    }
}
impl MessageTextBoxWorld for SceneDialogWorld {
    fn entity_scene(&mut self, owner: i32) -> Result<Option<i32>, String> {
        Ok(self.owners.contains_key(&owner).then_some(LIVE_SCENE))
    }
}
impl DialogTextWorld for SceneDialogWorld {
    fn bind_message(&mut self, key: DialogMessageKey) -> Result<(), String> {
        self.bound_message = Some(key);
        Ok(())
    }
    fn lock_controller(&mut self, controller: i32) -> Result<(), String> {
        self.commands
            .push(DialogWorldCommand::LockController(controller));
        Ok(())
    }
    fn unlock_controller(&mut self, controller: i32) -> Result<(), String> {
        self.commands
            .push(DialogWorldCommand::UnlockController(controller));
        Ok(())
    }
    fn stop_message_cue(&mut self) -> Result<(), String> {
        self.stop_bound_cue(true);
        Ok(())
    }
    fn manager_effect(&mut self, effect: DialogManagerEffect) -> Result<(), String> {
        self.commands.push(DialogWorldCommand::Manager(effect));
        Ok(())
    }
}
impl DialogTextFrameWorld for SceneDialogWorld {
    fn text_owner(&self, owner: i32) -> Option<TextBoxOwner> {
        self.owners.get(&owner).map(|o| TextBoxOwner {
            position: o.position,
            capsule: o.capsule,
        })
    }
    fn owner_dead(&self, owner: i32) -> bool {
        self.owners.get(&owner).is_some_and(|o| o.dead)
    }
    fn in_cutscene(&self) -> bool {
        self.cutscene
    }
    fn message_box_count(&self) -> usize {
        0
    }
    fn update_message_box(&mut self, index: usize, _: DialogFrame) -> Result<(), String> {
        Err(format!("message box {index} is not hosted"))
    }
    fn message_box_dead(&self, _: usize) -> bool {
        false
    }
    fn remove_message_box(&mut self, index: usize) -> Result<(), String> {
        Err(format!("message box {index} is not hosted"))
    }
    fn submit_subtitles(&mut self, _: usize, _: f32) -> Result<(), String> {
        // Subtitle presentation is separate from per-message cue playback.
        Ok(())
    }
}
