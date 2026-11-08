use magicka_runtime::text_box::TextBoxState;
use magicka_runtime::{
    dialog_action::*, dialog_definition::*, dialog_frame::*, dialog_manager::*,
    interaction_playback::*, message_playback::*, scene_loading::custom_hash,
};
use magicka_sim::damage_delivery::NetworkRole;
struct Host {
    sink: DialogSink,
    visible: [bool; 8],
    animating: [bool; 8],
    cutscene_visible: bool,
    locks: Vec<i32>,
    unlocks: Vec<i32>,
    effects: Vec<DialogManagerEffect>,
    fail_unfreeze: bool,
    character: Option<DialogActionCharacter>,
    senders: Vec<u16>,
    character_ids: Vec<i32>,
    locators: Vec<i32>,
    boxes: Option<[TextBoxState; 8]>,
    frame_log: Vec<String>,
    dead_owners: [bool; 8],
    additional: Vec<bool>,
    message_dead: Vec<bool>,
    in_cutscene: bool,
    fail_update: Option<DialogFrameTarget>,
    fail_end: bool,
    clear_cutscene_on_message: bool,
}
impl Default for Host {
    fn default() -> Self {
        Self {
            sink: DialogSink::Slot(0),
            visible: [false; 8],
            animating: [false; 8],
            cutscene_visible: false,
            locks: vec![],
            unlocks: vec![],
            effects: vec![],
            fail_unfreeze: false,
            character: None,
            senders: vec![],
            character_ids: vec![],
            locators: vec![],
            boxes: None,
            frame_log: vec![],
            dead_owners: [false; 8],
            additional: vec![],
            message_dead: vec![],
            in_cutscene: false,
            fail_update: None,
            fail_end: false,
            clear_cutscene_on_message: false,
        }
    }
}
impl MessageHost for Host {
    type Error = &'static str;
    fn resolve_owner(&mut self, _: i32) -> Result<Option<MessageOwner>, Self::Error> {
        Ok(self.character.map(|c| MessageOwner {
            id: c.owner,
            dead: false,
            character: true,
            dead_state: false,
            animation_allowed: true,
            special_idle: 0,
        }))
    }
    fn cue_stopping(&self) -> Option<bool> {
        None
    }
    fn effect(
        &mut self,
        definition: &MessageDefinition,
        effect: MessageEffect,
    ) -> Result<(), Self::Error> {
        if let (Some(boxes), DialogSink::Slot(i)) = (&mut self.boxes, self.sink) {
            match effect {
                MessageEffect::Show { text, .. } => {
                    boxes[i].initialize_state(Some("AB"), [80, 20], definition.texts[text].ttl)
                }
                MessageEffect::Hide => boxes[i].hide(),
                _ => {}
            }
        }
        let visible = match self.sink {
            DialogSink::Slot(i) => &mut self.visible[i],
            DialogSink::Cutscene => &mut self.cutscene_visible,
        };
        match effect {
            MessageEffect::Show { .. } => *visible = true,
            MessageEffect::Hide => *visible = false,
            _ => {}
        }
        Ok(())
    }
}
impl InteractionHost for Host {
    type Error = &'static str;
    fn advance_message(
        &mut self,
        _: usize,
        d: &MessageDefinition,
        p: &mut MessagePlayback,
        placement: MessagePlacement,
    ) -> Result<bool, Self::Error> {
        p.advance(d, placement, self).map_err(|_| "message error")
    }
    fn lock_input(&mut self, id: i32) -> Result<(), Self::Error> {
        self.locks.push(id);
        Ok(())
    }
    fn unlock_input(&mut self, id: i32) -> Result<(), Self::Error> {
        self.unlocks.push(id);
        Ok(())
    }
    fn end_message(&mut self, _: usize, _: &MessageDefinition) -> Result<(), Self::Error> {
        self.frame_log.push("end".into());
        if self.fail_end {
            Err("end failed")
        } else {
            Ok(())
        }
    }
}
impl DialogFrameHost for Host {
    fn update_textbox(
        &mut self,
        manager: &mut DialogManager,
        target: DialogFrameTarget,
        frame: DialogFrame,
        _: &mut [[f32; 4]; 4],
    ) -> Result<(), Self::Error> {
        self.frame_log.push(format!("update {target:?}"));
        if self.fail_update == Some(target) {
            return Err("update failed");
        }
        if let (Some(boxes), DialogFrameTarget::Slot(i)) = (&mut self.boxes, target) {
            let update = boxes[i]
                .begin_update(frame.dt, frame.channel, None)
                .map_err(|_| "box error")?;
            if let Some(update) = update {
                if update.advance {
                    manager
                        .advance_textbox(DialogSink::Slot(i), self.in_cutscene, self)
                        .map_err(|_| "advance error")?;
                }
                self.boxes.as_mut().unwrap()[i].finish_update(update);
            }
        }
        Ok(())
    }
    fn slot_owner_dead(&mut self, slot: usize) -> Result<bool, Self::Error> {
        self.frame_log.push(format!("owner {slot}"));
        Ok(self.dead_owners[slot])
    }
    fn additional_count(&self) -> usize {
        self.additional.len()
    }
    fn additional_visible(&self, index: usize) -> bool {
        self.additional[index]
    }
    fn remove_additional(&mut self, index: usize) -> Result<(), Self::Error> {
        self.frame_log.push(format!("remove additional {index}"));
        self.additional.remove(index);
        Ok(())
    }
    fn message_box_count(&self) -> usize {
        self.message_dead.len()
    }
    fn update_message_box(
        &mut self,
        manager: &mut DialogManager,
        index: usize,
        _: DialogFrame,
    ) -> Result<(), Self::Error> {
        self.frame_log.push(format!("message {index}"));
        if self.clear_cutscene_on_message {
            manager.draw_cutscene_text = false;
        }
        Ok(())
    }
    fn message_box_dead(&self, index: usize) -> bool {
        self.message_dead[index]
    }
    fn remove_message_box(&mut self, index: usize) -> Result<(), Self::Error> {
        self.frame_log.push(format!("remove message {index}"));
        self.message_dead.remove(index);
        Ok(())
    }
    fn play_state_in_cutscene(&self) -> bool {
        self.in_cutscene
    }
    fn submit_subtitles(&mut self, channel: usize, height: f32) -> Result<(), Self::Error> {
        self.frame_log.push(format!("subtitles {channel} {height}"));
        Ok(())
    }
}
impl DialogManagerHost for Host {
    fn bind(&mut self, sink: DialogSink, _: ActiveInteraction) {
        self.sink = sink;
    }
    fn visible(&self, sink: DialogSink) -> bool {
        if let (Some(boxes), DialogSink::Slot(i)) = (&self.boxes, sink) {
            return boxes[i].visible();
        }
        match sink {
            DialogSink::Slot(i) => self.visible[i],
            DialogSink::Cutscene => self.cutscene_visible,
        }
    }
    fn animating(&self, sink: DialogSink) -> bool {
        if let (Some(boxes), DialogSink::Slot(i)) = (&self.boxes, sink) {
            return boxes[i].animating();
        }
        match sink {
            DialogSink::Slot(i) => self.animating[i],
            DialogSink::Cutscene => false,
        }
    }
    fn manager_effect(&mut self, effect: DialogManagerEffect) -> Result<(), Self::Error> {
        self.effects.push(effect);
        if let Some(boxes) = &mut self.boxes {
            match effect {
                DialogManagerEffect::FinishAnimation {
                    sink: DialogSink::Slot(i),
                    remove_ttl_advance,
                } => boxes[i].finish_animation(remove_ttl_advance, true),
                DialogManagerEffect::Hide(DialogSink::Slot(i)) => boxes[i].hide(),
                _ => {}
            }
        }
        match effect {
            DialogManagerEffect::FinishAnimation {
                sink: DialogSink::Slot(i),
                ..
            } => self.animating[i] = false,
            DialogManagerEffect::Hide(DialogSink::Slot(i)) => self.visible[i] = false,
            DialogManagerEffect::Hide(DialogSink::Cutscene) => self.cutscene_visible = false,
            DialogManagerEffect::Unfreeze if self.fail_unfreeze => return Err("unfreeze failed"),
            _ => {}
        }
        Ok(())
    }
}
impl DialogActionHost for Host {
    fn resolve_character(&mut self, id: i32) -> Result<Option<DialogActionCharacter>, Self::Error> {
        self.character_ids.push(id);
        Ok(self.character)
    }
    fn resolve_sender(&mut self, id: u16) -> Result<Option<DialogActionCharacter>, Self::Error> {
        self.senders.push(id);
        Ok(self.character)
    }
    fn locator(&mut self, id: i32) -> Result<[f32; 3], Self::Error> {
        self.locators.push(id);
        Ok([4.0, 5.0, 6.0])
    }
    fn cutscene(&self) -> bool {
        false
    }
}
impl DialogAvatarHost for Host {
    fn avatar_controller(&mut self, id: i32) -> Result<Option<i32>, Self::Error> {
        self.character_ids.push(id);
        Ok(self.character.and_then(|c| c.controller))
    }
}
fn manager(names: &[&str], lock: bool) -> DialogManager {
    let mut xml = String::from("<Dialogs>");
    for name in names {
        xml.push_str(&format!(r#"<dialog id="{name}" repeatFrom="-1"><interact lock="{lock}"><message><text>#line</text></message></interact></dialog>"#));
    }
    xml.push_str("</Dialogs>");
    DialogManager::new(DialogDefinition::read_collection(&[xml.as_bytes()]).unwrap()).unwrap()
}
fn screen() -> InteractionLocation {
    InteractionLocation::Screen([0.0; 2])
}
fn frame(dt: f32) -> DialogFrame {
    DialogFrame { channel: 0, dt }
}
#[test]
fn targeted_end_matches_dialog_or_nonnull_controller_without_global_completion() {
    let mut manager = manager(&["a", "b", "c", "d"], true);
    let mut host = Host::default();
    for (id, controller) in [("a", Some(7)), ("b", Some(8)), ("c", None), ("d", Some(7))] {
        manager
            .start(custom_hash(id), screen(), controller, false, &mut host)
            .unwrap();
    }
    manager.holdoff = 0.0625;
    manager.awaiting_input = true;
    manager.draw_cutscene_text = true;
    host.effects.clear();
    manager.end_for_controller(None, &mut host).unwrap();
    assert!(manager.dialog_active(custom_hash("c")));
    manager.end_for_controller(Some(7), &mut host).unwrap();
    assert!(!manager.dialog_active(custom_hash("a")));
    assert!(manager.dialog_active(custom_hash("b")));
    assert_eq!(
        host.effects,
        [
            DialogManagerEffect::Hide(DialogSink::Slot(0)),
            DialogManagerEffect::Hide(DialogSink::Slot(3))
        ]
    );
    manager.end_dialog(custom_hash("b"), &mut host).unwrap();
    assert!(!manager.dialog_active(custom_hash("b")));
    assert_eq!(host.unlocks, [7, 7, 8]);
    assert_eq!(
        host.effects,
        [
            DialogManagerEffect::Hide(DialogSink::Slot(0)),
            DialogManagerEffect::Hide(DialogSink::Slot(3)),
            DialogManagerEffect::Hide(DialogSink::Slot(1))
        ]
    );
    assert_eq!(manager.holdoff, 0.0625);
    assert!(manager.awaiting_input && manager.draw_cutscene_text);
    assert!(!manager.is_done(custom_hash("a"), 0).unwrap());
    assert!(!manager.is_done(custom_hash("b"), 0).unwrap());
    assert_eq!(
        manager
            .dialog(custom_hash("a"))
            .unwrap()
            .interaction(0)
            .unwrap()
            .controller,
        Some(7)
    );
}
#[test]
fn targeted_end_failure_keeps_active_slot_and_does_not_hide() {
    let mut manager = manager(&["a"], true);
    let mut host = Host {
        fail_end: true,
        ..Host::default()
    };
    manager
        .start(custom_hash("a"), screen(), Some(7), false, &mut host)
        .unwrap();
    host.effects.clear();
    assert!(matches!(
        manager.end_dialog(custom_hash("a"), &mut host),
        Err(DialogManagerError::Playback(_))
    ));
    assert!(manager.dialog_active(custom_hash("a")));
    assert!(host.effects.is_empty() && host.unlocks.is_empty());
}
#[test]
fn avatar_end_reads_controller_per_active_slot_and_short_circuits_null() {
    let mut manager = manager(&["a", "b"], false);
    let mut host = Host::default();
    manager
        .start(custom_hash("a"), screen(), Some(7), false, &mut host)
        .unwrap();
    manager
        .start(custom_hash("b"), screen(), None, false, &mut host)
        .unwrap();
    host.character = Some(DialogActionCharacter {
        owner: 42,
        avatar: true,
        controller: None,
    });
    manager.end_avatar(42, &mut host).unwrap();
    assert_eq!(host.character_ids, [42, 42]);
    assert!(manager.dialog_active(custom_hash("a")) && manager.dialog_active(custom_hash("b")));
    host.character_ids.clear();
    host.character.as_mut().unwrap().controller = Some(7);
    manager.end_avatar(42, &mut host).unwrap();
    assert_eq!(host.character_ids, [42, 42, 42, 42]);
    assert!(!manager.dialog_active(custom_hash("a")) && manager.dialog_active(custom_hash("b")));
    assert_eq!(host.unlocks, [7]);
}
#[test]
fn collection_replacement_clears_slots_without_cleanup_and_restores_progress() {
    let mut manager = manager(&["a", "done"], false);
    let mut host = Host::default();
    manager
        .start(custom_hash("done"), screen(), Some(1), false, &mut host)
        .unwrap();
    manager.network_advance(0, &mut host).unwrap();
    assert!(manager.is_done(custom_hash("done"), 0).unwrap());
    host.visible[0] = false;
    manager
        .start(custom_hash("a"), screen(), Some(7), false, &mut host)
        .unwrap();
    host.effects.clear();
    host.unlocks.clear();
    manager.holdoff = 0.0625;
    manager.awaiting_input = true;
    manager.draw_cutscene_text = true;
    let replacement = DialogCollectionState::new(9,DialogDefinition::read_collection(&[b"<Dialogs><Dialog id=\"a\"><Interact><Message><Text>#new</Text></Message></Interact></Dialog></Dialogs>"]).unwrap()).unwrap();
    let old = manager.set_dialogs(replacement);
    assert_eq!(manager.collection_token(), 9);
    assert!(manager.active().iter().all(Option::is_none));
    assert!(host.effects.is_empty() && host.unlocks.is_empty());
    assert!(host.visible[0]);
    assert_eq!(manager.holdoff, 0.0625);
    assert!(manager.awaiting_input && manager.draw_cutscene_text);
    assert!(!manager.is_done(custom_hash("a"), 0).unwrap());
    let replacement = manager.set_dialogs(old);
    assert_eq!(replacement.token, 9);
    assert!(manager.is_done(custom_hash("done"), 0).unwrap());
    let a = manager.dialog(custom_hash("a")).unwrap();
    assert_eq!(a.cursor().interacted, 1);
    assert_eq!(a.interaction(0).unwrap().controller, Some(7));
    assert!(manager.active().iter().all(Option::is_none));
}
#[test]
fn frame_order_prunes_shifted_lists_then_cutscene_and_subtitles() {
    let mut manager = manager(&[], false);
    manager.draw_cutscene_text = true;
    manager.draw_subtitles = true;
    manager.subtitle_height = 24.;
    manager.holdoff = 0.2;
    let mut host = Host {
        additional: vec![false, false, true],
        message_dead: vec![true, true, false],
        in_cutscene: true,
        ..Host::default()
    };
    manager
        .update(frame(0.1), &mut [[0.; 4]; 4], &mut host)
        .unwrap();
    assert_eq!(manager.holdoff, 0.1);
    assert_eq!(host.additional, [true]);
    assert_eq!(host.message_dead, [false]);
    let mut expected: Vec<String> = (0..8).map(|i| format!("update Slot({i})")).collect();
    expected.extend(
        [
            "update Additional(0)",
            "remove additional 0",
            "update Additional(0)",
            "remove additional 0",
            "update Additional(0)",
            "message 0",
            "remove message 0",
            "message 0",
            "remove message 0",
            "message 0",
            "update Cutscene",
            "subtitles 0 24",
        ]
        .map(str::to_owned),
    );
    assert_eq!(host.frame_log, expected);
}
#[test]
fn dead_owner_ends_without_completion_or_global_unlock_and_failure_keeps_slot() {
    for fail in [false, true] {
        let mut manager = manager(&["dead"], true);
        let mut host = Host {
            fail_end: fail,
            ..Host::default()
        };
        manager
            .start(custom_hash("dead"), screen(), Some(7), false, &mut host)
            .unwrap();
        host.dead_owners[0] = true;
        host.effects.clear();
        let result = manager.update(frame(0.1), &mut [[0.; 4]; 4], &mut host);
        assert_eq!(manager.holdoff, 0.1);
        assert!(!manager.is_done(custom_hash("dead"), 0).unwrap());
        assert_eq!(manager.dialog_active(custom_hash("dead")), fail);
        if fail {
            assert!(matches!(result, Err(DialogManagerError::Playback(_))));
            assert!(host.effects.is_empty());
            assert!(host.unlocks.is_empty());
        } else {
            result.unwrap();
            assert_eq!(host.unlocks, [7]);
            assert_eq!(
                host.effects,
                [DialogManagerEffect::Hide(DialogSink::Slot(0))]
            );
        }
        assert_eq!(&host.frame_log[..3], ["update Slot(0)", "owner 0", "end"]);
    }
}
#[test]
fn frame_failure_stops_later_phases_and_cutscene_gate_reads_callback_changes() {
    let mut manager = manager(&[], false);
    manager.draw_cutscene_text = true;
    manager.draw_subtitles = true;
    let mut host = Host {
        fail_update: Some(DialogFrameTarget::Slot(2)),
        message_dead: vec![false],
        in_cutscene: true,
        ..Host::default()
    };
    assert_eq!(
        manager.update(frame(0.25), &mut [[0.; 4]; 4], &mut host),
        Err(DialogManagerError::Host("update failed"))
    );
    assert_eq!(manager.holdoff, -0.25);
    assert_eq!(
        host.frame_log,
        ["update Slot(0)", "update Slot(1)", "update Slot(2)"]
    );
    host.fail_update = None;
    host.clear_cutscene_on_message = true;
    host.frame_log.clear();
    manager
        .update(frame(0.), &mut [[0.; 4]; 4], &mut host)
        .unwrap();
    assert!(!host.frame_log.iter().any(|s| s == "update Cutscene"));
    assert_eq!(&host.frame_log[8..], ["message 0", "subtitles 0 0"]);
}
#[test]
fn actual_textbox_ttl_reenters_manager_and_input_cancels_next_ttl() {
    let xml = b"<Dialogs><Dialog id=\"ttl\" repeatFrom=\"-1\"><Interact><Message><Text ttl=\"0.1\">#one</Text><Text ttl=\"0.2\">#two</Text></Message></Interact></Dialog></Dialogs>";
    let mut manager =
        DialogManager::new(DialogDefinition::read_collection(&[xml]).unwrap()).unwrap();
    let mut host = Host {
        boxes: Some(std::array::from_fn(|_| TextBoxState::default())),
        ..Host::default()
    };
    manager
        .start(custom_hash("ttl"), screen(), Some(7), false, &mut host)
        .unwrap();
    let b = &mut host.boxes.as_mut().unwrap()[0];
    b.typing.publish_primitives(4);
    let update = b.begin_update(0.25, 0, None).unwrap().unwrap();
    b.finish_update(update);
    b.pre_render(
        0,
        0.1,
        [800., 600.],
        [
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ],
    )
    .unwrap();
    manager
        .update(frame(0.11), &mut [[0.; 4]; 4], &mut host)
        .unwrap();
    let b = &mut host.boxes.as_mut().unwrap()[0];
    assert_eq!(b.ttl, 0.2);
    assert_eq!(b.typing.visible_characters, 0);
    manager.tick_input(0.2);
    manager
        .advance_controller(Some(7), false, NetworkRole::Offline, &mut host)
        .unwrap();
    let b = &host.boxes.as_ref().unwrap()[0];
    assert!(!b.automatic_advance);
    assert!(b.typing.finished());
    assert!(manager.dialog_active(custom_hash("ttl")));
    manager
        .advance_controller(Some(7), false, NetworkRole::Offline, &mut host)
        .unwrap();
    assert!(manager.is_done(custom_hash("ttl"), 0).unwrap());
    assert!(!host.boxes.as_ref().unwrap()[0].grow);
}
#[test]
fn start_selects_nonvisible_slot_and_input_finishes_animation_before_interaction() {
    let mut manager = manager(&["intro"], true);
    let mut host = Host::default();
    host.visible[0] = true;
    assert_eq!(
        manager
            .start(custom_hash("intro"), screen(), Some(7), false, &mut host)
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        host.effects,
        [DialogManagerEffect::Freeze, DialogManagerEffect::LimitInput]
    );
    assert!(manager.can_advance(Some(7)));
    assert!(!manager.can_advance(Some(8)));
    assert_eq!(
        manager
            .start(custom_hash("intro"), screen(), Some(8), false, &mut host)
            .unwrap(),
        None
    );
    manager
        .advance_controller(Some(7), false, NetworkRole::Server, &mut host)
        .unwrap();
    assert!(manager.dialog_active(custom_hash("intro")));
    manager.tick_input(0.2);
    host.animating[1] = true;
    manager
        .advance_controller(Some(7), false, NetworkRole::Server, &mut host)
        .unwrap();
    assert_eq!(
        &host.effects[2..],
        [
            DialogManagerEffect::NetworkAdvance { current: 0 },
            DialogManagerEffect::FinishAnimation {
                sink: DialogSink::Slot(1),
                remove_ttl_advance: true
            }
        ]
    );
    assert!(!manager.is_done(custom_hash("intro"), 0).unwrap());
    manager
        .advance_controller(Some(7), false, NetworkRole::Offline, &mut host)
        .unwrap();
    assert!(manager.is_done(custom_hash("intro"), 0).unwrap());
    assert!(!manager.dialog_active(custom_hash("intro")));
    assert!(!manager.can_advance(Some(7)));
    assert_eq!(host.unlocks, [7]);
    assert_eq!(manager.holdoff, 0.2);
    assert_eq!(
        &host.effects[4..],
        [
            DialogManagerEffect::Unfreeze,
            DialogManagerEffect::UnlimitInput
        ]
    );
}
#[test]
fn all_eight_occupied_slots_overwrite_zero_without_ending_displaced_interaction() {
    let names = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
    let mut manager = manager(&names, false);
    let mut host = Host::default();
    for (i, name) in names.iter().enumerate() {
        assert_eq!(
            manager
                .start(
                    custom_hash(name),
                    screen(),
                    Some(i as i32),
                    false,
                    &mut host
                )
                .unwrap(),
            Some(if i == 8 { 0 } else { i })
        );
    }
    assert_eq!(manager.active()[0].unwrap().dialog, custom_hash("i"));
    assert_eq!(
        manager
            .dialog(custom_hash("a"))
            .unwrap()
            .interaction(0)
            .unwrap()
            .controller,
        Some(0)
    );
    assert!(host.unlocks.is_empty());
    assert_eq!(host.locks.len(), 9);
}
#[test]
fn network_current_index_matches_multiple_dialogs_and_ignores_cooldown() {
    let mut manager = manager(&["a", "b"], false);
    let mut host = Host::default();
    for name in ["a", "b"] {
        manager
            .start(custom_hash(name), screen(), Some(7), false, &mut host)
            .unwrap();
    }
    assert!(manager.holdoff > 0.0);
    manager.network_advance(0, &mut host).unwrap();
    assert!(manager.active().iter().all(Option::is_none));
    assert!(
        manager.is_done(custom_hash("a"), 0).unwrap()
            && manager.is_done(custom_hash("b"), 0).unwrap()
    );
    assert_eq!(host.unlocks, [7, 7]);
}
#[test]
fn textbox_cutscene_completion_keeps_cooldown_and_reset_keeps_interacted_count() {
    let mut manager = manager(&["a"], true);
    let mut host = Host::default();
    manager
        .start(custom_hash("a"), screen(), Some(7), true, &mut host)
        .unwrap();
    assert!(manager.draw_cutscene_text);
    manager.holdoff = 0.0625;
    manager
        .advance_controller(Some(7), true, NetworkRole::Offline, &mut host)
        .unwrap();
    assert!(manager.dialog_active(custom_hash("a")));
    manager
        .advance_textbox(DialogSink::Cutscene, true, &mut host)
        .unwrap();
    assert_eq!(manager.holdoff, 0.0625);
    assert!(!manager.draw_cutscene_text);
    assert!(manager.is_done(custom_hash("a"), 0).unwrap());
    manager.reset(&mut host).unwrap();
    assert!(!manager.is_done(custom_hash("a"), 0).unwrap());
    assert_eq!(
        manager
            .dialog(custom_hash("a"))
            .unwrap()
            .cursor()
            .interacted,
        1
    );
    assert_eq!(
        manager
            .dialog(custom_hash("a"))
            .unwrap()
            .interaction(0)
            .unwrap()
            .completed,
        1
    );
    assert_eq!(manager.holdoff, 0.0625);
}
#[test]
fn missing_dialog_policies_and_failed_unfreeze_preserve_completed_active_slot() {
    let mut manager = manager(&["a"], true);
    let mut host = Host::default();
    assert_eq!(
        manager
            .start(
                42,
                InteractionLocation::Entity(None),
                None,
                false,
                &mut host
            )
            .unwrap(),
        None
    );
    assert_eq!(
        manager.start(42, screen(), None, false, &mut host),
        Err(DialogManagerError::MissingDialog(42))
    );
    manager
        .start(custom_hash("a"), screen(), Some(7), false, &mut host)
        .unwrap();
    host.fail_unfreeze = true;
    assert_eq!(
        manager.network_advance(0, &mut host),
        Err(DialogManagerError::Host("unfreeze failed"))
    );
    assert!(manager.dialog_active(custom_hash("a")));
    assert!(manager.is_done(custom_hash("a"), 0).unwrap());
    assert_eq!(
        manager
            .dialog(custom_hash("a"))
            .unwrap()
            .interaction(0)
            .unwrap()
            .controller,
        None
    );
    assert_eq!(manager.holdoff, 0.2);
}

#[test]
fn manager_completion_facts_drive_loaded_script_without_manual_flag_copies() {
    use magicka_runtime::{
        animation_action::AnimationAction, dialog_state::DialogFacts,
        level_variables::LevelVariables, scene_script::SceneScriptLayout, script_runtime::*,
    };
    struct World {
        dialogs: DialogManager,
    }
    impl ScriptWorld for World {
        type Error = &'static str;
        fn facts(&self) -> ScriptWorldFacts {
            ScriptWorldFacts::default()
        }
        fn dialogs(&self) -> Option<DialogFacts<'_>> {
            Some(self.dialogs.facts(0))
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
        fn animation(
            &mut self,
            _: &mut ScriptRuntime,
            _: &AnimationAction,
        ) -> Result<(), Self::Error> {
            Err("unexpected animation")
        }
        fn action_executed(&mut self, _: &mut ScriptRuntime, _: usize) -> Result<(), Self::Error> {
            Ok(())
        }
    }
    let mut world = World {
        dialogs: manager(&["intro"], false),
    };
    let mut host = Host::default();
    let layout = SceneScriptLayout::read(br#"<Scene><trigger><if><dialogDone dialog="intro" interactIndex="0"/></if><then><counter name="completed" value="1"/></then></trigger></Scene>"#, false).unwrap();
    let mut script = ScriptRuntime::new(layout, LevelVariables::default());
    world
        .dialogs
        .start(custom_hash("intro"), screen(), Some(7), false, &mut host)
        .unwrap();
    script
        .update(0.0, NetworkRole::Offline, &mut world)
        .unwrap();
    assert_eq!(script.variables.counter(custom_hash("completed")), 0);
    world.dialogs.network_advance(0, &mut host).unwrap();
    script
        .update(0.0, NetworkRole::Offline, &mut world)
        .unwrap();
    assert_eq!(script.variables.counter(custom_hash("completed")), 1);
}

#[test]
fn dialog_actions_resolve_numeric_character_and_locator_positions() {
    for (position, expected, screen_position) in [
        ("1,2", [1.0, 2.0, 0.0], true),
        ("1,2,3", [1.0, 3.0, 0.0], false),
        ("locator", [4.0, 5.0, 6.0], false),
    ] {
        let mut manager = manager(&["intro"], false);
        let mut host = Host::default();
        let action = DialogAction::read(
            "startDialog",
            [
                ("dialog", "intro"),
                ("position", position),
                ("forceOnScreen", "true"),
            ],
        )
        .unwrap();
        action.execute(Some(11), &mut manager, &mut host).unwrap();
        let state = manager
            .dialog(custom_hash("intro"))
            .unwrap()
            .interaction(0)
            .unwrap();
        assert_eq!(state.world_position, expected);
        assert_eq!(state.screen_position, screen_position);
        assert!(host.senders.is_empty());
        if position == "locator" {
            assert_eq!(host.character_ids, [custom_hash("locator")]);
            assert_eq!(host.locators, [custom_hash("locator")]);
        }
    }
    for avatar in [false, true] {
        let mut manager = manager(&["intro"], false);
        let mut host = Host {
            character: Some(DialogActionCharacter {
                owner: 42,
                avatar,
                controller: Some(77),
            }),
            ..Default::default()
        };
        let action = DialogAction::read("startDialog", [("dialog", "intro")]).unwrap();
        action.execute(Some(11), &mut manager, &mut host).unwrap();
        assert_eq!(host.senders, [11]);
        assert!(host.locators.is_empty());
        assert_eq!(
            manager
                .dialog(custom_hash("intro"))
                .unwrap()
                .interaction(0)
                .unwrap()
                .last_owner,
            Some(42)
        );
        assert_eq!(host.locks, if avatar { vec![77] } else { vec![] });
    }
}

#[test]
fn loaded_dialog_actions_dispatch_through_shared_queues_and_keep_cancelled_sender() {
    use magicka_runtime::{
        animation_action::AnimationAction, dialog_state::DialogFacts,
        level_variables::LevelVariables, scene_script::SceneScriptLayout, script_runtime::*,
    };
    struct World {
        dialogs: DialogManager,
        host: Host,
    }
    impl ScriptWorld for World {
        type Error = DialogActionError<&'static str>;
        fn facts(&self) -> ScriptWorldFacts {
            ScriptWorldFacts::default()
        }
        fn dialogs(&self) -> Option<DialogFacts<'_>> {
            Some(self.dialogs.facts(0))
        }
        fn dialog_action(
            &mut self,
            _: &mut ScriptRuntime,
            action: &DialogAction,
            sender: Option<u16>,
        ) -> Option<Result<(), Self::Error>> {
            Some(
                action
                    .execute(sender, &mut self.dialogs, &mut self.host)
                    .map(|_| ()),
            )
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
        fn animation(
            &mut self,
            _: &mut ScriptRuntime,
            _: &AnimationAction,
        ) -> Result<(), Self::Error> {
            Err(DialogActionError::Host("unexpected animation"))
        }
        fn action_executed(&mut self, _: &mut ScriptRuntime, _: usize) -> Result<(), Self::Error> {
            Ok(())
        }
    }
    let layout = SceneScriptLayout::read(br#"<Scene><trigger id="owner" autorun="false"><then><startDialog dialog="intro" delay="0.25"/><killDialogs/></then></trigger></Scene>"#, false).unwrap();
    let mut script = ScriptRuntime::new(layout, LevelVariables::default());
    let mut world = World {
        dialogs: manager(&["intro"], false),
        host: Host::default(),
    };
    script
        .trigger_action_with_sender::<DialogActionError<&str>>(0, Some(11))
        .unwrap();
    script
        .clear_delayed::<DialogActionError<&str>>(custom_hash("owner"))
        .unwrap();
    script
        .trigger_action_with_sender::<DialogActionError<&str>>(0, Some(22))
        .unwrap();
    script
        .update_action(0, 0.125, NetworkRole::Offline, &mut world)
        .unwrap();
    assert!(!world.dialogs.dialog_active(custom_hash("intro")));
    script
        .update_action(0, 0.125, NetworkRole::Offline, &mut world)
        .unwrap();
    assert_eq!(world.host.senders, [11]);
    assert!(world.dialogs.dialog_active(custom_hash("intro")));
    assert_eq!(script.history(), [0]);
    script.trigger_action::<DialogActionError<&str>>(1).unwrap();
    script
        .update_action(1, 0.0, NetworkRole::Offline, &mut world)
        .unwrap();
    assert!(!world.dialogs.dialog_active(custom_hash("intro")));
    assert_eq!(script.history(), [0, 1]);
}

#[test]
fn start_quick_execute_is_empty_and_kill_quick_execute_ends_active_dialog() {
    let mut manager = manager(&["intro"], false);
    let mut host = Host::default();
    let start =
        DialogAction::read("startDialog", [("dialog", "intro"), ("position", "1,2")]).unwrap();
    start.quick_execute(&mut manager, &mut host).unwrap();
    assert!(manager.active().iter().all(Option::is_none));
    start.execute(None, &mut manager, &mut host).unwrap();
    DialogAction::read("killDialogs", [])
        .unwrap()
        .quick_execute(&mut manager, &mut host)
        .unwrap();
    assert!(manager.active().iter().all(Option::is_none));
    assert!(!manager.is_done(custom_hash("intro"), 0).unwrap());
}
