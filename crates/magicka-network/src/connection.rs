//! Modern client liveness policy; independent of retail networking.
use crate::transport::PLAYER_LEASE;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionLost {
    SnapshotTimeout,
    OwnershipRemoved,
    /// The host or a dungeon master removed this player.
    Kicked,
}
impl ConnectionLost {
    pub fn notice(self) -> &'static str {
        match self {
            Self::SnapshotTimeout => "Disconnected: host timed out. Restart this client to rejoin.",
            Self::OwnershipRemoved => {
                "Disconnected: player slot released. Restart this client to rejoin."
            }
            Self::Kicked => "Removed from the game by the host.",
        }
    }
}
/// Only newly accepted authoritative snapshots may renew this clock.
/// Disconnect is terminal: delayed snapshots cannot silently resume controls.
pub struct SnapshotWatchdog {
    last_snapshot: Instant,
    lost: Option<ConnectionLost>,
}
impl SnapshotWatchdog {
    pub fn new(now: Instant) -> Self {
        Self {
            last_snapshot: now,
            lost: None,
        }
    }
    pub fn lost(&self) -> Option<ConnectionLost> {
        self.lost
    }
    pub fn update(&mut self, now: Instant) {
        if self.lost.is_none() && now.saturating_duration_since(self.last_snapshot) >= PLAYER_LEASE
        {
            self.lost = Some(ConnectionLost::SnapshotTimeout);
        }
    }
    pub fn accepted_snapshot(&mut self, now: Instant) -> bool {
        self.update(now);
        if self.lost.is_some() {
            return false;
        }
        self.last_snapshot = now;
        true
    }
    pub fn kicked(&mut self) {
        if self.lost.is_none() {
            self.lost = Some(ConnectionLost::Kicked);
        }
    }
    pub fn ownership_removed(&mut self) {
        if self.lost.is_none() {
            self.lost = Some(ConnectionLost::OwnershipRemoved);
        }
    }
}
