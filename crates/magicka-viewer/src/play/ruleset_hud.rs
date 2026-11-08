//! Ruleset presentation (research/rulesets.md): TimedObjectiveRuleset's time
//! box; SurvivalRuleset's score bar, multipliers, scrolling kill scores, wave
//! indicator and wave health bar; VersusRuleset's countdown, clock,
//! game-over and unlock texts; and the InGameMenu*Statistics sheet with its
//! offline Retry (Restart StartOfLevel) and Quit (ChallengeExit) choices.
//! Layout is computed by pure functions over the session state; the Bevy
//! side keeps a pool of text and quad meshes.
//!
//! Not yet drawn: the Survival time-multiplier pie and Versus clock pie
//! (PieEffect), the Versus per-player Score panels, and the native
//! statistics menu artwork (the sheet is laid out as plain text).
use super::lifecycle::LeaveSession;
use super::{Play, PlayAppExt};
use crate::native_text::{geometry_mesh, upload_overlay_font, OverlayFontMaterial};
use crate::play_hud::{upload_texture, HudMaterial};
use crate::text_box_visual::{background_mesh, read_asset, TextBoxMaterial};
use bevy::prelude::*;
use magicka_content::assets::Asset;
use magicka_runtime::{
    end_game::EndGameCondition,
    hud::HudQuad,
    localization::LocalizationTable,
    ruleset::{
        SurvivalRuleset, TimedObjectiveRuleset, VersusMode, VersusRuleset, TIME_ELAPSED_TEXT,
    },
    scene_loading::custom_hash,
    text_geometry::{GeometryFont, TextAlign, TextGeometry, TextStyle},
};
use std::{collections::HashMap, sync::Arc};

mod sheet;

const LAYER: f32 = 205.;

/// The MagickaFonts the rulesets draw with (FontManager spacing).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum HudFont {
    Maiandra14,
    Maiandra16,
    Maiandra18,
    MenuDefault,
    MenuOption,
    MenuTitle,
    VersusText,
}

impl HudFont {
    const ALL: [HudFont; 7] = [
        Self::Maiandra14,
        Self::Maiandra16,
        Self::Maiandra18,
        Self::MenuDefault,
        Self::MenuOption,
        Self::MenuTitle,
        Self::VersusText,
    ];
    fn file(self) -> (&'static str, i32) {
        match self {
            Self::Maiandra14 => ("Maiandra14", 0),
            Self::Maiandra16 => ("Maiandra16", 1),
            Self::Maiandra18 => ("Maiandra18", 1),
            Self::MenuDefault => ("MenuDefault", 1),
            Self::MenuOption => ("MenuOption", 0),
            Self::MenuTitle => ("MenuTitle", 0),
            Self::VersusText => ("VersusText", 1),
        }
    }
}

/// One Text.Draw: `origin` is the native draw position (alignment anchor,
/// top edge), `shadow` the DrawShadows offset and alpha.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HudLine {
    pub font: HudFont,
    pub text: String,
    pub origin: [f32; 2],
    pub align: TextAlign,
    pub color: [f32; 4],
    pub shadow: Option<([f32; 2], f32)>,
    pub scale: f32,
}

impl HudLine {
    fn new(font: HudFont, text: String, origin: [f32; 2], align: TextAlign) -> Self {
        Self {
            font,
            text,
            origin,
            align,
            color: [1.0; 4],
            shadow: None,
            scale: 1.0,
        }
    }
    fn shadowed(mut self, offset: f32) -> Self {
        self.shadow = Some(([offset, offset], 1.0));
        self
    }
}

/// Which texture a quad batch samples, and its draw layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum HudAtlas {
    /// UI/HUD/hud, below pies, portraits and texts.
    Hud,
    /// UI/HUD/hud above pies and portraits, below texts (Versus frames).
    HudOverlay,
    /// UI/HUD/hud above the texts (Survival's TOTAL_SCORE_OVERLAY).
    HudTop,
    /// UI/HUD/versus_countdown.
    Countdown,
    /// The boss/healthbar atlas used by BossHealthBar.
    Boss,
    /// A player slot's Gamer.Avatar.Portrait (Versus Score panels).
    Portrait(u8),
    /// The statistics sheets: untextured shade, UI/Menu/FadeBox, UI/Menu/Pages.
    Shade,
    FadeBox,
    Pages,
}

impl HudAtlas {
    const BATCHES: [HudAtlas; 12] = [
        Self::Shade,
        Self::FadeBox,
        Self::Pages,
        Self::Hud,
        Self::HudOverlay,
        Self::HudTop,
        Self::Countdown,
        Self::Boss,
        Self::Portrait(0),
        Self::Portrait(1),
        Self::Portrait(2),
        Self::Portrait(3),
    ];
    fn layer(self) -> f32 {
        match self {
            Self::Hud | Self::Countdown | Self::Boss => LAYER - 0.5,
            Self::Portrait(_) => LAYER - 0.35,
            Self::HudOverlay => LAYER - 0.3,
            Self::HudTop => LAYER + 0.5,
            Self::Shade => LAYER - 0.9,
            Self::FadeBox => LAYER - 0.8,
            Self::Pages => LAYER - 0.7,
        }
    }
}

/// PieEffect over UI/HUD/hud (Shaders/PieEffect, PROVEN disassembly): a
/// fan of 32 rim vertices at angle fraction f · MaxAngle and Radius, through
/// the Transform (rotation −90° mirrored, scale `scale`) at `center`; the
/// rim UVs stay on the full circle (32 texels around `uv_center`) plus
/// TextureOffset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PieFan {
    pub center: [f32; 2],
    pub scale: f32,
    pub radius: f32,
    pub max_angle: f32,
    pub uv_center: [f32; 2],
    pub uv_offset: [f32; 2],
}

