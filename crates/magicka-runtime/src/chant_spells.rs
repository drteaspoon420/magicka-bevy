//! ChantSpells / ChantSpellManager (research/chant-spells.md): every conjured
//! element becomes an `elementeffect_<element>` particle effect with a
//! coloured DynamicLight that orbits the caster while it chants; combining
//! or cancelling elements merges two orbs, and a full queue makes the new orb
//! escape. Orbs stop once the caster stops chanting (the queue was cast).
use crate::arcane_blast::DynamicLight;
use crate::particles::{EffectRef, EffectSystem};
use crate::scene_loading::custom_hash;
use magicka_sim::math_approximation::fast_sin_cos;
use magicka_sim::spells::Elements as E;

/// ChantSpellManager.MAXELEMENTS (slot 0 is never used).
pub const MAX_ELEMENTS: usize = 128;

type Vector = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChantState {
    Orbiting,
    Escaping,
    Merging,
    MoveToTarget,
}

/// Defines.ChantEffects by Defines.ElementIndex.
fn effect_name(element: E) -> &'static str {
    match element {
        E::EARTH => "elementeffect_earth",
        E::WATER => "elementeffect_water",
        E::COLD => "elementeffect_cold",
        E::FIRE => "elementeffect_fire",
        E::LIGHTNING => "elementeffect_lightning",
        E::ARCANE => "elementeffect_arcane",
        E::LIFE => "elementeffect_life",
        E::SHIELD => "elementeffect_shield",
        E::ICE => "elementeffect_ice",
        E::STEAM => "elementeffect_steam",
        E::POISON => "elementeffect_poison",
        _ => "elementeffect_cold",
    }
}

/// The ChantSpells light colour (Spell.<ELEMENT>COLOR; Earth and Steam
/// get none), before the 0.75 scale.
fn light_color(element: E) -> Vector {
    match element {
        E::WATER => [0.0, 0.7, 1.3],
        E::COLD => [1.0, 1.0, 1.4],
        E::FIRE => [1.8, 0.6, 0.4],
        E::LIGHTNING => [0.75, 0.5, 1.0],
        E::ARCANE => [2.0, 0.4, 0.6],
        E::LIFE | E::POISON => [0.2, 1.6, 0.2],
        E::SHIELD => [2.0, 1.5, 1.0],
        E::ICE => [0.8, 0.9, 1.4],
        _ => [0.0; 3],
    }
}

/// Facts about the owner each update.
#[derive(Clone, Copy, Debug)]
pub struct OwnerFacts {
    pub position: Vector,
    pub chanting: bool,
    pub dead: bool,
    pub polymorphed: bool,
    /// Wet without the WetLightning passive.
    pub wet: bool,
    /// Owner.SpellQueue.Count.
    pub queue_len: usize,
}

#[derive(Clone, Debug)]
pub struct ChantSpell {
    pub state: ChantState,
    pub element: E,
    new_element: E,
    pub owner: Option<u16>,
    horizontal_time: f32,
    vertical_time: f32,
    horizontal_offset: f32,
    vertical_offset: f32,
    horizontal_speed: f32,
    vertical_speed: f32,
    target: Vector,
    source: Vector,
    pub position: Vector,
    merge_target: usize,
    pub ttl: f32,
    pub light: Option<DynamicLight>,
    effect: EffectRef,
}

/// What an update asks of the session (wet lightning feedback).
#[derive(Clone, Debug, PartialEq)]
pub enum ChantRequest {
    /// Orbiting Lightning on a wet owner: half-magnitude self Lightning
    /// damage, a bolt from the orb to the owner, and the first Lightning
    /// removed from the owner's queue (and icons).
    WetLightning { owner: u16, from: Vector },
}

#[derive(Clone, Debug)]
pub struct ChantSpellManager {
    slots: Vec<Option<ChantSpell>>,
}

impl Default for ChantSpellManager {
    fn default() -> Self {
        Self {
            slots: vec![None; MAX_ELEMENTS],
        }
    }
}

fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn normalize(v: Vector) -> Vector {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 {
        v.map(|x| x / l)
    } else {
        // XNA Vector3.Normalize(0) yields NaN; keep the orb still instead.
        [0.0; 3]
    }
}
fn wrap_angle(mut a: f32) -> f32 {
    use std::f32::consts::PI;
    a = (a + PI).rem_euclid(2.0 * PI) - PI;
    a
}

