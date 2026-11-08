//! Shared scene script ownership. Unsupported definitions fail when reached.
//! World callbacks are synchronous and may reenter this same runtime.
use crate::{
    action_queue::ActionQueue,
    ai_event::{AiEvent, ResolveError},
    animation_action::AnimationAction,
    audio_action::{AudioActionError, AudioActionHost},
    avatar_move::{
        AvatarMoveAction, AvatarMoveError, AvatarMoveEvents, AvatarMoveHost, AvatarMoveLevel,
    },
    dialog_action::{DialogAction, DialogActionKind},
    dialog_state::{DialogError, DialogFacts},
    effect_action::EffectActionHost,
    entity_command::{EntityCommandError, EntityCommandHost},
    give_order::{GiveOrderError, GiveOrderHost},
    level_variables::LevelVariables,
    presence::{PresenceError, PresenceFacts},
    scene_command::{SceneCommandError, SceneCommandHost},
    scene_flow::{CheckpointLevel, SceneFlowError, SceneFlowHost},
    scene_loading::custom_hash,
    scene_script::{ActionDefinition, ConditionDefinition, SceneScriptLayout},
    script_condition::{ConditionError, ConditionFacts},
    spawn_action::{SpawnError, SpawnHost, SpawnLevel, SpawnState},
    spawn_phys::{SpawnPhysError, SpawnPhysHost, SpawnPhysLevel, SpawnPhysState},
    toggle_action::{ToggleError, ToggleHost},
    trigger::Trigger,
    trigger_control::TriggerControlKind,
    world_condition::{WorldConditionError, WorldConditionFacts},
};
use magicka_sim::damage_delivery::NetworkRole;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, Default)]
pub struct ScriptWorldFacts {
    pub cutscene: bool,
    pub player_count: i32,
}

