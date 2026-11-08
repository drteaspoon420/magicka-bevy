//! GameScene rulesets (research/rulesets.md): the scene `<RuleSet>` node and
//! one deterministic state machine per native IRuleset. Each takes plain
//! facts (counts, level variables, statistics) and returns plain commands or
//! result sheets, so scene hosts and future script bindings can drive them
//! without access to scene internals.
use crate::scene_script::XmlElement;

/// Actions whose native class has an `Area` property (WaveActions sets it
/// by reflection).
pub const AREA_ACTIONS: &[&str] = &[
    "assignitem",
    "activatedispenser",
    "remove",
    "camerashake",
    "giveorder",
    "spawnmagick",
    "damage",
    "spawnclonewizards",
    "cameralock",
    "spawn",
    "playsound",
    "spawneffect",
    "scrollscore",
    "overkill",
    "kill",
    "releasegrip",
    "spawnitem",
    "setposition",
    "spawnboss",
    "spawndispenser",
    "spawnanimprop",
    "setfaction",
    "spawnelemental",
    "spawngenericboss",
    "spawnphys",
];

/// The placeholder area name of wave `w`'s WaveActions `a`.
pub fn wave_area_placeholder(w: usize, a: usize) -> String {
    format!("#survivalarea{w}_{a}")
}

/// Rulesets (the type attribute).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulesetKind {
    Survival,
    TimedObjective,
    Versus,
}

#[derive(Clone, Debug)]
pub struct WaveActionsLayout {
    /// The hidden trigger holding the actions.
    pub trigger: i32,
    /// Native WaveActions with no actions never keep a wave executing.
    pub has_actions: bool,
    /// The authored area (None: "any" or absent, picked at Initialize).
    pub area: Option<String>,
    pub placeholder: String,
    pub delay: f32,
    /// (template, amount) of each Spawn (GetTotalHitPoins).
    pub spawns: Vec<(String, u32)>,
}

#[derive(Clone, Debug)]
pub struct RulesetLayout {
    pub kind: RulesetKind,
    pub node: XmlElement,
    /// spawnAreas (Survival).
    pub areas: Vec<String>,
    /// Survival waves (scene `<wave>` nodes after the ruleset).
    pub waves: Vec<Vec<WaveActionsLayout>>,
}

impl RulesetLayout {
    pub fn read(node: &XmlElement) -> Option<Self> {
        let kind = node
            .attributes
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("type"))
            .and_then(|(_, v)| match v.to_ascii_lowercase().as_str() {
                "survival" => Some(RulesetKind::Survival),
                "timedobjective" => Some(RulesetKind::TimedObjective),
                "versus" => Some(RulesetKind::Versus),
                _ => None,
            })?;
        let areas = node
            .children
            .iter()
            .filter(|c| c.name.eq_ignore_ascii_case("spawnAreas"))
            .flat_map(|c| c.children.iter().map(|a| a.inner_text.trim().to_owned()))
            .collect();
        Some(Self {
            kind,
            node: node.clone(),
            areas,
            waves: Vec::new(),
        })
    }
}

mod packs;
mod packs_native;
mod survival;
mod timed_objective;
mod versus;

pub use packs::*;
pub use survival::*;
pub use timed_objective::*;
pub use versus::*;

/// InGameMenuSurvivalStatistics: one playing player's row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurvivalPlayerResult {
    pub slot: usize,
    pub kills: u32,
    pub deaths: u32,
}

/// InGameMenuSurvivalStatistics.OnEnter's sheet and leaderboard entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurvivalResults {
    pub players: Vec<SurvivalPlayerResult>,
    /// StatisticsManager.SurvivalTotalScore (counted up on the sheet).
    pub total_score: i32,
    /// WaveIndex (mCurrentWaveNr, 1-based) as leaderboard Data1.
    pub wave: usize,
}

/// InGameMenuVersusStatistics.OnEnter's inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersusResults {
    pub mode: VersusMode,
    pub teams: bool,
    /// Per-player kills and deaths (StatisticsManager.GetScore).
    pub players: Vec<SurvivalPlayerResult>,
    /// GetScores / GetTeamScores (None: King, Pyrite).
    pub scores: Option<(crate::slots::PerSlot<i16>, [i16; 2])>,
}

/// The statistics sheet PlayState opens when a non-campaign ruleset ends.
#[derive(Clone, Debug, PartialEq)]
pub enum RulesetResults {
    Survival(SurvivalResults),
    TimedObjective(TimedObjectiveResults),
    Versus(VersusResults),
}

/// GameScene.RuleSet: the scene's live ruleset.
#[derive(Clone, Debug)]
pub enum SceneRuleset {
    Survival(Box<SurvivalRuleset>),
    TimedObjective(Box<TimedObjectiveRuleset>),
    Versus(Box<VersusRuleset>),
}

impl SceneRuleset {
    pub fn kind(&self) -> RulesetKind {
        match self {
            Self::Survival(_) => RulesetKind::Survival,
            Self::TimedObjective(_) => RulesetKind::TimedObjective,
            Self::Versus(_) => RulesetKind::Versus,
        }
    }
    pub fn survival(&self) -> Option<&SurvivalRuleset> {
        match self {
            Self::Survival(r) => Some(r),
            _ => None,
        }
    }
    pub fn survival_mut(&mut self) -> Option<&mut SurvivalRuleset> {
        match self {
            Self::Survival(r) => Some(r),
            _ => None,
        }
    }
    pub fn timed_objective(&self) -> Option<&TimedObjectiveRuleset> {
        match self {
            Self::TimedObjective(r) => Some(r),
            _ => None,
        }
    }
    pub fn timed_objective_mut(&mut self) -> Option<&mut TimedObjectiveRuleset> {
        match self {
            Self::TimedObjective(r) => Some(r),
            _ => None,
        }
    }
    pub fn versus(&self) -> Option<&VersusRuleset> {
        match self {
            Self::Versus(r) => Some(r),
            _ => None,
        }
    }
    pub fn versus_mut(&mut self) -> Option<&mut VersusRuleset> {
        match self {
            Self::Versus(r) => Some(r),
            _ => None,
        }
    }
}