impl PieFan {
    /// (screen position, texel UV before TextureOffset) for the centre and
    /// the 32 rim vertices.
    pub(crate) fn vertices(&self) -> Vec<([f32; 2], [f32; 2])> {
        let mut out = vec![(self.center, self.uv_center)];
        for i in 1..33 {
            let f = (i - 1) as f32 / 31.0;
            let a = f * self.max_angle;
            let (lx, ly) = (a.cos() * self.radius, a.sin() * self.radius);
            // Transform rows (0, −s), (−s, 0): x' = −s·ly, y' = −s·lx.
            let position = [
                self.center[0] - self.scale * ly,
                self.center[1] - self.scale * lx,
            ];
            let t = (f - 0.25) * std::f32::consts::TAU;
            let uv = [
                self.uv_center[0] - t.cos() * 32.0,
                self.uv_center[1] + t.sin() * 32.0,
            ];
            out.push((position, uv));
        }
        out
    }
}

/// One Versus Score panel's player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PanelPlayer {
    pub slot: usize,
    pub color: [f32; 3],
    pub respawn: i32,
}

/// Everything one frame draws.
#[derive(Default, Debug)]
pub(crate) struct HudFrame {
    pub lines: Vec<HudLine>,
    pub quads: Vec<(HudAtlas, HudQuad)>,
    pub pies: Vec<PieFan>,
    /// A TimedObjective text box: (texture, centre, size).
    pub text_box: Option<(String, [f32; 2], [f32; 2])>,
}

type Measure<'a> = &'a dyn Fn(HudFont, &str) -> [f32; 2];
type LineHeight<'a> = &'a dyn Fn(HudFont) -> f32;

fn loc(strings: &LocalizationTable, key: &str) -> String {
    strings
        .get_string(custom_hash(&key.to_lowercase()))
        .unwrap_or_default()
}

/// TimedObjectiveRuleset.RenderData: the time text measured into a box at
/// (border + w/2, border + h/2), cyan Maiandra16 at (border, border).
pub(crate) fn timed_objective_frame(
    r: &TimedObjectiveRuleset,
    strings: &LocalizationTable,
    measure: Measure,
    frame: &mut HudFrame,
) {
    let text = r.hud_text(&loc(strings, TIME_ELAPSED_TEXT));
    let [w, h] = measure(HudFont::Maiandra16, &text);
    let (w, h) = (w.trunc(), h.trunc());
    let border = r.border_size as f32;
    let center = [
        (w as i32 / 2) as f32 + border,
        (h as i32 / 2) as f32 + border,
    ];
    frame.text_box = Some((r.texture.clone(), center, [w, h]));
    let mut line = HudLine::new(
        HudFont::Maiandra16,
        text,
        [
            center[0] - (w as i32 / 2) as f32,
            center[1] - (h as i32 / 2) as f32,
        ],
        TextAlign::Left,
    );
    line.color = [0.0, 1.0, 1.0, 1.0];
    frame.lines.push(line);
}

/// SurvivalRuleset.RenderData + WaveIndicator (+ the wave health bar,
/// drawn by the boss bar quads).
pub(crate) fn survival_frame(
    r: &SurvivalRuleset,
    strings: &LocalizationTable,
    screen: [f32; 2],
    line_height: LineHeight,
    frame: &mut HudFrame,
) {
    let [sw, sh] = screen;
    let bar = [(sw as i32 / 2 - (0.8 * sw) as i32 / 2) as f32, 32.0];
    let at = |o: [f32; 2]| [bar[0] + o[0], bar[1] + o[1]];
    let quad = |pos: [f32; 2], size: [f32; 2], atlas: [f32; 4], sx: f32| HudQuad {
        bounds: [pos[0], pos[1], pos[0] + size[0] * sx, pos[1] + size[1]],
        atlas,
        color: [1.0; 4],
    };
    frame.quads.push((
        HudAtlas::Hud,
        quad(bar, [512.0, 64.0], [0.0, 256.0, 512.0, 320.0], 1.0),
    ));
    frame.quads.push((
        HudAtlas::Hud,
        quad(
            at([48.0, 0.0]),
            [96.0, 32.0],
            [128.0, 320.0, 224.0, 352.0],
            r.damage_amount(),
        ),
    ));
    frame.quads.push((
        HudAtlas::Hud,
        quad(
            at([48.0, 0.0]),
            [112.0, 32.0],
            [0.0, 320.0, 112.0, 352.0],
            1.0,
        ),
    ));
    let total_half = line_height(HudFont::MenuDefault) * 0.5;
    let multi_half = line_height(HudFont::Maiandra16) * 0.5;
    frame.lines.push(
        HudLine::new(
            HudFont::MenuDefault,
            r.shown_total_score.to_string(),
            at([461.0, 16.0 - total_half]),
            TextAlign::Right,
        )
        .shadowed(1.0),
    );
    frame.lines.push(
        HudLine::new(
            HudFont::Maiandra16,
            format!("{}x", r.damage_multiplier),
            at([110.0, 2.0]),
            TextAlign::Right,
        )
        .shadowed(1.0),
    );
    frame.lines.push(
        HudLine::new(
            HudFont::Maiandra16,
            format!("{}x", r.time_multiplier),
            at([188.0, 20.0 - multi_half]),
            TextAlign::Center,
        )
        .shadowed(1.0),
    );
    frame.lines.push(
        HudLine::new(
            HudFont::MenuDefault,
            format!("{}x", r.total_multiplier),
            at([224.0, 32.0 - total_half]),
            TextAlign::Center,
        )
        .shadowed(1.0),
    );
    frame.quads.push((
        HudAtlas::HudTop,
        quad(
            at([256.0, 0.0]),
            [208.0, 32.0],
            [256.0, 320.0, 464.0, 352.0],
            1.0,
        ),
    ));
    // The time-multiplier pie (radius 20, unscaled, UV centre (32, 384)).
    frame.pies.push(PieFan {
        center: at([188.0, 20.0]),
        scale: 1.0,
        radius: 20.0,
        max_angle: r.normalized_time() * std::f32::consts::TAU,
        uv_center: [32.0, 384.0],
        uv_offset: [0.0; 2],
    });
    let scale = sh / 720.0;
    let origin = [(sw * 0.05 + 0.5).floor(), (sh * 0.05 + 0.5).floor()];
    let scroll = line_height(HudFont::Maiandra16);
    for s in r.scroll_scores.iter().filter(|s| !s.dead()) {
        let mut text = loc_id(strings, s.name);
        if s.multi_kills > 1 {
            text.push_str(&format!(" x{}", s.multi_kills));
        }
        text.push(' ');
        if s.score > 0 {
            text.push('+');
        }
        text.push_str(&s.score.to_string());
        let mut line = HudLine::new(
            HudFont::Maiandra14,
            text,
            [origin[0], origin[1] + scroll * 2.0 + s.position * 16.0],
            TextAlign::Left,
        );
        line.color[3] = s.alpha;
        line.shadow = Some(([1.0, 1.0], s.alpha));
        line.scale = scale;
        frame.lines.push(line);
    }
    let wave = loc(strings, "#challenge_wave").replace("#1;", &r.wave_number.to_string());
    frame.lines.push(HudLine::new(
        HudFont::Maiandra16,
        wave,
        [sw * 0.5, 8.0],
        TextAlign::Center,
    ));
}

