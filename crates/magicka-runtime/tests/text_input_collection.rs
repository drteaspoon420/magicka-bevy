use magicka_content::assets::{BitmapFont, Glyph};
use magicka_runtime::{
    bitmap_font::FontMetrics,
    dialog_input::DialogInputKind as Input,
    dialog_manager::DialogManager,
    error_message_box::ErrorMessageBoxState,
    localization::LocalizationTable,
    message_box_collection::{
        MessageBoxCollectionError, MessageBoxDispatchError, MessageBoxDrawHost, MessageBoxDrawKind,
        MessageBoxEvent, MessageBoxHandle, MessageBoxInput, MessageBoxes, TextInputCharacter,
        TextInputCollectionError,
    },
    message_box_device::BoxDirection,
    text_input_draw::{TextInputDrawCommand as C, TextInputDrawContext, TextInputDrawHost},
    text_input_message_box::{
        TextInputError, TextInputHost, TextInputMessageBoxState, TextInputType,
    },
    waiting_message_box::WaitingMessageBoxState,
};
fn font() -> FontMetrics {
    FontMetrics::from_content(&BitmapFont {
        line_height: 10,
        base_line: 8,
        default_character: '?',
        texture: None,
        glyphs: vec![Glyph {
            character: '?',
            advance_width: 8,
            origin: [0; 2],
            size: [1; 2],
            left_side_bearing: 0,
            force_white: false,
        }],
        kerning: vec![],
    })
    .unwrap()
}
#[derive(Default)]
struct Host {
    edits: Vec<Option<Vec<u16>>>,
    completions: Vec<(u64, Vec<u16>, bool)>,
    fail: Option<&'static str>,
}
impl TextInputHost for Host {
    type Error = &'static str;
    fn register(&mut self, _: &mut TextInputMessageBoxState) -> Result<(), Self::Error> {
        panic!("collection must own registration")
    }
    fn set_text(
        &mut self,
        _: &mut TextInputMessageBoxState,
        text: Option<&[u16]>,
    ) -> Result<(), Self::Error> {
        self.edits.push(text.map(<[u16]>::to_vec));
        if self.fail == Some("text") {
            Err("text")
        } else {
            Ok(())
        }
    }
    fn complete(
        &mut self,
        state: &mut TextInputMessageBoxState,
        id: u64,
        text: &[u16],
    ) -> Result<(), Self::Error> {
        self.completions
            .push((id, text.to_vec(), state.base.killed));
        if self.fail == Some("complete") {
            Err("complete")
        } else {
            Ok(())
        }
    }
}
fn event(kind: Input) -> MessageBoxEvent {
    MessageBoxInput {
        kind,
        left_released: true,
        inside_ok: false,
    }
    .into()
}
fn type_event(unit: u16) -> MessageBoxEvent {
    MessageBoxEvent {
        character: Some(TextInputCharacter {
            unit,
            clr_is_number: (48..=57).contains(&unit),
        }),
        ..event(Input::Type)
    }
}
fn dispatch(
    boxes: &mut MessageBoxes,
    manager: &mut DialogManager,
    e: MessageBoxEvent,
    host: &mut Host,
) -> Result<(), MessageBoxDispatchError<&'static str>> {
    boxes.dispatch_with_text(manager, e, || Ok(()), |_| Ok(()), |_, _| Ok(()), host)
}
fn insert(boxes: &mut MessageBoxes, capacity: i32) -> MessageBoxHandle {
    boxes.insert_text_input(TextInputMessageBoxState::new([800., 600.], capacity).unwrap())
}
#[test]
fn show_registers_before_capacity_or_text_failure_and_keeps_duplicate_references() {
    let mut boxes = MessageBoxes::default();
    let mut manager = DialogManager::new(vec![]).unwrap();
    let mut host = Host::default();
    let h = insert(&mut boxes, 5);
    boxes.get_text_input_mut(h).unwrap().input_type = TextInputType::DATE;
    assert_eq!(
        boxes.show_text_input(h, Some(7), &mut manager, false, &mut host),
        Err(TextInputCollectionError::TextInput(
            TextInputError::CapacityExceeded
        ))
    );
    assert_eq!(manager.holdoff, 0.2);
    assert_eq!(boxes.active(), [h]);
    assert!(host.edits.is_empty());
    boxes.get_text_input_mut(h).unwrap().input_type = TextInputType::STANDARD;
    host.fail = Some("text");
    manager.holdoff = 0.7;
    assert_eq!(
        boxes.show_text_input(h, Some(8), &mut manager, true, &mut host),
        Err(TextInputCollectionError::TextInput(TextInputError::Host(
            "text"
        )))
    );
    assert_eq!(manager.holdoff, 0.7);
    assert_eq!(boxes.active(), [h, h]);
    let s = boxes.get_text_input(h).unwrap();
    assert!(!s.base.killed);
    assert_eq!(s.completion, Some(8));
    let wrong = boxes.insert(ErrorMessageBoxState::new([800., 600.], String::new()));
    assert_eq!(
        boxes.show_text_input(wrong, None, &mut manager, false, &mut host),
        Err(TextInputCollectionError::Collection(
            MessageBoxCollectionError::WrongKind
        ))
    );
}
#[test]
fn type_and_move_obey_holdoff_route_to_top_reference_and_hide_before_editing() {
    let mut boxes = MessageBoxes::default();
    let mut manager = DialogManager::new(vec![]).unwrap();
    let mut host = Host::default();
    let a = insert(&mut boxes, 8);
    let b = insert(&mut boxes, 8);
    for h in [a, b] {
        boxes
            .show_text_input(h, None, &mut manager, false, &mut host)
            .unwrap();
    }
    host.edits.clear();
    dispatch(&mut boxes, &mut manager, type_event(65), &mut host).unwrap();
    assert!(host.edits.is_empty());
    // Missing payload is also ignored before the native gate opens.
    dispatch(&mut boxes, &mut manager, event(Input::Type), &mut host).unwrap();
    manager.holdoff = 0.;
    dispatch(&mut boxes, &mut manager, type_event(65), &mut host).unwrap();
    assert!(boxes.get_text_input(a).unwrap().text().is_empty());
    assert_eq!(boxes.get_text_input(b).unwrap().text(), [65]);
    let mut hidden = 0;
    let e = MessageBoxEvent {
        direction: Some(BoxDirection::Left),
        ..event(Input::Move)
    };
    boxes
        .dispatch_with_text(
            &mut manager,
            e,
            || {
                hidden += 1;
                Ok(())
            },
            |_| Ok(()),
            |_, _| Ok(()),
            &mut host,
        )
        .unwrap();
    assert_eq!(hidden, 1);
    assert!(boxes.get_text_input(b).unwrap().text().is_empty());
    assert_eq!(host.edits.len(), 3);
    boxes.remove_index(1).unwrap();
    dispatch(&mut boxes, &mut manager, type_event(66), &mut host).unwrap();
    assert_eq!(boxes.get_text_input(a).unwrap().text(), [66]);
}
#[test]
fn missing_payload_host_and_daisy_do_not_create_default_input() {
    let mut boxes = MessageBoxes::default();
    let mut manager = DialogManager::new(vec![]).unwrap();
    let mut host = Host::default();
    let h = insert(&mut boxes, 8);
    boxes
        .show_text_input(h, None, &mut manager, true, &mut host)
        .unwrap();
    let mut hidden = 0;
    assert_eq!(
        boxes.dispatch_with_text(
            &mut manager,
            event(Input::Move),
            || {
                hidden += 1;
                Ok(())
            },
            |_| Ok(()),
            |_, _| Ok(()),
            &mut host
        ),
        Err(MessageBoxDispatchError::MissingInputPayload("direction"))
    );
    assert_eq!(hidden, 1);
    assert!(boxes.get_text_input(h).unwrap().text().is_empty());
    assert_eq!(
        dispatch(&mut boxes, &mut manager, event(Input::Type), &mut host),
        Err(MessageBoxDispatchError::MissingInputPayload("character"))
    );
    dispatch(
        &mut boxes,
        &mut manager,
        MessageBoxEvent {
            daisy_wheel: true,
            ..event(Input::Type)
        },
        &mut host,
    )
    .unwrap();
    assert_eq!(
        boxes.dispatch::<()>(
            &mut manager,
            Input::Type,
            true,
            false,
            || Ok(()),
            |_| Ok(())
        ),
        Err(MessageBoxDispatchError::MissingTextInputHost)
    );
    boxes
        .dispatch::<()>(
            &mut manager,
            Input::MouseRelease,
            true,
            false,
            || Ok(()),
            |_| Ok(()),
        )
        .unwrap();
    // Inherited Escape does not check DaisyWheel and does not complete text.
    dispatch(
        &mut boxes,
        &mut manager,
        MessageBoxEvent {
            daisy_wheel: true,
            ..event(Input::Escape)
        },
        &mut host,
    )
    .unwrap();
    assert!(boxes.get_text_input(h).unwrap().base.killed);
    assert!(host.completions.is_empty());
}
#[test]
fn cancel_bypasses_holdoff_while_select_failure_keeps_live_object() {
    let mut boxes = MessageBoxes::default();
    let mut manager = DialogManager::new(vec![]).unwrap();
    let mut host = Host::default();
    let h = insert(&mut boxes, 8);
    boxes
        .show_text_input(h, Some(7), &mut manager, false, &mut host)
        .unwrap();
    let mut e = event(Input::MouseClick);
    e.inside_cancel = true;
    dispatch(&mut boxes, &mut manager, e, &mut host).unwrap();
    assert_eq!(host.completions, [(7, vec![], false)]);
    boxes
        .show_text_input(h, Some(8), &mut manager, true, &mut host)
        .unwrap();
    manager.holdoff = 0.;
    dispatch(&mut boxes, &mut manager, type_event(65), &mut host).unwrap();
    boxes.get_text_input_mut(h).unwrap().base.alpha = 1.;
    host.fail = Some("complete");
    assert_eq!(
        dispatch(&mut boxes, &mut manager, event(Input::Select), &mut host),
        Err(MessageBoxDispatchError::TextInput(TextInputError::Host(
            "complete"
        )))
    );
    assert!(!boxes.get_text_input(h).unwrap().base.killed);
    assert_eq!(host.completions.last(), Some(&(8, vec![65], false)));
}
struct Draws {
    seen: Vec<MessageBoxHandle>,
    chars: Vec<u16>,
    font: FontMetrics,
    commands: Vec<C>,
}
impl TextInputDrawHost for Draws {
    type Error = ();
    fn command(&mut self, command: C) -> Result<(), ()> {
        if let C::SetInput(text) = &command {
            self.chars = text.clone();
            self.chars.push(0);
        }
        self.commands.push(command);
        Ok(())
    }
    fn input_characters(&self) -> &[u16] {
        &self.chars
    }
    fn okay_title(&mut self) -> Result<Vec<u16>, ()> {
        Ok(vec![79, 75])
    }
}
impl MessageBoxDrawHost for Draws {
    type Error = ();
    fn planned(&mut self, h: MessageBoxHandle, _: MessageBoxDrawKind) -> Result<(), ()> {
        self.seen.push(h);
        Ok(())
    }
    fn text_input(
        &mut self,
        h: MessageBoxHandle,
        s: &mut TextInputMessageBoxState,
        dt: f32,
        screen: [f32; 2],
        daisy_wheel: bool,
    ) -> Result<(), ()> {
        self.seen.push(h);
        let f = self.font.clone();
        s.draw_input(
            dt,
            TextInputDrawContext {
                screen,
                daisy_wheel,
                base_font: &f,
                input_font: &f,
                message_height: 10.,
            },
            self,
        )
        .unwrap();
        Ok(())
    }
}
#[test]
fn duplicate_submissions_draw_real_state_twice_and_survive_active_removal() {
    let mut boxes = MessageBoxes::default();
    let mut manager = DialogManager::new(vec![]).unwrap();
    let mut host = Host::default();
    let h = insert(&mut boxes, 8);
    for _ in 0..2 {
        boxes
            .show_text_input(h, None, &mut manager, true, &mut host)
            .unwrap();
    }
    boxes.begin_frame(2).unwrap();
    for i in 0..2 {
        boxes.update_index(i, 2, [800., 600.], false).unwrap();
    }
    boxes.remove_index(0).unwrap();
    boxes.remove_index(0).unwrap();
    let mut draws = Draws {
        seen: vec![],
        chars: vec![0; 10],
        font: font(),
        commands: vec![],
    };
    boxes
        .visit_submitted(2, 0.125, [800., 600.], false, &mut draws)
        .unwrap();
    assert_eq!(draws.seen, [h, h]);
    let s = boxes.get_text_input(h).unwrap();
    assert_eq!(s.base.alpha, 1.);
    assert_eq!(s.blink_timer, 0.25);
    assert_eq!(
        draws
            .commands
            .iter()
            .filter(|c| matches!(c, C::Begin))
            .count(),
        2
    );
    assert_eq!(
        boxes.draw_submitted_all(2, 0., [800., 600.], false),
        Err(MessageBoxCollectionError::MissingTextInputDrawHost)
    );
    boxes.begin_frame(2).unwrap();
    assert!(boxes.submissions(2).unwrap().is_empty());
    assert!(boxes.get_text_input(h).is_some());
}
#[test]
fn mixed_z_order_and_existing_error_input_remain_concrete() {
    let mut boxes = MessageBoxes::default();
    let mut manager = DialogManager::new(vec![]).unwrap();
    let mut host = Host::default();
    let f = font();
    let text = insert(&mut boxes, 8);
    boxes
        .show_text_input(text, None, &mut manager, true, &mut host)
        .unwrap();
    let error = boxes.insert(ErrorMessageBoxState::new([800., 600.], String::new()));
    boxes.show(error, "X", &f, &mut manager, true).unwrap();
    dispatch(&mut boxes, &mut manager, type_event(65), &mut host).unwrap();
    assert!(boxes.get_text_input(text).unwrap().text().is_empty());
    dispatch(&mut boxes, &mut manager, event(Input::Select), &mut host).unwrap();
    assert_eq!(boxes.get(error).unwrap().timer, 0.);
    let waiting = boxes.insert_waiting(WaitingMessageBoxState::new([800., 600.]));
    let mut strings = LocalizationTable::default();
    strings.strings.insert(1, "W".into());
    boxes
        .show_waiting(waiting, 1, &strings, &f, &mut manager, true)
        .unwrap();
    for i in 0..3 {
        boxes.update_index(i, 0, [800., 600.], false).unwrap();
    }
    assert_eq!(boxes.submissions(0).unwrap(), [waiting, error, text]); // waiting precedes equal-Z error/text
    let mut draws = Draws {
        seen: vec![],
        chars: vec![0; 10],
        font: f,
        commands: vec![],
    };
    boxes
        .visit_submitted(0, 0., [800., 600.], false, &mut draws)
        .unwrap();
    assert_eq!(draws.seen, [waiting, error, text]);
}
