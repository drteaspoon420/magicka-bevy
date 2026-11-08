//! Data crossing the Lua ↔ engine boundary. Lua never touches the ECS: it
//! reads a [`WorldSnapshot`] and emits [`Command`]s that the runtime
//! validates and applies at a fixed simulation phase.
use magicka_definitions::DefinitionId;
use serde::{Deserialize, Serialize};

/// A generation-checked entity reference. A handle whose generation no
/// longer matches the live entity is stale and resolves to nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntityRef {
    pub handle: u16,
    pub generation: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityInfo {
    pub entity: Option<EntityRef>,
    /// Template or unit name.
    pub name: String,
    pub position: [f32; 3],
    pub facing: [f32; 3],
    pub health: f32,
    pub max_health: f32,
    pub alive: bool,
    /// Player slot for wizards.
    pub player: Option<usize>,
    pub faction: i32,
    /// Active status effects (lowercase names, e.g. "wet", "burning").
    #[serde(default)]
    pub statuses: Vec<String>,
    #[serde(default)]
    pub ethereal: bool,
}

/// A held item's identity and its numbers before upgrades.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemInfo {
    /// Equipment slot (0 weapon, 1 staff, …).
    pub slot: i32,
    /// Retail item template name.
    pub name: String,
    /// Schema numbers of the item before upgrade overlays.
    pub numbers: std::collections::BTreeMap<String, f64>,
}

/// A live upgrade overlay (per player, per item or ability).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Upgrade {
    pub id: String,
    pub owner: String,
    pub player: usize,
    /// Retail item name or definition ID (lowercase).
    pub target: String,
    pub key: String,
    pub change: magicka_definitions::schema::Change,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub slot: usize,
    pub name: String,
    pub profile_id: Option<String>,
    pub entity: Option<EntityRef>,
    /// Registry Magick IDs this player has unlocked.
    pub magicks: Vec<String>,
    #[serde(default)]
    pub items: Vec<ItemInfo>,
}

/// What scripts may read this tick. Built by the runtime adapter.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub tick: u64,
    pub time: f64,
    pub level: String,
    pub scene: String,
    pub players: Vec<PlayerInfo>,
    pub entities: Vec<EntityInfo>,
    /// Native level conditions scripts defined, by ID, evaluated against
    /// the live scene this tick (`Levels.ConditionMet`).
    #[serde(default)]
    pub conditions: std::collections::BTreeMap<String, bool>,
    /// Live upgrade overlays.
    #[serde(default)]
    pub upgrades: Vec<Upgrade>,
    /// The scene is indoors (weather and sky Magicks are refused).
    #[serde(default)]
    pub indoors: bool,
}
impl WorldSnapshot {
    pub fn entity(&self, e: EntityRef) -> Option<&EntityInfo> {
        self.entities.iter().find(|i| i.entity == Some(e))
    }
}

/// An authoritative change requested by a script.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    Damage {
        target: EntityRef,
        amount: f32,
        element: Option<String>,
        source: Option<EntityRef>,
    },
    Heal {
        target: EntityRef,
        amount: f32,
    },
    Teleport {
        target: EntityRef,
        position: [f32; 3],
    },
    Spawn {
        unit: String,
        position: [f32; 3],
        /// Script tag returned with the `entity_spawned` event.
        tag: String,
    },
    Remove {
        target: EntityRef,
    },
    /// The normal elemental spell path (plan F4), distinct from Magicks.
    CastElements {
        caster: EntityRef,
        elements: Vec<String>,
        delivery: String,
        target_position: Option<[f32; 3]>,
    },
    InvokeMagick {
        caster: EntityRef,
        magick: DefinitionId,
        target_position: Option<[f32; 3]>,
    },
    UnlockMagick {
        player: usize,
        magick: DefinitionId,
    },
    /// Presentation only; clients simulate it locally.
    Particle {
        effect: String,
        position: Option<[f32; 3]>,
        owner: Option<EntityRef>,
    },
    Sound {
        sound: String,
        position: Option<[f32; 3]>,
        owner: Option<EntityRef>,
    },
    Message {
        text: String,
        player: Option<usize>,
    },
    /// A typed native level action (`script_schema::ScriptCall`).
    LevelAction {
        name: String,
        arguments: Vec<(String, String)>,
    },
    /// Define a native level condition, evaluated every tick.
    DefineCondition {
        id: String,
        name: String,
        arguments: Vec<(String, String)>,
    },
    /// Add a native `<Trigger>` (scene XML) to the current scene.
    AddTrigger {
        xml: String,
    },
    /// Report players entering a sphere as `OnTriggerEnter`.
    WatchArea {
        name: String,
        position: [f32; 3],
        radius: f32,
    },
    /// Add a per-player upgrade overlay on a schema-upgradable number.
    Upgrade(Upgrade),
    RemoveUpgrade {
        id: String,
    },
    /// A world operation (`world_ops::WORLD_OPS`): its qualified name and
    /// the Lua arguments as JSON (entities as `{__entity, generation}`).
    World {
        op: String,
        args: Vec<serde_json::Value>,
    },
    /// Spawn a tracked group: `OnWaveStart`, then `OnWaveComplete` when
    /// every member is dead.
    SpawnWave {
        name: String,
        units: Vec<(String, [f32; 3])>,
    },
}

