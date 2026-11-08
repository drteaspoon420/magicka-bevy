//! Pure spatial-audio maths for the play mode (research/audio-xact.md,
//! "2026-10-30 — Spatial audio"): the native listener placement, the
//! X3DAudio stereo pan law Cue.Apply3D produces, the doppler factor, XACT
//! cue instance limiting and the XAudio2 track filter. No Bevy types; every
//! function is deterministic and unit-tested here.
//!
//! Provenance:
//! - Listener (PROVEN, MagickCamera.Update IL_067d..IL_06da): position =
//!   camera ground position + (0, 20, 0), Forward = Vector3.Down, Up =
//!   Vector3.Forward. The listener velocity and DopplerScale are never
//!   written; the only emitter velocity write is UnderGroundAttack's.
//! - Pan law (INFERRED from the FAudio reimplementation of X3DAudio, which
//!   XNA's Cue.Apply3D wraps through XACT3DCalculate/XACT3DApply): XNA
//!   negates Z to enter X3DAudio's left-handed frame; right = top × front;
//!   the emitter direction is projected onto the plane normal to `top`,
//!   azimuth = atan2(dot(right), dot(front)); stereo output uses the side
//!   speakers at ±90° and splits the energy linearly between them; the
//!   default volume curve XACT substitutes is flat, so X3DAudio attenuates
//!   nothing and the `Distance` RPC curve carries all distance gain.
//! - Doppler (INFERRED, same source): factor = (c − v_l)/(c − v_e) with the
//!   velocities projected onto the emitter→listener line, clamped to
//!   [0.5, 4]; with zero velocities it is identically 1.
//! - Instance limits (INFERRED, FAudio FACT): behaviour = flags >> 3; at the
//!   limit Fail stops the new cue, Replace* fades the chosen instance over
//!   the cue's fade-out and plays the new one.
//! - Filter (INFERRED, XAudio2 XAUDIO2_FILTER_PARAMETERS): a state-variable
//!   filter with radian frequency 2·sin(π·f/rate) capped at 1 and 1/Q
//!   capped at 1.5; type 0 low-pass, 1 band-pass, 2 high-pass, 3 notch.
use magicka_content::xact::InstanceBehavior;

pub type Vec3 = [f32; 3];

/// MagickCamera.Update: listener height above the camera ground position.
pub const LISTENER_HEIGHT: f32 = 20.0;

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}

/// XNA → X3DAudio: the framework negates Z (right- to left-handed).
fn to_x3d(v: Vec3) -> Vec3 {
    [v[0], v[1], -v[2]]
}

/// The XNA AudioListener the camera maintains (XNA right-handed vectors).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    pub position: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
}

impl Listener {
    /// MagickCamera.Update: 20 above the ground position, looking down,
    /// with world "forward" (−Z) as up.
    pub fn from_camera_ground(ground: Vec3) -> Self {
        Self {
            position: [ground[0], ground[1] + LISTENER_HEIGHT, ground[2]],
            forward: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, -1.0],
        }
    }
}

/// Linear channel weights for a mono emitter; they sum to one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StereoGain {
    pub left: f32,
    pub right: f32,
}

impl StereoGain {
    pub const CENTRE: Self = Self {
        left: 0.5,
        right: 0.5,
    };
    /// No panning: both channels pass unchanged (non-3D cues).
    pub const UNITY: Self = Self {
        left: 1.0,
        right: 1.0,
    };
}

/// X3DAudio's azimuth of `emitter` in the listener's horizontal plane:
/// 0 ahead, +π/2 right, measured clockwise from `forward`, in [0, 2π).
/// `None` when the emitter sits on the listener's vertical axis.
pub fn azimuth(listener: &Listener, emitter: Vec3) -> Option<f32> {
    let front = to_x3d(listener.forward);
    let top = to_x3d(listener.up);
    let right = cross(top, front);
    let d = sub(to_x3d(emitter), to_x3d(listener.position));
    let along_top = dot(d, top);
    let proj = sub(
        d,
        [top[0] * along_top, top[1] * along_top, top[2] * along_top],
    );
    if length(proj) <= 1e-6 {
        return None;
    }
    let x = dot(front, proj);
    let y = dot(right, proj);
    let mut a = y.atan2(x);
    if a < 0.0 {
        a += std::f32::consts::TAU;
    }
    Some(a)
}

