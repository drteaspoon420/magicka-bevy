//! XACT cue playback for the play mode (research/audio-xact.md): session
//! audio requests become Bevy audio entities playing decoded retail waves.
//! Each frame the voice level is recomputed from the XGS category chain and
//! volume RPCs (AttackTime since start, the cue's Volume variable, emitter
//! Distance to the listener, locator NormalizedDistance). 3D cues are
//! panned from the camera listener (`spatial.rs`), cue instance limits and
//! authored fade-outs are applied, and track filters run in the decoder.
//! Reverb/DSP presets, the music focus/threat system and doppler (natively
//! inert: no velocities are ever set) are not presented.
use super::spatial::{self, Admission, FilterParams, Listener, LiveInstance, StereoGain};
use super::Play;
use super::PlayAppExt;
use bevy::audio::{
    AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackMode, PlaybackSettings, Volume,
};
use bevy::prelude::*;
use magicka_runtime::audio::{AudioLibrary, AudioRequest, CueInstanceLimit, SoundSource, Voice};
use std::collections::HashMap;
use std::sync::Arc;

use crate::pcm_audio::{PcmClip, VoiceControl};

/// Live voice counts for autopilot logs: [playing, music playing, started].
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct AudioStats(pub [usize; 3]);

/// One playing (or delayed) voice of a cue.
struct LiveVoice {
    entity: Option<Entity>,
    key: Option<u32>,
    /// Cue instance this voice belongs to (one per Start/Music request).
    instance: u32,
    cue: (u16, i32),
    voice: Voice,
    volume_db: f32,
    source: SoundSource,
    position: Option<[f32; 3]>,
    magnitude: Option<f32>,
    music: bool,
    /// Seconds since the cue started (the voice starts at `voice.delay`).
    age: f32,
    /// Queued by the cue's instance limit: not started yet.
    queued: bool,
    /// Authored fade-out in progress: (remaining, total) seconds.
    fade: Option<(f32, f32)>,
    /// Last computed level (dB), for Replace Quietest.
    gain_db: f32,
    control: Option<Arc<VoiceControl>>,
}

impl LiveVoice {
    fn spatial(&self) -> bool {
        matches!(
            self.source,
            SoundSource::Emitter(_) | SoundSource::Locator { apply_3d: true, .. }
        ) && self.position.is_some()
    }
}

#[derive(Resource)]
pub struct AudioState {
    library: Arc<AudioLibrary>,
    clips: HashMap<(String, u16, bool), Handle<PcmClip>>,
    voices: Vec<LiveVoice>,
    rng: u32,
    scene_key: (u32, u32),
    paused: bool,
    next_instance: u32,
}

pub fn plugin(app: &mut App, content: &std::path::Path) {
    let library = AudioLibrary::load(content);
    for e in &library.errors {
        warn!("audio: {e}");
    }
    crate::pcm_audio::register(app);
    let library = Arc::new(library);
    app.insert_resource(super::AudioLibraryShare(library.clone()));
    app.insert_resource(AudioState {
        library,
        clips: HashMap::new(),
        voices: Vec::new(),
        rng: 0x9e37_79b9,
        scene_key: (0, 0),
        paused: false,
        next_instance: 1,
    })
    .insert_resource(AudioStats::default())
    // The decoded library and clip cache outlive sessions; voices do not.
    .add_session_reset(|world| {
        let mut state = world.resource_mut::<AudioState>();
        state.voices.clear();
        state.scene_key = (0, 0);
        state.paused = false;
        world.insert_resource(AudioStats::default());
    })
    .add_play_systems(
        Update,
        play_audio
            .run_if(super::frontend_loading::ready)
            .after(super::step_session),
    );
}

/// What the limiter decided for a new cue instance.
enum Verdict {
    Play,
    Queue,
}

