//! Host-language calls into the scene script runtime (research/script-schema.md):
//! native action/condition classes defined from a class name and properties
//! run through the same pools, queues and hosts as authored triggers.
use magicka_runtime::avatar::PlayerInput;
use magicka_runtime::play_session::PlaySession;
use magicka_runtime::scene_loading::custom_hash;
use magicka_runtime::script_schema::ScriptCall;
use std::path::PathBuf;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

fn frames(s: &mut PlaySession, n: usize) {
    for _ in 0..n {
        s.step(1.0 / 60.0);
        s.handle_input(PlayerInput::default(), 16.0 / 9.0);
        s.events.clear();
    }
}

fn session() -> Option<PlaySession> {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return None;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    frames(&mut s, 2);
    Some(s)
}

#[test]
fn host_calls_run_counters_conditions_and_delays() {
    let Some(mut s) = session() else { return };
    let counter = custom_hash("hostprobe");
    let set = ScriptCall::new("counter")
        .arg("name", "HostProbe")
        .arg("action", "set")
        .arg("value", 5);
    assert_eq!(s.call_script_action(&set, None), Ok(1));
    assert_eq!(s.runtime.variables.counter(counter), 5);

    let equal = s
        .define_script_condition(
            &ScriptCall::new("Counter")
                .arg("name", "hostprobe")
                .arg("comparemethod", "EQUAL")
                .arg("value", 6),
        )
        .unwrap();
    assert_eq!(s.script_condition_met(equal, None), Ok(false));

    // INC adds Value (default 0). A delayed instance waits for the scene update, like authored actions;
    // two OnTriggers queue two executions half a second apart.
    let inc = s
        .define_script_action(
            &ScriptCall::new("Counter")
                .arg("name", "hostprobe")
                .arg("action", "INC")
                .arg("value", 1)
                .arg("delay", 0.5f32),
        )
        .unwrap();
    assert_eq!(s.invoke_script_action(inc, None), Ok(0));
    assert_eq!(s.invoke_script_action(inc, None), Ok(0));
    frames(&mut s, 25);
    assert_eq!(s.runtime.variables.counter(counter), 5);
    frames(&mut s, 10);
    assert_eq!(s.script_condition_met(equal, None), Ok(true));
    frames(&mut s, 35);
    assert_eq!(s.runtime.variables.counter(counter), 7);
    // Executions join the scene history (save/replay) like authored ones.
    assert!(s.runtime.history().contains(&inc));
}

#[test]
fn host_triggers_autorun_once_their_conditions_hold() {
    let Some(mut s) = session() else { return };
    let id = s
        .add_script_trigger(
            r#"<Trigger id="HostGate" repeat="false">
                 <If><Counter name="hostgate" value="1"/></If>
                 <Then><Counter name="hostgate_out" action="set" value="9"/></Then>
               </Trigger>"#,
        )
        .unwrap();
    assert_eq!(id, custom_hash("hostgate"));
    assert!(s
        .add_script_trigger(r#"<Trigger id="hostgate"><Then/></Trigger>"#)
        .is_err());
    frames(&mut s, 3);
    assert_eq!(s.runtime.variables.counter(custom_hash("hostgate_out")), 0);
    s.call_script_action(
        &ScriptCall::new("Counter")
            .arg("name", "hostgate")
            .arg("value", 1)
            .arg("action", "set"),
        None,
    )
    .unwrap();
    frames(&mut s, 3);
    assert_eq!(s.runtime.variables.counter(custom_hash("hostgate_out")), 9);
    assert!(s.runtime.trigger(id).unwrap().has_triggered);
}

#[test]
fn rarely_used_native_classes_run_through_host_calls() {
    let Some(mut s) = session() else { return };
    // EnableDark/DisableDark drive the tutorial dark-screen fade alone.
    s.call_script_action(&ScriptCall::new("EnableDark"), None)
        .unwrap();
    frames(&mut s, 30);
    assert_eq!(s.world.tutorial.dark_alpha, 1.0);
    s.call_script_action(&ScriptCall::new("DisableDark"), None)
        .unwrap();
    frames(&mut s, 30);
    assert_eq!(s.world.tutorial.dark_alpha, 0.0);

    // IsCampaignLooped: campaign saves are never looped in the rewrite.
    let looped = s
        .define_script_condition(&ScriptCall::new("IsCampaignLooped"))
        .unwrap();
    assert_eq!(s.script_condition_met(looped, None), Ok(false));

    // PhysicsPresent counts PhysicsEntity entries of the global "any" area.
    let props = s.world.entities.iter().filter(|e| {
        e.added
            && !e.removed
            && matches!(
                e.kind,
                magicka_runtime::scene_world::EntityKind::Physics { .. }
            )
    });
    let n = props.count() as i32;
    let present = s
        .define_script_condition(&ScriptCall::new("PhysicsPresent").arg("nr", n))
        .unwrap();
    assert_eq!(s.script_condition_met(present, None), Ok(true), "{n} props");

    // ReleaseGrip on an absent ID is a no-op; PhysAnimation on one throws.
    assert_eq!(
        s.call_script_action(&ScriptCall::new("ReleaseGrip").arg("id", "nobody"), None),
        Ok(1)
    );
    assert!(s
        .call_script_action(
            &ScriptCall::new("PhysAnimation")
                .arg("id", "nobody")
                .arg("name", "door"),
            None
        )
        .is_err());
    // Values are checked against the native property types up front.
    assert!(s
        .call_script_action(&ScriptCall::new("Counter").arg("value", "lots"), None)
        .unwrap_err()
        .contains("InvalidValue"));
}

#[test]
fn phys_animation_scrubs_a_level_part_as_the_prop_is_damaged() {
    let Some(mut s) = session() else { return };
    let Some(part) = s.world.collision.parts().first().map(|p| p.name.clone()) else {
        panic!("wc_s1 has no animated parts");
    };
    let area = s.world.level.locators[0].name.clone();
    s.call_script_action(
        &ScriptCall::new("SpawnPhys")
            .arg("object", "dungeons_secret_wall")
            .arg("area", area.as_str())
            .arg("id", "hostwall"),
        None,
    )
    .unwrap();
    frames(&mut s, 1);
    let handle = s
        .world
        .entities
        .iter()
        .find(|e| e.unique_id == custom_hash("hostwall"))
        .unwrap()
        .handle;
    s.call_script_action(
        &ScriptCall::new("PhysAnimation")
            .arg("id", "hostwall")
            .arg("name", part.as_str())
            .arg("start", 0.0f32)
            .arg("end", 2.0f32)
            .arg("children", false),
        None,
    )
    .unwrap();
    let before = s.world.collision.parts()[0].playback;
    let state = s.world.physics_damage.get_mut(&handle).unwrap();
    state.health = state.maximum * 0.5;
    frames(&mut s, 1);
    let after = s.world.collision.parts()[0].playback;
    assert_ne!(before, after);
    // First change: from End (fraction 1) back to the halfway point;
    // AnimatedLevelPart.Play keeps a reversed interval as end -> start.
    assert_eq!(
        (
            after.start,
            after.end,
            after.speed,
            after.looping,
            after.playing
        ),
        (2.0, 1.0, -1.0, false, true),
        "{after:?}"
    );
    assert!(after.time < 2.0 && after.time > 1.0);
}
