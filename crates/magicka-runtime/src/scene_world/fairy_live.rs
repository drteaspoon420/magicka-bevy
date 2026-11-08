//! Scene host for the single-player revival fairy (`revival_fairy`;
//! research/revival-fairy.md): PlayState.mSpawnFairies, SetSpawnFairies,
//! SpawnFairies, RemoveFairyFrom, the State.UpdateState/ApplyState fairy
//! flags and Fairy.Update's world side effects (effects, cues, camera,
//! Revive, crash damage, ethereal grace).
use super::*;
use crate::end_game::GameType;
use crate::revival_fairy::{FairyEvent, OwnerFacts};
use crate::revival_fairy::{
    CRASH_DAMAGE_AMOUNT, CRASH_DAMAGE_MAGNITUDE, CUE_DEATH, EFFECT_CRASH, EFFECT_ORBIT,
};

impl<C: SceneContent> SceneWorld<C> {
    /// Game.Instance.ConnectedPlayerCount as this world sees it: playing
    /// slots (a dormant slot-zero avatar does not count).
    pub fn connected_player_count(&self) -> usize {
        self.players.iter().flatten().filter(|p| p.playing).count()
    }

    /// TriggerCheckpoint.Execute's fairy gate: SpawnFairy (the action's own
    /// flag, checked by the caller) && ConnectedPlayerCount == 1 && GameType
    /// != Versus.
    pub fn single_player_non_versus_game(&self) -> bool {
        self.connected_player_count() == 1 && self.game_type != GameType::Versus
    }

    /// PlayState.SetSpawnFairies, as CreatePlayer calls it: with
    /// Level.SpawnFairy, one player and a Campaign/Mythos level, every slot
    /// with an avatar is flagged.
    pub fn set_spawn_fairies(&mut self) {
        if !self.level_spawn_fairy
            || !self.game_type.is_campaign()
            || self.connected_player_count() != 1
        {
            return;
        }
        for slot in 0..self.players.len() {
            self.spawn_fairies[slot] = self.players[slot].as_ref().is_some_and(|p| p.playing);
        }
    }

    /// PlayState.SpawnFairies (GameScene.Initialize after the players are
    /// placed): Initialize (no greeting) every flagged slot's fairy, unless
    /// a Campaign/Mythos game has more than one connected player.
    pub fn spawn_fairies(&mut self) {
        if self.connected_player_count() != 1 && self.game_type.is_campaign() {
            return;
        }
        for slot in 0..self.players.len() {
            if self.spawn_fairies[slot] && self.players[slot].as_ref().is_some_and(|p| p.playing) {
                self.initialize_revival_fairy_slot(slot, false);
            }
        }
    }

    /// State.ApplyState's fairy step (checkpoint restart): with one
    /// connected player a flagged slot's fairy is initialised (no greeting),
    /// every other fairy is killed.
    pub fn apply_spawn_fairies(&mut self) {
        let single = self.connected_player_count() == 1;
        for slot in 0..self.players.len() {
            if !self.players[slot].as_ref().is_some_and(|p| p.playing) {
                continue;
            }
            if single && self.spawn_fairies[slot] {
                self.initialize_revival_fairy_slot(slot, false);
            } else {
                self.kill_revival_fairy(slot);
            }
        }
    }

    /// State.UpdateState (PlayState.UpdateCheckPoint): mSpawnFairies[i] =
    /// the connected player's avatar has an active fairy; the array is
    /// copied back to the PlayState.
    pub(super) fn sync_spawn_fairies_from_active(&mut self) {
        for slot in 0..self.players.len() {
            let playing = self.players[slot].as_ref().is_some_and(|p| p.playing);
            self.spawn_fairies[slot] = playing && self.revival_fairies[slot].active;
        }
    }

    /// Avatar.RevivalFairy.Initialize(playState, showGreeting).
    pub fn initialize_revival_fairy_slot(&mut self, slot: usize, show_greeting: bool) {
        let Some(owner) = self.owner_facts(slot) else {
            return;
        };
        self.revival_fairies.reserve_slots(slot + 1);
        let events = self.revival_fairies[slot].initialize(show_greeting, owner.position);
        self.log.push(format!("fairy {slot} initialize"));
        self.apply_fairy_events(slot, events);
    }

    /// Avatar.RevivalFairy.Kill (hot joins, ApplyState, scene teardown).
    pub fn kill_revival_fairy(&mut self, slot: usize) {
        if self.revival_fairies.get(slot).is_none() {
            return;
        }
        let was_active = self.revival_fairies[slot].active;
        let events = self.revival_fairies[slot].kill();
        if was_active {
            self.log.push(format!("fairy {slot} kill"));
        }
        self.apply_fairy_events(slot, events);
    }

    /// UpdateMiscA's defeat exemption: `ConnectedPlayerCount == 1 &&
    /// Avatar.RevivalFairy.Active` keeps a dead player from counting as
    /// defeated.
    pub fn revival_fairy_blocks_defeat(&self, slot: usize) -> bool {
        self.connected_player_count() == 1 && self.revival_fairies[slot].active
    }

    fn owner_facts(&self, slot: usize) -> Option<OwnerFacts> {
        let player = self.players.get(slot)?.as_ref().filter(|p| p.playing)?;
        let body = self.physics.bodies.get(player.body)?;
        Some(OwnerFacts {
            dead: player.dead(),
            position: body.position,
            direction: body.direction(),
            last_position_on_ground: body.last_position_on_ground(),
        })
    }

