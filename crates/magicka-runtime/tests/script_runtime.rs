use magicka_runtime::{
    animation_action::AnimationAction, level_variables::LevelVariables, scene_loading::custom_hash,
    scene_script::SceneScriptLayout, script_runtime::*,
};
use magicka_sim::damage_delivery::NetworkRole;

#[derive(Default)]
struct World {
    dialogs: Option<magicka_runtime::dialog_state::DialogLedger>,
    presence: Option<(
        magicka_runtime::presence::PresenceAreas,
        Vec<magicka_runtime::presence::PresenceEntity>,
    )>,
    notifications: Vec<i32>,
    senders: Vec<Option<u16>>,
    executed: Vec<usize>,
    animation_count: usize,
    fail_animation: bool,
    requeue_animation: bool,
    nested_trigger: Option<i32>,
}
impl ScriptWorld for World {
    type Error = &'static str;
    fn dialogs(&self) -> Option<magicka_runtime::dialog_state::DialogFacts<'_>> {
        self.dialogs
            .as_ref()
            .map(|ledger| magicka_runtime::dialog_state::DialogFacts {
                ledger,
                active_dialogs: &[None; 8],
                message_box_count: 0,
            })
    }
    fn presence(&self) -> Option<magicka_runtime::presence::PresenceFacts<'_>> {
        self.presence
            .as_ref()
            .map(|(areas, entities)| magicka_runtime::presence::PresenceFacts { areas, entities })
    }
    fn facts(&self) -> ScriptWorldFacts {
        ScriptWorldFacts {
            cutscene: false,
            player_count: 1,
        }
    }
    fn choose_group(&mut self, _: usize) -> Result<usize, Self::Error> {
        Ok(0)
    }
    fn notify_trigger(
        &mut self,
        rt: &mut ScriptRuntime,
        id: i32,
        _: usize,
        sender: Option<u16>,
    ) -> Result<(), Self::Error> {
        assert!(rt.trigger(id).unwrap().has_triggered);
        self.notifications.push(id);
        self.senders.push(sender);
        if let Some(other) = self.nested_trigger.take() {
            assert!(!rt
                .execute_trigger(id, None, false, NetworkRole::Server, self)
                .unwrap());
            assert!(rt
                .execute_trigger(other, None, false, NetworkRole::Server, self)
                .unwrap());
        }
        Ok(())
    }
    fn animation(
        &mut self,
        rt: &mut ScriptRuntime,
        _: &AnimationAction,
    ) -> Result<(), Self::Error> {
        self.animation_count += 1;
        if self.fail_animation {
            return Err("animation failed");
        }
        if self.requeue_animation {
            self.requeue_animation = false;
            rt.trigger_action::<Self::Error>(0).unwrap();
        }
        Ok(())
    }
    fn action_executed(&mut self, rt: &mut ScriptRuntime, index: usize) -> Result<(), Self::Error> {
        assert_eq!(rt.history().last(), Some(&index));
        self.executed.push(index);
        Ok(())
    }
}
fn runtime(xml: &str) -> ScriptRuntime {
    ScriptRuntime::new(
        SceneScriptLayout::read(xml.as_bytes(), false).unwrap(),
        LevelVariables::default(),
    )
}

#[test]
fn signed_hash_order_and_live_triggered_facts_drive_variable_actions() {
    let mut names = ["alpha", "omega"];
    names.sort_by_key(|name| custom_hash(name));
    let [low, high] = names;
    // Higher ID appears first in XML, but its predicate sees the lower ID's update.
    let mut rt = runtime(&format!(
        r#"<Scene>
    <trigger id="{high}"><if><triggered trigger="{low}"/></if><then><counter name="result" value="2"/></then></trigger>
    <trigger id="{low}"><then><counter name="result" action="SET" value="1"/></then></trigger></Scene>"#
    ));
    let mut world = World::default();
    rt.update(0.125, NetworkRole::Offline, &mut world).unwrap();
    assert_eq!(rt.layout().triggers[0].id, custom_hash(low));
    assert_eq!(rt.variables.counter(custom_hash("result")), 3);
    assert_eq!(rt.history(), [1, 0]);
}

#[test]
fn common_action_queue_ticks_per_alternative_and_preserves_delay_boundary() {
    let mut rt = runtime(
        r#"<Scene><trigger id="root"><then><random/><random/>
    <counter name="result" value="1" delay="0.375"/></then></trigger></Scene>"#,
    );
    let mut world = World::default();
    rt.update(0.125, NetworkRole::Offline, &mut world).unwrap();
    assert_eq!(rt.queue(0).unwrap().state.countdown, 0.125);
    assert!(rt.history().is_empty());
    rt.update(0.0625, NetworkRole::Offline, &mut world).unwrap();
    assert_eq!(rt.variables.counter(custom_hash("result")), 1);
    assert_eq!(rt.history(), [0]);
    assert_eq!(rt.queue(0).unwrap().state.countdown, 0.375);
}

