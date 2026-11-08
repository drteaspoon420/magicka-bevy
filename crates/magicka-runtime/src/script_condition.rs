//! Recovered predicates over explicit world facts. Unknown conditions are not
//! replaced with permissive truth values.
use crate::level_variables::LevelVariables;
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug)]
pub enum ConditionKind {
    Counter { id: i32, compare: i8, value: i32 },
    Timer { id: i32, compare: i8, value: f32 },
    Triggered { id: i32 },
    NoCutsceneActive,
    PlayerCount { compare: i8, value: i32 },
}
#[derive(Clone, Copy, Debug)]
pub struct ScriptCondition {
    pub invert: bool,
    pub kind: ConditionKind,
}
pub struct ConditionFacts<'a> {
    pub variables: &'a LevelVariables,
    pub triggered: &'a BTreeMap<i32, bool>,
    pub cutscene: bool,
    pub player_count: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionError {
    MissingTrigger(i32),
}
fn compare<T: PartialOrd>(actual: T, expected: T, method: i8) -> bool {
    match method {
        -1 => actual < expected,
        0 => actual == expected,
        1 => actual > expected,
        _ => false,
    }
}
impl ScriptCondition {
    pub fn is_met(self, facts: &ConditionFacts<'_>) -> Result<bool, ConditionError> {
        let met = match self.kind {
            ConditionKind::Counter {
                id,
                compare: method,
                value,
            } => compare(facts.variables.counter(id), value, method),
            ConditionKind::Timer {
                id,
                compare: method,
                value,
            } => compare(facts.variables.timer(id), value, method),
            ConditionKind::Triggered { id } => *facts
                .triggered
                .get(&id)
                .ok_or(ConditionError::MissingTrigger(id))?,
            ConditionKind::NoCutsceneActive => !facts.cutscene,
            ConditionKind::PlayerCount {
                compare: method,
                value,
            } => compare(
                crate::slots::scripted_party_count(facts.player_count, value, method),
                value,
                method,
            ),
        };
        Ok(self.invert ^ met)
    }
}
