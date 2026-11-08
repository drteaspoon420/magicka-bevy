//! Recovered SpawnItem action lifecycle. The world supplies live item hosting.
//! See research/pickups.md; Nr is stored natively but Execute spawns one item.
use crate::{
    scene_loading::custom_hash,
    script_attributes::{boolean, number, ScriptReadError},
};

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnItemAction {
    pub delay: f32,
    pub area: i32,
    pub item: Option<String>,
    pub item_hash: i32,
    pub on_pickup: i32,
    pub unique_id: Option<i32>,
    pub nr: i32,
    pub physics_enabled: bool,
    pub ignore_tractor_pull: bool,
    pub attach_to: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpawnItemState {
    pub placeholder: Option<u16>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnItemMessage {
    pub handle: u16,
    pub template: i32,
    pub position: [f32; 3],
    pub orientation: [f32; 16],
    pub physics_enabled: bool,
    pub ignore_tractor_pull: bool,
    pub on_pickup: i32,
    pub attach_to: i32,
}

pub trait SpawnItemHost {
    type Error;
    fn is_client(&self) -> bool;
    fn is_server(&self) -> bool;
    fn new_placeholder(&mut self) -> Result<u16, Self::Error>;
    fn load_item(&mut self, path: &str) -> Result<(), Self::Error>;
    fn locator(&mut self, area: i32) -> Result<[f32; 16], Self::Error>;
    /// GetCachedWeapon then OnPickup assignment and Detach.
    fn copy_and_detach(
        &mut self,
        handle: u16,
        template: i32,
        on_pickup: i32,
    ) -> Result<(), Self::Error>;
    fn move_to(
        &mut self,
        handle: u16,
        position: [f32; 3],
        orientation: [f32; 16],
    ) -> Result<(), Self::Error>;
    fn add_entity(&mut self, handle: u16) -> Result<(), Self::Error>;
    fn configure(
        &mut self,
        handle: u16,
        immovable: bool,
        ignore_tractor_pull: bool,
        unique_id: Option<i32>,
    ) -> Result<(), Self::Error>;
    fn enable_body(&mut self, handle: u16) -> Result<(), Self::Error>;
    /// Only attach if this animated part exists, as in native TryGetValue.
    fn attach_if_present(&mut self, handle: u16, part: i32) -> Result<(), Self::Error>;
    fn send_spawn(&mut self, message: SpawnItemMessage) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum SpawnItemError<E> {
    MissingPlaceholder,
    MissingItem,
    Host(E),
}
impl<E> From<E> for SpawnItemError<E> {
    fn from(e: E) -> Self {
        Self::Host(e)
    }
}

fn hash(value: &str) -> Result<i32, ScriptReadError> {
    if !value.is_ascii() {
        return Err(ScriptReadError::UnsupportedName(value.into()));
    }
    // Action.Read lowercases string property values before invoking setters.
    Ok(custom_hash(&value.to_ascii_lowercase()))
}

impl SpawnItemAction {
    pub fn read<'a>(
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        let mut a = Self {
            delay: 0.0,
            area: 0,
            item: None,
            item_hash: 0,
            on_pickup: 0,
            unique_id: None,
            nr: 1,
            physics_enabled: false,
            ignore_tractor_pull: false,
            attach_to: 0,
        };
        for (key, value) in attrs {
            match key.to_ascii_lowercase().as_str() {
                "delay" => a.delay = number(value, key)?,
                "area" => a.area = hash(value)?,
                "item" => {
                    a.item_hash = hash(value)?;
                    a.item = Some(value.to_ascii_lowercase());
                }
                "onpickup" => a.on_pickup = hash(value)?,
                "id" => {
                    a.unique_id = if value.is_empty() {
                        None
                    } else {
                        Some(hash(value)?)
                    }
                }
                "nr" => a.nr = number(value, key)?,
                "physicsenabled" => a.physics_enabled = boolean(value, key)?,
                "ignoretractorpull" => a.ignore_tractor_pull = boolean(value, key)?,
                "attachto" => a.attach_to = hash(value)?,
                _ => {}
            }
        }
        Ok(a)
    }
    pub fn initialize<H: SpawnItemHost + ?Sized>(
        &self,
        state: &mut SpawnItemState,
        host: &mut H,
    ) -> Result<(), SpawnItemError<H::Error>> {
        state.placeholder = Some(host.new_placeholder()?);
        let name = self.item.as_deref().ok_or(SpawnItemError::MissingItem)?;
        host.load_item(&format!("Data/Items/Wizard/{name}"))?;
        Ok(())
    }
    pub fn execute<H: SpawnItemHost + ?Sized>(
        &self,
        state: &SpawnItemState,
        host: &mut H,
    ) -> Result<(), SpawnItemError<H::Error>> {
        if host.is_client() {
            return Ok(());
        }
        let handle = state
            .placeholder
            .ok_or(SpawnItemError::MissingPlaceholder)?;
        let mut orientation = host.locator(self.area)?;
        let position = [orientation[12], orientation[13], orientation[14]];
        orientation[12..15].fill(0.0);
        host.copy_and_detach(handle, self.item_hash, self.on_pickup)?;
        host.move_to(handle, position, orientation)?;
        host.add_entity(handle)?;
        host.configure(
            handle,
            !self.physics_enabled,
            self.ignore_tractor_pull,
            self.unique_id,
        )?;
        host.enable_body(handle)?;
        if self.attach_to != 0 {
            host.attach_if_present(handle, self.attach_to)?;
        }
        if host.is_server() {
            host.send_spawn(SpawnItemMessage {
                handle,
                template: self.item_hash,
                position,
                orientation,
                physics_enabled: self.physics_enabled,
                ignore_tractor_pull: self.ignore_tractor_pull,
                on_pickup: self.on_pickup,
                attach_to: self.attach_to,
            })?;
        }
        Ok(())
    }
    /// QuickExecute is empty in the native action.
    pub fn quick_execute(&self) {}
}
