//! The tool menu, laid out like Garry's Mod's spawn menu: a spawn browser on
//! the left (catalogue tabs, search, category tree, thumbnail grid, plus a
//! tab of staged entries) and a tool panel on the right (Tools / Utilities
//! tabs, quick filter, categorised list, and the selected item's settings).
//!
//! Games draw it with [`ToolMenu::show`] from their own egui system to add
//! utility pages and custom tool panels through [`MenuHost`].
use crate::{
    catalogue::{CategoryNode, PropertyDef, PropertyKind, PropertyValue},
    gizmo::GizmoMode,
    tools::{self, ToolKind},
    FreeCam, Inspector, Selection, SpawnCatalogue, SpawnRequest, StagedSpawns, ThumbnailSettings,
    Thumbnails, ToolMode, ToolModeSettings, ToolRegistry, HOTBAR_SLOTS,
};
use bevy::{ecs::system::SystemParam, prelude::*};
use bevy_egui::{egui, EguiContexts};

/// A game page in the Utilities tab.
#[derive(Clone, Debug)]
pub struct UtilityPage {
    pub id: String,
    pub label: String,
    pub category: String,
}

/// Game hooks into the menu. Every method has a no-op default.
pub trait MenuHost {
    fn utility_pages(&self) -> Vec<UtilityPage> {
        Vec::new()
    }
    fn utility_ui(&mut self, _ui: &mut egui::Ui, _page: &str) {}
    /// Settings for a [`ToolKind::Custom`] tool.
    fn tool_ui(&mut self, _ui: &mut egui::Ui, _tool: &str) {}
    /// Inspector for a selected live entity.
    fn live_ui(&mut self, _ui: &mut egui::Ui, _id: u64) {}
}

pub struct NoHost;
impl MenuHost for NoHost {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RightTab {
    #[default]
    Tools,
    Utilities,
}

/// Built-in utility page with tool mode settings.
pub const SETTINGS_PAGE: &str = "toolmode/settings";

#[derive(Resource, Default)]
pub struct MenuState {
    /// Catalogue tab, or None for the staged list.
    pub tab: Option<String>,
    /// Category path below the tab.
    pub path: Vec<String>,
    pub search: String,
    pub right: RightTab,
    pub quick_filter: String,
    pub page: Option<String>,
    pub icon_size: f32,
}

#[derive(SystemParam)]
pub struct ToolMenu<'w> {
    pub mode: ResMut<'w, ToolMode>,
    pub state: ResMut<'w, MenuState>,
    pub catalogue: Res<'w, SpawnCatalogue>,
    pub registry: Res<'w, ToolRegistry>,
    pub staged: ResMut<'w, StagedSpawns>,
    pub thumbs: Option<ResMut<'w, Thumbnails>>,
    pub thumb_settings: Res<'w, ThumbnailSettings>,
    pub settings: Res<'w, ToolModeSettings>,
    pub freecam: ResMut<'w, FreeCam>,
    pub spawns: MessageWriter<'w, SpawnRequest>,
    pub inspector: ResMut<'w, Inspector>,
}

pub(crate) fn default_menu(mut contexts: EguiContexts, mut menu: ToolMenu) -> Result {
    if let (true, Ok(ctx)) = (menu.mode.menu_open, contexts.ctx_mut()) {
        menu.show(ctx, &mut NoHost);
    }
    Ok(())
}

/// Short label for a key ("KeyQ" -> "Q").
pub fn key_label(key: KeyCode) -> String {
    let name = format!("{key:?}");
    name.strip_prefix("Key")
        .or_else(|| name.strip_prefix("Digit"))
        .unwrap_or(&name)
        .to_string()
}

fn header(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(egui::Color32::from_rgb(70, 150, 230))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                egui::RichText::new(text)
                    .strong()
                    .color(egui::Color32::WHITE),
            );
        });
}

