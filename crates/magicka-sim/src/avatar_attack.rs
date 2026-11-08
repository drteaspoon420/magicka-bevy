//! Avatar.Attack selection, independent of animation and equipment services.
//! Provenance and remaining integration: research/avatar-attack.md.
use crate::{avatar_cast_input::CastInput, damage::CastType};

#[derive(Clone, Copy, Debug, Default)]
pub struct AttackContext {
    pub current_spell: bool,
    pub casting: bool,
    pub charging: bool,
    pub input_limited: bool,
    pub item_cooling_down: bool,
    /// Queue is nonempty and its first element is not All.
    pub queued_elements: bool,
    pub weapon_charged: bool,
    pub focused: bool,
    pub gripped: bool,
    pub grip_type: i32,
    pub break_free_strength: f32,
    pub weapon_cast_enabled: bool,
    pub attacking: bool,
    pub animation_time: f32,
    pub animation_duration: f32,
    pub weapon_class: i32,
    pub wielding_gun: bool,
    pub networked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AttackEvent {
    SendWeaponCast,
    DisableTooFancyForFireballs,
    AddBreakFree(i32),
    /// Native Animation enum value and Character.Attack's gun argument.
    Attack {
        animation: i32,
        wielding_gun: bool,
    },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AttackInput {
    pub heavy_weapon_reload: bool,
}

impl AttackInput {
    /// Random is requested only when the native ordinary-weapon branch needs it.
    pub fn press(
        &mut self,
        cast: &mut CastInput,
        ctx: AttackContext,
        mut random: impl FnMut() -> f64,
        events: &mut Vec<AttackEvent>,
    ) {
        if ctx.current_spell
            || ctx.casting
            || ctx.charging
            || ctx.input_limited
            || ctx.item_cooling_down
        {
            return;
        }
        if ctx.queued_elements || (ctx.weapon_charged && !ctx.focused) {
            if cast.cast_type == CastType::Blast && !ctx.gripped && ctx.weapon_cast_enabled {
                cast.pressed = CastType::Weapon;
                cast.cast_type = CastType::Weapon;
                if ctx.networked {
                    events.push(AttackEvent::SendWeaponCast);
                }
                events.push(AttackEvent::DisableTooFancyForFireballs);
            }
            return;
        }
        events.push(AttackEvent::DisableTooFancyForFireballs);
        if ctx.grip_type == 0 && ctx.gripped {
            events.push(AttackEvent::AddBreakFree(ctx.break_free_strength as i32));
        }
        // Signed native bgt: equality does not open the next attack window.
        if ctx.attacking
            && ctx
                .animation_time
                .partial_cmp(&(ctx.animation_duration - 0.1))
                != Some(std::cmp::Ordering::Greater)
            && !ctx.focused
        {
            return;
        }
        let animation = if ctx.weapon_class == 17 {
            if ctx.attacking && !ctx.focused {
                return;
            }
            let animation = if self.heavy_weapon_reload { 26 } else { 25 };
            self.heavy_weapon_reload = !self.heavy_weapon_reload;
            animation
        } else if random() < 0.5 {
            26
        } else {
            25
        };
        events.push(AttackEvent::Attack {
            animation,
            wielding_gun: ctx.wielding_gun,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_spell_selects_weapon_cast_before_melee_and_obeys_gates() {
        let mut input = AttackInput::default();
        let mut cast = CastInput::default();
        let mut events = vec![];
        let mut ctx = AttackContext {
            queued_elements: true,
            weapon_cast_enabled: true,
            networked: true,
            ..Default::default()
        };
        ctx.gripped = true;
        input.press(&mut cast, ctx, || panic!("unexpected RNG"), &mut events);
        assert!(events.is_empty());
        ctx.gripped = false;
        input.press(&mut cast, ctx, || panic!("unexpected RNG"), &mut events);
        assert_eq!(cast.cast_type, CastType::Weapon);
        assert_eq!(cast.pressed, CastType::Weapon);
        assert_eq!(
            events,
            [
                AttackEvent::SendWeaponCast,
                AttackEvent::DisableTooFancyForFireballs
            ]
        );
    }

    #[test]
    fn focused_charged_weapon_attacks_instead_of_releasing() {
        let mut input = AttackInput::default();
        let mut cast = CastInput::default();
        let mut events = vec![];
        input.press(
            &mut cast,
            AttackContext {
                weapon_charged: true,
                focused: true,
                attacking: true,
                wielding_gun: true,
                ..Default::default()
            },
            || 0.5,
            &mut events,
        );
        assert_eq!(cast.cast_type, CastType::Blast);
        assert_eq!(
            events.last(),
            Some(&AttackEvent::Attack {
                animation: 25,
                wielding_gun: true
            })
        );
    }

    #[test]
    fn early_guards_preserve_reload_and_emit_nothing() {
        for gate in 0..5 {
            let mut ctx = AttackContext::default();
            match gate {
                0 => ctx.current_spell = true,
                1 => ctx.casting = true,
                2 => ctx.charging = true,
                3 => ctx.input_limited = true,
                _ => ctx.item_cooling_down = true,
            }
            let mut input = AttackInput {
                heavy_weapon_reload: true,
            };
            let mut events = vec![];
            input.press(
                &mut CastInput::default(),
                ctx,
                || panic!("unexpected RNG"),
                &mut events,
            );
            assert!(events.is_empty());
            assert!(input.heavy_weapon_reload);
        }
    }

    #[test]
    fn grip_and_attack_window_precede_heavy_weapon_alternation() {
        let mut input = AttackInput::default();
        let mut cast = CastInput::default();
        let mut events = vec![];
        let mut ctx = AttackContext {
            gripped: true,
            break_free_strength: 2.9,
            attacking: true,
            animation_time: 0.9,
            animation_duration: 1.0,
            weapon_class: 17,
            ..Default::default()
        };
        input.press(&mut cast, ctx, || panic!("unexpected RNG"), &mut events);
        assert_eq!(
            events,
            [
                AttackEvent::DisableTooFancyForFireballs,
                AttackEvent::AddBreakFree(2)
            ]
        );
        ctx.animation_time = 1.0;
        events.clear();
        input.press(&mut cast, ctx, || panic!("unexpected RNG"), &mut events);
        assert_eq!(
            events.len(),
            2,
            "heavy weapons cannot queue while attacking without focus"
        );
        ctx.attacking = false;
        for animation in [25, 26, 25] {
            events.clear();
            input.press(&mut cast, ctx, || panic!("unexpected RNG"), &mut events);
            assert_eq!(
                events.last(),
                Some(&AttackEvent::Attack {
                    animation,
                    wielding_gun: false
                })
            );
        }
    }
}