fn loc_id(strings: &LocalizationTable, id: i32) -> String {
    strings.get_string(id).unwrap_or_default()
}

/// VersusRuleset.RenderData (original HUD): the clock (frame quads and
/// remaining-time pie), the countdown digit, the game-over text, the Score
/// panels, the last ten clock seconds and the Krietor unlock line.
#[allow(clippy::too_many_arguments)]
pub(crate) fn versus_frame(
    r: &VersusRuleset,
    game_over: bool,
    panels: &[(bool, Vec<PanelPlayer>, i32)],
    strings: &LocalizationTable,
    screen: [f32; 2],
    hud: [f32; 2],
    line_height: LineHeight,
    measure: Measure,
    frame: &mut HudFrame,
) {
    let [sw, sh] = screen;
    let mid = sw * 0.5;
    let half = hud[0] * 0.5;
    let quad = |atlas: HudAtlas, b: [f32; 4], a: [f32; 4], color: [f32; 4]| {
        (
            atlas,
            HudQuad {
                bounds: b,
                atlas: a,
                color,
            },
        )
    };
    if r.time_limit > 0.0 {
        frame.quads.push(quad(
            HudAtlas::Hud,
            [mid - 47.0, 6.0, mid + 47.0, 100.0],
            [384.0, 128.0, 478.0, 222.0],
            [1.0; 4],
        ));
        frame.pies.push(PieFan {
            center: [mid, 54.0],
            scale: 1.5,
            radius: 32.0,
            max_angle: std::f32::consts::TAU * (r.time_limit_timer / r.time_limit),
            uv_center: [0.0; 2],
            uv_offset: [0.9375, 0.1875],
        });
        frame.quads.push(quad(
            HudAtlas::HudOverlay,
            [mid - 53.0, 0.0, mid + 53.0, 106.0],
            [half, 128.0, half + 106.0, 234.0],
            [1.0; 4],
        ));
    }
    // Score.Draw for each panel, outward from the clock.
    let (mut left, mut right) = (0.0f32, 0.0f32);
    let score_half = line_height(HudFont::VersusText) * 0.5;
    for (left_aligned, players, score) in panels {
        if players.is_empty() {
            continue;
        }
        let n = players.len() as f32;
        let width = n * 74.0 + 80.0;
        let x0 = if *left_aligned {
            let x = mid - left - 48.0;
            left += width + 16.0;
            x
        } else {
            let x = mid + right + 48.0;
            right += width + 16.0;
            x
        };
        let num = if *left_aligned { x0 - width } else { x0 + 80.0 };
        let end = num + n * 74.0;
        let white = [1.0; 4];
        frame.quads.push(if *left_aligned {
            quad(
                HudAtlas::Hud,
                [num - 23.0, 0.0, num, 106.0],
                [half, 0.0, half + 23.0, 106.0],
                white,
            )
        } else {
            quad(
                HudAtlas::Hud,
                [end, 0.0, end + 23.0, 106.0],
                [half + 23.0, 0.0, half, 106.0],
                white,
            )
        });
        for (j, p) in players.iter().enumerate() {
            let x = num + j as f32 * 74.0;
            frame.quads.push(quad(
                HudAtlas::Hud,
                [x, 0.0, x + 74.0, 32.0],
                [half + 23.0, 0.0, half + 97.0, 32.0],
                white,
            ));
            // DrawPlayer: backdrop, portrait (tinted lower half, then upper).
            frame.quads.push(quad(
                HudAtlas::Hud,
                [x + 5.0, 37.0, x + 69.0, 101.0],
                [448.0, 0.0, 512.0, 64.0],
                white,
            ));
            let portrait = HudAtlas::Portrait(p.slot as u8);
            let c = p.color;
            // Portrait UVs are normalized (atlas size 1×1).
            frame.quads.push(quad(
                portrait,
                [x + 5.0, 37.0, x + 69.0, 101.0],
                [0.0, 0.5, 1.0, 1.0],
                [c[0], c[1], c[2], 1.0],
            ));
            frame.quads.push(quad(
                portrait,
                [x + 5.0, 37.0, x + 69.0, 101.0],
                [0.0, 0.0, 1.0, 0.5],
                white,
            ));
            if p.respawn > 0 {
                frame.lines.push(HudLine::new(
                    HudFont::VersusText,
                    p.respawn.to_string(),
                    [x + 41.0, 69.0 - score_half],
                    TextAlign::Center,
                ));
            }
            frame.quads.push(quad(
                HudAtlas::HudOverlay,
                [x, 32.0, x + 74.0, 106.0],
                [half + 23.0, 32.0, half + 97.0, 106.0],
                white,
            ));
        }
        frame.quads.push(if *left_aligned {
            quad(
                HudAtlas::HudOverlay,
                [end, 0.0, end + 64.0, 90.0],
                [half + 97.0, 0.0, half + 161.0, 90.0],
                white,
            )
        } else {
            quad(
                HudAtlas::HudOverlay,
                [num - 64.0, 0.0, num, 90.0],
                [half + 161.0, 0.0, half + 97.0, 90.0],
                white,
            )
        });
        let hide = r.settings.mode == VersusMode::Brawl && *score < 0;
        if !hide {
            let x = if *left_aligned { x0 - 50.0 } else { x0 + 51.0 };
            let mut line = HudLine::new(
                HudFont::VersusText,
                score.to_string(),
                [x, 24.0 + 37.0 - score_half + 8.0],
                TextAlign::Center,
            );
            line.scale = 0.8;
            frame.lines.push(line);
        }
    }
    let value = r.countdown_timer as i32;
    if r.countdown_timer >= 0.0 && value >= 0 {
        let fraction = r.countdown_timer - value as f32;
        let t = 1.0 - fraction;
        let scale = (2.0 * 0.001f32.powf(t) + 1.0) * ((1.0 - 0.0001f32.powf(t)) / 2.0);
        let (size, shift) = if value <= 0 {
            (256.0 * scale * 1.5, 16.0)
        } else {
            (256.0 * scale, 0.0)
        };
        let x = mid - size * 0.5 + shift;
        let y = sh * 0.5 - size * 0.5;
        let u = value as f32 * 256.0;
        frame.quads.push((
            HudAtlas::Countdown,
            HudQuad {
                bounds: [x, y, x + size, y + size],
                atlas: [u, 0.0, u + 256.0, 256.0],
                color: [1.0; 4],
            },
        ));
    }
    if game_over {
        frame.lines.push(HudLine::new(
            HudFont::VersusText,
            loc(strings, "#game_over"),
            [mid, sh * 0.35],
            TextAlign::Center,
        ));
    }
    if let Some(seconds) = r.clock_seconds().filter(|&s| s > 0) {
        frame.lines.push(
            HudLine::new(
                HudFont::VersusText,
                seconds.to_string(),
                [mid + 1.0, 54.0 - line_height(HudFont::VersusText) * 0.5],
                TextAlign::Center,
            )
            .shadowed(2.0),
        );
    }
    if r.sudden_death {
        frame.lines.push(
            HudLine::new(
                HudFont::VersusText,
                loc(strings, "#opt_vs_sudden_death"),
                [mid, 96.0],
                TextAlign::Center,
            )
            .shadowed(2.0),
        );
    } else if let Some((magick, left)) = r.next_unlock() {
        let seconds = if left < 10.0 { left as i32 } else { 0 };
        let name = magicka_sim::spells::MagickType::try_from(magick)
            .ok()
            .and_then(magicka_runtime::tome_magick::name_key)
            .map(|k| loc(strings, &k))
            .unwrap_or_default();
        if seconds > 0 {
            let width = measure(HudFont::VersusText, &name)[0];
            frame.lines.push(
                HudLine::new(HudFont::VersusText, name, [mid, 96.0], TextAlign::Center)
                    .shadowed(2.0),
            );
            frame.lines.push(
                HudLine::new(
                    HudFont::VersusText,
                    seconds.to_string(),
                    [mid + width * 0.5 + 8.0, 96.0],
                    TextAlign::Left,
                )
                .shadowed(2.0),
            );
        }
    }
}

