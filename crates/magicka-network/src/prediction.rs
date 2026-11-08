//! Owned-wizard movement prediction and reconciliation (plan milestone B4).
//!
//! The client records every input it sends with its sequence number and
//! frame time. When an authoritative state arrives with the host's newest
//! applied sequence (`PlayerState::last_input`), the predictor restarts
//! from the authoritative position and replays the inputs the host has not
//! applied yet. Visible corrections are smoothed. Teleports, respawns,
//! deaths and epoch changes snap. Prediction is presentation only: the
//! host never consumes it, and collisions, knockback and status effects are
//! corrected by the next authoritative state.
use crate::control::Controls;
use std::collections::VecDeque;

/// Errors above this (metres) snap instead of blending.
pub const SNAP_DISTANCE: f32 = 2.5;
/// Fraction of the visible correction removed per second.
pub const CORRECTION_RATE: f32 = 10.0;
/// Inputs kept for replay (about 2 s at 60 Hz).
const MAX_PENDING: usize = 120;
/// Host updates in a row that applied a move without moving the wizard, after
/// which the client stops predicting movement. Only walking is modelled here,
/// and the host can refuse it for many reasons: a wall, a cutscene, a dialog,
/// being frozen or knocked down. Predicting anyway shows the wizard walking
/// away from where it really stands.
const STALLED_UPDATES: u32 = 3;
/// Host movement below this (metres) between updates counts as standing still.
const STALLED_DISTANCE: f32 = 0.005;

/// The motion model the client shares with the host: walking toward the
/// aim point (or along the pad direction) at the wizard's current speed.
pub fn step(position: [f32; 3], controls: &Controls, speed: f32, dt: f32) -> [f32; 3] {
    let direction = match (controls.pad_movement, controls.aim) {
        (Some(pad), _) if pad[0] != 0.0 || pad[1] != 0.0 => {
            let len = (pad[0] * pad[0] + pad[1] * pad[1]).sqrt().min(1.0);
            let n = (pad[0] * pad[0] + pad[1] * pad[1]).sqrt();
            Some([pad[0] / n * len, pad[1] / n * len])
        }
        (_, Some(aim)) if controls.move_down => {
            let d = [aim[0] - position[0], aim[2] - position[2]];
            let n = (d[0] * d[0] + d[1] * d[1]).sqrt();
            (n > 0.1).then(|| [d[0] / n, d[1] / n])
        }
        _ => None,
    };
    match direction {
        Some(d) => [
            position[0] + d[0] * speed * dt,
            position[1],
            position[2] + d[1] * speed * dt,
        ],
        None => position,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Predictor {
    pending: VecDeque<(u64, f32, Controls)>,
    /// Authoritative position, the newest applied sequence and the epoch.
    authority: Option<([f32; 3], u64, u64)>,
    predicted: Option<[f32; 3]>,
    /// Visible offset still being blended away (predicted + offset).
    offset: [f32; 3],
    /// The last reconciliation's prediction error (metres).
    pub last_error: Option<f32>,
    /// Consecutive host updates that refused a move.
    stalled: u32,
}

impl Predictor {
    /// An input was sent (`dt`: the frame time it covers).
    pub fn record(&mut self, sequence: u64, dt: f32, controls: Controls, speed: f32) {
        self.pending.push_back((sequence, dt, controls));
        while self.pending.len() > MAX_PENDING {
            self.pending.pop_front();
        }
        if self.is_stalled() {
            return;
        }
        if let Some(p) = &mut self.predicted {
            *p = step(*p, &controls, speed, dt);
        }
    }

    /// The host is refusing to move the wizard, so nothing is predicted.
    pub fn is_stalled(&self) -> bool {
        self.stalled >= STALLED_UPDATES
    }

    /// An authoritative state for the owned wizard. `discontinuity`: the
    /// host respawned, teleported or killed it (always snap).
    pub fn reconcile(
        &mut self,
        position: [f32; 3],
        last_input: u64,
        epoch: u64,
        speed: f32,
        discontinuity: bool,
    ) {
        let new_epoch = self.authority.is_none_or(|(_, _, e)| e != epoch);
        if self
            .authority
            .is_some_and(|(_, seq, e)| e == epoch && last_input < seq)
        {
            return; // older than what we already reconciled
        }
        if let Some((before, seq, _)) = self.authority.filter(|_| !new_epoch) {
            if distance(before, position) >= STALLED_DISTANCE {
                self.stalled = 0;
            } else if last_input > seq
                && self
                    .pending
                    .iter()
                    .any(|(s, dt, c)| *s <= last_input && step(before, c, speed, *dt) != before)
            {
                // New inputs asked to move, and it did not.
                self.stalled += 1;
            }
        } else {
            self.stalled = 0;
        }
        self.authority = Some((position, last_input, epoch));
        if new_epoch {
            self.pending.clear();
        }
        self.pending.retain(|(seq, _, _)| *seq > last_input);
        let replayed = if self.is_stalled() {
            position
        } else {
            self.pending
                .iter()
                .fold(position, |p, (_, dt, c)| step(p, c, speed, *dt))
        };
        let before = self.visible();
        let error = before.map(|b| distance(b, replayed));
        self.last_error = error;
        self.predicted = Some(replayed);
        self.offset = match before {
            Some(b) if !discontinuity && !new_epoch && error.unwrap_or(0.0) < SNAP_DISTANCE => {
                [b[0] - replayed[0], b[1] - replayed[1], b[2] - replayed[2]]
            }
            _ => [0.0; 3],
        };
    }

    /// Blend the visible correction away.
    pub fn update(&mut self, dt: f32) {
        let keep = (1.0 - CORRECTION_RATE * dt).clamp(0.0, 1.0);
        for o in &mut self.offset {
            *o *= keep;
        }
    }

    /// Where to draw the owned wizard.
    pub fn visible(&self) -> Option<[f32; 3]> {
        self.predicted.map(|p| {
            [
                p[0] + self.offset[0],
                p[1] + self.offset[1],
                p[2] + self.offset[2],
            ]
        })
    }
    /// Inputs not yet applied by the host.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}
