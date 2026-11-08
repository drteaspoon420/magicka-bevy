//! ErrorMessageBox derived Draw and Show ordering; no singleton/device ownership.
use crate::{
    bitmap_font::{FontError, FontMetrics},
    localization::LocalizationTable,
    menu_text_item::MenuTextItemState,
    message_box::{MessageBoxDraw, MessageBoxState, MESSAGE_BOX_SIZE},
    scene_loading::custom_hash,
    text_geometry::TextAlign,
};
pub const ERROR_BOX_Z_INDEX: i32 = 2002;
pub fn default_message_id() -> i32 {
    custom_hash("#network_32")
}
pub fn ok_title_id() -> i32 {
    custom_hash("#add_menu_ok")
}

pub struct ErrorMessageBoxState {
    pub base: MessageBoxState,
    pub timer: f32,
    pub line_offset: f32,
    pub message: String,
    /// Constructor-owned button position does not follow later screen changes.
    pub ok_position: [f32; 2],
    pub ok_selected: bool,
    pub ok_button: Option<MenuTextItemState>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErrorMessageBoxDraw {
    pub background: Option<MessageBoxDraw>,
    pub message_position: [f32; 2],
    pub ok_position: [f32; 2],
}
#[derive(Debug, PartialEq, Eq)]
pub enum ErrorMessageBoxShowError<E> {
    Registration(E),
    Font(FontError),
}
impl ErrorMessageBoxState {
    /// Original singleton constructor: LOC_ANY base caption and localized LOC_OK.
    pub fn new_localized(
        screen: [f32; 2],
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<Self, FontError> {
        let mut state = Self::new(screen, strings.get_string(default_message_id()).unwrap());
        state.ok_button = Some(MenuTextItemState::new_localized(
            ok_title_id(),
            state.ok_position,
            TextAlign::Center,
            40,
            strings,
            font,
        )?);
        Ok(state)
    }
    /// Caller supplies the constructor's already initialized base message.
    pub fn new(screen: [f32; 2], initial_message: String) -> Self {
        let base = MessageBoxState::new(screen);
        Self {
            ok_position: [base.center[0], base.center[1] + 64.],
            base,
            timer: 0.,
            line_offset: 0.,
            message: initial_message,
            ok_selected: false,
            ok_button: None,
        }
    }
    pub fn new_with_ok_button(
        screen: [f32; 2],
        initial_message: String,
        ok_title: &str,
        font: &FontMetrics,
    ) -> Result<Self, FontError> {
        let mut state = Self::new(screen, initial_message);
        state.ok_button = Some(MenuTextItemState::new(
            ok_title,
            state.ok_position,
            TextAlign::Center,
            40,
            font,
        )?);
        Ok(state)
    }
    pub fn show<E>(
        &mut self,
        text: &str,
        font: &FontMetrics,
        mut register: impl FnMut(&mut Self) -> Result<(), E>,
    ) -> Result<(), ErrorMessageBoxShowError<E>> {
        self.base.show();
        register(self).map_err(ErrorMessageBoxShowError::Registration)?;
        let wrapped = font
            .wrap(text, (MESSAGE_BOX_SIZE[0] * 0.9) as i32, true)
            .map_err(ErrorMessageBoxShowError::Font)?;
        self.message = wrapped.text;
        let lines = self.message.split('\n').count() as i32;
        self.line_offset = lines.wrapping_mul(font.line_height) as f32 * 0.5;
        self.timer = 5.;
        Ok(())
    }
    pub fn on_select(&mut self) {
        self.timer = 0.;
    }
    /// Native ButtonState.Released selects inside bounds; old state is ignored.
    pub fn on_mouse_click(&mut self, left_released: bool, inside_ok: bool) {
        if left_released && inside_ok {
            self.on_select();
        }
    }
    pub fn on_mouse_move(&mut self, inside_ok: bool) {
        self.ok_selected = inside_ok;
        if let Some(button) = &mut self.ok_button {
            button.selected = inside_ok;
        }
    }
    pub fn draw(&mut self, dt: f32, screen: [f32; 2], daisy_wheel: bool) -> ErrorMessageBoxDraw {
        let background = self.base.draw(dt, screen, daisy_wheel);
        if self.timer <= 0. {
            self.base.kill();
        }
        self.timer -= dt;
        // Derived code continues even if base Draw returned for DaisyWheel.
        ErrorMessageBoxDraw {
            background,
            message_position: [self.base.center[0], self.base.center[1] - self.line_offset],
            ok_position: self.ok_position,
        }
    }
}
