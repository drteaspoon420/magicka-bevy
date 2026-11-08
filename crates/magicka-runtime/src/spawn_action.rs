//! Script Spawn action: attribute decoding, Initialize-time AIEvent resolution,
//! Execute/QuickExecute host operation order and spawned-health bookkeeping.
use crate::{
    ai_event::{animation, clr_enum, AiEvent, AiEventLevel, ResolveError},
    scene_loading::custom_hash,
    scene_script::XmlElement,
    script_attributes::{boolean, identifier, number, ScriptReadError},
};
use magicka_sim::damage_delivery::NetworkRole;

pub const ORDERS: &[(&str, i32)] = &[
    ("none", 0),
    ("idle", 1),
    ("attack", 2),
    ("defend", 3),
    ("flee", 4),
    ("wander", 5),
    ("panic", 6),
];
pub const REACT_TO: &[(&str, i32)] = &[("none", 0), ("attack", 1), ("proximity", 2)];
const ANIMATION_IDLE: u16 = 1;
const ANIMATION_IDLE_AGG: u16 = 5;

#[derive(Clone, Debug)]
pub struct SpawnAction {
    pub delay: f32,
    /// Lowercased Type string; Initialize loads "Data/Characters/" + this.
    pub type_name: Option<String>,
    pub type_id: i32,
    pub area: i32,
    pub snap_to_nav_mesh: bool,
    pub amount: i32,
    pub unique_id: i32,
    pub dialog: i32,
    pub force_draw: bool,
    pub health: f32,
    pub order: u8,
    pub reaction: u8,
    pub react_to: u8,
    pub trigger: i32,
    pub animation: u16,
    pub spawn_animation: u16,
    pub idle_animation: u16,
    pub never_remove: bool,
    pub target_area: i32,
    pub wander_speed: f32,
    pub priority_target: i32,
    pub priority_ability: i32,
    pub mesh: i32,
    pub on_death: i32,
    pub on_damage: i32,
    pub force_camera: bool,
    pub force_nav_mesh: bool,
    pub cannot_die_without_explicit_kill: bool,
    /// The original action retains its XmlNode and builds events in Initialize.
    pub node: XmlElement,
}

/// HandleAndHealth. `None` is the ushort.MaxValue handle restored by ReadTag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnedHealth {
    pub handle: Option<u16>,
    pub health: f32,
}

/// Mutable per-instance state that the original keeps in Spawn fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpawnState {
    /// mEvents; `None` until Initialize.
    pub events: Option<Vec<AiEvent>>,
    pub spawned: Vec<SpawnedHealth>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NpcInstance {
    pub handle: u16,
    /// Cached instance is a GenericBoss.
    pub boss: bool,
}

/// Agent.SetOrder arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnOrder<'a> {
    pub order: u8,
    pub react_to: u8,
    pub reaction: u8,
    pub priority_target: i32,
    pub priority_ability: i32,
    pub trigger: i32,
    pub events: &'a [AiEvent],
}

/// Field assignments that Execute makes between Initialize and the animation
/// phase. Optional members are not assigned by QuickExecute.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnConfig<'a> {
    /// Execute assigns MaxHitPoints * Health here; QuickExecute assigns later.
    pub hit_points: Option<f32>,
    pub on_death: i32,
    pub on_damage: i32,
    pub wander_speed: Option<f32>,
    pub target_area: Option<i32>,
    pub order: SpawnOrder<'a>,
    pub dialog: i32,
    /// Locator with zero translation; DesiredDirection is its Forward.
    pub orientation: [f32; 16],
    pub desired_direction: [f32; 3],
    pub remove_after_death: bool,
    pub force_camera: Option<bool>,
    pub force_nav_mesh: Option<bool>,
    pub cannot_die_without_explicit_kill: Option<bool>,
}

