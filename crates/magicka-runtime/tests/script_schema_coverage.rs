//! Every native trigger Action/Condition class and writable property (the
//! generated `script_schema::native` tables) is read by the rewrite's scene
//! loader. A property counts as read when some valid value changes the
//! parsed definition; exceptions are listed with their native reasons.
use magicka_runtime::scene_script::{ActionDefinition, ConditionDefinition, SceneScriptLayout};
use magicka_runtime::script_schema::{
    action_class, condition_class, native, ClassSchema, Family, ScriptCall, ScriptValue, ValueKind,
};
use std::collections::BTreeSet;

/// Properties native Execute/InternalMet never reads.
const UNREAD_NATIVELY: &[(&str, &str, &str)] = &[
    ("PrintDebug", "EntityPos", "PrintDebug.Execute is empty"),
    ("PrintDebug", "Break", "PrintDebug.Execute is empty"),
    ("PrintDebug", "CounterName", "PrintDebug.Execute is empty"),
    ("PrintDebug", "Message", "PrintDebug.Execute is empty"),
    ("SpawnBoss", "Template", "stored by the setter, never read"),
    (
        "TelemetryTutorialBegin",
        "TutorialName",
        "Paradox telemetry upload only",
    ),
    (
        "TelemetryTutorialBegin",
        "StepName",
        "Paradox telemetry upload only",
    ),
    (
        "TelemetryTutorialStep",
        "TutorialName",
        "Paradox telemetry upload only",
    ),
    (
        "TelemetryTutorialStep",
        "StepName",
        "Paradox telemetry upload only",
    ),
];

/// Base values for properties whose strings must name real content or parse
/// as numbers, so that the all-properties base definition reads.
fn base_value(class: &str, property: &str) -> Option<&'static str> {
    Some(match (class, property) {
        ("BossMessage", "Message") => "0",
        ("ScrollScore", "Value") => "5",
        ("SpawnDispenser", "Amount") => "3",
        _ => return None,
    })
}

/// Properties left out of the base while probing another one that writes the
/// same native field (the later attribute wins).
fn aliases(class: &str, property: &str) -> &'static [&'static str] {
    match (class, property) {
        ("SetDialogHint", "Magick") => &["Custom"],
        _ => &[],
    }
}

fn candidates(kind: ValueKind) -> Vec<String> {
    let v: &[&str] = match kind {
        ValueKind::String => &[
            "probe_a", "probe_b", "true", "false", "left", "right", "1", "2", "0.5", "idle",
            "attack0", "1,2,3", "4,5,6",
        ],
        ValueKind::Float => &["0.375", "0.75", "2.5", "0"],
        ValueKind::Int => &["3", "7", "0"],
        ValueKind::Bool => &["true", "false"],
        ValueKind::Vector2 => &["1,2", "3,4"],
        ValueKind::Vector3 => &["1,2,3", "4,5,6"],
        ValueKind::Vector4 | ValueKind::Color => &["0.1,0.2,0.3,0.4", "0.5,0.6,0.7,0.8"],
        ValueKind::Enum(e) => return e.names().iter().map(|s| s.to_string()).collect(),
    };
    v.iter().map(|s| s.to_string()).collect()
}

fn parse(
    class: &ClassSchema,
    probe: Option<(&str, &str)>,
    skip_id: bool,
) -> Result<String, String> {
    let omit = probe.map_or(&[][..], |(p, _)| aliases(class.name, p));
    let mut call = ScriptCall::new(class.name);
    for p in class.properties {
        if omit.contains(&p.name)
            || (skip_id
                && p.name.eq_ignore_ascii_case("id")
                && probe.is_none_or(|(n, _)| n != p.name))
        {
            continue;
        }
        let value = match probe {
            Some((n, v)) if n == p.name => v.to_owned(),
            _ => base_value(class.name, p.name)
                .map(str::to_owned)
                .unwrap_or_else(|| candidates(p.kind)[1.min(candidates(p.kind).len() - 1)].clone()),
        };
        call = call.arg(p.name, ScriptValue::Str(value));
    }
    let element = call.element(class.family).map_err(|e| format!("{e:?}"))?;
    let mut layout = SceneScriptLayout::default();
    Ok(match class.family {
        Family::Action => {
            let i = layout
                .define_action(&element, 0)
                .map_err(|e| format!("{e:?}"))?;
            format!("{:?}", layout.actions[i])
        }
        Family::Condition => {
            let i = layout
                .define_condition(&element)
                .map_err(|e| format!("{e:?}"))?;
            format!("{:?}", layout.conditions[i])
        }
    })
}