#[test]
fn server_callback_can_reenter_with_current_trigger_state() {
    let mut rt = runtime(
        r#"<Scene><trigger id="first" autorun="false"><then><counter name="result" value="1"/></then></trigger>
    <trigger id="second" autorun="false"><if><triggered trigger="first"/></if><then><counter name="result" value="2"/></then></trigger></Scene>"#,
    );
    let mut world = World {
        nested_trigger: Some(custom_hash("second")),
        ..Default::default()
    };
    rt.execute_scene_trigger(
        custom_hash("first"),
        Some(7),
        false,
        NetworkRole::Server,
        &mut world,
    )
    .unwrap();
    assert_eq!(
        world.notifications,
        [custom_hash("first"), custom_hash("second")]
    );
    assert_eq!(rt.queue(0).unwrap().state.pending, 1);
    assert_eq!(rt.queue(1).unwrap().state.pending, 1);
    rt.update(0.0, NetworkRole::Server, &mut world).unwrap();
    assert_eq!(rt.variables.counter(custom_hash("result")), 3);
}

#[test]
fn animation_execute_requeues_same_instance_and_failure_leaves_consumed_work() {
    let xml = r#"<Scene><trigger id="root"><then><playAnimation name="door" delay="0.125"/></then></trigger></Scene>"#;
    let mut rt = runtime(xml);
    let mut world = World {
        requeue_animation: true,
        ..Default::default()
    };
    rt.update(0.125, NetworkRole::Offline, &mut world).unwrap();
    assert_eq!(rt.queue(0).unwrap().state.pending, 1);
    assert_eq!(rt.queue(0).unwrap().state.countdown, 0.125);
    rt.update(0.125, NetworkRole::Offline, &mut world).unwrap();
    assert_eq!(rt.history(), [0, 0]);
    assert_eq!(world.animation_count, 2);
    let mut failed = runtime(xml);
    world.fail_animation = true;
    assert_eq!(
        failed.update(0.125, NetworkRole::Offline, &mut world),
        Err(ScriptRuntimeError::World("animation failed"))
    );
    assert_eq!(failed.queue(0).unwrap().state.pending, 0);
    assert!(failed.history().is_empty());
    assert!(failed.trigger(custom_hash("root")).unwrap().has_triggered);
}

#[test]
fn scene_client_guard_differs_from_direct_execute_and_client_still_ticks_actions() {
    let mut rt = runtime(
        r#"<Scene><trigger id="root"><then><counter name="result" value="1"/></then></trigger></Scene>"#,
    );
    let mut world = World::default();
    assert!(!rt
        .execute_scene_trigger(123, None, true, NetworkRole::Client, &mut world)
        .unwrap());
    rt.update(0.125, NetworkRole::Client, &mut world).unwrap();
    assert_eq!(rt.variables.counter(custom_hash("result")), 0);
    rt.execute_trigger(
        custom_hash("root"),
        None,
        false,
        NetworkRole::Client,
        &mut world,
    )
    .unwrap();
    rt.update(0.125, NetworkRole::Client, &mut world).unwrap();
    assert_eq!(rt.variables.counter(custom_hash("result")), 1);
    assert!(world.notifications.is_empty());
}

#[test]
fn reached_unknown_definitions_fail_without_fabricating_behavior() {
    let mut world = World::default();
    let mut rt = runtime(
        r#"<Scene><trigger id="root"><if><unrecovered/></if><then><counter/></then></trigger></Scene>"#,
    );
    assert_eq!(
        rt.execute_trigger(
            custom_hash("root"),
            None,
            true,
            NetworkRole::Offline,
            &mut world
        ),
        Err(ScriptRuntimeError::UnsupportedCondition(0))
    );
    assert!(!rt.trigger(custom_hash("root")).unwrap().has_triggered);
    let mut rt = runtime(
        r#"<Scene><trigger id="root"><then><counter name="x" value="1"/><unrecoveredAction/></then></trigger></Scene>"#,
    );
    assert_eq!(
        rt.update(0.0, NetworkRole::Offline, &mut world),
        Err(ScriptRuntimeError::UnsupportedAction(1))
    );
    assert!(rt.trigger(custom_hash("root")).unwrap().has_triggered);
    assert_eq!(rt.queue(0).unwrap().state.pending, 1);
    assert_eq!(rt.variables.counter(custom_hash("x")), 0);
    rt.clear_delayed::<&str>(custom_hash("root")).unwrap_err();
    assert_eq!(rt.queue(0).unwrap().state.pending, 0);
}

