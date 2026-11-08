//! TextInputMessageBox.Draw CPU arithmetic and ordered native host operations.
//! See research/text-input-message-box.md; no graphics API dependency.
use crate::{
    bitmap_font::{FontError, FontMetrics},
    text_input_message_box::{TextInputMessageBoxState, TextInputType},
};

pub type InputDrawTransform = [[f32; 4]; 4];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputButton {
    Okay,
    Cancel,
}
#[derive(Clone, Debug, PartialEq)]
pub enum TextInputDrawCommand {
    ScreenSize([f32; 2]),
    /// Transform/color and page texture/vertex bindings, before effect Begin.
    PreparePage {
        transform: InputDrawTransform,
        color: [f32; 4],
    },
    Begin,
    Background,
    SetBlinker(Vec<u16>),
    MarkInputDirty,
    Color([f32; 4]),
    VertexColor(bool),
    Message([f32; 2]),
    SetInput(Vec<u16>),
    Input(InputDrawTransform),
    Blinker(InputDrawTransform),
    ButtonPosition(InputButton, [f32; 2]),
    ButtonAlpha(InputButton, f32),
    Button(InputButton),
    End,
}
#[derive(Debug, PartialEq, Eq)]
pub enum TextInputDrawError<E> {
    Host(E),
    Font(FontError),
}
/// Commands execute immediately, not as a transactional precomputed plan.
/// SetInput must update the actual retained Text buffer (including terminator
/// and stale tail) before a subsequent input_characters query.
pub trait TextInputDrawHost {
    type Error;
    fn command(&mut self, command: TextInputDrawCommand) -> Result<(), Self::Error>;
    fn input_characters(&self) -> &[u16];
    /// Native fresh LanguageManager lookup occurs after input/cursor drawing.
    fn okay_title(&mut self) -> Result<Vec<u16>, Self::Error>;
}
pub struct TextInputDrawContext<'a> {
    pub screen: [f32; 2],
    pub daisy_wheel: bool,
    /// Base mFont controls wrapping/row spacing/OK position; mText.Font controls
    /// line and cursor widths. They normally refer to the same mutable font.
    pub base_font: &'a FontMetrics,
    pub input_font: &'a FontMetrics,
    /// Retained constructor measurement; caption changes do not recalculate it.
    pub message_height: f32,
}
impl TextInputMessageBoxState {
    pub fn draw_input<H: TextInputDrawHost>(
        &mut self,
        dt: f32,
        context: TextInputDrawContext<'_>,
        host: &mut H,
    ) -> Result<bool, TextInputDrawError<H::Error>> {
        use TextInputDrawCommand as C;
        let emit = |host: &mut H, command| host.command(command).map_err(TextInputDrawError::Host);
        let measure = |text: &[u16], length| {
            context
                .input_font
                .measure_utf16(text, true, length)
                .map_err(TextInputDrawError::Font)
        };
        if context.daisy_wheel {
            return Ok(false);
        }
        self.base.center = [context.screen[0] * 0.5, context.screen[1] * 0.5];
        emit(host, C::ScreenSize(context.screen))?;
        self.base.advance_alpha(dt);
        let [cx, cy] = self.base.center;
        let mut transform = [
            [0., 1., 0., 0.],
            [-1., 0., 0., 0.],
            [0., 0., 1., 0.],
            [cx, cy, 0., 1.],
        ];
        let color = [1., 1., 1., self.base.alpha];
        emit(host, C::PreparePage { transform, color })?;
        emit(host, C::Begin)?;
        if self.background_enabled {
            emit(host, C::Background)?;
        }
        self.blink_timer -= dt;
        while self.blink_timer < 0. {
            self.blink_timer += 0.5;
            self.blink_line = !self.blink_line;
            emit(
                host,
                C::SetBlinker(if self.blink_line {
                    vec![95, 0]
                } else {
                    vec![0]
                }),
            )?;
            emit(host, C::MarkInputDirty)?;
        }
        emit(host, C::Color(color))?;
        let wrapped = self
            .wrapped_text(context.base_font)
            .map_err(TextInputDrawError::Font)?;
        let lines: Vec<_> = wrapped.split(|&c| c == 10).collect();
        let count = lines.len();
        let half_height = context.base_font.line_height as f32 * 0.5;
        let extra = if count > 1 {
            count as f32 * half_height
        } else {
            0.
        };
        emit(host, C::VertexColor(true))?;
        // Retain the subtract/add rounding performed by the native local Vector2.
        let mut local_y = cy;
        let caption_offset = context.message_height + extra;
        local_y -= caption_offset;
        emit(host, C::Message([cx, local_y]))?;
        local_y += caption_offset;
        emit(host, C::VertexColor(false))?;
        // The return is unused natively, but its errors and traversal still matter.
        measure(host.input_characters(), self.text().len() as i32)?;
        transform[0] = [1., 0., 0., 0.];
        transform[1] = [0., 1., 0., 0.];
        let mut y = (local_y - extra) * transform[0][0];
        for line in &lines {
            emit(host, C::SetInput(line.to_vec()))?;
            let width = measure(line, line.len() as i32)?[0];
            transform[3][0] = (cx - 0.5 * width * transform[0][0]).floor();
            transform[3][1] = y;
            emit(host, C::Input(transform))?;
            y += context.base_font.line_height as f32 * transform[0][0];
        }
        if self.input_type == TextInputType::DATE {
            if self.character_index < 10 {
                transform[3][0] += measure(host.input_characters(), self.character_index)?[0];
                emit(host, C::Blinker(transform))?;
            }
        } else if self.text().len() < self.capacity() {
            let width = if count == 1 {
                measure(host.input_characters(), self.text().len() as i32)?[0]
            } else {
                let line = lines.last().unwrap();
                measure(line, line.len() as i32)?[0]
            };
            transform[3][0] += width;
            emit(host, C::Blinker(transform))?;
        }
        let button_extra = if count > 3 {
            (count - 3) as f32 * half_height
        } else {
            0.
        };
        let title = host.okay_title().map_err(TextInputDrawError::Host)?;
        let title_height = context
            .base_font
            .measure_utf16(&title, true, i32::MAX)
            .map_err(TextInputDrawError::Font)?[1];
        let by = cy + (128. - title_height + button_extra);
        emit(host, C::ButtonPosition(InputButton::Okay, [cx - 50., by]))?;
        emit(host, C::ButtonPosition(InputButton::Cancel, [cx + 50., by]))?;
        for (button, enabled) in [
            (InputButton::Okay, self.okay_enabled),
            (InputButton::Cancel, self.cancel_enabled),
        ] {
            if enabled {
                emit(host, C::ButtonAlpha(button, self.base.alpha))?;
                emit(host, C::Button(button))?;
            }
        }
        emit(host, C::End)?;
        Ok(true)
    }
}
