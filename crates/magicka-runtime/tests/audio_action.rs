use magicka_runtime::{
    animation_action::AnimationAction, audio_action::*, level_variables::LevelVariables,
    scene_loading::custom_hash, scene_script::SceneScriptLayout, script_runtime::*,
};
use magicka_sim::damage_delivery::NetworkRole;

fn read(tag: &str, attrs: &[(&str, &str)]) -> AudioAction {
    AudioAction::read(tag, attrs.iter().copied()).unwrap()
}

#[test]
fn decoding_banks_cues_and_empty_strings() {
    let AudioActionKind::PlaySound(s) = read("playSound", &[]).kind else {
        panic!()
    };
    assert_eq!(
        (s.id, s.area, s.bank, s.cue, s.volume),
        (None, None, 0, 0, 0.0)
    );
    let AudioActionKind::PlaySound(s) = read(
        "PlaySound",
        &[("cue", "Wind_Loop"), ("id", ""), ("area", "")],
    )
    .kind
    else {
        panic!()
    };
    assert_eq!(
        (s.id, s.area, s.bank, s.cue),
        (None, None, BANK_AMBIENCE, custom_hash("wind_loop"))
    );
    let AudioActionKind::PlaySound(s) = read(
        "playsound",
        &[
            ("cue", "AdditionalMusic/Boss"),
            ("ID", "Amb"),
            ("area", "Fire"),
            ("radius", "4"),
            ("apply3d", "true"),
            ("volume", "0.5"),
        ],
    )
    .kind
    else {
        panic!()
    };
    assert_eq!(s.bank, 0x400);
    assert_eq!(
        (s.cue, s.id, s.area),
        (
            custom_hash("boss"),
            Some(custom_hash("amb")),
            Some(custom_hash("fire"))
        )
    );
    assert_eq!((s.radius, s.volume, s.apply_3d), (4.0, 0.5, true));
    assert!(AudioAction::read("playsound", [("cue", "a/b/c")]).is_err());
    assert!(AudioAction::read("playsound", [("cue", "nobank/x")]).is_err());
    // PlayMusic ignores components after the second; default bank is Music.
    assert_eq!(
        read("playMusic", &[("cue", "Misc/Theme/extra")]).kind,
        AudioActionKind::PlayMusic {
            bank: 0x100,
            cue: custom_hash("theme"),
            focus: None
        }
    );
    assert_eq!(
        read("playmusic", &[("cue", "Theme"), ("focusValue", "0")]).kind,
        AudioActionKind::PlayMusic {
            bank: BANK_MUSIC,
            cue: custom_hash("theme"),
            focus: Some(0.0)
        }
    );
    assert_eq!(
        read("playmusic", &[]).kind,
        AudioActionKind::PlayMusic {
            bank: 2,
            cue: 0,
            focus: None
        }
    );
    assert_eq!(
        read("stopSound", &[("id", "Amb"), ("instant", "true")]).kind,
        AudioActionKind::StopSound {
            id: custom_hash("amb"),
            instant: true
        }
    );
    assert_eq!(read("StopMusic", &[("delay", "2")]).delay, 2.0);
}

#[derive(Default)]
struct Host {
    ops: Vec<String>,
    known: Vec<i32>,
}
impl AudioActionHost for Host {
    type Error = &'static str;
    fn play_global_ambient(&mut self, _: i32, b: u16, _: i32, v: f32) -> Result<(), Self::Error> {
        self.ops.push(format!("global {b} {v}"));
        Ok(())
    }
    fn play_located_ambient(&mut self, _: i32, s: LocatedSound) -> Result<(), Self::Error> {
        self.ops
            .push(format!("located {} {}", s.radius, s.apply_3d));
        Ok(())
    }
    fn play_cue(&mut self, _: u16, cue: i32, _: f32) -> Result<bool, Self::Error> {
        self.ops.push("cue".into());
        Ok(self.known.contains(&cue))
    }
    fn play_unregistered_locator(&mut self, _: LocatedSound) -> Result<(), Self::Error> {
        self.ops.push("transient".into());
        Ok(())
    }
    fn stop_sound(&mut self, _: i32, instant: bool) -> Result<(), Self::Error> {
        self.ops.push(format!("stop {instant}"));
        Ok(())
    }
    fn play_music(&mut self, b: u16, _: i32, f: Option<f32>) -> Result<(), Self::Error> {
        self.ops.push(format!("music {b} {f:?}"));
        Ok(())
    }
    fn stop_music(&mut self) -> Result<(), Self::Error> {
        self.ops.push("stop music".into());
        Ok(())
    }
}

