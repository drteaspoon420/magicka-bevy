//! Item special readiness and recharge, separate from AI ability cooldowns.
use magicka_content::common::SpecialAbility;
use std::sync::atomic::{AtomicU32, Ordering};
static NEXT_BUFF_ID: AtomicU32 = AtomicU32::new(0x8000_0000);

/// A fresh special-ability buff identity (separate from BuffStorage IDs).
pub fn next_special_buff_id() -> u32 {
    NEXT_BUFF_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Debug)]
pub struct SpecialInvocation {
    pub ability: SpecialAbility,
    pub buff_id: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct ItemSpecial {
    pub buff_id: u32,
    pub recharge: f32,
    pub cooldown: f32,
    pub ability: SpecialAbility,
}
impl ItemSpecial {
    pub fn new(recharge: f32, ability: SpecialAbility) -> Self {
        Self {
            buff_id: next_special_buff_id(),
            recharge,
            cooldown: 0.,
            ability,
        }
    }
    pub fn ready(&self) -> bool {
        self.cooldown <= 0.
    }
    pub fn tick(&mut self, dt: f32) {
        self.cooldown -= dt;
    }
    /// ExecuteSpecialAbility consumes recharge regardless of Execute's result.
    pub fn execute(&mut self) -> Option<(Option<u16>, SpecialInvocation)> {
        if !self.ready() {
            return None;
        }
        let animation = if self.ability.animation.is_empty() {
            Some(58)
        } else {
            crate::animation_controller::animation_value(&self.ability.animation)
        };
        self.cooldown = self.recharge;
        Some((
            animation,
            SpecialInvocation {
                ability: self.ability.clone(),
                buff_id: Some(self.buff_id),
            },
        ))
    }
}

pub fn npc_item_cast(
    character: &mut crate::character::RuntimeCharacter,
    weapons: &mut [(i32, crate::npc_combat::MeleeWeapon)],
    slot: i32,
    requests: &mut Vec<SpecialInvocation>,
) -> Option<u16> {
    let (_, weapon) = weapons.iter_mut().find(|(s, _)| *s == slot)?;
    let (animation, cast) = weapon.special.as_mut()?.execute()?;
    if cast.ability.type_name.eq_ignore_ascii_case("invisibility") {
        character.just_cast_invisible = true;
    }
    requests.push(cast);
    animation
}
