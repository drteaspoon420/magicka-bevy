//! PlayState end-of-game handling for the offline campaign (UpdateMiscA and
//! Endgame). When every playing avatar is dead it calls Endgame(Defeat, 3 s);
//! the `endgame` script action calls Endgame(type, freeze, phony, 0). The
//! condition's text fades in once the timer reaches 0 and out from −2 s.
//! Once the timer is below −3 s with the text faded:
//! - a phony end clears itself;
//! - a campaign LevelComplete opens the end-of-level step (here: advance to
//!   the next campaign level);
//! - anything else restarts from the checkpoint.
//!
//! A living avatar cancels a pending Defeat. See research/end-game.md and
//! research/campaign-progression.md.

/// Defines.LOC_GAME_DEFEATED.
pub const DEFEATED_TEXT: &str = "#add_game_defeated";
/// Defines.LOC_GAME_LEVCOMP.
pub const LEVEL_COMPLETE_TEXT: &str = "#add_game_level_complete";
/// Defines.LOC_GAME_VICTORY.
pub const VICTORY_TEXT: &str = "#add_game_victory";

/// LevelManager.mCampaign (vanilla campaign order).
pub const VANILLA_CAMPAIGN: [&str; 12] = [
    "Wizard_Castle.lvl",
    "Mountaindale.lvl",
    "Highlands.lvl",
    "Havindr.lvl",
    "Battlefield.lvl",
    "EndOfWorld.lvl",
    "Wizard_Castle2.lvl",
    "Mines.lvl",
    "Swamp.lvl",
    "Niflheim.lvl",
    "Ruins.lvl",
    "EndOfWorld2.lvl",
];

/// LevelManager.mMythosCampaign (The Stars Are Left).
pub const MYTHOS_CAMPAIGN: [&str; 3] = [
    "Tsar/Tsar_WizardCastle.lvl",
    "Tsar/Tsar_Mountaindale.lvl",
    "Tsar/Tsar_Rlyeh.lvl",
];

/// LevelManager.mDungeonsCampaign (story challenges, SubMenuCutscene levels
/// -1 and -2).
pub const DUNGEONS_CAMPAIGN: [&str; 2] = [
    "Challenges/ch_dungeons_ch1.lvl",
    "Challenges/ch_dungeons_ch2.lvl",
];

/// LevelManager.hcStoryChallanges (level file stems).
pub const STORY_CHALLENGES: [&str; 4] = [
    "ch_vietnam",
    "ch_osotc",
    "ch_dungeons_ch1",
    "ch_dungeons_ch2",
];

/// Magicka.GameLogic.GameStates.GameType (the values are the native flags).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GameType {
    #[default]
    Campaign = 1,
    Challenge = 2,
    Versus = 4,
    Mythos = 8,
    StoryChallenge = 22,
}

impl GameType {
    /// LevelManager.GetLevel(fileName, out GameType): campaign lists, then
    /// the Challenges/ (story challenges by name) and Versus/ directories.
    /// Unlisted levels (custom) are treated as campaign levels.
    pub fn for_level(level: &str) -> Self {
        let level = level.replace('\\', "/");
        match campaign_position(&level) {
            Some((Campaign::Vanilla, _)) => return Self::Campaign,
            Some((Campaign::Mythos, _)) => return Self::Mythos,
            None => {}
        }
        let lower = level.to_ascii_lowercase();
        let stem = std::path::Path::new(&lower)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if lower.starts_with("challenges/") {
            if STORY_CHALLENGES.contains(&stem) {
                Self::StoryChallenge
            } else {
                Self::Challenge
            }
        } else if lower.starts_with("versus/") {
            Self::Versus
        } else {
            Self::Campaign
        }
    }
    pub fn is_campaign(self) -> bool {
        matches!(self, Self::Campaign | Self::Mythos)
    }
}

/// A campaign: GameType.Campaign or GameType.Mythos.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Campaign {
    Vanilla,
    Mythos,
}

impl Campaign {
    pub fn levels(self) -> &'static [&'static str] {
        match self {
            Campaign::Vanilla => &VANILLA_CAMPAIGN,
            Campaign::Mythos => &MYTHOS_CAMPAIGN,
        }
    }
}

/// The campaign and index of a level file, if it is a campaign level.
pub fn campaign_position(level: &str) -> Option<(Campaign, usize)> {
    [Campaign::Vanilla, Campaign::Mythos]
        .into_iter()
        .find_map(|c| {
            c.levels()
                .iter()
                .position(|l| l.eq_ignore_ascii_case(level))
                .map(|i| (c, i))
        })
}

