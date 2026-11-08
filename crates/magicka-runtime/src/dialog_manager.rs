//! DialogManager ownership, advancement and native-ordered UI frame phases.
use crate::{
    dialog_definition::DialogDefinition,
    dialog_frame::{DialogFrame, DialogFrameHost, DialogFrameTarget},
    dialog_state::{DialogError, DialogFacts, DialogLedger},
    interaction_playback::{
        DialogPlayback, DialogPlaybackError, InteractionHost, InteractionLocation,
    },
};
use magicka_sim::damage_delivery::NetworkRole;
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveInteraction {
    pub dialog: i32,
    pub interaction: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogSink {
    Slot(usize),
    Cutscene,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DialogManagerEffect {
    Freeze,
    LimitInput,
    Unfreeze,
    UnlimitInput,
    NetworkAdvance {
        current: i32,
    },
    FinishAnimation {
        sink: DialogSink,
        remove_ttl_advance: bool,
    },
    Hide(DialogSink),
    HideAdditional,
}
pub trait DialogManagerHost: InteractionHost {
    /// Select the UI sink and parent/interaction context for per-message hosts.
    fn bind(&mut self, sink: DialogSink, active: ActiveInteraction);
    fn visible(&self, sink: DialogSink) -> bool;
    fn animating(&self, sink: DialogSink) -> bool;
    fn manager_effect(&mut self, effect: DialogManagerEffect) -> Result<(), Self::Error>;
}
pub trait DialogAvatarHost: DialogManagerHost {
    /// Native Avatar.Player.Controller lookup, repeated at each active slot.
    fn avatar_controller(&mut self, avatar: i32) -> Result<Option<i32>, Self::Error>;
}
#[derive(Debug, PartialEq, Eq)]
pub enum DialogManagerLoadError {
    DuplicateDialog(i32),
}
#[derive(Debug, PartialEq, Eq)]
pub enum DialogManagerError<E> {
    MissingDialog(i32),
    InvalidSlot(usize),
    Cursor(DialogError),
    Playback(DialogPlaybackError<E>),
    Host(E),
}
/// Owned equivalent of a native DialogCollection reference. Tokens distinguish
/// caller-supplied collection identities even when their dialog IDs overlap.
pub struct DialogCollectionState {
    pub token: u64,
    dialogs: BTreeMap<i32, DialogPlayback>,
    ledger: DialogLedger,
}
impl DialogCollectionState {
    pub fn new(
        token: u64,
        definitions: Vec<DialogDefinition>,
    ) -> Result<Self, DialogManagerLoadError> {
        let mut collection = Self {
            token,
            dialogs: BTreeMap::new(),
            ledger: DialogLedger::default(),
        };
        for definition in definitions {
            let id = definition.id;
            if collection.dialogs.contains_key(&id) {
                return Err(DialogManagerLoadError::DuplicateDialog(id));
            }
            let playback = DialogPlayback::new(definition);
            collection.ledger.insert(id, playback.cursor());
            collection.dialogs.insert(id, playback);
        }
        Ok(collection)
    }
}
pub struct DialogManager {
    dialogs: BTreeMap<i32, DialogPlayback>,
    ledger: DialogLedger,
    collection_token: u64,
    active: [Option<ActiveInteraction>; 8],
    active_ids: [Option<i32>; 8],
    pub holdoff: f32,
    pub awaiting_input: bool,
    pub draw_cutscene_text: bool,
    pub cutscene_slot: usize,
    pub draw_subtitles: bool,
    pub subtitle_height: f32,
}
impl DialogManager {
    /// Finished flags are script facts even after their UI interaction ends.
    pub fn disk_boundary(&self) -> Result<serde_json::Value, String> {
        if self.active.iter().any(Option::is_some) {
            return Err("cannot save active dialog playback".into());
        }
        Ok(serde_json::json!(self
            .dialogs
            .iter()
            .map(|(&id, d)| {
                let (next, interacted, interactions) = d.disk_boundary();
                (
                    id,
                    next,
                    interacted,
                    self.ledger.finished.get(&id).copied().unwrap_or(false),
                    interactions,
                )
            })
            .collect::<Vec<_>>()))
    }
    /// DialogCollection.State.UpdateState: finished flags and every dialog's
    /// cursor and interaction state, captured even during active playback
    /// (the checkpoint restart resets the manager before applying it). Same
    /// rows as [`DialogManager::disk_boundary`].
    pub fn checkpoint_boundary(&self) -> serde_json::Value {
        serde_json::json!(self
            .dialogs
            .iter()
            .map(|(&id, d)| {
                let (next, interacted, interactions) = d.disk_boundary();
                (
                    id,
                    next,
                    interacted,
                    self.ledger.finished.get(&id).copied().unwrap_or(false),
                    interactions,
                )
            })
            .collect::<Vec<_>>())
    }
    pub fn restore_disk_boundary(&mut self, v: &serde_json::Value) -> Result<(), String> {
        let rows: Vec<(i32, i32, i32, bool, Vec<(i32, i32)>)> =
            serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        if rows.len() != self.dialogs.len() {
            return Err("saved dialog collection differs".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for (id, next, interacted, finished, states) in rows {
            if !seen.insert(id) {
                return Err("duplicate saved dialog".into());
            }
            let d = self.dialogs.get_mut(&id).ok_or("missing saved dialog")?;
            d.restore_disk_boundary(next, interacted, &states)?;
            self.ledger.dialogs.insert(id, d.cursor());
            self.ledger.finished.insert(id, finished);
        }
        Ok(())
    }
    pub fn new(definitions: Vec<DialogDefinition>) -> Result<Self, DialogManagerLoadError> {
        Ok(Self::from_collection(DialogCollectionState::new(
            0,
            definitions,
        )?))
    }
    pub fn from_collection(collection: DialogCollectionState) -> Self {
        Self {
            dialogs: collection.dialogs,
            ledger: collection.ledger,
            collection_token: collection.token,
            active: [None; 8],
            active_ids: [None; 8],
            holdoff: 0.0,
            awaiting_input: false,
            draw_cutscene_text: false,
            cutscene_slot: 0,
            draw_subtitles: false,
            subtitle_height: 0.0,
        }
    }
    pub fn collection_token(&self) -> u64 {
        self.collection_token
    }
    /// Native SetDialogs assigns the collection then clears the active slots.
    /// No End/Hide/reset/unlock callbacks occur. Return the old owned collection
    /// so a scene can retain and restore its existing progress/cue identities.
    pub fn set_dialogs(&mut self, collection: DialogCollectionState) -> DialogCollectionState {
        let old = DialogCollectionState {
            token: std::mem::replace(&mut self.collection_token, collection.token),
            dialogs: std::mem::replace(&mut self.dialogs, collection.dialogs),
            ledger: std::mem::replace(&mut self.ledger, collection.ledger),
        };
        for slot in 0..8 {
            self.set_active(slot, None);
        }
        old
    }
    pub fn active(&self) -> &[Option<ActiveInteraction>; 8] {
        &self.active
    }
    pub fn dialog(&self, id: i32) -> Option<&DialogPlayback> {
        self.dialogs.get(&id)
    }
    pub fn facts(&self, message_box_count: usize) -> DialogFacts<'_> {
        DialogFacts {
            ledger: &self.ledger,
            active_dialogs: &self.active_ids,
            message_box_count,
        }
    }
    pub fn dialog_active(&self, id: i32) -> bool {
        self.active_ids.contains(&Some(id))
    }
    /// CanAdvance does not apply controller cooldown or cutscene restrictions.
    pub fn can_advance(&self, controller: Option<i32>) -> bool {
        self.active.iter().flatten().any(|active| {
            self.dialogs[&active.dialog]
                .interaction(active.interaction)
                .unwrap()
                .controller
                == controller
        })
    }
    pub fn is_done(&self, id: i32, index: i32) -> Result<bool, DialogError> {
        self.ledger.is_done(id, index)
    }
    pub fn tick_input(&mut self, dt: f32) {
        self.holdoff -= dt;
    }
    /// DialogManager.Update. Counts and box state are reread after each callback;
    /// TTL callbacks may synchronously advance/replace active interactions.
    pub fn update<H: DialogFrameHost>(
        &mut self,
        frame: DialogFrame,
        view_projection: &mut [[f32; 4]; 4],
        host: &mut H,
    ) -> Result<(), DialogManagerError<H::Error>> {
        self.tick_input(frame.dt);
        for slot in 0..8 {
            host.update_textbox(self, DialogFrameTarget::Slot(slot), frame, view_projection)
                .map_err(DialogManagerError::Host)?;
            let sink = DialogSink::Slot(slot);
            if host.visible(sink)
                && host
                    .slot_owner_dead(slot)
                    .map_err(DialogManagerError::Host)?
            {
                if let Some(active) = self.active[slot] {
                    host.bind(sink, active);
                    self.dialogs[&active.dialog]
                        .end(active.interaction, host)
                        .map_err(DialogManagerError::Playback)?;
                    self.set_active(slot, None);
                }
                host.manager_effect(DialogManagerEffect::Hide(sink))
                    .map_err(DialogManagerError::Host)?;
            }
        }
        let mut index = 0;
        while index < host.additional_count() {
            host.update_textbox(
                self,
                DialogFrameTarget::Additional(index),
                frame,
                view_projection,
            )
            .map_err(DialogManagerError::Host)?;
            if host.additional_visible(index) {
                index += 1;
            } else {
                host.remove_additional(index)
                    .map_err(DialogManagerError::Host)?;
            }
        }
        let mut index = 0;
        while index < host.message_box_count() {
            host.update_message_box(self, index, frame)
                .map_err(DialogManagerError::Host)?;
            if host.message_box_dead(index) {
                host.remove_message_box(index)
                    .map_err(DialogManagerError::Host)?;
            } else {
                index += 1;
            }
        }
        if host.play_state_in_cutscene() && self.draw_cutscene_text {
            host.update_textbox(self, DialogFrameTarget::Cutscene, frame, view_projection)
                .map_err(DialogManagerError::Host)?;
        }
        if self.draw_subtitles {
            host.submit_subtitles(frame.channel, self.subtitle_height)
                .map_err(DialogManagerError::Host)?;
        }
        Ok(())
    }
    pub fn awaiting(&self, message_boxes: usize) -> bool {
        self.awaiting_input | (message_boxes > 0) | (self.holdoff > 0.0)
    }
    fn set_active(&mut self, slot: usize, active: Option<ActiveInteraction>) {
        self.active[slot] = active;
        self.active_ids[slot] = active.map(|a| a.dialog);
    }
    fn sync(&mut self, id: i32) {
        self.ledger.dialogs.insert(id, self.dialogs[&id].cursor());
    }
    pub fn start<H: DialogManagerHost>(
        &mut self,
        id: i32,
        location: InteractionLocation,
        controller: Option<i32>,
        cutscene: bool,
        host: &mut H,
    ) -> Result<Option<usize>, DialogManagerError<H::Error>> {
        if self.dialog_active(id) {
            return Ok(None);
        }
        if !self.dialogs.contains_key(&id) {
            return if matches!(location, InteractionLocation::Entity(_)) {
                Ok(None)
            } else {
                Err(DialogManagerError::MissingDialog(id))
            };
        }
        let selected = self
            .dialogs
            .get_mut(&id)
            .unwrap()
            .pop()
            .map_err(DialogManagerError::Cursor)?;
        self.sync(id);
        let Some(selected) = selected else {
            return Ok(None);
        };
        let slot = (0..8)
            .find(|&i| self.active[i].is_none() && !host.visible(DialogSink::Slot(i)))
            .unwrap_or(0);
        let active = ActiveInteraction {
            dialog: id,
            interaction: selected.interaction,
        };
        self.set_active(slot, Some(active));
        let sink = if cutscene {
            self.draw_cutscene_text = true;
            DialogSink::Cutscene
        } else {
            DialogSink::Slot(slot)
        };
        host.bind(sink, active);
        // StartDialog ignores the initial Advance result.
        self.dialogs
            .get_mut(&id)
            .unwrap()
            .begin(active.interaction, location, controller, host)
            .map_err(DialogManagerError::Playback)?;
        self.sync(id);
        if cutscene {
            self.cutscene_slot = slot;
        } else {
            self.draw_cutscene_text = false;
        }
        self.awaiting_input = false;
        self.holdoff = 0.2;
        if self.dialogs[&id].definition().interactions[active.interaction].lock_game {
            host.manager_effect(DialogManagerEffect::Freeze)
                .map_err(DialogManagerError::Host)?;
            host.manager_effect(DialogManagerEffect::LimitInput)
                .map_err(DialogManagerError::Host)?;
        }
        Ok(Some(slot))
    }
    fn finish<H: DialogManagerHost>(
        &mut self,
        slot: usize,
        input: bool,
        host: &mut H,
    ) -> Result<(), DialogManagerError<H::Error>> {
        let active = self.active[slot].unwrap();
        self.sync(active.dialog);
        self.ledger.mark_finished(active.dialog);
        if input {
            self.awaiting_input = false;
            self.holdoff = 0.2;
        }
        if self.dialogs[&active.dialog].definition().interactions[active.interaction].lock_game {
            host.manager_effect(DialogManagerEffect::Unfreeze)
                .map_err(DialogManagerError::Host)?;
            host.manager_effect(DialogManagerEffect::UnlimitInput)
                .map_err(DialogManagerError::Host)?;
        }
        self.set_active(slot, None);
        Ok(())
    }
    fn advance_slot<H: DialogManagerHost>(
        &mut self,
        slot: usize,
        sink: DialogSink,
        controller: Option<i32>,
        host: &mut H,
    ) -> Result<bool, DialogManagerError<H::Error>> {
        let active = self.active[slot].unwrap();
        host.bind(sink, active);
        let done = self
            .dialogs
            .get_mut(&active.dialog)
            .unwrap()
            .advance(active.interaction, controller, host)
            .map_err(DialogManagerError::Playback)?;
        self.sync(active.dialog);
        Ok(done)
    }
    pub fn advance_controller<H: DialogManagerHost>(
        &mut self,
        controller: Option<i32>,
        cutscene: bool,
        role: NetworkRole,
        host: &mut H,
    ) -> Result<(), DialogManagerError<H::Error>> {
        if cutscene || self.holdoff > 0.0 {
            return Ok(());
        }
        for slot in 0..8 {
            let Some(active) = self.active[slot] else {
                continue;
            };
            let interaction = self.dialogs[&active.dialog]
                .interaction(active.interaction)
                .unwrap();
            if interaction.controller != controller {
                continue;
            }
            if role != NetworkRole::Offline {
                host.manager_effect(DialogManagerEffect::NetworkAdvance {
                    current: interaction.current,
                })
                .map_err(DialogManagerError::Host)?;
            }
            let sink = DialogSink::Slot(slot);
            if host.animating(sink) {
                host.manager_effect(DialogManagerEffect::FinishAnimation {
                    sink,
                    remove_ttl_advance: true,
                })
                .map_err(DialogManagerError::Host)?;
            } else if self.advance_slot(slot, sink, controller, host)? {
                self.finish(slot, true, host)?;
            }
        }
        Ok(())
    }
    pub fn network_advance<H: DialogManagerHost>(
        &mut self,
        current: i32,
        host: &mut H,
    ) -> Result<(), DialogManagerError<H::Error>> {
        for slot in 0..8 {
            let Some(active) = self.active[slot] else {
                continue;
            };
            if self.dialogs[&active.dialog]
                .interaction(active.interaction)
                .unwrap()
                .current
                != current
            {
                continue;
            }
            let sink = DialogSink::Slot(slot);
            if host.animating(sink) {
                host.manager_effect(DialogManagerEffect::FinishAnimation {
                    sink,
                    remove_ttl_advance: true,
                })
                .map_err(DialogManagerError::Host)?;
            } else if self.advance_slot(slot, sink, None, host)? {
                self.finish(slot, true, host)?;
            }
        }
        Ok(())
    }
    pub fn advance_textbox<H: DialogManagerHost>(
        &mut self,
        sink: DialogSink,
        cutscene: bool,
        host: &mut H,
    ) -> Result<(), DialogManagerError<H::Error>> {
        if let DialogSink::Slot(slot) = sink {
            if slot >= 8 {
                return Err(DialogManagerError::InvalidSlot(slot));
            }
        }
        if cutscene {
            if sink == DialogSink::Cutscene
                && self.active[self.cutscene_slot].is_some()
                && self.advance_slot(self.cutscene_slot, sink, None, host)?
            {
                self.finish(self.cutscene_slot, false, host)?;
                self.draw_cutscene_text = false;
            }
        } else if let DialogSink::Slot(slot) = sink {
            if self.active[slot].is_some() && self.advance_slot(slot, sink, None, host)? {
                self.finish(slot, false, host)?;
            }
        }
        host.manager_effect(DialogManagerEffect::HideAdditional)
            .map_err(DialogManagerError::Host)
    }
    pub fn end_all<H: DialogManagerHost>(
        &mut self,
        host: &mut H,
    ) -> Result<(), DialogManagerError<H::Error>> {
        for slot in 0..8 {
            if let Some(active) = self.active[slot] {
                host.bind(DialogSink::Slot(slot), active);
                self.dialogs[&active.dialog]
                    .end(active.interaction, host)
                    .map_err(DialogManagerError::Playback)?;
            }
            self.set_active(slot, None);
            host.manager_effect(DialogManagerEffect::FinishAnimation {
                sink: DialogSink::Slot(slot),
                remove_ttl_advance: false,
            })
            .map_err(DialogManagerError::Host)?;
            host.manager_effect(DialogManagerEffect::Hide(DialogSink::Slot(slot)))
                .map_err(DialogManagerError::Host)?;
        }
        host.manager_effect(DialogManagerEffect::Hide(DialogSink::Cutscene))
            .map_err(DialogManagerError::Host)?;
        host.manager_effect(DialogManagerEffect::FinishAnimation {
            sink: DialogSink::Cutscene,
            remove_ttl_advance: false,
        })
        .map_err(DialogManagerError::Host)?;
        self.awaiting_input = false;
        host.manager_effect(DialogManagerEffect::Unfreeze)
            .map_err(DialogManagerError::Host)?;
        host.manager_effect(DialogManagerEffect::UnlimitInput)
            .map_err(DialogManagerError::Host)
    }
    fn end_matching<H: DialogManagerHost>(
        &mut self,
        host: &mut H,
        matches: impl Fn(&Self, ActiveInteraction) -> bool,
    ) -> Result<(), DialogManagerError<H::Error>> {
        for slot in 0..8 {
            let Some(active) = self.active[slot] else {
                continue;
            };
            if !matches(self, active) {
                continue;
            }
            let sink = DialogSink::Slot(slot);
            host.bind(sink, active);
            self.dialogs[&active.dialog]
                .end(active.interaction, host)
                .map_err(DialogManagerError::Playback)?;
            self.set_active(slot, None);
            host.manager_effect(DialogManagerEffect::Hide(sink))
                .map_err(DialogManagerError::Host)?;
        }
        Ok(())
    }
    /// Core End(Avatar) matching after the host resolves Avatar.Player.Controller.
    /// Null controllers never match null interactors.
    pub fn end_for_controller<H: DialogManagerHost>(
        &mut self,
        controller: Option<i32>,
        host: &mut H,
    ) -> Result<(), DialogManagerError<H::Error>> {
        let Some(controller) = controller else {
            return Ok(());
        };
        self.end_matching(host, |manager, active| {
            manager.dialogs[&active.dialog]
                .interaction(active.interaction)
                .unwrap()
                .controller
                == Some(controller)
        })
    }
    /// Full End(Avatar) property-read ordering. A non-null first controller read
    /// permits the second lookup used for reference equality with Interactor.
    pub fn end_avatar<H: DialogAvatarHost>(
        &mut self,
        avatar: i32,
        host: &mut H,
    ) -> Result<(), DialogManagerError<H::Error>> {
        for slot in 0..8 {
            let Some(active) = self.active[slot] else {
                continue;
            };
            if host
                .avatar_controller(avatar)
                .map_err(DialogManagerError::Host)?
                .is_none()
            {
                continue;
            }
            let interactor = self.dialogs[&active.dialog]
                .interaction(active.interaction)
                .unwrap()
                .controller;
            let controller = host
                .avatar_controller(avatar)
                .map_err(DialogManagerError::Host)?;
            if interactor != controller {
                continue;
            }
            let sink = DialogSink::Slot(slot);
            host.bind(sink, active);
            self.dialogs[&active.dialog]
                .end(active.interaction, host)
                .map_err(DialogManagerError::Playback)?;
            self.set_active(slot, None);
            host.manager_effect(DialogManagerEffect::Hide(sink))
                .map_err(DialogManagerError::Host)?;
        }
        Ok(())
    }
    pub fn end_dialog<H: DialogManagerHost>(
        &mut self,
        dialog: i32,
        host: &mut H,
    ) -> Result<(), DialogManagerError<H::Error>> {
        self.end_matching(host, |_, active| active.dialog == dialog)
    }
    pub fn reset<H: DialogManagerHost>(
        &mut self,
        host: &mut H,
    ) -> Result<(), DialogManagerError<H::Error>> {
        self.end_all(host)?;
        let ids: Vec<_> = self.dialogs.keys().copied().collect();
        for id in ids {
            self.dialogs.get_mut(&id).unwrap().reset();
            self.sync(id);
            self.ledger.finished.insert(id, false);
        }
        Ok(())
    }
}
