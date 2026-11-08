//! Persistent regular/cutscene text and per-message layout ownership for DialogManager.
use crate::{
    bitmap_font::{FontError, FontMetrics, MessageTextLayout},
    cutscene_text::CutsceneTextWidget,
    dialog_definition::{DialogDefinition, MessageDefinition},
    dialog_frame::{DialogFrame, DialogFrameHost, DialogFrameTarget},
    dialog_manager::{
        ActiveInteraction, DialogManager, DialogManagerEffect, DialogManagerError,
        DialogManagerHost, DialogSink,
    },
    interaction_playback::InteractionHost,
    localization::LocalizationTable,
    message_playback::{MessageHost, MessagePlacement, MessagePlayback, MessagePlaybackError},
    message_textbox::{MessageTextBoxError, MessageTextBoxHost, MessageTextBoxWorld},
    text_box::{TextBoxError, TextBoxOwner},
    text_box_initialize::{TextBoxFonts, TextBoxWidget},
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DialogMessageKey {
    pub collection: u64,
    pub dialog: i32,
    pub interaction: usize,
    pub message: usize,
}
/// Concrete world owns cues keyed by DialogMessageKey, entity positions,
/// input/camera/audio operations. DialogTextHost owns the cutscene text object.
pub trait DialogTextWorld: MessageTextBoxWorld {
    fn bind_message(&mut self, key: DialogMessageKey) -> Result<(), <Self as MessageHost>::Error>;
    fn lock_controller(&mut self, controller: i32) -> Result<(), <Self as MessageHost>::Error>;
    fn unlock_controller(&mut self, controller: i32) -> Result<(), <Self as MessageHost>::Error>;
    fn stop_message_cue(&mut self) -> Result<(), <Self as MessageHost>::Error>;
    fn manager_effect(
        &mut self,
        effect: DialogManagerEffect,
    ) -> Result<(), <Self as MessageHost>::Error>;
}
/// Scene facts and the native UI types distinct from regular TextBoxWidget.
pub trait DialogTextFrameWorld: DialogTextWorld {
    fn text_owner(&self, owner: i32) -> Option<TextBoxOwner>;
    fn owner_dead(&self, owner: i32) -> bool;
    fn in_cutscene(&self) -> bool;
    fn message_box_count(&self) -> usize;
    fn update_message_box(
        &mut self,
        index: usize,
        frame: DialogFrame,
    ) -> Result<(), <Self as MessageHost>::Error>;
    fn message_box_dead(&self, index: usize) -> bool;
    fn remove_message_box(&mut self, index: usize) -> Result<(), <Self as MessageHost>::Error>;
    fn submit_subtitles(
        &mut self,
        channel: usize,
        height: f32,
    ) -> Result<(), <Self as MessageHost>::Error>;
}
#[derive(Debug, PartialEq, Eq)]
pub enum DialogTextHostError<E> {
    World(E),
    Font(FontError),
    MissingContext,
    MissingLayout(DialogMessageKey),
    InvalidSlot(usize),
    Playback(MessagePlaybackError<MessageTextBoxError<E>>),
    TextBox(TextBoxError),
    Advancement(Box<DialogManagerError<DialogTextHostError<E>>>),
}
pub struct DialogTextHost<W> {
    pub boxes: [TextBoxWidget; 8],
    pub cutscene: CutsceneTextWidget,
    pub cutscene_submitted: [bool; 3],
    pub additional: Vec<TextBoxWidget>,
    pub layouts: BTreeMap<DialogMessageKey, MessageTextLayout>,
    pub strings: LocalizationTable,
    pub body_font: FontMetrics,
    pub name_font: FontMetrics,
    pub scene: Option<i32>,
    pub world: W,
    pub substitution_budget: usize,
    pub graphics_present: bool,
    pub collection_token: u64,
    context: Option<(DialogSink, ActiveInteraction)>,
}
impl<W: DialogTextWorld> DialogTextHost<W> {
    /// The render-thread half of the visible boxes (TextBox/CutsceneText
    /// PreRender: typing advance and placement) for runs without a
    /// presentation; typing, and hence TTL auto-advance, depends on it.
    pub fn pre_render_visible(
        &mut self,
        channel: usize,
        dt: f32,
        screen: [f32; 2],
        view_projection: [[f32; 4]; 4],
    ) -> Result<(), TextBoxError> {
        for widget in self.boxes.iter_mut().chain(self.additional.iter_mut()) {
            if widget.state.scene_initialized && widget.state.visible() {
                widget
                    .state
                    .pre_render(channel, dt, screen, view_projection)?;
            }
        }
        let submitted = self
            .cutscene_submitted
            .get(channel)
            .copied()
            .unwrap_or(false);
        let cutscene = &mut self.cutscene.widget.state;
        if cutscene.scene_initialized && cutscene.visible() && submitted {
            crate::cutscene_text::pre_render(cutscene, channel, dt, screen)?;
        }
        Ok(())
    }
    pub fn new(
        world: W,
        strings: LocalizationTable,
        body_font: FontMetrics,
        name_font: FontMetrics,
        substitution_budget: usize,
    ) -> Self {
        Self {
            boxes: std::array::from_fn(|_| TextBoxWidget::default()),
            cutscene: CutsceneTextWidget::default(),
            cutscene_submitted: [false; 3],
            additional: vec![],
            layouts: BTreeMap::new(),
            strings,
            body_font,
            name_font,
            scene: None,
            world,
            substitution_budget,
            graphics_present: true,
            collection_token: 0,
            context: None,
        }
    }
    /// Explicit initialization phase, not lazy first-Show localization. Repeated
    /// calls retain MessageTextLayout's native accumulated minimum dimensions.
    pub fn initialize_layouts(
        &mut self,
        definitions: &[DialogDefinition],
    ) -> Result<(), DialogTextHostError<<W as MessageHost>::Error>> {
        self.initialize_collection_layouts(self.collection_token, definitions)
    }
    pub fn select_collection(&mut self, token: u64) {
        self.collection_token = token;
        self.context = None;
    }
    /// Prepare another collection without rebinding the currently active one.
    pub fn initialize_collection_layouts(
        &mut self,
        token: u64,
        definitions: &[DialogDefinition],
    ) -> Result<(), DialogTextHostError<<W as MessageHost>::Error>> {
        for dialog in definitions {
            for (interaction, definition) in dialog.interactions.iter().enumerate() {
                for (message, definition) in definition.messages.iter().enumerate() {
                    let key = DialogMessageKey {
                        collection: token,
                        dialog: dialog.id,
                        interaction,
                        message,
                    };
                    self.layouts
                        .entry(key)
                        .or_default()
                        .initialize(
                            definition,
                            &self.strings,
                            &self.name_font,
                            self.substitution_budget,
                        )
                        .map_err(DialogTextHostError::Font)?;
                }
            }
        }
        Ok(())
    }
    fn key(
        &self,
        message: usize,
    ) -> Result<DialogMessageKey, DialogTextHostError<<W as MessageHost>::Error>> {
        let (_, active) = self.context.ok_or(DialogTextHostError::MissingContext)?;
        Ok(DialogMessageKey {
            collection: self.collection_token,
            dialog: active.dialog,
            interaction: active.interaction,
            message,
        })
    }
    /// Call from the manager frame host with the current owner position. The
    /// synchronous advancement can replace this box before its snapshot finishes.
    pub fn update_regular(
        &mut self,
        manager: &mut DialogManager,
        slot: usize,
        frame: DialogFrame,
        owner: Option<TextBoxOwner>,
        cutscene: bool,
    ) -> Result<(), DialogTextHostError<<W as MessageHost>::Error>> {
        let widget = self
            .boxes
            .get_mut(slot)
            .ok_or(DialogTextHostError::InvalidSlot(slot))?;
        let update = widget
            .state
            .begin_update(frame.dt, frame.channel, owner)
            .map_err(DialogTextHostError::TextBox)?;
        if let Some(update) = update {
            if update.advance {
                manager
                    .advance_textbox(DialogSink::Slot(slot), cutscene, self)
                    .map_err(|e| DialogTextHostError::Advancement(Box::new(e)))?;
            }
            self.boxes[slot].state.finish_update(update);
        }
        Ok(())
    }
}
impl<W: DialogTextWorld> InteractionHost for DialogTextHost<W> {
    type Error = DialogTextHostError<<W as MessageHost>::Error>;
    fn advance_message(
        &mut self,
        index: usize,
        definition: &MessageDefinition,
        playback: &mut MessagePlayback,
        placement: MessagePlacement,
    ) -> Result<bool, Self::Error> {
        let key = self.key(index)?;
        self.world
            .bind_message(key)
            .map_err(DialogTextHostError::World)?;
        let (sink, _) = self.context.unwrap();
        let layout = self
            .layouts
            .get(&key)
            .ok_or(DialogTextHostError::MissingLayout(key))?;
        if sink == DialogSink::Cutscene {
            let mut host = MessageTextBoxHost {
                widget: &mut self.cutscene,
                layout,
                strings: &self.strings,
                fonts: TextBoxFonts {
                    body: &self.name_font,
                    owner_name: &self.name_font,
                },
                scene: self.scene,
                world: &mut self.world,
                substitution_budget: self.substitution_budget,
            };
            return playback
                .advance(definition, placement, &mut host)
                .map_err(DialogTextHostError::Playback);
        }
        let DialogSink::Slot(slot) = sink else {
            unreachable!()
        };
        let widget = self
            .boxes
            .get_mut(slot)
            .ok_or(DialogTextHostError::InvalidSlot(slot))?;
        let layout = self
            .layouts
            .get(&key)
            .ok_or(DialogTextHostError::MissingLayout(key))?;
        let mut host = MessageTextBoxHost {
            widget,
            layout,
            strings: &self.strings,
            fonts: TextBoxFonts {
                body: &self.body_font,
                owner_name: &self.name_font,
            },
            scene: self.scene,
            world: &mut self.world,
            substitution_budget: self.substitution_budget,
        };
        playback
            .advance(definition, placement, &mut host)
            .map_err(DialogTextHostError::Playback)
    }
    fn lock_input(&mut self, controller: i32) -> Result<(), Self::Error> {
        self.world
            .lock_controller(controller)
            .map_err(DialogTextHostError::World)
    }
    fn unlock_input(&mut self, controller: i32) -> Result<(), Self::Error> {
        self.world
            .unlock_controller(controller)
            .map_err(DialogTextHostError::World)
    }
    fn end_message(&mut self, index: usize, _: &MessageDefinition) -> Result<(), Self::Error> {
        let key = self.key(index)?;
        self.world
            .bind_message(key)
            .map_err(DialogTextHostError::World)?;
        self.world
            .stop_message_cue()
            .map_err(DialogTextHostError::World)
    }
}
impl<W: DialogTextWorld> DialogManagerHost for DialogTextHost<W> {
    fn bind(&mut self, sink: DialogSink, active: ActiveInteraction) {
        self.context = Some((sink, active));
    }
    fn visible(&self, sink: DialogSink) -> bool {
        match sink {
            DialogSink::Slot(i) => self.boxes[i].state.visible(),
            DialogSink::Cutscene => self.cutscene.widget.state.visible(),
        }
    }
    fn animating(&self, sink: DialogSink) -> bool {
        match sink {
            DialogSink::Slot(i) => self.boxes[i].state.animating(),
            DialogSink::Cutscene => self.cutscene.widget.state.animating(),
        }
    }
    fn manager_effect(&mut self, effect: DialogManagerEffect) -> Result<(), Self::Error> {
        match effect {
            DialogManagerEffect::Hide(DialogSink::Cutscene) => {
                self.cutscene.widget.state.hide();
                Ok(())
            }
            DialogManagerEffect::FinishAnimation {
                sink: DialogSink::Cutscene,
                remove_ttl_advance,
            } => {
                self.cutscene
                    .widget
                    .state
                    .finish_animation(remove_ttl_advance, self.graphics_present);
                Ok(())
            }
            DialogManagerEffect::FinishAnimation {
                sink: DialogSink::Slot(i),
                remove_ttl_advance,
            } => {
                self.boxes
                    .get_mut(i)
                    .ok_or(DialogTextHostError::InvalidSlot(i))?
                    .state
                    .finish_animation(remove_ttl_advance, self.graphics_present);
                Ok(())
            }
            DialogManagerEffect::Hide(DialogSink::Slot(i)) => {
                self.boxes
                    .get_mut(i)
                    .ok_or(DialogTextHostError::InvalidSlot(i))?
                    .state
                    .hide();
                Ok(())
            }
            DialogManagerEffect::HideAdditional => {
                for widget in &mut self.additional {
                    widget.state.hide();
                }
                Ok(())
            }
            effect => self
                .world
                .manager_effect(effect)
                .map_err(DialogTextHostError::World),
        }
    }
}
impl<W: DialogTextFrameWorld> DialogFrameHost for DialogTextHost<W> {
    fn update_textbox(
        &mut self,
        manager: &mut DialogManager,
        target: DialogFrameTarget,
        frame: DialogFrame,
        _matrix: &mut [[f32; 4]; 4],
    ) -> Result<(), Self::Error> {
        match target {
            DialogFrameTarget::Slot(i) => {
                if i == 0 {
                    if let Some(submitted) = self.cutscene_submitted.get_mut(frame.channel) {
                        *submitted = false;
                    }
                }
                let widget = self
                    .boxes
                    .get(i)
                    .ok_or(DialogTextHostError::InvalidSlot(i))?;
                let owner = widget.state.owner.and_then(|id| self.world.text_owner(id));
                self.update_regular(manager, i, frame, owner, self.world.in_cutscene())
            }
            DialogFrameTarget::Additional(i) => {
                let widget = self
                    .additional
                    .get_mut(i)
                    .ok_or(DialogTextHostError::InvalidSlot(i))?;
                let owner = widget.state.owner.and_then(|id| self.world.text_owner(id));
                let update = widget
                    .state
                    .begin_update(frame.dt, frame.channel, owner)
                    .map_err(DialogTextHostError::TextBox)?;
                if let Some(update) = update {
                    if update.advance {
                        // Advance(box) finds no regular slot for this additional
                        // box, but still performs the native HideAdditional tail.
                        self.manager_effect(DialogManagerEffect::HideAdditional)?;
                    }
                    self.additional[i].state.finish_update(update);
                }
                Ok(())
            }
            DialogFrameTarget::Cutscene => {
                let state = &mut self.cutscene.widget.state;
                let owner = state.owner.and_then(|id| self.world.text_owner(id));
                let update = state
                    .begin_update(frame.dt, frame.channel, owner)
                    .map_err(DialogTextHostError::TextBox)?;
                if let Some(update) = update {
                    if update.advance {
                        manager
                            .advance_textbox(DialogSink::Cutscene, self.world.in_cutscene(), self)
                            .map_err(|e| DialogTextHostError::Advancement(Box::new(e)))?;
                    }
                    self.cutscene.widget.state.finish_update(update);
                    self.cutscene_submitted[frame.channel] = true;
                }
                Ok(())
            }
        }
    }
    fn slot_owner_dead(&mut self, slot: usize) -> Result<bool, Self::Error> {
        let widget = self
            .boxes
            .get(slot)
            .ok_or(DialogTextHostError::InvalidSlot(slot))?;
        Ok(widget
            .state
            .owner
            .is_some_and(|id| self.world.owner_dead(id)))
    }
    fn additional_count(&self) -> usize {
        self.additional.len()
    }
    fn additional_visible(&self, index: usize) -> bool {
        self.additional[index].state.visible()
    }
    fn remove_additional(&mut self, index: usize) -> Result<(), Self::Error> {
        self.additional.remove(index);
        Ok(())
    }
    fn message_box_count(&self) -> usize {
        self.world.message_box_count()
    }
    fn update_message_box(
        &mut self,
        _: &mut DialogManager,
        index: usize,
        frame: DialogFrame,
    ) -> Result<(), Self::Error> {
        self.world
            .update_message_box(index, frame)
            .map_err(DialogTextHostError::World)
    }
    fn message_box_dead(&self, index: usize) -> bool {
        self.world.message_box_dead(index)
    }
    fn remove_message_box(&mut self, index: usize) -> Result<(), Self::Error> {
        self.world
            .remove_message_box(index)
            .map_err(DialogTextHostError::World)
    }
    fn play_state_in_cutscene(&self) -> bool {
        self.world.in_cutscene()
    }
    fn submit_subtitles(&mut self, channel: usize, height: f32) -> Result<(), Self::Error> {
        self.world
            .submit_subtitles(channel, height)
            .map_err(DialogTextHostError::World)
    }
}
