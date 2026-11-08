//! SpraySpell.CastArea's immediate numeric dispatch plan; see spray-area.md.
use crate::{
    damage::{calculate_damage, CastType, Damage, DamageError},
    spell_cast::{EffectKind, EffectLifecycle, EffectOwner},
    spells::{Elements, Spell, SpellType},
};

/// Private-timer portion of the native shared SpraySpell cache. This is
/// shared by Force, Self, Weapon, and Area; it is not an Area-only cache.
#[derive(Clone, Debug)]
pub struct SprayTimerCache {
    free: Vec<f32>,
}
impl Default for SprayTimerCache {
    fn default() -> Self {
        Self {
            free: vec![0.0; 16],
        }
    }
}
impl SprayTimerCache {
    /// Native GetFromCache removes the last entry and does not grow the pool.
    /// None exposes exhaustion rather than silently inventing a fresh entry.
    pub fn acquire(&mut self) -> Option<f32> {
        self.free.pop()
    }
    /// ReturnToCache appends without clearing private TTL. Call once per
    /// acquired effect, with that effect's final private TTL.
    pub fn release(&mut self, private_ttl: f32) {
        self.free.push(private_ttl);
    }
}

/// Area reuses the pool entry's private TTL without assigning it. Fresh
/// entries supply zero; hosts retaining pooled effects supply their old TTL.
#[derive(Clone, Debug)]
pub struct SprayArea {
    pub lifecycle: EffectLifecycle,
    pub private_ttl: f32,
}
impl SprayArea {
    pub fn begin(from_staff: bool, timestamp: f64, pooled_private_ttl: f32) -> Self {
        let mut lifecycle = EffectLifecycle::begin(CastType::Area, from_staff, timestamp).unwrap();
        lifecycle.max_ttl = lifecycle.min_ttl;
        Self {
            lifecycle,
            private_ttl: pooled_private_ttl,
        }
    }

    pub fn update(
        &mut self,
        dt: f32,
        owner: EffectOwner,
        mut deinitialize: impl FnMut(&mut EffectLifecycle),
    ) -> (bool, f32) {
        if self.private_ttl <= 0.0 || self.lifecycle.max_ttl <= 0.0 {
            deinitialize(&mut self.lifecycle);
            return (false, 0.6);
        }
        let (keep, _) = self
            .lifecycle
            .update(dt, EffectKind::Spray, owner, deinitialize);
        (keep, 0.6)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SprayAreaPlan {
    pub radius: f32,
    pub damages: [Damage; 5],
    /// Electrocute(owner, 1) is requested only for Water without Steam.
    pub electrocute_owner: bool,
    /// Native checks A–D, not E, and chooses the first Fire-bearing slot.
    pub greased_owner_damage: Option<Damage>,
}

/// Host supplies the owner position, caster exclusion, timestamp, and cast
/// source position. Callbacks must execute synchronously in native order.
pub trait SprayAreaHost {
    fn full_blast(&mut self, radius: f32, damages: [Damage; 5]);
    fn electrocute_owner(&mut self, multiplier: f32);
    fn damage_greased_owner(&mut self, damage: Damage);
}

impl SprayAreaPlan {
    pub fn dispatch(&self, host: &mut impl SprayAreaHost) {
        host.full_blast(self.radius, self.damages);
        if self.electrocute_owner {
            host.electrocute_owner(1.0);
        }
        if let Some(damage) = self.greased_owner_damage {
            host.damage_greased_owner(damage);
        }
    }
}

pub fn greased_damage(damages: &[Damage; 5], greased: bool) -> Option<Damage> {
    greased
        .then(|| {
            damages[..4]
                .iter()
                .find(|d| d.element & Elements::FIRE.0 != 0)
                .copied()
        })
        .flatten()
}

pub fn plan(spell: &Spell, greased: bool) -> Result<SprayAreaPlan, DamageError> {
    let mut damages = [Damage::default(); 5];
    for (slot, damage) in
        damages
            .iter_mut()
            .zip(calculate_damage(spell, SpellType::Spray, CastType::Area)?)
    {
        *slot = damage;
    }
    let elements = damages.iter().fold(0, |all, d| all | d.element);
    Ok(SprayAreaPlan {
        radius: spell.blast_size() * 10.0,
        electrocute_owner: elements & (Elements::WATER.0 | Elements::STEAM.0) == Elements::WATER.0,
        greased_owner_damage: greased_damage(&damages, greased),
        damages,
    })
}
