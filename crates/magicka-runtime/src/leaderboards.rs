//! StatisticsManager local leaderboards (research/rulesets.md): per
//! challenge (LevelManager.Challenges order) up to eight LeaderBoardData
//! entries sorted by ComparerScoreBeforeData. Native keeps them in memory;
//! the rewrite persists them because play runs in its own process.
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// LeaderBoardData.MAX_LOCAL_LEADERBOARDS.
pub const MAX_LOCAL: usize = 8;

/// LeaderBoardData.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderboardEntry {
    pub score: i32,
    /// Joined GamerTags of the playing players.
    pub name: String,
    /// Survival: the wave (byte); TimedObjective: the time's float bits.
    pub data1: i32,
}

impl LeaderboardEntry {
    /// The TimedObjective time stored in Data1.
    pub fn time(&self) -> f32 {
        f32::from_bits(self.data1 as u32)
    }
}

/// ComparerScoreBeforeData: score descending, then Data1 as a float
/// descending.
fn compare(x: &LeaderboardEntry, y: &LeaderboardEntry) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    if x.score == y.score {
        let (a, b) = (x.time(), y.time());
        if a > b {
            Less
        } else if a < b {
            Greater
        } else {
            Equal
        }
    } else {
        y.score.cmp(&x.score)
    }
}

/// LevelManager.Challenges: every Challenges/*.lvl except the story
/// challenges, with ch_vietnam listed in both.
pub fn is_leaderboard_challenge(level_file: &str) -> bool {
    let lower = level_file.replace('\\', "/").to_ascii_lowercase();
    let Some(name) = lower.strip_prefix("challenges/") else {
        return false;
    };
    let stem = name.trim_end_matches(".lvl");
    stem == "ch_vietnam" || !crate::end_game::STORY_CHALLENGES.contains(&stem)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalLeaderboards {
    /// Level file (Challenges/x.lvl, lowercase) → entries.
    pub boards: BTreeMap<String, Vec<LeaderboardEntry>>,
}

impl LocalLeaderboards {
    /// AddLocalEntry: insert, sort (stable List.Sort comparer order is not
    /// guaranteed natively; ties keep insertion order here), keep eight.
    pub fn add(&mut self, level_file: &str, entry: LeaderboardEntry) {
        let board = self
            .boards
            .entry(level_file.to_ascii_lowercase())
            .or_default();
        board.push(entry);
        board.sort_by(compare);
        board.truncate(MAX_LOCAL);
    }

    pub fn board(&self, level_file: &str) -> &[LeaderboardEntry] {
        self.boards
            .get(&level_file.to_ascii_lowercase())
            .map_or(&[], Vec::as_slice)
    }

    /// `{"level": [{"score", "name", "data1"}, …]}`; unreadable files are
    /// empty boards.
    pub fn load(path: &std::path::Path) -> Self {
        let Some(Value::Object(map)) = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        else {
            return Self::default();
        };
        let boards = map
            .into_iter()
            .map(|(level, entries)| {
                let entries = entries
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| {
                        Some(LeaderboardEntry {
                            score: e["score"].as_i64()? as i32,
                            name: e["name"].as_str()?.to_owned(),
                            data1: e["data1"].as_i64()? as i32,
                        })
                    })
                    .take(MAX_LOCAL)
                    .collect();
                (level, entries)
            })
            .collect();
        Self { boards }
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let value: serde_json::Map<String, Value> = self
            .boards
            .iter()
            .map(|(level, entries)| {
                let list = entries
                    .iter()
                    .map(|e| json!({"score": e.score, "name": e.name, "data1": e.data1}))
                    .collect();
                (level.clone(), Value::Array(list))
            })
            .collect();
        let bytes = serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?;
        std::fs::write(path, bytes)
    }

    /// The statistics menus' entry: Survival skips a zero score and stores
    /// the wave; TimedObjective stores the time's bits.
    pub fn entry_for(
        results: &crate::ruleset::RulesetResults,
        names: &[String],
    ) -> Option<LeaderboardEntry> {
        let name = names.join(", ");
        match results {
            crate::ruleset::RulesetResults::Survival(s) => {
                (s.total_score != 0).then(|| LeaderboardEntry {
                    score: s.total_score,
                    name,
                    data1: i32::from(s.wave as u8),
                })
            }
            crate::ruleset::RulesetResults::TimedObjective(t) => Some(LeaderboardEntry {
                score: t.total,
                name,
                data1: t.seconds.to_bits() as i32,
            }),
            crate::ruleset::RulesetResults::Versus(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(score: i32, data: f32) -> LeaderboardEntry {
        LeaderboardEntry {
            score,
            name: "P".into(),
            data1: data.to_bits() as i32,
        }
    }

    #[test]
    fn boards_keep_the_best_eight_by_score_then_data() {
        let mut b = LocalLeaderboards::default();
        for i in 0..10 {
            b.add("Challenges/ch_glade.lvl", e(i * 10, 0.0));
        }
        b.add("Challenges/ch_glade.lvl", e(90, 5.0));
        let board = b.board("challenges/CH_GLADE.lvl");
        assert_eq!(board.len(), 8);
        assert_eq!(board[0], e(90, 5.0), "equal score: larger data first");
        assert_eq!(board[7].score, 30);
        assert!(is_leaderboard_challenge("Challenges/ch_vietnam.lvl"));
        assert!(!is_leaderboard_challenge("Challenges/ch_osotc.lvl"));
        assert!(!is_leaderboard_challenge("Versus/vs_boat.lvl"));
        let dir = std::env::temp_dir().join(format!("lb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("boards.json");
        b.save(&path).unwrap();
        assert_eq!(LocalLeaderboards::load(&path), b);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
