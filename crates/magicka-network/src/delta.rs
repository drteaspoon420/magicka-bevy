//! Baselines and deltas (protocol 12, plan milestone B2). The host sends
//! each client either a full [`GameFrame`] or a [`FrameDelta`] against the
//! newest frame that client acknowledged. Keyed collections carry only
//! changed/added items and removed keys; everything else only when it
//! changed. The encoder proves every delta by reconstructing the frame and
//! falls back to a full frame if the result is not identical, so a delta
//! can never silently diverge. Clients apply a delta only on top of a
//! frame they hold; otherwise they drop it and keep acknowledging their
//! last good frame until the host sends a full one.
use crate::{
    lobby::{LobbySettings, ParticipantState},
    session::{
        GameFrame, ModUiEntry, NpcState, PlayerState, PresentationEventState, RulesState,
        VisualEffectState,
    },
    world::WorldPresentation,
    RemoteBody,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// Changes to one keyed list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyed<K, V> {
    /// Added or changed items.
    pub changed: Vec<V>,
    pub removed: Vec<K>,
    /// The full key order, only when it differs from the natural result
    /// (surviving base order, then new items in frame order).
    pub order: Option<Vec<K>>,
}
impl<K, V> Default for Keyed<K, V> {
    fn default() -> Self {
        Self {
            changed: Vec::new(),
            removed: Vec::new(),
            order: None,
        }
    }
}
impl<K, V> Keyed<K, V> {
    fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty() && self.order.is_none()
    }
}

fn diff<K: Ord + Clone, V: PartialEq + Clone>(
    base: &[V],
    cur: &[V],
    key: impl Fn(&V) -> K,
) -> Keyed<K, V> {
    let old: BTreeMap<K, &V> = base.iter().map(|v| (key(v), v)).collect();
    let new: BTreeMap<K, &V> = cur.iter().map(|v| (key(v), v)).collect();
    let changed = cur
        .iter()
        .filter(|v| old.get(&key(v)) != Some(v))
        .cloned()
        .collect();
    let removed: Vec<K> = base
        .iter()
        .map(&key)
        .filter(|k| !new.contains_key(k))
        .collect();
    let natural: Vec<K> = base
        .iter()
        .map(&key)
        .filter(|k| new.contains_key(k))
        .chain(cur.iter().map(&key).filter(|k| !old.contains_key(k)))
        .collect();
    let actual: Vec<K> = cur.iter().map(&key).collect();
    Keyed {
        changed,
        removed,
        order: (natural != actual).then_some(actual),
    }
}

fn apply<K: Ord + Clone, V: Clone>(
    base: &[V],
    delta: &Keyed<K, V>,
    key: impl Fn(&V) -> K,
) -> Result<Vec<V>, DeltaError> {
    let mut by_key: BTreeMap<K, V> = base.iter().map(|v| (key(v), v.clone())).collect();
    let mut order: Vec<K> = base.iter().map(&key).collect();
    for k in &delta.removed {
        if by_key.remove(k).is_none() {
            return Err(DeltaError::Inconsistent);
        }
    }
    order.retain(|k| by_key.contains_key(k));
    for v in &delta.changed {
        let k = key(v);
        if by_key.insert(k.clone(), v.clone()).is_none() {
            order.push(k);
        }
    }
    if let Some(explicit) = &delta.order {
        if explicit.len() != by_key.len() || explicit.iter().any(|k| !by_key.contains_key(k)) {
            return Err(DeltaError::Inconsistent);
        }
        order = explicit.clone();
    }
    order
        .into_iter()
        .map(|k| by_key.remove(&k))
        .collect::<Option<Vec<_>>>()
        .ok_or(DeltaError::Inconsistent)
}

/// A frame relative to an acknowledged baseline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameDelta {
    pub tick: u64,
    pub time: f64,
    pub epoch: Option<u64>,
    pub scene: Option<String>,
    pub level: Option<String>,
    pub bodies: Keyed<u16, RemoteBody>,
    pub players: Keyed<usize, PlayerState>,
    pub npcs: Keyed<u16, NpcState>,
    pub effects: Keyed<(u16, i32), VisualEffectState>,
    pub rules: Option<Option<RulesState>>,
    pub lobby: Option<LobbySettings>,
    pub participants: Option<Vec<ParticipantState>>,
    pub world: Option<WorldPresentation>,
    pub mod_ui: Keyed<(String, String), ModUiEntry>,
    pub events: Keyed<u64, PresentationEventState>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Payload {
    Full(Box<GameFrame>),
    Delta(Box<FrameDelta>),
}