#[test]
fn execute_trigger_delivers_each_queued_sender_and_respects_scene_client_guard() {
    let xml = r#"<Scene><trigger id="owner" autorun="false"><then>
    <executeTrigger trigger="target" delay="0.125"/></then></trigger>
    <trigger id="target" repeat="true" autorun="false"><then><counter name="x" value="1"/></then></trigger></Scene>"#;
    let mut rt = runtime(xml);
    let mut world = World::default();
    rt.trigger_action_with_sender::<&str>(0, Some(7)).unwrap();
    rt.trigger_action_with_sender::<&str>(0, Some(8)).unwrap();
    assert_eq!(
        rt.update_action(0, 0.25, NetworkRole::Server, &mut world)
            .unwrap(),
        2
    );
    assert_eq!(world.senders, [Some(7), Some(8)]);
    assert_eq!(rt.queue(1).unwrap().state.pending, 2);
    rt.update_action(1, 0.0, NetworkRole::Server, &mut world)
        .unwrap();
    assert_eq!(rt.variables.counter(custom_hash("x")), 2);
    let mut client = runtime(xml);
    client
        .trigger_action_with_sender::<&str>(0, Some(9))
        .unwrap();
    client
        .update_action(0, 0.125, NetworkRole::Client, &mut world)
        .unwrap();
    assert!(!client.trigger(custom_hash("target")).unwrap().has_triggered);
    assert_eq!(client.history(), [0]);
}

#[test]
fn clearing_execute_trigger_keeps_old_sender_arguments() {
    let mut rt = runtime(
        r#"<Scene><trigger id="owner" autorun="false"><then>
    <executeTrigger trigger="target"/></then></trigger>
    <trigger id="target" repeat="true" autorun="false"><then/></trigger></Scene>"#,
    );
    let mut world = World::default();
    rt.trigger_action_with_sender::<&str>(0, Some(7)).unwrap();
    rt.clear_delayed::<&str>(custom_hash("owner")).unwrap();
    rt.trigger_action_with_sender::<&str>(0, Some(8)).unwrap();
    rt.update_action(0, 0.0, NetworkRole::Server, &mut world)
        .unwrap();
    assert_eq!(world.senders, [Some(7)]);
    rt.trigger_action_with_sender::<&str>(0, Some(9)).unwrap();
    rt.update_action(0, 0.0, NetworkRole::Server, &mut world)
        .unwrap();
    assert_eq!(world.senders, [Some(7), Some(8)]);
}

