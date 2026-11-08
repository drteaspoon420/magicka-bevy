//! Spell-owned lights, separate from particle emitters (research/spell-lighting.md).
use crate::{arcane_blade::CapsuleLight, arcane_blast::DynamicLight, avatar::AvatarEvent};
use magicka_sim::{
    character_physics::{quaternion_lerp, Orientation},
    spells::{Elements as E, Spell},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct SprayLights {
    active: BTreeMap<u16, SprayLight>,
    fading: Vec<DynamicLight>,
}
struct SprayLight {
    range: f32,
    light: DynamicLight,
}

impl SprayLights {
    pub fn event(&mut self, owner: u16, event: &AvatarEvent, orientation: Orientation) {
        match event {
            AvatarEvent::SprayStarted {
                spell,
                range,
                position,
                direction,
            } => {
                self.stop(owner);
                if let Some(light) = spray_light(spell, *range, *position, *direction) {
                    self.active.insert(
                        owner,
                        SprayLight {
                            range: *range,
                            light,
                        },
                    );
                }
            }
            AvatarEvent::SprayDecalUpdate(frame) => {
                if let Some(spray) = self.active.get_mut(&owner) {
                    // CastUpdate steers the half-range light by Quaternion.Lerp
                    // from its previous heading to the owner's body orientation.
                    let old = std::array::from_fn(|i| spray.light.position[i] - frame.position[i]);
                    let matrix = crate::particles::create_world([0.0; 3], old, [0.0, 1.0, 0.0]);
                    let previous = Orientation {
                        right: [matrix[0], matrix[1], matrix[2]],
                        up: [matrix[4], matrix[5], matrix[6]],
                        backward: [matrix[8], matrix[9], matrix[10]],
                    };
                    let rotation = quaternion_lerp(
                        previous.to_quaternion(),
                        orientation.to_quaternion(),
                        (frame.dt * 4.0).clamp(0.0, 1.0),
                    );
                    let forward = Orientation::from_quaternion(rotation).forward();
                    spray.light.position =
                        std::array::from_fn(|i| frame.position[i] + forward[i] * spray.range * 0.5);
                }
            }
            AvatarEvent::SprayStopped => self.stop(owner),
            _ => {}
        }
    }
    fn stop(&mut self, owner: u16) {
        if let Some(mut spray) = self.active.remove(&owner) {
            spray.light.stop();
            self.fading.push(spray.light);
        }
    }
    pub fn retain_owners(&mut self, owners: &BTreeSet<u16>) {
        let removed: Vec<_> = self
            .active
            .keys()
            .filter(|h| !owners.contains(h))
            .copied()
            .collect();
        for owner in removed {
            self.stop(owner);
        }
    }
    pub fn update(&mut self, dt: f32) {
        for spray in self.active.values_mut() {
            spray.light.update(dt);
        }
        for light in &mut self.fading {
            light.update(dt);
        }
        self.fading.retain(|light| !light.is_removed());
    }
    pub fn lights(&self) -> impl Iterator<Item = &DynamicLight> {
        self.active
            .values()
            .map(|s| &s.light)
            .chain(self.fading.iter())
    }
}

/// SpraySpell.CastForce: unnormalised weighted sum of five Spell colours.
/// Water, Steam and Poison do not contribute to this light.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn spray_light(
    spell: &Spell,
    range: f32,
    source: [f32; 3],
    direction: [f32; 3],
) -> Option<DynamicLight> {
    let mut color = [0.0; 3];
    for (element, tint) in [
        (E::ARCANE, [2.0, 0.4, 0.6]),
        (E::COLD, [1.0, 1.0, 1.4]),
        (E::FIRE, [1.8, 0.6, 0.4]),
        (E::LIFE, [0.2, 1.6, 0.2]),
        (E::LIGHTNING, [0.75, 0.5, 1.0]),
    ] {
        for i in 0..3 {
            color[i] += tint[i] * spell.get(element);
        }
    }
    if !(color.iter().map(|v| v * v).sum::<f32>() > f32::from_bits(1)) {
        return None;
    }
    let diffuse = color.map(|v| v * 1.5);
    let position = std::array::from_fn(|i| source[i] + direction[i] * range * 0.5);
    let specular = diffuse.iter().map(|v| v * v).sum::<f32>().sqrt();
    let mut light = DynamicLight::new(position, diffuse, 1.0, range * 1.25, 2.0, specular, -1.0);
    light.ambient = diffuse.map(|v| v * 0.25);
    Some(light)
}

