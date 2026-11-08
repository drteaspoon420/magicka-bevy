use magicka_runtime::{
    frontend_cutscene::{
        parse_map_cutscenes, CueState, MapCutscene, MapCutsceneEvent as Event, MapCutscenePlayback,
        MapPhase,
    },
    scene_loading::custom_hash,
};

fn scene() -> MapCutscene {
    parse_map_cutscenes(
        br##"<Cutscenes><Cutscene id="3">
        <Camera>
          <zoom time="8">2</zoom><zoom>1</zoom>
          <position time="8">80,160</position><position>0,0</position>
        </Camera>
        <Dialog subtitles="#Example">Additional/ExampleCue</Dialog>
    </Cutscene></Cutscenes>"##,
    )
    .unwrap()
    .remove(0)
    .1
}

#[test]
fn camera_sorts_tracks_clamps_endpoints_and_uses_smoothstep() {
    let cutscene = scene();
    assert_eq!(cutscene.duration, 8.0);
    assert_eq!(cutscene.dialog_bank, 512);
    assert_eq!(cutscene.dialog, custom_hash("examplecue"));
    assert_eq!(cutscene.subtitles, custom_hash("#example"));
    assert_eq!(cutscene.camera(-1.0).position, [0.0, 0.0]);
    assert_eq!(cutscene.camera(9.0).position, [80.0, 160.0]);
    assert_eq!(cutscene.camera(2.0).position, [12.5, 25.0]);
    assert_eq!(cutscene.camera(2.0).zoom, 1.15625);
    assert_eq!(
        cutscene.camera(8.0).image_transform([2048.0; 2]),
        ([352.0, 192.0], 2.0)
    );
}

#[test]
fn duplicate_keys_use_last_equal_key_and_tracks_hold_independently() {
    let scene = parse_map_cutscenes(
        br#"<Cutscenes><Cutscene id="0"><camera>
        <zoom>1</zoom><position time="2">1,2</position>
        <position time="2">3,4</position><position time="4">7,8</position>
    </camera></Cutscene></Cutscenes>"#,
    )
    .unwrap()
    .remove(0)
    .1;
    assert_eq!(scene.duration, 4.0);
    assert_eq!(scene.camera(0.0).position, [1.0, 2.0]);
    assert_eq!(scene.camera(2.0).position, [3.0, 4.0]);
    assert_eq!(scene.camera(3.0).position, [5.0, 6.0]);
    assert_eq!(scene.camera(100.0).zoom, 1.0);
}

#[test]
fn malformed_tracks_and_cues_fail_explicitly() {
    for body in [
        "<camera><zoom>1</zoom></camera>",
        "<camera><position>1,2</position></camera>",
        "<camera><position>1</position><zoom>1</zoom></camera>",
        "<camera><position>1,2</position><zoom>NaN</zoom></camera>",
        "<camera><position>1,2</position><zoom>1</zoom></camera><dialog>a/b/c</dialog>",
        "<camera><position>1,2</position><zoom>1</zoom></camera><dialog>invalid/cue</dialog>",
    ] {
        let xml = format!("<Cutscenes><Cutscene id=\"0\">{body}</Cutscene></Cutscenes>");
        assert!(parse_map_cutscenes(xml.as_bytes()).is_err(), "{body}");
    }
}

#[test]
fn chapter_to_map_starts_audio_and_subtitles_once_without_carrying_overshoot() {
    let mut playback = MapCutscenePlayback::enter(true, Some(&scene()), true);
    assert!(playback.update(2.5, CueState::Prepared).is_empty());
    assert_eq!(playback.phase, MapPhase::Chapter);
    assert_eq!(
        playback.update(1.0, CueState::Prepared),
        [Event::Riffle, Event::PlayCue, Event::ShowSubtitles]
    );
    assert_eq!(playback.time, 0.0);
    assert!(playback.update(0.25, CueState::Playing).is_empty());
    assert_eq!(playback.time, 0.25);
}

