use magicka_sim::{blast_damage, damage::Damage, spells::Elements};

#[test]
fn cubic_falloff_preserves_full_strength_inside_and_zero_at_radius() {
    assert_eq!(blast_damage::scale(-1.0, 4.0), 1.0);
    assert_eq!(blast_damage::scale(0.0, 4.0), 1.0);
    assert_eq!(blast_damage::scale(2.0, 4.0), 0.875);
    assert_eq!(blast_damage::scale(4.0, 4.0), 0.0);
    // Range rejection belongs to the host; this function does not clamp.
    assert_eq!(blast_damage::scale(8.0, 4.0), -7.0);
}

#[test]
fn collection_changes_only_magnitude_and_any_beam_exempts_all_slots() {
    let mut slots = [Damage::default(); 5];
    slots[0].element = Elements::FIRE.0;
    slots[0].amount = 40.0;
    slots[0].magnitude = 8.0;
    let mut expected = slots;
    expected[0].magnitude = 7.0;
    assert_eq!(blast_damage::collection(slots, 2.0, 4.0), expected);
    slots[4].element = Elements::ARCANE.0;
    assert_eq!(blast_damage::collection(slots, 2.0, 4.0), slots);
}