pub trait ScriptWorld {
    type Error;
    fn dispenser_action(
        &mut self,
        _action: &crate::dispenser::DispenserAction,
    ) -> Option<Result<(), Self::Error>> {
        None
    }
    fn damage_action(
        &mut self,
        _action: &crate::script_damage::ScriptDamageAction,
    ) -> Option<Result<(), Self::Error>> {
        None
    }
    fn tutorial_action(
        &mut self,
        _action: &crate::tutorial::TutorialAction,
    ) -> Option<Result<(), Self::Error>> {
        None
    }
    /// PhysAnimation.Execute (QuickExecute repeats it).
    fn phys_animation_action(
        &mut self,
        _action: &crate::phys_animation::PhysAnimationAction,
    ) -> Option<Result<(), Self::Error>> {
        None
    }
    /// Campaign actions; `quick` is a QuickExecute replay.
    fn campaign_action(
        &mut self,
        _action: &crate::campaign_action::CampaignAction,
        _quick: bool,
    ) -> Option<Result<(), Self::Error>> {
        None
    }
    /// Boss-fight actions; `quick` is a QuickExecute replay.
    fn boss_action(
        &mut self,
        _action: &crate::boss_action::BossAction,
        _quick: bool,
    ) -> Option<Result<(), Self::Error>> {
        None
    }
    fn tutorial_done(&self) -> Option<bool> {
        None
    }
    fn facts(&self) -> ScriptWorldFacts;
    fn dialog_action(
        &mut self,
        _runtime: &mut ScriptRuntime,
        _action: &DialogAction,
        _sender: Option<u16>,
    ) -> Option<Result<(), Self::Error>> {
        None
    }
    /// Spawn capability. Missing capability fails when a Spawn executes.
    fn spawn_host(&mut self) -> Option<&mut dyn SpawnHost<Error = Self::Error>> {
        None
    }
    fn spawn_elemental_host(
        &mut self,
    ) -> Option<&mut dyn crate::spawn_elemental::SpawnElementalHost<Error = Self::Error>> {
        None
    }
    /// GiveOrder capability. Missing capability fails when a GiveOrder executes.
    fn give_order_host(&mut self) -> Option<&mut dyn GiveOrderHost<Error = Self::Error>> {
        None
    }
    /// Cutscene/input/fade/camera capability.
    fn scene_command_host(&mut self) -> Option<&mut dyn SceneCommandHost<Error = Self::Error>> {
        None
    }
    /// PlaySound/StopSound/PlayMusic/StopMusic capability.
    fn audio_host(&mut self) -> Option<&mut dyn AudioActionHost<Error = Self::Error>> {
        None
    }
    /// SpawnEffect/StopEffect/StartEffect capability.
    fn effect_host(&mut self) -> Option<&mut dyn EffectActionHost<Error = Self::Error>> {
        None
    }
    fn avatar_move_host(&mut self) -> Option<&mut dyn AvatarMoveHost<Error = Self::Error>> {
        None
    }
    fn spawn_magick_host(
        &mut self,
    ) -> Option<&mut dyn crate::magick_books::SpawnMagickHost<Error = Self::Error>> {
        None
    }
    fn spawn_item_host(
        &mut self,
    ) -> Option<&mut dyn crate::spawn_item::SpawnItemHost<Error = Self::Error>> {
        None
    }
    fn spawn_phys_host(&mut self) -> Option<&mut dyn SpawnPhysHost<Error = Self::Error>> {
        None
    }
    /// SetPosition/Remove/Kill capability.
    fn entity_command_host(&mut self) -> Option<&mut dyn EntityCommandHost<Error = Self::Error>> {
        None
    }
    /// Light/POI/tutorial/hint/achievement capability.
    fn toggle_host(&mut self) -> Option<&mut dyn ToggleHost<Error = Self::Error>> {
        None
    }
    /// Character/player/boss/achievement/DLC condition facts.
    fn condition_facts(&self) -> Option<&dyn WorldConditionFacts> {
        None
    }
    /// ChangeScene/EnterScene/TriggerCheckpoint/FoundSecretArea capability.
    fn scene_flow_host(&mut self) -> Option<&mut dyn SceneFlowHost<Error = Self::Error>> {
        None
    }
    fn presence(&self) -> Option<PresenceFacts<'_>> {
        None
    }
    fn dialogs(&self) -> Option<DialogFacts<'_>> {
        None
    }
    fn choose_group(&mut self, count: usize) -> Result<usize, Self::Error>;
    fn notify_trigger(
        &mut self,
        runtime: &mut ScriptRuntime,
        id: i32,
        group: usize,
        sender: Option<u16>,
    ) -> Result<(), Self::Error>;
    fn animation(
        &mut self,
        runtime: &mut ScriptRuntime,
        action: &AnimationAction,
    ) -> Result<(), Self::Error>;
    /// Successful Execute has already appended this index to action history.
    fn action_executed(
        &mut self,
        runtime: &mut ScriptRuntime,
        index: usize,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScriptRuntimeError<E> {
    MissingTrigger(i32),
    InvalidGroup(usize),
    InvalidAction(usize),
    UnsupportedCondition(usize),
    UnsupportedAction(usize),
    Condition(ConditionError),
    Presence(PresenceError),
    Dialog(DialogError),
    WorldCondition(usize, WorldConditionError),
    MissingSender(usize),
    MissingDialogActionHost(usize),
    MissingTutorialHost(usize),
    MissingCampaignHost(usize),
    MissingBossHost(usize),
    MissingDispenserHost(usize),
    MissingDamageHost(usize),
    MissingSpawnHost(usize),
    MissingSpawnElementalHost(usize),
    MissingGiveOrderHost(usize),
    MissingSceneCommandHost(usize),
    MissingAudioHost(usize),
    MissingEffectHost(usize),
    MissingAvatarMoveHost(usize),
    MissingEntityCommandHost(usize),
    MissingToggleHost(usize),
    MissingSceneFlowHost(usize),
    MissingPhysAnimationHost(usize),
    SceneFlow(usize, SceneFlowError<E>),
    CheckpointInitialize(usize, E),
    Toggle(usize, ToggleError<E>),
    EntityCommand(usize, EntityCommandError<E>),
    MissingSpawnPhysHost(usize),
    AvatarMoveInitialize(usize, AvatarMoveError<E>),
    SpawnPhysInitialize(usize, E),
    SpawnPhys(usize, SpawnPhysError<E>),
    NotInitialized(usize),
    NotEffect(usize),
    Audio(usize, AudioActionError<E>),
    NotSceneCommand(usize),
    SceneCommand(usize, SceneCommandError<E>),
    NotSpawn(usize),
    NotGiveOrder(usize),
    Spawn(usize, SpawnError<E>),
    GiveOrder(usize, GiveOrderError<E>),
    ActionInitialize(usize, ResolveError<E>),
    SpawnItem(usize, crate::spawn_item::SpawnItemError<E>),
    MissingSpawnItemHost(usize),
    MissingSpawnMagickHost(usize),
    World(E),
}

/// The level capabilities Action.Initialize needs (Spawn, AvatarMove,
/// SpawnPhys, checkpoints, GiveOrder, SpawnItem, dispensers).
pub trait ScriptLevel: SpawnLevel + AvatarMoveLevel + SpawnPhysLevel + CheckpointLevel {}
impl<T: SpawnLevel + AvatarMoveLevel + SpawnPhysLevel + CheckpointLevel + ?Sized> ScriptLevel
    for T
{
}

/// Name of the hidden, non-autorun trigger that owns actions defined through
/// [`ScriptRuntime::define_action`], so that the scene update ticks their
/// delay queues like any authored action.
pub const HOST_CALL_TRIGGER: &str = "#hostcalls";

#[derive(Debug, PartialEq, Eq)]
pub enum HostCallError<E> {
    Call(crate::script_schema::CallError),
    Layout(crate::scene_script::LayoutError),
    Runtime(ScriptRuntimeError<E>),
}
impl<E> From<ScriptRuntimeError<E>> for HostCallError<E> {
    fn from(e: ScriptRuntimeError<E>) -> Self {
        Self::Runtime(e)
    }
}

#[derive(Clone)]
pub struct ScriptRuntime {
    pub variables: LevelVariables,
    layout: SceneScriptLayout,
    queues: Vec<Option<ActionQueue>>,
    wave_triggers: BTreeSet<i32>,
    history: Vec<usize>,
    arguments: Vec<VecDeque<Option<u16>>>,
    spawns: Vec<SpawnState>,
    elementals: Vec<crate::spawn_elemental::SpawnElementalState>,
    order_events: Vec<Option<Vec<AiEvent>>>,
    avatar_events: Vec<Option<AvatarMoveEvents>>,
    phys: Vec<SpawnPhysState>,
    items: Vec<crate::spawn_item::SpawnItemState>,
    checkpoints: Vec<Option<[[f32; 16]; 4]>>,
    /// Live-game mode: actions without a reconstruction are skipped instead of
    /// failing the scene. Each skipped execution is recorded in `skipped`.
    pub skip_unsupported: bool,
    pub skipped: Vec<usize>,
}
mod disk_state;

impl ScriptRuntime {
    fn queue_for(a: &ActionDefinition) -> Option<ActionQueue> {
        match a {
            ActionDefinition::Animation(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Variable(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Control(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Dialog(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Tutorial(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Campaign(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Boss(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Dispenser(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Damage(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Spawn(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::SpawnElemental(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::GiveOrder(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Scene(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Audio(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Effect(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::AvatarMove(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::SpawnPhys(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::SpawnItem(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::SpawnMagick(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Entity(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Toggle(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Flow(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::PhysAnimation(a) => Some(ActionQueue::new(a.delay)),
            ActionDefinition::Unsupported(_) => None,
        }
    }
    pub fn new(mut layout: SceneScriptLayout, variables: LevelVariables) -> Self {
        // GameScene stores triggers in SortedList<int, Trigger>.
        layout.triggers.sort_by_key(|t| t.id);
        let queues = layout.actions.iter().map(Self::queue_for).collect();
        let arguments = vec![VecDeque::new(); layout.actions.len()];
        let spawns = vec![SpawnState::default(); layout.actions.len()];
        let wave_triggers = layout
            .ruleset
            .iter()
            .flat_map(|r| &r.waves)
            .flatten()
            .map(|a| a.trigger)
            .collect();
        Self {
            wave_triggers,
            variables,
            layout,
            queues,
            history: vec![],
            arguments,
            order_events: vec![None; spawns.len()],
            avatar_events: vec![None; spawns.len()],
            phys: vec![SpawnPhysState::default(); spawns.len()],
            elementals: vec![crate::spawn_elemental::SpawnElementalState::default(); spawns.len()],
            items: vec![crate::spawn_item::SpawnItemState::default(); spawns.len()],
            checkpoints: vec![None; spawns.len()],
            skip_unsupported: false,
            skipped: Vec::new(),
            spawns,
        }
    }
    /// Trigger.Initialize for every trigger in sorted ID order, over the final
    /// Then's groups. Common instances shared by random groups initialize once
    /// per occurrence; actions from replaced Then elements are not initialized.
    /// Spawn, GiveOrder, AvatarMove and SpawnPhys override the empty base
    /// Initialize among supported actions.
    pub fn initialize_actions<
        L: SpawnLevel + AvatarMoveLevel + SpawnPhysLevel + CheckpointLevel + ?Sized,
    >(
        &mut self,
        level: &mut L,
    ) -> Result<(), ScriptRuntimeError<L::Error>> {
        let ids: Vec<_> = self
            .layout
            .triggers
            .iter()
            .filter(|t| !self.wave_triggers.contains(&t.id))
            .map(|t| t.id)
            .collect();
        self.initialize_action_groups(level, &ids)
    }

    /// WaveActions.Initialize follows authored wave/action order, after normal
    /// Trigger.Initialize and the host's wave area binding.
    pub fn initialize_wave_actions<
        L: SpawnLevel + AvatarMoveLevel + SpawnPhysLevel + CheckpointLevel + ?Sized,
    >(
        &mut self,
        level: &mut L,
    ) -> Result<(), ScriptRuntimeError<L::Error>> {
        let ids: Vec<_> = self
            .layout
            .ruleset
            .iter()
            .flat_map(|r| &r.waves)
            .flatten()
            .map(|a| a.trigger)
            .collect();
        self.initialize_action_groups(level, &ids)
    }

    fn initialize_action_groups<
        L: SpawnLevel + AvatarMoveLevel + SpawnPhysLevel + CheckpointLevel + ?Sized,
    >(
        &mut self,
        level: &mut L,
        ids: &[i32],
    ) -> Result<(), ScriptRuntimeError<L::Error>> {
        for id in ids {
            let trigger = self.trigger(*id).expect("owned action group").clone();

            for group in &trigger.actions {
                for &index in group {
                    self.initialize_action(index, level)?;
                }
            }
        }
        Ok(())
    }

    /// Action.Initialize for one pool index (empty for most actions).
    fn initialize_action<L: ScriptLevel + ?Sized>(
        &mut self,
        index: usize,
        level: &mut L,
    ) -> Result<(), ScriptRuntimeError<L::Error>> {
        match &self.layout.actions[index] {
            ActionDefinition::Dispenser(a) => a
                .initialize(level)
                .map_err(|e| ScriptRuntimeError::ActionInitialize(index, e))?,
            ActionDefinition::Spawn(a) => a
                .initialize(&mut self.spawns[index], level)
                .map_err(|e| ScriptRuntimeError::ActionInitialize(index, e))?,
            ActionDefinition::AvatarMove(a) => {
                self.avatar_events[index] = Some(
                    a.initialize(level)
                        .map_err(|e| ScriptRuntimeError::AvatarMoveInitialize(index, e))?,
                )
            }
            ActionDefinition::SpawnItem(a) => level
                .initialize_spawn_item(a, &mut self.items[index])
                .ok_or(ScriptRuntimeError::MissingSpawnItemHost(index))?
                .map_err(|e| ScriptRuntimeError::SpawnItem(index, e))?,
            ActionDefinition::SpawnPhys(a) => a
                .initialize(&mut self.phys[index], level)
                .map_err(|e| ScriptRuntimeError::SpawnPhysInitialize(index, e))?,
            ActionDefinition::Flow(a) => {
                if let Some(m) = a
                    .initialize(level)
                    .map_err(|e| ScriptRuntimeError::CheckpointInitialize(index, e))?
                {
                    self.checkpoints[index] = Some(m);
                }
            }
            ActionDefinition::GiveOrder(a) => {
                self.order_events[index] = Some(
                    a.initialize(level)
                        .map_err(|e| ScriptRuntimeError::ActionInitialize(index, e))?,
                )
            }
            _ => {}
        }
        Ok(())
    }
    // ---- Host-language calls -------------------------------------------
    //
    // Native actions and conditions are a class name plus reflected
    // properties (see script_schema). These entry points let another
    // scripting layer define and run them through exactly the paths authored
    // triggers use: definitions join the shared pools, actions are owned by
    // a hidden trigger so their delay queues tick in `update`, and execution
    // appends history and notifies the world as usual.

    /// Grow the per-action state to cover newly appended definitions.
    fn grow_action_state(&mut self) {
        let n = self.layout.actions.len();
        for i in self.queues.len()..n {
            self.queues.push(Self::queue_for(&self.layout.actions[i]));
        }
        self.arguments.resize(n, VecDeque::new());
        self.spawns.resize(n, SpawnState::default());
        self.order_events.resize(n, None);
        self.avatar_events.resize(n, None);
        self.phys.resize(n, SpawnPhysState::default());
        self.elementals
            .resize(n, crate::spawn_elemental::SpawnElementalState::default());
        self.items
            .resize(n, crate::spawn_item::SpawnItemState::default());
        self.checkpoints.resize(n, None);
    }

    fn host_trigger_index(&mut self) -> usize {
        let id = custom_hash(HOST_CALL_TRIGGER);
        if let Some(i) = self.layout.triggers.iter().position(|t| t.id == id) {
            return i;
        }
        let mut trigger = Trigger::new(id, vec![], vec![vec![]]);
        trigger.name = HOST_CALL_TRIGGER.into();
        trigger.autorun = false;
        trigger.repeat = 0.0;
        self.insert_trigger(trigger)
    }

    /// Keep GameScene's SortedList order when adding a trigger.
    fn insert_trigger(&mut self, trigger: Trigger) -> usize {
        let at = self.layout.triggers.partition_point(|t| t.id < trigger.id);
        self.layout.triggers.insert(at, trigger);
        at
    }

    /// Define (and Initialize) an action from a host call. The returned pool
    /// index can be invoked any number of times, like a native instance.
    pub fn define_action<L: ScriptLevel + ?Sized>(
        &mut self,
        call: &crate::script_schema::ScriptCall,
        level: &mut L,
    ) -> Result<usize, HostCallError<L::Error>> {
        let element = call
            .element(crate::script_schema::Family::Action)
            .map_err(HostCallError::Call)?;
        let owner = custom_hash(HOST_CALL_TRIGGER);
        let index = self
            .layout
            .define_action(&element, owner)
            .map_err(HostCallError::Layout)?;
        self.grow_action_state();
        let t = self.host_trigger_index();
        self.layout.triggers[t].actions[0].push(index);
        self.initialize_action(index, level)?;
        Ok(index)
    }

    /// Action.OnTrigger with `sender`, then run whatever is due now (a zero
    /// Delay executes immediately). Returns the executions performed now;
    /// delayed ones run in later `update` calls.
    pub fn invoke_action<W: ScriptWorld>(
        &mut self,
        index: usize,
        sender: Option<u16>,
        role: NetworkRole,
        world: &mut W,
    ) -> Result<usize, ScriptRuntimeError<W::Error>> {
        self.trigger_action_with_sender(index, sender)?;
        self.update_action(index, 0.0, role, world)
    }

    /// Define and invoke a one-off action. Prefer define_action +
    /// invoke_action for repeated calls: every definition stays in the pool.
    pub fn call_action<E, W>(
        &mut self,
        call: &crate::script_schema::ScriptCall,
        sender: Option<u16>,
        role: NetworkRole,
        world: &mut W,
    ) -> Result<usize, HostCallError<E>>
    where
        W: ScriptWorld<Error = E> + ScriptLevel<Error = E>,
    {
        let index = self.define_action(call, world)?;
        Ok(self.invoke_action(index, sender, role, world)?)
    }

    /// Define a condition from a host call; evaluate it with `condition_met`.
    pub fn define_condition<E>(
        &mut self,
        call: &crate::script_schema::ScriptCall,
    ) -> Result<usize, HostCallError<E>> {
        let element = call
            .element(crate::script_schema::Family::Condition)
            .map_err(HostCallError::Call)?;
        self.layout
            .define_condition(&element)
            .map_err(HostCallError::Layout)
    }

    /// Add a whole `<Trigger>` (If/Then children, attributes as authored)
    /// at runtime, Initialize its actions and return its ID hash. It then
    /// autoruns, repeats and can be executed like a scene trigger.
    pub fn add_trigger<L: ScriptLevel + ?Sized>(
        &mut self,
        element: &crate::scene_script::XmlElement,
        looped: bool,
        level: &mut L,
    ) -> Result<i32, HostCallError<L::Error>> {
        let trigger = self
            .layout
            .define_trigger(element, looped)
            .map_err(HostCallError::Layout)?;
        if self.trigger(trigger.id).is_some() {
            return Err(HostCallError::Layout(
                crate::scene_script::LayoutError::DuplicateTrigger(trigger.id),
            ));
        }
        self.grow_action_state();
        let id = trigger.id;
        let actions: Vec<usize> = trigger.actions.iter().flatten().copied().collect();
        self.insert_trigger(trigger);
        for index in actions {
            self.initialize_action(index, level)?;
        }
        Ok(id)
    }
    pub fn spawn_state(&self, index: usize) -> Option<&SpawnState> {
        matches!(
            self.layout.actions.get(index),
            Some(ActionDefinition::Spawn(_))
        )
        .then(|| &self.spawns[index])
    }
    /// Save/load Tag replacement (see spawn_action::read_tag).
    pub fn spawn_state_mut(&mut self, index: usize) -> Option<&mut SpawnState> {
        matches!(
            self.layout.actions.get(index),
            Some(ActionDefinition::Spawn(_))
        )
        .then(|| &mut self.spawns[index])
    }
    /// QuickExecute for cutscene/input/fade/camera commands; no history.
    pub fn quick_execute_scene_command<W: ScriptWorld>(
        &mut self,
        index: usize,
        world: &mut W,
    ) -> Result<(), ScriptRuntimeError<W::Error>> {
        let ActionDefinition::Scene(a) = self
            .layout
            .actions
            .get(index)
            .ok_or(ScriptRuntimeError::InvalidAction(index))?
        else {
            return Err(ScriptRuntimeError::NotSceneCommand(index));
        };
        let host = world
            .scene_command_host()
            .ok_or(ScriptRuntimeError::MissingSceneCommandHost(index))?;
        a.quick_execute(host)
            .map_err(|e| ScriptRuntimeError::SceneCommand(index, e))
    }
    /// Action.QuickExecute for any supported definition, as used when a saved
    /// scene replays executed actions. No history is appended and queues are
    /// untouched. Empty native overrides return Ok without host access.
    pub fn quick_execute<W: ScriptWorld>(
        &mut self,
        index: usize,
        world: &mut W,
    ) -> Result<(), ScriptRuntimeError<W::Error>> {
        let definition = self
            .layout
            .actions
            .get(index)
            .ok_or(ScriptRuntimeError::InvalidAction(index))?
            .clone();
        match definition {
            ActionDefinition::Variable(a) => a.action.quick_execute(&mut self.variables),
            // PlayAnimation/Stop/Resume, ExecuteTrigger/ClearDelayedActions,
            // all audio, AvatarMove and SpawnPhys QuickExecute bodies are empty.
            ActionDefinition::Animation(_)
            | ActionDefinition::Control(_)
            | ActionDefinition::Audio(_)
            | ActionDefinition::AvatarMove(_)
            | ActionDefinition::SpawnPhys(_)
            | ActionDefinition::SpawnItem(_)
            | ActionDefinition::SpawnMagick(_)
            | ActionDefinition::Flow(_) => {}
            // Tutorial hint QuickExecute overrides are empty, but
            // EnableDark/DisableDark's QuickExecute calls Execute.
            ActionDefinition::Tutorial(a) => {
                if matches!(a.command, crate::tutorial::TutorialCommand::Dark(_)) {
                    world
                        .tutorial_action(&a)
                        .ok_or(ScriptRuntimeError::MissingTutorialHost(index))?
                        .map_err(ScriptRuntimeError::World)?;
                }
            }
            ActionDefinition::PhysAnimation(a) => {
                world
                    .phys_animation_action(&a)
                    .ok_or(ScriptRuntimeError::MissingPhysAnimationHost(index))?
                    .map_err(ScriptRuntimeError::World)?;
            }
            ActionDefinition::Dispenser(a) => {
                world
                    .dispenser_action(&a)
                    .ok_or(ScriptRuntimeError::MissingDispenserHost(index))?
                    .map_err(ScriptRuntimeError::World)?;
            }
            ActionDefinition::Boss(a) => {
                if a.quick_executes() {
                    world
                        .boss_action(&a, true)
                        .ok_or(ScriptRuntimeError::MissingBossHost(index))?
                        .map_err(ScriptRuntimeError::World)?;
                }
            }
            ActionDefinition::Campaign(a) => {
                if a.quick_executes() {
                    world
                        .campaign_action(&a, true)
                        .ok_or(ScriptRuntimeError::MissingCampaignHost(index))?
                        .map_err(ScriptRuntimeError::World)?;
                }
            }
            ActionDefinition::Damage(a) => {
                world
                    .damage_action(&a)
                    .ok_or(ScriptRuntimeError::MissingDamageHost(index))?
                    .map_err(ScriptRuntimeError::World)?;
            }
            ActionDefinition::Dialog(a) => {
                // StartDialog.QuickExecute is empty; KillDialogs calls Execute.
                if matches!(a.kind, DialogActionKind::Kill) {
                    world
                        .dialog_action(self, &a, None)
                        .ok_or(ScriptRuntimeError::MissingDialogActionHost(index))?
                        .map_err(ScriptRuntimeError::World)?;
                }
            }
            ActionDefinition::Spawn(_) => self.quick_execute_spawn(index, world)?,
            ActionDefinition::SpawnElemental(a) => {
                let host = world
                    .spawn_elemental_host()
                    .ok_or(ScriptRuntimeError::MissingSpawnElementalHost(index))?;
                a.quick_execute(&mut self.elementals[index], host)
                    .map_err(ScriptRuntimeError::World)?;
            }
            ActionDefinition::GiveOrder(_) => self.quick_execute_give_order(index, world)?,
            ActionDefinition::Scene(_) => self.quick_execute_scene_command(index, world)?,
            ActionDefinition::Effect(_) => self.quick_execute_effect(index, world)?,
            ActionDefinition::Entity(a) => {
                let host = world
                    .entity_command_host()
                    .ok_or(ScriptRuntimeError::MissingEntityCommandHost(index))?;
                a.execute(host)
                    .map_err(|e| ScriptRuntimeError::EntityCommand(index, e))?;
            }
            ActionDefinition::Toggle(a) => {
                let host = world
                    .toggle_host()
                    .ok_or(ScriptRuntimeError::MissingToggleHost(index))?;
                a.quick_execute(host)
                    .map_err(|e| ScriptRuntimeError::Toggle(index, e))?;
            }
            ActionDefinition::Unsupported(_) => {
                if self.skip_unsupported {
                    self.skipped.push(index);
                } else {
                    return Err(ScriptRuntimeError::UnsupportedAction(index));
                }
            }
        }
        Ok(())
    }
    /// Effect QuickExecute; no history.
    pub fn quick_execute_effect<W: ScriptWorld>(
        &mut self,
        index: usize,
        world: &mut W,
    ) -> Result<(), ScriptRuntimeError<W::Error>> {
        let ActionDefinition::Effect(a) = self
            .layout
            .actions
            .get(index)
            .ok_or(ScriptRuntimeError::InvalidAction(index))?
        else {
            return Err(ScriptRuntimeError::NotEffect(index));
        };
        let host = world
            .effect_host()
            .ok_or(ScriptRuntimeError::MissingEffectHost(index))?;
        a.quick_execute(host).map_err(ScriptRuntimeError::World)
    }
    pub fn avatar_move_events(&self, index: usize) -> Option<&AvatarMoveEvents> {
        self.avatar_events.get(index)?.as_ref()
    }
    pub fn spawn_phys_state(&self, index: usize) -> Option<&SpawnPhysState> {
        matches!(
            self.layout.actions.get(index),
            Some(ActionDefinition::SpawnPhys(_))
        )
        .then(|| &self.phys[index])
    }
    /// Save/load Tag replacement (see spawn_phys::read_tag).
    pub fn spawn_phys_state_mut(&mut self, index: usize) -> Option<&mut SpawnPhysState> {
        matches!(
            self.layout.actions.get(index),
            Some(ActionDefinition::SpawnPhys(_))
        )
        .then(|| &mut self.phys[index])
    }
    /// Resolved GiveOrder events after Initialize.
    pub fn give_order_events(&self, index: usize) -> Option<&[AiEvent]> {
        self.order_events.get(index)?.as_deref()
    }
    /// GiveOrder.QuickExecute. Not part of base Update; no history is appended.
    pub fn quick_execute_give_order<W: ScriptWorld>(
        &mut self,
        index: usize,
        world: &mut W,
    ) -> Result<(), ScriptRuntimeError<W::Error>> {
        let ActionDefinition::GiveOrder(a) = self
            .layout
            .actions
            .get(index)
            .ok_or(ScriptRuntimeError::InvalidAction(index))?
        else {
            return Err(ScriptRuntimeError::NotGiveOrder(index));
        };
        let host = world
            .give_order_host()
            .ok_or(ScriptRuntimeError::MissingGiveOrderHost(index))?;
        a.quick_execute(self.order_events[index].as_deref(), host)
            .map_err(|e| ScriptRuntimeError::GiveOrder(index, e))
    }
    /// Spawn.QuickExecute. Not part of base Update; no history is appended.
    pub fn quick_execute_spawn<W: ScriptWorld>(
        &mut self,
        index: usize,
        world: &mut W,
    ) -> Result<(), ScriptRuntimeError<W::Error>> {
        let ActionDefinition::Spawn(a) = self
            .layout
            .actions
            .get(index)
            .ok_or(ScriptRuntimeError::InvalidAction(index))?
        else {
            return Err(ScriptRuntimeError::NotSpawn(index));
        };
        let host = world
            .spawn_host()
            .ok_or(ScriptRuntimeError::MissingSpawnHost(index))?;
        a.quick_execute(&mut self.spawns[index], host)
            .map_err(|e| ScriptRuntimeError::Spawn(index, e))
    }
    pub fn layout(&self) -> &SceneScriptLayout {
        &self.layout
    }
    pub fn history(&self) -> &[usize] {
        &self.history
    }
    /// Trigger.State.ResetState for the listed trigger IDs: not triggered,
    /// no repeat time, and their actions' delay/queue cleared (tags kept).
    pub fn reset_triggers(&mut self, ids: &[i32]) {
        for trigger in self
            .layout
            .triggers
            .iter_mut()
            .filter(|t| ids.contains(&t.id))
        {
            trigger.has_triggered = false;
            trigger.time_till_repeat = 0.0;
            for &index in trigger.actions.iter().flatten() {
                if let Some(Some(queue)) = self.queues.get_mut(index) {
                    queue.reset();
                }
            }
        }
    }
    /// Level scenes are persistent objects: a revisited GameScene keeps its
    /// Trigger.mHasTriggered flags and its actions' delayed queues, while
    /// GameScene.Initialize runs Trigger.ResetTimers (repeat timer to zero)
    /// on every trigger. `previous` is this scene's runtime from its last
    /// visit; level variables stay with the current runtime.
    pub fn restore_revisit(&mut self, previous: &ScriptRuntime) {
        if previous.layout.triggers.len() != self.layout.triggers.len()
            || previous.queues.len() != self.queues.len()
        {
            return;
        }
        for (trigger, old) in self
            .layout
            .triggers
            .iter_mut()
            .zip(&previous.layout.triggers)
        {
            if trigger.id == old.id {
                trigger.has_triggered = old.has_triggered;
            }
            trigger.time_till_repeat = 0.0;
        }
        self.queues.clone_from(&previous.queues);
        self.arguments.clone_from(&previous.arguments);
    }
    /// GameScene.State.ApplyState's startup actions: the executed-action
    /// history without actions owned by the ignored triggers.
    pub fn history_excluding(&self, ignored: &[i32]) -> Vec<usize> {
        let owner = |index: usize| {
            self.layout
                .triggers
                .iter()
                .find(|t| t.actions.iter().flatten().any(|&a| a == index))
                .map(|t| t.id)
        };
        self.history
            .iter()
            .copied()
            .filter(|&i| owner(i).is_none_or(|id| !ignored.contains(&id)))
            .collect()
    }
    /// XML element name of an unsupported action (for skip reports).
    pub fn unsupported_name(&self, index: usize) -> Option<&str> {
        match self.layout.actions.get(index)? {
            ActionDefinition::Unsupported(n) => Some(n.name.as_str()),
            _ => None,
        }
    }
    pub fn trigger(&self, id: i32) -> Option<&Trigger> {
        self.layout.triggers.iter().find(|t| t.id == id)
    }
    pub fn queue(&self, index: usize) -> Option<&ActionQueue> {
        self.queues.get(index).and_then(Option::as_ref)
    }
    pub fn queue_mut(&mut self, index: usize) -> Option<&mut ActionQueue> {
        self.queues.get_mut(index).and_then(Option::as_mut)
    }
    pub fn trigger_action<E>(&mut self, index: usize) -> Result<(), ScriptRuntimeError<E>> {
        self.trigger_action_with_sender(index, None)
    }
    pub fn trigger_action_with_sender<E>(
        &mut self,
        index: usize,
        sender: Option<u16>,
    ) -> Result<(), ScriptRuntimeError<E>> {
        match self.queues.get_mut(index) {
            Some(Some(queue)) => {
                queue.trigger();
                if matches!(self.layout.actions[index], ActionDefinition::Control(a) if a.kind == TriggerControlKind::Execute)
                    || matches!(self.layout.actions[index], ActionDefinition::Dialog(a) if matches!(a.kind, DialogActionKind::Start(_)))
                {
                    self.arguments[index].push_back(sender);
                }
                Ok(())
            }
            Some(None) if self.skip_unsupported => {
                self.skipped.push(index);
                Ok(())
            }
            Some(None) => Err(ScriptRuntimeError::UnsupportedAction(index)),
            None => Err(ScriptRuntimeError::InvalidAction(index)),
        }
    }
    /// GameScene.ExecuteTrigger's client guard, unlike direct Trigger.Execute.
    pub fn execute_scene_trigger<W: ScriptWorld>(
        &mut self,
        id: i32,
        sender: Option<u16>,
        ignore: bool,
        role: NetworkRole,
        world: &mut W,
    ) -> Result<bool, ScriptRuntimeError<W::Error>> {
        if role == NetworkRole::Client {
            return Ok(false);
        }
        self.execute_trigger(id, sender, ignore, role, world)
    }
    pub fn execute_trigger<W: ScriptWorld>(
        &mut self,
        id: i32,
        sender: Option<u16>,
        ignore: bool,
        role: NetworkRole,
        world: &mut W,
    ) -> Result<bool, ScriptRuntimeError<W::Error>> {
        let index = self
            .layout
            .triggers
            .iter()
            .position(|t| t.id == id)
            .ok_or(ScriptRuntimeError::MissingTrigger(id))?;
        let trigger = &self.layout.triggers[index];
        let epsilon = f32::from_bits(1);
        if !(ignore
            || !trigger.has_triggered
            || (trigger.repeat > -epsilon && trigger.time_till_repeat < epsilon))
        {
            return Ok(false);
        }
        let mut met = true;
        for group in &trigger.conditions {
            met = true;
            for &condition in group {
                met = self.condition_met(condition, sender, &*world)?;
                if !met {
                    break;
                }
            }
            if met {
                break;
            }
        }
        if !(met || ignore) {
            return Ok(false);
        }
        let trigger = &mut self.layout.triggers[index];
        trigger.has_triggered = true;
        trigger.time_till_repeat = trigger.repeat;
        let count = trigger.actions.len();
        let group = if count > 1 {
            world
                .choose_group(count)
                .map_err(ScriptRuntimeError::World)?
        } else {
            0
        };
        let actions = self.layout.triggers[index]
            .actions
            .get(group)
            .ok_or(ScriptRuntimeError::InvalidGroup(group))?
            .clone();
        if role == NetworkRole::Server {
            world
                .notify_trigger(self, id, group, sender)
                .map_err(ScriptRuntimeError::World)?;
        }
        for action in actions {
            self.trigger_action_with_sender(action, sender)?;
        }
        Ok(true)
    }
    /// Condition.IsMet for one pool index (Invert applied), as a trigger's
    /// If group evaluates it.
    pub fn condition_met<W: ScriptWorld + ?Sized>(
        &self,
        condition: usize,
        sender: Option<u16>,
        world: &W,
    ) -> Result<bool, ScriptRuntimeError<W::Error>> {
        match &self.layout.conditions[condition] {
            ConditionDefinition::TutorialDone { invert } => Ok(invert
                ^ world
                    .tutorial_done()
                    .ok_or(ScriptRuntimeError::UnsupportedCondition(condition))?),
            ConditionDefinition::Dialog(c) => c
                .is_met(
                    world
                        .dialogs()
                        .ok_or(ScriptRuntimeError::Dialog(DialogError::MissingWorldFacts))?,
                )
                .map_err(ScriptRuntimeError::Dialog),
            ConditionDefinition::World(c) => {
                let facts = world
                    .condition_facts()
                    .ok_or(ScriptRuntimeError::WorldCondition(
                        condition,
                        WorldConditionError::MissingWorldFacts,
                    ))?;
                c.is_met(facts, sender)
                    .map_err(|e| ScriptRuntimeError::WorldCondition(condition, e))
            }
            ConditionDefinition::Presence(c) => c
                .is_met(world.presence().ok_or(ScriptRuntimeError::Presence(
                    PresenceError::MissingWorldFacts,
                ))?)
                .map_err(ScriptRuntimeError::Presence),
            ConditionDefinition::Known(c) => {
                let facts = world.facts();
                let triggered: BTreeMap<_, _> = self
                    .layout
                    .triggers
                    .iter()
                    .map(|t| (t.id, t.has_triggered))
                    .collect();
                c.is_met(&ConditionFacts {
                    variables: &self.variables,
                    triggered: &triggered,
                    cutscene: facts.cutscene,
                    player_count: facts.player_count,
                })
                .map_err(ScriptRuntimeError::Condition)
            }
            ConditionDefinition::Unsupported(_) => {
                Err(ScriptRuntimeError::UnsupportedCondition(condition))
            }
        }
    }
    /// Scene trigger phase only. LevelVariables::update runs earlier in Level.Update.
    pub fn update<W: ScriptWorld>(
        &mut self,
        dt: f32,
        role: NetworkRole,
        world: &mut W,
    ) -> Result<(), ScriptRuntimeError<W::Error>> {
        for index in 0..self.layout.triggers.len() {
            if self.wave_triggers.contains(&self.layout.triggers[index].id) {
                continue;
            }
            let trigger = &mut self.layout.triggers[index];
            trigger.time_till_repeat -= dt;
            let (id, autorun) = (trigger.id, trigger.autorun);
            if autorun && role != NetworkRole::Client {
                self.execute_trigger(id, None, false, role, world)?;
            }
            let groups = self.layout.triggers[index].actions.clone();
            for group in groups {
                for action in group {
                    self.update_action(action, dt, role, world)?;
                }
            }
        }
        Ok(())
    }
    /// WaveActions.Update runs only the current wave's actions, after normal
    /// triggers. Action.OnTrigger does not evaluate a scene trigger predicate.
    pub fn update_wave_actions<W: ScriptWorld>(
        &mut self,
        ids: &[i32],
        fired: &[i32],
        dt: f32,
        role: NetworkRole,
        world: &mut W,
    ) -> Result<(), ScriptRuntimeError<W::Error>> {
        for id in ids {
            let groups = self.trigger(*id).expect("owned wave").actions.clone();
            if fired.contains(id) {
                for &i in groups.iter().flatten() {
                    self.trigger_action(i)?;
                }
            }
            for i in groups.into_iter().flatten() {
                self.update_action(i, dt, role, world)?;
            }
        }
        Ok(())
    }

    pub fn wave_actions_finished(&self, id: i32) -> bool {
        self.trigger(id).is_some_and(|t| {
            t.actions
                .iter()
                .flatten()
                .all(|&i| self.queue(i).is_some_and(ActionQueue::finished))
        })
    }

    pub fn update_action<W: ScriptWorld>(
        &mut self,
        index: usize,
        dt: f32,
        role: NetworkRole,
        world: &mut W,
    ) -> Result<usize, ScriptRuntimeError<W::Error>> {
        if index >= self.queues.len() {
            return Err(ScriptRuntimeError::InvalidAction(index));
        }
        if self.queues[index].is_none() {
            if self.skip_unsupported {
                return Ok(0);
            }
            return Err(ScriptRuntimeError::UnsupportedAction(index));
        }
        // SpawnPhys.Update refreshes damageable health before base Action.Update.
        if matches!(
            self.layout.actions[index],
            ActionDefinition::SpawnElemental(_)
        ) {
            let host = world
                .spawn_elemental_host()
                .ok_or(ScriptRuntimeError::MissingSpawnElementalHost(index))?;
            crate::spawn_elemental::SpawnElementalAction::track(&mut self.elementals[index], host);
        }
        if let ActionDefinition::SpawnPhys(action) = &self.layout.actions[index] {
            if self.phys[index]
                .entity
                .is_some_and(|e| e.damageable || e.animated)
            {
                let host = world
                    .spawn_phys_host()
                    .ok_or(ScriptRuntimeError::MissingSpawnPhysHost(index))?;
                action
                    .track(&mut self.phys[index], host)
                    .map_err(ScriptRuntimeError::World)?;
            }
        }
        // Spawn.Update refreshes tracked health before base Action.Update.
        if matches!(self.layout.actions[index], ActionDefinition::Spawn(_))
            && !self.spawns[index].spawned.is_empty()
        {
            let host = world
                .spawn_host()
                .ok_or(ScriptRuntimeError::MissingSpawnHost(index))?;
            crate::spawn_action::SpawnAction::track(&mut self.spawns[index], host)
                .map_err(|e| ScriptRuntimeError::Spawn(index, e))?;
        }
        let queue = self.queues[index].as_mut().unwrap();
        queue.state.countdown -= dt;
        let mut count = 0;
        loop {
            let queue = self.queues[index].as_mut().unwrap();
            if !(queue.state.countdown <= 0.0 && queue.state.pending > 0) {
                break;
            }
            queue.state.pending -= 1;
            queue.state.countdown += queue.delay;
            // Release the queue borrow before Execute: callbacks can requeue this
            // very instance and the loop must observe their mutations immediately.
            match self.layout.actions[index].clone() {
                ActionDefinition::Variable(a) => a.action.execute(&mut self.variables),
                ActionDefinition::Animation(a) => world
                    .animation(self, &a)
                    .map_err(ScriptRuntimeError::World)?,
                ActionDefinition::Control(a) => match a.kind {
                    TriggerControlKind::Execute => {
                        let sender = self.arguments[index]
                            .pop_front()
                            .ok_or(ScriptRuntimeError::MissingSender(index))?;
                        self.execute_scene_trigger(a.target, sender, false, role, world)?;
                    }
                    TriggerControlKind::ClearDelayed => {
                        if a.target == 0
                            || a.target == custom_hash("all")
                            || a.target == custom_hash("allbutthis")
                        {
                            let ids: Vec<_> = self
                                .layout
                                .triggers
                                .iter()
                                .filter(|t| {
                                    a.target != custom_hash("allbutthis") || t.id != a.owner
                                })
                                .map(|t| t.id)
                                .collect();
                            for id in ids {
                                self.clear_delayed(id)?;
                            }
                        } else {
                            self.clear_delayed(a.target)?;
                        }
                    }
                },
                ActionDefinition::Dialog(a) => {
                    let sender = if matches!(a.kind, DialogActionKind::Start(_)) {
                        self.arguments[index].pop_front().flatten()
                    } else {
                        None
                    };
                    world
                        .dialog_action(self, &a, sender)
                        .ok_or(ScriptRuntimeError::MissingDialogActionHost(index))?
                        .map_err(ScriptRuntimeError::World)?;
                }
                ActionDefinition::Tutorial(a) => {
                    world
                        .tutorial_action(&a)
                        .ok_or(ScriptRuntimeError::MissingTutorialHost(index))?
                        .map_err(ScriptRuntimeError::World)?;
                }
                ActionDefinition::Dispenser(a) => {
                    world
                        .dispenser_action(&a)
                        .ok_or(ScriptRuntimeError::MissingDispenserHost(index))?
                        .map_err(ScriptRuntimeError::World)?;
                }
                ActionDefinition::Boss(a) => {
                    world
                        .boss_action(&a, false)
                        .ok_or(ScriptRuntimeError::MissingBossHost(index))?
                        .map_err(ScriptRuntimeError::World)?;
                }
                ActionDefinition::Campaign(a) => {
                    world
                        .campaign_action(&a, false)
                        .ok_or(ScriptRuntimeError::MissingCampaignHost(index))?
                        .map_err(ScriptRuntimeError::World)?;
                }
                ActionDefinition::Spawn(a) => {
                    let host = world
                        .spawn_host()
                        .ok_or(ScriptRuntimeError::MissingSpawnHost(index))?;
                    a.execute(&mut self.spawns[index], host)
                        .map_err(|e| ScriptRuntimeError::Spawn(index, e))?;
                }
                ActionDefinition::SpawnElemental(a) => {
                    let host = world
                        .spawn_elemental_host()
                        .ok_or(ScriptRuntimeError::MissingSpawnElementalHost(index))?;
                    a.execute(&mut self.elementals[index], host)
                        .map_err(ScriptRuntimeError::World)?;
                }
                ActionDefinition::GiveOrder(a) => {
                    let host = world
                        .give_order_host()
                        .ok_or(ScriptRuntimeError::MissingGiveOrderHost(index))?;
                    a.execute(self.order_events[index].as_deref(), host)
                        .map_err(|e| ScriptRuntimeError::GiveOrder(index, e))?;
                }
                ActionDefinition::Scene(a) => {
                    let host = world
                        .scene_command_host()
                        .ok_or(ScriptRuntimeError::MissingSceneCommandHost(index))?;
                    a.execute(host)
                        .map_err(|e| ScriptRuntimeError::SceneCommand(index, e))?;
                }
                // All four audio QuickExecute overrides are empty.
                ActionDefinition::Audio(a) => {
                    let host = world
                        .audio_host()
                        .ok_or(ScriptRuntimeError::MissingAudioHost(index))?;
                    a.execute(host)
                        .map_err(|e| ScriptRuntimeError::Audio(index, e))?;
                }
                ActionDefinition::Effect(a) => {
                    let host = world
                        .effect_host()
                        .ok_or(ScriptRuntimeError::MissingEffectHost(index))?;
                    a.execute(host).map_err(ScriptRuntimeError::World)?;
                }
                ActionDefinition::AvatarMove(_) => {
                    let events = self.avatar_events[index]
                        .as_ref()
                        .ok_or(ScriptRuntimeError::NotInitialized(index))?;
                    let host = world
                        .avatar_move_host()
                        .ok_or(ScriptRuntimeError::MissingAvatarMoveHost(index))?;
                    AvatarMoveAction::execute(events, host).map_err(ScriptRuntimeError::World)?;
                }
                ActionDefinition::SpawnMagick(a) => {
                    let host = world
                        .spawn_magick_host()
                        .ok_or(ScriptRuntimeError::MissingSpawnMagickHost(index))?;
                    a.execute(host).map_err(ScriptRuntimeError::World)?;
                }
                ActionDefinition::SpawnItem(a) => {
                    let host = world
                        .spawn_item_host()
                        .ok_or(ScriptRuntimeError::MissingSpawnItemHost(index))?;
                    a.execute(&self.items[index], host)
                        .map_err(|e| ScriptRuntimeError::SpawnItem(index, e))?;
                }
                ActionDefinition::SpawnPhys(a) => {
                    let host = world
                        .spawn_phys_host()
                        .ok_or(ScriptRuntimeError::MissingSpawnPhysHost(index))?;
                    a.execute(&mut self.phys[index], host)
                        .map_err(|e| ScriptRuntimeError::SpawnPhys(index, e))?;
                }
                // QuickExecute of all three repeats Execute.
                ActionDefinition::Entity(a) => {
                    let host = world
                        .entity_command_host()
                        .ok_or(ScriptRuntimeError::MissingEntityCommandHost(index))?;
                    a.execute(host)
                        .map_err(|e| ScriptRuntimeError::EntityCommand(index, e))?;
                }
                ActionDefinition::Toggle(a) => {
                    let host = world
                        .toggle_host()
                        .ok_or(ScriptRuntimeError::MissingToggleHost(index))?;
                    a.execute(host)
                        .map_err(|e| ScriptRuntimeError::Toggle(index, e))?;
                }
                ActionDefinition::Damage(a) => {
                    world
                        .damage_action(&a)
                        .ok_or(ScriptRuntimeError::MissingDamageHost(index))?
                        .map_err(ScriptRuntimeError::World)?;
                }
                ActionDefinition::PhysAnimation(a) => {
                    world
                        .phys_animation_action(&a)
                        .ok_or(ScriptRuntimeError::MissingPhysAnimationHost(index))?
                        .map_err(ScriptRuntimeError::World)?;
                }
                ActionDefinition::Flow(mut a) => {
                    let host = world
                        .scene_flow_host()
                        .ok_or(ScriptRuntimeError::MissingSceneFlowHost(index))?;
                    let result = a.execute(self.checkpoints[index].as_ref(), host);
                    // EnterScene's SpawnPlayers mutation persists on the instance.
                    self.layout.actions[index] = ActionDefinition::Flow(a);
                    result.map_err(|e| ScriptRuntimeError::SceneFlow(index, e))?;
                }
                ActionDefinition::Unsupported(_) => unreachable!(),
            }
            // GameScene.ActionExecute: a Spawn moves to the end instead of
            // repeating.
            if matches!(self.layout.actions[index], ActionDefinition::Spawn(_)) {
                self.history.retain(|&i| i != index);
            }
            self.history.push(index);
            world
                .action_executed(self, index)
                .map_err(ScriptRuntimeError::World)?;
            count += 1;
        }
        Ok(count)
    }
    pub fn clear_delayed<E>(&mut self, id: i32) -> Result<(), ScriptRuntimeError<E>> {
        let groups = self
            .trigger(id)
            .ok_or(ScriptRuntimeError::MissingTrigger(id))?
            .actions
            .clone();
        for group in groups {
            for index in group {
                if let Some(queue) = self.queue_mut(index) {
                    queue.clear_delayed();
                } else if !self.skip_unsupported {
                    return Err(ScriptRuntimeError::UnsupportedAction(index));
                }
            }
        }
        Ok(())
    }
}
