//! Session side of Versus (research/rulesets.md): the lobby settings, and
//! the VersusRuleset commands that need equipment or UnlockedMagicks.
use super::*;
use crate::ruleset::{RulesetKind, VersusCommand, VersusSettings};
use magicka_sim::status::Status as S;

/// SetupPlayer's StopStatusEffects mask.
const SETUP_STATUSES: u16 = S::Burning as u16
    | S::Wet as u16
    | S::Frozen as u16
    | S::Cold as u16
    | S::Poisoned as u16
    | S::Healing as u16
    | S::Greased as u16
    | S::Steamed as u16;
/// Avatar.DropMagicks: MagickType values below 34 except Revive.
const DROPPED_MAGICKS: u8 = 34;
const DROP_TIMEOUT: f32 = 20.0;

impl PlaySession {
    /// The lobby's Versus choice (GameScene construction): settings and
    /// Player.Team per slot. Re-initializes a Versus scene's ruleset.
    pub fn set_versus(
        &mut self,
        settings: VersusSettings,
        teams: impl IntoIterator<Item = i32>,
    ) -> Result<(), SessionError> {
        self.world.versus_settings = settings;
        for (slot, team) in teams.into_iter().enumerate() {
            self.world
                .set_player_team(slot, if settings.teams { team } else { 0 });
        }
        let layout = self.runtime.layout().ruleset.clone();
        if layout
            .as_ref()
            .is_some_and(|l| l.kind == RulesetKind::Versus)
        {
            self.world
                .initialize_ruleset(layout.as_ref(), &mut self.runtime.variables)
                .map_err(SessionError::World)?;
            self.apply_versus_requests();
        }
        Ok(())
    }

    pub(super) fn apply_versus_requests(&mut self) {
        for command in std::mem::take(&mut self.world.pending_versus) {
            if let Err(e) = self.versus_command(command) {
                self.world.log.push(format!("versus: {e:?}"));
            }
        }
        for slot in std::mem::take(&mut self.world.versus_drop_magicks) {
            if let Err(e) = self.drop_magicks(slot) {
                self.world.log.push(format!("versus drop: {e:?}"));
            }
        }
    }

    fn versus_command(&mut self, command: VersusCommand) -> Result<(), SessionError> {
        match command {
            VersusCommand::SetupPlayer { slot, area } => {
                let alive = self.world.players[slot].as_ref().is_some_and(|p| !p.dead());
                let m = self.versus_matrix(area);
                if alive {
                    self.versus_move_player(slot, m)
                } else {
                    self.versus_revive(slot, m)
                }
            }
            VersusCommand::RevivePlayer { slot, area } => {
                let m = self.versus_matrix(area);
                self.versus_revive(slot, m)
            }
            VersusCommand::TerminatePlayer { slot } => {
                if let Some(p) = self.world.players[slot].as_mut() {
                    p.terminate();
                }
                Ok(())
            }
            VersusCommand::UnlockMagick(m) => {
                let n = self.world.players.len();
                self.unlocked_magicks.reserve_slots(n);
                for (slot, bits) in self.unlocked_magicks.iter_mut().enumerate() {
                    if self.world.players[slot].as_ref().is_some_and(|p| p.playing) {
                        *bits |= 1u64 << m;
                    }
                }
                Ok(())
            }
            VersusCommand::ClearUnlockedMagicks => {
                let n = self.world.players.len();
                self.unlocked_magicks.fill(n, 0);
                Ok(())
            }
            other => Err(SessionError::World(format!(
                "unrouted versus command {other:?}"
            ))),
        }
    }

    /// GetMatrix's failure leaves the identity matrix (PROVEN, decompiled).
    fn versus_matrix(&mut self, area: i32) -> [f32; 16] {
        self.world.versus_matrix(area).unwrap_or_else(|| {
            self.world
                .log
                .push(format!("versus: no ground for area {area}"));
            magicka_content::skinned::IDENTITY
        })
    }

