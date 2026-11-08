use magicka_runtime::{
    animation_action::AnimationAction,
    level_variables::LevelVariables,
    scene_command::*,
    scene_loading::custom_hash,
    scene_script::{ActionDefinition, SceneScriptLayout},
    script_runtime::*,
};
use magicka_sim::damage_delivery::NetworkRole;

fn read(tag: &str, attrs: &[(&str, &str)]) -> SceneCommand {
    SceneCommand::read(tag, attrs.iter().copied()).unwrap()
}

#[test]
fn color_packing_follows_xna_pack_unorm() {
    assert_eq!(pack_unorm8(0.5), 128); // 127.5 ties to even
    assert_eq!(pack_unorm8(0.5 / 255.0 * 1.0), 0); // 0.5 ties to even
    assert_eq!(pack_unorm8(f32::NAN), 0);
    assert_eq!(pack_unorm8(f32::INFINITY), 255);
    assert_eq!(pack_unorm8(-2.0), 0);
    // Retail spelling with byte-like values clamps to one.
    assert_eq!(
        xna_color("255,255,255,0", "color").unwrap(),
        [255, 255, 255, 0]
    );
    assert_eq!(
        xna_color("0.0,0.0,0.0,0.25", "color").unwrap(),
        [0, 0, 0, 64]
    );
    assert_eq!(xna_color("0,0,0,0.9,7", "color").unwrap(), [0, 0, 0, 230]);
    assert!(xna_color("0,0,0", "color").is_err());
    assert!(xna_vector3("1,2", "offset").is_err());
    assert_eq!(xna_vector3("1,2,3,4", "offset").unwrap(), [1.0, 2.0, 3.0]);
}

#[test]
fn decoding_defaults_and_setter_rules() {
    use SceneCommandKind::*;
    assert_eq!(
        read("Cutscene", &[("id", "ignored")]).kind,
        Cutscene {
            on_skip: 0,
            skip_bar_move: false,
            kill_dialogs: true
        }
    );
    assert_eq!(
        read("cameraMove", &[]).kind,
        CameraMove {
            lock_input: false,
            time: 1.0,
            target: 0,
            offset: [0.0; 3],
            magnification: -1.0,
            interpolation: CameraInterpolation::Interpolated
        }
    );
    assert_eq!(read("cameraRelease", &[]).kind, CameraRelease { time: 1.0 });
    assert_eq!(
        read("cameraMagnify", &[]).kind,
        CameraMagnify {
            magnification: 1.0,
            time: 1.0
        }
    );
    assert_eq!(
        read("cameraShake", &[("area", "")]).kind,
        CameraShake {
            magnitude: 0.0,
            time: 0.0,
            area: None
        }
    );
    let c = read(
        "camerashake",
        &[("Area", "Quake"), ("magnitude", "2"), ("delay", "0.5")],
    );
    assert_eq!(
        c.kind,
        CameraShake {
            magnitude: 2.0,
            time: 0.0,
            area: Some(custom_hash("quake"))
        }
    );
    assert_eq!(c.delay, 0.5);
    let m = read(
        "cameramove",
        &[
            ("target", "Gate"),
            ("offset", "0,0,-2"),
            ("interpolation", "LINEAR"),
        ],
    );
    assert!(
        matches!(m.kind, CameraMove { target, offset: [0.0, 0.0, -2.0], interpolation: CameraInterpolation::Linear, .. } if target == custom_hash("gate"))
    );
    assert!(SceneCommand::read("cameraLock", [("area", "")]).is_err());
    assert!(SceneCommand::read("fade", [("color", "1,1")]).is_err());
    assert!(SceneCommand::read("cameramove", [("interpolation", "smooth")]).is_err());
    // Properties of other commands are ignored like any unknown attribute.
    assert_eq!(read("disableInput", &[("time", "x")]).kind, DisableInput);
}

