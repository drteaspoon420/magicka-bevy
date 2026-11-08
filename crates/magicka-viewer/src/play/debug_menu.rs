//! Experimental developer menu (`--features debug-ui`, toggled with F1).
//! Not an original feature: it drives the rewrite's session directly
//! (level/scene switching, NPC spawning, teleporting, triggers, cutscene
//! skip, avatar health) for testing. `--debug-menu` opens it at startup.
//! F1 shows the `bevy-toolmode` tool menu (`editor.rs`): spawn browser,
//! tools, and these debug sections as Utilities pages.
//! Debug rendering (`draw.rs`, `--debug-draw`) and render/time toggles
//! (`toggles.rs`, `--debug-render`) work with the menu closed.
use super::PlayAppExt;
use super::{reload, teleport_to_area, NpcAssets, Play};
pub(super) mod animations;
mod draw;
mod entities;
mod player;
mod state;
mod toggles;
pub(super) use animations::AnimationPreview;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

#[derive(Resource)]
struct DebugMenu {
    levels: Vec<String>,
    templates: Vec<String>,
    template: String,
    order: String,
    distance: f32,
    spawn_point: String,
    area_filter: String,
    trigger_filter: String,
    ignore_conditions: bool,
    message: String,
    loadout: player::LoadoutMenu,
    state: state::StateMenu,
    level_filter: String,
    entities: entities::EntityMenu,
}

const ORDERS: &[&str] = &["attack", "idle", "defend", "flee", "wander", "none"];

pub(super) fn plugin(app: &mut App) {
    let args: Vec<_> = std::env::args().collect();
    let value = |name| {
        args.windows(2)
            .find(|a| a[0] == name)
            .map(|a| a[1].as_str())
    };
    let requested = value("--debug-windows").unwrap_or("");
    let page = PAGES
        .iter()
        .find(|(id, _, _)| requested.split(',').any(|v| v == *id))
        .map(|(id, _, _)| id.to_string());
    let open = args.iter().any(|a| a == "--debug-menu") || page.is_some();
    // The editor plugin (added first) owns egui; F1 shows its menu with
    // these pages, drawn by `menu` below.
    app.world_mut()
        .resource_mut::<bevy_toolmode::ToolMode>()
        .menu_open = open;
    if let Some(page) = page {
        app.insert_resource(bevy_toolmode::menu::MenuState {
            right: bevy_toolmode::menu::RightTab::Utilities,
            page: Some(page),
            ..default()
        });
    }
    app.insert_resource(DebugMenu {
        levels: Vec::new(),
        templates: Vec::new(),
        template: "goblin_pirate".into(),
        order: "attack".into(),
        distance: 6.0,
        spawn_point: "start".into(),
        area_filter: String::new(),
        trigger_filter: String::new(),
        ignore_conditions: true,
        message: String::new(),
        loadout: Default::default(),
        level_filter: String::new(),
        entities: Default::default(),
        state: Default::default(),
    })
    .insert_resource(AnimationPreview::from_argument(value("--debug-animation")))
    .add_play_systems(PostStartup, list_content)
    .add_play_systems(Update, (toggle, player::sync_visuals, entities::cleanup))
    .add_play_systems(
        EguiPrimaryContextPass,
        (menu.before(bevy_toolmode::inspector::window), overlays),
    );
    draw::plugin(app, value("--debug-draw"), value("--debug-draw-radius"));
    toggles::plugin(app, value("--debug-render"));
}

/// Labels and text that stay visible with the menu closed.
fn overlays(
    mut contexts: EguiContexts,
    play: Res<Play>,
    draw: Res<draw::DebugDraw>,
    render: Res<toggles::DebugRender>,
    real: Res<Time<Real>>,
    trace: Res<toggles::TriggerTrace>,
    mut fps: Local<f32>,
    cameras: Query<(&Camera, &GlobalTransform), With<super::PlayCamera>>,
) {
    draw::draw_labels(&mut contexts, &draw, &play, &cameras);
    toggles::draw_text(
        &mut contexts,
        &render,
        &play,
        &real,
        &trace,
        &mut fps,
        &cameras,
    );
}