    /// MovePlayer (+0.15 m above the capsule height), full health and the
    /// SetupPlayer status stop.
    fn versus_move_player(&mut self, slot: usize, m: [f32; 16]) -> Result<(), SessionError> {
        let (position, orientation) = locator_parts(&m);
        let p = self.world.players[slot]
            .as_mut()
            .ok_or_else(|| SessionError::World("versus player absent".into()))?;
        let body = &mut self.world.physics.bodies[p.body];
        let position = [
            position[0],
            position[1] + body.length * 0.5 + body.radius + 0.15,
            position[2],
        ];
        body.move_to(position, orientation);
        body.set_movement([0.0; 3]);
        p.reset_to_idle(body);
        let health = &mut p.character.combat.health;
        health.hit_points = health.max_hit_points;
        body.owner.hit_points = health.hit_points;
        p.character.stop_status_effects(SETUP_STATUSES, true);
        Ok(())
    }

    /// RevivePlayer: freeze liquids around the point, robe defaults, a fresh
    /// avatar raised by the capsule height + 0.15, ethereal for 2.25 s,
    /// not FRIENDLY, full health, revive animation and cue.
    fn versus_revive(&mut self, slot: usize, m: [f32; 16]) -> Result<(), SessionError> {
        let (origin, orientation) = locator_parts(&m);
        self.world.freeze_water(
            origin,
            [2.0, 0.0, 0.0],
            std::f32::consts::TAU,
            2.0,
            &[magicka_sim::damage::Damage {
                attack: magicka_sim::damage::attack::STATUS,
                element: magicka_sim::spells::Elements::COLD.0,
                amount: 100.0,
                magnitude: 4.0,
            }],
        );
        let robe = if slot == 0 {
            self.character.clone()
        } else {
            self.remote_characters
                .get(&slot)
                .cloned()
                .ok_or_else(|| SessionError::World("missing remote character".into()))?
        };
        self.reset_player_equipment(slot, &robe)?;
        let handle = self.player_avatar(slot)?.handle;
        self.world.cancel_player_abilities(slot, handle);
        let p = self.world.players[slot].as_mut().unwrap();
        let body = &mut self.world.physics.bodies[p.body];
        let position = [
            origin[0],
            origin[1] + body.length * 0.5 + body.radius + 0.15,
            origin[2],
        ];
        body.move_to(position, orientation);
        body.set_movement([0.0; 3]);
        p.character.combat.dead = false;
        p.character.combat.overkilled = false;
        p.terminated = false;
        p.character.timed_ethereal(0.0, true);
        p.character.timed_ethereal(2.25, false);
        let health = &mut p.character.combat.health;
        health.hit_points = p.character.definition.max_hit_points;
        body.owner.hit_points = health.hit_points;
        let revive = crate::character_motion::animation_index("revive")
            .expect("revive is an Animations value");
        p.begin_resurrection(body, revive);
        if let Ok(e) = self.world.entity_mut(handle) {
            e.dead = false;
            e.removed = false;
            e.faction &= !crate::camera::FACTION_FRIENDLY;
        }
        self.world.play_cue(16, "magick_revive", Some(position));
        Ok(())
    }

