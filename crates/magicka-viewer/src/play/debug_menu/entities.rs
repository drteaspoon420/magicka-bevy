//! Live effect spawning and decorative GLB placement; unsupported hosts are explicit.
use super::{egui, Play};
use bevy::{gltf::GltfAssetLabel, prelude::*};
use magicka_runtime::particles::EffectRef;
use std::path::Path;

#[derive(Component)]
pub(super) struct DebugProp;

#[derive(Default)]
pub(super) struct EntityMenu {
    kind: usize,
    filter: String,
    selected: String,
    distance: f32,
    dynamic_pickups: bool,
    effects: Vec<(String, i32)>,
    props: Vec<String>,
    physics: Vec<String>,
    pickups: Vec<String>,
    refs: Vec<EffectRef>,
    spawned_props: Vec<(u16, u64)>,
    session: Option<(u64, u64)>,
}

fn files(root: &Path, directory: &Path, extension: &str, result: &mut Vec<String>) {
    for entry in std::fs::read_dir(directory).into_iter().flatten().flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            files(root, &path, extension, result);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(extension))
        {
            if let Ok(relative) = path.strip_prefix(root) {
                result.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    result.sort();
}

impl EntityMenu {
    pub(super) fn load(play: &Play) -> Self {
        let mut menu = Self {
            distance: 4.0,
            ..default()
        };
        menu.effects = play
            .session
            .effects
            .library
            .effects
            .iter()
            .map(|(&hash, e)| (e.name.clone(), hash))
            .collect();
        menu.effects.sort();
        files(
            &play.cache,
            &play.cache.join("gltf/Models"),
            "glb",
            &mut menu.props,
        );
        menu.props.retain(|p| {
            !["/Characters/", "/Effects/", "/Missiles/"]
                .iter()
                .any(|v| p.contains(v))
        });
        files(
            &play.content.join("Data/PhysicsEntities"),
            &play.content.join("Data/PhysicsEntities"),
            "xnb",
            &mut menu.physics,
        );
        files(
            &play.content.join("Data/items"),
            &play.content.join("Data/items"),
            "xnb",
            &mut menu.pickups,
        );
        menu.pickups.retain(|name| name.starts_with("Wizard/"));
        menu
    }
}

pub(super) fn section(
    ui: &mut egui::Ui,
    menu: &mut EntityMenu,
    message: &mut String,
    play: &mut Play,
    commands: &mut Commands,
    assets: &AssetServer,
) {
    // References belong to one EffectSystem, and cannot cross a scene reload.
    let key = (play.reloads as u64, play.session.scene_changes as u64);
    if menu.session != Some(key) {
        menu.refs.clear();
        menu.spawned_props.clear();
        menu.session = Some(key);
    }
    menu.refs.retain(|r| play.session.effects.is_active(r));
    egui::ComboBox::from_label("Entity type")
        .selected_text(
            [
                "Particles",
                "Static prop preview",
                "Physics props",
                "Item pickups",
                "Magick books",
            ][menu.kind],
        )
        .show_ui(ui, |ui| {
            for (i, label) in [
                "Particles",
                "Static prop preview",
                "Physics props",
                "Item pickups",
                "Magick books",
            ]
            .iter()
            .enumerate()
            {
                if ui.selectable_value(&mut menu.kind, i, *label).changed() {
                    menu.selected.clear();
                }
            }
        });
    ui.text_edit_singleline(&mut menu.filter);
    let filter = menu.filter.to_ascii_lowercase();
    let names: Vec<&str> = match menu.kind {
        0 => menu.effects.iter().map(|(name, _)| name.as_str()).collect(),
        1 => menu.props.iter().map(String::as_str).collect(),
        2 => menu.physics.iter().map(String::as_str).collect(),
        3 => menu.pickups.iter().map(String::as_str).collect(),
        4 => magicka_runtime::magick_books::BOOK_MAGICKS
            .iter()
            .copied()
            .filter(|n| !n.is_empty() && *n != "none")
            .collect(),
        _ => Vec::new(),
    };
    egui::ScrollArea::vertical()
        .id_salt("entity catalogue")
        .max_height(220.0)
        .show(ui, |ui| {
            for name in names {
                if name.to_ascii_lowercase().contains(&filter) {
                    ui.selectable_value(&mut menu.selected, name.to_owned(), name);
                }
            }
        });
    ui.add(egui::Slider::new(&mut menu.distance, 1.0..=20.0).text("Distance ahead"));
    match menu.kind {
        1 => {
            ui.label("Decorative preview: no collision or interaction.");
        }
        2 => {
            ui.label("Props use template mass and collision data in Avian. Character contacts, damage effects and animated props remain pending.");
        }
        3 => {
            ui.label("Left-click equipment to collect it; hold click to approach. Dropped equipment uses Avian physics.");
        }
        4 => {
            ui.label("Left-click books to unlock their magick for this session. Save profiles are pending.");
            let unlocked: Vec<_> = magicka_runtime::magick_books::BOOK_MAGICKS
                .iter()
                .enumerate()
                .filter(|(i, n)| {
                    !n.is_empty() && play.session.unlocked_magicks[0] & (1u64 << i) != 0
                })
                .map(|(_, name)| *name)
                .collect();
            ui.label(format!("Unlocked: {}", unlocked.join(", ")));
            if ui
                .button("Give all magicks (including semi-custom)")
                .clicked()
            {
                for bits in &mut play.session.unlocked_magicks {
                    *bits |= magicka_sim::spells::ALL_MAGICKS_MASK;
                }
            }
        }
        _ => {}
    }
    if menu.kind == 3 || menu.kind == 4 {
        ui.checkbox(&mut menu.dynamic_pickups, "Movable pickup (Avian physics)");
    }
    if ui
        .add_enabled(
            !menu.selected.is_empty(),
            egui::Button::new("Spawn selected"),
        )
        .clicked()
    {
        let body = play.session.body();
        let direction = body.direction();
        let position = std::array::from_fn(|i| body.position[i] + direction[i] * menu.distance);
        if menu.kind == 0 {
            let hash = menu
                .effects
                .iter()
                .find(|(name, _)| *name == menu.selected)
                .unwrap()
                .1;
            *message = match play.session.effects.start_at(hash, position, direction) {
                Some(reference) => {
                    menu.refs.push(reference);
                    format!("Started {}", menu.selected)
                }
                None => "Effect pool full or definition unavailable".into(),
            };
        } else if menu.kind == 2 {
            let mut transform = magicka_content::skinned::IDENTITY;
            transform[12..15].copy_from_slice(&position);
            *message = match play.session.spawn_physics_prop(&menu.selected, transform) {
                Ok(handle) => {
                    menu.spawned_props
                        .push((handle, play.session.world.physics_generations[&handle]));
                    format!("Spawned physics prop {} ({handle})", menu.selected)
                }
                Err(error) => format!("Physics prop spawn failed: {error:?}"),
            };
        } else if menu.kind == 4 {
            let mut transform = magicka_content::skinned::IDENTITY;
            transform[12..15].copy_from_slice(&position);
            if let Some(magick) = magicka_runtime::magick_books::magick_id(&menu.selected) {
                *message = match play.session.spawn_magick_book(magick, transform) {
                    Ok(handle) => {
                        let book = play.session.world.book_pickups.get_mut(&handle).unwrap();
                        book.immovable = !menu.dynamic_pickups;
                        book.body_enabled = menu.dynamic_pickups;
                        format!("Spawned {} book ({handle})", menu.selected)
                    }
                    Err(e) => format!("Book spawn failed: {e:?}"),
                };
            }
        } else if menu.kind == 3 {
            let mut transform = magicka_content::skinned::IDENTITY;
            transform[12..15].copy_from_slice(&position);
            let name = menu
                .selected
                .strip_prefix("Wizard/")
                .unwrap_or(&menu.selected)
                .trim_end_matches(".xnb");
            *message = match play.session.spawn_item_pickup(name, transform) {
                Ok(handle) => {
                    play.session
                        .world
                        .item_pickups
                        .get_mut(&handle)
                        .unwrap()
                        .immovable = !menu.dynamic_pickups;
                    format!("Spawned equipment pickup {name} ({handle})")
                }
                Err(e) => format!("Item spawn failed: {e:?}"),
            };
        } else {
            commands.spawn((
                DebugProp,
                WorldAssetRoot(
                    assets.load(GltfAssetLabel::Scene(0).from_asset(menu.selected.clone())),
                ),
                Transform::from_translation(Vec3::from(position)),
            ));
            *message = format!("Placed decorative preview {}", menu.selected);
        }
    }
    if menu.kind == 4 && ui.button("Pick up nearby book").clicked() {
        *message = match play.session.nearby_book() {
            Some(handle) => match play.session.pickup_book(handle) {
                Ok(magick) => format!(
                    "Unlocked {}",
                    magicka_runtime::magick_books::BOOK_MAGICKS[magick as usize]
                ),
                Err(e) => format!("Book pickup failed: {e:?}"),
            },
            None => "No magick book in range".into(),
        };
    }
    if menu.kind == 3 && ui.button("Pick up nearby item").clicked() {
        *message = match play.session.nearby_item() {
            Some(handle) => match play.session.pickup_item(handle) {
                Ok(result) => format!("Pickup {handle}: {result:?}"),
                Err(e) => format!("Pickup failed: {e:?}"),
            },
            None => "No pickable item in range".into(),
        };
    }
    if ui.button("Stop spawned particles").clicked() {
        for mut reference in menu.refs.drain(..) {
            play.session.effects.stop(&mut reference);
        }
    }
    if ui.button("Remove spawned physics props").clicked() {
        for (handle, generation) in menu.spawned_props.drain(..) {
            if play.session.world.physics_generations.get(&handle) != Some(&generation) {
                continue;
            }
            if let Some(e) = play
                .session
                .world
                .entities
                .iter_mut()
                .find(|e| e.handle == handle)
            {
                e.removed = true;
            }
            if let Some(physics) = &mut play.session.pickup_physics {
                physics.remove(handle);
            }
        }
    }
    if ui.button("Remove prop previews").clicked() {
        commands.queue(|world: &mut World| {
            let entities: Vec<_> = world
                .query_filtered::<Entity, With<DebugProp>>()
                .iter(world)
                .collect();
            for entity in entities {
                world.despawn(entity);
            }
        });
    }
}

pub(super) fn cleanup(
    mut commands: Commands,
    play: Res<Play>,
    props: Query<Entity, With<DebugProp>>,
    mut seen: Local<Option<(usize, usize)>>,
) {
    let key = (play.reloads as usize, play.session.scene_changes as usize);
    if seen.is_some_and(|old| old != key) {
        for entity in &props {
            commands.entity(entity).despawn();
        }
    }
    *seen = Some(key);
}