/// What the host sends (replaces raw `GameFrame` messages in protocol 12).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameUpdate {
    /// Host frame number; strictly increasing within a host session.
    pub id: u64,
    pub session: u64,
    /// The acknowledged frame a delta applies to.
    pub base: Option<u64>,
    pub payload: Payload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeltaError {
    /// The client does not hold the base frame.
    MissingBase,
    /// The delta does not fit its base.
    Inconsistent,
    Invalid,
}

fn changed<T: PartialEq + Clone>(base: &T, cur: &T) -> Option<T> {
    (base != cur).then(|| cur.clone())
}

pub fn encode(base: &GameFrame, cur: &GameFrame) -> FrameDelta {
    FrameDelta {
        tick: cur.physics.tick,
        time: cur.time,
        epoch: changed(&base.physics.epoch, &cur.physics.epoch),
        scene: changed(&base.physics.scene, &cur.physics.scene),
        level: changed(&base.level, &cur.level),
        bodies: diff(&base.physics.bodies, &cur.physics.bodies, |b| b.handle),
        players: diff(&base.players, &cur.players, |p| p.slot),
        npcs: diff(&base.npcs, &cur.npcs, |n| n.handle),
        effects: diff(&base.effects, &cur.effects, |e| (e.slot, e.instance)),
        rules: changed(&base.rules, &cur.rules),
        lobby: changed(&base.lobby, &cur.lobby),
        participants: changed(&base.participants, &cur.participants),
        world: changed(&base.world, &cur.world),
        mod_ui: diff(&base.mod_ui, &cur.mod_ui, |e| {
            (e.table.clone(), e.key.clone())
        }),
        events: diff(&base.events, &cur.events, |e| e.id),
    }
}

pub fn decode(base: &GameFrame, delta: &FrameDelta) -> Result<GameFrame, DeltaError> {
    let mut frame = base.clone();
    frame.physics.tick = delta.tick;
    frame.time = delta.time;
    if let Some(e) = delta.epoch {
        frame.physics.epoch = e;
    }
    if let Some(s) = &delta.scene {
        frame.physics.scene = s.clone();
    }
    if let Some(l) = &delta.level {
        frame.level = l.clone();
    }
    frame.physics.bodies = apply(&base.physics.bodies, &delta.bodies, |b| b.handle)?;
    frame.players = apply(&base.players, &delta.players, |p| p.slot)?;
    frame.npcs = apply(&base.npcs, &delta.npcs, |n| n.handle)?;
    frame.effects = apply(&base.effects, &delta.effects, |e| (e.slot, e.instance))?;
    if let Some(r) = &delta.rules {
        frame.rules = r.clone();
    }
    if let Some(l) = delta.lobby {
        frame.lobby = l;
    }
    if let Some(p) = &delta.participants {
        frame.participants = p.clone();
    }
    if let Some(w) = &delta.world {
        frame.world = w.clone();
    }
    frame.mod_ui = apply(&base.mod_ui, &delta.mod_ui, |e| {
        (e.table.clone(), e.key.clone())
    })?;
    frame.events = apply(&base.events, &delta.events, |e| e.id)?;
    if !frame.valid() {
        return Err(DeltaError::Invalid);
    }
    Ok(frame)
}

impl FrameDelta {
    pub fn is_empty(&self) -> bool {
        self.epoch.is_none()
            && self.scene.is_none()
            && self.level.is_none()
            && self.bodies.is_empty()
            && self.players.is_empty()
            && self.npcs.is_empty()
            && self.effects.is_empty()
            && self.rules.is_none()
            && self.lobby.is_none()
            && self.participants.is_none()
            && self.world.is_none()
            && self.mod_ui.is_empty()
            && self.events.is_empty()
    }
}