#[test]
fn every_native_class_has_a_reader() {
    let mut unsupported = vec![];
    for class in native::ACTIONS.iter().chain(native::CONDITIONS) {
        let element = ScriptCall::new(class.name).element(class.family).unwrap();
        let mut layout = SceneScriptLayout::default();
        match class.family {
            Family::Action => {
                // Missing required attributes may fail; only the tag matters.
                if let Ok(i) = layout.define_action(&element, 0) {
                    if matches!(layout.actions[i], ActionDefinition::Unsupported(_)) {
                        unsupported.push(class.name);
                    }
                }
            }
            Family::Condition => {
                if let Ok(i) = layout.define_condition(&element) {
                    if matches!(layout.conditions[i], ConditionDefinition::Unsupported(_)) {
                        unsupported.push(class.name);
                    }
                }
            }
        }
    }
    assert!(unsupported.is_empty(), "no reader for {unsupported:?}");
    assert_eq!((native::ACTIONS.len(), native::CONDITIONS.len()), (113, 24));
}

#[test]
fn every_native_property_is_read() {
    let mut problems = vec![];
    for class in native::ACTIONS.iter().chain(native::CONDITIONS) {
        if let Err(e) = parse(class, None, false) {
            problems.push(format!("{}: base fails: {e}", class.name));
            continue;
        }
        for p in class.properties {
            let exempt = UNREAD_NATIVELY
                .iter()
                .any(|(c, n, _)| *c == class.name && *n == p.name);
            let read = [false, true].iter().any(|&skip_id| {
                let seen: BTreeSet<_> = candidates(p.kind)
                    .iter()
                    .filter_map(|v| parse(class, Some((p.name, v)), skip_id).ok())
                    .collect();
                seen.len() > 1
            });
            if read == exempt {
                problems.push(format!(
                    "{}.{}: {}",
                    class.name,
                    p.name,
                    if exempt {
                        "listed as unread but read"
                    } else {
                        "ignored"
                    }
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn calls_validate_against_native_property_types() {
    use magicka_runtime::script_schema::CallError;
    assert!(action_class("spawn").is_some_and(|c| c.name == "Spawn"));
    assert!(condition_class("PHYSICSPRESENT").is_some());
    // Enum.Parse ignores case and accepts integers; bool.Parse rejects "1".
    assert!(ScriptCall::new("giveorder")
        .arg("order", "ATTACK")
        .element(Family::Action)
        .is_ok());
    assert_eq!(
        ScriptCall::new("Kill")
            .arg("factions", "evil,wild")
            .arg("area", "yard")
            .element(Family::Action)
            .unwrap()
            .attributes,
        [
            ("factions".to_string(), "evil,wild".to_string()),
            ("area".into(), "yard".into())
        ]
    );
    assert_eq!(
        ScriptCall::new("Remove")
            .arg("force", 1)
            .element(Family::Action)
            .unwrap_err(),
        CallError::InvalidValue {
            class: "Remove",
            property: "Force",
            value: "1".into()
        }
    );
    assert!(matches!(
        ScriptCall::new("Spawn").element(Family::Condition),
        Err(CallError::UnknownClass(Family::Condition, _))
    ));
    // Unknown properties pass through (native ignores them).
    assert!(ScriptCall::new("Kill")
        .arg("colour", 3)
        .element(Family::Action)
        .is_ok());
    // Typed values format into native-parsable text.
    assert_eq!(
        ScriptValue::from([1.0, 0.5, -2.0]).to_attribute(),
        "1.0,0.5,-2.0"
    );
    assert_eq!(
        ScriptValue::from(0.1f32).to_attribute().parse::<f32>(),
        Ok(0.1)
    );
}