#[derive(Resource)]
struct RulesetHudAssets {
    fonts: HashMap<HudFont, (Arc<GeometryFont>, Handle<Image>)>,
    strings: LocalizationTable,
    atlases: HashMap<HudAtlas, (Handle<HudMaterial>, [f32; 2])>,
    boxes: HashMap<String, Handle<TextBoxMaterial>>,
    hud_size: [f32; 2],
    /// PlayableCharacter `<character>` (lowercase) → `<portrait>` path.
    portraits: HashMap<String, String>,
    /// The portrait path each slot's batch shows.
    portrait_loaded: [Option<String>; 4],
    /// Level.DisplayName (the .lvl `<Name>`).
    level_name: String,
}

#[derive(Component)]
struct HudPie;

/// Profile.Avatar's portrait: Data/Characters/Playable/*.xml `<portrait>`
/// by `<character>`; the default avatar's Thumb_Portrait otherwise.
fn playable_portraits(content: &std::path::Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let dir = content.join("Data/Characters/Playable");
    for path in std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
    {
        let Ok(xml) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lower = xml.to_lowercase();
        let tag = |name: &str| {
            let open = format!("<{name}>");
            let start = lower.find(&open)? + open.len();
            let end = lower[start..].find('<')? + start;
            Some(xml[start..end].trim().to_string())
        };
        if let (Some(character), Some(portrait)) = (tag("character"), tag("portrait")) {
            out.insert(character.to_lowercase(), portrait);
        }
    }
    out
}

const DEFAULT_PORTRAIT: &str = "Models/Characters/Wizard/Thumb_Portrait";