/// A player's view of a frame: presentation-only emitters and one-shot
/// events beyond `radius` of their wizard are dropped. Players, NPCs,
/// bodies, rules and world state are always kept (gameplay is never
/// culled). Without the player's wizard in the frame nothing is culled.
pub fn interest(frame: &GameFrame, slot: usize, radius: f32) -> GameFrame {
    let Some(me) = frame.players.iter().find(|p| p.slot == slot) else {
        return frame.clone();
    };
    let near = |p: [f32; 3]| {
        (0..3).map(|i| (p[i] - me.position[i]).powi(2)).sum::<f32>() <= radius * radius
    };
    let mut view = frame.clone();
    view.effects
        .retain(|e| near([e.transform[12], e.transform[13], e.transform[14]]));
    view.events.retain(|e| {
        e.owner.is_some_and(|o| {
            frame
                .players
                .iter()
                .any(|p| p.handle == o && p.slot == slot)
        }) || e.position.is_none_or(near)
    });
    view
}

/// Frames a host can still delta against (about 3 s at 20 Hz).
pub const HISTORY: usize = 64;

/// The host's encoder: frame history and per-base delta cache.
#[derive(Default)]
pub struct Encoder {
    next: u64,
    session: u64,
    history: VecDeque<(u64, GameFrame)>,
    /// Encoded updates for the current frame by base, shared by clients
    /// with the same acknowledgement.
    cache: BTreeMap<Option<u64>, GameUpdate>,
    /// Disable deltas (A/B measurement, fallback).
    pub full_only: bool,
}
impl Encoder {
    /// Record a new frame; returns its ID.
    pub fn push(&mut self, frame: GameFrame) -> u64 {
        if self.session != frame.physics.session {
            self.history.clear();
            self.session = frame.physics.session;
        }
        self.next += 1;
        self.history.push_back((self.next, frame));
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.cache.clear();
        self.next
    }
    pub fn current(&self) -> Option<&(u64, GameFrame)> {
        self.history.back()
    }
    /// The update for a client whose newest acknowledged frame is `acked`.
    pub fn update_for(&mut self, acked: Option<u64>) -> Option<GameUpdate> {
        let (id, cur) = self.history.back()?.clone();
        let base = acked
            .filter(|_| !self.full_only)
            .filter(|a| *a < id)
            .and_then(|a| self.history.iter().find(|(i, _)| *i == a))
            .filter(|(_, b)| b.physics.session == cur.physics.session)
            .map(|(i, f)| (*i, f.clone()));
        let key = base.as_ref().map(|(i, _)| *i);
        if let Some(u) = self.cache.get(&key) {
            return Some(u.clone());
        }
        let payload = match &base {
            Some((_, b)) => {
                let delta = encode(b, &cur);
                // Prove the delta reproduces the frame exactly.
                if decode(b, &delta).as_ref() == Ok(&cur) {
                    Payload::Delta(Box::new(delta))
                } else {
                    Payload::Full(Box::new(cur.clone()))
                }
            }
            None => Payload::Full(Box::new(cur.clone())),
        };
        let update = GameUpdate {
            id,
            session: cur.physics.session,
            base: matches!(payload, Payload::Delta(_))
                .then_some(key)
                .flatten(),
            payload,
        };
        self.cache.insert(key, update.clone());
        Some(update)
    }
}

/// The client's decoder: recent reconstructed frames by ID.
#[derive(Default)]
pub struct Decoder {
    frames: VecDeque<(u64, GameFrame)>,
    session: u64,
    newest: u64,
}
impl Decoder {
    /// Reconstruct an update. Out-of-date updates and deltas without a base
    /// are refused without changing state.
    pub fn apply(&mut self, update: GameUpdate) -> Result<GameFrame, DeltaError> {
        if update.session != self.session {
            self.frames.clear();
            self.session = update.session;
            self.newest = 0;
        }
        if update.id <= self.newest {
            return Err(DeltaError::Inconsistent);
        }
        let frame = match update.payload {
            Payload::Full(frame) => {
                if !frame.valid() || frame.physics.session != update.session {
                    return Err(DeltaError::Invalid);
                }
                *frame
            }
            Payload::Delta(delta) => {
                let base = update.base.ok_or(DeltaError::Invalid)?;
                let (_, base) = self
                    .frames
                    .iter()
                    .find(|(i, _)| *i == base)
                    .ok_or(DeltaError::MissingBase)?;
                decode(base, &delta)?
            }
        };
        self.newest = update.id;
        self.frames.push_back((update.id, frame.clone()));
        while self.frames.len() > 32 {
            self.frames.pop_front();
        }
        Ok(frame)
    }
    /// The newest frame this client holds (what it acknowledges).
    pub fn acknowledged(&self) -> Option<u64> {
        (self.newest != 0).then_some(self.newest)
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
