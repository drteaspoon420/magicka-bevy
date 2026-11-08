//! `Magicka.Levels.Versus` (research/rulesets.md): VersusRuleset and its
//! lobby modes as one deterministic state machine. Inputs are per-player
//! facts and Random.Next draws; outputs are commands the scene host applies
//! (player setup/revival, luggage, end of game, magick unlocks). The lobby
//! offers DeathMatch, Brawl and Krietor ("Tourney"); King and Pyrite are
//! constructible natively but unreachable from SubMenuCharacterSelect.
use super::*;
use crate::scene_loading::custom_hash;

pub const TEAM_RED: i32 = 0x1000;
pub const TEAM_BLUE: i32 = 0x2000;
/// VersusRuleset.CountDown(4) at Initialize and Krietor.Reset.
pub const COUNTDOWN: f32 = 4.0;
/// Respawn delay of DeathMatch, Brawl and King.
pub const RESPAWN_TIME: f32 = 5.0;
/// Krietor.RESET_TIME.
pub const KRIETOR_RESET_TIME: f32 = 2.0;
/// Luggage drop despawn (SpawnItemEvent/SpawnMagickEvent(…, 12)).
pub const LUGGAGE_DESPAWN: f32 = 12.0;
/// Additional-bank cues.
pub const SOUND_GAME_START: &str = "game_start";
pub const SOUND_GAME_END: &str = "game_end";
pub const SOUND_GAME_CLOCK: &str = "game_clock";

/// Rulesets (Versus values).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VersusMode {
    DeathMatch,
    Brawl,
    Pyrite,
    Krietor,
    King,
}

impl VersusMode {
    /// SubMenuCharacterSelect's game mode box.
    pub const LOBBY: [VersusMode; 3] = [Self::DeathMatch, Self::Brawl, Self::Krietor];
    /// The lobby label key.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::DeathMatch => Some("#versus_dm"),
            Self::Brawl => Some("#versus_brawl"),
            Self::Krietor => Some("#versus_tourney"),
            Self::King | Self::Pyrite => None,
        }
    }
}

/// The Settings dropdown values (Settings constructors).
pub const TIME_LIMITS: [i32; 5] = [0, 5, 10, 30, 50];
pub const KRIETOR_TIME_LIMITS: [i32; 2] = [5, 10];
pub const DEATHMATCH_SCORE_LIMITS: [i32; 5] = [i32::MAX, 5, 10, 20, 50];
pub const KING_SCORE_LIMITS: [i32; 5] = [0, 5, 10, 20, 50];
pub const LIVES: [i32; 5] = [1, 5, 10, 20, 50];
/// High, medium, low, off.
pub const LUGGAGE_INTERVALS: [i32; 4] = [15, 30, 90, 0];

/// One mode's lobby settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersusSettings {
    pub mode: VersusMode,
    /// Minutes (0 unlimited).
    pub time_limit: i32,
    pub score_limit: i32,
    pub lives: i32,
    pub teams: bool,
    /// Seconds (0 off).
    pub luggage_interval: i32,
}

impl VersusSettings {
    /// The constructors' default selections.
    pub fn defaults(mode: VersusMode) -> Self {
        let base = Self {
            mode,
            time_limit: 0,
            score_limit: 0,
            lives: 0,
            teams: false,
            luggage_interval: 0,
        };
        match mode {
            VersusMode::DeathMatch => Self {
                time_limit: TIME_LIMITS[1],
                score_limit: DEATHMATCH_SCORE_LIMITS[2],
                luggage_interval: LUGGAGE_INTERVALS[1],
                ..base
            },
            VersusMode::Brawl => Self {
                time_limit: TIME_LIMITS[1],
                lives: LIVES[1],
                luggage_interval: LUGGAGE_INTERVALS[1],
                ..base
            },
            VersusMode::Krietor => Self {
                time_limit: KRIETOR_TIME_LIMITS[1],
                ..base
            },
            VersusMode::King => Self {
                time_limit: TIME_LIMITS[0],
                score_limit: KING_SCORE_LIMITS[3],
                ..base
            },
            VersusMode::Pyrite => base,
        }
    }
}

