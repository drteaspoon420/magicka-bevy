//! HitList and SpraySpell.Execute; scene queries remain host operations.
use crate::{
    damage::{CastType, Damage},
    impact_motion::Vector,
    spells::Elements,
};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Default)]
pub struct HitList(BTreeMap<u16, f32>);
impl HitList {
    /// Original one-argument Add overwrites an existing cooldown.
    pub fn add(&mut self, handle: u16) {
        self.0.insert(handle, 0.25);
    }
    /// SortedList.Add(handle, ttl) behind a ContainsKey guard: an existing
    /// entry keeps its remaining time.
    pub fn add_for(&mut self, handle: u16, ttl: f32) {
        self.0.entry(handle).or_insert(ttl);
    }
    /// The `HitList[handle] = ttl` indexer: inserts or overwrites.
    pub fn set(&mut self, handle: u16, ttl: f32) {
        self.0.insert(handle, ttl);
    }
    pub fn contains(&self, handle: u16) -> bool {
        self.0.contains_key(&handle)
    }
    pub fn remaining(&self, handle: u16) -> Option<f32> {
        self.0.get(&handle).copied()
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn update(&mut self, dt: f32) {
        self.0.retain(|_, ttl| {
            *ttl -= dt;
            ttl.is_nan() || *ttl > 0.0
        });
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub origin: Vector,
    pub delta: Vector,
}
#[derive(Clone, Copy, Debug)]
pub struct SpraySource {
    pub owner_handle: u16,
    pub owner_position: Vector,
    pub owner_direction: Vector,
    pub cast_position: Vector,
    pub cast_direction: Vector,
    pub weapon_position: Vector,
    pub weapon_direction: Vector,
}
#[derive(Clone, Copy, Debug)]
pub struct SprayTarget {
    pub handle: u16,
    pub shield: bool,
    pub damageable: bool,
}
pub trait SprayWorld {
    fn scene_intersection(&mut self, segment: Segment) -> Option<f32>;
    fn shields(&mut self) -> Vec<u16>;
    fn shield_intersection(&mut self, handle: u16, segment: Segment) -> Option<f32>;
    /// Equivalent GetEntities(..., ignoreProtectedEntities:true), in original order.
    fn targets(&mut self, center: Vector, range: f32) -> Vec<SprayTarget>;
    fn arc_intersects(
        &mut self,
        target: u16,
        origin: Vector,
        direction: Vector,
        range: f32,
        angle: f32,
        height: f32,
    ) -> bool;
    fn damage(
        &mut self,
        target: u16,
        attacker: u16,
        damages: [Damage; 5],
        timestamp: f64,
        position: Vector,
    );
    fn electrocute(&mut self, target: u16, attacker: u16, multiplier: f32);
}
#[derive(Clone, Debug)]
pub struct SprayPulse {
    pub damages: [Damage; 5],
    pub timestamp: f64,
    pub angle: f32,
    pub hits: HitList,
}
impl SprayPulse {
    /// A single Execute call. Caller runs HitList.Update once per CastUpdate;
    /// repeated Execute calls in one update must not decrement cooldown again.
    pub fn execute(
        &mut self,
        range: f32,
        cast_type: CastType,
        from_staff: bool,
        source: SpraySource,
        world: &mut impl SprayWorld,
    ) {
        let (origin, direction) = if cast_type == CastType::Weapon {
            (source.weapon_position, source.weapon_direction)
        } else if from_staff {
            (source.cast_position, source.cast_direction)
        } else {
            (source.cast_position, source.owner_direction)
        };
        let mut segment = Segment {
            origin,
            delta: direction.map(|v| v * range),
        };
        let mut factor = range * 0.1;
        if let Some(fraction) = world.scene_intersection(segment) {
            factor *= fraction;
            segment.delta = segment.delta.map(|v| v * fraction);
        }
        for shield in world.shields() {
            if let Some(fraction) = world.shield_intersection(shield, segment) {
                factor *= fraction;
                segment.delta = segment.delta.map(|v| v * fraction);
            }
        }
        let water = self.damages.iter().fold(0, |a, d| a | d.element)
            & (Elements::WATER | Elements::STEAM).0
            == Elements::WATER.0;
        if water {
            world.electrocute(source.owner_handle, source.owner_handle, 0.25);
        }
        let mut targets = world.targets(source.owner_position, range);
        // List.Remove removes the first matching owner, not all duplicates.
        if let Some(i) = targets.iter().position(|t| t.handle == source.owner_handle) {
            targets.remove(i);
        }
        let mut damage = self.damages;
        for slot in &mut damage {
            slot.magnitude *= 0.25;
        }
        for target in targets {
            if !target.damageable || self.hits.contains(target.handle) {
                continue;
            }
            let hit = (target.shield
                && world.arc_intersects(target.handle, origin, direction, range, self.angle, 5.0))
                || world.arc_intersects(
                    target.handle,
                    origin,
                    direction,
                    range * factor,
                    self.angle,
                    5.0,
                );
            if hit {
                world.damage(
                    target.handle,
                    source.owner_handle,
                    damage,
                    self.timestamp,
                    origin,
                );
                self.hits.add(target.handle);
                if water {
                    world.electrocute(target.handle, source.owner_handle, 0.25);
                }
            }
        }
    }
}
