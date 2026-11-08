use magicka_runtime::{animation_action::*, scene_loading::custom_hash};
#[test]
fn play_defaults_case_insensitive_attributes_and_untrimmed_path_components() {
    let action = AnimationAction::read(
        "PLAYanimation",
        [("NAME", "Root/ Door/"), ("ignored", "anything")],
    )
    .unwrap();
    assert_eq!(
        action.path,
        [custom_hash("root"), custom_hash(" door"), custom_hash("")]
    );
    assert!(action.children);
    assert_eq!(action.delay, 0.0);
    let AnimationOperation::Play(command) = action.operation else {
        panic!()
    };
    assert_eq!(
        (command.start, command.end, command.speed),
        (-1.0, -1.0, 1.0)
    );
    assert!(!command.looping && !command.resume);
}
#[test]
fn resume_omitted_loop_differs_from_explicit_false_and_parses_numeric_attributes() {
    let action = AnimationAction::read(
        "resumeAnimation",
        [
            ("name", "door"),
            ("speed", " -2.5 "),
            ("length", "0.3"),
            ("delay", "1e-1"),
            ("children", " FALSE "),
        ],
    )
    .unwrap();
    let AnimationOperation::Resume {
        length,
        speed,
        looping,
    } = action.operation
    else {
        panic!()
    };
    assert_eq!((length, speed, looping), (0.3, -2.5, None));
    assert!(!action.children);
    assert_eq!(action.delay, 0.1);
    let action =
        AnimationAction::read("resumeAnimation", [("name", "door"), ("loop", "false")]).unwrap();
    assert!(matches!(
        action.operation,
        AnimationOperation::Resume {
            looping: Some(false),
            ..
        }
    ));
}
#[test]
fn unsupported_names_invalid_typed_values_and_missing_names_report_errors() {
    assert_eq!(
        AnimationAction::read("playAnimation", []).unwrap_err(),
        ActionReadError::MissingName
    );
    assert!(matches!(
        AnimationAction::read("playAnimation", [("name", "DÖOR")]),
        Err(ActionReadError::UnsupportedName(_))
    ));
    assert!(matches!(
        AnimationAction::read("playAnimation", [("name", "door"), ("loop", "1")]),
        Err(ActionReadError::InvalidAttribute(_))
    ));
    assert!(matches!(
        AnimationAction::read("playAnimation", [("name", "door"), ("start", "1,000")]),
        Err(ActionReadError::InvalidAttribute(_))
    ));
    let action =
        AnimationAction::read("stopAnimation", [("name", "door"), ("speed", "ignored")]).unwrap();
    assert!(matches!(action.operation, AnimationOperation::Stop));
    action.quick_execute();
}
