//! WaitingMessageBox's raw localization, persistent dots and abort ordering.
use crate::{
    bitmap_font::{FontError, FontMetrics},
    localization::LocalizationTable,
    message_box::MessageBoxState,
    scene_loading::custom_hash,
};
pub fn default_message_id() -> i32 {
    custom_hash("#network_24")
}
pub struct WaitingMessageBoxState {
    pub base: MessageBoxState,
    pub message_id: i32,
    pub message: Option<String>,
    pub characters: Vec<u16>,
    pub dot_position: [f32; 2],
    pub timer: f32,
    dot_count: i32,
    pub disable_back: bool,
    /// Host-owned callable identity; no native delegate serialization is claimed.
    pub on_abort: Option<u64>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum WaitingShowError<E> {
    Font(FontError),
    Registration(E),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaitingMessageBoxDraw {
    pub screen: [f32; 2],
    pub overlay_color: [f32; 4],
    pub text_color: [f32; 4],
    pub message_position: [f32; 2],
    pub dot_position: [f32; 2],
    pub dot_count: i32,
}
impl WaitingMessageBoxState {
    pub fn new(screen: [f32; 2]) -> Self {
        Self {
            base: MessageBoxState::new(screen),
            message_id: 0,
            message: Some(String::new()),
            characters: vec![0; 512],
            dot_position: [0.; 2],
            timer: 0.,
            dot_count: 0,
            disable_back: false,
            on_abort: None,
        }
    }
    fn refresh_text(
        &mut self,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<(), FontError> {
        self.message = strings.get_string(self.message_id);
        let units: Vec<_> = self
            .message
            .as_deref()
            .unwrap_or("")
            .encode_utf16()
            .collect();
        if units.len() > self.characters.len() {
            self.characters.resize(units.len(), 0);
        }
        self.characters[..units.len()].copy_from_slice(&units);
        if units.len() < self.characters.len() {
            self.characters[units.len()] = 0;
        }
        self.dot_position = self.base.center;
        self.dot_position[0] += font.measure_utf16(&self.characters, true, i32::MAX)?[0] * 0.5;
        Ok(())
    }
    pub fn show<E>(
        &mut self,
        id: i32,
        strings: &LocalizationTable,
        font: &FontMetrics,
        mut register: impl FnMut(&mut Self) -> Result<(), E>,
    ) -> Result<(), WaitingShowError<E>> {
        self.message_id = id;
        self.refresh_text(strings, font)
            .map_err(WaitingShowError::Font)?;
        if self.base.killed {
            self.base.show();
            register(self).map_err(WaitingShowError::Registration)?;
        }
        Ok(())
    }
    pub fn language_changed(
        &mut self,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<(), FontError> {
        if self.message_id != 0 {
            self.refresh_text(strings, font)?;
        }
        Ok(())
    }
    pub fn kill(&mut self) {
        self.on_abort = None;
        self.base.kill();
    }
    pub fn controller_escape<E>(
        &mut self,
        mut abort: impl FnMut(u64, &mut Self) -> Result<(), E>,
    ) -> Result<(), E> {
        if !self.disable_back {
            if let Some(id) = self.on_abort {
                abort(id, self)?;
            }
            self.kill();
        }
        Ok(())
    }
    pub fn dot_count(&self) -> i32 {
        self.dot_count
    }
    pub fn dots(&self) -> &'static str {
        match self.dot_count {
            1 => ".",
            2 => "..",
            3 => "...",
            4 => "....",
            _ => "",
        }
    }
    /// Native prefix before effect/background/message commands. A rendering
    /// failure in those commands must not advance the dot timer.
    pub fn begin_draw(&mut self, dt: f32, screen: [f32; 2]) -> WaitingMessageBoxDraw {
        self.base.advance_alpha(dt);
        WaitingMessageBoxDraw {
            screen,
            overlay_color: [1., 1., 1., self.base.alpha * 0.5],
            text_color: [1., 1., 1., self.base.alpha],
            message_position: self.base.center,
            dot_position: self.dot_position,
            dot_count: self.dot_count,
        }
    }
    /// Called after successful message drawing, before the dots Draw command.
    pub fn advance_dots(&mut self, dt: f32) -> i32 {
        self.timer += dt * 2.5;
        if self.timer > 1. {
            self.timer = 0.;
            self.dot_count += 1;
            if self.dot_count > 4 {
                self.dot_count = 0;
            }
        }
        self.dot_count
    }
    /// Convenience for a successful complete draw; device failures require the
    /// explicit prefix/message/dot phases above.
    pub fn draw(&mut self, dt: f32, screen: [f32; 2]) -> WaitingMessageBoxDraw {
        let mut plan = self.begin_draw(dt, screen);
        plan.dot_count = self.advance_dots(dt);
        plan
    }
}
