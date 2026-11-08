//! BossStatusEffected's damage and nine-slot status state. Unlike characters,
//! bosses cannot be pushed, knocked down, entangled or charmed.
use magicka_sim::{
    character_impact::result as R,
    damage::{attack, Damage},
    spells::Elements as E,
    status::{Status as S, StatusEffect, StatusOwner},
};

#[derive(Clone, Debug)]
pub struct BossHealth {
    pub hp: f32,
    pub max: f32,
    pub resistances: [magicka_sim::resistance::Resistance; 11],
    pub statuses: [StatusEffect; 9],
    pub mask: u16,
    pub dry_timer: f32,
}
impl BossHealth {
    pub fn new(max: f32) -> Self {
        Self {
            hp: max,
            max,
            resistances: [magicka_sim::resistance::Resistance::IDENTITY; 11],
            statuses: [StatusEffect::default(); 9],
            mask: 0,
            dry_timer: 0.,
        }
    }
    pub fn has(&self, status: S) -> bool {
        self.mask & status as u16 != 0
    }
    fn index(status: S) -> usize {
        (status as u16).trailing_zeros() as usize
    }
    fn clear(&mut self, status: S) {
        self.statuses[Self::index(status)] = StatusEffect::default();
    }
    fn active(&self, status: S) -> bool {
        !self.statuses[Self::index(status)].dead()
    }
    fn add_status(&mut self, mut incoming: StatusEffect) -> u16 {
        if incoming.dead() {
            return 0;
        }
        let mut removed = false;
        match incoming.kind {
            S::Burning => {
                if self.active(S::Cold)
                    || self.active(S::Wet)
                    || self.active(S::Frozen)
                    || self.dry_timer > 0.
                {
                    for status in [S::Cold, S::Wet, S::Frozen] {
                        self.clear(status);
                    }
                    removed = true;
                }
            }
            S::Cold => {
                if self.active(S::Burning) || self.dry_timer > 0. {
                    self.clear(S::Burning);
                    self.dry_timer = 0.9;
                    removed = true;
                }
                incoming.magnitude *= self.resistances[2].multiplier;
                if self.has(S::Wet) {
                    self.statuses[Self::index(S::Frozen)] =
                        StatusEffect::new(S::Frozen, 0., incoming.magnitude, false);
                    self.clear(S::Wet);
                }
                if self.has(S::Frozen) {
                    self.statuses[Self::index(S::Frozen)].magnitude += incoming.magnitude;
                    incoming.magnitude = 0.;
                }
            }
            S::Wet => {
                if self.has(S::Burning) || self.dry_timer > 0. {
                    self.clear(S::Burning);
                    removed = true;
                }
                if self.has(S::Greased) {
                    self.clear(S::Greased);
                }
            }
            S::Greased => {
                if self.has(S::Wet) {
                    self.clear(S::Wet);
                }
            }
            S::Poisoned => incoming.magnitude *= self.resistances[10].multiplier,
            S::Healing if self.has(S::Poisoned) => {
                self.clear(S::Poisoned);
            }
            _ => {}
        }
        if removed {
            return 0x40;
        }
        let index = Self::index(incoming.kind);
        self.statuses[index] = self.statuses[index]
            .merged(incoming)
            .expect("same status slot");
        0x20
    }
    pub fn scalar_damage(&mut self, mut amount: f32, element: u32) {
        if element & E::FIRE.0 != 0 && self.has(S::Greased) {
            amount *= 2.;
        }
        self.hp = (self.hp - amount).min(self.max);
    }
    /// Native cached mask is rebuilt from all nine slots after their ticks.
    pub fn update(&mut self, dt: f32, dead: bool) {
        self.dry_timer -= dt;
        if dead {
            self.statuses = [StatusEffect::default(); 9];
            self.mask = 0;
            return;
        }
        let mut mask = 0;
        for index in 0..9 {
            let owner = StatusOwner {
                is_character: false,
                in_cutscene: false,
                hit_points: self.hp,
                max_hit_points: self.max,
            };
            let status = &mut self.statuses[index];
            let damage = status.update(dt, owner).damage;
            if status.dead() {
                *status = StatusEffect::default();
            } else if status.kind != S::Wet || status.magnitude >= 1. {
                mask |= status.kind as u16;
            }
            if let Some((amount, element)) = damage {
                self.scalar_damage(amount as f32, element.0);
            }
        }
        self.mask = mask;
    }
    pub fn damage(&mut self, mut d: Damage) -> u16 {
        let mut sum = 0.;
        let mut count = 0.;
        for i in 0..11 {
            if d.element & (1 << i) != 0 {
                let r = self.resistances[i];
                d.amount = if d.element == E::EARTH.0 && r.modifier != 0. {
                    (d.amount + r.modifier).max(0.) as i32 as f32
                } else {
                    d.amount + r.modifier as i32 as f32
                };
                sum += r.multiplier;
                count += 1.;
            }
        }
        if count != 0. {
            d.magnitude *= sum / count;
        }
        if d.magnitude.abs() <= f32::from_bits(1) {
            return R::DEFLECTED;
        }
        let mut result = 0;
        if d.attack & attack::STATUS != 0 && sum.abs() > f32::from_bits(1) {
            for (element, kind, i) in [
                (E::FIRE, S::Burning, 3),
                (E::COLD, S::Cold, 2),
                (E::WATER, S::Wet, 1),
                (E::POISON, S::Poisoned, 10),
                (E::LIFE, S::Healing, 6),
                (E::STEAM, S::Wet, 9),
            ] {
                if d.element & element.0 != 0 && self.resistances[i].multiplier > f32::from_bits(1)
                {
                    result |=
                        self.add_status(StatusEffect::new(kind, d.amount, d.magnitude, false));
                }
            }
        }
        if d.attack & attack::DAMAGE != 0 {
            if d.element & E::LIGHTNING.0 != 0 && self.has(S::Wet) {
                d.amount *= 2.;
            }
            if d.element & E::LIFE.0 != 0 {
                self.statuses[Self::index(S::Poisoned)].magnitude -= d.magnitude;
            }
            if d.element & E::PHYSICAL_ELEMENTS.0 != 0 && self.has(S::Frozen) {
                d.amount = (d.amount - 200.).max(0.) * 3.;
                d.magnitude = d.magnitude.max(1.);
            }
            d.amount *= d.magnitude;
            self.hp -= d.amount;
            result |= R::HIT;
            if d.amount > 0. {
                result |= R::DAMAGED;
            } else if d.amount < 0. {
                result |= R::HEALED;
            } else {
                result |= R::DEFLECTED;
            }
            if d.attack & attack::PIERCING != 0 && d.magnitude > 0. && d.amount > 0. {
                result |= R::PIERCED;
            }
        }
        self.hp = self.hp.min(self.max);
        if d.amount == 0. {
            result |= R::DEFLECTED;
        }
        if self.hp <= 0. {
            result |= R::KILLED;
        }
        result
    }
}
