use magicka_runtime::{
    animation_action::AnimationAction, level_variables::LevelVariables, scene_loading::custom_hash,
    scene_script::SceneScriptLayout, script_runtime::*, toggle_action::*,
};
use magicka_sim::damage_delivery::NetworkRole;

fn read(tag: &str, attrs: &[(&str, &str)]) -> ToggleAction {
    ToggleAction::read(tag, attrs.iter().copied()).unwrap()
}

#[test]
fn decoding_enums_and_light_id_lists() {
    assert_eq!(
        read(
            "DisableLight",
            &[
                ("id", "Torch1,torch2, torch3"),
                ("effect", "Linear"),
                ("time", "2")
            ]
        )
        .kind,
        ToggleKind::Light {
            enable: false,
            ids: Some(vec![
                custom_hash("torch1"),
                custom_hash("torch2"),
                custom_hash(" torch3")
            ]),
            effect: 1,
            time: 2.0
        }
    );
    assert_eq!(
        read("enableElement", &[("element", "Water,Life,Shield")]).kind,
        ToggleKind::Element {
            enable: true,
            elements: 0xc2
        }
    );
    assert_eq!(
        read("disableCastType", &[("type", "blast")]).kind,
        ToggleKind::CastType {
            enable: false,
            cast_type: 0
        }
    );
    assert_eq!(
        read("unlockAchievement", &[("achievement", "Wizard_Mastery")]).kind,
        ToggleKind::UnlockAchievement {
            name: Some("wizard_mastery".into())
        }
    );
    assert_eq!(
        read("printDebug", &[("message", "hi")]).kind,
        ToggleKind::PrintDebug
    );
    assert!(ToggleAction::read("enableLight", [("effect", "strobe")]).is_err());
    assert!(ToggleAction::read("printdebug", [("break", "maybe")]).is_err());
    assert!(ToggleAction::read("enableelement", [("element", "plasma")]).is_err());
}

#[derive(Default)]
struct Host {
    ops: Vec<String>,
    lights: Vec<i32>,
    pois: Vec<i32>,
}
impl ToggleHost for Host {
    type Error = &'static str;
    fn set_light(
        &mut self,
        id: i32,
        enable: bool,
        effect: i32,
        time: f32,
    ) -> Result<bool, Self::Error> {
        self.ops.push(format!("light {enable} {effect} {time}"));
        Ok(self.lights.contains(&id))
    }
    fn set_interactable_enabled(&mut self, id: i32, enabled: bool) -> Result<bool, Self::Error> {
        self.ops.push(format!("poi {enabled}"));
        Ok(self.pois.contains(&id))
    }
    fn set_tutorial_element(&mut self, e: i32, enabled: bool) -> Result<(), Self::Error> {
        self.ops.push(format!("element {e} {enabled}"));
        Ok(())
    }
    fn set_tutorial_cast_type(&mut self, c: i32, enabled: bool) -> Result<(), Self::Error> {
        self.ops.push(format!("cast {c} {enabled}"));
        Ok(())
    }
    fn remove_hint(&mut self) -> Result<(), Self::Error> {
        self.ops.push("remove hint".into());
        Ok(())
    }
    fn set_tutorial_push(&mut self, enabled: bool) -> Result<(), Self::Error> {
        self.ops.push(format!("push {enabled}"));
        Ok(())
    }
    fn unlock_magick(&mut self, magick: u8) -> Result<(), Self::Error> {
        self.ops.push(format!("unlock {magick}"));
        Ok(())
    }
    fn award_achievement(&mut self, name: Option<&str>) -> Result<(), Self::Error> {
        self.ops.push(format!("award {name:?}"));
        Ok(())
    }
}

#[test]
fn execute_and_quick_execute_rules() {
    let mut host = Host {
        lights: vec![custom_hash("a"), custom_hash("b")],
        pois: vec![custom_hash("door")],
        ..Default::default()
    };
    let light = read(
        "enablelight",
        &[("id", "a,b"), ("effect", "flicker"), ("time", "3")],
    );
    let actions = [
        light.clone(),
        read("disablepoi", &[("id", "door")]),
        read("enableelement", &[("element", "fire")]),
        read("enablecasttype", &[("type", "self")]),
        read("removehint", &[]),
        read("unlockachievement", &[("achievement", "x")]),
        read("printdebug", &[]),
    ];
    for a in &actions {
        a.execute(&mut host).unwrap();
    }
    assert_eq!(
        host.ops,
        [
            "light true 2 3",
            "light true 2 3",
            "poi false",
            "element 8 true",
            "cast 3 true",
            "remove hint",
            "award Some(\"x\")"
        ]
    );
    host.ops.clear();
    for a in &actions {
        a.quick_execute(&mut host).unwrap();
    }
    assert_eq!(
        host.ops,
        [
            "light true 0 0",
            "light true 0 0",
            "poi false",
            "element 8 true",
            "cast 3 true",
            "remove hint"
        ]
    );
    // Missing keys fail after earlier lights were already changed.
    host.ops.clear();
    assert_eq!(
        read("disablelight", &[("id", "a,zz,b")]).execute(&mut host),
        Err(ToggleError::MissingLight(custom_hash("zz")))
    );
    assert_eq!(host.ops.len(), 2);
    assert_eq!(
        read("disablelight", &[]).execute(&mut host),
        Err(ToggleError::MissingLightIds)
    );
    assert_eq!(
        read("enablepoi", &[("id", "gate")]).execute(&mut host),
        Err(ToggleError::MissingInteractable(custom_hash("gate")))
    );
}

#[derive(Default)]
struct World {
    host: Host,
}
impl ScriptWorld for World {
    type Error = &'static str;
    fn facts(&self) -> ScriptWorldFacts {
        ScriptWorldFacts::default()
    }
    fn toggle_host(&mut self) -> Option<&mut dyn ToggleHost<Error = Self::Error>> {
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
fn general_quick_execute_dispatch() {
    let layout = SceneScriptLayout::read(
        br#"<Scene><trigger id="t" autorun="false"><then>
        <counter name="c" value="5" action="set"/><timer name="t1" value="2"/><playSound cue="x"/>
        <removeHint/><unlockAchievement achievement="a"/><unrecoveredAction/></then></trigger></Scene>"#,
        false,
    )
    .unwrap();
    let mut rt = ScriptRuntime::new(layout, LevelVariables::default());
    let mut world = World::default();
    // Timer only modifies an existing timer.
    rt.variables.add_timer(custom_hash("t1"), false, 0.0);
    for index in 0..5 {
        rt.quick_execute(index, &mut world).unwrap();
    }
    // Counter.QuickExecute is empty; Timer executes; audio has no host and is empty.
    assert_eq!(rt.variables.counter(custom_hash("c")), 0);
    assert_eq!(rt.variables.timer(custom_hash("t1")), 2.0);
    assert_eq!(world.host.ops, ["remove hint"]);
    assert_eq!(rt.history(), &[] as &[usize]);
    assert_eq!(
        rt.quick_execute(5, &mut world),
        Err(ScriptRuntimeError::UnsupportedAction(5))
    );
    assert_eq!(
        rt.quick_execute(6, &mut world),
        Err(ScriptRuntimeError::InvalidAction(6))
    );
    rt.execute_trigger(
        custom_hash("t"),
        None,
        false,
        NetworkRole::Server,
        &mut world,
    )
    .unwrap_err();
}
