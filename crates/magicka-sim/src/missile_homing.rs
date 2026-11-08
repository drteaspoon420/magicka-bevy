//! MissileEntity.Update's homing step (research/judgement-spray.md).
use crate::impact_motion::Vector;

/// MissileEntity's homing state: target, power (0 = off), tolerance and
/// mLastKnownTargetPosition (zeroed by Initialize, so a target-less homing
/// missile steers towards the world origin).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homing {
    pub target: Option<u16>,
    pub power: f32,
    pub tolerance: f32,
    pub last_known: Vector,
}

impl Homing {
    /// Initialize(owner, target, homing, ...): tolerance -0.75.
    pub fn new(target: Option<u16>, power: f32) -> Self {
        Self {
            target,
            power,
            tolerance: -0.75,
            last_known: [0.0; 3],
        }
    }

    /// The homing block of MissileEntity.Update. `target` is the target's
    /// position and radius while it has a body. Returns the new velocity.
    /// The axis is the unnormalised cross product, as in the original:
    /// Quaternion.CreateFromAxisAngle and Vector3.Transform then use a
    /// non-unit quaternion.
    pub fn steer(
        &mut self,
        target: Option<(Vector, f32)>,
        position: Vector,
        velocity: Vector,
        dt: f32,
    ) -> Vector {
        if self.power <= 0.0 {
            return velocity;
        }
        if let Some((p, radius)) = target {
            self.last_known = [p[0], p[1] + radius, p[2]];
        }
        let len_sq =
            velocity[0] * velocity[0] + velocity[1] * velocity[1] + velocity[2] * velocity[2];
        if len_sq <= 1e-6 {
            return velocity;
        }
        let v = normalize(velocity);
        let to = normalize([
            self.last_known[0] - position[0],
            self.last_known[1] - position[1],
            self.last_known[2] - position[2],
        ]);
        let axis = [
            v[1] * to[2] - v[2] * to[1],
            v[2] * to[0] - v[0] * to[2],
            v[0] * to[1] - v[1] * to[0],
        ];
        let dot = v[0] * to[0] + v[1] * to[1] + v[2] * to[2];
        if dot <= self.tolerance {
            self.power = 0.0;
        }
        let half = dt * 10.0 * self.power * 0.5;
        let (s, w) = (half.sin(), half.cos());
        transform(velocity, [axis[0] * s, axis[1] * s, axis[2] * s, w])
    }
}

fn normalize(v: Vector) -> Vector {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / l)
}

/// XNA Vector3.Transform(Vector3, Quaternion) (no normalisation).
pub fn transform(v: Vector, q: [f32; 4]) -> Vector {
    let (x2, y2, z2) = (q[0] + q[0], q[1] + q[1], q[2] + q[2]);
    let (wx, wy, wz) = (q[3] * x2, q[3] * y2, q[3] * z2);
    let (xx, xy, xz) = (q[0] * x2, q[0] * y2, q[0] * z2);
    let (yy, yz, zz) = (q[1] * y2, q[1] * z2, q[2] * z2);
    [
        v[0] * (1.0 - yy - zz) + v[1] * (xy - wz) + v[2] * (xz + wy),
        v[0] * (xy + wz) + v[1] * (1.0 - xx - zz) + v[2] * (yz - wx),
        v[0] * (xz - wy) + v[1] * (yz + wx) + v[2] * (1.0 - xx - yy),
    ]
}