/// Stereo matrix coefficients for a mono emitter: the energy is split
/// linearly between the side speakers (right at π/2, left at 3π/2).
pub fn pan(listener: &Listener, emitter: Vec3) -> StereoGain {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    let Some(a) = azimuth(listener, emitter) else {
        return StereoGain::CENTRE;
    };
    let left_at = 3.0 * FRAC_PI_2;
    if (FRAC_PI_2..=left_at).contains(&a) {
        // Behind: from the right speaker round to the left one.
        let t = (a - FRAC_PI_2) / PI;
        StereoGain {
            left: t,
            right: 1.0 - t,
        }
    } else {
        // Ahead: from the left speaker (3π/2) round to the right one (5π/2).
        let a = if a < FRAC_PI_2 { a + TAU } else { a };
        let t = (a - left_at) / PI;
        StereoGain {
            left: 1.0 - t,
            right: t,
        }
    }
}

/// XGS `SpeedOfSound` initial value (magicka.xgs variable 5).
#[cfg_attr(not(test), allow(dead_code))]
pub const SPEED_OF_SOUND: f32 = 343.0;

/// X3DAudio doppler pitch scalar; 1 when nothing moves. Only
/// UnderGroundAttack ever sets an emitter velocity and no sound maps
/// `DopplerPitchScalar` through an RPC, so natively nothing is audible;
/// the viewer does not wire this (documented, tested formula only).
#[cfg_attr(not(test), allow(dead_code))]
pub fn doppler_factor(
    listener_position: Vec3,
    listener_velocity: Vec3,
    emitter_position: Vec3,
    emitter_velocity: Vec3,
    doppler_scale: f32,
) -> f32 {
    if doppler_scale <= 0.0 {
        return 1.0;
    }
    let to_listener = sub(listener_position, emitter_position);
    let len = length(to_listener);
    if len <= 1e-6 {
        return 1.0;
    }
    let n = [
        to_listener[0] / len,
        to_listener[1] / len,
        to_listener[2] / len,
    ];
    let limit = SPEED_OF_SOUND - f32::EPSILON;
    let vl = (dot(listener_velocity, n) * doppler_scale).clamp(-limit, limit);
    let ve = (dot(emitter_velocity, n) * doppler_scale).clamp(-limit, limit);
    ((SPEED_OF_SOUND - vl) / (SPEED_OF_SOUND - ve)).clamp(0.5, 4.0)
}

/// A playing instance of one cue, as the limiter sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveInstance {
    pub id: u32,
    /// Start order (lower = older).
    pub order: u64,
    /// Current level, for Replace Quietest.
    pub gain_db: f32,
    /// XACT sound priority, for Replace Lowest Priority.
    pub priority: u8,
}

/// The limiter's verdict for a new play of a cue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Play,
    /// Fail To Play: the new cue is dropped.
    Fail,
    /// Queue: hold the new cue until an instance ends.
    Queue,
    /// Replace: stop (fade out) this instance, then play.
    Replace(u32),
}

/// XACT cue instance limiting over the cue's currently playing instances
/// (stopping ones excluded by the caller). `limit` 255 is unlimited.
pub fn admit(live: &[LiveInstance], limit: u8, behavior: InstanceBehavior) -> Admission {
    if limit == 255 || live.len() < limit as usize {
        return Admission::Play;
    }
    let pick = |key: &dyn Fn(&LiveInstance) -> f32| {
        live.iter()
            .min_by(|a, b| key(a).total_cmp(&key(b)).then(a.order.cmp(&b.order)))
            .map_or(Admission::Fail, |i| Admission::Replace(i.id))
    };
    match behavior {
        InstanceBehavior::FailToPlay => Admission::Fail,
        InstanceBehavior::Queue => Admission::Queue,
        InstanceBehavior::ReplaceOldest => pick(&|i| i.order as f32),
        InstanceBehavior::ReplaceQuietest => pick(&|i| i.gain_db),
        InstanceBehavior::ReplaceLowestPriority => pick(&|i| i.priority as f32),
    }
}

