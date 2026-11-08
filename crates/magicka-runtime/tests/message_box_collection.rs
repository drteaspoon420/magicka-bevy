use magicka_content::assets::{BitmapFont, Glyph};
use magicka_runtime::{
    bitmap_font::{FontError, FontMetrics},
    dialog_input::DialogInputKind as Input,
    dialog_manager::DialogManager,
    error_message_box::{ErrorMessageBoxShowError, ErrorMessageBoxState},
    message_box_collection::{
        ErrorMessageBoxes, MessageBoxCollectionError, MessageBoxDispatchError,
    },
};
fn font(empty: bool) -> FontMetrics {
    FontMetrics::from_content(&BitmapFont {
        line_height: 10,
        base_line: 8,
        default_character: '?',
        texture: None,
        glyphs: if empty {
            vec![]
        } else {
            vec![Glyph {
                character: '?',
                advance_width: 8,
                origin: [0; 2],
                size: [1; 2],
                left_side_bearing: 0,
                force_white: false,
            }]
        },
        kerning: vec![],
    })
    .unwrap()
}
#[test]
fn duplicate_show_keeps_one_object_and_draws_it_for_each_reference() {
    let mut boxes = ErrorMessageBoxes::default();
    let h = boxes.insert(ErrorMessageBoxState::new([800., 600.], String::new()));
    let mut manager = DialogManager::new(vec![]).unwrap();
    for _ in 0..2 {
        boxes
            .show(h, "A", &font(false), &mut manager, false)
            .unwrap();
    }
    assert_eq!(boxes.active(), &[h, h]);
    boxes.begin_frame(0).unwrap();
    for index in 0..2 {
        boxes.update_index(index, 0, [800., 600.], false).unwrap();
    }
    let draws = boxes.draw_submitted(0, 0.1, [800., 600.], false).unwrap();
    assert_eq!(draws.len(), 2);
    assert_eq!(draws[0].1.background.unwrap().color[3], 0.4);
    assert_eq!(draws[1].1.background.unwrap().color[3], 0.8);
    assert!((boxes.get(h).unwrap().timer - 4.8).abs() < 1e-6);
}
#[test]
fn active_removal_retains_submitted_object_and_channels_are_independent() {
    let mut boxes = ErrorMessageBoxes::default();
    let h = boxes.insert(ErrorMessageBoxState::new([800., 600.], String::new()));
    let mut manager = DialogManager::new(vec![]).unwrap();
    boxes
        .show(h, "A", &font(false), &mut manager, false)
        .unwrap();
    boxes.get_mut(h).unwrap().base.kill(); // killed at alpha zero
    boxes.begin_frame(1).unwrap();
    boxes.update_index(0, 1, [800., 600.], false).unwrap();
    assert!(boxes.dead_index(0).unwrap());
    boxes.remove_index(0).unwrap();
    assert!(boxes.active().is_empty());
    assert_eq!(boxes.submissions(1).unwrap(), &[h]);
    assert_eq!(
        boxes
            .draw_submitted(1, 0.1, [800., 600.], false)
            .unwrap()
            .len(),
        1
    );
    boxes.begin_frame(0).unwrap();
    assert_eq!(boxes.submissions(1).unwrap(), &[h]);
    boxes.begin_frame(1).unwrap();
    assert!(boxes.submissions(1).unwrap().is_empty());
    assert!(boxes.get(h).is_some());
}
#[test]
fn failed_wrapping_keeps_registered_reference_and_show_mutations() {
    let mut boxes = ErrorMessageBoxes::default();
    let h = boxes.insert(ErrorMessageBoxState::new([800., 600.], "old".into()));
    let mut manager = DialogManager::new(vec![]).unwrap();
    assert_eq!(
        boxes.show(h, "A", &font(true), &mut manager, false),
        Err(ErrorMessageBoxShowError::Font(FontError::EmptyFont))
    );
    assert_eq!(boxes.active(), &[h]);
    assert_eq!(manager.holdoff, 0.2);
    assert_eq!(boxes.get(h).unwrap().message, "old");
    assert!(!boxes.get(h).unwrap().base.killed);
}
#[test]
fn equal_z_gui_order_matches_native_binary_search_insertion() {
    let mut boxes = ErrorMessageBoxes::default();
    let mut manager = DialogManager::new(vec![]).unwrap();
    let handles: Vec<_> = (0..5)
        .map(|_| {
            let h = boxes.insert(ErrorMessageBoxState::new([800., 600.], String::new()));
            boxes
                .show(h, "A", &font(false), &mut manager, false)
                .unwrap();
            h
        })
        .collect();
    for i in 0..5 {
        boxes.update_index(i, 0, [800., 600.], false).unwrap();
    }
    assert_eq!(
        boxes.submissions(0).unwrap(),
        &[handles[1], handles[0], handles[3], handles[4], handles[2]]
    );
    // GUI order does not change input's active-list last-reference selection.
    manager.holdoff = 0.;
    boxes
        .dispatch(
            &mut manager,
            Input::Select,
            false,
            false,
            || Ok::<_, ()>(()),
            |_| panic!("unexpected fallback"),
        )
        .unwrap();
    assert_eq!(boxes.get(handles[4]).unwrap().timer, 0.);
    assert_eq!(boxes.get(handles[2]).unwrap().timer, 5.);
}
#[test]
fn dispatch_executes_mouse_escape_and_empty_select_fallback() {
    let mut boxes = ErrorMessageBoxes::default();
    let mut manager = DialogManager::new(vec![]).unwrap();
    let h = boxes.insert(ErrorMessageBoxState::new([800., 600.], String::new()));
    boxes
        .show(h, "A", &font(false), &mut manager, false)
        .unwrap();
    boxes
        .dispatch(
            &mut manager,
            Input::Select,
            false,
            false,
            || Ok::<_, ()>(()),
            |_| panic!("unexpected fallback"),
        )
        .unwrap();
    assert_eq!(boxes.get(h).unwrap().timer, 5.); // holdoff
    boxes
        .dispatch(
            &mut manager,
            Input::MouseClick,
            true,
            true,
            || Ok::<_, ()>(()),
            |_| panic!("unexpected fallback"),
        )
        .unwrap();
    assert_eq!(boxes.get(h).unwrap().timer, 0.);
    boxes
        .dispatch(
            &mut manager,
            Input::Escape,
            false,
            false,
            || Ok::<_, ()>(()),
            |_| panic!("unexpected fallback"),
        )
        .unwrap();
    assert!(boxes.get(h).unwrap().base.killed);
    manager.holdoff = 0.;
    let mut hidden = false;
    boxes
        .dispatch(
            &mut manager,
            Input::Move,
            false,
            false,
            || {
                hidden = true;
                Ok::<_, ()>(())
            },
            |_| panic!("unexpected fallback"),
        )
        .unwrap();
    assert!(hidden);
    boxes.remove_index(0).unwrap();
    boxes
        .dispatch(
            &mut manager,
            Input::Select,
            false,
            false,
            || Ok::<_, ()>(()),
            |m| {
                m.awaiting_input = true;
                Ok(())
            },
        )
        .unwrap();
    assert!(manager.awaiting_input);
    assert!(matches!(
        boxes.dispatch(
            &mut manager,
            Input::Escape,
            false,
            false,
            || Ok::<_, ()>(()),
            |_| Ok(())
        ),
        Err(MessageBoxDispatchError::EmptyStack(_))
    ));
    assert_eq!(
        boxes.begin_frame(3),
        Err(MessageBoxCollectionError::InvalidChannel(3))
    );
}

