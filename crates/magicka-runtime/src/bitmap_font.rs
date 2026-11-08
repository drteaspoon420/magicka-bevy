//! CPU text metrics from PolygonHead.BitmapFont; no font rasterization or assets.
use crate::{
    dialog_definition::MessageDefinition,
    localization::{LocalizationError, LocalizationTable},
};
use magicka_content::assets::BitmapFont;
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
pub enum FontError {
    DuplicateGlyph(u16),
    DuplicateKerning(u16, u16),
    NonBmpGlyph,
    EmptyFont,
    InvalidUtf16,
    /// Text.SetText(null) writes character zero, even for a zero-length buffer.
    EmptyTextBuffer,
    Localization(LocalizationError),
}
#[derive(Clone, Debug)]
pub struct FontMetrics {
    pub line_height: i32,
    pub spacing: i32,
    default_character: u16,
    first_advance: Option<i32>,
    advances: BTreeMap<u16, i32>,
    kerning: BTreeMap<(u16, u16), i32>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct WrappedText {
    pub text: String,
    pub line_width: i32,
}
impl FontMetrics {
    /// BitmapFont.Read(BitmapFont) copies into an existing font but resets
    /// spacing to zero, ignoring source FontManager custom spacing.
    pub fn read_from(&mut self, source: &Self) {
        self.clone_from(source);
        self.spacing = 0;
    }
    /// Read initializes spacing to zero; FontManager subsequently overrides it.
    pub fn from_content(font: &BitmapFont) -> Result<Self, FontError> {
        fn unit(c: char) -> Result<u16, FontError> {
            u16::try_from(c as u32).map_err(|_| FontError::NonBmpGlyph)
        }
        let mut advances = BTreeMap::new();
        for glyph in &font.glyphs {
            let c = unit(glyph.character)?;
            if advances.insert(c, glyph.advance_width).is_some() {
                return Err(FontError::DuplicateGlyph(c));
            }
        }
        let mut kerning = BTreeMap::new();
        for &(left, right, amount) in &font.kerning {
            let pair = (unit(left)?, unit(right)?);
            if kerning.insert(pair, amount).is_some() {
                return Err(FontError::DuplicateKerning(pair.0, pair.1));
            }
        }
        Ok(Self {
            line_height: font.line_height,
            spacing: 0,
            default_character: unit(font.default_character)?,
            first_advance: font.glyphs.first().map(|g| g.advance_width),
            advances,
            kerning,
        })
    }
    pub fn glyph_advance(&self, c: u16) -> Result<(bool, i32), FontError> {
        if let Some(&advance) = self.advances.get(&c) {
            return Ok((true, advance));
        }
        Ok((
            false,
            self.advances
                .get(&self.default_character)
                .copied()
                .or(self.first_advance)
                .ok_or(FontError::EmptyFont)?,
        ))
    }
    pub fn calc_kern(&self, left: u16, right: u16) -> i32 {
        self.kerning.get(&(left, right)).copied().unwrap_or(0)
    }
    fn add_glyph(
        &self,
        width: &mut i32,
        left: &mut u16,
        c: u16,
        kern: bool,
    ) -> Result<(), FontError> {
        let (_, advance) = self.glyph_advance(c)?;
        if kern {
            *width = width.wrapping_add(self.calc_kern(*left, c));
            *left = c;
        }
        *width = width.wrapping_add(advance).wrapping_add(self.spacing);
        Ok(())
    }
    pub fn measure(&self, text: &str, kern: bool) -> Result<[f32; 2], FontError> {
        let units: Vec<_> = text.encode_utf16().collect();
        self.measure_utf16(&units, kern, i32::MAX)
    }
    /// Length is a signed CLR UTF-16 character count. A negative length visits no
    /// characters. Newline is still measured as a glyph after resetting width.
    pub fn measure_utf16(
        &self,
        text: &[u16],
        kern: bool,
        length: i32,
    ) -> Result<[f32; 2], FontError> {
        let mut size = [0.0f32, self.line_height as f32];
        let (mut width, mut left, mut tag) = (0i32, 0u16, false);
        for &c in text.iter().take(usize::try_from(length).unwrap_or(0)) {
            if tag {
                match c {
                    91 => {
                        self.add_glyph(&mut width, &mut left, c, kern)?;
                        tag = false;
                    }
                    93 => tag = false,
                    _ => {}
                }
                continue;
            }
            if c == 0 {
                break;
            }
            if c == 10 {
                size[1] += self.line_height as f32;
                size[0] = size[0].max(width as f32);
                width = 0;
            }
            if c == 91 {
                tag = true;
                continue;
            }
            self.add_glyph(&mut width, &mut left, c, kern)?;
        }
        size[0] = size[0].max(width as f32);
        if size[0] > 1.0 {
            size[0] -= self.spacing as f32;
        }
        Ok(size)
    }
    pub fn wrap(
        &self,
        text: &str,
        mut target_width: i32,
        kern: bool,
    ) -> Result<WrappedText, FontError> {
        let units: Vec<_> = text.encode_utf16().collect();
        loop {
            let mut output = vec![];
            let mut start = 0;
            let mut restart = false;
            while start < units.len() {
                let (mut split, mut split_width, mut width, mut left, mut tag) =
                    (units.len(), 0, 0, 0, false);
                let mut i = start;
                while i < units.len() {
                    let c = units[i];
                    if tag {
                        match c {
                            91 => {
                                self.add_glyph(&mut width, &mut left, c, kern)?;
                                tag = false;
                            }
                            93 => tag = false,
                            _ => {}
                        }
                    } else {
                        if c == 32 && (width <= target_width || i < split) {
                            split = i;
                            split_width = width;
                        }
                        if c == 0 {
                            break;
                        }
                        if c == 10 {
                            split = i;
                            break;
                        }
                        if c == 91 {
                            tag = true;
                        } else {
                            self.add_glyph(&mut width, &mut left, c, kern)?;
                        }
                    }
                    if width > target_width && i >= split {
                        break;
                    }
                    i += 1;
                }
                if i == units.len() && width <= target_width {
                    split_width = width;
                    split = i;
                }
                if split_width > target_width {
                    target_width = split_width;
                    restart = true;
                    break;
                }
                output.extend_from_slice(&units[start..split]);
                if split < units.len() {
                    output.push(10);
                }
                start = split + 1;
            }
            if !restart {
                return Ok(WrappedText {
                    text: String::from_utf16(&output).map_err(|_| FontError::InvalidUtf16)?,
                    line_width: target_width,
                });
            }
        }
    }
}

/// Persistent Message.Initialize output. Reinitialization replaces text entries
/// while preserving the prior componentwise maximum box size, as native code does.
#[derive(Debug, Default)]
pub struct MessageTextLayout {
    pub texts: Vec<Option<String>>,
    pub minimum_box_size: [f32; 2],
}
impl MessageTextLayout {
    pub fn initialize(
        &mut self,
        definition: &MessageDefinition,
        strings: &LocalizationTable,
        font: &FontMetrics,
        substitution_budget: usize,
    ) -> Result<(), FontError> {
        self.texts = vec![None; definition.texts.len()];
        for (i, text) in definition.texts.iter().enumerate() {
            let expanded = strings
                .expand_message(text, substitution_budget)
                .map_err(FontError::Localization)?;
            let wrapped = font.wrap(&expanded, definition.width, true)?;
            self.texts[i] = Some(wrapped.text);
            let size = font.measure(self.texts[i].as_deref().unwrap(), true)?;
            for (old, new) in self.minimum_box_size.iter_mut().zip(size) {
                *old = old.max(new);
            }
        }
        Ok(())
    }
}
