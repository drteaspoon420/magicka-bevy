//! `Magicka.Levels.TimedObjectiveRuleset` (research/rulesets.md): a played
//! timer, level counters as objectives, the elapsed-time HUD text and the
//! InGameMenuTimedObjectiveStatistics score sheet. Pure: every input is the
//! level's counters/timers, every output is plain data.
use super::*;
use crate::level_variables::{LevelVariables, TimerFrame};
use crate::scene_loading::custom_hash;

/// LOC_TIME_ELAPSED ("#1;" is replaced by the formatted time).
pub const TIME_ELAPSED_TEXT: &str = "#challenge_time";
/// Constructor defaults.
pub const DEFAULT_TIMER: &str = "timestorage";
pub const DEFAULT_TEXTURE: &str = "UI/HUD/Dialog_Say";
pub const DEFAULT_BORDER_SIZE: i32 = 32;
/// RenderData.ZIndex.
pub const HUD_Z_INDEX: i32 = 205;

/// One `<objective>`: level counter `counter` shown as `name`.
#[derive(Clone, Debug, PartialEq)]
pub struct Objective {
    /// Authored localization key (NameID is its lowercase custom hash).
    pub name: String,
    pub name_id: i32,
    pub counter: i32,
    /// Last observed counter value (Update copies one change per frame).
    pub value: i32,
    pub max_value: i32,
    pub score: i32,
}

/// `<timeBonus>`: a multiplier interpolated over the played time in minutes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimeBonus {
    pub counter: i32,
    pub min_time: f32,
    pub max_time: f32,
    pub min_time_bonus: f32,
    pub max_time_bonus: f32,
}

impl TimeBonus {
    /// TimeBonus.GetBonus: clamp, then lerp from the max-time bonus (slow)
    /// to the min-time bonus (fast).
    pub fn bonus(&self, minutes: f32) -> f32 {
        let t = if minutes > self.max_time {
            self.max_time
        } else if self.min_time > minutes {
            self.min_time
        } else {
            minutes
        };
        let amount = 1.0 - (t - self.min_time) / (self.max_time - self.min_time);
        self.max_time_bonus + (self.min_time_bonus - self.max_time_bonus) * amount
    }
}

/// One statistics row.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectiveResult {
    /// The authored localization key (LanguageManager.GetString(NameID)).
    pub name: String,
    pub value: i32,
    /// "value/max" is shown only for a positive max.
    pub max_value: i32,
    pub score: i32,
}

/// InGameMenuTimedObjectiveStatistics.OnEnter's sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct TimedObjectiveResults {
    pub seconds: f32,
    pub time_text: String,
    /// TimeSuccess: mission complete rather than failed.
    pub success: bool,
    pub objectives: Vec<ObjectiveResult>,
    /// No objective below its max ("goodcompany").
    pub all_complete: bool,
    pub bonus: i32,
    /// Objective scores plus the bonus (the leaderboard score).
    pub total: i32,
}

#[derive(Clone, Debug)]
pub struct TimedObjectiveRuleset {
    pub timer: i32,
    pub texture: String,
    pub border_size: i32,
    pub objectives: Vec<Objective>,
    pub time_bonus: TimeBonus,
    time_elapsed: f32,
    last_updated_elapsed: f32,
    /// The HUD clock last sent to RenderData.SetTime (see `hud_text`).
    pub time_text: String,
    /// Server-side RulesetMessage pending (offline: informational only).
    pub network_dirty: bool,
}

fn attribute<'a>(node: &'a XmlElement, name: &str) -> Option<&'a str> {
    node.attributes
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn lower_id(value: &str) -> i32 {
    custom_hash(&value.to_lowercase())
}

/// The statistics/HUD clock: TimeSpan.FromSeconds rounds to milliseconds,
/// then prints `H:MM:SS` (hours only from one hour; TimeSpan.Hours is mod 24).
pub fn format_time(seconds: f32) -> String {
    let ms = (seconds as f64 * 1000.0).round() as i64;
    let total = ms.div_euclid(1000);
    let (h, m, s) = ((total / 3600) % 24, (total / 60) % 60, total % 60);
    if seconds >= 3600.0 {
        format!("{h}:{m:02}:{s:02}")
    } else if seconds >= 60.0 {
        format!("0:{m:02}:{s:02}")
    } else {
        format!("0:00:{s:02}")
    }
}

