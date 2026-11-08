//! PolygonHead.Text's retained UTF-16 buffer and dirty glyph publication.
use crate::{
    bitmap_font::FontError,
    text_geometry::{GeometryError, GeometryFont, TextGeometry, TextStyle},
};

#[derive(Clone, Debug, Default)]
pub struct TextGeometryCache {
    pub dirty: bool,
    pub geometry: TextGeometry,
}
impl TextGeometryCache {
    pub fn get(
        &mut self,
        font: &GeometryFont,
        characters: &[u16],
        style: TextStyle,
    ) -> Result<&TextGeometry, GeometryError> {
        if self.dirty {
            let geometry = TextGeometry::build(font, characters, style)?;
            self.geometry = geometry;
            self.dirty = false;
        }
        Ok(&self.geometry)
    }
}
#[derive(Clone, Debug)]
pub struct RetainedText {
    pub characters: Vec<u16>,
    pub end_index: usize,
    pub dynamic: bool,
    pub style: TextStyle,
    pub cache: TextGeometryCache,
}
impl RetainedText {
    pub fn new(capacity: usize, dynamic: bool, style: TextStyle) -> Self {
        Self {
            characters: vec![0; capacity],
            end_index: 0,
            dynamic,
            style,
            cache: TextGeometryCache::default(),
        }
    }
    pub fn set_text(&mut self, text: Option<&[u16]>) -> Result<(), FontError> {
        if let Some(text) = text {
            if text.len() > self.characters.len() {
                self.characters.resize(text.len(), 0);
            }
            self.characters[..text.len()].copy_from_slice(text);
            if text.len() < self.characters.len() {
                self.characters[text.len()] = 0;
            }
            self.end_index = text.len();
        } else {
            *self
                .characters
                .first_mut()
                .ok_or(FontError::EmptyTextBuffer)? = 0;
            self.end_index = 0;
        }
        self.cache.dirty = true;
        Ok(())
    }
    pub fn geometry(&mut self, font: &GeometryFont) -> Result<&TextGeometry, GeometryError> {
        self.cache.get(font, &self.characters, self.style)
    }
}
