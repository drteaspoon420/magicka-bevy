//! Character.SpawnGibs and the Gib entity (research/gibs.md).
//!
//! Spawn arithmetic, cache reuse, TTL, sinking and collision filtering follow
//! the static CIL. JigLibX's box body is reimplemented as a swept sphere of
//! half the box side against level collision (documented solver divergence).
use crate::particles::DotNetRandom;
use magicka_sim::spray_pulse::Segment;

pub type Vec3 = [f32; 3];

/// Gib.GORE_GIB_*_EFFECTS / GORE_SPLASH_EFFECTS by BloodType
/// (regular, green, black, wood, insect; 5 = none is "").
pub const GORE_GIB_SMALL: [&str; 5] = [
    "gore_gib_regular_small",
    "gore_gib_green_small",
    "gore_gib_black_small",
    "gore_gib_wood_small",
    "gore_gib_insect_small",
];
pub const GORE_GIB_MEDIUM: [&str; 5] = [
    "gore_gib_regular_medium",
    "gore_gib_green_medium",
    "gore_gib_black_medium",
    "gore_gib_wood_medium",
    "gore_gib_insect_medium",
];
pub const GORE_GIB_LARGE: [&str; 5] = [
    "gore_gib_regular_large",
    "gore_gib_green_large",
    "gore_gib_black_large",
    "gore_gib_wood_large",
    "gore_gib_insect_large",
];
pub const GORE_GIB_TRAIL: [&str; 5] = [
    "gore_gib_regular_trail",
    "gore_gib_green_trail",
    "gore_gib_black_trail",
    "gore_gib_wood_trail",
    "gore_gib_insect_trail",
];
/// BloodType.none.
pub const BLOOD_NONE: i32 = 5;
/// Barrier.Ice_Barrier_Death_Effect (frozen gibbing).
pub const ICE_DEATH_EFFECT: &str = "barrier_ice_death";
/// DeadState.SOUND_GIB (Banks.Misc).
pub const SOUND_GIB: &str = "misc_gib";
/// PlayState: Gib.InitializeCache(256).
pub const CACHE_SIZE: usize = 256;
/// MaterialProperties(elasticity 0, static 0.8, dynamic 0.8).
const FRICTION: f32 = 0.8;

/// CharacterTemplate GibReference with its model's first mesh bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct GibDefinition {
    /// Content-relative model path without extension (forward slashes).
    pub model: String,
    pub mass: f32,
    pub scale: f32,
    /// Model.Meshes[0].BoundingSphere radius (Gib.mRadius; unscaled).
    pub radius: f32,
}

/// A live Gib entity.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveGib {
    pub id: u32,
    pub model: String,
    pub scale: f32,
    pub radius: f32,
    /// Body mass (Entity.SetMass with the gib definition mass).
    pub mass: f32,
    pub position: Vec3,
    /// Body orientation quaternion (x, y, z, w).
    pub orientation: [f32; 4],
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub ttl: f32,
    pub frozen: bool,
    pub blood: i32,
    /// Body active (SetInactive once TTL < 1).
    pub active: bool,
}

/// Facts of the gibbed character that SpawnGibs reads.
#[derive(Clone, Copy, Debug)]
pub struct GibSource {
    /// Character.Position (body centre).
    pub position: Vec3,
    /// Capsule radius and length (GetRandomPositionOnCollisionSkin).
    pub capsule_radius: f32,
    pub capsule_length: f32,
    /// Character.Radius.
    pub radius: f32,
    /// Body mass (gore effect size).
    pub mass: f32,
    /// mDeathStatusEffects includes Frozen.
    pub frozen: bool,
    pub blood: i32,
}

/// One-shot or follow presentation request.
#[derive(Clone, Debug, PartialEq)]
pub enum GibVisual {
    /// EffectManager.StartEffect at a position/direction.
    OneShot {
        name: &'static str,
        position: Vec3,
        direction: Vec3,
    },
    /// Trail effect following gib `id` (started at Initialize).
    Trail { id: u32, name: String },
    /// OnCollision blood splash unless gib `id`'s previous splash is active.
    Splash {
        id: u32,
        name: &'static str,
        position: Vec3,
        direction: Vec3,
    },
}

