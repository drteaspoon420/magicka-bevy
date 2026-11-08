use magicka_content::assets::{BitmapFont, Glyph};
use magicka_runtime::{
    bitmap_font::FontMetrics, dialog_definition::*, dialog_frame::DialogFrame, dialog_manager::*,
    dialog_text_host::*, interaction_playback::InteractionLocation,
    localization::LocalizationTable, message_playback::*, message_textbox::MessageTextBoxWorld,
    scene_loading::custom_hash, text_box_initialize::TextBoxInitHost,
};
use magicka_sim::damage_delivery::NetworkRole;
use std::collections::BTreeMap;

#[derive(Default)]
struct World {
    bound: Option<DialogMessageKey>,
    cues: BTreeMap<DialogMessageKey, bool>,
    played: Vec<DialogMessageKey>,
    stopped: Vec<DialogMessageKey>,
    ended: Vec<DialogMessageKey>,
    locks: Vec<i32>,
    unlocks: Vec<i32>,
    effects: Vec<DialogManagerEffect>,
    cutscene: bool,
}
impl MessageHost for World {
    type Error = &'static str;
    fn resolve_owner(&mut self, _: i32) -> Result<Option<MessageOwner>, Self::Error> {
        Ok(None)
    }
    fn cue_stopping(&self) -> Option<bool> {
        self.bound.and_then(|k| self.cues.get(&k).copied())
    }
    fn effect(&mut self, _: &MessageDefinition, effect: MessageEffect) -> Result<(), Self::Error> {
        let key = self.bound.ok_or("no cue binding")?;
        match effect {
            MessageEffect::PlayCue { .. } => {
                self.played.push(key);
                self.cues.insert(key, false);
            }
            MessageEffect::StopCue => {
                self.stopped.push(key);
                self.cues.insert(key, true);
            }
            _ => return Err("unexpected world effect"),
        }
        Ok(())
    }
}
impl TextBoxInitHost for World {
    type Error = &'static str;
    fn load_background(&mut self, path: &str) -> Result<(), Self::Error> {
        assert_eq!(path, "UI/HUD/Dialog_Say");
        Ok(())
    }
}
impl MessageTextBoxWorld for World {
    fn entity_scene(&mut self, _: i32) -> Result<Option<i32>, &'static str> {
        Ok(Some(7))
    }
}
impl DialogTextWorld for World {
    fn bind_message(&mut self, key: DialogMessageKey) -> Result<(), &'static str> {
        self.bound = Some(key);
        Ok(())
    }
    fn lock_controller(&mut self, id: i32) -> Result<(), &'static str> {
        self.locks.push(id);
        Ok(())
    }
    fn unlock_controller(&mut self, id: i32) -> Result<(), &'static str> {
        self.unlocks.push(id);
        Ok(())
    }
    fn stop_message_cue(&mut self) -> Result<(), &'static str> {
        let key = self.bound.ok_or("no cue binding")?;
        self.ended.push(key);
        if let Some(cue) = self.cues.get_mut(&key) {
            *cue = true;
        }
        Ok(())
    }
    fn manager_effect(&mut self, effect: DialogManagerEffect) -> Result<(), &'static str> {
        self.effects.push(effect);
        Ok(())
    }
}
impl DialogTextFrameWorld for World {
    fn text_owner(&self, _: i32) -> Option<magicka_runtime::text_box::TextBoxOwner> {
        None
    }
    fn owner_dead(&self, _: i32) -> bool {
        false
    }
    fn in_cutscene(&self) -> bool {
        self.cutscene
    }
    fn message_box_count(&self) -> usize {
        0
    }
    fn update_message_box(&mut self, _: usize, _: DialogFrame) -> Result<(), &'static str> {
        Err("no message boxes")
    }
    fn message_box_dead(&self, _: usize) -> bool {
        false
    }
    fn remove_message_box(&mut self, _: usize) -> Result<(), &'static str> {
        Err("no message boxes")
    }
    fn submit_subtitles(&mut self, _: usize, _: f32) -> Result<(), &'static str> {
        Err("no subtitles")
    }
}
fn font(height: i32, advance: i32, spacing: i32) -> FontMetrics {
    let source = BitmapFont {
        line_height: height,
        base_line: height - 2,
        default_character: '?',
        texture: None,
        kerning: vec![],
        glyphs: ['A', 'B', 'C', '?']
            .into_iter()
            .map(|character| Glyph {
                character,
                origin: [0; 2],
                size: [advance, height],
                advance_width: advance,
                left_side_bearing: 0,
                force_white: false,
            })
            .collect(),
    };
    let mut metrics = FontMetrics::from_content(&source).unwrap();
    metrics.spacing = spacing;
    metrics
}
fn definitions() -> Vec<DialogDefinition> {
    let mut definitions = DialogDefinition::read_collection(&[br##"<Dialogs>
      <Dialog id="a" repeatFrom="-1"><Interact lock="true">
        <Message character="#name"><Text ttl="0.1">#one</Text><Text ttl="0.2">#two</Text></Message>
        <Message><Text>#three</Text></Message>
      </Interact></Dialog>
      <Dialog id="b" repeatFrom="-1"><Interact><Message><Text>#two</Text></Message></Interact></Dialog>
    </Dialogs>"##]).unwrap();
    for dialog in &mut definitions {
        for interaction in &mut dialog.interactions {
            for message in &mut interaction.messages {
                for text in &mut message.texts {
                    text.sound = 7;
                }
            }
        }
    }
    definitions
}
fn key(dialog: &str, message: usize) -> DialogMessageKey {
    DialogMessageKey {
        collection: 0,
        dialog: custom_hash(dialog),
        interaction: 0,
        message,
    }
}
fn host() -> DialogTextHost<World> {
    let mut strings = LocalizationTable::default();
    for (id, text) in [
        ("#name", "A"),
        ("#one", "AA"),
        ("#two", "BB"),
        ("#three", "CC"),
    ] {
        strings.strings.insert(custom_hash(id), text.into());
    }
    let mut host = DialogTextHost::new(
        World::default(),
        strings,
        font(14, 2, 0),
        font(16, 7, 1),
        100,
    );
    host.scene = Some(7);
    host
}
#[test]
fn concurrent_dialogs_keep_distinct_layouts_boxes_and_cues_through_end_all() {
    let definitions = definitions();
    let mut host = host();
    host.initialize_layouts(&definitions).unwrap();
    let mut manager = DialogManager::new(definitions).unwrap();
    for (i, id) in ["a", "b"].into_iter().enumerate() {
        assert_eq!(
            manager
                .start(
                    custom_hash(id),
                    InteractionLocation::Screen([0.; 2]),
                    Some(i as i32),
                    false,
                    &mut host
                )
                .unwrap(),
            Some(i)
        );
    }
    assert_eq!(host.boxes[0].state.text.as_deref(), Some("AA"));
    assert_eq!(host.boxes[1].state.text.as_deref(), Some("BB"));
    assert!(host.boxes[0].show_name && !host.boxes[1].show_name);
    assert_eq!(host.world.played, [key("a", 0), key("b", 0)]);
    assert!(host.world.stopped.is_empty());
    manager.network_advance(0, &mut host).unwrap(); // finish animations first
    manager.network_advance(0, &mut host).unwrap(); // advance both dialogs
    assert_eq!(host.boxes[0].state.text.as_deref(), Some("BB"));
    assert_eq!(host.world.stopped, [key("a", 0)]);
    assert_eq!(host.world.played, [key("a", 0), key("b", 0), key("a", 0)]);
    assert!(!manager.dialog_active(custom_hash("b")));
    manager.end_all(&mut host).unwrap();
    assert_eq!(host.world.ended, [key("a", 0)]);
    // EndAll finishes a growing regular box before Hide: it remains visible
    // at scale one until subsequent updates shrink it, as native code does.
    assert!(host.boxes.iter().all(|b| !b.state.grow));
    assert!(host.boxes[0].state.visible());
    host.update_regular(
        &mut manager,
        0,
        DialogFrame {
            channel: 0,
            dt: 0.125,
        },
        None,
        false,
    )
    .unwrap();
    assert!(!host.boxes[0].state.visible());
    assert_eq!(host.world.unlocks, [1, 0]);
}
#[test]
fn owned_box_ttl_replacement_preserves_new_state_and_transitions_message_context() {
    let definitions = definitions();
    let mut host = host();
    host.initialize_layouts(&definitions).unwrap();
    let mut manager = DialogManager::new(definitions).unwrap();
    manager
        .start(
            custom_hash("a"),
            InteractionLocation::Screen([0.; 2]),
            Some(3),
            false,
            &mut host,
        )
        .unwrap();
    host.boxes[0].state.typing.publish_primitives(4);
    host.manager_effect(DialogManagerEffect::FinishAnimation {
        sink: DialogSink::Slot(0),
        remove_ttl_advance: false,
    })
    .unwrap();
    host.update_regular(
        &mut manager,
        0,
        DialogFrame {
            channel: 1,
            dt: 0.11,
        },
        None,
        false,
    )
    .unwrap();
    let b = &host.boxes[0].state;
    assert_eq!(b.text.as_deref(), Some("BB"));
    assert_eq!(b.ttl, 0.2);
    assert_eq!(b.typing.visible_characters, 0);
    assert_eq!(b.channels[1].size, b.size);
    manager.tick_input(0.2);
    manager
        .advance_controller(Some(3), false, NetworkRole::Offline, &mut host)
        .unwrap();
    assert!(!host.boxes[0].state.automatic_advance);
    manager
        .advance_controller(Some(3), false, NetworkRole::Offline, &mut host)
        .unwrap();
    assert_eq!(host.boxes[0].state.text.as_deref(), Some("CC"));
    assert_eq!(host.world.played.last(), Some(&key("a", 1)));
    assert_eq!(host.world.stopped, [key("a", 0)]);
    manager.end_all(&mut host).unwrap();
    assert_eq!(host.world.ended, [key("a", 1)]);
}
#[test]
fn explicit_layout_reinitialization_retains_minimum_and_missing_layout_fails_start() {
    let definitions = definitions();
    let mut host = host();
    let mut manager = DialogManager::new(definitions.clone()).unwrap();
    assert!(manager
        .start(
            custom_hash("a"),
            InteractionLocation::Screen([0.; 2]),
            Some(3),
            false,
            &mut host
        )
        .is_err());
    assert!(manager.dialog_active(custom_hash("a")));
    assert!(!host.boxes[0].state.grow);
    assert_eq!(host.world.locks, [3]);
    host.strings
        .strings
        .insert(custom_hash("#one"), "AAAAAA".into());
    host.initialize_layouts(&definitions).unwrap();
    let old = host.layouts[&key("a", 0)].minimum_box_size;
    host.strings.strings.insert(custom_hash("#one"), "A".into());
    host.initialize_layouts(&definitions).unwrap();
    assert_eq!(host.layouts[&key("a", 0)].minimum_box_size, old);
    assert_eq!(host.layouts[&key("a", 0)].texts[0].as_deref(), Some("A"));
    assert_eq!(host.layouts.len(), 3);
}
#[test]
fn frame_host_updates_owned_regular_boxes_and_prunes_additional_after_ttl_hide() {
    let definitions = definitions();
    let mut host = host();
    host.initialize_layouts(&definitions).unwrap();
    let mut manager = DialogManager::new(definitions).unwrap();
    manager
        .start(
            custom_hash("a"),
            InteractionLocation::Screen([0.; 2]),
            None,
            false,
            &mut host,
        )
        .unwrap();
    let mut additional = magicka_runtime::text_box_initialize::TextBoxWidget::default();
    additional
        .state
        .initialize_state(Some("AA"), [20, 10], 0.01);
    additional.state.typing.publish_primitives(4);
    additional.state.finish_animation(false, true);
    host.additional.push(additional);
    manager
        .update(
            DialogFrame {
                channel: 0,
                dt: 0.02,
            },
            &mut [[0.; 4]; 4],
            &mut host,
        )
        .unwrap();
    assert!(!host.additional[0].state.grow);
    assert_eq!(host.boxes[0].state.text.as_deref(), Some("AA"));
    assert!(manager.dialog_active(custom_hash("a")));
    manager
        .update(
            DialogFrame {
                channel: 0,
                dt: 0.125,
            },
            &mut [[0.; 4]; 4],
            &mut host,
        )
        .unwrap();
    assert!(host.additional.is_empty());
    assert_eq!(host.boxes[0].state.scale, 0.58);
}
#[test]
fn overlapping_ids_in_replacement_collection_keep_old_boxes_layouts_and_cues() {
    let definitions = definitions();
    let mut host = host();
    host.initialize_layouts(&definitions).unwrap();
    let mut manager = DialogManager::new(definitions.clone()).unwrap();
    manager
        .start(
            custom_hash("a"),
            InteractionLocation::Screen([0.; 2]),
            None,
            false,
            &mut host,
        )
        .unwrap();
    host.strings
        .strings
        .insert(custom_hash("#one"), "CC".into());
    host.initialize_collection_layouts(1, &definitions).unwrap();
    assert_eq!(host.collection_token, 0);
    assert_eq!(host.layouts[&key("a", 0)].texts[0].as_deref(), Some("AA"));
    let replacement = DialogCollectionState::new(1, definitions).unwrap();
    let old = manager.set_dialogs(replacement);
    host.select_collection(1);
    assert_eq!(
        manager
            .start(
                custom_hash("a"),
                InteractionLocation::Screen([0.; 2]),
                None,
                false,
                &mut host
            )
            .unwrap(),
        Some(1)
    );
    assert_eq!(host.boxes[0].state.text.as_deref(), Some("AA"));
    assert_eq!(host.boxes[1].state.text.as_deref(), Some("CC"));
    let new_key = DialogMessageKey {
        collection: 1,
        ..key("a", 0)
    };
    assert_eq!(host.world.played, [key("a", 0), new_key]);
    assert!(host.world.stopped.is_empty() && host.world.ended.is_empty());
    assert_eq!(host.world.cues.len(), 2);
    assert_eq!(host.layouts.len(), 6);
    manager.set_dialogs(old);
    host.select_collection(0);
    assert_eq!(
        manager
            .dialog(custom_hash("a"))
            .unwrap()
            .cursor()
            .interacted,
        1
    );
    assert_eq!(host.layouts[&key("a", 0)].texts[0].as_deref(), Some("AA"));
}
#[test]
fn cutscene_playback_is_owned_and_completion_keeps_one_submitted_frame() {
    let mut definitions = definitions();
    definitions[0].interactions[0].messages.truncate(1);
    definitions[0].interactions[0].messages[0].texts.truncate(1);
    let mut host = host();
    host.world.cutscene = true;
    host.initialize_layouts(&definitions).unwrap();
    let mut manager = DialogManager::new(definitions).unwrap();
    manager
        .start(
            custom_hash("a"),
            InteractionLocation::Screen([0.; 2]),
            None,
            true,
            &mut host,
        )
        .unwrap();
    assert_eq!(host.cutscene.widget.state.size[0], 640);
    assert_eq!(host.cutscene.widget.state.text.as_deref(), Some("AA"));
    assert!(!host.boxes[0].state.scene_initialized);
    host.cutscene.widget.state.typing.publish_primitives(4);
    host.cutscene.widget.state.finish_animation(false, true);
    manager
        .update(
            DialogFrame {
                channel: 0,
                dt: 0.11,
            },
            &mut [[0.; 4]; 4],
            &mut host,
        )
        .unwrap();
    assert!(!manager.draw_cutscene_text && !manager.dialog_active(custom_hash("a")));
    assert!(host.cutscene_submitted[0]);
    assert!(host.cutscene.widget.state.visible()); // submitted snapshot still has scale 1
    manager
        .update(
            DialogFrame { channel: 0, dt: 0. },
            &mut [[0.; 4]; 4],
            &mut host,
        )
        .unwrap();
    assert!(!host.cutscene_submitted[0]);
    assert_eq!(host.cutscene.widget.state.scale, 1.); // no further cutscene update after flag clears
}