// Real manager frame integration; no active gameplay dialogs in this fixture.
struct FrameHost {
    boxes: ErrorMessageBoxes,
}
impl magicka_runtime::interaction_playback::InteractionHost for FrameHost {
    type Error = MessageBoxCollectionError;
    fn advance_message(
        &mut self,
        _: usize,
        _: &magicka_runtime::dialog_definition::MessageDefinition,
        _: &mut magicka_runtime::message_playback::MessagePlayback,
        _: magicka_runtime::message_playback::MessagePlacement,
    ) -> Result<bool, Self::Error> {
        panic!("unexpected gameplay message")
    }
    fn lock_input(&mut self, _: i32) -> Result<(), Self::Error> {
        panic!("unexpected lock")
    }
    fn unlock_input(&mut self, _: i32) -> Result<(), Self::Error> {
        panic!("unexpected unlock")
    }
    fn end_message(
        &mut self,
        _: usize,
        _: &magicka_runtime::dialog_definition::MessageDefinition,
    ) -> Result<(), Self::Error> {
        panic!("unexpected message end")
    }
}
impl magicka_runtime::dialog_manager::DialogManagerHost for FrameHost {
    fn bind(
        &mut self,
        _: magicka_runtime::dialog_manager::DialogSink,
        _: magicka_runtime::dialog_manager::ActiveInteraction,
    ) {
        panic!("unexpected bind")
    }
    fn visible(&self, _: magicka_runtime::dialog_manager::DialogSink) -> bool {
        false
    }
    fn animating(&self, _: magicka_runtime::dialog_manager::DialogSink) -> bool {
        false
    }
    fn manager_effect(
        &mut self,
        _: magicka_runtime::dialog_manager::DialogManagerEffect,
    ) -> Result<(), Self::Error> {
        panic!("unexpected effect")
    }
}
impl magicka_runtime::dialog_frame::DialogFrameHost for FrameHost {
    fn update_textbox(
        &mut self,
        _: &mut DialogManager,
        _: magicka_runtime::dialog_frame::DialogFrameTarget,
        _: magicka_runtime::dialog_frame::DialogFrame,
        _: &mut [[f32; 4]; 4],
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn slot_owner_dead(&mut self, _: usize) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn additional_count(&self) -> usize {
        0
    }
    fn additional_visible(&self, _: usize) -> bool {
        panic!("no additional boxes")
    }
    fn remove_additional(&mut self, _: usize) -> Result<(), Self::Error> {
        panic!("no additional boxes")
    }
    fn message_box_count(&self) -> usize {
        self.boxes.active().len()
    }
    fn update_message_box(
        &mut self,
        _: &mut DialogManager,
        index: usize,
        frame: magicka_runtime::dialog_frame::DialogFrame,
    ) -> Result<(), Self::Error> {
        self.boxes
            .update_index(index, frame.channel, [800., 600.], false)
    }
    fn message_box_dead(&self, index: usize) -> bool {
        self.boxes.dead_index(index).unwrap()
    }
    fn remove_message_box(&mut self, index: usize) -> Result<(), Self::Error> {
        self.boxes.remove_index(index)
    }
    fn play_state_in_cutscene(&self) -> bool {
        false
    }
    fn submit_subtitles(&mut self, _: usize, _: f32) -> Result<(), Self::Error> {
        panic!("no subtitles")
    }
}
#[test]
fn real_manager_removes_both_duplicate_entries_after_submitting_each() {
    let mut host = FrameHost {
        boxes: ErrorMessageBoxes::default(),
    };
    let mut manager = DialogManager::new(vec![]).unwrap();
    let h = host
        .boxes
        .insert(ErrorMessageBoxState::new([800., 600.], String::new()));
    for _ in 0..2 {
        host.boxes
            .show(h, "A", &font(false), &mut manager, false)
            .unwrap();
    }
    host.boxes.get_mut(h).unwrap().base.kill();
    host.boxes.begin_frame(0).unwrap();
    manager
        .update(
            magicka_runtime::dialog_frame::DialogFrame {
                channel: 0,
                dt: 0.1,
            },
            &mut [[0.; 4]; 4],
            &mut host,
        )
        .unwrap();
    assert!(host.boxes.active().is_empty());
    assert_eq!(host.boxes.submissions(0).unwrap(), &[h, h]);
    assert_eq!(
        host.boxes
            .draw_submitted(0, 0.1, [800., 600.], false)
            .unwrap()
            .len(),
        2
    );
    host.boxes.begin_frame(0).unwrap();
    manager
        .update(
            magicka_runtime::dialog_frame::DialogFrame {
                channel: 0,
                dt: 0.1,
            },
            &mut [[0.; 4]; 4],
            &mut host,
        )
        .unwrap();
    assert!(host
        .boxes
        .draw_submitted(0, 0.1, [800., 600.], false)
        .unwrap()
        .is_empty());
}
