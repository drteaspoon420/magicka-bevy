//! Render and time debug toggles (developer menu, Diagnostics → Render /
//! time toggles) and the on-screen stats/camera text. Every toggle is also a
//! `--debug-render a,b,...` startup option. Presentation-only except `pause`
//! and `slowmo`, which scale the virtual clock the session steps with.
use super::super::{
    cutaway::CutawaySettings, lifecycle::PlayAppExt, AvatarRoot, LevelRoot, MissileRoot, NpcRoot,
    Play, PlayCamera, PlaySpot, PlaySun,
};
use super::egui;
use crate::terrain::{TerrainMaterial, TerrainNoDiscard, TerrainUnlit, NO_DISCARD, UNLIT};
use bevy::prelude::*;
use bevy_egui::EguiContexts;

/// Option names accepted by `--debug-render` and shown in the menu.
pub(super) const TOGGLES: [(&str, &str); 17] = [
    ("cutaway", "Cutaway walls"),
    ("cutaway-tint", "Cutaway: tint instead of cut"),
    ("unlit", "Unlit terrain (albedo only)"),
    ("no-discard", "Terrain: no alpha discard"),
    ("no-shadows", "No sun/spot shadows"),
    ("no-bloom", "No bloom"),
    ("hide-level", "Hide level geometry"),
    ("hide-npcs", "Hide NPCs"),
    ("hide-player", "Hide player"),
    ("hide-missiles", "Hide missiles"),
    ("pause", "Pause simulation"),
    ("slowmo", "Slow motion (x0.25)"),
    ("stats", "Stats text (FPS, scene, counts)"),
    ("camera-info", "Camera text"),
    ("player-info", "Player text"),
    ("scripts", "Script text (errors, fired triggers)"),
    ("trace-triggers", "Log each trigger as it fires"),
];

#[derive(Resource, Clone, Debug, PartialEq)]
pub(in crate::play) struct DebugRender {
    pub(in crate::play) on: [bool; TOGGLES.len()],
    /// Single-step request while paused (frames left to run).
    step: u32,
}

impl DebugRender {
    fn from_argument(value: Option<&str>) -> Self {
        let requested = value.unwrap_or("");
        for name in requested.split(',').filter(|v| !v.is_empty()) {
            if !TOGGLES.iter().any(|(n, _)| *n == name) {
                let known: Vec<_> = TOGGLES.iter().map(|(n, _)| *n).collect();
                warn!(
                    "--debug-render: unknown option {name:?}; known: {}",
                    known.join(", ")
                );
            }
        }
        Self {
            on: TOGGLES.map(|(name, _)| requested.split(',').any(|v| v == name)),
            step: 0,
        }
    }

    pub(in crate::play) fn get(&self, name: &str) -> bool {
        TOGGLES
            .iter()
            .position(|(n, _)| *n == name)
            .is_some_and(|i| self.on[i])
    }

    fn set(&mut self, name: &str, value: bool) {
        if let Some(i) = TOGGLES.iter().position(|(n, _)| *n == name) {
            self.on[i] = value;
        }
    }
}