pub fn property_ui(ui: &mut egui::Ui, def: &PropertyDef, value: &mut PropertyValue) -> bool {
    ui.horizontal(|ui| {
        ui.label(&def.label);
        match (&def.kind, value) {
            (PropertyKind::Bool, PropertyValue::Bool(v)) => ui.checkbox(v, "").changed(),
            (PropertyKind::Float { min, max }, PropertyValue::Float(v)) => {
                ui.add(egui::Slider::new(v, *min..=*max)).changed()
            }
            (PropertyKind::Int { min, max }, PropertyValue::Int(v)) => {
                ui.add(egui::Slider::new(v, *min..=*max)).changed()
            }
            (PropertyKind::Choice(options), PropertyValue::Text(v)) => {
                let mut changed = false;
                egui::ComboBox::from_id_salt(&def.key)
                    .selected_text(v.clone())
                    .show_ui(ui, |ui| {
                        for o in options {
                            changed |= ui.selectable_value(v, o.clone(), o).changed();
                        }
                    });
                changed
            }
            (_, PropertyValue::Text(v)) => ui.text_edit_singleline(v).changed(),
            (_, v) => {
                ui.label(format!("{v:?}"));
                false
            }
        }
    })
    .inner
}

fn tree_ui(
    ui: &mut egui::Ui,
    node: &CategoryNode,
    prefix: &mut Vec<String>,
    path: &mut Vec<String>,
) {
    for child in &node.children {
        prefix.push(child.name.clone());
        let selected = *path == *prefix;
        let label = format!("{} ({})", child.name, child.total);
        if child.children.is_empty() {
            if ui.selectable_label(selected, label).clicked() {
                *path = prefix.clone();
            }
        } else {
            let id = ui.make_persistent_id(prefix.join("/"));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                .show_header(ui, |ui| {
                    if ui.selectable_label(selected, label).clicked() {
                        *path = prefix.clone();
                    }
                })
                .body(|ui| tree_ui(ui, child, prefix, path));
        }
        prefix.pop();
    }
}