/// Gib cache plus the live gibs. The cache is a LIFO list whose entries
/// keep the radius of their previous use: SpawnGibs reads `fromCache.Radius`
/// *before* Initialize, so a gib's spawn height uses the stale radius.
#[derive(Clone, Debug)]
pub struct Gibs {
    /// Cached gibs' (radius, body mass) from their previous use.
    cache: Vec<(f32, f32)>,
    pub live: Vec<LiveGib>,
    next_id: u32,
    /// Character.sRandom stand-in (time-seeded in the original).
    random: DotNetRandom,
    /// SpawnGibsEvent.sRandom is separate from Character.sRandom.
    event_random: DotNetRandom,
}

impl Default for Gibs {
    fn default() -> Self {
        Self::new(0x6769_6273)
    }
}

pub use magicka_sim::math_approximation::fast_sin_cos;

fn normalize(v: Vec3) -> Vec3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        v
    }
}
fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// Integrate a unit quaternion by angular velocity over `dt`.
fn integrate(q: [f32; 4], w: Vec3, dt: f32) -> [f32; 4] {
    let angle = dot(w, w).sqrt() * dt;
    if angle <= 0.0 {
        return q;
    }
    let axis = normalize(w);
    let (s, c) = ((angle * 0.5).sin(), (angle * 0.5).cos());
    let r = [axis[0] * s, axis[1] * s, axis[2] * s, c];
    // r * q (world-space rotation applied after q).
    let out = [
        r[3] * q[0] + r[0] * q[3] + r[1] * q[2] - r[2] * q[1],
        r[3] * q[1] - r[0] * q[2] + r[1] * q[3] + r[2] * q[0],
        r[3] * q[2] + r[0] * q[1] - r[1] * q[0] + r[2] * q[3],
        r[3] * q[3] - r[0] * q[0] - r[1] * q[1] - r[2] * q[2],
    ];
    let l = out.iter().map(|v| v * v).sum::<f32>().sqrt();
    out.map(|v| v / l)
}

/// XNA row-major world matrix (Right, Up, Backward, Translation rows).
pub fn matrix(q: [f32; 4], position: Vec3) -> [f32; 16] {
    let [x, y, z, w] = q;
    [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + w * z),
        2.0 * (x * z - w * y),
        0.0,
        2.0 * (x * y - w * z),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + w * x),
        0.0,
        2.0 * (x * z + w * y),
        2.0 * (y * z - w * x),
        1.0 - 2.0 * (x * x + y * y),
        0.0,
        position[0],
        position[1],
        position[2],
        1.0,
    ]
}

/// Body orientation Backward (row 3 of the XNA matrix, +Z of the body).
pub fn backward(q: [f32; 4]) -> Vec3 {
    let [x, y, z, w] = q;
    [
        2.0 * (x * z + w * y),
        2.0 * (y * z - w * x),
        1.0 - 2.0 * (x * x + y * y),
    ]
}

impl Gibs {
    pub fn new(seed: i32) -> Self {
        Self {
            // Never-used entries: radius unset, mass NaN (see spawn_physics).
            cache: vec![(0.0, f32::NAN); CACHE_SIZE],
            live: Vec::new(),
            next_id: 1,
            random: DotNetRandom::new(seed),
            event_random: DotNetRandom::new(seed),
        }
    }

    pub fn cached(&self) -> usize {
        self.cache.len()
    }

