//! Retail XACT wave banks: every entry parses; PCM and MS-ADPCM entries
//! decode to their recorded duration (research/audio-xact.md).
use magicka_content::xact::{WaveBank, WaveCodec};
use std::path::{Path, PathBuf};

fn audio() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        })
        .join("Audio");
    root.is_dir().then_some(root)
}

#[test]
fn retail_wave_banks_decode_to_their_durations() {
    let Some(dir) = audio() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut banks = 0;
    let (mut decoded, mut wma, mut loud) = (0, 0, 0);
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("xwb") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let bank = WaveBank::parse(&bytes).unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
        banks += 1;
        for (i, e) in bank.entries.iter().enumerate() {
            if e.format.codec == WaveCodec::Wma {
                wma += 1;
                continue;
            }
            let pcm = bank
                .decode(&bytes, i)
                .unwrap_or_else(|err| panic!("{} #{i}: {err:?}", bank.name));
            let frames = pcm.frames() as i64;
            let duration = e.duration as i64;
            let slack = match e.format.codec {
                // A final partial block may carry padding nibbles.
                WaveCodec::Adpcm => e.format.adpcm_block_bytes() as i64 * 2,
                _ => 0,
            };
            assert!(
                frames >= duration && frames - duration <= slack,
                "{} #{i}: {frames} frames for duration {duration}",
                bank.name
            );
            if pcm.samples.iter().any(|&s| s.unsigned_abs() > 1000) {
                loud += 1;
            }
            decoded += 1;
        }
    }
    eprintln!("{banks} banks, {decoded} decoded, {wma} xWMA, {loud} audible");
    assert_eq!(banks, 12);
    assert!(decoded > 2000 && loud * 10 > decoded * 9);
}

fn wave_refs(
    sound: &magicka_content::xact::Sound,
    out: &mut Vec<magicka_content::xact::WaveRef>,
    other: &mut Vec<u8>,
) {
    use magicka_content::xact::{ClipEvent, SoundBody};
    match &sound.body {
        SoundBody::Simple(w) => out.push(*w),
        SoundBody::Complex(clips) => {
            for clip in clips {
                for e in &clip.events {
                    match e {
                        ClipEvent::PlayWave { waves, .. } => {
                            out.extend(waves.iter().map(|v| v.wave))
                        }
                        ClipEvent::Other { id, .. } => other.push(*id),
                    }
                }
            }
        }
    }
}

#[test]
fn retail_sound_banks_resolve_every_wave() {
    use magicka_content::xact::{CueEntry, CueSounds, SoundBank};
    use std::collections::HashMap;
    let Some(dir) = audio() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut wave_counts = HashMap::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("xwb") {
            let bank = WaveBank::parse(&std::fs::read(&path).unwrap()).unwrap();
            wave_counts.insert(bank.name.to_ascii_lowercase(), bank.entries.len());
        }
    }
    let (mut cues, mut refs, mut others) = (0, 0, Vec::new());
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("xsb") {
            continue;
        }
        let bank = SoundBank::parse(&std::fs::read(&path).unwrap())
            .unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
        for cue in &bank.cues {
            cues += 1;
            let mut waves = Vec::new();
            match &cue.sounds {
                CueSounds::Single(s) => wave_refs(s, &mut waves, &mut others),
                CueSounds::Variations { entries, .. } => {
                    for (e, _) in entries {
                        match e {
                            CueEntry::Sound(s) => wave_refs(s, &mut waves, &mut others),
                            CueEntry::Wave(w) => waves.push(*w),
                        }
                    }
                }
            }
            assert!(
                !waves.is_empty(),
                "{}:{} plays nothing",
                bank.name,
                cue.name
            );
            for w in waves {
                let name = bank
                    .wave_banks
                    .get(w.bank as usize)
                    .unwrap_or_else(|| panic!("{}:{} bank {}", bank.name, cue.name, w.bank));
                let count = wave_counts[&name.to_ascii_lowercase()];
                assert!(
                    (w.track as usize) < count,
                    "{}:{} track {} of {name} ({count})",
                    bank.name,
                    cue.name,
                    w.track
                );
                refs += 1;
            }
        }
    }
    eprintln!("{cues} cues, {refs} wave references, other events {others:?}");
    assert!(cues > 1000);
}

