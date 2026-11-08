//! SummonPhoenix singleton (research/phoenix.md).
use magicka_sim::phoenix::{Cast, PhoenixEvent, SummonPhoenix};

#[test]
fn blast_when_the_descend_clip_finishes() {
    let mut p = SummonPhoenix::default();
    assert_eq!(p.cast(false, Some(1), 2.0), Cast::Started);
    assert_eq!(p.cast(false, Some(1), 2.0), Cast::Busy, "one at a time");
    assert_eq!(p.update(0.5, 1.0), None);
    assert_eq!(p.update(0.5, 1.0), Some(PhoenixEvent::Blast));
    assert!(p.finished && !p.registered);
    assert_eq!(p.cast(false, Some(1), 3.0), Cast::Started, "free again");
}

/// Native quirk: an indoor cast clears sFinished without registering the
/// effect, so nothing ever resets it and later casts are refused.
#[test]
fn a_lone_indoor_cast_leaves_the_singleton_busy() {
    let mut p = SummonPhoenix::default();
    assert_eq!(p.cast(true, Some(1), 0.0), Cast::Indoors);
    for _ in 0..100 {
        assert_eq!(p.update(0.1, 1.0), None);
    }
    assert_eq!(p.cast(false, Some(1), 0.0), Cast::Busy);
}

/// An indoor cast during a descent turns it into the 1.5 s fail shake.
#[test]
fn indoor_cast_during_a_descent_cancels_the_blast() {
    let mut p = SummonPhoenix::default();
    p.cast(false, None, 0.0);
    p.cast(true, None, 0.0);
    assert_eq!(p.update(1.0, 0.5), None);
    assert_eq!(p.update(0.5, 0.5), Some(PhoenixEvent::FailShake));
    assert!(p.finished);
}

#[test]
fn offsets_and_revival_points() {
    assert_eq!(SummonPhoenix::offset(true, 1.0, 0.0), [2.0, 0.0, -2.0]);
    assert_eq!(SummonPhoenix::offset(false, 1.0, 0.0), [1.5, 0.0, -1.5]);
    let o = SummonPhoenix::revive_offset(0);
    assert!((o[2] + 1.0).abs() < 1e-6 && o[0].abs() < 1e-6, "{o:?}");
    let o = SummonPhoenix::revive_offset(1);
    assert!((o[0] + 1.0).abs() < 1e-6, "{o:?}");
}