    /// Character.SpawnGibs (without decals). Returns presentation requests.
    pub fn spawn(&mut self, source: GibSource, gibs: &[GibDefinition]) -> Vec<GibVisual> {
        let mut visuals = Vec::new();
        let blood = (0..BLOOD_NONE)
            .contains(&source.blood)
            .then_some(source.blood as usize);
        for gib in gibs {
            // GetFromCache returns null when empty and the loop breaks.
            let Some((stale_radius, _)) = self.cache.pop() else {
                break;
            };
            let mut position = self.random_position_on_skin(source);
            position[1] += stale_radius.max(0.1);
            let r = source.radius.max(0.2);
            let theta = std::f32::consts::PI * 2.0 * self.random.next_double() as f32;
            let phi = ((2.0 * self.random.next_double() as f32 - 1.0) as f64).acos() as f32;
            let dir = normalize([
                r * theta.cos() * phi.sin(),
                r * theta.sin() * phi.sin(),
                r * phi.cos(),
            ]);
            let mut velocity = scale(dir, 5.0);
            velocity[1] = velocity[1].abs();
            let ttl = 10.0 + self.random.next_double() as f32 * 10.0;
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            self.live.push(LiveGib {
                id,
                model: gib.model.clone(),
                scale: gib.scale,
                radius: gib.radius,
                mass: gib.mass,
                position,
                orientation: [0.0, 0.0, 0.0, 1.0],
                velocity,
                angular_velocity: scale(dir, -5.0),
                ttl,
                frozen: source.frozen,
                blood: source.blood,
                active: true,
            });
            if let (false, Some(b)) = (source.frozen, blood) {
                visuals.push(GibVisual::Trail {
                    id,
                    name: GORE_GIB_TRAIL[b].to_owned(),
                });
            }
        }
        let direction = [0.0, 0.0, -1.0];
        if source.frozen {
            visuals.push(GibVisual::OneShot {
                name: ICE_DEATH_EFFECT,
                position: source.position,
                direction,
            });
        } else if let Some(b) = blood {
            // Both the small and the medium effect start below mass 250.
            if source.mass < 250.0 {
                visuals.push(GibVisual::OneShot {
                    name: GORE_GIB_SMALL[b],
                    position: source.position,
                    direction,
                });
            }
            visuals.push(GibVisual::OneShot {
                name: if source.mass > 1000.0 {
                    GORE_GIB_LARGE[b]
                } else {
                    GORE_GIB_MEDIUM[b]
                },
                position: source.position,
                direction,
            });
        }
        visuals
    }

    /// Items.SpawnGibsEvent: selected debris, without the full character's
    /// burst/death effects or blood decals. Native Initialize still uses the
    /// owner's blood trail, including when the selected model is a shield.
    pub fn spawn_event(
        &mut self,
        source: GibSource,
        body_velocity: Vec3,
        gibs: &[GibDefinition],
    ) -> Vec<GibVisual> {
        let mut visuals = Vec::new();
        for gib in gibs {
            if self.cache.pop().is_none() {
                // Unlike Character.SpawnGibs, the event continues its index
                // loop when the cache has no entry.
                continue;
            }
            let position = self.random_position_on_skin(source);
            let direction = normalize(std::array::from_fn(|i| position[i] - source.position[i]));
            let velocity = std::array::from_fn(|i| {
                let range = if i == 1 { 9.0 } else { 3.0 };
                direction[i]
                    * (self.event_random.next_double() as f32 * range + body_velocity[i] * 0.1)
            });
            let ttl = 10.0 + self.event_random.next_double() as f32 * 10.0;
            // Initialize gives the box identity orientation and sides equal
            // to the unscaled model radius. BodyAngImpulse uses its inverse
            // inertia: mass * (side² + side²) / 12 on each axis.
            let inertia = gib.mass * (gib.radius * gib.radius * 2.0) / 12.0;
            let angular_velocity = std::array::from_fn(|_| {
                let impulse = (self.event_random.next_double() - 0.5) as f32 * gib.mass * 0.5;
                if inertia > 0.0 {
                    impulse / inertia
                } else {
                    0.0
                }
            });
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            self.live.push(LiveGib {
                id,
                model: gib.model.clone(),
                scale: gib.scale,
                radius: gib.radius,
                mass: gib.mass,
                position,
                orientation: [0.0, 0.0, 0.0, 1.0],
                velocity,
                angular_velocity,
                ttl,
                frozen: source.frozen,
                blood: source.blood,
                active: true,
            });
            if !source.frozen && (0..BLOOD_NONE).contains(&source.blood) {
                visuals.push(GibVisual::Trail {
                    id,
                    name: GORE_GIB_TRAIL[source.blood as usize].to_owned(),
                });
            }
        }
        visuals
    }

