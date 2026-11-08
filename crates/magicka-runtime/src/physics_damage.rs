//! Numeric damage/status state of DamageablePhysicsEntity. Geometry, impulses,
//! gibs and presentation remain with the scene host.
use magicka_sim::{
    character_impact::result as R,
    damage::{attack as A, Damage},
    resistance::Resistance,
    spells::Elements as E,
    status::{Status as S, StatusEffect, StatusOwner},
};

#[derive(Clone, Debug)]
pub struct PhysicsDamage {
    pub health: f32,
    pub maximum: f32,
    pub mass: f32,
    pub solid: bool,
    /// Native melee queries use the template's first collision box.
    pub collision_box: Option<magicka_content::physics::CollisionBox>,
    pub resistances: Vec<Resistance>,
    pub statuses: [StatusEffect; 9],
    /// Rebuilt by UpdateStatusEffects, rather than eagerly by AddStatusEffect.
    pub status_mask: u16,
    pub on_death: i32,
    pub on_damage: i32,
    /// PhysicsEntity.mConditions (template), run on contacts.
    pub conditions: std::sync::Arc<Vec<magicka_content::common::EventCollection>>,
    /// PhysicsEntity.mHitList: 0.25 s per touching entity.
    pub hit_list: magicka_sim::spray_pulse::HitList,
    /// BossDamageZone: InternalDamage forwards to the parent boss's
    /// Damage(zone index, ...). Deliveries are queued here and applied by the
    /// boss host after the damaging phase (research/boss-battles.md).
    pub boss_zone: Option<u8>,
    pub boss_inbox: Vec<(Damage, u8)>,
    /// Death's Life hit flag/HP must be visible to subsequent callbacks now.
    pub death_boss: Option<std::sync::Arc<std::sync::Mutex<crate::boss_death::Death>>>,
    /// Set by the world each step (`boss_scale.rs`).
    pub death_damage_divisor: f32,
    pub death_commands: Vec<crate::boss_death::Command>,
    pub death_previous: Option<[f32; 3]>,
    pub boss_capsule: Option<BossCapsule>,
    /// An AnimatedPhysicsEntity with IsDamageable false: `Some(pushable)`.
    /// Collections and status effects are Deflected; a single damage passes
    /// only when pushed and the prop is pushable.
    pub deflects: Option<bool>,
    /// PhysAnimation controls (mAnimations) and their health tracking.
    pub animations: crate::phys_animation::PhysAnimations,
}

/// A native BossDamageZone capsule independent of a CharacterBody.
#[derive(Clone, Copy, Debug)]
pub struct BossCapsule {
    pub origin: [f32; 3],
    pub previous: [f32; 3],
    pub length: f32,
    pub radius: f32,
    pub ethereal: bool,
}

impl PhysicsDamage {
    /// The damage queued for this zone's boss (`boss_scale.rs`).
    pub fn take_boss_inbox(&mut self, divisor: f32) -> Vec<(Damage, u8)> {
        let mut inbox = std::mem::take(&mut self.boss_inbox);
        for (damage, _) in &mut inbox {
            *damage = crate::boss_scale::scaled(*damage, divisor);
        }
        inbox
    }

