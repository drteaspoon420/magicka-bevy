//! Developer loadout controls; asset conversion stays in the private cache.
use super::{egui, DebugMenu, Play};
use crate::play::{AvatarAssets, AvatarRoot};
use bevy::{gltf::GltfAssetLabel, prelude::*};
use magicka_content::Content;
use std::{collections::HashMap, path::Path};

#[derive(Default)]
pub(super) struct LoadoutMenu {
    robes: Vec<(String, String)>,
    staffs: Vec<(String, String)>,
    weapons: Vec<(String, String)>,
    robe: String,
    name: String,
    network_ready: bool,
    staff: String,
    weapon: String,
    colour: [f32; 3],
    visual: Option<(String, [String; 2])>,
    revision: u64,
}

fn glbs(dir: &Path, root: &Path, files: &mut HashMap<String, String>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            glbs(&path, root, files);
        } else if path.extension().is_some_and(|e| e == "glb") {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(rel.to_ascii_lowercase(), rel);
        }
    }
}

impl LoadoutMenu {
    pub(super) fn load(play: &Play, colour: [f32; 3]) -> Self {
        let mut files = HashMap::new();
        glbs(&play.cache.join("gltf"), &play.cache, &mut files);
        let mut menu = Self {
            robe: play.character.clone(),
            colour,
            ..Default::default()
        };
        for entry in std::fs::read_dir(play.content.join("Data/Characters"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = entry
                .path()
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if !name.to_ascii_lowercase().starts_with("wizard") {
                continue;
            }
            let Ok(template) = magicka_runtime::play_content::load_character(&play.content, &name)
            else {
                continue;
            };
            if ![0, 1]
                .iter()
                .all(|slot| template.equipment.iter().any(|(s, _)| s == slot))
            {
                continue;
            }
            if let Some(file) =
                files.get(&format!("gltf/data/characters/{name}.glb").to_ascii_lowercase())
            {
                menu.robes.push((name, file.clone()));
            }
        }
        for entry in std::fs::read_dir(play.content.join("Data/items/Wizard"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            let Ok(xnb) = magicka_xnb::xnb::parse(&bytes) else {
                continue;
            };
            let Some(Ok(parsed)) = magicka_content::parse_primary(&xnb) else {
                continue;
            };
            let Content::Item(item) = parsed.content else {
                continue;
            };
            let Some(model) = &item.model else {
                continue;
            };
            let name = entry
                .path()
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let model = magicka_content::mesh::resolve_reference(
                &format!("Data/items/Wizard/{name}"),
                model,
            );
            let Some(file) = files.get(&format!("gltf/{model}.glb").to_ascii_lowercase()) else {
                continue;
            };
            let list = if item.weapon_class == 18 {
                &mut menu.staffs
            } else {
                &mut menu.weapons
            };
            list.push((name, file.clone()));
        }
        menu.robes.sort();
        menu.staffs.sort();
        menu.weapons.sort();
        for (slot, item) in &play.session.avatar().items {
            if *slot == 0 {
                menu.weapon = item.definition.name.clone();
            }
            if *slot == 1 {
                menu.staff = item.definition.name.clone();
            }
        }
        // Show the starting equipment too, once its models are exported.
        menu.visual = menu.paths();
        menu
    }
    fn paths(&self) -> Option<(String, [String; 2])> {
        let lookup = |list: &[(String, String)], name: &str| {
            list.iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, p)| p.clone())
        };
        Some((
            lookup(&self.robes, &self.robe)?,
            [
                lookup(&self.weapons, &self.weapon)?,
                lookup(&self.staffs, &self.staff)?,
            ],
        ))
    }
}

fn select(ui: &mut egui::Ui, label: &str, current: &mut String, list: &[(String, String)]) {
    egui::ComboBox::from_label(label)
        .selected_text(current.as_str())
        .height(300.0)
        .show_ui(ui, |ui| {
            for (name, _) in list {
                ui.selectable_value(current, name.clone(), name);
            }
        });
}

