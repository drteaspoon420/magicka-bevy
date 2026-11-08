//! Magicka.GameLogic.Entities.Shield: the spell shield entity (Sphere from
//! Shield Area, Disc from Shield Force, Wall). Initialize (orientation,
//! overlap kills), Update (decay, fade, death), InternalDamage and geometry.
//! See research/shield-entity.md.
use crate::{
    box_intersection::OrientedBox,
    damage::{attack, Damage},
    geometry::jiglib_segment_segment,
    impact_motion::Vector,
    shield_intersection::{HollowSphere, ShieldGeometry},
    spray_pulse::Segment,
};
use std::f32::consts::PI;

/// Magicka ShieldType.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ShieldKind {
    Sphere,
    Disc,
    Wall,
}

/// Shield.WALL_LENGTH (half length along Forward).
pub const WALL_LENGTH: f32 = 6.0;
/// Shield.mMaxHitPoints.
pub const MAX_HIT_POINTS: f32 = 5000.0;

/// Spell.SHIELDCOLOR / ARCANECOLOR / LIFECOLOR.
pub const SHIELD_COLOR: Vector = [2.0, 1.5, 1.0];
pub const ARCANE_COLOR: Vector = [2.0, 0.4, 0.6];
pub const LIFE_COLOR: Vector = [0.2, 1.6, 0.2];

/// DamageResult bits used here.
pub const RESULT_HIT: u16 = 2;
pub const RESULT_DEFLECTED: u16 = 0x100;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShieldEntity {
    pub owner: Option<u16>,
    pub position: Vector,
    /// Body orientation rows (XNA Right, Up, Forward); Forward is the cast
    /// direction exactly as passed (not normalized).
    pub right: Vector,
    pub up: Vector,
    pub forward: Vector,
    pub radius: f32,
    pub kind: ShieldKind,
    pub hit_points: f32,
    pub max_hit_points: f32,
    /// mDamageTimer (persists in cached instances; fresh shields start at 0).
    pub damage_timer: f32,
    pub intersect_kill_timer: f32,
    /// mTint (rgb colour, w fade) and mTargetTint.W.
    pub tint: [f32; 4],
    pub target_alpha: f32,
    pub dead: bool,
    /// Noise texture offsets (render data).
    pub noise: [[f32; 2]; 3],
}

fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(a: Vector) -> Vector {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}
fn angle2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] * b[0] + a[1] * b[1]).clamp(-1.0, 1.0).acos()
}
fn normalize2(a: [f32; 2]) -> [f32; 2] {
    let l = (a[0] * a[0] + a[1] * a[1]).sqrt();
    [a[0] / l, a[1] / l]
}

impl ShieldKind {
    /// HollowSphere.MaxAngle: π/4 for discs, π otherwise.
    pub fn max_angle(self) -> f32 {
        if self == ShieldKind::Disc {
            PI / 4.0
        } else {
            PI
        }
    }
}

/// Shield.CircleCircleIntersect (XZ plane).
#[allow(clippy::too_many_arguments)]
pub fn circle_circle_intersect(
    center_a: [f32; 2],
    dir_a: [f32; 2],
    radius_a: f32,
    max_angle_a: f32,
    center_b: [f32; 2],
    dir_b: [f32; 2],
    radius_b: f32,
    max_angle_b: f32,
) -> bool {
    let d = ((center_a[0] - center_b[0]).powi(2) + (center_a[1] - center_b[1]).powi(2)).sqrt();
    if d + (radius_a - radius_b).abs() <= f32::EPSILON {
        return angle2(dir_a, dir_b) < max_angle_a + max_angle_b;
    }
    if d < radius_b + radius_a {
        let a = (radius_a * radius_a - radius_b * radius_b + d * d) / (2.0 * d);
        let h = (radius_a * radius_a - a * a).sqrt();
        let m = [
            center_a[0] + (center_b[0] - center_a[0]) * a / d,
            center_a[1] + (center_b[1] - center_a[1]) * a / d,
        ];
        let p = [
            m[0] + h * (center_b[1] - center_a[1]) / d,
            m[1] - h * (center_b[0] - center_a[0]) / d,
        ];
        let q = [
            m[0] - h * (center_b[1] - center_a[1]) / d,
            m[1] + h * (center_b[0] - center_a[0]) / d,
        ];
        let from = |c: [f32; 2], x: [f32; 2]| normalize2([x[0] - c[0], x[1] - c[1]]);
        let b1 = angle2(dir_b, from(center_b, p));
        let b2 = angle2(dir_b, from(center_b, q));
        let a1 = angle2(dir_a, from(center_a, p));
        let a2 = angle2(dir_a, from(center_a, q));
        return (b1 <= max_angle_b || b2 <= max_angle_b)
            && (a1 <= max_angle_a || a2 <= max_angle_a);
    }
    false
}

