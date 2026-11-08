//! Grease.GreaseField and the Grease magick's spray sweep (research/grease.md).
use crate::spells::{inclusive_opposites, Elements};

/// GreaseField.GREASEFIELD_TTL, BURN_RATE and GREASE_RADIUS.
pub const FIELD_TTL: f32 = 60.0;
pub const BURN_RATE: f32 = 3.0;
pub const FIELD_RADIUS: f32 = 1.5;
/// Grease.GREASESPRAY_TTL and NR_OF_FIELDS: six fields over 0.5 s.
pub const SPRAY_TTL: f32 = 0.5;
pub const SPRAY_FIELDS: f32 = 6.0;
/// The spray's reach before shields/scene clip it, and the field inset.
pub const SPRAY_RANGE: f32 = 6.0;
/// GreaseSplash.NUM_OF_FIELDS and RANGE.
pub const SPLASH_FIELDS: usize = 7;
pub const SPLASH_RANGE: f32 = 3.0;

/// One GreaseField's simulation state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GreaseField {
    pub ttl: f32,
    pub temperature: f32,
    pub burning: bool,
}

/// What a damage or burn did to the field's fire state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireChange {
    None,
    Ignited,
    Extinguished,
}

impl Default for GreaseField {
    fn default() -> Self {
        Self {
            ttl: FIELD_TTL,
            temperature: 0.0,
            burning: false,
        }
    }
}

impl GreaseField {
    /// GreaseField.InternalDamage for one Damage: Fire heats by the
    /// magnitude (igniting at 1); an element inclusively opposite to Fire
    /// (Water, Cold) cools it (extinguishing at 0). Damage is never taken.
    pub fn damage(&mut self, element: Elements, magnitude: f32) -> FireChange {
        if element.0 & Elements::FIRE.0 == Elements::FIRE.0 {
            self.temperature = (self.temperature + magnitude).min(1.0);
            if !self.burning && self.temperature >= 1.0 {
                self.burning = true;
                return FireChange::Ignited;
            }
        } else if inclusive_opposites(element, Elements::FIRE) {
            self.temperature = (self.temperature - magnitude).max(0.0);
            if self.burning && self.temperature <= 0.0 {
                self.burning = false;
                return FireChange::Extinguished;
            }
        }
        FireChange::None
    }

    /// GreaseField.Burn: like Fire damage, but on ignition the temperature
    /// is set to the given value (not left at 1).
    pub fn burn(&mut self, temperature: f32) -> FireChange {
        self.temperature = (self.temperature + temperature).min(1.0);
        if !self.burning && self.temperature >= 1.0 {
            self.burning = true;
            self.temperature = temperature;
            return FireChange::Ignited;
        }
        FireChange::None
    }

    /// GreaseField.Update's timers: burning raises the temperature by dt/2
    /// (max 1) and burns the TTL three times as fast. Returns whether the
    /// field is dead (TTL < 0) afterwards. The host then damages everything
    /// within FIELD_RADIUS once per hit-list period while burning.
    pub fn update(&mut self, dt: f32) -> bool {
        if self.burning {
            self.temperature = (self.temperature + dt * 0.5).min(1.0);
            self.ttl -= dt * BURN_RATE;
        } else {
            self.ttl -= dt;
        }
        self.ttl < 0.0
    }

    /// GreaseField.GetDanger for AI avoidance.
    pub fn danger(&self) -> f32 {
        if self.burning {
            self.temperature * 10.0
        } else {
            0.5
        }
    }
}

/// Grease.Update's sweep yaw for a spray step: -63° at the start (step 6),
/// +45° at step 0, centred at step 2.5. Fields spawn at whole steps 5..0
/// (-45°..+45°).
pub fn spray_yaw(step: f32) -> f32 {
    -(step - 2.5) / 2.5 * std::f32::consts::FRAC_PI_4
}

/// The spray step (TTL / 0.5 · 6) for a TTL.
pub fn spray_step(ttl: f32) -> f32 {
    ttl / SPRAY_TTL * SPRAY_FIELDS
}

/// Grease.Update: whether this frame (TTL `before` -> `after`) crosses into
/// a new whole step, and that step. A field spawns at it when the clipped
/// reach minus the field inset is positive.
pub fn spray_spawn_step(before: f32, after: f32) -> Option<f32> {
    let old = spray_step(before).floor();
    let new = spray_step(after).floor();
    (new < old && new >= 0.0).then_some(new)
}

/// GreaseSplash's yaw for field `i` (cumulative: each field turns the
/// running direction by a further PI/7·i, as the original reuses it).
pub fn splash_yaw(i: usize) -> f32 {
    std::f32::consts::PI / SPLASH_FIELDS as f32 * i as f32
}

/// Grease.GreaseField.InitializeCache(72): the fixed field pool.
pub const POOL_SIZE: usize = 72;

/// A placed field: where it is, who made it and its fire state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedField {
    pub owner: Option<u16>,
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// mTimeStamp: the owner's PlayTime at Initialize.
    pub timestamp: f64,
    pub field: GreaseField,
    /// Bumped on every (re)initialisation, so presentation can tell a
    /// recycled slot from the field it replaced.
    pub serial: u32,
}

/// GreaseField's static cache. GetInstance takes the least recently
/// acquired field (index 0) and moves it to the back, so the 73rd field
/// reinitialises the oldest one even while it is alive.
#[derive(Clone, Debug)]
pub struct GreasePool {
    slots: Vec<Option<PlacedField>>,
    order: Vec<usize>,
    serial: u32,
}
impl Default for GreasePool {
    fn default() -> Self {
        Self {
            slots: vec![None; POOL_SIZE],
            order: (0..POOL_SIZE).collect(),
            serial: 0,
        }
    }
}
impl GreasePool {
    /// GetInstance + Initialize: returns the slot used.
    pub fn spawn(
        &mut self,
        owner: Option<u16>,
        position: [f32; 3],
        normal: [f32; 3],
        timestamp: f64,
    ) -> usize {
        let slot = self.order.remove(0);
        self.order.push(slot);
        self.serial = self.serial.wrapping_add(1);
        self.slots[slot] = Some(PlacedField {
            owner,
            position,
            normal,
            timestamp,
            field: GreaseField::default(),
            serial: self.serial,
        });
        slot
    }
    pub fn get(&self, slot: usize) -> Option<&PlacedField> {
        self.slots.get(slot)?.as_ref()
    }
    pub fn get_mut(&mut self, slot: usize) -> Option<&mut PlacedField> {
        self.slots.get_mut(slot)?.as_mut()
    }
    /// Removable once dead (EntityManager drops it).
    pub fn remove(&mut self, slot: usize) {
        if let Some(s) = self.slots.get_mut(slot) {
            *s = None;
        }
    }
    pub fn clear(&mut self) {
        self.slots.iter_mut().for_each(|s| *s = None);
    }
    pub fn iter(&self) -> impl Iterator<Item = (usize, &PlacedField)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| (i, s)))
    }
    pub fn len(&self) -> usize {
        self.iter().count()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
