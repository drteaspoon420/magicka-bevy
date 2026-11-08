//! Gameplay-visible XNAnimation AnimationController state (clip time, speed,
//! crossfade queue, HasFinished, Looped/CrossfadeFinished events) and
//! Character.GoToAnimation clip selection including weapon animation sets.
//! Bone poses are sampled by `character_pose`; see research/avatar-events.md
//! and research/character-casting.md.
use crate::dialog_enums::ANIMATIONS;
use magicka_content::character::AnimationClipAction;
use std::collections::HashMap;

pub const ANIMATION_NONE: u16 = 0;
pub const ANIMATION_IDLE: u16 = 1;

/// Animations enum value for a (case-insensitive) member name.
pub fn animation_value(name: &str) -> Option<u16> {
    let lower = name.to_ascii_lowercase();
    ANIMATIONS
        .iter()
        .find(|(n, _)| *n == lower)
        .map(|&(_, v)| v as u16)
}
/// First Animations member name with this value (aliases share values).
pub fn animation_name(value: u16) -> &'static str {
    ANIMATIONS
        .iter()
        .find(|&&(_, v)| v == i32::from(value))
        .map_or("?", |&(n, _)| n)
}

fn keyed(set: &[AnimationClipAction]) -> HashMap<u16, AnimationClipAction> {
    set.iter()
        .filter_map(|a| animation_value(&a.animation).map(|v| (v, a.clone())))
        .collect()
}

/// WeaponClass.Staff: never selects an animation set.
pub const WEAPON_CLASS_STAFF: u8 = 18;

/// A CharacterTemplate's animation sets keyed by Animations value: the
/// Default set plus the sets of the equipped non-staff weapons (equipment
/// order), with clip durations from the skeleton model.
#[derive(Clone, Debug, Default)]
pub struct AnimationSet {
    pub actions: HashMap<u16, AnimationClipAction>,
    pub weapon_sets: Vec<HashMap<u16, AnimationClipAction>>,
    pub durations: HashMap<String, f32>,
}
impl AnimationSet {
    pub fn new(set: &[AnimationClipAction], durations: HashMap<String, f32>) -> Self {
        Self {
            actions: keyed(set),
            weapon_sets: Vec::new(),
            durations,
        }
    }
    /// `sets` are the template's 27 AnimationClips arrays; `weapon_classes`
    /// the WeaponClass of each equipment slot in order.
    pub fn with_weapons(
        sets: &[Vec<AnimationClipAction>],
        weapon_classes: &[u8],
        durations: HashMap<String, f32>,
    ) -> Self {
        let mut set = Self::new(sets.first().map_or(&[][..], Vec::as_slice), durations);
        set.weapon_sets = weapon_classes
            .iter()
            .filter(|&&c| c != WEAPON_CLASS_STAFF)
            .filter_map(|&c| sets.get(usize::from(c)))
            .map(|s| keyed(s))
            .collect();
        set
    }
    /// GoToAnimation/ForceAnimation selection: the first equipped non-staff
    /// weapon whose set defines the animation, else the Default set.
    pub fn lookup(&self, animation: u16) -> Option<&AnimationClipAction> {
        self.weapon_sets
            .iter()
            .find_map(|s| s.get(&animation))
            .or_else(|| self.actions.get(&animation))
    }
}

/// AnimationController events in the order they were raised during Update
/// (Character subscribes OnAnimationLooped / OnCrossfadeFinished).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControllerSignal {
    Looped,
    CrossfadeFinished,
}

/// What UpdateChannelPoses reads: the playing clip at `time`, and while
/// fading the target clip's first key blended by `crossfade_amount`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoseSample<'a> {
    pub clip: &'a str,
    pub duration: f32,
    pub time: f32,
    pub looping: bool,
    pub crossfade: Option<&'a str>,
    pub crossfade_amount: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct Clip {
    name: String,
    duration: f32,
}

/// Presentation request emitted by CrossFade/StartClip.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipCue {
    pub clip: String,
    pub fade: f32,
    pub looping: bool,
}

