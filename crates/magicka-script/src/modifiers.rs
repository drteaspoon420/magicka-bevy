//! Active modifiers: deterministic, server-owned state. KV gives the
//! duration, stacking and stat properties; an optional Lua handler gets
//! `OnCreated`, `OnIntervalThink` and `OnDestroy`. The runtime adapter turns
//! the summed properties into gameplay each tick.
use crate::types::EntityRef;
use magicka_definitions::{DefinitionId, Definitions, Stacking};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveModifier {
    pub id: u64,
    pub target: EntityRef,
    pub modifier: DefinitionId,
    /// The mod that applied it (removed with that mod).
    pub owner: String,
    pub caster: Option<EntityRef>,
    pub stacks: u32,
    /// Seconds left; `None` lasts until removed.
    pub remaining: Option<f64>,
    think: f64,
}

/// A lifecycle callback the host must run for a modifier.
#[derive(Clone, Debug, PartialEq)]
pub enum ModifierEvent {
    Created(ActiveModifier),
    Think(ActiveModifier),
    Destroyed(ActiveModifier),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModifierStore {
    next: u64,
    active: Vec<ActiveModifier>,
}

impl ModifierStore {
    pub fn active(&self) -> &[ActiveModifier] {
        &self.active
    }
    pub fn on(&self, target: EntityRef) -> impl Iterator<Item = &ActiveModifier> {
        self.active.iter().filter(move |m| m.target == target)
    }
    pub fn has(&self, target: EntityRef, modifier: &DefinitionId) -> bool {
        self.on(target).any(|m| &m.modifier == modifier)
    }
    pub fn stacks(&self, target: EntityRef, modifier: &DefinitionId) -> u32 {
        self.on(target)
            .filter(|m| &m.modifier == modifier)
            .map(|m| m.stacks)
            .sum()
    }

    /// Apply per the definition's stacking rule.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &mut self,
        defs: &Definitions,
        target: EntityRef,
        modifier: &DefinitionId,
        duration: Option<f64>,
        stacks: u32,
        caster: Option<EntityRef>,
        owner: &str,
        events: &mut Vec<ModifierEvent>,
    ) -> Result<u64, String> {
        let def = defs
            .modifiers
            .get(modifier)
            .ok_or_else(|| format!("unknown modifier {modifier}"))?;
        let duration = duration
            .or_else(|| def.common.numbers.get("Duration").copied())
            .filter(|d| *d > 0.0);
        if duration.is_some_and(|d| !d.is_finite()) {
            return Err("duration must be finite".into());
        }
        let max = def.common.numbers.get("MaxStacks").copied().unwrap_or(1.0) as u32;
        let stacks = stacks.max(1);
        if def.stacking != Stacking::Independent {
            if let Some(existing) = self
                .active
                .iter_mut()
                .find(|m| m.target == target && &m.modifier == modifier)
            {
                existing.remaining = duration;
                if def.stacking == Stacking::Stack {
                    existing.stacks = (existing.stacks + stacks).min(max.max(1));
                }
                return Ok(existing.id);
            }
        }
        self.next += 1;
        let active = ActiveModifier {
            id: self.next,
            target,
            modifier: modifier.clone(),
            owner: owner.into(),
            caster,
            stacks: if def.stacking == Stacking::Stack {
                stacks.min(max.max(1))
            } else {
                1
            },
            remaining: duration,
            think: 0.0,
        };
        events.push(ModifierEvent::Created(active.clone()));
        self.active.push(active);
        Ok(self.next)
    }

    fn remove_where(
        &mut self,
        events: &mut Vec<ModifierEvent>,
        pred: impl Fn(&ActiveModifier) -> bool,
    ) {
        let (gone, kept): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.active).into_iter().partition(pred);
        self.active = kept;
        events.extend(gone.into_iter().map(ModifierEvent::Destroyed));
    }
    pub fn remove(
        &mut self,
        target: EntityRef,
        modifier: &DefinitionId,
        events: &mut Vec<ModifierEvent>,
    ) {
        self.remove_where(events, |m| m.target == target && &m.modifier == modifier);
    }
    /// Everything a mod applied (mod unload).
    pub fn remove_owner(&mut self, owner: &str, events: &mut Vec<ModifierEvent>) {
        self.remove_where(events, |m| m.owner == owner);
    }
    /// Everything on a removed or stale entity.
    pub fn retain_targets(
        &mut self,
        live: impl Fn(EntityRef) -> bool,
        events: &mut Vec<ModifierEvent>,
    ) {
        self.remove_where(events, |m| !live(m.target));
    }

    /// Advance timers: expirations and `TickInterval` thinks, in a stable
    /// order (application order).
    pub fn update(&mut self, defs: &Definitions, dt: f64, events: &mut Vec<ModifierEvent>) {
        for m in &mut self.active {
            if let Some(interval) = defs
                .modifiers
                .get(&m.modifier)
                .and_then(|d| d.common.numbers.get("TickInterval").copied())
                .filter(|i| *i > 0.0)
            {
                m.think += dt;
                while m.think >= interval {
                    m.think -= interval;
                    events.push(ModifierEvent::Think(m.clone()));
                }
            }
            if let Some(r) = &mut m.remaining {
                *r -= dt;
            }
        }
        self.remove_where(events, |m| m.remaining.is_some_and(|r| r <= 0.0));
    }

    /// Summed stat properties on `target` (each × stacks).
    pub fn properties(&self, defs: &Definitions, target: EntityRef) -> BTreeMap<String, f64> {
        let mut out = BTreeMap::new();
        for m in self.on(target) {
            if let Some(def) = defs.modifiers.get(&m.modifier) {
                for (k, v) in &def.properties {
                    *out.entry(k.clone()).or_default() += v * m.stacks as f64;
                }
            }
        }
        out
    }
}
