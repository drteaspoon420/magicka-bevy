//! RailGunSpell's Self branch (Life healing/Arcane self damage).
//! See research/live-self-cast.md; Force/Area/Weapon branches are separate.
use crate::{
    damage::{calculate_damage, CastType, Damage, DamageError, SpellType},
    self_spray::SELF_EFFECTS,
    spell_cast::{EffectKind, EffectLifecycle, EffectOwner},
    spells::{Elements, Spell},
};

pub trait SelfBeamHost {
    fn start_effect(&mut self, index: usize, name: &'static str);
    fn damage_self(&mut self, damages: [Damage; 5], timestamp: f64);
    fn update_effects(&mut self);
    fn deinitialize(&mut self, lifecycle: &mut EffectLifecycle);
}
#[derive(Clone, Debug)]
pub struct SelfBeam {
    pub lifecycle: EffectLifecycle,
    pub spell: Spell,
    pub damages: [Damage; 5],
    pub ttl: f32,
    pub has_cast: bool,
}
impl SelfBeam {
    pub fn cast(
        spell: Spell,
        from_staff: bool,
        timestamp: f64,
        host: &mut impl SelfBeamHost,
    ) -> Result<Self, DamageError> {
        let lifecycle = EffectLifecycle::begin(CastType::SelfCast, from_staff, timestamp).unwrap();
        let mut damages = [Damage::default(); 5];
        for (slot, damage) in damages.iter_mut().zip(calculate_damage(
            &spell,
            SpellType::Beam,
            CastType::SelfCast,
        )?) {
            *slot = damage;
        }
        for (index, name) in SELF_EFFECTS.iter().enumerate() {
            // RailGun uses element-mask membership, unlike Spray's integer
            // magnitude gate. Fractional magnitudes still start an effect.
            if spell.element.intersects(Elements(1 << index)) {
                host.start_effect(index, name);
            }
        }
        Ok(Self {
            lifecycle,
            spell,
            damages,
            ttl: 1.,
            has_cast: false,
        })
    }
    pub fn update(
        &mut self,
        dt: f32,
        owner: EffectOwner,
        host: &mut impl SelfBeamHost,
    ) -> (bool, f32) {
        // Delivery precedes the death/cancellation check, even on first update.
        if !self.has_cast {
            host.damage_self(self.damages, self.lifecycle.time_stamp);
            self.has_cast = true;
        }
        host.update_effects();
        if owner.dead || (owner.avatar && owner.cast_none) {
            self.ttl = 0.;
        }
        self.ttl -= dt;
        if self.ttl <= 0. || owner.dead || owner.cast_none {
            host.deinitialize(&mut self.lifecycle);
            return (false, 0.0325);
        }
        let (keep, _) = self
            .lifecycle
            .update(dt, EffectKind::RailGun, owner, |l| host.deinitialize(l));
        // The base writes into a separate local; derived out speed persists.
        (keep, 0.0325)
    }
}
