//! SpraySpell's 24 direction buckets and four retained decal references.
//! See evidence/decals/spray-il.txt and research/decals.md.
use crate::{
    decals::{Decals, Reference, Request, COLD, SCORCHED, WATER},
    level_collision::SceneHit,
    particles::DotNetRandom,
};
use magicka_sim::{
    spells::{Elements as E, Spell},
    spray_pulse::Segment,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Empty,
    Miss,
    Live(Reference),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub dt: f32,
    /// Range*0.1 after level/shield clipping, independent of charge ramp.
    pub scale: f32,
    pub direction: [f32; 3],
    pub position: [f32; 3],
    /// Always CastSource even when Force presentation uses WeaponSource.
    pub cast_position: [f32; 3],
}
#[derive(Debug)]
pub struct Update {
    pub key: usize,
    pub fire: bool,
    /// Newly projected marks; the last two can start fire_scourge.
    pub created: Vec<(usize, [f32; 3])>,
}
#[derive(Clone, Debug)]
pub struct SprayDecals {
    slots: [[Slot; 4]; 24],
    cell: u8,
    status: bool,
    fire: bool,
    random: DotNetRandom,
}
impl Default for SprayDecals {
    fn default() -> Self {
        Self {
            slots: [[Slot::Empty; 4]; 24],
            cell: SCORCHED,
            status: false,
            fire: false,
            random: DotNetRandom::new(0x7370_7261),
        }
    }
}
/// MagickaMath.Angle(Vector2) then NormalizeAngle: acos(X), Z sign,
/// add 2pi if negative, floor into 24 buckets. Does not normalize direction.
pub fn direction_key(direction: [f32; 3]) -> Option<usize> {
    let mut angle = (direction[0].clamp(-1.0, 1.0) as f64).acos() as f32;
    if direction[2] < 0.0 {
        angle *= -1.0;
    }
    if angle < 0.0 {
        angle += std::f32::consts::TAU;
    }
    let key = (angle * (12.0 / std::f32::consts::PI)).floor();
    if !key.is_finite() || !(0.0..24.0).contains(&key) {
        None
    } else {
        Some(key as usize)
    }
}
impl SprayDecals {
    pub fn begin(&mut self, spell: Spell) {
        self.clear();
        // StatusEffects enum is 0x64e (includes Steam/Life/Poison).
        self.status = spell.element.0 & 0x64e != 0;
        self.fire = spell.get(E::FIRE) >= 1.0;
        if spell.element.0 & E::FIRE.0 != 0 {
            self.cell = SCORCHED;
        } else if spell.element.0 & E::COLD.0 != 0 {
            self.cell = COLD;
        } else if spell.element.0 & E::WATER.0 != 0 {
            self.cell = WATER;
        }
        // Other status elements leave the cached mDecalType unchanged.
    }
    pub fn clear(&mut self) {
        self.slots = [[Slot::Empty; 4]; 24];
    }
    pub fn update(
        &mut self,
        frame: Frame,
        pool: &mut Decals,
        mut query: impl FnMut(Segment) -> Option<SceneHit>,
    ) -> Option<Update> {
        if !self.status {
            return None;
        }
        let key = direction_key(frame.direction)?;
        let mut result = Update {
            key,
            fire: self.fire,
            created: vec![],
        };
        for k in 0..4 {
            match self.slots[key][k] {
                Slot::Live(r) => {
                    if !pool.add_alpha(r, frame.dt * 0.5 * (k + 1) as f32) {
                        self.slots[key][k] = Slot::Empty;
                    }
                }
                Slot::Miss => {}
                Slot::Empty => {
                    let range = frame.scale * 10.0 + 2.0;
                    let extent = (k + 1) as f32 * frame.scale;
                    let scale = if extent.is_nan() {
                        extent
                    } else {
                        extent.max(2.5)
                    };
                    let offset = range * 0.2 + k as f32 * 0.2 * range * 0.6;
                    let mut origin = std::array::from_fn(|i| {
                        frame.cast_position[i] + frame.direction[i] * offset
                    });
                    let distance = origin
                        .iter()
                        .zip(frame.position)
                        .map(|(a, b)| (a - b) * (a - b))
                        .sum::<f32>()
                        .sqrt();
                    if distance < range - 1.0 {
                        // Native jitter is X and Y, not X and Z; draws even at k=0.
                        for axis in [0, 1] {
                            origin[axis] +=
                                (self.random.next_double() - 0.5) as f32 * scale * 0.125 * k as f32;
                        }
                        if let Some(hit) = query(Segment {
                            origin,
                            delta: [0.0, -2.0, 0.0],
                        }) {
                            if let Some(r) = pool.add(Request {
                                cell: self.cell,
                                scale: [scale; 2],
                                position: hit.hit.position,
                                normal: hit.hit.normal,
                                direction: None,
                                ttl: 60.0,
                                color: [1.0, 1.0, 1.0, 0.0],
                                animated_owner: hit.animated_owner,
                            }) {
                                self.slots[key][k] = Slot::Live(r);
                            }
                            result.created.push((k, hit.hit.position));
                        } else {
                            self.slots[key][k] = Slot::Miss;
                        }
                    } else {
                        self.slots[key][k] = Slot::Miss;
                    }
                }
            }
        }
        Some(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn frame() -> Frame {
        Frame {
            dt: 0.1,
            scale: 0.7,
            direction: [1.0, 0.0, 0.0],
            position: [0.0, 1.0, 0.0],
            cast_position: [0.0, 1.0, 0.0],
        }
    }
    fn hit(s: Segment) -> Option<SceneHit> {
        Some(SceneHit {
            hit: magicka_sim::collision_skin::PrimitiveHit {
                fraction: 0.5,
                position: [s.origin[0], 0.0, s.origin[2]],
                normal: [0.0, 1.0, 0.0],
            },
            primitive: 0,
            animated_part: None,
            animated_owner: None,
        })
    }
    fn spray() -> SprayDecals {
        let mut s = SprayDecals::default();
        s.begin(Spell::default_for(E::FIRE));
        s
    }
    #[test]
    fn retained_slots_accumulate_alpha_without_moving_or_refreshing_ttl() {
        let mut s = spray();
        let mut p = Decals::new(100, 1);
        let u = s.update(frame(), &mut p, hit).unwrap();
        assert_eq!(u.created.len(), 4);
        p.update(0.0);
        assert!(p.frame.iter().all(|p| p.color[3] == 0.0));
        let positions: Vec<_> = p.frame.iter().map(|p| p.transform).collect();
        let mut f = frame();
        f.cast_position[0] += 1.0;
        assert!(s
            .update(f, &mut p, |_| panic!("retained marks must not re-query"))
            .unwrap()
            .created
            .is_empty());
        p.update(0.0);
        for (k, p) in p.frame.iter().enumerate() {
            assert_eq!(p.color[3], 0.05 * (k + 1) as f32);
            assert_eq!(p.transform, positions[k]);
            assert_eq!(p.args[2], 60.0);
        }
    }
    #[test]
    fn miss_disables_bucket_slot_until_deinitialize() {
        let mut s = spray();
        let mut p = Decals::new(100, 1);
        let mut queries = 0;
        s.update(frame(), &mut p, |_| {
            queries += 1;
            None
        });
        assert_eq!(queries, 4);
        s.update(frame(), &mut p, |_| panic!("missed slots must not retry"));
        s.clear();
        assert_eq!(s.update(frame(), &mut p, hit).unwrap().created.len(), 4);
    }
    #[test]
    fn reused_reference_is_invalidated_then_recreated_next_frame() {
        let mut s = spray();
        let mut p = Decals::new(100, 1);
        s.update(frame(), &mut p, hit);
        p.clear();
        assert!(s.update(frame(), &mut p, hit).unwrap().created.is_empty());
        assert_eq!(s.update(frame(), &mut p, hit).unwrap().created.len(), 4);
    }
    #[test]
    fn direction_keys_follow_acos_and_floor() {
        for (d, k) in [
            ([1.0, 0.0, 0.0], 0),
            ([0.0, 0.0, 1.0], 6),
            ([-1.0, 0.0, 0.0], 12),
            ([0.0, 0.0, -1.0], 18),
        ] {
            assert_eq!(direction_key(d), Some(k));
        }
        // A horizontal length below one is not normalized by the native Angle.
        assert_eq!(direction_key([0.0, 0.0, 0.0]), Some(6));
    }
    #[test]
    fn distance_gate_disables_slots_without_a_ground_query() {
        let mut s = spray();
        let mut p = Decals::new(100, 1);
        let mut f = frame();
        f.cast_position = [100.0, 1.0, 0.0];
        s.update(f, &mut p, |_| {
            panic!("distance rejection must precede query")
        });
        s.update(frame(), &mut p, |_| {
            panic!("distance-rejected slots remain disabled")
        });
        p.update(0.0);
        assert!(p.frame.is_empty());
    }
    #[test]
    fn cold_and_water_select_their_atlas_cells_and_alpha_caps() {
        for (element, cell) in [(E::COLD, COLD), (E::WATER, WATER)] {
            let mut s = SprayDecals::default();
            s.begin(Spell::default_for(element));
            let mut p = Decals::new(100, 1);
            s.update(frame(), &mut p, hit);
            let mut f = frame();
            f.dt = 10.0;
            s.update(f, &mut p, hit);
            p.update(0.0);
            assert!(p
                .frame
                .iter()
                .all(|p| p.args[0] + 8.0 * p.args[1] == cell as f32 && p.color[3] == 1.0));
        }
    }
}
