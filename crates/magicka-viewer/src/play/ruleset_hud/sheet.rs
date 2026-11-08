//! InGameMenuSurvivalStatistics / InGameMenuTimedObjectiveStatistics /
//! InGameMenuVersusStatistics laid out as the native InGameMenu draws them
//! (research/rulesets.md): the half-black screen shade, the 600×600 FadeBox
//! panel at 0.666 alpha, the white selection strip, the Pages kill/death
//! icons and the menu items (Retry, Disconnect when hosting, Quit).
use super::{loc, HudAtlas, HudFont, HudFrame, HudLine, LineHeight, Measure};
use magicka_runtime::{
    hud::HudQuad,
    localization::LocalizationTable,
    ruleset::{RulesetResults, VersusMode, VersusResults},
    text_geometry::TextAlign,
};

/// The sheet's inputs beyond the results.
pub(crate) struct SheetContext<'a> {
    pub strings: &'a LocalizationTable,
    pub screen: [f32; 2],
    pub fade_box: [f32; 2],
    /// Level.DisplayName key.
    pub level_name: &'a str,
    /// GamerTag and colour per slot.
    pub names: [String; 4],
    pub colors: [[f32; 3]; 4],
    /// Player.Team per slot.
    pub teams: magicka_runtime::slots::PerSlot<i32>,
    /// Seconds since the sheet opened (Survival's score count-up).
    pub time: f32,
    pub selected: usize,
    /// NetworkManager.State: offline Retry/Quit; server Retry, Disconnect,
    /// Lobby; client Disconnect (Ready needs a lobby round-trip).
    pub network: magicka_runtime::play_session::NetworkMode,
}

/// What each menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SheetItem {
    /// Restart(StartOfLevel).
    Retry,
    /// Endgame(ChallengeExit) — Disconnect, Quit and Lobby all leave.
    Leave,
}

/// Defines.PLAYERCOLORS[0] and [3] (team strips).
const RED: [f32; 3] = [240.0 / 255.0, 20.0 / 255.0, 20.0 / 255.0];
const BLUE: [f32; 3] = [0.0, 132.0 / 255.0, 1.0];

/// The visible items' actions for a network state.
pub(crate) fn item_actions(network: magicka_runtime::play_session::NetworkMode) -> Vec<SheetItem> {
    use magicka_runtime::play_session::NetworkMode as N;
    match network {
        N::Offline => vec![SheetItem::Retry, SheetItem::Leave],
        N::Server => vec![SheetItem::Retry, SheetItem::Leave, SheetItem::Leave],
        N::Client => vec![SheetItem::Leave],
    }
}

pub(crate) fn items(ctx: &SheetContext) -> Vec<(SheetItem, String)> {
    use magicka_runtime::play_session::NetworkMode as N;
    let keys: &[&str] = match ctx.network {
        N::Offline => &["#add_menu_retry", "#add_menu_quit"],
        N::Server => &["#add_menu_retry", "#network_10", "#menu_lobby"],
        N::Client => &["#network_10"],
    };
    item_actions(ctx.network)
        .into_iter()
        .zip(keys)
        .map(|(a, k)| (a, loc(ctx.strings, k)))
        .collect()
}

fn quad(atlas: HudAtlas, bounds: [f32; 4], src: [f32; 4], color: [f32; 4]) -> (HudAtlas, HudQuad) {
    (
        atlas,
        HudQuad {
            bounds,
            atlas: src,
            color,
        },
    )
}

