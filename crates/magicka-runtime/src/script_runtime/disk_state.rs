//! Versioned state only: definitions and initialized content remain in the scene.
use super::*;
use serde_json::{json, Value};

impl ScriptRuntime {
    fn disk_signature(&self) -> String {
        let mut text = format!("{:?}{:?}", self.layout.actions, self.layout.conditions);
        for t in &self.layout.triggers {
            text.push_str(&format!(
                "{:?}",
                (t.id, t.repeat, t.autorun, &t.conditions, &t.actions)
            ));
        }
        let hash = text.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        });
        format!("{hash:016x}")
    }
    pub fn disk_state(&self) -> Value {
        let timers = |v: &[crate::level_variables::LevelTimer]| {
            v.iter()
                .map(|t| (t.id, t.time, t.paused))
                .collect::<Vec<_>>()
        };
        json!({"version":1,"signature":self.disk_signature(),
            "counters":self.variables.counters,"timers":timers(&self.variables.timers),"played_timers":timers(&self.variables.played_timers),
            "triggers":self.layout.triggers.iter().map(|t|(t.id,t.has_triggered,t.time_till_repeat)).collect::<Vec<_>>(),
            "queues":self.queues.iter().map(|q|q.map(|q|(q.state.countdown,q.state.pending))).collect::<Vec<_>>(),
            "history":self.history,"arguments":self.arguments,
            "spawns":self.spawns.iter().map(|s|s.spawned.iter().map(|s|(s.handle,s.health)).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "phys":self.phys.iter().map(|s|(s.health.handle,s.health.health)).collect::<Vec<_>>(),
            "elementals":self.elementals.iter().map(|s|s.handles.clone()).collect::<Vec<_>>(),"checkpoints":self.checkpoints})
    }
    /// Validate everything on a clone; a failed read never partly mutates scripts.
    pub fn restore_disk_state(&mut self, v: &Value) -> Result<(), String> {
        if v["version"] != 1 || v["signature"].as_str() != Some(&self.disk_signature()) {
            return Err("save script layout/version does not match installed content".into());
        }
        let mut next = self.clone();
        let field = |name: &str| {
            v.get(name)
                .cloned()
                .ok_or_else(|| format!("missing script state {name}"))
        };
        macro_rules! read {
            ($name:literal) => {
                serde_json::from_value(field($name)?).map_err(|e| format!("{}: {e}", $name))?
            };
        }
        next.variables.counters = read!("counters");
        for (name, output) in [
            ("timers", &mut next.variables.timers),
            ("played_timers", &mut next.variables.played_timers),
        ] {
            let timers: Vec<(i32, f32, bool)> =
                serde_json::from_value(field(name)?).map_err(|e| e.to_string())?;
            if timers.iter().any(|t| !t.1.is_finite()) {
                return Err("non-finite timer".into());
            }
            *output = timers
                .into_iter()
                .map(|(id, time, paused)| crate::level_variables::LevelTimer { id, time, paused })
                .collect();
        }
        let triggers: Vec<(i32, bool, f32)> = read!("triggers");
        let queues: Vec<Option<(f32, i32)>> = read!("queues");
        let spawns: Vec<Vec<(Option<u16>, f32)>> = read!("spawns");
        let phys: Vec<(i32, f32)> = read!("phys");
        let elementals: Vec<Vec<u16>> = read!("elementals");
        next.history = read!("history");
        next.arguments = read!("arguments");
        next.checkpoints = read!("checkpoints");
        let n = next.layout.actions.len();
        if triggers.len() != next.layout.triggers.len()
            || [
                queues.len(),
                spawns.len(),
                phys.len(),
                elementals.len(),
                next.arguments.len(),
                next.checkpoints.len(),
            ]
            .iter()
            .any(|len| *len != n)
            || next.history.iter().any(|i| *i >= n)
        {
            return Err("invalid script-state dimensions".into());
        }
        for (t, (id, triggered, time)) in next.layout.triggers.iter_mut().zip(triggers) {
            if id != t.id || !time.is_finite() {
                return Err("invalid trigger state".into());
            }
            t.has_triggered = triggered;
            t.time_till_repeat = time;
        }
        for (q, state) in next.queues.iter_mut().zip(queues) {
            match (q, state) {
                (Some(q), Some((countdown, pending))) if countdown.is_finite() && pending >= 0 => {
                    q.state = crate::action_queue::ActionQueueState { countdown, pending }
                }
                (None, None) => {}
                _ => return Err("invalid action queue".into()),
            }
        }
        for (s, records) in next.spawns.iter_mut().zip(spawns) {
            if records.iter().any(|r| !r.1.is_finite()) {
                return Err("invalid spawn health".into());
            }
            s.spawned = records
                .into_iter()
                .map(|(handle, health)| crate::spawn_action::SpawnedHealth { handle, health })
                .collect();
        }
        for (s, (handle, health)) in next.phys.iter_mut().zip(phys) {
            if !health.is_finite() {
                return Err("invalid prop health".into());
            }
            s.health = crate::spawn_phys::PhysHealth { handle, health };
        }
        for (s, handles) in next.elementals.iter_mut().zip(elementals) {
            s.handles = handles;
        }
        *self = next;
        Ok(())
    }
}