    /// DamageablePhysicsEntity.SpawnGibs (gib part; the OnDeath trigger and
    /// death conditions it also runs are the host's). Gibs start at the
    /// entity position; speed is the *cached* gib's stale body mass × 1.25,
    /// read before Initialize; the direction's Y term carries 3 × radius.
    /// Blood type is none (5), so no gore effects or blood trails start; a
    /// non-empty template GibTrailEffect follows each gib unless frozen.
    /// `fresh_definition_mass` retains the user-requested tutorial-board
    /// divergence; other props use the native never-used body mass of 1.
    pub fn spawn_physics(
        &mut self,
        position: Vec3,
        radius: f32,
        frozen: bool,
        trail: &str,
        gibs: &[GibDefinition],
        fresh_definition_mass: bool,
    ) -> Vec<GibVisual> {
        let mut visuals = Vec::new();
        for gib in gibs {
            let Some((_, stale_mass)) = self.cache.pop() else {
                break;
            };
            // Native never-used Body mass is 1. The user-requested
            // stronger tutorial-board launch is an explicit caller policy;
            // ordinary props, including tower debris, keep the native mass.
            let stale_mass = if stale_mass.is_nan() {
                if fresh_definition_mass {
                    gib.mass
                } else {
                    1.0
                }
            } else {
                stale_mass
            };
            let speed = stale_mass * 1.25;
            let r = if radius < 0.2 { 0.2 } else { radius };
            let theta = std::f32::consts::PI * 2.0 * self.random.next_double() as f32;
            let phi = ((2.0 * self.random.next_double() as f32 - 1.0) as f64).acos() as f32;
            let dir = normalize([
                r * (theta as f64).cos() as f32 * (phi as f64).sin() as f32,
                3.0 * r * (theta as f64).sin() as f32 * (phi as f64).sin() as f32,
                r * (phi as f64).cos() as f32,
            ]);
            let mut velocity = scale(dir, speed);
            velocity[1] = velocity[1].abs();
            let ttl = 10.0 + self.random.next_double() as f32 * 10.0;
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            self.live.push(LiveGib {
                id,
                model: gib.model.clone(),
                scale: gib.scale,
                radius: gib.radius,
                mass: gib.mass,
                position,
                orientation: [0.0, 0.0, 0.0, 1.0],
                velocity,
                angular_velocity: scale(dir, -5.0),
                ttl,
                frozen,
                blood: BLOOD_NONE,
                active: true,
            });
            if !frozen && !trail.is_empty() {
                visuals.push(GibVisual::Trail {
                    id,
                    name: trail.to_owned(),
                });
            }
        }
        visuals
    }

    /// Character.GetRandomPositionOnCollisionSkin.
    fn random_position_on_skin(&mut self, s: GibSource) -> Vec3 {
        let set_between = |min: f32, max: f32, t: f32| min * (1.0 - t) + max * t;
        let pi = std::f32::consts::PI;
        let a = set_between(-pi, pi, self.random.next_double() as f32);
        let b = set_between(-pi / 2.0, pi / 2.0, self.random.next_double() as f32);
        let (sin_a, cos_a) = fast_sin_cos(a);
        let (sin_b, cos_b) = fast_sin_cos(b);
        let height = s.capsule_radius * 2.0 + s.capsule_length;
        [
            cos_a * cos_b * s.capsule_radius + s.position[0],
            sin_b * height * 0.5 + s.position[1],
            sin_a * cos_b * s.capsule_radius + s.position[2],
        ]
    }

    /// Gib.Update for every gib, then the physics step of active bodies;
    /// gibs whose TTL reached zero are removed and return to the cache.
    pub fn update(
        &mut self,
        dt: f32,
        gravity: Vec3,
        scene: Option<&crate::level_collision::SceneCollision>,
        visuals: &mut Vec<GibVisual>,
    ) {
        for gib in &mut self.live {
            gib.ttl -= dt;
            if gib.ttl < 1.0 {
                gib.active = false;
                gib.position[1] -= dt * gib.radius * 2.0;
            }
            if gib.active {
                step(gib, dt, gravity, scene, visuals);
            }
        }
        let cache = &mut self.cache;
        self.live.retain(|g| {
            // Removable: TTL <= 0; Deinitialize returns it to the cache.
            if g.ttl <= 0.0 {
                cache.push((g.radius, g.mass));
                false
            } else {
                true
            }
        });
    }
}

