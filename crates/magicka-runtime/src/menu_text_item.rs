//! MenuTextItem bounds, retained UTF-16 and native row-vector Draw transform.
use crate::{
    bitmap_font::{FontError, FontMetrics},
    localization::{LocalizationError, LocalizationTable},
    text_geometry::TextAlign,
};
pub const MENU_COLOR: [f32; 4] = [0.1, 0.05, 0., 0.7];
pub const MENU_COLOR_SELECTED: [f32; 4] = [1.5, 1.5, 1.5, 1.];
pub const MENU_COLOR_DISABLED: [f32; 4] = [0., 0., 0., 0.4];

pub struct MenuTextItemState {
    pub characters: Vec<u16>,
    /// Native cached Name; LanguageChanged updates characters, not this cache.
    pub text: Option<String>,
    pub hash: i32,
    pub position: [f32; 2],
    pub scale: f32,
    pub max_width: f32,
    pub align: TextAlign,
    pub size: [f32; 2],
    pub top_left: [f32; 2],
    pub bottom_right: [f32; 2],
    pub enabled: bool,
    pub selected: bool,
    pub color: [f32; 4],
    pub color_selected: [f32; 4],
    pub color_disabled: [f32; 4],
    pub alpha: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuTextDraw {
    pub transform: [[f32; 4]; 4],
    pub color: [f32; 4],
}
fn min(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else {
        a.min(b)
    }
}
fn max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else {
        a.max(b)
    }
}
impl MenuTextItemState {
    pub fn new(
        text: &str,
        position: [f32; 2],
        align: TextAlign,
        capacity: usize,
        font: &FontMetrics,
    ) -> Result<Self, FontError> {
        let mut state = Self {
            characters: vec![0; capacity],
            text: Some(String::new()),
            hash: 0,
            position,
            scale: 1.,
            max_width: 0.,
            align,
            size: [0.; 2],
            top_left: [0.; 2],
            bottom_right: [0.; 2],
            enabled: true,
            selected: false,
            color: MENU_COLOR,
            color_selected: MENU_COLOR_SELECTED,
            color_disabled: MENU_COLOR_DISABLED,
            alpha: 1.,
        };
        state.set_text(text, font)?;
        Ok(state)
    }
    pub fn set_text(&mut self, text: &str, font: &FontMetrics) -> Result<(), FontError> {
        self.set_nullable_text(Some(text), font)
    }
    pub fn new_localized(
        id: i32,
        position: [f32; 2],
        align: TextAlign,
        capacity: usize,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<Self, FontError> {
        let mut state = Self::new("", position, align, capacity, font)?;
        state.set_title(id, strings, font)?;
        Ok(state)
    }
    fn set_characters(&mut self, text: Option<&str>) -> Result<(), FontError> {
        if text.is_none() {
            *self
                .characters
                .first_mut()
                .ok_or(FontError::EmptyTextBuffer)? = 0;
            return Ok(());
        }
        let units: Vec<_> = text.unwrap().encode_utf16().collect();
        if units.len() > self.characters.len() {
            self.characters.resize(units.len(), 0);
        }
        self.characters[..units.len()].copy_from_slice(&units);
        if units.len() < self.characters.len() {
            self.characters[units.len()] = 0;
        }
        Ok(())
    }
    fn measure_then_update_bounds(&mut self, font: &FontMetrics) -> Result<(), FontError> {
        self.size = font.measure_utf16(&self.characters, true, i32::MAX)?;
        self.update_bounds(font)
    }
    pub fn set_nullable_text(
        &mut self,
        text: Option<&str>,
        font: &FontMetrics,
    ) -> Result<(), FontError> {
        self.set_characters(text)?;
        self.text = text.map(str::to_owned);
        self.measure_then_update_bounds(font)
    }
    /// Native Hash setter changes only mValue.
    pub fn set_hash(&mut self, id: i32) {
        self.hash = id;
    }
    pub fn set_title(
        &mut self,
        id: i32,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<(), FontError> {
        self.hash = id;
        self.text = strings.get_string(id);
        let text = self.text.clone();
        self.set_characters(text.as_deref())?;
        self.measure_then_update_bounds(font)
    }
    /// Native SetText(int) performs two lookups and assigns Name after Text.
    pub fn set_localized_text(
        &mut self,
        id: i32,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<(), FontError> {
        self.hash = id;
        self.set_characters(strings.get_string(id).as_deref())?;
        self.text = strings.get_string(id);
        self.measure_then_update_bounds(font)
    }
    pub fn wrap_text(
        &mut self,
        length: i32,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<(), FontError> {
        let text = strings
            .get_string(self.hash)
            .ok_or(FontError::Localization(LocalizationError::NullString(
                self.hash,
            )))?;
        let wrapped = font.wrap(&text, length, true)?;
        self.set_text(&wrapped.text, font)
    }
    pub fn language_changed(
        &mut self,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<(), FontError> {
        if self.hash != 0 {
            self.set_characters(strings.get_string(self.hash).as_deref())?;
            self.update_bounds(font)?;
        }
        Ok(())
    }
    pub fn set_position(
        &mut self,
        position: [f32; 2],
        font: &FontMetrics,
    ) -> Result<(), FontError> {
        self.position = position;
        self.update_bounds(font)
    }
    pub fn set_scale(&mut self, scale: f32, font: &FontMetrics) -> Result<(), FontError> {
        self.scale = scale;
        self.update_bounds(font)
    }
    pub fn set_max_width(&mut self, width: f32, font: &FontMetrics) -> Result<(), FontError> {
        self.max_width = width;
        self.update_bounds(font)
    }
    pub fn update_bounds(&mut self, font: &FontMetrics) -> Result<(), FontError> {
        self.size = font.measure_utf16(&self.characters, true, i32::MAX)?;
        self.size[0] = max(self.size[0], 20.);
        self.size[1] = max(self.size[1], 20.);
        if self.max_width > f32::from_bits(1) {
            self.size[0] = min(self.size[0], self.max_width);
        }
        let width = self.size[0] * self.scale;
        let half_height = self.size[1] * 0.5 * self.scale;
        let (left, right) = match self.align {
            TextAlign::Left => (self.position[0], self.position[0] + width),
            TextAlign::Center => (
                self.position[0] - self.size[0] * 0.5 * self.scale,
                self.position[0] + self.size[0] * 0.5 * self.scale,
            ),
            TextAlign::Right => (self.position[0] - width, self.position[0]),
        };
        self.top_left = [left, self.position[1] - half_height];
        self.bottom_right = [right, self.position[1] + half_height];
        Ok(())
    }
    pub fn inside_bounds(&self, point: [f32; 2]) -> bool {
        (point[0] >= self.top_left[0])
            & (point[1] >= self.top_left[1])
            & (point[0] <= self.bottom_right[0])
            & (point[1] <= self.bottom_right[1])
    }
    pub fn current_color(&self) -> [f32; 4] {
        if !self.enabled {
            self.color_disabled
        } else if self.selected {
            self.color_selected
        } else {
            self.color
        }
    }
    pub fn draw_plan(&self, scale: f32, font: &FontMetrics) -> Result<MenuTextDraw, FontError> {
        let x_scale = if self.max_width > f32::from_bits(1) {
            min(
                self.max_width / font.measure_utf16(&self.characters, true, i32::MAX)?[0],
                1.,
            ) * scale
        } else {
            scale
        };
        let mut transform = [[0.; 4]; 4];
        transform[0][0] = x_scale;
        transform[1][1] = scale;
        transform[3][0] = self.position[0];
        transform[3][1] = self.position[1] - font.line_height as f32 * 0.5 * scale + 0.5;
        transform[3][3] = 1.;
        let mut color = self.current_color();
        color[3] *= self.alpha;
        Ok(MenuTextDraw { transform, color })
    }
}