    pub fn has_capsule(&self) -> bool {
        self.death_boss.is_some() || self.boss_capsule.is_some()
    }
    pub fn capsule_ethereal(&self) -> bool {
        self.boss_capsule.is_some_and(|c| c.ethereal)
            || self
                .death_boss
                .as_ref()
                .is_some_and(|d| d.lock().unwrap().ethereal)
    }
    pub fn death_capsule(&self, handle: u16) -> Option<crate::missiles::MissileCallbackCapsule> {
        if let Some(c) = self.boss_capsule {
            return (!c.ethereal).then_some(crate::missiles::MissileCallbackCapsule {
                handle,
                old_axis: magicka_sim::spray_pulse::Segment {
                    origin: c.previous,
                    delta: [0., c.length, 0.],
                },
                new_axis: magicka_sim::spray_pulse::Segment {
                    origin: c.origin,
                    delta: [0., c.length, 0.],
                },
                radius: c.radius,
                old_body_position: c.previous,
                solid: false,
            });
        }
        let actor = self.death_boss.as_ref()?.lock().ok()?;
        if actor.ethereal {
            return None;
        }
        let old = self.death_previous.unwrap_or(actor.body_position);
        Some(crate::missiles::MissileCallbackCapsule {
            handle,
            old_axis: magicka_sim::spray_pulse::Segment {
                origin: old,
                delta: [0., 1.2, 0.],
            },
            new_axis: magicka_sim::spray_pulse::Segment {
                origin: actor.body_position,
                delta: [0., 1.2, 0.],
            },
            radius: 0.6,
            old_body_position: old,
            solid: false,
        })
    }
    /// BossDamageZone ignores the supplied segment radius.
    pub fn death_segment(&self, segment: magicka_sim::spray_pulse::Segment) -> Option<f32> {
        if let Some(c) = self.boss_capsule {
            return (!c.ethereal)
                .then(|| {
                    magicka_sim::capsule_intersection::segment_fraction(
                        segment,
                        c.origin,
                        [0., 1., 0.],
                        c.length,
                        c.radius,
                    )
                })
                .flatten();
        }
        let actor = self.death_boss.as_ref()?.lock().ok()?;
        if actor.ethereal {
            return None;
        }
        magicka_sim::capsule_intersection::segment_fraction(
            segment,
            actor.body_position,
            [0., 1., 0.],
            1.2,
            0.6,
        )
    }
    pub fn from_template(t: &magicka_content::physics::PhysicsEntityTemplate) -> Self {
        let mut resistances = vec![Resistance::IDENTITY; 11];
        for &(element, multiplier, modifier) in &t.resistances {
            let index = (element as u32).trailing_zeros() as usize;
            resistances[index] = Resistance {
                against: E(element as u32),
                multiplier,
                modifier,
                status_resistance: false,
            };
        }
        Self {
            health: t.max_hitpoints as f32,
            maximum: t.max_hitpoints as f32,
            mass: t.mass,
            solid: t.solid,
            collision_box: t.boxes.first().cloned(),
            resistances,
            statuses: [StatusEffect::default(); 9],
            status_mask: 0,
            on_death: 0,
            on_damage: 0,
            conditions: std::sync::Arc::new(t.conditions.0.clone()),
            hit_list: Default::default(),
            boss_zone: None,
            boss_inbox: Vec::new(),
            death_boss: None,
            death_damage_divisor: 1.0,
            death_commands: Vec::new(),
            death_previous: None,
            boss_capsule: None,
            deflects: None,
            animations: crate::phys_animation::PhysAnimations::new(t.max_hitpoints as f32),
        }
    }

    /// A BossDamageZone: an immovable solid box that never takes damage
    /// itself (the zone's health stays 1, so it is never dead).
    pub fn boss_zone(index: u8, collision_box: magicka_content::physics::CollisionBox) -> Self {
        Self {
            health: 1.0,
            maximum: 1.0,
            mass: f32::INFINITY,
            solid: true,
            collision_box: Some(collision_box),
            resistances: vec![Resistance::IDENTITY; 11],
            statuses: [StatusEffect::default(); 9],
            status_mask: 0,
            on_death: 0,
            on_damage: 0,
            conditions: Default::default(),
            hit_list: Default::default(),
            boss_zone: Some(index),
            boss_inbox: Vec::new(),
            death_boss: None,
            death_damage_divisor: 1.0,
            death_commands: Vec::new(),
            death_previous: None,
            boss_capsule: None,
            deflects: None,
            animations: crate::phys_animation::PhysAnimations::new(1.0),
        }
    }
    fn index(s: S) -> usize {
        (s as u16).trailing_zeros() as usize
    }

