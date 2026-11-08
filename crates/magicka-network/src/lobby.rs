//! Lobby policy and non-player roles. These are rewrite extensions, not
//! retail behavior: the original game allowed at most four players and no
//! joining after the game started (research/multiplayer-scaling.md).
//!
//! Seats identify connections. A player seat is the player's slot; non-player
//! roles take seats at or above [`OBSERVER_SEAT_BASE`].
use serde::{Deserialize, Serialize};

/// The original game's player count and the default lobby limit.
pub const DEFAULT_PLAYER_LIMIT: u16 = 4;
/// Largest configurable player limit, reachable only through a server's
/// `--lobby players=N`. Authored protocol policy bounding snapshot size, not a
/// measured transport capacity.
pub const PLAYER_SLOT_CAP: usize = 256;
/// Largest limit offered by in-game settings: the lobby menu and dungeon master
/// commands (user decision 2026-11-04). A host configured higher keeps its
/// configured limit as the ceiling.
pub const IN_GAME_PLAYER_LIMIT: u16 = 16;
/// Largest number of non-player participants in one frame (same policy).
pub const PARTICIPANT_CAP: usize = 256;
/// First seat number used by non-player roles.
pub const OBSERVER_SEAT_BASE: usize = 1 << 16;

pub fn is_player_seat(seat: usize) -> bool {
    seat < OBSERVER_SEAT_BASE
}

/// What a connection does in the session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    /// Owns a wizard in a player slot.
    #[default]
    Player,
    /// Watches without an avatar; the camera follows players or roams.
    Spectator,
    /// Spectates with game-master powers: spawn/possess/smite/heal NPCs and
    /// change lobby policy.
    DungeonMaster,
    /// Pilots an enemy NPC; after its death, takes over another one.
    Invader,
}

impl Role {
    pub const ALL: [Role; 4] = [
        Role::Player,
        Role::Spectator,
        Role::DungeonMaster,
        Role::Invader,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Role::Player => "Player",
            Role::Spectator => "Spectator",
            Role::DungeonMaster => "Dungeon Master",
            Role::Invader => "Invader",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "player" => Some(Role::Player),
            "spectator" | "spectate" | "watch" => Some(Role::Spectator),
            "dm" | "gm" | "dungeon-master" | "dungeonmaster" => Some(Role::DungeonMaster),
            "invader" | "invade" => Some(Role::Invader),
            _ => None,
        }
    }
}

/// Host policy, chosen in the lobby and published in every frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbySettings {
    /// Player slots (1..=PLAYER_SLOT_CAP). The original game's four is the
    /// default.
    pub max_players: u16,
    /// Players may join a game in progress and spawn beside an existing
    /// player. When locked (e.g. for PvP), joiners wait as spectators until
    /// the next round or level.
    pub hot_join: bool,
    pub spectators: bool,
    pub dungeon_masters: bool,
    pub invaders: bool,
    /// A dead client may ask to come back at will (the demo's F5). Not in the
    /// original game and a cheat in normal play, so a host must opt in.
    pub debug_respawn: bool,
    /// Above four players, bosses last about as long as for four. At up to four
    /// the game is native either way, and native boss health ignores the party.
    pub boss_scale: bool,
}

impl Default for LobbySettings {
    fn default() -> Self {
        Self {
            max_players: DEFAULT_PLAYER_LIMIT,
            hot_join: true,
            spectators: true,
            // Powerful roles are opt-in for the host.
            dungeon_masters: false,
            invaders: false,
            debug_respawn: false,
            boss_scale: true,
        }
    }
}

/// How the host admits a connection asking for a role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Player,
    /// A non-player role. `queued` players wait for a slot (full lobby or
    /// locked hot join) and are promoted at the next round/level.
    Observer {
        role: Role,
        queued: bool,
    },
    Rejected(&'static str),
}

impl LobbySettings {
    /// PvP keeps hot join locked by default.
    pub fn versus() -> Self {
        Self {
            hot_join: false,
            ..Self::default()
        }
    }
    pub fn valid(&self) -> bool {
        (1..=PLAYER_SLOT_CAP).contains(&usize::from(self.max_players))
    }
    /// The highest limit an in-game command may set on a host started with
    /// `configured` players.
    pub fn in_game_limit_ceiling(configured: u16) -> u16 {
        configured.max(IN_GAME_PLAYER_LIMIT)
    }
    pub fn allows(&self, role: Role) -> bool {
        match role {
            Role::Player => true,
            Role::Spectator => self.spectators,
            Role::DungeonMaster => self.dungeon_masters,
            Role::Invader => self.invaders,
        }
    }

