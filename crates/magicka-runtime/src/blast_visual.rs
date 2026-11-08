//! Non-beam FullBlast burst wedges. Decals use the shared retained pool;
//! full-blast shield clipping remains pending.
use magicka_sim::{damage::Damage, spells::Elements};

const EFFECTS: [&str; 11] = [
    "area_ground_earth",
    "area_water1_b",
    "area_cold1_b",
    "area_fire1_b",
    "area_lightning1_b",
    "area_arcane1_b",
    "area_life1_b",
    "area_shield1_b",
    "area_ice1_b",
    "area_steam1_b",
    "area_poison1_b",
];

/// XNA RotationY.Right for the eight original radial segments.
pub fn direction(segment: usize) -> [f32; 3] {
    let angle = segment as f32 * 0.125 * std::f32::consts::TAU;
    let (sin, cos) = angle.sin_cos();
    [cos, 0.0, -sin]
}

/// BlastVisual scales horizontal basis rows by scalar * radius * 0.1.
/// Duplicate damage slots deliberately produce duplicate effect requests.
pub fn bursts(
    radius: f32,
    position: [f32; 3],
    damages: &[Damage; 5],
    scalars: [f32; 8],
) -> Vec<(&'static str, [f32; 16])> {
    wedges(radius, position, damages, scalars, false)
}

/// GroundBlast supplies all-one scalars (scene/shield clipping disabled).
pub fn ground_bursts(
    radius: f32,
    position: [f32; 3],
    damages: &[Damage; 5],
) -> Vec<(&'static str, [f32; 16])> {
    wedges(radius, position, damages, [1.0; 8], true)
}

fn wedges(
    radius: f32,
    position: [f32; 3],
    damages: &[Damage; 5],
    scalars: [f32; 8],
    ground: bool,
) -> Vec<(&'static str, [f32; 16])> {
    const GROUND: [&str; 11] = [
        "area_ground_earth",
        "area_ground_water",
        "area_ground_cold",
        "area_ground_fire",
        "area_ground_lightning",
        "area_ground_arcane",
        "area_ground_life",
        "area_ground_shield",
        "area_ground_ice",
        "area_ground_steam",
        "area_ground_poison",
    ];
    if !ground && damages.iter().fold(0, |mask, d| mask | d.element) & Elements::BEAMS.0 != 0 {
        return vec![]; // ArcaneBlast is a separate spell entity.
    }
    let mut result = vec![];
    for damage in damages.iter().filter(|d| d.element != 0) {
        let names = if ground { &GROUND } else { &EFFECTS };
        let Some(&name) = names.get(damage.element.trailing_zeros() as usize) else {
            continue;
        };
        for (i, scalar) in scalars.into_iter().enumerate() {
            let [cos, _, minus_sin] = direction(i);
            let scale = scalar * (radius * 0.1);
            result.push((
                name,
                [
                    cos * scale,
                    0.0,
                    minus_sin * scale,
                    0.0,
                    0.0,
                    1.0,
                    0.0,
                    0.0,
                    -minus_sin * scale,
                    0.0,
                    cos * scale,
                    0.0,
                    position[0],
                    position[1],
                    position[2],
                    1.0,
                ],
            ));
        }
    }
    result
}

/// GroundBlast enables a spell-colored flash for the non-Earth elements.
/// Unlike FullBlast this does not create an ArcaneBlast disc entity.
pub fn ground_light(
    radius: f32,
    position: [f32; 3],
    damages: &[Damage; 5],
) -> Option<crate::arcane_blast::DynamicLight> {
    let elements = damages.iter().fold(0, |mask, d| mask | d.element) & !Elements::EARTH.0;
    let spell = magicka_sim::spells::Spell::default_for(Elements(elements));
    if spell.total_magnitude() <= 0.0 {
        return None;
    }
    Some(crate::arcane_blast::DynamicLight::new(
        position,
        magicka_sim::lightning::spell_color(&spell),
        6.0,
        radius * 2.0,
        4.0,
        1.0,
        1.0,
    ))
}
