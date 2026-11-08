//! PolygonHead Character.FromGlyph / Text.UpdateVertices CPU reconstruction.
use crate::{
    bitmap_font::{FontError, FontMetrics},
    text_box::TypingState,
};
use magicka_content::{
    assets::{Asset, BitmapFont, Glyph},
    xna::XnaObject,
};
use std::collections::BTreeMap;

/// Magicka.Defines.DIALOGUE_COLOR_DEFAULT (static constructor).
pub const DIALOGUE_COLOR_DEFAULT: [f32; 4] = [0.7037, 0.5576, 0.3202, 1.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphQuad {
    pub input: u16,
    pub vertices: [TextVertex; 4],
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}
#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub default_color: [f32; 4],
    pub formatting: bool,
    pub kerning: bool,
    pub align: TextAlign,
}
impl Default for TextStyle {
    fn default() -> Self {
        Self {
            default_color: [1.0; 4],
            formatting: true,
            kerning: true,
            align: TextAlign::Left,
        }
    }
}
impl TextStyle {
    pub fn dialogue() -> Self {
        Self {
            default_color: DIALOGUE_COLOR_DEFAULT,
            ..Self::default()
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum GeometryError {
    Font(FontError),
    MissingAtlas,
    InvalidAtlasSize,
    BufferIndex(usize),
    UnsupportedDigit(u16),
}
#[derive(Clone, Debug)]
pub struct GeometryFont {
    pub metrics: FontMetrics,
    pub atlas_size: [i32; 2],
    glyphs: BTreeMap<u16, Glyph>,
    default: u16,
    first: Option<Glyph>,
}
impl GeometryFont {
    pub fn from_content(font: &BitmapFont) -> Result<Self, GeometryError> {
        let Some(XnaObject::Asset(asset)) = &font.texture else {
            return Err(GeometryError::MissingAtlas);
        };
        let Asset::Texture2D(texture) = asset.as_ref() else {
            return Err(GeometryError::MissingAtlas);
        };
        Self::with_atlas_size(font, [texture.width, texture.height])
    }
    /// Explicit dimensions for hosts/tests that own the atlas separately.
    pub fn with_atlas_size(font: &BitmapFont, atlas_size: [i32; 2]) -> Result<Self, GeometryError> {
        if atlas_size.iter().any(|&v| v <= 0) {
            return Err(GeometryError::InvalidAtlasSize);
        }
        let metrics = FontMetrics::from_content(font).map_err(GeometryError::Font)?;
        Ok(Self {
            metrics,
            atlas_size,
            glyphs: font
                .glyphs
                .iter()
                .map(|g| (g.character as u16, g.clone()))
                .collect(),
            default: font.default_character as u16,
            first: font.glyphs.first().cloned(),
        })
    }
    pub fn glyph(&self, c: u16) -> Result<&Glyph, GeometryError> {
        self.glyphs
            .get(&c)
            .or_else(|| self.glyphs.get(&self.default))
            .or(self.first.as_ref())
            .ok_or(GeometryError::Font(FontError::EmptyFont))
    }
    fn quad(
        &self,
        input: u16,
        color: [f32; 4],
        offset: &mut [f32; 2],
        left: &mut u16,
        kern: bool,
    ) -> Result<GlyphQuad, GeometryError> {
        let glyph = self.glyph(input)?;
        if kern {
            offset[0] += self.metrics.calc_kern(*left, input) as f32;
        }
        offset[0] += self.metrics.spacing as f32;
        let uv0 = [
            glyph.origin[0] as f32 / self.atlas_size[0] as f32,
            glyph.origin[1] as f32 / self.atlas_size[1] as f32,
        ];
        let uv1 = [
            uv0[0] + glyph.size[0] as f32 / self.atlas_size[0] as f32,
            uv0[1] + glyph.size[1] as f32 / self.atlas_size[1] as f32,
        ];
        let left_x = glyph.left_side_bearing as f32;
        let right_x = glyph.left_side_bearing.wrapping_add(glyph.size[0]) as f32;
        // BitmapFont.Read creates reusable Characters with Vector4.One. ForceWhite
        // glyphs skip Text's color assignment entirely, including alpha.
        let color = if glyph.force_white { [1.0; 4] } else { color };
        let positions = [
            [left_x, 0.0],
            [right_x, 0.0],
            [right_x, glyph.size[1] as f32],
            [left_x, glyph.size[1] as f32],
        ];
        let uvs = [uv0, [uv1[0], uv0[1]], uv1, [uv0[0], uv1[1]]];
        let vertices = std::array::from_fn(|i| TextVertex {
            position: [positions[i][0] + offset[0], positions[i][1] + offset[1]],
            uv: uvs[i],
            color,
        });
        offset[0] += glyph.advance_width as f32;
        *left = input;
        Ok(GlyphQuad { input, vertices })
    }
}
fn at(text: &[u16], i: usize) -> Result<u16, GeometryError> {
    text.get(i).copied().ok_or(GeometryError::BufferIndex(i))
}
fn digit(c: u16) -> Result<bool, GeometryError> {
    if c > 127 {
        return Err(GeometryError::UnsupportedDigit(c));
    }
    Ok((48..=57).contains(&c))
}
/// Native permissive ParseFloat: delimiter and index mutation are part of the API.
/// It does not trim, check comma delimiters, or accept exponents.
pub fn parse_float(text: &[u16], index: &mut usize) -> Result<f32, GeometryError> {
    let mut positive = true;
    match at(text, *index)? {
        45 => {
            positive = false;
            *index += 1;
        }
        43 => *index += 1,
        _ => {}
    }
    let (mut value, mut factor) = (0.0f32, 1.0f32);
    while *index < text.len() {
        let c = at(text, *index)?;
        if c == 46 {
            *index += 1;
        }
        if !digit(c)? {
            break;
        }
        value *= 10.0;
        value += (c - 48) as f32;
        *index += 1;
    }
    while *index < text.len() {
        let c = at(text, *index)?;
        if !digit(c)? {
            break;
        }
        factor *= 0.1;
        value += (c - 48) as f32 * factor;
        *index += 1;
    }
    Ok(if positive { value } else { 0.0 - value })
}
fn align_line(quads: &mut [GlyphQuad], width: f32, align: TextAlign) {
    let shift = match align {
        TextAlign::Left => return,
        TextAlign::Center => (width * -0.5).floor(),
        TextAlign::Right => (-width).floor(),
    };
    for quad in quads {
        for vertex in &mut quad.vertices {
            vertex.position[0] += shift;
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextGeometry {
    pub quads: Vec<GlyphQuad>,
}
impl TextGeometry {
    pub fn primitive_count(&self) -> i32 {
        (self.quads.len() as i32).wrapping_mul(2)
    }
    /// Original Text's six-index pattern and unchecked ushort casts. At >16,384
    /// glyphs indices wrap; an adapter must not silently claim native parity while
    /// replacing them with unrestricted indices.
    pub fn indices(&self) -> Vec<u16> {
        self.quads
            .iter()
            .enumerate()
            .flat_map(|(i, _)| {
                let b = i * 4;
                [
                    b as u16,
                    (b + 1) as u16,
                    (b + 2) as u16,
                    (b + 2) as u16,
                    (b + 3) as u16,
                    b as u16,
                ]
            })
            .collect()
    }
    pub fn build(
        font: &GeometryFont,
        text: &[u16],
        style: TextStyle,
    ) -> Result<Self, GeometryError> {
        let mut quads = vec![];
        let (
            mut offset,
            mut color,
            mut tag,
            mut color_index,
            mut restore_index,
            mut left,
            mut line_start,
        ) = ([0.0; 2], style.default_color, false, 0, 0, 0, 0);
        let mut i = 0;
        while i < text.len() {
            let c = text[i];
            if tag {
                match c {
                    91 => {
                        quads.push(font.quad(c, color, &mut offset, &mut left, style.kerning)?);
                        tag = false;
                        color_index = 0;
                    }
                    93 => {
                        tag = false;
                        color_index = 0;
                    }
                    _ => {}
                }
                if c == [99, 61][color_index] {
                    color_index += 1;
                    if color_index == 2 {
                        color_index = 0;
                        for component in &mut color[..3] {
                            i += 1;
                            *component = parse_float(text, &mut i)?;
                        }
                        if at(text, i)? == 44 {
                            i += 1;
                            color[3] = parse_float(text, &mut i)?;
                        } else {
                            color[3] = 1.0;
                        }
                        i -= 1;
                    }
                }
                if c == [47, 99][restore_index] {
                    restore_index += 1;
                    if restore_index == 2 {
                        restore_index = 0;
                        color = style.default_color;
                    }
                }
            } else {
                if c == 0 {
                    break;
                }
                if c == 91 && style.formatting {
                    tag = true;
                    color_index = 0;
                } else if c == 10 {
                    align_line(&mut quads[line_start..], offset[0], style.align);
                    line_start = quads.len();
                    offset[0] = 0.0;
                    offset[1] += font.metrics.line_height as f32;
                } else {
                    quads.push(font.quad(c, color, &mut offset, &mut left, style.kerning)?);
                }
            }
            i += 1;
        }
        align_line(&mut quads[line_start..], offset[0], style.align);
        Ok(Self { quads })
    }
    /// Host draw phase: publish only after successful CPU construction. Native GPU
    /// upload errors remain a renderer concern; failed construction keeps typing
    /// dirty and its previous primitive count intact.
    pub fn rebuild_typing(
        font: &GeometryFont,
        typing: &mut TypingState,
        style: TextStyle,
    ) -> Result<Self, GeometryError> {
        let geometry = Self::build(font, &typing.characters, style)?;
        typing.publish_primitives(geometry.primitive_count());
        Ok(geometry)
    }
}

/// Copy every glyph cell of an RGBA atlas into a new atlas with `gutter`
/// transparent texels around each cell (shelf packing in glyph order) and
/// return the font with updated glyph origins.
///
/// DIVERGENCE (display fix): the retail menu font atlases pack glyph rows
/// with no gap, so bilinear sampling of a quad at a fractional pixel
/// position (scrolling credits, the native +0.5 title offsets) reads up to
/// half a texel of the next row. Native samples the same way; padding keeps
/// glyph pixels and quads identical while edges fade to transparency.
pub fn padded_atlas(
    font: &magicka_content::assets::BitmapFont,
    rgba: &[u8],
    size: [i32; 2],
    gutter: i32,
) -> (magicka_content::assets::BitmapFont, Vec<u8>, [i32; 2]) {
    let width = size[0].max(1);
    let mut out_font = font.clone();
    // Shelf layout.
    let (mut x, mut y, mut shelf) = (gutter, gutter, 0);
    let mut placed = Vec::with_capacity(font.glyphs.len());
    for g in &font.glyphs {
        let [w, h] = g.size;
        if x + w + gutter > width && x > gutter {
            x = gutter;
            y += shelf + gutter;
            shelf = 0;
        }
        placed.push([x, y]);
        x += w + gutter;
        shelf = shelf.max(h);
    }
    let height = y + shelf + gutter;
    let new_width = width.max(
        font.glyphs
            .iter()
            .map(|g| g.size[0] + 2 * gutter)
            .max()
            .unwrap_or(0),
    );
    let mut pixels = vec![0u8; (new_width * height * 4) as usize];
    for ((g, out), [nx, ny]) in font.glyphs.iter().zip(&mut out_font.glyphs).zip(placed) {
        for row in 0..g.size[1] {
            for col in 0..g.size[0] {
                let (sx, sy) = (g.origin[0] + col, g.origin[1] + row);
                if sx < 0 || sy < 0 || sx >= size[0] || sy >= size[1] {
                    continue;
                }
                let src = ((sy * size[0] + sx) * 4) as usize;
                let dst = (((ny + row) * new_width + nx + col) * 4) as usize;
                pixels[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
            }
        }
        out.origin = [nx, ny];
    }
    (out_font, pixels, [new_width, height])
}

#[cfg(test)]
mod padded_atlas_tests {
    use super::padded_atlas;
    use magicka_content::assets::{BitmapFont, Glyph};

    #[test]
    fn touching_rows_get_transparent_gutters_and_keep_their_pixels() {
        // Two 2×2 glyphs stacked with no gap in a 2×4 atlas.
        let glyph = |c, y| Glyph {
            character: c,
            origin: [0, y],
            size: [2, 2],
            advance_width: 2,
            left_side_bearing: 0,
            force_white: false,
        };
        let font = BitmapFont {
            line_height: 2,
            base_line: 2,
            default_character: 'a',
            texture: None,
            glyphs: vec![glyph('a', 0), glyph('b', 2)],
            kerning: vec![],
        };
        let rgba: Vec<u8> = (0..8).flat_map(|i| [i as u8, 0, 0, 255]).collect();
        let (out, pixels, size) = padded_atlas(&font, &rgba, [2, 4], 1);
        let at = |x: i32, y: i32| pixels[((y * size[0] + x) * 4) as usize..][..4].to_vec();
        for (g, src_y) in out.glyphs.iter().zip([0, 2]) {
            for r in 0..2 {
                for c in 0..2 {
                    let v = ((src_y + r) * 2 + c) as u8;
                    assert_eq!(at(g.origin[0] + c, g.origin[1] + r), vec![v, 0, 0, 255]);
                }
            }
            // Every texel around the cell is transparent.
            for c in -1..3 {
                assert_eq!(at(g.origin[0] + c, g.origin[1] - 1)[3], 0);
                assert_eq!(at(g.origin[0] + c, g.origin[1] + 2)[3], 0);
            }
        }
    }
}
