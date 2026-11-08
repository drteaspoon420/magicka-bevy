//! VersusRuleset host (research/rulesets.md): player facts in, world-level
//! commands applied here; player setup, revival and magick unlocks are
//! queued for PlaySession, which owns equipment and UnlockedMagicks.
use super::*;
use crate::ruleset::{
    RulesetLayout, SceneRuleset, VersusCommand, VersusFacts, VersusPlayer, VersusRuleset,
    SOUND_GAME_CLOCK,
};

/// EndGameCondition.EndOffGame.
const END_OFF_GAME: u8 = 8;

impl<C: SceneContent> SceneWorld<C> {
    pub fn versus(&self) -> Option<&VersusRuleset> {
        self.ruleset.as_ref().and_then(SceneRuleset::versus)
    }

    /// Run `f` with the Versus state detached so it can draw world RNG.
    fn with_versus<R>(&mut self, f: impl FnOnce(&mut VersusRuleset, &mut Self) -> R) -> Option<R> {
        let Some(SceneRuleset::Versus(mut r)) = self.ruleset.take() else {
            return None;
        };
        let out = f(&mut r, self);
        self.ruleset = Some(SceneRuleset::Versus(r));
        Some(out)
    }

    /// GameScene construction of the Versus ruleset. Native players gather
    /// in the lobby before the scene exists; the rewrite's hosts let them
    /// join a running scene, so Initialize waits for a second player
    /// (rewrite extension) instead of ending a one-player match at once.
    pub(super) fn initialize_versus(&mut self, layout: &RulesetLayout) {
        let r = VersusRuleset::new(layout, self.versus_settings);
        self.ruleset = Some(SceneRuleset::Versus(Box::new(r)));
        self.versus_waiting = true;
        self.start_versus_when_ready();
    }

    /// VersusRuleset.Initialize once two players are connected.
    fn start_versus_when_ready(&mut self) {
        let facts = self.versus_facts();
        if !self.versus_waiting || facts.players.iter().filter(|p| p.connected).count() < 2 {
            return;
        }
        self.versus_waiting = false;
        // VersusRuleset.Initialize: StatisticsManager.SurvivalReset.
        self.statistics.reset();
        self.avatar_dead_seen = crate::slots::PerSlot::default();
        for (i, p) in self.players.iter().enumerate() {
            self.avatar_dead_seen[i] = p.as_ref().is_some_and(|p| p.dead());
        }
        let commands = self
            .with_versus(|r, _| r.initialize(&facts))
            .unwrap_or_default();
        self.apply_versus(commands);
    }

    pub fn versus_facts(&self) -> VersusFacts {
        let luggage = self
            .presence
            .get(custom_hash("any"))
            .ok()
            .is_some_and(|any| {
                any.type_count(custom_hash(crate::ruleset::LUGGAGE_ITEM)) > 0
                    || any.type_count(custom_hash(crate::ruleset::LUGGAGE_MAGICK)) > 0
            });
        VersusFacts {
            players: {
                let mut players = crate::slots::PerSlot::default();
                for (i, p) in self.players.iter().enumerate() {
                    players[i] = p
                        .as_ref()
                        .map_or_else(VersusPlayer::default, |p| VersusPlayer {
                            connected: true,
                            in_lobby: false,
                            playing: p.playing,
                            team: self.player_teams[i],
                            alive: !p.dead(),
                        });
                }
                players
            },
            game_ended: self.game_ended || self.pending_end_game.is_some(),
            luggage_present: luggage,
            host: self.role != NetworkRole::Client,
        }
    }

    pub(super) fn update_versus(&mut self, dt: f32) {
        if self.role == NetworkRole::Client {
            return;
        }
        if self.versus_waiting {
            self.start_versus_when_ready();
            return;
        }
        let facts = self.versus_facts();
        let commands = self
            .with_versus(|r, w| r.update(dt, &facts, &mut |n| w.random_index(n)))
            .unwrap_or_default();
        self.apply_versus(commands);
    }

    /// VersusRuleset.OnPlayerKill from AddKillEvent.
    pub(super) fn versus_player_kill(&mut self, attacker: usize, target: usize) {
        let facts = self.versus_facts();
        let commands = self
            .with_versus(|r, _| r.on_player_kill(attacker, Some(target), &facts))
            .unwrap_or_default();
        self.apply_versus(commands);
    }

