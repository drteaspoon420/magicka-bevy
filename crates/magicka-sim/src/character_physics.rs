//! Character physics: Magicka CharacterBody control law on a compact
//! reimplementation of the JigLibX subset Magicka uses for characters.
//! See research/character-physics.md for evidence and divergences.
//!
//! Recovered gameplay rules are transcribed: CharacterBody.AddExternalForces,
//! the ground/fall-damage part of ProcessCollisionPoints, Magicka's
//! CustomCapsuleStaticMesh step/slope classification, the JigLibX Normal solver
//! order and its impulse/min-separation arithmetic. Character bodies have zero
//! inverse inertia (CharacterBody ctor), so all angular solver terms vanish and
//! are omitted. Generic geometry (closest points) is reimplemented; ties and
//! rounding are not bit-identical to JigLibX.
pub use crate::geometry::{point_triangle_distance_sq, segment_triangle_distance_sq};
use crate::geometry::{segment_triangle_hit, tri_point};
use crate::impact_motion::Vector;

pub const GRAVITY: Vector = [0.0, -9.82 * 2.0, 0.0];
pub const ALLOWED_PENETRATION: f32 = 0.001;
pub const COLLISION_TOLERANCE: f32 = 0.05;
pub const STEP_SIZE: f32 = 0.51;
const NUM_COLLISION_ITERATIONS: usize = 2;
const NUM_CONTACT_ITERATIONS: usize = 4;
const MAX_VEL_MAG: f32 = 5.0;
const VEL_MAX: f32 = 100.0;
const MAX_TIME_SKYWALKER: f32 = 0.2;
/// CLR `float.Epsilon`: the smallest positive subnormal, not machine epsilon.
const CLR_FLOAT_EPSILON: f32 = f32::from_bits(1);

fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: Vector, s: f32) -> Vector {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn length_squared(a: Vector) -> f32 {
    dot(a, a)
}
/// JiggleMath.NormalizeSafe: leaves near-zero vectors unchanged.
fn normalize_safe(a: Vector) -> Vector {
    let l = length_squared(a);
    if l == 0.0 {
        return a;
    }
    scale(a, 1.0 / l.sqrt())
}

/// World-space collision triangle produced by a static geometry provider.
#[derive(Clone, Copy, Debug)]
pub struct CollisionTriangle {
    pub vertices: [Vector; 3],
    /// Unit plane normal, normalize(cross(v1 - v0, v2 - v0)).
    pub normal: Vector,
    /// Provider-unique key of the owning (skin, primitive) pair. Sphere sweeps
    /// produce one contact per primitive, as in the original detector loop.
    pub primitive: u32,
    /// ProcessCollisionPoints records the primitive *index within its skin*
    /// as CollisionMaterials: static layer i reports i; animated parts and
    /// force fields (single-primitive skins) always report 0 (Generic).
    pub ground_material: u8,
    pub water_ice: bool,
    pub water_surface: bool,
}
impl CollisionTriangle {
    pub fn new(a: Vector, b: Vector, c: Vector, primitive: u32, ground_material: u8) -> Self {
        Self {
            vertices: [a, b, c],
            normal: normalize_safe(cross(sub(b, a), sub(c, a))),
            primitive,
            ground_material,
            water_ice: false,
            water_surface: false,
        }
    }
    fn plane_distance(&self, p: Vector) -> f32 {
        dot(self.normal, sub(p, self.vertices[0]))
    }
}

/// Static level collision. Providers return candidates whose bounds touch
/// `min..max` in the original octree's candidate order where available.
pub trait StaticGeometry {
    fn triangles_in_box(&self, min: Vector, max: Vector, out: &mut Vec<CollisionTriangle>);
}

/// Brute-force triangle soup (tests and small dynamic sets).
impl StaticGeometry for Vec<CollisionTriangle> {
    fn triangles_in_box(&self, min: Vector, max: Vector, out: &mut Vec<CollisionTriangle>) {
        for t in self {
            let lo: Vector = std::array::from_fn(|i| {
                t.vertices[0][i].min(t.vertices[1][i]).min(t.vertices[2][i])
            });
            let hi: Vector = std::array::from_fn(|i| {
                t.vertices[0][i].max(t.vertices[1][i]).max(t.vertices[2][i])
            });
            if (0..3).all(|i| hi[i] >= min[i] && lo[i] <= max[i]) {
                out.push(*t);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Orientation (XNA Matrix/Quaternion subset; yaw-only in practice)

pub type Quaternion = [f32; 4];

/// XNA Matrix rows Right/Up/Backward as columns of a 3x3 basis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orientation {
    pub right: Vector,
    pub up: Vector,
    pub backward: Vector,
}
impl Default for Orientation {
    fn default() -> Self {
        Self::IDENTITY
    }
}
impl Orientation {
    pub const IDENTITY: Self = Self {
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        backward: [0.0, 0.0, 1.0],
    };
    pub fn forward(&self) -> Vector {
        scale(self.backward, -1.0)
    }
    /// Matrix with Forward = dir, Up = up, Right = cross(dir, up) (Turn).
    pub fn from_forward_up(dir: Vector, up: Vector) -> Self {
        Self {
            right: cross(dir, up),
            up,
            backward: scale(dir, -1.0),
        }
    }
    /// XNA Quaternion.CreateFromRotationMatrix.
    pub fn to_quaternion(&self) -> Quaternion {
        let (m11, m12, m13) = (self.right[0], self.right[1], self.right[2]);
        let (m21, m22, m23) = (self.up[0], self.up[1], self.up[2]);
        let (m31, m32, m33) = (self.backward[0], self.backward[1], self.backward[2]);
        let trace = m11 + m22 + m33;
        if trace > 0.0 {
            let s = (trace + 1.0).sqrt();
            let w = s * 0.5;
            let s = 0.5 / s;
            [(m23 - m32) * s, (m31 - m13) * s, (m12 - m21) * s, w]
        } else if m11 >= m22 && m11 >= m33 {
            let s = (1.0 + m11 - m22 - m33).sqrt();
            let inv = 0.5 / s;
            [
                0.5 * s,
                (m12 + m21) * inv,
                (m13 + m31) * inv,
                (m23 - m32) * inv,
            ]
        } else if m22 > m33 {
            let s = (1.0 + m22 - m11 - m33).sqrt();
            let inv = 0.5 / s;
            [
                (m21 + m12) * inv,
                0.5 * s,
                (m32 + m23) * inv,
                (m31 - m13) * inv,
            ]
        } else {
            let s = (1.0 + m33 - m11 - m22).sqrt();
            let inv = 0.5 / s;
            [
                (m31 + m13) * inv,
                (m32 + m23) * inv,
                0.5 * s,
                (m12 - m21) * inv,
            ]
        }
    }
    /// XNA Matrix.CreateFromQuaternion.
    pub fn from_quaternion(q: Quaternion) -> Self {
        let [x, y, z, w] = q;
        let (xx, yy, zz) = (x * x, y * y, z * z);
        let (xy, zw, zx, yw, yz, xw) = (x * y, z * w, z * x, y * w, y * z, x * w);
        Self {
            right: [1.0 - 2.0 * (yy + zz), 2.0 * (xy + zw), 2.0 * (zx - yw)],
            up: [2.0 * (xy - zw), 1.0 - 2.0 * (zz + xx), 2.0 * (yz + xw)],
            backward: [2.0 * (zx + yw), 2.0 * (yz - xw), 1.0 - 2.0 * (yy + xx)],
        }
    }
}

/// XNA Quaternion.Lerp: sign-corrected linear blend, then normalize.
pub fn quaternion_lerp(a: Quaternion, b: Quaternion, amount: f32) -> Quaternion {
    let inv = 1.0 - amount;
    let d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let r: Quaternion = if d >= 0.0 {
        std::array::from_fn(|i| inv * a[i] + amount * b[i])
    } else {
        std::array::from_fn(|i| inv * a[i] - amount * b[i])
    };
    let n = 1.0 / (r[0] * r[0] + r[1] * r[1] + r[2] * r[2] + r[3] * r[3]).sqrt();
    r.map(|v| v * n)
}

// ---------------------------------------------------------------------------
// Character body

/// Facts the body reads from its owning Character each step.
#[derive(Clone, Copy, Debug)]
pub struct OwnerFacts {
    pub entangled: bool,
    pub levitating: bool,
    pub knocked_down: bool,
    pub turn_speed: f32,
    pub water_depth: f32,
    pub hit_points: f32,
    pub can_be_knocked_down: bool,
    pub ignore_collision_damage: bool,
    /// False while Character.OnCollision rejects owner-less skins
    /// (ethereal or collision-ignore timer).
    pub collides_with_level: bool,
}
impl Default for OwnerFacts {
    fn default() -> Self {
        Self {
            entangled: false,
            levitating: false,
            knocked_down: false,
            turn_speed: 0.0,
            water_depth: 0.0,
            hit_points: 1.0,
            can_be_knocked_down: true,
            ignore_collision_damage: false,
            collides_with_level: true,
        }
    }
}

/// Side effects ProcessCollisionPoints asks the owner to perform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BodyEvent {
    /// One WaterMesh contact: owner applies the self-shield/status filters.
    LiquidStatus {
        depth: f32,
        lava: bool,
    },
    /// Earth fall/impact damage; `knockdown` when amount > 100.
    ImpactDamage {
        amount: f32,
        knockdown: bool,
    },
    KnockDown,
    /// Crossed the scene kill plane (GameScene.mKillPlane callback).
    KillPlane,
    /// ProcessCollisionPoints: Character.Drown (HitPoints > 0, a WaterMesh
    /// contact and WaterDepth >= -HeightOffset), before water damping.
    /// `lava` when the ground material is Lava (9).
    Drown {
        depth: f32,
        lava: bool,
    },
}

#[derive(Clone, Debug)]
pub struct CharacterBody {
    // JigLibX Body state
    pub position: Vector,
    pub orientation: Orientation,
    pub velocity: Vector,
    force: Vector,
    pub mass: f32,
    pub apply_gravity: bool,
    old_position: Vector,
    stored: (Vector, Orientation, Vector),
    inactive_time: f32,
    // Capsule primitive (local position y = -length/2, axis +Y)
    pub radius: f32,
    pub length: f32,
    // CharacterBody state
    pub is_flying: bool,
    pub fly_height: f32,
    additional_force: Vector,
    last_position_on_ground: Vector,
    pub ground_contact_normal: Vector,
    time_since_ground_contact: f32,
    is_touching_ground: bool,
    is_leaving_ground: bool,
    is_pushed: bool,
    impulse: Vector,
    pub allow_move: bool,
    pub allow_rotate: bool,
    moving: bool,
    is_on_ice: bool,
    is_in_water: bool,
    is_on_grease: bool,
    pub is_jumping: bool,
    pub slow: f32,
    movement: Vector,
    desired_direction: Vector,
    pub max_speed: f32,
    speed_multiplier: f32,
    pub ground_material: u8,
    pub run_backwards: bool,
    pub owner: OwnerFacts,
    /// In the physics system (false after the owner is removed/terminated).
    pub active: bool,
    /// A host engine owns this body's movement and collision. It remains a
    /// live combat target; the local solver must not move it a second time.
    pub externally_driven: bool,
}

impl CharacterBody {
    /// Character.Initialize(template) + CharacterBody ctor defaults.
    pub fn new(radius: f32, length: f32, mass: f32, max_speed: f32) -> Self {
        let mut body = Self {
            position: [0.0; 3],
            orientation: Orientation::IDENTITY,
            velocity: [0.0; 3],
            force: [0.0; 3],
            mass,
            apply_gravity: true,
            old_position: [0.0; 3],
            stored: ([0.0; 3], Orientation::IDENTITY, [0.0; 3]),
            inactive_time: 0.0,
            radius,
            length,
            is_flying: false,
            fly_height: 2.0,
            additional_force: [0.0; 3],
            last_position_on_ground: [0.0; 3],
            ground_contact_normal: [0.0; 3],
            time_since_ground_contact: 0.0,
            is_touching_ground: false,
            is_leaving_ground: false,
            is_pushed: false,
            impulse: [0.0; 3],
            allow_move: true,
            allow_rotate: true,
            moving: false,
            is_on_ice: false,
            is_in_water: false,
            is_on_grease: false,
            is_jumping: false,
            slow: 1.0,
            movement: [0.0; 3],
            desired_direction: [0.0, 0.0, -1.0],
            max_speed,
            speed_multiplier: 1.0,
            ground_material: 0,
            run_backwards: false,
            owner: OwnerFacts::default(),
            active: true,
            externally_driven: false,
        };
        body.enable();
        body
    }

    /// CharacterBody.EnableBody resets.
    pub fn enable(&mut self) {
        self.movement = [0.0; 3];
        self.moving = false;
        self.is_touching_ground = true;
        self.is_leaving_ground = false;
        self.is_pushed = false;
        self.impulse = [0.0; 3];
        self.allow_move = true;
        self.allow_rotate = true;
        self.is_on_ice = false;
        self.is_on_grease = false;
        self.is_jumping = false;
    }

    /// The JigLibX Body.Position setter (Portal teleports): the velocity is
    /// kept. The old position follows so the next step does not sweep the
    /// jump (INFERRED; research/portal.md).
    pub fn set_position(&mut self, position: Vector) {
        self.position = position;
        self.old_position = position;
    }

    /// Supply authoritative host physics while retaining the spell-facing
    /// capsule, input, animation and combat state. Synchronize both current
    /// and old positions, including on spawn, travel and capsule changes.
    pub fn sync_external_motion(
        &mut self,
        position: Vector,
        orientation: Orientation,
        velocity: Vector,
        grounded: bool,
    ) {
        self.externally_driven = true;
        self.set_position(position);
        self.orientation = orientation;
        self.velocity = velocity;
        self.is_touching_ground = grounded;
        self.time_since_ground_contact = if grounded { 0.0 } else { 0.3 };
        self.moving = length_squared([velocity[0], 0.0, velocity[2]]) > 1e-6;
    }

    /// Drain spell impulses and vortex velocity additions for an external
    /// movement host. Mass conversion and entanglement gates have already
    /// run in AddImpulse/AdditionalForce; ordinary walking and gravity belong
    /// to the host. Match AddExternalForces' velocity-dependent force damping.
    pub fn take_external_velocity_change(&mut self) -> Vector {
        let mut delta = self.impulse;
        self.impulse = [0.0; 3];
        if length_squared(self.additional_force) > 1e-6 {
            let v = add(self.velocity, delta);
            if length_squared(v) > 1e-6 {
                let n = self.additional_force_length();
                let d = dot(scale(self.additional_force, 1.0 / n), scale(v, 1.0 / n));
                self.additional_force = scale(self.additional_force, (1.0 - d).clamp(0.0, 1.0));
            }
            delta = add(delta, self.additional_force);
        }
        self.additional_force = [0.0; 3];
        self.is_pushed = false;
        self.is_leaving_ground = false;
        delta
    }

    /// Body.MoveTo: teleport and zero velocity.
    pub fn move_to(&mut self, position: Vector, orientation: Orientation) {
        self.position = position;
        self.orientation = orientation;
        self.velocity = [0.0; 3];
        self.old_position = position;
        self.inactive_time = 0.0;
    }

    /// Geometry submitted to collision detectors before and after this step.
    pub fn capsule_motion(&self) -> crate::capsule_pair::CapsuleMotion {
        let axis = |position| {
            let (start, end) = self.capsule(position);
            crate::spray_pulse::Segment {
                origin: start,
                delta: sub(end, start),
            }
        };
        crate::capsule_pair::CapsuleMotion {
            old_axis: axis(self.old_position),
            new_axis: axis(self.position),
            old_radius: self.radius,
            new_radius: self.radius,
            old_body_position: self.old_position,
        }
    }

    /// Character.SetCapsuleForm changes dimensions and moves the body center
    /// to preserve the feet. Its Body.MoveTo also zeroes velocity and copies
    /// current position to old state; changing fields alone misses that.
    pub fn set_capsule_form(&mut self, length: f32, radius: f32) {
        let mut position = self.position;
        position[1] -= (self.length - length) * 0.5;
        position[1] -= self.radius - radius;
        self.length = length;
        self.radius = radius;
        self.move_to(position, self.orientation);
    }

    /// Character.HeightOffset (feet relative to body position).
    pub fn height_offset(&self) -> f32 {
        -self.radius - 0.5 * self.length
    }
    /// CharacterBody.IsInWater (a WaterMesh contact this step).
    pub fn is_in_water(&self) -> bool {
        self.is_in_water
    }
    pub fn feet(&self) -> Vector {
        add(self.position, [0.0, self.height_offset(), 0.0])
    }
    pub fn movement(&self) -> Vector {
        self.movement
    }
    pub fn desired_direction(&self) -> Vector {
        self.desired_direction
    }
    pub fn direction(&self) -> Vector {
        self.orientation.forward()
    }
    pub fn is_touching_ground(&self) -> bool {
        self.is_touching_ground
    }
    pub fn is_leaving_ground(&self) -> bool {
        self.is_leaving_ground
    }
    pub fn moving(&self) -> bool {
        self.moving
    }
    pub fn is_pushed(&self) -> bool {
        self.is_pushed
    }
    pub fn last_position_on_ground(&self) -> Vector {
        self.last_position_on_ground
    }
    pub fn speed_multiplier(&self) -> f32 {
        self.speed_multiplier
    }
    /// SpeedMultiplier setter clamps to [0, 100].
    pub fn set_speed_multiplier(&mut self, value: f32) {
        // Math.Min/Math.Max propagate NaN, as clamp does.
        self.speed_multiplier = value.clamp(0.0, 100.0);
    }
    pub fn set_on_grease(&mut self, value: bool) {
        self.is_on_grease = value;
    }
    pub fn is_on_ice(&self) -> bool {
        self.is_on_ice
    }

    /// CharacterBody.Movement setter.
    pub fn set_movement(&mut self, mut value: Vector) {
        value[1] = 0.0;
        let l = length_squared(value);
        if l > 1e-6 {
            let l = l.sqrt();
            let inv = 1.0 / l;
            self.desired_direction = scale(value, inv);
            self.movement = if l > 1.0 {
                self.desired_direction
            } else {
                value
            };
        } else {
            self.movement = [0.0; 3];
            self.moving = false;
        }
    }
    /// CharacterBody.DesiredDirection setter.
    pub fn set_desired_direction(&mut self, mut value: Vector) {
        value[1] = 0.0;
        let l = length_squared(value);
        if l > 1e-6 {
            self.desired_direction = scale(value, 1.0 / l.sqrt());
        }
    }
    /// AdditionalForce setter gate.
    pub fn set_additional_force(&mut self, value: Vector) {
        if !self.owner.entangled && self.speed_multiplier > 1e-6 {
            self.additional_force = value;
        }
    }
    /// CharacterBody.AddImpulse (impulse in momentum units).
    pub fn add_impulse(&mut self, impulse: Vector) {
        if self.owner.entangled {
            return;
        }
        self.impulse = add(self.impulse, scale(impulse, 1.0 / self.mass));
        self.is_pushed = true;
        if self.impulse[1] > 0.5 {
            self.is_leaving_ground = true;
        }
    }
    /// CharacterBody.AddImpulseVelocity.
    pub fn add_impulse_velocity(&mut self, impulse: Vector) {
        if self.owner.entangled {
            return;
        }
        self.impulse = add(self.impulse, impulse);
        self.is_pushed = true;
        if self.impulse[1] > 2.7 {
            self.is_leaving_ground = true;
        }
    }
    /// CharacterBody.AddJump.
    pub fn add_jump(&mut self, impulse: Vector) {
        if self.owner.entangled
            || !self.is_touching_ground
            || self.is_leaving_ground
            || !self.allow_move
        {
            return;
        }
        self.impulse = add(self.impulse, impulse);
        self.is_jumping = true;
        self.is_leaving_ground = true;
        self.velocity = [0.0; 3];
    }

    fn turn(&mut self, mut dir: Vector, turn_speed: f32, dt: f32) {
        dir[1] = 0.0;
        let target = Orientation::from_forward_up(dir, [0.0, 1.0, 0.0]);
        let q0 = self.orientation.to_quaternion();
        let q1 = target.to_quaternion();
        let q = quaternion_lerp(q0, q1, (dt * turn_speed).clamp(0.0, 1.0));
        self.orientation = Orientation::from_quaternion(q);
    }

    /// CharacterBody.AddExternalForces. `fly_ground` supplies the NavMesh
    /// nearest position for flying bodies.
    pub fn add_external_forces(&mut self, dt: f32, fly_ground: Option<Vector>) {
        self.force = [0.0; 3];
        let mut v = self.velocity;
        if self.is_flying {
            self.time_since_ground_contact = 0.0;
            if let Some(mut ground) = fly_ground {
                ground[1] += self.fly_height;
                let num = 0.2 * (ground[1] - self.position[1]) - 0.02 * v[1];
                v[1] += num;
            }
        } else {
            self.time_since_ground_contact += dt;
        }
        self.is_touching_ground =
            self.time_since_ground_contact <= MAX_TIME_SKYWALKER || self.owner.levitating;
        let mut moved = false;
        v = add(v, self.impulse);
        self.impulse = [0.0; 3];
        self.velocity = v;
        if self.is_leaving_ground {
            self.time_since_ground_contact = 0.3;
            self.is_touching_ground = false;
            self.is_leaving_ground = false;
        } else if self.is_pushed {
            if self.is_touching_ground {
                if self.owner.knocked_down {
                    self.is_pushed = false;
                    v = [0.0; 3];
                } else {
                    // The inner touching-ground check is always true here.
                    let x: f64 = if !self.is_on_grease && !self.is_on_ice && !self.owner.levitating
                    {
                        0.02
                    } else {
                        0.1
                    };
                    v = scale(v, x.powf(dt as f64) as f32);
                    if v[1] > 0.0 {
                        v[1] = 0.0;
                    }
                    if length_squared(v) < self.max_speed * self.max_speed * 0.33 {
                        self.is_pushed = false;
                    }
                }
            }
            self.velocity = v;
            if length_squared(v) > 1e-6 && v[0] + v[2] > CLR_FLOAT_EPSILON {
                let back = normalize_safe(scale(v, -1.0));
                self.turn(back, self.owner.turn_speed, dt);
            }
        } else if self.is_touching_ground {
            if !self.is_on_ice && !self.is_on_grease && !self.owner.levitating {
                v[0] = 0.0;
                v[2] = 0.0;
            }
            if !self.is_flying && !self.is_jumping && v[1] > 0.0 {
                v[1] *= 0.8;
            }
            self.is_jumping = false;
            self.is_pushed = false;
            let mut wanted = self.movement;
            if self.is_in_water {
                let f = (1.0 - self.owner.water_depth * 0.5).clamp(0.5, 1.0);
                wanted = scale(wanted, f);
            }
            let mut delta = sub(scale(wanted, self.max_speed * self.speed_multiplier), v);
            delta[1] = 0.0;
            if self.is_on_ice || self.is_on_grease || self.owner.levitating {
                let f = (dt * 1.0).min(1.0);
                delta[0] *= f;
                delta[2] *= f;
            }
            delta[0] *= self.slow;
            delta[2] *= self.slow;
            if self.allow_move {
                v = add(delta, v);
                moved = length_squared([v[0], 0.0, v[2]]) >= 1e-6;
            }
            if self.allow_rotate {
                let dir = if self.run_backwards {
                    scale(self.desired_direction, -1.0)
                } else {
                    self.desired_direction
                };
                self.turn(dir, self.owner.turn_speed, dt);
            }
            self.velocity = v;
        }
        if length_squared(self.additional_force) > 1e-6 {
            let mut v = self.velocity;
            if length_squared(v) > 1e-6 {
                // Original divides velocity by |force| (not |velocity|).
                let n = self.additional_force_length();
                let vv = scale(v, 1.0 / n);
                let fv = scale(self.additional_force, 1.0 / n);
                let d = dot(fv, vv);
                self.additional_force = scale(self.additional_force, (1.0 - d).clamp(0.0, 1.0));
            }
            v = add(v, self.additional_force);
            self.velocity = v;
            self.additional_force = [0.0; 3];
        }
        self.slow = 1.0;
        self.moving = moved;
        if self.apply_gravity {
            self.force = add(self.force, scale(GRAVITY, self.mass));
        }
        self.is_on_grease = false;
    }
    fn additional_force_length(&self) -> f32 {
        length_squared(self.additional_force).sqrt()
    }

    fn capsule(&self, position: Vector) -> (Vector, Vector) {
        // Local capsule position (0,-L/2,0) with RotateX(-90°): end = +Y*L.
        // The body's yaw does not change a vertical axis.
        let start = add(position, [0.0, -0.5 * self.length, 0.0]);
        (start, add(start, [0.0, self.length, 0.0]))
    }
}

// ---------------------------------------------------------------------------
// Collision detection (CustomCapsuleStaticMesh)

#[derive(Clone, Copy, Debug)]
struct ContactPoint {
    /// Contact position relative to the body's old position (water depth
    /// and liquid handling are pending consumers).
    #[allow(dead_code)]
    r0: Vector,
    world_point: Vector,
    initial_penetration: f32,
    min_separation_vel: f32,
    denominator: f32,
}

/// One CollisionInfo between a character capsule (Skin0) and a level
/// primitive. Every static-geometry skin (LevelModel, AnimatedLevelPart,
/// ForceField) is tagged with the LevelModel, so all contacts are level
/// contacts for ProcessCollisionPoints.
#[derive(Clone, Debug)]
struct Contact {
    body: usize,
    other: Option<usize>,
    /// Only level/ice contacts feed ground detection and terrain impact damage.
    level_contact: bool,
    primitive: Option<u32>,
    /// Unit direction pointing toward the body (DirToBody0).
    normal: Vector,
    points: Vec<ContactPoint>,
    ground_material: u8,
    water_ice: bool,
    water_surface: bool,
    satisfied: bool,
}

/// CustomCapsuleStaticMesh.CollDetectCapsuleStaticMeshOverlap. One contact
/// per triangle, as in the original (it notifies inside the loop).
fn capsule_mesh_overlap(
    body: &CharacterBody,
    body_index: usize,
    old_pos: Vector,
    new_pos: Vector,
    candidates: &[CollisionTriangle],
    out: &mut Vec<Contact>,
) {
    let radius = body.radius;
    let num = COLLISION_TOLERANCE + radius;
    let num2 = num * num;
    let (p_new, e_new) = body.capsule(new_pos);
    let (p_old, e_old) = body.capsule(old_pos);
    let old_bottom = p_old[1].min(e_old[1]) - radius;
    let new_bottom = p_new[1].min(e_new[1]) - radius;
    let vdir = normalize_safe(body.velocity);
    for tri in candidates {
        let d_start = tri.plane_distance(p_new);
        let d_end = tri.plane_distance(e_new);
        if (d_start > num && d_end > num) || (d_start < -num && d_end < -num) {
            continue;
        }
        let (dist_sq, ..) = segment_triangle_distance_sq(p_new, sub(e_new, p_new), &tri.vertices);
        if dist_sq >= num2 {
            continue;
        }
        let (dist_sq, seg_t, s, t) =
            segment_triangle_distance_sq(p_old, sub(e_old, p_old), &tri.vertices);
        let dist = dist_sq.sqrt();
        let mut penetration = radius - dist;
        let point = tri_point(&tri.vertices, s, t);
        let seg_point = add(p_old, scale(sub(e_old, p_old), seg_t));
        let mut normal = if dist_sq > 1e-6 {
            normalize_safe(sub(seg_point, point))
        } else {
            tri.normal
        };
        if dot(tri.normal, vdir) > 0.7 {
            continue;
        }
        let ys = tri.vertices.map(|v| v[1]);
        let top = ys[0].max(ys[1]).max(ys[2]);
        let bottom = ys[0].min(ys[1]).min(ys[2]);
        let step_new = top - new_bottom <= STEP_SIZE;
        let mut step_old = top - old_bottom <= STEP_SIZE && tri.normal[1] > 0.7;
        let mut step_low = bottom - new_bottom <= STEP_SIZE && tri.normal[1] > 0.7;
        if step_new || step_old || step_low {
            // Step/ground: vertical push by the height the triangle rises above
            // the old capsule bottom under the closest segment point.
            let mut origin = seg_point;
            origin[1] -= radius;
            let height = (p_old[1] - e_old[1]).abs() + 2.0 * radius;
            let e0 = sub(tri.vertices[1], tri.vertices[0]);
            let e1 = sub(tri.vertices[2], tri.vertices[0]);
            if let Some((ts, _, _)) =
                segment_triangle_hit(origin, [0.0, height, 0.0], tri.vertices[0], e0, e1)
            {
                penetration = ts * height;
            }
            normal[0] = 0.0;
            normal[2] = 0.0;
            normal = normalize_safe(normal);
        } else if tri.normal[1] < 0.0 {
            normal[1] = 0.0;
            normal = normalize_safe(normal);
        } else {
            step_old = top - old_bottom <= STEP_SIZE && tri.normal[1] < 0.3;
            step_low = bottom - new_bottom <= STEP_SIZE && tri.normal[1] < 0.3;
            if !step_new && !step_old && !step_low {
                normal[1] = 0.0;
                normal = normalize_safe(normal);
            }
        }
        if normal[1] < 0.0 {
            normal[1] = -normal[1];
        }
        out.push(Contact {
            level_contact: true,
            primitive: Some(tri.primitive),
            other: None,
            body: body_index,
            normal,
            points: vec![ContactPoint {
                r0: sub(point, old_pos),
                world_point: point,
                initial_penetration: penetration,
                min_separation_vel: 0.0,
                denominator: 0.0,
            }],
            ground_material: tri.ground_material,
            water_ice: tri.water_ice,
            water_surface: tri.water_surface,
            satisfied: false,
        });
    }
}

/// Intersection.SweptSpherePlane/TriangleIntersection; returns the depth only
/// (the sweep caller ignores the returned point/normal).
fn swept_sphere_triangle(
    old_c: Vector,
    new_c: Vector,
    radius: f32,
    tri: &CollisionTriangle,
    old_d: f32,
    new_d: f32,
) -> Option<f32> {
    if new_d >= old_d || new_d > radius {
        return None;
    }
    let f = (new_d - radius) / (new_d - old_d);
    if !(0.0..=1.0).contains(&f) {
        return None;
    }
    let n = tri.normal;
    let pt = sub(
        add(old_c, scale(sub(new_c, old_c), f)),
        scale(n, radius.min(old_d)),
    );
    let depth = radius - new_d;
    let [a, b, c] = tri.vertices;
    let inside = [(a, sub(b, a)), (b, sub(c, b)), (c, sub(a, c))]
        .iter()
        .all(|&(p, e)| dot(sub(pt, p), cross(e, n)) <= 0.0);
    if inside {
        return Some(depth);
    }
    let motion = sub(new_c, old_c);
    let mm = dot(motion, motion);
    let r2 = radius * radius;
    for (p, e) in [(a, sub(b, a)), (b, sub(c, b)), (c, sub(a, c))] {
        let ee = dot(e, e);
        if ee.abs() < 1e-6 {
            continue;
        }
        let w = sub(p, old_c);
        let em = dot(e, motion);
        let wm = dot(w, motion);
        let ew = dot(e, w);
        let ww = dot(w, w);
        let qa = ee * mm - em * em;
        if qa.abs() < 1e-6 {
            continue;
        }
        let qb = 2.0 * (ew * em - ee * wm);
        let qc = ee * (ww - r2) - ew * ew;
        let disc = qb * qb - 4.0 * qa * qc;
        if disc < 0.0 {
            continue;
        }
        let t = (-qb - disc.sqrt()) / (2.0 * qa);
        if (0.0..=1.0).contains(&t) {
            let centre = add(old_c, scale(motion, t));
            let u = dot(sub(centre, p), e) / ee;
            if (0.0..=1.0).contains(&u) {
                return Some(depth);
            }
        }
    }
    if mm.abs() < 1e-6 {
        return None;
    }
    for p in [a, b, c] {
        let w = sub(p, old_c);
        let qb = -2.0 * dot(w, motion);
        let qc = dot(w, w) - r2;
        let disc = qb * qb - 4.0 * mm * qc;
        if disc >= 0.0 {
            let t = (-qb - disc.sqrt()) / (2.0 * mm);
            if (0.0..=1.0).contains(&t) {
                return Some(depth);
            }
        }
    }
    None
}

/// CollDetectSphereStaticMesh.CollDetectSphereStaticMeshSweep against one
/// primitive (≤10 points, summed normals, one contact). The original mixes a
/// mesh-local triangle with world-space spheres here; with world-space
/// triangles this is exact for identity transforms (static layers) and
/// consistent for transformed animated parts (documented divergence).
fn sphere_mesh_sweep(
    body_index: usize,
    old_c: Vector,
    new_c: Vector,
    radius: f32,
    old_pos: Vector,
    candidates: &[CollisionTriangle],
    out: &mut Vec<Contact>,
) {
    let num = COLLISION_TOLERANCE + radius;
    let num2 = num * num;
    let mut normal_sum = [0.0; 3];
    let mut points = Vec::new();
    let ground_material = candidates.first().map_or(0, |t| t.ground_material);
    for tri in candidates {
        let old_d = tri.plane_distance(old_c);
        if old_d <= 0.0 {
            continue;
        }
        let new_d = tri.plane_distance(new_c);
        if new_d > num {
            continue;
        }
        let (dist_sq, s, t) = point_triangle_distance_sq(old_c, &tri.vertices);
        let dist = dist_sq.sqrt();
        let closest = tri_point(&tri.vertices, s, t);
        let dir = normalize_safe(sub(old_c, closest));
        let (n, depth) = if dist_sq < num2 {
            let n = if dist > CLR_FLOAT_EPSILON {
                dir
            } else {
                tri.normal
            };
            (n, radius - dist)
        } else if new_d < old_d {
            match swept_sphere_triangle(old_c, new_c, radius, tri, old_d, new_d) {
                Some(depth) => (if dist > 1e-6 { dir } else { tri.normal }, depth),
                None => continue,
            }
        } else {
            continue;
        };
        let at = sub(old_c, scale(n, radius));
        if points.len() < 10 {
            points.push(ContactPoint {
                r0: sub(at, old_pos),
                world_point: at,
                initial_penetration: depth,
                min_separation_vel: 0.0,
                denominator: 0.0,
            });
        }
        normal_sum = add(normal_sum, n);
    }
    if !points.is_empty() {
        out.push(Contact {
            level_contact: true,
            primitive: candidates.first().map(|t| t.primitive),
            other: None,
            body: body_index,
            normal: normalize_safe(normal_sum),
            points,
            ground_material,
            water_ice: candidates.first().is_some_and(|t| t.water_ice),
            water_surface: candidates.first().is_some_and(|t| t.water_surface),
            satisfied: false,
        });
    }
}

/// Split candidates into per-primitive runs, in first-appearance order.
fn group_by_primitive(candidates: &[CollisionTriangle]) -> Vec<Vec<CollisionTriangle>> {
    let mut groups: Vec<Vec<CollisionTriangle>> = Vec::new();
    for t in candidates {
        match groups.iter_mut().find(|g| g[0].primitive == t.primitive) {
            Some(g) => g.push(*t),
            None => groups.push(vec![*t]),
        }
    }
    groups
}

/// CustomCapsulePlane against the scene kill plane (normal up at `plane_y`):
/// a collision is reported when either capsule end sphere, at its old or new
/// position, lies within tolerance + radius of the plane (signed distance).
fn capsule_touches_kill_plane(
    body: &CharacterBody,
    old_pos: Vector,
    new_pos: Vector,
    plane_y: f32,
) -> bool {
    let reach = COLLISION_TOLERANCE + body.radius;
    let (old_start, old_end) = body.capsule(old_pos);
    let (new_start, new_end) = body.capsule(new_pos);
    (new_start[1] - plane_y).min(old_start[1] - plane_y) < reach
        || (new_end[1] - plane_y).min(old_end[1] - plane_y) < reach
}

fn detect_body(
    body: &CharacterBody,
    index: usize,
    old_pos: Vector,
    new_pos: Vector,
    geometry: &dyn StaticGeometry,
    scratch: &mut Vec<CollisionTriangle>,
    out: &mut Vec<Contact>,
) {
    let r = body.radius;
    let moved = sub(new_pos, old_pos);
    scratch.clear();
    if length_squared(moved) < 0.25 * r * r {
        let (a, b) = body.capsule(new_pos);
        let min = std::array::from_fn(|i| a[i].min(b[i]) - r);
        let max = std::array::from_fn(|i| a[i].max(b[i]) + r);
        geometry.triangles_in_box(min, max, scratch);
        capsule_mesh_overlap(body, index, old_pos, new_pos, scratch, out);
        return;
    }
    // CollDetectCapsulseStaticMeshSweep: spheres along the capsule axis.
    let count = 2 + (body.length / (2.0 * r)) as usize;
    let (old_start, _) = body.capsule(old_pos);
    let (new_start, _) = body.capsule(new_pos);
    for i in 0..count {
        let along = i as f32 * body.length / (count as f32 - 1.0);
        let old_c = add(old_start, [0.0, along, 0.0]);
        let new_c = add(new_start, [0.0, along, 0.0]);
        let min = std::array::from_fn(|k| old_c[k].min(new_c[k]) - r);
        let max = std::array::from_fn(|k| old_c[k].max(new_c[k]) + r);
        scratch.clear();
        geometry.triangles_in_box(min, max, scratch);
        for group in group_by_primitive(scratch) {
            sphere_mesh_sweep(index, old_c, new_c, r, old_pos, &group, out);
        }
    }
}

// ---------------------------------------------------------------------------
// World

/// Mutable character-pair callback over the predicted detection state.
pub type PairContactCallback<'a> =
    dyn FnMut(usize, usize, &mut CharacterBody, &mut CharacterBody) -> bool + 'a;

/// Shield geometry and synchronous world-owned contact callbacks.
/// Character-side acceptance of an obstacle contact (body, obstacle).
pub type ObstacleCallback<'a> = dyn FnMut(usize, usize, &mut CharacterBody) -> bool + 'a;

pub struct ShieldContacts<'a> {
    pub geometry: &'a [crate::hollow_sphere_box::PrimitiveMotion<
        crate::shield_intersection::ShieldGeometry,
    >],
    pub accept: &'a mut dyn FnMut(usize, usize, &mut CharacterBody) -> bool,
    /// Runs only after capsule contact detection, before inserting a solver
    /// contact. Both bodies hold the native predicted detection state.
    pub pair: Option<&'a mut PairContactCallback<'a>>,
    /// Immovable capsules (Solid Barrier primitive 0; Barrier.PostCollision
    /// ignores the barrier skin) and the character-side acceptance.
    pub obstacles: &'a [crate::capsule_pair::CapsuleMotion],
    pub accept_obstacle: Option<&'a mut ObstacleCallback<'a>>,
}

#[derive(Clone, Debug)]
pub struct PhysicsWorld {
    pub bodies: Vec<CharacterBody>,
    /// Kill plane Y (GameScene default -50, scene KillPlanePosition).
    pub kill_plane: f32,
    contacts: Vec<Contact>,
    rng: u64,
    scratch: Vec<CollisionTriangle>,
}

impl PhysicsWorld {
    /// AnimatedLevelPart.PostCollision accepts floor normals above 0.7.
    pub fn supporting_primitives(&self) -> Vec<(usize, u32)> {
        self.contacts
            .iter()
            .filter(|c| c.normal[1] > 0.7)
            .filter_map(|c| c.primitive.map(|p| (c.body, p)))
            .collect()
    }

    /// Generated world contact points, before any surface-height approximation.
    pub fn contact_points(&self, body: usize) -> Vec<Vector> {
        self.contacts
            .iter()
            .filter(|c| c.body == body)
            .flat_map(|c| c.points.iter().map(|p| p.world_point))
            .collect()
    }

    pub fn new(kill_plane: f32) -> Self {
        Self {
            bodies: Vec::new(),
            kill_plane,
            contacts: Vec::new(),
            rng: 0x9E37_79B9_7F4A_7C15,
            scratch: Vec::new(),
        }
    }

    /// Deterministic replacement for the original unseeded System.Random
    /// contact shuffle in DetectAllCollisions.
    fn next_index(&mut self, bound: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % bound as u64) as usize
    }

    /// PhysicsSystem.Integrate restricted to character bodies against static
    /// geometry. `fly_ground(i)` gives flying bodies their navmesh height.
    /// Returns ProcessCollisionPoints owner side effects per body.
    pub fn integrate(
        &mut self,
        dt: f32,
        geometry: &dyn StaticGeometry,
        fly_ground: &dyn Fn(usize, Vector) -> Option<Vector>,
    ) -> Vec<(usize, BodyEvent)> {
        self.integrate_with_pairs(dt, geometry, fly_ground, &|_, _| false)
    }

    /// Pair acceptance is supplied by the world host after character filtering.
    /// Existing integrate callers retain static-only behavior.
    pub fn integrate_with_pairs(
        &mut self,
        dt: f32,
        geometry: &dyn StaticGeometry,
        fly_ground: &dyn Fn(usize, Vector) -> Option<Vector>,
        accept_pair: &dyn Fn(usize, usize) -> bool,
    ) -> Vec<(usize, BodyEvent)> {
        self.integrate_with_shields(dt, geometry, fly_ground, accept_pair, None)
    }

    /// Character and Shield callbacks run at the predicted body state before
    /// native shield contacts enter the Normal solver. Shield.PostCollision
    /// ignores its own skin (0 for round, 1 for wall), leaving character mass.
    pub fn integrate_with_shields(
        &mut self,
        dt: f32,
        geometry: &dyn StaticGeometry,
        fly_ground: &dyn Fn(usize, Vector) -> Option<Vector>,
        accept_pair: &dyn Fn(usize, usize) -> bool,
        mut shields: Option<ShieldContacts<'_>>,
    ) -> Vec<(usize, BodyEvent)> {
        let mut events = Vec::new();
        // CopyAllCurrentStatesToOld
        for b in self
            .bodies
            .iter_mut()
            .filter(|b| b.active && !b.externally_driven)
        {
            b.old_position = b.position;
        }
        // GetAllExternalForces
        for (i, b) in self
            .bodies
            .iter_mut()
            .enumerate()
            .filter(|(_, b)| b.active && !b.externally_driven)
        {
            let ground = if b.is_flying {
                fly_ground(i, b.position)
            } else {
                None
            };
            b.add_external_forces(dt, ground);
        }
        // DetectAllCollisions: predict, detect, restore.
        self.contacts.clear();
        let mut scratch = std::mem::take(&mut self.scratch);
        // Store every initially active body before detection. Callbacks share
        // this prediction until the complete detection pass has finished;
        // mass and CharacterBody impulse fields are outside the stored state.
        let predicted_bodies: Vec<_> = self
            .bodies
            .iter()
            .enumerate()
            .filter_map(|(i, b)| (b.active && !b.externally_driven).then_some(i))
            .collect();
        for &i in &predicted_bodies {
            let b = &mut self.bodies[i];
            b.stored = (b.position, b.orientation, b.velocity);
            b.velocity = add(b.velocity, scale(b.force, dt / b.mass));
            b.position = add(b.position, scale(b.velocity, dt));
        }
        for &i in &predicted_bodies {
            let b = &self.bodies[i];
            if b.owner.collides_with_level {
                detect_body(
                    b,
                    i,
                    b.old_position,
                    b.position,
                    geometry,
                    &mut scratch,
                    &mut self.contacts,
                );
            }
            // The kill-plane callback runs even when Character.OnCollision
            // rejects owner-less skins (`flag &= ...` does not short-circuit).
            if capsule_touches_kill_plane(b, b.old_position, b.position, self.kill_plane) {
                events.push((i, BodyEvent::KillPlane));
            }
        }
        self.scratch = scratch;
        for i in 0..self.bodies.len() {
            for j in i + 1..self.bodies.len() {
                if !self.bodies[i].active
                    || self.bodies[i].externally_driven
                    || !self.bodies[j].active
                    || self.bodies[j].externally_driven
                    || !accept_pair(i, j)
                {
                    continue;
                }
                let motion = |body: &CharacterBody| {
                    let axis = |position| {
                        let (start, end) = body.capsule(position);
                        crate::spray_pulse::Segment {
                            origin: start,
                            delta: sub(end, start),
                        }
                    };
                    crate::capsule_pair::CapsuleMotion {
                        old_axis: axis(body.old_position),
                        new_axis: axis(body.position),
                        old_radius: body.radius,
                        new_radius: body.radius,
                        old_body_position: body.old_position,
                    }
                };
                let (a, b) = (motion(&self.bodies[i]), motion(&self.bodies[j]));
                if let Some(pair) = crate::capsule_pair::detect(a, b, COLLISION_TOLERANCE, || {
                    self.next_index(360) as u16
                }) {
                    if let Some(callback) = shields.as_mut().and_then(|s| s.pair.as_mut()) {
                        let (before, after) = self.bodies.split_at_mut(j);
                        let (body_a, body_b) = (&mut before[i], &mut after[0]);
                        let accepted = callback(i, j, body_a, body_b);
                        if !accepted {
                            continue;
                        }
                    }
                    self.contacts.push(Contact {
                        level_contact: false,
                        primitive: None,
                        body: i,
                        other: Some(j),
                        normal: pair.normal,
                        points: vec![ContactPoint {
                            r0: pair.offset0,
                            world_point: add(self.bodies[i].old_position, pair.offset0),
                            initial_penetration: pair.penetration,
                            min_separation_vel: 0.0,
                            denominator: 0.0,
                        }],
                        ground_material: 0,
                        water_ice: false,
                        water_surface: false,
                        satisfied: false,
                    });
                }
            }
        }
        if let Some(shields) = &mut shields {
            for i in 0..self.bodies.len() {
                if !self.bodies[i].active || self.bodies[i].externally_driven {
                    continue;
                }
                for (j, shield) in shields.geometry.iter().copied().enumerate() {
                    let body = &self.bodies[i];
                    let axis = |position| {
                        let (start, end) = body.capsule(position);
                        crate::spray_pulse::Segment {
                            origin: start,
                            delta: sub(end, start),
                        }
                    };
                    let motion = crate::capsule_pair::CapsuleMotion {
                        old_axis: axis(body.old_position),
                        new_axis: axis(body.position),
                        old_radius: body.radius,
                        new_radius: body.radius,
                        old_body_position: body.old_position,
                    };
                    use crate::shield_intersection::ShieldGeometry;
                    let detected = match (shield.old, shield.new) {
                        (ShieldGeometry::Sphere(old), ShieldGeometry::Sphere(new)) => {
                            crate::hollow_sphere_capsule::detect(
                                crate::hollow_sphere_box::PrimitiveMotion {
                                    old,
                                    new,
                                    owner_old_position: shield.owner_old_position,
                                },
                                motion,
                                COLLISION_TOLERANCE,
                                || self.next_index(360) as u16,
                            )
                            .map(|c| (c, c.normal.map(|v| -v), c.offset1))
                        }
                        (ShieldGeometry::Wall(old), ShieldGeometry::Wall(new)) => {
                            crate::capsule_box::detect(
                                motion,
                                crate::hollow_sphere_box::PrimitiveMotion {
                                    old,
                                    new,
                                    owner_old_position: shield.owner_old_position,
                                },
                                COLLISION_TOLERANCE,
                                || self.next_index(360) as u16,
                            )
                            .map(|c| (c, c.normal, c.offset0))
                        }
                        _ => None,
                    };
                    let Some((contact, normal, offset)) = detected else {
                        continue;
                    };
                    let body = &mut self.bodies[i];
                    let accepted = (shields.accept)(i, j, body);
                    if accepted {
                        self.contacts.push(Contact {
                            body: i,
                            other: None,
                            level_contact: false,
                            primitive: None,
                            normal,
                            points: vec![ContactPoint {
                                r0: offset,
                                world_point: contact.point,
                                initial_penetration: contact.penetration,
                                min_separation_vel: 0.0,
                                denominator: 0.0,
                            }],
                            ground_material: 0,
                            water_ice: false,
                            water_surface: false,
                            satisfied: false,
                        });
                    }
                }
            }
        }
        if let Some(shields) = &mut shields {
            for i in 0..self.bodies.len() {
                if !self.bodies[i].active || self.bodies[i].externally_driven {
                    continue;
                }
                for (k, obstacle) in shields.obstacles.iter().copied().enumerate() {
                    let body = &self.bodies[i];
                    let axis = |position| {
                        let (start, end) = body.capsule(position);
                        crate::spray_pulse::Segment {
                            origin: start,
                            delta: sub(end, start),
                        }
                    };
                    let motion = crate::capsule_pair::CapsuleMotion {
                        old_axis: axis(body.old_position),
                        new_axis: axis(body.position),
                        old_radius: body.radius,
                        new_radius: body.radius,
                        old_body_position: body.old_position,
                    };
                    let Some(pair) =
                        crate::capsule_pair::detect(motion, obstacle, COLLISION_TOLERANCE, || {
                            self.next_index(360) as u16
                        })
                    else {
                        continue;
                    };
                    let accepted = match shields.accept_obstacle.as_mut() {
                        Some(accept) => accept(i, k, &mut self.bodies[i]),
                        None => true,
                    };
                    if accepted {
                        self.contacts.push(Contact {
                            level_contact: false,
                            primitive: None,
                            body: i,
                            other: None,
                            normal: pair.normal,
                            points: vec![ContactPoint {
                                r0: pair.offset0,
                                world_point: add(self.bodies[i].old_position, pair.offset0),
                                initial_penetration: pair.penetration,
                                min_separation_vel: 0.0,
                                denominator: 0.0,
                            }],
                            ground_material: 0,
                            water_ice: false,
                            water_surface: false,
                            satisfied: false,
                        });
                    }
                }
            }
        }
        for i in 1..self.contacts.len() {
            let j = self.next_index(i + 1);
            self.contacts.swap(i, j);
        }
        // Body.RestoreState restores transform/rate, including callback edits
        // to position/velocity. Restore the original active snapshot even if
        // a callback disabled a body; mass, impulse and owner mutations survive.
        for i in predicted_bodies {
            let b = &mut self.bodies[i];
            (b.position, b.orientation, b.velocity) = b.stored;
        }
        // ProcessCollisionPoints
        for i in 0..self.bodies.len() {
            if self.bodies[i].active && !self.bodies[i].externally_driven {
                self.process_collision_points(i, dt, &mut events);
            }
        }
        self.contacts.retain(|c| !c.water_surface);
        self.handle_all_constraints(dt, NUM_COLLISION_ITERATIONS);
        for b in self
            .bodies
            .iter_mut()
            .filter(|b| b.active && !b.externally_driven)
        {
            b.velocity = add(b.velocity, scale(b.force, dt / b.mass));
        }
        self.handle_all_constraints(dt, NUM_CONTACT_ITERATIONS);
        if !self.contacts.is_empty() {
            for b in self
                .bodies
                .iter_mut()
                .filter(|b| b.active && !b.externally_driven)
            {
                // UpdateDeactivation + DampForDeactivation (threshold 0.5 m/s,
                // deactivation time 1 s). CharacterBody never freezes.
                if length_squared(b.velocity) > 0.25 {
                    b.inactive_time = 0.0;
                } else {
                    b.inactive_time += dt;
                }
                let frac = b.inactive_time;
                if frac >= 0.5 {
                    let s = (1.0 - (frac - 0.5) / 0.5).clamp(0.0, 1.0);
                    if length_squared(b.velocity) * s * s / 0.25 > 1.0 {
                        b.velocity = scale(b.velocity, s);
                    }
                }
            }
        }
        for b in self
            .bodies
            .iter_mut()
            .filter(|b| b.active && !b.externally_driven)
        {
            b.velocity = b.velocity.map(|v| v.clamp(-VEL_MAX, VEL_MAX));
            b.position = add(b.position, scale(b.velocity, dt));
        }
        events
    }

    /// CharacterBody.ProcessCollisionPoints for retained level/liquid contacts.
    fn process_collision_points(
        &mut self,
        index: usize,
        dt: f32,
        events: &mut Vec<(usize, BodyEvent)>,
    ) {
        let mut impact_seen = false;
        let mut on_ice = false;
        let mut in_water = false;
        let mut water_depth = 0.0f32;
        let body = &mut self.bodies[index];
        for c in self
            .contacts
            .iter()
            .filter(|c| c.body == index && c.other.is_none() && c.level_contact)
        {
            if c.water_surface {
                in_water = true;
                body.ground_material = c.ground_material;
                if let Some(point) = c.points.first() {
                    water_depth = water_depth.max(crate::liquid_contact::contact_depth(
                        body.position[1],
                        body.height_offset(),
                        point.world_point[1],
                    ));
                }
                // Native evaluates each liquid contact with the maximum depth
                // seen so far, before the eventual Drown call.
                if water_depth >= crate::liquid_contact::STATUS_DEPTH {
                    events.push((
                        index,
                        BodyEvent::LiquidStatus {
                            depth: water_depth,
                            lava: c.ground_material == 9,
                        },
                    ));
                }
                continue;
            }
            // Skin0 is this body: dirToBody negated, then dotted with velocity.
            let dir = scale(c.normal, -1.0);
            let into = dot(body.velocity, dir);
            if !body.is_touching_ground
                && !body.is_jumping
                && !body.owner.ignore_collision_damage
                && into > 0.0
            {
                // `default(Damage).AttackProperty | Knockback == Knockback` is
                // always true, so every later qualifying contact this step is
                // skipped entirely (`continue`), including ground detection.
                if impact_seen {
                    continue;
                }
                impact_seen = true;
                let k = 2.7f32 as f64;
                let factor = 100.0f32 / (10f64.powf(k) as f32);
                let amount = ((into as f64).powf(k) as f32) * factor;
                if amount > 50.0 {
                    events.push((
                        index,
                        BodyEvent::ImpactDamage {
                            amount,
                            knockdown: amount > 100.0,
                        },
                    ));
                }
            }
            if dir[1] < -0.7 {
                body.time_since_ground_contact = 0.0;
                if body.owner.hit_points > 0.0 {
                    body.last_position_on_ground = body.position;
                }
                body.ground_contact_normal = c.normal;
                body.ground_material = c.ground_material;
                on_ice = c.water_ice;
            }
            if (body.is_pushed || body.is_leaving_ground)
                && into > 0.0
                && c.normal[1].abs() < 0.3
                && body.owner.can_be_knocked_down
            {
                events.push((index, BodyEvent::KnockDown));
            }
        }
        // Flags distinguish the shared frozen solid and molten/water meshes.
        body.is_on_ice = on_ice;
        body.is_in_water = in_water;
        body.owner.water_depth = water_depth;
        if crate::liquid_contact::should_drown(
            body.owner.hit_points,
            in_water,
            water_depth,
            body.height_offset(),
        ) {
            // Character.Drown zeroes HitPoints at once.
            body.owner.hit_points = 0.0;
            events.push((
                index,
                BodyEvent::Drown {
                    depth: water_depth,
                    lava: body.ground_material == 9,
                },
            ));
        }
        if in_water {
            body.velocity = scale(
                body.velocity,
                crate::liquid_contact::velocity_multiplier(water_depth, dt),
            );
        }
    }

    /// PhysicsSystem.HandleAllConstraints with the Normal solver.
    fn handle_all_constraints(&mut self, dt: f32, iterations: usize) {
        for c in &mut self.contacts {
            Self::preprocess(c, &self.bodies[c.body], dt);
            if let Some(other) = c.other {
                for p in &mut c.points {
                    p.denominator =
                        (1.0 / self.bodies[c.body].mass + 1.0 / self.bodies[other].mass).max(1e-6);
                }
            }
        }
        // JigLibX initialises its per-iteration `gotOne` to true, so every
        // iteration runs; the first pass walks the list backwards.
        let mut forward = true;
        for _ in 0..iterations {
            forward = !forward;
            let n = self.contacts.len();
            for k in 0..n {
                let idx = if forward { k } else { n - 1 - k };
                if self.contacts[idx].satisfied {
                    continue;
                }
                if self.process(idx) {
                    // Body.SetConstraintsAndCollisionsUnsatisfied.
                    let body = self.contacts[idx].body;
                    let other = self.contacts[idx].other;
                    for c in &mut self.contacts {
                        if c.body == body
                            || c.other == Some(body)
                            || other.is_some_and(|b| c.body == b || c.other == Some(b))
                        {
                            c.satisfied = false;
                        }
                    }
                }
            }
        }
    }

    fn preprocess(c: &mut Contact, body: &CharacterBody, dt: f32) {
        c.satisfied = false;
        for p in &mut c.points {
            p.denominator = (1.0 / body.mass).max(1e-6);
            p.min_separation_vel = if p.initial_penetration > ALLOWED_PENETRATION {
                (p.initial_penetration - ALLOWED_PENETRATION) / dt
            } else {
                let f = (-0.1 * (p.initial_penetration - ALLOWED_PENETRATION)
                    / (1e-6 + ALLOWED_PENETRATION))
                    .clamp(1e-6, 1.0);
                f * (p.initial_penetration - ALLOWED_PENETRATION) / dt.max(1e-6)
            };
            if p.min_separation_vel > MAX_VEL_MAG {
                p.min_separation_vel = MAX_VEL_MAG;
            }
        }
    }

    /// PhysicsSystem.ProcessCollision for a zero-inertia body vs static mesh.
    /// Character material properties are (0,0,0), so restitution and
    /// friction products are zero and the friction pass applies nothing.
    fn process(&mut self, idx: usize) -> bool {
        let c = &mut self.contacts[idx];
        c.satisfied = true;
        let mut applied = false;
        for p in &c.points {
            let velocity = self.bodies[c.body].velocity;
            let relative = c
                .other
                .map_or(velocity, |other| sub(velocity, self.bodies[other].velocity));
            let vn = dot(relative, c.normal);
            if vn > p.min_separation_vel {
                continue;
            }
            let mut target = 0.0;
            if target < 0.001 {
                target = p.min_separation_vel;
            }
            let dv = target - vn;
            if dv <= 0.001 {
                continue;
            }
            let impulse = dv / p.denominator;
            let body = &mut self.bodies[c.body];
            body.velocity = add(body.velocity, scale(c.normal, impulse / body.mass));
            if let Some(other) = c.other {
                let body = &mut self.bodies[other];
                body.velocity = sub(body.velocity, scale(c.normal, impulse / body.mass));
            }
            applied = true;
        }
        applied
    }
}