    pub fn radius(&self) -> f32 {
        if let Some(c) = self.boss_capsule {
            return c.radius;
        }
        if self.death_boss.is_some() {
            return 0.75;
        }
        self.collision_box.as_ref().map_or(0.0, |b| {
            b.sides.iter().map(|v| v * v).sum::<f32>().sqrt() * 0.5
        })
    }

    pub fn world_box(
        &self,
        transform: &[f32; 16],
    ) -> Option<magicka_sim::box_intersection::OrientedBox> {
        let b = self.collision_box.as_ref()?;
        Some(crate::physics_props::collision_box(b, transform))
    }

    /// InternalDamage(collection) visits all five slots in order. Each slot
    /// can raise death and damage callbacks, both with a null sender.
    pub fn deliver(
        &mut self,
        damages: &[Damage],
        features: u8,
        triggers: &mut Vec<(i32, Option<u16>)>,
    ) -> u16 {
        if let Some(actor) = &self.death_boss {
            let mut actor = actor.lock().expect("Death actor lock poisoned");
            let mut result = 0;
            for &damage in damages {
                // Death.Damage reads the amount alone, and never the magnitude.
                let damage = Damage {
                    amount: damage.amount / self.death_damage_divisor,
                    ..damage
                };
                let (r, commands) = actor.damage(damage, features);
                result |= r;
                self.death_commands.extend(commands);
            }
            return result;
        }
        if self.boss_capsule.is_some_and(|c| c.ethereal) {
            return 0;
        }
        if let Some(pushable) = self.deflects {
            let pushed = damages.len() == 1 && damages[0].attack & A::PUSHED == A::PUSHED;
            if !(pushed && pushable) {
                return R::DEFLECTED;
            }
        }
        if self.boss_zone.is_some() {
            // INFERRED: the boss's own DamageResult is computed when the queue
            // is applied; callers see a hit.
            self.boss_inbox.extend(
                damages
                    .iter()
                    .filter(|d| d.attack != 0)
                    .map(|&d| (d, features)),
            );
            return R::HIT;
        }
        let mut result = 0;
        let mut all = u16::MAX;
        for &damage in damages {
            let r = self.damage(damage, features);
            result |= r;
            all &= r;
            if r & R::KILLED != 0 && self.on_death != 0 {
                triggers.push((self.on_death, None));
            }
            if r & R::DAMAGED != 0 && self.on_damage != 0 {
                triggers.push((self.on_damage, None));
            }
        }
        if damages.len() == 5 {
            // Preserve InternalDamage(collection)'s recovered OR/AND mask,
            // including its handling of flags common to every slot.
            result & !(all ^ R::DEFLECTED)
        } else {
            result
        }
    }
    fn has(&self, s: S) -> bool {
        self.status_mask & s as u16 == s as u16
    }
    fn stop(&mut self, s: S) {
        self.statuses[Self::index(s)] = StatusEffect::default();
    }
    fn live(&self, s: S) -> bool {
        !self.statuses[Self::index(s)].dead()
    }
    fn resistance(&self, e: E) -> f32 {
        self.resistances[e.0.trailing_zeros() as usize].multiplier
    }

    pub fn add_status(&mut self, mut effect: StatusEffect) -> u16 {
        if effect.dead() {
            return 0;
        }
        let mut removed = false;
        match effect.kind {
            S::Burning => {
                if self.live(S::Cold) || self.live(S::Wet) || self.live(S::Frozen) {
                    self.stop(S::Cold);
                    self.stop(S::Wet);
                    let frozen = &mut self.statuses[Self::index(S::Frozen)];
                    frozen.magnitude -= effect.magnitude;
                    if frozen.magnitude <= 0. {
                        self.stop(S::Frozen);
                    }
                    removed = true;
                }
            }
            S::Cold => {
                if self.live(S::Burning) {
                    self.stop(S::Burning);
                    removed = true;
                }
                effect.magnitude *= self.resistance(E::COLD) * (0.5 + 0.5 * (500. / self.mass));
                if self.has(S::Wet) {
                    self.statuses[Self::index(S::Frozen)] =
                        StatusEffect::new(S::Frozen, 0., 0.5, false);
                    self.stop(S::Wet);
                }
                if self.has(S::Frozen) {
                    self.statuses[Self::index(S::Frozen)].magnitude += effect.magnitude;
                    effect.magnitude = 0.;
                }
            }
            S::Wet => {
                if self.live(S::Burning) {
                    self.stop(S::Burning);
                    removed = true;
                }
            }
            S::Poisoned => effect.magnitude *= self.resistance(E::POISON),
            _ => {}
        }
        if removed {
            return 0x40;
        }
        let index = Self::index(effect.kind);
        self.statuses[index] = self.statuses[index]
            .merged(effect)
            .expect("same status slot");
        0x20
    }