fn list_content(
    play: Res<Play>,
    npcs: Res<NpcAssets>,
    mut menu: ResMut<DebugMenu>,
    colour: Res<crate::robe::RobeColour>,
) {
    menu.levels = match magicka_runtime::play_content::level_files(&play.content) {
        Ok(levels) => levels,
        Err(e) => {
            menu.message = format!("Level catalogue: {e:?}");
            Vec::new()
        }
    };
    if play.scenarios.is_dir() {
        match magicka_runtime::play_content::authored_level_files(&play.scenarios) {
            Ok(levels) => menu.levels.extend(levels),
            Err(e) => menu.message = format!("Custom scenario catalogue: {e:?}"),
        }
    }
    let mut templates: Vec<String> = npcs.files.keys().cloned().collect();
    templates.sort();
    menu.templates = templates;
    menu.loadout = player::LoadoutMenu::load(&play, colour.0);
    menu.entities = entities::EntityMenu::load(&play);
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut menu: ResMut<DebugMenu>,
    mut play: ResMut<Play>,
    network: Option<NonSend<super::network::NetworkPlay>>,
) {
    if keys.just_pressed(KeyCode::F6)
        && !matches!(
            network.as_deref(),
            Some(super::network::NetworkPlay::Client(_))
        )
    {
        spawn_selected(&mut menu, &mut play);
    }
}

/// Utilities pages: (id, label, category). `--debug-windows ID` opens one.
const PAGES: &[(&str, &str, &str)] = &[
    ("diagnostics", "Diagnostics", "Debug"),
    ("levels", "Levels / scenes", "Debug"),
    ("scripts", "Scripts / teleport", "Debug"),
    ("state", "Script state / checkpoint", "Debug"),
    ("player", "Player equipment", "Player"),
    ("animations", "Player animations", "Player"),
    ("enemies", "Enemies (F6)", "Spawning"),
    ("entities", "Other entities", "Spawning"),
];

/// The debug sections behind the tool menu's Utilities tab.
struct DebugHost<'a, 'w, 's> {
    menu: &'a mut DebugMenu,
    play: &'a mut Play,
    network: Option<&'a mut super::network::NetworkPlay>,
    preview: &'a mut AnimationPreview,
    commands: &'a mut Commands<'w, 's>,
    assets: &'a AssetServer,
    cutaway: &'a mut super::cutaway::CutawaySettings,
    cutaway_status: &'a super::cutaway::CutawayStatus,
    debug_draw: &'a mut draw::DebugDraw,
    debug_render: &'a mut toggles::DebugRender,
}

impl bevy_toolmode::MenuHost for DebugHost<'_, '_, '_> {
    fn utility_pages(&self) -> Vec<bevy_toolmode::menu::UtilityPage> {
        PAGES
            .iter()
            .map(|(id, label, category)| bevy_toolmode::menu::UtilityPage {
                id: id.to_string(),
                label: label.to_string(),
                category: category.to_string(),
            })
            .collect()
    }

    fn utility_ui(&mut self, ui: &mut egui::Ui, page: &str) {
        let (menu, play) = (&mut *self.menu, &mut *self.play);
        if self.network.is_some() && !matches!(page, "diagnostics" | "player") {
            ui.label("This demo session exposes diagnostics and player setup. World changes are controlled by the host simulation.");
            return;
        }
        match page {
            "diagnostics" => {
                status(ui, play);
                if self.cutaway.enabled {
                    ui.label(format!("cutaway: {}", self.cutaway_status.label()));
                }
                toggles::section(ui, self.debug_render, self.cutaway);
                draw::section(ui, self.debug_draw);
                avatar_section(ui, menu, play);
            }
            "levels" => level_section(ui, menu, play),
            "player" => player::section(
                ui,
                &mut menu.loadout,
                &mut menu.message,
                play,
                self.network.as_deref_mut(),
            ),
            "animations" => animations::section(ui, self.preview, play),
            "enemies" => spawn_section(ui, menu, play),
            "entities" => entities::section(
                ui,
                &mut menu.entities,
                &mut menu.message,
                play,
                self.commands,
                self.assets,
            ),
            "state" => state::section(ui, &mut menu.state, &mut menu.message, play),
            "scripts" => {
                teleport_section(ui, menu, play);
                ui.separator();
                trigger_section(ui, menu, play);
            }
            _ => {}
        }
        if !menu.message.is_empty() {
            ui.separator();
            ui.label(&menu.message);
        }
    }

    fn tool_ui(&mut self, ui: &mut egui::Ui, _tool: &str) {
        ui.label("Point at a character (or the ground for teleport) and use the primary button.");
    }

    fn live_ui(&mut self, ui: &mut egui::Ui, id: u64) {
        if self.network.is_some() {
            ui.label("Use the dungeon master tools on networked characters.");
            return;
        }
        super::editor::live_ui(ui, self.play, id as u16);
    }
}

