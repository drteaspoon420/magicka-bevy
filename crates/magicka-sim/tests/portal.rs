//! Portal numerics (research/portal.md).
use magicka_sim::portal::{EffectChange, Portal, PortalType};

#[test]
fn casts_swap_and_connect_within_thirty_metres() {
    let mut p = Portal::default();
    assert!(p.a.dead && p.b.dead);
    let first = p.place([0.0; 3], Some([0.0, 0.0, 0.0]));
    // The constructor makes A blue; the swap re-places B (orange) first.
    assert_eq!(first.kind, PortalType::Orange);
    assert_eq!(first.position, [0.0, 1.5, 0.0]);
    assert!(!first.connected);
    assert_eq!(
        p.update_connection(1.0),
        None,
        "one portal is not connected"
    );
    let second = p.place([20.0, 0.0, 0.0], None);
    assert_eq!(second.kind, PortalType::Blue);
    assert_eq!(
        second.position,
        [20.0, 0.0, 0.0],
        "a probe miss keeps the snap"
    );
    let health = p.update_connection(1.0).unwrap();
    assert!((health - (1.0 - 20.0f32.hypot(1.5) / 30.0)).abs() < 1e-6);
    // Magnification divides the squared range: 900 / 3 < 20.06².
    assert!(p.update_connection(2.0).is_some());
    assert_eq!(p.update_connection(3.0), None);
    assert!(p.update_connection(1.0).is_some());
    // A third cast re-places the oldest (orange) portal.
    let third = p.place([5.0, 0.0, 0.0], None);
    assert_eq!(third.kind, PortalType::Orange);
    assert!(third.connected, "Initialize reads the previous connection");
}

#[test]
fn snap_and_spawn_point() {
    assert_eq!(
        Portal::spawn_point([1.0, 0.0, 1.0], [0.0, 0.0, -1.0]),
        [1.0, 0.0, -1.0]
    );
    assert!(Portal::snap_accepted([0.0; 3], [3.0, 0.0, 4.0]));
    assert!(!Portal::snap_accepted([0.0; 3], [3.0, 0.1, 4.0]));
}

#[test]
fn touch_queues_once_while_contact_lasts() {
    let mut p = Portal::default();
    p.place([0.0; 3], None);
    p.place([10.0, 0.0, 0.0], None);
    assert!(!p.touch(5), "not connected before Portal.Update");
    p.update_connection(1.0);
    assert!(p.touch(5));
    assert!(!p.touch(5), "ignored for 0.1 s");
    for _ in 0..5 {
        p.update_entity(true, 1.0 / 60.0);
        assert!(!p.touch(5), "continued contact refreshes the entry");
    }
    for _ in 0..7 {
        p.update_entity(true, 1.0 / 60.0);
    }
    assert!(p.touch(5), "queued again after leaving for 0.1 s");
    // GetOutPos: the other portal plus 0.8 × the offset from the entry.
    assert_eq!(p.out_position(true, [10.5, 0.0, 1.0]), [0.4, 0.0, 0.8]);
}

#[test]
fn effects_follow_connection_and_kill_disconnects() {
    let mut p = Portal::default();
    p.place([0.0; 3], None);
    p.place([10.0, 0.0, 0.0], None);
    p.update_connection(1.0);
    assert_eq!(
        p.update_entity(true, 0.5),
        Some(EffectChange::Restart { connected: true })
    );
    assert_eq!(p.update_entity(true, 0.7), None);
    assert_eq!(p.a.effect_alpha, 1.0);
    p.kill();
    assert_eq!(p.update_connection(1.0), None);
    assert!(!p.connected);
}
