//! MagickCamera behaviours (FollowPlayers/FollowEntity/Locked/MoveToTarget),
//! Linear/Interpolated motion, camera-mesh and box confinement, bias,
//! magnification, hostile influence and shake. See research/play-session.md.
//! Frustum-plane skins (multiplayer screen bounds) are not modelled.
use crate::scene_command::CameraInterpolation;
use magicka_content::{level::TriggerArea, skinned::pose_matrix};
use magicka_sim::{geometry::point_triangle_distance_sq, impact_motion::Vector};

/// CLR `float.Epsilon`.
const CLR_FLOAT_EPSILON: f32 = f32::from_bits(1);
pub const CAMERA_OFFSET: Vector = [0.0, 144.0, 171.0];
pub const NEAR_CLIP: f32 = 105.0;
pub const FAR_CLIP: f32 = 500.0;
/// Factions.FRIENDLY.
pub const FACTION_FRIENDLY: i32 = 4;

pub fn default_fov() -> f32 {
    5.0f32.to_radians()
}

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
fn normalize(a: Vector) -> Vector {
    scale(a, 1.0 / dot(a, a).sqrt())
}

pub type Matrix = [f32; 16];

/// Matrix.CreateFromAxisAngle (3x3 part).
fn axis_angle(axis: Vector, angle: f32) -> [f32; 9] {
    let [x, y, z] = axis;
    let (sin, cos) = (angle.sin(), angle.cos());
    let (xx, yy, zz, xy, xz, yz) = (x * x, y * y, z * z, x * y, x * z, y * z);
    [
        xx + cos * (1.0 - xx),
        xy - cos * xy + sin * z,
        xz - cos * xz - sin * y,
        xy - cos * xy - sin * z,
        yy + cos * (1.0 - yy),
        yz - cos * yz + sin * x,
        xz - cos * xz + sin * y,
        yz - cos * yz - sin * x,
        zz + cos * (1.0 - zz),
    ]
}
fn transform_normal(v: Vector, m: &[f32; 9]) -> Vector {
    [
        v[0] * m[0] + v[1] * m[3] + v[2] * m[6],
        v[0] * m[1] + v[1] * m[4] + v[2] * m[7],
        v[0] * m[2] + v[1] * m[5] + v[2] * m[8],
    ]
}
/// Matrix.CreateLookAt.
pub fn look_at(position: Vector, target: Vector, up: Vector) -> Matrix {
    let z = normalize(sub(position, target));
    let x = normalize(cross(up, z));
    let y = cross(z, x);
    [
        x[0],
        y[0],
        z[0],
        0.0,
        x[1],
        y[1],
        z[1],
        0.0,
        x[2],
        y[2],
        z[2],
        0.0,
        -dot(x, position),
        -dot(y, position),
        -dot(z, position),
        1.0,
    ]
}
/// Matrix.CreatePerspectiveFieldOfView (vertical field of view).
pub fn perspective(fov: f32, aspect: f32, near: f32, far: f32) -> Matrix {
    let y = 1.0 / (fov * 0.5).tan();
    let mut m = [0.0; 16];
    m[0] = y / aspect;
    m[5] = y;
    m[10] = far / (near - far);
    m[11] = -1.0;
    m[14] = near * far / (near - far);
    m
}
pub fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    std::array::from_fn(|i| {
        let (r, c) = (i / 4, i % 4);
        (0..4).map(|k| a[r * 4 + k] * b[k * 4 + c]).sum()
    })
}
fn transform4(v: [f32; 4], m: &Matrix) -> [f32; 4] {
    std::array::from_fn(|c| (0..4).map(|k| v[k] * m[k * 4 + c]).sum())
}

