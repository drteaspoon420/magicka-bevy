//! Recovered UTF-16 editing and callback ordering. Rendering/IME remain host work.
//! See research/text-input-message-box.md for native methods and remaining gaps.
use crate::{
    bitmap_font::{FontError, FontMetrics},
    message_box::{MessageBoxState, MESSAGE_BOX_SIZE},
    message_box_device::BoxDirection,
};

pub const DATE_TEMPLATE: [u16; 10] = [77, 77, 47, 68, 68, 47, 89, 89, 89, 89];
pub const TEXT_INPUT_Z_INDEX: i32 = 2002;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextInputType(pub i32);
impl TextInputType {
    pub const STANDARD: Self = Self(0);
    pub const DATE: Self = Self(1);
    pub const NUMERIC_ONLY: Self = Self(2);
}

#[derive(Debug, PartialEq, Eq)]
pub enum TextInputError<E> {
    InvalidCapacity,
    CapacityExceeded,
    CharacterIndex(i32),
    InvalidOperation,
    Host(E),
}

/// Ordered external operations. Implementations may reenter/mutate the box.
/// Text is UTF-16, including isolated surrogates. Category facts are supplied
/// separately by the host, rather than substituting modern Unicode tables.
pub trait TextInputHost {
    type Error;
    fn register(&mut self, state: &mut TextInputMessageBoxState) -> Result<(), Self::Error>;
    fn set_text(
        &mut self,
        state: &mut TextInputMessageBoxState,
        text: Option<&[u16]>,
    ) -> Result<(), Self::Error>;
    fn complete(
        &mut self,
        state: &mut TextInputMessageBoxState,
        callback: u64,
        text: &[u16],
    ) -> Result<(), Self::Error>;
}

