//! Bounded client presentation between confirmed host poses; never predicts gameplay.
use crate::{session::GameFrame, FrameError, PhysicsReplica};
use bevy::math::{Mat3, Mat4, Quat, Vec3};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::{Duration, Instant},
};

/// Modern presentation policy. Receive timestamps work for both the fixed-step
/// dedicated host and the variable-frame-rate playable host without wire changes.
pub const INTERPOLATION_DELAY: Duration = Duration::from_millis(50);
const MAX_SAMPLES: usize = 32;

/// Adaptive jitter buffer bounds (modern policy, plan B4).
pub const MIN_DELAY: Duration = INTERPOLATION_DELAY;
pub const MAX_DELAY: Duration = Duration::from_millis(250);
/// The delay moves toward its target by at most this per received frame,
/// so presentation time never jumps.
const DELAY_STEP: Duration = Duration::from_millis(2);

#[derive(Default)]
pub struct GamePresentation {
    validation: PhysicsReplica,
    samples: VecDeque<(Instant, GameFrame)>,
    /// Adaptive delay state; `None` keeps the fixed 50 ms delay.
    adaptive: Option<Adaptive>,
}

/// RFC 3550-style interarrival jitter against the host clock.
#[derive(Clone, Copy, Debug)]
struct Adaptive {
    jitter: f64,
    delay: Duration,
    last: Option<(Instant, f64, u64)>,
}
impl GamePresentation {
    /// A presentation whose delay adapts to measured arrival jitter
    /// (50..=250 ms): delay = 50 ms + 3 × jitter, approached gradually.
    pub fn adaptive() -> Self {
        Self {
            adaptive: Some(Adaptive {
                jitter: 0.0,
                delay: MIN_DELAY,
                last: None,
            }),
            ..Default::default()
        }
    }
    /// When the newest sample arrived.
    pub fn newest_received(&self) -> Option<Instant> {
        self.samples.back().map(|(t, _)| *t)
    }
    /// The current interpolation delay.
    pub fn delay(&self) -> Duration {
        self.adaptive.map_or(INTERPOLATION_DELAY, |a| a.delay)
    }
    /// Smoothed arrival jitter (seconds; zero when not adaptive).
    pub fn jitter(&self) -> f64 {
        self.adaptive.map_or(0.0, |a| a.jitter)
    }
    fn observe(&mut self, frame: &GameFrame, received: Instant) {
        let Some(a) = &mut self.adaptive else {
            return;
        };
        let epoch = frame.physics.epoch;
        if let Some((at, time, last_epoch)) = a.last {
            if last_epoch == epoch && frame.time > time && received > at {
                let d = received.duration_since(at).as_secs_f64() - (frame.time - time);
                a.jitter += (d.abs() - a.jitter) / 16.0;
            }
        }
        a.last = Some((received, frame.time, epoch));
        let target = (MIN_DELAY.as_secs_f64() + 3.0 * a.jitter)
            .clamp(MIN_DELAY.as_secs_f64(), MAX_DELAY.as_secs_f64());
        let current = a.delay.as_secs_f64();
        let step = DELAY_STEP.as_secs_f64();
        a.delay = Duration::from_secs_f64(current + (target - current).clamp(-step, step));
    }
    pub fn current(&self) -> Option<&GameFrame> {
        self.samples.back().map(|(_, frame)| frame)
    }