#[derive(Default)]
struct Host {
    ops: Vec<String>,
    locator: Option<[f32; 16]>,
    areas: Vec<i32>,
}
fn translation(t: [f32; 3]) -> [f32; 16] {
    let mut m = [0.0; 16];
    m[12..15].copy_from_slice(&t);
    m
}
impl SceneCommandHost for Host {
    type Error = &'static str;
    fn begin_cutscene(&mut self, skip: i32, bar: bool, kill: bool) -> Result<(), Self::Error> {
        self.ops.push(format!(
            "begin {} {bar} {kill}",
            skip == custom_hash("skip")
        ));
        Ok(())
    }
    fn end_cutscene(&mut self, bar: bool) -> Result<(), Self::Error> {
        self.ops.push(format!("end {bar}"));
        Ok(())
    }
    fn limit_input(&mut self, limited: bool) -> Result<(), Self::Error> {
        self.ops.push(format!("limit {limited}"));
        Ok(())
    }
    fn fade(&mut self, fade_in: bool, c: [u8; 4], t: f32) -> Result<(), Self::Error> {
        self.ops.push(format!("fade {fade_in} {c:?} {t}"));
        Ok(())
    }
    fn clear_transition(&mut self) -> Result<(), Self::Error> {
        self.ops.push("clear".into());
        Ok(())
    }
    fn camera_release(&mut self, t: f32) -> Result<(), Self::Error> {
        self.ops.push(format!("release {t}"));
        Ok(())
    }
    fn try_locator(&mut self, _: i32) -> Result<Option<[f32; 16]>, Self::Error> {
        Ok(self.locator)
    }
    fn camera_ground_position(&mut self) -> Result<[f32; 3], Self::Error> {
        self.ops.push("ground".into());
        Ok([10.0, 0.0, 10.0])
    }
    fn camera_move_to(&mut self, p: [f32; 3], t: f32) -> Result<(), Self::Error> {
        self.ops.push(format!("move {p:?} {t}"));
        Ok(())
    }
    fn set_camera_interpolation(&mut self, v: CameraInterpolation) -> Result<(), Self::Error> {
        self.ops.push(format!("interp {v:?}"));
        Ok(())
    }
    fn set_camera_magnification(&mut self, v: f32) -> Result<(), Self::Error> {
        self.ops.push(format!("mag {v}"));
        Ok(())
    }
    fn set_camera_time(&mut self, v: f32) -> Result<(), Self::Error> {
        self.ops.push(format!("time {v}"));
        Ok(())
    }
    fn set_camera_lock_input(&mut self, v: bool) -> Result<(), Self::Error> {
        self.ops.push(format!("lock {v}"));
        Ok(())
    }
    fn locator(&mut self, _: i32) -> Result<[f32; 16], Self::Error> {
        self.locator.ok_or("No area")
    }
    fn camera_shake(&mut self, at: Option<[f32; 3]>, m: f32, t: f32) -> Result<(), Self::Error> {
        self.ops.push(format!("shake {at:?} {m} {t}"));
        Ok(())
    }
    fn camera_lock_on(&mut self, area: i32, t: f32) -> Result<bool, Self::Error> {
        self.ops.push(format!("lockon {t}"));
        Ok(self.areas.contains(&area))
    }
}

