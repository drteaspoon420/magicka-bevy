use magicka_network::control::*;
fn command(sequence: u64) -> InputCommand {
    InputCommand {
        session: 1,
        epoch: 1,
        sequence,
        controls: Controls::default(),
        history: Vec::new(),
    }
}
#[test]
fn connection_slots_and_input_clocks_reject_spoofing_without_consuming_sequence() {
    let mut host = InputAuthority::new(1, 1).unwrap();
    assert_eq!(host.accept(0, &command(1)), Err(InputError::Slot));
    // Slots beyond the original four are valid up to the lobby slot cap.
    let cap = magicka_network::lobby::PLAYER_SLOT_CAP;
    assert_eq!(host.accept(cap, &command(1)), Err(InputError::Slot));
    host.accept(4, &command(1)).unwrap();
    host.accept(cap - 1, &command(1)).unwrap();
    // Non-player seats (invaders piloting NPCs) have their own streams.
    let seat = magicka_network::lobby::OBSERVER_SEAT_BASE;
    host.accept(seat, &command(1)).unwrap();
    assert_eq!(host.accept(seat, &command(1)), Err(InputError::Stale));
    let mut wrong = command(100);
    wrong.session = 2;
    assert_eq!(host.accept(1, &wrong), Err(InputError::Session));
    host.accept(1, &command(1)).unwrap();
    assert_eq!(host.accept(1, &command(1)), Err(InputError::Stale));
    host.accept(2, &command(1)).unwrap();
    let mut bad = command(2);
    bad.controls.aim = Some([f32::NAN, 0., 0.]);
    assert_eq!(host.accept(1, &bad), Err(InputError::Invalid));
    host.accept(1, &command(2)).unwrap();
    assert_eq!(host.advance_epoch(1), Err(InputError::Epoch));
    host.advance_epoch(2).unwrap();
    assert_eq!(host.accept(1, &command(3)), Err(InputError::Epoch));
    let mut fresh = command(1);
    fresh.epoch = 2;
    host.accept(1, &fresh).unwrap();
    assert!(!serde_json::to_value(&fresh)
        .unwrap()
        .as_object()
        .unwrap()
        .contains_key("slot"));
}