/// A PieEffect fan as a TriangleList mesh (31 fan triangles).
fn pie_mesh(pie: Option<&PieFan>, screen: [f32; 2], hud: [f32; 2]) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let vertices = pie
        .map(PieFan::vertices)
        .unwrap_or_else(|| vec![([0.0; 2], [0.0; 2]); 3]);
    let offset = pie.map_or([0.0; 2], |p| p.uv_offset);
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vertices
            .iter()
            .map(|(p, _)| [p[0] - screen[0] * 0.5, screen[1] * 0.5 - p[1], 0.0])
            .collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vertices
            .iter()
            .map(|(_, uv)| [uv[0] / hud[0] + offset[0], uv[1] / hud[1] + offset[1]])
            .collect::<Vec<_>>(),
    );
    let n = vertices.len() as u16;
    let indices = (1..n.saturating_sub(1))
        .flat_map(|i| [0, i, i + 1])
        .collect();
    mesh.insert_indices(Indices::U16(indices));
    mesh
}

#[derive(Component)]
struct HudText {
    index: usize,
    shadow: bool,
    material: Handle<OverlayFontMaterial>,
}
#[derive(Component)]
struct HudQuads(HudAtlas);
#[derive(Component)]
struct HudTextBox;

/// Built mesh keys per pooled line: (font, text, align).
#[derive(Default)]
struct Built(Vec<Option<(HudFont, String, TextAlign)>>);

pub(super) fn plugin(app: &mut App) {
    app.init_play_resource::<SheetState>()
        .add_play_systems(Startup, setup)
        .add_play_systems(
            Update,
            (present, statistics_input).after(super::step_session),
        );
}

fn texture(play: &Play, name: &str) -> Result<magicka_content::assets::Texture2D, String> {
    // Content paths are case-insensitive (UI/Menu/pages.xnb).
    let relative = std::path::PathBuf::from(format!("{}.xnb", name.replace('\\', "/")));
    let path = magicka_runtime::play_content::resolve(&play.content, &relative)
        .unwrap_or_else(|| play.content.join(&relative));
    match read_asset(&path)? {
        Asset::Texture2D(t) => Ok(t),
        _ => Err(format!("expected texture {name}")),
    }
}

