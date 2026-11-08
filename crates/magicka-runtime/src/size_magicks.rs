//! Cached Grow/Shrink effects and their retained, invisible self auras.
//! See research/size-magicks.md and size-magicks/aura-construction-il.txt.
use crate::buffs::{next_buff_id, NonvisualKind, AURA_PERIOD, BUFF_TTL};
use magicka_sim::{
    damage::Damage,
    size_magicks::{CastResult, SizeEffect, SizeKind, SizeMagicks},
    spells::Elements,
};

pub trait SizeHost {
    fn size_owner(&self, owner: u16) -> Option<(f32, f32)>;
    fn apply_size(&mut self, owner: u16, scale: f32, mass: f32, speed: f32);
    fn reset_size(&mut self, owner: u16);
    fn size_buff(&mut self, owner: u16, id: u32, kind: NonvisualKind, ttl: f32);
}

#[derive(Clone, Debug)]
struct Auras {
    ids: [u32; 2],
    periods: [f32; 2],
}
impl Default for Auras {
    fn default() -> Self {
        Self {
            ids: [next_buff_id(), next_buff_id()],
            periods: [0.0; 2],
        }
    }
}
impl Auras {
    fn execute(&mut self, owner: u16, kind: SizeKind, dt: f32, host: &mut impl SizeHost) {
        let buffs = [
            NonvisualKind::BoostDamage(Damage {
                attack: 1,
                element: Elements::EARTH.0,
                amount: 0.0,
                magnitude: kind.earth_damage_multiplier(),
            }),
            NonvisualKind::ModifyHitPoints {
                multiplier: kind.health_multiplier(),
                modifier: 0.0,
            },
        ];
        for (i, buff) in buffs.into_iter().enumerate() {
            self.periods[i] -= dt;
            while self.periods[i] <= 0.0 {
                self.periods[i] += AURA_PERIOD;
                // AuraTarget.Self bypasses the area/ethereal filter.
                host.size_buff(owner, self.ids[i], buff, BUFF_TTL);
            }
        }
    }
}

#[derive(Clone, Debug)]
struct ActiveSize {
    owner: u16,
    state: SizeMagicks,
    auras: Auras,
}

#[derive(Clone, Debug, Default)]
pub struct SizeEffects {
    active: Vec<ActiveSize>,
    grow_cache: Vec<Auras>,
    shrink_cache: Vec<Auras>,
}
impl SizeEffects {
    fn cache(&mut self, kind: SizeKind) -> &mut Vec<Auras> {
        match kind {
            SizeKind::Grow => &mut self.grow_cache,
            SizeKind::Shrink => &mut self.shrink_cache,
        }
    }
    pub fn active(&self, owner: u16) -> Option<SizeEffect> {
        self.active
            .iter()
            .find(|e| e.owner == owner)?
            .state
            .active()
    }
    pub fn cast(&mut self, owner: u16, kind: SizeKind) -> CastResult {
        // GetInstance takes the last cached object even for refresh/cancel.
        let auras = self.cache(kind).pop().unwrap_or_default();
        if let Some(e) = self.active.iter_mut().find(|e| e.owner == owner) {
            let result = e.state.cast(kind);
            self.cache(kind).push(auras);
            return result;
        }
        let mut state = SizeMagicks::default();
        let result = state.cast(kind);
        self.active.push(ActiveSize {
            owner,
            state,
            auras,
        });
        result
    }
    pub fn update(&mut self, dt: f32, host: &mut impl SizeHost) {
        let mut i = 0;
        while i < self.active.len() {
            let e = &mut self.active[i];
            let kind = e.state.active().unwrap().kind;
            let removed = if let Some((hp, mass)) = host.size_owner(e.owner) {
                let frame = e.state.update(dt, hp, mass).unwrap();
                host.apply_size(e.owner, frame.scale, frame.mass, frame.speed_multiplier);
                // SpellManager updates both auras before IsDead/OnRemove.
                e.auras.execute(e.owner, kind, dt, host);
                frame.removed
            } else {
                true
            };
            if removed {
                let e = self.active.remove(i);
                host.reset_size(e.owner);
                self.cache(kind).push(e.auras);
            } else {
                i += 1;
            }
        }
    }
    pub fn clear(&mut self, host: &mut impl SizeHost) {
        for e in std::mem::take(&mut self.active) {
            let kind = e.state.active().unwrap().kind;
            host.reset_size(e.owner);
            self.cache(kind).push(e.auras);
        }
    }
    pub fn remove_owner(&mut self, owner: u16, host: &mut impl SizeHost) {
        if let Some(i) = self.active.iter().position(|e| e.owner == owner) {
            let e = self.active.remove(i);
            let kind = e.state.active().unwrap().kind;
            host.reset_size(owner);
            self.cache(kind).push(e.auras);
        }
    }
    /// Avatar objects persist across ordinary scene changes; rewrite handles
    /// can change. Retire effects whose NPC owners are not carried by this host.
    pub fn transition_players(&mut self, handles: &[(u16, u16)], host: &mut impl SizeHost) {
        let mut i = 0;
        while i < self.active.len() {
            if let Some((_, new)) = handles.iter().find(|(old, _)| *old == self.active[i].owner) {
                self.active[i].owner = *new;
                i += 1;
            } else {
                let owner = self.active[i].owner;
                self.remove_owner(owner, host);
            }
        }
    }
}