    /// Fairy.Update for every slot (EntityManager order: after the avatars).
    pub fn fairy_effect_instance(&self, slot: usize) -> Option<u64> {
        self.fairy_effects.get(&slot).copied()
    }

    pub(super) fn update_revival_fairies(&mut self, dt: f32) {
        if self.revival_fairies.is_empty() {
            return;
        }
        // NetworkServer's hot join kills the host's fairy and removes its
        // flag (INFERRED generalisation: every fairy gate needs exactly one
        // connected player).
        if self.connected_player_count() != 1 {
            for slot in 0..self.revival_fairies.len() {
                if self.revival_fairies[slot].active {
                    self.kill_revival_fairy(slot);
                }
                self.spawn_fairies[slot] = false;
            }
            return;
        }
        for slot in 0..self.revival_fairies.len() {
            if !self.revival_fairies[slot].active {
                continue;
            }
            let owner = self.owner_facts(slot);
            let nav = self.nav.as_ref();
            let snap = |point: [f32; 3]| {
                nav.and_then(|nav| nav.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
                    .map_or(point, |(_, point, _)| point)
            };
            let events = self.revival_fairies[slot].update(dt, owner.as_ref(), &snap);
            self.apply_fairy_events(slot, events);
        }
    }

    fn apply_fairy_events(&mut self, slot: usize, events: Vec<FairyEvent>) {
        for event in events {
            match event {
                FairyEvent::Greeting => {
                    // DialogManager.StartDialog(fairy_greet0/1) needs the
                    // dialog UI host; recorded for the presentation.
                    self.log.push(format!("fairy {slot} greeting"));
                }
                FairyEvent::StartOrbitEffect { position } => {
                    let instance = self.start_effect_transform(
                        custom_hash(EFFECT_ORBIT),
                        crate::particles::create_world(position, [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
                    );
                    self.fairy_effects.insert(slot, instance);
                }
                FairyEvent::MoveOrbitEffect {
                    position,
                    direction,
                } => {
                    if let Some(&instance) = self.fairy_effects.get(&slot) {
                        self.effect_commands.push(SceneEffectCommand::Move {
                            instance,
                            position,
                            direction,
                        });
                    }
                }
                FairyEvent::FollowCamera { position } => {
                    self.log.push(format!("fairy {slot} fly"));
                    if self.camera.behaviour == crate::camera::CameraBehaviour::FollowPlayers {
                        self.camera.follow(position);
                        self.revival_fairies[slot].camera_following = true;
                    }
                }
                FairyEvent::BodyMoved { position } => {
                    if self.revival_fairies[slot].camera_following && self.camera.is_following() {
                        self.camera.set_followed_position(Some(position));
                    }
                }
                FairyEvent::DeathCue { position } => {
                    self.play_cue(crate::audio::banks::ADDITIONAL, CUE_DEATH, Some(position));
                }
                FairyEvent::StopOrbitEffect => {
                    if let Some(instance) = self.fairy_effects.remove(&slot) {
                        self.effect_commands
                            .push(SceneEffectCommand::Stop { instance });
                    }
                }
                FairyEvent::StartCrashEffect { position } => {
                    self.start_effect_transform(
                        custom_hash(EFFECT_CRASH),
                        crate::particles::create_world(position, [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
                    );
                }
                FairyEvent::ReleaseCamera { time } => {
                    if self.camera.behaviour == crate::camera::CameraBehaviour::FollowEntity {
                        self.camera.release_keep_magnification(time);
                    }
                }
                FairyEvent::Revive { position, health } => {
                    self.log.push(format!("fairy {slot} revive"));
                    // Revive.Execute(position, playState): the position
                    // overload, no caster, no specific player.
                    self.revive(None, position, health);
                }
                FairyEvent::CircleDamage { origin, range } => {
                    let damage = magicka_sim::damage::Damage {
                        attack: magicka_sim::damage::attack::KNOCKBACK,
                        element: magicka_sim::spells::Elements::EARTH.0,
                        amount: CRASH_DAMAGE_AMOUNT,
                        magnitude: CRASH_DAMAGE_MAGNITUDE,
                    };
                    let t = self.play_time;
                    self.arc_damage(
                        None,
                        None,
                        origin,
                        [0.0, 0.0, -1.0],
                        range,
                        std::f32::consts::PI,
                        &[damage],
                        t,
                    );
                }
                FairyEvent::RemoveFairyFrom => {
                    self.spawn_fairies[slot] = false;
                }
                FairyEvent::Deactivated { ethereal_grace } => {
                    self.log.push(format!("fairy {slot} done"));
                    if let Some(player) = self.players.get_mut(slot).and_then(Option::as_mut) {
                        player.character.timed_ethereal(0.0, true);
                        player.character.timed_ethereal(ethereal_grace, false);
                    }
                }
                FairyEvent::Killed => {}
            }
        }
    }
}

/// A fairy's presentation state for the viewer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FairyView {
    pub slot: usize,
    pub state: crate::revival_fairy::FairyState,
    pub position: [f32; 3],
    pub effect_position: [f32; 3],
}

impl<C: SceneContent> SceneWorld<C> {
    /// Active fairies for drawing (effects already flow through
    /// `effect_commands`).
    pub fn fairy_views(&self) -> Vec<FairyView> {
        self.revival_fairies
            .iter()
            .enumerate()
            .filter(|(_, f)| f.active)
            .map(|(slot, f)| FairyView {
                slot,
                state: f.state,
                position: f.position,
                effect_position: f.effect_position,
            })
            .collect()
    }
}
