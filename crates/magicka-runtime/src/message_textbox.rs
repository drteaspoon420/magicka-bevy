//! Bind native MessagePlayback Show/Hide to an actual initialized TextBoxWidget.
use crate::{
    bitmap_font::MessageTextLayout,
    dialog_definition::MessageDefinition,
    localization::LocalizationTable,
    message_playback::{MessageEffect, MessageHost, MessageOwner, MessagePlacement},
    text_box_initialize::{
        TextBoxFonts, TextBoxInit, TextBoxInitError, TextBoxInitHost, TextBoxInitTarget,
        TextBoxLocation, TextBoxWidget,
    },
};

/// The caller binds cue ownership to this particular parent/interaction/message.
pub trait MessageTextBoxWorld:
    MessageHost + TextBoxInitHost<Error = <Self as MessageHost>::Error>
{
    fn entity_scene(&mut self, owner: i32) -> Result<Option<i32>, <Self as MessageHost>::Error>;
}
#[derive(Debug, PartialEq, Eq)]
pub enum MessageTextBoxError<E> {
    World(E),
    Initialize(TextBoxInitError<E>),
    MissingPreparedText(usize),
    MissingDefinitionText(usize),
    MissingEntityOwner,
}
/// Ephemeral binding; prepared layout and retained widget survive each advance.
pub struct MessageTextBoxHost<'a, W, T = TextBoxWidget> {
    pub widget: &'a mut T,
    pub layout: &'a MessageTextLayout,
    pub strings: &'a LocalizationTable,
    pub fonts: TextBoxFonts<'a>,
    pub scene: Option<i32>,
    pub world: &'a mut W,
    pub substitution_budget: usize,
}
impl<W: MessageTextBoxWorld, T: TextBoxInitTarget> MessageHost for MessageTextBoxHost<'_, W, T> {
    type Error = MessageTextBoxError<<W as MessageHost>::Error>;
    fn resolve_owner(&mut self, id: i32) -> Result<Option<MessageOwner>, Self::Error> {
        self.world
            .resolve_owner(id)
            .map_err(MessageTextBoxError::World)
    }
    fn cue_stopping(&self) -> Option<bool> {
        self.world.cue_stopping()
    }
    fn effect(
        &mut self,
        definition: &MessageDefinition,
        effect: MessageEffect,
    ) -> Result<(), Self::Error> {
        match effect {
            MessageEffect::Hide => {
                self.widget.state_mut().hide();
                Ok(())
            }
            MessageEffect::Show {
                text,
                placement,
                owner,
            } => {
                let (scene, location) = match placement {
                    MessagePlacement::Screen(position) => (
                        self.scene,
                        TextBoxLocation::Screen {
                            position,
                            name: definition.owner,
                        },
                    ),
                    MessagePlacement::World { position, .. } => (
                        self.scene,
                        match owner {
                            Some(owner) => TextBoxLocation::Entity(Some(owner)),
                            None => TextBoxLocation::World {
                                position,
                                name: definition.owner,
                            },
                        },
                    ),
                    MessagePlacement::Entity { .. } => {
                        let owner = owner.ok_or(MessageTextBoxError::MissingEntityOwner)?;
                        let scene = self
                            .world
                            .entity_scene(owner)
                            .map_err(MessageTextBoxError::World)?;
                        (scene, TextBoxLocation::Entity(Some(owner)))
                    }
                };
                let prepared = self
                    .layout
                    .texts
                    .get(text)
                    .ok_or(MessageTextBoxError::MissingPreparedText(text))?;
                let definition_text = definition
                    .texts
                    .get(text)
                    .ok_or(MessageTextBoxError::MissingDefinitionText(text))?;
                self.widget
                    .initialize_target(
                        TextBoxInit {
                            scene,
                            location,
                            text: prepared.as_deref(),
                            minimum_size: self.layout.minimum_box_size,
                            force_on_screen: definition.force_on_screen,
                            ttl: definition_text.ttl,
                        },
                        self.strings,
                        TextBoxFonts {
                            body: self.fonts.body,
                            owner_name: self.fonts.owner_name,
                        },
                        self.world,
                        self.substitution_budget,
                    )
                    .map_err(MessageTextBoxError::Initialize)
            }
            effect => self
                .world
                .effect(definition, effect)
                .map_err(MessageTextBoxError::World),
        }
    }
}
