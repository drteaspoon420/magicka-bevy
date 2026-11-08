//! SubMenuCutscene: the chapter page, then (one page turned) the campaign map
//! panning under its mask with the chapter's narration and subtitles, then a
//! half-second fade into the level. Playback rules are
//! `magicka_runtime::frontend_cutscene`; see research/frontend-cutscenes.md.
use super::{
    book::{Atlas, Font, Measure, PageDraw, Widget},
    pages::PageInput,
    sound::MenuDialog,
    Frontend, Session,
};
use crate::frontend_tome::{MenuId, TomeView};
use bevy::prelude::*;
use magicka_runtime::{
    frontend_cutscene::{
        parse_map_cutscenes, MapCutscene, MapCutsceneEvent, MapCutscenePlayback, MapPhase,
    },
    frontend_tome::TomeState,
    text_geometry::TextAlign,
};
use std::collections::HashMap;

/// DialogManager: subtitles sit this far above the bottom of the screen.
const SUBTITLE_MARGIN: f32 = 32.0;

/// One level's map cutscene and the map it pans (SubMenuCutscene.Level).
#[derive(Clone)]
pub(crate) struct MapEntry {
    pub cutscene: MapCutscene,
    pub map: &'static str,
    pub mask: &'static str,
    /// The map polygon (SetImageDimensions): 2048 for the campaign map,
    /// the texture's size for the Dungeons maps.
    pub size: [f32; 2],
}

impl MapEntry {
    /// DrawMap's transform as the masked widget's TextureScale/Offset over
    /// the 1024-pixel right page.
    fn uv(&self, time: f32) -> [f32; 4] {
        let (offset, zoom) = self.cutscene.camera(time).image_transform(self.size);
        let [w, h] = [self.size[0] * zoom, self.size[1] * zoom];
        [1024.0 / w, 1024.0 / h, -offset[0] / w, -offset[1] / h]
    }
}

/// LevelManager: Levels/Cutscenes.xml (vanilla), Levels/Tsar/Cutscenes.xml
/// (Mythos) by campaign index, and the two Dungeons files, by level file
/// (lower case).
pub(crate) fn load_entries(content: &str) -> HashMap<String, MapEntry> {
    use magicka_runtime::end_game::{DUNGEONS_CAMPAIGN, MYTHOS_CAMPAIGN, VANILLA_CAMPAIGN};
    let root = std::path::Path::new(content);
    let read = |file: &str| {
        magicka_runtime::play_content::resolve(root, std::path::Path::new(file))
            .and_then(|p| std::fs::read(p).ok())
            .map(|bytes| parse_map_cutscenes(&bytes))
            .transpose()
            .unwrap_or_else(|e| {
                error!("{file}: {e}");
                None
            })
            .unwrap_or_default()
    };
    let size = |texture: &str| -> [f32; 2] {
        magicka_runtime::play_content::resolve(
            root,
            std::path::Path::new(&format!("{texture}.xnb")),
        )
        .and_then(|p| crate::text_box_visual::read_asset(&p).ok())
        .and_then(|a| match a {
            magicka_content::assets::Asset::Texture2D(t) => Some([t.width as f32, t.height as f32]),
            _ => None,
        })
        .unwrap_or([1704.0, 652.0])
    };
    let mut out = HashMap::new();
    for (file, levels) in [
        ("Levels/Cutscenes.xml", &VANILLA_CAMPAIGN[..]),
        ("Levels/Tsar/Cutscenes.xml", &MYTHOS_CAMPAIGN[..]),
    ] {
        for (id, cutscene) in read(file) {
            if let Some(level) = usize::try_from(id).ok().and_then(|i| levels.get(i)) {
                out.insert(
                    level.to_ascii_lowercase(),
                    MapEntry {
                        cutscene,
                        map: "UI/Menu/CampaignMap",
                        mask: "UI/Menu/MapMask",
                        size: [2048.0; 2],
                    },
                );
            }
        }
    }
    for (file, level, map, mask) in [
        (
            "Levels/Challenges/Dungeons_Cutscenes.xml",
            DUNGEONS_CAMPAIGN[0],
            "UI/Menu/CampaignMap_Dungeons",
            "UI/Menu/MapMask_Dungeons",
        ),
        (
            "Levels/Challenges/Dungeons2_Cutscenes.xml",
            DUNGEONS_CAMPAIGN[1],
            "UI/Menu/CampaignMap_Dungeons2",
            "UI/Menu/MapMask_Dungeons2",
        ),
    ] {
        if let Some((_, cutscene)) = read(file).into_iter().next() {
            out.insert(
                level.to_ascii_lowercase(),
                MapEntry {
                    cutscene,
                    map,
                    mask,
                    size: size(map),
                },
            );
        }
    }
    out
}

