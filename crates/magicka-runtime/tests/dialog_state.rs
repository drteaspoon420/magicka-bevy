use magicka_runtime::{dialog_state::*, scene_loading::custom_hash};
#[test]
fn pop_advances_only_at_final_finite_repetition_and_wrap_resets_repetition_state() {
    let mut cursor = DialogCursor::new(
        vec![
            InteractionRepetition {
                repeat: 2,
                completed: 0,
            },
            InteractionRepetition::default(),
        ],
        0,
    );
    assert_eq!(
        cursor.pop().unwrap(),
        Some(DialogPop {
            interaction: 0,
            reset_all: false
        })
    );
    assert_eq!((cursor.next, cursor.interacted), (0, 0));
    cursor.interactions[0].completed = 1;
    assert_eq!(
        cursor.pop().unwrap(),
        Some(DialogPop {
            interaction: 0,
            reset_all: false
        })
    );
    assert_eq!((cursor.next, cursor.interacted), (1, 1));
    assert_eq!(
        cursor.pop().unwrap(),
        Some(DialogPop {
            interaction: 1,
            reset_all: true
        })
    );
    assert_eq!((cursor.next, cursor.interacted), (0, 2));
    assert_eq!(cursor.interactions[0].completed, 0);
    cursor.interactions[0].repeat = 0;
    assert_eq!(cursor.pop().unwrap().unwrap().interaction, 0);
    assert_eq!((cursor.next, cursor.interacted), (0, 2));
}
#[test]
fn reset_only_rewinds_and_finite_dialog_can_be_exhausted() {
    let mut cursor = DialogCursor::new(
        vec![InteractionRepetition {
            repeat: 1,
            completed: 1,
        }],
        -1,
    );
    cursor.pop().unwrap();
    assert!(cursor.pop().unwrap().is_none());
    cursor.reset();
    assert_eq!(
        (
            cursor.next,
            cursor.interacted,
            cursor.interactions[0].completed
        ),
        (0, 1, 1)
    );
    cursor.next = -1;
    assert_eq!(cursor.pop(), Err(DialogError::InvalidCursor(-1)));
}
#[test]
fn completion_requires_flag_and_strict_interacted_threshold_and_reset_preserves_count() {
    let id = custom_hash("intro");
    let mut ledger = DialogLedger::default();
    ledger.insert(
        id,
        DialogCursor::new(vec![InteractionRepetition::default()], -1),
    );
    ledger.dialogs.get_mut(&id).unwrap().pop().unwrap();
    assert!(!ledger.is_done(id, -1).unwrap());
    ledger.mark_finished(id);
    assert!(ledger.is_done(id, -1).unwrap());
    assert!(ledger.is_done(id, 0).unwrap());
    assert!(!ledger.is_done(id, 1).unwrap());
    ledger.reset();
    assert!(!ledger.is_done(id, 0).unwrap());
    assert_eq!(ledger.dialogs[&id].interacted, 1);
    ledger.mark_finished(42);
    assert_eq!(ledger.is_done(42, -1), Err(DialogError::MissingDialog(42)));
    assert_eq!(
        ledger.is_done(43, -1),
        Err(DialogError::MissingFinishedFlag(43))
    );
}
#[test]
fn any_dialog_active_checks_message_boxes_while_named_checks_interaction_slots() {
    let ledger = DialogLedger::default();
    let mut active = [None; 8];
    active[7] = Some(custom_hash("intro"));
    let facts = DialogFacts {
        ledger: &ledger,
        active_dialogs: &active,
        message_box_count: 0,
    };
    for attrs in [vec![], vec![("dialog", "ANY")], vec![("dialog", "")]] {
        assert!(!DialogCondition::read("dialogActive", attrs)
            .unwrap()
            .is_met(facts)
            .unwrap());
    }
    assert!(DialogCondition::read("dialogActive", [("dialog", "INTRO")])
        .unwrap()
        .is_met(facts)
        .unwrap());
    let no_slots = [None; 8];
    let boxes = DialogFacts {
        active_dialogs: &no_slots,
        message_box_count: 1,
        ..facts
    };
    assert!(DialogCondition::read("dialogActive", [])
        .unwrap()
        .is_met(boxes)
        .unwrap());
    assert!(
        !DialogCondition::read("dialogActive", [("dialog", " any ")])
            .unwrap()
            .is_met(boxes)
            .unwrap()
    );
}
#[test]
fn decoded_done_defaults_inversion_and_errors_preserve_lookup_behavior() {
    let mut ledger = DialogLedger::default();
    ledger.insert(0, DialogCursor::new(vec![], 0));
    ledger.mark_finished(0);
    let active = [None; 8];
    let facts = DialogFacts {
        ledger: &ledger,
        active_dialogs: &active,
        message_box_count: 0,
    };
    assert!(DialogCondition::read("dialogDone", [])
        .unwrap()
        .is_met(facts)
        .unwrap());
    assert!(
        !DialogCondition::read("dialogDone", [("invert", "true"), ("unknown", "invalid")])
            .unwrap()
            .is_met(facts)
            .unwrap()
    );
    assert!(DialogCondition::read("dialogDone", [("interactIndex", "bad")]).is_err());
    assert!(DialogCondition::read("dialogActive", [("dialog", "å")]).is_err());
    assert_eq!(
        DialogCondition::read("dialogDone", [("dialog", "absent"), ("invert", "true")])
            .unwrap()
            .is_met(facts),
        Err(DialogError::MissingFinishedFlag(custom_hash("absent")))
    );
}
