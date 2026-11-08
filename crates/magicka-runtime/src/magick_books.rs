//! BookOfMagick state and SpawnMagick, with presentation/physics supplied by hosts.
use crate::script_attributes::{boolean, identifier, number, ScriptReadError};
use magicka_sim::damage_delivery::NetworkRole;

pub const BOOK_MODEL: &str = "Models/Items_Wizard/magickbook_major";
pub const BOOK_MAGICKS: &[&str] = &[
    "none",
    "revive",
    "grease",
    "haste",
    "invisibility",
    "teleport",
    "fear",
    "charm",
    "thunderb",
    "rain",
    "tornado",
    "blizzard",
    "meteors",
    "conflagration",
    "thunders",
    "timewarp",
    "vortex",
    "sundead",
    "selemental",
    "sdeath",
    "sphoenix",
    "nullify",
    "corporealize",
    "ctd",
    "napalm",
    "portal",
    "tractorpull",
    "proppmagick",
    "levitate",
    "chainlightning",
    "confuse",
    "wave",
    "performanceenchantment",
    "judgementspray",
    "amalgameddon",
    // Rewrite addition: the semi-custom pool (research/custom-magicks.md),
    // indexed by MagickType after the unused sentinel 35 (empty, never
    // spawnable). Native scripts only name the 35 entries above.
    "",
    "grow",
    "shrink",
    "etherealize",
    "polymorph",
    "earthquake",
    "votal",
    "knockdownchance",
    "summoncross",
    "vladzap",
    "spawnslime",
    "slimeoverkill",
    "greasesplash",
    "etherealclone",
    "inflictbleeding",
    "inflictpoison",
];

/// The MagickType sentinel between the native and semi-custom books.
pub const SENTINEL: u8 = 35;

pub fn magick_id(value: &str) -> Option<u8> {
    if value.trim().is_empty() {
        return None;
    }
    BOOK_MAGICKS
        .iter()
        .position(|name| name.eq_ignore_ascii_case(value.trim()))
        .map(|v| v as u8)
        .or_else(|| {
            value
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|v| (*v as usize) < BOOK_MAGICKS.len() && *v != SENTINEL)
        })
}

#[derive(Clone, Debug)]
pub struct Book {
    pub magick: u8,
    pub transform: [f32; 16],
    pub immovable: bool,
    pub body_enabled: bool,
    pub velocity: [f32; 3],
    pub timeout: f32,
    pub resting_timer: f32,
    pub on_pickup: i32,
    pub dead: bool,
    /// Cache reuse invalidates presentation references for the prior lifetime.
    pub generation: u64,
}
impl Book {
    pub fn resting(&self) -> bool {
        self.resting_timer < 0.0
    }
    pub fn permanent(&self) -> bool {
        self.timeout <= 0.0 && !self.dead
    }
    pub fn kill(&mut self) {
        self.dead = true;
        self.timeout = 0.0;
    }
    /// Native Update: timeout <=0 at entry is indefinite, not already expired.
    pub fn update(
        &mut self,
        dt: f32,
        body_active: bool,
        client: bool,
        timeout_effect_active: bool,
    ) -> BookUpdate {
        let mut events = BookUpdate::default();
        if self.dead {
            return events;
        }
        if self.timeout > 0.0 {
            self.timeout -= dt;
            if self.timeout <= 0.0 {
                events.disappear = true;
                if !client {
                    self.kill();
                }
            } else if !timeout_effect_active && self.timeout <= 4.0 {
                events.start_timeout = true;
            }
        }
        if body_active {
            self.resting_timer = 1.0;
        } else {
            self.resting_timer -= dt;
        }
        events
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BookUpdate {
    pub start_timeout: bool,
    pub disappear: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnMagickAction {
    pub delay: f32,
    pub area: i32,
    pub magick: u8,
    pub immovable: bool,
    pub timeout: f32,
    pub unique_id: i32,
    pub on_pickup: i32,
}
pub trait SpawnMagickHost {
    type Error;
    fn role(&self) -> NetworkRole;
    fn locator(&mut self, area: i32) -> Result<[f32; 16], Self::Error>;
    fn spawn_book(
        &mut self,
        action: &SpawnMagickAction,
        transform: [f32; 16],
    ) -> Result<u16, Self::Error>;
    fn send_book_spawn(
        &mut self,
        handle: u16,
        action: &SpawnMagickAction,
        transform: [f32; 16],
    ) -> Result<(), Self::Error>;
}
impl SpawnMagickAction {
    pub fn read<'a>(
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        let mut a = Self {
            delay: 0.0,
            area: 0,
            magick: 0,
            immovable: true,
            timeout: 0.0,
            unique_id: 0,
            on_pickup: 0,
        };
        for (key, value) in attrs {
            match key.to_ascii_lowercase().as_str() {
                "delay" => a.delay = number(value, key)?,
                "area" => a.area = identifier(value)?,
                "magick" => {
                    a.magick = magick_id(value)
                        .ok_or_else(|| ScriptReadError::InvalidAttribute(key.into()))?
                }
                "immovable" => a.immovable = boolean(value, key)?,
                "timeout" => a.timeout = number(value, key)?,
                "id" => a.unique_id = identifier(value)?,
                "onpickup" => a.on_pickup = identifier(value)?,
                _ => {}
            }
        }
        Ok(a)
    }
    pub fn execute<H: SpawnMagickHost + ?Sized>(&self, host: &mut H) -> Result<(), H::Error> {
        if host.role() == NetworkRole::Client {
            return Ok(());
        }
        let transform = host.locator(self.area)?;
        let handle = host.spawn_book(self, transform)?;
        if host.role() == NetworkRole::Server {
            host.send_book_spawn(handle, self, transform)?;
        }
        Ok(())
    }
}
