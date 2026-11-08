//! Authored hot-join flow. No avatar/connection exists until confirmation.
use super::local_coop::LobbySeat;
use bevy::prelude::*;
use magicka_runtime::saves::{new_player_id, SaveStore};
use serde_json::{json, Value};

pub(super) struct JoinProfile {
    pub pad: Entity,
    profiles: Vec<Value>,
    profile: usize,
    robes: Vec<String>,
    pub row: usize,
    pub error: Option<String>,
    pub family: crate::controller::Family,
}
impl JoinProfile {
    pub fn new(
        pad: Entity,
        content: &std::path::Path,
        cache: &std::path::Path,
        used: &[String],
        player_number: usize,
        family: crate::controller::Family,
    ) -> Result<Self, String> {
        let settings = SaveStore::new(cache.join("saves")).settings()?;
        let mut profiles: Vec<Value> = settings["profiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| {
                p["id"]
                    .as_str()
                    .is_some_and(|id| !used.iter().any(|u| u == id))
            })
            .cloned()
            .collect();
        let fresh = json!({"id":new_player_id()?, "name":format!("Player {}", player_number),
            "robe":"Wizard", "colour":crate::player_colors::player_color((player_number-1) % crate::player_colors::PLAYER_COLORS.len())});
        profiles.push(fresh);
        let mut robes: Vec<_> = std::fs::read_dir(content.join("Data/Characters"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let name = path.file_stem()?.to_str()?.to_owned();
                (path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("xnb"))
                    && name.to_ascii_lowercase().starts_with("wizard")
                    && cache
                        .join(format!("gltf/Data/Characters/{name}.glb"))
                        .is_file())
                .then_some(name)
            })
            .collect();
        if !robes.iter().any(|r| r == "Wizard") {
            robes.push("Wizard".into());
        }
        robes.sort_by_key(|r| (r != "Wizard", r.to_lowercase()));
        Ok(Self {
            pad,
            profiles,
            profile: 0,
            robes,
            row: 0,
            error: None,
            family,
        })
    }
    pub fn change(&mut self, backwards: bool) {
        let cycle = |i: usize, n: usize| (i + if backwards { n - 1 } else { 1 }) % n;
        let p = &mut self.profiles[self.profile];
        match self.row {
            0 => self.profile = cycle(self.profile, self.profiles.len()),
            1 => {
                let i = self
                    .robes
                    .iter()
                    .position(|r| p["robe"].as_str() == Some(r))
                    .unwrap_or(0);
                p["robe"] = self.robes[cycle(i, self.robes.len())].clone().into();
            }
            2 => {
                let colors = crate::player_colors::PLAYER_COLORS.map(|c| c.map(|v| v / 255.0));
                let c: [f32; 3] = serde_json::from_value(p["colour"].clone()).unwrap_or([1.; 3]);
                let i = colors.iter().position(|v| *v == c).unwrap_or(0);
                p["colour"] = json!(colors[cycle(i, colors.len())]);
            }
            _ => {}
        }
        self.error = None;
    }
    /// Queued editors may have opened before another pad claimed a profile.
    pub fn exclude_used(&mut self, used: &[String]) {
        let selected = self.profiles[self.profile]["id"].clone();
        self.profiles
            .retain(|p| !used.iter().any(|id| p["id"].as_str() == Some(id)));
        // Each editor owns a fresh, unique profile as its final option.
        self.profile = self
            .profiles
            .iter()
            .position(|p| p["id"] == selected)
            .unwrap_or(0);
    }
    pub fn text(&self) -> String {
        let p = &self.profiles[self.profile];
        let color: [f32; 3] = serde_json::from_value(p["colour"].clone()).unwrap_or([1.; 3]);
        let color_index = crate::player_colors::PLAYER_COLORS
            .iter()
            .position(|c| c.map(|v| v / 255.0) == color)
            .unwrap_or(0);
        let [confirm, back, ..] = self.family.face_names();
        let rows = [
            format!("Profile: {}", p["name"].as_str().unwrap_or("Player")),
            format!("Wizard: {}", p["robe"].as_str().unwrap_or("Wizard")),
            format!(
                "Colour: {}",
                [
                    "Red",
                    "Magenta",
                    "Purple",
                    "Blue",
                    "Cyan",
                    "Green",
                    "Lime",
                    "Yellow",
                    "Grey",
                    "Black",
                    "White",
                    "Dark blue",
                    "Brown",
                    "Orange",
                    "Pink"
                ][color_index]
            ),
            "Join game".into(),
        ];
        let rows = rows
            .iter()
            .enumerate()
            .map(|(i, r)| format!("{} {r}", if i == self.row { ">" } else { " " }))
            .collect::<Vec<_>>()
            .join("\n\n");
        format!(
            "Join game\n\n{rows}\n\nD-pad: select / change\n{confirm}: confirm   {back}: cancel{}",
            self.error
                .as_ref()
                .map(|e| format!("\n{e}"))
                .unwrap_or_default()
        )
    }
    pub fn selected(&self) -> Result<LobbySeat, String> {
        let mut p = self.profiles[self.profile].clone();
        // Old profiles may reference a robe whose converted mesh is absent.
        if !self.robes.iter().any(|r| p["robe"].as_str() == Some(r)) {
            p["robe"] = "Wizard".into();
        }
        p["pad"] = self.pad.to_bits().into();
        LobbySeat::from_json(&p)
    }
    pub fn save(&self, cache: &std::path::Path) -> Result<(), String> {
        let seat = self.selected()?;
        let store = SaveStore::new(cache.join("saves"));
        let mut settings = store.settings()?;
        let profiles = settings["profiles"]
            .as_array_mut()
            .ok_or("invalid profiles")?;
        let value = json!({"id":seat.id, "name":seat.name, "robe":seat.robe, "colour":seat.colour});
        if let Some(p) = profiles.iter_mut().find(|p| p["id"] == value["id"]) {
            *p = value;
        } else {
            profiles.push(value);
        }
        store.save_settings(&settings)
    }
}
