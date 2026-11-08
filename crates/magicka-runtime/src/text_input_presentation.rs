//! Retained caption/input/blinker/button state of TextInputMessageBox and a draw
//! host publishing per-draw geometry snapshots. See research/text-input-presentation.md.
use crate::{
    bitmap_font::{FontError, FontMetrics},
    error_message_box::ok_title_id,
    localization::LocalizationTable,
    menu_text_item::{MenuTextItemState, MENU_COLOR},
    retained_text::{RetainedText, TextGeometryCache},
    scene_loading::custom_hash,
    text_geometry::{GeometryError, GeometryFont, TextAlign, TextGeometry, TextStyle},
    text_input_draw::{InputButton, InputDrawTransform, TextInputDrawCommand, TextInputDrawHost},
    text_input_message_box::{TextInputHost, TextInputMessageBoxState},
};

pub const CAPTION_CAPACITY: usize = 512;
pub const BUTTON_CAPACITY: usize = 40;
/// MessageBox.mSize.X * 0.9, truncated by the native int cast.
pub const CAPTION_WRAP_WIDTH: i32 = (448.0f32 * 0.9) as i32;
pub fn cancel_title_id() -> i32 {
    custom_hash("#add_menu_cancel")
}

#[derive(Debug, PartialEq, Eq)]
pub enum PresentationError {
    Font(FontError),
    Geometry(GeometryError),
    /// GetString returned null where native code measures or wraps the result.
    MissingString(i32),
}
impl From<FontError> for PresentationError {
    fn from(error: FontError) -> Self {
        Self::Font(error)
    }
}
impl From<GeometryError> for PresentationError {
    fn from(error: GeometryError) -> Self {
        Self::Geometry(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTarget {
    Caption,
    Input,
    Blinker,
    Button(InputButton),
}
/// One native Text.Draw that reached DrawIndexedPrimitives. Geometry is copied
/// at draw time, so later SetText calls cannot alter earlier multiline captures.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDrawSnapshot {
    pub target: TextTarget,
    pub transform: InputDrawTransform,
    pub color: [f32; 4],
    pub vertex_color: bool,
    pub geometry: TextGeometry,
}

pub struct MenuButton {
    pub item: MenuTextItemState,
    /// mTitle is a static formatting-enabled Text with the item's alignment.
    pub geometry: TextGeometryCache,
}
impl MenuButton {
    fn new(
        id: i32,
        position: [f32; 2],
        align: TextAlign,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<Self, FontError> {
        let mut item =
            MenuTextItemState::new_localized(id, position, align, BUTTON_CAPACITY, strings, font)?;
        item.color_selected = [1.; 4];
        item.color = MENU_COLOR;
        Ok(Self {
            item,
            geometry: TextGeometryCache {
                dirty: true,
                ..Default::default()
            },
        })
    }
    fn style(&self) -> TextStyle {
        TextStyle {
            align: self.item.align,
            ..TextStyle::default()
        }
    }
}

pub struct TextInputPresentation {
    pub caption: RetainedText,
    pub message_hash: i32,
    /// Constructor measurement of the wrapped caption; never recomputed.
    pub message_height: f32,
    /// Native add_LanguageChanged count: base, string and (hash only) int ctors.
    pub language_subscriptions: u32,
    pub input: RetainedText,
    pub blinker: RetainedText,
    pub okay: MenuButton,
    pub cancel: MenuButton,
}
impl TextInputPresentation {
    /// TextInputMessageBox(string, int). `state` must have been created with the
    /// same max length; its base center provides constructor positions.
    pub fn new(
        state: &TextInputMessageBoxState,
        message: &str,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<Self, PresentationError> {
        let wrapped = font.wrap(message, CAPTION_WRAP_WIDTH, true)?.text;
        let mut caption = RetainedText::new(
            CAPTION_CAPACITY,
            false,
            TextStyle {
                default_color: MENU_COLOR,
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        );
        let units: Vec<u16> = wrapped.encode_utf16().collect();
        caption.set_text(Some(&units))?;
        let message_height = font.measure(&wrapped, true)?[1];
        let input_style = TextStyle {
            formatting: false,
            ..TextStyle::default()
        };
        let capacity = state.capacity() + 2;
        let input = RetainedText::new(capacity, true, input_style);
        let blinker = RetainedText::new(capacity, true, input_style);
        let ok_text = strings
            .get_string(ok_title_id())
            .ok_or(PresentationError::MissingString(ok_title_id()))?;
        let [cx, cy] = state.base.center;
        let y = cy + 128. - font.measure(&ok_text, true)?[1];
        let okay = MenuButton::new(
            ok_title_id(),
            [cx - 50., y],
            TextAlign::Right,
            strings,
            font,
        )?;
        // Native fetches the Cancel string but positions it with OK's height.
        let cancel = MenuButton::new(
            cancel_title_id(),
            [cx + 50., y],
            TextAlign::Left,
            strings,
            font,
        )?;
        Ok(Self {
            caption,
            message_hash: 0,
            message_height,
            language_subscriptions: 2,
            input,
            blinker,
            okay,
            cancel,
        })
    }
    /// TextInputMessageBox(int, int): the chained string constructor, then the
    /// hash and a third LanguageChanged subscription.
    pub fn new_localized(
        state: &TextInputMessageBoxState,
        message_id: i32,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<Self, PresentationError> {
        let message = strings
            .get_string(message_id)
            .ok_or(PresentationError::MissingString(message_id))?;
        let mut presentation = Self::new(state, &message, strings, font)?;
        presentation.message_hash = message_id;
        presentation.language_subscriptions = 3;
        Ok(presentation)
    }
    pub fn button(&mut self, button: InputButton) -> &mut MenuButton {
        match button {
            InputButton::Okay => &mut self.okay,
            InputButton::Cancel => &mut self.cancel,
        }
    }
    /// One delegate invocation. The caption receives the raw (unwrapped) string;
    /// height and button positions are untouched.
    pub fn language_changed_once(
        &mut self,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<(), PresentationError> {
        for button in [InputButton::Okay, InputButton::Cancel] {
            let button = self.button(button);
            button.item.language_changed(strings, font)?;
            if button.item.hash != 0 {
                button.geometry.dirty = true;
            }
        }
        if self.message_hash != 0 {
            let text = strings.get_string(self.message_hash);
            let units = text.map(|t| t.encode_utf16().collect::<Vec<_>>());
            self.caption.set_text(units.as_deref())?;
        }
        Ok(())
    }
    /// A single LanguageChanged event reaches this box once per subscription.
    pub fn language_changed(
        &mut self,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<(), PresentationError> {
        for _ in 0..self.language_subscriptions {
            self.language_changed_once(strings, font)?;
        }
        Ok(())
    }
    /// ResolutionMessageBox.Complete handler. Editor and blink fields are kept.
    pub fn change_resolution(
        &mut self,
        state: &mut TextInputMessageBoxState,
        size: [i32; 2],
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<(), PresentationError> {
        let ok_text = strings
            .get_string(ok_title_id())
            .ok_or(PresentationError::MissingString(ok_title_id()))?;
        let height = font.measure(&ok_text, true)?[1];
        let center = [size[0] as f32 * 0.5, size[1] as f32 * 0.5];
        state.base.center = center;
        self.okay
            .item
            .set_position([center[0] - 50., center[1] + (128. - height)], font)?;
        self.cancel
            .item
            .set_position([center[0] + 50., center[1] + (128. - height)], font)?;
        Ok(())
    }
}

/// Forwards registration/completion and applies the editor's mText.SetText.
pub struct PresentationEditHost<'a, H> {
    pub inner: &'a mut H,
    pub input: &'a mut RetainedText,
}
impl<H: TextInputHost> TextInputHost for PresentationEditHost<'_, H> {
    type Error = H::Error;
    fn register(&mut self, state: &mut TextInputMessageBoxState) -> Result<(), H::Error> {
        self.inner.register(state)
    }
    fn set_text(
        &mut self,
        state: &mut TextInputMessageBoxState,
        text: Option<&[u16]>,
    ) -> Result<(), H::Error> {
        // Text.SetText cannot fail for a nonempty buffer; capacity is max+2.
        self.input
            .set_text(text)
            .expect("text-entry buffer is never empty");
        self.inner.set_text(state, text)
    }
    fn complete(
        &mut self,
        state: &mut TextInputMessageBoxState,
        callback: u64,
        text: &[u16],
    ) -> Result<(), H::Error> {
        self.inner.complete(state, callback, text)
    }
}

/// Executes TextInputMessageBox.Draw host operations against retained objects.
pub struct PresentationDrawHost<'a> {
    pub presentation: &'a mut TextInputPresentation,
    pub strings: &'a LocalizationTable,
    pub font: &'a GeometryFont,
    pub effect_color: [f32; 4],
    pub vertex_color: bool,
    pub screen: [f32; 2],
    pub page: Option<(InputDrawTransform, [f32; 4])>,
    pub background: bool,
    pub draws: Vec<TextDrawSnapshot>,
}
impl<'a> PresentationDrawHost<'a> {
    pub fn new(
        presentation: &'a mut TextInputPresentation,
        strings: &'a LocalizationTable,
        font: &'a GeometryFont,
    ) -> Self {
        Self {
            presentation,
            strings,
            font,
            effect_color: [1.; 4],
            vertex_color: true,
            screen: [0.; 2],
            page: None,
            background: false,
            draws: vec![],
        }
    }
    fn record(
        &mut self,
        target: TextTarget,
        transform: InputDrawTransform,
        geometry: TextGeometry,
    ) {
        // Text.Draw returns before any device call when PrimitiveCount is zero.
        if geometry.primitive_count() != 0 {
            self.draws.push(TextDrawSnapshot {
                target,
                transform,
                color: self.effect_color,
                vertex_color: self.vertex_color,
                geometry,
            });
        }
    }
    fn draw_retained(
        &mut self,
        target: TextTarget,
        transform: InputDrawTransform,
    ) -> Result<(), PresentationError> {
        let text = match target {
            TextTarget::Caption => &mut self.presentation.caption,
            TextTarget::Input => &mut self.presentation.input,
            TextTarget::Blinker => &mut self.presentation.blinker,
            TextTarget::Button(_) => unreachable!(),
        };
        let geometry = text.geometry(self.font)?.clone();
        self.record(target, transform, geometry);
        Ok(())
    }
}
fn translation(x: f32, y: f32) -> InputDrawTransform {
    [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [x, y, 0., 1.],
    ]
}
impl TextInputDrawHost for PresentationDrawHost<'_> {
    type Error = PresentationError;
    fn command(&mut self, command: TextInputDrawCommand) -> Result<(), PresentationError> {
        use TextInputDrawCommand as C;
        match command {
            C::ScreenSize(size) => self.screen = size,
            C::PreparePage { transform, color } => {
                self.effect_color = color;
                self.page = Some((transform, color));
            }
            C::Begin | C::End => {}
            C::Background => self.background = true,
            C::SetBlinker(text) => self.presentation.blinker.set_text(Some(&text))?,
            C::MarkInputDirty => self.presentation.input.cache.dirty = true,
            C::Color(color) => self.effect_color = color,
            C::VertexColor(enabled) => self.vertex_color = enabled,
            C::Message([x, y]) => self.draw_retained(TextTarget::Caption, translation(x, y))?,
            C::SetInput(text) => self.presentation.input.set_text(Some(&text))?,
            C::Input(transform) => self.draw_retained(TextTarget::Input, transform)?,
            C::Blinker(transform) => self.draw_retained(TextTarget::Blinker, transform)?,
            C::ButtonPosition(button, position) => {
                let metrics = &self.font.metrics;
                self.presentation
                    .button(button)
                    .item
                    .set_position(position, metrics)?;
            }
            C::ButtonAlpha(button, alpha) => self.presentation.button(button).item.alpha = alpha,
            C::Button(button) => {
                let font = self.font;
                let target = self.presentation.button(button);
                let plan = target.item.draw_plan(target.item.scale, &font.metrics)?;
                let style = target.style();
                let geometry = target
                    .geometry
                    .get(font, &target.item.characters, style)?
                    .clone();
                self.effect_color = plan.color;
                self.record(TextTarget::Button(button), plan.transform, geometry);
            }
        }
        Ok(())
    }
    fn input_characters(&self) -> &[u16] {
        &self.presentation.input.characters
    }
    fn okay_title(&mut self) -> Result<Vec<u16>, PresentationError> {
        self.strings
            .get_string(ok_title_id())
            .map(|t| t.encode_utf16().collect())
            .ok_or(PresentationError::MissingString(ok_title_id()))
    }
}