#[derive(Resource, Default)]
pub(crate) struct MapCutscenes {
    entries: HashMap<String, MapEntry>,
    playing: Option<Playing>,
    /// Black after the fade until the level's loading screen takes over.
    black: bool,
}

struct Playing {
    playback: MapCutscenePlayback,
    entry: Option<MapEntry>,
    chapter: String,
    title: String,
    subtitle_key: Option<i32>,
    subtitle: Option<String>,
    fade: Option<(f32, f32)>,
}

pub(crate) fn plugin(app: &mut App, content: &str) {
    app.insert_resource(MapCutscenes {
        entries: load_entries(content),
        ..default()
    })
    .add_systems(
        Update,
        run.after(super::pages::run)
            .in_set(super::book::BookDrawSet),
    );
}

/// SubMenuCutscene.Level setter and OnEnter.
fn enter(map: &mut MapCutscenes, s: &Frontend, level: &str, dialog: &mut MenuDialog) {
    let entry = map.entries.get(&level.to_ascii_lowercase()).cloned();
    let (chapter, title) = super::pages::level_keys(&s.content, level)
        .map(|(name, description)| {
            (
                super::pages::loc(s, &name),
                super::pages::loc(s, &description),
            )
        })
        .unwrap_or_default();
    match &entry {
        Some(e) => dialog.prepare(e.cutscene.dialog_bank, e.cutscene.dialog),
        None => dialog.clear(),
    }
    let subtitle_key = entry.as_ref().map(|e| e.cutscene.subtitles);
    map.playing = Some(Playing {
        playback: MapCutscenePlayback::enter(
            true,
            entry.as_ref().map(|e| &e.cutscene),
            subtitle_key.is_some(),
        ),
        entry,
        chapter,
        // The short description, if it fits on the chapter page.
        title: if title.chars().count() > 64 {
            String::new()
        } else {
            title
        },
        subtitle_key,
        subtitle: None,
        fade: None,
    });
}

