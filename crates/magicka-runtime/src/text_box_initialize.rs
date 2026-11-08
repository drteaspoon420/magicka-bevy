//! Native TextBox Initialize overloads, owner-name sizing and Draw text origins.
use crate::{
    bitmap_font::{FontError, FontMetrics},
    localization::{LocalizationError, LocalizationTable},
    text_box::TextBoxState,
};

#[derive(Clone, Copy, Debug)]
pub enum TextBoxLocation {
    Screen {
        position: [f32; 2],
        name: i32,
    },
    World {
        position: [f32; 3],
        name: i32,
    },
    /// Native Entity wrapper takes its UniqueID as the name key; null uses zero.
    Entity(Option<i32>),
}
#[derive(Clone, Copy, Debug)]
pub struct TextBoxInit<'a> {
    pub scene: Option<i32>,
    pub location: TextBoxLocation,
    pub text: Option<&'a str>,
    pub minimum_size: [f32; 2],
    pub force_on_screen: bool,
    pub ttl: f32,
}
pub struct TextBoxFonts<'a> {
    /// Fixed constructor font Maiandra14; public Initialize's iFont is unused.
    pub body: &'a FontMetrics,
    /// Fixed constructor font Maiandra16.
    pub owner_name: &'a FontMetrics,
}
pub trait TextBoxInitHost {
    type Error;
    fn load_background(&mut self, path: &str) -> Result<(), Self::Error>;
}
pub trait TextBoxInitTarget {
    fn state_mut(&mut self) -> &mut TextBoxState;
    fn initialize_target<H: TextBoxInitHost>(
        &mut self,
        init: TextBoxInit<'_>,
        strings: &LocalizationTable,
        fonts: TextBoxFonts<'_>,
        host: &mut H,
        budget: usize,
    ) -> Result<(), TextBoxInitError<H::Error>>;
}
#[derive(Debug, PartialEq, Eq)]
pub enum TextBoxInitError<E> {
    MissingScene,
    Localization(LocalizationError),
    Font(FontError),
    Host(E),
    UnsupportedBoxSize,
}
#[derive(Clone, Debug)]
pub struct TextBoxWidget {
    pub state: TextBoxState,
    pub scene: Option<i32>,
    pub show_name: bool,
    pub owner_name: Option<String>,
    pub name_characters: Vec<u16>,
}
impl Default for TextBoxWidget {
    fn default() -> Self {
        Self {
            state: TextBoxState::default(),
            scene: None,
            show_name: false,
            owner_name: None,
            name_characters: vec![0; 64],
        }
    }
}
fn box_integer<E>(value: f32) -> Result<i32, TextBoxInitError<E>> {
    // Out-of-range/NaN CLR conv.i4 behavior is not silently replaced by Rust's
    // saturating float cast. Ordinary finite dimensions truncate toward zero.
    if !value.is_finite() || !(-2147483648.0..2147483648.0).contains(&value) {
        return Err(TextBoxInitError::UnsupportedBoxSize);
    }
    Ok(value as i32)
}
impl TextBoxWidget {
    pub fn initialize<H: TextBoxInitHost>(
        &mut self,
        init: TextBoxInit<'_>,
        strings: &LocalizationTable,
        fonts: TextBoxFonts<'_>,
        host: &mut H,
        substitution_budget: usize,
    ) -> Result<(), TextBoxInitError<H::Error>> {
        let scene = init.scene.ok_or(TextBoxInitError::MissingScene)?;
        let name = match init.location {
            TextBoxLocation::Screen { position, name } => {
                self.state.owner = None;
                self.state.world_position = [position[0], position[1], 0.0];
                self.state.screen_position = true;
                name
            }
            TextBoxLocation::World { position, name } => {
                self.state.owner = None;
                self.state.world_position = position;
                self.state.screen_position = false;
                name
            }
            TextBoxLocation::Entity(owner) => {
                self.state.owner = owner;
                self.state.screen_position = false;
                owner.unwrap_or(0)
            }
        };
        if !self.state.grow {
            self.state.scale = 0.0;
        }
        self.state.grow = true;
        self.state.force_on_screen = init.force_on_screen;
        self.state.ttl = init.ttl;
        self.state.automatic_advance = init.ttl > 0.0;
        self.state.text = init.text.map(str::to_owned);
        if let Some(text) = init.text {
            let parsed = strings
                .parse_references(text, substitution_budget)
                .map_err(TextBoxInitError::Localization)?;
            self.state.typing.set_text(Some(&parsed));
            self.state.text = Some(parsed);
        }
        self.scene = Some(scene);
        self.state.scene_initialized = true;
        host.load_background("UI/HUD/Dialog_Say")
            .map_err(TextBoxInitError::Host)?;
        let lookup = strings.try_get_string(name);
        self.show_name = lookup.found;
        let mut minimum = init.minimum_size;
        if self.show_name {
            self.owner_name = lookup.value.map(str::to_owned);
            let chars: Vec<_> = lookup.value.unwrap_or_default().encode_utf16().collect();
            if chars.len() > self.name_characters.len() {
                self.name_characters = vec![0; chars.len()];
            }
            self.name_characters[..chars.len()].copy_from_slice(&chars);
            if chars.len() < self.name_characters.len() {
                self.name_characters[chars.len()] = 0;
            }
            let size = fonts
                .owner_name
                .measure_utf16(&self.name_characters, true, i32::MAX)
                .map_err(TextBoxInitError::Font)?;
            minimum[0] = (minimum[0] + 20.0).max(size[0]);
            minimum[1] += size[1];
        }
        if self.state.text.as_ref().is_some_and(|s| !s.is_empty()) {
            let size = fonts
                .body
                .measure_utf16(&self.state.typing.characters, true, i32::MAX)
                .map_err(TextBoxInitError::Font)?;
            self.state.size[0] = box_integer(size[0].max(minimum[0]))?;
            self.state.size[1] = box_integer(size[1].max(minimum[1]))?;
        }
        Ok(())
    }
    /// Native render coordinates, Y down. Body uses a 20-pixel indent and the
    /// owner-name font's LineHeight when a name is shown.
    pub fn draw_origins(
        &self,
        center: [f32; 2],
        scale: f32,
        name_line_height: i32,
    ) -> TextBoxDrawOrigins {
        let half = [
            self.state.size[0] as f32 * 0.5,
            self.state.size[1] as f32 * 0.5,
        ];
        let corner = [center[0] - half[0] * scale, center[1] - half[1] * scale];
        TextBoxDrawOrigins {
            owner_name: self.show_name.then_some(corner),
            body: if self.show_name {
                [
                    center[0] - (half[0] - 20.0) * scale,
                    center[1] - (half[1] - name_line_height as f32) * scale,
                ]
            } else {
                corner
            },
        }
    }
}
impl TextBoxInitTarget for TextBoxWidget {
    fn state_mut(&mut self) -> &mut TextBoxState {
        &mut self.state
    }
    fn initialize_target<H: TextBoxInitHost>(
        &mut self,
        init: TextBoxInit<'_>,
        strings: &LocalizationTable,
        fonts: TextBoxFonts<'_>,
        host: &mut H,
        budget: usize,
    ) -> Result<(), TextBoxInitError<H::Error>> {
        self.initialize(init, strings, fonts, host, budget)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextBoxDrawOrigins {
    pub owner_name: Option<[f32; 2]>,
    pub body: [f32; 2],
}
