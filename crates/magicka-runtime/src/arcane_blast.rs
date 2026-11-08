//! `Magicka.GameLogic.Spells.ArcaneBlast` presentation and the
//! `Magicka.Graphics.Lights.DynamicLight` it enables (research/arcane-blast.md).
//!
//! FullBlast with any beam element (Arcane/Life) starts one ArcaneBlast: two
//! counter-rotating additive discs per beam element (EffectTextures/ArcaneDisc,
//! one 1/8 atlas row per element) that grow from the blast position, and a
//! fading point light in the spell colour. The `beam_<element>_explosion`
//! particle effects it also starts are handled by the session.

// Negated comparisons keep the original's NaN behaviour.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use magicka_sim::spells::{Elements, Spell};

type Vector = [f32; 3];

/// `Railgun.ELEMENT_OFFSET_LOOKUP` (shared with the beam atlas).
pub use crate::beam_mesh::ELEMENT_OFFSET_V;

/// ArcaneBlast state after `Initialize(owner, position, color, radius, element)`.
#[derive(Clone, Debug, PartialEq)]
pub struct ArcaneBlast {
    pub position: Vector,
    pub max_radius: f32,
    pub elements: u32,
    pub color: Vector,
    pub ttl: f32,
    pub rotation: f32,
}

/// One AdditiveEffect draw: the unit XZ quad (−1..1, UV 0..1) under `world`,
/// sampled at `uv · (1, 0.125) + (0, v_offset)` with ColorTint (1, 1, 1, alpha).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiscDraw {
    /// World-space corners (TriangleFan order).
    pub positions: [Vector; 4],
    pub uvs: [[f32; 2]; 4],
    pub alpha: f32,
}

/// sVertices: TriangleFan of VertexPositionTexture.
const QUAD: [([f32; 2], [f32; 2]); 4] = [
    ([-1.0, -1.0], [0.0, 0.0]),
    ([1.0, -1.0], [1.0, 0.0]),
    ([1.0, 1.0], [1.0, 1.0]),
    ([-1.0, 1.0], [0.0, 1.0]),
];

impl ArcaneBlast {
    /// `Blast.FullBlast(…, DamageCollection5)` IL_0021–IL_004b: colour of
    /// `DefaultSpell(all elements)`, mTTL 1. mRotation is not reset by
    /// Initialize (pooled instances keep it); a fresh instance starts at 0.
    pub fn new(position: Vector, max_radius: f32, elements: u32) -> Self {
        let spell = Spell::default_for(Elements(elements));
        Self {
            position,
            max_radius,
            elements,
            color: magicka_sim::lightning::spell_color(&spell),
            ttl: 1.0,
            rotation: 0.0,
        }
    }

    /// `get_IsDead`: !(mTTL > −1).
    pub fn is_dead(&self) -> bool {
        !(self.ttl > -1.0)
    }

    /// `Update`: mTTL −= 5·dt, mRotation = WrapAngle(mRotation + 4·dt).
    pub fn update(&mut self, dt: f32) {
        self.ttl -= dt * 5.0;
        self.rotation = wrap_angle(self.rotation + dt * 4.0);
    }

    /// Bounding radius `(1 − mTTL) · mMaxRadius`.
    pub fn radius(&self) -> f32 {
        (1.0 - self.ttl) * self.max_radius
    }

    /// RenderData.Alpha = min(1, 4·mTTL + 1); Draw tints by 0.5·Alpha.
    pub fn tint_alpha(&self) -> f32 {
        (self.ttl * 4.0 + 1.0).min(1.0) * 0.5
    }

