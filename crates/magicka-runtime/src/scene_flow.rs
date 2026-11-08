//! ChangeScene, EnterScene, TriggerCheckpoint, FoundSecretArea and EndGame:
//! level-flow trigger actions. Scene loading itself belongs to the Level host.
use crate::{
    ai_event::{clr_enum_signed, AiEvent, MoveEvent},
    scene_loading::custom_hash,
    scene_script::XmlElement,
    script_attributes::{boolean, identifier, number, ScriptReadError},
};
use magicka_sim::damage_delivery::NetworkRole;

pub const TRANSITIONS: &[(&str, i32)] = &[
    ("invalid", -1),
    ("none", 0),
    ("fade", 1),
    ("crossfade", 2),
    ("numberoftransitions", 3),
];

/// SpawnPoint: scene hash, four "<name><i>" locator hashes, SpawnPlayers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpawnPoint {
    pub scene: i32,
    pub locations: [i32; 4],
    pub spawn_players: bool,
}

fn numbered(name: &str) -> Result<[i32; 4], ScriptReadError> {
    if !name.is_ascii() {
        return Err(ScriptReadError::UnsupportedName(name.into()));
    }
    let lower = name.to_ascii_lowercase();
    Ok(std::array::from_fn(|i| custom_hash(&format!("{lower}{i}"))))
}

/// EnterScene.OnSceneChanged data, run by the Level host after the switch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnterSceneCallback {
    /// `None` when MoveTarget is null or empty.
    pub move_targets: Option<[i32; 4]>,
    pub facing_direction: bool,
    pub move_trigger: i32,
    pub ignore_triggers: bool,
    /// `None` when InstantTrigger is null or empty.
    pub instant_trigger: Option<i32>,
}

