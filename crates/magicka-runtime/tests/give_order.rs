use magicka_runtime::{
    ai_event::{AiEvent, AiEventLevel},
    animation_action::AnimationAction,
    give_order::*,
    level_variables::LevelVariables,
    scene_loading::custom_hash,
    scene_script::{ActionDefinition, SceneScriptLayout},
    script_runtime::*,
    spawn_action::SpawnLevel,
};
use magicka_sim::damage_delivery::NetworkRole;
use std::collections::BTreeMap;

fn layout(actions: &str) -> SceneScriptLayout {
    SceneScriptLayout::read(
        format!(
            r#"<Scene><trigger id="root" autorun="false"><then>{actions}</then></trigger></Scene>"#
        )
        .as_bytes(),
        false,
    )
    .unwrap()
}
fn order(actions: &str) -> GiveOrderAction {
    match layout(actions).actions.remove(0) {
        ActionDefinition::GiveOrder(a) => *a,
        other => panic!("{other:?}"),
    }
}

#[test]
fn decoding_defaults_specific_ids_and_factions() {
    let a = order("<giveOrder/>");
    assert_eq!((a.order, a.react_to, a.reaction), (0, 0, 2));
    assert_eq!((a.id, a.type_id, a.area, a.factions), (None, 0, 0, 0));
    assert_eq!((a.wander_speed, a.priority_ability), (1.0, -1));
    assert!(!a.explicit_faction);
    let a = order(
        r#"<GiveOrder ID="" order="Panic" reactTo="attack,proximity" reaction="wander"
        factions="evil, wild" explicit="true" targetArea="any" type="Goblin" animation="spec_action0"
        idleAnimation="idle" priorityTarget="Boss" speed="0.6" unknown="x"><Face target="x"/></GiveOrder>"#,
    );
    assert_eq!(a.id, None, "empty ID is not specific");
    assert_eq!((a.order, a.react_to, a.reaction), (6, 3, 5));
    assert_eq!((a.factions, a.target_area), (3, 0));
    assert_eq!(a.type_id, custom_hash("goblin"));
    assert_eq!((a.animation, a.idle_animation), (180, 1));
    assert_eq!(
        (a.priority_target, a.wander_speed),
        (custom_hash("boss"), 0.6)
    );
    assert!(a.explicit_faction);
    assert_eq!(
        order(r#"<giveorder id="Guard"/>"#).id,
        Some(custom_hash("guard"))
    );
    for bad in [
        r#"<giveOrder order="charge"/>"#,
        r#"<giveOrder factions="orcs"/>"#,
        r#"<giveOrder><face bogus="1"/></giveOrder>"#,
    ] {
        assert!(
            SceneScriptLayout::read(
                format!(r#"<Scene><trigger id="r"><then>{bad}</then></trigger></Scene>"#)
                    .as_bytes(),
                false
            )
            .is_err(),
            "{bad}"
        );
    }
}

#[derive(Default)]
struct Host {
    role: Option<NetworkRole>,
    ops: Vec<String>,
    npcs: BTreeMap<i32, u16>,
    gripped: Vec<u16>,
    areas: BTreeMap<i32, Vec<OrderCandidate>>,
    orders: Vec<(u16, f32, i32, u8, usize)>,
}
impl GiveOrderHost for Host {
    type Error = &'static str;
    fn network_role(&self) -> NetworkRole {
        self.role.unwrap_or(NetworkRole::Offline)
    }
    fn npc_by_id(&mut self, id: i32) -> Result<Option<u16>, Self::Error> {
        Ok(self.npcs.get(&id).copied())
    }
    fn present_characters(
        &mut self,
        area: i32,
    ) -> Result<Option<Vec<OrderCandidate>>, Self::Error> {
        self.ops.push("present".into());
        Ok(self.areas.get(&area).cloned())
    }
    fn update_present(&mut self, area: i32) -> Result<bool, Self::Error> {
        self.ops.push("update_present".into());
        Ok(self.areas.contains_key(&area))
    }
    fn release_attached(&mut self, npc: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("release {npc}"));
        Ok(())
    }
    fn is_gripped(&mut self, npc: u16) -> Result<bool, Self::Error> {
        Ok(self.gripped.contains(&npc))
    }
    fn release_gripper_attached(&mut self, npc: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("gripper {npc}"));
        Ok(())
    }
    fn apply_order(&mut self, npc: u16, c: &OrderCommand<'_>) -> Result<(), Self::Error> {
        self.ops.push(format!("order {npc}"));
        self.orders.push((
            npc,
            c.wander_speed,
            c.target_area,
            c.order.order,
            c.order.events.len(),
        ));
        Ok(())
    }
    fn set_special_idle(&mut self, npc: u16, a: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("idle {npc} {a}"));
        Ok(())
    }
    fn go_to_animation(&mut self, npc: u16, a: u16, blend: f32) -> Result<(), Self::Error> {
        self.ops.push(format!("goto {npc} {a} {blend}"));
        Ok(())
    }
    fn order_panic(&mut self, npc: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("panic {npc}"));
        Ok(())
    }
    fn send_event_animation(
        &mut self,
        npc: u16,
        a: u16,
        blend: f32,
        idle: u16,
    ) -> Result<(), Self::Error> {
        self.ops.push(format!("send {npc} {a} {blend} {idle}"));
        Ok(())
    }
}
fn candidate(handle: u16, npc: bool, type_name: &str, faction: i32) -> OrderCandidate {
    OrderCandidate {
        handle,
        npc,
        type_id: custom_hash(type_name),
        original_faction: faction,
    }
}