    /// `players=8,hotjoin=off,spectators=on,dm=off,invaders=on`. Keys may
    /// appear in any order; unspecified keys keep `self`'s values.
    pub fn parse_onto(mut self, spec: &str) -> Result<Self, String> {
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (key, value) = part.split_once('=').unwrap_or((part, "on"));
            let flag = || match value.to_ascii_lowercase().as_str() {
                "on" | "1" | "true" | "yes" | "open" => Ok(true),
                "off" | "0" | "false" | "no" | "locked" | "lock" => Ok(false),
                _ => Err(format!(
                    "lobby option {key:?} expects on/off, got {value:?}"
                )),
            };
            match key.to_ascii_lowercase().as_str() {
                "players" | "max" | "limit" => {
                    self.max_players = value
                        .parse()
                        .ok()
                        .filter(|n: &u16| (1..=PLAYER_SLOT_CAP).contains(&usize::from(*n)))
                        .ok_or_else(|| {
                            format!("players expects 1..={PLAYER_SLOT_CAP}, got {value:?}")
                        })?
                }
                "hotjoin" | "hot-join" | "join" => self.hot_join = flag()?,
                "spectators" | "spectator" => self.spectators = flag()?,
                "dm" | "dungeon-masters" | "dungeonmasters" => self.dungeon_masters = flag()?,
                "invaders" | "invader" | "invasion" => self.invaders = flag()?,
                "respawn" | "debug-respawn" => self.debug_respawn = flag()?,
                "bossscale" | "boss-scale" => self.boss_scale = flag()?,
                _ => return Err(format!("unknown lobby option {key:?}")),
            }
        }
        Ok(self)
    }

    /// Admission for `requested` with `players` slots in use. `in_progress`
    /// is true once the round/level has begun (every hosted game today).
    pub fn admit(&self, requested: Role, players: usize, in_progress: bool) -> Admission {
        match requested {
            Role::Player => {
                let full = players >= usize::from(self.max_players);
                let locked = in_progress && !self.hot_join;
                if !full && !locked {
                    Admission::Player
                } else if self.spectators {
                    Admission::Observer {
                        role: Role::Spectator,
                        queued: true,
                    }
                } else if full {
                    Admission::Rejected("the lobby is full")
                } else {
                    Admission::Rejected("joining a game in progress is locked")
                }
            }
            role if self.allows(role) => Admission::Observer {
                role,
                queued: false,
            },
            _ => Admission::Rejected("the host does not allow that role"),
        }
    }
}

/// A connected non-player as clients see it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParticipantState {
    pub seat: usize,
    pub role: Role,
    pub name: String,
    /// The NPC an invader or dungeon master currently pilots.
    pub possessed: Option<u16>,
    /// Waiting for a player slot.
    pub queued: bool,
}

impl ParticipantState {
    pub fn valid(&self) -> bool {
        self.role != Role::Player
            && !self.name.trim().is_empty()
            && self.name.chars().count() <= 32
            && self.name.chars().all(crate::security::displayable)
    }
}

/// Client → host: switch role (spectator joins in, player steps out, ...).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoleRequest {
    pub sequence: u64,
    pub role: Role,
    pub name: String,
}

/// Dungeon-master powers. The host validates every command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DmCommand {
    /// Spawn a character template at a world position.
    SpawnNpc {
        template: String,
        position: [f32; 3],
    },
    /// Take control of an NPC (also used by invaders choosing a body).
    Possess {
        npc: u16,
    },
    /// Return a possessed NPC to its AI.
    Release,
    /// Kill a character outright.
    Smite {
        handle: u16,
    },
    /// Restore a character to full health.
    Heal {
        handle: u16,
    },
    SetHotJoin(bool),
    SetPlayerLimit(u16),
    /// Remove a connection from the game and refuse its address for the rest of
    /// the session (protocol 18).
    Kick {
        seat: usize,
    },
    /// Editor spawn (protocol 15): a `kind:name` catalogue id (npc, prop,
    /// item, book, fx) at a model transform (XNA storage).
    Spawn {
        spawnable: String,
        transform: [f32; 16],
        /// NPC order; None for the default.
        order: Option<String>,
        /// Items and books: movable instead of fixed.
        movable: bool,
    },
    /// Editor move of a live entity to a model transform (protocol 15).
    Move {
        handle: u16,
        transform: [f32; 16],
    },
    /// Editor removal of an NPC, prop, item or book (protocol 15).
    Remove {
        handle: u16,
    },
}

/// A `kind:name` editor catalogue id: a known kind and a relative content
/// name without parent-directory steps.
pub fn spawnable_id(value: &str) -> bool {
    let Some((kind, name)) = value.split_once(':') else {
        return false;
    };
    ["npc", "prop", "item", "book", "fx"].contains(&kind)
        && !name.is_empty()
        && value.len() <= 160
        && !name.contains("..")
        && !name.starts_with('/')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-./ ".contains(&b))
}

fn finite_transform(m: &[f32; 16]) -> bool {
    m.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.0)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DmRequest {
    pub sequence: u64,
    pub command: DmCommand,
}

impl DmRequest {
    pub fn valid(&self) -> bool {
        match &self.command {
            DmCommand::SpawnNpc { template, position } => {
                crate::session::identifier(template)
                    && position
                        .iter()
                        .all(|v| v.is_finite() && v.abs() <= 1_000_000.0)
            }
            DmCommand::SetPlayerLimit(n) => (1..=PLAYER_SLOT_CAP).contains(&usize::from(*n)),
            DmCommand::Spawn {
                spawnable,
                transform,
                order,
                ..
            } => {
                spawnable_id(spawnable)
                    && finite_transform(transform)
                    && order.as_deref().is_none_or(crate::session::identifier)
            }
            DmCommand::Move { transform, .. } => finite_transform(transform),
            _ => true,
        }
    }
}

#[cfg(test)]
mod boss_scale_tests {
    use super::LobbySettings;

    #[test]
    fn boss_scale_is_on_by_default_and_the_spec_turns_it_off() {
        assert!(LobbySettings::default().boss_scale);
        let off = LobbySettings::default()
            .parse_onto("bossscale=off")
            .unwrap();
        assert!(!off.boss_scale);
    }
}
