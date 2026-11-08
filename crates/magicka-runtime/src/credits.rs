//! Credits (RollCredits): `Content/Data/<file>` (default Credits.xml) lists
//! localized credit keys with font, colour and padding; Initialize wraps
//! each to 400 px and measures its lines. Start places the text at half the
//! screen height; Update fades the sky map to black over 2 s, fades the black
//! band in at 2/s, then scrolls Speed⁻¹ half-screens per second and finally
//! fades out into Endgame(EndOffGame) (research/credits.md).
use crate::bitmap_font::FontMetrics;
use crate::localization::LocalizationTable;
use crate::scene_loading::custom_hash;

/// MagickaFont in enum order with FontManager's file and custom spacing.
pub const FONTS: &[(&str, &str, i32)] = &[
    ("Maiandra14", "Languages/eng/Font/Maiandra14", 0),
    ("Maiandra16", "Languages/eng/Font/Maiandra16", 1),
    ("Maiandra18", "Languages/eng/Font/Maiandra18", 1),
    ("Stonecross28", "Languages/eng/Font/Stonecross28", 1),
    ("Stonecross36", "Languages/eng/Font/Stonecross36", 1),
    ("Stonecross50", "Languages/eng/Font/Stonecross50", 1),
    ("MenuDefault", "Languages/eng/Font/MenuDefault", 1),
    ("MenuOption", "Languages/eng/Font/MenuOption", 0),
    ("MenuTitle", "Languages/eng/Font/MenuTitle", 0),
    ("VersusText", "Languages/eng/Font/Versustext", 1),
    ("PDX_UI", "Languages/eng/Font/PDXUIFont", 1),
    ("PDX_UI_Small", "Languages/eng/Font/PDXUISmallFont", 1),
    (
        "PDX_UI_Small_Bold",
        "Languages/eng/Font/PDXUISmallHeaderFont",
        0,
    ),
    ("PDX_UI_Bold", "Languages/eng/Font/PDXUIBoldFont", 1),
    ("PDX_Points", "Languages/eng/Font/PDXPointsFont", 0),
    ("PDX_Edit", "Languages/eng/Font/PDXEditFont", 0),
];
/// MagickaFont.Maiandra18, the default credit font.
pub const DEFAULT_FONT: usize = 2;
/// Credits.Initialize's wrap width.
pub const LINE_WIDTH: i32 = 400;

#[derive(Clone, Debug, PartialEq)]
pub struct CreditEntry {
    /// Hash of the lowercased inner text (a `#key`).
    pub id: i32,
    pub font: usize,
    pub color: [f32; 3],
    pub padding: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreditsFile {
    /// SCROLL_SPEED (default 16).
    pub speed: f32,
    pub entries: Vec<CreditEntry>,
}

#[derive(Debug, PartialEq)]
pub enum CreditsError {
    Xml(String),
    MissingCredits,
    Attribute(String),
}

impl CreditsFile {
    /// The first <credits> element; <credit> children in order.
    pub fn parse(bytes: &[u8]) -> Result<Self, CreditsError> {
        let root =
            crate::scene_script::xml(bytes).map_err(|e| CreditsError::Xml(format!("{e:?}")))?;
        let credits = if root.name.eq_ignore_ascii_case("credits") {
            &root
        } else {
            root.children
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case("credits"))
                .ok_or(CreditsError::MissingCredits)?
        };
        let bad = |k: &str| CreditsError::Attribute(k.into());
        let mut speed = 16.0;
        for (k, v) in &credits.attributes {
            if k.eq_ignore_ascii_case("speed") {
                speed = v.trim().parse().map_err(|_| bad(k))?;
            }
        }
        let mut entries = Vec::new();
        for c in credits
            .children
            .iter()
            .filter(|c| c.name.eq_ignore_ascii_case("credit"))
        {
            let mut e = CreditEntry {
                id: custom_hash(&c.inner_text.to_lowercase()),
                font: DEFAULT_FONT,
                color: [1.0; 3],
                padding: 0.0,
            };
            for (k, v) in &c.attributes {
                match k.to_ascii_lowercase().as_str() {
                    "color" => {
                        let v = v.replace(' ', "");
                        let parts: Vec<&str> = v.split(',').collect();
                        for i in 0..3 {
                            e.color[i] = parts
                                .get(i)
                                .and_then(|p| p.parse().ok())
                                .ok_or_else(|| bad(k))?;
                        }
                    }
                    "font" => {
                        e.font = FONTS
                            .iter()
                            .position(|(name, _, _)| name.eq_ignore_ascii_case(v.trim()))
                            .ok_or_else(|| bad(k))?
                    }
                    "padding" => e.padding = v.trim().parse().map_err(|_| bad(k))?,
                    _ => {}
                }
            }
            entries.push(e);
        }
        Ok(Self { speed, entries })
    }
}

