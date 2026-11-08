//! PolygonHead particle effects and Magicka's EffectManager, presentation
//! free: VisualEffect XML, ContinuousEmitter/PulseEmitter emission,
//! ParticleHelper's cached random tables, the 32768-slot ParticleSystem and
//! the 256-slot effect instance table. See research/particles.md.
//!
//! Matrices are XNA row-major with row vectors (translation in 12..15).
use crate::particle_lights::{
    parse_variation_type, BatchedPointLight, LightRecord, LightTransition, ParticleLight,
    ParticleLightBatcher, PointLightBatcher, PointLightRef,
};
use crate::scene_script::{xml, XmlElement};
use magicka_content::skinned::{mul, Mat, IDENTITY};
use std::{collections::HashMap, path::Path, sync::OnceLock};

pub type Vec3 = [f32; 3];

// ---------------------------------------------------------------------------
// System.Random (.NET Framework, Knuth subtractive generator)

/// `System.Random` of the .NET Framework the game runs on.
#[derive(Clone, Debug)]
pub struct DotNetRandom {
    seed: [i32; 56],
    inext: usize,
    inextp: usize,
}

impl DotNetRandom {
    const MBIG: i32 = i32::MAX;
    const MSEED: i32 = 161_803_398;

    pub fn new(seed: i32) -> Self {
        let subtraction = if seed == i32::MIN {
            i32::MAX
        } else {
            seed.abs()
        };
        let mut a = [0i32; 56];
        let mut mj = Self::MSEED - subtraction;
        a[55] = mj;
        let mut mk = 1i32;
        for i in 1..55 {
            let ii = (21 * i) % 55;
            a[ii] = mk;
            mk = mj - mk;
            if mk < 0 {
                mk += Self::MBIG;
            }
            mj = a[ii];
        }
        for _ in 1..5 {
            for i in 1..56 {
                a[i] = a[i].wrapping_sub(a[1 + (i + 30) % 55]);
                if a[i] < 0 {
                    a[i] += Self::MBIG;
                }
            }
        }
        Self {
            seed: a,
            inext: 0,
            inextp: 21,
        }
    }

    fn internal_sample(&mut self) -> i32 {
        let mut next = self.inext + 1;
        if next >= 56 {
            next = 1;
        }
        let mut nextp = self.inextp + 1;
        if nextp >= 56 {
            nextp = 1;
        }
        let mut value = self.seed[next].wrapping_sub(self.seed[nextp]);
        if value == Self::MBIG {
            value -= 1;
        }
        if value < 0 {
            value += Self::MBIG;
        }
        self.seed[next] = value;
        self.inext = next;
        self.inextp = nextp;
        value
    }

    pub fn next_int(&mut self) -> i32 {
        self.internal_sample()
    }
    pub fn next_double(&mut self) -> f64 {
        f64::from(self.internal_sample()) * (1.0 / f64::from(Self::MBIG))
    }
    /// `Next(maxValue)`.
    pub fn next_below(&mut self, max: i32) -> i32 {
        (self.next_double() * f64::from(max)) as i32
    }
}

// ---------------------------------------------------------------------------
// ParticleHelper

pub const RANDOM_CACHE_SIZE: usize = 65536;

struct RandomTables {
    floats: Vec<f32>,
    bytes: Vec<u8>,
}

/// ParticleHelper's static tables from `new Random(0)`: 65536 NextDouble
/// floats, then 65536 `Next(100)` bytes.
fn tables() -> &'static RandomTables {
    static TABLES: OnceLock<RandomTables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut random = DotNetRandom::new(0);
        let floats = (0..RANDOM_CACHE_SIZE)
            .map(|_| random.next_double() as f32)
            .collect();
        let bytes = (0..RANDOM_CACHE_SIZE)
            .map(|_| random.next_below(100) as u8)
            .collect();
        RandomTables { floats, bytes }
    })
}

/// An emitter's `mRandomOffset` cursor into the cached tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RandomCursor(pub u16);

impl RandomCursor {
    fn float(&mut self) -> f32 {
        let v = tables().floats[usize::from(self.0)];
        self.0 = self.0.wrapping_add(1);
        v
    }
    fn byte(&mut self) -> u8 {
        let v = tables().bytes[usize::from(self.0)];
        self.0 = self.0.wrapping_add(1);
        v
    }
    /// GetRandomValue(min, max).
    fn range(&mut self, min: f32, max: f32) -> f32 {
        self.float() * (max - min) + min
    }
    /// GetRandomValue(min, max, distribution): Math.Pow in double.
    fn dist(&mut self, min: f32, max: f32, distribution: f32) -> f32 {
        pow(self.float(), distribution) * (max - min) + min
    }
    /// GetRandomSign: -1 when the byte is below the negative percentage.
    fn sign(&mut self, percent_negative: i32) -> f32 {
        if i32::from(self.byte()) >= percent_negative {
            1.0
        } else {
            -1.0
        }
    }
    /// GetRandomDirectionCone.
    fn cone(&mut self, orientation: &Mat, angle: f32, distribution: f32) -> Vec3 {
        let z = pow(self.float(), distribution) * (angle / std::f32::consts::PI) * 2.0 - 1.0;
        let a = self.float() * (std::f32::consts::PI * 2.0);
        let r = (1.0 - z * z).sqrt();
        let d = [r * cos(a), r * sin(a), z];
        transform_normal(d, orientation)
    }
    /// GetRandomDirectionArc.
    #[allow(clippy::too_many_arguments)]
    fn arc(
        &mut self,
        orientation: &Mat,
        horizontal: f32,
        horizontal_distribution: f32,
        vertical_min: f32,
        vertical_max: f32,
        vertical_distribution: f32,
    ) -> Vec3 {
        let h = self.float() * 2.0 - 1.0;
        let h = pow(h.abs(), horizontal_distribution) * sign(h) * horizontal;
        let mut d = [sin(h), 0.0, -cos(h)];
        let v = self.float() * 2.0 - 1.0;
        let v = (pow(v.abs(), vertical_distribution) * sign(v) * (vertical_max - vertical_min)
            + (vertical_min + vertical_max))
            * 0.5;
        let c = cos(v);
        d[1] = sin(v);
        d[0] *= c;
        d[2] *= c;
        transform_normal(d, orientation)
    }
}

fn pow(x: f32, y: f32) -> f32 {
    f64::from(x).powf(f64::from(y)) as f32
}
fn sin(x: f32) -> f32 {
    f64::from(x).sin() as f32
}
fn cos(x: f32) -> f32 {
    f64::from(x).cos() as f32
}
/// Math.Sign(float) as a float factor (0 for zero).
fn sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// XNA math

