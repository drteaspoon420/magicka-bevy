use magicka_runtime::{
    ai_event::*,
    animation_action::AnimationAction,
    level_variables::LevelVariables,
    scene_loading::custom_hash,
    scene_script::{ActionDefinition, LayoutError, SceneScriptLayout},
    script_runtime::*,
    spawn_action::*,
};
use magicka_sim::damage_delivery::NetworkRole;
use std::collections::BTreeMap;

fn layout(actions: &str) -> Result<SceneScriptLayout, LayoutError> {
    SceneScriptLayout::read(
        format!(r#"<Scene><trigger id="root"><then>{actions}</then></trigger></Scene>"#).as_bytes(),
        false,
    )
}
fn spawn(actions: &str) -> SpawnAction {
    match layout(actions).unwrap().actions.remove(0) {
        ActionDefinition::Spawn(s) => *s,
        other => panic!("{other:?}"),
    }
}

#[test]
fn attribute_defaults_enums_and_setter_rules() {
    let s = spawn(r#"<spawn type="Goblin_Spear"/>"#);
    assert_eq!(s.type_name.as_deref(), Some("goblin_spear"));
    assert_eq!(s.type_id, custom_hash("goblin_spear"));
    assert_eq!(
        (s.amount, s.health, s.order, s.reaction, s.react_to),
        (1, 1.0, 2, 2, 3)
    );
    assert_eq!((s.mesh, s.priority_ability, s.wander_speed), (-1, -1, 1.0));
    assert_eq!(
        (s.area, s.unique_id, s.target_area, s.delay),
        (0, 0, 0, 0.0)
    );
    let s = spawn(
        r#"<Spawn TYPE="a" nr="3" order="Wander" reactTo="proximity, attack" reaction="4"
        animation="SPEC_ACTION0" spawnAnimation="revive" idleAnimation="hit_slide" area="Spawn_A"
        targetArea="ANY" health="0.25" delay="1.5" delayed="ignored" unknown="x" meshId="1"
        snapToNavMesh="true" neverRemove="True" onDeath="Dead" forceNavMesh="true"/>"#,
    );
    assert_eq!((s.amount, s.order, s.react_to, s.reaction), (3, 5, 3, 4));
    assert_eq!(
        (s.animation, s.spawn_animation, s.idle_animation),
        (180, 150, 23)
    );
    assert_eq!(s.area, custom_hash("spawn_a"));
    assert_eq!(s.target_area, 0);
    assert_eq!((s.health, s.delay, s.mesh), (0.25, 1.5, 1));
    assert!(s.snap_to_nav_mesh && s.never_remove && s.force_nav_mesh);
    assert_eq!(s.on_death, custom_hash("dead"));
    // Health's setter rejects out-of-range values but NaN passes its comparisons.
    assert!(spawn(r#"<spawn health="NaN"/>"#).health.is_nan());
    for bad in [
        r#"<spawn health="1.5"/>"#,
        r#"<spawn order="charge"/>"#,
        r#"<spawn reactto="256"/>"#,
        r#"<spawn animation="not_an_animation"/>"#,
        r#"<spawn nr="many"/>"#,
        r#"<spawn><Run/></spawn>"#,
        r#"<spawn><move position="x" unknown="1"/></spawn>"#,
        r#"<spawn>text</spawn>"#,
    ] {
        assert!(layout(bad).is_err(), "{bad}");
    }
}

#[test]
fn ai_events_decode_defaults_and_normalization() {
    let s = spawn(
        r#"<spawn><!-- skipped --><Move position="Gate" speed="0.5" trigger="Arrived"/>
        <move position="1,2,3" facingDirection="0,0,2" delay="1"/><face target="Boss"/>
        <animation animation="idle" blendTime="0.25"/><kill/><loop type="REVERSE"/></spawn>"#,
    );
    let events = AiEvent::read_children(&s.node).unwrap();
    let [AiEvent::Move(a), AiEvent::Move(b), AiEvent::Face(f), AiEvent::Animation(n), AiEvent::Kill(k), AiEvent::Loop(l)] =
        events.as_slice()
    else {
        panic!("{events:?}")
    };
    assert_eq!(
        (a.waypoint_id, a.speed, a.trigger),
        (custom_hash("gate"), 0.5, custom_hash("arrived"))
    );
    assert_eq!((b.waypoint, b.waypoint_id), ([1.0, 2.0, 3.0], 0));
    assert!(b.fixed_direction);
    assert_eq!(b.direction, [0.0, 0.0, 1.0]);
    assert_eq!((f.target_id, f.speed), (custom_hash("boss"), 1.0));
    assert_eq!((n.animation, n.blend_time), (1, 0.25));
    assert!(k.remove);
    assert_eq!(l.kind, LoopType::Reverse);
    assert!(xna_normalize([0.0; 3])[0].is_nan());
}

struct Level {
    locators: BTreeMap<i32, [f32; 16]>,
    area_draws: Vec<i32>,
    loads: Vec<Option<String>>,
}
impl AiEventLevel for Level {
    type Error = &'static str;
    fn locator(&self, id: i32) -> Option<[f32; 16]> {
        self.locators.get(&id).copied()
    }
    fn trigger_area_location(&mut self, id: i32) -> Result<Option<[f32; 3]>, Self::Error> {
        self.area_draws.push(id);
        Ok((id == custom_hash("yard")).then_some([self.area_draws.len() as f32, 0.0, 0.0]))
    }
}
impl SpawnLevel for Level {
    fn load_character_template(&mut self, name: Option<&str>) -> Result<(), Self::Error> {
        self.loads.push(name.map(Into::into));
        name.map(|_| ()).ok_or("Data/Characters/ not found")
    }
}
fn locator(translation: [f32; 3], forward: [f32; 3]) -> [f32; 16] {
    let mut m = [0.0; 16];
    m[0] = 1.0;
    m[5] = 1.0;
    m[8..11].copy_from_slice(&forward.map(|v| -v));
    m[12..15].copy_from_slice(&translation);
    m[15] = 1.0;
    m
}
fn level() -> Level {
    Level {
        locators: [(
            custom_hash("gate"),
            locator([5.0, 0.0, 7.0], [1.0, 0.0, 0.0]),
        )]
        .into(),
        area_draws: vec![],
        loads: vec![],
    }
}

#[test]
fn initialize_resolves_move_lookups_in_attribute_order() {
    let s = spawn(
        r#"<spawn type="t"><move position="gate" facingDirection="true"/>
        <move facingDirection="0,3,0" position="gate"/><move position="yard"/>
        <move position="nowhere"/><move position="gate"/></spawn>"#,
    );
    let mut level = level();
    let mut state = SpawnState::default();
    s.initialize(&mut state, &mut level).unwrap();
    let events = state.events.clone().unwrap();
    let moves: Vec<_> = events
        .iter()
        .map(|e| match e {
            AiEvent::Move(m) => *m,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        (moves[0].waypoint, moves[0].direction),
        ([5.0, 0.0, 7.0], [1.0, 0.0, 0.0])
    );
    assert!(moves[0].fixed_direction);
    // A later locator overwrites an earlier facing triple; FixedDirection stays.
    assert_eq!(moves[1].direction, [1.0, 0.0, 0.0]);
    assert!(moves[1].fixed_direction);
    assert_eq!(moves[2].waypoint, [1.0, 0.0, 0.0]);
    assert_eq!(
        (moves[3].waypoint, moves[3].waypoint_id),
        ([0.0; 3], custom_hash("nowhere"))
    );
    assert_eq!(level.loads, [Some("t".to_string())]);
    // Each Initialize redraws trigger-area positions.
    s.initialize(&mut state, &mut level).unwrap();
    let AiEvent::Move(m) = state.events.as_ref().unwrap()[2] else {
        unreachable!()
    };
    assert_eq!(m.waypoint, [3.0, 0.0, 0.0]);
    let missing_type = spawn("<spawn/>");
    assert_eq!(
        missing_type.initialize(&mut SpawnState::default(), &mut level),
        Err(ResolveError::Level("Data/Characters/ not found"))
    );
}

/// Recorded (hit points, wander speed, force camera, desired direction, remove after death).
type ConfigRecord = (Option<f32>, Option<f32>, Option<bool>, [f32; 3], bool);
#[derive(Default)]
struct Host {
    role: Option<NetworkRole>,
    ops: Vec<String>,
    next_handle: u16,
    bosses: Vec<u16>,
    hit_points: BTreeMap<u16, f32>,
    resurrecting: Vec<u16>,
    survival: bool,
    orders: Vec<(u8, u8, u8, usize)>,
    configs: Vec<ConfigRecord>,
    messages: Vec<SpawnMessage>,
}
impl SpawnHost for Host {
    type Error = &'static str;
    fn network_role(&self) -> NetworkRole {
        self.role.unwrap_or(NetworkRole::Server)
    }
    fn template_move_abilities(&mut self, type_id: i32) -> Result<i32, Self::Error> {
        self.ops.push("template".into());
        (type_id != 0).then_some(4).ok_or("no template")
    }
    fn acquire_npc(&mut self) -> Result<NpcInstance, Self::Error> {
        self.next_handle += 1;
        self.ops.push(format!("acquire {}", self.next_handle));
        Ok(NpcInstance {
            handle: self.next_handle,
            boss: self.bosses.contains(&self.next_handle),
        })
    }
    fn locator(&mut self, area: i32) -> Result<[f32; 16], Self::Error> {
        self.ops.push("locator".into());
        (area == custom_hash("gate"))
            .then(|| locator([5.0, 1.0, 7.0], [0.0, 0.0, -1.0]))
            .ok_or("No area")
    }
    fn nearest_nav_position(
        &mut self,
        p: [f32; 3],
        abilities: i32,
    ) -> Result<[f32; 3], Self::Error> {
        self.ops.push(format!("nav {abilities}"));
        Ok([p[0], 0.0, p[2]])
    }
    fn initialize_npc(
        &mut self,
        npc: u16,
        _: i32,
        mesh: i32,
        p: [f32; 3],
        id: i32,
    ) -> Result<f32, Self::Error> {
        self.ops.push(format!(
            "init {npc} mesh {mesh} at {p:?} id {}",
            id == custom_hash("guard")
        ));
        self.hit_points.insert(npc, 200.0);
        Ok(200.0)
    }
    fn force_draw(&mut self, npc: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("force_draw {npc}"));
        Ok(())
    }
    fn configure(&mut self, npc: u16, c: &SpawnConfig<'_>) -> Result<(), Self::Error> {
        self.ops.push(format!("configure {npc}"));
        if let Some(hp) = c.hit_points {
            self.hit_points.insert(npc, hp);
        }
        self.orders.push((
            c.order.order,
            c.order.react_to,
            c.order.reaction,
            c.order.events.len(),
        ));
        self.configs.push((
            c.hit_points,
            c.wander_speed,
            c.force_camera,
            c.desired_direction,
            c.remove_after_death,
        ));
        assert_eq!(&c.orientation[12..15], &[0.0; 3]);
        Ok(())
    }
    fn force_animation(&mut self, npc: u16, a: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("animation {npc} {a}"));
        Ok(())
    }
    fn begin_resurrection(&mut self, npc: u16, a: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("resurrect {npc} {a}"));
        self.resurrecting.push(npc);
        Ok(())
    }
    fn set_special_idle(&mut self, npc: u16, a: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("special_idle {npc} {a}"));
        Ok(())
    }
    fn in_resurrection_state(&mut self, npc: u16) -> Result<bool, Self::Error> {
        Ok(self.resurrecting.contains(&npc))
    }
    fn survival_ruleset(&self) -> bool {
        self.survival
    }
    fn survival_added(&mut self, npc: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("survival {npc}"));
        Ok(())
    }
    fn add_entity(&mut self, npc: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("add {npc}"));
        Ok(())
    }
    fn set_hit_points(&mut self, npc: u16, hp: f32) -> Result<(), Self::Error> {
        self.ops.push(format!("hp {npc} {hp}"));
        self.hit_points.insert(npc, hp);
        Ok(())
    }
    fn hit_points(&mut self, handle: u16) -> Result<Option<f32>, Self::Error> {
        Ok(self.hit_points.get(&handle).copied())
    }
    fn send_spawn(&mut self, m: &SpawnMessage) -> Result<(), Self::Error> {
        self.ops.push(format!("send {}", m.handle));
        self.messages.push(*m);
        Ok(())
    }
}

