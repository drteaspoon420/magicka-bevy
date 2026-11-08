//! Numerical/control-flow port of Character.AddStatusEffect.
//! See output/research/character-status.md. Cached mask and cue-dependent
//! branches intentionally follow the original, including the four-slot mask bug.
use crate::resistance::Resistance;
use crate::spells::Elements as E;
use crate::status::{Status as S, StatusEffect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum StatusResult {
    None = 0,
    Added = 0x20,
    Removed = 0x40,
}

/// Presentation/owner calls emitted in original order. Drying carries no
/// geometry here; the owner supplies its position and direction when rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusEvent {
    DryingEffect,
    StopCue(S),
    PlaySteamCue,
    PlayStatusCue(S),
    StopCurrentSpell,
    Hud { status: S, added: bool },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CueState {
    pub exists: u16,
    pub playing: u16,
}
impl CueState {
    /// Synchronous owner cue calls issued by AddStatusEffect. Playback failure
    /// and backend disposal remain caller-owned facts.
    pub fn observe(&mut self, event: StatusEvent) {
        match event {
            StatusEvent::PlayStatusCue(kind) => {
                self.exists |= kind as u16;
                self.playing |= kind as u16;
            }
            StatusEvent::StopCue(kind) => self.playing &= !(kind as u16),
            _ => {}
        }
    }
    fn playing(self, kind: S) -> bool {
        self.exists & self.playing & kind as u16 != 0
    }
    fn exists(self, kind: S) -> bool {
        self.exists & kind as u16 != 0
    }
}

#[derive(Clone)]
pub struct StatusContext<'a> {
    pub immortal: bool,
    pub ethereal: bool,
    pub can_have_status: bool,
    pub avatar: bool,
    pub current_spell: bool,
    pub base: &'a [Resistance; 11],
    /// Required for avatars; ignored for other characters.
    pub equipment: Option<&'a [Resistance; 11]>,
    /// Only resistance buffs, preserving list order.
    pub buffs: &'a [Resistance],
    pub cues: CueState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusError {
    MissingAvatarEquipment,
    InvalidNoneStatus,
}

#[derive(Clone, Debug, Default)]
pub struct CharacterStatuses {
    pub(crate) slots: [StatusEffect; 9],
    /// The original cached mask can disagree with the live slots.
    pub current_mask: u16,
    pub dry_timer: f32,
}

impl CharacterStatuses {
    pub fn has(&self, kind: S) -> bool {
        self.current_mask & kind as u16 == kind as u16
    }
    /// Character.GetStatusEffects: all nine slots, dead ones included.
    pub fn slots(&self) -> &[StatusEffect; 9] {
        &self.slots
    }
    pub fn effect(&self, kind: S) -> Option<&StatusEffect> {
        index(kind).map(|i| &self.slots[i])
    }
    /// Restore a slot from a fixture/network state. Does not rebuild the mask.
    pub fn set_effect(&mut self, effect: StatusEffect) -> Result<(), StatusError> {
        let i = index(effect.kind).ok_or(StatusError::InvalidNoneStatus)?;
        self.slots[i] = effect;
        Ok(())
    }
    /// Character.AddBuff (Resistance): scale the slot's magnitude, zeroing
    /// it for a |multiplier| below 1e-6. Slots are addressed by index only.
    pub fn scale_magnitude(&mut self, kind: S, multiplier: f32) {
        if let Some(i) = index(kind) {
            let slot = &mut self.slots[i];
            if multiplier.abs() < 1e-6 {
                slot.magnitude = 0.0;
            } else {
                slot.magnitude *= multiplier;
            }
        }
    }
    /// Character.StopStatusEffects: stop every slot whose type is in `mask`
    /// and its cue. The cached mask updates on the next status update.
    pub fn stop_effects(&mut self, mask: u16, events: &mut Vec<StatusEvent>) {
        for slot in &mut self.slots {
            if slot.kind != S::None && slot.kind as u16 & mask != 0 {
                slot.stop();
                events.push(StatusEvent::StopCue(slot.kind));
            }
        }
    }
    fn active_slot(&self, kind: S) -> bool {
        !self.slots[index(kind).unwrap()].dead()
    }
    fn stop(&mut self, kind: S) {
        self.slots[index(kind).unwrap()].stop();
    }
    fn clear(&mut self, kind: S) {
        self.slots[index(kind).unwrap()] = StatusEffect::default();
    }

    pub fn add(
        &mut self,
        mut incoming: StatusEffect,
        ctx: &StatusContext<'_>,
        events: &mut Vec<StatusEvent>,
    ) -> Result<StatusResult, StatusError> {
        if ctx.immortal || ctx.ethereal || !ctx.can_have_status {
            return Ok(StatusResult::None);
        }
        let mut result = StatusResult::None;
        if !incoming.dead() {
            let slot = index(incoming.kind).ok_or(StatusError::InvalidNoneStatus)?;
            // Validate required owner data before mutating state.
            if ctx.avatar && ctx.equipment.is_none() {
                return Err(StatusError::MissingAvatarEquipment);
            }
            let mut removed = false;
            let mut resisted = false;
            let mut resistance_element = None;
            match incoming.kind {
                S::Burning => {
                    if self.active_slot(S::Cold) {
                        self.stop(S::Cold);
                        removed = true;
                    }
                    if self.active_slot(S::Wet) {
                        self.stop(S::Wet);
                        if self.dry_timer < 0.0 && ctx.cues.playing(S::Wet) {
                            self.dry_timer = 0.9;
                            events.extend([
                                StatusEvent::DryingEffect,
                                StatusEvent::StopCue(S::Wet),
                                StatusEvent::PlaySteamCue,
                            ]);
                        }
                        removed = true;
                    } else if self.active_slot(S::Frozen) {
                        let frozen = &mut self.slots[index(S::Frozen).unwrap()];
                        frozen.magnitude -= incoming.magnitude;
                        events.push(StatusEvent::DryingEffect);
                        if frozen.magnitude <= 0.0
                            && self.dry_timer < 0.0
                            && ctx.cues.playing(S::Frozen)
                        {
                            events.push(StatusEvent::StopCue(S::Frozen));
                            self.dry_timer = 0.9;
                            events.push(StatusEvent::DryingEffect);
                            if ctx.cues.exists(S::Wet) {
                                events.push(StatusEvent::StopCue(S::Wet));
                            }
                            events.push(StatusEvent::PlaySteamCue);
                        }
                        removed = true;
                    }
                    if self.has(S::Greased) {
                        incoming.dps *= 4.0;
                        self.clear(S::Greased);
                    }
                    resistance_element = Some(E::FIRE);
                }
                S::Cold => {
                    if self.active_slot(S::Burning) || self.dry_timer > 0.0 {
                        if ctx.cues.playing(S::Burning) {
                            self.stop(S::Burning);
                            self.dry_timer = 0.9;
                            events.extend([
                                StatusEvent::DryingEffect,
                                StatusEvent::StopCue(S::Burning),
                                StatusEvent::PlaySteamCue,
                            ]);
                        }
                        removed = true;
                    }
                    if self.has(S::Wet) && !ctx.base[E::ICE.index() as usize].status_resistance {
                        if ctx.current_spell {
                            events.push(StatusEvent::StopCurrentSpell);
                        }
                        self.slots[index(S::Frozen).unwrap()] =
                            StatusEffect::new(S::Frozen, 0.0, 1.0, false);
                        self.current_mask |= S::Frozen as u16;
                        if !ctx.cues.playing(S::Frozen) {
                            events.push(StatusEvent::PlayStatusCue(S::Frozen));
                        }
                        if ctx.cues.playing(S::Wet) {
                            events.push(StatusEvent::StopCue(S::Wet));
                        }
                        self.stop(S::Wet);
                        self.stop(S::Cold);
                        removed = true;
                    } else if self.has(S::Frozen) {
                        self.slots[index(S::Frozen).unwrap()].magnitude = 1.0;
                        removed = true;
                    } else {
                        resistance_element = Some(E::COLD);
                    }
                }
                S::Wet => {
                    if self.has(S::Burning) || self.dry_timer > 0.0 {
                        if ctx.cues.playing(S::Burning) {
                            events.push(StatusEvent::StopCue(S::Burning));
                            self.dry_timer = 0.9;
                            events.extend([StatusEvent::DryingEffect, StatusEvent::PlaySteamCue]);
                        }
                        self.clear(S::Burning);
                        removed = true;
                    }
                    if self.has(S::Greased) {
                        self.clear(S::Greased);
                    }
                    resistance_element = Some(E::WATER);
                }
                S::Greased => {
                    if self.has(S::Wet) {
                        self.clear(S::Wet);
                    }
                    resistance_element = Some(E::WATER);
                }
                S::Steamed => {
                    if self.has(S::Cold) {
                        self.clear(S::Cold);
                    }
                    resistance_element = Some(E::STEAM);
                }
                S::Poisoned => resistance_element = Some(E::POISON),
                S::Healing => {
                    if self.has(S::Poisoned) {
                        if ctx.avatar {
                            events.push(StatusEvent::Hud {
                                status: S::Poisoned,
                                added: false,
                            });
                        }
                        self.clear(S::Poisoned);
                    }
                    if self.has(S::Bleeding) {
                        self.clear(S::Bleeding);
                    }
                    resistance_element = Some(E::LIFE);
                }
                S::Frozen | S::Bleeding => {}
                S::None => unreachable!(),
            }
            if let Some(element) = resistance_element {
                let i = element.index() as usize;
                let mut multiplier = ctx.base[i].multiplier;
                let mut modifier = ctx.base[i].modifier;
                if ctx.avatar {
                    let equipment = ctx.equipment.unwrap();
                    multiplier *= equipment[i].multiplier;
                    modifier += equipment[i].modifier;
                }
                for buff in ctx.buffs {
                    if buff.against.0 & element.0 == element.0 {
                        multiplier *= buff.multiplier;
                        modifier += buff.modifier;
                    }
                }
                if incoming.kind == S::Burning {
                    incoming.dps = (incoming.dps + modifier) * multiplier;
                } else {
                    incoming.dps += modifier;
                    incoming.magnitude *= multiplier;
                }
                // CLR float.Epsilon is the smallest positive subnormal, NOT
                // Rust f32::EPSILON (machine epsilon).
                let zero = |v: f32| v.abs() < f32::from_bits(1);
                resisted = zero(incoming.magnitude)
                    || (!matches!(incoming.kind, S::Cold | S::Wet) && zero(incoming.dps))
                    || (incoming.kind != S::Greased && ctx.base[i].status_resistance);
            }
            if !removed && !resisted {
                if ctx.avatar {
                    events.push(StatusEvent::Hud {
                        status: incoming.kind,
                        added: true,
                    });
                }
                if matches!(incoming.kind, S::Burning | S::Wet | S::Frozen)
                    && !self.has(incoming.kind)
                    && !ctx.cues.playing(incoming.kind)
                {
                    events.push(StatusEvent::PlayStatusCue(incoming.kind));
                }
                self.slots[slot] = self.slots[slot]
                    .merged(incoming)
                    .expect("slot type matches status");
                result = StatusResult::Added;
            } else if removed {
                result = StatusResult::Removed;
            }
        }
        // Original IL_1284: compare the shifted BIT VALUE with 9, not index.
        // This visits Burning/Wet/Frozen/Cold only, even with nine live slots.
        self.current_mask = 0;
        for i in 0..4 {
            if !self.slots[i].dead() {
                self.current_mask |= 1 << i;
            }
        }
        Ok(result)
    }
}

fn index(kind: S) -> Option<usize> {
    (kind != S::None).then(|| (kind as u16).trailing_zeros() as usize)
}
