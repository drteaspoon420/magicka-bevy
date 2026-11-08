//! PlayState's loading fade/readiness gate. It does not substitute an elapsed
//! camera timer for the final intro cue's actual completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    FadeIn,
    Waiting,
    FadeOut,
    Complete,
}

#[derive(Clone, Copy, Debug)]
pub struct LoadingGate {
    pub phase: Phase,
    pub alpha: f32,
    elapsed: f32,
}
impl Default for LoadingGate {
    fn default() -> Self {
        Self {
            phase: Phase::FadeIn,
            alpha: 0.0,
            elapsed: 0.0,
        }
    }
}
impl LoadingGate {
    /// LoadingScreen.FadeIn(0.5), PlayState's readiness/Cue.IsStopped wait,
    /// then LoadingScreen.FadeOut(0.5). No input can bypass the wait.
    pub fn update(&mut self, dt: f32, assets_ready: bool, cue_finished: bool) {
        match self.phase {
            Phase::FadeIn => {
                self.elapsed += dt;
                self.alpha = (self.elapsed / 0.5).min(1.0);
                if self.alpha >= 1.0 {
                    self.phase = Phase::Waiting;
                    self.elapsed = 0.0;
                }
            }
            Phase::Waiting => {
                if assets_ready && cue_finished {
                    self.phase = Phase::FadeOut;
                    self.elapsed = 0.0;
                }
            }
            Phase::FadeOut => {
                self.elapsed += dt;
                self.alpha = (1.0 - self.elapsed / 0.5).max(0.0);
                if self.alpha <= 0.0 {
                    self.phase = Phase::Complete;
                }
            }
            Phase::Complete => {}
        }
    }
    pub fn blocks_play(&self) -> bool {
        self.phase != Phase::Complete
    }
}

/// SubMenuCharacterSelect starts the vanilla chapter-zero intro even when
/// resuming that chapter. Mythos/chapter maps and hot-join are separate paths.
pub fn uses_opening_intro(vanilla_campaign: bool, chapter: Option<usize>, hot_join: bool) -> bool {
    vanilla_campaign && chapter == Some(0) && !hot_join
}
