//! SpraySpell.CastWeapon and its one-shot Helper.ArcDamage geometry.
//! Native locations and limitations: research/spray-weapon.md.
use crate::{
    damage::{calculate_damage, CastType, Damage, DamageError},
    impact_motion::Vector,
    spell_cast::{EffectKind, EffectLifecycle, EffectOwner},
    spells::{Elements, Spell, SpellType},
};

pub const SWORD_EFFECTS: [&str; 11] = [
    "",
    "weapon_spray_water",
    "weapon_spray_cold",
    "weapon_spray_fire",
    "weapon_spray_lightning",
    "",
    "",
    "",
    "",
    "weapon_spray_steam",
    "weapon_spray_poison",
];

#[derive(Clone, Debug)]
pub struct SprayWeapon {
    pub lifecycle: EffectLifecycle,
    pub private_ttl: f32,
}
impl SprayWeapon {
    pub fn begin(from_staff: bool, timestamp: f64) -> Self {
        // Native invokes the Self base method, then overwrites CastType.
        let mut lifecycle =
            EffectLifecycle::begin(CastType::SelfCast, from_staff, timestamp).unwrap();
        lifecycle.cast_type = CastType::Weapon;
        Self {
            private_ttl: lifecycle.max_ttl,
            lifecycle,
        }
    }
    pub fn update(&mut self, dt: f32, owner: EffectOwner) -> (bool, f32) {
        // Only Force decrements the private timer in SpraySpell.CastUpdate.
        if self.private_ttl <= 0.0 || self.lifecycle.max_ttl <= 0.0 {
            self.lifecycle.active = false;
            return (false, 0.6);
        }
        let (keep, _) = self
            .lifecycle
            .update(dt, EffectKind::Spray, owner, |l| l.active = false);
        (keep, 0.6)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SprayWeaponPlan {
    pub damages: [Damage; 5],
    pub effect_indices: Vec<usize>,
    pub greased_owner_damage: Option<Damage>,
}
impl SprayWeaponPlan {
    pub const RANGE: f32 = 6.0;
    pub const ANGLE: f32 = std::f32::consts::FRAC_PI_2;
    pub fn new(spell: &Spell, greased: bool) -> Result<Self, DamageError> {
        let mut damages = [Damage::default(); 5];
        for (slot, damage) in
            damages
                .iter_mut()
                .zip(calculate_damage(spell, SpellType::Spray, CastType::Weapon)?)
        {
            *slot = damage;
        }
        let effect_indices = Elements::BY_INDEX
            .iter()
            .enumerate()
            .filter_map(|(i, e)| (e.0 & 1614 != 0 && spell.get(*e) as i32 > 0).then_some(i))
            .collect();
        Ok(Self {
            greased_owner_damage: crate::spray_area::greased_damage(&damages, greased),
            damages,
            effect_indices,
        })
    }
}

/// Helper.ArcDamage: full 3D distance for falloff, planar center direction for
/// the cone. Call after the entity query and ethereal/caster exclusions. Native
/// does not perform a capsule arc intersection or a line-of-sight test here.
pub fn target_scale(
    origin: Vector,
    direction: Vector,
    range: f32,
    angle: f32,
    target: Vector,
    radius: f32,
) -> Option<f32> {
    let delta = [
        target[0] - origin[0],
        target[1] - origin[1],
        target[2] - origin[2],
    ];
    let distance = delta.iter().map(|v| v * v).sum::<f32>().sqrt();
    let planar = (delta[0] * delta[0] + delta[2] * delta[2]).sqrt();
    let angle = if angle.is_nan() {
        angle
    } else {
        angle.min(std::f32::consts::PI)
    };
    let cosine = f64::from(angle).cos() as f32;
    let dot = delta[0] / planar * direction[0]
        + (0.0 / planar) * direction[1]
        + delta[2] / planar * direction[2];
    if planar.partial_cmp(&1e-6) != Some(std::cmp::Ordering::Less)
        && (dot.is_nan() || cosine.is_nan() || dot < cosine)
    {
        return None;
    }
    let scale = 1.0 - (distance - radius) / range;
    // Native signed ble permits unordered values.
    if scale <= 0.0 {
        None
    } else {
        Some(scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arc_uses_planar_center_angle_and_unclamped_three_dimensional_falloff() {
        let scale = |target, radius| {
            target_scale(
                [0.0; 3],
                [0.0, 0.0, -1.0],
                6.0,
                SprayWeaponPlan::ANGLE,
                target,
                radius,
            )
        };
        assert_eq!(scale([0.0, 0.0, -3.0], 0.0), Some(0.5));
        assert_eq!(scale([0.0, 0.0, 3.0], 1.0), None);
        assert_eq!(scale([0.0, 0.0, -6.0], 0.0), None);
        assert_eq!(scale([0.0, 0.0, 0.0], 1.0), Some(1.0 + 1.0 / 6.0));
        assert_eq!(scale([0.0, 4.0, -3.0], 0.0), Some(1.0 - 5.0 / 6.0));
    }
    #[test]
    fn weapon_lifetime_uses_self_base_and_does_not_decrement_private_timer() {
        let mut w = SprayWeapon::begin(true, 17.0);
        assert_eq!(w.lifecycle.min_ttl, 0.2);
        assert_eq!(w.lifecycle.max_ttl, 0.2);
        assert_eq!(w.lifecycle.cast_type, CastType::Weapon);
        let owner = EffectOwner {
            avatar: true,
            ..Default::default()
        };
        assert!(w.update(0.1, owner).0);
        assert!(w.update(0.1, owner).0);
        assert!(!w.update(0.1, owner).0);
        assert_eq!(w.private_ttl, 0.2);
    }
    #[test]
    fn fractional_elements_keep_damage_but_do_not_start_truncated_effects() {
        let mut spell = Spell::default_for(Elements::FIRE);
        spell.mag[3] = 0.75;
        let plan = SprayWeaponPlan::new(&spell, true).unwrap();
        assert!(plan.effect_indices.is_empty());
        assert!(plan.greased_owner_damage.is_some());
        assert!(plan.damages.iter().any(|d| d.amount > 0.0));
        spell.mag[3] = 1.0;
        assert_eq!(
            SprayWeaponPlan::new(&spell, false).unwrap().effect_indices,
            [3]
        );
    }
}
