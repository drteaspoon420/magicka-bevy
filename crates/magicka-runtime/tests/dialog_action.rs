use magicka_runtime::{dialog_action::*, scene_loading::custom_hash};
#[test]
fn defaults_ordered_position_setters_and_coordinate_bug_are_preserved() {
    let action = DialogAction::read("startDialog", []).unwrap();
    let DialogActionKind::Start(start) = action.kind else {
        panic!()
    };
    assert_eq!(start.dialog, 0);
    assert_eq!(start.owner, custom_hash("interactor"));
    assert!(!start.is_2d && !start.is_3d && !start.force_on_screen);
    let action = DialogAction::read(
        "startDialog",
        [
            ("Position", "1,2"),
            ("position", "3,4,5"),
            ("dialog", "INTRO"),
            ("delay", "0.125"),
        ],
    )
    .unwrap();
    let DialogActionKind::Start(start) = action.kind else {
        panic!()
    };
    assert_eq!(start.position, [3.0, 5.0, 0.0]);
    assert!(start.is_3d && !start.is_2d);
    assert_eq!(start.dialog, custom_hash("intro"));
    assert_eq!(action.delay, 0.125);
    let action = DialogAction::read("startDialog", [("position", "1,2,3,4")]).unwrap();
    let DialogActionKind::Start(start) = action.kind else {
        panic!()
    };
    assert_eq!(start.owner, custom_hash("1,2,3,4"));
    assert!(!start.is_2d && !start.is_3d);
    assert!(DialogAction::read("startDialog", [("position", "1,bad")]).is_err());
    assert!(DialogAction::read("startDialog", [("forceOnScreen", "bad")]).is_err());
    assert!(DialogAction::read("killDialogs", [("unused", "bad")]).is_ok());
}