#[allow(clippy::too_many_arguments)]
fn menu(
    mut contexts: EguiContexts,
    mut tool_menu: bevy_toolmode::menu::ToolMenu,
    mut menu: ResMut<DebugMenu>,
    mut play: ResMut<Play>,
    mut network: Option<NonSendMut<super::network::NetworkPlay>>,
    mut preview: ResMut<AnimationPreview>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut cutaway: ResMut<super::cutaway::CutawaySettings>,
    cutaway_status: Res<super::cutaway::CutawayStatus>,
    (mut debug_draw, mut debug_render): (ResMut<draw::DebugDraw>, ResMut<toggles::DebugRender>),
) -> Result {
    if !tool_menu.mode.menu_open {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?.clone();
    let mut host = DebugHost {
        menu: &mut menu,
        play: &mut play,
        network: network.as_deref_mut(),
        preview: &mut preview,
        commands: &mut commands,
        assets: &assets,
        cutaway: &mut cutaway,
        cutaway_status: &cutaway_status,
        debug_draw: &mut debug_draw,
        debug_render: &mut debug_render,
    };
    tool_menu.show(&ctx, &mut host);
    Ok(())
}

fn status(ui: &mut egui::Ui, play: &Play) {
    let s = &play.session;
    let p = s.body().position;
    let hp = &s.avatar().character.combat.health;
    ui.label(format!(
        "{} / {}\npos {:.1} {:.1} {:.1}  HP {:.0}/{:.0}\ncutscene {}  input limited {}  NPCs {}",
        s.level_file(),
        s.scene_name(),
        p[0],
        p[1],
        p[2],
        hp.hit_points,
        hp.max_hit_points,
        s.world.cutscene.active,
        s.world.flags.input_limited,
        s.world.npcs.len(),
    ));
    if let Some(e) = &s.script_error {
        ui.colored_label(egui::Color32::LIGHT_RED, format!("scripts: {e}"));
    }
    ui.label(format!(
        "State {:?}, animation {}, time {:.2}s\nVelocity {:?}\nEffects {}, particles {}",
        s.avatar().motion.state,
        s.avatar().motion.controller.clip_name().unwrap_or("none"),
        s.avatar().motion.controller.time,
        s.body().velocity,
        s.effects.active_effects(),
        s.effects.particles.active().len()
    ));
    ui.label(
        "Left mouse: move/dialog; QWER ASDF: elements; right mouse: cast; F5: restart; Esc: quit.",
    );
}

fn level_section(ui: &mut egui::Ui, menu: &mut DebugMenu, play: &mut Play) {
    ui.horizontal(|ui| {
        ui.label("spawn point");
        ui.text_edit_singleline(&mut menu.spawn_point);
    });
    ui.label("Scenes of this level:");
    ui.horizontal_wrapped(|ui| {
        for scene in play.session.level_scene_names() {
            let current = scene == play.session.scene_name();
            if ui.selectable_label(current, &scene).clicked() {
                menu.message = match play.session.change_scene_named(&scene, &menu.spawn_point) {
                    Ok(()) => format!("changed to {scene}"),
                    Err(e) => format!("change scene failed: {e:?}"),
                };
            }
        }
    });
    ui.separator();
    ui.text_edit_singleline(&mut menu.level_filter);
    let filter = menu.level_filter.to_ascii_lowercase();
    let mut load = None;
    egui::ComboBox::from_label("load level")
        .selected_text(play.level.clone())
        .show_ui(ui, |ui| {
            for level in &menu.levels {
                if !level.to_ascii_lowercase().contains(&filter) {
                    continue;
                }
                if ui.selectable_label(*level == play.level, level).clicked() {
                    load = Some(level.clone());
                }
            }
        });
    if let Some(level) = load {
        let previous = std::mem::replace(&mut play.level, level.clone());
        let reloads = play.reloads;
        reload(play);
        if play.reloads == reloads {
            play.level = previous;
            menu.message = format!("could not load {level}");
        } else {
            menu.message = format!("loaded {level}");
        }
    }
    if ui.button("Restart level (F5)").clicked() {
        reload(play);
    }
}

fn spawn_section(ui: &mut egui::Ui, menu: &mut DebugMenu, play: &mut Play) {
    egui::ComboBox::from_label("template")
        .selected_text(menu.template.clone())
        .height(400.0)
        .show_ui(ui, |ui| {
            for t in &menu.templates {
                ui.selectable_value(&mut menu.template, t.clone(), t);
            }
        });
    egui::ComboBox::from_label("order")
        .selected_text(menu.order.clone())
        .show_ui(ui, |ui| {
            for o in ORDERS {
                ui.selectable_value(&mut menu.order, (*o).to_string(), *o);
            }
        });
    ui.add(egui::Slider::new(&mut menu.distance, 1.0..=20.0).text("distance ahead"));
    if ui.button("Spawn selected (F6)").clicked() {
        spawn_selected(menu, play);
    }
}

fn spawn_selected(menu: &mut DebugMenu, play: &mut Play) {
    menu.message = match play
        .session
        .spawn_npc(&menu.template, &menu.order, menu.distance)
    {
        Ok(handle) => format!("spawned {} (#{handle})", menu.template),
        Err(e) => format!("spawn failed: {e}"),
    };
}

fn teleport_section(ui: &mut egui::Ui, menu: &mut DebugMenu, play: &mut Play) {
    ui.text_edit_singleline(&mut menu.area_filter);
    let filter = menu.area_filter.to_ascii_lowercase();
    egui::ScrollArea::vertical()
        .id_salt("areas")
        .max_height(160.0)
        .show(ui, |ui| {
            for area in play.session.world.area_names() {
                if !area.to_ascii_lowercase().contains(&filter) {
                    continue;
                }
                if ui.button(&area).clicked() {
                    teleport_to_area(&mut play.session, &area);
                    menu.message = format!("teleported to {area}");
                }
            }
        });
}

fn trigger_section(ui: &mut egui::Ui, menu: &mut DebugMenu, play: &mut Play) {
    ui.checkbox(&mut menu.ignore_conditions, "ignore conditions");
    ui.text_edit_singleline(&mut menu.trigger_filter);
    let filter = menu.trigger_filter.to_ascii_lowercase();
    egui::ScrollArea::vertical()
        .id_salt("triggers")
        .max_height(200.0)
        .show(ui, |ui| {
            for name in play.session.trigger_names() {
                if !name.to_ascii_lowercase().contains(&filter) {
                    continue;
                }
                if ui.button(&name).clicked() {
                    menu.message = match play.session.run_trigger(&name, menu.ignore_conditions) {
                        Ok(true) => format!("ran {name}"),
                        Ok(false) => format!("{name}: not run (conditions/repeat)"),
                        Err(e) => format!("{name}: {e}"),
                    };
                }
            }
        });
}

fn avatar_section(ui: &mut egui::Ui, menu: &mut DebugMenu, play: &mut Play) {
    ui.horizontal(|ui| {
        if ui.button("Heal").clicked() {
            if let Some(p) = play.session.world.players[0].as_mut() {
                let max = p.character.combat.health.max_hit_points;
                p.character.combat.health.hit_points = max;
                menu.message = "healed".into();
            }
        }
        if ui.button("Kill").clicked() {
            if let Some(p) = play.session.world.players[0].as_mut() {
                p.terminate();
            }
        }
        if ui.button("Unlock all magicks").clicked() {
            let all = magicka_sim::spells::ALL_MAGICKS_MASK;
            for bits in &mut play.session.unlocked_magicks {
                *bits |= all;
            }
            menu.message = "all magicks unlocked (combo then Space)".into();
        }
        if ui.button("Skip cutscene").clicked() {
            // The first SkipCutscene only shows the tip.
            play.session.skip_cutscene();
            play.session.skip_cutscene();
        }
    });
}