impl VersusSettings {
    /// A launch spec: `mode[,teams][,time=M][,score=N][,lives=N][,luggage=S]`
    /// with mode `deathmatch|brawl|tourney|krietor|king|pyrite`; unspecified
    /// options keep the mode's defaults.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut parts = spec.split(',').map(str::trim);
        let mode = match parts.next().unwrap_or("").to_ascii_lowercase().as_str() {
            "deathmatch" | "dm" => VersusMode::DeathMatch,
            "brawl" => VersusMode::Brawl,
            "tourney" | "krietor" | "kreitor" => VersusMode::Krietor,
            "king" => VersusMode::King,
            "pyrite" => VersusMode::Pyrite,
            other => return Err(format!("unknown versus mode {other:?}")),
        };
        let mut s = Self::defaults(mode);
        for part in parts {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            let number = || {
                value
                    .parse::<i32>()
                    .map_err(|_| format!("bad {key} value {value:?}"))
            };
            match key.to_ascii_lowercase().as_str() {
                "teams" => s.teams = value.is_empty() || value == "1" || value == "true",
                "time" => s.time_limit = number()?,
                "score" => s.score_limit = number()?,
                "lives" => s.lives = number()?,
                "luggage" => s.luggage_interval = number()?,
                _ => return Err(format!("unknown versus option {key:?}")),
            }
        }
        Ok(s)
    }

    /// Player.Team per slot when teams are on: alternating red/blue (the
    /// lobby lets players pick; INFERRED default).
    pub fn default_teams(&self, count: usize) -> crate::slots::PerSlot<i32> {
        let mut teams = crate::slots::PerSlot::default();
        for i in 0..count {
            teams[i] = match (self.teams, i % 2) {
                (false, _) => 0,
                (true, 0) => TEAM_RED,
                (true, _) => TEAM_BLUE,
            };
        }
        teams
    }
}

/// The `<Areas>` spawn lists and Krietor's `<Tournament>` unlocks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VersusAreas {
    pub all: Vec<i32>,
    pub team_a: Vec<i32>,
    pub team_b: Vec<i32>,
}

fn attr<'a>(node: &'a XmlElement, name: &str) -> Option<&'a str> {
    node.attributes
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn area_nodes(node: &XmlElement) -> impl Iterator<Item = &XmlElement> {
    node.children
        .iter()
        .filter(|c| c.name.eq_ignore_ascii_case("areas"))
        .flat_map(|c| c.children.iter())
        .filter(|a| a.name.eq_ignore_ascii_case("area"))
}

fn team_of(area: &XmlElement) -> Option<usize> {
    let team = attr(area, "team")?;
    if team.eq_ignore_ascii_case("b") || team.eq_ignore_ascii_case("blue") {
        Some(1)
    } else if team.eq_ignore_ascii_case("a") || team.eq_ignore_ascii_case("red") {
        Some(0)
    } else {
        None
    }
}

impl VersusAreas {
    /// VersusRuleset constructor: every area in `all`, team lists by tag.
    pub fn read(node: &XmlElement) -> Self {
        let mut r = Self::default();
        for area in area_nodes(node) {
            let id = custom_hash(&area.inner_text.to_lowercase());
            match team_of(area) {
                Some(0) => r.team_a.push(id),
                Some(_) => r.team_b.push(id),
                None => {}
            }
            r.all.push(id);
        }
        r
    }

    /// Krietor constructor: tourney areas only, team ones excluded from `all`.
    pub fn read_tourney(node: &XmlElement) -> Self {
        let mut r = Self::default();
        for area in area_nodes(node) {
            if !attr(area, "tourney").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
                continue;
            }
            let id = custom_hash(&area.inner_text.to_lowercase());
            match team_of(area) {
                Some(0) => r.team_a.push(id),
                Some(_) => r.team_b.push(id),
                None => r.all.push(id),
            }
        }
        r
    }
}

/// Krietor `<Tournament><magick unlock="start|fraction">`: (fraction, magick)
/// for fractions below 1, sorted by fraction (stable for equal keys).
pub fn read_unlocks(node: &XmlElement) -> Vec<(f32, u8)> {
    let mut out: Vec<(f32, u8)> = node
        .children
        .iter()
        .filter(|c| c.name.eq_ignore_ascii_case("tournament"))
        .flat_map(|c| c.children.iter())
        .filter(|m| m.name.eq_ignore_ascii_case("magick"))
        .filter_map(|m| {
            let magick = crate::magick_books::magick_id(&m.inner_text)?;
            let key = match attr(m, "unlock") {
                Some(v) if v.eq_ignore_ascii_case("start") => 0.0,
                Some(v) => v.trim().parse().unwrap_or(1.0),
                None => 1.0,
            };
            (key < 1.0).then_some((key, magick))
        })
        .collect();
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// One player slot's facts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VersusPlayer {
    pub connected: bool,
    pub in_lobby: bool,
    pub playing: bool,
    /// Player.Team (TEAM_RED / TEAM_BLUE flags, 0 without teams).
    pub team: i32,
    /// Avatar exists and is not dead.
    pub alive: bool,
}

