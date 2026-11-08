use magicka_runtime::blast_visual::bursts;
use magicka_sim::{damage::Damage, spells::Elements};

#[test]
fn ground_wedges_use_ground_lookup_even_with_beam_elements() {
    let mut damages = [Damage::default(); 5];
    damages[0].element = Elements::EARTH.0;
    damages[1].element = Elements::ARCANE.0;
    let effects = magicka_runtime::blast_visual::ground_bursts(5.0, [0.0; 3], &damages);
    assert_eq!(effects.len(), 16);
    assert_eq!(effects[0].0, "area_ground_earth");
    assert_eq!(effects[8].0, "area_ground_arcane");
    assert_eq!(effects[0].1[0], 0.5);
}

#[test]
fn wedges_preserve_slot_duplicates_translation_and_clipped_scale() {
    let mut damages = [Damage::default(); 5];
    damages[0].element = Elements::FIRE.0;
    damages[1] = damages[0];
    let result = bursts(10.0, [1.0, 2.0, 3.0], &damages, [0.5; 8]);
    assert_eq!(result.len(), 16);
    assert!(result.iter().all(|(name, _)| *name == "area_fire1_b"));
    assert_eq!(result[..8], result[8..]);
    for (_, matrix) in &result {
        assert_eq!(&matrix[12..], &[1.0, 2.0, 3.0, 1.0]);
        assert_eq!(matrix[5], 1.0);
        let radial_length = (matrix[0] * matrix[0] + matrix[2] * matrix[2]).sqrt();
        assert!((radial_length - 0.5).abs() < 0.00001);
    }
    assert!(result[2].1[2] < -0.49); // XNA +90-degree Y rotation.
}

#[test]
fn empty_slots_and_beam_blasts_do_not_emit_nonbeam_wedges() {
    let mut damages = [Damage::default(); 5];
    assert!(bursts(1.0, [0.0; 3], &damages, [1.0; 8]).is_empty());
    damages[0].element = Elements::EARTH.0;
    assert_eq!(
        bursts(1.0, [0.0; 3], &damages, [1.0; 8])[0].0,
        "area_ground_earth"
    );
    damages[1].element = Elements::ARCANE.0;
    assert!(bursts(1.0, [0.0; 3], &damages, [1.0; 8]).is_empty());
}

#[test]
fn ground_flash_excludes_earth_and_preserves_combined_element_slot() {
    let mut damages = [Damage::default(); 5];
    damages[0].element = Elements::EARTH.0;
    assert!(magicka_runtime::blast_visual::ground_light(5.0, [0.0; 3], &damages).is_none());
    damages[0].element |= Elements::ARCANE.0;
    let light =
        magicka_runtime::blast_visual::ground_light(5.0, [1.0, 2.0, 3.0], &damages).unwrap();
    assert_eq!(light.position, [1.0, 2.0, 3.0]);
    assert_eq!(light.radius, 10.0);
    assert_eq!(
        (
            light.dynamic_intensity,
            light.speed,
            light.specular,
            light.ttl
        ),
        (6.0, 4.0, 1.0, 1.0)
    );
    // Native trailing-zero lookup selects Earth dust for this combined slot.
    let effects = magicka_runtime::blast_visual::ground_bursts(5.0, light.position, &damages);
    assert_eq!(effects.len(), 8);
    assert!(effects.iter().all(|(name, _)| *name == "area_ground_earth"));
}
