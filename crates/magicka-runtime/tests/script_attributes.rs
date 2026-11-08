use magicka_runtime::{
    level_variables::*, scene_loading::custom_hash, script_attributes::*, script_condition::*,
};
#[test]
fn condition_defaults_case_hashing_enum_numbers_and_invert_are_preserved() {
    let condition = ScriptCondition::read(
        "COUNTER",
        [
            ("Name", " Flag "),
            ("CompareMethod", "GREATER"),
            ("Value", " +2 "),
            ("INVERT", "True"),
            ("unknown", "anything"),
        ],
    )
    .unwrap();
    assert!(condition.invert);
    assert!(
        matches!(condition.kind,ConditionKind::Counter {id,compare:1,value:2} if id==custom_hash(" flag "))
    );
    let condition = ScriptCondition::read("timer", [("compareMethod", "5")]).unwrap();
    assert!(matches!(
        condition.kind,
        ConditionKind::Timer {
            id: 0,
            compare: 5,
            value: 0.0
        }
    ));
    let condition = ScriptCondition::read("playerCount", [("nr", "4")]).unwrap();
    assert!(matches!(
        condition.kind,
        ConditionKind::PlayerCount {
            compare: 0,
            value: 4
        }
    ));
    let condition = ScriptCondition::read("triggered", [("trigger", "ROOT")]).unwrap();
    assert!(matches!(condition.kind,ConditionKind::Triggered {id} if id==custom_hash("root")));
}
#[test]
fn optional_timer_fields_distinguish_absence_from_explicit_zero_and_false() {
    let action = DecodedVariableAction::read("timer", [("name", "clock")]).unwrap();
    assert!(matches!(
        action.action,
        VariableAction::Timer {
            value: None,
            paused: None,
            ..
        }
    ));
    let action = DecodedVariableAction::read(
        "timer",
        [
            ("name", "clock"),
            ("value", "0"),
            ("paused", "false"),
            ("delay", "0.3"),
        ],
    )
    .unwrap();
    assert_eq!(action.delay, 0.3);
    assert!(matches!(
        action.action,
        VariableAction::Timer {
            value: Some(0.0),
            paused: Some(false),
            ..
        }
    ));
    let action =
        DecodedVariableAction::read("addTimer", [("name", "clock"), ("playedTimer", "true")])
            .unwrap();
    assert!(matches!(
        action.action,
        VariableAction::AddTimer {
            played: true,
            value: 0.0,
            ..
        }
    ));
    let action =
        DecodedVariableAction::read("counter", [("action", "DEC"), ("value", "2")]).unwrap();
    assert!(matches!(
        action.action,
        VariableAction::Counter {
            operation: 1,
            value: 2,
            id: 0
        }
    ));
}
#[test]
fn invalid_known_attributes_error_but_unknown_properties_are_ignored() {
    assert!(matches!(
        ScriptCondition::read("counter", [("name", "Ö")]),
        Err(ScriptReadError::UnsupportedName(_))
    ));
    assert!(matches!(
        ScriptCondition::read("counter", [("compareMethod", "nope")]),
        Err(ScriptReadError::InvalidAttribute(_))
    ));
    assert!(matches!(
        ScriptCondition::read("present", []),
        Err(ScriptReadError::UnsupportedTag(_))
    ));
    assert!(ScriptCondition::read("noCutsceneActive", [("value", "not parsed")]).is_ok());
    assert!(DecodedVariableAction::read("counter", [("paused", "not parsed")]).is_ok());
    assert!(DecodedVariableAction::read("timer", [("paused", "1")]).is_err());
}

#[test]
fn decoded_actions_and_conditions_share_case_normalized_level_state() {
    use std::collections::BTreeMap;
    let mut vars = LevelVariables::default();
    let condition = ScriptCondition::read(
        "counter",
        [
            ("name", "Progress"),
            ("compareMethod", "equal"),
            ("value", "2"),
        ],
    )
    .unwrap();
    let triggered = BTreeMap::new();
    let met = |vars: &LevelVariables| {
        condition
            .is_met(&ConditionFacts {
                variables: vars,
                triggered: &triggered,
                cutscene: false,
                player_count: 1,
            })
            .unwrap()
    };
    assert!(!met(&vars));
    let action = DecodedVariableAction::read(
        "counter",
        [("name", "PROGRESS"), ("action", "inc"), ("value", "2")],
    )
    .unwrap();
    action.action.execute(&mut vars);
    assert!(met(&vars));
    let add = DecodedVariableAction::read("addTimer", [("name", "Clock"), ("playedTimer", "true")])
        .unwrap();
    add.action.execute(&mut vars);
    vars.update(0.5, TimerFrame::default());
    let timer = ScriptCondition::read(
        "timer",
        [
            ("name", "clock"),
            ("compareMethod", "equal"),
            ("value", "0.5"),
        ],
    )
    .unwrap();
    assert_eq!(
        timer.is_met(&ConditionFacts {
            variables: &vars,
            triggered: &triggered,
            cutscene: false,
            player_count: 1
        }),
        Ok(true)
    );
}
