//! Shared decoded PCM source for gameplay and frontend cue playback.
//!
//! A clip may carry a `VoiceControl`: the decoder then emits stereo frames,
//! runs the authored XAudio2-style track filter and applies the X3DAudio
//! channel weights the play mode updates each frame
//! (`play/spatial.rs`, research/audio-xact.md "Spatial audio").
use bevy::{
    audio::{AddAudioSource, ChannelCount, Decodable, SampleRate, Source},
    prelude::*,
    reflect::TypePath,
};
use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::play::spatial::{FilterParams, StereoGain, Svf};

/// The frontend and gameplay may share an app; register the source once.
pub fn register(app: &mut App) {
    if !app.world().contains_resource::<Assets<PcmClip>>() {
        app.add_audio_source::<PcmClip>();
    }
}

/// Per-voice presentation shared between the game thread and the decoder:
/// the stereo weights (updated as the emitter or listener moves) and the
/// clip's fixed track filter.
#[derive(Debug)]
pub struct VoiceControl {
    left: AtomicU32,
    right: AtomicU32,
    filter: Option<FilterParams>,
}

impl VoiceControl {
    pub fn new(filter: Option<FilterParams>, gain: StereoGain) -> Self {
        Self {
            left: AtomicU32::new(gain.left.to_bits()),
            right: AtomicU32::new(gain.right.to_bits()),
            filter,
        }
    }

    pub fn set_pan(&self, gain: StereoGain) {
        self.left.store(gain.left.to_bits(), Ordering::Relaxed);
        self.right.store(gain.right.to_bits(), Ordering::Relaxed);
    }

    pub fn pan(&self) -> StereoGain {
        StereoGain {
            left: f32::from_bits(self.left.load(Ordering::Relaxed)),
            right: f32::from_bits(self.right.load(Ordering::Relaxed)),
        }
    }

    pub fn filter(&self) -> Option<FilterParams> {
        self.filter
    }
}

/// Decoded wave samples (interleaved f32), optionally looping forever.
#[derive(Asset, TypePath, Clone)]
pub struct PcmClip {
    channels: u16,
    rate: u32,
    samples: Arc<[f32]>,
    looping: bool,
    control: Option<Arc<VoiceControl>>,
}

pub struct PcmDecoder {
    clip: PcmClip,
    pos: usize,
    /// Stereo presentation state when the clip has a control.
    stereo: Option<StereoState>,
}

struct StereoState {
    control: Arc<VoiceControl>,
    filters: Option<[Svf; 2]>,
    /// The pending right sample of the current frame.
    pending: Option<f32>,
}

impl PcmDecoder {
    fn raw(&mut self) -> Option<f32> {
        if self.pos >= self.clip.samples.len() {
            if !self.clip.looping || self.clip.samples.is_empty() {
                return None;
            }
            self.pos = 0;
        }
        let v = self.clip.samples[self.pos];
        self.pos += 1;
        Some(v)
    }

    /// One source frame folded to stereo: mono is duplicated, extra
    /// channels beyond the second are dropped.
    fn frame(&mut self) -> Option<[f32; 2]> {
        let channels = self.clip.channels.max(1) as usize;
        let first = self.raw()?;
        let mut frame = [first, first];
        for c in 1..channels {
            let v = self.raw()?;
            if c == 1 {
                frame[1] = v;
            }
        }
        Some(frame)
    }
}

impl Iterator for PcmDecoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.stereo.is_none() {
            return self.raw();
        }
        if let Some(right) = self.stereo.as_mut().and_then(|s| s.pending.take()) {
            return Some(right);
        }
        let mut frame = self.frame()?;
        let stereo = self.stereo.as_mut()?;
        if let Some(filters) = stereo.filters.as_mut() {
            frame[0] = filters[0].process(frame[0]);
            frame[1] = filters[1].process(frame[1]);
        }
        let gain = stereo.control.pan();
        stereo.pending = Some(frame[1] * gain.right);
        Some(frame[0] * gain.left)
    }
}

impl Source for PcmDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        let n = if self.stereo.is_some() {
            2
        } else {
            self.clip.channels.max(1)
        };
        ChannelCount::new(n).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.clip.rate.max(1)).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        (!self.clip.looping).then(|| {
            Duration::from_secs_f64(
                self.clip.samples.len() as f64
                    / (self.clip.channels.max(1) as f64 * self.clip.rate.max(1) as f64),
            )
        })
    }
}

impl Decodable for PcmClip {
    type Decoder = PcmDecoder;
    fn decoder(&self) -> PcmDecoder {
        let stereo = self.control.clone().map(|control| StereoState {
            filters: control
                .filter()
                .map(|p| [Svf::new(p, self.rate), Svf::new(p, self.rate)]),
            control,
            pending: None,
        });
        PcmDecoder {
            clip: self.clone(),
            pos: 0,
            stereo,
        }
    }
}

