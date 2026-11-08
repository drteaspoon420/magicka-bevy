//! AudioManager cue playback over the retail XACT banks: PlayCue(bank,
//! hash) resolves the cue to concrete wave voices (variation choice, clip
//! play-wave events, loop counts, volume/pitch randomisation). Mixing,
//! categories, RPC curves and DSP are presentation work; see
//! research/audio-xact.md.
use crate::scene_loading::custom_hash;
use magicka_content::xact::{
    ClipEvent, CueEntry, CueSounds, GlobalSettings, Pcm, Sound, SoundBank, SoundBody, WaveBank,
    WaveRef,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{
    io::Write,
    process::{Command, Stdio},
};

/// AudioManager.Init bank order: index = trailing zeros of the Banks flag.
/// (sound bank, wave bank) file stems; index 0 is the generic pair.
pub const BANK_FILES: [(&str, &str); 11] = [
    ("Sound Bank", "Wave Bank"),
    ("Music", "Music"),
    ("Ambience", "Ambience"),
    ("UI", "UI"),
    ("Spells", "Spells"),
    ("Characters", "Characters"),
    ("Footsteps", "Footsteps"),
    ("Weapons", "Weapons"),
    ("Misc", "Misc"),
    ("Additional", "Additional"),
    ("AdditionalMusic", "AdditionalMusic"),
];

/// Banks flag → AudioManager bank index.
pub fn bank_index(flag: u16) -> Option<usize> {
    (flag != 0)
        .then(|| flag.trailing_zeros() as usize)
        .filter(|&i| i < BANK_FILES.len())
}

/// One wave to play for a cue.
#[derive(Clone, Debug, PartialEq)]
pub struct Voice {
    /// Lowercased wave bank name.
    pub wave_bank: String,
    pub track: u16,
    /// Clip event time offset (seconds).
    pub delay: f32,
    /// 255 = loop forever.
    pub loop_count: u8,
    pub volume_db: f32,
    /// Semitones.
    pub pitch: f32,
    /// XGS category index of the sound.
    pub category: u16,
    /// Indices into the XGS RPC curves the sound references.
    pub rpcs: Vec<usize>,
    /// The clip's track filter (complex sounds only).
    pub filter: Option<magicka_content::xact::ClipFilter>,
}

fn find_ci(dir: &Path, stem: &str, ext: &str) -> Option<PathBuf> {
    let wanted = format!("{stem}.{ext}").to_ascii_lowercase();
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().to_ascii_lowercase() == wanted)
        })
}

struct LoadedWaveBank {
    bytes: Vec<u8>,
    bank: WaveBank,
}

/// Per-cue instance limiting as authored in the sound bank.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CueInstanceLimit {
    /// 255 = unlimited.
    pub limit: u8,
    pub behavior: magicka_content::xact::InstanceBehavior,
    /// Seconds.
    pub fade_in: f32,
    pub fade_out: f32,
}

#[derive(Default)]
pub struct AudioLibrary {
    pub globals: Option<GlobalSettings>,
    sound_banks: Vec<Option<SoundBank>>,
    cues: HashMap<(usize, i32), usize>,
    wave_banks: HashMap<String, LoadedWaveBank>,
    pub errors: Vec<String>,
}