pub(super) fn section(
    ui: &mut egui::Ui,
    menu: &mut LoadoutMenu,
    message: &mut String,
    play: &mut Play,
    network: Option<&mut crate::play::network::NetworkPlay>,
) {
    if let Some(network) = network {
        if !menu.network_ready {
            let profile = network.profile(play);
            menu.name = profile.name;
            menu.robe = profile.robe;
            menu.colour = profile.colour;
            menu.network_ready = true;
        }
        ui.label("Name");
        ui.text_edit_singleline(&mut menu.name);
        select(ui, "Robe", &mut menu.robe, &menu.robes);
        ui.horizontal(|ui| {
            ui.label("Colour");
            ui.color_edit_button_rgb(&mut menu.colour);
        });
        if ui.button("Apply player setup").clicked() {
            let profile = magicka_network::session::PlayerProfile {
                name: menu.name.clone(),
                robe: menu.robe.clone(),
                colour: menu.colour,
            };
            *message = match network.setup(profile, play) {
                Ok(()) => "Player setup sent to host".into(),
                Err(e) => e,
            };
        }
        ui.label("The server owns equipment, unlocks and respawns.");
        let frame = match network {
            crate::play::network::NetworkPlay::Client(client) => {
                ui.label(&client.status);
                Some(client.frame.clone())
            }
            crate::play::network::NetworkPlay::Host(host) => host.frame(&play.session).ok(),
        };
        if let Some(frame) = frame {
            ui.separator();
            for player in frame.players {
                ui.label(format!(
                    "Player {}: {} — {}",
                    player.slot + 1,
                    player.profile.name,
                    player.profile.robe
                ));
            }
        }
        return;
    }
    select(ui, "Robe", &mut menu.robe, &menu.robes);
    select(ui, "Staff", &mut menu.staff, &menu.staffs);
    select(ui, "Weapon", &mut menu.weapon, &menu.weapons);
    ui.horizontal(|ui| {
        ui.label("Colour");
        ui.color_edit_button_rgb(&mut menu.colour);
    });
    ui.label("Apply resets the player in place. Colour updates the preview immediately.");
    if menu.staffs.is_empty() || menu.weapons.is_empty() {
        ui.label("Run ./demo.sh to export equipment models.");
    }
    if ui.button("Apply equipment").clicked() {
        let Some(paths) = menu.paths() else {
            *message = "Selected equipment has no exported model.".into();
            return;
        };
        match play
            .session
            .debug_player_loadout(&menu.robe, &menu.staff, &menu.weapon)
        {
            Ok(()) => {
                play.character = menu.robe.clone();
                play.debug_loadout =
                    Some((menu.robe.clone(), menu.staff.clone(), menu.weapon.clone()));
                play.pending = None;
                menu.visual = Some(paths);
                menu.revision += 1;
                *message = "Player equipment applied.".into();
            }
            Err(e) => *message = format!("Equipment failed: {e:?}"),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn sync_visuals(
    mut commands: Commands,
    menu: Res<DebugMenu>,
    server: Res<AssetServer>,
    mut avatar: ResMut<AvatarAssets>,
    roots: Query<Entity, With<AvatarRoot>>,
    mut colour: ResMut<crate::robe::RobeColour>,
    mut revision: Local<Option<u64>>,
    network: Option<NonSend<crate::play::network::NetworkPlay>>,
) {
    if network.is_some() {
        return;
    }
    let menu = &menu.loadout;
    if *revision != Some(menu.revision) {
        if let Some((robe, _items)) = &menu.visual {
            for root in &roots {
                commands.entity(root).despawn();
            }
            avatar.gltf = server.load(robe.clone());
            avatar.scene = server.load(GltfAssetLabel::Scene(0).from_asset(robe.clone()));
            avatar.graph = None;
            commands.spawn((
                AvatarRoot,
                Transform::default(),
                Visibility::default(),
                WorldAssetRoot(avatar.scene.clone()),
            ));
        }
        *revision = Some(menu.revision);
    }
    colour.0 = menu.colour;
}