impl ToolMenu<'_> {
    pub fn show(&mut self, ctx: &egui::Context, host: &mut dyn MenuHost) {
        let screen = ctx.content_rect();
        let margin = 12.0;
        let top = screen.top() + 36.0;
        let bottom = screen.bottom() - 120.0;
        let split = screen.left() + screen.width() * 0.6;
        let left = egui::Rect::from_min_max(
            egui::pos2(screen.left() + margin, top),
            egui::pos2(split - margin * 0.5, bottom),
        );
        let right = egui::Rect::from_min_max(
            egui::pos2(split + margin * 0.5, top),
            egui::pos2(screen.right() - margin, bottom),
        );
        if self.state.icon_size <= 0.0 {
            self.state.icon_size = 72.0;
        }
        egui::Window::new("Spawn menu")
            .id(egui::Id::new("toolmode_spawn_menu"))
            .fixed_rect(left)
            .collapsible(false)
            .title_bar(false)
            .show(ctx, |ui| self.spawn_panel(ui));
        egui::Window::new("Tools")
            .id(egui::Id::new("toolmode_tool_menu"))
            .fixed_rect(right)
            .collapsible(false)
            .title_bar(false)
            .show(ctx, |ui| self.tool_panel(ui, host));
    }

    fn spawn_panel(&mut self, ui: &mut egui::Ui) {
        let tabs = self.catalogue.tabs();
        if self.state.tab.is_none() && !tabs.is_empty() && self.staged.items.is_empty() {
            self.state.tab = Some(tabs[0].clone());
        }
        ui.horizontal_wrapped(|ui| {
            for tab in &tabs {
                let selected = self.state.tab.as_deref() == Some(tab.as_str());
                if ui.selectable_label(selected, tab).clicked() && !selected {
                    self.state.tab = Some(tab.clone());
                    self.state.path.clear();
                }
            }
            let staged = format!("Staged ({})", self.staged.items.len());
            if ui
                .selectable_label(self.state.tab.is_none(), staged)
                .clicked()
            {
                self.state.tab = None;
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut self.mode.stage_spawns,
                "Stage spawns (inactive until realized)",
            );
            ui.separator();
            ui.checkbox(&mut self.mode.active, "Tool mode");
            ui.checkbox(&mut self.mode.freecam, "Freecam");
        });
        ui.separator();
        let Some(tab) = self.state.tab.clone() else {
            self.staged_list(ui);
            return;
        };
        let tree = self.catalogue.tree(&tab);
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() * 0.3).clamp(140.0, 280.0));
                ui.add(egui::TextEdit::singleline(&mut self.state.search).hint_text("Search..."));
                egui::ScrollArea::vertical()
                    .id_salt("toolmode_tree")
                    .max_height(height - 30.0)
                    .show(ui, |ui| {
                        let all = format!("All {tab} ({})", tree.total);
                        if ui
                            .selectable_label(self.state.path.is_empty(), all)
                            .clicked()
                        {
                            self.state.path.clear();
                        }
                        tree_ui(ui, &tree, &mut Vec::new(), &mut self.state.path);
                    });
            });
            ui.separator();
            ui.vertical(|ui| self.icon_grid(ui, &tab, height));
        });
    }

    fn icon_grid(&mut self, ui: &mut egui::Ui, tab: &str, height: f32) {
        let entries: Vec<(String, String, String, String)> = self
            .catalogue
            .filter(tab, &self.state.path, &self.state.search)
            .iter()
            .map(|e| {
                (
                    e.id.clone(),
                    e.label.clone(),
                    e.description.clone(),
                    e.category.join("/"),
                )
            })
            .collect();
        ui.horizontal(|ui| {
            ui.label(format!("{} items", entries.len()));
            ui.add(egui::Slider::new(&mut self.state.icon_size, 40.0..=128.0).text("icon size"));
            if let Some(thumbs) = &self.thumbs {
                let (queued, _, _) = thumbs.progress();
                if queued > 0 {
                    ui.label(format!("rendering thumbnails: {queued} left"));
                }
            }
        });
        let size = self.state.icon_size;
        egui::ScrollArea::vertical()
            .id_salt("toolmode_icons")
            .max_height(height - 30.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    for (id, label, description, category) in entries {
                        let selected = self.mode.spawnable.as_deref() == Some(id.as_str());
                        let texture = self.thumbs.as_ref().and_then(|t| t.texture(&id));
                        let button = match texture {
                            Some(texture) => egui::Button::image(egui::Image::new(
                                egui::load::SizedTexture::new(texture, [size, size]),
                            )),
                            // No model, or nothing visible to render: a
                            // generated tile in its category's colour.
                            None => egui::Button::new(
                                egui::RichText::new(short(&label, size))
                                    .size(10.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .fill(tile_colour(&category))
                            .wrap(),
                        }
                        .selected(selected)
                        .min_size(egui::vec2(size, size));
                        let response = ui.add_sized([size + 8.0, size + 8.0], button);
                        if ui.is_rect_visible(response.rect) {
                            if let Some(thumbs) = self.thumbs.as_mut() {
                                thumbs.request(&id);
                            }
                        }
                        let response = response.on_hover_text(if description.is_empty() {
                            format!("{label}\n{id}")
                        } else {
                            format!("{label}\n{id}\n{description}")
                        });
                        if response.clicked() {
                            self.mode.spawnable = Some(id.clone());
                            self.mode.tool = tools::SPAWN.into();
                            self.mode.active = true;
                            self.state.right = RightTab::Tools;
                        }
                    }
                });
            });
    }

    fn staged_list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.staged.items.is_empty(),
                    egui::Button::new("Realize all"),
                )
                .clicked()
            {
                if let Some(reason) = self.mode.read_only.clone() {
                    self.mode.message = reason;
                } else {
                    let all = self.staged.realize_all();
                    self.mode.message = format!("Realized {} staged entries", all.len());
                    self.spawns.write_batch(all);
                }
            }
            if ui.button("Clear all").clicked() {
                self.staged.items.clear();
            }
            ui.checkbox(&mut self.staged.hide_ghosts, "Hide model ghosts");
        });
        ui.label("Staged entries exist only in the editor: place, turn and configure them, then realize.");
        let mut realize = None;
        let mut delete = None;
        egui::ScrollArea::vertical()
            .id_salt("toolmode_staged")
            .show(ui, |ui| {
                egui::Grid::new("toolmode_staged_grid")
                    .striped(true)
                    .num_columns(4)
                    .show(ui, |ui| {
                        for s in &self.staged.items {
                            let label = self
                                .catalogue
                                .get(&s.spawnable)
                                .map_or(s.spawnable.as_str(), |e| e.label.as_str());
                            let selected = self.mode.selection == Some(Selection::Staged(s.id));
                            if ui
                                .selectable_label(selected, format!("#{} {label}", s.id))
                                .clicked()
                            {
                                self.mode.selection = Some(Selection::Staged(s.id));
                                self.mode.tool = tools::SELECT.into();
                                self.mode.active = true;
                            }
                            let p = s.transform.translation;
                            ui.label(format!("{:.1} {:.1} {:.1}", p.x, p.y, p.z));
                            ui.label(if s.note.is_empty() {
                                ""
                            } else {
                                s.note.as_str()
                            });
                            ui.horizontal(|ui| {
                                if ui.small_button("realize").clicked() {
                                    realize = Some(s.id);
                                }
                                if ui.small_button("delete").clicked() {
                                    delete = Some(s.id);
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
        if let Some(id) = realize {
            if let Some(reason) = self.mode.read_only.clone() {
                self.mode.message = reason;
            } else if let Some(r) = self.staged.realize(id) {
                self.spawns.write(r);
            }
        }
        if let Some(id) = delete {
            self.staged.remove(id);
        }
    }

    fn tool_panel(&mut self, ui: &mut egui::Ui, host: &mut dyn MenuHost) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.state.right, RightTab::Tools, "Tools");
            ui.selectable_value(&mut self.state.right, RightTab::Utilities, "Utilities");
        });
        ui.separator();
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() * 0.32).clamp(130.0, 220.0));
                ui.add(
                    egui::TextEdit::singleline(&mut self.state.quick_filter)
                        .hint_text("Quick filter..."),
                );
                egui::ScrollArea::vertical()
                    .id_salt("toolmode_tool_list")
                    .max_height(height - 30.0)
                    .show(ui, |ui| match self.state.right {
                        RightTab::Tools => self.tool_list(ui),
                        RightTab::Utilities => self.utility_list(ui, host),
                    });
            });
            ui.separator();
            ui.vertical(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt("toolmode_tool_settings")
                    .max_height(height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.state.right {
                        RightTab::Tools => self.tool_settings(ui, host),
                        RightTab::Utilities => match self.state.page.clone() {
                            Some(page) if page == SETTINGS_PAGE => self.settings_page(ui),
                            Some(page) => host.utility_ui(ui, &page),
                            None => {
                                ui.label("Pick a page on the left.");
                            }
                        },
                    });
            });
        });
    }

    fn matches(&self, text: &str) -> bool {
        let f = self.state.quick_filter.to_ascii_lowercase();
        f.is_empty() || text.to_ascii_lowercase().contains(&f)
    }

    fn tool_list(&mut self, ui: &mut egui::Ui) {
        let categories: Vec<String> = self
            .registry
            .categories()
            .into_iter()
            .map(String::from)
            .collect();
        for category in categories {
            let tools: Vec<(String, String)> = self
                .registry
                .tools
                .iter()
                .filter(|t| t.category == category && self.matches(&t.label))
                .map(|t| (t.id.clone(), t.label.clone()))
                .collect();
            if tools.is_empty() {
                continue;
            }
            header(ui, &category);
            for (id, label) in tools {
                if ui.selectable_label(self.mode.tool == id, label).clicked() {
                    self.mode.tool = id.clone();
                    self.mode.active = true;
                    self.mode.add_to_hotbar(&id);
                }
            }
            ui.add_space(4.0);
        }
    }

    fn utility_list(&mut self, ui: &mut egui::Ui, host: &mut dyn MenuHost) {
        let mut pages = host.utility_pages();
        pages.push(UtilityPage {
            id: SETTINGS_PAGE.into(),
            label: "Tool mode settings".into(),
            category: "Tool mode".into(),
        });
        let mut categories: Vec<String> = Vec::new();
        for p in &pages {
            if !categories.contains(&p.category) {
                categories.push(p.category.clone());
            }
        }
        for category in categories {
            let shown: Vec<&UtilityPage> = pages
                .iter()
                .filter(|p| p.category == category && self.matches(&p.label))
                .collect();
            if shown.is_empty() {
                continue;
            }
            header(ui, &category);
            for p in shown {
                let selected = self.state.page.as_deref() == Some(p.id.as_str());
                if ui.selectable_label(selected, &p.label).clicked() {
                    self.state.page = Some(p.id.clone());
                }
            }
            ui.add_space(4.0);
        }
    }

    fn hotbar_row(&mut self, ui: &mut egui::Ui, tool: &str) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Hotbar:");
            for slot in 0..HOTBAR_SLOTS {
                let here = self.mode.hotbar[slot].as_deref() == Some(tool);
                let key = key_label(self.settings.hotbar_keys[slot]);
                if ui
                    .selectable_label(here, key)
                    .on_hover_text("Put this tool on that key")
                    .clicked()
                {
                    for s in self.mode.hotbar.iter_mut() {
                        if s.as_deref() == Some(tool) {
                            *s = None;
                        }
                    }
                    if !here {
                        self.mode.hotbar[slot] = Some(tool.to_string());
                    }
                }
            }
        });
    }

    fn tool_settings(&mut self, ui: &mut egui::Ui, host: &mut dyn MenuHost) {
        let Some(tool) = self.registry.get(&self.mode.tool).cloned() else {
            ui.label("Pick a tool on the left.");
            return;
        };
        ui.heading(&tool.label);
        ui.label(egui::RichText::new(&tool.help).small());
        self.hotbar_row(ui, &tool.id);
        ui.separator();
        match tool.kind {
            ToolKind::Spawn => self.spawn_settings(ui),
            ToolKind::Select => self.select_settings(ui, host),
            ToolKind::Remove => {
                ui.label(format!("{} staged entries", self.staged.items.len()));
            }
            ToolKind::Realize => {
                ui.label(format!(
                    "{} staged entries waiting",
                    self.staged.items.len()
                ));
                if ui.button("Realize all now").clicked() {
                    if let Some(reason) = self.mode.read_only.clone() {
                        self.mode.message = reason;
                    } else {
                        let all = self.staged.realize_all();
                        self.spawns.write_batch(all);
                    }
                }
            }
            ToolKind::Inspect => {
                ui.checkbox(&mut self.inspector.open, "Show the inspector window");
                ui.label("Reflected components can be edited, added (types with a registered Default) and removed. Unregistered components are listed by name only.");
            }
            ToolKind::Custom => host.tool_ui(ui, &tool.id),
        }
    }

    fn spawn_settings(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(
            &mut self.mode.stage_spawns,
            "Stage (inactive) instead of spawning",
        );
        let mut yaw = self.mode.spawn_yaw.to_degrees();
        if ui
            .add(egui::Slider::new(&mut yaw, 0.0..=359.0).text("facing (deg)"))
            .changed()
        {
            self.mode.spawn_yaw = yaw.to_radians();
        }
        let Some(entry) = self
            .mode
            .spawnable
            .as_ref()
            .and_then(|id| self.catalogue.get(id))
            .cloned()
        else {
            ui.label("Pick a spawnable in the spawn menu.");
            return;
        };
        if let Some(texture) = self.thumbs.as_ref().and_then(|t| t.texture(&entry.id)) {
            ui.image(egui::load::SizedTexture::new(texture, [96.0, 96.0]));
        }
        ui.strong(&entry.label);
        ui.label(egui::RichText::new(&entry.id).small().weak());
        if !entry.description.is_empty() {
            ui.label(&entry.description);
        }
        let mut values = entry.resolve(&self.mode.spawn_properties);
        for def in &entry.properties {
            if let Some(v) = values.get_mut(&def.key) {
                if property_ui(ui, def, v) {
                    self.mode
                        .spawn_properties
                        .insert(def.key.clone(), v.clone());
                }
            }
        }
    }

    fn select_settings(&mut self, ui: &mut egui::Ui, host: &mut dyn MenuHost) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.mode.gizmo, GizmoMode::Universal, "Move + rotate");
            ui.selectable_value(&mut self.mode.gizmo, GizmoMode::Translate, "Move");
            ui.selectable_value(&mut self.mode.gizmo, GizmoMode::Rotate, "Rotate");
        });
        ui.add(egui::Slider::new(&mut self.mode.snap, 0.0..=2.0).text("move snap"));
        ui.add(egui::Slider::new(&mut self.mode.angle_snap, 0.0..=90.0).text("angle snap"));
        ui.separator();
        match self.mode.selection {
            None => {
                ui.label("Nothing selected. Click a staged entry or a selectable entity.");
            }
            Some(Selection::Live(id)) => {
                ui.strong(format!("Live entity #{id}"));
                host.live_ui(ui, id);
            }
            Some(Selection::Staged(id)) => self.staged_inspector(ui, id),
        }
    }

    fn staged_inspector(&mut self, ui: &mut egui::Ui, id: u64) {
        let catalogue = &self.catalogue;
        let Some(s) = self.staged.get_mut(id) else {
            return;
        };
        let entry = catalogue.get(&s.spawnable).cloned();
        ui.strong(format!(
            "Staged #{id}: {}",
            entry
                .as_ref()
                .map_or(s.spawnable.as_str(), |e| e.label.as_str())
        ));
        ui.horizontal(|ui| {
            ui.label("position");
            for v in [
                &mut s.transform.translation.x,
                &mut s.transform.translation.y,
                &mut s.transform.translation.z,
            ] {
                ui.add(egui::DragValue::new(v).speed(0.05));
            }
        });
        let (yaw, _, _) = s.transform.rotation.to_euler(EulerRot::YXZ);
        let mut degrees = yaw.to_degrees();
        if ui
            .add(egui::Slider::new(&mut degrees, -180.0..=180.0).text("facing (deg)"))
            .changed()
        {
            s.transform.rotation = Quat::from_rotation_y(degrees.to_radians());
        }
        if let Some(entry) = &entry {
            for def in &entry.properties {
                let v = s
                    .properties
                    .entry(def.key.clone())
                    .or_insert_with(|| def.default.clone());
                property_ui(ui, def, v);
            }
        }
        ui.horizontal(|ui| {
            ui.label("note");
            ui.text_edit_singleline(&mut s.note);
        });
        let mut realize = false;
        let mut delete = false;
        let mut duplicate = false;
        ui.horizontal(|ui| {
            realize = ui.button("Realize").clicked();
            duplicate = ui.button("Duplicate").clicked();
            delete = ui.button("Delete").clicked();
        });
        if realize {
            if let Some(reason) = self.mode.read_only.clone() {
                self.mode.message = reason;
            } else if let Some(r) = self.staged.realize(id) {
                self.spawns.write(r);
                self.mode.selection = None;
            }
        }
        if duplicate {
            if let Some(copy) = self.staged.duplicate(id, Vec3::X) {
                self.mode.selection = Some(Selection::Staged(copy));
            }
        }
        if delete {
            self.staged.remove(id);
            self.mode.selection = None;
        }
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tool mode");
        let key = |k: Option<KeyCode>| k.map_or("unbound".into(), key_label);
        ui.label(format!(
            "{}: menu   {}: tool mode   {}: freecam",
            key(self.settings.toggle_menu),
            key(self.settings.toggle_tool_mode),
            key(self.settings.toggle_freecam)
        ));
        ui.checkbox(
            &mut self.mode.active,
            "Tool mode (hotbar replaces abilities)",
        );
        ui.checkbox(
            &mut self.mode.freecam,
            "Freecam (camera decoupled from the game)",
        );
        ui.checkbox(&mut self.inspector.open, "Entity inspector window");
        ui.add(
            egui::Slider::new(&mut self.freecam.speed, 0.5..=200.0)
                .logarithmic(true)
                .text("freecam speed"),
        );
        let mut fov = self.freecam.fov.to_degrees();
        if ui
            .add(egui::Slider::new(&mut fov, 20.0..=110.0).text("freecam fov"))
            .changed()
        {
            self.freecam.fov = fov.to_radians();
        }
        ui.label("Freecam: hold right mouse to look and fly with WASD, Q/E down/up, Shift faster, wheel speed. Arrows and PageUp/PageDown always fly.");
        ui.separator();
        ui.heading("Thumbnails");
        if let Some(thumbs) = self.thumbs.as_mut() {
            let (queued, ready, failed) = thumbs.progress();
            ui.label(format!(
                "{ready} ready, {queued} queued, {failed} without a model or failed; {} rendered this run",
                thumbs.rendered
            ));
            match &self.thumb_settings.cache_dir {
                Some(dir) => ui.label(format!("cache: {}", dir.display())),
                None => ui.label("cache: memory only"),
            };
            if ui.button("Regenerate all thumbnails").clicked() {
                thumbs.regenerate(&self.thumb_settings);
            }
        }
    }
}

/// A stable muted colour per category path for generated tiles.
pub fn tile_colour(category: &str) -> egui::Color32 {
    let hash = category.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    let hue = (hash % 360) as f32 / 360.0;
    egui::Color32::from(egui::ecolor::Hsva::new(hue, 0.45, 0.42, 1.0))
}

fn short(label: &str, size: f32) -> String {
    let max = (size / 5.0) as usize * 2;
    if label.chars().count() > max {
        let mut s: String = label.chars().take(max.saturating_sub(1)).collect();
        s.push('…');
        s
    } else {
        label.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_colours_are_stable_per_category() {
        assert_eq!(tile_colour("Effects/Fire"), tile_colour("Effects/Fire"));
        assert_ne!(tile_colour("Effects/Fire"), tile_colour("Effects/Ice"));
    }

    #[test]
    fn key_labels_drop_prefixes() {
        assert_eq!(key_label(KeyCode::KeyQ), "Q");
        assert_eq!(key_label(KeyCode::Digit1), "1");
        assert_eq!(key_label(KeyCode::F2), "F2");
    }
}
