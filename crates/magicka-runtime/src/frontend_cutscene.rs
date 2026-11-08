//! Campaign-map cutscene data and menu playback, independent of presentation.
//! Static CIL provenance and remaining integration: research/frontend-cutscenes.md.
use crate::{scene_loading::custom_hash, scene_script::XmlElement};

#[derive(Clone, Debug, PartialEq)]
pub struct CameraKey<const N: usize> {
    pub time: f32,
    pub value: [f32; N],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapCamera {
    pub position: [f32; 2],
    pub zoom: f32,
}

impl MapCamera {
    /// SubMenuCutscene.DrawMap's image transform in right-page coordinates.
    pub fn image_transform(self, image_size: [f32; 2]) -> ([f32; 2], f32) {
        (
            std::array::from_fn(|i| image_size[i] / 4.0 - self.position[i] * self.zoom),
            self.zoom,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapCutscene {
    pub zoom_keys: Vec<CameraKey<1>>,
    pub position_keys: Vec<CameraKey<2>>,
    pub dialog_bank: u16,
    pub dialog: i32,
    pub subtitles: i32,
    pub duration: f32,
}

fn attribute<'a>(node: &'a XmlElement, name: &str) -> Option<&'a str> {
    node.attributes
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn key<const N: usize>(node: &XmlElement) -> Result<CameraKey<N>, String> {
    let time = attribute(node, "time")
        .unwrap_or("0")
        .trim()
        .parse::<f32>()
        .map_err(|e| format!("camera time: {e}"))?;
    let values = node.inner_text.split(',').collect::<Vec<_>>();
    if values.len() != N {
        return Err(format!("{} requires {N} components", node.name));
    }
    let mut value = [0.0_f32; N];
    for (out, text) in value.iter_mut().zip(values) {
        *out = text
            .trim()
            .parse()
            .map_err(|e| format!("camera value: {e}"))?;
    }
    // Invalid camera tracks fail explicitly rather than poisoning a renderer.
    if !time.is_finite() || value.iter().any(|v| !v.is_finite()) {
        return Err("nonfinite camera key".into());
    }
    Ok(CameraKey { time, value })
}

impl MapCutscene {
    /// Cutscene(XmlNode). Native code sorts both tracks, then takes the later
    /// final key as duration. A missing track would throw in the original.
    pub fn from_xml(node: &XmlElement) -> Result<Self, String> {
        let mut zoom_keys = Vec::new();
        let mut position_keys = Vec::new();
        let mut dialog_bank = 1;
        let mut dialog = 0;
        let mut subtitles = 0;
        for child in &node.children {
            if child.name.eq_ignore_ascii_case("camera") {
                for entry in &child.children {
                    if entry.name.eq_ignore_ascii_case("zoom") {
                        zoom_keys.push(key(entry)?);
                    } else if entry.name.eq_ignore_ascii_case("position") {
                        position_keys.push(key(entry)?);
                    }
                }
            } else if child.name.eq_ignore_ascii_case("dialog") {
                if let Some(text) = attribute(child, "subtitles") {
                    subtitles = custom_hash(&text.to_lowercase());
                }
                let text = child.inner_text.to_lowercase();
                let parts = text.split('/').collect::<Vec<_>>();
                let cue = match parts.as_slice() {
                    [cue] => {
                        dialog_bank = 1;
                        *cue
                    }
                    [bank, cue] => {
                        let names = [
                            "wavebank",
                            "music",
                            "ambience",
                            "ui",
                            "spells",
                            "characters",
                            "footsteps",
                            "weapons",
                            "misc",
                            "additional",
                            "additionalmusic",
                        ];
                        let index = names
                            .iter()
                            .position(|name| name == bank)
                            .ok_or_else(|| format!("unknown cutscene bank {bank}"))?;
                        dialog_bank = 1 << index;
                        *cue
                    }
                    _ => return Err("invalid cutscene sound description".into()),
                };
                dialog = custom_hash(cue);
            }
        }
        zoom_keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        position_keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        let duration = zoom_keys
            .last()
            .ok_or("missing zoom keys")?
            .time
            .max(position_keys.last().ok_or("missing position keys")?.time);
        Ok(Self {
            zoom_keys,
            position_keys,
            dialog_bank,
            dialog,
            subtitles,
            duration,
        })
    }

    pub fn camera(&self, time: f32) -> MapCamera {
        MapCamera {
            position: sample(&self.position_keys, time),
            zoom: sample(&self.zoom_keys, time)[0],
        }
    }
}

/// Parse a retail Cutscenes.xml without loading textures or executing game code.
pub fn parse_map_cutscenes(bytes: &[u8]) -> Result<Vec<(i32, MapCutscene)>, String> {
    let root = crate::scene_script::xml(bytes).map_err(|e| format!("cutscenes XML: {e:?}"))?;
    let mut result = Vec::new();
    for child in &root.children {
        if child.name.eq_ignore_ascii_case("cutscene") {
            let id = attribute(child, "id")
                .ok_or("missing cutscene id")?
                .parse()
                .map_err(|e| format!("cutscene id: {e}"))?;
            result.push((id, MapCutscene::from_xml(child)?));
        }
    }
    Ok(result)
}

fn sample<const N: usize>(keys: &[CameraKey<N>], time: f32) -> [f32; N] {
    for (index, key) in keys.iter().enumerate() {
        if time < key.time {
            if index == 0 {
                return key.value;
            }
            let previous = &keys[index - 1];
            let t = ((time - previous.time) / (key.time - previous.time)).clamp(0.0, 1.0);
            let t = t * t * (3.0 - 2.0 * t);
            return std::array::from_fn(|i| {
                previous.value[i] + (key.value[i] - previous.value[i]) * t
            });
        }
    }
    keys.last().expect("validated camera track").value
}

/// XACT observations are supplied by the presentation host, never inferred from
/// camera duration. Prepared cues are not stopped cues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CueState {
    Absent,
    Prepared,
    Playing,
    Stopping,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MapCutsceneEvent {
    Riffle,
    PlayCue,
    ShowSubtitles,
    StopCue,
    HideSubtitles,
    FadeToBlack {
        seconds: f32,
    },
    /// Host resolves campaign/save/network state and starts the level here.
    LoadLevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapPhase {
    Chapter,
    Map,
    FinishPending,
    Fading,
    Complete,
}

#[derive(Clone, Debug)]
pub struct MapCutscenePlayback {
    pub phase: MapPhase,
    pub time: f32,
    title_remaining: f32,
    duration: f32,
    has_subtitles: bool,
}

impl MapCutscenePlayback {
    /// OnEnter. `play` is the consumed submenu Play flag. Missing cutscene
    /// data or Play=false still takes the half-second transition path.
    pub fn enter(play: bool, cutscene: Option<&MapCutscene>, has_subtitles: bool) -> Self {
        Self {
            phase: if play && cutscene.is_some() {
                MapPhase::Chapter
            } else {
                MapPhase::FinishPending
            },
            time: 0.0,
            title_remaining: 3.0,
            duration: cutscene.map_or(0.0, |c| c.duration),
            has_subtitles,
        }
    }

    /// ControllerA, or ControllerMouseAction with left button released.
    /// Native mouse handling does not inspect the previous button state.
    pub fn skip(
        &mut self,
        tome_open: bool,
        cue: CueState,
        controller_a: bool,
    ) -> Vec<MapCutsceneEvent> {
        if !tome_open || matches!(self.phase, MapPhase::Complete | MapPhase::Fading) {
            return Vec::new();
        }
        self.phase = MapPhase::FinishPending;
        if (controller_a && cue != CueState::Absent) || cue == CueState::Playing {
            vec![MapCutsceneEvent::StopCue]
        } else {
            Vec::new()
        }
    }

    /// One native Update call. Deliberately does not carry title-page overshoot
    /// into map time, or start the fade in the same frame as natural completion.
    pub fn update(&mut self, dt: f32, cue: CueState) -> Vec<MapCutsceneEvent> {
        use MapCutsceneEvent::*;
        match self.phase {
            MapPhase::FinishPending => {
                self.phase = MapPhase::Fading;
                let mut events = Vec::new();
                if cue == CueState::Playing {
                    events.push(StopCue);
                }
                events.extend([HideSubtitles, FadeToBlack { seconds: 0.5 }]);
                events
            }
            MapPhase::Chapter => {
                self.title_remaining -= dt;
                let mut events = Vec::new();
                if self.title_remaining <= 0.0 {
                    self.phase = MapPhase::Map;
                    events.push(Riffle);
                    if !matches!(cue, CueState::Absent | CueState::Playing) {
                        events.push(PlayCue);
                    }
                    if self.has_subtitles {
                        events.push(ShowSubtitles);
                    }
                }
                events
            }
            MapPhase::Map => {
                self.time += dt;
                if self.time > self.duration
                    && matches!(
                        cue,
                        CueState::Absent | CueState::Stopping | CueState::Stopped
                    )
                {
                    self.phase = MapPhase::FinishPending;
                }
                Vec::new()
            }
            MapPhase::Fading | MapPhase::Complete => Vec::new(),
        }
    }

    /// RenderManager.TransitionEnd: hand off once, after the actual fade ends.
    pub fn transition_end(&mut self) -> Option<MapCutsceneEvent> {
        if self.phase != MapPhase::Fading {
            return None;
        }
        self.phase = MapPhase::Complete;
        Some(MapCutsceneEvent::LoadLevel)
    }
}
