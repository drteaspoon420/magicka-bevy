//! Menu audio: MenuState's `music_menu`, the Tome's book and page cues and
//! the submenus' select/mouse-over cues, from the decoded retail banks that
//! play mode also uses.
use super::{Frontend, Session};
use crate::{frontend_tome::TomeView, pcm_audio::PcmClip};
use bevy::{
    audio::{AudioPlayer, PlaybackMode, PlaybackSettings, Volume},
    prelude::*,
};
use magicka_runtime::{
    audio::{banks, AudioLibrary, Voice},
    frontend_cutscene::CueState,
    frontend_tome::TomeEvent,
    scene_loading::custom_hash,
};
use std::sync::Arc;

struct Pending {
    voice: Voice,
    age: f32,
    music: bool,
    pages: Option<f32>,
    dialog: bool,
}

#[derive(Resource, Default)]
pub(crate) struct MenuSound {
    library: Option<Arc<AudioLibrary>>,
    pending: Vec<Pending>,
    /// Music voices and their cue gain (before the user's music volume).
    music: Vec<(Entity, f32)>,
    music_on: bool,
    rng: u32,
}

/// Cues requested by menu logic this frame (UI bank).
#[derive(Resource, Default)]
pub(crate) struct MenuCues(pub Vec<&'static str>);

/// One tracked dialog cue (SubMenuCutscene's narration) with the XACT
/// states its playback logic observes. `prepare` (AudioManager.GetCue),
/// then `play`/`stop`; a cue missing from the banks stays `Absent`.
#[derive(Resource)]
pub(crate) struct MenuDialog {
    cue: Option<(u16, i32)>,
    play: bool,
    stop: bool,
    state: CueState,
    entities: Vec<Entity>,
}
impl Default for MenuDialog {
    fn default() -> Self {
        Self {
            cue: None,
            play: false,
            stop: false,
            state: CueState::Absent,
            entities: Vec::new(),
        }
    }
}
impl MenuDialog {
    pub(crate) fn prepare(&mut self, bank: u16, cue: i32) {
        self.stop = !self.entities.is_empty();
        self.cue = Some((bank, cue));
        self.play = false;
        self.state = CueState::Prepared;
    }
    pub(crate) fn clear(&mut self) {
        self.stop = !self.entities.is_empty();
        self.cue = None;
        self.play = false;
        self.state = CueState::Absent;
    }
    pub(crate) fn play(&mut self) {
        if self.state == CueState::Prepared {
            self.play = true;
        }
    }
    pub(crate) fn stop(&mut self) {
        if matches!(self.state, CueState::Prepared | CueState::Playing) {
            self.stop = true;
            self.play = false;
            self.state = CueState::Stopped;
        }
    }
    pub(crate) fn state(&self) -> CueState {
        self.state
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<MenuSound>()
        .init_resource::<MenuCues>()
        .init_resource::<MenuDialog>()
        .add_systems(Update, play.after(crate::frontend_tome::TomeSet));
}

impl MenuSound {
    fn random(&mut self) -> f32 {
        self.rng = if self.rng == 0 { 0x9e37_79b9 } else { self.rng };
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }
    fn start(&mut self, bank: u16, cue: &str, music: bool, pages: Option<f32>) {
        if !self.start_hashed(bank, custom_hash(cue), music, pages, false) {
            warn!("menu cue missing: {cue}");
        }
    }
    fn start_hashed(
        &mut self,
        bank: u16,
        cue: i32,
        music: bool,
        pages: Option<f32>,
        dialog: bool,
    ) -> bool {
        let Some(library) = self.library.clone() else {
            return false;
        };
        let mut random = || self.random();
        let Some(voices) = library.resolve(bank, cue, &mut random) else {
            return false;
        };
        for voice in voices {
            self.pending.push(Pending {
                voice,
                age: 0.0,
                music,
                pages,
                dialog,
            });
        }
        true
    }
}

#[allow(clippy::too_many_arguments)]
fn play(
    mut commands: Commands,
    mut sound: ResMut<MenuSound>,
    mut cues: ResMut<MenuCues>,
    mut dialog: ResMut<MenuDialog>,
    mut tome: ResMut<TomeView>,
    state: Res<Frontend>,
    intro: Res<crate::frontend_cinema::IntroControl>,
    audio: Option<Res<crate::play::AudioLibraryShare>>,
    mut clips: ResMut<Assets<PcmClip>>,
    entities: Query<Entity>,
    mut sinks: Query<&mut bevy::audio::AudioSink>,
    time: Res<Time>,
) {
    let sound = &mut *sound;
    if sound.library.is_none() {
        sound.library = audio.map(|a| a.0.clone());
    }
    let events: Vec<TomeEvent> = std::mem::take(&mut tome.events);
    let requested = std::mem::take(&mut cues.0);
    if sound.library.is_none() {
        // Silent: a cue that cannot play is absent, never left prepared.
        if std::mem::take(&mut dialog.play) {
            dialog.state = CueState::Absent;
        }
        return;
    }
    // MenuState.OnEnter: PlayMusic(music_menu) while the Tome is the
    // current state; the opening and play sessions replace it.
    let wanted = tome.visible
        && matches!(state.session, Session::Idle)
        && !intro.active
        && !intro.requested
        && state.page != 7;
    if wanted != sound.music_on {
        sound.music_on = wanted;
        for (entity, _) in std::mem::take(&mut sound.music) {
            if entities.get(entity).is_ok() {
                commands.entity(entity).despawn();
            }
        }
        sound.pending.retain(|p| !p.music);
        if wanted {
            sound.start(banks::MUSIC, "music_menu", true, None);
        }
    }
    for event in events {
        match event {
            TomeEvent::BookOpen => sound.start(banks::UI, "ui_menu_bookopen", false, None),
            TomeEvent::BookClose => sound.start(banks::UI, "ui_menu_bookclose", false, None),
            // PageSoundVariables: the number of riffled pages.
            TomeEvent::Pages(n) if n > 0 => {
                sound.start(banks::UI, "ui_menu_tomepage", false, Some(n as f32))
            }
            _ => {}
        }
    }
    for cue in requested {
        sound.start(banks::UI, cue, false, None);
    }
    if std::mem::take(&mut dialog.stop) {
        sound.pending.retain(|p| !p.dialog);
        for entity in std::mem::take(&mut dialog.entities) {
            if entities.get(entity).is_ok() {
                commands.entity(entity).despawn();
            }
        }
    }
    if std::mem::take(&mut dialog.play) {
        let started = dialog
            .cue
            .is_some_and(|(bank, cue)| sound.start_hashed(bank, cue, false, None, true));
        dialog.state = if started {
            CueState::Playing
        } else {
            CueState::Absent
        };
    }
    let volumes = [
        state.settings["music"].as_f64().unwrap_or(0.8) as f32,
        state.settings["sfx"].as_f64().unwrap_or(1.0) as f32,
    ];
    let dt = time.delta_secs();
    let library = sound.library.clone().unwrap();
    let mut started = Vec::new();
    sound.pending.retain_mut(|p| {
        p.age += dt;
        if p.age < p.voice.delay {
            return true;
        }
        let pages = p.pages;
        let variables = |name: &str| -> Option<f32> {
            match name {
                "Pages" => pages,
                "AttackTime" | "ReleaseTime" => Some(0.0),
                _ => None,
            }
        };
        let pcm = match library.try_decode(&p.voice.wave_bank, p.voice.track) {
            Ok(pcm) => pcm,
            Err(error) => {
                warn!("menu cue: {error}");
                return false;
            }
        };
        let cue_gain = 10f32.powf(library.gain_db(&p.voice, &variables) / 20.0);
        let gain = cue_gain * if p.music { volumes[0] } else { volumes[1] };
        let speed = 2f32.powf((p.voice.pitch + library.rpc_pitch(&p.voice, &variables)) / 12.0);
        let entity = commands
            .spawn((
                AudioPlayer(clips.add(PcmClip::from_pcm(pcm, p.voice.loop_count))),
                PlaybackSettings {
                    mode: PlaybackMode::Despawn,
                    volume: Volume::Linear(gain),
                    speed,
                    ..PlaybackSettings::DESPAWN
                },
            ))
            .id();
        if p.music {
            started.push((entity, cue_gain));
        }
        if p.dialog {
            dialog.entities.push(entity);
        }
        false
    });
    sound.music.extend(started);
    // A playing cue stops once every voice has been heard to its end.
    dialog.entities.retain(|e| entities.get(*e).is_ok());
    if dialog.state == CueState::Playing
        && dialog.entities.is_empty()
        && !sound.pending.iter().any(|p| p.dialog)
    {
        dialog.state = CueState::Stopped;
    }
    // AudioManager.VolumeMusic applies at once (the options page).
    for (entity, cue_gain) in &sound.music {
        if let Ok(mut sink) = sinks.get_mut(*entity) {
            sink.set_volume(Volume::Linear(cue_gain * volumes[0]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // FFmpeg resolution shared with the runtime xWMA tests.
    mod ffmpeg {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../magicka-runtime/tests/support/ffmpeg.rs"
        ));
    }

    #[test]
    fn menu_cues_resolve_in_the_retail_banks() {
        let content = magicka_content::retail_content_dir();
        if !content.is_dir() {
            eprintln!("retail content absent; skipping");
            return;
        }
        if !ffmpeg::require() {
            return;
        }
        let library = AudioLibrary::load(&content);
        for (bank, cue) in [
            (banks::MUSIC, "music_menu"),
            (banks::UI, "ui_menu_bookopen"),
            (banks::UI, "ui_menu_bookclose"),
            (banks::UI, "ui_menu_tomepage"),
            (banks::UI, "ui_menu_select"),
            (banks::UI, "ui_menu_mouseover"),
        ] {
            let voices = library
                .resolve(bank, custom_hash(cue), &mut || 0.5)
                .unwrap_or_else(|| panic!("{cue} missing"));
            assert!(!voices.is_empty(), "{cue} has no voices");
            for voice in &voices {
                library
                    .try_decode(&voice.wave_bank, voice.track)
                    .unwrap_or_else(|e| panic!("{cue}: {e}"));
            }
        }
    }
}
