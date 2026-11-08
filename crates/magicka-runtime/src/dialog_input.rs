//! Native DialogManager UI input gates and routing decisions.
use crate::dialog_manager::DialogManager;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogInputKind {
    Select,
    Move,
    Type,
    MouseClick,
    Escape,
    MouseMove,
    MouseRelease,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogInputRoute {
    Ignore,
    AdvanceController,
    MessageBox { index: usize, hide_mouse: bool },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmptyMessageBoxStack;
impl DialogManager {
    /// Native registration sets holdoff before appending, except paused PlayState.
    /// Caller appends the actual object after this call (without deduplication).
    pub fn register_message_box(&mut self, paused_play_state: bool) {
        if !paused_play_state {
            self.holdoff = 0.2;
        }
    }
    /// A route describes the recovered dispatch only; caller executes its host
    /// callback or Advance(controller), including that method's cutscene gate.
    pub fn input_route(
        &self,
        input: DialogInputKind,
        count: usize,
    ) -> Result<DialogInputRoute, EmptyMessageBoxStack> {
        use DialogInputKind::*;
        use DialogInputRoute::*;
        match input {
            MouseRelease => Ok(Ignore),
            Select if self.holdoff > 0. => Ok(Ignore),
            Select if count == 0 => Ok(AdvanceController),
            Move | Type if self.holdoff > 0. || count == 0 => Ok(Ignore),
            _ => Ok(MessageBox {
                index: count.checked_sub(1).ok_or(EmptyMessageBoxStack)?,
                hide_mouse: input == Move,
            }),
        }
    }
}
