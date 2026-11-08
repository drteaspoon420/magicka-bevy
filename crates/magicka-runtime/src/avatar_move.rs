//! Script AvatarMove: Initialize-time mapping of PlayerId/PlayerN event lists to
//! the four player slots, and Execute's assignment to live avatars.
use crate::{
    ai_event::{AiEvent, AiEventLevel, ResolveError},
    scene_loading::custom_hash,
    scene_script::XmlElement,
    script_attributes::{number, ScriptReadError},
};

pub const PLAYER_SLOTS: usize = 4;

#[derive(Clone, Debug)]
pub struct AvatarMoveAction {
    pub delay: f32,
    pub node: XmlElement,
}

/// mEvents: one optional event list per player slot.
pub type AvatarMoveEvents = [Option<Vec<AiEvent>>; PLAYER_SLOTS];

pub trait AvatarMoveLevel: AiEventLevel {
    /// Entity.GetByID(id) identity, if any entity has that unique ID.
    fn entity_by_id(&self, id: i32) -> Option<u16>;
    /// Game.Instance.Players[i].Avatar identities.
    fn player_avatars(&self) -> Vec<Option<u16>>;
}

pub trait AvatarMoveHost {
    type Error;
    fn player_avatars(&self) -> Vec<Option<u16>>;
    /// players[i].Avatar.Events = events.
    fn set_avatar_events(&mut self, player: usize, events: &[AiEvent]) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum AvatarMoveError<E> {
    Event(ResolveError<E>),
    /// "Invalid node ... in AvatarMove! Expected Player1-Player4."
    InvalidNode(String),
    /// A PlayerN digit below '1' indexes before the array.
    SlotOutOfRange(String),
    /// "Unable to map AvatarMove event" after more than ten wraps.
    Unmappable(String),
}

fn is_player_id(name: &str) -> bool {
    name.as_bytes()
        .get(..8)
        .is_some_and(|p| p.eq_ignore_ascii_case(b"playerid"))
}
/// `StartsWith("player")` and a final char.IsDigit (ASCII digits only here).
fn player_digit(name: &str) -> Option<u8> {
    let last = *name.as_bytes().last()?;
    let player = name
        .as_bytes()
        .get(..6)
        .is_some_and(|p| p.eq_ignore_ascii_case(b"player"));
    (player && last.is_ascii_digit()).then_some(last)
}

impl AvatarMoveAction {
    /// Validates node names and event children at load; the original reports
    /// these at Initialize.
    pub fn read(node: &XmlElement) -> Result<Self, ScriptReadError> {
        let mut delay = 0.0;
        for (key, value) in &node.attributes {
            if key.eq_ignore_ascii_case("delay") {
                delay = number(value, key)?;
            }
        }
        if node.non_comment_text_children {
            return Err(ScriptReadError::UnsupportedTag("#text".into()));
        }
        for child in &node.children {
            if !is_player_id(&child.name) && player_digit(&child.name).is_none() {
                return Err(ScriptReadError::UnsupportedTag(child.name.clone()));
            }
            AiEvent::read_children(child)?;
        }
        Ok(Self {
            delay,
            node: node.clone(),
        })
    }

    pub fn initialize<L: AvatarMoveLevel + ?Sized>(
        &self,
        level: &mut L,
    ) -> Result<AvatarMoveEvents, AvatarMoveError<L::Error>> {
        let resolve = |node: &XmlElement, level: &mut L| {
            node.children
                .iter()
                .map(|c| AiEvent::resolve(c, level))
                .collect::<Result<Vec<_>, _>>()
                .map_err(AvatarMoveError::Event)
        };
        let mut events: AvatarMoveEvents = Default::default();
        for child in self.node.children.iter().filter(|c| is_player_id(&c.name)) {
            // The last name attribute wins; missing name hashes "".
            let mut name = String::new();
            for (key, value) in &child.attributes {
                if key.eq_ignore_ascii_case("name") {
                    name = value.to_ascii_lowercase();
                }
            }
            let list = resolve(child, level)?;
            let Some(entity) = level.entity_by_id(custom_hash(&name)) else {
                continue;
            };
            if let Some(slot) = level
                .player_avatars()
                .iter()
                .position(|a| *a == Some(entity))
            {
                events[slot] = Some(list);
            }
        }
        for child in &self.node.children {
            let Some(digit) = player_digit(&child.name) else {
                if !is_player_id(&child.name) {
                    return Err(AvatarMoveError::InvalidNode(child.name.clone()));
                }
                continue;
            };
            let list = resolve(child, level)?;
            let mut slot = digit as i32 - 49;
            let mut wraps = 0;
            loop {
                if slot > 3 {
                    slot = 0;
                    wraps += 1;
                    if wraps > 10 {
                        return Err(AvatarMoveError::Unmappable(child.name.clone()));
                    }
                }
                if slot < 0 {
                    return Err(AvatarMoveError::SlotOutOfRange(child.name.clone()));
                }
                if events[slot as usize].is_none() {
                    break;
                }
                slot += 1;
            }
            events[slot as usize] = Some(list);
        }
        Ok(events)
    }

    /// Assigns mapped events to players that currently have an avatar.
    /// QuickExecute is empty. Rewrite extension: slots beyond the original
    /// four follow the authored list of `slot % 4`, so scripted walks
    /// (doors, cutscenes) also move the extra players of a larger lobby.
    pub fn execute<H: AvatarMoveHost + ?Sized>(
        events: &AvatarMoveEvents,
        host: &mut H,
    ) -> Result<(), H::Error> {
        let avatars = host.player_avatars();
        for (player, avatar) in avatars.iter().enumerate() {
            if let (Some(_), Some(list)) = (avatar, &events[player % PLAYER_SLOTS]) {
                host.set_avatar_events(player, list)?;
            }
        }
        Ok(())
    }
}