    /// Validate before changing either the authoritative state or presentation clock.
    pub fn apply(&mut self, frame: GameFrame, received: Instant) -> Result<(), FrameError> {
        if self
            .samples
            .back()
            .is_some_and(|(time, _)| received < *time)
        {
            return Err(FrameError::Stale);
        }
        let mut handles = std::collections::BTreeSet::new();
        if !frame.valid()
            || frame.players.iter().any(|p| {
                let basis = Mat3::from_cols(p.right.into(), p.up.into(), p.backward.into());
                !handles.insert(p.handle)
                    || (basis.determinant() - 1.).abs() > 0.02
                    || basis.x_axis.dot(basis.y_axis).abs() > 0.02
                    || basis.y_axis.dot(basis.z_axis).abs() > 0.02
                    || basis.z_axis.dot(basis.x_axis).abs() > 0.02
            })
        {
            return Err(FrameError::InvalidBody);
        }
        if self
            .current()
            .is_some_and(|old| old.physics.epoch == frame.physics.epoch && old.level != frame.level)
        {
            return Err(FrameError::Scene);
        }
        self.validation.apply(frame.physics.clone())?;
        // A death/respawn, replacement, or omission ends a player's pose history,
        // even if a later snapshot reuses its slot and handle within this epoch.
        if let Some(previous) = self.current() {
            let discontinuities: Vec<_> = previous
                .players
                .iter()
                .filter(|old| {
                    frame
                        .players
                        .iter()
                        .find(|p| p.slot == old.slot)
                        .is_none_or(|new| {
                            new.handle != old.handle
                                || new.profile.robe != old.profile.robe
                                || new.dead != old.dead
                        })
                })
                .map(|p| p.slot)
                .collect();
            for (_, sample) in &mut self.samples {
                sample
                    .players
                    .retain(|p| !discontinuities.contains(&p.slot));
            }
        }
        // NPC handles have no wire generation. An observed omission, template
        // replacement or death transition is therefore a lifetime boundary.
        if let Some(previous) = self.current() {
            let newest: BTreeMap<_, _> = frame.npcs.iter().map(|n| (n.handle, n)).collect();
            let discontinuities: BTreeSet<_> = previous
                .npcs
                .iter()
                .filter(|old| {
                    newest
                        .get(&old.handle)
                        .is_none_or(|new| new.template != old.template || new.dead != old.dead)
                })
                .map(|n| n.handle)
                .collect();
            for (_, sample) in &mut self.samples {
                sample.npcs.retain(|n| !discontinuities.contains(&n.handle));
            }
        }
        if self
            .current()
            .is_none_or(|old| old.physics.epoch != frame.physics.epoch)
        {
            self.samples.clear();
        }
        // Several snapshots drained in one render frame have no measurable
        // receive spacing. Keep the newest instead of inventing a host tick rate.
        if self
            .samples
            .back()
            .is_some_and(|(time, _)| *time == received)
        {
            self.samples.pop_back();
        }
        self.observe(&frame, received);
        self.samples.push_back((received, frame));
        while self.samples.len() > MAX_SAMPLES {
            self.samples.pop_front();
        }
        Ok(())
    }

