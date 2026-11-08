//! Retail cue resolution (research/audio-xact.md).
use magicka_runtime::audio::{bank_index, AudioLibrary};
use magicka_runtime::scene_loading::custom_hash;
use std::path::PathBuf;
#[path = "support/ffmpeg.rs"]
mod ffmpeg;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

#[test]
fn bank_flags_map_to_audio_manager_indices() {
    assert_eq!(bank_index(1), Some(0));
    assert_eq!(bank_index(2), Some(1));
    assert_eq!(bank_index(0x10), Some(4));
    assert_eq!(bank_index(0x400), Some(10));
    assert_eq!(bank_index(0), None);
}

#[test]
fn retail_status_cues_resolve_to_decodable_voices() {
    let Some(root) = content() else {
        return;
    };
    if !ffmpeg::require() {
        return;
    }
    let lib = AudioLibrary::load(&root);
    for name in [
        "spell_fire_onfire",
        "spell_water_drip",
        "spell_ice_frozen",
        "spell_steam_hit",
    ] {
        let voices = lib
            .resolve(0x10, custom_hash(name), &mut || 0.5)
            .expect(name);
        assert!(!voices.is_empty(), "{name}");
        assert_eq!(
            voices.iter().any(|v| v.loop_count == 255),
            name != "spell_steam_hit",
            "status cues contain indefinite loops; steam is a one-shot"
        );
        eprintln!("status cue {name}: {voices:?}");
        for voice in voices {
            assert!(lib.decode(&voice.wave_bank, voice.track).unwrap().frames() > 0);
        }
    }
}

#[test]
fn retail_cues_resolve_to_decodable_voices() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    if !ffmpeg::require() {
        return;
    }
    let lib = AudioLibrary::load(&root);
    assert!(lib.errors.is_empty(), "{:?}", lib.errors);
    let mut seed = 0x1234_5678u32;
    let mut random = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    for (bank, cue) in [
        (0x10u16, "spell_mine01"),
        (0x10, "spell_earth_barrier"),
        (2, "music_havindr_night"),
        (8, "ui_checkpoint01"),
    ] {
        let hash = custom_hash(cue);
        assert!(lib.has_cue(bank, hash), "{cue}");
        let voices = lib.resolve(bank, hash, &mut random).unwrap();
        assert!(!voices.is_empty(), "{cue} produced no voices");
        for v in &voices {
            let pcm = lib.decode(&v.wave_bank, v.track).expect("decodable");
            assert!(pcm.frames() > 100, "{cue}: {} frames", pcm.frames());
            assert!(v.volume_db > -60.0 && v.volume_db < 12.0, "{cue}: {v:?}");
        }
    }
    // Music loops forever.
    let music = lib
        .resolve(2, custom_hash("music_havindr_night"), &mut random)
        .unwrap();
    assert!(music.iter().any(|v| v.loop_count == 255), "{music:?}");
}

#[test]
fn rpc_gain_follows_distance_and_categories() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let lib = AudioLibrary::load(&root);
    let mut r = || 0.5f32;
    let voices = lib
        .resolve(0x10, custom_hash("spell_mine01"), &mut r)
        .unwrap();
    let v = &voices[0];
    let near = lib.gain_db(v, &|n| match n {
        "Distance" => Some(0.0),
        "AttackTime" => Some(10_000.0),
        _ => None,
    });
    let far = lib.gain_db(v, &|n| match n {
        "Distance" => Some(150.0),
        "AttackTime" => Some(10_000.0),
        _ => None,
    });
    assert!(near > -20.0 && near < 12.0, "{near}");
    assert!(far < near - 30.0, "{near} {far}");
    let music = lib
        .resolve(2, custom_hash("music_havindr_night"), &mut r)
        .unwrap();
    // AttackTime (ms since play) fades in from −59 dB over 0.67 s; then the
    // Music category (−9 dB) and MusicThreat 2 (−2.4 dB) remain.
    let start = lib.gain_db(&music[0], &|_| None);
    let steady = lib.gain_db(&music[0], &|n| (n == "AttackTime").then_some(10_000.0));
    assert!(start < -60.0, "{start}");
    assert!((steady - (-11.35)).abs() < 0.5, "{steady}");
}

#[test]
fn havindr_start_requests_music_and_ambience() {
    use magicka_runtime::audio::AudioRequest;
    use magicka_runtime::play_session::PlaySession;
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let lib = AudioLibrary::load(&root);
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    let mut music = 0;
    let mut starts = 0;
    for (r, position) in &s.audio {
        match r {
            AudioRequest::Music { bank, cue } => {
                assert!(lib.has_cue(*bank, *cue), "music {cue}");
                music += 1;
            }
            AudioRequest::Start {
                bank, cue, source, ..
            } => {
                assert!(lib.has_cue(*bank, *cue), "{:?}", lib.cue_name(*bank, *cue));
                if matches!(source, magicka_runtime::audio::SoundSource::Locator { .. }) {
                    assert!(position.is_some());
                }
                starts += 1;
            }
            _ => {}
        }
    }
    eprintln!("music {music}, starts {starts}");
    assert!(music >= 1 && starts >= 1);
}
