//! Per-message traffic counters and connection timing (rewrite extension,
//! plan milestone B0). Byte counts are the postcard payload Lightyear
//! serializes for each message, before channel/packet headers; they compare
//! message designs, not exact datagram sizes.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Instant};

/// One message type in one direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KindStats {
    pub messages: u64,
    pub bytes: u64,
    /// Largest single payload.
    pub max_bytes: u64,
    /// Messages dropped before use (invalid, stale, unknown baseline).
    pub rejected: u64,
}

/// Round trip and jitter from heartbeat echoes (RFC 3550 style smoothing).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Timing {
    pub rtt_ms: Option<f32>,
    pub jitter_ms: f32,
    /// Age of the newest accepted game state.
    pub snapshot_age_ms: Option<f32>,
    /// Owned-avatar prediction error at the latest reconciliation.
    pub prediction_error: Option<f32>,
    /// Last state the host confirmed this client acknowledged.
    pub acked_baseline: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrafficStats {
    pub sent: BTreeMap<String, KindStats>,
    pub received: BTreeMap<String, KindStats>,
    pub timing: Timing,
    #[serde(skip)]
    started: Option<Instant>,
    /// Seconds covered by these counters (set by [`TrafficStats::seconds`]).
    #[serde(default)]
    pub elapsed: f32,
}

/// Postcard payload size, as Lightyear serializes messages.
pub fn payload_size<T: Serialize>(message: &T) -> u64 {
    postcard::serialize_with_flavor(message, postcard::ser_flavors::Size::default())
        .map_or(0, |n: usize| n as u64)
}

fn record(map: &mut BTreeMap<String, KindStats>, kind: &str, bytes: u64) {
    let entry = map.entry(kind.to_string()).or_default();
    entry.messages += 1;
    entry.bytes += bytes;
    entry.max_bytes = entry.max_bytes.max(bytes);
}

impl TrafficStats {
    pub fn sent<T: Serialize>(&mut self, kind: &str, message: &T) -> u64 {
        let bytes = payload_size(message);
        self.sent_bytes(kind, bytes);
        bytes
    }
    pub fn sent_bytes(&mut self, kind: &str, bytes: u64) {
        self.started.get_or_insert_with(Instant::now);
        record(&mut self.sent, kind, bytes);
    }
    pub fn received<T: Serialize>(&mut self, kind: &str, message: &T) {
        self.started.get_or_insert_with(Instant::now);
        record(&mut self.received, kind, payload_size(message));
    }
    pub fn rejected(&mut self, kind: &str) {
        self.received.entry(kind.to_string()).or_default().rejected += 1;
    }
    /// A heartbeat echo arrived `rtt_ms` after its heartbeat was sent.
    pub fn round_trip(&mut self, rtt_ms: f32) {
        if !rtt_ms.is_finite() || rtt_ms < 0.0 {
            return;
        }
        let t = &mut self.timing;
        if let Some(old) = t.rtt_ms {
            t.jitter_ms += ((rtt_ms - old).abs() - t.jitter_ms) / 16.0;
            t.rtt_ms = Some(old + (rtt_ms - old) / 8.0);
        } else {
            t.rtt_ms = Some(rtt_ms);
        }
    }
    pub fn total_sent_bytes(&self) -> u64 {
        self.sent.values().map(|k| k.bytes).sum()
    }
    pub fn total_received_bytes(&self) -> u64 {
        self.received.values().map(|k| k.bytes).sum()
    }
    /// Seconds since the first counted message.
    pub fn seconds(&mut self) -> f32 {
        self.elapsed = self.started.map_or(0.0, |s| s.elapsed().as_secs_f32());
        self.elapsed
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// Reproducible QA export.
    pub fn to_json(&mut self) -> String {
        self.seconds();
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    /// `direction,kind,messages,bytes,max_bytes,rejected` rows.
    pub fn to_csv(&self) -> String {
        let mut out = String::from("direction,kind,messages,bytes,max_bytes,rejected\n");
        for (direction, map) in [("sent", &self.sent), ("received", &self.received)] {
            for (kind, k) in map {
                out.push_str(&format!(
                    "{direction},{kind},{},{},{},{}\n",
                    k.messages, k.bytes, k.max_bytes, k.rejected
                ));
            }
        }
        out
    }
}
