//! Debug rendering (developer menu, Diagnostics → Debug rendering): gizmo
//! wireframes for level data the game never draws (trigger areas, locators,
//! navmesh, collision, camera mesh, lights, effect placements, prop boxes),
//! live entities/bodies, mesh bounding boxes, and on-screen name labels.
//! `--debug-draw triggers,entities,labels,...` enables layers at startup.
use super::super::{lifecycle::PlayAppExt, Play, PlayCamera};
use super::egui;
use bevy::{
    camera::visibility::RenderLayers, gizmos::aabb::AabbGizmoConfigGroup, math::Isometry3d,
    prelude::*,
};
use bevy_egui::EguiContexts;
use magicka_content::level::{LevelModel, TriMesh};
use magicka_runtime::scene_loading::custom_hash;
use magicka_runtime::scene_world::EntityKind;

/// Gizmos only render on the gameplay camera (not the 2D overlay, which
/// would draw world coordinates as pixels).
const GIZMO_LAYER: usize = 14;
/// Labels drawn per frame at most (nearest first).
const MAX_LABELS: usize = 160;

/// Layer names accepted by `--debug-draw` and shown in the menu.
pub(super) const LAYERS: [&str; 16] = [
    "triggers",
    "locators",
    "entities",
    "bodies",
    "contacts",
    "ai",
    "missiles",
    "navmesh",
    "collision",
    "camera",
    "lights",
    "effects",
    "live-effects",
    "props",
    "aabbs",
    "labels",
];

#[derive(Resource, Clone, Debug)]
pub(in crate::play) struct DebugDraw {
    /// One flag per [`LAYERS`] entry.
    pub(in crate::play) layers: [bool; LAYERS.len()],
    /// Draw through walls.
    pub(in crate::play) xray: bool,
    /// Navmesh/collision/camera-mesh triangles farther than this from the
    /// player are skipped (keeps big levels interactive).
    pub(in crate::play) radius: f32,
    /// Labels farther than this from the player are skipped.
    pub(in crate::play) label_radius: f32,
}

impl DebugDraw {
    fn from_argument(value: Option<&str>, radius: Option<&str>) -> Self {
        let requested = value.unwrap_or("");
        for name in requested.split(',').filter(|v| !v.is_empty()) {
            if name != "all" && name != "xray" && !LAYERS.contains(&name) {
                warn!(
                    "--debug-draw: unknown layer {name:?}; known: all, xray, {}",
                    LAYERS.join(", ")
                );
            }
        }
        let radius = radius.and_then(|r| r.parse().ok()).unwrap_or(30.0);
        Self {
            layers: LAYERS.map(|name| requested.split(',').any(|v| v == name || v == "all")),
            xray: requested.split(',').any(|v| v == "xray"),
            radius,
            label_radius: radius.max(40.0),
        }
    }

    fn on(&self, name: &str) -> bool {
        LAYERS
            .iter()
            .position(|l| *l == name)
            .is_some_and(|i| self.layers[i])
    }
}

pub(super) fn plugin(app: &mut App, argument: Option<&str>, radius: Option<&str>) {
    app.insert_resource(DebugDraw::from_argument(argument, radius))
        .add_play_systems(PostStartup, gizmo_layer)
        .add_play_systems(Update, (configure, draw));
}

/// Menu section (Diagnostics window).
pub(super) fn section(ui: &mut egui::Ui, settings: &mut DebugDraw) {
    egui::CollapsingHeader::new("Debug rendering")
        .default_open(settings.layers.iter().any(|v| *v))
        .show(ui, |ui| {
            egui::Grid::new("debug_draw_layers")
                .num_columns(3)
                .show(ui, |ui| {
                    for (i, name) in LAYERS.iter().enumerate() {
                        ui.checkbox(&mut settings.layers[i], *name);
                        if i % 3 == 2 {
                            ui.end_row();
                        }
                    }
                });
            ui.horizontal(|ui| {
                ui.checkbox(&mut settings.xray, "x-ray");
                if ui.button("all").clicked() {
                    settings.layers = [true; LAYERS.len()];
                }
                if ui.button("none").clicked() {
                    settings.layers = [false; LAYERS.len()];
                }
            });
            ui.add(egui::Slider::new(&mut settings.radius, 5.0..=200.0).text("mesh radius"));
            ui.add(egui::Slider::new(&mut settings.label_radius, 5.0..=200.0).text("label radius"));
            ui.label(
                "Triggers: yellow, green while occupied. Locators: cyan axes. Entities: \
                 orange NPC, teal player, grey dead. Bodies: purple + velocity. \
                 Contacts: red. AI: path + waypoint. Missiles: white + velocity. \
                 Navmesh: blue (water teal, jump violet, fly pink). Collision: grey. \
                 Camera mesh: magenta. Lights: amber. Effects: red cross (live: \
                 green active / grey inactive). Props: green boxes.",
            );
            ui.label(format!(
                "Startup: --debug-draw {} (or all, xray) --debug-draw-radius N",
                LAYERS.join(",")
            ));
        });
}