/// Oriented trigger-area box (corner position, rotation rows, side lengths).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraBox {
    pub corner: Vector,
    pub axes: [Vector; 3],
    pub sides: Vector,
}
impl CameraBox {
    pub fn from_area(area: &TriggerArea) -> Self {
        let m = pose_matrix([0.0; 3], area.orientation, [1.0; 3]);
        Self {
            corner: area.position,
            axes: std::array::from_fn(|i| [m[i * 4], m[i * 4 + 1], m[i * 4 + 2]]),
            sides: area.sides,
        }
    }
    /// Box.GetDistanceToPoint's closest point inside the box.
    pub fn closest_point(&self, p: Vector) -> Vector {
        let offset = sub(p, self.corner);
        let mut out = self.corner;
        for i in 0..3 {
            let local = dot(offset, self.axes[i]).clamp(0.0, self.sides[i]);
            out = add(out, scale(self.axes[i], local));
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraBehaviour {
    FollowPlayers,
    FollowEntity,
    Locked,
    MoveToTarget,
}

impl CameraBehaviour {
    /// The native byte value (MagickCamera.State.Write casts to byte).
    pub fn to_byte(self) -> u8 {
        match self {
            CameraBehaviour::FollowPlayers => 0,
            CameraBehaviour::FollowEntity => 1,
            CameraBehaviour::Locked => 2,
            CameraBehaviour::MoveToTarget => 3,
        }
    }
    pub fn from_byte(value: u8) -> Option<Self> {
        Some(match value {
            0 => CameraBehaviour::FollowPlayers,
            1 => CameraBehaviour::FollowEntity,
            2 => CameraBehaviour::Locked,
            3 => CameraBehaviour::MoveToTarget,
            _ => return None,
        })
    }
}

/// MagickCamera.State: the fields UpdateState captures and ApplyState
/// restores (research/checkpoints.md, 2026-10-30). `following` is the
/// followed entity's position (the original keeps the entity reference);
/// Write/Read drop the box and the followed entity.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraState {
    pub magnification: f32,
    pub target_magnification: f32,
    pub time: f32,
    pub behaviour: CameraBehaviour,
    pub following: Option<Vector>,
    pub target: Vector,
    pub current: Vector,
    pub camera_box: Option<CameraBox>,
}

impl CameraState {
    /// The subset MagickCamera.State.Write persists (box and entity dropped).
    pub fn persisted(&self) -> Self {
        Self {
            following: None,
            camera_box: None,
            ..self.clone()
        }
    }
}

/// One followed player for GetCenterPosition.
#[derive(Clone, Copy, Debug)]
pub struct CameraSubject {
    pub position: Vector,
    pub dead: bool,
    /// !IsLeavingGround && IsTouchingGround.
    pub grounded: bool,
    /// Capsule.Length * .5 + Capsule.Radius.
    pub feet_offset: f32,
}

#[derive(Clone, Debug)]
pub struct MagickCamera {
    inspection_view: Option<CameraView>,
    current: Vector,
    target: Vector,
    velocity: Vector,
    time: f32,
    time_start: f32,
    magnification: f32,
    target_magnification: f32,
    default_magnification: f32,
    player_magnification: f32,
    player_magnification_ttl: f32,
    dynamic_magnify: Option<(f32, f32)>,
    bias: Vector,
    target_bias: Vector,
    bias_time: f32,
    pub interpolation: CameraInterpolation,
    pub behaviour: CameraBehaviour,
    following: Option<Vector>,
    snap: Option<Vec<[Vector; 3]>>,
    camera_box: Option<CameraBox>,
    pub lock_input: bool,
    shake_magnitude: f32,
    shake_ttl: f32,
    shake_start_ttl: f32,
    direction: Vector,
    up: Vector,
}

/// Optional application-owned view shared by rendering, projection and picking.
/// This is an inspection facility, not recovered retail camera behavior.
#[derive(Clone, Copy, Debug)]
pub struct CameraView {
    pub position: Vector,
    pub direction: Vector,
    pub up: Vector,
    pub fov: f32,
}

impl MagickCamera {
    /// Constructor defaults plus Init(): Forward rotated by Rotate(0, -40°).
    pub fn new(snap: Option<Vec<[Vector; 3]>>) -> Self {
        let up = [0.0, 1.0, 0.0];
        let mut direction = [0.0, 0.0, -1.0];
        direction = transform_normal(direction, &axis_angle(up, 0.0));
        let limit = std::f32::consts::PI * 9.0 / 20.0;
        let mut pitch = -(40.0f32.to_radians());
        let total = direction[1].asin() + pitch;
        if total > limit || total < -limit {
            pitch = limit - total;
        }
        let axis = normalize(cross(direction, up));
        direction = transform_normal(direction, &axis_angle(axis, pitch));
        Self {
            current: [0.0; 3],
            target: [0.0; 3],
            velocity: [0.0; 3],
            time: 1.0,
            time_start: 1.0,
            magnification: 1.0,
            target_magnification: 1.0,
            default_magnification: 1.0,
            player_magnification: 1.0,
            player_magnification_ttl: 0.0,
            dynamic_magnify: None,
            bias: [0.0; 3],
            target_bias: [0.0; 3],
            bias_time: 10.0,
            interpolation: CameraInterpolation::Interpolated,
            behaviour: CameraBehaviour::FollowPlayers,
            following: None,
            snap,
            camera_box: None,
            lock_input: false,
            shake_magnitude: 0.0,
            shake_ttl: 0.0,
            shake_start_ttl: 0.0,
            direction,
            up,
            inspection_view: None,
        }
    }
    pub fn ground_position(&self) -> Vector {
        self.current
    }
    pub fn position(&self) -> Vector {
        self.inspection_view
            .map_or_else(|| add(self.current, CAMERA_OFFSET), |v| v.position)
    }
    pub fn direction(&self) -> Vector {
        self.inspection_view.map_or(self.direction, |v| v.direction)
    }
    pub fn up(&self) -> Vector {
        self.inspection_view.map_or(self.up, |v| v.up)
    }
    pub fn fov(&self) -> f32 {
        self.inspection_view
            .map_or_else(|| default_fov() / self.magnification, |v| v.fov)
    }
    pub fn set_inspection_view(&mut self, view: Option<CameraView>) {
        self.inspection_view = view;
    }
    pub fn magnification(&self) -> f32 {
        self.magnification
    }
    /// MagickCamera.State.UpdateState.
    pub fn state(&self) -> CameraState {
        CameraState {
            magnification: self.magnification,
            target_magnification: self.target_magnification,
            time: self.time,
            behaviour: self.behaviour,
            following: self.following,
            target: self.target,
            current: self.current,
            camera_box: self.camera_box,
        }
    }
    /// MagickCamera.State.ApplyState: the captured magnification, target
    /// magnification, time, behaviour, target, box, following and current
    /// position. The caller's preceding Release(0.001) leaves Interpolated
    /// interpolation, unlocked input and mTimeStart = 0.001; `mPlayers` is
    /// rebuilt from the playing avatars by the session each update.
    pub fn apply_state(&mut self, state: &CameraState) {
        self.magnification = state.magnification;
        self.target_magnification = state.target_magnification;
        self.time = state.time;
        self.behaviour = state.behaviour;
        self.target = state.target;
        self.camera_box = state.camera_box;
        self.following = state.following;
        self.current = state.current;
    }

    // Script-facing setters ------------------------------------------------

    /// `Time` setter: mTime = mTimeStart = value.
    pub fn set_time(&mut self, value: f32) {
        self.time = value;
        self.time_start = value;
    }
    /// `Magnification` setter (target magnification).
    pub fn set_magnification(&mut self, value: f32) {
        self.target_magnification = value;
    }
    pub fn set_default_magnification(&mut self, value: f32) {
        if (self.target_magnification - self.default_magnification).abs() < 0.0001 {
            self.target_magnification = value;
        }
        self.default_magnification = value;
    }
    pub fn set_player_magnification(&mut self, value: f32, ttl: f32) {
        self.player_magnification = value;
        self.player_magnification_ttl = ttl;
    }
    pub fn set_dynamic_magnification(&mut self, range: Option<(f32, f32)>) {
        self.dynamic_magnify = range;
    }
    pub fn set_bias(&mut self, bias: Vector, time: f32) {
        self.target_bias = bias;
        self.bias_time = time;
    }
    /// Release(time): back to FollowPlayers, Interpolated, default zoom.
    pub fn release(&mut self, time: f32) {
        self.set_time(time);
        self.camera_box = None;
        self.behaviour = CameraBehaviour::FollowPlayers;
        self.interpolation = CameraInterpolation::Interpolated;
        self.target_magnification = self.default_magnification;
        self.lock_input = false;
        self.following = None;
    }
    /// Release_NoMagReset(time).
    pub fn release_keep_magnification(&mut self, time: f32) {
        self.set_time(time);
        self.behaviour = CameraBehaviour::FollowPlayers;
        self.interpolation = CameraInterpolation::Interpolated;
        self.lock_input = false;
        self.following = None;
    }
    /// LockOn(box, time).
    pub fn lock_on(&mut self, camera_box: CameraBox, time: f32) {
        self.set_time(time);
        self.camera_box = Some(camera_box);
    }
    /// Follow(entity): the caller supplies the entity position each update.
    pub fn follow(&mut self, position: Vector) {
        self.following = Some(position);
        self.behaviour = CameraBehaviour::FollowEntity;
    }
    /// Whether a Follow(entity) is still active (Release clears it).
    pub fn is_following(&self) -> bool {
        self.following.is_some()
    }
    pub fn set_followed_position(&mut self, position: Option<Vector>) {
        self.following = position;
    }
    /// MoveTo(target, time).
    pub fn move_to(&mut self, target: Vector, time: f32) {
        self.behaviour = CameraBehaviour::MoveToTarget;
        self.target = target;
        self.set_time(time);
    }
    /// CameraShake(magnitude, ttl).
    pub fn shake(&mut self, magnitude: f32, ttl: f32) {
        self.shake_magnitude = magnitude;
        self.shake_start_ttl = ttl;
        self.shake_ttl = ttl;
    }

    /// SnapPosition: box first, else nearest camera-mesh point.
    fn snap_position(&self, p: Vector) -> Vector {
        if let Some(b) = &self.camera_box {
            return b.closest_point(p);
        }
        let Some(tris) = &self.snap else {
            return p;
        };
        let mut best = f32::MAX;
        let mut point = [0.0; 3];
        for tri in tris {
            let (d, s, t) = point_triangle_distance_sq(p, tri);
            if d < best {
                best = d;
                point = add(
                    tri[0],
                    add(scale(sub(tri[1], tri[0]), s), scale(sub(tri[2], tri[0]), t)),
                );
            }
        }
        point
    }
    fn confined(&self) -> bool {
        self.snap.is_some() || self.camera_box.is_some()
    }
    /// SetPosition(Vector3, bool iSnapToMesh).
    pub fn set_position(&mut self, p: Vector, snap: bool) {
        self.current = if snap && self.confined() {
            self.snap_position(p)
        } else {
            p
        };
    }

    fn center(subjects: &[CameraSubject]) -> Option<Vector> {
        let mut o = [0.0; 3];
        let (mut feet, mut alive, mut grounded) = (0.0f32, 0, 0);
        for s in subjects.iter().filter(|s| !s.dead) {
            alive += 1;
            o = add(o, s.position);
            if s.grounded {
                grounded += 1;
                feet += s.position[1] - s.feet_offset;
            }
        }
        if alive == 0 {
            return None;
        }
        o = scale(o, 1.0 / alive as f32);
        // A feet-height sum of exactly zero keeps the body Y.
        if feet.abs() >= CLR_FLOAT_EPSILON {
            o[1] = feet / grounded as f32;
        }
        Some(o)
    }

    /// GetInfluenceVector over living non-friendly characters (not local
    /// avatars): average of offsets weighted by 0.99775^(distance²).
    fn influence(&self, hostiles: &[Vector]) -> Vector {
        if hostiles.is_empty() {
            return [0.0; 3];
        }
        let mut sum = [0.0; 3];
        for &p in hostiles {
            let d = sub(p, self.current);
            let power = 0.997_749_984_264_373_8f64.powf(dot(d, d) as f64) as f32;
            sum = add(sum, scale(d, power));
        }
        scale(sum, 1.0 / hostiles.len() as f32)
    }

    /// MagickCamera.Update. `hostiles` are positions for the influence vector.
    pub fn update(&mut self, subjects: &[CameraSubject], hostiles: &[Vector], dt: f32) {
        match self.behaviour {
            CameraBehaviour::FollowPlayers => {
                match Self::center(subjects) {
                    Some(center) => {
                        self.target = add(add(center, self.influence(hostiles)), self.bias);
                    }
                    None => self.target = self.current,
                }
                self.time += (1.0 - self.time) * dt;
                if self.confined() {
                    self.target = self.snap_position(self.target);
                }
            }
            CameraBehaviour::FollowEntity => match self.following {
                None => self.behaviour = CameraBehaviour::FollowPlayers,
                Some(p) => {
                    self.target = p;
                    if self.confined() {
                        self.target = self.snap_position(self.target);
                    }
                }
            },
            CameraBehaviour::Locked | CameraBehaviour::MoveToTarget => {}
        }
        let k = 0.02f64.powf((dt / self.bias_time) as f64) as f32;
        self.bias = add(self.target_bias, scale(sub(self.bias, self.target_bias), k));
        let (next, zoom);
        if self.time > 0.0 {
            next = match self.interpolation {
                CameraInterpolation::Linear => {
                    let to = sub(self.target, self.current);
                    let length = dot(to, to).sqrt();
                    let step = 100.0 * dt / self.time;
                    if step < length {
                        add(self.current, scale(to, step / length))
                    } else {
                        self.target
                    }
                }
                CameraInterpolation::Interpolated => {
                    if dt <= CLR_FLOAT_EPSILON {
                        self.current
                    } else {
                        let to = sub(self.target, self.current);
                        let mut v = scale(self.velocity, 1.0 / dt);
                        let accel = sub(scale(to, 2.0 / self.time), scale(v, 0.5));
                        v = scale(add(v, accel), dt);
                        if dot(v, v) > dot(to, to) {
                            self.target
                        } else {
                            add(self.current, v)
                        }
                    }
                }
            };
            let mut a = self.target_magnification;
            if self.behaviour == CameraBehaviour::FollowPlayers
                && self.player_magnification_ttl > 0.0
            {
                a *= self.player_magnification;
            }
            let t = 0.02f64.powf((dt / self.time) as f64) as f32;
            zoom = a + (self.magnification - a) * t;
        } else {
            zoom = self.target_magnification;
            next = self.target;
        }
        self.player_magnification_ttl -= dt;
        self.velocity = sub(next, self.current);
        self.current = add(self.current, self.velocity);
        if let (Some((min, max)), CameraBehaviour::FollowPlayers) =
            (self.dynamic_magnify, self.behaviour)
        {
            let position = add(self.current, CAMERA_OFFSET);
            let mut widest = 0.0f32;
            for s in subjects {
                let d = normalize(sub(s.position, position));
                let angle = dot(self.direction, d).clamp(-1.0, 1.0).acos();
                if angle >= widest {
                    widest = angle;
                }
            }
            self.target_magnification = (0.5 * default_fov() / widest).clamp(min, max);
        }
        // mCanZoomIn is only cleared by frustum-plane collisions (not wired).
        self.magnification += zoom - self.magnification;
        if self.shake_ttl > 0.0 && dt > CLR_FLOAT_EPSILON {
            // FastSin approximation replaced by sin (cosmetic).
            let angle = wrap_angle(self.shake_ttl * 60.0);
            self.current[1] += angle.sin()
                * self.shake_magnitude
                * (self.shake_ttl / self.shake_start_ttl)
                * 0.0333;
        }
        self.shake_ttl -= dt;
    }

    pub fn view(&self) -> Matrix {
        let p = self.position();
        look_at(p, add(p, self.direction()), self.up())
    }
    pub fn projection(&self, aspect: f32) -> Matrix {
        perspective(self.fov(), aspect, NEAR_CLIP, FAR_CLIP)
    }

    /// Camera-to-far-plane delta from native GetMousePick, for collision picking.
    pub fn mouse_ray(&self, ndc: [f32; 2], aspect: f32) -> Vector {
        let vp = multiply(&self.view(), &self.projection(aspect));
        let Ok(inverse) = crate::live_collision::inverse(&vp) else {
            return [0.0; 3];
        };
        let v = transform4([ndc[0], ndc[1], 1.0, 1.0], &inverse);
        sub(scale([v[0], v[1], v[2]], 1.0 / v[3]), self.position())
    }

    /// KeyboardMouseController.GetMousePick: unproject the cursor at the far
    /// plane and intersect the camera ray with the horizontal plane through
    /// `plane_y`. Falls back to the camera position.
    pub fn mouse_pick(&self, ndc: [f32; 2], aspect: f32, plane_y: f32) -> Vector {
        let vp = multiply(&self.view(), &self.projection(aspect));
        let Ok(inverse) = crate::live_collision::inverse(&vp) else {
            return self.position();
        };
        let v = transform4([ndc[0], ndc[1], 1.0, 1.0], &inverse);
        let far = scale([v[0], v[1], v[2]], 1.0 / v[3]);
        let origin = self.position();
        let dir = sub(far, origin);
        if dir[1].abs() < 1e-5 {
            return origin;
        }
        let mut t = (plane_y - origin[1]) / dir[1];
        if t < 0.0 {
            if t < -1e-5 {
                return origin;
            }
            t = 0.0;
        }
        add(origin, scale(dir, t))
    }
}

/// MathHelper.WrapAngle.
fn wrap_angle(angle: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut a = (angle as f64).rem_euclid(tau as f64) as f32;
    if a > std::f32::consts::PI {
        a -= tau;
    }
    a
}