#[allow(clippy::too_many_arguments)]
fn run(
    mut map: ResMut<MapCutscenes>,
    mut state: ResMut<Frontend>,
    mut tome: ResMut<TomeView>,
    mut dialog: ResMut<MenuDialog>,
    mut draw: ResMut<PageDraw>,
    mut assets: ResMut<super::book::BookAssets>,
    mut images: ResMut<Assets<Image>>,
    input: Res<PageInput>,
    time: Res<Time>,
) {
    let map = &mut *map;
    // Hold black until the session takes the window (or failed to start).
    if map.black {
        let starting = matches!(state.session, Session::Idle) && state.launch.is_some();
        if starting || matches!(state.session, Session::Preparing(_)) {
            draw.screen()
                .push(Widget::fill([0.0, 0.0], input.screen, [0.0, 0.0, 0.0, 1.0]));
        } else {
            map.black = false;
        }
    }
    let current = tome.current();
    if current == Some(MenuId::Cutscene) && map.playing.is_none() {
        if let Some(level) = state.cutscene_level.take() {
            enter(map, &state, &level, &mut dialog);
        }
    }
    if current != Some(MenuId::Cutscene) {
        if map.playing.take().is_some() {
            dialog.clear();
        }
        return;
    }
    let Some(playing) = map.playing.as_mut() else {
        return;
    };
    let mut measure = Measure {
        assets: &mut assets,
        images: &mut images,
    };
    let open = tome
        .tome
        .as_ref()
        .is_some_and(|t| t.state == TomeState::Open);
    let dt = time.delta_secs();
    let mut events = Vec::new();
    // ControllerA (any cue stops) and ControllerMouseAction (a playing cue
    // stops); both only with the book open.
    if input.confirm {
        events.extend(playing.playback.skip(open, dialog.state(), true));
    } else if input.released {
        events.extend(playing.playback.skip(open, dialog.state(), false));
    }
    // Submenus update while the book is open.
    if open {
        events.extend(playing.playback.update(dt, dialog.state()));
    }
    if let Some((age, seconds)) = playing.fade.as_mut() {
        *age += dt;
        if *age >= *seconds {
            events.extend(playing.playback.transition_end());
        }
    }
    for event in events {
        match event {
            MapCutsceneEvent::Riffle => {
                if let Some(t) = tome.tome.as_mut() {
                    t.riffle(true, 1);
                }
            }
            MapCutsceneEvent::PlayCue => dialog.play(),
            MapCutsceneEvent::StopCue => dialog.stop(),
            MapCutsceneEvent::ShowSubtitles => {
                // OnEnter wraps the subtitles to the screen width less 128.
                playing.subtitle = playing.subtitle_key.and_then(|key| {
                    let text = state.strings.try_get_string(key).value?;
                    Some(measure.assets.wrap(
                        Font::Maiandra14,
                        text,
                        input.screen[0] as i32 - 128,
                        measure.images,
                    ))
                });
            }
            MapCutsceneEvent::HideSubtitles => playing.subtitle = None,
            MapCutsceneEvent::FadeToBlack { seconds } => playing.fade = Some((0.0, seconds)),
            MapCutsceneEvent::LoadLevel => {
                // OnTransitionEnd: pop the cutscene (and SubMenuEndGame
                // beneath it) instantly, then start the level.
                if let Some(t) = tome.tome.as_mut() {
                    t.pop_instant();
                    if t.current() == Some(MenuId::EndGame) {
                        t.pop_instant();
                    }
                }
                state.launch = state.pending_launch.take();
                map.black = true;
            }
        }
    }
    let Some(playing) = map.playing.as_ref() else {
        return;
    };
    if map.black {
        // The fade ended this frame: stay black until the loading screen.
        map.playing = None;
        dialog.clear();
        draw.screen()
            .push(Widget::fill([0.0, 0.0], input.screen, [0.0, 0.0, 0.0, 1.0]));
        return;
    }
    // Draw (current content) and DrawOld (the chapter, turning away).
    let chapter = |out: &mut super::book::PageWriter<'_>| {
        let color = magicka_runtime::menu_text_item::MENU_COLOR;
        out.push(
            Widget::text(
                Font::MenuTitle,
                playing.chapter.clone(),
                [512.0, 360.0],
                TextAlign::Center,
            )
            .color(color),
        );
        out.push(
            Widget::text(
                Font::MenuTitle,
                playing.title.clone(),
                [512.0, 480.0],
                TextAlign::Center,
            )
            .color(color),
        );
    };
    let on_map = !matches!(playing.playback.phase, MapPhase::Chapter);
    match (&playing.entry, on_map) {
        (Some(entry), true) => {
            draw.right(MenuId::Cutscene).push(
                Widget::masked(
                    Atlas::Content(entry.map.into()),
                    Atlas::Content(entry.mask.into()),
                    [0.0, 0.0],
                    [1024.0, 1024.0],
                )
                .uv_transform(entry.uv(playing.playback.time)),
            );
            chapter(&mut draw.right_old(MenuId::Cutscene));
        }
        (Some(_), false) => chapter(&mut draw.right(MenuId::Cutscene)),
        (None, _) => {}
    }
    // DialogManager subtitles: centred, a one-pixel shadow at 0.8 alpha.
    if let Some(text) = &playing.subtitle {
        let height = measure.text(Font::Maiandra14, text)[1];
        let position = [
            input.screen[0] * 0.5,
            input.screen[1] - SUBTITLE_MARGIN - height,
        ];
        let mut screen = draw.screen();
        screen.push(
            Widget::text(
                Font::Maiandra14,
                text.clone(),
                [position[0] + 1.0, position[1] + 1.0],
                TextAlign::Center,
            )
            .color([0.0, 0.0, 0.0, 0.8]),
        );
        screen.push(
            Widget::text(Font::Maiandra14, text.clone(), position, TextAlign::Center)
                .color([1.0; 4]),
        );
    }
    if let Some((age, seconds)) = playing.fade {
        let alpha = (age / seconds).min(1.0);
        draw.screen().push(Widget::fill(
            [0.0, 0.0],
            input.screen,
            [0.0, 0.0, 0.0, alpha],
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_campaign_level_after_the_first_and_both_dungeons_have_a_map() {
        let content = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../target/Magicka/Content");
        if !content.is_dir() {
            eprintln!("retail content absent; skipping");
            return;
        }
        use magicka_runtime::end_game::{DUNGEONS_CAMPAIGN, MYTHOS_CAMPAIGN, VANILLA_CAMPAIGN};
        let entries = load_entries(content.to_str().unwrap());
        for level in VANILLA_CAMPAIGN[1..]
            .iter()
            .chain(&MYTHOS_CAMPAIGN)
            .chain(&DUNGEONS_CAMPAIGN)
        {
            assert!(
                entries.contains_key(&level.to_ascii_lowercase()),
                "{level} has no map"
            );
        }
        // Dungeons maps keep their own polygon size.
        let dungeons = &entries[&DUNGEONS_CAMPAIGN[0].to_ascii_lowercase()];
        assert_ne!(dungeons.size, [2048.0; 2]);
        // The narration cues resolve in the retail banks.
        let library = magicka_runtime::audio::AudioLibrary::load(&content);
        for (level, entry) in &entries {
            let c = &entry.cutscene;
            assert!(
                library
                    .resolve(c.dialog_bank, c.dialog, &mut || 0.5)
                    .is_some(),
                "{level} cue missing"
            );
        }
    }
}