impl AudioState {
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// The cue's playing instances as the XACT limiter counts them: neither
    /// queued nor fading out.
    fn live_instances(&self, cue: (u16, i32)) -> Vec<LiveInstance> {
        let mut out: Vec<LiveInstance> = Vec::new();
        for v in self
            .voices
            .iter()
            .filter(|v| v.cue == cue && v.fade.is_none() && !v.queued)
        {
            if let Some(i) = out.iter_mut().find(|i| i.id == v.instance) {
                i.gain_db = i.gain_db.max(v.gain_db);
            } else {
                out.push(LiveInstance {
                    id: v.instance,
                    order: v.instance as u64,
                    gain_db: v.gain_db,
                    priority: 0,
                });
            }
        }
        out
    }

    /// Cue.Play against the cue's instance limit (research/audio-xact.md,
    /// spatial audio): fail, queue, or fade the replaced instance out.
    fn admit(&mut self, cue: (u16, i32), limit: Option<CueInstanceLimit>) -> Option<Verdict> {
        let Some(limit) = limit else {
            return Some(Verdict::Play);
        };
        let live = self.live_instances(cue);
        match spatial::admit(&live, limit.limit, limit.behavior) {
            Admission::Play => Some(Verdict::Play),
            Admission::Fail => None,
            Admission::Queue => Some(Verdict::Queue),
            Admission::Replace(id) => {
                self.fade_instance(id, limit.fade_out);
                Some(Verdict::Play)
            }
        }
    }

    fn fade_instance(&mut self, instance: u32, seconds: f32) {
        for v in self.voices.iter_mut().filter(|v| v.instance == instance) {
            if v.fade.is_none() {
                v.fade = Some((seconds.max(0.0), seconds.max(0.0)));
            }
        }
    }