#[test]
fn retail_global_settings_parse_and_sounds_reference_known_curves() {
    use magicka_content::xact::{CueEntry, CueSounds, GlobalSettings, SoundBank};
    let Some(dir) = audio() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let g = GlobalSettings::parse(&std::fs::read(dir.join("magicka.xgs")).unwrap()).unwrap();
    for c in &g.categories {
        eprintln!(
            "category {} vol {:.1} dB parent {}",
            c.name, c.volume_db, c.parent
        );
    }
    let names: Vec<_> = g.variables.iter().map(|v| v.name.as_str()).collect();
    eprintln!("variables {names:?}");
    assert!(names.contains(&"NormalizedDistance") && names.contains(&"Volume"));
    let codes: std::collections::HashSet<u32> = g.rpcs.iter().map(|r| r.code).collect();
    let mut referenced = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("xsb") {
            continue;
        }
        let bank = SoundBank::parse(&std::fs::read(&path).unwrap()).unwrap();
        let mut sounds = Vec::new();
        for cue in &bank.cues {
            match &cue.sounds {
                CueSounds::Single(s) => sounds.push(s.clone()),
                CueSounds::Variations { entries, .. } => {
                    for (e, _) in entries {
                        if let CueEntry::Sound(s) = e {
                            sounds.push(s.clone());
                        }
                    }
                }
            }
        }
        for s in sounds {
            for code in &s.rpc_codes {
                assert!(codes.contains(code), "{}: rpc code {code}", bank.name);
                *referenced.entry(*code).or_insert(0) += 1;
            }
            assert!(
                (s.category as usize) < g.categories.len(),
                "{}: category {}",
                bank.name,
                s.category
            );
        }
    }
    for r in g
        .rpcs
        .iter()
        .filter(|r| referenced.contains_key(&r.code))
        .take(12)
    {
        eprintln!(
            "rpc {} var {} param {} points {:?} x{}",
            r.code,
            g.variables[r.variable as usize].name,
            r.parameter,
            r.points.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(),
            referenced[&r.code]
        );
    }
    assert!(!referenced.is_empty());
}

