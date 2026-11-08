//! PushSpell Force/Area expanding pulse. See research/push-spell.md.
use crate::{
    damage::{attack, CastType, Damage},
    impact_motion::Vector,
    spell_cast::EffectLifecycle,
    spells::Elements,
    spray_pulse::Segment,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug)]
pub struct PushTarget {
    pub handle: u16,
    pub position: Vector,
    pub dead: bool,
    pub damageable: bool,
    pub missile: bool,
}
pub trait PushWorld {
    fn push_targets(&mut self, center: Vector, range: f32) -> Vec<PushTarget>;
    fn push_arc(
        &mut self,
        target: u16,
        origin: Vector,
        direction: Vector,
        range: f32,
        angle: f32,
        height: f32,
    ) -> bool;
    fn push_obstructed(&mut self, segment: Segment) -> bool;
    fn push_damage(
        &mut self,
        target: u16,
        attacker: u16,
        damage: Damage,
        timestamp: f64,
        position: Vector,
    );
    fn push_impulse(
        &mut self,
        target: u16,
        direction: Vector,
        elevation: f32,
        force: f32,
        distance: f32,
    );
}
#[derive(Clone, Debug)]
pub struct PushSpell {
    pub lifecycle: EffectLifecycle,
    pub range: f32,
    pub time: f32,
    pub angle: f32,
    pub elevation: f32,
    pub force: f32,
    pub damage: Damage,
    pub initial_direction: Vector,
    pub hits: BTreeSet<u16>,
}
impl PushSpell {
    pub fn cast(
        cast_type: CastType,
        from_staff: bool,
        timestamp: f64,
        power: &mut f32,
        additional_radius: f32,
        direction: Vector,
    ) -> Option<Self> {
        if !matches!(cast_type, CastType::Force | CastType::Area) {
            return None;
        }
        let mut lifecycle = EffectLifecycle::begin(cast_type, from_staff, timestamp)?;
        lifecycle.min_ttl = 0.4;
        let area = cast_type == CastType::Area;
        let force = 50. + 500. * *power;
        let elevation = *power * 0.1;
        *power = 0.;
        Some(Self {
            lifecycle,
            range: if area { 3.5 + additional_radius } else { 10. },
            time: 0.25,
            angle: if area {
                std::f32::consts::PI
            } else {
                std::f32::consts::PI / 6.
            },
            elevation,
            force,
            damage: Damage {
                attack: attack::PUSHED,
                element: Elements::EARTH.0,
                amount: force,
                magnitude: 1.5,
            },
            initial_direction: direction,
            hits: BTreeSet::new(),
        })
    }
    /// PushSpell.CastWeapon: base CastWeapon then mMinTTL = 0.4; mTime,
    /// mRange, mAngle and mDamage are not assigned. A fresh pooled effect has
    /// mTime = 0 (a finished one ≤ 0), so CastUpdate never pushes and the
    /// effect ends once mMinTTL < 0. Reuse of an effect stopped while its
    /// mTime was still positive is not modelled.
    pub fn cast_weapon(from_staff: bool, timestamp: f64, direction: Vector) -> Self {
        let mut lifecycle = EffectLifecycle::begin(CastType::Weapon, from_staff, timestamp)
            .expect("weapon lifecycle");
        lifecycle.min_ttl = 0.4;
        Self {
            lifecycle,
            range: 0.,
            time: 0.,
            angle: 0.,
            elevation: 0.,
            force: 0.,
            damage: Damage::default(),
            initial_direction: direction,
            hits: BTreeSet::new(),
        }
    }
    /// Owner position stays live; direction is captured at cast time. This
    /// derived update does not call the base lifecycle or inspect button release.
    pub fn update(
        &mut self,
        dt: f32,
        owner: u16,
        position: Vector,
        world: &mut impl PushWorld,
    ) -> (bool, Vec<u16>) {
        self.time -= dt;
        self.lifecycle.min_ttl -= dt;
        let mut applied = vec![];
        if self.time > 0. {
            let range =
                (1. - self.time.clamp(0., 0.25) / 0.25) * self.range * 0.75 + self.range * 0.25;
            for target in world.push_targets(position, range) {
                if target.handle == owner
                    || self.hits.contains(&target.handle)
                    || target.dead
                    || (self.angle != std::f32::consts::PI
                        && !world.push_arc(
                            target.handle,
                            position,
                            self.initial_direction,
                            range,
                            self.angle,
                            4.,
                        ))
                {
                    continue;
                }
                let delta = std::array::from_fn(|i| target.position[i] - position[i]);
                if world.push_obstructed(Segment {
                    origin: position,
                    delta,
                }) {
                    continue;
                }
                let length =
                    (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
                if length > f32::from_bits(1) {
                    self.hits.insert(target.handle);
                    applied.push(target.handle);
                    if target.damageable && !target.missile {
                        world.push_damage(
                            target.handle,
                            owner,
                            self.damage,
                            self.lifecycle.time_stamp,
                            position,
                        );
                    } else {
                        // Native length includes Y, then Y is zeroed before division.
                        let direction = [delta[0] / length, 0., delta[2] / length];
                        world.push_impulse(
                            target.handle,
                            direction,
                            0.17453292 + self.elevation * std::f32::consts::FRAC_PI_4 * 0.5,
                            self.force,
                            1.5,
                        );
                    }
                }
            }
        } else if self.lifecycle.min_ttl < 0. {
            self.lifecycle.active = false;
            self.hits.clear();
            return (false, applied);
        }
        (true, applied)
    }
}