impl ShieldEntity {
    /// Shield.Initialize. Overlapping existing shields are killed (HitPoints
    /// 0) and this shield gets a 0.25 s intersect-kill timer.
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        owner: Option<u16>,
        position: Vector,
        radius: f32,
        direction: Vector,
        kind: ShieldKind,
        hit_points: f32,
        color: Vector,
        others: &mut [ShieldEntity],
    ) -> Self {
        let forward = direction;
        let right = normalize(cross(forward, [0.0, 1.0, 0.0]));
        let up = normalize(cross(cross(forward, [0.0, 1.0, 0.0]), forward));
        let mut shield = Self {
            owner,
            position,
            right,
            up,
            forward,
            radius,
            kind,
            hit_points,
            max_hit_points: MAX_HIT_POINTS,
            damage_timer: 0.0,
            intersect_kill_timer: 0.0,
            tint: [color[0], color[1], color[2], 0.0],
            target_alpha: 4.0,
            dead: false,
            noise: [[0.0; 2]; 3],
        };
        let max_angle = kind.max_angle();
        let center_a = [position[0], position[2]];
        let dir_a = normalize2([direction[0], direction[2]]);
        for other in others.iter_mut() {
            let center_b = [other.position[0], other.position[2]];
            let dir_b = [other.forward[0], other.forward[2]];
            let round = |k: ShieldKind| k != ShieldKind::Wall;
            let hit = if round(kind) && round(other.kind) {
                circle_circle_intersect(
                    center_a,
                    dir_a,
                    radius,
                    max_angle,
                    center_b,
                    dir_b,
                    other.radius,
                    other.kind.max_angle(),
                )
            } else if kind == ShieldKind::Wall && other.kind == ShieldKind::Wall {
                let seg = |c: [f32; 2], d: [f32; 2]| {
                    (
                        [c[0] - d[0] * WALL_LENGTH, c[1] - d[1] * WALL_LENGTH, 0.0],
                        [d[0] * 2.0 * WALL_LENGTH, d[1] * 2.0 * WALL_LENGTH, 0.0],
                    )
                };
                let (o0, d0) = seg(center_a, dir_a);
                let (o1, d1) = seg(center_b, dir_b);
                jiglib_segment_segment(o0, d0, o1, d1).0 <= 0.25
            } else {
                // One wall, one round shield: the wall's segment against the
                // round shield's hollow sphere.
                let (wall_pos, wall_fwd, round_shield) = if kind == ShieldKind::Wall {
                    (
                        position,
                        direction,
                        (other.position, other.forward, other.radius, other.kind),
                    )
                } else {
                    (
                        other.position,
                        other.forward,
                        (position, direction, radius, kind),
                    )
                };
                let segment = Segment {
                    origin: std::array::from_fn(|i| wall_pos[i] - wall_fwd[i] * WALL_LENGTH),
                    delta: wall_fwd.map(|v| v * WALL_LENGTH * 2.0),
                };
                HollowSphere {
                    center: round_shield.0,
                    radius: round_shield.2,
                    forward: round_shield.1,
                    max_angle: round_shield.3.max_angle(),
                }
                .segment_fraction(segment)
                .is_some()
            };
            if hit {
                other.kill();
                shield.intersect_kill_timer = 0.25;
            }
        }
        shield
    }

    /// Shield.Kill: HitPoints 0 (fade-out, then dead).
    pub fn kill(&mut self) {
        self.hit_points = 0.0;
    }

    /// Shield.Update (gameplay part): intersect-kill timer, 10 HP every
    /// 0.1 s, fade toward the target alpha (rate 25, or 10 when dying), and
    /// death once dying and faded below 0.01. Returns `dead`.
    pub fn update(&mut self, dt: f32) -> bool {
        self.damage_timer -= dt;
        let mut rate = 25.0;
        if self.intersect_kill_timer > 0.0 {
            self.intersect_kill_timer -= dt;
            if self.intersect_kill_timer < 0.0 {
                self.hit_points = 0.0;
            }
        }
        while self.damage_timer < 0.0 {
            self.damage_timer += 0.1;
            self.hit_points -= 10.0;
        }
        if self.hit_points <= 0.0 {
            rate = 10.0;
            self.target_alpha = 0.0;
            if self.tint[3] <= 0.01 {
                self.dead = true;
            }
        } else {
            self.target_alpha = 1.0;
        }
        if self.hit_points > self.max_hit_points {
            self.hit_points = self.max_hit_points;
        }
        self.tint[3] += (self.target_alpha - self.tint[3]) * dt * rate;
        let n = 0.5 * (self.max_hit_points - self.hit_points) / self.max_hit_points + 0.05;
        self.noise[2][1] -= dt * n;
        self.noise[0][1] += dt * 0.3 * n;
        self.noise[0][0] -= dt * 0.7 * n;
        self.noise[1][1] += dt * 0.7 * n;
        self.noise[1][0] += dt * 0.4 * n;
        self.dead
    }

    /// Shield.InternalDamage(Damage): Deflected unless Damage/Status; with
    /// the Damage feature a non-negative amount·magnitude is subtracted.
    pub fn internal_damage(&mut self, damage: &Damage, feature_damage: bool) -> u16 {
        if damage.attack & (attack::DAMAGE | attack::STATUS) == 0 {
            return RESULT_DEFLECTED;
        }
        if feature_damage && damage.amount * damage.magnitude >= 0.0 {
            self.hit_points -= damage.amount * damage.magnitude;
        }
        RESULT_HIT
    }

    /// The collision primitive SegmentIntersect uses: the hollow sphere for
    /// Sphere/Disc, the 0.4 × 6 × 12 box for Wall.
    pub fn geometry(&self) -> ShieldGeometry {
        match self.kind {
            ShieldKind::Wall => ShieldGeometry::Wall(OrientedBox {
                center: self.position,
                axes: [self.right, self.up, self.forward.map(|v| -v)],
                side_lengths: [0.4, 6.0, 12.0],
            }),
            _ => ShieldGeometry::Sphere(HollowSphere {
                center: self.position,
                radius: self.radius,
                forward: self.forward,
                max_angle: self.kind.max_angle(),
            }),
        }
    }
}

/// ShieldSpell pure-Shield casts: (position, radius, kind, hit points,
/// colour). Force places a disc 3.5 m behind the caster; Area a sphere at
/// the caster tinted by Shield/Arcane/Life magnitudes.
pub fn pure_shield(
    area: bool,
    owner_position: Vector,
    owner_direction: Vector,
    shield_magnitude: f32,
    arcane_magnitude: f32,
    life_magnitude: f32,
) -> (Vector, f32, ShieldKind, f32, Vector) {
    let radius = 5.5;
    if area {
        let c = |base: Vector, m: f32| base.map(|v| v * 2.0 * m.sqrt());
        let s = c(SHIELD_COLOR, shield_magnitude);
        let a = c(ARCANE_COLOR, arcane_magnitude);
        let l = c(LIFE_COLOR, life_magnitude);
        let color = std::array::from_fn(|i| s[i] + a[i] + l[i]);
        (owner_position, radius, ShieldKind::Sphere, 1000.0, color)
    } else {
        let position =
            std::array::from_fn(|i| owner_position[i] - owner_direction[i] * (radius - 2.0));
        (position, radius, ShieldKind::Disc, 800.0, SHIELD_COLOR)
    }
}