#[derive(Clone, Debug)]
pub struct AnimationController {
    clip: Option<Clip>,
    pub time: f32,
    clip_speed: f32,
    pub speed: f32,
    looping: bool,
    playing: bool,
    finished: bool,
    crossfade: Option<(Clip, bool)>,
    crossfade_enabled: bool,
    crossfade_time: f32,
    crossfade_duration: f32,
    queue: Option<(Clip, f32)>,
    /// crossFadeInterpolationAmount (only UpdateCrossFadeTime writes it).
    crossfade_amount: f32,
    /// Character.mCurrentAnimation (Animations value).
    pub current_animation: u16,
    pub cues: Vec<ClipCue>,
    /// Events raised since the owner last drained them.
    pub signals: Vec<ControllerSignal>,
}

impl Default for AnimationController {
    fn default() -> Self {
        Self {
            clip: None,
            time: 0.0,
            clip_speed: 1.0,
            speed: 1.0,
            looping: false,
            playing: false,
            finished: false,
            crossfade: None,
            crossfade_enabled: false,
            crossfade_time: 0.0,
            crossfade_duration: 0.0,
            queue: None,
            crossfade_amount: 0.0,
            current_animation: ANIMATION_NONE,
            cues: Vec::new(),
            signals: Vec::new(),
        }
    }
}

impl AnimationController {
    /// XNAnimation Stop pauses clip time; a queued crossfade still advances.
    pub fn stop(&mut self) {
        self.playing = false;
    }
    /// Dedicated animated entities use clip names directly rather than the
    /// CharacterTemplate Animations table (e.g. SummonDeath.MagickDeath).
    pub fn start_named_clip(&mut self, name: &str, duration: f32, looping: bool) {
        self.queue = None;
        self.set_clip_speed(1.0);
        self.start_clip(
            Clip {
                name: name.into(),
                duration,
            },
            looping,
        );
    }

    pub fn crossfade_named_clip(&mut self, name: &str, duration: f32, fade: f32, looping: bool) {
        self.set_clip_speed(1.0);
        self.cross_fade(
            Clip {
                name: name.into(),
                duration,
            },
            fade,
            looping,
        );
    }