/// A command and the mod that issued it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Issued {
    pub owner: String,
    pub command: Command,
}

/// Lifecycle callbacks, in the order the engine calls them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    ModLoad,
    MatchSetup,
    GameStart,
    LevelStart,
    PlayerJoin,
    LevelEnd,
    MatchEnd,
    ModUnload,
}
impl Lifecycle {
    pub fn function(self) -> &'static str {
        match self {
            Self::ModLoad => "OnModLoad",
            Self::MatchSetup => "OnMatchSetup",
            Self::GameStart => "OnGameStart",
            Self::LevelStart => "OnLevelStart",
            Self::PlayerJoin => "OnPlayerJoin",
            Self::LevelEnd => "OnLevelEnd",
            Self::MatchEnd => "OnMatchEnd",
            Self::ModUnload => "OnModUnload",
        }
    }
}

/// A filter's verdict on a pending engine action.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Allow,
    Deny,
    /// The action with these fields changed (only the filter's mutable
    /// fields; anything else is ignored).
    Modified(serde_json::Map<String, serde_json::Value>),
}

/// What a failing filter callback means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnError {
    /// Ignore the failing filter (cosmetic or tunable actions).
    FailOpen,
    /// Deny the action (purchases, admissions, anything exploitable).
    FailClosed,
}

/// A pre-action filter the engine offers to scripts.
#[derive(Clone, Copy, Debug)]
pub struct FilterSpec {
    pub name: &'static str,
    pub mutable: &'static [&'static str],
    pub on_error: OnError,
}

/// The engine's built-in filters.
pub const FILTERS: &[FilterSpec] = &[
    FilterSpec {
        name: "damage",
        mutable: &["amount"],
        on_error: OnError::FailOpen,
    },
    FilterSpec {
        name: "heal",
        mutable: &["amount"],
        on_error: OnError::FailOpen,
    },
    FilterSpec {
        name: "cast",
        mutable: &[],
        on_error: OnError::FailOpen,
    },
    FilterSpec {
        name: "pickup",
        mutable: &[],
        on_error: OnError::FailClosed,
    },
    FilterSpec {
        name: "currency",
        mutable: &["amount"],
        on_error: OnError::FailClosed,
    },
    FilterSpec {
        name: "purchase",
        mutable: &["price"],
        on_error: OnError::FailClosed,
    },
];

/// A script failure, with enough context to find it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptError {
    pub owner: String,
    pub callback: String,
    pub entity: Option<EntityRef>,
    /// Lua message and traceback.
    pub message: String,
}
impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.owner, self.callback)?;
        if let Some(e) = self.entity {
            write!(f, " (entity {}#{})", e.handle, e.generation)?;
        }
        write!(f, ": {}", self.message)
    }
}

/// Resource bounds per match VM. Modern policy, not retail values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub memory_bytes: usize,
    /// Lua VM instructions per callback invocation.
    pub instructions: u64,
    pub timers_per_mod: usize,
    pub subscriptions_per_mod: usize,
    pub commands_per_tick: usize,
    pub spawns_per_tick: usize,
    pub net_table_bytes_per_mod: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: 64 << 20,
            instructions: 5_000_000,
            timers_per_mod: 1024,
            subscriptions_per_mod: 1024,
            commands_per_tick: 4096,
            spawns_per_tick: 64,
            net_table_bytes_per_mod: 256 << 10,
        }
    }
}
