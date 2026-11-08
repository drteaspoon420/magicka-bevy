//! Retained message-box objects, active references and per-channel GUI submissions.
use crate::{
    bitmap_font::{FontError, FontMetrics},
    dialog_input::{DialogInputKind, DialogInputRoute, EmptyMessageBoxStack},
    dialog_manager::DialogManager,
    error_message_box::{ErrorMessageBoxDraw, ErrorMessageBoxShowError, ErrorMessageBoxState},
    localization::LocalizationTable,
    message_box::MessageBoxState,
    message_box_device::BoxDirection,
    text_input_message_box::{TextInputError, TextInputHost, TextInputMessageBoxState},
    waiting_message_box::{WaitingMessageBoxDraw, WaitingMessageBoxState, WaitingShowError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageBoxHandle(usize);
#[derive(Debug, PartialEq, Eq)]
pub enum MessageBoxCollectionError {
    InvalidHandle,
    InvalidIndex(usize),
    InvalidChannel(usize),
    MissingOkButton,
    WrongKind,
    MissingTextInputDrawHost,
    MissingTextInputLanguageHost,
}
#[derive(Debug, PartialEq, Eq)]
pub enum MessageBoxLanguageError {
    Collection(MessageBoxCollectionError),
    Font(FontError),
}
#[derive(Debug, PartialEq, Eq)]
pub enum MessageBoxDispatchError<E> {
    EmptyStack(EmptyMessageBoxStack),
    Collection(MessageBoxCollectionError),
    Callback(E),
    MissingAbortHost,
    MissingTextInputHost,
    MissingInputPayload(&'static str),
    TextInput(TextInputError<E>),
}
pub enum MessageBoxObject {
    Error(Box<ErrorMessageBoxState>),
    Waiting(Box<WaitingMessageBoxState>),
    TextInput(Box<TextInputMessageBoxState>),
}
impl MessageBoxObject {
    fn base(&self) -> &MessageBoxState {
        match self {
            Self::Error(s) => &s.base,
            Self::Waiting(s) => &s.base,
            Self::TextInput(s) => &s.base,
        }
    }
    fn base_mut(&mut self) -> &mut MessageBoxState {
        match self {
            Self::Error(s) => &mut s.base,
            Self::Waiting(s) => &mut s.base,
            Self::TextInput(s) => &mut s.base,
        }
    }
    fn z_index(&self) -> i32 {
        match self {
            Self::Error(_) => 2002,
            Self::Waiting(_) => 2000,
            Self::TextInput(_) => 2002,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MessageBoxDrawKind {
    Error(ErrorMessageBoxDraw),
    Waiting(WaitingMessageBoxDraw),
}
#[derive(Clone, Copy, Debug)]
pub struct MessageBoxInput {
    pub kind: DialogInputKind,
    pub left_released: bool,
    pub inside_ok: bool,
}
/// Payloads for concrete text-entry handlers, after controller validation.
#[derive(Clone, Copy, Debug)]
pub struct TextInputCharacter {
    pub unit: u16,
    pub clr_is_number: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct MessageBoxEvent {
    pub input: MessageBoxInput,
    pub inside_cancel: bool,
    pub direction: Option<BoxDirection>,
    pub character: Option<TextInputCharacter>,
    pub daisy_wheel: bool,
}
impl From<MessageBoxInput> for MessageBoxEvent {
    fn from(input: MessageBoxInput) -> Self {
        Self {
            input,
            inside_cancel: false,
            direction: None,
            character: None,
            daisy_wheel: false,
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum TextInputCollectionError<E> {
    Collection(MessageBoxCollectionError),
    TextInput(TextInputError<E>),
}
/// Immediate draw visitor. Native text operations execute through its concrete
/// TextInputDrawHost; no objects or duplicate references are skipped.
pub trait MessageBoxDrawHost {
    type Error;
    fn planned(
        &mut self,
        handle: MessageBoxHandle,
        draw: MessageBoxDrawKind,
    ) -> Result<(), Self::Error>;
    fn text_input(
        &mut self,
        handle: MessageBoxHandle,
        state: &mut TextInputMessageBoxState,
        dt: f32,
        screen: [f32; 2],
        daisy_wheel: bool,
    ) -> Result<(), Self::Error>;
}
#[derive(Debug, PartialEq, Eq)]
pub enum MessageBoxDrawError<E> {
    Collection(MessageBoxCollectionError),
    Host(E),
}
#[derive(Default)]
pub struct MessageBoxes {
    objects: Vec<MessageBoxObject>,
    active: Vec<MessageBoxHandle>,
    submitted: [Vec<MessageBoxHandle>; 3],
}
/// Compatibility name for the earlier error-only renderer/fixtures.
pub type ErrorMessageBoxes = MessageBoxes;
impl MessageBoxes {
    /// Dispatch a registered object's virtual callback, including killed objects.
    /// ErrorMessageBox inherits the empty base callback; its OK item is not a
    /// SubMenu child and is therefore not automatically relocalized.
    pub fn language_changed(
        &mut self,
        handle: MessageBoxHandle,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<(), MessageBoxLanguageError> {
        let object = self
            .objects
            .get_mut(handle.0)
            .ok_or(MessageBoxLanguageError::Collection(
                MessageBoxCollectionError::InvalidHandle,
            ))?;
        match object {
            MessageBoxObject::Error(_) => Ok(()),
            MessageBoxObject::TextInput(_) => Err(MessageBoxLanguageError::Collection(
                MessageBoxCollectionError::MissingTextInputLanguageHost,
            )),
            MessageBoxObject::Waiting(state) => state
                .language_changed(strings, font)
                .map_err(MessageBoxLanguageError::Font),
        }
    }
    pub fn insert(&mut self, state: ErrorMessageBoxState) -> MessageBoxHandle {
        let handle = MessageBoxHandle(self.objects.len());
        self.objects.push(MessageBoxObject::Error(Box::new(state)));
        handle
    }
    pub fn get(&self, handle: MessageBoxHandle) -> Option<&ErrorMessageBoxState> {
        match self.objects.get(handle.0)? {
            MessageBoxObject::Error(s) => Some(s),
            _ => None,
        }
    }
    pub fn get_mut(&mut self, handle: MessageBoxHandle) -> Option<&mut ErrorMessageBoxState> {
        match self.objects.get_mut(handle.0)? {
            MessageBoxObject::Error(s) => Some(s),
            _ => None,
        }
    }
    pub fn insert_waiting(&mut self, state: WaitingMessageBoxState) -> MessageBoxHandle {
        let handle = MessageBoxHandle(self.objects.len());
        self.objects
            .push(MessageBoxObject::Waiting(Box::new(state)));
        handle
    }
    pub fn get_waiting(&self, handle: MessageBoxHandle) -> Option<&WaitingMessageBoxState> {
        match self.objects.get(handle.0)? {
            MessageBoxObject::Waiting(s) => Some(s),
            _ => None,
        }
    }
    pub fn get_waiting_mut(
        &mut self,
        handle: MessageBoxHandle,
    ) -> Option<&mut WaitingMessageBoxState> {
        match self.objects.get_mut(handle.0)? {
            MessageBoxObject::Waiting(s) => Some(s),
            _ => None,
        }
    }
    pub fn show_waiting(
        &mut self,
        handle: MessageBoxHandle,
        id: i32,
        strings: &LocalizationTable,
        font: &FontMetrics,
        manager: &mut DialogManager,
        paused_play_state: bool,
    ) -> Result<(), WaitingShowError<MessageBoxCollectionError>> {
        let object = self
            .objects
            .get_mut(handle.0)
            .ok_or(WaitingShowError::Registration(
                MessageBoxCollectionError::InvalidHandle,
            ))?;
        let MessageBoxObject::Waiting(state) = object else {
            return Err(WaitingShowError::Registration(
                MessageBoxCollectionError::WrongKind,
            ));
        };
        let active = &mut self.active;
        state.show(id, strings, font, |_| {
            manager.register_message_box(paused_play_state);
            active.push(handle);
            Ok(())
        })
    }
    pub fn active(&self) -> &[MessageBoxHandle] {
        &self.active
    }
    pub fn submissions(
        &self,
        channel: usize,
    ) -> Result<&[MessageBoxHandle], MessageBoxCollectionError> {
        self.submitted
            .get(channel)
            .map(Vec::as_slice)
            .ok_or(MessageBoxCollectionError::InvalidChannel(channel))
    }
    pub fn begin_frame(&mut self, channel: usize) -> Result<(), MessageBoxCollectionError> {
        self.submitted
            .get_mut(channel)
            .ok_or(MessageBoxCollectionError::InvalidChannel(channel))?
            .clear();
        Ok(())
    }
    pub fn show(
        &mut self,
        handle: MessageBoxHandle,
        text: &str,
        font: &FontMetrics,
        manager: &mut DialogManager,
        paused_play_state: bool,
    ) -> Result<(), ErrorMessageBoxShowError<MessageBoxCollectionError>> {
        let object =
            self.objects
                .get_mut(handle.0)
                .ok_or(ErrorMessageBoxShowError::Registration(
                    MessageBoxCollectionError::InvalidHandle,
                ))?;
        let active = &mut self.active;
        let MessageBoxObject::Error(state) = object else {
            return Err(ErrorMessageBoxShowError::Registration(
                MessageBoxCollectionError::WrongKind,
            ));
        };
        state.show(text, font, |_| {
            manager.register_message_box(paused_play_state);
            active.push(handle);
            Ok(())
        })
    }
    /// Methods below match DialogFrameHost's native update/dead/remove loop.
    pub fn update_index(
        &mut self,
        index: usize,
        channel: usize,
        screen: [f32; 2],
        daisy_wheel: bool,
    ) -> Result<(), MessageBoxCollectionError> {
        let handle = *self
            .active
            .get(index)
            .ok_or(MessageBoxCollectionError::InvalidIndex(index))?;
        let submissions = self
            .submitted
            .get_mut(channel)
            .ok_or(MessageBoxCollectionError::InvalidChannel(channel))?;
        if self.objects[handle.0]
            .base_mut()
            .update(screen, daisy_wheel)
        {
            let z = self.objects[handle.0].z_index();
            let (mut low, mut high) = (0, submissions.len());
            let index = loop {
                let mid = (high - low) / 2 + low;
                if mid == low {
                    break mid
                        + usize::from(
                            mid < high && z > self.objects[submissions[mid].0].z_index(),
                        );
                }
                let other = self.objects[submissions[mid].0].z_index();
                if z > other {
                    low = mid + 1;
                } else if z < other {
                    high = mid;
                } else {
                    break mid + 1;
                }
            };
            submissions.insert(index, handle);
        }
        Ok(())
    }
    pub fn dead_index(&self, index: usize) -> Result<bool, MessageBoxCollectionError> {
        let handle = self
            .active
            .get(index)
            .ok_or(MessageBoxCollectionError::InvalidIndex(index))?;
        Ok(self.objects[handle.0].base().dead())
    }
    pub fn remove_index(&mut self, index: usize) -> Result<(), MessageBoxCollectionError> {
        if index >= self.active.len() {
            return Err(MessageBoxCollectionError::InvalidIndex(index));
        }
        self.active.remove(index);
        Ok(())
    }
    /// Retains native GUI insertion order and duplicate references; does not consume list.
    pub fn draw_submitted(
        &mut self,
        channel: usize,
        dt: f32,
        screen: [f32; 2],
        daisy_wheel: bool,
    ) -> Result<Vec<(MessageBoxHandle, ErrorMessageBoxDraw)>, MessageBoxCollectionError> {
        let submissions = self
            .submitted
            .get(channel)
            .ok_or(MessageBoxCollectionError::InvalidChannel(channel))?;
        submissions
            .iter()
            .map(|&handle| match &mut self.objects[handle.0] {
                MessageBoxObject::Error(state) => Ok((handle, state.draw(dt, screen, daisy_wheel))),
                _ => Err(MessageBoxCollectionError::WrongKind),
            })
            .collect()
    }
    pub fn draw_submitted_all(
        &mut self,
        channel: usize,
        dt: f32,
        screen: [f32; 2],
        daisy_wheel: bool,
    ) -> Result<Vec<(MessageBoxHandle, MessageBoxDrawKind)>, MessageBoxCollectionError> {
        let list = self
            .submitted
            .get(channel)
            .ok_or(MessageBoxCollectionError::InvalidChannel(channel))?;
        list.iter()
            .map(|&h| {
                Ok((
                    h,
                    match &mut self.objects[h.0] {
                        MessageBoxObject::Error(s) => {
                            MessageBoxDrawKind::Error(s.draw(dt, screen, daisy_wheel))
                        }
                        MessageBoxObject::Waiting(s) => {
                            MessageBoxDrawKind::Waiting(s.draw(dt, screen))
                        }
                        MessageBoxObject::TextInput(_) => {
                            return Err(MessageBoxCollectionError::MissingTextInputDrawHost)
                        }
                    },
                ))
            })
            .collect()
    }
    /// Bounds are supplied by the menu renderer. Error variant ignores payloads
    /// for Type/Move; Select's empty-stack fallback invokes the real caller host.
    pub fn dispatch<E>(
        &mut self,
        manager: &mut DialogManager,
        input: DialogInputKind,
        left_released: bool,
        inside_ok: bool,
        hide_mouse: impl FnMut() -> Result<(), E>,
        advance: impl FnMut(&mut DialogManager) -> Result<(), E>,
    ) -> Result<(), MessageBoxDispatchError<E>> {
        self.dispatch_inner(
            manager,
            MessageBoxInput {
                kind: input,
                left_released,
                inside_ok,
            }
            .into(),
            hide_mouse,
            advance,
            |_, _| Err(MessageBoxDispatchError::MissingAbortHost),
            |_, _| Err(MessageBoxDispatchError::MissingTextInputHost),
        )
    }
    pub fn dispatch_native<E>(
        &mut self,
        manager: &mut DialogManager,
        input: MessageBoxInput,
        hide_mouse: impl FnMut() -> Result<(), E>,
        advance: impl FnMut(&mut DialogManager) -> Result<(), E>,
        mut abort: impl FnMut(u64, &mut WaitingMessageBoxState) -> Result<(), E>,
    ) -> Result<(), MessageBoxDispatchError<E>> {
        self.dispatch_inner(
            manager,
            input.into(),
            hide_mouse,
            advance,
            |id, state| abort(id, state).map_err(MessageBoxDispatchError::Callback),
            |_, _| Err(MessageBoxDispatchError::MissingTextInputHost),
        )
    }
    fn dispatch_inner<E>(
        &mut self,
        manager: &mut DialogManager,
        event: MessageBoxEvent,
        mut hide_mouse: impl FnMut() -> Result<(), E>,
        mut advance: impl FnMut(&mut DialogManager) -> Result<(), E>,
        mut abort: impl FnMut(
            u64,
            &mut WaitingMessageBoxState,
        ) -> Result<(), MessageBoxDispatchError<E>>,
        mut text_input: impl FnMut(
            &mut TextInputMessageBoxState,
            MessageBoxEvent,
        ) -> Result<(), MessageBoxDispatchError<E>>,
    ) -> Result<(), MessageBoxDispatchError<E>> {
        let input = event.input.kind;
        match manager
            .input_route(input, self.active.len())
            .map_err(MessageBoxDispatchError::EmptyStack)?
        {
            DialogInputRoute::Ignore => Ok(()),
            DialogInputRoute::AdvanceController => {
                advance(manager).map_err(MessageBoxDispatchError::Callback)
            }
            DialogInputRoute::MessageBox {
                index,
                hide_mouse: hide,
            } => {
                if hide {
                    hide_mouse().map_err(MessageBoxDispatchError::Callback)?;
                }
                let handle = self.active[index];
                match &mut self.objects[handle.0] {
                    MessageBoxObject::Error(state) => match input {
                        DialogInputKind::Select => state.on_select(),
                        DialogInputKind::Escape => state.base.kill(),
                        DialogInputKind::MouseClick => {
                            state.on_mouse_click(event.input.left_released, event.input.inside_ok)
                        }
                        DialogInputKind::MouseMove => state.on_mouse_move(event.input.inside_ok),
                        _ => {}
                    },
                    MessageBoxObject::TextInput(state) => {
                        if input == DialogInputKind::Escape {
                            state.base.kill();
                        } else {
                            text_input(state, event)?;
                        }
                    }
                    MessageBoxObject::Waiting(state) => {
                        if input == DialogInputKind::Escape {
                            state.controller_escape(&mut abort)?;
                        }
                    }
                }
                Ok(())
            }
        }
    }
    /// Coordinate-based dispatch uses the retained native menu bounds.
    pub fn dispatch_pointer<E>(
        &mut self,
        manager: &mut DialogManager,
        input: DialogInputKind,
        left_released: bool,
        point: [f32; 2],
        hide_mouse: impl FnMut() -> Result<(), E>,
        advance: impl FnMut(&mut DialogManager) -> Result<(), E>,
    ) -> Result<(), MessageBoxDispatchError<E>> {
        let inside = if matches!(
            input,
            DialogInputKind::MouseClick | DialogInputKind::MouseMove
        ) {
            let handle = self
                .active
                .last()
                .ok_or(MessageBoxDispatchError::EmptyStack(EmptyMessageBoxStack))?;
            if let Some(state) = self.get(*handle) {
                state
                    .ok_button
                    .as_ref()
                    .ok_or(MessageBoxDispatchError::Collection(
                        MessageBoxCollectionError::MissingOkButton,
                    ))?
                    .inside_bounds(point)
            } else {
                false
            } // Waiting mouse handlers are empty.
        } else {
            false
        };
        self.dispatch(manager, input, left_released, inside, hide_mouse, advance)
    }
}

struct TextRegistrationHost<'a, H> {
    active: &'a mut Vec<MessageBoxHandle>,
    handle: MessageBoxHandle,
    manager: &'a mut DialogManager,
    paused_play_state: bool,
    host: &'a mut H,
}
impl<H: TextInputHost> TextInputHost for TextRegistrationHost<'_, H> {
    type Error = H::Error;
    fn register(&mut self, _: &mut TextInputMessageBoxState) -> Result<(), Self::Error> {
        self.manager.register_message_box(self.paused_play_state);
        self.active.push(self.handle);
        Ok(())
    }
    fn set_text(
        &mut self,
        state: &mut TextInputMessageBoxState,
        text: Option<&[u16]>,
    ) -> Result<(), Self::Error> {
        self.host.set_text(state, text)
    }
    fn complete(
        &mut self,
        state: &mut TextInputMessageBoxState,
        callback: u64,
        text: &[u16],
    ) -> Result<(), Self::Error> {
        self.host.complete(state, callback, text)
    }
}
impl MessageBoxes {
    pub fn insert_text_input(&mut self, state: TextInputMessageBoxState) -> MessageBoxHandle {
        let handle = MessageBoxHandle(self.objects.len());
        self.objects
            .push(MessageBoxObject::TextInput(Box::new(state)));
        handle
    }
    pub fn get_text_input(&self, handle: MessageBoxHandle) -> Option<&TextInputMessageBoxState> {
        match self.objects.get(handle.0)? {
            MessageBoxObject::TextInput(s) => Some(s),
            _ => None,
        }
    }
    pub fn get_text_input_mut(
        &mut self,
        handle: MessageBoxHandle,
    ) -> Option<&mut TextInputMessageBoxState> {
        match self.objects.get_mut(handle.0)? {
            MessageBoxObject::TextInput(s) => Some(s),
            _ => None,
        }
    }
    /// This wrapper owns registration; the supplied host owns retained Text and
    /// completion. Its register method is not invoked a second time.
    pub fn show_text_input<H: TextInputHost>(
        &mut self,
        handle: MessageBoxHandle,
        callback: Option<u64>,
        manager: &mut DialogManager,
        paused_play_state: bool,
        host: &mut H,
    ) -> Result<(), TextInputCollectionError<H::Error>> {
        let object = self
            .objects
            .get_mut(handle.0)
            .ok_or(TextInputCollectionError::Collection(
                MessageBoxCollectionError::InvalidHandle,
            ))?;
        let MessageBoxObject::TextInput(state) = object else {
            return Err(TextInputCollectionError::Collection(
                MessageBoxCollectionError::WrongKind,
            ));
        };
        let mut registration = TextRegistrationHost {
            active: &mut self.active,
            handle,
            manager,
            paused_play_state,
            host,
        };
        state
            .show(callback, &mut registration)
            .map_err(TextInputCollectionError::TextInput)
    }
    pub fn dispatch_with_text<H: TextInputHost>(
        &mut self,
        manager: &mut DialogManager,
        event: MessageBoxEvent,
        hide_mouse: impl FnMut() -> Result<(), H::Error>,
        advance: impl FnMut(&mut DialogManager) -> Result<(), H::Error>,
        mut abort: impl FnMut(u64, &mut WaitingMessageBoxState) -> Result<(), H::Error>,
        host: &mut H,
    ) -> Result<(), MessageBoxDispatchError<H::Error>> {
        self.dispatch_inner(
            manager,
            event,
            hide_mouse,
            advance,
            |id, state| abort(id, state).map_err(MessageBoxDispatchError::Callback),
            |state, event| {
                if event.daisy_wheel {
                    return Ok(());
                }
                let result = match event.input.kind {
                    DialogInputKind::Select => state.on_select(false, host),
                    DialogInputKind::Move => {
                        let direction = event
                            .direction
                            .ok_or(MessageBoxDispatchError::MissingInputPayload("direction"))?;
                        state.on_move(direction, false, host)
                    }
                    DialogInputKind::Type => {
                        let character = event
                            .character
                            .ok_or(MessageBoxDispatchError::MissingInputPayload("character"))?;
                        state.on_text_input(character.unit, character.clr_is_number, false, host)
                    }
                    DialogInputKind::MouseClick => state.on_mouse_click(
                        event.input.left_released,
                        event.input.inside_ok,
                        event.inside_cancel,
                        false,
                        host,
                    ),
                    DialogInputKind::MouseMove => {
                        state.on_mouse_move(event.input.inside_ok, event.inside_cancel, false);
                        Ok(())
                    }
                    DialogInputKind::Escape | DialogInputKind::MouseRelease => Ok(()),
                };
                result.map_err(MessageBoxDispatchError::TextInput)
            },
        )
    }
    /// Host executes each concrete draw immediately in native scene order.
    /// This also visits references retained after active-list removal.
    pub fn visit_submitted<H: MessageBoxDrawHost>(
        &mut self,
        channel: usize,
        dt: f32,
        screen: [f32; 2],
        daisy_wheel: bool,
        host: &mut H,
    ) -> Result<(), MessageBoxDrawError<H::Error>> {
        let list = self
            .submitted
            .get(channel)
            .ok_or(MessageBoxDrawError::Collection(
                MessageBoxCollectionError::InvalidChannel(channel),
            ))?;
        for &handle in list {
            match &mut self.objects[handle.0] {
                MessageBoxObject::Error(s) => host.planned(
                    handle,
                    MessageBoxDrawKind::Error(s.draw(dt, screen, daisy_wheel)),
                ),
                MessageBoxObject::Waiting(s) => {
                    host.planned(handle, MessageBoxDrawKind::Waiting(s.draw(dt, screen)))
                }
                MessageBoxObject::TextInput(s) => {
                    host.text_input(handle, s, dt, screen, daisy_wheel)
                }
            }
            .map_err(MessageBoxDrawError::Host)?;
        }
        Ok(())
    }
    pub fn language_changed_with_text(
        &mut self,
        handle: MessageBoxHandle,
        strings: &LocalizationTable,
        font: &FontMetrics,
        mut text_host: impl FnMut(
            &mut TextInputMessageBoxState,
            &LocalizationTable,
            &FontMetrics,
        ) -> Result<(), FontError>,
    ) -> Result<(), MessageBoxLanguageError> {
        if let Some(state) = self.get_text_input_mut(handle) {
            text_host(state, strings, font).map_err(MessageBoxLanguageError::Font)
        } else {
            self.language_changed(handle, strings, font)
        }
    }
}