/// 3D/instance inventory of the retail banks (research/audio-xact.md,
/// 2026-10-30 spatial audio): the XACT3D cue variables exist, only
/// `Distance` carries volume curves, and every complex cue's instance
/// limit and behaviour flags are in the documented value set.
#[test]
fn retail_spatial_and_instance_data_inventory() {
    use magicka_content::xact::{CueEntry, CueSounds, GlobalSettings, Sound, SoundBank, SoundBody};
    use std::collections::BTreeMap;
    let Some(dir) = audio() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let g = GlobalSettings::parse(&std::fs::read(dir.join("magicka.xgs")).unwrap()).unwrap();
    for (i, v) in g.variables.iter().enumerate() {
        eprintln!(
            "variable {i} {} flags {:#04x} initial {} min {} max {}",
            v.name, v.flags, v.initial, v.min, v.max
        );
    }
    for c in &g.categories {
        eprintln!(
            "category {} max_instances {} flags {:#04x} fade {}/{} parent {}",
            c.name, c.max_instances, c.instance_flags, c.fade_in, c.fade_out, c.parent
        );
    }
    let names: Vec<_> = g.variables.iter().map(|v| v.name.as_str()).collect();
    for required in [
        "Distance",
        "DopplerPitchScalar",
        "OrientationAngle",
        "SpeedOfSound",
    ] {
        assert!(names.contains(&required), "variable {required} missing");
    }
    // Curves by (variable, parameter).
    let mut by_var: BTreeMap<(String, u16), Vec<u32>> = BTreeMap::new();
    for r in &g.rpcs {
        let v = g.variables[r.variable as usize].name.clone();
        by_var.entry((v, r.parameter)).or_default().push(r.code);
    }
    for ((v, p), codes) in &by_var {
        eprintln!("curves var {v} param {p}: {} codes", codes.len());
    }
    let mut limits: BTreeMap<(u8, u8), usize> = BTreeMap::new();
    let mut simple_cues = 0;
    let mut fades = 0;
    let mut filters: BTreeMap<u32, usize> = BTreeMap::new();
    let mut distance_sounds = 0;
    let mut angle_or_doppler = 0;
    let mut sounds_total = 0;
    let mut referenced: BTreeMap<u32, usize> = BTreeMap::new();
    let mut limited_examples = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("xsb") {
            continue;
        }
        let bank = SoundBank::parse(&std::fs::read(&path).unwrap()).unwrap();
        for cue in &bank.cues {
            let mut sounds: Vec<&Sound> = Vec::new();
            match &cue.sounds {
                CueSounds::Single(s) => {
                    simple_cues += 1;
                    sounds.push(s);
                }
                CueSounds::Variations { entries, .. } => {
                    for (e, _) in entries {
                        if let CueEntry::Sound(s) = e {
                            sounds.push(s);
                        }
                    }
                }
            }
            if !matches!(cue.sounds, CueSounds::Single(_)) || cue.instance_limit != 255 {
                *limits
                    .entry((cue.instance_limit, cue.instance_flags))
                    .or_default() += 1;
                if cue.instance_limit != 255 && limited_examples.len() < 40 {
                    limited_examples.push(format!(
                        "{}/{} limit {} flags {:#04x} fade {}/{}",
                        bank.name,
                        cue.name,
                        cue.instance_limit,
                        cue.instance_flags,
                        cue.fade_in,
                        cue.fade_out
                    ));
                }
            }
            if cue.fade_in > 0.0 || cue.fade_out > 0.0 {
                fades += 1;
            }
            for s in sounds {
                sounds_total += 1;
                let mut has_distance = false;
                for code in &s.rpc_codes {
                    *referenced.entry(*code).or_default() += 1;
                    if let Some(r) = g.rpcs.iter().find(|r| r.code == *code) {
                        match g.variables[r.variable as usize].name.as_str() {
                            "Distance" => has_distance = true,
                            "OrientationAngle" | "DopplerPitchScalar" => angle_or_doppler += 1,
                            _ => {}
                        }
                    }
                }
                distance_sounds += has_distance as usize;
                if let SoundBody::Complex(clips) = &s.body {
                    for c in clips {
                        *filters.entry(c.filter_raw).or_default() += 1;
                    }
                }
            }
        }
    }
    eprintln!("simple cues {simple_cues}, cues with fades {fades}");
    for ((limit, flags), n) in &limits {
        eprintln!("complex cue instance_limit {limit} flags {flags:#04x}: {n}");
    }
    for e in &limited_examples {
        eprintln!("limited: {e}");
    }
    eprintln!(
        "sounds {sounds_total}, with Distance curves {distance_sounds}, with angle/doppler curves {angle_or_doppler}"
    );
    for (raw, n) in &filters {
        let f = magicka_content::xact::ClipFilter::from_raw(*raw);
        eprintln!("clip filter raw {raw:#010x}: {n} clips -> {f:?}");
    }
    for ((v, p), codes) in &by_var {
        if matches!(
            v.as_str(),
            "Distance" | "OrientationAngle" | "DopplerPitchScalar"
        ) {
            for code in codes {
                let r = g.rpcs.iter().find(|r| r.code == *code).unwrap();
                eprintln!(
                    "curve {code} var {v} param {p} refs {} points {:?}",
                    referenced.get(code).copied().unwrap_or(0),
                    r.points
                        .iter()
                        .map(|q| (q.x, q.y, q.kind))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    assert!(distance_sounds > 0);
    assert_eq!(
        angle_or_doppler, 0,
        "no sound references angle/doppler curves"
    );
    for ((_, flags), _) in &limits {
        assert!(flags & !0x1f == 0, "unexpected instance flags {flags:#04x}");
    }
}