#[test]
fn both_camera_and_cue_must_finish_before_fade_and_host_handoff() {
    for cue in [CueState::Absent, CueState::Stopping, CueState::Stopped] {
        let mut playback = MapCutscenePlayback::enter(true, Some(&scene()), false);
        playback.update(3.0, CueState::Playing);
        playback.update(8.0, cue);
        assert_eq!(
            playback.phase,
            MapPhase::Map,
            "duration test is strictly greater"
        );
        playback.update(0.25, CueState::Prepared);
        assert_eq!(playback.phase, MapPhase::Map, "prepared is not stopped");
        playback.update(0.25, CueState::Playing);
        assert_eq!(playback.phase, MapPhase::Map, "camera alone cannot finish");
        assert!(playback.update(0.0, cue).is_empty());
        assert_eq!(playback.phase, MapPhase::FinishPending);
        assert_eq!(playback.transition_end(), None);
        assert_eq!(
            playback.update(0.0, cue),
            [Event::HideSubtitles, Event::FadeToBlack { seconds: 0.5 }]
        );
        assert!(playback.update(20.0, cue).is_empty());
        assert_eq!(
            playback.phase,
            MapPhase::Fading,
            "host owns transition duration"
        );
        assert_eq!(playback.transition_end(), Some(Event::LoadLevel));
        assert_eq!(playback.transition_end(), None);
    }
}

#[test]
fn skip_requires_open_tome_and_handles_prepared_and_playing_cues() {
    for controller in [false, true] {
        let mut playback = MapCutscenePlayback::enter(true, Some(&scene()), true);
        assert!(playback
            .skip(false, CueState::Playing, controller)
            .is_empty());
        assert_eq!(playback.phase, MapPhase::Chapter);
        let expected = if controller {
            vec![Event::StopCue]
        } else {
            vec![]
        };
        assert_eq!(
            playback.skip(true, CueState::Prepared, controller),
            expected
        );
        assert_eq!(
            playback.update(0.0, CueState::Playing),
            [
                Event::StopCue,
                Event::HideSubtitles,
                Event::FadeToBlack { seconds: 0.5 }
            ]
        );
        assert!(playback
            .skip(true, CueState::Playing, controller)
            .is_empty());
    }
}

#[test]
fn missing_data_or_disabled_play_uses_the_same_fade_path() {
    let cutscene = scene();
    for (play, data) in [(true, None), (false, Some(&cutscene))] {
        let mut playback = MapCutscenePlayback::enter(play, data, false);
        assert_eq!(playback.phase, MapPhase::FinishPending);
        assert_eq!(
            playback.update(0.0, CueState::Absent),
            [Event::HideSubtitles, Event::FadeToBlack { seconds: 0.5 }]
        );
    }
}

#[test]
fn retail_camera_catalogues_decode_and_sample_without_target_execution() {
    let content =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !content.is_dir() {
        return;
    }
    let mut total = 0;
    for (file, expected) in [
        ("Levels/Cutscenes.xml", 12),
        ("Levels/Tsar/Cutscenes.xml", 3),
        ("Levels/Challenges/Dungeons_Cutscenes.xml", 1),
        ("Levels/Challenges/Dungeons2_Cutscenes.xml", 1),
    ] {
        let entries = parse_map_cutscenes(&std::fs::read(content.join(file)).unwrap()).unwrap();
        assert_eq!(entries.len(), expected, "{file}");
        for (_, scene) in &entries {
            for time in [
                -1.0,
                0.0,
                scene.duration / 4.0,
                scene.duration,
                scene.duration + 1.0,
            ] {
                let camera = scene.camera(time);
                assert!(camera.zoom.is_finite() && camera.position.iter().all(|v| v.is_finite()));
            }
        }
        total += entries.len();
    }
    assert_eq!(total, 17);
}
