//! Developer menu window "Script state": live trigger flags (fire/reset),
//! level variables, scene flags, the in-memory checkpoint, and a preview of
//! exactly what a save-slot write stores (`PlaySession::disk_save`).
//! Variable and scene IDs are custom hashes; authored names are not kept.
use super::{egui, Play};

#[derive(Default)]
pub(super) struct StateMenu {
    filter: String,
    only_fired: bool,
    /// Pretty `disk_save()` JSON (or its error), refreshed on demand.
    save_preview: Option<String>,
    save_filter: String,
}

fn matches(filter: &str, text: &str) -> bool {
    filter.is_empty()
        || text
            .to_ascii_lowercase()
            .contains(&filter.to_ascii_lowercase())
}

fn json_block(ui: &mut egui::Ui, id: &str, text: &str, filter: &str) {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(260.0)
        .show(ui, |ui| {
            let shown: String = text
                .lines()
                .filter(|l| matches(filter, l))
                .take(4000)
                .collect::<Vec<_>>()
                .join("\n");
            ui.label(egui::RichText::new(shown).monospace().size(10.0));
        });
}

pub(super) fn section(
    ui: &mut egui::Ui,
    menu: &mut StateMenu,
    message: &mut String,
    play: &mut Play,
) {
    ui.horizontal(|ui| {
        ui.label("Filter");
        ui.text_edit_singleline(&mut menu.filter);
        ui.checkbox(&mut menu.only_fired, "fired only");
    });
    let s = &mut play.session;
    match &s.script_error {
        Some(e) => {
            ui.colored_label(egui::Color32::LIGHT_RED, format!("scripts stopped: {e}"));
        }
        None => {
            ui.label(format!("scripts running in {}", s.scene_name()));
        }
    }
    if !s.runtime.skipped.is_empty() {
        let mut names: Vec<_> = s
            .runtime
            .skipped
            .iter()
            .map(|&i| s.runtime.unsupported_name(i).unwrap_or("?").to_owned())
            .collect();
        names.sort();
        names.dedup();
        ui.colored_label(
            egui::Color32::YELLOW,
            format!("skipped unsupported actions: {}", names.join(", ")),
        );
    }

    let triggers = &s.runtime.layout().triggers;
    let fired = triggers.iter().filter(|t| t.has_triggered).count();
    let mut fire: Option<String> = None;
    let mut reset: Option<i32> = None;
    egui::CollapsingHeader::new(format!("Triggers ({fired}/{} fired)", triggers.len()))
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("state_triggers")
                .max_height(260.0)
                .show(ui, |ui| {
                    egui::Grid::new("state_trigger_grid")
                        .num_columns(4)
                        .striped(true)
                        .show(ui, |ui| {
                            for t in triggers {
                                let label = if t.name.is_empty() {
                                    format!("#{:08x}", t.id)
                                } else {
                                    t.name.clone()
                                };
                                if (menu.only_fired && !t.has_triggered)
                                    || !matches(&menu.filter, &label)
                                {
                                    continue;
                                }
                                let colour = if t.has_triggered {
                                    egui::Color32::LIGHT_GREEN
                                } else {
                                    egui::Color32::GRAY
                                };
                                ui.colored_label(colour, &label);
                                ui.label(format!(
                                    "{}{}",
                                    if t.has_triggered { "fired" } else { "-" },
                                    if t.autorun { "" } else { " (manual)" }
                                ));
                                ui.label(if t.repeat >= 0.0 {
                                    format!(
                                        "repeat {:.1}s (next {:.1})",
                                        t.repeat, t.time_till_repeat
                                    )
                                } else {
                                    String::new()
                                });
                                ui.horizontal(|ui| {
                                    if !t.name.is_empty() && ui.small_button("fire").clicked() {
                                        fire = Some(t.name.clone());
                                    }
                                    if t.has_triggered && ui.small_button("reset").clicked() {
                                        reset = Some(t.id);
                                    }
                                });
                                ui.end_row();
                            }
                        });
                });
        });
    if let Some(name) = fire {
        match s.run_trigger(&name, true) {
            Ok(ran) => *message = format!("trigger {name}: ran={ran}"),
            Err(e) => *message = format!("trigger {name}: {e}"),
        }
    }
    if let Some(id) = reset {
        s.runtime.reset_triggers(&[id]);
    }

    let vars = &s.runtime.variables;
    egui::CollapsingHeader::new(format!(
        "Variables ({} counters, {} timers)",
        vars.counters.len(),
        vars.timers.len() + vars.played_timers.len()
    ))
    .show(ui, |ui| {
        for (id, value) in &vars.counters {
            let line = format!("counter #{id:08x} = {value}");
            if matches(&menu.filter, &line) {
                ui.monospace(line);
            }
        }
        for (kind, list) in [
            ("timer", &vars.timers),
            ("played timer", &vars.played_timers),
        ] {
            for t in list {
                ui.monospace(format!(
                    "{kind} #{:08x} = {:.2}{}",
                    t.id,
                    t.time,
                    if t.paused { " (paused)" } else { "" }
                ));
            }
        }
    });

    egui::CollapsingHeader::new("Scene flags").show(ui, |ui| {
        ui.label(
            egui::RichText::new(format!("{:#?}", s.world.flags))
                .monospace()
                .size(10.0),
        );
        ui.monospace(format!(
            "pending triggers {:?}\nscene changes {}  checkpoint generation {}",
            s.world.pending_triggers, s.scene_changes, s.checkpoint_generation
        ));
    });

    egui::CollapsingHeader::new(match &s.checkpoint {
        Some(_) => "Checkpoint (in memory)".to_owned(),
        None => "Checkpoint (none yet)".to_owned(),
    })
    .show(ui, |ui| {
        let Some(cp) = &s.checkpoint else {
            ui.label("No checkpoint stored this level.");
            return;
        };
        ui.monospace(format!(
            "scene #{:08x}  ignored triggers {}",
            cp.scene,
            cp.ignored.len()
        ));
        for (slot, m) in cp.matrices.iter().enumerate() {
            ui.monospace(format!(
                "player {slot} spawn {:.2} {:.2} {:.2}",
                m[12], m[13], m[14]
            ));
        }
        let triggers = &cp.runtime().layout().triggers;
        let fired: Vec<_> = triggers
            .iter()
            .filter(|t| t.has_triggered)
            .map(|t| {
                if t.name.is_empty() {
                    format!("#{:08x}", t.id)
                } else {
                    t.name.clone()
                }
            })
            .collect();
        ui.label(format!(
            "stored triggers fired {}/{}: {}",
            fired.len(),
            triggers.len(),
            fired.join(", ")
        ));
        ui.label(format!(
            "stored counters {}  replayed actions {}  animated parts {}",
            cp.runtime().variables.counters.len(),
            cp.runtime().history_excluding(&cp.ignored).len(),
            cp.animation_count()
        ));
        let players = serde_json::to_string_pretty(cp.players()).unwrap_or_default();
        egui::CollapsingHeader::new("stored players").show(ui, |ui| {
            json_block(ui, "cp_players", &players, &menu.filter);
        });
        if let Some(dialogs) = cp.dialogs() {
            let dialogs = serde_json::to_string_pretty(dialogs).unwrap_or_default();
            egui::CollapsingHeader::new("stored dialogs").show(ui, |ui| {
                json_block(ui, "cp_dialogs", &dialogs, &menu.filter);
            });
        }
    });

    egui::CollapsingHeader::new("Save slot contents (disk_save preview)").show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() {
                menu.save_preview = Some(match s.disk_save() {
                    Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_default(),
                    Err(e) => format!("not saveable now: {e}"),
                });
            }
            ui.label("Filter lines");
            ui.text_edit_singleline(&mut menu.save_filter);
        });
        if let Some(text) = &menu.save_preview {
            json_block(ui, "save_preview", text, &menu.save_filter);
        }
    });
}