    /// RenderData.Draw: for each set element index (0..11), Transform0
    /// (RotY(rot)·radius) and Transform1 (RotY(−rot)·0.9·radius), both
    /// translated to the blast position.
    pub fn draws(&self) -> Vec<DiscDraw> {
        let radius = self.radius();
        let alpha = self.tint_alpha();
        let mut out = Vec::new();
        for (index, &v_offset) in ELEMENT_OFFSET_V.iter().enumerate() {
            if self.elements & (1 << index) == 0 {
                continue;
            }
            for (angle, scale) in [(self.rotation, radius), (-self.rotation, radius * 0.9)] {
                let (s, c) = angle.sin_cos();
                let positions = QUAD.map(|([x, z], _)| {
                    // XNA CreateRotationY (M13 = −sin, M31 = sin) with
                    // row vectors: x' = x·cos + z·sin, z' = −x·sin + z·cos.
                    let rx = x * c + z * s;
                    let rz = -x * s + z * c;
                    [
                        self.position[0] + rx * scale,
                        self.position[1],
                        self.position[2] + rz * scale,
                    ]
                });
                let uvs = QUAD.map(|(_, [u, v])| [u, v * 0.125 + v_offset]);
                out.push(DiscDraw {
                    positions,
                    uvs,
                    alpha,
                });
            }
        }
        out
    }

    /// The DynamicLight enabled by Initialize: Initialize(position, colour,
    /// intensity 2, radius 2·maxRadius, speed 2, specular 1, TTL 0.25·mTTL).
    pub fn light(&self) -> DynamicLight {
        DynamicLight::new(
            self.position,
            self.color,
            2.0,
            self.max_radius * 2.0,
            2.0,
            1.0,
            self.ttl * 0.25,
        )
    }
}

/// MathHelper.WrapAngle.
fn wrap_angle(a: f32) -> f32 {
    let two_pi = std::f32::consts::TAU;
    let mut a = (a + std::f32::consts::PI).rem_euclid(two_pi) - std::f32::consts::PI;
    if a <= -std::f32::consts::PI {
        a += two_pi;
    }
    a
}

/// A pooled DynamicLight (a PointLight) with PolygonHead's linear enable /
/// disable transition (LightTransitionType 1, time 1/speed).
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicLight {
    pub position: Vector,
    pub diffuse: Vector,
    /// PointLight.ctor's AmbientColor (0.2, 0.2, 0.2); DynamicLight never
    /// assigns it.
    pub ambient: Vector,
    pub dynamic_intensity: f32,
    pub radius: f32,
    pub speed: f32,
    pub specular: f32,
    pub ttl: f32,
    enabled: bool,
    time: f32,
    transition_time: f32,
    intensity: f32,
    removed: bool,
}

impl DynamicLight {
    /// Initialize(9) followed by Enable: Light.Enable(scene, transition 1,
    /// 1/speed) zeroes mIntensity and mTime.
    pub fn new(
        position: Vector,
        diffuse: Vector,
        dynamic_intensity: f32,
        radius: f32,
        speed: f32,
        specular: f32,
        ttl: f32,
    ) -> Self {
        Self {
            position,
            diffuse,
            ambient: [0.2; 3],
            dynamic_intensity,
            radius,
            speed,
            specular,
            ttl,
            enabled: true,
            time: 0.0,
            transition_time: 1.0 / speed,
            intensity: 0.0,
            removed: false,
        }
    }

    /// A network client's copy of a host light, at its current brightness.
    pub fn mirrored(
        position: Vector,
        diffuse: Vector,
        intensity: f32,
        radius: f32,
        specular: f32,
    ) -> Self {
        let mut light = Self::new(position, diffuse, intensity, radius, 1.0, specular, -1.0);
        light.intensity = 1.0;
        light
    }

    pub fn is_removed(&self) -> bool {
        self.removed
    }

    /// Effective Light.Draw intensity: mIntensity · mDynamicIntensity.
    pub fn draw_intensity(&self) -> f32 {
        self.intensity * self.dynamic_intensity
    }

