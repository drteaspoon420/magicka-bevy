use magicka_network::session::QueuedSpell;
use magicka_sim::spells::{Elements as E, MagickType, Spell, MAGICK_COMBOS};

#[test]
fn weighted_elements_and_recognized_magicks_round_trip_without_defaults() {
    let mut original = Spell::default_for(E::FIRE | E::ICE | E::STEAM);
    original.mag[3] = 1.25;
    original.mag[8] = 2.5;
    original.mag[9] = 3.75;
    let wire = QueuedSpell::from(&original);
    let decoded: QueuedSpell = serde_json::from_slice(&serde_json::to_vec(&wire).unwrap()).unwrap();
    assert_eq!(decoded.spell(), Some(original));
    for &(magick, _) in MAGICK_COMBOS {
        let original = Spell::magick(magick);
        assert_eq!(QueuedSpell::from(&original).spell(), Some(original));
    }
    assert_eq!(MagickType::try_from(0), Ok(MagickType::None));
    assert!(MagickType::try_from(u8::MAX).is_err());
}

#[test]
fn malformed_spell_fields_are_rejected() {
    let valid = QueuedSpell::from(&Spell::default_for(E::FIRE));
    for value in [f32::NAN, f32::INFINITY, -1.] {
        let mut bad = valid.clone();
        bad.magnitudes[3] = value;
        assert!(bad.spell().is_none());
    }
    let mut bad = valid.clone();
    bad.elements = 1 << 31;
    assert!(bad.spell().is_none());
    let mut bad = valid.clone();
    bad.magick = u8::MAX;
    assert!(bad.spell().is_none());
    let mut bad = valid;
    bad.magick = MagickType::Rain as u8;
    assert!(
        bad.spell().is_none(),
        "ordinary queue slot cannot carry a magick identity"
    );
}
