use magicka_runtime::{
    ai_event::AiEvent,
    scene_flow::*,
    scene_loading::custom_hash,
    scene_script::{ActionDefinition, SceneScriptLayout},
};
use magicka_sim::damage_delivery::NetworkRole;

fn flow(xml: &str) -> Result<SceneFlowAction, magicka_runtime::scene_script::LayoutError> {
    let layout = SceneScriptLayout::read(
        format!(r#"<Scene><trigger id="t"><then>{xml}</then></trigger></Scene>"#).as_bytes(),
        false,
    )?;
    match layout.actions.into_iter().next().unwrap() {
        ActionDefinition::Flow(a) => Ok(*a),
        other => panic!("{other:?}"),
    }
}

#[test]
fn decoding_spawn_points_and_children() {
    let a = flow(r#"<changeScene scene="BF_S2" spawnPoint="Start" spawnPlayers="true" transition="none" transitionTime="2" saveNPCs="true"/>"#).unwrap();
    let SceneFlowKind::ChangeScene {
        spawn,
        transition,
        time,
        save_npcs,
    } = a.kind
    else {
        panic!()
    };
    assert_eq!(spawn.scene, custom_hash("bf_s2"));
    assert_eq!(
        spawn.locations,
        std::array::from_fn(|i| custom_hash(&format!("start{i}")))
    );
    assert!(spawn.spawn_players && save_npcs);
    assert_eq!((transition, time), (0, 2.0));
    assert!(matches!(
        flow(r#"<changeScene transition="invalid"/>"#).unwrap().kind,
        SceneFlowKind::ChangeScene { transition: -1, .. }
    ));
    let e = flow(r#"<enterScene scene="x" moveTarget="Door" instantTrigger="" trigger="arrived" ignoreTriggers="true"/>"#).unwrap();
    let SceneFlowKind::EnterScene {
        callback,
        transition,
        time,
        ..
    } = e.kind
    else {
        panic!()
    };
    assert_eq!((transition, time), (1, 1.0));
    assert_eq!(
        callback.move_targets,
        Some(std::array::from_fn(|i| custom_hash(&format!("door{i}"))))
    );
    assert_eq!(callback.instant_trigger, None);
    assert_eq!(callback.move_trigger, custom_hash("arrived"));
    let c = flow(r#"<triggerCheckpoint spawnPoint="cp"><!-- c --><Ignore trigger="Boss"/></triggerCheckpoint>"#).unwrap();
    let SceneFlowKind::TriggerCheckpoint {
        spawn_fairy,
        save_to_disk,
        ignore,
        ..
    } = c.kind
    else {
        panic!()
    };
    assert!(spawn_fairy && save_to_disk);
    assert_eq!(ignore, [custom_hash("boss")]);
    assert!(flow("<triggerCheckpoint><keep/></triggerCheckpoint>").is_err());
    assert!(matches!(
        flow(r#"<foundSecretArea id="Cave"/>"#).unwrap().kind,
        SceneFlowKind::FoundSecretArea { id: Some(ref s) } if s == "cave"
    ));
}

#[derive(Default)]
struct Host {
    role: Option<NetworkRole>,
    ops: Vec<String>,
    players: [FlowPlayer; 4],
    single: bool,
    scenes: Vec<(SpawnPoint, Option<EnterSceneCallback>)>,
}
impl SceneFlowHost for Host {
    type Error = &'static str;
    fn network_role(&self) -> NetworkRole {
        self.role.unwrap_or(NetworkRole::Offline)
    }
    fn send_scene_change(&mut self, enter: bool, _: SpawnPoint) -> Result<(), Self::Error> {
        self.ops.push(format!("send {enter}"));
        Ok(())
    }
    fn go_to_scene(
        &mut self,
        s: SpawnPoint,
        _: i32,
        _: f32,
        _: bool,
        c: Option<EnterSceneCallback>,
    ) -> Result<(), Self::Error> {
        self.ops.push("go".into());
        self.scenes.push((s, c));
        Ok(())
    }
    fn locator(&mut self, id: i32) -> Result<[f32; 16], Self::Error> {
        let mut m = [0.0; 16];
        m[12] = (id == custom_hash("cp1")) as i32 as f32 * 5.0;
        Ok(m)
    }
    fn show_checkpoint_text(&mut self) -> Result<(), Self::Error> {
        self.ops.push("text".into());
        Ok(())
    }
    fn players(&self) -> Vec<FlowPlayer> {
        self.players.to_vec()
    }
    fn single_player_non_versus(&self) -> bool {
        self.single
    }
    fn initialize_revival_fairy(&mut self, p: usize) -> Result<(), Self::Error> {
        self.ops.push(format!("fairy {p}"));
        Ok(())
    }
    fn update_checkpoint(
        &mut self,
        _: &[[f32; 16]; 4],
        ignore: &[i32],
        save: bool,
    ) -> Result<(), Self::Error> {
        self.ops.push(format!("checkpoint {} {save}", ignore.len()));
        Ok(())
    }
    fn play_ui_cue(&mut self, cue: i32) -> Result<(), Self::Error> {
        assert_eq!(cue, custom_hash("ui_checkpoint01"));
        self.ops.push("cue".into());
        Ok(())
    }
    fn revive_player(&mut self, p: usize, at: [f32; 3]) -> Result<(), Self::Error> {
        self.ops.push(format!("revive {p} {}", at[0]));
        Ok(())
    }
    fn heal_player(&mut self, p: usize, m: f32) -> Result<(), Self::Error> {
        self.ops.push(format!("heal {p} {m}"));
        Ok(())
    }
    fn found_secret_area(&mut self, id: Option<&str>) -> Result<(), Self::Error> {
        self.ops.push(format!("secret {id:?}"));
        Ok(())
    }
    fn end_game(&mut self, condition: u8, freeze: bool, phony: bool) -> Result<(), Self::Error> {
        self.ops
            .push(format!("endgame {condition} {freeze} {phony}"));
        Ok(())
    }
}
const ALIVE: FlowPlayer = FlowPlayer {
    playing: true,
    avatar: Some(FlowAvatar {
        dead: false,
        staff_of_war: false,
    }),
};

#[test]
fn execute_paths_and_checkpoint_order() {
    let mut host = Host {
        role: Some(NetworkRole::Server),
        ..Default::default()
    };
    let mut change = flow(r#"<changeScene scene="a" spawnPoint="s"/>"#).unwrap();
    change.execute(None, &mut host).unwrap();
    let mut enter = flow(r#"<enterScene scene="b" spawnPoint="s"/>"#).unwrap();
    enter.execute(None, &mut host).unwrap();
    assert_eq!(host.ops, ["send false", "go", "send true", "go"]);
    assert!(!host.scenes[0].0.spawn_players);
    assert!(
        host.scenes[1].0.spawn_players,
        "EnterScene forces SpawnPlayers"
    );
    assert!(host.scenes[1].1.is_some());
    // The forced flag persists on the action instance.
    assert!(matches!(
        enter.kind,
        SceneFlowKind::EnterScene {
            spawn: SpawnPoint {
                spawn_players: true,
                ..
            },
            ..
        }
    ));
    let mut client = Host {
        role: Some(NetworkRole::Client),
        ..Default::default()
    };
    change.execute(None, &mut client).unwrap();
    assert!(client.ops.is_empty());

    let mut cp = flow(r#"<triggerCheckpoint spawnPoint="cp" saveToDisk="false"><ignore trigger="x"/></triggerCheckpoint>"#).unwrap();
    assert_eq!(
        cp.execute(None, &mut host),
        Err(SceneFlowError::NotInitialized)
    );
    let dead = FlowPlayer {
        playing: true,
        avatar: Some(FlowAvatar {
            dead: true,
            staff_of_war: false,
        }),
    };
    let staff = FlowPlayer {
        playing: true,
        avatar: Some(FlowAvatar {
            dead: false,
            staff_of_war: true,
        }),
    };
    let mut host = Host {
        players: [ALIVE, dead, FlowPlayer::default(), staff],
        single: true,
        ..Default::default()
    };
    struct L;
    impl magicka_runtime::ai_event::AiEventLevel for L {
        type Error = &'static str;
        fn locator(&self, _: i32) -> Option<[f32; 16]> {
            None
        }
        fn trigger_area_location(&mut self, _: i32) -> Result<Option<[f32; 3]>, Self::Error> {
            Ok(None)
        }
    }
    impl CheckpointLevel for L {
        fn scene_locator(&mut self, _: i32) -> Result<[f32; 16], Self::Error> {
            Ok([1.0; 16])
        }
    }
    let matrices = cp.initialize(&mut L).unwrap().unwrap();
    cp.execute(Some(&matrices), &mut host).unwrap();
    assert_eq!(
        host.ops,
        [
            "text",
            "fairy 0",
            "checkpoint 1 false",
            "cue",
            "heal 0 1",
            "revive 1 5",
            "heal 3 1",
            "heal 3 2"
        ]
    );
    let mut no_avatar = Host {
        players: [
            FlowPlayer {
                playing: true,
                avatar: None,
            },
            FlowPlayer::default(),
            FlowPlayer::default(),
            FlowPlayer::default(),
        ],
        ..Default::default()
    };
    assert_eq!(
        cp.execute(Some(&matrices), &mut no_avatar),
        Err(SceneFlowError::MissingAvatar(0))
    );
}

#[derive(Default)]
struct Entered {
    ops: Vec<String>,
}
impl SceneEnteredHost for Entered {
    type Error = &'static str;
    fn players(&self) -> Vec<FlowPlayer> {
        vec![ALIVE, FlowPlayer::default(), ALIVE, ALIVE]
    }
    fn try_locator(&mut self, id: i32) -> Result<Option<[f32; 16]>, Self::Error> {
        let mut m = [0.0; 16];
        m[10] = -1.0;
        m[12] = 3.0;
        Ok((id != custom_hash("door3")).then_some(m))
    }
    fn set_avatar_events(&mut self, p: usize, events: &[AiEvent]) -> Result<(), Self::Error> {
        let AiEvent::Move(m) = events[0] else {
            panic!()
        };
        self.ops.push(format!(
            "events {p} {:?} {:?} {}",
            m.waypoint, m.direction, m.fixed_direction
        ));
        Ok(())
    }
    fn set_ignore_triggers(&mut self, p: usize, v: bool) -> Result<(), Self::Error> {
        self.ops.push(format!("ignore {p} {v}"));
        Ok(())
    }
    fn execute_trigger(&mut self, id: i32) -> Result<(), Self::Error> {
        self.ops
            .push(format!("trigger {}", id == custom_hash("go")));
        Ok(())
    }
}

#[test]
fn enter_scene_callback_assigns_moves_then_instant_trigger() {
    let e = flow(r#"<enterScene moveTarget="door" instantTrigger="go" facingDirection="true" ignoreTriggers="true"/>"#).unwrap();
    let SceneFlowKind::EnterScene { callback, .. } = e.kind else {
        panic!()
    };
    let mut host = Entered::default();
    callback.on_scene_changed(&mut host).unwrap();
    assert_eq!(
        host.ops,
        [
            "events 0 [3.0, 0.0, 0.0] [-0.0, -0.0, 1.0] true",
            "ignore 0 true",
            "events 2 [3.0, 0.0, 0.0] [-0.0, -0.0, 1.0] true",
            "ignore 2 true",
            "trigger true"
        ]
    );
}

#[test]
fn endgame_reads_type_freeze_phony_and_delay() {
    let a = flow(r#"<endgame type="LevelComplete" phony="true" freezeGame="false" delay="1.66"/>"#)
        .unwrap();
    assert_eq!(a.delay, 1.66);
    assert_eq!(
        a.kind,
        SceneFlowKind::EndGame {
            condition: 2,
            freeze: false,
            phony: true
        }
    );
    let mut d = flow(r#"<endGame type="Defeat"/>"#).unwrap();
    let mut host = Host::default();
    d.execute(None, &mut host).unwrap();
    assert_eq!(host.ops, ["endgame 3 true false"]);
}
