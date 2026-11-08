use magicka_runtime::action_queue::*;
use std::cell::RefCell;
#[test]
fn additional_trigger_keeps_countdown_and_catchup_notifies_after_each_execution() {
    let mut queue = ActionQueue::new(0.25);
    queue.trigger();
    assert_eq!(
        queue.update::<()>(0.125, |_| Ok(()), |_| Ok(())).unwrap(),
        0
    );
    let countdown = queue.state.countdown;
    queue.trigger();
    queue.trigger();
    assert_eq!(queue.state.countdown, countdown);
    let order = RefCell::new(vec![]);
    assert_eq!(
        queue
            .update::<()>(
                0.625,
                |q| {
                    order.borrow_mut().push((true, q.state.pending));
                    Ok(())
                },
                |q| {
                    order.borrow_mut().push((false, q.state.pending));
                    Ok(())
                }
            )
            .unwrap(),
        3
    );
    assert_eq!(
        *order.borrow(),
        [
            (true, 2),
            (false, 2),
            (true, 1),
            (false, 1),
            (true, 0),
            (false, 0)
        ]
    );
    assert!(queue.finished());
}
#[test]
fn empty_queue_ticks_clear_preserves_countdown_and_trigger_restarts_finished_queue() {
    let mut queue = ActionQueue::new(1.0);
    queue.update::<()>(2.0, |_| panic!(), |_| panic!()).unwrap();
    assert_eq!(queue.state.countdown, -2.0);
    queue.trigger();
    assert_eq!(queue.state.countdown, 1.0);
    queue.clear_delayed();
    assert_eq!(queue.state.countdown, 1.0);
    assert!(queue.finished());
    queue.reset();
    assert_eq!(queue.state, ActionQueueState::default());
    assert_eq!(queue.delay, 1.0);
}
#[test]
fn zero_negative_and_nan_delays_keep_original_comparisons() {
    for delay in [0.0, -1.0] {
        let mut queue = ActionQueue::new(delay);
        for _ in 0..4 {
            queue.trigger();
        }
        assert_eq!(queue.update::<()>(0.0, |_| Ok(()), |_| Ok(())).unwrap(), 4);
    }
    let mut queue = ActionQueue::new(f32::NAN);
    queue.trigger();
    assert_eq!(
        queue.update::<()>(1.0, |_| panic!(), |_| panic!()).unwrap(),
        0
    );
    assert!(!queue.finished());
}
#[test]
fn execute_failure_advances_state_and_skips_notification_and_later_work() {
    let mut queue = ActionQueue::new(0.5);
    queue.trigger();
    queue.trigger();
    assert_eq!(
        queue.update(1.0, |_| Err("execute failed"), |_| panic!()),
        Err("execute failed")
    );
    assert_eq!(
        queue.state,
        ActionQueueState {
            countdown: 0.0,
            pending: 1
        }
    );
    assert_eq!(queue.update::<()>(0.0, |_| Ok(()), |_| Ok(())).unwrap(), 1);
}
#[test]
fn reentrant_trigger_on_last_execution_resets_timer_and_pending_overflow_wraps() {
    let mut queue = ActionQueue::new(1.0);
    queue.trigger();
    assert_eq!(
        queue
            .update::<()>(
                3.0,
                |q| {
                    q.trigger();
                    Ok(())
                },
                |_| Ok(())
            )
            .unwrap(),
        1
    );
    assert_eq!(
        queue.state,
        ActionQueueState {
            countdown: 1.0,
            pending: 1
        }
    );
    queue.state.pending = i32::MAX;
    queue.trigger();
    assert_eq!(queue.state.pending, i32::MIN);
    assert!(!queue.finished());
    queue.trigger();
    assert_eq!(queue.state.pending, 1);
}
