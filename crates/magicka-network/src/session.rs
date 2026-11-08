//! Shared metadata and authoritative player snapshots for the demo session.
use crate::PhysicsFrame;
use magicka_sim::spells::{Elements, MagickType, Spell};
use serde::{Deserialize, Serialize};

/// Lossless queued spell state for the modern protocol, not the retail wire layout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueuedSpell {
    pub elements: u32,
    pub magnitudes: [f32; 11],
    pub magick: u8,
}
impl From<&Spell> for QueuedSpell {
    fn from(spell: &Spell) -> Self {
        Self {
            elements: spell.element.0,
            magnitudes: spell.mag,
            magick: spell.magick as u8,
        }
    }
}
impl QueuedSpell {
    pub fn spell(&self) -> Option<Spell> {
        let magick = MagickType::try_from(self.magick).ok()?;
        let mask = Elements::BY_INDEX.iter().fold(0, |bits, e| bits | e.0);
        if (self.elements != Elements::ALL.0
            && (self.elements & !mask != 0 || magick != MagickType::None))
            || self.magnitudes.iter().any(|m| !m.is_finite() || *m < 0.)
        {
            return None;
        }
        Some(Spell {
            element: Elements(self.elements),
            mag: self.magnitudes,
            magick,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerProfile {
    pub name: String,
    pub robe: String,
    pub colour: [f32; 3],
}
impl Default for PlayerProfile {
    fn default() -> Self {
        Self {
            name: "Wizard".into(),
            robe: "Wizard".into(),
            colour: [1.; 3],
        }
    }
}
pub fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
/// Largest robe tint component: well above the retail palette, and low enough
/// that a joiner cannot flood every other screen with bloom or overflow it.
pub const MAX_COLOUR_COMPONENT: f32 = 16.0;
/// Robe tints are linear RGB multipliers. The retail player palette includes
/// components above one (for example 363/255); they are not clamped sRGB.
pub fn valid_player_colour(colour: &[f32]) -> bool {
    colour.len() == 3
        && colour
            .iter()
            .all(|c| c.is_finite() && (0.0..=MAX_COLOUR_COMPONENT).contains(c))
}
/// A persistent profile identity (`saves::new_player_id`: 128 random bits).
pub fn valid_profile_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

impl PlayerProfile {
    pub fn valid(&self) -> bool {
        !self.name.trim().is_empty()
            && self.name.chars().count() <= 32
            && self.name.chars().all(crate::security::displayable)
            && identifier(&self.robe)
            && self.robe.to_ascii_lowercase().starts_with("wizard")
            && valid_player_colour(&self.colour)
    }
}
/// The profile a joining player selected before connecting (protocol 11).
/// The host validates it before any avatar is created; a queued spectator
/// keeps it until promotion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileSubmit {
    pub profile: PlayerProfile,
    /// Persistent local profile identity (parked saves): 128 random bits known
    /// to the player and the hosts they joined. Whoever knows it can claim that
    /// profile's parked player (`research/multiplayer-security.md`, MS-R3).
    pub profile_id: Option<String>,
}
impl ProfileSubmit {
    /// Why this profile cannot be admitted, if it cannot.
    pub fn problem(&self) -> Option<&'static str> {
        let p = &self.profile;
        if p.name.trim().is_empty() {
            Some("the profile needs a name")
        } else if p.name.chars().count() > 32 || !p.name.chars().all(crate::security::displayable) {
            Some("the profile name is too long or has characters that cannot be shown")
        } else if !identifier(&p.robe) || !p.robe.to_ascii_lowercase().starts_with("wizard") {
            Some("the profile has no valid robe")
        } else if !valid_player_colour(&p.colour) {
            Some("the profile colour is invalid")
        } else if self
            .profile_id
            .as_deref()
            .is_some_and(|id| !valid_profile_id(id))
        {
            Some("the profile id is invalid")
        } else {
            None
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileRequest {
    pub sequence: u64,
    pub profile: PlayerProfile,
    /// The joiner's persistent profile ID (rewrite extension). A host that
    /// loaded a save restores that profile's parked player. Not broadcast.
    #[serde(default)]
    pub profile_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileReply {
    pub sequence: u64,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerState {
    pub slot: usize,
    pub handle: u16,
    pub profile: PlayerProfile,
    pub position: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub backward: [f32; 3],
    pub animation: u16,
    pub animation_time: f32,
    pub animation_speed: f32,
    pub health: f32,
    pub dead: bool,
    pub items: [String; 2],
    /// Sorted equipped attachment slots, including robe accessories. This is
    /// authoritative item lifetime state, independent of health and death.
    pub attached_slots: Vec<i32>,
    pub unlocked_magicks: u64,
    pub queue: Vec<QueuedSpell>,
    /// Stored charge queues corresponding to items[0] (weapon) and items[1] (staff).
    pub item_queues: [Vec<QueuedSpell>; 2],
    /// Protocol 11: newest input sequence the host applied for this player,
    /// for client prediction reconciliation (0: none yet).
    #[serde(default)]
    pub last_input: u64,
    /// Protocol 12: unlocked registered (mod) Magicks by definition ID —
    /// stable names, never match-local indices.
    #[serde(default)]
    pub registered_magicks: Vec<String>,
}
impl PlayerState {
    pub fn valid_attached_slots(&self) -> bool {
        self.attached_slots.first().is_none_or(|slot| *slot >= 0)
            && self
                .attached_slots
                .windows(2)
                .all(|slots| slots[0] < slots[1])
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NpcState {
    pub handle: u16,
    pub template: String,
    /// Render origin is the character's feet, independent of client physics.
    pub transform: [f32; 16],
    pub clip: Option<String>,
    pub clip_time: f32,
    pub looping: bool,
    pub health: f32,
    pub dead: bool,
    /// Health bar presentation (character template): maximum health, bar
    /// count and radius, so clients draw the host's bars.
    #[serde(default)]
    pub max_health: f32,
    #[serde(default)]
    pub health_bars: i32,
    #[serde(default)]
    pub radius: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisualEffectState {
    pub slot: u16,
    pub instance: i32,
    pub definition: i32,
    pub transform: [f32; 16],
    /// Host clock (ms of [`GameFrame::time`]) when the emitter started.
    /// Stable for the emitter's life: clients derive elapsed time locally,
    /// so a running emitter costs no bandwidth (protocol 12).
    pub started_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameFrame {
    pub physics: PhysicsFrame,
    pub level: String,
    pub players: Vec<PlayerState>,
    pub npcs: Vec<NpcState>,
    pub effects: Vec<VisualEffectState>,
    /// Ruleset HUD and end-of-game state (the RulesetMessage /
    /// GameEndMessage role in the modern protocol). Per-slot arrays use
    /// server slots.
    #[serde(default)]
    pub rules: Option<RulesState>,
    /// Rewrite extension: the host's lobby policy (player limit, hot join).
    #[serde(default)]
    pub lobby: crate::lobby::LobbySettings,
    /// Rewrite extension: connected non-player roles (spectators, dungeon
    /// masters, invaders). Players are listed in `players`.
    #[serde(default)]
    pub participants: Vec<crate::lobby::ParticipantState>,
    /// Rewrite extension (protocol 10): script-driven world state.
    #[serde(default)]
    pub world: crate::world::WorldPresentation,
    /// Protocol 12: server-owned mod UI state (`NetTables`), sorted by
    /// (table, key). Clients display it; they never author it.
    #[serde(default)]
    pub mod_ui: Vec<ModUiEntry>,
    /// Protocol 12: host simulation clock (seconds) of this frame.
    #[serde(default)]
    pub time: f64,
    /// Protocol 12: recent transient presentation events (one-shot
    /// particles and sounds), each with a unique ID so clients play it once
    /// and late joiners skip stale ones.
    #[serde(default)]
    pub events: Vec<PresentationEventState>,
}

/// One net table entry; the value is JSON text (postcard cannot carry
/// self-describing values).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModUiEntry {
    pub table: String,
    pub key: String,
    pub json: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresentationEventState {
    /// Unique per host session; never reused.
    pub id: u64,
    pub sound: bool,
    /// Effect definition, retail effect name or sound cue.
    pub name: String,
    pub position: Option<[f32; 3]>,
    /// Character handle the event attaches to.
    pub owner: Option<u16>,
    /// Host tick when it started.
    pub tick: u64,
}
pub const MAX_MOD_UI_ENTRIES: usize = 4096;
pub const MAX_EVENTS: usize = 256;

/// One scrolling Survival kill score.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScrollScoreState {
    pub name: i32,
    pub score: i32,
    pub multi_kills: i32,
    pub position: f32,
    pub alpha: f32,
}

/// The host's ruleset presentation state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RulesetState {
    Survival {
        wave: u32,
        shown_total_score: i32,
        time_multiplier: i32,
        damage_multiplier: i32,
        total_multiplier: i32,
        damage_amount: f32,
        normalized_time: f32,
        health: f32,
        health_alpha: f32,
        scroll: Vec<ScrollScoreState>,
    },
    TimedObjective {
        /// (counter id, value) of each objective and the time bonus.
        counters: Vec<(i32, i32)>,
        timer: f32,
        time_text: String,
        hud: bool,
    },
    Versus {
        /// Rulesets value (3 DeathMatch, 4 Brawl, 5 Pyrite, 6 Krietor, 7 King).
        mode: u8,
        teams_enabled: bool,
        waiting: bool,
        countdown: f32,
        time_limit: f32,
        time: f32,
        scores: Vec<i16>,
        team_scores: [i16; 2],
        respawns: Vec<i16>,
        team_respawns: [i16; 2],
        respawn_timers: Vec<f32>,
        sudden_death: bool,
        unlocks: Vec<(f32, u8)>,
        teams: Vec<i32>,
    },
}

/// Ruleset, statistics and end-of-game state for clients.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RulesState {
    pub ruleset: Option<RulesetState>,
    /// StatisticsManager kills/deaths per server slot and Survival total.
    pub kills: Vec<u32>,
    pub deaths: Vec<u32>,
    pub total_score: i32,
    /// EndGameCondition, text alpha, statistics sheet shown.
    pub end_condition: u8,
    pub end_text_alpha: f32,
    pub statistics: bool,
}
impl GameFrame {
    pub fn valid(&self) -> bool {
        self.invalid_part().is_none()
    }
    /// The first part of this frame that fails validation, for the host's log.
    pub fn invalid_part(&self) -> Option<&'static str> {
        if self.physics.validate().is_err()
            || !self.time.is_finite()
            || self.time < 0.0
            || self.players.len() > crate::lobby::PLAYER_SLOT_CAP
            || self.participants.len() > crate::lobby::PARTICIPANT_CAP
            || !self.lobby.valid()
            || self.npcs.len() > 4096
            || self.effects.len() > 256
            || self.level.len() > 256
            || !self.level.ends_with(".lvl")
            || self
                .level
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
            || self
                .level
                .bytes()
                .any(|b| !(b.is_ascii_alphanumeric() || b"/_-.".contains(&b)))
        {
            return Some("header");
        }
        let mut slots = std::collections::BTreeSet::new();
        let players_valid = self.players.iter().all(|p| {
            p.slot < crate::lobby::PLAYER_SLOT_CAP
                && slots.insert(p.slot)
                && p.profile.valid()
                && p.items.iter().all(|s| identifier(s))
                && p.valid_attached_slots()
                && p.position
                    .iter()
                    .chain(&p.right)
                    .chain(&p.up)
                    .chain(&p.backward)
                    .all(|v| v.is_finite())
                && [p.health, p.animation_time, p.animation_speed]
                    .iter()
                    .all(|v| v.is_finite())
                && p.registered_magicks.len() <= 4096
                && p.registered_magicks
                    .iter()
                    .all(|m| m.len() <= 130 && m.contains(':'))
                && p.queue.len() <= 5
                && p.queue.iter().all(|spell| spell.spell().is_some())
                && p.item_queues.iter().all(|queue| {
                    queue.len() <= 5 && queue.iter().all(|spell| spell.spell().is_some())
                })
                && [&p.right, &p.up, &p.backward]
                    .iter()
                    .all(|v| (v.iter().map(|c| c * c).sum::<f32>() - 1.).abs() < 0.02)
        });
        let mut handles = std::collections::BTreeSet::new();
        let npcs_valid = self.npcs.iter().all(|n| {
            handles.insert(n.handle)
                && !self.players.iter().any(|p| p.handle == n.handle)
                && identifier(&n.template)
                && n.transform.iter().all(|v| v.is_finite())
                && n.clip.as_ref().is_none_or(|c| {
                    !c.is_empty() && c.len() <= 128 && !c.chars().any(char::is_control)
                })
                && n.clip_time.is_finite()
                && n.clip_time >= 0.
                && n.health.is_finite()
        });
        let mut effects = std::collections::BTreeSet::new();
        let effects_valid = self.effects.iter().all(|e| {
            e.slot < 256
                && effects.insert(e.slot)
                && e.instance > 0
                && e.transform.iter().all(|v| v.is_finite())
                && e.started_ms >= 0
                && e.started_ms as f64 <= self.time * 1000.0 + 1.0
        });
        let rules_valid = self.rules.as_ref().is_none_or(|r| {
            r.end_text_alpha.is_finite()
                && match &r.ruleset {
                    Some(RulesetState::Survival { scroll, .. }) => scroll.len() <= 16,
                    Some(RulesetState::TimedObjective {
                        counters,
                        time_text,
                        timer,
                        ..
                    }) => counters.len() <= 32 && time_text.len() <= 64 && timer.is_finite(),
                    Some(RulesetState::Versus {
                        unlocks,
                        scores,
                        respawns,
                        respawn_timers,
                        teams,
                        ..
                    }) => {
                        unlocks.len() <= 32
                            && [
                                scores.len(),
                                respawns.len(),
                                respawn_timers.len(),
                                teams.len(),
                            ]
                            .iter()
                            .all(|n| *n <= crate::lobby::PLAYER_SLOT_CAP)
                            && respawn_timers.iter().all(|t| t.is_finite())
                    }
                    None => true,
                }
                && r.kills.len() <= crate::lobby::PLAYER_SLOT_CAP
                && r.deaths.len() <= crate::lobby::PLAYER_SLOT_CAP
        });
        let mut seats = std::collections::BTreeSet::new();
        let participants_valid = self
            .participants
            .iter()
            .all(|p| seats.insert(p.seat) && p.valid() && !crate::lobby::is_player_seat(p.seat));
        let mod_ui_valid = self.mod_ui.len() <= MAX_MOD_UI_ENTRIES
            && self
                .mod_ui
                .windows(2)
                .all(|w| (&w[0].table, &w[0].key) < (&w[1].table, &w[1].key))
            && self
                .mod_ui
                .iter()
                .all(|e| e.table.len() <= 64 && e.key.len() <= 128 && e.json.len() <= 64 << 10);
        let mut event_ids = std::collections::BTreeSet::new();
        let events_valid = self.events.len() <= MAX_EVENTS
            && self.events.iter().all(|e| {
                event_ids.insert(e.id)
                    && !e.name.is_empty()
                    && e.name.len() <= 256
                    && e.position.is_none_or(|p| p.iter().all(|v| v.is_finite()))
            });
        [
            (players_valid, "players"),
            (npcs_valid, "npcs"),
            (effects_valid, "effects"),
            (rules_valid, "rules"),
            (participants_valid, "participants"),
            (self.world.valid(), "world"),
            (mod_ui_valid, "mod ui"),
            (events_valid, "events"),
        ]
        .into_iter()
        .find_map(|(valid, part)| (!valid).then_some(part))
    }
}

/// A client's request to a server mod (protocol 12): `channel` names the
/// mod and action (`boosted.buy`); `json` carries its data. The host runs
/// each (player, request ID) at most once and replies with the result.
/// Requests ask; they never state outcomes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModRequest {
    pub request_id: u64,
    pub channel: String,
    pub json: String,
}
impl ModRequest {
    pub fn valid(&self) -> bool {
        self.request_id != 0
            && !self.channel.is_empty()
            && self.channel.len() <= 128
            && self.channel.contains('.')
            && self.json.len() <= 16 << 10
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModReply {
    pub request_id: u64,
    pub json: String,
}

/// A request to respawn the connection's own dead player. The server supplies the loadout.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RespawnRequest {
    pub session: u64,
    pub epoch: u64,
    pub sequence: u64,
}
