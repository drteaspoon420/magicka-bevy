//! SpawnElemental's retained handles, including its distinct checkpoint replay.
use crate::script_attributes::{boolean, identifier, number, ScriptReadError};

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnElementalAction {
    pub delay: f32,
    pub area: i32,
    pub unique_id: i32,
    pub nr: i32,
    pub snap_to_navmesh: bool,
    pub proximity: f32,
}

#[derive(Clone, Debug, Default)]
pub struct SpawnElementalState {
    pub handles: Vec<u16>,
}

pub trait SpawnElementalHost {
    type Error;
    fn is_client(&self) -> bool;
    fn spawn_elemental(
        &mut self,
        action: &SpawnElementalAction,
        // QuickExecute does not set proximity on the cached instance.
        proximity: Option<f32>,
    ) -> Result<u16, Self::Error>;
    fn elemental_dead(&self, handle: u16) -> bool;
}

impl SpawnElementalAction {
    pub fn read<'a>(
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        let mut a = Self {
            delay: 0.,
            area: 0,
            unique_id: 0,
            nr: 0,
            snap_to_navmesh: false,
            proximity: 0.,
        };
        for (key, value) in attrs {
            match key.to_ascii_lowercase().as_str() {
                "delay" => a.delay = number(value, key)?,
                "area" => a.area = identifier(value)?,
                "id" => a.unique_id = identifier(value)?,
                "nr" => a.nr = number(value, key)?,
                "snaptonavmesh" => a.snap_to_navmesh = boolean(value, key)?,
                "proximity" => a.proximity = number(value, key)?,
                _ => return Err(ScriptReadError::InvalidAttribute(key.into())),
            }
        }
        Ok(a)
    }

    pub fn execute<H: SpawnElementalHost + ?Sized>(
        &self,
        state: &mut SpawnElementalState,
        host: &mut H,
    ) -> Result<(), H::Error> {
        if !host.is_client() {
            for _ in 0..self.nr {
                state
                    .handles
                    .push(host.spawn_elemental(self, Some(self.proximity))?);
            }
        }
        Ok(())
    }

    pub fn quick_execute<H: SpawnElementalHost + ?Sized>(
        &self,
        state: &mut SpawnElementalState,
        host: &mut H,
    ) -> Result<(), H::Error> {
        if !host.is_client() {
            for handle in &mut state.handles {
                *handle = host.spawn_elemental(self, None)?;
            }
        }
        Ok(())
    }

    pub fn track<H: SpawnElementalHost + ?Sized>(state: &mut SpawnElementalState, host: &H) {
        state.handles.retain(|&handle| !host.elemental_dead(handle));
    }
}