#[test]
fn specific_and_area_paths_preserve_order_filters_and_panic_quirk() {
    let a = order(
        r#"<giveOrder id="guard" order="panic" animation="idle" idleAnimation="spec_action0" speed="0.5"/>"#,
    );
    let events = [];
    let mut host = Host {
        role: Some(NetworkRole::Client),
        npcs: [(custom_hash("guard"), 7)].into(),
        gripped: vec![7],
        ..Default::default()
    };
    a.execute(Some(&events), &mut host).unwrap();
    assert_eq!(
        host.ops,
        [
            "release 7",
            "gripper 7",
            "order 7",
            "idle 7 180",
            "goto 7 1 0.2",
            "panic 7",
            "send 7 1 0.2 180"
        ]
    );
    assert_eq!(host.orders[0], (7, 0.5, 0, 6, 0));
    // Missing or non-NPC specific targets are silently ignored.
    let mut host = Host::default();
    a.execute(Some(&events), &mut host).unwrap();
    assert!(host.ops.is_empty());
    assert_eq!(
        a.execute(None, &mut host),
        Err(GiveOrderError::NotInitialized)
    );

    let area = custom_hash("yard");
    let present = vec![
        candidate(1, true, "orc", 0),
        candidate(2, true, "goblin", 1),
        candidate(3, false, "goblin", 1),
        candidate(4, true, "troll", 3),
        candidate(5, true, "imp", 2),
    ];
    let mut host = Host {
        areas: [(area, present.clone())].into(),
        ..Default::default()
    };
    // Area path: faction bit match, no OrderPanic, offline sends nothing.
    order(r#"<giveOrder area="yard" factions="evil" order="panic" animation="idle"/>"#)
        .execute(Some(&events), &mut host)
        .unwrap();
    assert_eq!(
        host.ops,
        [
            "present",
            "release 2",
            "order 2",
            "goto 2 1 0.2",
            "release 4",
            "order 4",
            "goto 4 1 0.2"
        ]
    );
    let handles = |xml: &str| {
        let mut host = Host {
            areas: [(area, present.clone())].into(),
            ..Default::default()
        };
        order(xml).execute(Some(&[]), &mut host).unwrap();
        host.orders.iter().map(|o| o.0).collect::<Vec<_>>()
    };
    assert_eq!(
        handles(r#"<giveOrder area="yard" factions="evil" explicit="true"/>"#),
        [2]
    );
    assert_eq!(
        handles(r#"<giveOrder area="yard" type="any"/>"#),
        [1, 2, 4, 5]
    );
    assert_eq!(
        handles(r#"<giveOrder area="yard" type="imp" factions="evil"/>"#),
        [2, 4, 5]
    );
    // Defaults: type 0 and faction NONE match only explicit NONE factions.
    assert_eq!(handles(r#"<giveOrder area="yard"/>"#), Vec::<u16>::new());
    assert_eq!(handles(r#"<giveOrder area="yard" explicit="true"/>"#), [1]);
    assert_eq!(
        order(r#"<giveOrder area="nowhere"/>"#).execute(Some(&events), &mut host),
        Err(GiveOrderError::MissingArea(custom_hash("nowhere")))
    );
}

#[test]
fn quick_execute_refreshes_nonzero_area_first() {
    let mut host = Host {
        areas: [(custom_hash("yard"), vec![])].into(),
        npcs: [(custom_hash("guard"), 3)].into(),
        ..Default::default()
    };
    order(r#"<giveOrder id="guard" area="yard"/>"#)
        .quick_execute(Some(&[]), &mut host)
        .unwrap();
    assert_eq!(host.ops, ["update_present", "release 3", "order 3"]);
    host.ops.clear();
    order(r#"<giveOrder id="guard"/>"#)
        .quick_execute(Some(&[]), &mut host)
        .unwrap();
    assert_eq!(host.ops, ["release 3", "order 3"]);
    assert_eq!(
        order(r#"<giveOrder id="guard" area="gone"/>"#).quick_execute(Some(&[]), &mut host),
        Err(GiveOrderError::MissingArea(custom_hash("gone")))
    );
}

struct Level;
impl AiEventLevel for Level {
    type Error = &'static str;
    fn locator(&self, id: i32) -> Option<[f32; 16]> {
        let mut m = [0.0; 16];
        m[12] = 9.0;
        (id == custom_hash("gate")).then_some(m)
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
struct World {
    host: Option<Host>,
}
impl ScriptWorld for World {
    type Error = &'static str;
    fn facts(&self) -> ScriptWorldFacts {
        ScriptWorldFacts::default()
    }
    fn give_order_host(&mut self) -> Option<&mut dyn GiveOrderHost<Error = Self::Error>> {
        self.host.as_mut().map(|h| h as _)
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
fn runtime_initializes_events_and_dispatches_through_queue() {
    let mut rt = ScriptRuntime::new(
        layout(r#"<giveOrder id="guard" delay="0.5"><move position="gate"/></giveOrder>"#),
        LevelVariables::default(),
    );
    let mut world = World { host: None };
    assert!(rt.give_order_events(0).is_none());
    rt.execute_trigger(
        custom_hash("root"),
        None,
        false,
        NetworkRole::Server,
        &mut world,
    )
    .unwrap();
    // Executing before Initialize is an explicit error.
    world.host = Some(Host::default());
    assert_eq!(
        rt.update_action(0, 0.5, NetworkRole::Server, &mut world),
        Err(ScriptRuntimeError::GiveOrder(
            0,
            GiveOrderError::NotInitialized
        ))
    );
    rt.initialize_actions(&mut Level).unwrap();
    let [AiEvent::Move(m)] = rt.give_order_events(0).unwrap() else {
        panic!()
    };
    assert_eq!(m.waypoint, [9.0, 0.0, 0.0]);
    world
        .host
        .as_mut()
        .unwrap()
        .npcs
        .insert(custom_hash("guard"), 4);
    rt.trigger_action::<&str>(0).unwrap();
    assert_eq!(
        rt.update_action(0, 0.5, NetworkRole::Client, &mut world),
        Ok(1)
    );
    assert_eq!(world.host.as_ref().unwrap().orders[0], (4, 1.0, 0, 0, 1));
    assert_eq!(rt.history(), [0]);
    rt.quick_execute_give_order(0, &mut world).unwrap();
    assert_eq!(world.host.as_ref().unwrap().orders.len(), 2);
    world.host = None;
    assert_eq!(
        rt.quick_execute_give_order(0, &mut world),
        Err(ScriptRuntimeError::MissingGiveOrderHost(0))
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