    /// Avatar death transitions: StatisticsManager.AddKillEvent once, and
    /// VersusRuleset.OnPlayerDeath per native call (OverKill and Terminate
    /// call it before Die calls it again), each followed by DropMagicks.
    pub(super) fn track_avatar_deaths(&mut self) {
        for slot in 0..self.players.len() {
            let Some(p) = self.players[slot].as_ref() else {
                self.avatar_dead_seen[slot] = false;
                continue;
            };
            let dead = p.playing && p.dead();
            if !dead {
                self.avatar_dead_seen[slot] = false;
                continue;
            }
            if self.avatar_dead_seen[slot] {
                continue;
            }
            self.avatar_dead_seen[slot] = true;
            let calls = if p.terminated || p.character.combat.overkilled {
                2
            } else {
                1
            };
            let handle = p.handle;
            self.note_kill(handle);
            if self.versus().is_none() || self.versus_waiting {
                continue;
            }
            for _ in 0..calls {
                let facts = self.versus_facts();
                let commands = self
                    .with_versus(|r, _| r.on_player_death(slot, &facts))
                    .unwrap_or_default();
                self.apply_versus(commands);
                if self.versus().is_some_and(VersusRuleset::drop_magicks) {
                    self.versus_drop_magicks.push(slot);
                }
            }
        }
    }

    fn random_index(&mut self, n: usize) -> usize {
        ((self.random() * n as f32) as usize).min(n.saturating_sub(1))
    }

    fn apply_versus(&mut self, commands: Vec<VersusCommand>) {
        for command in commands {
            match command {
                VersusCommand::EndGame => {
                    self.pending_end_game = Some((END_OFF_GAME, true, false));
                }
                VersusCommand::PlayCue(name) => {
                    let key = self.play_cue(crate::audio::banks::ADDITIONAL, name, None);
                    if name == SOUND_GAME_CLOCK {
                        self.versus_clock_cue = Some(key);
                    }
                }
                VersusCommand::StopClock => {
                    if let Some(key) = self.versus_clock_cue.take() {
                        self.stop_cue(key);
                    }
                }
                VersusCommand::LimitInput => self.flags.input_limited = true,
                VersusCommand::UnlimitInput => self.flags.input_limited = false,
                VersusCommand::SpawnLuggage { item, area } => {
                    let Some(packs) = self.survival_packs.clone() else {
                        continue;
                    };
                    if let Err(e) = self.spawn_versus_luggage(item, area, &packs) {
                        self.log.push(format!("versus luggage: {e}"));
                    }
                }
                VersusCommand::RemoveFriendly { slot } => {
                    if let Some(handle) = self.players[slot].as_ref().map(|p| p.handle) {
                        if let Ok(e) = self.entity_mut(handle) {
                            e.faction &= !crate::camera::FACTION_FRIENDLY;
                            e.original_faction &= !crate::camera::FACTION_FRIENDLY;
                        }
                    }
                }
                VersusCommand::SetTeam { slot, team } => {
                    self.set_player_team(slot, team);
                }
                VersusCommand::ResetArena => {
                    for liquid in &mut self.collision.collision_mut().water {
                        let magnitude = if liquid.freeze.auto_freeze { 2.0 } else { -2.0 };
                        liquid.freeze.freeze_all(magnitude);
                    }
                    self.nullify();
                }
                other => self.pending_versus.push(other),
            }
        }
    }

    /// Player.Team with the avatar's faction team bits (Avatar.Initialize).
    pub fn set_player_team(&mut self, slot: usize, team: i32) {
        const TEAMS: i32 = crate::ruleset::TEAM_RED | crate::ruleset::TEAM_BLUE;
        self.player_teams[slot] = team;
        if let Some(handle) = self.players[slot].as_ref().map(|p| p.handle) {
            if let Ok(e) = self.entity_mut(handle) {
                e.faction = (e.faction & !TEAMS) | team;
                e.original_faction = (e.original_faction & !TEAMS) | team;
            }
        }
    }

    /// VersusRuleset.GetMatrix: the area's locator (else a random point of
    /// the trigger area) dropped onto scene geometry from 1 m above within
    /// 3 m, else onto a liquid's ice; None when neither is hit.
    pub fn versus_matrix(&mut self, area: i32) -> Option<[f32; 16]> {
        let mut locator = match self.locators.get(&area) {
            Some(m) => *m,
            None => {
                let p = self.random_area_point(area)?;
                let mut m = magicka_content::skinned::IDENTITY;
                m[12..15].copy_from_slice(&p);
                m
            }
        };
        let segment = magicka_sim::spray_pulse::Segment {
            origin: [locator[12], locator[13] + 1.0, locator[14]],
            delta: [0.0, -3.0, 0.0],
        };
        let collision = self.collision.collision();
        let hit = collision
            .segment_intersect(segment, false)
            .map(|h| h.hit.position)
            .or_else(|| {
                collision
                    .liquid_segment_intersect(segment, true, true, false)
                    .map(|h| h.position)
            })?;
        locator[12..15].copy_from_slice(&hit);
        Some(locator)
    }
}
