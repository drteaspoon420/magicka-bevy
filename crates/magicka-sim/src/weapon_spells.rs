//! Item spell storage and Avatar weapon-cast dispatch. See research/weapon-spells.md.
use crate::{
    avatar_input::AvatarInput,
    damage::CastType,
    spells::{self, Elements, QueueEvent, Spell},
};
#[derive(Clone, Debug, Default)]
pub struct WeaponSpells {
    queue: Vec<Spell>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WeaponSpellEvent {
    ChantMerge {
        a: Elements,
        b: Elements,
        out: Elements,
    },
    StopEffect(usize),
    StartEffect(usize),
    /// Consumer computes original GetColor from this combined spell.
    RefreshColor(Spell),
    ClearIcons,
    DisableTooFancyForFireballs,
    AwardEnchanter,
}
impl Spell {
    /// Original Normalize divides all fields, including zeros, without a zero guard.
    pub fn normalized(self) -> Self {
        let total = self.total_magnitude();
        let mut result = self;
        for magnitude in &mut result.mag {
            *magnitude /= total;
        }
        result
    }
}
impl WeaponSpells {
    /// Restore stored values without normalization, merging or gameplay events.
    /// The caller validates snapshot capacity and fields before restoring them.
    pub fn from_stored_queue(queue: Vec<Spell>) -> Self {
        Self { queue }
    }
    pub fn queue(&self) -> &[Spell] {
        &self.queue
    }
    pub fn charged(&self) -> bool {
        !self.queue.is_empty()
    }
    pub fn peek(&self) -> Spell {
        spells::combine(&self.queue)
    }
    pub fn retrieve(&mut self, events: &mut Vec<WeaponSpellEvent>) -> Spell {
        let spell = self.peek();
        self.queue.clear();
        events.extend((0..11).map(WeaponSpellEvent::StopEffect));
        spell
    }
    pub fn add(
        &mut self,
        spell: Spell,
        lightning_precedence: bool,
        events: &mut Vec<WeaponSpellEvent>,
    ) -> bool {
        let target = self.queue.len() + 1;
        if self.queue.len() < 5 {
            self.queue.push(spell.normalized());
            let mut merges = vec![];
            spells::find_opposites_and_combinables(&mut self.queue, &mut merges);
            for event in merges {
                // Item passes null Player, so no icon operations occur.
                if let QueueEvent::ChantMerge { a, b, out } = event {
                    events.push(WeaponSpellEvent::ChantMerge { a, b, out });
                }
            }
        } else {
            let mut index = spells::find_opposites(&self.queue, 4, spell.element);
            if index >= 0 {
                self.queue.remove(index as usize);
            } else {
                index = spells::find_revertables(&self.queue, 4, spell.element);
                if index < 0 {
                    index = spells::find_combines(&self.queue, 4, spell.element);
                }
                if index >= 0 {
                    self.queue[index as usize] = spell + self.queue[index as usize];
                }
            }
            if lightning_precedence && index == -1 {
                index = spells::find_different_element(&self.queue, 4, Elements::LIGHTNING);
                if index >= 0 {
                    self.queue[index as usize] = Spell::default_for(Elements::LIGHTNING);
                }
            }
        }
        let grew = self.queue.len() == target;
        for (i, element) in Elements::BY_INDEX.iter().enumerate() {
            if spell.element & *element == *element {
                events.push(WeaponSpellEvent::StopEffect(i));
                if grew {
                    events.push(WeaponSpellEvent::StartEffect(i));
                }
            }
        }
        events.push(WeaponSpellEvent::RefreshColor(self.peek()));
        grew
    }
}
/// Continuation selected by Avatar.CastSpell; actual Spell.Cast/base CastSpell is
/// a caller operation. The released spell has already been removed from the item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AvatarCastDispatch {
    Base,
    Enchanted,
    Release(Spell),
}
impl AvatarInput {
    pub fn dispatch_weapon_cast(
        &mut self,
        cast_type: CastType,
        remote_avatar: bool,
        weapon: &mut WeaponSpells,
        events: &mut Vec<WeaponSpellEvent>,
    ) -> AvatarCastDispatch {
        if self.prepared.element != Elements::ALL {
            events.push(WeaponSpellEvent::DisableTooFancyForFireballs);
        }
        if self.prepared.element == Elements::ALL || cast_type != CastType::Weapon {
            return AvatarCastDispatch::Base;
        }
        if weapon.charged() && self.queue.is_empty() {
            return AvatarCastDispatch::Release(weapon.retrieve(events));
        }
        for &spell in &self.queue {
            weapon.add(spell, false, events);
        }
        events.push(WeaponSpellEvent::ClearIcons);
        self.queue.clear();
        self.chanted_spells = 0;
        if !remote_avatar {
            events.push(WeaponSpellEvent::AwardEnchanter);
        }
        AvatarCastDispatch::Enchanted
    }
}
