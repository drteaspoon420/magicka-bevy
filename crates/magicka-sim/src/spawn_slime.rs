//! SpawnSlime / SpawnSlimeOverkill batch selection and placement.
//! Evidence: research/spawn-slime.md. Spawning and navmesh queries are host work.
use crate::{impact_motion::Vector, spells::Elements};

pub const SPREAD: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Small,
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slime {
    pub size: Size,
    pub element: Elements,
}
impl Slime {
    /// Native GetTemplateId: Large and unrecognized elements fall back to
    /// poison-small; the large template hashes are defined but never returned.
    pub fn template(self) -> &'static str {
        use Size::*;
        let arcane = matches!(
            self.element,
            Elements::EARTH
                | Elements::WATER
                | Elements::COLD
                | Elements::FIRE
                | Elements::LIGHTNING
                | Elements::ARCANE
        );
        let poison = matches!(
            self.element,
            Elements::LIFE | Elements::SHIELD | Elements::ICE | Elements::STEAM | Elements::POISON
        );
        match (self.size, arcane, poison) {
            (Small, true, _) => "dungeons_slimecube_arcane_small",
            (Medium, true, _) => "dungeons_slimecube_arcane",
            (Medium, _, true) => "dungeons_slimecube_poison",
            _ => "dungeons_slimecube_poison_small",
        }
    }
}

/// One element is chosen for the entire batch, before any placement draws.
/// `next_index` models MagickaMath.Random.Next(11) including rejection draws.
/// Unsupported-only masks are refused instead of entering the native infinite
/// rejection loop; ordinary dispatch passes NONE and never reaches that case.
pub fn batch(
    parent: &str,
    elements: Elements,
    overkill: bool,
    mut next_index: impl FnMut() -> usize,
) -> Option<Vec<Slime>> {
    use Size::*;
    let (sizes, default): (Vec<Size>, Elements) = match parent {
        "dungeons_slimecube_poison_large" => (
            if overkill {
                vec![Small; 8]
            } else {
                vec![Medium; 3]
            },
            Elements::POISON,
        ),
        "dungeons_slimecube_poison_medium"
        | "dungeons_slimecube_medium"
        | "dungeons_slimecube_poison"
        | "dungeons_acolyte_a" => (vec![Small; 3], Elements::POISON),
        "dungeons_slimecube_arcane" => (vec![Small; 3], Elements::ARCANE),
        "dungeons_acolyte_c" => (vec![Medium, Medium, Small, Small], Elements::POISON),
        // Native default ignores the supplied element mask, even when nonzero.
        _ => {
            return Some(vec![Slime {
                size: Small,
                element: Elements::POISON,
            }])
        }
    };
    let element = if elements == Elements::NONE {
        default
    } else {
        if elements.0 & 0x7ff == 0 {
            return None;
        }
        loop {
            let index = next_index();
            let candidate = *Elements::BY_INDEX.get(index)?;
            if elements.intersects(candidate) {
                break candidate;
            }
        }
    };
    Some(
        sizes
            .into_iter()
            .map(|size| Slime { size, element })
            .collect(),
    )
}

/// Native CreateEntities draws a uniform horizontal disc of radius five.
/// Preserve double multiplication before narrowing the angle to float.
pub fn position(origin: Vector, radial: f64, angular: f64) -> Vector {
    let radius = radial.sqrt() as f32;
    let angle = (angular * f64::from(std::f32::consts::TAU)) as f32;
    let x = (f64::from(radius) * f64::from(angle).cos()) as f32;
    let z = (f64::from(radius) * f64::from(angle).sin()) as f32;
    [origin[0] + SPREAD * x, origin[1], origin[2] + SPREAD * z]
}

/// SpawnAnimation None/idle/idle_agg do not enter RessurectionState.
pub fn needs_resurrection(animation: u16) -> bool {
    !matches!(animation, 0 | 1 | 5)
}