/// The campaign level after `level` (SaveData.Level + 1, wrapping to 0 with
/// Looped), or `None` for a level outside the campaigns.
pub fn next_campaign_level(level: &str) -> Option<(&'static str, bool)> {
    let (campaign, i) = campaign_position(level)?;
    let levels = campaign.levels();
    let next = i + 1;
    if next >= levels.len() {
        Some((levels[0], true))
    } else {
        Some((levels[next], false))
    }
}

/// Magicka.Levels.EndGameCondition.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum EndGameCondition {
    #[default]
    None = 0,
    Victory,
    LevelComplete,
    Defeat,
    VersusPlayer,
    VersusTeam,
    EightyeightMilesPerHour,
    ChallengeExit,
    EndOffGame,
    Disconnected,
    ToBeContinued,
}

/// Lowercased enum names for script parsing (values are the indices).
pub const CONDITIONS: &[(&str, i32)] = &[
    ("none", 0),
    ("victory", 1),
    ("levelcomplete", 2),
    ("defeat", 3),
    ("versusplayer", 4),
    ("versusteam", 5),
    ("eightyeightmilesperhour", 6),
    ("challengeexit", 7),
    ("endoffgame", 8),
    ("disconnected", 9),
    ("tobecontinued", 10),
];

impl EndGameCondition {
    pub fn from_value(v: u8) -> Self {
        use EndGameCondition::*;
        [
            None,
            Victory,
            LevelComplete,
            Defeat,
            VersusPlayer,
            VersusTeam,
            EightyeightMilesPerHour,
            ChallengeExit,
            EndOffGame,
            Disconnected,
            ToBeContinued,
        ]
        .get(v as usize)
        .copied()
        .unwrap_or(None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EndGame {
    pub condition: EndGameCondition,
    /// mEndGameTimer (3 while no condition).
    pub timer: f32,
    /// mEndGameTextAlpha (RenderData.PostTextAlpha).
    pub text_alpha: f32,
    /// mEndGameActive: the end action ran.
    pub active: bool,
    /// Restart(this, RestartType.Checkpoint) requested.
    pub restart: bool,
    /// Campaign LevelComplete reached SubMenuEndGame (advance the campaign).
    pub level_complete: bool,
    /// mEndGamePhony.
    pub phony: bool,
    /// ControlManager.LimitInput(this) from a freezing Endgame.
    pub input_limited: bool,
    /// PlayState.mGameType.
    pub game_type: GameType,
    /// The current scene has a ruleset (GameScene.RuleSet != null).
    pub has_ruleset: bool,
    /// A non-campaign ruleset ended: InGameMenu shows the ruleset's
    /// statistics sheet and play pauses (mIsPaused).
    pub statistics: bool,
    /// ChallengeExit or the ToBeContinued transition ended: SubMenuEndGame
    /// (leave play for the menu).
    pub exit_to_menu: bool,
    /// ToBeContinued's 8 s fade to black in progress (elapsed seconds).
    pub to_be_continued_fade: Option<f32>,
}

impl Default for EndGame {
    fn default() -> Self {
        Self {
            condition: EndGameCondition::None,
            timer: 3.0,
            text_alpha: 0.0,
            active: false,
            restart: false,
            level_complete: false,
            phony: false,
            input_limited: false,
            game_type: GameType::Campaign,
            has_ruleset: false,
            statistics: false,
            exit_to_menu: false,
            to_be_continued_fade: None,
        }
    }
}

/// ToBeContinued: BeginTransition(Fade, black, 8 s).
pub const TO_BE_CONTINUED_FADE: f32 = 8.0;

impl EndGame {
    /// PlayState.Endgame: the timer only ever shortens.
    pub fn begin(&mut self, condition: EndGameCondition, freeze: bool, phony: bool, time: f32) {
        self.phony = phony;
        self.condition = condition;
        self.timer = self.timer.min(time);
        if condition == EndGameCondition::ToBeContinued && self.to_be_continued_fade.is_none() {
            self.to_be_continued_fade = Some(0.0);
        }
        if freeze {
            self.input_limited = true;
        }
    }

    /// The localization key of the text RenderData shows (None: no text).
    pub fn text(&self) -> Option<&'static str> {
        match self.condition {
            EndGameCondition::Defeat => Some(DEFEATED_TEXT),
            EndGameCondition::LevelComplete => Some(LEVEL_COMPLETE_TEXT),
            EndGameCondition::Victory => Some(VICTORY_TEXT),
            _ => None,
        }
    }

    /// Phony LevelComplete: UpdateMiscA freezes animated level parts and
    /// entities (iDeltaTime2 = 0) until the phony end clears itself.
    pub fn freezes_entities(&self) -> bool {
        self.phony && self.condition == EndGameCondition::LevelComplete
    }

    /// The UpdateMiscA end-game section for one frame. `all_dead`: no
    /// connected player has a living avatar. End-game music is not
    /// presented, so the music gates read as "not playing" (the original
    /// keeps the text until the music has finished).
    pub fn update(&mut self, dt: f32, all_dead: bool) {
        // OnTransitionEnd: input unlimited, SubMenuEndGame, PopState.
        if let Some(t) = &mut self.to_be_continued_fade {
            *t += dt;
            if *t >= TO_BE_CONTINUED_FADE && !self.exit_to_menu {
                self.input_limited = false;
                self.exit_to_menu = true;
            }
        }
        if self.timer > 0.0 {
            if all_dead && self.game_type != GameType::Versus {
                if self.condition != EndGameCondition::Defeat {
                    self.begin(EndGameCondition::Defeat, false, false, 3.0);
                }
            } else if self.condition == EndGameCondition::Defeat {
                self.condition = EndGameCondition::None;
            }
        }
        if self.condition != EndGameCondition::None {
            if self.timer <= -2.0 && self.to_be_continued_fade.is_none() {
                self.text_alpha = (self.text_alpha - dt).max(0.0);
            } else if self.timer <= 0.0 {
                self.text_alpha = (self.text_alpha + dt).min(1.0);
            }
            // ToBeContinued starts the victory music, which (INFERRED) holds
            // the text past the 8 s transition that leaves play.
            let music_holds = self.to_be_continued_fade.is_some();
            if !self.active && self.condition == EndGameCondition::ChallengeExit {
                self.active = true;
                self.input_limited = false;
                self.exit_to_menu = true;
            } else if !self.active && self.timer < -3.0 && self.text_alpha <= 0.0 && !music_holds {
                if self.phony {
                    self.condition = EndGameCondition::None;
                    self.phony = false;
                    self.input_limited = false;
                } else {
                    self.active = true;
                    if !self.game_type.is_campaign() {
                        if self.has_ruleset {
                            self.statistics = true;
                        } else {
                            self.restart = true;
                        }
                    } else if self.condition == EndGameCondition::LevelComplete {
                        // Campaign: SubMenuEndGame with input unlimited.
                        self.input_limited = false;
                        self.level_complete = true;
                    } else {
                        self.restart = true;
                    }
                }
            }
            self.timer -= dt;
        } else {
            self.text_alpha = 0.0;
            self.timer = 3.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(e: &mut EndGame, seconds: f32) {
        for _ in 0..(seconds / 0.05) as usize {
            e.update(0.05, false);
        }
    }

    #[test]
    fn game_types_follow_the_level_manager_lists() {
        assert_eq!(GameType::for_level("Havindr.lvl"), GameType::Campaign);
        assert_eq!(GameType::for_level("Tsar/Tsar_Rlyeh.lvl"), GameType::Mythos);
        assert_eq!(
            GameType::for_level("Challenges/ch_glade.lvl"),
            GameType::Challenge
        );
        assert_eq!(
            GameType::for_level("Challenges/ch_vietnam.lvl"),
            GameType::StoryChallenge
        );
        assert_eq!(GameType::for_level("Versus/vs_boat.lvl"), GameType::Versus);
    }

    #[test]
    fn non_campaign_rulesets_open_statistics_instead_of_advancing() {
        let mut e = EndGame {
            game_type: GameType::StoryChallenge,
            has_ruleset: true,
            ..Default::default()
        };
        e.begin(EndGameCondition::LevelComplete, true, false, 0.0);
        run(&mut e, 3.2);
        assert!(e.statistics && !e.level_complete && !e.restart);
        let mut e = EndGame {
            game_type: GameType::Challenge,
            ..Default::default()
        };
        e.begin(EndGameCondition::Victory, true, false, 0.0);
        run(&mut e, 3.2);
        assert!(e.restart, "no ruleset: Restart(Checkpoint)");
    }

    #[test]
    fn challenge_exit_leaves_at_once_and_versus_has_no_defeat() {
        let mut e = EndGame::default();
        e.begin(EndGameCondition::ChallengeExit, false, false, 0.0);
        e.update(0.05, false);
        assert!(e.exit_to_menu && e.active);
        let mut e = EndGame {
            game_type: GameType::Versus,
            ..Default::default()
        };
        e.update(0.05, true);
        assert_eq!(e.condition, EndGameCondition::None);
    }

    #[test]
    fn to_be_continued_fades_to_the_menu_without_restarting() {
        let mut e = EndGame {
            game_type: GameType::StoryChallenge,
            ..Default::default()
        };
        e.begin(EndGameCondition::ToBeContinued, true, false, 0.0);
        run(&mut e, 7.9);
        assert!(!e.exit_to_menu && !e.restart);
        run(&mut e, 0.2);
        assert!(e.exit_to_menu && !e.restart && !e.input_limited);
    }
}
