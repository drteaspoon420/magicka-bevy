//! Retained DecalManager state recovered from CIL. See research/decals.md.
//! IDs are deterministic generations, rather than DateTime.Now.Ticks.
use crate::{live_collision::inverse, particles::DotNetRandom};
use std::{cmp::Reverse, collections::BinaryHeap};

pub const SCORCHED: u8 = 20;
pub const COLD: u8 = 22;
pub const WATER: u8 = 23;
pub const GREASE: u8 = 32;
pub const CRACKS: u8 = 35;
/// Defines.DecalLimit, medium/default setting (`Quality::decal_limit`).
pub const DEFAULT_LIMIT: usize = 300;

#[derive(Clone, Copy, Debug)]
pub struct BloodDamage {
    pub position: [f32; 3],
    pub attack_position: [f32; 3],
    pub radius: f32,
    pub blood: i32,
    pub amount: f32,
    pub magnitude: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference {
    pub index: usize,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Projection {
    pub transform: [f32; 16],
    pub inverse: [f32; 16],
    /// Atlas column, row, remaining lifetime.
    pub args: [f32; 3],
    pub color: [f32; 4],
    pub animated_owner: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub cell: u8,
    pub scale: [f32; 2],
    pub position: [f32; 3],
    pub direction: Option<[f32; 3]>,
    pub normal: [f32; 3],
    pub ttl: f32,
    pub color: [f32; 4],
    pub animated_owner: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Decals {
    slots: Vec<Projection>,
    generations: Vec<Option<u64>>,
    free: BinaryHeap<Reverse<usize>>,
    last: Option<usize>,
    next_generation: u64,
    random: DotNetRandom,
    /// Native Update copies render data before aging, retaining holes.
    pub frame: Vec<Projection>,
}

impl Default for Decals {
    fn default() -> Self {
        Self::new(
            crate::global_settings::decal_quality().decal_limit(),
            0x6465_6361,
        )
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.map(|x| x / length)
}

impl Decals {
    /// FullBlast/GroundBlast decal selection and downward placement.
    pub fn blast(
        &mut self,
        radius: f32,
        position: [f32; 3],
        damages: &[magicka_sim::damage::Damage; 5],
        scalars: [f32; 8],
        ground: bool,
        scene: &crate::level_collision::SceneCollision,
    ) {
        use magicka_sim::spells::Elements as E;
        if !ground && damages.iter().fold(0, |m, d| m | d.element) & E::BEAMS.0 != 0 {
            return;
        }
        for d in damages {
            let cell = match d.element {
                x if x == E::FIRE.0 => Some(SCORCHED),
                x if x == E::COLD.0 => Some(COLD),
                x if x == E::WATER.0 => Some(WATER),
                x if ground && x & (E::EARTH | E::ICE).0 != 0 => Some(CRACKS),
                _ => None,
            };
            let Some(cell) = cell else { continue };
            for (i, fraction) in scalars.into_iter().enumerate() {
                let direction = crate::blast_visual::direction(i)
                    .map(|x| x * (radius * (fraction * (radius * 0.1))));
                let origin = std::array::from_fn(|j| position[j] + direction[j]);
                if let Some(hit) = scene.segment_intersect(
                    magicka_sim::spray_pulse::Segment {
                        origin,
                        delta: [0.0, -2.0, 0.0],
                    },
                    false,
                ) {
                    let mut position = hit.hit.position;
                    position[1] += 0.1;
                    let alpha = if cell == CRACKS {
                        1.0
                    } else {
                        (d.magnitude / 5.0).clamp(0.25, 0.75)
                    };
                    self.add(Request {
                        cell,
                        scale: [radius; 2],
                        position,
                        direction: Some(direction),
                        normal: [0.0, 1.0, 0.0],
                        ttl: 60.0,
                        color: [1.0, 1.0, 1.0, alpha],
                        animated_owner: hit.animated_owner,
                    });
                }
            }
        }
    }
    /// Character.InternalDamage. The native query ignores its success flag;
    /// use zero position/normal on a miss, preserving the degenerate transform.
    pub fn blood_damage(&mut self, b: BloodDamage, scene: &crate::level_collision::SceneCollision) {
        if !(0..5).contains(&b.blood) {
            return;
        }
        let mut origin = b.position;
        for axis in [0, 2] {
            origin[axis] += ((self.random.next_double() - 0.5) * 2.0 * b.radius as f64) as f32;
        }
        let mut delta = normalize(std::array::from_fn(|i| {
            b.position[i] - b.attack_position[i]
        }));
        delta[1] = -2.0;
        let hit = scene.segment_intersect(
            magicka_sim::spray_pulse::Segment {
                origin,
                delta: delta.map(|x| x * 5.0),
            },
            false,
        );
        let cell = (b.blood * 4 + (b.amount * b.magnitude / 400.0).min(3.0) as i32) as u8;
        self.add(Request {
            cell,
            scale: [4.0; 2],
            position: hit.map_or([0.0; 3], |h| h.hit.position),
            normal: hit.map_or([0.0; 3], |h| h.hit.normal),
            direction: None,
            ttl: 60.0,
            color: [1.0; 4],
            animated_owner: hit.and_then(|h| h.animated_owner),
        });
    }
    pub fn blood_gibs(
        &mut self,
        b: crate::gibs::GibSource,
        scene: &crate::level_collision::SceneCollision,
    ) {
        if b.frozen || !(0..5).contains(&b.blood) {
            return;
        }
        for _ in 0..if b.mass > 1000.0 { 6 } else { 4 } {
            let mut origin = b.position;
            for axis in [0, 2] {
                origin[axis] += (self.random.next_double() * b.radius as f64 * 2.0) as f32;
            }
            let delta = [
                ((self.random.next_double() - 0.5) * 4.0) as f32,
                -8.0,
                ((self.random.next_double() - 0.5) * 4.0) as f32,
            ];
            if let Some(hit) =
                scene.segment_intersect(magicka_sim::spray_pulse::Segment { origin, delta }, false)
            {
                let cell = (b.blood * 4 + self.random.next_below(3)) as u8;
                let scale = 3.0 + (self.random.next_double() * 3.0) as f32;
                self.add(Request {
                    cell,
                    scale: [scale; 2],
                    position: hit.hit.position,
                    normal: hit.hit.normal,
                    direction: None,
                    ttl: 60.0,
                    color: [1.0; 4],
                    animated_owner: hit.animated_owner,
                });
            }
        }
    }
    pub fn new(capacity: usize, seed: i32) -> Self {
        assert!(capacity > 0);
        let empty = Projection {
            transform: [0.0; 16],
            inverse: [0.0; 16],
            args: [0.0; 3],
            color: [0.0; 4],
            animated_owner: None,
        };
        Self {
            slots: vec![empty; capacity],
            generations: vec![None; capacity],
            free: (0..capacity).map(Reverse).collect(),
            last: None,
            next_generation: 0,
            random: DotNetRandom::new(seed),
            frame: vec![],
        }
    }
    pub fn clear(&mut self) {
        self.last = None;
        self.free = (0..self.slots.len()).map(Reverse).collect();
        self.generations.fill(None);
        self.frame.clear();
    }
    pub fn add(&mut self, r: Request) -> Option<Reference> {
        let index = if let Some(Reverse(i)) = self.free.pop() {
            i
        } else {
            let mut minimum = f32::MAX;
            let mut candidate = None;
            for (i, p) in self.slots.iter().enumerate() {
                if p.args[2] < minimum {
                    minimum = p.args[2];
                    candidate = Some(i);
                }
            }
            candidate?
        };
        let direction = r.direction.unwrap_or_else(|| loop {
            let up = normalize(std::array::from_fn(|_| {
                self.random.next_double() as f32 - 0.5
            }));
            let dot = up.iter().zip(r.normal).map(|(a, b)| a * b).sum::<f32>();
            if !dot.is_nan() && (-0.99..=0.99).contains(&dot) {
                break up;
            }
        });
        let right = normalize(cross(direction, r.normal));
        let up = cross(r.normal, right);
        let transform = [
            right[0] * r.scale[0],
            right[1] * r.scale[0],
            right[2] * r.scale[0],
            0.0,
            up[0] * r.scale[1],
            up[1] * r.scale[1],
            up[2] * r.scale[1],
            0.0,
            r.normal[0],
            r.normal[1],
            r.normal[2],
            0.0,
            r.position[0],
            r.position[1],
            r.position[2],
            1.0,
        ];
        self.slots[index] = Projection {
            transform,
            inverse: inverse(&transform).unwrap_or([f32::NAN; 16]),
            args: [(r.cell % 8) as f32, (r.cell / 8) as f32, r.ttl],
            color: r.color,
            animated_owner: r.animated_owner,
        };
        self.next_generation = self.next_generation.wrapping_add(1);
        self.generations[index] = Some(self.next_generation);
        self.last = Some(self.last.map_or(index, |last| last.max(index)));
        Some(Reference {
            index,
            generation: self.next_generation,
        })
    }
    /// Every live decal, for publishing to network clients.
    pub fn live(&self) -> impl Iterator<Item = (Reference, &Projection)> {
        self.slots
            .iter()
            .zip(&self.generations)
            .enumerate()
            .filter_map(|(index, (projection, generation))| {
                generation.map(|generation| (Reference { index, generation }, projection))
            })
    }
    /// Network clients: this slot looks as the host says. Clients add no decals
    /// of their own, so the host's slot numbers can be used directly.
    pub fn mirror(&mut self, r: Reference, projection: Projection) {
        if r.index >= self.slots.len() {
            return;
        }
        self.slots[r.index] = projection;
        self.generations[r.index] = Some(r.generation);
        self.free.retain(|Reverse(i)| *i != r.index);
        self.last = Some(self.last.map_or(r.index, |last| last.max(r.index)));
    }
    pub fn get(&self, r: Reference) -> Option<&Projection> {
        (self.generations.get(r.index) == Some(&Some(r.generation))).then(|| &self.slots[r.index])
    }
    /// AddAlphaBlendedDecalAlpha checks the reference before clamping alpha.
    pub fn add_alpha(&mut self, r: Reference, additional: f32) -> bool {
        if self.generations.get(r.index) != Some(&Some(r.generation)) {
            return false;
        }
        self.slots[r.index].color[3] = (self.slots[r.index].color[3] + additional).clamp(0.0, 1.0);
        true
    }
    /// DecalManager.GetDecalTTL: None once the reference is stale.
    pub fn ttl(&self, r: Reference) -> Option<f32> {
        self.get(r).map(|p| p.args[2])
    }
    /// DecalManager.SetDecalTTL: false once the reference is stale.
    pub fn set_ttl(&mut self, r: Reference, ttl: f32) -> bool {
        if self.generations.get(r.index) != Some(&Some(r.generation)) {
            return false;
        }
        self.slots[r.index].args[2] = ttl;
        true
    }
    /// DecalManager.SetDecal(ref, ttl, transform, alpha): false (and the
    /// caller re-adds) once the reference is stale.
    pub fn set(&mut self, r: Reference, ttl: f32, transform: &[f32; 16], alpha: f32) -> bool {
        if self.generations.get(r.index) != Some(&Some(r.generation)) {
            return false;
        }
        let p = &mut self.slots[r.index];
        p.transform = *transform;
        p.inverse = inverse(transform).unwrap_or([f32::NAN; 16]);
        p.args[2] = ttl;
        p.color[3] = alpha;
        true
    }
    pub fn has_owner(&self, owner: u32) -> bool {
        self.slots
            .iter()
            .zip(&self.generations)
            .any(|(p, generation)| generation.is_some() && p.animated_owner == Some(owner))
    }
    /// AnimatedLevelPart.Update / TransformDecal: right-multiply each live
    /// projection by inverse(old part world) * new part world.
    pub fn transform_owner(&mut self, owner: u32, delta: &[f32; 16]) {
        for (p, generation) in self.slots.iter_mut().zip(&self.generations) {
            if generation.is_some() && p.animated_owner == Some(owner) {
                p.transform = magicka_content::skinned::mul(&p.transform, delta);
                p.inverse = inverse(&p.transform).unwrap_or([f32::NAN; 16]);
            }
        }
    }
    pub fn update(&mut self, dt: f32) {
        self.frame = self
            .last
            .map_or_else(Vec::new, |last| self.slots[..=last].to_vec());
        let age =
            dt * (1.0 + 19.0 * (1.0 - self.free.len() as f32 / self.slots.len() as f32).powf(8.0));
        if let Some(last) = self.last {
            for i in 0..=last {
                let old = self.slots[i].args[2];
                let new = old - age;
                self.slots[i].args[2] = new;
                if new < f32::from_bits(1) && old >= f32::from_bits(1) {
                    self.generations[i] = None;
                    self.free.push(Reverse(i));
                }
            }
        }
        while let Some(last) = self.last {
            if self.slots[last].args[2].partial_cmp(&0.0) != Some(std::cmp::Ordering::Less) {
                break;
            }
            self.last = last.checked_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn floor() -> crate::level_collision::SceneCollision {
        use magicka_content::level::TriMesh;
        let mut layers = vec![TriMesh::default(); 10];
        layers[0] = TriMesh {
            vertices: vec![
                [-100.0, 0.0, -100.0],
                [100.0, 0.0, -100.0],
                [0.0, 0.0, 100.0],
            ],
            triangles: vec![[0, 2, 1]],
        };
        crate::level_collision::SceneCollision {
            static_skin: crate::level_collision::StaticLevelCollision::from_layers(&layers)
                .unwrap(),
            force_fields: vec![],
            animated_parts: vec![],
            water: vec![],
        }
    }
    fn request(ttl: f32) -> Request {
        Request {
            cell: COLD,
            scale: [4.0; 2],
            position: [1.0, 2.0, 3.0],
            direction: Some([1.0, 0.0, 0.0]),
            normal: [0.0, 1.0, 0.0],
            ttl,
            color: [1.0; 4],
            animated_owner: None,
        }
    }
    #[test]
    fn snapshot_precedes_aging_and_pool_pressure_is_native() {
        let mut d = Decals::new(2, 1);
        let r = d.add(request(60.0)).unwrap();
        d.update(1.0);
        assert_eq!(d.frame[0].args, [6.0, 2.0, 60.0]);
        assert_eq!(d.get(r).unwrap().args[2], 60.0 - (1.0 + 19.0 / 256.0));
        d.add(request(60.0));
        d.update(1.0);
        assert_eq!(
            d.get(r).unwrap().args[2],
            60.0 - (1.0 + 19.0 / 256.0) - 20.0
        );
    }
    #[test]
    fn lowest_ttl_first_tie_and_stale_reference() {
        let mut d = Decals::new(2, 1);
        let first = d.add(request(2.0)).unwrap();
        let second = d.add(request(2.0)).unwrap();
        let replacement = d.add(request(3.0)).unwrap();
        assert_eq!(replacement.index, first.index);
        assert!(d.get(first).is_none());
        assert!(d.get(second).is_some());
        d.update(1.0);
        assert!(d.get(second).is_none());
        assert_eq!(d.add(request(60.0)).unwrap().index, 0);
    }
    #[test]
    fn projection_uses_normal_as_unscaled_depth_axis() {
        let mut d = Decals::new(1, 1);
        let r = d.add(request(60.0)).unwrap();
        let p = d.get(r).unwrap();
        assert_eq!(&p.transform[8..12], &[0.0, 1.0, 0.0, 0.0]);
        assert_eq!(&p.transform[12..16], &[1.0, 2.0, 3.0, 1.0]);
        assert!(p.inverse.iter().all(|x| x.is_finite()));
    }
    #[test]
    fn blast_retains_quadratic_offset_and_beam_suppression() {
        use magicka_sim::{damage::Damage, spells::Elements as E};
        let mut d = Decals::new(30, 1);
        let mut damage = [Damage::default(); 5];
        damage[0] = Damage {
            element: E::FIRE.0,
            magnitude: 2.0,
            ..Default::default()
        };
        d.blast(4.0, [0.0, 1.0, 0.0], &damage, [1.0; 8], false, &floor());
        d.update(0.0);
        assert_eq!(d.frame.len(), 8);
        assert_eq!(d.frame[0].transform[12], 1.6);
        assert_eq!(d.frame[0].transform[13], 0.1);
        assert_eq!(d.frame[0].args, [4.0, 2.0, 60.0]);
        assert_eq!(d.frame[0].color[3], 0.4);
        damage[1].element = E::ARCANE.0;
        d.clear();
        d.blast(4.0, [0.0, 1.0, 0.0], &damage, [1.0; 8], false, &floor());
        d.update(0.0);
        assert!(d.frame.is_empty());
    }
    #[test]
    fn blood_variants_and_frozen_gib_gate() {
        let mut d = Decals::new(30, 1);
        d.blood_damage(
            BloodDamage {
                position: [0.0, 1.0, 0.0],
                attack_position: [0.0, 1.0, -1.0],
                radius: 0.5,
                blood: 1,
                amount: 1200.0,
                magnitude: 1.0,
            },
            &floor(),
        );
        d.update(0.0);
        assert_eq!(d.frame[0].args, [7.0, 0.0, 60.0]);
        let mut source = crate::gibs::GibSource {
            position: [0.0, 1.0, 0.0],
            capsule_radius: 0.5,
            capsule_length: 1.0,
            radius: 0.5,
            mass: 1200.0,
            frozen: true,
            blood: 4,
        };
        d.clear();
        d.blood_gibs(source, &floor());
        d.update(0.0);
        assert!(d.frame.is_empty());
        source.frozen = false;
        d.blood_gibs(source, &floor());
        d.update(0.0);
        assert_eq!(d.frame.len(), 6);
        assert!(d.frame.iter().all(|p| p.args[0] <= 2.0 && p.args[1] == 2.0));
    }
}