fn gizmo_layer(
    mut commands: Commands,
    cameras: Query<(Entity, Option<&RenderLayers>), With<PlayCamera>>,
) {
    for (camera, layers) in &cameras {
        let layers = layers.cloned().unwrap_or_default().with(GIZMO_LAYER);
        commands.entity(camera).insert(layers);
    }
}

fn configure(settings: Res<DebugDraw>, mut store: ResMut<GizmoConfigStore>) {
    if !settings.is_changed() {
        return;
    }
    let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    config.render_layers = RenderLayers::layer(GIZMO_LAYER);
    // -1 draws in front of everything.
    config.depth_bias = if settings.xray { -1.0 } else { 0.0 };
    config.line.width = 1.5;
    let (config, aabb) = store.config_mut::<AabbGizmoConfigGroup>();
    config.render_layers = RenderLayers::layer(GIZMO_LAYER);
    config.depth_bias = if settings.xray { -1.0 } else { 0.0 };
    aabb.draw_all = settings.on("aabbs");
    aabb.default_color = Some(srgb(0.25, 0.45, 0.25));
}

/// Colours in the scene's encoded space (`post.wgsl` decodes 1 − 2^−x), so
/// keep channels at or below ~0.5 to avoid bloom blow-out.
fn srgb(r: f32, g: f32, b: f32) -> Color {
    Color::srgb(r, g, b)
}

fn xna(m: &[f32; 16]) -> Mat4 {
    // XNA row-vector matrices are glam column-major arrays as-is.
    Mat4::from_cols_array(m)
}

fn quat(q: [f32; 4]) -> Quat {
    Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize()
}

/// Oriented box with its corner at `position` and extents `sides` along the
/// rotated axes (TriggerArea / CollisionBox convention, research: presence).
fn corner_box(position: [f32; 3], orientation: [f32; 4], sides: [f32; 3]) -> Transform {
    let rotation = quat(orientation);
    let sides = Vec3::from(sides);
    Transform {
        translation: Vec3::from(position) + rotation * (sides * 0.5),
        rotation,
        scale: sides,
    }
}

fn player(play: &Play) -> Vec3 {
    Vec3::from(play.session.body().position)
}

fn triangles(gizmos: &mut Gizmos, mesh: &TriMesh, near: Vec3, radius: f32, color: Color) {
    let r2 = radius * radius;
    for t in &mesh.triangles {
        let mut p = [Vec3::ZERO; 3];
        let mut ok = true;
        for (k, &i) in t.iter().enumerate() {
            match usize::try_from(i).ok().and_then(|i| mesh.vertices.get(i)) {
                Some(v) => p[k] = Vec3::from(*v),
                None => ok = false,
            }
        }
        if ok && p.iter().any(|v| v.distance_squared(near) < r2) {
            gizmos.linestrip([p[0], p[1], p[2], p[0]], color);
        }
    }
}