    /// Membership, health, loadouts, queues and effects always use the newest frame.
    /// Only poses and continuous times within compatible animations are blended.
    /// Sample 50 ms behind receive time; hold the newest pose on underflow/loss.
    pub fn sample(&self, now: Instant) -> Option<GameFrame> {
        let mut result = self.current()?.clone();
        let target = now.checked_sub(self.delay()).unwrap_or(now);
        if target >= self.samples.back()?.0 {
            return Some(result);
        }
        let index = self
            .samples
            .iter()
            .rposition(|(time, _)| *time <= target)
            .unwrap_or(0);
        let (start, old) = &self.samples[index];
        let (end, new) = self.samples.get(index + 1).unwrap_or(&self.samples[index]);
        let alpha = if end > start {
            (target.saturating_duration_since(*start).as_secs_f64()
                / end.duration_since(*start).as_secs_f64())
            .clamp(0., 1.) as f32
        } else {
            1.
        };
        for player in &mut result.players {
            let matches = |p: &&crate::session::PlayerState| {
                p.slot == player.slot
                    && p.handle == player.handle
                    && p.profile.robe == player.profile.robe
                    && p.dead == player.dead
            };
            let (Some(a), Some(b)) = (
                old.players.iter().find(matches),
                new.players.iter().find(matches),
            ) else {
                continue;
            };
            player.position = lerp(a.position, b.position, alpha);
            let rotation = |p: &crate::session::PlayerState| {
                Quat::from_mat3(&Mat3::from_cols(
                    Vec3::from(p.right),
                    Vec3::from(p.up),
                    Vec3::from(p.backward),
                ))
                .normalize()
            };
            let basis = Mat3::from_quat(rotation(a).slerp(rotation(b), alpha));
            player.right = basis.x_axis.to_array();
            player.up = basis.y_axis.to_array();
            player.backward = basis.z_axis.to_array();
            if a.animation == player.animation
                && b.animation == player.animation
                && b.animation_time >= a.animation_time
            {
                player.animation_time =
                    a.animation_time + (b.animation_time - a.animation_time) * alpha;
            }
        }
        let old_npcs: BTreeMap<_, _> = old.npcs.iter().map(|n| (n.handle, n)).collect();
        let new_npcs: BTreeMap<_, _> = new.npcs.iter().map(|n| (n.handle, n)).collect();
        for npc in &mut result.npcs {
            let matches =
                |n: &&&crate::session::NpcState| n.template == npc.template && n.dead == npc.dead;
            let (Some(a), Some(b)) = (
                old_npcs.get(&npc.handle).filter(matches),
                new_npcs.get(&npc.handle).filter(matches),
            ) else {
                continue;
            };
            if let Some(transform) = blend_transform(a.transform, b.transform, alpha) {
                npc.transform = transform;
            }
            if a.clip == npc.clip
                && b.clip == npc.clip
                && a.looping == npc.looping
                && b.looping == npc.looping
                && b.clip_time >= a.clip_time
            {
                npc.clip_time = a.clip_time + (b.clip_time - a.clip_time) * alpha;
            }
        }
        let old_bodies: BTreeMap<_, _> = old.physics.bodies.iter().map(|b| (b.key(), b)).collect();
        let new_bodies: BTreeMap<_, _> = new.physics.bodies.iter().map(|b| (b.key(), b)).collect();
        for body in &mut result.physics.bodies {
            let matches =
                |b: &&&crate::RemoteBody| b.asset == body.asset && b.center == body.center;
            let (Some(a), Some(b)) = (
                old_bodies.get(&body.key()).filter(matches),
                new_bodies.get(&body.key()).filter(matches),
            ) else {
                continue;
            };
            body.position = lerp(a.position, b.position, alpha);
            body.rotation = Quat::from_array(a.rotation)
                .normalize()
                .slerp(Quat::from_array(b.rotation).normalize(), alpha)
                .to_array();
        }
        Some(result)
    }
}

fn lerp(a: [f32; 3], b: [f32; 3], alpha: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * alpha)
}

// Matrix element interpolation shrinks rotating models. Decompose only ordinary
// affine scale/rotation/translation poses; unsupported finite wire matrices keep
// their newest confirmed value rather than introducing shear or nonfinite poses.
fn blend_transform(a: [f32; 16], b: [f32; 16], alpha: f32) -> Option<[f32; 16]> {
    let decompose = |values: [f32; 16]| {
        let matrix = Mat4::from_cols_array(&values);
        if values[3] != 0.
            || values[7] != 0.
            || values[11] != 0.
            || values[15] != 1.
            || matrix.determinant().abs() < 1e-6
        {
            return None;
        }
        let (scale, rotation, position) = matrix.to_scale_rotation_translation();
        if !scale.is_finite()
            || !rotation.is_finite()
            || !rotation.is_normalized()
            || !position.is_finite()
            || !Mat4::from_scale_rotation_translation(scale, rotation, position)
                .abs_diff_eq(matrix, 1e-4)
        {
            return None;
        }
        Some((scale, rotation.normalize(), position))
    };
    let (sa, ra, pa) = decompose(a)?;
    let (sb, rb, pb) = decompose(b)?;
    // A handedness change passes through a singular scale and is discontinuous.
    if sa.signum() != sb.signum() {
        return None;
    }
    let matrix = Mat4::from_scale_rotation_translation(
        sa.lerp(sb, alpha),
        ra.slerp(rb, alpha),
        pa.lerp(pb, alpha),
    );
    matrix.is_finite().then(|| matrix.to_cols_array())
}