/// The scene facts one update reads.
#[derive(Clone, Debug, Default)]
pub struct VersusFacts {
    /// Rewrite extension: any number of slots (the original has four).
    pub players: crate::slots::PerSlot<VersusPlayer>,
    pub game_ended: bool,
    /// The "any" area holds a luggage NPC.
    pub luggage_present: bool,
    /// Offline/server (setup, luggage and scoring are host-only).
    pub host: bool,
}

impl VersusFacts {
    fn connected_count(&self) -> usize {
        self.players.iter().filter(|p| p.connected).count()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum VersusCommand {
    /// SetupPlayer: GetMatrix(area), then move a living avatar (full health,
    /// statuses cleared) or revive a dead one there.
    SetupPlayer {
        slot: usize,
        area: i32,
    },
    /// RevivePlayer at GetMatrix(area) (King respawns).
    RevivePlayer {
        slot: usize,
        area: i32,
    },
    /// Avatar.Terminate (Krietor sudden-death losers).
    TerminatePlayer {
        slot: usize,
    },
    /// EndGame: game_end cue, PlayState.Endgame(EndOffGame, freeze, 0).
    EndGame,
    /// SpawnLuggage at GetAnyArea (12 s despawning drops).
    SpawnLuggage {
        item: bool,
        area: i32,
    },
    LimitInput,
    UnlimitInput,
    PlayCue(&'static str),
    StopClock,
    /// SpellManager.UnlockMagick for every playing player.
    UnlockMagick(u8),
    /// Initialize: UnlockedMagicks = 0 for all players.
    ClearUnlockedMagicks,
    /// Initialize: Avatar.Faction &= ~FRIENDLY.
    RemoveFriendly {
        slot: usize,
    },
    /// King Initialize with teams: alternate red/blue Player.Team.
    SetTeam {
        slot: usize,
        team: i32,
    },
    /// Krietor.Reset: liquids FreezeAll(±2) and Nullify.NullifyArea.
    ResetArena,
}

#[derive(Clone, Debug)]
pub struct VersusRuleset {
    pub settings: VersusSettings,
    pub areas: VersusAreas,
    /// Krietor unlocks still pending (fraction, magick).
    pub unlocks: Vec<(f32, u8)>,
    authored_unlocks: Vec<(f32, u8)>,
    pub time_limit: f32,
    pub time_limit_timer: f32,
    pub countdown_timer: f32,
    intro_cue_timer: f32,
    clock_started: bool,
    pub scores: crate::slots::PerSlot<i16>,
    pub team_scores: [i16; 2],
    /// Brawl lives left per player / team.
    pub respawns: crate::slots::PerSlot<i16>,
    pub team_respawns: [i16; 2],
    pub respawn_timers: crate::slots::PerSlot<f32>,
    luggage_timer: f32,
    initialized_spawns: bool,
    pub sudden_death: bool,
    reset_timer: f32,
}

fn team_index(team: i32) -> Option<usize> {
    if team & TEAM_RED == TEAM_RED {
        Some(0)
    } else if team & TEAM_BLUE == TEAM_BLUE {
        Some(1)
    } else {
        None
    }
}

impl VersusRuleset {
    pub fn new(layout: &RulesetLayout, settings: VersusSettings) -> Self {
        let krietor = settings.mode == VersusMode::Krietor;
        let unlocks = if krietor {
            read_unlocks(&layout.node)
        } else {
            Vec::new()
        };
        Self {
            settings,
            areas: if krietor {
                VersusAreas::read_tourney(&layout.node)
            } else {
                VersusAreas::read(&layout.node)
            },
            unlocks: unlocks.clone(),
            authored_unlocks: unlocks,
            time_limit: 0.0,
            time_limit_timer: 0.0,
            countdown_timer: 0.0,
            intro_cue_timer: 0.0,
            clock_started: false,
            scores: Default::default(),
            team_scores: [0; 2],
            respawns: Default::default(),
            team_respawns: [0; 2],
            respawn_timers: Default::default(),
            luggage_timer: 0.0,
            initialized_spawns: false,
            sudden_death: false,
            reset_timer: 0.0,
        }
    }

    /// GetAnyArea.
    pub fn any_area(&self, next: &mut dyn FnMut(usize) -> usize) -> Option<i32> {
        (!self.areas.all.is_empty()).then(|| self.areas.all[next(self.areas.all.len())])
    }

    /// GetTeamArea(Factions).
    pub fn team_area(&self, team: i32, next: &mut dyn FnMut(usize) -> usize) -> Option<i32> {
        let list = match team_index(team) {
            Some(0) => &self.areas.team_a,
            Some(_) => &self.areas.team_b,
            None => return self.any_area(next),
        };
        (!list.is_empty()).then(|| list[next(list.len())])
    }

    /// CanRevive (Revive magick in Versus): only King with teams, same team.
    pub fn can_revive(&self, reviver_team: i32, revivee_team: i32) -> bool {
        self.settings.mode == VersusMode::King
            && self.settings.teams
            && reviver_team & revivee_team != 0
    }

    /// DropMagicks: dying players drop their unlocked magicks as books.
    pub fn drop_magicks(&self) -> bool {
        matches!(
            self.settings.mode,
            VersusMode::DeathMatch | VersusMode::Brawl
        )
    }

    /// Initialize (base, then the mode).
    pub fn initialize(&mut self, facts: &VersusFacts) -> Vec<VersusCommand> {
        use VersusCommand::*;
        let s = self.settings;
        let mut out = vec![ClearUnlockedMagicks];
        for (slot, p) in facts.players.iter().enumerate() {
            if p.connected && (p.playing || p.in_lobby) {
                out.push(RemoveFriendly { slot });
            }
        }
        self.countdown(&mut out);
        out.push(StopClock);
        self.clock_started = false;
        self.intro_cue_timer = 0.0;
        self.time_limit = s.time_limit as f32 * 60.0;
        self.time_limit_timer = self.time_limit;
        let slots = facts.players.len().max(crate::slots::ORIGINAL_PLAYER_LIMIT);
        self.scores = crate::slots::PerSlot::filled(slots, 0);
        self.team_scores = [0; 2];
        self.respawn_timers = crate::slots::PerSlot::filled(slots, 0.0);
        self.luggage_timer = s.luggage_interval as f32;
        self.initialized_spawns = false;
        self.sudden_death = false;
        self.reset_timer = 0.0;
        match s.mode {
            VersusMode::Brawl => {
                let lives = s.lives as u8 as i16;
                self.respawns = crate::slots::PerSlot::filled(slots, lives);
                self.team_respawns = [lives; 2];
            }
            VersusMode::King if s.teams => {
                let mut n = 0;
                for (slot, p) in facts.players.iter().enumerate() {
                    if p.connected && !p.in_lobby {
                        let team = if n % 2 == 0 { TEAM_RED } else { TEAM_BLUE };
                        out.push(SetTeam { slot, team });
                        n += 1;
                    }
                }
            }
            VersusMode::Krietor => {
                self.unlocks = self.authored_unlocks.clone();
                self.unlocks.retain(|&(key, magick)| {
                    if key <= 0.0 {
                        out.push(UnlockMagick(magick));
                        false
                    } else {
                        true
                    }
                });
                out.push(StopClock);
            }
            _ => {}
        }
        out
    }

    fn countdown(&mut self, out: &mut Vec<VersusCommand>) {
        self.countdown_timer = COUNTDOWN;
        out.push(VersusCommand::LimitInput);
    }

    fn end_game(&self, out: &mut Vec<VersusCommand>) {
        // EndGame never sets mGameOver: each call stops the clock and
        // plays game_end again (PlayState.Endgame keeps the earliest time).
        out.push(VersusCommand::StopClock);
        out.push(VersusCommand::PlayCue(SOUND_GAME_END));
        out.push(VersusCommand::EndGame);
    }

    /// Update (host): the mode's update, then the base update.
    pub fn update(
        &mut self,
        dt: f32,
        facts: &VersusFacts,
        next: &mut dyn FnMut(usize) -> usize,
    ) -> Vec<VersusCommand> {
        use VersusCommand::*;
        let mut out = Vec::new();
        let mode = self.settings.mode;
        if mode == VersusMode::DeathMatch && !self.initialized_spawns {
            if facts.host && !self.areas.all.is_empty() {
                let mut spawns = self.areas.all.clone();
                let start = next(spawns.len());
                for (slot, p) in facts.players.iter().enumerate() {
                    if p.connected && !p.in_lobby && !spawns.is_empty() {
                        let i = start % spawns.len();
                        out.push(SetupPlayer {
                            slot,
                            area: spawns.remove(i),
                        });
                    }
                }
            }
            self.initialized_spawns = true;
        }
        if matches!(
            mode,
            VersusMode::DeathMatch | VersusMode::Brawl | VersusMode::King
        ) {
            for slot in 0..self.respawn_timers.len().max(facts.players.len()) {
                let left = (self.respawn_timers[slot] - dt).max(0.0);
                if self.respawn_timers[slot] > 0.0 && left <= 0.0 && facts.players[slot].playing {
                    if let Some(area) = self.team_area(facts.players[slot].team, next) {
                        out.push(if mode == VersusMode::King {
                            RevivePlayer { slot, area }
                        } else {
                            SetupPlayer { slot, area }
                        });
                    }
                }
                self.respawn_timers[slot] = left;
            }
        }
        if matches!(mode, VersusMode::DeathMatch | VersusMode::Brawl)
            && self.settings.luggage_interval > 0
        {
            if self.luggage_timer <= 0.0 {
                let roll = next(3);
                if !facts.luggage_present {
                    if let Some(area) = self.any_area(next) {
                        out.push(SpawnLuggage {
                            item: roll != 0,
                            area,
                        });
                    }
                }
                self.luggage_timer = self.settings.luggage_interval as f32;
            }
            self.luggage_timer -= dt;
        }
        if mode == VersusMode::Krietor {
            self.update_krietor(dt, facts, next, &mut out);
        } else if self.time_limit > 0.0 && !facts.game_ended {
            self.time_limit_timer -= dt;
            if self.time_limit_timer <= 0.0 {
                self.end_game(&mut out);
                self.time_limit_timer = if mode == VersusMode::DeathMatch {
                    0.0
                } else {
                    self.time_limit
                };
            }
        }
        if mode != VersusMode::King
            && mode != VersusMode::Pyrite
            && self.time_limit_timer <= 10.0
            && !self.clock_started
            && self.time_limit_timer > 0.0
        {
            self.clock_started = true;
            out.push(PlayCue(SOUND_GAME_CLOCK));
        }
        // VersusRuleset.Update.
        if self.intro_cue_timer < 0.25 {
            self.intro_cue_timer += dt;
            if self.intro_cue_timer >= 0.25 {
                out.push(PlayCue(SOUND_GAME_START));
            }
        }
        if facts.connected_count() <= 1 {
            self.end_game(&mut out);
        }
        if self.countdown_timer > -1.0 {
            self.countdown_timer -= dt;
            if self.countdown_timer <= 0.0 {
                out.push(UnlimitInput);
            }
        }
        out
    }

    fn update_krietor(
        &mut self,
        dt: f32,
        facts: &VersusFacts,
        next: &mut dyn FnMut(usize) -> usize,
        out: &mut Vec<VersusCommand>,
    ) {
        if self.time_limit > 0.0
            && !facts.game_ended
            && self.reset_timer <= 0.0
            && self.countdown_timer <= 0.0
        {
            self.time_limit_timer -= dt;
            if self.time_limit_timer <= 0.0 {
                let n = self.scores.len().max(facts.players.len()).max(1);
                let best = (0..n).fold(0, |b, i| {
                    if self.scores[i] > self.scores[b] {
                        i
                    } else {
                        b
                    }
                });
                let tie = (0..n).any(|i| i != best && self.scores[i] == self.scores[best]);
                if !tie {
                    self.end_game(out);
                    self.time_limit_timer = 0.0;
                } else {
                    self.sudden_death = true;
                    self.time_limit = 0.0;
                    self.time_limit_timer = 0.0;
                    out.push(VersusCommand::StopClock);
                    self.reset(facts, next, out);
                }
            }
        }
        if self.reset_timer > 0.0 {
            self.reset_timer -= dt;
            if self.reset_timer <= 0.0 {
                self.reset_timer = 0.0;
                self.reset(facts, next, out);
            }
        }
        let elapsed = self.time_limit - self.time_limit_timer;
        let limit = self.time_limit;
        self.unlocks.retain(|&(key, magick)| {
            if elapsed > key * limit {
                out.push(VersusCommand::UnlockMagick(magick));
                false
            } else {
                true
            }
        });
    }

    /// Krietor.Reset: new round positions, arena reset, countdown.
    fn reset(
        &mut self,
        facts: &VersusFacts,
        next: &mut dyn FnMut(usize) -> usize,
        out: &mut Vec<VersusCommand>,
    ) {
        use VersusCommand::*;
        let place = |list: &[i32],
                     filter: &dyn Fn(usize) -> bool,
                     out: &mut Vec<_>,
                     next: &mut dyn FnMut(usize) -> usize| {
            let mut spawns = list.to_vec();
            if spawns.is_empty() {
                return;
            }
            let start = next(spawns.len());
            for slot in 0..facts.players.len() {
                if facts.players[slot].playing && filter(slot) && !spawns.is_empty() {
                    let i = start % spawns.len();
                    out.push(SetupPlayer {
                        slot,
                        area: spawns.remove(i),
                    });
                }
            }
        };
        if facts.host {
            if self.sudden_death && !self.settings.teams {
                let best = (0..facts.players.len())
                    .filter(|&s| facts.players[s].playing)
                    .map(|s| self.scores[s])
                    .max()
                    .unwrap_or(0);
                let mut spawns = self.areas.all.clone();
                let start = if spawns.is_empty() {
                    0
                } else {
                    next(spawns.len())
                };
                for slot in 0..facts.players.len() {
                    let p = facts.players[slot];
                    if !p.playing {
                        continue;
                    }
                    if self.scores[slot] == best {
                        if !spawns.is_empty() {
                            let i = start % spawns.len();
                            out.push(SetupPlayer {
                                slot,
                                area: spawns.remove(i),
                            });
                        }
                    } else if p.alive {
                        out.push(TerminatePlayer { slot });
                    }
                }
            } else if self.settings.teams {
                let red = |s: usize| facts.players[s].team & TEAM_RED == TEAM_RED;
                let blue = |s: usize| facts.players[s].team & TEAM_BLUE == TEAM_BLUE;
                place(&self.areas.team_a, &red, out, next);
                place(&self.areas.team_b, &blue, out, next);
            } else {
                place(&self.areas.all, &|_| true, out, next);
            }
        }
        out.push(ResetArena);
        self.countdown(out);
    }

    /// OnPlayerKill (StatisticsManager.AddKillEvent between enemy avatars).
    pub fn on_player_kill(
        &mut self,
        attacker: usize,
        target: Option<usize>,
        facts: &VersusFacts,
    ) -> Vec<VersusCommand> {
        let mut out = Vec::new();
        if self.settings.mode != VersusMode::DeathMatch || !facts.host || facts.game_ended {
            return out;
        }
        if target == Some(attacker) {
            self.scores[attacker] -= 1;
        } else {
            self.scores[attacker] += 1;
        }
        let limit = self.settings.score_limit;
        let attacker_team = facts.players[attacker].team;
        if let Some(t) = team_index(attacker_team) {
            let flag = if t == 0 { TEAM_RED } else { TEAM_BLUE };
            if target.is_some_and(|s| facts.players[s].team & flag == flag) {
                self.team_scores[t] -= 1;
            } else {
                self.team_scores[t] += 1;
            }
            if i32::from(self.team_scores[t]) >= limit {
                self.end_game(&mut out);
            }
        } else if i32::from(self.scores[attacker]) >= limit {
            self.end_game(&mut out);
        }
        out
    }

    /// OnPlayerDeath (Avatar.Die, OverKill and Terminate). `facts` already
    /// shows the dying player as dead.
    pub fn on_player_death(&mut self, slot: usize, facts: &VersusFacts) -> Vec<VersusCommand> {
        let mut out = Vec::new();
        match self.settings.mode {
            VersusMode::DeathMatch | VersusMode::King => {
                if !facts.game_ended && self.respawn_timers[slot] <= 0.0 {
                    self.respawn_timers[slot] = RESPAWN_TIME;
                }
            }
            VersusMode::Brawl => {
                if !facts.host || facts.game_ended {
                    return out;
                }
                let holding = |s: usize, timers: &crate::slots::PerSlot<f32>| {
                    let p = facts.players[s];
                    p.playing && (p.alive || timers[s] > 0.0)
                };
                if let Some(t) = team_index(facts.players[slot].team) {
                    self.team_respawns[t] -= 1;
                    self.respawns[slot] -= 1;
                    if self.team_respawns[t] >= 0 {
                        self.respawn_timers[slot] = RESPAWN_TIME;
                        return out;
                    }
                    let flag = if t == 0 { TEAM_RED } else { TEAM_BLUE };
                    let team_alive = (0..facts.players.len()).any(|s| {
                        facts.players[s].team & flag == flag && holding(s, &self.respawn_timers)
                    });
                    if !team_alive {
                        self.end_game(&mut out);
                    }
                } else {
                    self.respawns[slot] -= 1;
                    if self.respawns[slot] >= 0 {
                        self.respawn_timers[slot] = RESPAWN_TIME;
                        return out;
                    }
                    let left = (0..facts.players.len())
                        .filter(|&s| holding(s, &self.respawn_timers))
                        .count();
                    if left <= 1 {
                        self.end_game(&mut out);
                    }
                }
            }
            VersusMode::Krietor => {
                if !facts.host || facts.game_ended || self.reset_timer > 0.0 {
                    return out;
                }
                if let Some(t) = team_index(facts.players[slot].team) {
                    let flag = if t == 0 { TEAM_RED } else { TEAM_BLUE };
                    let team_alive = facts
                        .players
                        .iter()
                        .any(|p| p.playing && p.team & flag == flag && p.alive);
                    if !team_alive {
                        let winner = 1 - t;
                        self.scores[winner] += 1;
                        self.team_scores[winner] += 1;
                        self.round_over(&mut out);
                    }
                } else {
                    let survivors: Vec<usize> = (0..facts.players.len())
                        .filter(|&s| {
                            s != slot && facts.players[s].playing && facts.players[s].alive
                        })
                        .collect();
                    if survivors.len() > 1 {
                        return out;
                    }
                    if let Some(&last) = survivors.last() {
                        self.scores[last] += 1;
                    }
                    self.round_over(&mut out);
                }
            }
            VersusMode::Pyrite => {}
        }
        out
    }

    fn round_over(&mut self, out: &mut Vec<VersusCommand>) {
        if self.sudden_death {
            self.end_game(out);
        } else {
            self.reset_timer = KRIETOR_RESET_TIME;
        }
    }

    /// The Teams property (King reports false even with teams).
    pub fn teams(&self) -> bool {
        self.settings.teams && !matches!(self.settings.mode, VersusMode::King | VersusMode::Pyrite)
    }

    /// GetScores / GetTeamScores (Brawl: lives left; King and Pyrite: none).
    pub fn mode_scores(&self) -> Option<(crate::slots::PerSlot<i16>, [i16; 2])> {
        match self.settings.mode {
            VersusMode::Brawl => Some((self.respawns.clone(), self.team_respawns)),
            VersusMode::DeathMatch | VersusMode::Krietor => {
                Some((self.scores.clone(), self.team_scores))
            }
            VersusMode::King | VersusMode::Pyrite => None,
        }
    }

    /// RenderData's next-unlock icon and its seconds left (shown below 10 s).
    pub fn next_unlock(&self) -> Option<(u8, f32)> {
        let &(key, magick) = self.unlocks.first()?;
        let left = key * self.time_limit - (self.time_limit - self.time_limit_timer);
        Some((magick, left))
    }

    /// RenderData.SetTimeText: the last ten seconds as a whole number.
    pub fn clock_seconds(&self) -> Option<i32> {
        (self.time_limit_timer <= 10.0 && self.time_limit > 0.0)
            .then_some(self.time_limit_timer as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_script::SceneScriptLayout;

    fn layout() -> RulesetLayout {
        SceneScriptLayout::read(
            br#"<Scene><RuleSet type="Versus"><Areas>
              <area team="A" tourney="true">A0</area><area team="A">A1</area>
              <area team="B" tourney="true">B0</area><area team="B">B1</area>
              <area tourney="true">F0</area><area tourney="true">F1</area><area>F2</area>
            </Areas><tournament><magick unlock="start">revive</magick>
              <magick unlock="0.5">haste</magick><magick>grease</magick></tournament>
            </RuleSet></Scene>"#,
            false,
        )
        .unwrap()
        .ruleset
        .unwrap()
    }

    fn players(n: usize) -> crate::slots::PerSlot<VersusPlayer> {
        crate::slots::PerSlot::from_array::<4>(std::array::from_fn(|i| VersusPlayer {
            connected: i < n,
            playing: i < n,
            alive: i < n,
            ..Default::default()
        }))
    }

    fn facts(n: usize) -> VersusFacts {
        VersusFacts {
            players: players(n),
            host: true,
            ..Default::default()
        }
    }

    #[test]
    fn launch_specs_parse_onto_the_mode_defaults() {
        let s = VersusSettings::parse("tourney,teams,time=5").unwrap();
        assert_eq!(
            (s.mode, s.teams, s.time_limit),
            (VersusMode::Krietor, true, 5)
        );
        assert_eq!(
            s.default_teams(4),
            [TEAM_RED, TEAM_BLUE, TEAM_RED, TEAM_BLUE].into()
        );
        let d = VersusSettings::parse("deathmatch").unwrap();
        assert_eq!(d, VersusSettings::defaults(VersusMode::DeathMatch));
        assert!(VersusSettings::parse("chess").is_err());
    }

    #[test]
    fn areas_follow_each_constructor() {
        let l = layout();
        let all = VersusAreas::read(&l.node);
        assert_eq!(
            (all.all.len(), all.team_a.len(), all.team_b.len()),
            (7, 2, 2)
        );
        let t = VersusAreas::read_tourney(&l.node);
        assert_eq!(t.all, [custom_hash("f0"), custom_hash("f1")]);
        assert_eq!((t.team_a.len(), t.team_b.len()), (1, 1));
        assert_eq!(
            read_unlocks(&l.node),
            [(0.0, 1), (0.5, 3)],
            "unlock 1 never happens"
        );
    }

    #[test]
    fn deathmatch_spawns_scores_and_ends_at_the_limit() {
        let mut r = VersusRuleset::new(&layout(), VersusSettings::defaults(VersusMode::DeathMatch));
        let f = facts(2);
        let init = r.initialize(&f);
        assert!(init.contains(&VersusCommand::LimitInput));
        assert!(init.contains(&VersusCommand::RemoveFriendly { slot: 1 }));
        let mut next = |n: usize| n - 1;
        let first = r.update(0.1, &f, &mut next);
        let setups: Vec<_> = first
            .iter()
            .filter(|c| matches!(c, VersusCommand::SetupPlayer { .. }))
            .collect();
        assert_eq!(setups.len(), 2, "both players get distinct spawns");
        assert_ne!(setups[0], setups[1]);
        for _ in 0..9 {
            assert!(r.on_player_kill(0, Some(1), &f).is_empty());
        }
        assert!(r
            .on_player_kill(0, Some(1), &f)
            .contains(&VersusCommand::EndGame));
        assert_eq!(r.on_player_kill(1, Some(1), &f), vec![]);
        assert_eq!(r.scores[1], -1, "suicide");
        r.on_player_death(1, &f);
        assert_eq!(r.respawn_timers[1], RESPAWN_TIME);
        let mut out = vec![];
        for _ in 0..51 {
            out.extend(r.update(0.1, &f, &mut next));
        }
        assert!(out
            .iter()
            .any(|c| matches!(c, VersusCommand::SetupPlayer { slot: 1, .. })));
        assert!(out.contains(&VersusCommand::UnlimitInput), "4 s countdown");
        assert!(out.contains(&VersusCommand::PlayCue(SOUND_GAME_START)));
    }

    #[test]
    fn a_lone_connected_player_ends_the_match() {
        let mut r = VersusRuleset::new(&layout(), VersusSettings::defaults(VersusMode::Brawl));
        let f = facts(1);
        r.initialize(&f);
        assert!(r
            .update(0.1, &f, &mut |_| 0)
            .contains(&VersusCommand::EndGame));
    }

    #[test]
    fn brawl_ends_when_lives_run_out() {
        let mut s = VersusSettings::defaults(VersusMode::Brawl);
        s.lives = 1;
        let mut r = VersusRuleset::new(&layout(), s);
        let mut f = facts(2);
        r.initialize(&f);
        f.players[1].alive = false;
        assert!(r.on_player_death(1, &f).is_empty(), "one life left");
        assert_eq!(r.respawn_timers[1], RESPAWN_TIME);
        r.respawn_timers[1] = 0.0;
        assert!(r.on_player_death(1, &f).contains(&VersusCommand::EndGame));
    }

    #[test]
    fn krietor_rounds_unlocks_and_sudden_death() {
        let mut s = VersusSettings::defaults(VersusMode::Krietor);
        s.time_limit = 5;
        let mut r = VersusRuleset::new(&layout(), s);
        let mut f = facts(3);
        let init = r.initialize(&f);
        assert!(
            init.contains(&VersusCommand::UnlockMagick(1)),
            "start unlock"
        );
        let mut next = |_| 0;
        // The round clock waits for the countdown.
        for _ in 0..41 {
            r.update(0.1, &f, &mut next);
        }
        assert_eq!(r.time_limit_timer, 300.0);
        f.players[2].alive = false;
        assert!(r.on_player_death(2, &f).is_empty(), "two still alive");
        f.players[1].alive = false;
        r.on_player_death(1, &f);
        assert_eq!(r.scores[0], 1);
        let mut out = vec![];
        for _ in 0..21 {
            out.extend(r.update(0.1, &f, &mut next));
        }
        assert!(out.contains(&VersusCommand::ResetArena));
        assert_eq!(
            out.iter()
                .filter(|c| matches!(c, VersusCommand::SetupPlayer { .. }))
                .count(),
            2,
            "two free-for-all spawns"
        );
        // Half time unlocks haste.
        r.countdown_timer = 0.0;
        r.time_limit_timer = 149.0;
        assert!(r
            .update(0.1, &f, &mut next)
            .contains(&VersusCommand::UnlockMagick(3)));
        // A tie at time-out starts sudden death instead of ending.
        r.scores = [1, 1, 0, 0].into();
        r.time_limit_timer = 0.05;
        let out = r.update(0.1, &f, &mut next);
        assert!(r.sudden_death && !out.contains(&VersusCommand::EndGame));
        assert!(out.contains(&VersusCommand::TerminatePlayer { slot: 2 }) || !f.players[2].alive);
        f.players[1].alive = false;
        assert!(r.on_player_death(1, &f).contains(&VersusCommand::EndGame));
    }
}