impl ChantSpell {
    /// ChantSpells(element, owner): orbiting (escaping when the owner's
    /// queue already holds five), starting at the cast source.
    fn new(
        element: E,
        owner: u16,
        queue_len: usize,
        cast_source: Vector,
        owner_position: Vector,
        direction: Vector,
        random: &mut impl FnMut() -> f32,
        effects: &mut EffectSystem,
    ) -> Self {
        let position = if cast_source.iter().map(|v| v * v).sum::<f32>() > f32::EPSILON {
            cast_source
        } else {
            owner_position
        };
        let mut s = Self {
            state: if queue_len >= 5 {
                ChantState::Escaping
            } else {
                ChantState::Orbiting
            },
            element,
            new_element: E::NONE,
            owner: Some(owner),
            horizontal_time: 0.0,
            vertical_time: 0.0,
            horizontal_offset: 0.0,
            vertical_offset: 0.0,
            horizontal_speed: 0.0,
            vertical_speed: 0.0,
            target: position,
            source: owner_position,
            position,
            merge_target: 0,
            ttl: 0.0,
            light: None,
            effect: EffectRef::default(),
        };
        s.randomize(random);
        s.start_visuals(direction, effects, random);
        s
    }

    fn randomize(&mut self, random: &mut impl FnMut() -> f32) {
        self.horizontal_time = 0.0;
        self.vertical_time = 0.0;
        self.horizontal_offset = random() * 0.5 + 0.75;
        self.vertical_offset = random();
        self.horizontal_speed = random() + 0.5;
        self.vertical_speed = (random() - 0.5) * 2.0;
        if self.vertical_speed < 0.0 {
            self.horizontal_speed *= -1.0;
        }
        self.target = [
            self.position[0] + random() - 0.5,
            self.position[1],
            self.position[2] + random() - 0.5,
        ];
    }

    fn start_visuals(
        &mut self,
        direction: Vector,
        effects: &mut EffectSystem,
        random: &mut impl FnMut() -> f32,
    ) {
        let color = light_color(self.element).map(|c| c * 0.75);
        self.light = if color.iter().map(|c| c * c).sum::<f32>() > 0.1 {
            // Initialize(position, color, 0.5, 7, 5, 1) + AmbientColor
            // color·0.25 + Enable; no TTL.
            let mut light = DynamicLight::new(self.position, color, 0.5, 7.0, 5.0, 1.0, -1.0);
            light.ambient = color.map(|c| c * 0.25);
            Some(light)
        } else {
            None
        };
        self.effect = effects
            .start_at(
                custom_hash(effect_name(self.element)),
                self.position,
                direction,
            )
            .unwrap_or_default();
        self.ttl = 1.0 + random();
    }

    fn stop(&mut self, effects: &mut EffectSystem) {
        effects.stop(&mut self.effect);
        if let Some(light) = &mut self.light {
            light.stop();
        }
    }
}