impl AudioLibrary {
    /// Load every sound bank and wave bank under `<content>/Audio`.
    pub fn load(content: &Path) -> Self {
        let dir = content.join("Audio");
        let mut lib = Self::default();
        match find_ci(&dir, "magicka", "xgs")
            .ok_or_else(|| "magicka.xgs missing".to_string())
            .and_then(|p| std::fs::read(&p).map_err(|e| format!("{p:?}: {e}")))
            .and_then(|b| GlobalSettings::parse(&b).map_err(|e| format!("magicka.xgs: {e:?}")))
        {
            Ok(g) => lib.globals = Some(g),
            Err(e) => lib.errors.push(e),
        }
        for (i, (sound, wave)) in BANK_FILES.iter().enumerate() {
            let bank = find_ci(&dir, sound, "xsb")
                .ok_or_else(|| format!("{sound}.xsb missing"))
                .and_then(|p| std::fs::read(&p).map_err(|e| format!("{p:?}: {e}")))
                .and_then(|b| SoundBank::parse(&b).map_err(|e| format!("{sound}.xsb: {e:?}")));
            match bank {
                Ok(bank) => {
                    for (c, cue) in bank.cues.iter().enumerate() {
                        lib.cues
                            .insert((i, custom_hash(&cue.name.to_ascii_lowercase())), c);
                    }
                    lib.sound_banks.push(Some(bank));
                }
                Err(e) => {
                    lib.errors.push(e);
                    lib.sound_banks.push(None);
                }
            }
            let key = wave.to_ascii_lowercase();
            if lib.wave_banks.contains_key(&key) {
                continue;
            }
            match find_ci(&dir, wave, "xwb")
                .ok_or_else(|| format!("{wave}.xwb missing"))
                .and_then(|p| std::fs::read(&p).map_err(|e| format!("{p:?}: {e}")))
                .and_then(|bytes| {
                    WaveBank::parse(&bytes)
                        .map(|bank| LoadedWaveBank { bytes, bank })
                        .map_err(|e| format!("{wave}.xwb: {e:?}"))
                }) {
                Ok(w) => {
                    lib.wave_banks.insert(key, w);
                }
                Err(e) => lib.errors.push(e),
            }
        }
        // Sound banks also reference wave banks without their own sound-bank
        // slot, notably Loc for campaign dialog and character voice cues.
        let referenced: std::collections::HashSet<_> = lib
            .sound_banks
            .iter()
            .flatten()
            .flat_map(|bank| bank.wave_banks.iter().map(|name| name.to_ascii_lowercase()))
            .collect();
        for name in referenced {
            if lib.wave_banks.contains_key(&name) {
                continue;
            }
            let loaded = find_ci(&dir, &name, "xwb")
                .ok_or_else(|| format!("referenced wave bank {name}.xwb missing"))
                .and_then(|path| std::fs::read(&path).map_err(|e| format!("{path:?}: {e}")))
                .and_then(|bytes| {
                    WaveBank::parse(&bytes)
                        .map(|bank| LoadedWaveBank { bytes, bank })
                        .map_err(|e| format!("{name}.xwb: {e:?}"))
                });
            match loaded {
                Ok(bank) => {
                    lib.wave_banks.insert(name, bank);
                }
                Err(error) => lib.errors.push(error),
            }
        }
        lib
    }

    pub fn has_cue(&self, bank_flag: u16, cue: i32) -> bool {
        bank_index(bank_flag).is_some_and(|b| self.cues.contains_key(&(b, cue)))
    }

    /// The cue's XACT instance limit data (research/audio-xact.md, spatial
    /// audio): simple cues are unlimited.
    pub fn cue_instance_limit(&self, bank_flag: u16, cue: i32) -> Option<CueInstanceLimit> {
        let b = bank_index(bank_flag)?;
        let index = *self.cues.get(&(b, cue))?;
        let c = &self.sound_banks[b].as_ref()?.cues[index];
        Some(CueInstanceLimit {
            limit: c.instance_limit,
            behavior: c.instance_behavior(),
            fade_in: c.fade_in,
            fade_out: c.fade_out,
        })
    }

    /// The cue's name (for logs).
    pub fn cue_name(&self, bank_flag: u16, cue: i32) -> Option<&str> {
        let b = bank_index(bank_flag)?;
        let index = *self.cues.get(&(b, cue))?;
        Some(&self.sound_banks[b].as_ref()?.cues[index].name)
    }

    /// SoundBank.GetCue + Cue.Play: the voices one play produces. `random`
    /// returns uniform values in [0, 1) (XACT's own generator is not
    /// reproduced).
    pub fn resolve(
        &self,
        bank_flag: u16,
        cue: i32,
        random: &mut impl FnMut() -> f32,
    ) -> Option<Vec<Voice>> {
        let b = bank_index(bank_flag)?;
        let bank = self.sound_banks[b].as_ref()?;
        let cue = &bank.cues[*self.cues.get(&(b, cue))?];
        let mut voices = Vec::new();
        let wave = |w: WaveRef| {
            bank.wave_banks
                .get(w.bank as usize)
                .map(|n| n.to_ascii_lowercase())
        };
        let rpcs = |codes: &[u32]| -> Vec<usize> {
            let Some(g) = &self.globals else {
                return Vec::new();
            };
            codes
                .iter()
                .filter_map(|c| g.rpcs.iter().position(|r| r.code == *c))
                .collect()
        };
        match &cue.sounds {
            CueSounds::Single(sound) => sound_voices(sound, &wave, &rpcs, random, &mut voices),
            CueSounds::Variations { entries, .. } => {
                let total: f32 = entries.iter().map(|(_, w)| w.max(0.0)).sum();
                let mut pick = random() * total;
                let mut chosen = entries.first().map(|(e, _)| e);
                for (e, w) in entries {
                    if pick < *w {
                        chosen = Some(e);
                        break;
                    }
                    pick -= w;
                }
                match chosen? {
                    CueEntry::Sound(sound) => {
                        sound_voices(sound, &wave, &rpcs, random, &mut voices)
                    }
                    CueEntry::Wave(w) => voices.push(Voice {
                        wave_bank: wave(*w)?,
                        track: w.track,
                        delay: 0.0,
                        loop_count: 0,
                        volume_db: 0.0,
                        pitch: 0.0,
                        category: 1,
                        rpcs: Vec::new(),
                        filter: None,
                    }),
                }
            }
        }
        Some(voices)
    }

