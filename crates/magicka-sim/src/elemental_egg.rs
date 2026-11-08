//! ElementalEgg damage memory and hatching lifecycle, recovered from CIL.
//! Physics, assets, proximity entity queries and NPC spawning belong to hosts.
//! See research/summon-elemental.md for evidence and unresolved ordering.
use crate::{damage::Damage, spells::Elements as E};

pub const MODEL: &str = "Models/Characters/Elemental/elemental_mesh";
pub const ANIMATION_MODEL: &str = "Models/Characters/Elemental/elemental_animation";
pub const CLIP: &str = "spawn";

pub const ACCEPTABLE: u32 = 1662;
pub const RANDOM_ELEMENTS: [E; 8] = [
    E::ARCANE,
    E::COLD,
    E::POISON,
    E::FIRE,
    E::LIFE,
    E::WATER,
    E::STEAM,
    E::LIGHTNING,
];

/// SummonElemental positional overload draws two positive horizontal values,
/// then normalizes. The native zero-vector singularity is preserved.
pub fn random_direction(x: f64, z: f64) -> [f32; 3] {
    let (x, z) = (x as f32, z as f32);
    let inverse = (x * x + z * z).sqrt().recip();
    [x * inverse, 0.0 * inverse, z * inverse]
}

/// Owner and position overloads both offset three metres before nav snapping.
pub fn cast_point(position: [f32; 3], direction: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| position[i] + direction[i] * 3.0)
}

/// Initialize probes from requested+1Y down10Y, falls back to the requested
/// point on a scene miss, then adds the Fire template's capsule radius.
pub fn egg_position(requested: [f32; 3], scene_hit: Option<[f32; 3]>, radius: f32) -> [f32; 3] {
    let mut point = scene_hit.unwrap_or(requested);
    point[1] += radius;
    point
}

/// ElementalEgg.SegmentIntersect uses a vertical axis and closest segment
/// points, including the caller's segment radius. It is not a ray entry time.
pub fn segment_intersect(
    capsule_origin: [f32; 3],
    length: f32,
    radius: f32,
    segment: crate::spray_pulse::Segment,
    segment_radius: f32,
) -> Option<[f32; 3]> {
    let (distance_squared, _, fraction) = crate::geometry::jiglib_segment_segment(
        capsule_origin,
        [0.0, length, 0.0],
        segment.origin,
        segment.delta,
    );
    let reach = radius + segment_radius;
    (distance_squared <= reach * reach)
        .then(|| std::array::from_fn(|i| segment.origin[i] + segment.delta[i] * fraction))
}

#[derive(Clone, Debug)]
pub struct ElementalEgg {
    pub spawn_element: E,
    pub summoner: Option<u16>,
    pub proximity: f32,
    pub proximity_timer: f32,
    pub resting_timer: f32,
    // Keep enumeration order; native selection uses strict greater-than.
    memory: Vec<(E, f32)>,
}

impl Default for ElementalEgg {
    fn default() -> Self {
        Self {
            spawn_element: E::NONE,
            summoner: None,
            proximity: 0.0,
            proximity_timer: 0.0,
            resting_timer: 1.0,
            memory: Vec::new(),
        }
    }
}

impl ElementalEgg {
    /// Initialize retains proximity/resting state on cached reuse.
    pub fn initialize(&mut self) {
        self.memory.clear();
        self.spawn_element = E::NONE;
        self.summoner = None;
        self.proximity_timer = 0.0;
    }

    pub fn dead(&self) -> bool {
        self.spawn_element != E::NONE
    }
    pub fn resting(&self) -> bool {
        self.resting_timer < 0.0
    }

    fn remember(&mut self, element: E, value: f32) {
        if let Some(entry) = self.memory.iter_mut().find(|entry| entry.0 == element) {
            entry.1 = value;
        } else {
            self.memory.push((element, value));
        }
    }

    /// Latest Amount*Magnitude replaces memory, without health subtraction,
    /// attack-property gating, damage-feature gating or summation.
    pub fn internal_damage(&mut self, damage: Damage) -> bool {
        if damage.element & ACCEPTABLE == 0 {
            return false;
        }
        let value = damage.amount * damage.magnitude;
        if damage.element.count_ones() > 1 {
            for index in 0..11 {
                let element = 1u32 << index;
                if element & damage.element == element && element & ACCEPTABLE == element {
                    self.remember(E(element), value);
                }
            }
        } else {
            self.remember(E(damage.element), value);
        }
        true
    }

    /// Host supplies whether an Avatar centre is within the exact proximity
    /// sphere. One random bounded draw occurs only on a successful scan.
    /// Native subtracts one second once, rather than catching up all scans.
    pub fn update(
        &mut self,
        dt: f32,
        velocity_squared: f32,
        avatar_in_proximity: bool,
        random_index: impl FnOnce(usize) -> usize,
    ) {
        self.proximity_timer += dt;
        self.spawn_element = E::NONE;
        let mut strongest = f32::MIN;
        for &(element, value) in &self.memory {
            if value > strongest {
                self.spawn_element = element;
                strongest = value;
            }
        }
        if velocity_squared > 1e-6 {
            self.resting_timer = 1.0;
        } else {
            self.resting_timer -= dt;
        }
        if self.proximity > 0.0 && self.proximity_timer > 1.0 {
            self.proximity_timer -= 1.0;
            if avatar_in_proximity {
                self.spawn_element = RANDOM_ELEMENTS[random_index(RANDOM_ELEMENTS.len())];
            }
        }
    }

    /// Kill marks removal with Shield, which is outside the hatchable mask.
    pub fn kill(&mut self) {
        if self.spawn_element == E::NONE {
            self.spawn_element = E::SHIELD;
        }
    }

    /// Deinitialize spawns only on authority and for an acceptable element.
    pub fn hatch_element(&self, client: bool) -> Option<E> {
        (!client && self.spawn_element.0 & ACCEPTABLE != 0).then_some(self.spawn_element)
    }
}
