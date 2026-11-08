//! Tool mode HUD: the hotbar that stands in for the game's ability bar,
//! a status line, and labels over staged entries.
use crate::{
    menu::key_label, Selection, SpawnCatalogue, StagedSpawns, ToolCamera, ToolMode,
    ToolModeSettings, ToolRegistry,
};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

pub(crate) fn draw(
    mut contexts: EguiContexts,
    mode: Res<ToolMode>,
    settings: Res<ToolModeSettings>,
    registry: Res<ToolRegistry>,
    catalogue: Res<SpawnCatalogue>,
    staged: Res<StagedSpawns>,
    cameras: Query<(&Camera, &Transform), With<ToolCamera>>,
) -> Result {
    if !settings.hud || !(mode.active || mode.freecam) {
        return Ok(());
    }
    // No egui context yet (or no window): nothing to draw on.
    let Ok(ctx) = contexts.ctx_mut() else {
        return Ok(());
    };
    let screen = ctx.content_rect();
    if mode.active {
        egui::Area::new(egui::Id::new("toolmode_hotbar"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -12.0])
            .interactable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (slot, tool) in mode.hotbar.iter().enumerate() {
                        let selected = tool.as_deref() == Some(mode.tool.as_str());
                        let label = tool
                            .as_deref()
                            .and_then(|t| registry.get(t))
                            .map_or("", |t| t.label.as_str());
                        let fill = if selected {
                            egui::Color32::from_rgb(60, 130, 210)
                        } else {
                            egui::Color32::from_black_alpha(170)
                        };
                        egui::Frame::new()
                            .fill(fill)
                            .corner_radius(4)
                            .inner_margin(6)
                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(60)))
                            .show(ui, |ui| {
                                ui.set_min_size(egui::vec2(78.0, 40.0));
                                ui.set_max_width(78.0);
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(key_label(settings.hotbar_keys[slot]))
                                            .small()
                                            .color(egui::Color32::from_gray(200)),
                                    );
                                    ui.label(
                                        egui::RichText::new(label).color(egui::Color32::WHITE),
                                    );
                                });
                            });
                    }
                });
            });
    }
    // Status line.
    let mut parts = Vec::new();
    if mode.active {
        let tool = registry
            .get(&mode.tool)
            .map_or(mode.tool.as_str(), |t| t.label.as_str());
        parts.push(format!("Tool mode: {tool}"));
        if mode.tool == crate::tools::SPAWN {
            let what = mode
                .spawnable
                .as_deref()
                .and_then(|id| catalogue.get(id))
                .map_or("nothing selected", |e| e.label.as_str());
            parts.push(format!(
                "{}{what}",
                if mode.stage_spawns {
                    "staging "
                } else {
                    "spawning "
                }
            ));
        }
        if mode.tool == crate::tools::SELECT {
            parts.push(mode.gizmo.label().into());
        }
    }
    if mode.freecam {
        parts.push("freecam (hold right mouse: look + WASD/QE)".into());
    }
    if !staged.items.is_empty() {
        parts.push(format!("{} staged", staged.items.len()));
    }
    if let Some(reason) = &mode.read_only {
        parts.push(reason.clone());
    }
    egui::Area::new(egui::Id::new("toolmode_status"))
        .anchor(
            egui::Align2::CENTER_BOTTOM,
            [0.0, if mode.active { -76.0 } else { -12.0 }],
        )
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_black_alpha(150))
                .corner_radius(4)
                .inner_margin(egui::Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    // Fixed width so changing text does not move the anchor.
                    ui.set_width(560.0);
                    ui.label(egui::RichText::new(parts.join("  |  ")).color(egui::Color32::WHITE));
                    if !mode.message.is_empty() {
                        ui.label(
                            egui::RichText::new(&mode.message)
                                .small()
                                .color(egui::Color32::from_rgb(255, 230, 140)),
                        );
                    }
                });
        });
    // Staged entry labels.
    let Some((camera, t)) = cameras.iter().next() else {
        return Ok(());
    };
    let view = GlobalTransform::from(*t);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("toolmode_labels"),
    ));
    for s in &staged.items {
        let Ok(p) = camera.world_to_viewport(&view, s.transform.translation + Vec3::Y * 2.4) else {
            continue;
        };
        let pos = egui::pos2(p.x, p.y);
        if !screen.contains(pos) {
            continue;
        }
        let label = catalogue
            .get(&s.spawnable)
            .map_or(s.spawnable.as_str(), |e| e.label.as_str());
        let selected = mode.selection == Some(Selection::Staged(s.id));
        painter.text(
            pos,
            egui::Align2::CENTER_BOTTOM,
            format!("{label} (inactive #{})", s.id),
            egui::FontId::proportional(12.0),
            if selected {
                egui::Color32::from_rgb(255, 240, 80)
            } else {
                egui::Color32::from_rgb(120, 220, 255)
            },
        );
    }
    Ok(())
}
