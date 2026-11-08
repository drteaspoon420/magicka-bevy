//! EtherealClone's native parent gate and square placement. The custom player
//! caster extension is separate from native eligibility (ethereal-clone.md).
use crate::impact_motion::Vector;

pub const PARENT: &str = "dungeons_human_adventurer";
pub const TEMPLATE: &str = "dungeons_human_adventurer_clone";
pub const SPREAD: f64 = 12.0;

pub fn native_eligible(parent: &str) -> bool {
    parent == PARENT
}

/// SpawnClone creates an offset whose Y equals the owner's Y, then adds that
/// offset to the owner position before the navmesh query. Keep the doubled Y.
pub fn position(origin: Vector, x: f64, z: f64) -> Vector {
    [
        origin[0] + ((x - 0.5) * SPREAD) as f32,
        origin[1] + origin[1],
        origin[2] + ((z - 0.5) * SPREAD) as f32,
    ]
}
