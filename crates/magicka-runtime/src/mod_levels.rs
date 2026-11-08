//! Lua-driven levels (plan workstream G). Server scripts reach the native
//! level runtime through typed host calls: actions defined once and invoked
//! many times, conditions evaluated every tick, and native `<Trigger>`s
//! added to the scene. Native XML triggers and actions keep running
//! unchanged. Level callbacks (`OnTriggerEnter`, `OnNpcKilled`,
//! `OnCheckpoint`, `OnWaveStart`, `OnWaveComplete`) come from authoritative
//! state, so clients (and late joiners) never run level Lua: they see its
//! results.
use crate::play_session::PlaySession;
use crate::script_schema::ScriptCall;
use magicka_script::{EntityRef, WorldSnapshot};
use std::collections::{BTreeMap, BTreeSet};

/// A tracked group of spawned units.
#[derive(Clone, Debug, Default)]
pub struct Wave {
    pub expected: usize,
    pub members: BTreeSet<EntityRef>,
    pub complete: bool,
}

#[derive(Clone, Debug, Default)]
pub struct LevelBindings {
    /// The scene runtime the indices below belong to.
    scene: String,
    level: String,
    /// Defined native actions by (name, arguments): define once, invoke
    /// many times.
    actions: BTreeMap<(String, Vec<(String, String)>), usize>,
    /// Script condition definitions; the index is redefined per scene.
    conditions: BTreeMap<String, (ScriptCall, Option<usize>)>,
    pub areas: BTreeMap<String, ([f32; 3], f32, BTreeSet<EntityRef>)>,
    pub waves: BTreeMap<String, Wave>,
    alive: BTreeMap<EntityRef, bool>,
    checkpoint: u64,
}

pub fn call(name: &str, arguments: &[(String, String)]) -> ScriptCall {
    arguments.iter().fold(ScriptCall::new(name), |c, (k, v)| {
        c.arg(k.as_str(), v.as_str())
    })
}

/// What happened in the level this tick, for the script callbacks.
#[derive(Clone, Debug, PartialEq)]
pub enum LevelEvent {
    TriggerEnter { area: String, entity: EntityRef },
    NpcKilled { entity: EntityRef, name: String },
    Checkpoint { generation: u64 },
    WaveStart { wave: String },
    WaveComplete { wave: String },
}

impl LevelBindings {
    /// A new scene runtime invalidates native action/condition indices.
    pub fn sync_scene(&mut self, session: &PlaySession) {
        if self.scene != session.scene_name() || self.level != session.level_file() {
            self.scene = session.scene_name().to_string();
            self.level = session.level_file().to_string();
            self.actions.clear();
            for (_, index) in self.conditions.values_mut() {
                *index = None;
            }
        }
    }

    /// Run a native action (defined once per scene, then invoked).
    pub fn run_action(
        &mut self,
        session: &mut PlaySession,
        name: &str,
        arguments: &[(String, String)],
    ) -> Result<(), String> {
        let key = (name.to_ascii_lowercase(), arguments.to_vec());
        let index = match self.actions.get(&key) {
            Some(i) => *i,
            None => {
                let i = session.define_script_action(&call(name, arguments))?;
                self.actions.insert(key, i);
                i
            }
        };
        session.invoke_script_action(index, None).map(drop)
    }

    pub fn define_condition(&mut self, id: String, name: &str, arguments: &[(String, String)]) {
        self.conditions.insert(id, (call(name, arguments), None));
    }

    /// Evaluate every defined condition against the live scene.
    pub fn conditions(&mut self, session: &mut PlaySession) -> BTreeMap<String, bool> {
        let mut out = BTreeMap::new();
        for (id, (call, index)) in &mut self.conditions {
            if index.is_none() {
                *index = session.define_script_condition(call).ok();
            }
            if let Some(i) = *index {
                if let Ok(met) = session.script_condition_met(i, None) {
                    out.insert(id.clone(), met);
                }
            }
        }
        out
    }

    /// Derive level callbacks from this tick's authoritative snapshot.
    pub fn observe(&mut self, session: &PlaySession, snapshot: &WorldSnapshot) -> Vec<LevelEvent> {
        let mut events = Vec::new();
        for (name, (center, radius, inside)) in &mut self.areas {
            let now: BTreeSet<EntityRef> = snapshot
                .entities
                .iter()
                .filter(|e| e.player.is_some() && e.alive)
                .filter(|e| {
                    let d = (0..3)
                        .map(|i| (e.position[i] - center[i]).powi(2))
                        .sum::<f32>();
                    d <= *radius * *radius
                })
                .filter_map(|e| e.entity)
                .collect();
            for entity in now.difference(inside) {
                events.push(LevelEvent::TriggerEnter {
                    area: name.clone(),
                    entity: *entity,
                });
            }
            *inside = now;
        }
        for e in &snapshot.entities {
            let (Some(entity), None) = (e.entity, e.player) else {
                continue;
            };
            let was = self.alive.insert(entity, e.alive);
            if was == Some(true) && !e.alive {
                events.push(LevelEvent::NpcKilled {
                    entity,
                    name: e.name.clone(),
                });
            }
        }
        let live: BTreeSet<EntityRef> = snapshot.entities.iter().filter_map(|e| e.entity).collect();
        self.alive.retain(|e, _| live.contains(e));
        for (name, wave) in &mut self.waves {
            if wave.complete || wave.members.len() < wave.expected {
                continue;
            }
            let all_dead = wave.members.iter().all(|m| {
                snapshot
                    .entities
                    .iter()
                    .find(|e| e.entity == Some(*m))
                    .is_none_or(|e| !e.alive)
            });
            if all_dead {
                wave.complete = true;
                events.push(LevelEvent::WaveComplete { wave: name.clone() });
            }
        }
        if session.checkpoint_generation != self.checkpoint {
            self.checkpoint = session.checkpoint_generation;
            events.push(LevelEvent::Checkpoint {
                generation: self.checkpoint,
            });
        }
        events
    }
}
