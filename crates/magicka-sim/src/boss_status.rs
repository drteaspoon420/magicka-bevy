//! `Magicka.GameLogic.Entities.Bosses.BossStatusEffected`: a named boss's
//! hit points, resistances, status effects and floating damage numbers.
//! Presentation (blood effects, numbers) is returned as events. See
//! output/research/boss-battles.md.
use crate::character_impact::{feature, result};
use crate::damage::{attack, Damage};
use crate::resistance::Resistance;
use crate::spells::Elements as E;
use crate::status::{Status, StatusEffect, StatusOwner};

/// StatusEffect.StatusIndex.
pub fn status_index(kind: Status) -> usize {
    (kind as u16).trailing_zeros() as usize
}

const STATUS_ADDED: u16 = 0x20;
const STATUS_REMOVED: u16 = 0x40;

/// Presentation requested by the boss damage code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BossEvent {
    /// EffectManager.StartEffect(BloodEffect) at the attack position,
    /// direction Vector3.Right.
    Blood { position: [f32; 3] },
    /// FloatingTextManager.AddNumber(amount, NotifierTextPostion, 0.4,
    /// locked) for a new locked number.
    LockedNumber { amount: f32 },
    /// FloatingTextManager.AddToNumber on the current locked number.
    AddToNumber { amount: f32 },
    /// ReleaseNumber of the current locked number.
    ReleaseNumber,
    /// UpdateNumberPosition of the current locked number.
    MoveNumber,
    /// An accumulated status/healing total, AddNumber(.., 0.7, unlocked).
    Number { amount: f32 },
}

/// Which class's Damage/UpdateStatusEffects overrides apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BossRules {
    /// BossStatusEffected's own rules.
    #[default]
    Standard,
    /// Cthulhu.Damage/UpdateStatusEffects: Fire only burns a wet or cold
    /// Cthulhu; Cold, Water and Steam (as Wet) are the other statuses; no
    /// Life cure; the last-damage time resets only with Notify; Wet never
    /// updates (it stays until cleared).
    Cthulhu,
}

#[derive(Clone, Debug)]
pub struct BossStatus {
    pub rules: BossRules,
    pub hit_points: f32,
    pub max_hit_points: f32,
    pub statuses: [StatusEffect; 9],
    /// mCurrentStatusEffects (rebuilt by UpdateStatusEffects).
    pub current: u16,
    pub resistances: [Resistance; 11],
    fire_accumulation: f32,
    fire_timer: f32,
    poison_accumulation: f32,
    poison_timer: f32,
    healing_accumulation: f32,
    healing_timer: f32,
    /// mLastDamageIndex >= 0.
    pub locked_number: bool,
    pub time_since_last_damage: f32,
    pub time_since_last_status_damage: f32,
    dry_timer: f32,
}

impl Default for BossStatus {
    fn default() -> Self {
        Self {
            rules: BossRules::Standard,
            hit_points: 0.0,
            max_hit_points: 0.0,
            statuses: [StatusEffect::default(); 9],
            current: 0,
            resistances: std::array::from_fn(|i| Resistance {
                against: E::BY_INDEX[i],
                multiplier: 1.0,
                modifier: 0.0,
                status_resistance: false,
            }),
            fire_accumulation: 0.0,
            fire_timer: 0.25,
            poison_accumulation: 0.0,
            poison_timer: 0.25,
            healing_accumulation: 0.0,
            healing_timer: 0.25,
            locked_number: false,
            time_since_last_damage: 0.0,
            time_since_last_status_damage: 0.0,
            dry_timer: 0.0,
        }
    }
}

fn slot(statuses: &mut [StatusEffect; 9], kind: Status) -> &mut StatusEffect {
    &mut statuses[status_index(kind)]
}

impl BossStatus {
    pub fn has(&self, kind: Status) -> bool {
        self.current & kind as u16 == kind as u16
    }
    pub fn magnitude(&self, kind: Status) -> f32 {
        self.statuses[status_index(kind)].magnitude
    }
    fn clear(&mut self, kind: Status) {
        *slot(&mut self.statuses, kind) = StatusEffect::default();
    }

