//! Portal magick (research/portal.md): two singleton PortalEntity instances
//! (blue and orange). Each cast swaps them and re-places the first one;
//! while both stand within 30 m (scaled by the camera magnification) they
//! are connected and anything touching one comes out of the other. Scene,
//! navmesh and body queries live in the host.
use crate::spray_pulse::HitList;

pub type Vector = [f32; 3];

/// Execute: the spawn point is the owner + Direction · 2.
pub const SPAWN_AHEAD: f32 = 2.0;
/// Execute fails (magick_fail) when the navmesh point is over 5 m away.
pub const MAX_SNAP_SQ: f32 = 25.0;
/// GetNearestPosition movement: Water | Dynamic.
pub const NAV_MOVEMENT: u8 = 0x81;
/// Initialize: probe from 2 m above the point, 4 m down; the body hovers
/// 1.5 m above the hit.
pub const PROBE_RISE: f32 = 2.0;
pub const PROBE_DROP: f32 = 4.0;
pub const HOVER: f32 = 1.5;
/// Capsule(0, −1, 0; RotationX(−π/2), radius 0.5, length 2): the axis runs
/// along Backward = +Y, from 1 m below the body to 1 m above.
pub const RADIUS: f32 = 0.5;
pub const HALF_LENGTH: f32 = 1.0;
/// Portal.Update: connected while distance² ≤ 900 / Magnification.
pub const MAX_DISTANCE_SQ: f32 = 900.0;
/// OnCollision: sIgnoredEntities[handle] = 0.1.
pub const IGNORE_TIME: f32 = 0.1;
/// GetOutPos: the offset from the entry portal, scaled by 0.8.
pub const OUT_SCALE: f32 = 0.8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalType {
    Blue,
    Orange,
}

impl PortalType {
    /// EFFECT_BLUE / EFFECT_ORANGE.
    pub fn effect(self) -> &'static str {
        match self {
            PortalType::Blue => "magick_portal_blue",
            PortalType::Orange => "magick_portal_orange",
        }
    }
    /// RenderData.TextureOffset over EffectTextures/portal (TextureScale
    /// 0.5 × 1): the left half is blue, the right half orange.
    pub fn texture_offset(self) -> f32 {
        match self {
            PortalType::Blue => 0.0,
            PortalType::Orange => 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PortalEntity {
    pub kind: PortalType,
    pub position: Vector,
    /// mDead (the constructor starts dead).
    pub dead: bool,
    /// mEffectAlpha: the additive billboard fades in over a second.
    pub effect_alpha: f32,
    /// mConnected: the connection state its particle effect reflects.
    pub connected: bool,
}

impl PortalEntity {
    fn new(kind: PortalType) -> Self {
        Self {
            kind,
            position: [0.0; 3],
            dead: true,
            effect_alpha: 0.0,
            connected: false,
        }
    }

    /// The collision capsule segment (bottom, top) and radius.
    pub fn capsule(&self) -> (Vector, Vector, f32) {
        let [x, y, z] = self.position;
        ([x, y - HALF_LENGTH, z], [x, y + HALF_LENGTH, z], RADIUS)
    }
}

/// What PortalEntity.Update asks of the host's particle effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectChange {
    /// Stop it, then start the portal's effect when `connected`.
    Restart { connected: bool },
}

#[derive(Debug, Clone)]
pub struct Portal {
    /// sPortalA (the one the next cast swaps away) and sPortalB.
    pub a: PortalEntity,
    pub b: PortalEntity,
    /// sConnected, recomputed by Portal.Update.
    pub connected: bool,
    /// sIgnoredEntities (aged by sPortalA's Update).
    pub ignored: HitList,
}

impl Default for Portal {
    fn default() -> Self {
        Self {
            a: PortalEntity::new(PortalType::Blue),
            b: PortalEntity::new(PortalType::Orange),
            connected: false,
            ignored: HitList::default(),
        }
    }
}

impl Portal {
    /// Execute(owner): the point before the navmesh snap.
    pub fn spawn_point(owner_position: Vector, owner_direction: Vector) -> Vector {
        std::array::from_fn(|i| owner_position[i] + owner_direction[i] * SPAWN_AHEAD)
    }

    /// Execute's distance check between the point and its navmesh snap.
    pub fn snap_accepted(point: Vector, snapped: Vector) -> bool {
        let d: f32 = (0..3).map(|i| (snapped[i] - point[i]).powi(2)).sum();
        d <= MAX_SNAP_SQ
    }

    /// Helper.Swap(sPortalA, sPortalB) then sPortalA.Initialize: the body at
    /// the probe hit + 1.5 (or the snapped point when the probe misses).
    /// Returns the re-placed portal; it starts its effect only when the
    /// portals were already connected.
    pub fn place(&mut self, snapped: Vector, ground: Option<Vector>) -> PortalEntity {
        std::mem::swap(&mut self.a, &mut self.b);
        let position = match ground {
            Some([x, y, z]) => [x, y + HOVER, z],
            None => snapped,
        };
        self.a.position = position;
        self.a.dead = false;
        self.a.effect_alpha = 0.0;
        self.a.connected = self.connected;
        self.a
    }

    /// Portal.Kill (Nullify): both portals die.
    pub fn kill(&mut self) {
        self.a.dead = true;
        self.b.dead = true;
    }

    /// Portal.Update (SpellManager): the connection state; also the
    /// `spell_shield` loop's Health variable 1 − distance / 30.
    pub fn update_connection(&mut self, magnification: f32) -> Option<f32> {
        let d: f32 = (0..3)
            .map(|i| (self.a.position[i] - self.b.position[i]).powi(2))
            .sum();
        self.connected = !self.a.dead && !self.b.dead && d <= MAX_DISTANCE_SQ / magnification;
        self.connected.then(|| 1.0 - d.sqrt() / 30.0)
    }

    /// OnCollision against an entity (not a barrier, shield or grease
    /// field): true when it should be queued for teleport. Contact keeps
    /// refreshing the 0.1 s ignore entry, so an arrival does not bounce.
    pub fn touch(&mut self, handle: u16) -> bool {
        if !self.connected {
            return false;
        }
        let queue = !self.ignored.contains(handle);
        self.ignored.set(handle, IGNORE_TIME);
        queue
    }

    /// GetOutPos for an entity at `position` touching `entry` (true = A).
    pub fn out_position(&self, entry_is_a: bool, position: Vector) -> Vector {
        let (from, to) = if entry_is_a {
            (self.a.position, self.b.position)
        } else {
            (self.b.position, self.a.position)
        };
        std::array::from_fn(|i| to[i] + (position[i] - from[i]) * OUT_SCALE)
    }

    /// PortalEntity.Update for A or B: the ignore list ages on A; the
    /// effect follows connection changes; alpha rises to 1.
    pub fn update_entity(&mut self, is_a: bool, dt: f32) -> Option<EffectChange> {
        if is_a {
            self.ignored.update(dt);
        }
        let connected = self.connected;
        let p = if is_a { &mut self.a } else { &mut self.b };
        let change = (p.connected != connected).then(|| {
            p.connected = connected;
            EffectChange::Restart { connected }
        });
        p.effect_alpha = (p.effect_alpha + dt).min(1.0);
        change
    }
}
