use magicka_runtime::{action_queue::ActionQueue, trigger::*};
use magicka_sim::damage_delivery::NetworkRole;
#[derive(Debug, PartialEq)]
enum Event {
    Condition(usize),
    Choose(usize),
    Notify(i32, usize, Option<u16>),
    Trigger(usize),
    Update(usize),
    Clear(usize),
}
#[derive(Default)]
struct Host {
    events: Vec<Event>,
    conditions: Vec<bool>,
    choice: usize,
    queues: Vec<ActionQueue>,
}
impl TriggerHost for Host {
    type Error = &'static str;
    fn condition(&mut self, i: usize, _: Option<u16>) -> Result<bool, Self::Error> {
        self.events.push(Event::Condition(i));
        Ok(self.conditions[i])
    }
    fn choose_group(&mut self, n: usize) -> Result<usize, Self::Error> {
        self.events.push(Event::Choose(n));
        Ok(self.choice)
    }
    fn notify_server(&mut self, id: i32, group: usize, s: Option<u16>) -> Result<(), Self::Error> {
        self.events.push(Event::Notify(id, group, s));
        Ok(())
    }
    fn trigger_action(&mut self, i: usize, _: Option<u16>) -> Result<(), Self::Error> {
        self.events.push(Event::Trigger(i));
        if let Some(q) = self.queues.get_mut(i) {
            q.trigger();
        }
        Ok(())
    }
    fn update_action(&mut self, i: usize, dt: f32) -> Result<(), Self::Error> {
        self.events.push(Event::Update(i));
        if let Some(q) = self.queues.get_mut(i) {
            q.update::<()>(dt, |_| Ok(()), |_| Ok(())).unwrap();
        }
        Ok(())
    }
    fn clear_action(&mut self, i: usize) -> Result<(), Self::Error> {
        self.events.push(Event::Clear(i));
        if let Some(q) = self.queues.get_mut(i) {
            q.clear_delayed();
        }
        Ok(())
    }
}
#[test]
fn conditions_short_circuit_and_server_notification_precedes_selected_actions() {
    let mut trigger = Trigger::new(7, vec![vec![0, 1], vec![2, 3]], vec![vec![0], vec![1, 2]]);
    let mut host = Host {
        conditions: vec![false, true, true, true],
        choice: 1,
        ..Default::default()
    };
    assert!(trigger
        .execute(Some(9), false, NetworkRole::Server, &mut host)
        .unwrap());
    assert_eq!(
        host.events,
        [
            Event::Condition(0),
            Event::Condition(2),
            Event::Condition(3),
            Event::Choose(2),
            Event::Notify(7, 1, Some(9)),
            Event::Trigger(1),
            Event::Trigger(2)
        ]
    );
    assert!(!trigger
        .execute(None, false, NetworkRole::Offline, &mut host)
        .unwrap());
}
#[test]
fn ignored_conditions_still_evaluate_and_empty_condition_groups_are_true() {
    let mut trigger = Trigger::new(0, vec![vec![0]], vec![vec![]]);
    let mut host = Host {
        conditions: vec![false],
        ..Default::default()
    };
    assert!(!trigger
        .execute(None, false, NetworkRole::Offline, &mut host)
        .unwrap());
    assert!(!trigger.has_triggered);
    assert!(trigger
        .execute(None, true, NetworkRole::Offline, &mut host)
        .unwrap());
    assert_eq!(host.events, [Event::Condition(0), Event::Condition(0)]);
    let mut trigger = Trigger::new(0, vec![vec![], vec![0]], vec![vec![]]);
    host.events.clear();
    assert!(trigger
        .execute(None, false, NetworkRole::Offline, &mut host)
        .unwrap());
    assert!(host.events.is_empty());
}
#[test]
fn client_skips_autorun_but_updates_all_shared_actions_in_group_order() {
    let groups = compose_action_groups(vec![vec![0], vec![1]], &[2]);
    assert_eq!(groups, [vec![0, 2], vec![1, 2]]);
    let mut trigger = Trigger::new(0, vec![], groups);
    let mut host = Host {
        queues: vec![ActionQueue::new(1.0); 3],
        ..Default::default()
    };
    host.queues[2].trigger();
    trigger
        .update(0.25, NetworkRole::Client, &mut host)
        .unwrap();
    assert_eq!(host.queues[2].state.countdown, 0.5);
    assert!(!trigger.has_triggered);
    assert_eq!(
        host.events,
        [
            Event::Update(0),
            Event::Update(2),
            Event::Update(1),
            Event::Update(2)
        ]
    );
    trigger.clear_delayed(&mut host).unwrap();
    assert!(host.queues[2].finished());
}
#[test]
fn repeat_subnormal_boundaries_and_autorun_before_action_update_are_preserved() {
    let mut trigger = Trigger::new(0, vec![], vec![vec![0]]);
    trigger.repeat = 0.5;
    let mut host = Host::default();
    trigger
        .update(0.0, NetworkRole::Offline, &mut host)
        .unwrap();
    assert_eq!(host.events, [Event::Trigger(0), Event::Update(0)]);
    assert_eq!(trigger.time_till_repeat, 0.5);
    trigger
        .update(0.5, NetworkRole::Offline, &mut host)
        .unwrap();
    assert_eq!(
        host.events
            .iter()
            .filter(|e| matches!(e, Event::Trigger(0)))
            .count(),
        2
    );
    trigger.time_till_repeat = f32::from_bits(1);
    assert!(!trigger
        .execute(None, false, NetworkRole::Offline, &mut host)
        .unwrap());
    trigger.time_till_repeat = 0.0;
    trigger.repeat = -f32::from_bits(1);
    assert!(!trigger
        .execute(None, false, NetworkRole::Offline, &mut host)
        .unwrap());
    trigger.repeat = 0.0;
    assert!(trigger
        .execute(None, false, NetworkRole::Offline, &mut host)
        .unwrap());
}
#[test]
fn invalid_host_choice_keeps_preselection_trigger_mutations() {
    let mut trigger = Trigger::new(0, vec![], vec![vec![], vec![]]);
    trigger.repeat = 2.0;
    let mut host = Host {
        choice: 3,
        ..Default::default()
    };
    assert_eq!(
        trigger.execute(None, false, NetworkRole::Offline, &mut host),
        Err(TriggerError::InvalidGroup(3))
    );
    assert!(trigger.has_triggered);
    assert_eq!(trigger.time_till_repeat, 2.0);
    assert_eq!(compose_action_groups(vec![], &[0, 1]), [vec![0, 1]]);
}