fn initialized(xml: &str) -> (SpawnAction, SpawnState) {
    let s = spawn(xml);
    let mut state = SpawnState::default();
    s.initialize(&mut state, &mut level()).unwrap();
    (s, state)
}

#[test]
fn execute_preserves_host_order_boss_skip_and_animation_phases() {
    let (s, mut state) = initialized(
        r#"<spawn type="orc" nr="2" area="gate" id="guard" health="0.5" snapToNavMesh="true"
        forceDraw="true" animation="idle" spawnAnimation="spawn" idleAnimation="spec_action0"
        meshId="1" order="defend" reactTo="none" speed="0.5"><kill/></spawn>"#,
    );
    let mut host = Host {
        bosses: vec![2],
        survival: true,
        ..Default::default()
    };
    s.execute(&mut state, &mut host).unwrap();
    let ops: Vec<_> = host.ops.iter().map(String::as_str).collect();
    assert_eq!(
        ops,
        [
            "template",
            "acquire 1",
            "locator",
            "nav 4",
            "init 1 mesh 1 at [5.0, 0.0, 7.0] id true",
            "force_draw 1",
            "configure 1",
            "animation 1 1",
            "resurrect 1 170",
            "special_idle 1 180",
            "survival 1",
            "add 1",
            "send 1",
            "acquire 2",
            "acquire 3",
            "locator",
            "nav 4",
            "init 3 mesh 1 at [5.0, 0.0, 7.0] id true",
            "force_draw 3",
            "configure 3",
            "animation 3 1",
            "resurrect 3 170",
            "special_idle 3 180",
            "survival 3",
            "add 3",
            "send 3",
        ]
    );
    assert_eq!(host.orders[0], (3, 0, 2, 1));
    assert_eq!(
        host.configs[0],
        (Some(100.0), Some(0.5), Some(false), [0.0, 0.0, -1.0], true)
    );
    assert_eq!(
        state.spawned,
        [
            SpawnedHealth {
                handle: Some(1),
                health: 100.0
            },
            SpawnedHealth {
                handle: Some(3),
                health: 100.0
            },
        ]
    );
    assert_eq!(host.messages[0].force_draw, Some(true));
    assert_eq!(host.messages[0].mesh, Some(1));
    assert!(host.messages[0].include_color);

    // Idle spawn animations skip resurrection, so special idle is forced.
    let (s, mut state) = initialized(
        r#"<spawn type="orc" area="gate" spawnAnimation="idle_agg" idleAnimation="idle"/>"#,
    );
    let mut host = Host {
        role: Some(NetworkRole::Offline),
        ..Default::default()
    };
    s.execute(&mut state, &mut host).unwrap();
    assert!(host.ops.contains(&"animation 1 1".to_string()));
    assert!(!host
        .ops
        .iter()
        .any(|o| o.starts_with("resurrect") || o.starts_with("send")));

    let mut client = Host {
        role: Some(NetworkRole::Client),
        ..Default::default()
    };
    s.execute(&mut state, &mut client).unwrap();
    assert!(client.ops.is_empty());
    let uninitialized = spawn(r#"<spawn type="orc"/>"#);
    assert_eq!(
        uninitialized.execute(&mut SpawnState::default(), &mut Host::default()),
        Err(SpawnError::NotInitialized)
    );
    assert_eq!(
        spawn("<spawn/>").execute(&mut SpawnState::default(), &mut Host::default()),
        Err(SpawnError::Host("no template"))
    );
}

#[test]
fn quick_execute_tracking_and_tags() {
    let (s, mut state) = initialized(
        r#"<spawn type="orc" area="gate" forceDraw="true" forceCamera="true" spawnAnimation="spawn"
        idleAnimation="spec_action0" speed="0.5"/>"#,
    );
    let bytes = write_tag(&[
        SpawnedHealth {
            handle: Some(9),
            health: 40.0,
        },
        SpawnedHealth {
            handle: Some(4),
            health: 12.5,
        },
    ]);
    assert_eq!(bytes.len(), 12);
    let (records, used) = read_tag(&bytes).unwrap();
    assert_eq!(used, 12);
    assert!(records.iter().all(|r| r.handle.is_none()));
    assert!(read_tag(&(-1i32).to_le_bytes()).is_none());
    assert!(read_tag(&bytes[..10]).is_none());
    state.spawned = records;
    let mut host = Host::default();
    // Unresolved handles are skipped by Update's bookkeeping.
    SpawnAction::track(&mut state, &mut host).unwrap();
    s.quick_execute(&mut state, &mut host).unwrap();
    let ops: Vec<_> = host.ops.iter().map(String::as_str).collect();
    assert_eq!(
        &ops[..9],
        [
            "template",
            "acquire 1",
            "locator",
            "init 1 mesh -1 at [5.0, 1.0, 7.0] id false",
            "configure 1",
            "resurrect 1 170",
            "special_idle 1 180",
            "add 1",
            "hp 1 40",
        ]
    );
    assert_eq!(host.configs[0], (None, None, None, [0.0, 0.0, -1.0], true));
    assert_eq!(host.messages[0].force_draw, None);
    assert!(!host.messages[0].include_color);
    assert_eq!(
        state.spawned[1],
        SpawnedHealth {
            handle: Some(2),
            health: 12.5
        }
    );

    host.hit_points.insert(1, 0.0);
    host.hit_points.insert(2, 7.0);
    SpawnAction::track(&mut state, &mut host).unwrap();
    assert_eq!(
        state.spawned,
        [SpawnedHealth {
            handle: Some(2),
            health: 7.0
        }]
    );
    host.hit_points.remove(&2);
    assert_eq!(
        SpawnAction::track(&mut state, &mut host),
        Err(SpawnError::MissingCharacter(2))
    );
}

struct World {
    host: Option<Host>,
}
impl ScriptWorld for World {
    type Error = &'static str;
    fn facts(&self) -> ScriptWorldFacts {
        ScriptWorldFacts::default()
    }
    fn spawn_host(&mut self) -> Option<&mut dyn SpawnHost<Error = Self::Error>> {
        self.host.as_mut().map(|h| h as _)
    }
    fn choose_group(&mut self, count: usize) -> Result<usize, Self::Error> {
        Ok(count - 1)
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
fn runtime_initializes_final_groups_and_schedules_spawn() {
    let xml = r#"<Scene>
      <trigger id="b" autorun="false"><then><spawn type="replaced"/></then>
        <then><random><counter name="x" value="1"/></random><random/><spawn type="orc" area="gate" delay="0.5"/></then></trigger>
      <trigger id="a" autorun="false"><then><spawn type="second"/></then></trigger></Scene>"#;
    let layout = SceneScriptLayout::read(xml.as_bytes(), false).unwrap();
    let mut rt = ScriptRuntime::new(layout, LevelVariables::default());
    let mut level = level();
    rt.initialize_actions(&mut level).unwrap();
    // Sorted signed hash order, then each random group; shared Spawn loads twice.
    let mut ids = [("a", custom_hash("a")), ("b", custom_hash("b"))];
    ids.sort_by_key(|(_, id)| *id);
    let expected: Vec<Option<String>> = ids
        .iter()
        .flat_map(|(name, _)| match *name {
            "a" => vec![Some("second".to_string())],
            _ => vec![Some("orc".to_string()), Some("orc".to_string())],
        })
        .collect();
    assert_eq!(level.loads, expected);
    assert!(
        rt.spawn_state(0).unwrap().events.is_none(),
        "replaced Then is not initialized"
    );
    let spawn_index = 2;
    assert!(rt.spawn_state(spawn_index).unwrap().events.is_some());
    assert!(rt.spawn_state(1).is_none());

    let mut world = World { host: None };
    assert!(rt
        .execute_trigger(
            custom_hash("b"),
            None,
            false,
            NetworkRole::Server,
            &mut world
        )
        .unwrap());
    assert_eq!(
        rt.update_action(spawn_index, 0.25, NetworkRole::Server, &mut world),
        Ok(0)
    );
    assert_eq!(
        rt.update_action(spawn_index, 0.25, NetworkRole::Server, &mut world),
        Err(ScriptRuntimeError::MissingSpawnHost(spawn_index))
    );
    world.host = Some(Host::default());
    // Failed Execute consumed the pending entry; retriggering restarts the delay.
    rt.trigger_action::<&str>(spawn_index).unwrap();
    assert_eq!(
        rt.update_action(spawn_index, 0.25, NetworkRole::Server, &mut world),
        Ok(0)
    );
    assert_eq!(
        rt.update_action(spawn_index, 0.25, NetworkRole::Server, &mut world),
        Ok(1)
    );
    assert_eq!(rt.history(), [spawn_index]);
    assert_eq!(rt.spawn_state(spawn_index).unwrap().spawned.len(), 1);
    // Update tracks health before the base queue phase, even with nothing queued.
    world.host.as_mut().unwrap().hit_points.insert(1, -1.0);
    assert_eq!(
        rt.update_action(spawn_index, 1.0, NetworkRole::Server, &mut world),
        Ok(0)
    );
    assert!(rt.spawn_state(spawn_index).unwrap().spawned.is_empty());

    rt.spawn_state_mut(spawn_index).unwrap().spawned = read_tag(&write_tag(&[SpawnedHealth {
        handle: None,
        health: 3.0,
    }]))
    .unwrap()
    .0;
    rt.quick_execute_spawn(spawn_index, &mut world).unwrap();
    assert_eq!(
        rt.spawn_state(spawn_index).unwrap().spawned[0].handle,
        Some(2)
    );
    assert_eq!(rt.history(), [spawn_index]);
    assert_eq!(
        rt.quick_execute_spawn(1, &mut world),
        Err(ScriptRuntimeError::NotSpawn(1))
    );
}

impl magicka_runtime::avatar_move::AvatarMoveLevel for Level {
    fn entity_by_id(&self, _: i32) -> Option<u16> {
        None
    }
    fn player_avatars(&self) -> Vec<Option<u16>> {
        ([None; 4]).to_vec()
    }
}
impl magicka_runtime::spawn_phys::SpawnPhysLevel for Level {
    fn load_physics_template(&mut self, _: Option<&str>) -> Result<i32, Self::Error> {
        Ok(0)
    }
    fn damageable_from_cache(&mut self) -> Result<u16, Self::Error> {
        Ok(0)
    }
    fn new_physics_entity(&mut self) -> Result<u16, Self::Error> {
        Ok(0)
    }
}

impl magicka_runtime::scene_flow::CheckpointLevel for Level {
    fn scene_locator(&mut self, _: i32) -> Result<[f32; 16], Self::Error> {
        Ok([0.0; 16])
    }
}
