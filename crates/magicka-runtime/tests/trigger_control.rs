use magicka_runtime::{scene_loading::custom_hash, trigger_control::*};
#[test]
fn defaults_normalized_untrimmed_targets_and_unknown_properties() {
    let execute = TriggerControlAction::read("ExecuteTrigger", 42, []).unwrap();
    assert_eq!(execute.target, 0);
    assert_eq!(execute.delay, 0.0);
    assert_eq!(execute.kind, TriggerControlKind::Execute);
    let clear = TriggerControlAction::read(
        "clearDelayedActions",
        42,
        [
            ("TRIGGER", " ALLBUTTHIS "),
            ("delay", "0.125"),
            ("unknown", "x"),
        ],
    )
    .unwrap();
    assert_eq!(clear.target, custom_hash(" allbutthis "));
    assert_eq!(clear.owner, 42);
    assert_eq!(clear.delay, 0.125);
    assert_eq!(clear.kind, TriggerControlKind::ClearDelayed);
    clear.quick_execute();
    assert!(TriggerControlAction::read("executeTrigger", 0, [("delay", "invalid")]).is_err());
    assert!(TriggerControlAction::read("executeTrigger", 0, [("trigger", "å")]).is_err());
}