/// One wrapped line (Credits' Text + ElementData).
#[derive(Clone, Debug, PartialEq)]
pub struct CreditLine {
    pub text: String,
    pub font: usize,
    pub color: [f32; 3],
    pub height: f32,
    /// The entry's padding on its first line only.
    pub padding: f32,
    pub alpha: f32,
    pub position: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Credits {
    pub lines: Vec<CreditLine>,
    pub total_length: f32,
    div_scroll: f32,
    pub started: bool,
    pub fade_out: bool,
    pub fading_out: bool,
    /// mSkymapFadeInTime (0..1 over 2 s): sky map colour → black.
    pub sky_fade: f32,
    /// mFadeInAlpha: the black band and text group alpha.
    pub fade_in_alpha: f32,
    pub position: f32,
    pub start_index: usize,
    pub end_index: usize,
    /// RenderData.StartIndex/EndIndex: the drawn line range, only updated
    /// while scrolling (the band fades in over an empty range at first).
    pub render_range: (usize, usize),
}

/// What the session must do after an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreditsEvent {
    /// First fade-out update: PlayState.Endgame(EndOffGame, no freeze, not
    /// phony) and a 1 s fade to black.
    BeginFadeOut,
}

impl Credits {
    /// Credits.Initialize: wrap and measure each entry's localized text.
    /// `font(i)` supplies MagickaFont i with its FontManager spacing.
    pub fn initialize(
        file: &CreditsFile,
        strings: &LocalizationTable,
        font: impl Fn(usize) -> Option<FontMetrics>,
    ) -> Result<Self, String> {
        let mut lines = Vec::new();
        let mut total_length = 0.0;
        for (m, e) in file.entries.iter().enumerate() {
            let metrics = font(e.font).ok_or_else(|| format!("font {}", FONTS[e.font].0))?;
            let text = strings.get_string(e.id).unwrap_or_default();
            let wrapped = metrics
                .wrap(&text, LINE_WIDTH, true)
                .map_err(|err| format!("wrap: {err:?}"))?;
            for (n, line) in wrapped.text.split('\n').enumerate() {
                let height = metrics
                    .measure(line, true)
                    .map_err(|err| format!("measure: {err:?}"))?[1];
                lines.push(CreditLine {
                    text: line.to_owned(),
                    font: e.font,
                    color: e.color,
                    height,
                    padding: if n == 0 { e.padding } else { 0.0 },
                    alpha: 0.0,
                    position: 0.0,
                });
                // Native indexes mTextDatas with the entry index m, not
                // the line just added, so wrapped entries skew the total.
                let d = &lines[m];
                total_length += d.height + d.padding;
            }
        }
        Ok(Self {
            lines,
            total_length,
            div_scroll: 1.0 / file.speed,
            ..Self::default()
        })
    }

    /// Credits.Start.
    pub fn start(&mut self, screen_height: f32) {
        self.fade_in_alpha = 0.0;
        self.position = screen_height * 0.5;
        self.start_index = 0;
        let mut sum = 0.0;
        for (i, l) in self.lines.iter().enumerate() {
            sum += l.height + l.padding;
            if sum > self.position {
                self.end_index = i;
                break;
            }
        }
        self.fade_out = false;
        self.fading_out = false;
        self.started = true;
        self.sky_fade = 0.0;
    }

    /// Credits.Update (before fade-out completes).
    pub fn update(&mut self, dt: f32, screen_height: f32) -> Option<CreditsEvent> {
        if !self.started {
            return None;
        }
        if self.fade_out {
            if !self.fading_out {
                self.fading_out = true;
                return Some(CreditsEvent::BeginFadeOut);
            }
            return None;
        }
        if self.sky_fade < 1.0 {
            self.sky_fade = (self.sky_fade + dt * 0.5).min(1.0);
            return None;
        }
        self.fade_in_alpha = (self.fade_in_alpha + dt * 2.0).min(1.0);
        if self.fade_in_alpha >= 1.0 {
            let half = screen_height * 0.5;
            self.position -= dt * self.div_scroll * half;
            if self.position <= -self.total_length {
                self.fade_out = true;
            }
            let mut y = self.position;
            self.start_index = 0;
            self.end_index = self.lines.len();
            for i in 0..self.lines.len() {
                let l = &mut self.lines[i];
                y += l.padding;
                l.position = y;
                let top = (y / l.height).min(1.0);
                let bottom = ((half - (y + l.height)) / l.height).clamp(0.0, 1.0);
                l.alpha = top * bottom;
                // Native quirk: StartIndex starts at 0 so `i < StartIndex`
                // never holds; only EndIndex narrows.
                if top <= 0.0 && i < self.start_index {
                    self.start_index = i;
                } else if bottom <= 0.0 && i < self.end_index {
                    self.end_index = i;
                }
                y += l.height;
            }
            self.render_range = (self.start_index, self.end_index);
        }
        None
    }

