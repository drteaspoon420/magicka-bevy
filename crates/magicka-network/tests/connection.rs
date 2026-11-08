use magicka_network::{connection::*, transport::PLAYER_LEASE};
use std::time::{Duration, Instant};

#[test]
fn fresh_snapshots_renew_and_silence_disconnects_at_the_deadline() {
    let start = Instant::now();
    let mut watch = SnapshotWatchdog::new(start);
    let almost = start + PLAYER_LEASE - Duration::from_millis(1);
    watch.update(almost);
    assert_eq!(watch.lost(), None);
    assert!(watch.accepted_snapshot(almost));
    watch.update(start + PLAYER_LEASE);
    assert_eq!(watch.lost(), None);
    watch.update(almost + PLAYER_LEASE);
    assert_eq!(watch.lost(), Some(ConnectionLost::SnapshotTimeout));
    assert!(!watch.accepted_snapshot(almost + PLAYER_LEASE));
}
#[test]
fn late_snapshot_and_removed_ownership_cannot_resume_a_session() {
    let start = Instant::now();
    let mut watch = SnapshotWatchdog::new(start);
    assert!(!watch.accepted_snapshot(start + PLAYER_LEASE));
    watch.ownership_removed();
    assert_eq!(watch.lost(), Some(ConnectionLost::SnapshotTimeout));
    let mut watch = SnapshotWatchdog::new(start);
    watch.ownership_removed();
    assert!(!watch.accepted_snapshot(start + Duration::from_millis(1)));
    assert_eq!(watch.lost(), Some(ConnectionLost::OwnershipRemoved));
}
