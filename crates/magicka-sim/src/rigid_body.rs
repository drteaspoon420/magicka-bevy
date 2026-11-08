//! JigLibX Body integration stages. See research/rigid-body-integration.md.
use crate::impact_motion::Vector;

/// Row-major XNA matrix; orientation and inertia tensors have M44 = 1.
pub type Matrix = [[f32; 4]; 4];
pub const IDENTITY: Matrix = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];

pub(crate) fn multiply(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j] + a[i][3] * b[3][j]
        })
    })
}
pub(crate) fn transform(v: Vector, m: Matrix) -> Vector {
    std::array::from_fn(|i| v[0] * m[0][i] + v[1] * m[1][i] + v[2] * m[2][i] + m[3][i])
}
fn sum(a: Vector, b: Vector) -> Vector {
    std::array::from_fn(|i| a[i] + b[i])
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TransformRate {
    pub velocity: Vector,
    pub angular_velocity: Vector,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyTransform {
    pub position: Vector,
    pub orientation: Matrix,
}
impl Default for BodyTransform {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            orientation: IDENTITY,
        }
    }
}
impl BodyTransform {
    pub fn apply_rate(&mut self, rate: TransformRate, dt: f32) {
        self.position = sum(self.position, rate.velocity.map(|v| v * dt));
        let w = rate.angular_velocity;
        let speed = ((w[0] * w[0] + w[1] * w[1] + w[2] * w[2]) as f64).sqrt() as f32;
        if speed > 0. {
            let [x, y, z] = w.map(|v| v / speed);
            let angle = speed * dt;
            let c = (angle as f64).cos() as f32;
            let s = (angle as f64).sin() as f32;
            let (xx, yy, zz, xy, xz, yz) = (x * x, y * y, z * z, x * y, x * z, y * z);
            let rotation = [
                [
                    xx + c * (1. - xx),
                    xy - c * xy + s * z,
                    xz - c * xz - s * y,
                    0.,
                ],
                [
                    xy - c * xy - s * z,
                    yy + c * (1. - yy),
                    yz - c * yz + s * x,
                    0.,
                ],
                [
                    xz - c * xz + s * y,
                    yz - c * yz - s * x,
                    zz + c * (1. - zz),
                    0.,
                ],
                [0., 0., 0., 1.],
            ];
            self.orientation = multiply(self.orientation, rotation);
        }
    }
}

/// Required host state, deliberately without invented mass or gravity defaults.
#[derive(Clone, Debug)]
pub struct RigidBody {
    pub current: BodyTransform,
    pub old: BodyTransform,
    pub rate: TransformRate,
    pub auxiliary: TransformRate,
    pub inverse_mass: f32,
    pub mass: f32,
    pub apply_gravity: bool,
    pub force: Vector,
    pub torque: Vector,
    pub body_inertia: Matrix,
    pub body_inverse_inertia: Matrix,
    pub world_inertia: Matrix,
    pub world_inverse_inertia: Matrix,
    pub active: bool,
    pub immovable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GravityAxis {
    None,
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gravity {
    pub vector: Vector,
    pub magnitude: f32,
    pub axis: GravityAxis,
}
impl Gravity {
    /// PhysicsSystem.set_Gravity, including the overwritten equal-axis sentinel.
    pub fn new(vector: Vector) -> Self {
        let mut axis = 0;
        if vector[1].abs() > vector[0].abs() {
            axis = 1;
        }
        if vector[2].abs() > vector[axis].abs() {
            axis = 2;
        }
        Self {
            vector,
            magnitude: ((vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2])
                as f64)
                .sqrt() as f32,
            axis: [GravityAxis::X, GravityAxis::Y, GravityAxis::Z][axis],
        }
    }
    pub fn magicka() -> Self {
        Self::new([0., -9.82 * 2., 0.])
    }
}

impl RigidBody {
    pub fn clear_forces(&mut self) {
        self.force = [0.; 3];
        self.torque = [0.; 3];
    }

    /// `None` represents no current PhysicsSystem, not zero gravity.
    pub fn add_gravity_to_external_force(&mut self, gravity: Option<Gravity>) {
        if self.apply_gravity {
            if let Some(gravity) = gravity {
                self.force = sum(self.force, gravity.vector.map(|g| g * self.mass));
            }
        }
    }

    pub fn add_external_forces(&mut self, gravity: Option<Gravity>) {
        self.clear_forces();
        self.add_gravity_to_external_force(gravity);
    }

    pub fn set_mass(&mut self, mass: f32, gravity: Option<Gravity>) {
        self.mass = mass;
        self.inverse_mass = 1. / mass;
        self.add_external_forces(gravity);
    }

    pub fn set_inverse_mass(&mut self, inverse_mass: f32) {
        self.inverse_mass = inverse_mass;
        self.mass = 1. / inverse_mass;
    }

    pub fn update_velocity(&mut self, dt: f32, has_contacts: bool) {
        if !self.immovable && self.active {
            self.rate.velocity = sum(
                self.rate.velocity,
                self.force.map(|f| f * (dt * self.inverse_mass)),
            );
            let delta = transform(self.torque.map(|t| t * dt), self.world_inverse_inertia);
            self.rate.angular_velocity = sum(self.rate.angular_velocity, delta);
            if has_contacts {
                self.rate.angular_velocity = self.rate.angular_velocity.map(|w| w * 0.99);
            }
        }
    }

    /// The host forwards this pair to CollisionSkin.SetTransform when a skin exists.
    pub fn update_position(&mut self, dt: f32) -> Option<(BodyTransform, BodyTransform)> {
        if self.immovable || !self.active {
            return None;
        }
        self.integrate(self.rate, dt);
        Some((self.old, self.current))
    }

    pub fn update_position_with_aux(
        &mut self,
        dt: f32,
        gravity_axis: GravityAxis,
    ) -> Option<(BodyTransform, BodyTransform)> {
        if self.immovable || !self.active {
            self.auxiliary = TransformRate::default();
            return None;
        }
        let parallel = match gravity_axis {
            GravityAxis::None => None,
            GravityAxis::X => Some(0),
            GravityAxis::Y => Some(1),
            GravityAxis::Z => Some(2),
        };
        if let Some(parallel) = parallel {
            for i in 0..3 {
                if i != parallel {
                    self.auxiliary.velocity[i] *= 0.1;
                }
            }
        }
        let combined = TransformRate {
            velocity: sum(self.rate.velocity, self.auxiliary.velocity),
            angular_velocity: sum(self.rate.angular_velocity, self.auxiliary.angular_velocity),
        };
        self.integrate(combined, dt);
        self.auxiliary = TransformRate::default();
        Some((self.old, self.current))
    }

    fn integrate(&mut self, rate: TransformRate, dt: f32) {
        let momentum = transform(self.rate.angular_velocity, self.world_inertia);
        self.current.apply_rate(rate, dt);
        let orientation = self.current.orientation;
        let inverse = std::array::from_fn(|i| std::array::from_fn(|j| orientation[j][i]));
        self.world_inverse_inertia =
            multiply(multiply(inverse, self.body_inverse_inertia), orientation);
        self.world_inertia = multiply(multiply(inverse, self.body_inertia), orientation);
        self.rate.angular_velocity = transform(momentum, self.world_inverse_inertia);
    }
}
