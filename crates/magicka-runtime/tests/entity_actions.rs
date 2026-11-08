use magicka_runtime::{
    ai_event::{AiEvent, AiEventLevel},
    animation_action::AnimationAction,
    avatar_move::*,
    level_variables::LevelVariables,
    scene_loading::custom_hash,
    scene_script::{ActionDefinition, SceneScriptLayout},
    script_runtime::*,
    spawn_action::SpawnLevel,
    spawn_phys::*,
};
use magicka_sim::damage_delivery::NetworkRole;

fn layout(actions: &str) -> Result<SceneScriptLayout, magicka_runtime::scene_script::LayoutError> {
    SceneScriptLayout::read(
        format!(
            r#"<Scene><trigger id="t" autorun="false"><then>{actions}</then></trigger></Scene>"#
        )
        .as_bytes(),
        false,
    )
}

#[derive(Default)]
struct Level {
    avatars: [Option<u16>; 4],
    ids: Vec<(i32, u16)>,
    max_hp: i32,
    next: u16,
    ops: Vec<String>,
}
impl AiEventLevel for Level {
    type Error = &'static str;
    fn locator(&self, _: i32) -> Option<[f32; 16]> {
        None
    }
    fn trigger_area_location(&mut self, _: i32) -> Result<Option<[f32; 3]>, Self::Error> {
        Ok(None)
    }
}
impl SpawnLevel for Level {
    fn load_character_template(&mut self, _: Option<&str>) -> Result<(), Self::Error> {
        Ok(())
    }
}
impl AvatarMoveLevel for Level {
    fn entity_by_id(&self, id: i32) -> Option<u16> {
        self.ids.iter().find(|(i, _)| *i == id).map(|(_, h)| *h)
    }
    fn player_avatars(&self) -> Vec<Option<u16>> {
        (self.avatars).to_vec()
    }
}
impl SpawnPhysLevel for Level {
    fn load_physics_template(&mut self, object: Option<&str>) -> Result<i32, Self::Error> {
        self.ops.push(format!("load {object:?}"));
        Ok(self.max_hp)
    }
    fn damageable_from_cache(&mut self) -> Result<u16, Self::Error> {
        self.next += 1;
        self.ops.push(format!("cache {}", self.next));
        Ok(self.next)
    }
    fn new_physics_entity(&mut self) -> Result<u16, Self::Error> {
        self.next += 1;
        self.ops.push(format!("new {}", self.next));
        Ok(self.next)
    }
}

fn avatar_move(xml: &str) -> AvatarMoveAction {
    match layout(xml).unwrap().actions.remove(0) {
        ActionDefinition::AvatarMove(a) => *a,
        other => panic!("{other:?}"),
    }
}
fn delays(events: &AvatarMoveEvents) -> Vec<Option<f32>> {
    events
        .iter()
        .map(|e| {
            e.as_ref().map(|l| match l[0] {
                AiEvent::Move(m) => m.delay,
                _ => -1.0,
            })
        })
        .collect()
}

#[test]
fn avatar_move_maps_player_ids_and_wraps_numbered_slots() {
    let mut level = Level {
        avatars: [Some(10), Some(11), None, Some(13)],
        ids: vec![(custom_hash("wizard_b"), 11), (custom_hash("npc"), 99)],
        ..Default::default()
    };
    // PlayerId fills slot 1 first; Player2 then wraps forward to slot 2.
    let a = avatar_move(
        r#"<avatarMove delay="0.5"><PlayerId name="Wizard_B"><move delay="1"/></PlayerId>
        <PlayerID name="npc"><move delay="9"/></PlayerID><PlayerId name="ghost"><move delay="8"/></PlayerId>
        <Player2><move delay="2"/></Player2><player4><move delay="4"/></player4><Player4><move delay="5"/></Player4></avatarMove>"#,
    );
    assert_eq!(a.delay, 0.5);
    let events = a.initialize(&mut level).unwrap();
    assert_eq!(
        delays(&events),
        [Some(5.0), Some(1.0), Some(2.0), Some(4.0)]
    );
    // A fifth numbered list cannot find a free slot.
    let full =
        avatar_move("<avatarMove><Player1/><Player2/><Player3/><Player4/><Player1/></avatarMove>");
    assert_eq!(
        full.initialize(&mut level),
        Err(AvatarMoveError::Unmappable("Player1".into()))
    );
    assert_eq!(
        avatar_move("<avatarMove><Player0/></avatarMove>").initialize(&mut level),
        Err(AvatarMoveError::SlotOutOfRange("Player0".into()))
    );
    for bad in [
        "<avatarMove><Wizard1x/></avatarMove>",
        "<avatarMove><Player1><run/></Player1></avatarMove>",
        "<avatarMove>text</avatarMove>",
    ] {
        assert!(layout(bad).is_err(), "{bad}");
    }
    // Player5 is numbered: slot index 4 wraps to slot 0.
    let wrapped = avatar_move("<avatarMove><Player5><move delay=\"7\"/></Player5></avatarMove>")
        .initialize(&mut level)
        .unwrap();
    assert_eq!(delays(&wrapped)[0], Some(7.0));
}