    /// BossStatusEffected.UpdateDamage: locked-number lifetime and the
    /// once-per-second status/healing totals.
    pub fn update_damage(&mut self, dt: f32, dead: bool, events: &mut Vec<BossEvent>) {
        self.time_since_last_damage += dt;
        self.time_since_last_status_damage += dt;
        if self.locked_number {
            if self.time_since_last_damage > 0.333 || dead {
                events.push(BossEvent::ReleaseNumber);
                self.locked_number = false;
            } else {
                events.push(BossEvent::MoveNumber);
            }
        }
        for (accumulation, timer) in [
            (&mut self.healing_accumulation, &mut self.healing_timer),
            (&mut self.fire_accumulation, &mut self.fire_timer),
            (&mut self.poison_accumulation, &mut self.poison_timer),
        ] {
            *timer -= dt;
            if *timer <= 0.0 {
                *timer += 1.0;
                if *accumulation != 0.0 {
                    events.push(BossEvent::Number {
                        amount: *accumulation,
                    });
                    *accumulation = 0.0;
                    self.time_since_last_status_damage = 0.0;
                }
            }
        }
    }

    /// BossStatusEffected.UpdateStatusEffects. Each status tick's direct
    /// damage is returned for the owning zone (`Entity.Damage(amount,
    /// element)`, which each boss routes through its own rules).
    pub fn update_statuses(&mut self, dt: f32, dead: bool) -> Vec<(f32, E)> {
        self.dry_timer -= dt;
        let mut mask = 0u16;
        let mut ticks = Vec::new();
        if dead {
            self.statuses = [StatusEffect::default(); 9];
        } else {
            let owner = StatusOwner {
                is_character: false,
                in_cutscene: false,
                hit_points: self.hit_points,
                max_hit_points: self.max_hit_points,
            };
            let frozen_wet = self.rules == BossRules::Cthulhu;
            for s in &mut self.statuses {
                if frozen_wet && s.kind == Status::Wet {
                    // Cthulhu: Wet only stops its effect.
                    if s.magnitude >= 1.0 {
                        mask |= s.kind as u16;
                    }
                    continue;
                }
                let tick = s.update(dt, owner);
                if let Some((amount, element)) = tick.damage {
                    ticks.push((amount as f32, element));
                }
                if s.dead() {
                    *s = StatusEffect::default();
                } else if s.kind == Status::Wet {
                    if s.magnitude >= 1.0 {
                        mask |= s.kind as u16;
                    }
                } else {
                    mask |= s.kind as u16;
                }
            }
        }
        self.current = mask;
        ticks
    }

    /// BossStatusEffected.AddStatusEffect.
    pub fn add_status(&mut self, mut effect: StatusEffect) -> u16 {
        if effect.dead() {
            return 0;
        }
        let mut removed = false;
        let alive = |s: &Self, k: Status| !s.statuses[status_index(k)].dead();
        match effect.kind {
            Status::Burning => {
                if alive(self, Status::Cold)
                    || alive(self, Status::Wet)
                    || alive(self, Status::Frozen)
                    || self.dry_timer > 0.0
                {
                    self.clear(Status::Cold);
                    self.clear(Status::Wet);
                    self.clear(Status::Frozen);
                    removed = true;
                }
            }
            Status::Cold => {
                let mut magnitude = effect.magnitude;
                if alive(self, Status::Burning) || self.dry_timer > 0.0 {
                    self.clear(Status::Burning);
                    self.dry_timer = 0.9;
                    removed = true;
                }
                magnitude *= self.resistances[status_index_element(E::COLD)].multiplier;
                if self.has(Status::Wet) {
                    *slot(&mut self.statuses, Status::Frozen) =
                        StatusEffect::new(Status::Frozen, 0.0, magnitude, false);
                    self.clear(Status::Wet);
                }
                if self.has(Status::Frozen) {
                    slot(&mut self.statuses, Status::Frozen).magnitude += magnitude;
                    magnitude = 0.0;
                }
                effect.magnitude = magnitude;
            }
            Status::Wet => {
                if self.has(Status::Burning) || self.dry_timer > 0.0 {
                    self.clear(Status::Burning);
                    removed = true;
                }
                if self.has(Status::Greased) {
                    self.clear(Status::Greased);
                }
            }
            Status::Greased => {
                if self.has(Status::Wet) {
                    self.clear(Status::Wet);
                }
            }
            Status::Poisoned => {
                effect.magnitude *= self.resistances[status_index_element(E::POISON)].multiplier;
            }
            Status::Healing if self.has(Status::Poisoned) => {
                self.clear(Status::Poisoned);
            }
            _ => {}
        }
        if removed {
            return STATUS_REMOVED;
        }
        let s = slot(&mut self.statuses, effect.kind);
        // operator+ (a dead or None slot takes the new effect).
        *s = s.merged(effect).unwrap_or(effect);
        STATUS_ADDED
    }

