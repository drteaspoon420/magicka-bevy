//! PlayState cutscene state: BeginCutscene/EndCutscene, the skip tip and
//! SkipCutscene (first press shows the tip, the next runs the OnSkip trigger),
//! and the black-bar timer drawn by CutsceneRenderData.
//! See research/cutscene-skip.md.

/// PlayState.CUTSCENE_BLACKBAR_SIZE: bar height as a fraction of the screen.
pub const BLACKBAR_SIZE: f32 = 0.1;
/// "#cut_press_skip"; `#1;` is replaced by the controller's skip button
/// glyph (SpaceBar for keyboard/mouse).
pub const SKIP_TIP_TEXT: &str = "#cut_press_skip";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CutsceneState {
    /// mCutscene.
    pub active: bool,
    pub skipped: bool,
    /// mCutsceneSkipTrigger (0 = none).
    pub skip_trigger: i32,
    pub show_skip_tip: bool,
    pub skip_tip_alpha: f32,
    /// mCutsceneTimer: black-bar slide (0 hidden .. 1 fully in).
    pub timer: f32,
}

impl CutsceneState {
    pub fn begin(&mut self, on_skip: i32, skip_bar_move: bool) {
        self.skipped = false;
        self.active = true;
        self.timer = if skip_bar_move { 1.0 } else { 0.0 };
        self.skip_trigger = on_skip;
    }

    /// EndCutscene. SkipBarMove only affects the (unused) bar removal flag.
    pub fn end(&mut self) {
        self.active = false;
        self.skipped = false;
    }

    /// PlayState.ChangingScene.
    pub fn changing_scene(&mut self) {
        self.skipped = false;
    }

    /// IsInCutscene: active, or the bars are still fully in.
    pub fn in_cutscene(&self) -> bool {
        self.active | (self.timer >= 1.0)
    }

    /// SkipCutscene. Returns the trigger to execute (ignoring nothing; the
    /// caller runs ExecuteTrigger(id, null, false)). Clients never skip.
    pub fn skip(&mut self, client: bool) -> Option<i32> {
        if self.skipped || client {
            return None;
        }
        if !self.show_skip_tip {
            self.show_skip_tip = true;
            return None;
        }
        self.skipped = true;
        let trigger = std::mem::take(&mut self.skip_trigger);
        (trigger != 0).then_some(trigger)
    }

    /// PlayState.Update cutscene section (after the end-game timer).
    pub fn update(&mut self, dt: f32) {
        if self.active {
            self.skip_tip_alpha = if self.skip_trigger != 0 && self.show_skip_tip {
                (self.skip_tip_alpha + dt * 2.0).min(1.0)
            } else {
                (self.skip_tip_alpha - dt * 2.0).max(0.0)
            };
            self.timer = (self.timer + dt).min(1.0);
        } else if self.timer > 0.0 {
            self.timer = (self.timer - dt).max(0.0);
            if self.show_skip_tip {
                self.skip_tip_alpha = (self.skip_tip_alpha - dt).max(0.0);
            }
        } else {
            self.skip_tip_alpha = 0.0;
            self.show_skip_tip = false;
        }
    }

    /// CutsceneRenderData bars in screen pixels (top-left origin): the top
    /// bar's visible height and the bottom bar's top edge, plus whether the
    /// GUI object is drawn this frame.
    pub fn bars(&self, screen_height: f32) -> Option<(f32, f32)> {
        if !self.active && self.timer <= 0.0 {
            return None;
        }
        let h = screen_height * BLACKBAR_SIZE;
        Some((h * self.timer, screen_height - h * self.timer))
    }
}

/// KeyboardMouseController.OnKeyDown: outside message boxes and pause, a
/// key whose XNA `Keys` value has none of the bits 0x1B skips the cutscene
/// (literal `(Key & 27) == 0` test; Space qualifies, Enter/Escape do not).
pub fn key_skips_cutscene(xna_key: u32) -> bool {
    xna_key & 0x1B == 0
}