impl ChantSpellManager {
    /// ChantSpellManager.Add: the smallest free slot (IntHeap), if any.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        element: E,
        owner: u16,
        queue_len: usize,
        cast_source: Vector,
        owner_position: Vector,
        direction: Vector,
        random: &mut impl FnMut() -> f32,
        effects: &mut EffectSystem,
    ) -> Option<usize> {
        let index = (1..MAX_ELEMENTS).find(|&i| self.slots[i].is_none())?;
        self.slots[index] = Some(ChantSpell::new(
            element,
            owner,
            queue_len,
            cast_source,
            owner_position,
            direction,
            random,
            effects,
        ));
        Some(index)
    }

    /// ChantSpellManager.Merge: the owner's first non-merging orbs of
    /// element `a` and `b` merge into `out` (none: they vanish).
    pub fn merge(&mut self, owner: u16, a: E, b: E, out: E) {
        let (mut first, mut second) = (None, None);
        for (i, slot) in self.slots.iter().enumerate().skip(1) {
            let Some(s) = slot else { continue };
            if s.owner != Some(owner) || s.state == ChantState::Merging {
                continue;
            }
            if s.element == a && first.is_none() {
                first = Some(i);
            } else if s.element == b && second.is_none() {
                second = Some(i);
            }
        }
        let (Some(first), Some(second)) = (first, second) else {
            return;
        };
        // MergeWith: both merge toward each other, both carry the result.
        for (i, target) in [(first, second), (second, first)] {
            let s = self.slots[i].as_mut().unwrap();
            s.state = ChantState::Merging;
            s.merge_target = target;
            if out != E::NONE {
                s.new_element = out;
            }
        }
    }

    /// ChantSpellManager.Update for every active orb.
    pub fn update(
        &mut self,
        dt: f32,
        facts: &mut impl FnMut(u16) -> Option<OwnerFacts>,
        random: &mut impl FnMut() -> f32,
        effects: &mut EffectSystem,
        requests: &mut Vec<ChantRequest>,
    ) {
        for i in 1..MAX_ELEMENTS {
            let Some(mut s) = self.slots[i].take() else {
                continue;
            };
            let mut keep = true;
            match s.state {
                ChantState::Orbiting => {
                    let owner = s.owner.and_then(&mut *facts);
                    match owner {
                        None => {
                            s.owner = None;
                            s.state = ChantState::Escaping;
                            s.ttl = 1.0;
                        }
                        Some(o) if o.dead || o.polymorphed => {
                            s.owner = None;
                            s.state = ChantState::Escaping;
                            s.ttl = 1.0;
                        }
                        Some(o) if s.element == E::LIGHTNING && o.wet => {
                            requests.push(ChantRequest::WetLightning {
                                owner: s.owner.unwrap(),
                                from: s.position,
                            });
                            s.state = ChantState::MoveToTarget;
                            s.ttl = 0.0;
                        }
                        Some(o) => {
                            s.source = o.position;
                            if !o.chanting {
                                s.stop(effects);
                                keep = false;
                            } else {
                                s.vertical_time =
                                    wrap_angle(s.vertical_time + dt * 4.0 * s.vertical_speed);
                                s.horizontal_time =
                                    wrap_angle(s.horizontal_time + dt * 5.0 * s.horizontal_speed);
                                let (hs, hc) = fast_sin_cos(s.horizontal_time);
                                let (vs, _) = fast_sin_cos(s.vertical_time);
                                s.target = [
                                    s.source[0] + hs * s.horizontal_offset,
                                    s.source[1] + vs * s.vertical_offset,
                                    s.source[2] + hc * s.horizontal_offset,
                                ];
                            }
                        }
                    }
                }
                ChantState::Escaping => {
                    s.ttl -= dt;
                    let n = (s.ttl * std::f32::consts::PI).sin();
                    let d = normalize(sub(s.target, s.position));
                    s.target = [
                        s.target[0] + d[0] + n + 0.4,
                        s.target[1] + d[1] + n * 0.2 + 0.4,
                        s.target[2] + d[2] + n * -1.2 - 0.4,
                    ];
                    if s.ttl < 0.0 {
                        s.stop(effects);
                        keep = false;
                    }
                }
                ChantState::Merging => {
                    let partner = s.merge_target;
                    match self
                        .slots
                        .get(partner)
                        .and_then(|p| p.as_ref())
                        .map(|p| p.position)
                    {
                        Some(p) => {
                            s.target = p;
                            let d = sub(s.position, p);
                            if d.iter().map(|v| v * v).sum::<f32>() < 0.02 {
                                if let Some(mut other) = self.slots[partner].take() {
                                    other.stop(effects);
                                }
                                if s.new_element != E::NONE {
                                    // Reinitialize as the combined element.
                                    s.stop(effects);
                                    s.state = ChantState::Orbiting;
                                    s.element = s.new_element;
                                    s.new_element = E::NONE;
                                    s.randomize(random);
                                    let queue_full = s
                                        .owner
                                        .and_then(&mut *facts)
                                        .is_some_and(|o| o.queue_len >= 5);
                                    if queue_full {
                                        s.state = ChantState::Escaping;
                                    }
                                    s.start_visuals([0.0, 0.0, -1.0], effects, random);
                                } else {
                                    s.stop(effects);
                                    keep = false;
                                }
                                if keep {
                                    self.slots[i] = Some(s);
                                }
                                continue;
                            }
                        }
                        None => {
                            // The partner already stopped (native reads a
                            // default struct): fade out.
                            s.stop(effects);
                            keep = false;
                        }
                    }
                }
                ChantState::MoveToTarget => {
                    if s.ttl <= 0.0 {
                        s.stop(effects);
                        keep = false;
                    }
                }
            }
            if keep {
                let d = normalize(sub(s.target, s.position));
                s.position = std::array::from_fn(|k| s.position[k] + d[k] * dt * 5.0);
                effects.update_position_direction(&mut s.effect, s.position, [0.0, 0.0, -1.0]);
                if let Some(light) = &mut s.light {
                    light.position = s.position;
                }
                self.slots[i] = Some(s);
            }
        }
    }

    /// Active orbs (presentation and tests).
    pub fn active(&self) -> impl Iterator<Item = &ChantSpell> {
        self.slots.iter().flatten()
    }

    /// Lights of active orbs, updated with their transitions.
    pub fn update_lights(&mut self, dt: f32) {
        for s in self.slots.iter_mut().flatten() {
            if let Some(light) = &mut s.light {
                light.update(dt);
            }
        }
    }
}