/// Level.GoToScene arguments, held until the Level host switches scenes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingScene {
    pub spawn: SpawnPoint,
    /// Transitions value (`TRANSITIONS`).
    pub transition: i32,
    pub time: f32,
    pub save_npcs: bool,
    pub callback: Option<EnterSceneCallback>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SceneFlowKind {
    ChangeScene {
        spawn: SpawnPoint,
        transition: i32,
        time: f32,
        save_npcs: bool,
    },
    EnterScene {
        spawn: SpawnPoint,
        transition: i32,
        time: f32,
        save_npcs: bool,
        callback: EnterSceneCallback,
    },
    TriggerCheckpoint {
        spawn_points: [i32; 4],
        spawn_fairy: bool,
        save_to_disk: bool,
        ignore: Vec<i32>,
    },
    /// Lowercased ID; `None` passes null.
    FoundSecretArea { id: Option<String> },
    /// PlayState.Endgame(Type, FreezeGame, Phony, 0). Defaults: freeze
    /// true, phony false, type None. QuickExecute is empty.
    EndGame {
        condition: u8,
        freeze: bool,
        phony: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneFlowAction {
    pub delay: f32,
    pub kind: SceneFlowKind,
}

pub const TAGS: &[&str] = &[
    "changescene",
    "enterscene",
    "triggercheckpoint",
    "foundsecretarea",
    "endgame",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlowAvatar {
    pub dead: bool,
    /// Equipment[1] item type is "staff_of_war".
    pub staff_of_war: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlowPlayer {
    pub playing: bool,
    pub avatar: Option<FlowAvatar>,
}

pub trait SceneFlowHost {
    type Error;
    fn network_role(&self) -> NetworkRole;
    /// Server-side TriggerActionMessage and SetAllClientsBusy.
    fn send_scene_change(&mut self, enter: bool, spawn: SpawnPoint) -> Result<(), Self::Error>;
    /// Level.GoToScene(spawn, transition, time, saveNPCs, callback).
    fn go_to_scene(
        &mut self,
        spawn: SpawnPoint,
        transition: i32,
        time: f32,
        save_npcs: bool,
        callback: Option<EnterSceneCallback>,
    ) -> Result<(), Self::Error>;
    /// GameScene.GetLocator(int, out Matrix) (missing key throws).
    fn locator(&mut self, id: i32) -> Result<[f32; 16], Self::Error>;
    /// "#add_checkpoint" text box (Maiandra18, 2 s) added to DialogManager.
    fn show_checkpoint_text(&mut self) -> Result<(), Self::Error>;
    fn players(&self) -> Vec<FlowPlayer>;
    /// ConnectedPlayerCount == 1 and GameType != Versus.
    fn single_player_non_versus(&self) -> bool;
    /// Avatar.RevivalFairy.Initialize(playState, showGreeting: true).
    fn initialize_revival_fairy(&mut self, player: usize) -> Result<(), Self::Error>;
    /// PlayState.UpdateCheckPoint(matrices, ignoreList, saveToDisk).
    fn update_checkpoint(
        &mut self,
        matrices: &[[f32; 16]; 4],
        ignore: &[i32],
        save: bool,
    ) -> Result<(), Self::Error>;
    /// AudioManager.PlayCue(Banks.UI, hash("ui_checkpoint01")).
    fn play_ui_cue(&mut self, cue: i32) -> Result<(), Self::Error>;
    /// Revive.GetInstance, SetSpecificPlayer, Execute at the position.
    fn revive_player(&mut self, player: usize, position: [f32; 3]) -> Result<(), Self::Error>;
    /// Avatar.HitPoints = MaxHitPoints * multiplier.
    fn heal_player(&mut self, player: usize, multiplier: f32) -> Result<(), Self::Error>;
    /// Profile.FoundSecretArea(playState, id).
    fn found_secret_area(&mut self, id: Option<&str>) -> Result<(), Self::Error>;
    /// PlayState.Endgame(condition, freeze, phony, 0).
    fn end_game(&mut self, condition: u8, freeze: bool, phony: bool) -> Result<(), Self::Error>;
}

/// Initialize-time GameScene.GetLocator for TriggerCheckpoint.
pub trait CheckpointLevel: crate::ai_event::AiEventLevel {
    fn scene_locator(&mut self, id: i32) -> Result<[f32; 16], Self::Error>;
}

/// Hosts for EnterScene.OnSceneChanged in the new current scene.
pub trait SceneEnteredHost {
    type Error;
    fn players(&self) -> Vec<FlowPlayer>;
    /// CurrentScene.TryGetLocator.
    fn try_locator(&mut self, id: i32) -> Result<Option<[f32; 16]>, Self::Error>;
    fn set_avatar_events(&mut self, player: usize, events: &[AiEvent]) -> Result<(), Self::Error>;
    fn set_ignore_triggers(&mut self, player: usize, ignore: bool) -> Result<(), Self::Error>;
    /// CurrentScene.ExecuteTrigger(id, null, ignoreConditions: false).
    fn execute_trigger(&mut self, id: i32) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum SceneFlowError<E> {
    /// TriggerCheckpoint executed before Initialize resolved its locators.
    NotInitialized,
    /// Avatar null while playing and not dead (the original dereferences it).
    MissingAvatar(usize),
    Host(E),
}
impl<E> From<E> for SceneFlowError<E> {
    fn from(e: E) -> Self {
        Self::Host(e)
    }
}

impl EnterSceneCallback {
    pub fn on_scene_changed<H: SceneEnteredHost + ?Sized>(
        &self,
        host: &mut H,
    ) -> Result<(), H::Error> {
        if let Some(targets) = self.move_targets {
            let players = host.players();
            for (i, player) in players.iter().enumerate() {
                if !(player.playing && player.avatar.is_some()) {
                    continue;
                }
                let Some(m) = host.try_locator(targets[i])? else {
                    continue;
                };
                let event = AiEvent::Move(MoveEvent {
                    waypoint: [m[12], m[13], m[14]],
                    waypoint_id: 0,
                    direction: [-m[8], -m[9], -m[10]],
                    fixed_direction: self.facing_direction,
                    delay: 0.0,
                    speed: 1.0,
                    trigger: self.move_trigger,
                });
                host.set_avatar_events(i, &[event])?;
                host.set_ignore_triggers(i, self.ignore_triggers)?;
            }
        }
        if let Some(id) = self.instant_trigger {
            host.execute_trigger(id)?;
        }
        Ok(())
    }
}

impl SceneFlowAction {
    pub fn read(node: &XmlElement) -> Result<Self, ScriptReadError> {
        use SceneFlowKind::*;
        let mut kind = match node.name.to_ascii_lowercase().as_str() {
            "changescene" => ChangeScene {
                spawn: SpawnPoint::default(),
                transition: 1,
                time: 1.0,
                save_npcs: false,
            },
            "enterscene" => EnterScene {
                spawn: SpawnPoint::default(),
                transition: 1,
                time: 1.0,
                save_npcs: false,
                callback: EnterSceneCallback {
                    move_targets: None,
                    facing_direction: false,
                    move_trigger: 0,
                    ignore_triggers: false,
                    instant_trigger: None,
                },
            },
            "triggercheckpoint" => {
                // The constructor accepts only <ignore trigger="..."/> children.
                if node.non_comment_text_children {
                    return Err(ScriptReadError::UnsupportedTag("#text".into()));
                }
                let mut ignore = vec![];
                for child in &node.children {
                    if !child.name.eq_ignore_ascii_case("ignore") {
                        return Err(ScriptReadError::UnsupportedTag(child.name.clone()));
                    }
                    for (k, v) in &child.attributes {
                        if k.eq_ignore_ascii_case("trigger") {
                            ignore.push(identifier(v)?);
                        }
                    }
                }
                TriggerCheckpoint {
                    spawn_points: [0; 4],
                    spawn_fairy: true,
                    save_to_disk: true,
                    ignore,
                }
            }
            "foundsecretarea" => FoundSecretArea { id: None },
            "endgame" => EndGame {
                condition: 0,
                freeze: true,
                phony: false,
            },
            _ => return Err(ScriptReadError::UnsupportedTag(node.name.clone())),
        };
        let mut delay = 0.0;
        for (key, value) in &node.attributes {
            let key = key.as_str();
            match (&mut kind, key.to_ascii_lowercase().as_str()) {
                (_, "delay") => delay = number(value, key)?,
                (ChangeScene { spawn, .. }, "scene") | (EnterScene { spawn, .. }, "scene") => {
                    spawn.scene = identifier(value)?
                }
                (ChangeScene { spawn, .. }, "spawnpoint")
                | (EnterScene { spawn, .. }, "spawnpoint") => spawn.locations = numbered(value)?,
                (ChangeScene { spawn, .. }, "spawnplayers") => {
                    spawn.spawn_players = boolean(value, key)?
                }
                (ChangeScene { transition, .. }, "transition")
                | (EnterScene { transition, .. }, "transition") => {
                    *transition = clr_enum_signed(value, TRANSITIONS, key)?
                }
                (ChangeScene { time, .. }, "transitiontime")
                | (EnterScene { time, .. }, "transitiontime") => *time = number(value, key)?,
                (ChangeScene { save_npcs, .. }, "savenpcs")
                | (EnterScene { save_npcs, .. }, "savenpcs") => *save_npcs = boolean(value, key)?,
                (EnterScene { callback, .. }, "movetarget") => {
                    callback.move_targets = if value.is_empty() {
                        None
                    } else {
                        Some(numbered(value)?)
                    }
                }
                (EnterScene { callback, .. }, "facingdirection") => {
                    callback.facing_direction = boolean(value, key)?
                }
                (EnterScene { callback, .. }, "instanttrigger") => {
                    callback.instant_trigger =
                        (!value.is_empty()).then(|| identifier(value)).transpose()?
                }
                (EnterScene { callback, .. }, "trigger") => {
                    callback.move_trigger = identifier(value)?
                }
                (EnterScene { callback, .. }, "ignoretriggers") => {
                    callback.ignore_triggers = boolean(value, key)?
                }
                (TriggerCheckpoint { spawn_points, .. }, "spawnpoint") => {
                    *spawn_points = numbered(value)?
                }
                (TriggerCheckpoint { spawn_fairy, .. }, "spawnfairy") => {
                    *spawn_fairy = boolean(value, key)?
                }
                (TriggerCheckpoint { save_to_disk, .. }, "savetodisk") => {
                    *save_to_disk = boolean(value, key)?
                }
                (EndGame { condition, .. }, "type") => {
                    *condition = clr_enum_signed(value, crate::end_game::CONDITIONS, key)? as u8
                }
                (EndGame { freeze, .. }, "freezegame") => *freeze = boolean(value, key)?,
                (EndGame { phony, .. }, "phony") => *phony = boolean(value, key)?,
                (FoundSecretArea { id }, "id") => {
                    if !value.is_ascii() {
                        return Err(ScriptReadError::UnsupportedName(value.into()));
                    }
                    *id = Some(value.to_ascii_lowercase())
                }
                _ => {}
            }
        }
        Ok(Self { delay, kind })
    }

    /// TriggerCheckpoint.Initialize: four GetLocator calls (random for areas).
    pub fn initialize<L: CheckpointLevel + ?Sized>(
        &self,
        level: &mut L,
    ) -> Result<Option<[[f32; 16]; 4]>, L::Error> {
        let SceneFlowKind::TriggerCheckpoint { spawn_points, .. } = &self.kind else {
            return Ok(None);
        };
        let mut out = [[0.0; 16]; 4];
        for (m, id) in out.iter_mut().zip(spawn_points) {
            *m = level.scene_locator(*id)?;
        }
        Ok(Some(out))
    }

    /// Execute. EnterScene permanently sets its SpawnPlayers flag first.
    /// All four QuickExecute overrides are empty.
    pub fn execute<H: SceneFlowHost + ?Sized>(
        &mut self,
        checkpoint: Option<&[[f32; 16]; 4]>,
        host: &mut H,
    ) -> Result<(), SceneFlowError<H::Error>> {
        match &mut self.kind {
            SceneFlowKind::ChangeScene {
                spawn,
                transition,
                time,
                save_npcs,
            } => {
                if host.network_role() == NetworkRole::Client {
                    return Ok(());
                }
                if host.network_role() == NetworkRole::Server {
                    host.send_scene_change(false, *spawn)?;
                }
                host.go_to_scene(*spawn, *transition, *time, *save_npcs, None)?;
            }
            SceneFlowKind::EnterScene {
                spawn,
                transition,
                time,
                save_npcs,
                callback,
            } => {
                if host.network_role() == NetworkRole::Client {
                    return Ok(());
                }
                spawn.spawn_players = true;
                if host.network_role() == NetworkRole::Server {
                    host.send_scene_change(true, *spawn)?;
                }
                host.go_to_scene(*spawn, *transition, *time, *save_npcs, Some(*callback))?;
            }
            SceneFlowKind::TriggerCheckpoint {
                spawn_points,
                spawn_fairy,
                save_to_disk,
                ignore,
            } => {
                let matrices = checkpoint.ok_or(SceneFlowError::NotInitialized)?;
                host.show_checkpoint_text()?;
                let players = host.players();
                if *spawn_fairy && host.single_player_non_versus() {
                    if let Some(i) = players.iter().position(|p| p.playing) {
                        if players[i].avatar.is_none() {
                            return Err(SceneFlowError::MissingAvatar(i));
                        }
                        host.initialize_revival_fairy(i)?;
                    }
                }
                host.update_checkpoint(matrices, ignore, *save_to_disk)?;
                host.play_ui_cue(custom_hash("ui_checkpoint01"))?;
                for (i, player) in players.iter().enumerate() {
                    if !player.playing {
                        continue;
                    }
                    match player.avatar {
                        Some(FlowAvatar { dead: true, .. }) => {
                            if host.network_role() != NetworkRole::Client {
                                let m = host.locator(spawn_points[i])?;
                                host.revive_player(i, [m[12], m[13], m[14]])?;
                            }
                        }
                        Some(FlowAvatar { staff_of_war, .. }) => {
                            host.heal_player(i, 1.0)?;
                            if staff_of_war {
                                host.heal_player(i, 2.0)?;
                            }
                        }
                        None => return Err(SceneFlowError::MissingAvatar(i)),
                    }
                }
            }
            SceneFlowKind::FoundSecretArea { id } => host.found_secret_area(id.as_deref())?,
            SceneFlowKind::EndGame {
                condition,
                freeze,
                phony,
            } => host.end_game(*condition, *freeze, *phony)?,
        }
        Ok(())
    }
}