/// TriggerActionMessage(SpawnNPC). The host reads live Position (and Color
/// for Execute) from the instance when sending.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnMessage {
    pub handle: u16,
    pub template: i32,
    pub id: i32,
    pub direction: [f32; 3],
    /// Execute-only Bool0/Arg/Color.
    pub force_draw: Option<bool>,
    pub mesh: Option<i32>,
    pub include_color: bool,
    pub dialog: i32,
    pub animation: u16,
    pub spawn_animation: u16,
    pub idle_animation: u16,
}

pub trait SpawnLevel: AiEventLevel {
    /// Optional item initialization capability for scene hosts.
    fn initialize_spawn_item(
        &mut self,
        _action: &crate::spawn_item::SpawnItemAction,
        _state: &mut crate::spawn_item::SpawnItemState,
    ) -> Option<Result<(), crate::spawn_item::SpawnItemError<Self::Error>>> {
        None
    }

    /// Content.Load<CharacterTemplate>("Data/Characters/" + Type); a missing
    /// Type concatenates null and the load fails natively.
    fn load_character_template(&mut self, name: Option<&str>) -> Result<(), Self::Error>;
}

pub trait SpawnHost {
    type Error;
    fn network_role(&self) -> NetworkRole;
    /// CharacterTemplate.GetCachedTemplate(type).MoveAbilities.
    fn template_move_abilities(&mut self, type_id: i32) -> Result<i32, Self::Error>;
    /// NonPlayerCharacter.GetInstance: pops the cache or constructs; never null.
    fn acquire_npc(&mut self) -> Result<NpcInstance, Self::Error>;
    /// GameScene.GetLocator(int, out Matrix), including "any", random trigger
    /// area placement and the missing-area exception.
    fn locator(&mut self, area: i32) -> Result<[f32; 16], Self::Error>;
    fn nearest_nav_position(
        &mut self,
        point: [f32; 3],
        move_abilities: i32,
    ) -> Result<[f32; 3], Self::Error>;
    /// NonPlayerCharacter.Initialize(template, mesh, position, uniqueID); returns MaxHitPoints.
    fn initialize_npc(
        &mut self,
        npc: u16,
        type_id: i32,
        mesh: i32,
        position: [f32; 3],
        unique_id: i32,
    ) -> Result<f32, Self::Error>;
    fn force_draw(&mut self, npc: u16) -> Result<(), Self::Error>;
    fn configure(&mut self, npc: u16, config: &SpawnConfig<'_>) -> Result<(), Self::Error>;
    fn force_animation(&mut self, npc: u16, animation: u16) -> Result<(), Self::Error>;
    /// Assign SpawnAnimation then ChangeState(RessurectionState.Instance).
    fn begin_resurrection(&mut self, npc: u16, animation: u16) -> Result<(), Self::Error>;
    fn set_special_idle(&mut self, npc: u16, animation: u16) -> Result<(), Self::Error>;
    fn in_resurrection_state(&mut self, npc: u16) -> Result<bool, Self::Error>;
    fn survival_ruleset(&self) -> bool;
    /// Faction = EVIL, then SurvivalRuleset.AddedCharacter(npc, false).
    fn survival_added(&mut self, npc: u16) -> Result<(), Self::Error>;
    fn add_entity(&mut self, npc: u16) -> Result<(), Self::Error>;
    fn set_hit_points(&mut self, npc: u16, hit_points: f32) -> Result<(), Self::Error>;
    /// Entity.GetFromHandle(handle) as Character; `None` when not a Character.
    fn hit_points(&mut self, handle: u16) -> Result<Option<f32>, Self::Error>;
    fn send_spawn(&mut self, message: &SpawnMessage) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum SpawnError<E> {
    NotInitialized,
    /// Update's `as Character` returned null; the original dereferences it.
    MissingCharacter(u16),
    Host(E),
}
impl<E> From<E> for SpawnError<E> {
    fn from(e: E) -> Self {
        Self::Host(e)
    }
}

impl SpawnAction {
    pub fn read(node: &XmlElement) -> Result<Self, ScriptReadError> {
        let mut action = Self {
            delay: 0.0,
            type_name: None,
            type_id: 0,
            area: 0,
            snap_to_nav_mesh: false,
            amount: 1,
            unique_id: 0,
            dialog: 0,
            force_draw: false,
            health: 1.0,
            order: 2,
            reaction: 2,
            react_to: 3,
            trigger: 0,
            animation: 0,
            spawn_animation: 0,
            idle_animation: 0,
            never_remove: false,
            target_area: 0,
            wander_speed: 1.0,
            priority_target: 0,
            priority_ability: -1,
            mesh: -1,
            on_death: 0,
            on_damage: 0,
            force_camera: false,
            force_nav_mesh: false,
            cannot_die_without_explicit_kill: false,
            node: node.clone(),
        };
        for (key, value) in &node.attributes {
            let key = key.as_str();
            match key.to_ascii_lowercase().as_str() {
                "delay" => action.delay = number(value, key)?,
                "area" => action.area = identifier(value)?,
                "snaptonavmesh" => action.snap_to_nav_mesh = boolean(value, key)?,
                "targetarea" => {
                    action.target_area = identifier(value)?;
                    if action.target_area == custom_hash("any") {
                        action.target_area = 0;
                    }
                }
                "speed" => action.wander_speed = number(value, key)?,
                "type" => {
                    action.type_id = identifier(value)?;
                    action.type_name = Some(value.to_ascii_lowercase());
                }
                "nr" => action.amount = number(value, key)?,
                "health" => {
                    let health: f32 = number(value, key)?;
                    // Setter assigns before its range check; NaN passes it.
                    if health > 1.0 || health < 0.0 {
                        return Err(ScriptReadError::InvalidAttribute(key.into()));
                    }
                    action.health = health;
                }
                "id" => action.unique_id = identifier(value)?,
                "dialog" => action.dialog = identifier(value)?,
                "order" => action.order = clr_enum(value, ORDERS, key, 255)? as u8,
                "reactto" => action.react_to = clr_enum(value, REACT_TO, key, 255)? as u8,
                "reaction" => action.reaction = clr_enum(value, ORDERS, key, 255)? as u8,
                "trigger" => action.trigger = identifier(value)?,
                "neverremove" => action.never_remove = boolean(value, key)?,
                "animation" => action.animation = animation(value, key)?,
                "idleanimation" => action.idle_animation = animation(value, key)?,
                "spawnanimation" => action.spawn_animation = animation(value, key)?,
                "prioritytarget" => action.priority_target = identifier(value)?,
                "priorityability" => action.priority_ability = number(value, key)?,
                "ondeath" => action.on_death = identifier(value)?,
                "ondamage" => action.on_damage = identifier(value)?,
                "forcedraw" => action.force_draw = boolean(value, key)?,
                "meshid" => action.mesh = number(value, key)?,
                "forcecamera" => action.force_camera = boolean(value, key)?,
                "forcenavmesh" => action.force_nav_mesh = boolean(value, key)?,
                "cannotdiewithoutexplicitkill" => {
                    action.cannot_die_without_explicit_kill = boolean(value, key)?
                }
                _ => {}
            }
        }
        // Validate event children at load; Initialize resolves level lookups.
        AiEvent::read_children(node)?;
        Ok(action)
    }

    /// Action.Initialize: load the template, then rebuild each child event,
    /// repeating MoveEvent's level lookups.
    pub fn initialize<L: SpawnLevel + ?Sized>(
        &self,
        state: &mut SpawnState,
        level: &mut L,
    ) -> Result<(), ResolveError<L::Error>> {
        level
            .load_character_template(self.type_name.as_deref())
            .map_err(ResolveError::Level)?;
        let events = self
            .node
            .children
            .iter()
            .map(|child| AiEvent::resolve(child, level))
            .collect::<Result<Vec<_>, _>>()?;
        state.events = Some(events);
        Ok(())
    }

    fn placement<H: SpawnHost + ?Sized>(
        &self,
        host: &mut H,
        move_abilities: i32,
    ) -> Result<([f32; 16], [f32; 3]), H::Error> {
        let mut locator = host.locator(self.area)?;
        if self.snap_to_nav_mesh {
            let point = [locator[12], locator[13], locator[14]];
            let p = host.nearest_nav_position(point, move_abilities)?;
            locator[12..15].copy_from_slice(&p);
        }
        let position = [locator[12], locator[13], locator[14]];
        let mut orientation = locator;
        orientation[12..15].fill(0.0);
        Ok((orientation, position))
    }

    fn animate<H: SpawnHost + ?Sized>(
        &self,
        host: &mut H,
        npc: u16,
        quick: bool,
    ) -> Result<(), H::Error> {
        if self.animation != 0 {
            host.force_animation(npc, self.animation)?;
        }
        if !matches!(
            self.spawn_animation,
            0 | ANIMATION_IDLE | ANIMATION_IDLE_AGG
        ) {
            host.begin_resurrection(npc, self.spawn_animation)?;
        }
        if self.idle_animation != 0 {
            host.set_special_idle(npc, self.idle_animation)?;
            if !quick && !host.in_resurrection_state(npc)? {
                host.force_animation(npc, self.idle_animation)?;
            }
        }
        if host.survival_ruleset() {
            host.survival_added(npc)?;
        }
        Ok(())
    }

    pub fn execute<H: SpawnHost + ?Sized>(
        &self,
        state: &mut SpawnState,
        host: &mut H,
    ) -> Result<(), SpawnError<H::Error>> {
        if host.network_role() == NetworkRole::Client {
            return Ok(());
        }
        let move_abilities = host.template_move_abilities(self.type_id)?;
        let events = state.events.as_ref().ok_or(SpawnError::NotInitialized)?;
        for _ in 0..self.amount {
            let mut npc = host.acquire_npc()?;
            while npc.boss {
                npc = host.acquire_npc()?;
            }
            let handle = npc.handle;
            let (orientation, position) = self.placement(host, move_abilities)?;
            let max_hit_points =
                host.initialize_npc(handle, self.type_id, self.mesh, position, self.unique_id)?;
            if self.force_draw {
                host.force_draw(handle)?;
            }
            let hit_points = max_hit_points * self.health;
            let direction = [-orientation[8], -orientation[9], -orientation[10]];
            host.configure(
                handle,
                &SpawnConfig {
                    hit_points: Some(hit_points),
                    on_death: self.on_death,
                    on_damage: self.on_damage,
                    wander_speed: Some(self.wander_speed),
                    target_area: Some(self.target_area),
                    order: self.order(events),
                    dialog: self.dialog,
                    orientation,
                    desired_direction: direction,
                    remove_after_death: !self.never_remove,
                    force_camera: Some(self.force_camera),
                    force_nav_mesh: Some(self.force_nav_mesh),
                    cannot_die_without_explicit_kill: Some(self.cannot_die_without_explicit_kill),
                },
            )?;
            self.animate(host, handle, false)?;
            host.add_entity(handle)?;
            let health = host
                .hit_points(handle)?
                .ok_or(SpawnError::MissingCharacter(handle))?;
            state.spawned.push(SpawnedHealth {
                handle: Some(handle),
                health,
            });
            if host.network_role() == NetworkRole::Server {
                host.send_spawn(&self.message(handle, direction, false))?;
            }
        }
        Ok(())
    }

    /// Respawn one instance per saved health record, restoring its health after
    /// AddEntity. Unlike Execute: no boss skip, ForceDraw, health scaling,
    /// wander/target/camera/navmesh/kill flags or special-idle animation.
    pub fn quick_execute<H: SpawnHost + ?Sized>(
        &self,
        state: &mut SpawnState,
        host: &mut H,
    ) -> Result<(), SpawnError<H::Error>> {
        if host.network_role() == NetworkRole::Client {
            return Ok(());
        }
        let move_abilities = host.template_move_abilities(self.type_id)?;
        let events = state.events.clone().ok_or(SpawnError::NotInitialized)?;
        for i in 0..state.spawned.len() {
            let handle = host.acquire_npc()?.handle;
            let (orientation, position) = self.placement(host, move_abilities)?;
            host.initialize_npc(handle, self.type_id, self.mesh, position, self.unique_id)?;
            let direction = [-orientation[8], -orientation[9], -orientation[10]];
            host.configure(
                handle,
                &SpawnConfig {
                    hit_points: None,
                    on_death: self.on_death,
                    on_damage: self.on_damage,
                    wander_speed: None,
                    target_area: None,
                    order: self.order(&events),
                    dialog: self.dialog,
                    orientation,
                    desired_direction: direction,
                    remove_after_death: !self.never_remove,
                    force_camera: None,
                    force_nav_mesh: None,
                    cannot_die_without_explicit_kill: None,
                },
            )?;
            self.animate(host, handle, true)?;
            host.add_entity(handle)?;
            host.set_hit_points(handle, state.spawned[i].health)?;
            state.spawned[i].handle = Some(handle);
            if host.network_role() == NetworkRole::Server {
                host.send_spawn(&self.message(handle, direction, true))?;
            }
        }
        Ok(())
    }

    /// Spawn.Update's bookkeeping, run before base Action.Update.
    pub fn track<H: SpawnHost + ?Sized>(
        state: &mut SpawnState,
        host: &mut H,
    ) -> Result<(), SpawnError<H::Error>> {
        let mut i = 0;
        while i < state.spawned.len() {
            if let Some(handle) = state.spawned[i].handle {
                let health = host
                    .hit_points(handle)?
                    .ok_or(SpawnError::MissingCharacter(handle))?;
                if health <= 0.0 {
                    state.spawned.remove(i);
                    continue;
                }
                state.spawned[i].health = health;
            }
            i += 1;
        }
        Ok(())
    }

    fn order<'a>(&self, events: &'a [AiEvent]) -> SpawnOrder<'a> {
        SpawnOrder {
            order: self.order,
            react_to: self.react_to,
            reaction: self.reaction,
            priority_target: self.priority_target,
            priority_ability: self.priority_ability,
            trigger: self.trigger,
            events,
        }
    }