    /// The protected Damage(float, Elements): a direct status tick.
    pub fn status_damage(&mut self, mut amount: f32, element: E) {
        if element.has(E::FIRE) && self.has(Status::Greased) {
            amount *= 2.0;
        }
        self.hit_points -= amount;
        if element == E::FIRE {
            self.fire_accumulation += amount;
        } else if element == E::POISON {
            self.poison_accumulation += amount;
        } else if element == E::LIFE && self.hit_points < self.max_hit_points {
            self.healing_accumulation += amount;
        }
        if self.hit_points > self.max_hit_points {
            self.hit_points = self.max_hit_points;
        }
    }

    /// BossStatusEffected.Damage(Damage, attacker, position, features).
    pub fn damage(
        &mut self,
        mut d: Damage,
        dead: bool,
        attack_position: [f32; 3],
        features: u8,
        events: &mut Vec<BossEvent>,
    ) -> u16 {
        if dead {
            return result::DEFLECTED;
        }
        let element = E(d.element);
        let mut out = 0;
        let (mut multipliers, mut count) = (0.0f32, 0.0f32);
        for (i, r) in self.resistances.iter().enumerate() {
            let e = E::BY_INDEX[i];
            if element.has(e) {
                if element == E::EARTH && r.modifier != 0.0 {
                    d.amount = ((d.amount + r.modifier).max(0.0)) as i32 as f32;
                } else {
                    d.amount += r.modifier as i32 as f32;
                }
                multipliers += r.multiplier;
                count += 1.0;
            }
        }
        if count != 0.0 {
            d.magnitude *= multipliers / count;
        }
        if d.magnitude.abs() <= f32::from_bits(1) {
            return result::DEFLECTED;
        }
        let resist = |s: &Self, e: E| s.resistances[status_index_element(e)].multiplier;
        let cthulhu = self.rules == BossRules::Cthulhu;
        if cthulhu && d.attack & attack::STATUS != 0 && multipliers.abs() > f32::from_bits(1) {
            let eps = f32::from_bits(1);
            if element.has(E::FIRE)
                && resist(self, E::FIRE) > eps
                && (self.has(Status::Wet) || self.has(Status::Cold))
            {
                out |= self.add_status(StatusEffect::new(
                    Status::Burning,
                    d.amount,
                    d.magnitude,
                    false,
                ));
            }
            for (e, kind) in [
                (E::COLD, Status::Cold),
                (E::WATER, Status::Wet),
                (E::STEAM, Status::Wet),
            ] {
                if element.has(e) && resist(self, e) > eps {
                    out |= self.add_status(StatusEffect::new(kind, d.amount, d.magnitude, false));
                }
            }
        } else if d.attack & attack::STATUS != 0 && multipliers.abs() > f32::from_bits(1) {
            for (e, kind) in [
                (E::FIRE, Status::Burning),
                (E::COLD, Status::Cold),
                (E::WATER, Status::Wet),
                (E::POISON, Status::Poisoned),
                (E::LIFE, Status::Healing),
                (E::STEAM, Status::Wet),
            ] {
                if element.has(e) && resist(self, e) > f32::from_bits(1) {
                    out |= self.add_status(StatusEffect::new(kind, d.amount, d.magnitude, false));
                }
            }
        }
        if d.attack & attack::DAMAGE != 0 {
            if element.has(E::LIGHTNING) && self.has(Status::Wet) {
                d.amount *= 2.0;
            }
            if element.has(E::LIFE) && !cthulhu {
                let poison = slot(&mut self.statuses, Status::Poisoned);
                poison.magnitude -= d.magnitude;
                if poison.magnitude <= 0.0 {
                    poison.stop();
                }
            }
            if element.intersects(E::PHYSICAL_ELEMENTS) {
                if self.has(Status::Frozen) {
                    d.amount = (d.amount - 200.0).max(0.0);
                    d.magnitude = d.magnitude.max(1.0);
                    d.amount *= 3.0;
                } else {
                    events.push(BossEvent::Blood {
                        position: attack_position,
                    });
                }
            }
            d.amount *= d.magnitude;
            self.hit_points -= d.amount;
            if d.attack & attack::PIERCING != 0 && d.magnitude > 0.0 && d.amount > 0.0 {
                out |= result::PIERCED;
            }
            if d.amount > 0.0 {
                out |= result::DAMAGED;
            }
            if d.amount == 0.0 {
                out |= result::DEFLECTED;
            }
            if d.amount < 0.0 {
                out |= result::HEALED;
            }
            out |= result::HIT;
            if d.amount != 0.0 && !(cthulhu && features & feature::NOTIFY == 0) {
                self.time_since_last_damage = 0.0;
            }
            if features & feature::NOTIFY != 0 {
                if self.locked_number {
                    events.push(BossEvent::AddToNumber { amount: d.amount });
                } else {
                    events.push(BossEvent::LockedNumber { amount: d.amount });
                    self.locked_number = true;
                }
            }
        }
        if self.hit_points > self.max_hit_points {
            self.hit_points = self.max_hit_points;
        }
        if d.amount == 0.0 {
            out |= result::DEFLECTED;
        }
        if self.hit_points <= 0.0 {
            out |= result::KILLED;
        }
        out
    }
}

