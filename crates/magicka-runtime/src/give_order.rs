//! Script GiveOrder action: attribute decoding, Initialize-time AIEvent
//! resolution and Exec/QuickExecute host operation order.
use crate::{
    ai_event::{animation, clr_enum, AiEvent, AiEventLevel, ResolveError},
    presence::factions,
    scene_loading::custom_hash,
    scene_script::XmlElement,
    script_attributes::{boolean, identifier, number, ScriptReadError},
    spawn_action::{SpawnOrder, ORDERS, REACT_TO},
};
use magicka_sim::damage_delivery::NetworkRole;

const ORDER_PANIC: u8 = 6;
/// Literal blend time passed to GoToAnimation and the network message.
pub const EVENT_ANIMATION_BLEND: f32 = 0.2;

#[derive(Clone, Debug)]
pub struct GiveOrderAction {
    pub delay: f32,
    pub order: u8,
    pub react_to: u8,
    pub reaction: u8,
    pub trigger: i32,
    pub animation: u16,
    pub idle_animation: u16,
    pub factions: i32,
    /// `Some` when ID was set to a non-empty string (mIsSpecific).
    pub id: Option<i32>,
    pub type_id: i32,
    pub area: i32,
    pub target_area: i32,
    pub wander_speed: f32,
    pub priority_target: i32,
    pub priority_ability: i32,
    pub explicit_faction: bool,
    pub node: XmlElement,
}

/// A TriggerArea.PresentCharacters entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderCandidate {
    pub handle: u16,
    pub npc: bool,
    pub type_id: i32,
    pub original_faction: i32,
}

/// AI.WanderSpeed, AI.TargetArea then AI.SetOrder.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderCommand<'a> {
    pub wander_speed: f32,
    pub target_area: i32,
    pub order: SpawnOrder<'a>,
}

