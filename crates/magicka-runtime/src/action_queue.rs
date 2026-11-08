//! Action.OnTrigger/Update/ClearDelayed timing with synchronous host operations.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActionQueueState {
    pub countdown: f32,
    pub pending: i32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActionQueue {
    pub delay: f32,
    pub state: ActionQueueState,
}
impl ActionQueue {
    pub fn new(delay: f32) -> Self {
        Self {
            delay,
            state: ActionQueueState::default(),
        }
    }
    pub fn trigger(&mut self) {
        if self.state.pending <= 0 {
            self.state.pending = 1;
            self.state.countdown = self.delay;
        } else {
            self.state.pending = self.state.pending.wrapping_add(1);
        }
    }
    pub fn finished(&self) -> bool {
        self.state.pending == 0
    }
    pub fn clear_delayed(&mut self) {
        self.state.pending = 0;
    }
    /// State.Reset changes countdown/pending only, preserving Delay and tag.
    /// This timing-only API does not store derived action tags.
    pub fn reset(&mut self) {
        self.state = ActionQueueState::default();
    }
    /// Countdown advances even with no queued work. A successful Execute is
    /// immediately followed by scene notification. Both callbacks may mutate
    /// timing/requeue; no arbitrary per-frame cap changes native catch-up.
    /// Failure leaves pre-execution mutations in place and stops this update.
    pub fn update<E>(
        &mut self,
        dt: f32,
        mut execute: impl FnMut(&mut Self) -> Result<(), E>,
        mut notify: impl FnMut(&mut Self) -> Result<(), E>,
    ) -> Result<usize, E> {
        self.state.countdown -= dt;
        let mut count = 0;
        while self.state.countdown <= 0.0 && self.state.pending > 0 {
            self.state.pending -= 1;
            self.state.countdown += self.delay;
            execute(self)?;
            notify(self)?;
            count += 1;
        }
        Ok(count)
    }
}
