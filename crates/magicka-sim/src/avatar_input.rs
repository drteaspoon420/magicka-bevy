//! Avatar.HandleCombo and Character.CombineSpell, without controller polling.
//! See research/avatar-input.md. Caster damage is delivered synchronously.
use crate::damage::{calculate_damage, CastType, Damage, DamageError, SpellType};
use crate::spells::{self, Direction, Elements as E, MagickTree, MagickType, QueueEvent, Spell};
use crate::status::Status;
#[derive(Clone, Copy, Debug, Default)]
pub struct ConjureContext {
    pub in_event: bool,
    pub polymorphed: bool,
    pub charging: bool,
    pub networked: bool,
    pub status_mask: u16,
    pub wet_lightning_passive: bool,
    pub can_chant: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ConjureEvent {
    SendConjure(E),
    WetLightningTip,
    Queue(QueueEvent),
    ElementSound(E),
    MagickTip,
}
#[derive(Clone, Debug)]
pub struct AvatarInput {
    pub directions: Vec<Direction>,
    pub queue: Vec<Spell>,
    pub chant_direction: Direction,
    pub chanted_spells: i32,
    pub chanting_magick: bool,
    pub prepared: Spell,
}
impl Default for AvatarInput {
    fn default() -> Self {
        Self {
            directions: vec![],
            queue: vec![],
            chant_direction: Direction::Center,
            chanted_spells: 0,
            chanting_magick: false,
            prepared: Spell::default(),
        }
    }
}
impl AvatarInput {
    /// `deliver` must call the original-equivalent caster Damage route immediately.
    /// It receives all five slots, including default entries, with halved magnitudes.
    /// Effects preceding the callback (network message/tip) are supplied to it in
    /// `events`; consumers decide how to dispatch those external effects.
    pub fn direction(
        &mut self,
        direction: Direction,
        ctx: ConjureContext,
        tree: &MagickTree,
        permissions: (impl Fn(E) -> bool, impl Fn(MagickType) -> bool),
        mut deliver: impl FnMut([Damage; 5], &mut Vec<ConjureEvent>),
        events: &mut Vec<ConjureEvent>,
    ) -> Result<(), DamageError> {
        if ctx.in_event || ctx.polymorphed || ctx.charging {
            return Ok(());
        }
        self.chant_direction = direction;
        self.directions.push(direction);
        let spell = spells::handle_combo(&mut self.directions, permissions.0);
        if spell.element != E::NONE {
            if ctx.networked {
                events.push(ConjureEvent::SendConjure(spell.element));
            }
            let wet = ctx.status_mask & Status::Wet as u16 != 0;
            let greased = ctx.status_mask & Status::Greased as u16 != 0;
            if (spell.element.intersects(E::LIGHTNING) && wet && !ctx.wet_lightning_passive)
                || (spell.element.intersects(E::FIRE) && greased)
            {
                if wet {
                    events.push(ConjureEvent::WetLightningTip);
                }
                // The original explicitly selects Lightning even for Fire.
                let generated = calculate_damage(&spell, SpellType::Lightning, CastType::SelfCast)?;
                let mut damages = [Damage::default(); 5];
                for (slot, damage) in damages.iter_mut().zip(generated) {
                    *slot = damage;
                }
                for slot in &mut damages {
                    slot.magnitude *= 0.5;
                }
                deliver(damages, events);
                self.directions.clear();
                return Ok(());
            }
            let mut queue_events = vec![];
            if spells::try_add_to_queue(&mut self.queue, 5, spell, &mut queue_events) {
                self.chanted_spells = self.chanted_spells.wrapping_add(1);
            }
            events.extend(queue_events.into_iter().map(ConjureEvent::Queue));
            events.push(ConjureEvent::ElementSound(spell.element));
            self.directions.clear();
        }
        if !ctx.can_chant {
            self.chant_direction = Direction::Center;
        }
        self.chanting_magick = tree.exact_magick(&self.queue).is_some_and(permissions.1);
        if self.chanting_magick {
            events.push(ConjureEvent::MagickTip);
        }
        Ok(())
    }
    /// Character.CombineSpell stores the combined spell and clears its queue.
    /// This prepares a spell; it does not spawn a cast/projectile or reset it.
    pub fn combine_spell(&mut self) -> Spell {
        self.prepared = spells::combine(&self.queue);
        self.queue.clear();
        self.prepared
    }
}