    pub fn has_finished(&self) -> bool {
        self.finished
    }
    pub fn crossfade_enabled(&self) -> bool {
        self.crossfade_enabled
    }
    pub fn is_looping(&self) -> bool {
        self.looping
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }
    /// AnimationClip.Duration of the playing clip (0 without a clip).
    pub fn clip_duration(&self) -> f32 {
        self.clip.as_ref().map_or(0.0, |c| c.duration)
    }
    pub fn pose_sample(&self) -> Option<PoseSample<'_>> {
        let clip = self.clip.as_ref()?;
        Some(PoseSample {
            clip: &clip.name,
            duration: clip.duration,
            time: self.time,
            looping: self.looping,
            crossfade: self
                .crossfade_enabled
                .then(|| self.crossfade.as_ref().map(|(c, _)| c.name.as_str()))
                .flatten(),
            crossfade_amount: self.crossfade_amount,
        })
    }
    pub fn clip_speed(&self) -> f32 {
        self.clip_speed
    }
    /// ClipSpeed setter clamps negatives to zero.
    fn set_clip_speed(&mut self, value: f32) {
        self.clip_speed = if value < 0.0 { 0.0 } else { value };
    }
    pub fn clip_name(&self) -> Option<&str> {
        self.clip.as_ref().map(|c| c.name.as_str())
    }
    fn lookup(set: &AnimationSet, name: &str) -> Clip {
        Clip {
            name: name.to_string(),
            duration: set.durations.get(name).copied().unwrap_or(0.0),
        }
    }
    /// StartClip (forward playback).
    fn start_clip(&mut self, clip: Clip, looping: bool) {
        self.cues.push(ClipCue {
            clip: clip.name.clone(),
            fade: 0.0,
            looping,
        });
        self.clip = Some(clip);
        self.finished = false;
        self.playing = true;
        self.looping = looping;
        self.crossfade_enabled = false;
        self.time = 0.0;
    }
    /// CrossFade(clip, fadeTime, loop).
    fn cross_fade(&mut self, clip: Clip, fade: f32, looping: bool) {
        self.playing = true;
        if self.crossfade_enabled {
            // Queue one clip; the queued loop flag overwrites the active one.
            self.crossfade = self.crossfade.take().map(|(c, _)| (c, looping));
            self.queue = Some((clip, fade));
            self.crossfade_duration =
                self.crossfade_time + (self.crossfade_duration - self.crossfade_time) * 0.5;
            return;
        }
        self.cues.push(ClipCue {
            clip: clip.name.clone(),
            fade,
            looping,
        });
        self.crossfade = Some((clip, looping));
        self.crossfade_duration = fade;
        self.crossfade_time = 0.0;
        self.crossfade_enabled = true;
        self.finished = false;
    }

    /// Character.Initialize: ClearCrossfadeQueue + StartClip of `animation`.
    pub fn start(&mut self, set: &AnimationSet, animation: u16) {
        self.queue = None;
        if let Some(action) = set.lookup(animation) {
            self.set_clip_speed(action.speed);
            let clip = Self::lookup(set, &action.clip);
            self.start_clip(clip, action.looping);
            self.current_animation = animation;
        }
    }

    /// Character.ForceAnimation (Default weapon set only): StartClip without
    /// a crossfade. The original dereferences the missing clip action after
    /// falling back to idle (NullReferenceException); this plays idle instead.
    pub fn force_animation(&mut self, set: &AnimationSet, animation: u16) {
        self.queue = None;
        let (animation, action) = match set.lookup(animation) {
            Some(a) => (animation, a),
            None => match set.lookup(ANIMATION_IDLE) {
                Some(a) => (ANIMATION_IDLE, a),
                None => return,
            },
        };
        self.current_animation = animation;
        self.set_clip_speed(action.speed);
        let clip = Self::lookup(set, &action.clip);
        self.start_clip(clip, action.looping);
    }

    /// Character.GoToAnimation (Default weapon set only).
    pub fn go_to_animation(&mut self, set: &AnimationSet, animation: u16, time: f32) {
        if self.current_animation == animation && !self.finished {
            return;
        }
        self.current_animation = animation;
        match set.lookup(animation) {
            Some(action) => {
                self.set_clip_speed(action.speed);
                let fade = if action.blend_time > 0.0 {
                    action.blend_time
                } else {
                    time
                };
                let clip = Self::lookup(set, &action.clip);
                self.cross_fade(clip, fade, action.looping);
            }
            None => {
                // Missing animation: idle crossfaded *without* looping.
                self.current_animation = ANIMATION_IDLE;
                if let Some(action) = set.lookup(ANIMATION_IDLE) {
                    self.set_clip_speed(action.speed);
                    let fade = if action.blend_time > 0.0 {
                        action.blend_time
                    } else {
                        time
                    };
                    let clip = Self::lookup(set, &action.clip);
                    self.cross_fade(clip, fade, false);
                }
            }
        }
    }

    /// AnimationController.Update time phases (elapsed × Speed × ClipSpeed).
    pub fn update(&mut self, dt: f32) {
        let Some(duration) = self.clip.as_ref().map(|c| c.duration) else {
            return;
        };
        let elapsed = dt * (self.speed * self.clip_speed);
        if self.playing {
            self.time += elapsed;
            if self.time < 0.0 || self.time > duration {
                if self.looping {
                    self.signals.push(ControllerSignal::Looped);
                    if duration > 0.0 {
                        while self.time > duration {
                            self.time -= duration;
                        }
                        while self.time < 0.0 {
                            self.time += duration;
                        }
                    } else {
                        self.time = 0.0;
                    }
                } else {
                    self.time = if self.time > duration { duration } else { 0.0 };
                    if !self.crossfade_enabled {
                        self.playing = false;
                        self.finished = true;
                    }
                }
            }
        }
        if self.crossfade_enabled {
            self.crossfade_time += elapsed;
            if self.crossfade_time > self.crossfade_duration {
                self.crossfade_enabled = false;
                self.crossfade_amount = 0.0;
                self.crossfade_duration = 0.0;
                self.crossfade_time = 0.0;
                // CrossfadeFinished is raised before StartClip of the target.
                self.signals.push(ControllerSignal::CrossfadeFinished);
                if let Some((clip, looping)) = self.crossfade.take() {
                    // The presentation already blended toward this clip.
                    self.clip = Some(clip);
                    self.finished = false;
                    self.playing = true;
                    self.looping = looping;
                    self.time = 0.0;
                }
                if let Some((clip, fade)) = self.queue.take() {
                    let looping = self.looping;
                    self.cross_fade(clip, fade, looping);
                }
            } else if self.crossfade_duration > f32::from_bits(1) {
                self.crossfade_amount = self.crossfade_time / self.crossfade_duration;
            } else {
                self.crossfade_amount = 1.0;
            }
        }
    }
}