/// Spell.ElementIndex / Defines.ElementIndex.
fn status_index_element(e: E) -> usize {
    e.0.trailing_zeros() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> BossStatus {
        let mut b = BossStatus {
            hit_points: 5000.0,
            max_hit_points: 5000.0,
            ..Default::default()
        };
        b.resistances[0].modifier = -200.0;
        b.resistances[0].multiplier = 0.75;
        for e in [E::ARCANE, E::LIGHTNING, E::FIRE] {
            b.resistances[status_index_element(e)].multiplier = 0.5;
        }
        b
    }
    fn hit(element: E, amount: f32) -> Damage {
        Damage {
            attack: attack::DAMAGE,
            element: element.0,
            amount,
            magnitude: 1.0,
        }
    }

    #[test]
    fn earth_modifier_clamps_then_multiplies() {
        let mut b = machine();
        let mut ev = vec![];
        let r = b.damage(hit(E::EARTH, 500.0), false, [0.0; 3], 0, &mut ev);
        assert_eq!(b.hit_points, 5000.0 - 300.0 * 0.75);
        assert!(r & result::DAMAGED != 0 && r & result::HIT != 0);
        assert_eq!(ev, vec![BossEvent::Blood { position: [0.0; 3] }]);
        // Below the modifier: clamped to zero, deflected.
        let r = b.damage(hit(E::EARTH, 100.0), false, [0.0; 3], 0, &mut ev);
        assert!(r & result::DEFLECTED != 0);
    }

    #[test]
    fn mixed_elements_average_multipliers_and_truncate_modifiers() {
        let mut b = machine();
        b.resistances[1].modifier = -10.7;
        let mut ev = vec![];
        // Water (x1, -10) + Fire (x0.5): magnitude 0.75, amount 100 - 10.
        b.damage(
            hit(E(E::WATER.0 | E::FIRE.0), 100.0),
            false,
            [0.0; 3],
            0,
            &mut ev,
        );
        assert_eq!(b.hit_points, 5000.0 - 90.0 * 0.75);
    }

    #[test]
    fn frozen_physical_hits_lose_200_then_triple() {
        let mut b = machine();
        b.add_status(StatusEffect::new(Status::Wet, 0.0, 1.0, false));
        b.update_statuses(0.0, false);
        b.add_status(StatusEffect::new(Status::Cold, 0.0, 1.0, false));
        b.update_statuses(0.0, false);
        assert!(b.has(Status::Frozen));
        let mut ev = vec![];
        b.damage(hit(E::ICE, 300.0), false, [0.0; 3], 0, &mut ev);
        assert_eq!(b.hit_points, 5000.0 - 300.0);
        assert!(ev.is_empty(), "frozen hits draw no blood");
    }

    #[test]
    fn burning_ticks_and_status_numbers_once_per_second() {
        let mut b = machine();
        let mut ev = vec![];
        b.damage(
            Damage {
                attack: attack::STATUS,
                element: E::FIRE.0,
                amount: 60.0,
                magnitude: 1.0,
            },
            false,
            [0.0; 3],
            0,
            &mut ev,
        );
        let mut total = 0.0;
        for _ in 0..60 {
            for (amount, element) in b.update_statuses(1.0 / 60.0, false) {
                assert_eq!(element, E::FIRE);
                b.status_damage(amount, element);
                total += amount;
            }
            b.update_damage(1.0 / 60.0, false, &mut ev);
        }
        assert!(b.has(Status::Burning));
        assert!(total > 0.0);
        assert_eq!(b.hit_points, 5000.0 - total);
        assert!(ev.iter().any(|e| matches!(e, BossEvent::Number { .. })));
    }

    #[test]
    fn locked_number_accumulates_then_releases() {
        let mut b = machine();
        let mut ev = vec![];
        b.damage(
            hit(E::WATER, 10.0),
            false,
            [0.0; 3],
            feature::NOTIFY,
            &mut ev,
        );
        b.damage(
            hit(E::WATER, 5.0),
            false,
            [0.0; 3],
            feature::NOTIFY,
            &mut ev,
        );
        assert_eq!(
            ev,
            vec![
                BossEvent::LockedNumber { amount: 10.0 },
                BossEvent::AddToNumber { amount: 5.0 }
            ]
        );
        ev.clear();
        b.update_damage(0.4, false, &mut ev);
        assert_eq!(ev[0], BossEvent::ReleaseNumber);
    }
}
