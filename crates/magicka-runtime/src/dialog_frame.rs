//! Host boundary for DialogManager.Update, including synchronous UI reentry.
use crate::dialog_manager::{DialogManager, DialogManagerHost};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialogFrame {
    pub channel: usize,
    pub dt: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogFrameTarget {
    Slot(usize),
    Additional(usize),
    Cutscene,
}
pub trait DialogFrameHost: DialogManagerHost {
    /// Implement TextBox begin-update, synchronous manager advancement, then
    /// finish-update. The matrix is the native shared ref argument.
    fn update_textbox(
        &mut self,
        manager: &mut DialogManager,
        target: DialogFrameTarget,
        frame: DialogFrame,
        view_projection: &mut [[f32; 4]; 4],
    ) -> Result<(), Self::Error>;
    /// True only for a present, dead owner. Called only after visible succeeds.
    fn slot_owner_dead(&mut self, slot: usize) -> Result<bool, Self::Error>;
    fn additional_count(&self) -> usize;
    fn additional_visible(&self, index: usize) -> bool;
    fn remove_additional(&mut self, index: usize) -> Result<(), Self::Error>;
    fn message_box_count(&self) -> usize;
    fn update_message_box(
        &mut self,
        manager: &mut DialogManager,
        index: usize,
        frame: DialogFrame,
    ) -> Result<(), Self::Error>;
    fn message_box_dead(&self, index: usize) -> bool;
    fn remove_message_box(&mut self, index: usize) -> Result<(), Self::Error>;
    /// Both CurrentState is PlayState and its IsInCutscene must be true.
    fn play_state_in_cutscene(&self) -> bool;
    fn submit_subtitles(&mut self, channel: usize, height: f32) -> Result<(), Self::Error>;
}