pub(super) fn plugin(app: &mut App, argument: Option<&str>) {
    app.insert_resource(DebugRender::from_argument(argument))
        .add_play_systems(
            Update,
            (
                sync_cutaway,
                sync_terrain,
                sync_time,
                no_shadows.after(super::super::sync_level_lights),
                no_bloom.after(super::super::sync_post),
                trace_triggers.after(super::super::step_session),
            ),
        )
        .init_play_resource::<TriggerTrace>()
        .add_play_systems(
            PostUpdate,
            hide_roots.before(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
        );
}

/// Menu section (Diagnostics window).
pub(super) fn section(
    ui: &mut egui::Ui,
    settings: &mut DebugRender,
    cutaway: &mut CutawaySettings,
) {
    // The cutaway's own settings stay the source of truth.
    settings.set("cutaway", cutaway.enabled);
    settings.set("cutaway-tint", cutaway.debug_view);
    egui::CollapsingHeader::new("Render / time toggles")
        .default_open(settings.on.iter().any(|v| *v))
        .show(ui, |ui| {
            for (i, (name, label)) in TOGGLES.iter().enumerate() {
                ui.checkbox(&mut settings.on[i], *label)
                    .on_hover_text(format!("--debug-render {name}"));
            }
            if settings.get("pause") && ui.button("Step one frame").clicked() {
                settings.step = 1;
            }
            let names: Vec<_> = TOGGLES.iter().map(|(n, _)| *n).collect();
            ui.label(format!("Startup: --debug-render {}", names.join(",")));
        });
    cutaway.enabled = settings.get("cutaway");
    cutaway.debug_view = settings.get("cutaway-tint");
}

/// Startup flags reach the cutaway; afterwards the menu keeps both in step.
fn sync_cutaway(
    settings: Res<DebugRender>,
    mut cutaway: ResMut<CutawaySettings>,
    mut started: Local<bool>,
) {
    if *started {
        return;
    }
    *started = true;
    if settings.get("cutaway") {
        cutaway.enabled = true;
    }
    if settings.get("cutaway-tint") {
        cutaway.enabled = true;
        cutaway.debug_view = true;
    }
}

/// Live `TerrainUnlit` / `TerrainNoDiscard`: new materials read the
/// resources, existing ones get their flag bits rewritten on change.
fn sync_terrain(
    settings: Res<DebugRender>,
    mut unlit: ResMut<TerrainUnlit>,
    mut no_discard: ResMut<TerrainNoDiscard>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    mut applied: Local<Option<(bool, bool)>>,
) {
    let wanted = (settings.get("unlit"), settings.get("no-discard"));
    let current = *applied.get_or_insert((unlit.0, no_discard.0));
    if current == wanted {
        return;
    }
    *applied = Some(wanted);
    unlit.0 = wanted.0;
    no_discard.0 = wanted.1;
    let ids: Vec<_> = materials.iter().map(|(id, _)| id).collect();
    for id in ids {
        let Some(material) = materials.get_mut(id) else {
            continue;
        };
        let material = material.into_inner();
        for (bit, on) in [(UNLIT, wanted.0), (NO_DISCARD, wanted.1)] {
            if on {
                material.params.flags.x |= bit;
            } else {
                material.params.flags.x &= !bit;
            }
        }
    }
}

fn sync_time(mut settings: ResMut<DebugRender>, mut time: ResMut<Time<Virtual>>) {
    let speed = if settings.get("slowmo") { 0.25 } else { 1.0 };
    if time.relative_speed() != speed {
        time.set_relative_speed(speed);
    }
    let pause = settings.get("pause") && settings.step == 0;
    if settings.step > 0 {
        settings.step -= 1;
    }
    if pause != time.is_paused() {
        if pause {
            time.pause();
        } else {
            time.unpause();
        }
    }
}

/// Overrides `sync_level_lights`, which rewrites both every frame.
fn no_shadows(
    settings: Res<DebugRender>,
    mut lights: ResMut<crate::lighting::MagickaLights>,
    mut sun: Query<&mut DirectionalLight, With<PlaySun>>,
    mut spot: Query<&mut SpotLight, With<PlaySpot>>,
) {
    if !settings.get("no-shadows") {
        return;
    }
    lights.sun_shadow = false;
    for mut light in &mut sun {
        light.shadow_maps_enabled = false;
    }
    for mut light in &mut spot {
        light.shadow_maps_enabled = false;
    }
}

/// Overrides `sync_post`'s bloom multiplier.
fn no_bloom(settings: Res<DebugRender>, mut post: Query<&mut crate::post::MagickaPost>) {
    if !settings.get("no-bloom") {
        return;
    }
    for mut p in &mut post {
        if p.bloom.y != 0.0 {
            p.bloom.y = 0.0;
        }
    }
}

type Roots<'w, 's, T> = Query<'w, 's, &'static mut Visibility, With<T>>;

/// Hide whole root groups. Only roots this system hid are shown again.
#[allow(clippy::type_complexity)]
fn hide_roots(
    settings: Res<DebugRender>,
    mut hidden: Local<[bool; 4]>,
    mut roots: ParamSet<(
        Roots<LevelRoot>,
        Roots<NpcRoot>,
        Roots<AvatarRoot>,
        Roots<MissileRoot>,
    )>,
) {
    let wanted =
        ["hide-level", "hide-npcs", "hide-player", "hide-missiles"].map(|n| settings.get(n));
    for (i, hide) in wanted.into_iter().enumerate() {
        if !hide && !hidden[i] {
            continue;
        }
        let target = if hide {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        let mut apply = |mut v: Mut<Visibility>| {
            if *v != target {
                *v = target;
            }
        };
        match i {
            0 => roots.p0().iter_mut().for_each(&mut apply),
            1 => roots.p1().iter_mut().for_each(&mut apply),
            2 => roots.p2().iter_mut().for_each(&mut apply),
            _ => roots.p3().iter_mut().for_each(&mut apply),
        }
        hidden[i] = hide;
    }
}

/// Triggers seen firing, newest last (shown by the `scripts` text).
#[derive(Resource, Default)]
pub(in crate::play) struct TriggerTrace {
    scene: Option<(u32, u32)>,
    fired: Vec<bool>,
    recent: Vec<String>,
}

fn trigger_label(t: &magicka_runtime::trigger::Trigger) -> String {
    if t.name.is_empty() {
        format!("#{:#x}", t.id)
    } else {
        t.name.clone()
    }
}

/// Diff `has_triggered` each frame (scene changes and reloads restart it).
fn trace_triggers(settings: Res<DebugRender>, play: Res<Play>, mut trace: ResMut<TriggerTrace>) {
    if !settings.get("trace-triggers") && !settings.get("scripts") {
        return;
    }
    let triggers = &play.session.runtime.layout().triggers;
    let scene = Some((play.reloads, play.session.scene_changes));
    if trace.scene != scene || trace.fired.len() != triggers.len() {
        trace.scene = scene;
        trace.fired = triggers.iter().map(|t| t.has_triggered).collect();
        trace.recent.clear();
        return;
    }
    for (i, t) in triggers.iter().enumerate() {
        // Repeating triggers keep the flag set: only their first run shows.
        if t.has_triggered && !trace.fired[i] {
            let label = trigger_label(t);
            if settings.get("trace-triggers") {
                info!("trigger fired: {label} ({})", play.session.scene_name());
            }
            trace.recent.push(label);
            if trace.recent.len() > 8 {
                trace.recent.remove(0);
            }
        }
        trace.fired[i] = t.has_triggered;
    }
}

/// Top-left text block (stats / camera / player).
pub(super) fn draw_text(
    contexts: &mut EguiContexts,
    settings: &DebugRender,
    play: &Play,
    real: &Time<Real>,
    trace: &TriggerTrace,
    fps: &mut f32,
    cameras: &Query<(&Camera, &GlobalTransform), With<PlayCamera>>,
) {
    let dt = real.delta_secs();
    if dt > 0.0 {
        *fps = if *fps == 0.0 {
            1.0 / dt
        } else {
            *fps * 0.95 + 0.05 / dt
        };
    }
    let mut lines = Vec::new();
    let s = &play.session;
    if settings.get("stats") {
        let world = &s.world;
        lines.push(format!("{:.0} fps  {:.2} ms", *fps, dt * 1000.0));
        lines.push(format!("{} / {}", s.level_file(), s.scene_name()));
        lines.push(format!(
            "entities {}  npcs {}  bodies {}  missiles {}  effects {}",
            world
                .entities
                .iter()
                .filter(|e| e.added && !e.removed)
                .count(),
            world.npcs.len(),
            world.physics.bodies.len(),
            world.missiles.len(),
            world.effects.effects.len(),
        ));
        lines.push(format!(
            "cutscene {}  input limited {}{}{}",
            world.cutscene.active,
            world.flags.input_limited,
            if settings.get("pause") {
                "  PAUSED"
            } else {
                ""
            },
            if settings.get("slowmo") {
                "  SLOWMO"
            } else {
                ""
            },
        ));
    }
    if settings.get("camera-info") {
        let c = s.camera();
        let p = Vec3::from(c.position());
        let d = Vec3::from(c.direction());
        lines.push(format!(
            "camera pos {:.1} {:.1} {:.1}  dir {:.2} {:.2} {:.2}  fov {:.2}°",
            p.x,
            p.y,
            p.z,
            d.x,
            d.y,
            d.z,
            c.fov().to_degrees()
        ));
        if let Ok((camera, _)) = cameras.single() {
            if let Some(size) = camera.physical_target_size() {
                lines.push(format!("target {}x{}", size.x, size.y));
            }
        }
    }
    if settings.get("player-info") {
        let body = s.body();
        let hp = &s.avatar().character.combat.health;
        let [x, y, z] = body.position;
        let [vx, vy, vz] = body.velocity;
        lines.push(format!(
            "player pos {x:.2} {y:.2} {z:.2}  vel {vx:.2} {vy:.2} {vz:.2}  HP {:.0}/{:.0}",
            hp.hit_points, hp.max_hit_points
        ));
    }
    if settings.get("scripts") {
        let triggers = &s.runtime.layout().triggers;
        lines.push(format!(
            "scripts: {}  triggers fired {}/{}  skipped actions {}",
            s.script_error.as_deref().unwrap_or("running"),
            triggers.iter().filter(|t| t.has_triggered).count(),
            triggers.len(),
            s.runtime.skipped.len(),
        ));
        if !trace.recent.is_empty() {
            lines.push(format!("recent: {}", trace.recent.join(", ")));
        }
    }
    if lines.is_empty() {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    egui::Area::new(egui::Id::new("debug_text"))
        .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_black_alpha(160))
                .inner_margin(4.0)
                .show(ui, |ui| {
                    for line in lines {
                        ui.label(
                            egui::RichText::new(line)
                                .monospace()
                                .size(11.0)
                                .color(egui::Color32::from_rgb(220, 230, 220)),
                        );
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_options_map_to_toggles() {
        let d = DebugRender::from_argument(Some("pause,no-bloom"));
        assert!(d.get("pause") && d.get("no-bloom"));
        assert!(!d.get("slowmo"));
        assert!(DebugRender::from_argument(None).on.iter().all(|v| !v));
    }
}