    /// DynamicLight.Update: Light.Update's transition, then the TTL
    /// countdown (TTL −1 means infinite) and Stop(false) at TTL ≤ 0.
    /// Velocity is zero and friction one for ArcaneBlast lights.
    pub fn update(&mut self, dt: f32) {
        if self.removed {
            return;
        }
        // Light.Update with an active transition (type 1: linear).
        if self.enabled {
            self.time += dt;
            if self.time >= self.transition_time {
                self.intensity = 1.0;
                self.enabled_steady();
            } else {
                self.intensity = self.time / self.transition_time;
            }
        } else {
            self.time -= dt;
            if self.time <= 0.0 {
                self.removed = true;
                self.intensity = 0.0;
                return;
            }
            self.intensity = self.time / self.transition_time;
        }
        if self.ttl != -1.0 {
            self.ttl -= dt;
            if self.ttl <= 0.0 {
                self.disable();
            }
        }
    }

    /// Steady state after a completed enable transition (mTransitionEffect
    /// cleared: intensity stays 1).
    fn enabled_steady(&mut self) {
        self.time = self.transition_time;
    }

    /// DynamicLight.Stop(false): fade out, then removed.
    pub fn stop(&mut self) {
        self.disable();
    }

    /// Stop(false) → Light.Disable(transition 1, 1/speed): mTime =
    /// mTransitionTime, then counts down.
    fn disable(&mut self) {
        if !self.enabled {
            return;
        }
        self.enabled = false;
        self.transition_time = 1.0 / self.speed;
        self.time = self.transition_time;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arcane_blast_grows_fades_and_dies_after_four_tenths() {
        let mut b = ArcaneBlast::new([1.0, 2.0, 3.0], 5.0, Elements::ARCANE.0);
        assert_eq!(b.radius(), 0.0);
        b.update(0.1);
        assert!((b.ttl - 0.5).abs() < 1e-6);
        assert!((b.radius() - 2.5).abs() < 1e-5);
        assert_eq!(b.tint_alpha(), 0.5);
        b.update(0.1);
        b.update(0.05);
        // TTL −0.25: alpha min(1, 0) · 0.5.
        assert!(b.tint_alpha().abs() < 1e-5);
        assert!(!b.is_dead());
        b.update(0.15);
        assert!(b.is_dead());
    }

    #[test]
    fn draws_two_counter_rotating_discs_per_beam_element() {
        let mut b = ArcaneBlast::new([0.0; 3], 4.0, Elements::ARCANE.0 | Elements::LIFE.0);
        b.update(0.1);
        let draws = b.draws();
        assert_eq!(draws.len(), 4);
        // Arcane row 0.5, Life row 0.625; V scaled by 0.125.
        assert_eq!(draws[0].uvs[2], [1.0, 0.625]);
        assert_eq!(draws[2].uvs[0], [0.0, 0.625]);
        // Second disc is 0.9 of the radius (corner distance √2·r).
        let len = |p: Vector| (p[0] * p[0] + p[2] * p[2]).sqrt();
        let r = b.radius();
        assert!((len(draws[0].positions[0]) - r * 2f32.sqrt()).abs() < 1e-4);
        assert!((len(draws[1].positions[0]) - 0.9 * r * 2f32.sqrt()).abs() < 1e-4);
    }

    #[test]
    fn dynamic_light_ramps_in_then_out_after_its_ttl() {
        let b = ArcaneBlast::new([0.0; 3], 5.0, Elements::LIFE.0);
        let mut light = b.light();
        assert_eq!(light.radius, 10.0);
        light.update(0.125);
        assert!((light.draw_intensity() - 0.5).abs() < 1e-6);
        // At TTL 0.25 Stop(false) restarts the timer at 0.5 s.
        light.update(0.125);
        assert!((light.draw_intensity() - 1.0).abs() < 1e-6);
        light.update(0.25);
        assert!((light.draw_intensity() - 1.0).abs() < 1e-6);
        light.update(0.25);
        assert!(light.is_removed());
    }
}
