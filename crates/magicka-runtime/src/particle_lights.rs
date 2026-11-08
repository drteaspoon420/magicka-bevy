//! PolygonHead's batched effect lights (research/particles.md, 2026-10-30):
//! `PointLightBatcher` (128 slots; one per effect `<Light>`) and
//! `ParticleLightBatcher` (256 slots; one per particle of an emitter with
//! `SpawnLights`). Both fill `BatchedPointLightEffect.LightProperties`
//! records whose vertex shader scales diffuse/specular/ambient by Intensity
//! and whose pixel shader attenuates with `sat(1 − d/Radius)²` exactly like
//! PointLightEffect (`output/evidence/shaders/BatchedPointLightEffect.asm.txt`).
//!
//! Update order (PlayState.UpdateMiscB, MethodDef 0x06000c37):
//! ParticleSystem.UpdateParticles, ParticleLightBatcher.Update,
//! PointLightBatcher.Update.

// Negated comparisons keep the original's NaN behaviour.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::light_variation::LightVariation;
use crate::particles::{DotNetRandom, Vec3};

/// One `BatchedPointLightEffect.LightProperties` record (48 bytes natively).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightRecord {
    pub position: Vec3,
    pub radius: f32,
    pub diffuse: Vec3,
    pub specular: f32,
    pub ambient: Vec3,
    /// Multiplies diffuse, specular and ambient in the vertex shader.
    pub intensity: f32,
}

impl LightRecord {
    /// A record the batched light shader would shade with: freed slots keep
    /// radius 0 (a degenerate sphere) and intensity 0 scales every colour
    /// to zero, so dropping them changes nothing visible.
    pub fn contributes(&self) -> bool {
        self.radius > 0.0 && self.intensity > 0.0
    }
}

/// `PolygonHead.Lights.LightTransitionType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LightTransition {
    #[default]
    None,
    Linear,
    Flicker,
}

impl LightTransition {
    /// `Enum.Parse(typeof(LightTransitionType), value, ignoreCase: true)`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "0" => Some(Self::None),
            "linear" | "1" => Some(Self::Linear),
            "flicker" | "2" => Some(Self::Flicker),
            _ => None,
        }
    }
}

/// `PolygonHead.Lights.LightVariationType` as the integer
/// `light_variation::LightVariation::intensity` takes.
pub fn parse_variation_type(s: &str) -> Option<i32> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" | "0" => Some(0),
        "sine" | "1" => Some(1),
        "flicker" | "2" => Some(2),
        "candle" | "3" => Some(3),
        "strobe" | "4" => Some(4),
        _ => None,
    }
}

/// `Light.GetIntensity(LightTransitionType, normalizedTime)` (MethodDef
/// 0x060000c6): Linear returns the normalised time, Flicker is on when a
/// shared `Random.NextDouble()` falls below it, None is 1.
pub fn transition_intensity(
    kind: LightTransition,
    normalized: f32,
    random: &mut DotNetRandom,
) -> f32 {
    match kind {
        LightTransition::Linear => normalized,
        LightTransition::Flicker => {
            if !((random.next_double() as f32) < normalized) {
                0.0
            } else {
                1.0
            }
        }
        LightTransition::None => 1.0,
    }
}

/// `PointLightBatcher.BatchedPointLight`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BatchedPointLight {
    pub radius: f32,
    pub diffuse: Vec3,
    pub specular: f32,
    pub ambient: Vec3,
    pub variation_type: i32,
    pub variation_speed: f32,
    pub variation_amount: f32,
}

/// `PointLightBatcher.PointLightReference`; `{-1, 0}` is the null reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointLightRef {
    pub id: i32,
    pub sig: i32,
}

impl Default for PointLightRef {
    fn default() -> Self {
        Self { id: -1, sig: 0 }
    }
}

/// `PointLightBatcher.LightLogic`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct EffectLightLogic {
    active: bool,
    variation_type: i32,
    variation_speed: f32,
    variation_amount: f32,
    variation: LightVariation,
    transition: LightTransition,
    transition_time: f32,
    transition_total: f32,
}

impl EffectLightLogic {
    /// `LightLogic.IsActive`: active, or still fading out (TransitionTime ≥ 0).
    fn is_active(&self) -> bool {
        self.active || !(self.transition_time < 0.0)
    }
}

pub const MAX_EFFECT_LIGHTS: usize = 128;