fn setup(
    mut commands: Commands,
    play: Res<Play>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut hud_materials: ResMut<Assets<HudMaterial>>,
    mut font_materials: ResMut<Assets<OverlayFontMaterial>>,
    mut box_materials: ResMut<Assets<TextBoxMaterial>>,
) {
    let result = (|| -> Result<(), String> {
        let mut fonts = HashMap::new();
        for font in HudFont::ALL {
            let (file, spacing) = font.file();
            let path = play.content.join(format!("Languages/eng/Font/{file}.xnb"));
            let Asset::BitmapFont(f) = read_asset(&path)? else {
                return Err(format!("expected font {file}"));
            };
            let (geometry, material) = upload_overlay_font(&f, &mut images, &mut font_materials)?;
            let mut geometry = (*geometry).clone();
            geometry.metrics.spacing = spacing;
            let image = font_materials
                .get(&material)
                .map(|m| m.texture.clone())
                .ok_or("font material")?;
            fonts.insert(font, (Arc::new(geometry), image));
        }
        let hud = texture(&play, "UI/HUD/hud")?;
        let hud_size = [hud.width as f32, hud.height as f32];
        let hud_image = upload_texture(&hud, &mut images)?;
        let countdown = texture(&play, "UI/HUD/versus_countdown")?;
        let countdown_image = upload_texture(&countdown, &mut images)?;
        let mut atlases = HashMap::new();
        for atlas in HudAtlas::BATCHES {
            let (image, size) = match atlas {
                HudAtlas::Shade => (
                    images.add(Image::new_fill(
                        bevy::render::render_resource::Extent3d {
                            width: 1,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                        bevy::render::render_resource::TextureDimension::D2,
                        &[255, 255, 255, 255],
                        bevy::render::render_resource::TextureFormat::Rgba8Unorm,
                        bevy::asset::RenderAssetUsages::RENDER_WORLD,
                    )),
                    [1.0, 1.0],
                ),
                HudAtlas::FadeBox | HudAtlas::Pages => {
                    let name = if atlas == HudAtlas::FadeBox {
                        "UI/Menu/FadeBox"
                    } else {
                        "UI/Menu/Pages"
                    };
                    let t = texture(&play, name)?;
                    (
                        upload_texture(&t, &mut images)?,
                        [t.width as f32, t.height as f32],
                    )
                }
                HudAtlas::Countdown => (
                    countdown_image.clone(),
                    [countdown.width as f32, countdown.height as f32],
                ),
                // Portraits get their texture when a player is known.
                HudAtlas::Portrait(_) => (hud_image.clone(), [1.0, 1.0]),
                _ => (hud_image.clone(), hud_size),
            };
            let material = hud_materials.add(HudMaterial {
                texture: image,
                saturation: Vec4::ONE,
            });
            atlases.insert(atlas, (material.clone(), size));
            commands.spawn((
                HudQuads(atlas),
                Mesh2d(meshes.add(crate::play_hud::geometry(&[], play.screen, [1.0, 1.0]))),
                MeshMaterial2d(material),
                Transform::from_xyz(0., 0., atlas.layer()),
                Visibility::Hidden,
            ));
        }
        // PieEffect's pixel shader is TextBoxEffect's (1 − 2^(−tex·tint)).
        let pie_material = box_materials.add(TextBoxMaterial {
            color: Vec4::ONE,
            texture: hud_image,
        });
        for _ in 0..2 {
            commands.spawn((
                HudPie,
                Mesh2d(meshes.add(pie_mesh(None, play.screen, hud_size))),
                MeshMaterial2d(pie_material.clone()),
                Transform::from_xyz(0., 0., LAYER - 0.4),
                Visibility::Hidden,
            ));
        }
        commands.insert_resource(RulesetHudAssets {
            fonts,
            strings: magicka_runtime::play_content::english_strings(&play.content, &["Wizard"]),
            atlases,
            boxes: HashMap::new(),
            hud_size,
            portraits: playable_portraits(&play.content),
            portrait_loaded: [None, None, None, None],
            level_name: std::fs::read_to_string(play.content.join("Levels").join(&play.level))
                .ok()
                .and_then(|t| {
                    let (_, rest) = t.split_once("<Name>")?;
                    Some(rest.split_once("</Name>")?.0.trim().to_owned())
                })
                .unwrap_or_default(),
        });
        Ok(())
    })();
    if let Err(e) = result {
        error!("ruleset HUD unavailable: {e}");
    }
}

fn build_text(font: &GeometryFont, text: &str, align: TextAlign) -> Result<Mesh, String> {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let geometry = TextGeometry::build(font, &utf16, TextStyle { align, ..default() })
        .map_err(|e| format!("{e:?}"))?;
    geometry_mesh(&geometry, geometry.quads.len() as i32)
}

/// The frame for the current session state.
/// The Score panels VersusRuleset.Initialize builds: two team panels (red
/// left, blue right) or one per player alternating left/right, each with
/// GetScores/GetTeamScores (Brawl: lives) and the respawn timers.
fn versus_panels(
    play: &Play,
    r: &VersusRuleset,
    robe: [f32; 3],
) -> Vec<(bool, Vec<PanelPlayer>, i32)> {
    use magicka_runtime::ruleset::{TEAM_BLUE, TEAM_RED};
    let world = &play.session.world;
    if r.settings.mode == VersusMode::Pyrite {
        return Vec::new();
    }
    let (scores, team_scores) = r.mode_scores().unwrap_or_default();
    let players: Vec<PanelPlayer> = world
        .players
        .iter()
        .flatten()
        .filter(|p| p.playing)
        .map(|p| PanelPlayer {
            slot: p.slot,
            // Defines.PLAYERCOLORS[Player.Color]; the local robe colour
            // stands in for slot 0's chosen colour.
            color: player_identity(play, robe).1[p.slot],
            respawn: r.respawn_timers[p.slot] as i32,
        })
        .collect();
    if r.settings.teams {
        let team = |flag: i32| -> Vec<PanelPlayer> {
            players
                .iter()
                .copied()
                .filter(|p| {
                    let t = world.player_teams[p.slot];
                    if flag == TEAM_RED {
                        t & TEAM_RED == TEAM_RED
                    } else {
                        t & TEAM_RED != TEAM_RED
                    }
                })
                .collect()
        };
        vec![
            (true, team(TEAM_RED), i32::from(team_scores[0])),
            (false, team(TEAM_BLUE), i32::from(team_scores[1])),
        ]
    } else {
        players
            .iter()
            .enumerate()
            .map(|(i, p)| (i % 2 == 0, vec![*p], i32::from(scores[p.slot])))
            .collect()
    }
}

/// GamerTag/colour per slot (profile, else "Player N" and PLAYERCOLORS).
fn player_identity(play: &Play, robe: [f32; 3]) -> ([String; 4], [[f32; 3]; 4]) {
    let names = std::array::from_fn(|slot| {
        play.session.player_profiles[slot]
            .as_ref()
            .map_or_else(|| format!("Player {}", slot + 1), |p| p.0.clone())
    });
    let colors = std::array::from_fn(|slot| super::slot_colour(&play.session, robe, slot));
    (names, colors)
}

fn sheet_context<'a>(
    play: &'a Play,
    assets: &'a RulesetHudAssets,
    robe: [f32; 3],
    state: &SheetState,
) -> sheet::SheetContext<'a> {
    let (names, colors) = player_identity(play, robe);
    sheet::SheetContext {
        strings: &assets.strings,
        screen: play.screen,
        fade_box: assets.atlases[&HudAtlas::FadeBox].1,
        level_name: &assets.level_name,
        names,
        colors,
        teams: play.session.world.player_teams.clone(),
        time: state.time,
        selected: state.selected,
        network: play.session.network_mode,
    }
}

/// The open statistics sheet's selection and age.
#[derive(Resource, Default)]
pub(crate) struct SheetState {
    time: f32,
    selected: usize,
    /// AddLocalEntry ran for this sheet.
    recorded: bool,
}

/// The menu host's local leaderboard file (shared with the frontend).
pub(crate) fn leaderboard_path(cache: &std::path::Path) -> std::path::PathBuf {
    cache.join("leaderboards.json")
}

fn frame_for(
    play: &Play,
    assets: &RulesetHudAssets,
    robe: [f32; 3],
    sheet_state: &SheetState,
) -> HudFrame {
    let measure = |font: HudFont, text: &str| {
        assets.fonts[&font]
            .0
            .metrics
            .measure(text, true)
            .unwrap_or([0.0; 2])
    };
    let line_height = |font: HudFont| assets.fonts[&font].0.metrics.line_height as f32;
    let world = &play.session.world;
    let mut frame = HudFrame::default();
    if play.session.end_game.statistics {
        if let Some(results) = play.session.ruleset_results() {
            let ctx = sheet_context(play, assets, robe, sheet_state);
            sheet::sheet(&results, &ctx, &line_height, &measure, &mut frame);
        }
        return frame;
    }
    if world.flags.cutscene {
        return frame;
    }
    if let Some(r) = world
        .timed_objective()
        .filter(|_| world.timed_objective_hud)
    {
        timed_objective_frame(r, &assets.strings, &measure, &mut frame);
    }
    if let Some(r) = world.survival() {
        survival_frame(r, &assets.strings, play.screen, &line_height, &mut frame);
        if let Some(bar) = world.survival_health_bar.filter(|b| b.alpha > 0.0) {
            for q in super::boss_bar_visual::quads(
                play.screen[0],
                0.8,
                bar.display_health,
                bar.power,
                bar.alpha,
            ) {
                frame.quads.push((HudAtlas::Boss, q));
            }
        }
    }
    if let Some(r) = world.versus() {
        let over = play.session.end_game.condition != EndGameCondition::None;
        let panels = versus_panels(play, r, robe);
        versus_frame(
            r,
            over,
            &panels,
            &assets.strings,
            play.screen,
            assets.hud_size,
            &line_height,
            &measure,
            &mut frame,
        );
    }
    frame
}

