//! The Challenge and Versus entries of SubMenuMain (research/rulesets.md).
//! Native picks the level and Versus mode inside SubMenuCharacterSelect's
//! lobby (LevelManager.Challenges / Versus, mGameModeBox); these pages list
//! the same levels by their `<Name>` and continue to character select.
//! Versus hosts a network session, since a match needs a second player.
use super::{
    book::{Font, Measure, PageDraw, Widget},
    pages::{back_button, back_item, loc, menu_text_item, Item, List, PageInput},
    Frontend, GameMode,
};
use crate::frontend_tome::{MenuId, TomeView};
use magicka_runtime::end_game::STORY_CHALLENGES;
use magicka_runtime::ruleset::{
    VersusMode, VersusSettings, DEATHMATCH_SCORE_LIMITS, KRIETOR_TIME_LIMITS, LIVES,
    LUGGAGE_INTERVALS, TIME_LIMITS,
};
use magicka_runtime::text_geometry::TextAlign;
use std::path::Path;

/// (file under Levels/, `<Name>` key) of each `*.lvl` in `dir`, by file name
/// (DirectoryInfo.GetFiles order), story challenges other than Vietnam
/// excluded.
pub(crate) fn level_list(content: &str, dir: &str) -> Vec<(String, String)> {
    let path = Path::new(content).join("Levels").join(dir);
    let mut out: Vec<_> = std::fs::read_dir(path)
        .map(|d| d.flatten().map(|e| e.path()).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lvl")))
        // LevelManager: story challenges are listed separately, except
        // ch_vietnam, which is in both lists.
        .filter(|p| {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let stem = stem.to_ascii_lowercase();
            stem == "ch_vietnam" || !STORY_CHALLENGES.contains(&stem.as_str())
        })
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let name = text
                .split_once("<Name>")
                .and_then(|(_, rest)| rest.split_once("</Name>"))
                .map(|(name, _)| name.trim().to_owned())?;
            let file = format!("{dir}/{}", p.file_name()?.to_string_lossy());
            Some((file, name))
        })
        .collect();
    out.sort();
    out
}

const TOP: f32 = 200.0;
const ROWS: usize = 12;

/// A scrolling column of level names with Back below.
#[derive(Default)]
pub(crate) struct LevelPage {
    list: List,
    levels: Vec<(String, String)>,
    first: usize,
}

