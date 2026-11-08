//! SpraySpell self-cast branch. Derived cleanup is supplied by the effect owner.
use crate::{
    damage::{calculate_damage, CastType, Damage, DamageError, SpellType},
    spell_cast::{EffectKind, EffectLifecycle, EffectOwner},
    spells::{Elements, Spell},
};
pub const SELF_EFFECTS: [&str; 11] = [
    "self_earth",
    "self_water",
    "self_cold",
    "self_fire",
    "self_lightning",
    "self_arcane",
    "self_life",
    "self_fire",
    "self_ice",
    "self_steam",
    "self_steam",
];
/// Synchronous cast operations. Character.Electrocute is empty in the original;
/// other IDamageable implementations must provide their own behavior.
pub trait SelfSprayHost {
    fn electrocute_self(&mut self, multiplier: f32);
    fn start_effect(&mut self, element_index: usize, name: &'static str);
    fn damage_self(&mut self, damages: [Damage; 5], timestamp: f64);
}
#[derive(Clone, Debug)]
pub struct SelfSpray {
    pub lifecycle: EffectLifecycle,
    pub spell: Spell,
    pub damages: [Damage; 5],
    pub ttl: f32,
}
impl SelfSpray {
    pub fn cast(
        spell: Spell,
        from_staff: bool,
        timestamp: f64,
        host: &mut impl SelfSprayHost,
    ) -> Result<Self, DamageError> {
        let lifecycle = EffectLifecycle::begin(CastType::SelfCast, from_staff, timestamp).unwrap();
        let mut damages = [Damage::default(); 5];
        for (slot, damage) in damages.iter_mut().zip(calculate_damage(
            &spell,
            SpellType::Spray,
            CastType::SelfCast,
        )?) {
            *slot = damage;
        }
        let all = damages.iter().fold(0, |mask, damage| mask | damage.element);
        if all & (Elements::WATER | Elements::STEAM).0 == Elements::WATER.0 {
            host.electrocute_self(1.0);
        }
        for (i, name) in SELF_EFFECTS.iter().enumerate() {
            // Original converts magnitude to int and tests >0. For finite game
            // magnitudes this is equivalent to >=1, not merely positive.
            if spell.mag[i] >= 1.0 {
                host.start_effect(i, name);
            }
        }
        host.damage_self(damages, timestamp);
        Ok(Self {
            lifecycle,
            spell,
            damages,
            ttl: 1.0,
        })
    }
    /// Caller updates the shared hit list before this branch; it is empty for a
    /// fresh self cast. Deinitialize must run the original derived cleanup.
    pub fn update(
        &mut self,
        dt: f32,
        owner: EffectOwner,
        controller_present: bool,
        mut rumble: impl FnMut(f32, f32),
        mut deinitialize: impl FnMut(&mut EffectLifecycle),
    ) -> (bool, f32) {
        if owner.avatar && controller_present {
            rumble(0.8, 0.8);
        }
        if self.ttl <= 0.0 || self.lifecycle.max_ttl <= 0.0 {
            deinitialize(&mut self.lifecycle);
            return (false, 0.6);
        }
        let (keep, _) = self
            .lifecycle
            .update(dt, EffectKind::Spray, owner, deinitialize);
        // Derived out parameter stays .6; base writes to a different local.
        (keep, 0.6)
    }
}