pub struct TextInputMessageBoxState {
    pub base: MessageBoxState,
    pub input_type: TextInputType,
    pub secure_mode: bool,
    pub background_enabled: bool,
    pub okay_enabled: bool,
    pub cancel_enabled: bool,
    pub okay_selected: bool,
    pub cancel_selected: bool,
    pub character_index: i32,
    pub completion: Option<u64>,
    /// Draw-owned fields; Show deliberately retains these.
    pub blink_timer: f32,
    pub blink_line: bool,
    text_only: bool,
    buffer: Vec<u16>,
    capacity: usize,
}
impl TextInputMessageBoxState {
    pub fn new(screen: [f32; 2], max_length: i32) -> Result<Self, TextInputError<()>> {
        if max_length <= 0 {
            return Err(TextInputError::InvalidCapacity);
        }
        Ok(Self {
            base: MessageBoxState::new(screen),
            input_type: TextInputType::STANDARD,
            secure_mode: false,
            background_enabled: true,
            okay_enabled: true,
            cancel_enabled: true,
            okay_selected: false,
            cancel_selected: false,
            character_index: 0,
            completion: None,
            blink_timer: 0.,
            blink_line: false,
            text_only: false,
            buffer: Vec::new(),
            capacity: max_length as usize,
        })
    }
    pub fn text(&self) -> &[u16] {
        &self.buffer
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn text_only(&self) -> bool {
        self.text_only
    }
    pub fn set_text_only(&mut self, value: bool) {
        if self.text_only != value {
            self.text_only = value;
            self.background_enabled = !value;
            self.okay_enabled = !value;
            self.cancel_enabled = !value;
        }
    }
    pub fn show_without_callback(&mut self) -> Result<(), TextInputError<()>> {
        Err(TextInputError::InvalidOperation)
    }
    /// Keyboard/controller-neutral prefix. DaisyWheel setup follows this in the
    /// native host and is deliberately not simulated by a keyboard-only API.
    pub fn show<H: TextInputHost>(
        &mut self,
        callback: Option<u64>,
        host: &mut H,
    ) -> Result<(), TextInputError<H::Error>> {
        self.completion = callback;
        self.base.show();
        host.register(self).map_err(TextInputError::Host)?;
        self.buffer.clear();
        if self.input_type == TextInputType::DATE {
            self.character_index = 0;
            if DATE_TEMPLATE.len() > self.capacity {
                return Err(TextInputError::CapacityExceeded);
            }
            self.buffer.extend_from_slice(&DATE_TEMPLATE);
            host.set_text(self, Some(&DATE_TEMPLATE))
                .map_err(TextInputError::Host)?;
        } else {
            host.set_text(self, None).map_err(TextInputError::Host)?;
        }
        Ok(())
    }
    pub fn display_text(&self) -> Vec<u16> {
        if self.secure_mode {
            vec![42; self.buffer.len()]
        } else {
            self.buffer.clone()
        }
    }
    fn refresh<H: TextInputHost>(&mut self, host: &mut H) -> Result<(), TextInputError<H::Error>> {
        let text = self.display_text();
        host.set_text(self, Some(&text))
            .map_err(TextInputError::Host)
    }
    fn replace_date_unit<E>(&mut self, unit: u16) -> Result<(), TextInputError<E>> {
        let index = self.character_index;
        let slot = usize::try_from(index)
            .ok()
            .and_then(|i| self.buffer.get_mut(i))
            .ok_or(TextInputError::CharacterIndex(index))?;
        *slot = unit;
        Ok(())
    }
    fn backspace<H: TextInputHost>(
        &mut self,
        host: &mut H,
    ) -> Result<(), TextInputError<H::Error>> {
        if self.input_type != TextInputType::DATE {
            self.buffer.pop();
        } else if self.character_index >= 0 {
            if self.character_index == 10 {
                self.character_index = 9;
            } else {
                self.character_index = self.character_index.wrapping_sub(1);
                if self.character_index > 0 {
                    let unit = DATE_TEMPLATE
                        .get(self.character_index as usize)
                        .ok_or(TextInputError::CharacterIndex(self.character_index))?;
                    if *unit == 47 {
                        self.character_index = self.character_index.wrapping_sub(1);
                    }
                }
            }
            if self.character_index >= 0 {
                let unit = *DATE_TEMPLATE
                    .get(self.character_index as usize)
                    .ok_or(TextInputError::CharacterIndex(self.character_index))?;
                self.replace_date_unit(unit)?;
            }
        }
        self.refresh(host)
    }
    /// clr_is_number is the result of System.Char.IsNumber for this u16. The
    /// controller's IsValidText filter runs before this concrete-box callback.
    pub fn on_text_input<H: TextInputHost>(
        &mut self,
        unit: u16,
        clr_is_number: bool,
        daisy_wheel: bool,
        host: &mut H,
    ) -> Result<(), TextInputError<H::Error>> {
        if daisy_wheel {
            return Ok(());
        }
        if unit == 8 {
            return self.backspace(host);
        }
        let date = self.input_type == TextInputType::DATE;
        if (date && self.character_index >= 10) || (!date && self.buffer.len() >= self.capacity) {
            return Ok(());
        }
        if self.input_type == TextInputType::STANDARD
            || (self.input_type == TextInputType::NUMERIC_ONLY && clr_is_number)
        {
            self.buffer.push(unit);
        } else if date && clr_is_number {
            self.character_index = self.character_index.max(0);
            self.replace_date_unit(unit)?;
            self.character_index = self.character_index.wrapping_add(1);
            if self.character_index < 9 && DATE_TEMPLATE[self.character_index as usize] == 47 {
                self.character_index += 1;
            }
        }
        self.refresh(host)
    }
    pub fn on_move<H: TextInputHost>(
        &mut self,
        direction: BoxDirection,
        daisy_wheel: bool,
        host: &mut H,
    ) -> Result<(), TextInputError<H::Error>> {
        if daisy_wheel {
            return Ok(());
        }
        if direction == BoxDirection::Left {
            self.backspace(host)?;
        }
        self.refresh(host)
    }
    pub fn on_mouse_move(&mut self, inside_okay: bool, inside_cancel: bool, daisy_wheel: bool) {
        if !daisy_wheel {
            self.okay_selected = inside_okay;
            self.cancel_selected = !inside_okay && inside_cancel;
        }
    }
    fn complete<H: TextInputHost>(
        &mut self,
        text: &[u16],
        host: &mut H,
    ) -> Result<(), TextInputError<H::Error>> {
        if let Some(callback) = self.completion {
            host.complete(self, callback, text)
                .map_err(TextInputError::Host)?;
        }
        self.base.kill();
        Ok(())
    }
    pub fn on_select<H: TextInputHost>(
        &mut self,
        daisy_wheel: bool,
        host: &mut H,
    ) -> Result<(), TextInputError<H::Error>> {
        if self.base.alpha < 1. || self.base.killed || self.text_only || daisy_wheel {
            return Ok(());
        }
        let text = self.buffer.clone();
        self.complete(&text, host)
    }
    pub fn on_mouse_click<H: TextInputHost>(
        &mut self,
        left_released: bool,
        inside_okay: bool,
        inside_cancel: bool,
        daisy_wheel: bool,
        host: &mut H,
    ) -> Result<(), TextInputError<H::Error>> {
        if daisy_wheel || !left_released {
            return Ok(());
        }
        if inside_okay && self.okay_enabled {
            self.on_select(false, host)
        } else if inside_cancel && self.cancel_enabled {
            self.complete(&[], host)
        } else {
            Ok(())
        }
    }
    pub fn wrapped_text(&self, font: &FontMetrics) -> Result<Vec<u16>, FontError> {
        let text = self.display_text();
        let width = (MESSAGE_BOX_SIZE[0] * 0.8) as i32 as f32;
        if font.measure_utf16(&text, true, i32::MAX)?[0] <= width {
            return Ok(text);
        }
        let mut out = Vec::new();
        let mut current = 0.;
        for unit in text {
            let advance = font.measure_utf16(&[unit], true, i32::MAX)?[0];
            if current + advance >= width {
                out.push(10);
                current = 0.;
            }
            out.push(unit);
            current += advance;
        }
        Ok(out)
    }
}