/// Min-heap free list (PolygonHead.IntHeap: Pop returns the smallest).
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
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `PolygonHead.Lights.PointLightBatcher` (singleton natively; one per
/// EffectSystem here). Always enabled: PlayState enables it on the scene.
#[derive(Clone, Debug)]
pub struct PointLightBatcher {
    unique: [i32; MAX_EFFECT_LIGHTS],
    lights: Vec<LightRecord>,
    logic: Vec<EffectLightLogic>,
    last_active: isize,
    free: FreeList,
    /// `mRandomizer` (signatures; time-seeded natively).
    randomizer: DotNetRandom,
    /// `Light.sRandom` (flicker transitions and variations; natively one
    /// time-seeded Random shared by every light).
    random: DotNetRandom,
}

impl Default for PointLightBatcher {
    fn default() -> Self {
        let mut b = Self {
            unique: [i32::MIN; MAX_EFFECT_LIGHTS],
            lights: vec![LightRecord::default(); MAX_EFFECT_LIGHTS],
            logic: vec![EffectLightLogic::default(); MAX_EFFECT_LIGHTS],
            last_active: -1,
            free: FreeList::default(),
            randomizer: DotNetRandom::new(0x4c69_6768),
            random: DotNetRandom::new(0x5061_7274),
        };
        b.clear();
        b
    }
}

impl PointLightBatcher {
    /// `Clear` (Level load / GameScene reset).
    pub fn clear(&mut self) {
        self.last_active = -1;
        self.free.reset(MAX_EFFECT_LIGHTS);
        self.unique = [i32::MIN; MAX_EFFECT_LIGHTS];
        for l in &mut self.lights {
            *l = LightRecord::default();
        }
    }

    /// `SpawnLight(position, light, out ref, transition, time)` (MethodDef
    /// 0x06000431): the null reference when the pool is exhausted; otherwise
    /// the lowest free slot with Intensity 0 until the next Update.
    pub fn spawn(
        &mut self,
        position: Vec3,
        light: &BatchedPointLight,
        transition: LightTransition,
        transition_time: f32,
    ) -> PointLightRef {
        if self.free.is_empty() {
            return PointLightRef::default();
        }
        let Some(i) = self.free.pop() else {
            return PointLightRef::default();
        };
        self.lights[i] = LightRecord {
            position,
            radius: light.radius,
            diffuse: light.diffuse,
            specular: light.specular,
            ambient: light.ambient,
            intensity: 0.0,
        };
        self.logic[i] = EffectLightLogic {
            active: true,
            variation_type: light.variation_type,
            variation_speed: light.variation_speed,
            variation_amount: light.variation_amount,
            variation: LightVariation::default(),
            transition,
            transition_time: 0.0,
            transition_total: transition_time,
        };
        self.last_active = self.last_active.max(i as isize);
        let sig = self.randomizer.next_below(2_147_483_646) + 1;
        self.unique[i] = sig;
        PointLightRef { id: i as i32, sig }
    }

    /// `DisableLight(ref, transition, time)` (MethodDef 0x06000433): starts
    /// the fade-out; the slot is freed by Update once TransitionTime < 0.
    pub fn disable(&mut self, r: &mut PointLightRef, transition: LightTransition, time: f32) {
        if self.valid(r) && self.logic[r.id as usize].active {
            let l = &mut self.logic[r.id as usize];
            l.transition = transition;
            l.transition_time = time;
            l.transition_total = time;
            l.active = false;
        }
    }

    /// `SetLightPosition` (MethodDef 0x06000434).
    pub fn set_position(&mut self, r: &mut PointLightRef, position: Vec3) {
        if self.valid(r) {
            self.lights[r.id as usize].position = position;
        }
    }

    /// `IsLightActive` (MethodDef 0x06000436): true while lit or fading out.
    pub fn is_active(&mut self, r: &mut PointLightRef) -> bool {
        if self.valid(r) {
            self.logic[r.id as usize].is_active()
        } else {
            false
        }
    }

    /// `ValidRef`: a stale reference is reset to the null reference.
    fn valid(&self, r: &mut PointLightRef) -> bool {
        if r.id >= 0 && self.unique[r.id as usize] == r.sig {
            return true;
        }
        *r = PointLightRef::default();
        false
    }

    /// `Update(channel, dt)` (MethodDef 0x06000438): transition timers,
    /// `Intensity = transition × variation`, and freeing finished fades.
    pub fn update(&mut self, dt: f32) {
        for i in 0..=self.last_active.max(-1) {
            let i = i as usize;
            if !self.logic[i].is_active() {
                continue;
            }
            let l = &mut self.logic[i];
            if l.active {
                l.transition_time += dt;
                if !(l.transition_time < l.transition_total) {
                    l.transition = LightTransition::None;
                }
            } else {
                l.transition_time -= dt;
            }
            if l.is_active() {
                let normalized = l.transition_time / l.transition_total;
                let mut intensity =
                    transition_intensity(l.transition, normalized, &mut self.random);
                intensity *= l.variation.intensity(
                    l.variation_type,
                    dt,
                    l.variation_speed,
                    l.variation_amount,
                    &mut self.random,
                );
                self.lights[i].intensity = intensity;
                continue;
            }
            self.lights[i].intensity = 0.0;
            self.lights[i].radius = 0.0;
            self.unique[i] = i32::MIN;
            self.free.push(i);
        }
        while self.last_active >= 0 && !self.logic[self.last_active as usize].is_active() {
            self.last_active -= 1;
        }
    }

    /// The instance range the batched draw covers (`0..=last_active`).
    pub fn active(&self) -> &[LightRecord] {
        &self.lights[..(self.last_active + 1) as usize]
    }

    /// The record behind a reference, if it is still valid.
    pub fn get(&self, r: &PointLightRef) -> Option<&LightRecord> {
        (r.id >= 0 && self.unique[r.id as usize] == r.sig).then(|| &self.lights[r.id as usize])
    }

    pub fn free_slots(&self) -> usize {
        self.free.0.len()
    }
}

/// `ParticleLightBatcher.ParticleLight` (filled by ParticleHelper.SpawnLight,
/// MethodDef 0x0600050f, from the particle's position, velocity, gravity,
/// drag and TTL plus the emitter's light keys).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleLight {
    pub position: Vec3,
    pub radius_start: f32,
    pub radius_end: f32,
    pub diffuse: Vec3,
    pub specular: f32,
    pub ambient: Vec3,
    pub velocity: Vec3,
    pub gravity: f32,
    pub drag: f32,
    pub ttl: f32,
}

