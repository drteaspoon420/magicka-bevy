//! ShieldSpell's Self branch (CastSelf/CastUpdate). Pure Shield and
//! Earth/Ice self shields go through Character.AddSelfShield; other elements
//! add self-casted resistance auras (immunity: multiplier 0) in a fixed
//! order. See research/shield-self.md.
use crate::{
    damage::CastType,
    spell_cast::EffectLifecycle,
    spells::{Elements as E, Spell},
};

/// Elements.InstantNonPhysical | Water | Cold | Fire.
const AURA_ELEMENTS: u32 = 0x270 | 0x2 | 0x4 | 0x8;

/// AddAura order in CastSelf, with each aura's immunity element.
pub const AURA_ORDER: [E; 7] = [
    E::LIGHTNING,
    E::FIRE,
    E::COLD,
    E::STEAM,
    E::WATER,
    E::LIFE,
    E::ARCANE,
];

/// Self-casted immunity auras (AuraTarget.All, Buff resistance ×0).
#[derive(Clone, Debug, PartialEq)]
pub struct AuraPlan {
    pub ttl: f32,
    pub radius: f32,
    /// RemoveSelfShield first (no physical element in the spell).
    pub remove_self_shield: bool,
    pub immunities: Vec<E>,
}

/// CastSelf's owner operations, in order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ShieldSelfPlan {
    /// Character.AddSelfShield(spell) (non-clients).
    pub add_self_shield: bool,
    /// ClearAura then AddAura per immunity (Character owners only).
    pub auras: Option<AuraPlan>,
}

pub fn plan(spell: &Spell) -> ShieldSelfPlan {
    let elements = spell.element.0 & !E::SHIELD.0;
    if elements == 0 {
        return ShieldSelfPlan {
            add_self_shield: true,
            auras: None,
        };
    }
    let physical = elements & E::PHYSICAL_ELEMENTS.0 != 0;
    let mut out = ShieldSelfPlan {
        add_self_shield: physical,
        auras: None,
    };
    if elements & AURA_ELEMENTS == 0 {
        return out;
    }
    // num = max(1, magnitudes of every single element contained).
    let mut num = 1.0f32;
    for e in E::BY_INDEX {
        if e.0 & elements == e.0 {
            num = spell.get(e).max(num);
        }
    }
    out.auras = Some(AuraPlan {
        ttl: 15.0 + 5.0 * num,
        radius: 1.5 * num,
        remove_self_shield: !physical,
        immunities: AURA_ORDER
            .into_iter()
            .filter(|e| elements & e.0 != 0)
            .collect(),
    });
    out
}

#[derive(Clone, Debug)]
pub struct ShieldSelf {
    pub lifecycle: EffectLifecycle,
    /// mMinTTL (0.5 at cast).
    pub min_ttl: f32,
}

impl ShieldSelf {
    pub fn cast(from_staff: bool, timestamp: f64) -> Self {
        Self::begin(CastType::SelfCast, from_staff, timestamp)
    }

    /// ShieldSpell's shared Cast* tail: base lifecycle, mMinTTL 0.5,
    /// mHasCast (CastUpdate is the same for every cast type).
    pub fn begin(cast_type: CastType, from_staff: bool, timestamp: f64) -> Self {
        Self {
            lifecycle: EffectLifecycle::begin(cast_type, from_staff, timestamp).unwrap(),
            min_ttl: 0.5,
        }
    }

    /// CastUpdate: turn speed 0.1; DeInitialize once the owner's animation
    /// has finished or mMinTTL < 0. Always returns false (no base update).
    pub fn update(&mut self, dt: f32, animation_finished: bool) -> (bool, f32) {
        self.min_ttl -= dt;
        if self.lifecycle.active && (animation_finished || self.min_ttl < 0.0) {
            self.lifecycle.active = false;
        }
        (false, 0.1)
    }
}
