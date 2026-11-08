use magicka_sim::{damage::Damage, liquid_freeze::magnitude, spells::Elements as E};
fn slot(element: E, mag: f32) -> Damage {
    Damage {
        element: element.0,
        magnitude: mag,
        ..Default::default()
    }
}
#[test]
fn cold_freezes_fire_thaws_and_water_only_cools_lava() {
    let expected = (1.0 - 0.75f32.powi(3)) * 3.0;
    assert_eq!(magnitude(&[slot(E::COLD, 1.0)], false, 1.0), expected);
    assert_eq!(
        magnitude(&[slot(E::FIRE, 1.0)], false, 0.5),
        -expected * 0.5
    );
    assert_eq!(magnitude(&[slot(E::WATER, 1.0)], false, 1.0), 0.0);
    assert_eq!(magnitude(&[slot(E::WATER, 1.0)], true, 1.0), expected);
}
#[test]
fn combined_masks_count_independently_and_ignore_amount_and_attack() {
    let mixed = slot(E::COLD | E::FIRE, 2.0);
    assert_eq!(magnitude(&[mixed], false, 1.0), 0.0);
    let mut cold = slot(E::COLD, 1.0);
    let original = magnitude(&[cold], false, 1.0);
    cold.amount = 9000.0;
    cold.attack = 0xffff;
    assert_eq!(magnitude(&[cold], false, 1.0), original);
    assert_eq!(magnitude(&[], true, 1.0), 0.0);
}