/// Reimplemented JigLibX step: gravity, a swept sphere of half the box side
/// against level collision, zero restitution and Coulomb friction (0.8).
/// Characters and other gibs never collide (Gib.OnCollision). Contact damps
/// angular velocity by 0.99 (Body.UpdateVelocity) and scales it with the
/// friction loss so resting gibs stop spinning (INFERRED).
fn step(
    gib: &mut LiveGib,
    dt: f32,
    gravity: Vec3,
    scene: Option<&crate::level_collision::SceneCollision>,
    visuals: &mut Vec<GibVisual>,
) {
    gib.velocity = add(gib.velocity, scale(gravity, dt));
    let delta = scale(gib.velocity, dt);
    let travel = dot(delta, delta).sqrt();
    let half = gib.radius * 0.5;
    let mut fraction = 1.0;
    let mut contact = None;
    if travel > 0.0 {
        let dir = scale(delta, 1.0 / travel);
        if let Some(hit) = scene.and_then(|s| {
            s.segment_intersect(
                Segment {
                    origin: gib.position,
                    delta: add(delta, scale(dir, half)),
                },
                false,
            )
        }) {
            fraction = ((hit.hit.fraction * (travel + half) - half).max(0.0) / travel).min(1.0);
            contact = Some(hit.hit.normal);
        }
    }
    gib.position = add(gib.position, scale(delta, fraction));
    if let Some(normal) = contact {
        // OnCollision: level contact while falling faster than 10.
        if !gib.frozen && (0..BLOOD_NONE).contains(&gib.blood) && gib.velocity[1] < -10.0 {
            visuals.push(GibVisual::Splash {
                id: gib.id,
                name: GORE_GIB_SMALL[gib.blood as usize],
                position: gib.position,
                direction: backward(gib.orientation),
            });
        }
        let vn = dot(gib.velocity, normal);
        if vn < 0.0 {
            gib.velocity = add(gib.velocity, scale(normal, -vn));
            let tangent = dot(gib.velocity, gib.velocity).sqrt();
            let keep = if tangent > 0.0 {
                (1.0 - FRICTION * -vn / tangent).max(0.0)
            } else {
                0.0
            };
            gib.velocity = scale(gib.velocity, keep);
            gib.angular_velocity = scale(gib.angular_velocity, 0.99 * keep);
        }
    }
    gib.orientation = integrate(gib.orientation, gib.angular_velocity, dt);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> GibSource {
        GibSource {
            position: [1.0, 2.0, 3.0],
            capsule_radius: 0.4,
            capsule_length: 1.0,
            radius: 0.4,
            mass: 100.0,
            frozen: false,
            blood: 0,
        }
    }
    fn defs(n: usize) -> Vec<GibDefinition> {
        (0..n)
            .map(|i| GibDefinition {
                model: format!("Models/Characters/gib_{i}"),
                mass: 10.0,
                scale: 1.0,
                radius: 0.3,
            })
            .collect()
    }

    #[test]
    fn event_debris_uses_skin_radial_launch_and_only_a_trail() {
        let mut gibs = Gibs::new(17);
        let mut source = source();
        source.capsule_length = 0.0;
        source.capsule_radius = 0.8;
        let definitions = defs(32);
        let mut reused = gibs.clone();
        reused.cache = vec![(7.0, 15.0); CACHE_SIZE];
        let visuals = gibs.spawn_event(source, [0.0; 3], &definitions);
        reused.spawn_event(source, [0.0; 3], &definitions);
        assert_eq!(
            gibs.live, reused.live,
            "event spawning ignores stale cache radius/mass"
        );
        assert_eq!(gibs.live.len(), definitions.len());
        assert_eq!(visuals.len(), definitions.len());
        assert!(visuals.iter().all(|v| matches!(v, GibVisual::Trail { .. })));
        assert!(gibs.live.iter().any(|g| g.velocity[1] < 0.0));
        for g in &gibs.live {
            let offset = std::array::from_fn(|i| g.position[i] - source.position[i]);
            // Native FastSinCos uses parabolas, so sampled points are not
            // exactly on a mathematical sphere; each axis stays in bounds.
            assert!(offset.iter().all(|v| v.abs() <= source.capsule_radius));
            let direction = normalize(offset);
            for (i, &d) in direction.iter().enumerate() {
                let radial_speed = g.velocity[i] / d;
                assert!((0.0..=if i == 1 { 9.0 } else { 3.0 }).contains(&radial_speed));
            }
            assert!((10.0..20.0).contains(&g.ttl));
            let spin_bound = 1.5 / (g.radius * g.radius);
            assert!(g.angular_velocity.iter().all(|v| v.abs() <= spin_bound));
        }
        let frozen = gibs.spawn_event(
            GibSource {
                frozen: true,
                ..source
            },
            [0.0; 3],
            &defs(1),
        );
        assert!(
            frozen.is_empty(),
            "frozen event debris has no trail or ice death burst"
        );
    }

    #[test]
    fn fast_sin_cos_matches_the_polygonhead_parabola() {
        let (s, c) = fast_sin_cos(0.0);
        assert_eq!(s, 0.0);
        // cos(0) uses the parabola at pi/2.
        let x = std::f32::consts::PI / 2.0;
        assert_eq!(c, 4.0 / std::f32::consts::PI * x - 0.405_284_73 * x * x);
        let (s, _) = fast_sin_cos(-1.0);
        assert_eq!(s, -4.0 / std::f32::consts::PI + 0.405_284_73);
    }

    #[test]
    fn spawn_draws_upward_five_unit_velocities_and_ttls() {
        let mut gibs = Gibs::new(1);
        let visuals = gibs.spawn(source(), &defs(3));
        assert_eq!(gibs.live.len(), 3);
        assert_eq!(gibs.cached(), CACHE_SIZE - 3);
        for g in &gibs.live {
            let speed = dot(g.velocity, g.velocity).sqrt();
            assert!((speed - 5.0).abs() < 1e-4);
            assert!(g.velocity[1] >= 0.0);
            assert!((10.0..20.0).contains(&g.ttl));
            // Angular velocity is −5 × the unflipped direction.
            assert!((dot(g.angular_velocity, g.angular_velocity).sqrt() - 5.0).abs() < 1e-4);
            // Fresh cache entries have radius 0: spawn height offset 0.1.
            assert!(g.position[1] >= 2.0 - 0.9 + 0.1 - 1e-5);
            assert!(g.position[1] <= 2.0 + 0.9 + 0.1 + 1e-5);
        }
        // Three trails, then small (mass < 250) and medium gore effects.
        let names: Vec<_> = visuals
            .iter()
            .map(|v| match v {
                GibVisual::Trail { name, .. } => name.as_str(),
                GibVisual::OneShot { name, .. } | GibVisual::Splash { name, .. } => name,
            })
            .collect();
        assert_eq!(
            names,
            [
                "gore_gib_regular_trail",
                "gore_gib_regular_trail",
                "gore_gib_regular_trail",
                "gore_gib_regular_small",
                "gore_gib_regular_medium",
            ]
        );
    }

    #[test]
    fn frozen_and_bloodless_gibbing_choose_original_effects() {
        let mut gibs = Gibs::new(2);
        let frozen = gibs.spawn(
            GibSource {
                frozen: true,
                ..source()
            },
            &defs(1),
        );
        assert_eq!(
            frozen,
            [GibVisual::OneShot {
                name: ICE_DEATH_EFFECT,
                position: [1.0, 2.0, 3.0],
                direction: [0.0, 0.0, -1.0],
            }]
        );
        let none = gibs.spawn(
            GibSource {
                blood: BLOOD_NONE,
                ..source()
            },
            &defs(1),
        );
        assert!(none.is_empty());
        let heavy = gibs.spawn(
            GibSource {
                mass: 2000.0,
                ..source()
            },
            &[],
        );
        assert!(matches!(
            heavy[..],
            [GibVisual::OneShot {
                name: "gore_gib_regular_large",
                ..
            }]
        ));
    }

    #[test]
    fn empty_cache_stops_spawning_and_removal_returns_stale_radius() {
        let mut gibs = Gibs::new(3);
        gibs.spawn(source(), &defs(CACHE_SIZE + 5));
        assert_eq!(gibs.live.len(), CACHE_SIZE);
        let mut visuals = Vec::new();
        gibs.update(25.0, [0.0, -19.64, 0.0], None, &mut visuals);
        assert!(gibs.live.is_empty());
        assert_eq!(gibs.cached(), CACHE_SIZE);
        // The next spawn's height uses the previous gib's radius (0.3).
        let mut again = Gibs::new(3);
        again.spawn(source(), &defs(1));
        let fresh = again.live[0].position[1];
        let mut reused = Gibs::new(3);
        reused.spawn(source(), &defs(1));
        reused.update(25.0, [0.0; 3], None, &mut visuals);
        reused.random = DotNetRandom::new(3);
        reused.spawn(source(), &defs(1));
        assert!((reused.live[0].position[1] - (fresh - 0.1 + 0.3)).abs() < 1e-5);
    }

    #[test]
    fn last_second_sinks_without_physics() {
        let mut gibs = Gibs::new(4);
        gibs.spawn(source(), &defs(1));
        gibs.live[0].ttl = 1.0;
        let start = gibs.live[0].position;
        let mut visuals = Vec::new();
        gibs.update(0.5, [0.0, -19.64, 0.0], None, &mut visuals);
        let g = &gibs.live[0];
        assert!(!g.active);
        assert_eq!(g.position[0], start[0]);
        assert!((g.position[1] - (start[1] - 0.5 * 0.3 * 2.0)).abs() < 1e-6);
    }
}
