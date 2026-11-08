//! Retail dialog messages reach the live audio queue with independent cue keys.
use magicka_runtime::{
    audio::{AudioLibrary, AudioRequest, SoundSource},
    dialog_definition::DialogDefinition,
    interaction_playback::InteractionLocation,
    play_content,
    play_session::PlaySession,
};
use std::path::{Path, PathBuf};
#[path = "support/ffmpeg.rs"]
mod ffmpeg;

#[test]
fn retail_dialog_voices_start_and_end_with_independent_audio_keys() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    if !ffmpeg::require() {
        return;
    }
    let path = play_content::level_dialogs(&root, "Wizard_Castle.lvl").unwrap();
    let bytes = std::fs::read(path).unwrap();
    let definitions = DialogDefinition::read_collection(&[&bytes]).unwrap();
    let voiced: Vec<_> = definitions
        .iter()
        .filter_map(|dialog| {
            let text = dialog
                .interactions
                .first()?
                .messages
                .first()?
                .texts
                .first()?;
            (text.sound != 0).then_some((dialog.id, text.sound_bank, text.sound))
        })
        .take(2)
        .collect();
    assert_eq!(voiced.len(), 2, "two voiced retail dialogs required");
    let mut session = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    session.disable_scripts();
    session.world.audio_requests.clear();
    {
        let manager = session.world.dialogs.as_mut().unwrap();
        let text = session.world.dialog_text.as_mut().unwrap();
        for &(id, _, _) in &voiced {
            assert!(manager
                .start(id, InteractionLocation::Screen([0.0; 2]), None, false, text)
                .unwrap()
                .is_some());
        }
    }
    session.world.update_dialogs(0.0, [[0.0; 4]; 4]);
    assert!(session.world.dialog_failure.is_none());
    let library = AudioLibrary::load(&root);
    let mut keys = Vec::new();
    for &(_, bank, cue) in &voiced {
        let key = session
            .world
            .audio_requests
            .iter()
            .find_map(|request| match request {
                AudioRequest::Start {
                    key,
                    bank: b,
                    cue: c,
                    source: SoundSource::Global,
                    ..
                } if *b == bank && *c == cue => Some(*key),
                _ => None,
            })
            .expect("message voice forwarded to audio");
        keys.push(key);
        let voices = library
            .resolve(bank, cue, &mut || 0.0)
            .expect("retail cue resolves");
        assert!(!voices.is_empty());
        for voice in voices {
            let pcm = library
                .try_decode(&voice.wave_bank, voice.track)
                .expect("voice decodes");
            assert!(!pcm.samples.is_empty());
            assert!(
                pcm.samples.iter().any(|&sample| sample != 0),
                "voice must contain an audible signal"
            );
        }
    }
    assert_ne!(keys[0], keys[1]);
    session.world.audio_requests.clear();
    session
        .world
        .dialogs
        .as_mut()
        .unwrap()
        .end_all(session.world.dialog_text.as_mut().unwrap())
        .unwrap();
    session.world.update_dialogs(0.0, [[0.0; 4]; 4]);
    for key in keys {
        assert!(session
            .world
            .audio_requests
            .contains(&AudioRequest::Stop { key, instant: true }));
    }
}
