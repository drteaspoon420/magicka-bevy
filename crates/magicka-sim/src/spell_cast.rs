//! Spell.Cast dispatch and shared SpellEffect lifecycle, without derived effects.
//! See research/spell-cast.md.
use crate::{
    damage::CastType,
    spells::{Elements, Spell, SpellType},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Push,
    Spray,
    Projectile,
    Shield,
    RailGun,
    Lightning,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastNotice {
    VanillaIce,
    LetOffSomeSteam,
    StateAlchemist,
    WizardKing,
    BeamsTip,
}
/// Operations are synchronous and ordered. Handles represent cached instances,
/// not just spell families; the owner must observe its new handle during cast.
pub trait SpellCastHost {
    type Handle: Copy;
    fn notice(&mut self, notice: CastNotice);
    fn acquire(&mut self, kind: EffectKind) -> Self::Handle;
    fn set_current(&mut self, handle: Self::Handle);
    fn cast(&mut self, handle: Self::Handle, spell: Spell, cast_type: CastType, from_staff: bool);
}
#[derive(Clone, Copy, Debug, Default)]
pub struct CastOwner {
    pub avatar: bool,
    pub remote: bool,
}
impl Spell {
    pub fn cast_with(
        &self,
        cast_type: CastType,
        from_staff: bool,
        owner: CastOwner,
        host: &mut impl SpellCastHost,
    ) {
        if cast_type == CastType::Blast {
            return;
        }
        let kind = self.spell_type();
        if owner.avatar && !owner.remote {
            if self.element == Elements::ICE {
                host.notice(CastNotice::VanillaIce);
            }
            if self.element.intersects(Elements::STEAM) {
                host.notice(CastNotice::LetOffSomeSteam);
            }
            let count = (self.element.0 & 0x7ff).count_ones();
            if count >= 3 {
                host.notice(CastNotice::StateAlchemist);
            }
            if count >= 5 {
                host.notice(CastNotice::WizardKing);
            }
        }
        let effect = match kind {
            SpellType::Push => EffectKind::Push,
            SpellType::Spray => EffectKind::Spray,
            SpellType::Projectile => EffectKind::Projectile,
            SpellType::Shield => EffectKind::Shield,
            SpellType::Beam => {
                if owner.avatar && cast_type == CastType::Force {
                    host.notice(CastNotice::BeamsTip);
                }
                EffectKind::RailGun
            }
            SpellType::Lightning => EffectKind::Lightning,
            SpellType::Magick => return,
        };
        let handle = host.acquire(effect);
        if matches!(
            cast_type,
            CastType::Force | CastType::Area | CastType::SelfCast | CastType::Weapon
        ) {
            host.set_current(handle);
            host.cast(handle, *self, cast_type, from_staff);
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectLifecycle {
    pub active: bool,
    pub min_ttl: f32,
    pub max_ttl: f32,
    pub from_staff: bool,
    pub cast_type: CastType,
    pub time_stamp: f64,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct EffectOwner {
    pub avatar: bool,
    pub dead: bool,
    pub button_none: bool,
    pub cast_none: bool,
}
impl EffectLifecycle {
    /// Only valid for the four base Cast methods. Does not initialize mSpell:
    /// individual derived effect methods are responsible for that field.
    pub fn begin(cast_type: CastType, from_staff: bool, time_stamp: f64) -> Option<Self> {
        let (min_ttl, max_ttl) = match cast_type {
            CastType::SelfCast => (0.2, 0.2),
            CastType::Force | CastType::Area | CastType::Weapon => (0.5, 1.5),
            _ => return None,
        };
        Some(Self {
            active: true,
            min_ttl,
            max_ttl,
            from_staff,
            cast_type,
            time_stamp,
        })
    }
    /// Returns (continue_cast, turn_speed). Cleanup is owned by the derived
    /// effect; base CastUpdate does not itself clear Active.
    pub fn update(
        &mut self,
        dt: f32,
        kind: EffectKind,
        owner: EffectOwner,
        mut deinitialize: impl FnMut(&mut Self),
    ) -> (bool, f32) {
        self.min_ttl -= dt;
        self.max_ttl -= dt;
        let released = self.active
            && self.min_ttl <= 0.0
            && owner.avatar
            && owner.button_none
            && kind != EffectKind::Projectile;
        if released
            || !self.active
            || owner.dead
            || (owner.avatar && owner.cast_none && kind != EffectKind::Projectile)
        {
            deinitialize(self);
            return (false, 0.75);
        }
        (true, 0.75)
    }
    pub fn stop(&mut self, mut deinitialize: impl FnMut(&mut Self)) {
        deinitialize(self);
        self.active = false;
    }
}
