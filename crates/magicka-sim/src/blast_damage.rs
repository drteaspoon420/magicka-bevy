//! Blast.BlastDamageScale; distance is skin-clipped and radius-subtracted by the host.
pub fn scale(distance: f32, radius: f32) -> f32 {
    let fraction = (distance / radius).max(0.0);
    1.0 - fraction * fraction * fraction
}

/// Collection FullBlast scales magnitude only, and exempts the entire
/// collection when any slot contains a beam element.
pub fn collection(
    mut damages: [crate::damage::Damage; 5],
    distance: f32,
    radius: f32,
) -> [crate::damage::Damage; 5] {
    let elements = damages.iter().fold(0, |all, damage| all | damage.element);
    if elements & crate::spells::Elements::BEAMS.0 == 0 {
        let multiplier = scale(distance, radius);
        for damage in &mut damages {
            damage.magnitude *= multiplier;
        }
    }
    damages
}
