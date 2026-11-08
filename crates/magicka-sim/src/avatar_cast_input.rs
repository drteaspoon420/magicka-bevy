//! Avatar casting button transitions. See research/avatar-cast-input.md.
use crate::{avatar_input::AvatarInput, damage::CastType, spells::Spell};

/// Original None/Blast alias is represented by CastType::Blast (numeric zero).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CastInput {
    pub cast_type: CastType,
    pub pressed: CastType,
    pub charge_unlocked: bool,
}
impl Default for CastInput {
    fn default() -> Self {
        Self {
            cast_type: CastType::Blast,
            pressed: CastType::Blast,
            charge_unlocked: false,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct CastInputContext {
    pub networked: bool,
    pub staff_equipped: bool,
    pub cast_enabled: bool,
    pub push_enabled: bool,
    pub queue_nonempty: bool,
    pub input_limited: bool,
    pub current_spell: bool,
    pub frozen: bool,
    pub flying: bool,
    pub pushed: bool,
    pub knocked_down: bool,
    pub casting: bool,
    pub charging: bool,
    pub item_special: bool,
    pub source_item_index: i32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CastInputEvent {
    SendCast(CastType),
    SendItemSpecial(i32),
    ExecuteItemSpecial(i32),
    ClearNextAttackAnimation,
    ClearIcons,
}
impl CastInput {
    /// Avatar.Update: completed Weapon/Self casts release their synthetic button.
    /// Force/Area remain controlled by their physical press/release paths.
    pub fn clear_completed_button(
        &mut self,
        current_spell: bool,
        networked: bool,
        events: &mut Vec<CastInputEvent>,
    ) {
        if !current_spell
            && self.cast_type == CastType::Blast
            && matches!(self.pressed, CastType::Weapon | CastType::SelfCast)
        {
            self.pressed = CastType::Blast;
            if networked {
                events.push(CastInputEvent::SendCast(CastType::Blast));
            }
        }
    }

    pub fn force_pressed(&mut self, ctx: CastInputContext, events: &mut Vec<CastInputEvent>) {
        self.press(CastType::Force, ctx, events);
    }
    pub fn area_pressed(&mut self, ctx: CastInputContext, events: &mut Vec<CastInputEvent>) {
        self.press(CastType::Area, ctx, events);
    }
    fn press(&mut self, kind: CastType, ctx: CastInputContext, events: &mut Vec<CastInputEvent>) {
        if ctx.staff_equipped
            && ctx.cast_enabled
            && (ctx.queue_nonempty || ctx.push_enabled)
            && self.pressed == CastType::Blast
            && self.cast_type == CastType::Blast
        {
            self.cast_type = kind;
            self.pressed = kind;
            if ctx.networked {
                events.push(CastInputEvent::SendCast(kind));
            }
        }
    }
    pub fn force_released(&mut self, networked: bool, events: &mut Vec<CastInputEvent>) {
        self.charge_unlocked = true;
        self.release(CastType::Force, networked, events);
    }
    pub fn area_released(&mut self, networked: bool, events: &mut Vec<CastInputEvent>) {
        self.release(CastType::Area, networked, events);
    }
    fn release(&mut self, kind: CastType, networked: bool, events: &mut Vec<CastInputEvent>) {
        if self.pressed == kind {
            self.pressed = CastType::Blast;
            if networked {
                events.push(CastInputEvent::SendCast(CastType::Blast));
            }
        }
    }
    /// Immediate CastSpell(true, "") is synchronous: it can mutate button state
    /// before SendCast reads it. The callback must implement that owner operation.
    pub fn special(
        &mut self,
        ctx: CastInputContext,
        mut cast_now: impl FnMut(&mut Self, &mut Vec<CastInputEvent>),
        events: &mut Vec<CastInputEvent>,
    ) {
        if ctx.input_limited && !ctx.current_spell {
            return;
        }
        if ctx.queue_nonempty {
            if ctx.cast_enabled && self.cast_type == CastType::Blast {
                self.pressed = CastType::SelfCast;
                self.cast_type = CastType::SelfCast;
                if ctx.frozen || ctx.flying || ctx.pushed || ctx.knocked_down {
                    cast_now(self, events);
                }
                if ctx.networked {
                    events.push(CastInputEvent::SendCast(self.pressed));
                }
            }
        } else if !ctx.casting && !ctx.charging && ctx.item_special {
            if ctx.networked {
                events.push(CastInputEvent::SendItemSpecial(ctx.source_item_index));
            }
            events.push(CastInputEvent::ExecuteItemSpecial(ctx.source_item_index));
        }
    }
    pub fn special_released(&mut self) {
        if self.pressed == CastType::SelfCast {
            self.pressed = CastType::Blast;
        }
    }
    pub fn attack_released(&mut self, wielding_gun: bool, events: &mut Vec<CastInputEvent>) {
        if self.cast_type == CastType::Blast && self.pressed == CastType::Weapon {
            self.pressed = CastType::Blast;
        }
        if wielding_gun {
            events.push(CastInputEvent::ClearNextAttackAnimation);
        }
    }
}
impl AvatarInput {
    /// Avatar override: clears icon display, combines, resets chant count, clears queue.
    pub fn combine_avatar_spell(&mut self, events: &mut Vec<CastInputEvent>) -> Spell {
        events.push(CastInputEvent::ClearIcons);
        self.prepared = crate::spells::combine(&self.queue);
        self.chanted_spells = 0;
        self.queue.clear();
        self.prepared
    }
}