#[test]
fn execute_and_quick_execute_host_operations() {
    let mut host = Host::default();
    for (tag, attrs) in [
        (
            "cutscene",
            vec![("onSkip", "Skip"), ("killDialogs", "false")],
        ),
        ("endCutscene", vec![]),
        ("disableInput", vec![]),
        ("enableInput", vec![]),
        (
            "fade",
            vec![
                ("fadeIn", "true"),
                ("color", "0.0,0.0,0.0,1"),
                ("time", "2"),
                ("override", "true"),
            ],
        ),
        ("fade", vec![("time", "1")]),
        ("cameraRelease", vec![("time", "0.5")]),
        (
            "cameraMove",
            vec![("offset", "0,1,0"), ("magnification", "0"), ("time", "3")],
        ),
        ("cameraMagnify", vec![("magnification", "1.5")]),
        ("cameraShake", vec![("magnitude", "2"), ("time", "1")]),
    ] {
        read(tag, &attrs).execute(&mut host).unwrap();
    }
    assert_eq!(
        host.ops,
        [
            "begin true false false",
            "end false",
            "limit true",
            "limit false",
            "fade true [0, 0, 0, 255] 2",
            "clear",
            "fade false [0, 0, 0, 0] 1",
            "release 0.5",
            "ground",
            "move [10.0, 1.0, 10.0] 3",
            "interp Interpolated",
            "lock false",
            "mag 1.5",
            "time 1",
            "shake None 2 1",
        ]
    );
    let mut host = Host {
        locator: Some(translation([1.0, 2.0, 3.0])),
        ..Default::default()
    };
    read(
        "cameraMove",
        &[
            ("target", "gate"),
            ("magnification", "2"),
            ("lockInput", "true"),
        ],
    )
    .execute(&mut host)
    .unwrap();
    read(
        "cameraShake",
        &[("area", "gate"), ("magnitude", "1"), ("time", "2")],
    )
    .execute(&mut host)
    .unwrap();
    assert_eq!(
        host.ops,
        [
            "move [1.0, 2.0, 3.0] 1",
            "interp Interpolated",
            "mag 2",
            "lock true",
            "shake Some([1.0, 2.0, 3.0]) 1 2"
        ]
    );
    let lock = read("cameraLock", &[("area", "box")]);
    assert_eq!(
        lock.execute(&mut Host::default()),
        Err(SceneCommandError::MissingArea(custom_hash("box")))
    );
    let mut host = Host {
        areas: vec![custom_hash("box")],
        ..Default::default()
    };
    lock.execute(&mut host).unwrap();
    assert_eq!(
        read("cameraShake", &[("area", "x")]).execute(&mut Host::default()),
        Err(SceneCommandError::Host("No area"))
    );

    let mut host = Host::default();
    for (tag, attrs) in [
        ("cutscene", vec![("skipBarMove", "false")]),
        ("endCutscene", vec![("skipBarMove", "false")]),
        ("disableInput", vec![]),
        ("fade", vec![("fadeIn", "true")]),
        ("cameraMove", vec![]),
        ("cameraLock", vec![("area", "missing")]),
    ] {
        read(tag, &attrs).quick_execute(&mut host).unwrap();
    }
    assert_eq!(
        host.ops,
        ["begin false true true", "end true", "limit true"]
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
    fn scene_command_host(&mut self) -> Option<&mut dyn SceneCommandHost<Error = Self::Error>> {
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
fn runtime_dispatches_scene_commands_in_queue_order() {
    let layout = SceneScriptLayout::read(
        br#"<Scene><trigger id="intro" autorun="false"><then><cutscene delay="0.5"/><disableInput/><fade/></then></trigger></Scene>"#,
        false,
    )
    .unwrap();
    assert!(layout
        .actions
        .iter()
        .all(|a| matches!(a, ActionDefinition::Scene(_))));
    let mut rt = ScriptRuntime::new(layout, LevelVariables::default());
    let mut world = World {
        host: Some(Host::default()),
    };
    rt.execute_trigger(
        custom_hash("intro"),
        None,
        false,
        NetworkRole::Server,
        &mut world,
    )
    .unwrap();
    for index in 0..3 {
        rt.update_action(index, 0.25, NetworkRole::Server, &mut world)
            .unwrap();
    }
    assert_eq!(
        world.host.as_ref().unwrap().ops,
        ["limit true", "fade false [0, 0, 0, 0] 0"]
    );
    rt.update_action(0, 0.25, NetworkRole::Server, &mut world)
        .unwrap();
    assert_eq!(
        world.host.as_ref().unwrap().ops[2],
        "begin false false true"
    );
    assert_eq!(rt.history(), [1, 2, 0]);
    rt.quick_execute_scene_command(2, &mut world).unwrap();
    assert_eq!(world.host.as_ref().unwrap().ops.len(), 3);
    world.host = None;
    rt.trigger_action::<&str>(1).unwrap();
    assert_eq!(
        rt.update_action(1, 0.0, NetworkRole::Server, &mut world),
        Err(ScriptRuntimeError::MissingSceneCommandHost(1))
    );
}