impl PcmClip {
    pub fn from_pcm(pcm: magicka_content::xact::Pcm, loops: u8) -> Self {
        let looping = loops == 255;
        let mut samples: Vec<f32> = pcm.samples.iter().map(|&s| s as f32 / 32768.0).collect();
        if !looping && loops > 0 {
            let one = samples.clone();
            for _ in 0..loops {
                samples.extend_from_slice(&one);
            }
        }
        Self {
            channels: pcm.channels,
            rate: pcm.sample_rate,
            samples: samples.into(),
            looping,
            control: None,
        }
    }

    /// The same samples (shared, not copied) presented through `control`.
    pub fn with_control(&self, control: Arc<VoiceControl>) -> Self {
        Self {
            control: Some(control),
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::spatial::FilterKind;
    use magicka_content::xact::Pcm;

    fn pcm() -> Pcm {
        Pcm {
            channels: 2,
            sample_rate: 2,
            samples: vec![i16::MIN, 0, i16::MAX, 16384],
        }
    }

    #[test]
    fn finite_loops_repeat_complete_interleaved_frames_and_end() {
        let clip = PcmClip::from_pcm(pcm(), 2);
        let decoder = clip.decoder();
        assert_eq!(decoder.total_duration(), Some(Duration::from_secs(3)));
        let samples: Vec<_> = decoder.collect();
        assert_eq!(samples.len(), 12);
        assert_eq!(&samples[..4], &[-1.0, 0.0, 32767.0 / 32768.0, 0.5]);
        assert_eq!(&samples[..4], &samples[8..]);
    }

    #[test]
    fn infinite_loops_have_no_duration_and_empty_clips_terminate() {
        let clip = PcmClip::from_pcm(pcm(), 255);
        assert_eq!(clip.decoder().total_duration(), None);
        assert_eq!(clip.decoder().take(12).count(), 12);
        let empty = PcmClip::from_pcm(
            Pcm {
                channels: 2,
                sample_rate: 2,
                samples: vec![],
            },
            255,
        );
        assert_eq!(empty.decoder().next(), None);
    }

    #[test]
    fn controlled_mono_clips_become_panned_stereo_frames() {
        let mono = PcmClip::from_pcm(
            Pcm {
                channels: 1,
                sample_rate: 4,
                samples: vec![16384, -16384, 32767],
            },
            0,
        );
        let control = Arc::new(VoiceControl::new(
            None,
            StereoGain {
                left: 0.25,
                right: 0.75,
            },
        ));
        let clip = mono.with_control(control.clone());
        let mut d = clip.decoder();
        assert_eq!(d.channels().get(), 2);
        assert_eq!(d.sample_rate().get(), 4);
        assert_eq!(d.total_duration(), Some(Duration::from_millis(750)));
        assert_eq!(d.next(), Some(0.125));
        assert_eq!(d.next(), Some(0.375));
        // The game thread moves the emitter between frames.
        control.set_pan(StereoGain::UNITY);
        assert_eq!(d.next(), Some(-0.5));
        assert_eq!(d.next(), Some(-0.5));
        assert_eq!(d.next(), Some(32767.0 / 32768.0));
        assert_eq!(d.next(), Some(32767.0 / 32768.0));
        assert_eq!(d.next(), None);
        // The uncontrolled clip is untouched.
        assert_eq!(mono.decoder().channels().get(), 1);
    }

    #[test]
    fn controlled_stereo_clips_keep_their_channels_and_filter_each() {
        let clip = PcmClip::from_pcm(pcm(), 0);
        let unity = Arc::new(VoiceControl::new(None, StereoGain::UNITY));
        let out: Vec<_> = clip.with_control(unity).decoder().collect();
        assert_eq!(out, vec![-1.0, 0.0, 32767.0 / 32768.0, 0.5]);
        // A fully closed low-pass (0 Hz) passes nothing on either channel.
        let closed = Arc::new(VoiceControl::new(
            Some(FilterParams {
                kind: FilterKind::LowPass,
                frequency: 0.0,
                q: 1.0,
            }),
            StereoGain::UNITY,
        ));
        let out: Vec<_> = clip.with_control(closed).decoder().collect();
        assert_eq!(out, vec![0.0; 4]);
        // A high-pass at 0 Hz is transparent for the first sample and the
        // state then follows: the decoder runs one filter per channel.
        let open = Arc::new(VoiceControl::new(
            Some(FilterParams {
                kind: FilterKind::HighPass,
                frequency: 0.0,
                q: 1.0,
            }),
            StereoGain::UNITY,
        ));
        let out: Vec<_> = clip.with_control(open).decoder().collect();
        assert_eq!(out, vec![-1.0, 0.0, 32767.0 / 32768.0, 0.5]);
    }
}
