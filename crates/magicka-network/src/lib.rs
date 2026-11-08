//! Modern host-owned replication. Independent of the retail networking protocol.
pub mod connection;
pub mod control;
pub mod delta;
pub mod discovery;
pub mod lobby;
pub mod prediction;
pub mod presentation;
pub mod security;
pub mod session;
pub mod telemetry;
pub mod transport;
pub mod world;
use magicka_physics::{BodyKey, BodyState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROTOCOL_VERSION: u16 = 20;
pub const MAX_BODIES: usize = 4096;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoteBody {
    pub handle: u16,
    pub generation: u64,
    pub asset: String,
    pub center: [f32; 3],
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub linear_velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub sleeping: bool,
}
impl RemoteBody {
    pub fn new(key: BodyKey, asset: String, center: [f32; 3], s: BodyState) -> Self {
        Self {
            handle: key.handle,
            generation: key.generation,
            asset,
            center,
            position: s.position,
            rotation: s.rotation,
            linear_velocity: s.linear_velocity,
            angular_velocity: s.angular_velocity,
            sleeping: s.sleeping,
        }
    }
    pub fn key(&self) -> BodyKey {
        BodyKey {
            handle: self.handle,
            generation: self.generation,
        }
    }
    pub fn state(&self) -> BodyState {
        BodyState {
            position: self.position,
            rotation: self.rotation,
            linear_velocity: self.linear_velocity,
            angular_velocity: self.angular_velocity,
            sleeping: self.sleeping,
        }
    }
    pub fn model_transform(&self) -> [f32; 16] {
        self.state().model_transform(self.center)
    }
    fn valid(&self) -> bool {
        let norm = self.rotation.iter().map(|v| v * v).sum::<f32>();
        self.position
            .iter()
            .chain(&self.rotation)
            .chain(&self.center)
            .chain(&self.linear_velocity)
            .chain(&self.angular_velocity)
            .all(|v| v.is_finite())
            && norm.is_finite()
            && (norm - 1.).abs() < 0.01
            && canonical_asset(&self.asset).is_ok_and(|a| a == self.asset)
    }
}
/// Canonical content-relative asset identity, never a filesystem/network URL.
pub fn canonical_asset(asset: &str) -> Result<String, FrameError> {
    if asset.is_empty()
        || asset.len() > 512
        || asset.starts_with('/')
        || asset.starts_with('\\')
        || asset.contains(':')
        || asset.contains('\0')
    {
        return Err(FrameError::InvalidBody);
    }
    let asset = asset.replace('\\', "/");
    let mut parts = Vec::new();
    for p in asset.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or(FrameError::InvalidBody)?;
            }
            _ => parts.push(p),
        }
    }
    let result = parts.join("/");
    if !result.starts_with("Models/") && !result.starts_with("Data/") {
        return Err(FrameError::InvalidBody);
    }
    Ok(result)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhysicsFrame {
    pub version: u16,
    /// New host process/session identity; reconnect must explicitly reset the replica.
    pub session: u64,
    /// Host increments on every scene rebuild, including same-scene restart.
    pub epoch: u64,
    pub tick: u64,
    pub scene: String,
    /// Full live set: omitted bodies are despawned, including after packet loss.
    pub bodies: Vec<RemoteBody>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    Version,
    Session,
    Stale,
    InvalidBody,
    DuplicateHandle,
    Limit,
    Scene,
    Lifetime,
}
impl PhysicsFrame {
    pub fn validate(&self) -> Result<(), FrameError> {
        if self.version != PROTOCOL_VERSION {
            return Err(FrameError::Version);
        }
        if self.session == 0 || self.epoch == 0 {
            return Err(FrameError::Session);
        }
        if self.bodies.len() > MAX_BODIES {
            return Err(FrameError::Limit);
        }
        if self.scene.is_empty() || self.scene.len() > 128 || !self.scene.is_ascii() {
            return Err(FrameError::Scene);
        }
        let mut keys = std::collections::BTreeSet::new();
        for b in &self.bodies {
            if !b.valid() {
                return Err(FrameError::InvalidBody);
            }
            if !keys.insert(b.handle) {
                return Err(FrameError::DuplicateHandle);
            }
        }
        Ok(())
    }
}
#[derive(Default, Debug)]
pub struct PhysicsReplica {
    current: Option<PhysicsFrame>,
    previous: Option<PhysicsFrame>,
    lifetimes: BTreeMap<u16, u64>,
}
impl PhysicsReplica {
    pub fn current(&self) -> Option<&PhysicsFrame> {
        self.current.as_ref()
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// Validate before mutation; stale and malformed frames leave the replica intact.
    pub fn apply(&mut self, frame: PhysicsFrame) -> Result<(), FrameError> {
        frame.validate()?;
        if let Some(old) = &self.current {
            if frame.session != old.session {
                return Err(FrameError::Session);
            }
            if (frame.epoch, frame.tick) <= (old.epoch, old.tick) {
                return Err(FrameError::Stale);
            }
            if frame.epoch == old.epoch && frame.scene != old.scene {
                return Err(FrameError::Scene);
            }
        }
        let same_epoch = self
            .current
            .as_ref()
            .is_some_and(|old| old.epoch == frame.epoch);
        if same_epoch {
            let live_keys: std::collections::BTreeSet<_> = self
                .current
                .as_ref()
                .unwrap()
                .bodies
                .iter()
                .map(|b| b.key())
                .collect();
            for b in &frame.bodies {
                if let Some(generation) = self.lifetimes.get(&b.handle) {
                    let live = live_keys.contains(&b.key());
                    if b.generation < *generation || (b.generation == *generation && !live) {
                        return Err(FrameError::Lifetime);
                    }
                }
            }
        } else {
            self.lifetimes.clear();
        }
        for b in &frame.bodies {
            self.lifetimes.insert(b.handle, b.generation);
        }
        self.previous = self.current.take().filter(|old| old.epoch == frame.epoch);
        self.current = Some(frame);
        Ok(())
    }
    /// Render between two confirmed host samples. New lifetimes never blend with old ones.
    pub fn sample(&self, alpha: f32) -> Vec<RemoteBody> {
        let Some(current) = &self.current else {
            return vec![];
        };
        let previous: BTreeMap<_, _> = self
            .previous
            .as_ref()
            .map(|f| f.bodies.iter().map(|b| (b.key(), b)).collect())
            .unwrap_or_default();
        let alpha = if alpha.is_finite() {
            alpha.clamp(0., 1.)
        } else {
            1.
        };
        current
            .bodies
            .iter()
            .map(|body| {
                let mut b = body.clone();
                if let Some(old) = previous
                    .get(&body.key())
                    .filter(|o| o.asset == body.asset && o.center == body.center)
                {
                    for i in 0..3 {
                        b.position[i] =
                            old.position[i] + (body.position[i] - old.position[i]) * alpha;
                    }
                    b.rotation = bevy::math::Quat::from_array(old.rotation)
                        .normalize()
                        .slerp(
                            bevy::math::Quat::from_array(body.rotation).normalize(),
                            alpha,
                        )
                        .to_array();
                }
                b
            })
            .collect()
    }
}