#[derive(Default)]
struct Host {
    avatars: [Option<u16>; 4],
    assigned: Vec<(usize, usize)>,
    hot_join: bool,
    hp: f32,
    ops: Vec<String>,
}
impl AvatarMoveHost for Host {
    type Error = &'static str;
    fn player_avatars(&self) -> Vec<Option<u16>> {
        (self.avatars).to_vec()
    }
    fn set_avatar_events(&mut self, player: usize, events: &[AiEvent]) -> Result<(), Self::Error> {
        self.assigned.push((player, events.len()));
        Ok(())
    }
}
impl SpawnPhysHost for Host {
    type Error = &'static str;
    fn is_hot_join_client(&self) -> bool {
        self.hot_join
    }
    fn return_damageable_to_cache(&mut self, h: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("return {h}"));
        Ok(())
    }
    fn locator(&mut self, area: i32) -> Result<[f32; 16], Self::Error> {
        self.ops.push("locator".into());
        (area == custom_hash("crate_spot"))
            .then_some([0.0; 16])
            .ok_or("No area")
    }
    fn initialize_entity(
        &mut self,
        h: u16,
        o: Option<&str>,
        _: [f32; 16],
        id: i32,
    ) -> Result<(), Self::Error> {
        self.ops
            .push(format!("init {h} {o:?} {}", id == custom_hash("box1")));
        Ok(())
    }
    fn set_damage_triggers(&mut self, h: u16, death: i32, _: i32) -> Result<(), Self::Error> {
        self.ops
            .push(format!("triggers {h} {}", death == custom_hash("broken")));
        Ok(())
    }
    fn add_entity(&mut self, h: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("add {h}"));
        Ok(())
    }
    fn hit_points(&mut self, _: u16) -> Result<f32, Self::Error> {
        Ok(self.hp)
    }
}

#[test]
fn avatar_move_execute_skips_players_without_avatars() {
    let mut events: AvatarMoveEvents = Default::default();
    events[0] = Some(vec![]);
    events[2] = Some(vec![]);
    let mut host = Host {
        avatars: [Some(1), Some(2), None, Some(4)],
        ..Default::default()
    };
    AvatarMoveAction::execute(&events, &mut host).unwrap();
    assert_eq!(host.assigned, [(0, 0)]);
}

fn spawn_phys(xml: &str) -> SpawnPhysAction {
    match layout(xml).unwrap().actions.remove(0) {
        ActionDefinition::SpawnPhys(a) => *a,
        other => panic!("{other:?}"),
    }
}