pub fn transform_point(v: Vec3, m: &Mat) -> Vec3 {
    [
        v[0] * m[0] + v[1] * m[4] + v[2] * m[8] + m[12],
        v[0] * m[1] + v[1] * m[5] + v[2] * m[9] + m[13],
        v[0] * m[2] + v[1] * m[6] + v[2] * m[10] + m[14],
    ]
}
pub fn transform_normal(v: Vec3, m: &Mat) -> Vec3 {
    [
        v[0] * m[0] + v[1] * m[4] + v[2] * m[8],
        v[0] * m[1] + v[1] * m[5] + v[2] * m[9],
        v[0] * m[2] + v[1] * m[6] + v[2] * m[10],
    ]
}
/// Matrix.CreateRotationY (System.Math in double, as XNA does).
pub fn rotation_y(radians: f32) -> Mat {
    let (s, c) = (sin(radians), cos(radians));
    [
        c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}
fn lerp_mat(a: &Mat, b: &Mat, t: f32) -> Mat {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}
fn lerp3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(v: Vec3) -> Vec3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let inv = 1.0 / l;
    v.map(|x| x * inv)
}
/// Matrix.CreateWorld(position, forward, up) as XNA defines it:
/// z = normalize(-forward), x = normalize(up × z), y = z × x (INFERRED: the
/// XNA framework assembly is not among the target files).
pub fn create_world(position: Vec3, forward: Vec3, up: Vec3) -> Mat {
    let z = normalize(forward.map(|v| -v));
    let x = normalize(cross(up, z));
    let y = cross(z, x);
    [
        x[0],
        x[1],
        x[2],
        0.0,
        y[0],
        y[1],
        y[2],
        0.0,
        z[0],
        z[1],
        z[2],
        0.0,
        position[0],
        position[1],
        position[2],
        1.0,
    ]
}
/// MagickaMath.MakeOrientationMatrix (forward is used as given).
pub fn orientation_matrix(forward: Vec3) -> Mat {
    let right = normalize(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    [
        right[0],
        right[1],
        right[2],
        0.0,
        up[0],
        up[1],
        up[2],
        0.0,
        -forward[0],
        -forward[1],
        -forward[2],
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

// ---------------------------------------------------------------------------
// XML

#[derive(Debug, Clone, PartialEq)]
pub enum EffectError {
    Xml(String),
    /// A value the original's int/float/bool/enum Parse would reject.
    Parse(String, String),
    /// A keyed field the original dereferences after reading (null there).
    Missing(&'static str),
    Io(String),
    /// Dictionary.Add of an effect name hash that is already registered.
    Duplicate(String),
}

fn attr<'a>(node: &'a XmlElement, name: &str) -> Option<&'a str> {
    node.attributes
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}
/// float.Parse(InvariantCulture): .NET Framework parses to double first.
fn parse_f32(s: &str) -> Result<f32, EffectError> {
    s.trim()
        .parse::<f64>()
        .map(|v| v as f32)
        .map_err(|_| EffectError::Parse("float".into(), s.into()))
}
fn parse_i32(s: &str) -> Result<i32, EffectError> {
    s.trim()
        .parse::<i32>()
        .map_err(|_| EffectError::Parse("int".into(), s.into()))
}
fn parse_bool(s: &str) -> Result<bool, EffectError> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(EffectError::Parse("bool".into(), s.into())),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyedValue {
    pub time: f32,
    pub value: f32,
}

/// ParticleHelper.GetKeyedValue: null → 1, one key → its value, otherwise
/// a linear blend between the last key at/before and the first after.
pub fn keyed_value(keys: Option<&[KeyedValue]>, time: f32) -> f32 {
    let Some(keys) = keys else {
        return 1.0;
    };
    if keys.len() == 1 {
        return keys[0].value;
    }
    let (mut a, mut b) = (None, None);
    for k in keys {
        if k.time <= time {
            a = Some(*k);
        }
        if k.time > time {
            b = Some(*k);
            break;
        }
    }
    match (a, b) {
        (Some(a), Some(b)) => {
            let t = (time - a.time) / (b.time - a.time);
            a.value * (1.0 - t) + b.value * t
        }
        (None, Some(b)) => b.value,
        (Some(a), None) => a.value,
        // Empty array: the original returns default(KeyedValue).Value.
        (None, None) => 0.0,
    }
}

// ---------------------------------------------------------------------------
// ContinuousEmitter

/// Keyed (KeyedValue[]) fields of ContinuousEmitter, by XML name.
const KEYED: [&str; 61] = [
    "ParticlesPerSecond",
    "PositionX",
    "PositionY",
    "PositionZ",
    "PositionXOffset",
    "PositionYOffset",
    "PositionZOffset",
    "RotationY",
    "Gravity",
    "Drag",
    "ColorR",
    "ColorG",
    "ColorB",
    "ColorAMin",
    "ColorAMax",
    "ColorADistribution",
    "HueMin",
    "HueMax",
    "HueDistribution",
    "SatMin",
    "SatMax",
    "SatDistribution",
    "ValueMin",
    "ValueMax",
    "ValueDistribution",
    "AlphaMin",
    "AlphaMax",
    "AlphaDistribution",
    "LifeTimeMin",
    "LifeTimeMax",
    "LifeTimeDistribution",
    "SizeStartMin",
    "SizeStartMax",
    "SizeStartDistribution",
    "SizeEndMin",
    "SizeEndMax",
    "SizeEndDistribution",
    "VelocityMin",
    "VelocityMax",
    "VelocityDistribution",
    "RotationMin",
    "RotationMax",
    "RotationSpeedMin",
    "RotationSpeedMax",
    "RotationPCCW",
    "SpreadConeAngle",
    "SpreadConeDistribution",
    "SpreadArcHorizontalAngle",
    "SpreadArcHorizontalDistribution",
    "SpreadArcVerticalMin",
    "SpreadArcVerticalMax",
    "SpreadArcVerticalDistribution",
    "LightRadius",
    "LightDiffuseR",
    "LightDiffuseG",
    "LightDiffuseB",
    "LightAmbientR",
    "LightAmbientG",
    "LightAmbientB",
    "LightSpecular",
    // Not a field name; keeps the array length a named constant below.
    "",
];
#[allow(dead_code)]
mod k {
    pub const PPS: usize = 0;
    pub const POS_X: usize = 1;
    pub const POS_Y: usize = 2;
    pub const POS_Z: usize = 3;
    pub const OFF_X: usize = 4;
    pub const OFF_Y: usize = 5;
    pub const OFF_Z: usize = 6;
    pub const ROT_Y: usize = 7;
    pub const GRAVITY: usize = 8;
    pub const DRAG: usize = 9;
    pub const COLOR_R: usize = 10;
    pub const COLOR_G: usize = 11;
    pub const COLOR_B: usize = 12;
    pub const COLOR_A_MIN: usize = 13;
    pub const COLOR_A_MAX: usize = 14;
    pub const COLOR_A_DIST: usize = 15;
    pub const HUE_MIN: usize = 16;
    pub const HUE_MAX: usize = 17;
    pub const HUE_DIST: usize = 18;
    pub const SAT_MIN: usize = 19;
    pub const SAT_MAX: usize = 20;
    pub const SAT_DIST: usize = 21;
    pub const VALUE_MIN: usize = 22;
    pub const VALUE_MAX: usize = 23;
    pub const VALUE_DIST: usize = 24;
    pub const ALPHA_MIN: usize = 25;
    pub const ALPHA_MAX: usize = 26;
    pub const ALPHA_DIST: usize = 27;
    pub const LIFE_MIN: usize = 28;
    pub const LIFE_MAX: usize = 29;
    pub const LIFE_DIST: usize = 30;
    pub const SIZE_START_MIN: usize = 31;
    pub const SIZE_START_MAX: usize = 32;
    pub const SIZE_START_DIST: usize = 33;
    pub const SIZE_END_MIN: usize = 34;
    pub const SIZE_END_MAX: usize = 35;
    pub const SIZE_END_DIST: usize = 36;
    pub const VEL_MIN: usize = 37;
    pub const VEL_MAX: usize = 38;
    pub const VEL_DIST: usize = 39;
    pub const ROT_MIN: usize = 40;
    pub const ROT_MAX: usize = 41;
    pub const ROT_SPEED_MIN: usize = 42;
    pub const ROT_SPEED_MAX: usize = 43;
    pub const ROT_PCCW: usize = 44;
    pub const CONE_ANGLE: usize = 45;
    pub const CONE_DIST: usize = 46;
    pub const ARC_H: usize = 47;
    pub const ARC_H_DIST: usize = 48;
    pub const ARC_V_MIN: usize = 49;
    pub const ARC_V_MAX: usize = 50;
    pub const ARC_V_DIST: usize = 51;
    pub const LIGHT_RADIUS: usize = 52;
    pub const LIGHT_DIFFUSE_R: usize = 53;
    pub const LIGHT_DIFFUSE_G: usize = 54;
    pub const LIGHT_DIFFUSE_B: usize = 55;
    pub const LIGHT_AMBIENT_R: usize = 56;
    pub const LIGHT_AMBIENT_G: usize = 57;
    pub const LIGHT_AMBIENT_B: usize = 58;
    pub const LIGHT_SPECULAR: usize = 59;
    pub const COUNT: usize = 60;
}

/// Shared emitter flags (both emitter kinds).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EmitterFlags {
    pub alpha_blended: bool,
    pub cone_spread: bool,
    pub rotation_aligned: bool,
    pub hsv: bool,
    pub colorize: bool,
    pub relative_velocity: bool,
    pub particle: u8,
    pub spawn_lights: bool,
    pub light_size_relative: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContinuousEmitter {
    keys: Vec<Option<Vec<KeyedValue>>>,
    pub flags: EmitterFlags,
    pub cursor: RandomCursor,
}

/// The fields both versions convert from degrees after reading.
const CONTINUOUS_RADIANS: [usize; 11] = [
    k::ROT_Y,
    k::HUE_MIN,
    k::HUE_MAX,
    k::ROT_MIN,
    k::ROT_MAX,
    k::ROT_SPEED_MIN,
    k::ROT_SPEED_MAX,
    k::CONE_ANGLE,
    k::ARC_H,
    k::ARC_V_MIN,
    k::ARC_V_MAX,
];

impl ContinuousEmitter {
    pub fn read(
        node: &XmlElement,
        keyframes_per_second: i32,
        version: i32,
        cursor: RandomCursor,
    ) -> Result<Self, EffectError> {
        let mut keys: Vec<Option<Vec<KeyedValue>>> = vec![None; k::COUNT];
        let mut flags = EmitterFlags::default();
        for child in &node.children {
            // Field lookup is case-sensitive (Dictionary<string, FieldInfo>).
            if let Some(i) = KEYED[..k::COUNT].iter().position(|n| *n == child.name) {
                if !child.children.is_empty() || child.non_element_children {
                    if child.non_element_children {
                        return Err(EffectError::Xml(format!(
                            "{}: non-element keyframe child",
                            child.name
                        )));
                    }
                    let mut values = Vec::with_capacity(child.children.len());
                    for key in &child.children {
                        let mut v = KeyedValue::default();
                        for (name, value) in &key.attributes {
                            if name.eq_ignore_ascii_case("time") {
                                v.time = parse_i32(value)? as f32 / keyframes_per_second as f32;
                            } else if name.eq_ignore_ascii_case("value") {
                                v.value = parse_f32(value)?;
                            }
                        }
                        values.push(v);
                    }
                    keys[i] = Some(values);
                } else {
                    let mut v = KeyedValue::default();
                    for (name, value) in &child.attributes {
                        if name.eq_ignore_ascii_case("value") {
                            v.value = parse_f32(value)?;
                        }
                    }
                    keys[i] = Some(vec![v]);
                }
                continue;
            }
            for (name, value) in &child.attributes {
                if !name.eq_ignore_ascii_case("value") {
                    continue;
                }
                read_flag(
                    &child.name,
                    value,
                    version >= 2,
                    true,
                    &mut flags,
                    &mut None,
                )?;
            }
        }
        if version < 2 {
            // ReadVersion1: alpha keys become the colour-alpha keys, then
            // colour, value and alpha are reset to constant 1.
            keys[k::COLOR_A_MIN] = keys[k::ALPHA_MIN].take();
            keys[k::COLOR_A_MAX] = keys[k::ALPHA_MAX].take();
            keys[k::COLOR_A_DIST] = keys[k::ALPHA_DIST].take();
            let one = || {
                Some(vec![KeyedValue {
                    time: 0.0,
                    value: 1.0,
                }])
            };
            for i in [
                k::COLOR_R,
                k::COLOR_G,
                k::COLOR_B,
                k::VALUE_MIN,
                k::VALUE_MAX,
                k::VALUE_DIST,
                k::ALPHA_MIN,
                k::ALPHA_MAX,
                k::ALPHA_DIST,
            ] {
                keys[i] = one();
            }
        }
        for i in CONTINUOUS_RADIANS {
            // The original loops over `.Length` and throws on a null array.
            let values = keys[i].as_mut().ok_or(EffectError::Missing(KEYED[i]))?;
            for v in values {
                v.value = to_radians(v.value);
            }
        }
        Ok(Self {
            keys,
            flags,
            cursor,
        })
    }

    fn key(&self, i: usize, t: f32) -> f32 {
        keyed_value(self.keys[i].as_deref(), t)
    }

    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        dt: f32,
        previous_total: f32,
        total: f32,
        timeline: f32,
        previous_timeline: f32,
        transform: &Mat,
        previous_transform: &Mat,
        multiplier: f32,
        particles: &mut ParticleSystem,
    ) {
        let inv = 1.0 / dt;
        let mut emitter_velocity = [
            (transform[12] - previous_transform[12]) * inv,
            (transform[13] - previous_transform[13]) * inv,
            (transform[14] - previous_transform[14]) * inv,
        ];
        let mut rate = (self.key(k::PPS, timeline) + self.key(k::PPS, timeline + dt)) * 0.5;
        rate *= multiplier;
        let before = previous_total * rate;
        let after = total * rate;
        let count = after as i32 - before as i32;
        if count <= 0 {
            return;
        }
        let at = |e: &Self, t: f32| {
            let mut m = rotation_y(e.key(k::ROT_Y, t));
            m[12] = e.key(k::POS_X, t);
            m[13] = e.key(k::POS_Y, t);
            m[14] = e.key(k::POS_Z, t);
            let offset = [e.key(k::OFF_X, t), e.key(k::OFF_Y, t), e.key(k::OFF_Z, t)];
            (m, offset)
        };
        let (m0, offset0) = at(self, previous_timeline);
        let m0 = mul(&m0, previous_transform);
        let (m1, offset1) = at(self, timeline);
        let m1 = mul(&m1, transform);
        let t = timeline;
        let f = self.flags;
        let hsv = f.hsv.then(|| {
            [
                [
                    self.key(k::HUE_MIN, t),
                    self.key(k::HUE_MAX, t),
                    self.key(k::HUE_DIST, t),
                ],
                [
                    self.key(k::SAT_MIN, t),
                    self.key(k::SAT_MAX, t),
                    self.key(k::SAT_DIST, t),
                ],
                [
                    self.key(k::VALUE_MIN, t),
                    self.key(k::VALUE_MAX, t),
                    self.key(k::VALUE_DIST, t),
                ],
                [
                    self.key(k::ALPHA_MIN, t),
                    self.key(k::ALPHA_MAX, t),
                    self.key(k::ALPHA_DIST, t),
                ],
            ]
        });
        let color_alpha = [
            self.key(k::COLOR_A_MIN, t),
            self.key(k::COLOR_A_MAX, t),
            self.key(k::COLOR_A_DIST, t),
        ];
        let rotation = [
            self.key(k::ROT_MIN, t),
            self.key(k::ROT_MAX, t),
            self.key(k::ROT_SPEED_MIN, t),
            self.key(k::ROT_SPEED_MAX, t),
        ];
        let pccw = self.key(k::ROT_PCCW, t) as i32;
        let spread = if f.cone_spread {
            [
                self.key(k::CONE_ANGLE, t),
                self.key(k::CONE_DIST, t),
                0.0,
                0.0,
                0.0,
            ]
        } else {
            [
                self.key(k::ARC_H, t),
                self.key(k::ARC_H_DIST, t),
                self.key(k::ARC_V_MIN, t),
                self.key(k::ARC_V_MAX, t),
                self.key(k::ARC_V_DIST, t),
            ]
        };
        let size_start = [
            self.key(k::SIZE_START_MIN, t),
            self.key(k::SIZE_START_MAX, t),
            self.key(k::SIZE_START_DIST, t),
        ];
        let size_end = [
            self.key(k::SIZE_END_MIN, t),
            self.key(k::SIZE_END_MAX, t),
            self.key(k::SIZE_END_DIST, t),
        ];
        let life = [
            self.key(k::LIFE_MIN, t),
            self.key(k::LIFE_MAX, t),
            self.key(k::LIFE_DIST, t),
        ];
        let velocity = [
            self.key(k::VEL_MIN, t),
            self.key(k::VEL_MAX, t),
            self.key(k::VEL_DIST, t),
        ];
        // ParticleLightProperties at the timeline: LightRadius is read
        // regardless of SpawnLights; the colours only when lights spawn.
        let light_radius = self.key(k::LIGHT_RADIUS, t);
        let mut light = ParticleLight::default();
        if f.spawn_lights {
            light.diffuse = [
                self.key(k::LIGHT_DIFFUSE_R, t),
                self.key(k::LIGHT_DIFFUSE_G, t),
                self.key(k::LIGHT_DIFFUSE_B, t),
            ];
            light.ambient = [
                self.key(k::LIGHT_AMBIENT_R, t),
                self.key(k::LIGHT_AMBIENT_G, t),
                self.key(k::LIGHT_AMBIENT_B, t),
            ];
            light.specular = self.key(k::LIGHT_SPECULAR, t);
            light.radius_start = light_radius;
            light.radius_end = light_radius;
        }
        let mut p = Particle {
            sprite: f.particle,
            alpha_blended: f.alpha_blended,
            drag: self.key(k::DRAG, t),
            gravity: self.key(k::GRAVITY, t),
            rotation_aligned: f.rotation_aligned,
            hsv: f.hsv,
            colorize: f.colorize,
            ..Particle::default()
        };
        if !f.hsv {
            p.color[0] = self.key(k::COLOR_R, t);
            p.color[1] = self.key(k::COLOR_G, t);
            p.color[2] = self.key(k::COLOR_B, t);
        }
        if !f.relative_velocity {
            emitter_velocity = [0.0; 3];
        }
        let step = 1.0 / count as f32;
        let mut lerp = step;
        let c = &mut self.cursor;
        for _ in 0..count {
            let offset = lerp3(offset0, offset1, lerp);
            let m = lerp_mat(&m0, &m1, lerp);
            let direction = if f.cone_spread {
                c.cone(&m, spread[0], spread[1])
            } else {
                c.arc(&m, spread[0], spread[1], spread[2], spread[3], spread[4])
            };
            // SetCommonParameters (ParticleParameters.EmitterVelocity is
            // never assigned, so it adds zero).
            let local = [
                (c.float() * 2.0 - 1.0) * offset[0],
                (c.float() * 2.0 - 1.0) * offset[1],
                (c.float() * 2.0 - 1.0) * offset[2],
            ];
            p.position = transform_point(local, &m);
            p.start_size = c.dist(size_start[0], size_start[1], size_start[2]);
            p.end_size = c.dist(size_end[0], size_end[1], size_end[2]);
            p.ttl = c.dist(life[0], life[1], life[2]);
            let speed = c.dist(velocity[0], velocity[1], velocity[2]);
            p.velocity = direction.map(|d| d * speed + 0.0);
            for (v, e) in p.velocity.iter_mut().zip(emitter_velocity) {
                *v += e;
            }
            if !f.rotation_aligned {
                p.rotation = c.range(rotation[0], rotation[1]);
                let s = c.range(rotation[2], rotation[3]);
                p.rotation_velocity = s * c.sign(pccw);
            }
            if let Some(h) = hsv {
                p.color[0] = c.dist(h[0][0], h[0][1], h[0][2]);
                p.color[1] = c.dist(h[1][0], h[1][1], h[1][2]);
                p.color[2] = c.dist(h[2][0], h[2][1], h[2][2]);
                p.color[3] = c.dist(h[3][0], h[3][1], h[3][2]);
            } else {
                p.color[3] = c.dist(color_alpha[0], color_alpha[1], color_alpha[2]);
            }
            particles.spawn(&p);
            if f.spawn_lights {
                if f.light_size_relative {
                    light.radius_start = p.start_size * light_radius;
                    light.radius_end = p.end_size * light_radius;
                }
                particles.spawn_light(&p, &light);
            }
            lerp += step;
        }
    }
}

fn to_radians(degrees: f32) -> f32 {
    // MathHelper.ToRadians: degrees · (float)(π/180) (INFERRED, XNA is not
    // among the target files).
    degrees * (std::f32::consts::PI / 180.0)
}

/// Non-keyed flag elements. `continuous` selects ContinuousEmitter's
/// readers; PulseEmitter additionally reads NrOfParticles.
fn read_flag(
    name: &str,
    value: &str,
    version2: bool,
    continuous: bool,
    flags: &mut EmitterFlags,
    count: &mut Option<i32>,
) -> Result<(), EffectError> {
    let is = |n: &str| name.eq_ignore_ascii_case(n);
    if is("BlendMode") {
        flags.alpha_blended = value.eq_ignore_ascii_case("Alpha");
    } else if is("SpreadType") {
        flags.cone_spread = value.eq_ignore_ascii_case("Cone");
    } else if is("RotationAligned") {
        flags.rotation_aligned = parse_bool(value)?;
    } else if is("RelativeVelocity") {
        // PulseEmitter.ReadVersion1 explicitly ignores RelativeVelocity.
        if continuous || version2 {
            flags.relative_velocity = parse_bool(value)?;
        }
    } else if is("ColorControlAlpha") {
        flags.hsv = !parse_bool(value)?;
    } else if is("HSV") {
        flags.hsv = parse_bool(value)?;
    } else if is("Colorize") {
        flags.colorize = parse_bool(value)?;
    } else if is("Particle") {
        flags.particle = value
            .trim()
            .parse::<u8>()
            .map_err(|_| EffectError::Parse("byte".into(), value.into()))?;
    } else if !continuous && is("NrOfParticles") {
        *count = Some(parse_i32(value)?);
    } else if version2 && is("SpawnLights") {
        flags.spawn_lights = parse_bool(value)?;
    } else if version2 && is("LightSizeRelative") {
        flags.light_size_relative = parse_bool(value)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// PulseEmitter

/// PulseEmitter float fields, by XML name.
const PULSE_FIELDS: [&str; 64] = [
    "PositionX",
    "PositionY",
    "PositionZ",
    "PositionXOffset",
    "PositionYOffset",
    "PositionZOffset",
    "RotationY",
    "Gravity",
    "Drag",
    "ColorR",
    "ColorG",
    "ColorB",
    "ColorAMin",
    "ColorAMax",
    "ColorADistribution",
    "HueMin",
    "HueMax",
    "HueDistribution",
    "SatMin",
    "SatMax",
    "SatDistribution",
    "ValueMin",
    "ValueMax",
    "ValueDistribution",
    "AlphaMin",
    "AlphaMax",
    "AlphaDistribution",
    "LifeTimeMin",
    "LifeTimeMax",
    "LifeTimeDistribution",
    "SizeStartMin",
    "SizeStartMax",
    "SizeStartDistribution",
    "SizeEndMin",
    "SizeEndMax",
    "SizeEndDistribution",
    "VelocityMin",
    "VelocityMax",
    "VelocityDistribution",
    "RotationMin",
    "RotationMax",
    "RotationSpeedMin",
    "RotationSpeedMax",
    "RotationPCCW",
    "SpreadConeAngle",
    "SpreadConeDistribution",
    "SpreadArcHorizontalAngle",
    "SpreadArcHorizontalDistribution",
    "SpreadArcVerticalMin",
    "SpreadArcVerticalMax",
    "SpreadArcVerticalDistribution",
    "TriggerTime",
    "LightRadius",
    "LightDiffuseR",
    "LightDiffuseG",
    "LightDiffuseB",
    "LightAmbientR",
    "LightAmbientG",
    "LightAmbientB",
    "LightSpecular",
    "",
    "",
    "",
    "",
];
const PULSE_COUNT: usize = 60;

#[derive(Clone, Debug, PartialEq)]
pub struct PulseEmitter {
    v: [f32; PULSE_COUNT],
    pub count: i32,
    pub flags: EmitterFlags,
    pub cursor: RandomCursor,
}

fn pulse_index(name: &str) -> usize {
    PULSE_FIELDS[..PULSE_COUNT]
        .iter()
        .position(|n| *n == name)
        .expect("pulse field")
}

impl PulseEmitter {
    pub fn read(
        node: &XmlElement,
        version: i32,
        cursor: RandomCursor,
    ) -> Result<Self, EffectError> {
        let mut v = [0.0f32; PULSE_COUNT];
        let mut flags = EmitterFlags::default();
        let mut count = None;
        for child in &node.children {
            if let Some(i) = PULSE_FIELDS[..PULSE_COUNT]
                .iter()
                .position(|n| *n == child.name)
            {
                let mut value = 0.0;
                for (name, raw) in &child.attributes {
                    if name.eq_ignore_ascii_case("value") {
                        value = parse_f32(raw)?;
                    }
                }
                v[i] = value;
                continue;
            }
            for (name, value) in &child.attributes {
                if name.eq_ignore_ascii_case("value") {
                    read_flag(
                        &child.name,
                        value,
                        version >= 2,
                        false,
                        &mut flags,
                        &mut count,
                    )?;
                }
            }
        }
        let i = pulse_index;
        if version < 2 {
            v[i("ColorAMin")] = v[i("AlphaMin")];
            v[i("ColorAMax")] = v[i("AlphaMax")];
            v[i("ColorADistribution")] = v[i("AlphaDistribution")];
            for n in [
                "ColorR",
                "ColorG",
                "ColorB",
                "ValueMin",
                "ValueMax",
                "ValueDistribution",
                "AlphaMin",
                "AlphaMax",
                "AlphaDistribution",
            ] {
                v[i(n)] = 1.0;
            }
        }
        for n in [
            "RotationY",
            "HueMin",
            "HueMax",
            "RotationMin",
            "RotationMax",
            "RotationSpeedMin",
            "RotationSpeedMax",
            "SpreadConeAngle",
            "SpreadArcHorizontalAngle",
            "SpreadArcVerticalMin",
            "SpreadArcVerticalMax",
        ] {
            v[i(n)] = to_radians(v[i(n)]);
        }
        Ok(Self {
            v,
            count: count.unwrap_or(0),
            flags,
            cursor,
        })
    }

    fn f(&self, name: &str) -> f32 {
        self.v[pulse_index(name)]
    }

    #[allow(clippy::too_many_arguments, clippy::neg_cmp_op_on_partial_ord)]
    fn update(
        &mut self,
        dt: f32,
        timeline: f32,
        previous_timeline: f32,
        transform: &Mat,
        previous_transform: &Mat,
        multiplier: f32,
        particles: &mut ParticleSystem,
    ) {
        let trigger = self.f("TriggerTime");
        // Negated comparisons keep the original's NaN behaviour.
        if !(timeline > trigger) || !(previous_timeline <= trigger || previous_timeline > timeline)
        {
            return;
        }
        let inv = 1.0 / dt;
        let mut emitter_velocity = [
            (transform[12] - previous_transform[12]) * inv,
            (transform[13] - previous_transform[13]) * inv,
            (transform[14] - previous_transform[14]) * inv,
        ];
        let mut orientation = rotation_y(self.f("RotationY"));
        orientation[12] = self.f("PositionX");
        orientation[13] = self.f("PositionY");
        orientation[14] = self.f("PositionZ");
        let orientation = mul(&orientation, transform);
        let pccw = self.f("RotationPCCW") as i32;
        let f = self.flags;
        let mut p = Particle {
            hsv: f.hsv,
            sprite: f.particle,
            drag: self.f("Drag"),
            gravity: self.f("Gravity"),
            rotation_aligned: f.rotation_aligned,
            ..Particle::default()
        };
        if !f.hsv {
            p.color[0] = self.f("ColorR");
            p.color[1] = self.f("ColorG");
            p.color[2] = self.f("ColorB");
        }
        if !f.relative_velocity {
            emitter_velocity = [0.0; 3];
        }
        let n = (self.count as f32 * multiplier) as i32;
        let g = |s: &Self, name: &str| s.f(name);
        let light_radius = g(self, "LightRadius");
        let mut light = ParticleLight::default();
        if f.spawn_lights {
            light.diffuse = [
                g(self, "LightDiffuseR"),
                g(self, "LightDiffuseG"),
                g(self, "LightDiffuseB"),
            ];
            light.ambient = [
                g(self, "LightAmbientR"),
                g(self, "LightAmbientG"),
                g(self, "LightAmbientB"),
            ];
            light.specular = g(self, "LightSpecular");
            light.radius_start = light_radius;
            light.radius_end = light_radius;
        }
        let (ox, oy, oz) = (
            g(self, "PositionXOffset"),
            g(self, "PositionYOffset"),
            g(self, "PositionZOffset"),
        );
        let ss = [
            g(self, "SizeStartMin"),
            g(self, "SizeStartMax"),
            g(self, "SizeStartDistribution"),
        ];
        let se = [
            g(self, "SizeEndMin"),
            g(self, "SizeEndMax"),
            g(self, "SizeEndDistribution"),
        ];
        let spread = [
            g(self, "SpreadConeAngle"),
            g(self, "SpreadConeDistribution"),
            g(self, "SpreadArcHorizontalAngle"),
            g(self, "SpreadArcHorizontalDistribution"),
            g(self, "SpreadArcVerticalMin"),
            g(self, "SpreadArcVerticalMax"),
            g(self, "SpreadArcVerticalDistribution"),
        ];
        let vel = [
            g(self, "VelocityMin"),
            g(self, "VelocityMax"),
            g(self, "VelocityDistribution"),
        ];
        let life = [
            g(self, "LifeTimeMin"),
            g(self, "LifeTimeMax"),
            g(self, "LifeTimeDistribution"),
        ];
        let rot = [
            g(self, "RotationMin"),
            g(self, "RotationMax"),
            g(self, "RotationSpeedMin"),
            g(self, "RotationSpeedMax"),
        ];
        let hsv = [
            [
                g(self, "HueMin"),
                g(self, "HueMax"),
                g(self, "HueDistribution"),
            ],
            [
                g(self, "SatMin"),
                g(self, "SatMax"),
                g(self, "SatDistribution"),
            ],
            [
                g(self, "ValueMin"),
                g(self, "ValueMax"),
                g(self, "ValueDistribution"),
            ],
        ];
        // Non-HSV pulses draw alpha from AlphaMin/Max (reset to 1 by
        // ReadVersion1), not from the ColorA copies.
        let alpha = [
            g(self, "AlphaMin"),
            g(self, "AlphaMax"),
            g(self, "AlphaDistribution"),
        ];
        let c = &mut self.cursor;
        for _ in 0..n {
            let local = [
                (c.float() * 2.0 - 1.0) * ox,
                (c.float() * 2.0 - 1.0) * oy,
                (c.float() * 2.0 - 1.0) * oz,
            ];
            p.position = transform_point(local, &orientation);
            p.start_size = c.dist(ss[0], ss[1], ss[2]);
            p.end_size = c.dist(se[0], se[1], se[2]);
            let direction = if f.cone_spread {
                c.cone(&orientation, spread[0], spread[1])
            } else {
                c.arc(
                    &orientation,
                    spread[2],
                    spread[3],
                    spread[4],
                    spread[5],
                    spread[6],
                )
            };
            let speed = c.dist(vel[0], vel[1], vel[2]);
            p.velocity = std::array::from_fn(|i| direction[i] * speed + emitter_velocity[i]);
            p.ttl = c.dist(life[0], life[1], life[2]);
            p.alpha_blended = f.alpha_blended;
            if !f.rotation_aligned {
                p.rotation = c.range(rot[0], rot[1]);
                let s = c.range(rot[2], rot[3]);
                p.rotation_velocity = s * c.sign(pccw);
            }
            if f.hsv {
                p.colorize = f.colorize;
                p.color[0] = c.dist(hsv[0][0], hsv[0][1], hsv[0][2]);
                p.color[1] = c.dist(hsv[1][0], hsv[1][1], hsv[1][2]);
                p.color[2] = c.dist(hsv[2][0], hsv[2][1], hsv[2][2]);
                p.color[3] = c.dist(alpha[0], alpha[1], alpha[2]);
            } else {
                p.color[3] = c.dist(alpha[0], alpha[1], alpha[2]);
            }
            particles.spawn(&p);
            if f.spawn_lights {
                if f.light_size_relative {
                    light.radius_start = p.start_size * light_radius;
                    light.radius_end = p.end_size * light_radius;
                }
                particles.spawn_light(&p, &light);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// VisualEffect

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VisualEffectType {
    #[default]
    Single,
    Looping,
    Infinite,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Emitter {
    Continuous(ContinuousEmitter),
    Pulse(Box<PulseEmitter>),
}

/// The effect-level `<Light>`: a PointLightBatcher light that follows the
/// effect transform (VisualEffect.FromFile, MethodDef 0x0600018b). Missing
/// children keep the zeroed BatchedPointLight defaults.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectLight {
    /// PositionX/Y/Z, transformed by the effect's Transform every update.
    pub offset: Vec3,
    pub light: BatchedPointLight,
    pub spawn_transition: LightTransition,
    pub spawn_transition_time: f32,
    pub despawn_transition: LightTransition,
    pub despawn_transition_time: f32,
}

/// A source effect: its emitters live in the library and are shared by all
/// instances (ParticleEmitterCollection is a reference type).
#[derive(Clone, Debug, PartialEq)]
pub struct EffectDefinition {
    pub name: String,
    pub kind: VisualEffectType,
    pub duration: f32,
    pub emitters: Vec<usize>,
    pub light: Option<EffectLight>,
}

#[derive(Clone, Debug, Default)]
pub struct EffectLibrary {
    pub effects: HashMap<i32, EffectDefinition>,
    pub emitters: Vec<Emitter>,
}

impl EffectLibrary {
    /// VisualEffect.FromFile for one document; `cursor` seeds each emitter's
    /// random offset (the original uses `(ushort)DateTime.Now.Ticks`).
    pub fn add_xml(
        &mut self,
        name: &str,
        bytes: &[u8],
        cursor: &mut impl FnMut() -> RandomCursor,
    ) -> Result<i32, EffectError> {
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let root = xml(bytes).map_err(|e| EffectError::Xml(format!("{e:?}")))?;
        let mut definition = EffectDefinition {
            name: name.to_owned(),
            kind: VisualEffectType::Single,
            duration: 0.0,
            emitters: Vec::new(),
            light: None,
        };
        if root.name.eq_ignore_ascii_case("Effect") {
            let mut keyframes = 10;
            let mut version = 1;
            for (key, value) in &root.attributes {
                if key.eq_ignore_ascii_case("type") {
                    definition.kind = match value.trim().to_ascii_lowercase().as_str() {
                        "single" => VisualEffectType::Single,
                        "looping" => VisualEffectType::Looping,
                        "infinite" => VisualEffectType::Infinite,
                        _ => {
                            return Err(EffectError::Parse(
                                "VisualEffectType".into(),
                                value.clone(),
                            ))
                        }
                    };
                } else if key.eq_ignore_ascii_case("duration") {
                    definition.duration = parse_f32(value)?;
                } else if key.eq_ignore_ascii_case("keyFramesPerSecond") {
                    keyframes = parse_i32(value)?;
                } else if key.eq_ignore_ascii_case("version") {
                    version = parse_i32(value)?;
                }
            }
            for child in &root.children {
                // GetEmitterType compares the type name case-sensitively.
                let emitter = match child.name.as_str() {
                    "ContinuousEmitter" => Some(Emitter::Continuous(ContinuousEmitter::read(
                        child,
                        keyframes,
                        version,
                        cursor(),
                    )?)),
                    "PulseEmitter" => Some(Emitter::Pulse(Box::new(PulseEmitter::read(
                        child,
                        version,
                        cursor(),
                    )?))),
                    _ => None,
                };
                if let Some(e) = emitter {
                    definition.emitters.push(self.emitters.len());
                    self.emitters.push(e);
                } else if child.name.eq_ignore_ascii_case("Light") {
                    let mut light = EffectLight::default();
                    let transition = |v: &str| {
                        LightTransition::parse(v).ok_or_else(|| {
                            EffectError::Parse("LightTransitionType".into(), v.into())
                        })
                    };
                    for p in &child.children {
                        if let Some(v) = attr(p, "Value") {
                            let n = p.name.to_ascii_lowercase();
                            let l = &mut light.light;
                            match n.as_str() {
                                "positionx" => light.offset[0] = parse_f32(v)?,
                                "positiony" => light.offset[1] = parse_f32(v)?,
                                "positionz" => light.offset[2] = parse_f32(v)?,
                                "diffuser" => l.diffuse[0] = parse_f32(v)?,
                                "diffuseg" => l.diffuse[1] = parse_f32(v)?,
                                "diffuseb" => l.diffuse[2] = parse_f32(v)?,
                                "ambientr" => l.ambient[0] = parse_f32(v)?,
                                "ambientg" => l.ambient[1] = parse_f32(v)?,
                                "ambientb" => l.ambient[2] = parse_f32(v)?,
                                "specular" => l.specular = parse_f32(v)?,
                                "radius" => l.radius = parse_f32(v)?,
                                "variationtype" => {
                                    l.variation_type = parse_variation_type(v).ok_or_else(|| {
                                        EffectError::Parse("LightVariationType".into(), v.into())
                                    })?
                                }
                                "variationspeed" => l.variation_speed = parse_f32(v)?,
                                "variationamount" => l.variation_amount = parse_f32(v)?,
                                "spawntransitiontype" => light.spawn_transition = transition(v)?,
                                "spawntransitiontime" => {
                                    light.spawn_transition_time = parse_f32(v)?
                                }
                                "despawntransitiontype" => {
                                    light.despawn_transition = transition(v)?
                                }
                                "despawntransitiontime" => {
                                    light.despawn_transition_time = parse_f32(v)?
                                }
                                _ => {}
                            }
                        }
                    }
                    definition.light = Some(light);
                }
            }
        }
        let hash = crate::scene_loading::custom_hash(&name.to_lowercase());
        if self.effects.contains_key(&hash) {
            return Err(EffectError::Duplicate(name.to_owned()));
        }
        self.effects.insert(hash, definition);
        Ok(hash)
    }

    /// EffectManager.ReadDirectory over `Content/Effects`: `*.xml` files of
    /// each directory, then its subdirectories, keyed by the lowercase stem.
    pub fn load_directory(dir: &Path) -> Result<Self, EffectError> {
        let mut library = Self::default();
        let mut next = 0u16;
        let mut cursor = || {
            // Deterministic stand-in for the tick-count seed.
            let c = RandomCursor(next);
            next = next.wrapping_add(7919);
            c
        };
        library.read_directory(dir, &mut cursor)?;
        Ok(library)
    }

    fn read_directory(
        &mut self,
        dir: &Path,
        cursor: &mut impl FnMut() -> RandomCursor,
    ) -> Result<(), EffectError> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| EffectError::Io(format!("{}: {e}", dir.display())))?
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries.iter().filter(|p| p.is_file()) {
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
            {
                continue;
            }
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let bytes = std::fs::read(path).map_err(|e| EffectError::Io(e.to_string()))?;
            self.add_xml(&stem, &bytes, cursor)
                .map_err(|e| EffectError::Xml(format!("{}: {e:?}", path.display())))?;
        }
        for path in entries.iter().filter(|p| p.is_dir()) {
            self.read_directory(path, cursor)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ParticleSystem

pub const MAX_PARTICLES: usize = 32768;

/// The spawn-time Particle struct.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Particle {
    pub position: Vec3,
    pub velocity: Vec3,
    pub gravity: f32,
    pub rotation: f32,
    pub ttl: f32,
    pub start_size: f32,
    pub end_size: f32,
    pub rotation_velocity: f32,
    pub drag: f32,
    pub sprite: u8,
    pub alpha_blended: bool,
    pub hsv: bool,
    pub colorize: bool,
    pub rotation_aligned: bool,
    pub color: [f32; 4],
}

/// ParticleLogic.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ParticleLogic {
    rotation_speed: f32,
    gravity: f32,
    drag: f32,
    ttl: f32,
    inv_max_ttl: f32,
    start_size: f32,
    end_size: f32,
}

/// ParticleGraphics: the 56-byte per-instance vertex record the shader reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleGraphics {
    pub position: Vec3,
    pub rotation: f32,
    pub radius: f32,
    pub normalized_lifetime: f32,
    pub color: [f32; 4],
    pub sprite: u8,
    pub alpha_blended: u8,
    /// 50 for HSV plus 5 for Colorize.
    pub hsv_colorize: u8,
    pub rotation_aligned: u8,
    pub velocity: Vec3,
}

/// IntHeap as a min-heap of free indices (Pop returns the smallest).
#[derive(Clone, Debug, Default)]
struct FreeList(std::collections::BinaryHeap<std::cmp::Reverse<usize>>);
impl FreeList {
    fn reset(&mut self, n: usize) {
        self.0 = (0..n).map(std::cmp::Reverse).collect();
    }
    fn pop(&mut self) -> Option<usize> {
        self.0.pop().map(|r| r.0)
    }
    fn push(&mut self, i: usize) {
        self.0.push(std::cmp::Reverse(i));
    }
}

#[derive(Clone, Debug)]
pub struct ParticleSystem {
    logic: Vec<ParticleLogic>,
    pub graphics: Vec<ParticleGraphics>,
    /// ParticleLightBatcher: the lights SpawnLights emitters attach to
    /// their particles (updated right after the particles natively).
    pub lights: ParticleLightBatcher,
    /// Presentation provenance; does not alter particle integration or GPU data.
    sources: Vec<Option<EffectRef>>,
    emitting_effect: Option<EffectRef>,
    free: FreeList,
    last_active: isize,
}

impl Default for ParticleSystem {
    fn default() -> Self {
        let mut s = Self {
            logic: vec![ParticleLogic::default(); MAX_PARTICLES],
            graphics: vec![ParticleGraphics::default(); MAX_PARTICLES],
            lights: ParticleLightBatcher::default(),
            sources: vec![None; MAX_PARTICLES],
            emitting_effect: None,
            free: FreeList::default(),
            last_active: -1,
        };
        s.clear();
        s
    }
}

impl ParticleSystem {
    pub fn clear(&mut self) {
        self.last_active = -1;
        self.free.reset(MAX_PARTICLES);
        self.lights.clear();
    }

    /// ParticleHelper.SpawnLight: the light copies the particle's position,
    /// velocity, gravity, drag and TTL (its colour comes from the emitter's
    /// Light keys, never from the particle colour). It is spawned even when
    /// the particle pool itself was full.
    pub fn spawn_light(&mut self, p: &Particle, light: &ParticleLight) {
        self.lights.spawn(&ParticleLight {
            position: p.position,
            velocity: p.velocity,
            gravity: p.gravity,
            drag: p.drag,
            ttl: p.ttl,
            ..*light
        });
    }

    pub fn spawn(&mut self, p: &Particle) {
        let Some(i) = self.free.pop() else {
            return;
        };
        self.graphics[i] = ParticleGraphics {
            position: p.position,
            rotation: p.rotation,
            radius: p.start_size,
            normalized_lifetime: 0.0,
            color: p.color,
            sprite: p.sprite,
            alpha_blended: u8::from(p.alpha_blended),
            hsv_colorize: if p.hsv { 50 } else { 0 } + if p.colorize { 5 } else { 0 },
            rotation_aligned: u8::from(p.rotation_aligned),
            velocity: p.velocity,
        };
        self.sources[i] = self.emitting_effect;
        self.logic[i] = ParticleLogic {
            rotation_speed: p.rotation_velocity,
            gravity: p.gravity,
            drag: p.drag,
            ttl: p.ttl,
            inv_max_ttl: 1.0 / p.ttl,
            start_size: p.start_size,
            end_size: p.end_size,
        };
        self.last_active = self.last_active.max(i as isize);
    }

    /// UpdateParticles: integrate every slot up to the last active one.
    pub fn update(&mut self, dt: f32) {
        for i in 0..=self.last_active.max(-1) {
            let i = i as usize;
            let logic = &mut self.logic[i];
            let g = &mut self.graphics[i];
            if logic.ttl >= 0.0 {
                logic.ttl -= dt;
                let remaining = logic.ttl * logic.inv_max_ttl;
                g.normalized_lifetime = 1.0 - remaining;
                g.velocity[1] += logic.gravity * dt;
                let drag = pow(logic.drag, dt);
                for v in &mut g.velocity {
                    *v *= drag;
                }
                for a in 0..3 {
                    g.position[a] += g.velocity[a] * dt;
                }
                g.rotation += logic.rotation_speed * dt;
                g.radius = logic.start_size * remaining + logic.end_size * (1.0 - remaining);
                if logic.ttl < 0.0 {
                    self.free.push(i);
                }
            } else {
                g.radius = 0.0;
            }
        }
        while self.last_active >= 0 && self.logic[self.last_active as usize].ttl < 0.0 {
            self.last_active -= 1;
        }
    }

    /// The instance range the renderer draws (`0..=last_active`).
    pub fn active(&self) -> &[ParticleGraphics] {
        &self.graphics[..(self.last_active + 1) as usize]
    }

    /// Source of each graphics slot, including particles whose emitter stopped.
    pub fn active_sources(&self) -> &[Option<EffectRef>] {
        &self.sources[..(self.last_active + 1) as usize]
    }
}

// ---------------------------------------------------------------------------
// EffectManager

pub const MAX_EFFECTS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectRef {
    pub id: i32,
    pub hash: i32,
}
impl Default for EffectRef {
    fn default() -> Self {
        Self { id: -1, hash: 0 }
    }
}

#[derive(Clone, Debug)]
struct EffectInstance {
    definition: Option<i32>,
    active: bool,
    total_time: f32,
    timeline: f32,
    duration: f32,
    kind: VisualEffectType,
    transform: Mat,
    last_transform: Mat,
    /// VisualEffect.mLightReference (per instance; the source struct's
    /// null reference is copied on start).
    light: PointLightRef,
}

/// Magicka's EffectManager plus the particle system it feeds.
pub struct EffectSystem {
    pub library: EffectLibrary,
    pub particles: ParticleSystem,
    /// PointLightBatcher: one light per instance of an effect with `<Light>`.
    pub effect_lights: PointLightBatcher,
    /// ParticleSystem.SetSpawnModifier (Defines.ParticleMultiplyer, High = 1).
    pub spawn_multiplier: f32,
    effects: Vec<EffectInstance>,
    unique: [i32; MAX_EFFECTS],
    free: FreeList,
    last_active: i32,
    randomizer: DotNetRandom,
}

/// Presentation-only snapshot of an active effect. Gameplay stays on the host.
pub struct EffectPresentation {
    pub reference: EffectRef,
    pub definition: i32,
    pub transform: Mat,
    pub elapsed: f32,
}

impl EffectSystem {
    pub fn presentation(&self) -> Vec<EffectPresentation> {
        self.effects
            .iter()
            .enumerate()
            .filter(|(_, e)| e.active)
            .map(|(slot, e)| EffectPresentation {
                reference: EffectRef {
                    id: slot as i32,
                    hash: self.unique[slot],
                },
                definition: e.definition.expect("active effect definition"),
                transform: e.transform,
                elapsed: e.total_time,
            })
            .collect()
    }

    /// Reconcile an emitter clock without replaying historical births on join.
    /// How long this effect has been running, while it is.
    pub fn elapsed(&self, r: &EffectRef) -> Option<f32> {
        self.valid(r)
            .then(|| self.effects[r.id as usize].total_time)
    }
    /// Whether this effect ends by itself (`VisualEffectType::Single`).
    pub fn ends_by_itself(&self, r: &EffectRef) -> bool {
        self.valid(r) && matches!(self.effects[r.id as usize].kind, VisualEffectType::Single)
    }
    pub fn set_elapsed(&mut self, r: &EffectRef, elapsed: f32) -> bool {
        if !self.valid(r) || !elapsed.is_finite() || elapsed < 0. {
            return false;
        }
        let e = &mut self.effects[r.id as usize];
        e.total_time = elapsed;
        e.timeline = match e.kind {
            VisualEffectType::Looping => elapsed % e.duration,
            _ => elapsed.min(e.duration),
        };
        true
    }

    pub fn new(library: EffectLibrary) -> Self {
        let mut free = FreeList::default();
        free.reset(MAX_EFFECTS);
        Self {
            library,
            particles: ParticleSystem::default(),
            effect_lights: PointLightBatcher::default(),
            spawn_multiplier: crate::global_settings::particle_quality().particle_multiplier(),
            effects: vec![EffectInstance::default(); MAX_EFFECTS],
            unique: [i32::MIN; MAX_EFFECTS],
            free,
            last_active: 0,
            // The original's EffectManager Random is time-seeded.
            randomizer: DotNetRandom::new(0x4d61_6769),
        }
    }

    fn valid(&self, r: &EffectRef) -> bool {
        r.id >= 0 && self.unique[r.id as usize] == r.hash
    }

    /// StartEffect(hash, transform).
    pub fn start(&mut self, hash: i32, transform: Mat) -> Option<EffectRef> {
        if self.free.0.is_empty() {
            return None;
        }
        let definition = self.library.effects.get(&hash)?;
        let (kind, duration) = (definition.kind, definition.duration);
        let i = self.free.pop()?;
        self.effects[i] = EffectInstance {
            definition: Some(hash),
            active: true,
            total_time: 0.0,
            timeline: 0.0,
            duration,
            kind,
            transform,
            last_transform: transform,
            light: PointLightRef::default(),
        };
        self.last_active = self.last_active.max(i as i32);
        let hash = self.randomizer.next_below(2_147_483_646) + 1;
        self.unique[i] = hash;
        Some(EffectRef { id: i as i32, hash })
    }

    /// StartEffect(hash, position, direction): CreateWorld with Up, or
    /// Forward when the direction is vertical.
    pub fn start_at(&mut self, hash: i32, position: Vec3, direction: Vec3) -> Option<EffectRef> {
        let up = if direction[0].abs() < f32::from_bits(1) && direction[2].abs() < f32::from_bits(1)
        {
            [0.0, 0.0, -1.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        self.start(hash, create_world(position, direction, up))
    }

    pub fn update_orientation(&mut self, r: &mut EffectRef, transform: Mat) -> bool {
        if !self.valid(r) {
            *r = EffectRef { id: -1, hash: 0 };
            return false;
        }
        self.effects[r.id as usize].transform = transform;
        true
    }

    /// A live effect's position.
    pub fn effect_position(&self, r: &EffectRef) -> Option<Vec3> {
        if !self.valid(r) {
            return None;
        }
        let m = &self.effects[r.id as usize].transform;
        Some([m[12], m[13], m[14]])
    }

    pub fn update_position_direction(
        &mut self,
        r: &mut EffectRef,
        position: Vec3,
        direction: Vec3,
    ) -> bool {
        if !self.valid(r) {
            *r = EffectRef { id: -1, hash: -1 };
            return false;
        }
        let mut m = orientation_matrix(direction);
        m[12..15].copy_from_slice(&position);
        self.effects[r.id as usize].transform = m;
        true
    }

    pub fn restart(&mut self, r: &mut EffectRef) -> bool {
        if !self.valid(r) {
            *r = EffectRef::default();
            return false;
        }
        let e = &mut self.effects[r.id as usize];
        e.total_time = 0.0;
        e.timeline = 0.0;
        true
    }

    pub fn is_active(&self, r: &EffectRef) -> bool {
        r.id >= 0 && self.unique[r.id as usize] == r.hash && self.effects[r.id as usize].active
    }

    /// EffectManager.Stop: VisualEffect.Stop disables the light with the
    /// despawn transition, then the slot is freed.
    pub fn stop(&mut self, r: &mut EffectRef) {
        if self.valid(r) {
            let i = r.id as usize;
            self.unique[i] = i32::MIN;
            self.stop_instance(i);
            self.free.push(i);
            *r = EffectRef::default();
        }
    }

    /// VisualEffect.Stop (MethodDef 0x06000180): mActive = false and
    /// PointLightBatcher.DisableLight(ref, despawn transition, time).
    fn stop_instance(&mut self, i: usize) {
        let e = &mut self.effects[i];
        e.active = false;
        let (transition, time) = e
            .definition
            .and_then(|h| self.library.effects.get(&h))
            .and_then(|d| d.light.as_ref())
            .map_or((LightTransition::None, 0.0), |l| {
                (l.despawn_transition, l.despawn_transition_time)
            });
        self.effect_lights.disable(&mut e.light, transition, time);
    }

    /// Every batched light the native light pass would draw this frame:
    /// particle lights, then effect lights (ParticleLightBatcher and
    /// PointLightBatcher are separate Scene lights; each draws its whole
    /// instance range). Zero-radius and zero-intensity slots are skipped as
    /// they shade nothing.
    pub fn draw_lights(&self) -> impl Iterator<Item = &LightRecord> + '_ {
        // PlayState enables ParticleLightBatcher only with ParticleLights on;
        // PointLightBatcher is always enabled.
        let particle_lights: &[LightRecord] = if crate::global_settings::particle_lights() {
            self.particles.lights.active()
        } else {
            &[]
        };
        particle_lights
            .iter()
            .chain(self.effect_lights.active())
            .filter(|l| l.contributes())
    }

    /// EffectManager.Update (descending slots), then the particle step
    /// (PlayState UpdateMiscB runs UpdateParticles later in the frame).
    pub fn update(&mut self, dt: f32) {
        for i in (0..=self.last_active.max(-1)).rev() {
            let i = i as usize;
            if self.effects[i].active {
                self.update_effect(i, dt);
                if !self.effects[i].active {
                    self.unique[i] = i32::MIN;
                    self.free.push(i);
                }
            }
        }
        while self.last_active >= 0 && !self.effects[self.last_active as usize].active {
            self.last_active -= 1;
        }
    }

    /// PlayState.UpdateMiscB order: ParticleSystem.UpdateParticles, then
    /// ParticleLightBatcher.Update, then PointLightBatcher.Update.
    pub fn update_particles(&mut self, dt: f32) {
        self.particles.update(dt);
        self.particles.lights.update(dt);
        self.effect_lights.update(dt);
    }

    /// VisualEffect.Update (MethodDef 0x06000182).
    fn update_effect(&mut self, i: usize, dt: f32) {
        let Some(hash) = self.effects[i].definition else {
            return;
        };
        // The light block runs before the dt guard: the offset is
        // transformed by the current Transform; a light that is not active
        // (never spawned, or its fade-out finished) is spawned with the
        // spawn transition, otherwise only its position is refreshed.
        if let Some(light) = self
            .library
            .effects
            .get(&hash)
            .and_then(|d| d.light.as_ref())
        {
            let e = &mut self.effects[i];
            let position = transform_point(light.offset, &e.transform);
            if !self.effect_lights.is_active(&mut e.light) {
                e.light = self.effect_lights.spawn(
                    position,
                    &light.light,
                    light.spawn_transition,
                    light.spawn_transition_time,
                );
            } else {
                self.effect_lights.set_position(&mut e.light, position);
            }
        }
        if dt < f32::from_bits(1) {
            return;
        }
        let e = &mut self.effects[i];
        if e.active {
            let previous_timeline = e.timeline;
            let previous_total = e.total_time;
            e.total_time += dt;
            e.timeline += dt;
            let mut stop = false;
            match e.kind {
                VisualEffectType::Single => {
                    if e.timeline > e.duration {
                        stop = true;
                    }
                }
                VisualEffectType::Looping => e.timeline %= e.duration,
                VisualEffectType::Infinite => {
                    if e.timeline > e.duration {
                        e.timeline = e.duration;
                    }
                }
            }
            let (total, timeline, transform, last) =
                (e.total_time, e.timeline, e.transform, e.last_transform);
            if stop {
                // Single past its duration: Stop() (the emitters still run
                // this frame; the light batcher updates later in the frame).
                self.stop_instance(i);
            }
            let emitters = self.library.effects[&hash].emitters.clone();
            self.particles.emitting_effect = Some(EffectRef {
                id: i as i32,
                hash: self.unique[i],
            });
            for index in emitters {
                match &mut self.library.emitters[index] {
                    Emitter::Continuous(c) => c.update(
                        dt,
                        previous_total,
                        total,
                        timeline,
                        previous_timeline,
                        &transform,
                        &last,
                        self.spawn_multiplier,
                        &mut self.particles,
                    ),
                    Emitter::Pulse(p) => p.update(
                        dt,
                        timeline,
                        previous_timeline,
                        &transform,
                        &last,
                        self.spawn_multiplier,
                        &mut self.particles,
                    ),
                }
            }
            self.particles.emitting_effect = None;
        }
        let e = &mut self.effects[i];
        e.last_transform = e.transform;
    }

    pub fn active_effects(&self) -> usize {
        self.effects.iter().filter(|e| e.active).count()
    }
    /// Active instances of a decoded effect definition (presentation audit).
    pub fn active_effects_by_hash(&self, hash: i32) -> usize {
        self.effects
            .iter()
            .filter(|e| e.active && e.definition == Some(hash))
            .count()
    }
}

impl Default for EffectInstance {
    fn default() -> Self {
        Self {
            definition: None,
            active: false,
            total_time: 0.0,
            timeline: 0.0,
            duration: 0.0,
            kind: VisualEffectType::Single,
            transform: IDENTITY,
            last_transform: IDENTITY,
            light: PointLightRef::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_field_indices_match_names() {
        assert_eq!(KEYED[k::PPS], "ParticlesPerSecond");
        assert_eq!(KEYED[k::ROT_Y], "RotationY");
        assert_eq!(KEYED[k::COLOR_A_DIST], "ColorADistribution");
        assert_eq!(KEYED[k::ALPHA_DIST], "AlphaDistribution");
        assert_eq!(KEYED[k::VEL_DIST], "VelocityDistribution");
        assert_eq!(KEYED[k::ROT_PCCW], "RotationPCCW");
        assert_eq!(KEYED[k::ARC_V_DIST], "SpreadArcVerticalDistribution");
        assert_eq!(KEYED[k::LIGHT_RADIUS], "LightRadius");
        assert_eq!(KEYED[k::COUNT - 1], "LightSpecular");
        assert_eq!(PULSE_FIELDS[PULSE_COUNT - 1], "LightSpecular");
    }
}
