//! Base Trigger selection/update; world conditions, RNG and actions are hosts.
use magicka_sim::damage_delivery::NetworkRole;

pub trait TriggerHost {
    type Error;
    /// Condition.IsMet, including derived behavior and Invert.
    fn condition(&mut self, index: usize, sender: Option<u16>) -> Result<bool, Self::Error>;
    /// Original Random.Next(count) or a deterministic recorded choice.
    fn choose_group(&mut self, count: usize) -> Result<usize, Self::Error>;
    fn notify_server(
        &mut self,
        trigger: i32,
        group: usize,
        sender: Option<u16>,
    ) -> Result<(), Self::Error>;
    fn trigger_action(&mut self, index: usize, sender: Option<u16>) -> Result<(), Self::Error>;
    fn update_action(&mut self, index: usize, dt: f32) -> Result<(), Self::Error>;
    fn clear_action(&mut self, index: usize) -> Result<(), Self::Error>;
}
#[derive(Debug, PartialEq, Eq)]
pub enum TriggerError<E> {
    Host(E),
    InvalidGroup(usize),
}
#[derive(Clone, Debug)]
pub struct Trigger {
    pub id: i32,
    /// Authored `id` attribute (diagnostics only; `id` is its hash).
    pub name: String,
    pub repeat: f32,
    pub time_till_repeat: f32,
    pub has_triggered: bool,
    pub autorun: bool,
    /// OR across groups, short-circuit AND within each group.
    pub conditions: Vec<Vec<usize>>,
    /// Indices refer to shared action instances, including repeated common ones.
    pub actions: Vec<Vec<usize>>,
}
impl Trigger {
    /// Defaults from Trigger.Read; a complete scene loader supplies both arrays.
    pub fn new(id: i32, conditions: Vec<Vec<usize>>, actions: Vec<Vec<usize>>) -> Self {
        Self {
            id,
            name: String::new(),
            repeat: -1.0,
            time_till_repeat: 0.0,
            has_triggered: false,
            autorun: true,
            conditions,
            actions,
        }
    }
    pub fn execute<H: TriggerHost>(
        &mut self,
        sender: Option<u16>,
        ignore_conditions: bool,
        role: NetworkRole,
        host: &mut H,
    ) -> Result<bool, TriggerError<H::Error>> {
        let epsilon = f32::from_bits(1);
        if !(ignore_conditions
            || !self.has_triggered
            || (self.repeat > -epsilon && self.time_till_repeat < epsilon))
        {
            return Ok(false);
        }
        let mut met = true;
        // IgnoreConditions still evaluates conditions before overriding their
        // final result; side effects/exceptions must not be skipped.
        for group in &self.conditions {
            met = true;
            for &condition in group {
                if !host
                    .condition(condition, sender)
                    .map_err(TriggerError::Host)?
                {
                    met = false;
                    break;
                }
            }
            if met {
                break;
            }
        }
        if !(met || ignore_conditions) {
            return Ok(false);
        }
        self.has_triggered = true;
        self.time_till_repeat = self.repeat;
        let group = if self.actions.len() > 1 {
            host.choose_group(self.actions.len())
                .map_err(TriggerError::Host)?
        } else {
            0
        };
        let actions = self
            .actions
            .get(group)
            .ok_or(TriggerError::InvalidGroup(group))?;
        if role == NetworkRole::Server {
            host.notify_server(self.id, group, sender)
                .map_err(TriggerError::Host)?;
        }
        for &action in actions {
            host.trigger_action(action, sender)
                .map_err(TriggerError::Host)?;
        }
        Ok(true)
    }
    pub fn update<H: TriggerHost>(
        &mut self,
        dt: f32,
        role: NetworkRole,
        host: &mut H,
    ) -> Result<(), TriggerError<H::Error>> {
        self.time_till_repeat -= dt;
        if self.autorun && role != NetworkRole::Client {
            self.execute(None, false, role, host)?;
        }
        // Do not deduplicate common action instances: the original updates each
        // group in order, so a shared action can tick repeatedly in one frame.
        for group in &self.actions {
            for &action in group {
                host.update_action(action, dt).map_err(TriggerError::Host)?;
            }
        }
        Ok(())
    }
    pub fn reset_timer(&mut self) {
        self.time_till_repeat = 0.0;
    }
    pub fn clear_delayed<H: TriggerHost>(
        &self,
        host: &mut H,
    ) -> Result<(), TriggerError<H::Error>> {
        for group in &self.actions {
            for &action in group {
                host.clear_action(action).map_err(TriggerError::Host)?;
            }
        }
        Ok(())
    }
}
/// ReadActions appends common action instances after each random alternative.
pub fn compose_action_groups(
    mut alternatives: Vec<Vec<usize>>,
    common: &[usize],
) -> Vec<Vec<usize>> {
    if alternatives.is_empty() {
        alternatives.push(vec![]);
    }
    for group in &mut alternatives {
        group.extend_from_slice(common);
    }
    alternatives
}