fn draw(settings: Res<DebugDraw>, play: Res<Play>, mut gizmos: Gizmos) {
    if !settings.layers.iter().any(|v| *v) {
        return;
    }
    let world = &play.session.world;
    let level: &LevelModel = &world.level;
    let near = player(&play);
    let r2 = settings.radius * settings.radius;

    if settings.on("triggers") {
        for area in &level.trigger_areas {
            let occupied = world
                .presence
                .get(custom_hash(&area.name.to_ascii_lowercase()))
                .is_ok_and(|p| !p.entities.is_empty() || !p.characters.is_empty());
            let color = if occupied {
                srgb(0.15, 0.5, 0.15)
            } else {
                srgb(0.5, 0.45, 0.1)
            };
            gizmos.cube(
                corner_box(area.position, area.orientation, area.sides),
                color,
            );
        }
    }
    if settings.on("locators") {
        for locator in &level.locators {
            let m = xna(&locator.transform);
            let (_, rotation, position) = m.to_scale_rotation_translation();
            gizmos.axes(
                Transform::from_translation(position).with_rotation(rotation),
                0.75,
            );
            if locator.radius > 0.0 {
                gizmos.sphere(
                    Isometry3d::from_translation(position),
                    locator.radius,
                    srgb(0.1, 0.4, 0.5),
                );
            }
        }
    }
    if settings.on("entities") {
        for e in world.entities.iter().filter(|e| e.added && !e.removed) {
            let p = Vec3::from(e.position);
            let color = match (e.dead, &e.kind) {
                (true, _) => srgb(0.25, 0.25, 0.25),
                (_, EntityKind::Avatar { .. }) => srgb(0.15, 0.45, 0.5),
                (_, EntityKind::Npc { .. }) => srgb(0.5, 0.3, 0.05),
                _ => srgb(0.45, 0.4, 0.3),
            };
            let radius = e.radius.max(0.1);
            if e.length > 0.0 {
                gizmos.primitive_3d(
                    &Capsule3d::new(radius, e.length),
                    Isometry3d::from_translation(p),
                    color,
                );
            } else {
                gizmos.sphere(Isometry3d::from_translation(p), radius, color);
            }
            // Facing (XNA Forward is −Backward, the third row negated).
            let forward = -Vec3::new(e.orientation[8], e.orientation[9], e.orientation[10]);
            if forward.length_squared() > 1e-6 {
                gizmos.arrow(p, p + forward.normalize() * (radius + 0.6), color);
            }
        }
    }
    if settings.on("bodies") {
        for body in &world.physics.bodies {
            let p = Vec3::from(body.position);
            let color = srgb(0.4, 0.15, 0.45);
            gizmos.primitive_3d(
                &Capsule3d::new(body.radius.max(0.05), body.length.max(0.0)),
                Isometry3d::from_translation(p),
                color,
            );
            let v = Vec3::from(body.velocity);
            if v.length_squared() > 0.01 {
                gizmos.arrow(p, p + v * 0.25, srgb(0.5, 0.1, 0.4));
            }
        }
    }
    if settings.on("contacts") {
        for i in 0..world.physics.bodies.len() {
            for c in world.physics.contact_points(i) {
                gizmos.cross(
                    Isometry3d::from_translation(Vec3::from(c)),
                    0.15,
                    srgb(0.5, 0.05, 0.05),
                );
            }
        }
    }
    if settings.on("ai") {
        for npc in world.npcs.values() {
            let Some(body) = world.physics.bodies.get(npc.body) else {
                continue;
            };
            let color = srgb(0.45, 0.45, 0.1);
            let start = Vec3::from(body.position);
            let points = std::iter::once(start).chain(
                npc.ai
                    .path
                    .iter()
                    .map(|n| Vec3::from(n.position) + Vec3::Y * 0.1),
            );
            gizmos.linestrip(points, color);
            let waypoint = Vec3::from(npc.ai.waypoint);
            if waypoint != Vec3::ZERO {
                gizmos.line(start, waypoint, color.with_alpha(0.4));
                gizmos.cross(Isometry3d::from_translation(waypoint), 0.3, color);
            }
        }
    }
    if settings.on("missiles") {
        for m in &world.missiles {
            let p = Vec3::from(m.body.position);
            let color = srgb(0.5, 0.5, 0.5);
            gizmos.sphere(
                Isometry3d::from_translation(p),
                m.body.radius.max(0.05),
                color,
            );
            gizmos.arrow(p, p + Vec3::from(m.body.velocity) * 0.1, color);
        }
    }
    if settings.on("live-effects") {
        for e in world.effects.effects.values() {
            let p = xna(&e.transform).transform_point3(Vec3::ZERO);
            let color = if e.effect.active {
                srgb(0.15, 0.5, 0.2)
            } else {
                srgb(0.25, 0.25, 0.25)
            };
            gizmos.cross(Isometry3d::from_translation(p), 0.35, color);
            if e.range > 0.0 && p.distance_squared(near) < r2 {
                gizmos.sphere(
                    Isometry3d::from_translation(p),
                    e.range,
                    color.with_alpha(0.3),
                );
            }
        }
    }
    if settings.on("navmesh") {
        let nav = &level.nav_mesh;
        for t in &nav.triangles {
            let p = t
                .vertices
                .map(|i| nav.vertices.get(usize::from(i)).copied().map(Vec3::from));
            let [Some(a), Some(b), Some(c)] = p else {
                continue;
            };
            if [a, b, c].iter().all(|v| v.distance_squared(near) >= r2) {
                continue;
            }
            // MovementProperties: Water 1, Jump 2, Fly 4 (research/navmesh.md).
            let color = if t.properties & 4 != 0 {
                srgb(0.5, 0.2, 0.4)
            } else if t.properties & 2 != 0 {
                srgb(0.35, 0.2, 0.5)
            } else if t.properties & 1 != 0 {
                srgb(0.1, 0.45, 0.45)
            } else {
                srgb(0.15, 0.25, 0.5)
            };
            let lift = Vec3::Y * 0.03;
            gizmos.linestrip([a + lift, b + lift, c + lift, a + lift], color);
        }
    }
    if settings.on("collision") {
        for mesh in &level.collision_layers {
            triangles(
                &mut gizmos,
                mesh,
                near,
                settings.radius,
                srgb(0.4, 0.4, 0.4),
            );
        }
    }
    if settings.on("camera") {
        if let Some(mesh) = &level.camera_mesh {
            triangles(
                &mut gizmos,
                mesh,
                near,
                settings.radius,
                srgb(0.5, 0.1, 0.5),
            );
        }
    }
    if settings.on("lights") {
        for light in &level.lights {
            let p = Vec3::from(light.position);
            let color = srgb(0.5, 0.35, 0.05);
            // LightType: 0 point, 1 directional, 2 spot (lighting.rs).
            if light.light_type == 1 {
                gizmos.arrow(
                    p,
                    p + Vec3::from(light.direction).normalize_or_zero() * 3.0,
                    color,
                );
            } else {
                gizmos.sphere(Isometry3d::from_translation(p), 0.25, color);
                if p.distance_squared(near) < r2 {
                    gizmos.sphere(
                        Isometry3d::from_translation(p),
                        light.range,
                        color.with_alpha(0.35),
                    );
                }
                if light.light_type == 2 {
                    gizmos.arrow(
                        p,
                        p + Vec3::from(light.direction).normalize_or_zero() * 2.0,
                        color,
                    );
                }
            }
        }
    }
    if settings.on("effects") {
        for effect in &level.effects {
            let p = Vec3::from(effect.position);
            let color = srgb(0.45, 0.2, 0.2);
            gizmos.cross(Isometry3d::from_translation(p), 0.3, color);
            let f = Vec3::from(effect.forward);
            if f.length_squared() > 1e-6 {
                gizmos.arrow(p, p + f.normalize(), color);
            }
        }
    }
    if settings.on("props") {
        for b in world.static_prop_boxes.values() {
            gizmos.cube(
                corner_box(b.position, b.orientation, b.sides),
                srgb(0.3, 0.45, 0.2),
            );
        }
        for (transform, _) in &level.physics_entities {
            let p = xna(transform).transform_point3(Vec3::ZERO);
            gizmos.cross(Isometry3d::from_translation(p), 0.4, srgb(0.3, 0.45, 0.2));
        }
    }
}

