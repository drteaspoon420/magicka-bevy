//! SetPosition, Remove, Kill, SetDialog and ReleaseGrip trigger actions through `EntityCommandHost`.
use crate::{
    presence::factions,
    scene_loading::custom_hash,
    script_attributes::{boolean, identifier, number, ScriptReadError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterSelector {
    /// Specific ID (non-empty ID setter).
    Id(i32),
    /// TriggerArea.PresentCharacters filtered by type "any", type or faction bits.
    Area {
        area: i32,
        type_id: i32,
        factions: i32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum EntityCommandKind {
    SetDialog {
        id: i32,
        dialog: i32,
    },
    SetPosition {
        /// Lowercased ID string; `None` makes the original dereference null.
        id: Option<String>,
        facing_direction: bool,
        area: i32,
    },
    Remove {
        target: CharacterSelector,
        force: bool,
    },
    Kill {
        target: CharacterSelector,
    },
    /// ReleaseGrip: NonPlayerCharacter targets only (type checks on both
    /// paths), each releasing its own victim and then its gripper's hold.
    ReleaseGrip {
        target: CharacterSelector,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityCommand {
    pub delay: f32,
    pub kind: EntityCommandKind,
}

pub const TAGS: &[&str] = &["setposition", "remove", "kill", "setdialog", "releasegrip"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityFacts {
    pub handle: u16,
    /// Character subclass with its HeightOffset.
    pub character: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AreaCharacter {
    pub handle: u16,
    pub type_id: i32,
    pub original_faction: i32,
}

pub trait EntityCommandHost {
    type Error;
    /// Players[slot] when Playing and Avatar != null.
    fn playing_avatar(&mut self, slot: usize) -> Result<Option<EntityFacts>, Self::Error>;
    fn entity_by_id(&mut self, id: i32) -> Result<Option<EntityFacts>, Self::Error>;
    fn locator(&mut self, area: i32) -> Result<[f32; 16], Self::Error>;
    /// LevelModel.CollisionSkin.SegmentIntersect position, if hit.
    fn level_segment(
        &mut self,
        origin: [f32; 3],
        delta: [f32; 3],
    ) -> Result<Option<[f32; 3]>, Self::Error>;
    fn body_orientation(&mut self, handle: u16) -> Result<[f32; 16], Self::Error>;
    fn set_desired_direction(
        &mut self,
        handle: u16,
        direction: [f32; 3],
    ) -> Result<(), Self::Error>;
    /// Body.Velocity = 0 then Body.MoveTo(position, orientation).
    fn move_body(
        &mut self,
        handle: u16,
        position: [f32; 3],
        orientation: [f32; 16],
    ) -> Result<(), Self::Error>;
    /// GameScene.GetTriggerArea(area).PresentCharacters; `None` for a missing key.
    fn present_characters(&mut self, area: i32) -> Result<Option<Vec<AreaCharacter>>, Self::Error>;
    fn clear_cannot_die(&mut self, handle: u16) -> Result<(), Self::Error>;
    fn kill(&mut self, handle: u16) -> Result<(), Self::Error>;
    /// Character.Terminate(iKillItems: true, iNetwork: false).
    fn terminate(&mut self, handle: u16) -> Result<(), Self::Error>;
    fn set_dialog(&mut self, handle: u16, dialog: i32) -> Result<(), Self::Error>;
    /// For a NonPlayerCharacter `handle` (anything else is ignored):
    /// ReleaseAttachedCharacter, then Gripper?.ReleaseAttachedCharacter.
    fn release_npc_grip(&mut self, handle: u16) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum EntityCommandError<E> {
    /// SetPosition with no ID: mID.Equals on null.
    MissingId,
    MissingArea(i32),
    Host(E),
}
impl<E> From<E> for EntityCommandError<E> {
    fn from(e: E) -> Self {
        Self::Host(e)
    }
}

impl EntityCommand {
    pub fn read<'a>(
        tag: &str,
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        let tag_lower = tag.to_ascii_lowercase();
        if !TAGS.contains(&tag_lower.as_str()) {
            return Err(ScriptReadError::UnsupportedTag(tag.into()));
        }
        let mut delay = 0.0;
        let mut dialog = 0;
        let (mut id_text, mut id, mut type_id, mut area, mut faction_mask) = (None, None, 0, 0, 0);
        let (mut facing, mut force) = (false, false);
        for (key, value) in attrs {
            let lower = key.to_ascii_lowercase();
            match lower.as_str() {
                "delay" => delay = number(value, key)?,
                "dialog" if tag_lower == "setdialog" => dialog = identifier(value)?,
                "id" => {
                    if !value.is_ascii() {
                        return Err(ScriptReadError::UnsupportedName(value.into()));
                    }
                    id_text = Some(value.to_ascii_lowercase());
                    id = (tag_lower == "setdialog" || !value.is_empty())
                        .then(|| identifier(value))
                        .transpose()?;
                }
                "area" => area = identifier(value)?,
                "facingdirection" if tag_lower == "setposition" => facing = boolean(value, key)?,
                "type" if tag_lower != "setposition" => type_id = identifier(value)?,
                "factions" if tag_lower != "setposition" => faction_mask = factions(value, key)?,
                "force" if tag_lower == "remove" => force = boolean(value, key)?,
                _ => {}
            }
        }
        let target = match id {
            Some(id) => CharacterSelector::Id(id),
            None => CharacterSelector::Area {
                area,
                type_id,
                factions: faction_mask,
            },
        };
        let kind = match tag_lower.as_str() {
            "setdialog" => EntityCommandKind::SetDialog {
                id: id.unwrap_or(0),
                dialog,
            },
            "setposition" => EntityCommandKind::SetPosition {
                id: id_text,
                facing_direction: facing,
                area,
            },
            "remove" => EntityCommandKind::Remove { target, force },
            "releasegrip" => EntityCommandKind::ReleaseGrip { target },
            _ => EntityCommandKind::Kill { target },
        };
        Ok(Self { delay, kind })
    }

    fn area_targets<H: EntityCommandHost + ?Sized>(
        host: &mut H,
        area: i32,
        type_id: i32,
        mask: i32,
    ) -> Result<Vec<u16>, EntityCommandError<H::Error>> {
        Ok(host
            .present_characters(area)?
            .ok_or(EntityCommandError::MissingArea(area))?
            .into_iter()
            .filter(|c| {
                type_id == custom_hash("any")
                    || c.type_id == type_id
                    || c.original_faction & mask != 0
            })
            .map(|c| c.handle)
            .collect())
    }

    /// Execute; every QuickExecute override calls Execute.
    pub fn execute<H: EntityCommandHost + ?Sized>(
        &self,
        host: &mut H,
    ) -> Result<(), EntityCommandError<H::Error>> {
        match &self.kind {
            EntityCommandKind::SetDialog { id, dialog } => {
                if let Some(EntityFacts {
                    handle,
                    character: Some(_),
                }) = host.entity_by_id(*id)?
                {
                    host.set_dialog(handle, *dialog)?;
                }
            }
            EntityCommandKind::SetPosition {
                id,
                facing_direction,
                area,
            } => {
                let id = id.as_deref().ok_or(EntityCommandError::MissingId)?;
                let slot = ["player1", "player2", "player3", "player4"]
                    .iter()
                    .position(|p| *p == id);
                let entity = match slot {
                    Some(slot) => host.playing_avatar(slot)?,
                    None => host.entity_by_id(custom_hash(id))?,
                };
                let Some(entity) = entity else {
                    return Ok(());
                };
                let mut orientation = host.locator(*area)?;
                let mut position = [orientation[12], orientation[13], orientation[14]];
                let forward = [-orientation[8], -orientation[9], -orientation[10]];
                orientation[12..15].fill(0.0);
                let origin = [position[0], position[1] + 1.0, position[2]];
                if let Some(hit) = host.level_segment(origin, [0.0, -5.0, 0.0])? {
                    position = hit;
                    if let Some(height_offset) = entity.character {
                        position[1] -= height_offset;
                        if *facing_direction {
                            host.set_desired_direction(entity.handle, forward)?;
                        } else {
                            orientation = host.body_orientation(entity.handle)?;
                            orientation[12..15].fill(0.0);
                        }
                    }
                }
                host.move_body(entity.handle, position, orientation)?;
            }
            EntityCommandKind::Remove { target, force } => match *target {
                CharacterSelector::Id(id) => match host.entity_by_id(id)? {
                    Some(EntityFacts {
                        handle,
                        character: Some(_),
                    }) => {
                        if *force {
                            host.clear_cannot_die(handle)?;
                        }
                        host.kill(handle)?;
                        host.terminate(handle)?;
                    }
                    Some(e) => host.kill(e.handle)?,
                    None => {}
                },
                CharacterSelector::Area {
                    area,
                    type_id,
                    factions,
                } => {
                    for handle in Self::area_targets(host, area, type_id, factions)? {
                        host.kill(handle)?;
                        host.terminate(handle)?;
                    }
                }
            },
            EntityCommandKind::Kill { target } => match *target {
                CharacterSelector::Id(id) => {
                    if let Some(e) = host.entity_by_id(id)? {
                        if e.character.is_some() {
                            host.clear_cannot_die(e.handle)?;
                        }
                        host.kill(e.handle)?;
                    }
                }
                CharacterSelector::Area {
                    area,
                    type_id,
                    factions,
                } => {
                    for handle in Self::area_targets(host, area, type_id, factions)? {
                        host.clear_cannot_die(handle)?;
                        host.kill(handle)?;
                    }
                }
            },
            EntityCommandKind::ReleaseGrip { target } => match *target {
                CharacterSelector::Id(id) => {
                    if let Some(e) = host.entity_by_id(id)? {
                        host.release_npc_grip(e.handle)?;
                    }
                }
                CharacterSelector::Area {
                    area,
                    type_id,
                    factions,
                } => {
                    for handle in Self::area_targets(host, area, type_id, factions)? {
                        host.release_npc_grip(handle)?;
                    }
                }
            },
        }
        Ok(())
    }
}