    /// The voice's current level in dB: its authored volume, its category
    /// chain, and every volume RPC evaluated at the variable values
    /// `variable(name)` supplies (XGS initial values otherwise). Point
    /// shapes other than linear are approximated linearly.
    pub fn gain_db(&self, voice: &Voice, variable: &impl Fn(&str) -> Option<f32>) -> f32 {
        let mut db = voice.volume_db;
        let Some(g) = &self.globals else {
            return db;
        };
        let mut category = voice.category as usize;
        let mut guard = 0;
        while let Some(c) = g.categories.get(category) {
            db += c.volume_db;
            category = c.parent as usize;
            guard += 1;
            if guard > 8 {
                break;
            }
        }
        for &i in &voice.rpcs {
            let r = &g.rpcs[i];
            if r.parameter != 0 {
                continue;
            }
            let Some(v) = g.variables.get(r.variable as usize) else {
                continue;
            };
            let x = variable(&v.name).unwrap_or(v.initial);
            db += r.evaluate(x) / 100.0;
        }
        db
    }

    /// Pitch RPCs (hundredths of a semitone) at the given variables.
    pub fn rpc_pitch(&self, voice: &Voice, variable: &impl Fn(&str) -> Option<f32>) -> f32 {
        let Some(g) = &self.globals else {
            return 0.0;
        };
        voice
            .rpcs
            .iter()
            .map(|&i| &g.rpcs[i])
            .filter(|r| r.parameter == 1)
            .filter_map(|r| {
                let v = g.variables.get(r.variable as usize)?;
                Some(r.evaluate(variable(&v.name).unwrap_or(v.initial)) / 100.0)
            })
            .sum()
    }

    /// Decode one wave. xWMA uses the local FFmpeg executable; PCM and
    /// MS-ADPCM remain pure Rust. The viewer caches the resulting PCM clip.
    pub fn decode(&self, wave_bank: &str, track: u16) -> Option<Pcm> {
        self.try_decode(wave_bank, track).ok()
    }

