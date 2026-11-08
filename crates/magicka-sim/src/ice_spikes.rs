//! IceSpikes (Blast.GroundBlast with an Ice damage; research/ground-area.md):
//! up to 18 `IceBarrier0_mesh` spikes on two rings, each playing a random
//! IceBarrier_animation clip forward at ClipSpeed 2 and then backward. Scene
//! probes, randomness and drawing are host work.
use crate::barrier::Clip;

pub type Vector = [f32; 3];

/// IceSpikes controller count: an outer ring of 12 and an inner ring of 6.
pub const SLOTS: usize = 18;
/// Probes drop this far (Segment.Delta.Y = −3).
pub const PROBE_DROP: f32 = 3.0;
/// Initialize: scale = radius / 4.332.
pub const SCALE_DIVISOR: f32 = 4.332;

/// The 18 probe origins: the outer ring at 2·1.666·scale (from 5π/6 in
/// π/6 steps, 0.5 m up), then the inner ring at 1.666·scale (continuing in
/// π/3 steps, 1.5 m up). Angles accumulate in f32 as the IL does.
pub fn probe_origins(center: Vector, radius: f32) -> [Vector; SLOTS] {
    let scale = radius / SCALE_DIVISOR;
    let mut angle = 2.617_993_8_f32;
    let mut out = [[0.0; 3]; SLOTS];
    for (i, origin) in out.iter_mut().enumerate() {
        let (ring, lift, step) = if i < 12 {
            (2.0, 0.5, std::f32::consts::PI / 6.0)
        } else {
            (1.0, 1.5, std::f32::consts::PI / 3.0)
        };
        // (float)Math.Sin(angle) · ring · 1.666 · scale, in IL order.
        let (s, c) = ((angle as f64).sin() as f32, (angle as f64).cos() as f32);
        let x = s * ring * 1.666 * scale;
        let z = c * ring * 1.666 * scale;
        *origin = [center[0] + x, center[1] + lift, center[2] + z];
        angle += step;
    }
    out
}

/// One drawn spike.
#[derive(Clone, Debug, PartialEq)]
pub struct Spike {
    pub position: Vector,
    /// CreateRotationY(NextDouble·2π).
    pub yaw: f32,
    pub scale: f32,
    /// Index into IceBarrier_animation's clips (emerge0..4).
    pub clip_index: usize,
    pub clip: Clip,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IceSpikes {
    pub spikes: Vec<Spike>,
    /// mDead: set when the first spike turns around.
    pub dead: bool,
}

impl IceSpikes {
    /// Initialize with the probe hits (None = no ground). For each hit, in
    /// slot order: yaw = NextDouble·2π, then clip = Next(len), both from
    /// `random` (uniform in [0, 1)). Returns None when no probe hit (the
    /// effect is not added).
    pub fn initialize(
        radius: f32,
        hits: &[Option<Vector>; SLOTS],
        clip_durations: &[f32],
        mut random: impl FnMut() -> f32,
    ) -> Option<Self> {
        let n = clip_durations.len();
        let scale = radius / SCALE_DIVISOR;
        let spikes: Vec<Spike> = hits
            .iter()
            .flatten()
            .map(|&position| {
                let yaw = random() * std::f32::consts::TAU;
                let clip_index = ((random() * n as f32) as usize).min(n - 1);
                Spike {
                    position,
                    yaw,
                    scale,
                    clip_index,
                    clip: Clip::start(clip_durations[clip_index], false),
                }
            })
            .collect();
        (!spikes.is_empty()).then_some(Self {
            spikes,
            dead: false,
        })
    }

    /// Update: a spike whose forward clip finished restarts it backward (and
    /// marks the effect dead); then every clip advances.
    pub fn update(&mut self, dt: f32) {
        for spike in &mut self.spikes {
            if !spike.clip.playing && !spike.clip.backward {
                spike.clip = Clip::start(spike.clip.duration, true);
                self.dead = true;
            }
            spike.clip.update(dt);
        }
    }

    /// IsDead: mDead and every drawn controller finished.
    pub fn is_dead(&self) -> bool {
        self.dead && self.spikes.iter().all(|s| !s.clip.playing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_form_two_rings_starting_at_five_sixths_pi() {
        let o = probe_origins([10.0, 1.0, -4.0], 4.332);
        // scale 1: outer radius 3.332, inner 1.666.
        for (i, p) in o.iter().enumerate() {
            let r = ((p[0] - 10.0).powi(2) + (p[2] + 4.0).powi(2)).sqrt();
            let (expected, lift) = if i < 12 { (3.332, 1.5) } else { (1.666, 2.5) };
            assert!((r - expected).abs() < 1e-4, "{i}: {r}");
            assert_eq!(p[1], lift);
        }
        // First outer probe at angle 5π/6: (sin, cos) = (0.5, −0.866).
        assert!((o[0][0] - (10.0 + 0.5 * 3.332)).abs() < 1e-4);
        assert!((o[0][2] - (-4.0 - 0.866_025 * 3.332)).abs() < 1e-3);
        // The inner ring continues from 5π/6 + 2π.
        assert!((o[12][0] - (10.0 + 0.5 * 1.666)).abs() < 1e-3);
    }

    #[test]
    fn spikes_rise_then_sink_and_the_effect_dies_after_both_passes() {
        let mut hits = [None; SLOTS];
        hits[0] = Some([0.0; 3]);
        hits[5] = Some([1.0, 0.0, 0.0]);
        assert!(IceSpikes::initialize(4.332, &[None; SLOTS], &[0.5], || 0.0).is_none());
        let mut draws = [0.25, 0.99, 0.5, 0.1].into_iter();
        let mut s =
            IceSpikes::initialize(8.664, &hits, &[0.5; 5], || draws.next().unwrap()).unwrap();
        assert_eq!(s.spikes.len(), 2);
        assert_eq!(s.spikes[0].scale, 2.0);
        assert_eq!(s.spikes[0].clip_index, 4);
        assert_eq!(s.spikes[1].clip_index, 0);
        assert!((s.spikes[0].yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        // ClipSpeed 2: 0.25 s up, 0.25 s down.
        let mut frames = 0;
        while !s.is_dead() {
            s.update(1.0 / 60.0);
            frames += 1;
            assert!(frames < 100);
        }
        assert!((29..=32).contains(&frames), "{frames}");
        assert!(s
            .spikes
            .iter()
            .all(|k| k.clip.backward && k.clip.time == 0.0));
    }
}