impl LevelPage {
    fn enter(&mut self, s: &Frontend, dir: &str) {
        self.levels = level_list(&s.content, dir);
        self.list.selected = Some(0);
        self.first = 0;
    }
    fn rows(
        &self,
        s: &Frontend,
        measure: &mut Measure,
        top: f32,
        out: &mut Vec<Widget>,
    ) -> Vec<Item> {
        let line = measure.line_height(Font::MenuOption) + 6.0;
        let mut items = Vec::new();
        for (row, (_, name)) in self.levels.iter().enumerate().skip(self.first).take(ROWS) {
            items.push(menu_text_item(
                out,
                measure,
                Font::MenuOption,
                loc(s, name),
                [512.0, top + line * (row - self.first) as f32],
                TextAlign::Center,
                self.list.selected == Some(row - self.first),
                true,
            ));
        }
        items
    }
    fn scroll(&mut self) {
        // Keep the highlighted row on the page.
        if let Some(sel) = self.list.selected {
            let rows = self.levels.len().min(ROWS);
            if sel + 1 == rows && self.first + rows < self.levels.len() {
                self.first += 1;
            } else if sel == 0 && self.first > 0 {
                self.first -= 1;
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct ChallengePage {
    levels: LevelPage,
}

impl ChallengePage {
    pub(crate) fn enter(&mut self, s: &Frontend) {
        self.levels.enter(s, "Challenges");
    }
    fn layout(&self, s: &Frontend, measure: &mut Measure) -> (Vec<Widget>, Vec<Item>) {
        let mut out = vec![Widget::text(
            Font::MenuTitle,
            loc(s, "#menu_main_02"),
            [512.0, 96.0],
            TextAlign::Center,
        )];
        let mut items = self.levels.rows(s, measure, TOP, &mut out);
        let back = items.len();
        out.push(back_button(self.levels.list.selected == Some(back)));
        items.push(back_item());
        (out, items)
    }
    pub(crate) fn update(
        &mut self,
        s: &mut Frontend,
        tome: &mut TomeView,
        input: &PageInput,
        measure: &mut Measure,
    ) {
        if input.back {
            tome.pop();
            return;
        }
        let (_, items) = self.layout(s, measure);
        let choice = self.levels.list.update(&items, input);
        self.levels.scroll();
        match choice {
            Some(i) if i + 1 == items.len() => tome.pop(),
            Some(i) => {
                // SetSettings(GameType.Challenge, level): no save slot.
                s.mode = GameMode::Challenge;
                s.fresh = true;
                s.online = false;
                s.campaign = self.levels.levels[self.levels.first + i].0.clone();
                tome.push(MenuId::Character, 1);
            }
            None => {}
        }
    }
    pub(crate) fn draw(&self, s: &Frontend, draw: &mut PageDraw, measure: &mut Measure) {
        let (out, _) = self.layout(s, measure);
        let mut page = draw.right(MenuId::Challenge);
        for widget in out {
            page.push(widget);
        }
    }
}

/// One Settings dropdown: label key, values and their display keys
/// (None: the number itself).
struct VersusOption {
    label: &'static str,
    values: &'static [i32],
    names: &'static [Option<&'static str>],
    default: usize,
}

const UNLIMITED: Option<&str> = Some("#menu_opt_alt_unlimited");
const TIME: VersusOption = VersusOption {
    label: "#menu_vs_03",
    values: &TIME_LIMITS,
    names: &[UNLIMITED, None, None, None, None],
    default: 1,
};
const LUGGAGE: VersusOption = VersusOption {
    label: "#opt_vs_luggageinterval",
    values: &LUGGAGE_INTERVALS,
    names: &[
        Some("#menu_opt_alt_04"),
        Some("#menu_opt_alt_05"),
        Some("#menu_opt_alt_06"),
        Some("#menu_opt_alt_02"),
    ],
    default: 1,
};
const TEAMS: VersusOption = VersusOption {
    label: "#menu_vs_09",
    values: &[0, 1],
    names: &[Some("#add_menu_no"), Some("#add_menu_yes")],
    default: 0,
};

/// DeathMatch/Brawl/Krietor.Settings constructors (option order).
fn mode_options(mode: VersusMode) -> Vec<VersusOption> {
    match mode {
        VersusMode::DeathMatch => vec![
            TIME,
            VersusOption {
                label: "#menu_vs_04",
                values: &DEATHMATCH_SCORE_LIMITS,
                names: &[UNLIMITED, None, None, None, None],
                default: 2,
            },
            TEAMS,
            LUGGAGE,
        ],
        VersusMode::Brawl => vec![
            TIME,
            VersusOption {
                label: "#opt_vs_lives",
                values: &LIVES,
                names: &[None, None, None, None, None],
                default: 1,
            },
            TEAMS,
            LUGGAGE,
        ],
        _ => vec![
            VersusOption {
                label: "#menu_vs_03",
                values: &KRIETOR_TIME_LIMITS,
                names: &[None, None],
                default: 1,
            },
            TEAMS,
        ],
    }
}

/// Versus: the game mode (mGameModeBox), its Settings dropdowns, then the
/// map; the chosen options become the `--versus` spec.
#[derive(Default)]
pub(crate) struct VersusPage {
    list: List,
    levels: Vec<(String, String)>,
    mode: usize,
    /// Selected index per option of the current mode.
    choices: Vec<usize>,
}

impl VersusPage {
    pub(crate) fn enter(&mut self, s: &Frontend) {
        self.levels = level_list(&s.content, "Versus");
        self.list.selected = Some(0);
        self.reset_choices();
    }
    fn reset_choices(&mut self) {
        self.choices = mode_options(VersusMode::LOBBY[self.mode])
            .iter()
            .map(|o| o.default)
            .collect();
    }
    fn settings(&self) -> VersusSettings {
        let mode = VersusMode::LOBBY[self.mode];
        let mut v = VersusSettings::defaults(mode);
        for (option, &i) in mode_options(mode).iter().zip(&self.choices) {
            let value = option.values[i];
            match option.label {
                "#menu_vs_03" => v.time_limit = value,
                "#menu_vs_04" => v.score_limit = value,
                "#opt_vs_lives" => v.lives = value,
                "#menu_vs_09" => v.teams = value != 0,
                _ => v.luggage_interval = value,
            }
        }
        v
    }
    fn layout(&self, s: &Frontend, measure: &mut Measure) -> (Vec<Widget>, Vec<Item>) {
        let mut out = vec![Widget::text(
            Font::MenuTitle,
            loc(s, "#menu_main_03"),
            [512.0, 96.0],
            TextAlign::Center,
        )];
        let selected = self.list.selected;
        let line = measure.line_height(Font::MenuOption) + 4.0;
        let mut items = Vec::new();
        let mut y = 160.0;
        let mut row = |out: &mut Vec<Widget>,
                       measure: &mut Measure,
                       text: String,
                       items: &mut Vec<Item>,
                       gap: f32| {
            y += gap;
            let index = items.len();
            items.push(menu_text_item(
                out,
                measure,
                Font::MenuOption,
                text,
                [512.0, y],
                TextAlign::Center,
                selected == Some(index),
                true,
            ));
            y += line;
        };
        let mode = VersusMode::LOBBY[self.mode];
        let label = mode.label().map_or_else(String::new, |k| loc(s, k));
        row(&mut out, measure, format!("< {label} >"), &mut items, 0.0);
        for (option, &i) in mode_options(mode).iter().zip(&self.choices) {
            let value = match option.names[i] {
                Some(key) => loc(s, key),
                None => option.values[i].to_string(),
            };
            row(
                &mut out,
                measure,
                format!("{}: < {value} >", loc(s, option.label)),
                &mut items,
                0.0,
            );
        }
        for (i, (_, name)) in self.levels.iter().enumerate() {
            let gap = if i == 0 { line * 0.5 } else { 0.0 };
            row(&mut out, measure, loc(s, name), &mut items, gap);
        }
        let back = items.len();
        out.push(back_button(selected == Some(back)));
        items.push(back_item());
        (out, items)
    }
    fn change(&mut self, row: usize, delta: isize) {
        if row == 0 {
            let n = VersusMode::LOBBY.len() as isize;
            self.mode = (self.mode as isize + delta).rem_euclid(n) as usize;
            self.reset_choices();
        } else if let Some(choice) = self.choices.get_mut(row - 1) {
            let n = mode_options(VersusMode::LOBBY[self.mode])[row - 1]
                .values
                .len() as isize;
            *choice = (*choice as isize + delta).rem_euclid(n) as usize;
        }
    }
    pub(crate) fn update(
        &mut self,
        s: &mut Frontend,
        tome: &mut TomeView,
        input: &PageInput,
        measure: &mut Measure,
    ) {
        if input.back {
            tome.pop();
            return;
        }
        let options = 1 + self.choices.len();
        if let Some(row) = self.list.selected.filter(|&r| r < options) {
            if input.direction[2] {
                self.change(row, -1);
            }
            if input.direction[3] {
                self.change(row, 1);
            }
        }
        let (_, items) = self.layout(s, measure);
        match self.list.update(&items, input) {
            Some(i) if i + 1 == items.len() => tome.pop(),
            Some(i) if i < options => self.change(i, 1),
            Some(i) => {
                let level = self.levels[i - options].0.clone();
                let v = self.settings();
                s.mode = GameMode::Versus;
                s.fresh = true;
                // A match needs a second player: host it.
                s.online = true;
                s.versus = Some(spec(&v));
                s.campaign = level;
                tome.push(MenuId::Character, 1);
            }
            None => {}
        }
    }
    pub(crate) fn draw(&self, s: &Frontend, draw: &mut PageDraw, measure: &mut Measure) {
        let (out, _) = self.layout(s, measure);
        let mut page = draw.right(MenuId::Versus);
        for widget in out {
            page.push(widget);
        }
    }
}

/// The `--versus` launch spec for these settings.
pub(crate) fn spec(v: &VersusSettings) -> String {
    let mode = match v.mode {
        VersusMode::DeathMatch => "deathmatch",
        VersusMode::Brawl => "brawl",
        VersusMode::King => "king",
        VersusMode::Pyrite => "pyrite",
        VersusMode::Krietor => "tourney",
    };
    format!(
        "{mode},time={},score={},lives={},luggage={}{}",
        v.time_limit,
        v.score_limit,
        v.lives,
        v.luggage_interval,
        if v.teams { ",teams" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::level_list;

    #[test]
    fn challenge_and_versus_lists_match_the_level_manager() {
        let root = magicka_content::retail_content_dir();
        if !root.is_dir() {
            return;
        }
        let root = root.to_str().unwrap();
        let challenges = level_list(root, "Challenges");
        assert_eq!(
            challenges.len(),
            18,
            "story challenges excluded except Vietnam"
        );
        assert!(challenges
            .iter()
            .all(|(f, n)| f.starts_with("Challenges/") && n.starts_with('#')));
        assert_eq!(level_list(root, "Versus").len(), 12);
    }

    #[test]
    fn versus_specs_round_trip_through_the_runtime_parser() {
        use magicka_runtime::ruleset::{VersusMode, VersusSettings};
        for mode in VersusMode::LOBBY {
            let mut v = VersusSettings::defaults(mode);
            v.teams = true;
            assert_eq!(VersusSettings::parse(&super::spec(&v)).unwrap(), v);
        }
    }
}