/// InGameMenu.Draw: shade, panel, then the derived IDraw.
pub(crate) fn sheet(
    results: &RulesetResults,
    ctx: &SheetContext,
    line_height: LineHeight,
    measure: Measure,
    frame: &mut HudFrame,
) {
    let [w, h] = ctx.screen;
    let s = h / 720.0;
    let alpha = (ctx.time * 5.0).min(1.0);
    frame.quads.push(quad(
        HudAtlas::Shade,
        [0.0, 0.0, w, h],
        [0.0, 0.0, 1.0, 1.0],
        [0.0, 0.0, 0.0, 0.5 * alpha],
    ));
    let bg = [600.0 * s, 600.0 * s];
    let x0 = (w - bg[0]) * 0.5;
    let y0 = (720.0 - 600.0) * 0.5 * s;
    let fade = [0.0, 0.0, ctx.fade_box[0], ctx.fade_box[1]];
    frame.quads.push(quad(
        HudAtlas::FadeBox,
        [x0, y0, x0 + bg[0], y0 + bg[1]],
        fade,
        [0.0, 0.0, 0.0, 0.666 * alpha],
    ));
    let text = |font, text: String, origin: [f32; 2], align, scale: f32| {
        let mut l = HudLine::new(font, text, origin, align);
        l.scale = scale;
        l.color[3] = alpha;
        l
    };
    let icons = |frame: &mut HudFrame, x: f32, y: f32| {
        for (dx, src_y) in [(0.0, 64.0), (100.0 * s, 0.0)] {
            frame.quads.push(quad(
                HudAtlas::Pages,
                [x + dx, y, x + dx + 32.0 * s, y + 32.0 * s],
                [1024.0, src_y, 1088.0, src_y + 64.0],
                [1.0, 1.0, 1.0, alpha],
            ));
        }
    };
    let mid = w * 0.5;
    let item_y = match results {
        RulesetResults::Survival(r) => {
            let mut y = h * 0.5 - 220.0 * s;
            frame.lines.push(text(
                HudFont::Maiandra18,
                loc(ctx.strings, ctx.level_name),
                [mid, y],
                TextAlign::Center,
                s,
            ));
            y += 20.0 * s;
            // The score counts up over one second.
            let shown = ((r.total_score as f32 * ctx.time.min(1.0)) as i32).min(r.total_score);
            frame.lines.push(text(
                HudFont::MenuTitle,
                shown.to_string(),
                [mid, y],
                TextAlign::Center,
                s,
            ));
            y += 80.0 * s;
            icons(frame, mid + 74.0 * s, y);
            let x = mid - 210.0 * s;
            y += 48.0 * s;
            for slot in 0..4 {
                if let Some(p) = r.players.iter().find(|p| p.slot == slot) {
                    frame.lines.push(text(
                        HudFont::Maiandra18,
                        ctx.names[slot].clone(),
                        [x, y],
                        TextAlign::Left,
                        1.0,
                    ));
                    frame.lines.push(text(
                        HudFont::Maiandra18,
                        p.kills.to_string(),
                        [x + 300.0 * s, y],
                        TextAlign::Center,
                        1.0,
                    ));
                    frame.lines.push(text(
                        HudFont::Maiandra18,
                        p.deaths.to_string(),
                        [x + 400.0 * s, y],
                        TextAlign::Center,
                        1.0,
                    ));
                }
                y += 45.0 * s;
            }
            h * 0.5 + 210.0 - 64.0
        }
        RulesetResults::TimedObjective(t) => {
            let big = line_height(HudFont::MenuOption);
            let small = line_height(HudFont::Maiandra18);
            let mut y = h * 0.5 - 300.0 + big * 2.0;
            let complete = if t.success {
                "#menu_mission_complete"
            } else {
                "#menu_mission_failed"
            };
            frame.lines.push(text(
                HudFont::MenuOption,
                loc(ctx.strings, complete),
                [mid, y],
                TextAlign::Center,
                1.0,
            ));
            y += big;
            frame.lines.push(text(
                HudFont::Maiandra18,
                loc(ctx.strings, "#challenge_total_score"),
                [mid, y],
                TextAlign::Center,
                1.0,
            ));
            y += small;
            frame.lines.push(text(
                HudFont::MenuOption,
                t.total.to_string(),
                [mid, y],
                TextAlign::Center,
                1.0,
            ));
            y += big;
            frame.lines.push(text(
                HudFont::Maiandra18,
                loc(ctx.strings, "#menu_mission_time"),
                [mid, y],
                TextAlign::Center,
                1.0,
            ));
            y += small;
            frame.lines.push(text(
                HudFont::MenuOption,
                t.time_text.clone(),
                [mid, y],
                TextAlign::Center,
                1.0,
            ));
            y += 1.5 * big;
            for o in &t.objectives {
                frame.lines.push(text(
                    HudFont::Maiandra18,
                    loc(ctx.strings, &o.name),
                    [mid - 300.0 + 48.0, y],
                    TextAlign::Left,
                    1.0,
                ));
                let value = if o.max_value > 0 {
                    format!("{}/{}", o.value, o.max_value)
                } else {
                    String::new()
                };
                frame.lines.push(text(
                    HudFont::Maiandra18,
                    value,
                    [mid + 128.0, y],
                    TextAlign::Right,
                    1.0,
                ));
                frame.lines.push(text(
                    HudFont::Maiandra18,
                    o.score.to_string(),
                    [mid + 300.0 - 48.0, y],
                    TextAlign::Right,
                    1.0,
                ));
                y += small;
            }
            y += small;
            // LOC_TIME_BONUS is "#menu_mission_time" natively.
            frame.lines.push(text(
                HudFont::Maiandra18,
                loc(ctx.strings, "#menu_mission_time"),
                [mid - 252.0, y],
                TextAlign::Left,
                1.0,
            ));
            frame.lines.push(text(
                HudFont::Maiandra18,
                t.bonus.to_string(),
                [mid + 252.0, y],
                TextAlign::Right,
                1.0,
            ));
            y + small * 2.0
        }
        RulesetResults::Versus(v) => {
            versus_sheet(v, ctx, line_height, frame, &text, &icons);
            h * 0.5 + 300.0 - items(ctx).len() as f32 * 32.0
        }
    };
    // Menu items and the selection strip.
    let mut y = item_y;
    let item_height = line_height(HudFont::Maiandra18) * s;
    let strip = match results {
        RulesetResults::Survival(_) => 200.0 * s,
        _ => bg[0],
    };
    for (i, (_, label)) in items(ctx).into_iter().enumerate() {
        let selected = i == ctx.selected;
        if selected {
            frame.quads.push(quad(
                HudAtlas::FadeBox,
                [mid - strip * 0.5, y, mid + strip * 0.5, y + item_height],
                fade,
                [1.0, 1.0, 1.0, 0.8 * alpha],
            ));
        }
        let mut line = text(HudFont::Maiandra18, label, [mid, y], TextAlign::Center, s);
        if selected {
            line.color = [0.0, 0.0, 0.0, alpha];
        }
        frame.lines.push(line);
        y += item_height;
    }
    let _ = measure;
}

