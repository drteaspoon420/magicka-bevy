use magicka_runtime::{dialog_definition::*, message_playback::*, scene_loading::custom_hash};
#[derive(Default)]
struct Host {
    owner: Option<MessageOwner>,
    cue: Option<bool>,
    effects: Vec<&'static str>,
    fail_show: bool,
    sounds: Vec<i32>,
    resolutions: usize,
}
impl MessageHost for Host {
    type Error = &'static str;
    fn resolve_owner(&mut self, _: i32) -> Result<Option<MessageOwner>, Self::Error> {
        self.resolutions += 1;
        Ok(self.owner)
    }
    fn cue_stopping(&self) -> Option<bool> {
        self.cue
    }
    fn effect(&mut self, _: &MessageDefinition, effect: MessageEffect) -> Result<(), Self::Error> {
        self.effects.push(match effect {
            MessageEffect::StopCue => "stop",
            MessageEffect::PlayCue { sound, .. } => {
                self.sounds.push(sound);
                "play"
            }
            MessageEffect::Animate {
                animation, blend, ..
            } => {
                assert_eq!(blend, 0.3);
                if animation == 1 {
                    "idle"
                } else {
                    "animate"
                }
            }
            MessageEffect::MoveCamera { time, .. } => {
                assert_eq!(time, 2.0);
                "move"
            }
            MessageEffect::Magnification(value) => {
                assert_eq!(value, 1.5);
                "zoom"
            }
            MessageEffect::ReleaseCamera(value) => {
                assert_eq!(value, 2.0);
                "release"
            }
            MessageEffect::Show { .. } => "show",
            MessageEffect::Hide => "hide",
        });
        if self.fail_show && matches!(effect, MessageEffect::Show { .. }) {
            return Err("show failed");
        }
        Ok(())
    }
}
fn definition() -> MessageDefinition {
    MessageDefinition {
        owner: custom_hash("owner"),
        message_type: 0,
        force_on_screen: true,
        facing: true,
        width: 600,
        texts: vec![TextDefinition {
            ttl: -1.0,
            animation: 151,
            sound_bank: 32,
            sound: 12,
            localization: 0,
            arguments: None,
        }],
    }
}
fn owner() -> MessageOwner {
    MessageOwner {
        id: 10,
        dead: false,
        character: true,
        dead_state: false,
        animation_allowed: true,
        special_idle: 0,
    }
}
#[test]
fn screen_advances_then_completes_without_stopping_cue_on_completion() {
    let mut playback = MessagePlayback::default();
    let mut host = Host {
        cue: Some(false),
        ..Default::default()
    };
    assert!(!playback
        .advance(&definition(), MessagePlacement::Screen([0.0; 2]), &mut host)
        .unwrap());
    assert_eq!(host.effects, ["stop", "play", "show"]);
    assert_eq!(playback.current, 1);
    host.effects.clear();
    assert!(playback
        .advance(&definition(), MessagePlacement::Screen([0.0; 2]), &mut host)
        .unwrap());
    assert_eq!(host.effects, ["hide"]);
    assert_eq!(playback.current, 0);
}
#[test]
fn world_character_orders_animation_audio_camera_and_completion_idle() {
    let placement = MessagePlacement::World {
        position: [0.0; 3],
        focus: true,
    };
    let mut playback = MessagePlayback::default();
    let mut host = Host {
        owner: Some(owner()),
        cue: Some(false),
        ..Default::default()
    };
    playback
        .advance(&definition(), placement, &mut host)
        .unwrap();
    assert_eq!(
        host.effects,
        ["stop", "animate", "play", "move", "zoom", "show"]
    );
    host.effects.clear();
    assert!(playback
        .advance(&definition(), placement, &mut host)
        .unwrap());
    assert_eq!(host.effects, ["stop", "hide", "release", "idle"]);
}
#[test]
fn world_fallback_plays_zero_sound_but_screen_does_not() {
    let mut definition = definition();
    definition.texts[0].sound = 0;
    let mut playback = MessagePlayback::default();
    let mut host = Host::default();
    playback
        .advance(
            &definition,
            MessagePlacement::World {
                position: [0.0; 3],
                focus: false,
            },
            &mut host,
        )
        .unwrap();
    assert_eq!(host.sounds, [0]);
    assert_eq!(host.effects, ["play", "show"]);
    playback.reset();
    host.effects.clear();
    host.sounds.clear();
    playback
        .advance(&definition, MessagePlacement::Screen([0.0; 2]), &mut host)
        .unwrap();
    assert_eq!(host.effects, ["show"]);
    assert!(host.sounds.is_empty());
}
#[test]
fn dead_entity_skips_without_hiding_or_resetting_and_empty_character_error_preserves_effects() {
    let placement = MessagePlacement::Entity {
        default_owner: Some(10),
        focus: true,
    };
    let mut definition = definition();
    definition.texts.push(definition.texts[0].clone());
    let mut playback = MessagePlayback::default();
    let mut host = Host {
        owner: Some(MessageOwner {
            dead: true,
            ..owner()
        }),
        cue: Some(false),
        ..Default::default()
    };
    assert!(playback.advance(&definition, placement, &mut host).unwrap());
    assert_eq!(playback.current, 2);
    assert_eq!(host.resolutions, 2);
    assert!(host.effects.is_empty());
    host.owner = Some(owner());
    definition.texts.clear();
    assert_eq!(
        playback.advance(&definition, placement, &mut host),
        Err(MessagePlaybackError::MissingText(0))
    );
    assert_eq!(playback.current, 0);
    assert_eq!(host.effects, ["stop", "hide", "release"]);
}
#[test]
fn entity_event_gate_and_failed_show_preserve_cursor_and_prior_effects() {
    let placement = MessagePlacement::Entity {
        default_owner: Some(10),
        focus: false,
    };
    let mut playback = MessagePlayback::default();
    let mut host = Host {
        owner: Some(MessageOwner {
            animation_allowed: false,
            ..owner()
        }),
        fail_show: true,
        ..Default::default()
    };
    assert_eq!(
        playback.advance(&definition(), placement, &mut host),
        Err(MessagePlaybackError::Host("show failed"))
    );
    assert_eq!(playback.current, 0);
    assert_eq!(host.effects, ["play", "show"]);
    host.fail_show = false;
    playback
        .advance(&definition(), placement, &mut host)
        .unwrap();
    host.effects.clear();
    host.owner.as_mut().unwrap().dead_state = true;
    assert!(playback
        .advance(&definition(), placement, &mut host)
        .unwrap());
    assert_eq!(host.effects, ["hide"]);
}

#[test]
fn parsed_dialog_selects_message_and_advances_all_texts_before_completion() {
    let definitions = DialogDefinition::read_collection(&[br#"<Dialogs><dialog id="intro" repeatFrom="-1"><interact><message><text sound="first">#first</text><text sound="second">#second</text></message></interact></dialog></Dialogs>"#]).unwrap();
    let dialog = &definitions[0];
    let selected = dialog.cursor().pop().unwrap().unwrap();
    let message = &dialog.interactions[selected.interaction].messages[0];
    let mut playback = MessagePlayback::default();
    let mut host = Host::default();
    for _ in 0..2 {
        assert!(!playback
            .advance(message, MessagePlacement::Screen([0.0; 2]), &mut host)
            .unwrap());
    }
    assert_eq!(host.sounds, [custom_hash("first"), custom_hash("second")]);
    assert!(playback
        .advance(message, MessagePlacement::Screen([0.0; 2]), &mut host)
        .unwrap());
    assert_eq!(host.effects, ["play", "show", "play", "show", "hide"]);
}
