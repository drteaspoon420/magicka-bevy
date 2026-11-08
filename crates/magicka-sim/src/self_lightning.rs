//! LightningSpell's Self branch: CastSelf starts the self-cast effect chosen
//! by `Defines.ElementIndex` and damages the owner at once with the
//! Lightning/Self collection. CastUpdate never casts a bolt for Self (the
//! bolt branch requires CastType != Self), so only the shared lifecycle
//! ends the effect. See research/self-lightning.md.
use crate::{
    damage::{calculate_damage, CastType, Damage, DamageError, SpellType},
    self_spray::SELF_EFFECTS,
    spell_cast::{EffectKind, EffectLifecycle, EffectOwner},
    spells::Spell,
};

pub trait SelfLightningHost {
    fn start_effect(&mut self, element_index: usize, name: &'static str);
    fn damage_self(&mut self, damages: [Damage; 5], timestamp: f64);
}

/// Defines.ElementIndex: `(int)(ln(mask) · (1/ln 2) + 0.5)`, 11 for All.
/// Combined masks round log2 (Lightning|Fire 0x18 → 5).
pub fn element_index(mask: u32) -> usize {
    if mask == 0xFFFF {
        return 11;
    }
    let one_over_ln2 = 1.0 / 2.0f64.ln();
    ((mask as f64).ln() * one_over_ln2 + 0.5) as i32 as usize
}

#[derive(Clone, Debug)]
pub struct SelfLightning {
    pub lifecycle: EffectLifecycle,
    pub damages: [Damage; 5],
}

impl SelfLightning {
    /// CastSelf: base lifecycle, self-cast effect, CalculateDamage, then the
    /// owner's IDamageable.Damage(collection) at its position.
    pub fn cast(
        spell: Spell,
        from_staff: bool,
        timestamp: f64,
        host: &mut impl SelfLightningHost,
    ) -> Result<Self, DamageError> {
        let lifecycle = EffectLifecycle::begin(CastType::SelfCast, from_staff, timestamp).unwrap();
        let index = element_index(spell.element.0);
        if let Some(name) = SELF_EFFECTS.get(index) {
            host.start_effect(index, name);
        }
        let mut damages = [Damage::default(); 5];
        for (slot, damage) in damages.iter_mut().zip(calculate_damage(
            &spell,
            SpellType::Lightning,
            CastType::SelfCast,
        )?) {
            *slot = damage;
        }
        host.damage_self(damages, timestamp);
        Ok(Self { lifecycle, damages })
    }

    /// CastUpdate for Self: turn speed 0.1, then the base update.
    pub fn update(
        &mut self,
        dt: f32,
        owner: EffectOwner,
        deinitialize: impl FnMut(&mut EffectLifecycle),
    ) -> (bool, f32) {
        let (keep, _) = self
            .lifecycle
            .update(dt, EffectKind::Lightning, owner, deinitialize);
        (keep, 0.1)
    }
}