#[test]
fn spawn_phys_reuses_one_entity_and_tracks_damageable_health() {
    let a = spawn_phys(
        r#"<SpawnPhys object="Crate_Wood" area="crate_spot" id="Box1" onDeath="broken"/>"#,
    );
    assert_eq!(a.object.as_deref(), Some("crate_wood"));
    let mut level = Level {
        max_hp: 50,
        ..Default::default()
    };
    let mut state = SpawnPhysState::default();
    a.initialize(&mut state, &mut level).unwrap();
    assert_eq!(
        state.health,
        PhysHealth {
            handle: 1,
            health: 50.0
        }
    );
    let mut host = Host {
        hp: 20.0,
        ..Default::default()
    };
    a.execute(&mut state, &mut host).unwrap();
    a.execute(&mut state, &mut host).unwrap();
    assert_eq!(
        host.ops,
        [
            "locator",
            "init 1 Some(\"crate_wood\") true",
            "triggers 1 true",
            "add 1",
            "locator",
            "init 1 Some(\"crate_wood\") true",
            "triggers 1 true",
            "add 1"
        ]
    );
    a.track(&mut state, &mut host).unwrap();
    assert_eq!(state.health.health, 20.0);
    assert_eq!(
        read_tag(&write_tag(state.health)),
        Some(PhysHealth {
            handle: 65535,
            health: 20.0
        })
    );

    // Non-damageable: plain entity, no triggers, untracked default health.
    let mut level = Level {
        max_hp: 0,
        ..Default::default()
    };
    let mut plain = SpawnPhysState::default();
    a.initialize(&mut plain, &mut level).unwrap();
    assert_eq!(level.ops, ["load Some(\"crate_wood\")", "new 1"]);
    let mut host = Host::default();
    a.execute(&mut plain, &mut host).unwrap();
    assert!(!host.ops.iter().any(|o| o.starts_with("triggers")));
    assert_eq!(plain.health, PhysHealth::default());

    // Hot-join clients return damageable entities and leave the action without one.
    let mut host = Host {
        hot_join: true,
        ..Default::default()
    };
    a.execute(&mut state, &mut host).unwrap();
    assert_eq!(host.ops, ["return 1"]);
    assert_eq!(state.entity, None);
    host.hot_join = false;
    host.ops.clear();
    assert_eq!(
        a.execute(&mut state, &mut host),
        Err(SpawnPhysError::MissingEntity)
    );
    assert_eq!(
        host.ops,
        ["locator"],
        "GetLocator precedes the null dereference"
    );
}

struct World {
    host: Host,
}
impl ScriptWorld for World {
    type Error = &'static str;
    fn facts(&self) -> ScriptWorldFacts {
        ScriptWorldFacts::default()
    }
    fn avatar_move_host(&mut self) -> Option<&mut dyn AvatarMoveHost<Error = Self::Error>> {
        Some(&mut self.host)
    }
    fn spawn_phys_host(&mut self) -> Option<&mut dyn SpawnPhysHost<Error = Self::Error>> {
        Some(&mut self.host)
    }
    fn choose_group(&mut self, _: usize) -> Result<usize, Self::Error> {
        Ok(0)
    }
    fn notify_trigger(
        &mut self,
        _: &mut ScriptRuntime,
        _: i32,
        _: usize,
        _: Option<u16>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn animation(&mut self, _: &mut ScriptRuntime, _: &AnimationAction) -> Result<(), Self::Error> {
        Ok(())
    }
    fn action_executed(&mut self, _: &mut ScriptRuntime, _: usize) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn runtime_initializes_and_executes_entity_actions() {
    let mut rt = ScriptRuntime::new(
        layout(r#"<avatarMove><Player1><move/></Player1></avatarMove><spawnPhys object="barrel" area="crate_spot"/>"#)
            .unwrap(),
        LevelVariables::default(),
    );
    let mut world = World {
        host: Host {
            avatars: [Some(1), None, None, None],
            hp: 3.0,
            ..Default::default()
        },
    };
    rt.execute_trigger(
        custom_hash("t"),
        None,
        false,
        NetworkRole::Server,
        &mut world,
    )
    .unwrap();
    assert_eq!(
        rt.update_action(0, 0.0, NetworkRole::Server, &mut world),
        Err(ScriptRuntimeError::NotInitialized(0))
    );
    let mut level = Level {
        max_hp: 10,
        ..Default::default()
    };
    rt.initialize_actions(&mut level).unwrap();
    rt.trigger_action::<&str>(0).unwrap();
    rt.update_action(0, 0.0, NetworkRole::Server, &mut world)
        .unwrap();
    assert_eq!(world.host.assigned, [(0, 1)]);
    rt.update_action(1, 0.0, NetworkRole::Server, &mut world)
        .unwrap();
    assert_eq!(
        rt.spawn_phys_state(1).unwrap().health.health,
        3.0,
        "tracked before execute"
    );
    assert!(world.host.ops.contains(&"add 1".to_string()));
    assert_eq!(rt.history(), [0, 1]);
}

impl magicka_runtime::scene_flow::CheckpointLevel for Level {
    fn scene_locator(&mut self, _: i32) -> Result<[f32; 16], Self::Error> {
        Ok([0.0; 16])
    }
}