    /// Stop(AsAuthored): the cue's fade-out, immediate when none.
    fn fade_key(&mut self, key: u32, instant: bool) {
        let instances: Vec<(u32, (u16, i32))> = self
            .voices
            .iter()
            .filter(|v| v.key == Some(key))
            .map(|v| (v.instance, v.cue))
            .collect();
        for (instance, cue) in instances {
            let seconds = if instant {
                0.0
            } else {
                self.library
                    .cue_instance_limit(cue.0, cue.1)
                    .map_or(0.0, |l| l.fade_out)
            };
            self.fade_instance(instance, seconds);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        key: Option<u32>,
        bank: u16,
        cue: i32,
        volume_db: f32,
        source: SoundSource,
        position: Option<[f32; 3]>,
        magnitude: Option<f32>,
        music: bool,
    ) {
        let library = self.library.clone();
        let mut rng = || self.random();
        let Some(voices) = library.resolve(bank, cue, &mut rng) else {
            return;
        };
        let Some(verdict) = self.admit((bank, cue), library.cue_instance_limit(bank, cue)) else {
            return;
        };
        let instance = self.next_instance;
        self.next_instance += 1;
        for voice in voices {
            self.voices.push(LiveVoice {
                entity: None,
                key,
                instance,
                cue: (bank, cue),
                voice,
                volume_db,
                source,
                position,
                magnitude,
                music,
                age: 0.0,
                queued: matches!(verdict, Verdict::Queue),
                fade: None,
                gain_db: volume_db,
                control: None,
            });
        }
    }

    fn clip(&mut self, voice: &Voice, clips: &mut Assets<PcmClip>) -> Option<Handle<PcmClip>> {
        let looping = voice.loop_count == 255;
        let id = (voice.wave_bank.clone(), voice.track, looping);
        if let Some(h) = self.clips.get(&id) {
            return Some(h.clone());
        }
        let pcm = match self.library.try_decode(&voice.wave_bank, voice.track) {
            Ok(pcm) => pcm,
            Err(error) => {
                warn!("audio: {}/{}: {error}", voice.wave_bank, voice.track);
                return None;
            }
        };
        let handle = clips.add(PcmClip::from_pcm(pcm, voice.loop_count));
        self.clips.insert(id, handle.clone());
        Some(handle)
    }
}

fn despawn_voice(commands: &mut Commands, v: &LiveVoice) {
    if let Some(e) = v.entity {
        commands.entity(e).despawn();
    }
}

#[allow(clippy::too_many_arguments)]
fn play_audio(
    mut commands: Commands,
    mut play: ResMut<Play>,
    mut state: ResMut<AudioState>,
    mut clips: ResMut<Assets<PcmClip>>,
    mut sinks: Query<&mut AudioSink>,
    time: Res<Time>,
    entities: Query<Entity>,
    mut stats: ResMut<AudioStats>,
    volumes: Res<super::save_slots::UserVolumes>,
    menu: Res<super::menu::PauseMenu>,
) {
    let paused = menu.visible;
    if paused {
        for sink in &mut sinks {
            sink.pause();
        }
        state.paused = true;
        return;
    }
    if state.paused {
        for sink in &mut sinks {
            sink.play();
        }
        state.paused = false;
    }
    let dt = time.delta_secs();
    // A new world (scene change, restart, next level) drops the old
    // scene's sounds; music continues until replaced.
    let key = (play.reloads, play.session.scene_changes);
    if key != state.scene_key {
        state.scene_key = key;
        let (keep, drop): (Vec<_>, Vec<_>) = std::mem::take(&mut state.voices)
            .into_iter()
            .partition(|v| v.music);
        for v in &drop {
            despawn_voice(&mut commands, v);
        }
        state.voices = keep;
    }
    for (request, position) in std::mem::take(&mut play.session.audio) {
        match request {
            AudioRequest::Start {
                key,
                bank,
                cue,
                volume_db,
                source,
                magnitude,
            } => state.start(
                Some(key),
                bank,
                cue,
                volume_db,
                source,
                position,
                magnitude,
                false,
            ),
            AudioRequest::Move { key, position } => {
                for v in state.voices.iter_mut().filter(|v| v.key == Some(key)) {
                    v.position = Some(position);
                }
            }
            AudioRequest::Replay { key, volume_db } => {
                for v in state.voices.iter_mut().filter(|v| v.key == Some(key)) {
                    v.volume_db = volume_db;
                }
            }
            AudioRequest::Stop { key, instant } => state.fade_key(key, instant),
            AudioRequest::Music { bank, cue } => {
                let (stop, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut state.voices)
                    .into_iter()
                    .partition(|v| v.music);
                for v in &stop {
                    despawn_voice(&mut commands, v);
                }
                state.voices = keep;
                state.start(None, bank, cue, 0.0, SoundSource::Global, None, None, true);
            }
            AudioRequest::StopMusic => {
                let (stop, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut state.voices)
                    .into_iter()
                    .partition(|v| v.music);
                for v in &stop {
                    despawn_voice(&mut commands, v);
                }
                state.voices = keep;
            }
        }
    }
    // Queued instances try again once their cue has room.
    let queued: Vec<(u32, (u16, i32))> = {
        let mut seen = Vec::new();
        for v in state.voices.iter().filter(|v| v.queued) {
            if !seen.contains(&(v.instance, v.cue)) {
                seen.push((v.instance, v.cue));
            }
        }
        seen
    };
    for (instance, cue) in queued {
        let limit = state.library.cue_instance_limit(cue.0, cue.1);
        let live: Vec<LiveInstance> = state
            .live_instances(cue)
            .into_iter()
            .filter(|i| i.id != instance)
            .collect();
        let verdict = limit.map_or(Admission::Play, |l| {
            spatial::admit(&live, l.limit, l.behavior)
        });
        match verdict {
            Admission::Play => {
                for v in state.voices.iter_mut().filter(|v| v.instance == instance) {
                    v.queued = false;
                }
            }
            Admission::Replace(id) => {
                let fade = limit.map_or(0.0, |l| l.fade_out);
                state.fade_instance(id, fade);
                for v in state.voices.iter_mut().filter(|v| v.instance == instance) {
                    v.queued = false;
                }
            }
            Admission::Fail | Admission::Queue => {}
        }
    }
    let ground = play.session.camera().ground_position();
    let listener = Listener::from_camera_ground(ground);
    let library = state.library.clone();
    let mut voices = std::mem::take(&mut state.voices);
    voices.retain_mut(|v| {
        if v.queued {
            return true;
        }
        v.age += dt;
        // Finished one-shots despawn themselves (PlaybackMode::Despawn).
        if let Some(e) = v.entity {
            if entities.get(e).is_err() {
                return false;
            }
        }
        let fade_gain = match v.fade.as_mut() {
            Some((remaining, total)) => {
                *remaining -= dt;
                if *remaining <= 0.0 || *total <= 0.0 {
                    despawn_voice(&mut commands, v);
                    return false;
                }
                *remaining / *total
            }
            None => 1.0,
        };
        let since = v.age - v.voice.delay;
        let distance = |p: [f32; 3]| {
            ((p[0] - ground[0]).powi(2) + (p[1] - ground[1]).powi(2) + (p[2] - ground[2]).powi(2))
                .sqrt()
        };
        let variables = |name: &str| -> Option<f32> {
            match name {
                "AttackTime" => Some((since.max(0.0) * 1000.0).min(15_000.0)),
                "ReleaseTime" => Some(0.0),
                "Volume" => Some(v.volume_db),
                "Magnitude" => v.magnitude,
                "Distance" => match v.source {
                    SoundSource::Emitter(_) | SoundSource::Locator { apply_3d: true, .. } => {
                        v.position.map(distance)
                    }
                    _ => Some(0.0),
                },
                "NormalizedDistance" => match v.source {
                    SoundSource::Locator { radius, .. } => v
                        .position
                        .map(|p| distance(p) / radius.max(f32::MIN_POSITIVE)),
                    _ => None,
                },
                _ => None,
            }
        };
        v.gain_db = library.gain_db(&v.voice, &variables);
        let gain = 10f32.powf(v.gain_db / 20.0)
            * fade_gain
            * if v.music { volumes.music } else { volumes.sfx };
        // Cue.Apply3D each frame: the listener-relative channel weights.
        let pan = v
            .spatial()
            .then(|| spatial::pan(&listener, v.position.unwrap_or(ground)));
        if let (Some(control), Some(pan)) = (&v.control, pan) {
            control.set_pan(pan);
        }
        match v.entity {
            None if since >= 0.0 => {
                let Some(handle) = state.clip(&v.voice, &mut clips) else {
                    return false;
                };
                let filter = v.voice.filter.as_ref().map(FilterParams::from_clip);
                let handle = if filter.is_some() || pan.is_some() {
                    let control =
                        Arc::new(VoiceControl::new(filter, pan.unwrap_or(StereoGain::UNITY)));
                    v.control = Some(control.clone());
                    let Some(base) = clips.get(&handle) else {
                        return false;
                    };
                    let clip = base.with_control(control);
                    clips.add(clip)
                } else {
                    handle
                };
                let speed =
                    2f32.powf((v.voice.pitch + library.rpc_pitch(&v.voice, &variables)) / 12.0);
                let settings = PlaybackSettings {
                    mode: PlaybackMode::Despawn,
                    volume: Volume::Linear(gain),
                    speed,
                    ..PlaybackSettings::DESPAWN
                };
                v.entity = Some(commands.spawn((AudioPlayer(handle), settings)).id());
                stats.0[2] += 1;
            }
            Some(e) => {
                if let Ok(mut sink) = sinks.get_mut(e) {
                    sink.set_volume(Volume::Linear(gain));
                }
            }
            None => {}
        }
        true
    });
    stats.0[0] = voices.iter().filter(|v| v.entity.is_some()).count();
    stats.0[1] = voices
        .iter()
        .filter(|v| v.music && v.entity.is_some())
        .count();
    state.voices = voices;
}

/// Share decoded bank storage with the loading host; no second bank copy.
pub(super) fn library(world: &World) -> Option<Arc<AudioLibrary>> {
    world
        .get_resource::<AudioState>()
        .map(|s| s.library.clone())
}