/// XAudio2 filter type (XAUDIO2_FILTER_TYPE order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    LowPass,
    BandPass,
    HighPass,
    Notch,
}

impl FilterKind {
    pub fn from_xact(kind: u8) -> Self {
        match kind {
            0 => Self::LowPass,
            1 => Self::BandPass,
            2 => Self::HighPass,
            _ => Self::Notch,
        }
    }
}

/// The authored track filter, in XAudio2 terms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterParams {
    pub kind: FilterKind,
    /// Cutoff / centre frequency in Hz.
    pub frequency: f32,
    /// Quality factor (XACT stores it in hundredths).
    pub q: f32,
}

impl FilterParams {
    /// From the parsed clip filter word (Q = q_raw / 100, INFERRED).
    pub fn from_clip(f: &magicka_content::xact::ClipFilter) -> Self {
        Self {
            kind: FilterKind::from_xact(f.kind),
            frequency: f.frequency,
            q: f.q_raw as f32 / 100.0,
        }
    }
}

/// XAUDIO2_MAX_FILTER_FREQUENCY.
pub const MAX_FILTER_FREQUENCY: f32 = 1.0;
/// XAUDIO2_MAX_FILTER_ONEOVERQ.
pub const MAX_FILTER_ONE_OVER_Q: f32 = 1.5;

/// One channel of the XAudio2-style state-variable filter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Svf {
    kind: FilterKind,
    /// Radian frequency 2·sin(π·f/rate), capped at 1.
    f: f32,
    one_over_q: f32,
    low: f32,
    band: f32,
}

impl Svf {
    pub fn new(params: FilterParams, sample_rate: u32) -> Self {
        let rate = sample_rate.max(1) as f32;
        let f = (2.0 * (std::f32::consts::PI * params.frequency.max(0.0) / rate).sin())
            .clamp(0.0, MAX_FILTER_FREQUENCY);
        let one_over_q = if params.q > 0.0 {
            (1.0 / params.q).min(MAX_FILTER_ONE_OVER_Q)
        } else {
            MAX_FILTER_ONE_OVER_Q
        };
        Self {
            kind: params.kind,
            f,
            one_over_q,
            low: 0.0,
            band: 0.0,
        }
    }