impl TimedObjectiveRuleset {
    /// The constructor's XML parse. Objectives need both a name and an id.
    pub fn read(node: &XmlElement) -> Self {
        let mut r = Self {
            timer: lower_id(attribute(node, "timer").unwrap_or(DEFAULT_TIMER)),
            texture: attribute(node, "texture")
                .unwrap_or(DEFAULT_TEXTURE)
                .to_owned(),
            border_size: attribute(node, "bordersize")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(DEFAULT_BORDER_SIZE),
            objectives: Vec::new(),
            time_bonus: TimeBonus::default(),
            time_elapsed: 0.0,
            last_updated_elapsed: 0.0,
            time_text: String::new(),
            network_dirty: false,
        };
        for child in &node.children {
            if child.name.eq_ignore_ascii_case("objective") {
                let name = attribute(child, "name").filter(|v| !v.is_empty());
                let id = attribute(child, "id").filter(|v| !v.is_empty());
                let (Some(name), Some(id)) = (name, id) else {
                    continue;
                };
                r.objectives.push(Objective {
                    name: name.to_owned(),
                    name_id: lower_id(name),
                    counter: lower_id(id),
                    value: 0,
                    max_value: attribute(child, "max")
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0),
                    score: attribute(child, "score")
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(1),
                });
            } else if child.name.eq_ignore_ascii_case("timebonus") {
                let number = |name, default| {
                    attribute(child, name)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(default)
                };
                r.time_bonus = TimeBonus {
                    counter: attribute(child, "counter").map_or(0, lower_id),
                    min_time: number("minTime", 0.0),
                    max_time: number("maxTime", 0.0),
                    min_time_bonus: number("minTimeBonus", 1.0),
                    max_time_bonus: number("maxTimeBonus", 1.0),
                };
            }
        }
        r
    }

    /// Initialize: the played timer starts at 0 and objectives read their
    /// counters.
    pub fn initialize(&mut self, variables: &mut LevelVariables) {
        self.time_text = "0:00:00".to_owned();
        self.network_dirty = true;
        variables.add_timer(self.timer, true, 0.0);
        self.time_elapsed = variables.timer(self.timer);
        self.last_updated_elapsed = 0.0;
        for o in &mut self.objectives {
            o.value = variables.counter(o.counter);
        }
    }

    /// Update (server/offline): the elapsed time follows the timer while
    /// play runs, at most one changed objective is copied per frame and the
    /// HUD text refreshes once a whole second has elapsed. Returns whether
    /// the HUD box is drawn this frame (not during cutscenes).
    pub fn update(&mut self, variables: &LevelVariables, frame: TimerFrame) -> bool {
        if !frame.paused && !frame.cutscene && !frame.game_ended {
            self.time_elapsed = variables.timer(self.timer);
        }
        if let Some(o) = self
            .objectives
            .iter_mut()
            .find(|o| variables.counter(o.counter) != o.value)
        {
            self.network_dirty = true;
            o.value = variables.counter(o.counter);
        }
        if self.time_elapsed - self.last_updated_elapsed >= 1.0 {
            self.last_updated_elapsed = self.time_elapsed;
            self.time_text = format_time(self.time_elapsed);
        }
        !frame.cutscene
    }

    /// The HUD box text: localized LOC_TIME_ELAPSED with `#1;` replaced.
    pub fn hud_text(&self, template: &str) -> String {
        template.replace("#1;", &self.time_text)
    }

    pub fn time_elapsed(&self) -> f32 {
        self.time_elapsed
    }

    /// TimeSuccess: the time-bonus counter is positive.
    pub fn time_success(&self, variables: &LevelVariables) -> bool {
        variables.counter(self.time_bonus.counter) > 0
    }

    /// GetBonusMultiplier: bonus(minutes) × the time-bonus counter.
    pub fn bonus_multiplier(&self, variables: &LevelVariables) -> f32 {
        self.time_bonus.bonus(variables.timer(self.timer) / 60.0)
            * variables.counter(self.time_bonus.counter) as f32
    }

    /// InGameMenuTimedObjectiveStatistics.OnEnter's sheet.
    pub fn results(&self, variables: &LevelVariables) -> TimedObjectiveResults {
        let seconds = variables.timer(self.timer);
        let mut sum = 0i32;
        let mut all_complete = true;
        let objectives = self
            .objectives
            .iter()
            .map(|o| {
                let value = variables.counter(o.counter);
                let score = value.wrapping_mul(o.score);
                sum = sum.wrapping_add(score);
                all_complete &= value >= o.max_value;
                ObjectiveResult {
                    name: o.name.clone(),
                    value,
                    max_value: o.max_value,
                    score,
                }
            })
            .collect();
        // C# (int) of a float: saturating in Rust; native out-of-range is
        // unspecified (UNKNOWN) and not reached by authored values.
        let bonus = (sum as f32 * self.bonus_multiplier(variables)) as i32;
        TimedObjectiveResults {
            seconds,
            time_text: format_time(seconds),
            success: self.time_success(variables),
            objectives,
            all_complete,
            bonus,
            total: sum.wrapping_add(bonus),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_script::SceneScriptLayout;

    fn vietnam() -> TimedObjectiveRuleset {
        let layout = SceneScriptLayout::read(
            br##"<Scene><RuleSet type="timedobjective" texture="UI/HUD/Dialog_Say_Old" bordersize="32" timer="timeTimer">
            <timeBonus minTime="0" minTimeBonus="3" maxTime="30" maxTimeBonus="1" counter="EvacuatedCounter" />
            <objective name="#Vtn_radiotowers" id="RadioTowerCounter" max="2" score="550"/>
            <objective name="#Vtn_extraction" id="EvacuatedCounter" max="1" score="2000"/>
            <objective name="" id="Ignored" max="1"/>
            </RuleSet></Scene>"##,
            false,
        )
        .unwrap();
        TimedObjectiveRuleset::read(&layout.ruleset.unwrap().node)
    }

    #[test]
    fn reads_objectives_and_time_bonus() {
        let r = vietnam();
        assert_eq!(r.timer, custom_hash("timetimer"));
        assert_eq!(r.texture, "UI/HUD/Dialog_Say_Old");
        assert_eq!(r.objectives.len(), 2, "nameless objective skipped");
        assert_eq!(r.objectives[0].counter, custom_hash("radiotowercounter"));
        assert_eq!(r.objectives[0].name_id, custom_hash("#vtn_radiotowers"));
        assert_eq!(r.time_bonus.counter, custom_hash("evacuatedcounter"));
        assert_eq!(r.time_bonus.bonus(0.0), 3.0);
        assert_eq!(r.time_bonus.bonus(15.0), 2.0);
        assert_eq!(r.time_bonus.bonus(45.0), 1.0);
    }

    #[test]
    fn clock_text_matches_the_three_native_formats() {
        assert_eq!(format_time(9.6), "0:00:09");
        assert_eq!(format_time(61.0), "0:01:01");
        assert_eq!(format_time(3725.0), "1:02:05");
    }

    #[test]
    fn timer_runs_with_play_and_scores_include_the_time_bonus() {
        let mut r = vietnam();
        let mut v = LevelVariables::default();
        r.initialize(&mut v);
        assert_eq!(r.hud_text("Time: #1;"), "Time: 0:00:00");
        let run = TimerFrame::default();
        for _ in 0..25 {
            v.update(0.1, run);
            r.update(&v, run);
        }
        assert_eq!(r.time_text, "0:00:02");
        let cutscene = TimerFrame {
            cutscene: true,
            ..run
        };
        v.update(100.0, cutscene);
        assert!(!r.update(&v, cutscene), "hidden in cutscenes");
        assert_eq!(r.time_text, "0:00:02", "played timer holds");
        v.set_counter(custom_hash("radiotowercounter"), 2);
        v.set_counter(custom_hash("evacuatedcounter"), 1);
        r.update(&v, run);
        assert_eq!(r.objectives[0].value, 2);
        assert_eq!(r.objectives[1].value, 0, "one change copied per frame");
        r.update(&v, run);
        assert_eq!(r.objectives[1].value, 1);
        v.set_timer(r.timer, 15.0 * 60.0);
        let sheet = r.results(&v);
        assert!(sheet.success && sheet.all_complete);
        assert_eq!(sheet.objectives[0].score, 1100);
        assert_eq!(sheet.bonus, (3100.0f32 * 2.0) as i32);
        assert_eq!(sheet.total, 3100 + 6200);
        assert_eq!(sheet.time_text, "0:15:00");
    }
}