pub fn beam_light(beam: &magicka_sim::railgun::RailRender) -> CapsuleLight {
    CapsuleLight {
        start: beam.light.0,
        end: beam.light.1,
        radius: 5.0,
        diffuse: beam.light_color,
        ambient: beam
            .light_color
            .map(|v| if beam.branch { v * 0.5 } else { 0.0 }),
        intensity: 0.6,
    }
}

pub fn bolt_light(bolt: &crate::play_session::LiveBolt) -> CapsuleLight {
    CapsuleLight {
        start: bolt.visual.from,
        end: bolt.visual.to,
        radius: 3.0 + 2.0 * bolt.visual.scale,
        diffuse: bolt.visual.color,
        ambient: [0.0; 3],
        intensity: bolt.ttl.max(0.0).sqrt() * 1.5,
    }
}

pub struct StatusLightOwner {
    pub handle: u16,
    pub position: [f32; 3],
    pub kind: StatusLightKind,
}
pub enum StatusLightKind {
    Character { burning: bool, steamed: bool },
    Physics { burning: bool, magnitude: f32 },
}
struct StatusLight {
    light: DynamicLight,
    variation: crate::light_variation::LightVariation,
    base_intensity: f32,
}
#[derive(Default)]
pub struct StatusLights {
    active: BTreeMap<u16, StatusLight>,
    fading: Vec<StatusLight>,
}
impl StatusLights {
    /// Character.UpdateStatusEffects: Steam precedes Burning. While steamed,
    /// an existing fire light remains, but a new one is not created.
    pub fn sync(
        &mut self,
        owners: impl IntoIterator<Item = StatusLightOwner>,
        dt: f32,
        random: &mut crate::particles::DotNetRandom,
    ) {
        let mut retained = BTreeSet::new();
        for owner in owners {
            retained.insert(owner.handle);
            if let Some(state) = self.active.get_mut(&owner.handle) {
                state.light.position = owner.position;
            }
            let (burning, steamed, base_intensity, ambient) = match owner.kind {
                StatusLightKind::Character { burning, steamed } => {
                    (burning, steamed, 1.0, [0.4, 0.2, 0.0])
                }
                StatusLightKind::Physics { burning, magnitude } => {
                    (burning, false, 3.0 + magnitude.max(0.0).sqrt(), [0.2; 3])
                }
            };
            if steamed {
                continue;
            }
            if burning {
                self.active
                    .entry(owner.handle)
                    .or_insert_with(|| {
                        let mut light = DynamicLight::new(
                            owner.position,
                            [1.0, 0.4, 0.0],
                            1.0,
                            5.0,
                            1.0,
                            0.5,
                            -1.0,
                        );
                        light.ambient = ambient;
                        StatusLight {
                            light,
                            variation: Default::default(),
                            base_intensity,
                        }
                    })
                    .base_intensity = base_intensity;
            } else {
                self.stop(owner.handle);
            }
        }
        let removed: Vec<_> = self
            .active
            .keys()
            .filter(|h| !retained.contains(h))
            .copied()
            .collect();
        for handle in removed {
            self.stop(handle);
        }
        for state in self.active.values_mut().chain(self.fading.iter_mut()) {
            state.light.update(dt);
            state.light.dynamic_intensity =
                state.base_intensity * state.variation.intensity(3, dt, 4.0, 0.2, random);
        }
        self.fading.retain(|s| !s.light.is_removed());
    }
    fn stop(&mut self, handle: u16) {
        if let Some(mut state) = self.active.remove(&handle) {
            state.light.stop();
            self.fading.push(state);
        }
    }
    pub fn lights(&self) -> impl Iterator<Item = &DynamicLight> {
        self.active
            .values()
            .chain(self.fading.iter())
            .map(|s| &s.light)
    }
}