#[test]
fn clear_scopes_use_owning_trigger_and_can_cancel_the_current_queue() {
    for (attribute, keep_owner) in [
        (r#"trigger="allbutthis""#, true),
        (r#"trigger="other""#, true),
        (r#"trigger="all""#, false),
        ("", false),
    ] {
        let mut rt = runtime(&format!(
            r#"<Scene><trigger id="owner" autorun="false"><then>
        <counter delay="1"/><clearDelayedActions {attribute}/></then></trigger>
        <trigger id="other" autorun="false"><then><counter delay="1"/></then></trigger></Scene>"#
        ));
        let mut world = World::default();
        for index in [0, 1, 1, 2] {
            rt.trigger_action::<&str>(index).unwrap();
        }
        let count = rt
            .update_action(1, 0.0, NetworkRole::Offline, &mut world)
            .unwrap();
        assert_eq!(
            rt.queue(0).unwrap().state.pending,
            if keep_owner { 1 } else { 0 }
        );
        assert_eq!(rt.queue(2).unwrap().state.pending, 0);
        assert_eq!(count, if keep_owner { 2 } else { 1 });
        assert_eq!(rt.queue(0).unwrap().state.countdown, 1.0);
    }
}

#[test]
fn missing_execute_target_consumes_sender_and_work_without_history() {
    let mut rt = runtime(
        r#"<Scene><trigger id="owner" autorun="false"><then>
    <executeTrigger trigger="absent"/></then></trigger></Scene>"#,
    );
    let mut world = World::default();
    rt.trigger_action_with_sender::<&str>(0, Some(7)).unwrap();
    assert_eq!(
        rt.update_action(0, 0.0, NetworkRole::Offline, &mut world),
        Err(ScriptRuntimeError::MissingTrigger(custom_hash("absent")))
    );
    assert_eq!(rt.queue(0).unwrap().state.pending, 0);
    assert!(rt.history().is_empty());
}

#[test]
fn loaded_presence_condition_consumes_world_membership_before_variable_dispatch() {
    use magicka_runtime::presence::*;
    let xml = r#"<Scene><trigger id="root"><if><idPresent id="player"/></if>
    <then><counter name="entered" value="1"/></then></trigger></Scene>"#;
    let mut rt = runtime(xml);
    let mut world = World::default();
    assert_eq!(
        rt.update(0.0, NetworkRole::Offline, &mut world),
        Err(ScriptRuntimeError::Presence(
            PresenceError::MissingWorldFacts
        ))
    );
    let mut areas = PresenceAreas::from_content(&[]).unwrap();
    let entities = vec![PresenceEntity {
        handle: 1,
        unique_id: custom_hash("player"),
        position: [0.0; 3],
        radius: 1.0,
        dead: false,
        ignored_avatar: false,
        physics: None,
        character: None,
    }];
    areas.reset();
    areas.count_any(&entities);
    world.presence = Some((areas, entities));
    rt.update(0.0, NetworkRole::Offline, &mut world).unwrap();
    assert_eq!(rt.variables.counter(custom_hash("entered")), 1);
    assert_eq!(rt.history(), [0]);
}

#[test]
fn loaded_dialog_done_waits_for_completion_flag_then_drives_script() {
    use magicka_runtime::dialog_state::*;
    let mut rt = runtime(
        r#"<Scene><trigger id="after"><if><dialogDone dialog="intro" interactIndex="0"/></if><then><counter name="done" value="1"/></then></trigger></Scene>"#,
    );
    let mut world = World::default();
    assert_eq!(
        rt.update(0.0, NetworkRole::Offline, &mut world),
        Err(ScriptRuntimeError::Dialog(DialogError::MissingWorldFacts))
    );
    let mut ledger = DialogLedger::default();
    ledger.insert(
        custom_hash("intro"),
        DialogCursor::new(vec![InteractionRepetition::default()], -1),
    );
    ledger
        .dialogs
        .get_mut(&custom_hash("intro"))
        .unwrap()
        .pop()
        .unwrap();
    world.dialogs = Some(ledger);
    rt.update(0.0, NetworkRole::Offline, &mut world).unwrap();
    assert_eq!(rt.variables.counter(custom_hash("done")), 0);
    world
        .dialogs
        .as_mut()
        .unwrap()
        .mark_finished(custom_hash("intro"));
    rt.update(0.0, NetworkRole::Offline, &mut world).unwrap();
    assert_eq!(rt.variables.counter(custom_hash("done")), 1);
    assert_eq!(rt.history(), [0]);
}

#[test]
fn dialog_action_requires_world_capability_and_consumes_work_before_failure() {
    let mut runtime = runtime(
        r#"<Scene><trigger><then><startDialog dialog="intro" position="1,2"/></then></trigger></Scene>"#,
    );
    let mut world = World::default();
    assert_eq!(
        runtime.update(0.0, NetworkRole::Offline, &mut world),
        Err(ScriptRuntimeError::MissingDialogActionHost(0))
    );
    assert_eq!(runtime.queue(0).unwrap().state.pending, 0);
    assert!(runtime.history().is_empty());
}

#[test]
fn wave_actions_tick_only_in_the_current_wave_and_honor_nested_action_delays() {
    let mut rt = runtime(
        r#"<Scene>
      <RuleSet type="Survival"><spawnAreas><area>north</area></spawnAreas></RuleSet>
      <wave><waveActions delay="1"><counter name="first" value="1" delay="0.5"/></waveActions></wave>
      <wave><waveActions><counter name="future" value="1"/></waveActions></wave>
    </Scene>"#,
    );
    let ids: Vec<_> = rt
        .layout()
        .ruleset
        .as_ref()
        .unwrap()
        .waves
        .iter()
        .map(|w| w[0].trigger)
        .collect();
    let mut world = World::default();
    rt.update(20.0, NetworkRole::Offline, &mut world).unwrap();
    assert_eq!(rt.queue(0).unwrap().state.countdown, 0.0);
    assert_eq!(rt.queue(1).unwrap().state.countdown, 0.0);
    rt.update_wave_actions(&ids[..1], &[ids[0]], 0.25, NetworkRole::Offline, &mut world)
        .unwrap();
    assert_eq!(rt.variables.counter(custom_hash("first")), 0);
    assert!(!rt.wave_actions_finished(ids[0]));
    rt.update_wave_actions(&ids[..1], &[], 0.25, NetworkRole::Offline, &mut world)
        .unwrap();
    assert_eq!(rt.variables.counter(custom_hash("first")), 1);
    assert!(rt.wave_actions_finished(ids[0]));
    assert_eq!(rt.variables.counter(custom_hash("future")), 0);
    assert_eq!(rt.queue(1).unwrap().state, Default::default());
}