    /// Decode with diagnostics, including missing FFmpeg or codec failures.
    /// `MAGICKA_FFMPEG` can select an executable outside PATH.
    pub fn try_decode(&self, wave_bank: &str, track: u16) -> Result<Pcm, String> {
        let w = self
            .wave_banks
            .get(wave_bank)
            .ok_or_else(|| format!("wave bank {wave_bank} not loaded"))?;
        let entry = w
            .bank
            .entries
            .get(track as usize)
            .ok_or_else(|| format!("wave bank {wave_bank} has no track {track}"))?;
        if entry.format.codec != magicka_content::xact::WaveCodec::Wma {
            return w
                .bank
                .decode(&w.bytes, track as usize)
                .map_err(|e| format!("{e:?}"));
        }
        let input = w
            .bank
            .xwma(&w.bytes, track as usize)
            .map_err(|e| format!("{e:?}"))?;
        let executable = std::env::var_os("MAGICKA_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
        let mut command = Command::new(&executable);
        // Nix store binaries resolve libraries by RPATH; a dev shell's
        // LD_LIBRARY_PATH can shadow them with libraries needing a newer glibc.
        if Path::new(&executable).starts_with("/nix/store") {
            command.env_remove("LD_LIBRARY_PATH");
        }
        let mut child = command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-threads",
                "1",
                "-f",
                "xwma",
                "-i",
                "pipe:0",
                "-f",
                "s16le",
                "-acodec",
                "pcm_s16le",
                "-threads",
                "1",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!("cannot start {executable:?} for xWMA voices: {e}; install FFmpeg or set MAGICKA_FFMPEG to its executable")
            })?;
        let mut stdin = child.stdin.take().ok_or("FFmpeg stdin unavailable")?;
        // Read output while feeding input: either pipe can exceed its OS
        // buffer for long dialog lines. Closing stdin signals end of stream.
        let output = std::thread::scope(|scope| {
            let writer = scope.spawn(move || stdin.write_all(&input));
            let output = child.wait_with_output();
            let written = writer.join().map_err(|_| "FFmpeg input writer panicked")?;
            let output = output.map_err(|e| format!("FFmpeg output: {e}"))?;
            if !output.status.success() {
                return Err(format!(
                    "FFmpeg exited {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            written.map_err(|e| format!("FFmpeg input: {e}"))?;
            Ok(output)
        })?;
        if output.stdout.is_empty() || output.stdout.len() % 2 != 0 {
            return Err("FFmpeg returned empty or incomplete PCM16 samples".into());
        }
        Ok(Pcm {
            channels: entry.format.channels,
            sample_rate: entry.format.sample_rate,
            samples: output
                .stdout
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&b| i16::from_le_bytes(b))
                .collect(),
        })
    }
}

fn lerp(range: (f32, f32), t: f32) -> f32 {
    range.0 + (range.1 - range.0) * t
}

fn sound_voices(
    sound: &Sound,
    wave: &impl Fn(WaveRef) -> Option<String>,
    rpcs: &impl Fn(&[u32]) -> Vec<usize>,
    random: &mut impl FnMut() -> f32,
    out: &mut Vec<Voice>,
) {
    let curves = rpcs(&sound.rpc_codes);
    match &sound.body {
        SoundBody::Simple(w) => {
            if let Some(bank) = wave(*w) {
                out.push(Voice {
                    wave_bank: bank,
                    track: w.track,
                    delay: 0.0,
                    loop_count: 0,
                    volume_db: sound.volume_db,
                    pitch: sound.pitch,
                    category: sound.category,
                    rpcs: curves.clone(),
                    filter: None,
                });
            }
        }
        SoundBody::Complex(clips) => {
            for clip in clips {
                for event in &clip.events {
                    let ClipEvent::PlayWave {
                        time,
                        random_offset,
                        loop_count,
                        waves,
                        pitch,
                        volume,
                        ..
                    } = event
                    else {
                        continue;
                    };
                    let total: f32 = waves.iter().map(|v| v.weight.max(0.0)).sum();
                    let mut pick = random() * total.max(f32::MIN_POSITIVE);
                    let mut chosen = waves.first();
                    for v in waves {
                        if pick < v.weight {
                            chosen = Some(v);
                            break;
                        }
                        pick -= v.weight;
                    }
                    let Some(v) = chosen else { continue };
                    let Some(bank) = wave(v.wave) else { continue };
                    out.push(Voice {
                        wave_bank: bank,
                        track: v.wave.track,
                        delay: time + random() * random_offset,
                        loop_count: *loop_count,
                        volume_db: sound.volume_db
                            + clip.volume_db
                            + volume.map_or(0.0, |r| lerp(r, random())),
                        pitch: sound.pitch + pitch.map_or(0.0, |r| lerp(r, random())),
                        category: sound.category,
                        rpcs: curves.clone(),
                        filter: clip.filter,
                    });
                }
            }
        }
    }
}

/// Where a started sound plays from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SoundSource {
    /// No emitter (AudioManager.PlayCue without 3D, GlobalAudio).
    Global,
    /// GameScene AudioLocator: the locator/area id, the NormalizedDistance
    /// radius, and whether Apply3D runs.
    Locator {
        area: i32,
        radius: f32,
        apply_3d: bool,
    },
    /// Apply3D with an emitter at this position.
    Emitter([f32; 3]),
}

/// Audio work requested by the simulation, consumed by the presentation.
#[derive(Clone, Debug, PartialEq)]
pub enum AudioRequest {
    /// Cue.Play. `volume_db` is the cue's "Volume" variable.
    Start {
        key: u32,
        bank: u16,
        cue: i32,
        volume_db: f32,
        source: SoundSource,
        /// SpellSoundVariables "Magnitude", when assigned.
        magnitude: Option<f32>,
    },
    /// AudioEmitter moved (Apply3D with the owner's new position).
    Move {
        key: u32,
        position: [f32; 3],
    },
    /// GlobalAudio replay: SetVolume, then Play (no-op while playing).
    Replay {
        key: u32,
        volume_db: f32,
    },
    Stop {
        key: u32,
        instant: bool,
    },
    /// AudioManager.PlayMusic (replaces the current music).
    Music {
        bank: u16,
        cue: i32,
    },
    StopMusic,
}

/// Banks flag values (Magicka.Audio.Banks).
pub mod banks {
    pub const WAVE_BANK: u16 = 1;
    pub const MUSIC: u16 = 2;
    pub const AMBIENCE: u16 = 4;
    pub const UI: u16 = 8;
    pub const SPELLS: u16 = 0x10;
    pub const CHARACTERS: u16 = 0x20;
    pub const FOOTSTEPS: u16 = 0x40;
    pub const WEAPONS: u16 = 0x80;
    pub const MISC: u16 = 0x100;
    pub const ADDITIONAL: u16 = 0x200;
}

/// Defines.SOUNDS_AREA by element index.
pub const SOUNDS_AREA: [&str; 11] = [
    "spell_earth_area",
    "spell_water_area",
    "spell_cold_area",
    "spell_fire_area",
    "spell_lightning_area",
    "spell_arcane_area",
    "spell_life_area",
    "spell_shield_area",
    "spell_ice_area",
    "spell_steam_area",
    "spell_poison_area",
];
