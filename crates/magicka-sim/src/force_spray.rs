//! Force spray initialization and timed pulse ownership. Presentation/liquid
//! operations remain synchronous host phases rather than fabricated effects.
use crate::{
    damage::{calculate_damage, CastType, Damage, DamageError, SpellType},
    spell_cast::{EffectKind, EffectLifecycle, EffectOwner},
    spells::{Elements, Spell},
    spray_pulse::{HitList, SprayPulse, SpraySource, SprayWorld},
};

#[derive(Clone, Copy, Debug)]
pub struct ForceCastFacts {
    pub from_staff: bool,
    pub character: bool,
    pub panic_cast: bool,
    pub greased: bool,
    pub timestamp: f64,
}
pub trait ForceSprayHost: SprayWorld {
    /// Original PlaySound, light setup, decal selection and visual starts, in
    /// that order. Called before greased-owner fire damage.
    fn initialize_force(&mut self, spell: Spell, range: f32, source: SpraySource);
    fn damage_owner_single(&mut self, damage: Damage, timestamp: f64, source: SpraySource);
    fn rumble(&mut self, left: f32, right: f32);
    /// Original full-range visual obstruction/light/decal/scourge updates then
    /// Liquid.Freeze. Host preserves their internal order and source quirks.
    fn update_force_presentation(
        &mut self,
        dt: f32,
        range: f32,
        from_staff: bool,
        source: SpraySource,
        damages: [Damage; 5],
    );
    fn deinitialize(&mut self, lifecycle: &mut EffectLifecycle);
}
#[derive(Clone, Debug)]
pub struct ForceSpray {
    pub lifecycle: EffectLifecycle,
    pub spell: Spell,
    pub ttl: f32,
    pub range: f32,
    pub range_multiplier: f32,
    pub pulse: SprayPulse,
}
impl ForceSpray {
    pub fn cast(
        spell: Spell,
        facts: ForceCastFacts,
        source: SpraySource,
        host: &mut impl ForceSprayHost,
    ) -> Result<Self, DamageError> {
        let mut lifecycle =
            EffectLifecycle::begin(CastType::Force, facts.from_staff, facts.timestamp).unwrap();
        if facts.character && !facts.panic_cast && lifecycle.from_staff {
            lifecycle.from_staff = false;
        }
        lifecycle.max_ttl = 4.0;
        let total = spell.mag[3] + spell.mag[2] + spell.mag[9] + spell.mag[1] + spell.mag[10];
        let range = 5.0 + total / 5.0 * 10.0 / 2.0;
        let mut damages = [Damage::default(); 5];
        for (slot, damage) in
            damages
                .iter_mut()
                .zip(calculate_damage(&spell, SpellType::Spray, CastType::Force)?)
        {
            *slot = damage;
        }
        host.initialize_force(spell, range, source);
        if facts.greased {
            // Original checks only A/B/C/D, delivering the first Fire entry as a
            // single damage payload (not a five-slot collection).
            if let Some(damage) = damages[..4]
                .iter()
                .find(|d| d.element & Elements::FIRE.0 != 0)
            {
                host.damage_owner_single(*damage, facts.timestamp, source);
            }
        }
        Ok(Self {
            lifecycle,
            spell,
            ttl: 4.0,
            range,
            range_multiplier: 0.0,
            pulse: SprayPulse {
                damages,
                timestamp: facts.timestamp,
                angle: std::f32::consts::PI / 10.0,
                hits: HitList::default(),
            },
        })
    }
    pub fn update(
        &mut self,
        dt: f32,
        owner: EffectOwner,
        controller_present: bool,
        source: SpraySource,
        host: &mut impl ForceSprayHost,
    ) -> (bool, f32) {
        self.pulse.hits.update(dt);
        if owner.avatar && controller_present {
            host.rumble(0.8, 0.8);
        }
        self.ttl -= dt;
        let multiplier = self.range_multiplier + dt * 4.0;
        // CLR Math.Min propagates NaN; Rust's min would select the finite input.
        self.range_multiplier = if multiplier.is_nan() {
            multiplier
        } else {
            multiplier.min(1.0)
        };
        if self.ttl > 0.0 {
            self.pulse.execute(
                self.range * self.range_multiplier,
                CastType::Force,
                self.lifecycle.from_staff,
                source,
                host,
            );
        }
        // Presentation/liquids still run on the final derived TTL frame.
        host.update_force_presentation(
            dt,
            self.range,
            self.lifecycle.from_staff,
            source,
            self.pulse.damages,
        );
        if self.ttl <= 0.0 || self.lifecycle.max_ttl <= 0.0 {
            host.deinitialize(&mut self.lifecycle);
            return (false, 0.6);
        }
        let (keep, _) = self
            .lifecycle
            .update(dt, EffectKind::Spray, owner, |l| host.deinitialize(l));
        (keep, 0.6)
    }
}