    /// OnTransitionEnd after the fade-out: the credits stop.
    pub fn finish(&mut self) {
        self.started = false;
        self.fade_in_alpha = 0.0;
        self.sky_fade = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credits(heights: &[(f32, f32)], speed: f32) -> Credits {
        Credits {
            lines: heights
                .iter()
                .map(|&(height, padding)| CreditLine {
                    text: "x".into(),
                    font: 0,
                    color: [1.0; 3],
                    height,
                    padding,
                    alpha: 0.0,
                    position: 0.0,
                })
                .collect(),
            total_length: heights.iter().map(|(h, p)| h + p).sum(),
            div_scroll: 1.0 / speed,
            ..Credits::default()
        }
    }

    #[test]
    fn parses_speed_fonts_colours_and_padding() {
        let f = CreditsFile::parse(
            br#"<Credits speed="8"><Credit padding="30" font="MenuOption" color="1, 0.82,0">#UI_A</Credit><Credit>#b</Credit></Credits>"#,
        )
        .unwrap();
        assert_eq!(f.speed, 8.0);
        assert_eq!(f.entries[0].font, 7);
        assert_eq!(f.entries[0].color, [1.0, 0.82, 0.0]);
        assert_eq!(f.entries[0].padding, 30.0);
        assert_eq!(f.entries[0].id, custom_hash("#ui_a"));
        assert_eq!(f.entries[1].font, DEFAULT_FONT);
    }

    #[test]
    fn sky_fades_for_two_seconds_then_band_then_scroll() {
        let mut c = credits(&[(20.0, 0.0); 3], 16.0);
        c.start(720.0);
        assert_eq!(c.position, 360.0);
        for _ in 0..8 {
            assert_eq!(c.update(0.25, 720.0), None);
        }
        assert_eq!(c.sky_fade, 1.0);
        assert_eq!(c.fade_in_alpha, 0.0);
        c.update(0.25, 720.0);
        assert_eq!(c.fade_in_alpha, 0.5);
        assert_eq!(c.position, 360.0, "no scroll before the band is opaque");
        c.update(0.25, 720.0);
        // Opaque: scrolls 0.25 · (1/16) · 360 = 5.625 px.
        assert_eq!(c.position, 360.0 - 5.625);
        assert_eq!(c.lines[0].position, c.position);
        assert_eq!(c.lines[1].position, c.position + 20.0);
    }

    #[test]
    fn scrolling_past_the_total_length_fades_out_once() {
        let mut c = credits(&[(20.0, 0.0)], 1.0);
        c.start(100.0);
        c.sky_fade = 1.0;
        c.fade_in_alpha = 1.0;
        // 50 px per second at speed 1 and height 100: 70 px take 1.4 s.
        let mut events = vec![];
        for _ in 0..20 {
            events.extend(c.update(0.1, 100.0));
        }
        assert_eq!(events, vec![CreditsEvent::BeginFadeOut]);
        assert!(c.fade_out && c.fading_out);
    }

    #[test]
    fn lines_fade_at_the_top_and_at_half_screen() {
        let mut c = credits(&[(10.0, 0.0), (10.0, 0.0)], 16.0);
        c.start(100.0);
        c.sky_fade = 1.0;
        c.fade_in_alpha = 1.0;
        c.position = 45.0;
        c.div_scroll = 0.0;
        c.update(0.1, 100.0);
        // Line 0 at 45: fully in the top, bottom (50 − 55)/10 → 0.
        assert_eq!(c.lines[0].alpha, 0.0);
        assert_eq!(c.end_index, 0);
        assert_eq!(c.render_range, (0, 0));
        c.position = 5.0;
        c.update(0.1, 100.0);
        assert_eq!(c.lines[0].alpha, 0.5, "half way out of the top");
        assert_eq!(c.lines[1].alpha, 1.0);
    }
}
