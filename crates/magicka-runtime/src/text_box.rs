//! Recovered TypingText and TextBox frame phases. Font/geometry construction is
//! owned by the host; primitive count is published by its draw/geometry phase.
#[derive(Debug, PartialEq, Eq)]
pub enum TextBoxError {
    BufferIndex(i32),
    PauseSyntax,
    UnsupportedPauseCharacter(u16),
    UnsupportedWhitespace(u16),
    InvalidChannel(usize),
    MissingOwner(i32),
}
#[derive(Clone, Debug)]
pub struct TypingState {
    pub characters: Vec<u16>,
    pub primitive_count: i32,
    pub next_character: f32,
    pub visible_characters: i32,
    pub character_index: i32,
    pub type_speed: f32,
    pub dirty: bool,
}
impl Default for TypingState {
    fn default() -> Self {
        Self::new(1024, 40.0)
    }
}
impl TypingState {
    pub fn new(capacity: usize, type_speed: f32) -> Self {
        Self {
            characters: vec![0; capacity],
            primitive_count: 0,
            next_character: 0.0,
            visible_characters: 0,
            character_index: -1,
            type_speed,
            dirty: false,
        }
    }
    pub fn set_text(&mut self, text: Option<&str>) {
        self.next_character = 0.0;
        self.character_index = -1;
        self.visible_characters = 0;
        let units: Vec<_> = text.unwrap_or_default().encode_utf16().collect();
        if units.len() > self.characters.len() {
            self.characters = vec![0; units.len()];
        }
        self.characters[..units.len()].copy_from_slice(&units);
        if units.len() < self.characters.len() {
            self.characters[units.len()] = 0;
        }
        self.dirty = true;
        // SetText/Reinitialise do not reset the previous primitive count.
    }
    pub fn finished(&self) -> bool {
        self.visible_characters == self.primitive_count / 2
    }
    pub fn finish(&mut self) {
        self.character_index = self.characters.len() as i32;
        self.visible_characters = self.primitive_count / 2;
    }
    /// Publish actual geometry output. Do not eagerly recompute in SetText:
    /// TypingText.Draw updates dirty vertices after PreRenderUpdate.
    pub fn publish_primitives(&mut self, count: i32) {
        self.primitive_count = count;
        self.dirty = false;
    }
    fn current(&self) -> Result<u16, TextBoxError> {
        usize::try_from(self.character_index)
            .ok()
            .and_then(|i| self.characters.get(i))
            .copied()
            .ok_or(TextBoxError::BufferIndex(self.character_index))
    }
    fn next(&mut self) -> Result<u16, TextBoxError> {
        self.character_index = self.character_index.wrapping_add(1);
        self.current()
    }
    fn digit(c: u16) -> Result<bool, TextBoxError> {
        if c > 127 && !matches!(c, 93) {
            return Err(TextBoxError::UnsupportedPauseCharacter(c));
        }
        Ok((48..=57).contains(&c))
    }
    fn whitespace(c: u16) -> Result<bool, TextBoxError> {
        // CLR's historical handling of Mongolian vowel separator is unresolved.
        if c == 0x180e {
            return Err(TextBoxError::UnsupportedWhitespace(c));
        }
        Ok(
            matches!(c, 9..=13 | 32 | 0x85 | 0xa0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000),
        )
    }
    pub fn update(&mut self, dt: f32) -> Result<(), TextBoxError> {
        self.next_character -= dt;
        while self.next_character < 0.0 && self.visible_characters < self.primitive_count / 2 {
            let mut c = self.next()?;
            if c == 91 {
                c = self.next()?;
                if c == 91 {
                    self.visible_characters = self.visible_characters.wrapping_add(1);
                    self.next_character += 1.0 / self.type_speed;
                } else if matches!(c, 80 | 112) {
                    if self.next()? != 61 {
                        return Err(TextBoxError::PauseSyntax);
                    }
                    let mut delay = 0.0f32;
                    loop {
                        c = self.next()?;
                        if !Self::digit(c)? {
                            break;
                        }
                        delay *= 10.0;
                        delay += (c - 48) as f32;
                    }
                    if c == 46 {
                        let mut factor = 0.1f32;
                        loop {
                            c = self.next()?;
                            if !Self::digit(c)? {
                                break;
                            }
                            delay += (c - 48) as f32 * factor;
                            factor *= 0.1;
                        }
                    }
                    if c != 93 {
                        return Err(TextBoxError::PauseSyntax);
                    }
                    self.next_character += delay;
                } else {
                    while c != 93 {
                        c = self.next()?;
                    }
                }
            } else {
                let punctuation = matches!(c, 33 | 44 | 46 | 58 | 59 | 63 | 0x2026 | 0x203c);
                let after = self.character_index.wrapping_add(1);
                let whitespace = if punctuation {
                    match usize::try_from(after)
                        .ok()
                        .and_then(|i| self.characters.get(i))
                    {
                        Some(&c) => Self::whitespace(c)?,
                        None => false,
                    }
                } else {
                    false
                };
                if punctuation && whitespace {
                    self.visible_characters = self.visible_characters.wrapping_add(1);
                    self.next_character += 0.25;
                } else if c != 10 {
                    self.visible_characters = self.visible_characters.wrapping_add(1);
                    self.next_character += 1.0 / self.type_speed;
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextBoxOwner {
    pub position: [f32; 3],
    pub capsule: Option<(f32, f32)>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextBoxRenderState {
    pub world_position: [f32; 3],
    pub screen_position: bool,
    pub force_on_screen: bool,
    pub scale: f32,
    pub size: [i32; 2],
}
#[derive(Clone, Copy, Debug)]
pub struct TextBoxUpdate {
    channel: usize,
    pub advance: bool,
}
#[derive(Clone, Debug, Default)]
pub struct TextBoxState {
    pub scene_initialized: bool,
    pub text: Option<String>,
    pub owner: Option<i32>,
    pub world_position: [f32; 3],
    pub screen_position: bool,
    pub force_on_screen: bool,
    pub size: [i32; 2],
    pub scale: f32,
    pub grow: bool,
    pub ttl: f32,
    pub automatic_advance: bool,
    pub typing: TypingState,
    pub channels: [TextBoxRenderState; 3],
}
impl TextBoxState {
    /// The common Initialize state prefix; host supplies resolved text and box
    /// dimensions after font/name/asset operations. Null leaves old typing intact.
    pub fn initialize_state(&mut self, text: Option<&str>, size: [i32; 2], ttl: f32) {
        if !self.grow {
            self.scale = 0.0;
        }
        self.grow = true;
        self.ttl = ttl;
        self.automatic_advance = ttl > 0.0;
        self.text = text.map(str::to_owned);
        if let Some(text) = text {
            self.typing.set_text(Some(text));
        }
        self.scene_initialized = true;
        if text.is_some_and(|s| !s.is_empty()) {
            self.size = size;
        }
    }
    pub fn hide(&mut self) {
        self.grow = false;
    }
    pub fn visible(&self) -> bool {
        self.grow || self.scale > f32::from_bits(1)
    }
    pub fn animating(&self) -> bool {
        (self.grow && self.scale < 0.99)
            || (!self.grow && self.scale > f32::from_bits(1))
            || !self.typing.finished()
    }
    pub fn finish_animation(&mut self, remove_ttl_advance: bool, graphics_present: bool) {
        if remove_ttl_advance {
            self.automatic_advance = false;
        }
        self.scale = if self.grow { 1.0 } else { 0.0 };
        if graphics_present {
            self.typing.finish();
        }
    }
    /// Split around the native synchronous DialogManager.Advance callback. The
    /// host must finish this phase even when the callback hides/reinitializes us.
    pub fn begin_update(
        &mut self,
        dt: f32,
        channel: usize,
        owner: Option<TextBoxOwner>,
    ) -> Result<Option<TextBoxUpdate>, TextBoxError> {
        if !self.scene_initialized || (self.scale <= f32::from_bits(1) && !self.grow) {
            return Ok(None);
        }
        if channel >= self.channels.len() {
            return Err(TextBoxError::InvalidChannel(channel));
        }
        self.scale = if self.grow {
            (self.scale + dt * 4.0).min(1.0)
        } else {
            (self.scale - dt * 8.0).max(0.0)
        };
        let position = if let Some(id) = self.owner {
            let owner = owner.ok_or(TextBoxError::MissingOwner(id))?;
            let mut pos = owner.position;
            if let Some((length, radius)) = owner.capsule {
                pos[1] += length * 0.5 + radius;
            }
            pos
        } else {
            self.world_position
        };
        self.channels[channel].world_position = position;
        if self.typing.finished() {
            self.ttl -= dt;
        }
        Ok(Some(TextBoxUpdate {
            channel,
            advance: self.automatic_advance && self.ttl < 0.0,
        }))
    }
    pub fn finish_update(&mut self, update: TextBoxUpdate) -> TextBoxRenderState {
        let state = &mut self.channels[update.channel];
        state.screen_position = self.screen_position;
        state.force_on_screen = self.force_on_screen;
        state.scale = self.scale;
        state.size = self.size;
        *state
    }
    pub fn pre_render(
        &mut self,
        channel: usize,
        dt: f32,
        screen: [f32; 2],
        view_projection: [[f32; 4]; 4],
    ) -> Result<[f32; 2], TextBoxError> {
        let state = *self
            .channels
            .get(channel)
            .ok_or(TextBoxError::InvalidChannel(channel))?;
        if state.scale > 0.99 {
            self.typing.update(dt)?;
        }
        let mut position = if state.screen_position {
            [
                (screen[0] * 0.5 * (1.0 + state.world_position[0])).floor(),
                (screen[1] * 0.5 * (1.0 + state.world_position[1])).floor(),
            ]
        } else {
            let p = state.world_position;
            let mut projected = [0.0; 4];
            for (i, value) in projected.iter_mut().enumerate() {
                *value = p[0] * view_projection[0][i]
                    + p[1] * view_projection[1][i]
                    + p[2] * view_projection[2][i]
                    + view_projection[3][i];
            }
            [
                (projected[0] / projected[3] * 0.5 + 0.5) * screen[0]
                    - (state.size[0] as f32 * 0.5 + 64.0) / 3.0 * state.scale,
                (projected[1] / projected[3] * -0.5 + 0.5) * screen[1]
                    - (state.size[1] as f32 * 0.5 + 64.0) * state.scale,
            ]
        };
        if state.force_on_screen {
            for i in 0..2 {
                let inset = (screen[i] * 0.05).floor();
                let half = (state.size[i] as f32 + 32.0) * 0.5;
                position[i] = position[i].max(half + inset);
                position[i] = position[i].min(screen[i] - half - inset);
            }
        }
        Ok(position)
    }
}
