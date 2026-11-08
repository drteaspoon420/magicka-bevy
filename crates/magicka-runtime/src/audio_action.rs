//! PlaySound/StopSound/PlayMusic/StopMusic trigger actions and GameScene's
//! ambient-sound registry. Cue playback itself is a backend responsibility.
use crate::{
    ai_event::clr_enum,
    dialog_enums::BANKS,
    scene_loading::custom_hash,
    script_attributes::{boolean, identifier, number, ScriptReadError},
};
use std::collections::BTreeMap;

pub const BANK_MUSIC: u16 = 2;
pub const BANK_AMBIENCE: u16 = 4;

fn bank(value: &str, key: &str) -> Result<u16, ScriptReadError> {
    clr_enum(value, BANKS, key, u16::MAX as i32).map(|v| v as u16)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaySound {
    /// `None` when ID is null or empty.
    pub id: Option<i32>,
    pub bank: u16,
    pub cue: i32,
    /// `None` when Area is null or empty.
    pub area: Option<i32>,
    pub volume: f32,
    pub radius: f32,
    pub apply_3d: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AudioActionKind {
    PlaySound(PlaySound),
    StopSound {
        id: i32,
        instant: bool,
    },
    PlayMusic {
        bank: u16,
        cue: i32,
        focus: Option<f32>,
    },
    StopMusic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioAction {
    pub delay: f32,
    pub kind: AudioActionKind,
}

pub const TAGS: &[&str] = &["playsound", "stopsound", "playmusic", "stopmusic"];

/// Located AudioLocator parameters (cue, volume, locator area, radius, 3D).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocatedSound {
    pub bank: u16,
    pub cue: i32,
    pub volume: f32,
    pub area: i32,
    pub radius: f32,
    pub apply_3d: bool,
}

pub trait AudioActionHost {
    type Error;
    /// GameScene.PlayAmbientSound(id, bank, cue, volume).
    fn play_global_ambient(
        &mut self,
        id: i32,
        bank: u16,
        cue: i32,
        volume: f32,
    ) -> Result<(), Self::Error>;
    /// GameScene.PlayAmbientSound(id, ..., area, radius, apply3D).
    fn play_located_ambient(&mut self, id: i32, sound: LocatedSound) -> Result<(), Self::Error>;
    /// AudioManager.GetCue, SetVariable("Volume"), Play. `false` when GetCue
    /// returned null (the original then throws NullReferenceException).
    fn play_cue(&mut self, bank: u16, cue: i32, volume: f32) -> Result<bool, Self::Error>;
    /// new AudioLocator(0, ...), Play, Update(scene); not registered.
    fn play_unregistered_locator(&mut self, sound: LocatedSound) -> Result<(), Self::Error>;
    fn stop_sound(&mut self, id: i32, instant: bool) -> Result<(), Self::Error>;
    /// PlayState.PlayMusic(bank, cue, focus).
    fn play_music(&mut self, bank: u16, cue: i32, focus: Option<f32>) -> Result<(), Self::Error>;
    /// AudioManager.StopMusic.
    fn stop_music(&mut self) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum AudioActionError<E> {
    /// GetCue returned null for an unregistered cue hash.
    MissingCue(i32),
    Host(E),
}
impl<E> From<E> for AudioActionError<E> {
    fn from(e: E) -> Self {
        Self::Host(e)
    }
}

impl AudioAction {
    pub fn read<'a>(
        tag: &str,
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        use AudioActionKind::*;
        let mut kind = match tag.to_ascii_lowercase().as_str() {
            "playsound" => PlaySound(self::PlaySound {
                id: None,
                // Fields default to zero until the Cue setter runs.
                bank: 0,
                cue: 0,
                area: None,
                volume: 0.0,
                radius: 0.0,
                apply_3d: false,
            }),
            "stopsound" => StopSound {
                id: 0,
                instant: false,
            },
            "playmusic" => PlayMusic {
                bank: BANK_MUSIC,
                cue: 0,
                focus: None,
            },
            "stopmusic" => StopMusic,
            _ => return Err(ScriptReadError::UnsupportedTag(tag.into())),
        };
        let mut delay = 0.0;
        for (key, value) in attrs {
            let lower = key.to_ascii_lowercase();
            if lower == "delay" {
                delay = number(value, key)?;
                continue;
            }
            let lowered = value.to_ascii_lowercase();
            match (&mut kind, lower.as_str()) {
                (PlaySound(s), "id") => {
                    s.id = (!value.is_empty()).then(|| identifier(value)).transpose()?
                }
                (PlaySound(s), "cue") => {
                    let parts: Vec<_> = lowered.split('/').collect();
                    let cue = match parts.as_slice() {
                        [cue] => {
                            s.bank = BANK_AMBIENCE;
                            *cue
                        }
                        [bank_name, cue] => {
                            s.bank = bank(bank_name, key)?;
                            *cue
                        }
                        _ => return Err(ScriptReadError::InvalidAttribute(key.into())),
                    };
                    s.cue = identifier(cue)?;
                }
                (PlaySound(s), "area") => {
                    s.area = (!value.is_empty()).then(|| identifier(value)).transpose()?
                }
                (PlaySound(s), "volume") => s.volume = number(value, key)?,
                (PlaySound(s), "radius") => s.radius = number(value, key)?,
                (PlaySound(s), "apply3d") => s.apply_3d = boolean(value, key)?,
                (StopSound { id, .. }, "id") => *id = identifier(value)?,
                (StopSound { instant, .. }, "instant") => *instant = boolean(value, key)?,
                (PlayMusic { bank: b, cue, .. }, "cue") => {
                    let parts: Vec<_> = lowered.split('/').collect();
                    if parts.len() > 1 {
                        *b = bank(parts[0], key)?;
                        *cue = identifier(parts[1])?;
                    } else {
                        *cue = identifier(&lowered)?;
                    }
                }
                (PlayMusic { focus, .. }, "focusvalue") => *focus = Some(number(value, key)?),
                _ => {}
            }
        }
        Ok(Self { delay, kind })
    }

    pub fn execute<H: AudioActionHost + ?Sized>(
        &self,
        host: &mut H,
    ) -> Result<(), AudioActionError<H::Error>> {
        match self.kind {
            AudioActionKind::PlaySound(s) => {
                let located = |area| LocatedSound {
                    bank: s.bank,
                    cue: s.cue,
                    volume: s.volume,
                    area,
                    radius: s.radius,
                    apply_3d: s.apply_3d,
                };
                match (s.id, s.area) {
                    (Some(id), None) => host.play_global_ambient(id, s.bank, s.cue, s.volume)?,
                    (Some(id), Some(area)) => host.play_located_ambient(id, located(area))?,
                    (None, None) => {
                        if !host.play_cue(s.bank, s.cue, s.volume)? {
                            return Err(AudioActionError::MissingCue(s.cue));
                        }
                    }
                    (None, Some(area)) => host.play_unregistered_locator(located(area))?,
                }
            }
            AudioActionKind::StopSound { id, instant } => host.stop_sound(id, instant)?,
            AudioActionKind::PlayMusic { bank, cue, focus } => host.play_music(bank, cue, focus)?,
            AudioActionKind::StopMusic => host.stop_music()?,
        }
        Ok(())
    }
}

/// Cue-level operations behind GameScene's sound dictionaries.
pub trait AmbientBackend {
    type Error;
    type Located;
    type Global;
    /// new AudioLocator(id, ...) followed by Play.
    fn start_located(&mut self, id: i32, sound: LocatedSound)
        -> Result<Self::Located, Self::Error>;
    /// new GlobalAudio(bank, cue, volume) followed by Play.
    fn start_global(
        &mut self,
        bank: u16,
        cue: i32,
        volume: f32,
    ) -> Result<Self::Global, Self::Error>;
    /// Existing GlobalAudio: SetVolume then Play.
    fn replay_global(&mut self, sound: &mut Self::Global, volume: f32) -> Result<(), Self::Error>;
    fn stop_located(&mut self, sound: &mut Self::Located, instant: bool)
        -> Result<(), Self::Error>;
    fn stop_global(&mut self, sound: &mut Self::Global, instant: bool) -> Result<(), Self::Error>;
}

/// GameScene mSounds/mGlobalSounds bookkeeping.
pub struct AmbientSounds<L, G> {
    pub located: BTreeMap<i32, L>,
    pub global: BTreeMap<i32, G>,
}
impl<L, G> Default for AmbientSounds<L, G> {
    fn default() -> Self {
        Self {
            located: BTreeMap::new(),
            global: BTreeMap::new(),
        }
    }
}
impl<L, G> AmbientSounds<L, G> {
    /// Located: an existing ID is left untouched (no replay, no replacement).
    pub fn play_located<B: AmbientBackend<Located = L, Global = G>>(
        &mut self,
        backend: &mut B,
        id: i32,
        sound: LocatedSound,
    ) -> Result<(), B::Error> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.located.entry(id) {
            entry.insert(backend.start_located(id, sound)?);
        }
        Ok(())
    }
    /// Global: an existing ID keeps its original cue and bank and replays at
    /// the new volume.
    pub fn play_global<B: AmbientBackend<Located = L, Global = G>>(
        &mut self,
        backend: &mut B,
        id: i32,
        bank: u16,
        cue: i32,
        volume: f32,
    ) -> Result<(), B::Error> {
        if let Some(sound) = self.global.get_mut(&id) {
            backend.replay_global(sound, volume)
        } else {
            let handle = backend.start_global(bank, cue, volume)?;
            self.global.insert(id, handle);
            Ok(())
        }
    }
    /// Located sounds are stopped and removed; otherwise a global sound is
    /// stopped but kept, so a later PlaySound with that ID replays it.
    pub fn stop<B: AmbientBackend<Located = L, Global = G>>(
        &mut self,
        backend: &mut B,
        id: i32,
        instant: bool,
    ) -> Result<(), B::Error> {
        if let Some(mut sound) = self.located.remove(&id) {
            if let Err(e) = backend.stop_located(&mut sound, instant) {
                // Stop precedes Remove natively; a throwing Stop keeps the entry.
                self.located.insert(id, sound);
                return Err(e);
            }
        } else if let Some(sound) = self.global.get_mut(&id) {
            backend.stop_global(sound, instant)?;
        }
        Ok(())
    }
}

/// SoundList-backed cue registry used by AudioManager.GetCue/PlayMusic:
/// keys are custom hashes of lowercased names. Duplicate names throw natively.
pub fn sound_dictionary<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeMap<i32, String>, String> {
    let mut out = BTreeMap::new();
    for name in names {
        if !name.is_ascii() {
            return Err(name.into());
        }
        if out
            .insert(custom_hash(&name.to_ascii_lowercase()), name.to_string())
            .is_some()
        {
            return Err(name.into());
        }
    }
    Ok(out)
}