/// One on-screen label: world anchor, text, colour.
struct Label {
    at: Vec3,
    text: String,
    color: egui::Color32,
}

fn labels(settings: &DebugDraw, play: &Play) -> Vec<Label> {
    let world = &play.session.world;
    let level = &world.level;
    let mut out = Vec::new();
    let mut push = |at: Vec3, text: String, color: egui::Color32| {
        out.push(Label { at, text, color });
    };
    if settings.on("triggers") {
        for area in &level.trigger_areas {
            let t = corner_box(area.position, area.orientation, area.sides);
            push(
                t.translation,
                area.name.clone(),
                egui::Color32::from_rgb(240, 220, 90),
            );
        }
    }
    if settings.on("locators") {
        for l in &level.locators {
            let at = xna(&l.transform).transform_point3(Vec3::ZERO);
            push(at, l.name.clone(), egui::Color32::from_rgb(110, 220, 240));
        }
    }
    if settings.on("entities") {
        for e in world.entities.iter().filter(|e| e.added && !e.removed) {
            let kind = match &e.kind {
                EntityKind::Avatar { slot } => format!("player {slot}"),
                EntityKind::Npc { .. } => e.template_name.clone().unwrap_or_else(|| "npc".into()),
                EntityKind::Physics { damageable } => if *damageable {
                    "physics (damageable)"
                } else {
                    "physics"
                }
                .into(),
                other => format!("{other:?}").to_ascii_lowercase(),
            };
            let mut text = format!("#{} {kind}", e.handle);
            if e.max_hit_points > 0.0 {
                text += &format!("\nHP {:.0}/{:.0}", e.hit_points, e.max_hit_points);
            }
            if e.dead {
                text += " (dead)";
            }
            let at = Vec3::from(e.position) + Vec3::Y * (e.length * 0.5 + e.radius + 0.3);
            push(at, text, egui::Color32::from_rgb(250, 180, 90));
        }
    }
    if settings.on("lights") {
        for l in &level.lights {
            push(
                Vec3::from(l.position),
                l.name.clone(),
                egui::Color32::from_rgb(250, 200, 120),
            );
        }
    }
    if settings.on("effects") {
        for e in &level.effects {
            push(
                Vec3::from(e.position),
                format!("{} ({})", e.id, e.effect),
                egui::Color32::from_rgb(240, 150, 150),
            );
        }
    }
    if settings.on("live-effects") {
        let library = &play.session.effects.library.effects;
        for (id, e) in &world.effects.effects {
            let name = library
                .get(&e.effect.effect)
                .map_or_else(|| format!("{:#x}", e.effect.effect), |l| l.name.clone());
            let at = xna(&e.transform).transform_point3(Vec3::ZERO);
            push(
                at,
                format!("fx {id}: {name}"),
                egui::Color32::from_rgb(150, 240, 160),
            );
        }
    }
    if settings.on("ai") {
        for npc in world.npcs.values() {
            if let Some(body) = world.physics.bodies.get(npc.body) {
                let at = Vec3::from(body.position) + Vec3::Y * (body.length + body.radius + 1.0);
                push(
                    at,
                    format!("ai event {} order {}", npc.ai.current_event, npc.ai.order),
                    egui::Color32::from_rgb(230, 230, 120),
                );
            }
        }
    }
    if settings.on("props") {
        for (transform, name) in &level.physics_entities {
            let at = xna(transform).transform_point3(Vec3::ZERO);
            push(at, name.clone(), egui::Color32::from_rgb(170, 230, 140));
        }
    }
    out
}