/// `ParticleLightBatcher.LightLogic`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ParticleLightLogic {
    radius_start: f32,
    radius_end: f32,
    velocity: Vec3,
    gravity: f32,
    drag: f32,
    ttl: f32,
    inv_max_ttl: f32,
}

pub const MAX_PARTICLE_LIGHTS: usize = 256;

/// `PolygonHead.Lights.ParticleLightBatcher`: every slot integrates like a
/// particle and fades with `sat(8·n·(1−n))` of its remaining life.
#[derive(Clone, Debug)]
pub struct ParticleLightBatcher {
    lights: Vec<LightRecord>,
    logic: Vec<ParticleLightLogic>,
    last_active: isize,
    free: FreeList,
}

impl Default for ParticleLightBatcher {
    fn default() -> Self {
        let mut b = Self {
            lights: vec![LightRecord::default(); MAX_PARTICLE_LIGHTS],
            logic: vec![
                ParticleLightLogic {
                    ttl: -1.0,
                    ..Default::default()
                };
                MAX_PARTICLE_LIGHTS
            ],
            last_active: -1,
            free: FreeList::default(),
        };
        b.clear();
        b
    }
}

impl ParticleLightBatcher {
    pub fn clear(&mut self) {
        self.last_active = -1;
        self.free.reset(MAX_PARTICLE_LIGHTS);
        for l in &mut self.logic {
            l.ttl = -1.0;
        }
        for l in &mut self.lights {
            *l = LightRecord::default();
        }
    }

    /// `SpawnLight` (MethodDef 0x0600048b): silently dropped when the 256
    /// slots are taken. Intensity stays 0 until the next Update.
    pub fn spawn(&mut self, light: &ParticleLight) {
        let Some(i) = self.free.pop() else {
            return;
        };
        self.lights[i] = LightRecord {
            position: light.position,
            radius: light.radius_start,
            diffuse: light.diffuse,
            specular: light.specular,
            ambient: light.ambient,
            intensity: 0.0,
        };
        self.logic[i] = ParticleLightLogic {
            radius_start: light.radius_start,
            radius_end: light.radius_end,
            velocity: light.velocity,
            gravity: light.gravity,
            drag: light.drag,
            ttl: light.ttl,
            inv_max_ttl: 1.0 / light.ttl,
        };
        self.last_active = self.last_active.max(i as isize);
    }

    /// `Update` (MethodDef 0x0600048c): the particle integration
    /// (gravity, `drag^dt`, position), `radius = lerp(end, start, n)` and
    /// `intensity = clamp(8·n·(1−n), 0, 1)` with `n = ttl / maxTTL`.
    pub fn update(&mut self, dt: f32) {
        for i in 0..=self.last_active.max(-1) {
            let i = i as usize;
            let l = &mut self.logic[i];
            let g = &mut self.lights[i];
            if l.ttl >= 0.0 {
                l.ttl -= dt;
                let n = l.ttl * l.inv_max_ttl;
                l.velocity[1] += l.gravity * dt;
                let drag = (f64::from(l.drag)).powf(f64::from(dt)) as f32;
                for v in &mut l.velocity {
                    *v *= drag;
                }
                for a in 0..3 {
                    g.position[a] += l.velocity[a] * dt;
                }
                g.radius = l.radius_start * n + l.radius_end * (1.0 - n);
                g.intensity = (n * (1.0 - n) * 8.0).clamp(0.0, 1.0);
                if l.ttl < 0.0 {
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

    /// The instance range the batched draw covers (`0..=last_active`).
    pub fn active(&self) -> &[LightRecord] {
        &self.lights[..(self.last_active + 1) as usize]
    }

    pub fn free_slots(&self) -> usize {
        self.free.0.len()
    }
}