#[allow(clippy::type_complexity)]
fn versus_sheet(
    v: &VersusResults,
    ctx: &SheetContext,
    line_height: LineHeight,
    frame: &mut HudFrame,
    text: &dyn Fn(HudFont, String, [f32; 2], TextAlign, f32) -> HudLine,
    icons: &dyn Fn(&mut HudFrame, f32, f32),
) {
    use magicka_runtime::ruleset::TEAM_RED;
    let [w, h] = ctx.screen;
    let s = h / 720.0;
    let mid = w * 0.5;
    let big = line_height(HudFont::MenuOption);
    let mut y = h * 0.5 - 200.0;
    let mode = match v.mode {
        VersusMode::DeathMatch => "#versus_dm",
        VersusMode::Brawl => "#versus_brawl",
        VersusMode::Krietor => "#versus_tourney",
        VersusMode::King => "#versus_king",
        VersusMode::Pyrite => "#versus_pyrite",
    };
    frame.lines.push(text(
        HudFont::MenuOption,
        loc(ctx.strings, mode),
        [mid, y],
        TextAlign::Center,
        1.0,
    ));
    y += big * 2.0;
    let strip = |frame: &mut HudFrame, y: f32, c: [f32; 3]| {
        frame.quads.push(quad(
            HudAtlas::FadeBox,
            [mid - 300.0 * s, y, mid + 300.0 * s, y + big],
            [0.0, 0.0, ctx.fade_box[0], ctx.fade_box[1]],
            [c[0], c[1], c[2], 0.8],
        ));
    };
    let sorted = |filter: &dyn Fn(usize) -> bool| {
        let mut list: Vec<_> = v
            .players
            .iter()
            .filter(|p| filter(p.slot))
            .copied()
            .collect();
        // Stable descending by kills (List.Sort comparer).
        list.sort_by_key(|p| std::cmp::Reverse(p.kills));
        list
    };
    let rows = |frame: &mut HudFrame,
                y: &mut f32,
                list: &[magicka_runtime::ruleset::SurvivalPlayerResult]| {
        for p in list {
            frame.lines.push(text(
                HudFont::Maiandra18,
                ctx.names[p.slot].clone(),
                [mid - 180.0, *y],
                TextAlign::Left,
                1.0,
            ));
            frame.lines.push(text(
                HudFont::Maiandra18,
                p.kills.to_string(),
                [mid + 10.0, *y],
                TextAlign::Left,
                1.0,
            ));
            frame.lines.push(text(
                HudFont::Maiandra18,
                p.deaths.to_string(),
                [mid + 110.0, *y],
                TextAlign::Left,
                1.0,
            ));
            icons(frame, mid + 40.0, *y);
            *y += 55.0;
        }
    };
    let winner = |name: String| loc(ctx.strings, "#add_vs_winner").replace("#player;", &name);
    let (scores, team_scores) = v.scores.clone().unwrap_or_default();
    if v.teams {
        let red = |slot: usize| ctx.teams[slot] & TEAM_RED == TEAM_RED;
        if team_scores[0] != team_scores[1] {
            let red_won = team_scores[0] > team_scores[1];
            let (win_color, lose_color) = if red_won { (RED, BLUE) } else { (BLUE, RED) };
            let (win_key, lose_key) = if red_won {
                ("#add_team_red", "#add_team_blue")
            } else {
                ("#add_team_blue", "#add_team_red")
            };
            strip(frame, y, win_color);
            frame.lines.push(text(
                HudFont::MenuOption,
                winner(loc(ctx.strings, win_key)),
                [mid, y],
                TextAlign::Center,
                1.0,
            ));
            y += big + 15.0;
            rows(frame, &mut y, &sorted(&|slot| red(slot) == red_won));
            y += big;
            strip(frame, y, lose_color);
            frame.lines.push(text(
                HudFont::MenuOption,
                loc(ctx.strings, lose_key),
                [mid, y],
                TextAlign::Center,
                1.0,
            ));
            y += big;
            rows(frame, &mut y, &sorted(&|slot| red(slot) != red_won));
        } else {
            frame.lines.push(text(
                HudFont::MenuOption,
                loc(ctx.strings, "#menu_stat_draw"),
                [mid, y],
                TextAlign::Center,
                1.0,
            ));
            y += big;
            rows(frame, &mut y, &sorted(&|_| true));
        }
    } else {
        // The best score among playing players; a tie is a draw, still drawn
        // over the default (index 10, white) strip with black text.
        let mut best: Option<(usize, i16)> = None;
        let mut tie = false;
        for p in &v.players {
            let score = scores[p.slot];
            match best {
                Some((_, b)) if score == b => tie = true,
                Some((_, b)) if score < b => {}
                _ => {
                    best = Some((p.slot, score));
                    tie = false;
                }
            }
        }
        if let Some((slot, _)) = best {
            let (label, color, black) = if tie {
                (
                    loc(ctx.strings, "#menu_stat_draw"),
                    [320.0 / 255.0; 3],
                    true,
                )
            } else {
                (winner(ctx.names[slot].clone()), ctx.colors[slot], false)
            };
            strip(frame, y, color);
            let mut line = text(HudFont::MenuOption, label, [mid, y], TextAlign::Center, 1.0);
            if black {
                line.color = [0.0, 0.0, 0.0, line.color[3]];
            }
            frame.lines.push(line);
        }
        y += big + 15.0;
        rows(frame, &mut y, &sorted(&|_| true));
    }
}