pub trait GiveOrderHost {
    type Error;
    fn network_role(&self) -> NetworkRole;
    /// Entity.GetByID(id) as NonPlayerCharacter.
    fn npc_by_id(&mut self, id: i32) -> Result<Option<u16>, Self::Error>;
    /// GameScene.GetTriggerArea(area).PresentCharacters; `None` for a missing key.
    fn present_characters(&mut self, area: i32)
        -> Result<Option<Vec<OrderCandidate>>, Self::Error>;
    /// TriggerArea.UpdatePresent(EntityManager); `false` for a missing key.
    fn update_present(&mut self, area: i32) -> Result<bool, Self::Error>;
    fn release_attached(&mut self, npc: u16) -> Result<(), Self::Error>;
    fn is_gripped(&mut self, npc: u16) -> Result<bool, Self::Error>;
    /// npc.Gripper.ReleaseAttachedCharacter().
    fn release_gripper_attached(&mut self, npc: u16) -> Result<(), Self::Error>;
    fn apply_order(&mut self, npc: u16, command: &OrderCommand<'_>) -> Result<(), Self::Error>;
    fn set_special_idle(&mut self, npc: u16, animation: u16) -> Result<(), Self::Error>;
    fn go_to_animation(&mut self, npc: u16, animation: u16, blend: f32) -> Result<(), Self::Error>;
    fn order_panic(&mut self, npc: u16) -> Result<(), Self::Error>;
    /// CharacterActionMessage(EventAnimation, animation, blend, idle animation).
    fn send_event_animation(
        &mut self,
        npc: u16,
        animation: u16,
        blend: f32,
        idle_animation: u16,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum GiveOrderError<E> {
    NotInitialized,
    /// Dictionary indexer KeyNotFoundException in GetTriggerArea.
    MissingArea(i32),
    Host(E),
}
impl<E> From<E> for GiveOrderError<E> {
    fn from(e: E) -> Self {
        Self::Host(e)
    }
}

impl GiveOrderAction {
    pub fn read(node: &XmlElement) -> Result<Self, ScriptReadError> {
        let mut action = Self {
            delay: 0.0,
            order: 0,
            react_to: 0,
            reaction: 2,
            trigger: 0,
            animation: 0,
            idle_animation: 0,
            factions: 0,
            id: None,
            type_id: 0,
            area: 0,
            target_area: 0,
            wander_speed: 1.0,
            priority_target: 0,
            priority_ability: -1,
            explicit_faction: false,
            node: node.clone(),
        };
        for (key, value) in &node.attributes {
            let key = key.as_str();
            match key.to_ascii_lowercase().as_str() {
                "delay" => action.delay = number(value, key)?,
                "order" => action.order = clr_enum(value, ORDERS, key, 255)? as u8,
                "reactto" => action.react_to = clr_enum(value, REACT_TO, key, 255)? as u8,
                "reaction" => action.reaction = clr_enum(value, ORDERS, key, 255)? as u8,
                "trigger" => action.trigger = identifier(value)?,
                "animation" => action.animation = animation(value, key)?,
                "idleanimation" => action.idle_animation = animation(value, key)?,
                "factions" => action.factions = factions(value, key)?,
                "id" => {
                    action.id = if value.is_empty() {
                        None
                    } else {
                        Some(identifier(value)?)
                    }
                }
                "type" => action.type_id = identifier(value)?,
                "area" => action.area = identifier(value)?,
                "targetarea" => {
                    action.target_area = identifier(value)?;
                    if action.target_area == custom_hash("any") {
                        action.target_area = 0;
                    }
                }
                "speed" => action.wander_speed = number(value, key)?,
                "prioritytarget" => action.priority_target = identifier(value)?,
                "priorityability" => action.priority_ability = number(value, key)?,
                "explicit" => action.explicit_faction = boolean(value, key)?,
                _ => {}
            }
        }
        AiEvent::read_children(node)?;
        Ok(action)
    }

    /// Action.Initialize: rebuild child events with MoveEvent level lookups.
    pub fn initialize<L: AiEventLevel + ?Sized>(
        &self,
        level: &mut L,
    ) -> Result<Vec<AiEvent>, ResolveError<L::Error>> {
        self.node
            .children
            .iter()
            .map(|child| AiEvent::resolve(child, level))
            .collect()
    }

    fn matches(&self, c: &OrderCandidate) -> bool {
        c.npc
            && (self.type_id == custom_hash("any")
                || c.type_id == self.type_id
                || (!self.explicit_faction && c.original_faction & self.factions != 0)
                || (self.explicit_faction && c.original_faction == self.factions))
    }

    fn command<H: GiveOrderHost + ?Sized>(
        &self,
        host: &mut H,
        npc: u16,
        events: &[AiEvent],
        panic: bool,
    ) -> Result<(), H::Error> {
        host.release_attached(npc)?;
        if host.is_gripped(npc)? {
            host.release_gripper_attached(npc)?;
        }
        host.apply_order(
            npc,
            &OrderCommand {
                wander_speed: self.wander_speed,
                target_area: self.target_area,
                order: SpawnOrder {
                    order: self.order,
                    react_to: self.react_to,
                    reaction: self.reaction,
                    priority_target: self.priority_target,
                    priority_ability: self.priority_ability,
                    trigger: self.trigger,
                    events,
                },
            },
        )?;
        if self.idle_animation != 0 {
            host.set_special_idle(npc, self.idle_animation)?;
        }
        if self.animation != 0 {
            host.go_to_animation(npc, self.animation, EVENT_ANIMATION_BLEND)?;
        }
        // Only the specific-ID branch calls OrderPanic.
        if panic && self.order == ORDER_PANIC {
            host.order_panic(npc)?;
        }
        if (self.animation != 0 || self.idle_animation != 0)
            && host.network_role() != NetworkRole::Offline
        {
            host.send_event_animation(
                npc,
                self.animation,
                EVENT_ANIMATION_BLEND,
                self.idle_animation,
            )?;
        }
        Ok(())
    }

    /// Exec. There is no client guard; Execute and ExecuteByHandle both reach it.
    pub fn execute<H: GiveOrderHost + ?Sized>(
        &self,
        events: Option<&[AiEvent]>,
        host: &mut H,
    ) -> Result<(), GiveOrderError<H::Error>> {
        let events = events.ok_or(GiveOrderError::NotInitialized)?;
        if let Some(id) = self.id {
            if let Some(npc) = host.npc_by_id(id)? {
                self.command(host, npc, events, true)?;
            }
            return Ok(());
        }
        let candidates = host
            .present_characters(self.area)?
            .ok_or(GiveOrderError::MissingArea(self.area))?;
        for candidate in candidates.iter().filter(|c| self.matches(c)) {
            self.command(host, candidate.handle, events, false)?;
        }
        Ok(())
    }

    /// QuickExecute refreshes a nonzero area's presence, even for specific IDs.
    pub fn quick_execute<H: GiveOrderHost + ?Sized>(
        &self,
        events: Option<&[AiEvent]>,
        host: &mut H,
    ) -> Result<(), GiveOrderError<H::Error>> {
        if self.area != 0 && !host.update_present(self.area)? {
            return Err(GiveOrderError::MissingArea(self.area));
        }
        self.execute(events, host)
    }
}