    /// Health/status portion of InternalDamage; feature bit 1 controls health
    /// mutation but does not suppress Damaged/Healed results or callbacks.
    pub fn damage(&mut self, mut d: Damage, features: u8) -> u16 {
        if self.health <= 0. {
            return 0;
        }
        let mut result = 0;
        d.apply_resistances_inclusive(&self.resistances, None, &[])
            .expect("no additional resistance");
        if d.attack & A::STATUS == A::STATUS {
            for (element, status) in [
                (E::FIRE, S::Burning),
                (E::COLD, S::Cold),
                (E::WATER, S::Wet),
                (E::POISON, S::Poisoned),
            ] {
                if d.element & element.0 == element.0 {
                    result |=
                        self.add_status(StatusEffect::new(status, d.amount, d.magnitude, false));
                }
            }
        }
        if d.attack & A::DAMAGE == A::DAMAGE {
            if d.element == E::LIGHTNING.0 && self.has(S::Wet) {
                d.amount *= 3.;
            }
            if d.element & E::LIFE.0 == E::LIFE.0 {
                let poison = &mut self.statuses[Self::index(S::Poisoned)];
                poison.magnitude -= d.magnitude;
                if poison.magnitude <= 0. {
                    poison.stop();
                }
            }
            if d.element & (E::EARTH.0 | E::ICE.0) != 0 && self.has(S::Frozen) {
                d.amount = (d.amount - 200.).max(0.);
                d.magnitude = d.magnitude.max(1.);
                d.amount *= 3.;
            }
            d.amount = (d.amount * d.magnitude) as i32 as f32;
            if features & 1 != 0 {
                self.health -= d.amount;
            }
            if d.attack & A::PIERCING != 0 && d.magnitude > 0. && d.amount > 0. {
                result |= R::PIERCED;
            }
            if d.amount > 0. {
                result |= R::DAMAGED;
            }
            if d.amount == 0. {
                result |= R::DEFLECTED;
            }
            if d.amount < 0. {
                result |= R::HEALED;
            }
        }
        if self.health > self.maximum {
            self.health = self.maximum;
        }
        if d.amount == 0. {
            result |= R::DEFLECTED;
        }
        if self.health <= 0. {
            result |= R::KILLED;
        }
        result
    }

    /// Direct status damage bypasses resistances and the OnDamage callback.
    /// Returns the number of SpawnGibs calls for the host's death callbacks.
    pub fn update(&mut self, dt: f32) -> usize {
        if self.health <= 0. {
            self.statuses = [StatusEffect::default(); 9];
            self.status_mask = 0;
            return 0;
        }
        let mut deaths = 0;
        self.status_mask = 0;
        for effect in &mut self.statuses {
            let tick = effect.update(
                dt,
                StatusOwner {
                    is_character: false,
                    in_cutscene: false,
                    hit_points: self.health,
                    max_hit_points: self.maximum,
                },
            );
            if let Some((amount, _)) = tick.damage {
                self.health -= amount as f32;
                if self.health <= 0. {
                    deaths += 1;
                }
            }
            if effect.dead() {
                *effect = StatusEffect::default();
            } else {
                self.status_mask |= effect.kind as u16;
            }
        }
        deaths
    }
}