/// Screen-space name labels, behind the debug windows.
pub(super) fn draw_labels(
    contexts: &mut EguiContexts,
    settings: &DebugDraw,
    play: &Play,
    cameras: &Query<(&Camera, &GlobalTransform), With<PlayCamera>>,
) {
    if !settings.on("labels") {
        return;
    }
    let (Ok(ctx), Ok((camera, transform))) = (contexts.ctx_mut(), cameras.single()) else {
        return;
    };
    let near = player(play);
    let r2 = settings.label_radius * settings.label_radius;
    let mut items: Vec<_> = labels(settings, play)
        .into_iter()
        .filter(|l| l.at.distance_squared(near) < r2)
        .filter_map(|l| {
            let screen = camera.world_to_viewport(transform, l.at).ok()?;
            Some((l.at.distance_squared(near), screen, l))
        })
        .collect();
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    let painter = ctx.layer_painter(egui::LayerId::background());
    let font = egui::FontId::monospace(11.0);
    for (_, screen, label) in items.into_iter().take(MAX_LABELS) {
        let pos = egui::pos2(screen.x, screen.y);
        let galley = painter.layout_no_wrap(label.text, font.clone(), label.color);
        let rect = egui::Align2::CENTER_BOTTOM
            .anchor_size(pos, galley.size())
            .expand(2.0);
        painter.rect_filled(rect, 2.0, egui::Color32::from_black_alpha(150));
        painter.galley(rect.min + egui::vec2(2.0, 2.0), galley, label.color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_enables_named_layers_only() {
        let d = DebugDraw::from_argument(Some("triggers,labels,xray"), Some("12"));
        assert!(d.on("triggers") && d.on("labels") && d.xray);
        assert!(!d.on("navmesh"));
        assert_eq!(d.radius, 12.0);
        assert!(DebugDraw::from_argument(Some("all"), None)
            .layers
            .iter()
            .all(|v| *v));
        assert!(DebugDraw::from_argument(None, None)
            .layers
            .iter()
            .all(|v| !v));
    }

    #[test]
    fn corner_box_centres_on_rotated_half_extent() {
        // 90° about +Y maps local +X to world −Z.
        let q = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let t = corner_box([1.0, 0.0, 0.0], [q.x, q.y, q.z, q.w], [2.0, 4.0, 6.0]);
        assert!((t.translation - Vec3::new(1.0 + 3.0, 2.0, -1.0)).length() < 1e-5);
        assert_eq!(t.scale, Vec3::new(2.0, 4.0, 6.0));
    }
}