#[test]
fn execute_routes_by_id_and_area() {
    let mut host = Host {
        known: vec![custom_hash("hit")],
        ..Default::default()
    };
    for (tag, attrs) in [
        (
            "playsound",
            vec![("id", "a"), ("cue", "x"), ("volume", "1")],
        ),
        (
            "playsound",
            vec![("id", "a"), ("area", "z"), ("radius", "3")],
        ),
        ("playsound", vec![("cue", "misc/hit")]),
        ("playsound", vec![("area", "z")]),
        ("stopsound", vec![("id", "a")]),
        ("playmusic", vec![("cue", "theme")]),
        ("stopmusic", vec![]),
    ] {
        read(tag, &attrs).execute(&mut host).unwrap();
    }
    assert_eq!(
        host.ops,
        [
            "global 4 1",
            "located 3 false",
            "cue",
            "transient",
            "stop false",
            "music 2 None",
            "stop music"
        ]
    );
    assert_eq!(
        read("playsound", &[("cue", "unknown")]).execute(&mut host),
        Err(AudioActionError::MissingCue(custom_hash("unknown")))
    );
}

#[derive(Default)]
struct Backend {
    ops: Vec<String>,
    fail_stop: bool,
}
impl AmbientBackend for Backend {
    type Error = &'static str;
    type Located = i32;
    type Global = (i32, f32);
    fn start_located(&mut self, id: i32, _: LocatedSound) -> Result<i32, Self::Error> {
        self.ops.push("start located".into());
        Ok(id)
    }
    fn start_global(&mut self, _: u16, cue: i32, volume: f32) -> Result<(i32, f32), Self::Error> {
        self.ops.push(format!("start global {volume}"));
        Ok((cue, volume))
    }
    fn replay_global(&mut self, sound: &mut (i32, f32), volume: f32) -> Result<(), Self::Error> {
        sound.1 = volume;
        self.ops.push(format!("replay {volume}"));
        Ok(())
    }
    fn stop_located(&mut self, _: &mut i32, instant: bool) -> Result<(), Self::Error> {
        self.ops.push(format!("stop located {instant}"));
        if self.fail_stop {
            Err("stop failed")
        } else {
            Ok(())
        }
    }
    fn stop_global(&mut self, _: &mut (i32, f32), instant: bool) -> Result<(), Self::Error> {
        self.ops.push(format!("stop global {instant}"));
        Ok(())
    }
}

#[test]
fn ambient_registry_preserves_gamescene_dictionary_rules() {
    let mut sounds = AmbientSounds::default();
    let mut b = Backend::default();
    let s = LocatedSound {
        bank: 4,
        cue: 1,
        volume: 1.0,
        area: 2,
        radius: 3.0,
        apply_3d: true,
    };
    sounds.play_located(&mut b, 7, s).unwrap();
    sounds.play_located(&mut b, 7, s).unwrap();
    sounds.play_global(&mut b, 7, 4, 5, 0.25).unwrap();
    sounds.play_global(&mut b, 7, 8, 9, 0.75).unwrap();
    // The original cue is kept; only volume changes.
    assert_eq!(sounds.global[&7], (5, 0.75));
    sounds.stop(&mut b, 7, true).unwrap();
    assert!(!sounds.located.contains_key(&7));
    sounds.stop(&mut b, 7, false).unwrap();
    sounds.stop(&mut b, 7, false).unwrap();
    assert!(
        sounds.global.contains_key(&7),
        "global entries survive StopSound"
    );
    sounds.stop(&mut b, 99, false).unwrap();
    assert_eq!(
        b.ops,
        [
            "start located",
            "start global 0.25",
            "replay 0.75",
            "stop located true",
            "stop global false",
            "stop global false",
        ]
    );
    sounds.play_located(&mut b, 8, s).unwrap();
    b.fail_stop = true;
    assert_eq!(sounds.stop(&mut b, 8, false), Err("stop failed"));
    assert!(sounds.located.contains_key(&8));

    let dict = sound_dictionary(["Goblin_Die", "orc_hit"]).unwrap();
    assert_eq!(dict[&custom_hash("goblin_die")], "Goblin_Die");
    assert!(sound_dictionary(["a", "A"]).is_err());
}

struct World {
    host: Option<Host>,
}
impl ScriptWorld for World {
    type Error = &'static str;
    fn facts(&self) -> ScriptWorldFacts {
        ScriptWorldFacts::default()
    }
    fn audio_host(&mut self) -> Option<&mut dyn AudioActionHost<Error = Self::Error>> {
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
fn runtime_dispatches_audio_and_reports_missing_capability() {
    let layout = SceneScriptLayout::read(
        br#"<Scene><trigger id="t" autorun="false"><then><playMusic cue="x"/><stopSound id="a"/></then></trigger></Scene>"#,
        false,
    )
    .unwrap();
    let mut rt = ScriptRuntime::new(layout, LevelVariables::default());
    let mut world = World {
        host: Some(Host::default()),
    };
    rt.execute_trigger(
        custom_hash("t"),
        None,
        false,
        NetworkRole::Server,
        &mut world,
    )
    .unwrap();
    rt.update_action(0, 0.0, NetworkRole::Server, &mut world)
        .unwrap();
    world.host = None;
    assert_eq!(
        rt.update_action(1, 0.0, NetworkRole::Server, &mut world),
        Err(ScriptRuntimeError::MissingAudioHost(1))
    );
    assert_eq!(rt.history(), [0]);
}
