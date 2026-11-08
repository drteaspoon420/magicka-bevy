use magicka_runtime::{level_variables::*, script_condition::*};
use std::collections::BTreeMap;
#[test]
fn counters_default_to_zero_and_arithmetic_wraps() {
    let mut vars = LevelVariables::default();
    assert_eq!(vars.counter(7), 0);
    vars.add_counter(7, i32::MAX);
    vars.add_counter(7, 1);
    assert_eq!(vars.counter(7), i32::MIN);
    VariableAction::Counter {
        id: 7,
        operation: 1,
        value: i32::MIN,
    }
    .execute(&mut vars);
    assert_eq!(vars.counter(7), 0);
    VariableAction::Counter {
        id: 7,
        operation: 2,
        value: 12,
    }
    .execute(&mut vars);
    VariableAction::Counter {
        id: 7,
        operation: 0,
        value: 3,
    }
    .quick_execute(&mut vars);
    assert_eq!(vars.counter(7), 12);
    VariableAction::Counter {
        id: 7,
        operation: 9,
        value: 3,
    }
    .execute(&mut vars);
    assert_eq!(vars.counter(7), 12);
}
#[test]
fn timer_lookup_prefers_played_scope_and_add_does_not_overwrite() {
    let mut vars = LevelVariables::default();
    vars.set_timer(7, 10.0);
    vars.pause_timer(7, true);
    assert_eq!(vars.timer(7), 0.0);
    assert!(vars.timers.is_empty());
    vars.add_timer(7, false, 1.0);
    vars.add_timer(7, false, 9.0);
    vars.add_timer(7, true, 2.0);
    assert_eq!(vars.timer(7), 2.0);
    vars.set_timer(7, 3.0);
    vars.pause_timer(7, true);
    assert_eq!(vars.timers[0].time, 1.0);
    assert!(!vars.timers[0].paused);
    assert_eq!(vars.played_timers[0].time, 3.0);
    assert!(vars.played_timers[0].paused);
}
#[test]
fn ordinary_timers_tick_through_pause_cutscene_and_game_end_but_played_timers_do_not() {
    let mut vars = LevelVariables::default();
    vars.add_timer(1, true, 0.0);
    vars.add_timer(2, false, 0.0);
    vars.add_timer(3, false, 0.0);
    vars.pause_timer(3, true);
    for frame in [
        TimerFrame {
            paused: true,
            ..Default::default()
        },
        TimerFrame {
            cutscene: true,
            ..Default::default()
        },
        TimerFrame {
            game_ended: true,
            ..Default::default()
        },
    ] {
        vars.update(0.5, frame);
    }
    assert_eq!(vars.timer(1), 0.0);
    assert_eq!(vars.timer(2), 1.5);
    assert_eq!(vars.timer(3), 0.0);
    vars.update(0.5, TimerFrame::default());
    assert_eq!(vars.timer(1), 0.5);
    assert_eq!(vars.timer(2), 2.0);
}
#[test]
fn timer_actions_preserve_optional_fields_and_execute_during_quick_replay() {
    let mut vars = LevelVariables::default();
    VariableAction::AddTimer {
        id: 1,
        played: true,
        value: 2.0,
    }
    .quick_execute(&mut vars);
    VariableAction::Timer {
        id: 1,
        value: None,
        paused: Some(true),
    }
    .execute(&mut vars);
    assert_eq!(vars.timer(1), 2.0);
    VariableAction::Timer {
        id: 1,
        value: Some(3.0),
        paused: None,
    }
    .quick_execute(&mut vars);
    assert!(vars.played_timers[0].paused);
    assert_eq!(vars.timer(1), 3.0);
}
#[test]
fn predicates_compare_actual_values_and_invert_including_invalid_enum_and_nan() {
    let mut vars = LevelVariables::default();
    vars.set_counter(1, 2);
    vars.add_timer(2, false, f32::NAN);
    let triggered = BTreeMap::new();
    let facts = ConditionFacts {
        variables: &vars,
        triggered: &triggered,
        cutscene: false,
        player_count: 3,
    };
    for (method, value, expected) in [(-1, 3, true), (0, 2, true), (1, 1, true), (2, 0, false)] {
        let condition = ScriptCondition {
            invert: false,
            kind: ConditionKind::Counter {
                id: 1,
                compare: method,
                value,
            },
        };
        assert_eq!(condition.is_met(&facts), Ok(expected));
        assert_eq!(
            ScriptCondition {
                invert: true,
                ..condition
            }
            .is_met(&facts),
            Ok(!expected)
        );
    }
    for method in [-1, 0, 1] {
        assert_eq!(
            ScriptCondition {
                invert: false,
                kind: ConditionKind::Timer {
                    id: 2,
                    compare: method,
                    value: 0.0
                }
            }
            .is_met(&facts),
            Ok(false)
        );
    }
    assert_eq!(
        ScriptCondition {
            invert: false,
            kind: ConditionKind::PlayerCount {
                compare: 0,
                value: 3
            }
        }
        .is_met(&facts),
        Ok(true)
    );
}
#[test]
fn triggered_missing_id_remains_error_and_cutscene_predicate_uses_live_flag() {
    let vars = LevelVariables::default();
    let triggered = BTreeMap::from([(7, true)]);
    let mut facts = ConditionFacts {
        variables: &vars,
        triggered: &triggered,
        cutscene: false,
        player_count: 1,
    };
    assert_eq!(
        ScriptCondition {
            invert: false,
            kind: ConditionKind::Triggered { id: 7 }
        }
        .is_met(&facts),
        Ok(true)
    );
    assert_eq!(
        ScriptCondition {
            invert: true,
            kind: ConditionKind::Triggered { id: 8 }
        }
        .is_met(&facts),
        Err(ConditionError::MissingTrigger(8))
    );
    let condition = ScriptCondition {
        invert: false,
        kind: ConditionKind::NoCutsceneActive,
    };
    assert_eq!(condition.is_met(&facts), Ok(true));
    facts.cutscene = true;
    assert_eq!(condition.is_met(&facts), Ok(false));
}