    /// Avatar.DropMagicks: a 20 s book for each unlocked magick except
    /// Revive, launched from the avatar; UnlockedMagicks cleared.
    fn drop_magicks(&mut self, slot: usize) -> Result<(), SessionError> {
        let bits = std::mem::take(&mut self.unlocked_magicks[slot]);
        let p = self.player_avatar(slot)?;
        let body = &self.world.physics.bodies[p.body];
        let position = body.position;
        let o = body.orientation;
        let mut transform = magicka_content::skinned::IDENTITY;
        transform[..3].copy_from_slice(&o.right);
        transform[4..7].copy_from_slice(&o.up);
        transform[8..11].copy_from_slice(&o.backward);
        for magick in (0..DROPPED_MAGICKS).filter(|&m| m != 1 && bits & (1u64 << m) != 0) {
            self.world
                .spawn_event_book(magick, position, transform, DROP_TIMEOUT, 3.0)
                .map_err(SessionError::World)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ruleset::{VersusMode, VersusSettings};
    use magicka_sim::damage_delivery::DamagePayload;

    fn arena() -> Option<PlaySession> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../target/Magicka/Content");
        if !root.is_dir() {
            return None;
        }
        let mut s = PlaySession::load(&root, "Versus/vs_traininggrounds.lvl", "Wizard").unwrap();
        s.join_player_at_start(1, "Wizard").unwrap();
        s.set_versus(VersusSettings::defaults(VersusMode::DeathMatch), [0; 4])
            .unwrap();
        Some(s)
    }

    fn kill(s: &mut PlaySession, target: usize, attacker: usize) {
        let attacker = s.world.players[attacker].as_ref().unwrap().handle;
        let p = s.world.players[target].as_mut().unwrap();
        let handle = p.handle;
        let hp = p.character.combat.health.hit_points;
        let body = &mut s.world.physics.bodies[p.body];
        crate::avatar::deliver_to_body(
            &mut p.character,
            body,
            &[magicka_sim::resistance::Resistance::IDENTITY; 11],
            true,
            handle,
            DamagePayload::Single(magicka_sim::damage::Damage {
                attack: 1,
                element: 0,
                amount: hp + 10.0,
                magnitude: 1.0,
            }),
            Some(attacker),
            [0.0; 3],
            0.0,
        );
    }

    #[test]
    fn deathmatch_spawns_scores_kills_and_respawns() {
        let Some(mut s) = arena() else {
            return;
        };
        assert_eq!(s.end_game.game_type, crate::end_game::GameType::Versus);
        let before: Vec<_> = (0..2)
            .map(|i| s.world.physics.bodies[s.world.players[i].as_ref().unwrap().body].position)
            .collect();
        s.step(1.0 / 60.0);
        let after: Vec<_> = (0..2)
            .map(|i| s.world.physics.bodies[s.world.players[i].as_ref().unwrap().body].position)
            .collect();
        assert!(
            before != after,
            "SetupPlayer moved the players to arena spawns"
        );
        assert!(s.world.flags.input_limited, "4 s countdown");
        let friendly = s
            .world
            .entity(s.world.players[1].as_ref().unwrap().handle)
            .unwrap()
            .faction
            & crate::camera::FACTION_FRIENDLY;
        assert_eq!(friendly, 0, "versus avatars are not FRIENDLY");
        for _ in 0..250 {
            s.step(1.0 / 60.0);
        }
        assert!(!s.world.flags.input_limited);
        kill(&mut s, 1, 0);
        for _ in 0..240 {
            s.step(1.0 / 60.0);
            if s.world.players[1].as_ref().unwrap().dead() {
                break;
            }
        }
        for _ in 0..30 {
            s.step(1.0 / 60.0);
        }
        let r = s.world.versus().unwrap();
        assert_eq!(r.scores[0], 1, "log: {:?}", s.world.log);
        assert_eq!(s.world.statistics.scores[1].deaths, 1);
        assert!(r.respawn_timers[1] > 0.0);
        for _ in 0..360 {
            s.step(1.0 / 60.0);
        }
        assert!(
            !s.world.players[1].as_ref().unwrap().dead(),
            "respawned after 5 s"
        );
        assert!(s.end_game.condition == crate::end_game::EndGameCondition::None);
    }
}

#[cfg(test)]
mod waiting_tests {
    use crate::play_session::PlaySession;
    use crate::ruleset::{VersusMode, VersusSettings};

    #[test]
    fn a_hosted_match_starts_when_the_second_player_joins() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../target/Magicka/Content");
        if !root.is_dir() {
            return;
        }
        let mut s = PlaySession::load(&root, "Versus/vs_boat.lvl", "Wizard").unwrap();
        s.set_versus(VersusSettings::defaults(VersusMode::Brawl), [0; 4])
            .unwrap();
        for _ in 0..60 {
            s.step(1.0 / 60.0);
        }
        assert!(s.world.versus_waiting);
        assert_eq!(
            s.end_game.condition,
            crate::end_game::EndGameCondition::None,
            "a lone host waits instead of ending the match"
        );
        s.join_player_at_start(1, "Wizard").unwrap();
        s.step(1.0 / 60.0);
        assert!(!s.world.versus_waiting);
        assert!(s.world.flags.input_limited, "the countdown started");
    }
}