#[allow(clippy::too_many_arguments)]
fn present(
    mut commands: Commands,
    play: Res<Play>,
    assets: Option<ResMut<RulesetHudAssets>>,
    mut built: Local<Built>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut font_materials: ResMut<Assets<OverlayFontMaterial>>,
    mut box_materials: ResMut<Assets<TextBoxMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut hud_materials: ResMut<Assets<HudMaterial>>,
    robe: Option<Res<crate::robe::RobeColour>>,
    sheet_state: Res<SheetState>,
    mut texts: Query<(&HudText, &Mesh2d, &mut Transform, &mut Visibility)>,
    mut quads: Query<(&HudQuads, &Mesh2d, &mut Visibility), Without<HudText>>,
    mut boxes: Query<
        (Entity, &Mesh2d, &mut Visibility),
        (With<HudTextBox>, Without<HudText>, Without<HudQuads>),
    >,
    mut pies: Query<
        (&Mesh2d, &mut Visibility),
        (
            With<HudPie>,
            Without<HudText>,
            Without<HudQuads>,
            Without<HudTextBox>,
        ),
    >,
) {
    let Some(mut assets) = assets else { return };
    let frame = frame_for(&play, &assets, robe.map_or([1.0; 3], |r| r.0), &sheet_state);
    let screen = play.screen;
    // Pie fans.
    for (i, (mesh, mut visibility)) in pies.iter_mut().enumerate() {
        let pie = frame.pies.get(i);
        *visibility = if pie.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if pie.is_some() {
            if let Some(mut m) = meshes.get_mut(&mesh.0) {
                *m = pie_mesh(pie, screen, assets.hud_size);
            }
        }
    }
    // Portrait textures for the Versus panel slots.
    for slot in 0..4u8 {
        if !frame
            .quads
            .iter()
            .any(|(a, _)| *a == HudAtlas::Portrait(slot))
        {
            continue;
        }
        let robe = play
            .session
            .player_robe(slot as usize)
            .unwrap_or("wizard")
            .to_lowercase();
        let path = assets
            .portraits
            .get(&robe)
            .cloned()
            .unwrap_or_else(|| DEFAULT_PORTRAIT.into());
        if assets.portrait_loaded[slot as usize].as_ref() == Some(&path) {
            continue;
        }
        match texture(&play, &path).and_then(|t| upload_texture(&t, &mut images)) {
            Ok(image) => {
                let handle = assets.atlases[&HudAtlas::Portrait(slot)].0.clone();
                if let Some(mut m) = hud_materials.get_mut(&handle) {
                    m.texture = image;
                }
            }
            Err(e) => warn!("versus portrait {path}: {e}"),
        }
        assets.portrait_loaded[slot as usize] = Some(path);
    }
    // Quads by atlas.
    for (q, mesh, mut visibility) in &mut quads {
        let list: Vec<HudQuad> = frame
            .quads
            .iter()
            .filter(|(a, _)| *a == q.0)
            .map(|(_, quad)| *quad)
            .collect();
        *visibility = if list.is_empty() {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        if !list.is_empty() {
            let size = assets.atlases[&q.0].1;
            if let Some(mut m) = meshes.get_mut(&mesh.0) {
                *m = crate::play_hud::geometry(&list, screen, size);
            }
        }
    }
    // The TimedObjective box.
    let mut box_shown = false;
    if let Some((texture_name, center, size)) = &frame.text_box {
        let material = match assets.boxes.get(texture_name) {
            Some(m) => Some(m.clone()),
            None => {
                match texture(&play, texture_name).and_then(|t| upload_texture(&t, &mut images)) {
                    Ok(image) => {
                        let m = box_materials.add(TextBoxMaterial {
                            color: Vec4::ONE,
                            texture: image,
                        });
                        assets.boxes.insert(texture_name.clone(), m.clone());
                        Some(m)
                    }
                    Err(e) => {
                        warn!("ruleset text box {texture_name}: {e}");
                        None
                    }
                }
            }
        };
        if let Some(material) = material {
            let mesh = background_mesh(*center, *size, 1.0, screen);
            if let Some((_, handle, mut visibility)) = boxes.iter_mut().next() {
                if let Some(mut m) = meshes.get_mut(&handle.0) {
                    *m = mesh;
                }
                *visibility = Visibility::Visible;
            } else {
                commands.spawn((
                    HudTextBox,
                    Mesh2d(meshes.add(mesh)),
                    MeshMaterial2d(material),
                    Transform::from_xyz(0., 0., LAYER - 0.2),
                    Visibility::Visible,
                ));
            }
            box_shown = true;
        }
    }
    if !box_shown {
        for (_, _, mut visibility) in &mut boxes {
            *visibility = Visibility::Hidden;
        }
    }
    // Text pool: two entities (shadow, text) per line.
    let have = built.0.len();
    for index in have..frame.lines.len() {
        let font = frame.lines[index].font;
        let image = assets.fonts[&font].1.clone();
        for shadow in [true, false] {
            let material = font_materials.add(OverlayFontMaterial {
                color: Vec4::ONE,
                texture: image.clone(),
            });
            let mesh = build_text(&assets.fonts[&font].0, "", TextAlign::Left)
                .unwrap_or_else(|_| Mesh::new(default(), default()));
            commands.spawn((
                HudText {
                    index,
                    shadow,
                    material: material.clone(),
                },
                Mesh2d(meshes.add(mesh)),
                MeshMaterial2d(material),
                Transform::from_xyz(0., 0., LAYER),
                Visibility::Hidden,
            ));
        }
        built.0.push(None);
    }
    let mut rebuilt = vec![false; built.0.len()];
    for (text, mesh, mut transform, mut visibility) in &mut texts {
        let Some(line) = frame.lines.get(text.index) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let key = (line.font, line.text.clone(), line.align);
        let stale = built
            .0
            .get(text.index)
            .is_some_and(|b| b.as_ref() != Some(&key));
        if stale {
            match build_text(&assets.fonts[&line.font].0, &line.text, line.align) {
                Ok(m) => {
                    if let Some(mut target) = meshes.get_mut(&mesh.0) {
                        *target = m;
                    }
                    if let Some(mut material) = font_materials.get_mut(&text.material) {
                        material.texture = assets.fonts[&line.font].1.clone();
                    }
                    rebuilt[text.index] = true;
                }
                Err(e) => warn!("ruleset HUD text: {e}"),
            }
        }
        let (offset, alpha) = match (text.shadow, line.shadow) {
            (true, Some((offset, alpha))) => (offset, alpha),
            (true, None) => {
                *visibility = Visibility::Hidden;
                continue;
            }
            (false, _) => ([0.0; 2], line.color[3]),
        };
        *visibility = if line.text.is_empty() {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        let [x, y] = [line.origin[0] + offset[0], line.origin[1] + offset[1]];
        transform.translation = Vec3::new(
            x - screen[0] * 0.5,
            screen[1] * 0.5 - y,
            LAYER + text.index as f32 * 0.001 - if text.shadow { 0.0005 } else { 0.0 },
        );
        transform.scale = Vec3::new(line.scale, line.scale, 1.0);
        if let Some(mut m) = font_materials.get_mut(&text.material) {
            m.color = if text.shadow {
                Vec4::new(0., 0., 0., alpha)
            } else {
                Vec4::new(line.color[0], line.color[1], line.color[2], alpha)
            };
        }
    }
    for (index, done) in rebuilt.into_iter().enumerate() {
        if done {
            if let Some(line) = frame.lines.get(index) {
                built.0[index] = Some((line.font, line.text.clone(), line.align));
            }
        }
    }
}

/// Offline statistics menu: Retry restarts the level from its start
/// (Restart StartOfLevel), Quit is Endgame(ChallengeExit) → SubMenuEndGame.
fn statistics_input(
    mut play: ResMut<Play>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut state: ResMut<SheetState>,
    mut exit: MessageWriter<LeaveSession>,
) {
    if play.session.end_game.exit_to_menu {
        exit.write(LeaveSession::default());
        return;
    }
    if !play.session.end_game.statistics {
        *state = SheetState::default();
        return;
    }
    state.time += time.delta_secs();
    // InGameMenuSurvival/TimedObjectiveStatistics.OnEnter: AddLocalEntry for
    // a LevelManager.Challenges level (hosts and offline only).
    if !state.recorded {
        state.recorded = true;
        let level = play.level.clone();
        if play.session.network_mode != magicka_runtime::play_session::NetworkMode::Client
            && magicka_runtime::leaderboards::is_leaderboard_challenge(&level)
        {
            if let Some(results) = play.session.ruleset_results() {
                let robe = [1.0; 3];
                let (names, _) = player_identity(&play, robe);
                let playing: Vec<String> = play
                    .session
                    .world
                    .players
                    .iter()
                    .flatten()
                    .filter(|p| p.playing)
                    .map(|p| names[p.slot].clone())
                    .collect();
                if let Some(entry) =
                    magicka_runtime::leaderboards::LocalLeaderboards::entry_for(&results, &playing)
                {
                    let path = leaderboard_path(&play.cache);
                    let mut boards = magicka_runtime::leaderboards::LocalLeaderboards::load(&path);
                    boards.add(&level, entry);
                    if let Err(e) = boards.save(&path) {
                        warn!("leaderboard save: {e}");
                    }
                }
            }
        }
    }
    let items = sheet::item_actions(play.session.network_mode);
    let count = items.len();
    if keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW) {
        state.selected = (state.selected + count - 1) % count;
    }
    if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS) {
        state.selected = (state.selected + 1) % count;
    }
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        if items[state.selected.min(count - 1)] == sheet::SheetItem::Retry {
            // Restart(StartOfLevel).
            super::reload(&mut play);
        } else {
            play.session
                .end_game
                .begin(EndGameCondition::ChallengeExit, false, false, 0.0);
        }
        *state = SheetState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings() -> LocalizationTable {
        LocalizationTable::default()
    }

    #[test]
    fn survival_bar_follows_native_offsets() {
        use magicka_runtime::ruleset::{RulesetKind, RulesetLayout};
        let layout = RulesetLayout {
            kind: RulesetKind::Survival,
            node: Default::default(),
            areas: vec!["a".into()],
            waves: vec![vec![]],
        };
        let r = SurvivalRuleset::new(&layout, |_| 0, |_| 1.0, 1);
        let mut f = HudFrame::default();
        survival_frame(&r, &strings(), [1280.0, 720.0], &|_| 20.0, &mut f);
        // BarPosition = (w/2 − 0.8w/2, 32).
        assert_eq!(f.quads[0].1.bounds, [128.0, 32.0, 640.0, 96.0]);
        let score = f
            .lines
            .iter()
            .find(|l| l.align == TextAlign::Right && l.font == HudFont::MenuDefault)
            .unwrap();
        assert_eq!(score.origin, [128.0 + 461.0, 32.0 + 16.0 - 10.0]);
        assert_eq!(f.lines.last().unwrap().origin, [640.0, 8.0]);
    }
}