    pub fn process(&mut self, x: f32) -> f32 {
        self.low += self.f * self.band;
        let high = x - self.low - self.one_over_q * self.band;
        self.band += self.f * high;
        match self.kind {
            FilterKind::LowPass => self.low,
            FilterKind::BandPass => self.band,
            FilterKind::HighPass => high,
            FilterKind::Notch => self.low + high,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn listener_sits_twenty_above_the_camera_ground_looking_down() {
        let l = Listener::from_camera_ground([10.0, 2.0, -30.0]);
        assert_eq!(l.position, [10.0, 22.0, -30.0]);
        assert_eq!(l.forward, [0.0, -1.0, 0.0]);
        assert_eq!(l.up, [0.0, 0.0, -1.0]);
    }

    #[test]
    fn screen_right_emitters_pan_right_and_screen_left_pan_left() {
        // The camera looks along −Z from +Z (CAMERAOFFSET 0,144,171), so
        // world +X is screen right; a ground emitter 20 to the right is 45°.
        let l = Listener::from_camera_ground([0.0, 0.0, 0.0]);
        assert!(near(azimuth(&l, [20.0, 0.0, 0.0]).unwrap(), FRAC_PI_4));
        let g = pan(&l, [20.0, 0.0, 0.0]);
        assert!(near(g.left, 0.25) && near(g.right, 0.75));
        let g = pan(&l, [-20.0, 0.0, 0.0]);
        assert!(near(g.left, 0.75) && near(g.right, 0.25));
        // Far to the right approaches the right speaker.
        let g = pan(&l, [2000.0, 0.0, 0.0]);
        assert!(g.right > 0.99 && near(g.left + g.right, 1.0));
    }

    #[test]
    fn emitters_ahead_or_on_the_screen_vertical_are_centred() {
        let l = Listener::from_camera_ground([5.0, 1.0, 5.0]);
        // Same X as the listener, anywhere along world Z (listener "up").
        for z in [-40.0, 5.0, 60.0] {
            let g = pan(&l, [5.0, 1.0, z]);
            assert!(near(g.left, 0.5) && near(g.right, 0.5));
        }
        // Directly below the listener, in its forward direction: azimuth 0.
        assert!(near(azimuth(&l, [5.0, 1.0, 5.0]).unwrap(), 0.0));
        // Exactly at the listener: no direction.
        assert_eq!(azimuth(&l, l.position), None);
    }

    #[test]
    fn emitters_above_the_listener_pan_from_behind() {
        // Above the listener height the emitter is "behind" the downward
        // forward vector: the weights still favour its side.
        let l = Listener::from_camera_ground([0.0, 0.0, 0.0]);
        let a = azimuth(&l, [20.0, 40.0, 0.0]).unwrap();
        assert!(near(a, 3.0 * FRAC_PI_4));
        let g = pan(&l, [20.0, 40.0, 0.0]);
        assert!(near(g.right, 0.75) && near(g.left, 0.25));
        let a = azimuth(&l, [0.0, 40.0, 0.0]).unwrap();
        assert!(near(a, PI));
        let g = pan(&l, [0.0, 40.0, 0.0]);
        assert!(near(g.left, 0.5) && near(g.right, 0.5));
        let a = azimuth(&l, [-20.0, 20.0, 0.0]).unwrap();
        assert!(near(a, 3.0 * FRAC_PI_2));
    }

    #[test]
    fn listener_motion_does_not_change_the_pan_of_a_moving_frame() {
        let a = Listener::from_camera_ground([100.0, 3.0, -200.0]);
        let b = Listener::from_camera_ground([0.0, 0.0, 0.0]);
        let (ga, gb) = (pan(&a, [112.0, 3.0, -190.0]), pan(&b, [12.0, 0.0, 10.0]));
        assert!(near(ga.left, gb.left) && near(ga.right, gb.right));
        let g = pan(&b, [12.0, 0.0, 10.0]);
        assert!(near(g.right, 0.5 + (12f32 / 20.0).atan() / PI));
    }

    #[test]
    fn doppler_is_unity_without_motion_and_clamped_with_it() {
        let f = doppler_factor([0.0, 20.0, 0.0], [0.0; 3], [30.0, 0.0, 0.0], [0.0; 3], 1.0);
        assert_eq!(f, 1.0);
        // Emitter approaching the listener at 34.3 m/s raises pitch.
        let f = doppler_factor(
            [0.0, 0.0, 0.0],
            [0.0; 3],
            [100.0, 0.0, 0.0],
            [-34.3, 0.0, 0.0],
            1.0,
        );
        assert!(near(f, 343.0 / (343.0 - 34.3)));
        // Receding faster than sound is clamped to one octave down.
        let f = doppler_factor(
            [0.0, 0.0, 0.0],
            [0.0; 3],
            [100.0, 0.0, 0.0],
            [10_000.0, 0.0, 0.0],
            1.0,
        );
        assert_eq!(f, 0.5);
        assert_eq!(
            doppler_factor([0.0; 3], [0.0; 3], [1.0, 0.0, 0.0], [-300.0, 0.0, 0.0], 0.0),
            1.0
        );
    }

    fn live(n: u32) -> Vec<LiveInstance> {
        (0..n)
            .map(|i| LiveInstance {
                id: 100 + i,
                order: 10 + i as u64,
                gain_db: -(i as f32) * 3.0,
                priority: 5u8.saturating_sub(i as u8),
            })
            .collect()
    }

    #[test]
    fn instance_limits_admit_below_the_limit_and_apply_the_behaviour_at_it() {
        use InstanceBehavior as B;
        assert_eq!(admit(&live(3), 255, B::FailToPlay), Admission::Play);
        assert_eq!(admit(&live(2), 3, B::FailToPlay), Admission::Play);
        assert_eq!(admit(&live(3), 3, B::FailToPlay), Admission::Fail);
        assert_eq!(admit(&live(3), 3, B::Queue), Admission::Queue);
        // Oldest = lowest start order (id 100); quietest = lowest gain
        // (id 102); lowest priority = smallest priority number (id 102).
        assert_eq!(
            admit(&live(3), 3, B::ReplaceOldest),
            Admission::Replace(100)
        );
        assert_eq!(
            admit(&live(4), 3, B::ReplaceQuietest),
            Admission::Replace(103)
        );
        assert_eq!(
            admit(&live(3), 3, B::ReplaceLowestPriority),
            Admission::Replace(102)
        );
        // Retail: spell cues are limit 10 / replace oldest (flags 0x12).
        assert_eq!(
            admit(&live(10), 10, InstanceBehavior::from_flags(0x12)),
            Admission::Replace(100)
        );
        assert_eq!(admit(&[], 1, B::FailToPlay), Admission::Play);
    }

    fn run(mut f: Svf, input: impl Iterator<Item = f32>) -> Vec<f32> {
        input.map(|x| f.process(x)).collect()
    }

    #[test]
    fn state_variable_filter_passes_and_blocks_as_its_type_says() {
        let rate = 48_000;
        let lp = Svf::new(
            FilterParams {
                kind: FilterKind::LowPass,
                frequency: 1000.0,
                q: 0.66,
            },
            rate,
        );
        // DC settles to unity through the low-pass.
        let out = run(lp, std::iter::repeat(1.0).take(4000));
        assert!(near(out[3999], 1.0));
        // Nyquist (alternating) is strongly attenuated by it.
        let out = run(lp, (0..4000).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }));
        assert!(out[3990..].iter().all(|v| v.abs() < 0.05));
        // The high-pass does the opposite.
        let hp = Svf::new(
            FilterParams {
                kind: FilterKind::HighPass,
                frequency: 1000.0,
                q: 0.66,
            },
            rate,
        );
        let out = run(hp, std::iter::repeat(1.0).take(4000));
        assert!(out[3999].abs() < 0.01);
        let out = run(hp, (0..4000).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }));
        assert!(out[3999].abs() > 0.9);
        // A 20 kHz low-pass at 44.1 kHz is nearly transparent (sw_s6 music).
        let open = Svf::new(
            FilterParams {
                kind: FilterKind::LowPass,
                frequency: 20_000.0,
                q: 1.0,
            },
            44_100,
        );
        assert_eq!(open.f, MAX_FILTER_FREQUENCY);
        assert_eq!(open.one_over_q, 1.0);
        // Q below XAudio2's floor clamps 1/Q to 1.5.
        let wide = Svf::new(
            FilterParams {
                kind: FilterKind::Notch,
                frequency: 500.0,
                q: 0.1,
            },
            rate,
        );
        assert_eq!(wide.one_over_q, MAX_FILTER_ONE_OVER_Q);
    }

    #[test]
    fn filter_params_come_from_the_clip_word() {
        let clip = magicka_content::xact::ClipFilter::from_raw(0x4e20_0321).unwrap();
        let p = FilterParams::from_clip(&clip);
        assert_eq!(p.kind, FilterKind::LowPass);
        assert_eq!(p.frequency, 20_000.0);
        assert!(near(p.q, 1.0));
        let clip = magicka_content::xact::ClipFilter::from_raw(0x0740_0213).unwrap();
        let p = FilterParams::from_clip(&clip);
        assert_eq!(p.kind, FilterKind::BandPass);
        assert_eq!(p.frequency, 1856.0);
        assert!(near(p.q, 0.66));
        assert_eq!(
            magicka_content::xact::ClipFilter::from_raw(0x03e8_5dc0),
            None
        );
    }
}
