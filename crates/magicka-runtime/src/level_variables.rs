//! Level counters and played/ordinary timer storage, independent of presentation.
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelTimer {
    pub id: i32,
    pub time: f32,
    pub paused: bool,
}
#[derive(Clone, Debug, Default)]
pub struct LevelVariables {
    pub counters: BTreeMap<i32, i32>,
    pub played_timers: Vec<LevelTimer>,
    pub timers: Vec<LevelTimer>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct TimerFrame {
    pub paused: bool,
    pub game_ended: bool,
    pub cutscene: bool,
}
impl LevelVariables {
    pub fn counter(&self, id: i32) -> i32 {
        *self.counters.get(&id).unwrap_or(&0)
    }
    pub fn set_counter(&mut self, id: i32, value: i32) {
        self.counters.insert(id, value);
    }
    pub fn add_counter(&mut self, id: i32, value: i32) {
        self.set_counter(id, self.counter(id).wrapping_add(value));
    }
    pub fn add_timer(&mut self, id: i32, played: bool, value: f32) {
        let timers = if played {
            &mut self.played_timers
        } else {
            &mut self.timers
        };
        if !timers.iter().any(|t| t.id == id) {
            timers.push(LevelTimer {
                id,
                time: value,
                paused: false,
            });
        }
    }
    fn timer_mut(&mut self, id: i32) -> Option<&mut LevelTimer> {
        self.played_timers
            .iter_mut()
            .chain(self.timers.iter_mut())
            .find(|t| t.id == id)
    }
    pub fn set_timer(&mut self, id: i32, value: f32) {
        if let Some(t) = self.timer_mut(id) {
            t.time = value;
        }
    }
    pub fn pause_timer(&mut self, id: i32, paused: bool) {
        if let Some(t) = self.timer_mut(id) {
            t.paused = paused;
        }
    }
    pub fn timer(&self, id: i32) -> f32 {
        self.played_timers
            .iter()
            .chain(self.timers.iter())
            .find(|t| t.id == id)
            .map_or(0.0, |t| t.time)
    }
    /// Level.Update's timer phase, before CurrentScene.Update. Caller preserves
    /// the original invocation frequency and supplies already-selected dt.
    pub fn update(&mut self, dt: f32, frame: TimerFrame) {
        if !frame.paused && !frame.game_ended && !frame.cutscene {
            for t in &mut self.played_timers {
                if !t.paused {
                    t.time += dt;
                }
            }
        }
        for t in &mut self.timers {
            if !t.paused {
                t.time += dt;
            }
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum VariableAction {
    Counter {
        id: i32,
        operation: i8,
        value: i32,
    },
    AddTimer {
        id: i32,
        played: bool,
        value: f32,
    },
    Timer {
        id: i32,
        value: Option<f32>,
        paused: Option<bool>,
    },
}
impl VariableAction {
    pub fn execute(self, variables: &mut LevelVariables) {
        match self {
            Self::Counter {
                id,
                operation,
                value,
            } => match operation {
                0 => variables.add_counter(id, value),
                1 => variables.add_counter(id, value.wrapping_neg()),
                2 => variables.set_counter(id, value),
                _ => {}
            },
            Self::AddTimer { id, played, value } => variables.add_timer(id, played, value),
            Self::Timer { id, value, paused } => {
                if let Some(value) = value {
                    variables.set_timer(id, value);
                }
                if let Some(paused) = paused {
                    variables.pause_timer(id, paused);
                }
            }
        }
    }
    /// Counter.QuickExecute is empty; Timer/AddTimer execute normally.
    pub fn quick_execute(self, variables: &mut LevelVariables) {
        if !matches!(self, Self::Counter { .. }) {
            self.execute(variables);
        }
    }
}