    fn message(&self, handle: u16, direction: [f32; 3], quick: bool) -> SpawnMessage {
        SpawnMessage {
            handle,
            template: self.type_id,
            id: self.unique_id,
            direction,
            force_draw: (!quick).then_some(self.force_draw),
            mesh: (!quick).then_some(self.mesh),
            include_color: !quick,
            dialog: self.dialog,
            animation: self.animation,
            spawn_animation: self.spawn_animation,
            idle_animation: self.idle_animation,
        }
    }
}

/// WriteTag: count then each health; handles are not saved.
pub fn write_tag(spawned: &[SpawnedHealth]) -> Vec<u8> {
    let mut out = (spawned.len() as i32).to_le_bytes().to_vec();
    for s in spawned {
        out.extend(s.health.to_le_bytes());
    }
    out
}

/// ReadTag: restores health records with the ushort.MaxValue handle. A negative
/// count makes the original List capacity constructor throw.
pub fn read_tag(bytes: &[u8]) -> Option<(Vec<SpawnedHealth>, usize)> {
    let count = usize::try_from(i32::from_le_bytes(bytes.get(..4)?.try_into().ok()?)).ok()?;
    let mut out = Vec::new();
    let mut offset = 4;
    for _ in 0..count {
        let health = f32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?);
        out.push(SpawnedHealth {
            handle: None,
            health,
        });
        offset += 4;
    }
    Some((out, offset))
}
