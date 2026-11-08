//! Editor mode / dungeon master on `bevy-toolmode` (not an original
//! feature). The tool hotbar replaces the element bar while tool mode is
//! on, the freecam drives the session's inspection view (so rendering and
//! picking agree), and tool edits go through `PlaySession::editor_*`
//! locally or as `DmCommand`s from a dungeon-master client to the host.
//! Built by default (`editor` feature); `debug-ui` adds developer pages to
//! the same menu.
use super::{
    equipment::HeldItem, network::NetworkPlay, step_session, sync_camera, AvatarRoot, NpcAssets,
    NpcRoot, OverlayCamera, Play, PlayAppExt, PlayCamera,
};
use bevy::{camera::visibility::RenderLayers, gltf::GltfAssetLabel, prelude::*};
use bevy_egui::{EguiGlobalSettings, EguiPlugin, PrimaryEguiContext};
use bevy_toolmode::{
    gizmo::ToolGizmos, FreeCam, LiveEdit, LiveEditKind, PropertyDef, PropertyKind, PropertyValue,
    Reticle, ReticleHit, Selectable, SpawnCatalogue, SpawnRequest, Spawnable, ThumbnailSettings,
    ToolAction, ToolButton, ToolCamera, ToolInfo, ToolKind, ToolMode, ToolModePlugin, ToolModeSet,
    ToolModeSettings, ToolRay, ToolRegistry,
};
use magicka_network::lobby::{DmCommand, Role};
use magicka_runtime::play_session::{EditorSpawnOptions, NPC_ORDERS};
use std::collections::HashMap;

pub(super) const POSSESS: &str = "dm_possess";
pub(super) const SMITE: &str = "dm_smite";
pub(super) const HEAL: &str = "dm_heal";
pub(super) const TELEPORT: &str = "teleport_player";

pub(super) fn plugin(app: &mut App) {
    egui_setup(app);
    #[cfg(not(feature = "debug-ui"))]
    app.add_play_systems(
        bevy_egui::EguiPrimaryContextPass,
        menu.before(bevy_toolmode::inspector::window),
    );
    let mut registry = ToolRegistry::default();
    for (id, label, help) in [
        (
            POSSESS,
            "Possess",
            "Primary: take control of the character under the cursor\nSecondary: release it\n(dungeon master role in a network session)",
        ),
        (SMITE, "Smite", "Primary: kill the character under the cursor"),
        (HEAL, "Heal", "Primary: restore the character under the cursor to full health"),
        (
            TELEPORT,
            "Teleport player",
            "Primary: move your wizard to the reticle",
        ),
    ] {
        registry.register(ToolInfo {
            id: id.into(),
            label: label.into(),
            category: "Dungeon master".into(),
            help: help.into(),
            kind: ToolKind::Custom,
            edits_world: true,
        });
    }
    app.add_plugins(ToolModePlugin)
        .insert_resource(registry)
        .insert_resource(ToolModeSettings {
            // Drawn with a game host: `menu` here, or `debug_menu::menu`
            // with the developer pages.
            draw_menu: false,
            // The session camera owns rendering and picking.
            apply_freecam_to_camera: false,
            ..default()
        })
        .configure_sets(Update, ToolModeSet::Ray.after(sync_camera))
        .add_play_systems(PostStartup, (attach_context, configure, catalogue))
        .add_play_systems(
            Update,
            (
                mark_camera,
                mark_selectables,
                role_policy.before(ToolModeSet::Input),
                freecam_view
                    .after(ToolModeSet::Input)
                    .after(step_session)
                    .before(sync_camera),
                pick.in_set(ToolModeSet::Pick),
                (apply_spawns, apply_edits, apply_actions).after(ToolModeSet::Interact),
                fixture.before(ToolModeSet::Interact),
                forward_log.after(step_session),
                clear_decorations,
                mark_ghost_meshes
                    .before(crate::lighting::apply_lit_materials)
                    .before(crate::lighting::exclude_standard_occluders)
                    .before(crate::terrain::apply_terrain_materials),
            ),
        )
        // Staged plans and selections belong to one level.
        .add_session_reset(|world| {
            world
                .resource_mut::<bevy_toolmode::StagedSpawns>()
                .items
                .clear();
            world.resource_mut::<ToolMode>().selection = None;
        });
}

/// egui draws on the overlay camera, after the HUD and dialogs.
/// The tool menu without developer pages (plain builds).
#[cfg(not(feature = "debug-ui"))]
struct EditorHost<'a> {
    play: &'a mut Play,
    networked: bool,
}

#[cfg(not(feature = "debug-ui"))]
impl bevy_toolmode::MenuHost for EditorHost<'_> {
    fn tool_ui(&mut self, ui: &mut bevy_egui::egui::Ui, _tool: &str) {
        ui.label("Point at a character (or the ground for teleport) and use the primary button.");
    }
    fn live_ui(&mut self, ui: &mut bevy_egui::egui::Ui, id: u64) {
        if self.networked {
            ui.label("Use the dungeon master tools on networked characters.");
            return;
        }
        live_ui(ui, self.play, id as u16);
    }
}

#[cfg(not(feature = "debug-ui"))]
fn menu(
    mut contexts: bevy_egui::EguiContexts,
    mut tool_menu: bevy_toolmode::menu::ToolMenu,
    mut play: ResMut<Play>,
    network: Option<NonSend<NetworkPlay>>,
) {
    if !tool_menu.mode.menu_open {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let ctx = ctx.clone();
    let mut host = EditorHost {
        play: &mut play,
        networked: network.is_some(),
    };
    tool_menu.show(&ctx, &mut host);
}

fn egui_setup(app: &mut App) {
    app.insert_resource(EguiGlobalSettings {
        auto_create_primary_context: false,
        // Clicks/keys over the menu do not reach the game.
        enable_absorb_bevy_input_system: true,
        ..default()
    })
    .add_plugins(EguiPlugin::default());
    // bevy_egui orders its 2D pass after the main pass but not against the
    // post-process set, where the overlay camera's bloom swaps the main
    // textures: on some frames egui drew into the texture bloom had already
    // consumed and the whole UI vanished. Bloom first, then egui on top.
    if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
        render.configure_sets(
            bevy::core_pipeline::Core2d,
            crate::post::BloomSet.before(bevy_egui::render::egui_pass),
        );
    }
}

fn attach_context(mut commands: Commands, cameras: Query<Entity, With<OverlayCamera>>) {
    for camera in &cameras {
        commands.entity(camera).insert(PrimaryEguiContext);
    }
}

/// Render layer the play camera adds for tool gizmos (the debug draw uses
/// the same one).
const GIZMO_LAYER: usize = 14;

fn configure(
    play: Res<Play>,
    mut thumbs: ResMut<ThumbnailSettings>,
    mut store: ResMut<GizmoConfigStore>,
) {
    // Private derived cache, like the rest of the converted assets. Missing
    // or stale thumbnails render on first run behind a progress screen;
    // `--regenerate-thumbnails` renders all of them again.
    thumbs.cache_dir = Some(play.cache.join("thumbnails"));
    thumbs.asset_root = Some(play.cache.clone());
    // Magicka (XNA) models face -Z: view their front.
    thumbs.view_direction = Vec3::new(0.55, 0.45, -1.0);
    let off = std::env::args().any(|a| a == "--no-thumbnails");
    thumbs.enabled = !off;
    thumbs.pregenerate = !off;
    thumbs.force_regenerate = std::env::args().any(|a| a == "--regenerate-thumbnails");
    let (config, _) = store.config_mut::<ToolGizmos>();
    config.render_layers = RenderLayers::layer(GIZMO_LAYER);
    config.depth_bias = -1.0;
}

fn mark_camera(
    mut commands: Commands,
    cameras: Query<(Entity, Option<&RenderLayers>), (With<PlayCamera>, Without<ToolCamera>)>,
) {
    for (camera, layers) in &cameras {
        let layers = layers.cloned().unwrap_or_default().with(GIZMO_LAYER);
        commands.entity(camera).try_insert((ToolCamera, layers));
    }
}

/// Files with `extension` under `root/dir`, relative to `root/dir` (the
/// form content names take, e.g. `barrel_explosive.xnb`), sorted.
fn files(root: &std::path::Path, dir: &str, extension: &str) -> Vec<String> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, ext: &str, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, ext, out);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
            {
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let base = root.join(dir);
    let mut out = Vec::new();
    walk(&base, &base, extension, &mut out);
    out.sort();
    out
}

/// Wizard item pickups with an exported model: (name, glb, "Staffs" or
/// "Weapons").
fn wizard_items(
    play: &Play,
    models: &HashMap<String, String>,
) -> Vec<(String, String, &'static str)> {
    let mut out = Vec::new();
    for rel in files(&play.content, "Data/items/Wizard", "xnb") {
        let Ok(bytes) = std::fs::read(play.content.join("Data/items/Wizard").join(&rel)) else {
            continue;
        };
        let Ok(xnb) = magicka_xnb::xnb::parse(&bytes) else {
            continue;
        };
        let Some(Ok(parsed)) = magicka_content::parse_primary(&xnb) else {
            continue;
        };
        let magicka_content::Content::Item(item) = parsed.content else {
            continue;
        };
        let Some(model) = &item.model else {
            continue;
        };
        let name = std::path::Path::new(&rel)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let model =
            magicka_content::mesh::resolve_reference(&format!("Data/items/Wizard/{name}"), model);
        if let Some(file) = models.get(&format!("gltf/{model}").to_ascii_lowercase()) {
            let kind = if item.weapon_class == 18 {
                "Staffs"
            } else {
                "Weapons"
            };
            out.push((name, file.clone(), kind));
        }
    }
    out
}

/// Decorative model previews spawned by the editor (viewer-only).
#[derive(Component)]
struct EditorDecoration;

/// Decorations belong to one level/scene.
fn clear_decorations(
    mut commands: Commands,
    play: Res<Play>,
    decorations: Query<Entity, With<EditorDecoration>>,
    mut seen: Local<Option<(u32, u64)>>,
) {
    let key = (play.reloads, play.session.scene_changes as u64);
    if seen.is_some_and(|old| old != key) {
        for entity in &decorations {
            commands.entity(entity).try_despawn();
        }
    }
    *seen = Some(key);
}

/// glTF exports under `dir` (relative to the cache), keyed by lowercase
/// path without extension.
fn exports(cache: &std::path::Path, dir: &str) -> HashMap<String, String> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut HashMap<String, String>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.extension().is_some_and(|e| e == "glb") {
                if let Ok(rel) = path.strip_prefix(root) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    out.insert(rel.trim_end_matches(".glb").to_ascii_lowercase(), rel);
                }
            }
        }
    }
    let mut out = HashMap::new();
    walk(cache, &cache.join(dir), &mut out);
    out
}

/// "Goblin_pirate_captain" -> "Goblin".
fn family(name: &str) -> String {
    let first = name.split(['_', ' ']).next().unwrap_or(name);
    let mut chars = first.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_else(|| "Other".into())
}

fn folder(path: &str) -> Vec<String> {
    let mut parts: Vec<String> = path.split('/').map(String::from).collect();
    parts.pop();
    parts
}

fn catalogue(
    play: Res<Play>,
    npcs: Res<NpcAssets>,
    network: Option<NonSend<NetworkPlay>>,
    mut catalogue: ResMut<SpawnCatalogue>,
) {
    let models = exports(&play.cache, "gltf");
    let mut entries = Vec::new();
    let order = PropertyDef {
        key: "order".into(),
        label: "Order".into(),
        kind: PropertyKind::Choice(NPC_ORDERS.iter().map(|o| o.to_string()).collect()),
        default: PropertyValue::Text("attack".into()),
    };
    let movable = PropertyDef {
        key: "movable".into(),
        label: "Movable (physics)".into(),
        kind: PropertyKind::Bool,
        default: PropertyValue::Bool(false),
    };
    let mut characters: Vec<&String> = npcs.files.values().collect();
    characters.sort();
    for path in characters {
        let name = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".glb");
        entries.push(Spawnable {
            id: format!("npc:{name}"),
            label: name.replace('_', " "),
            category: vec!["Characters".into(), family(name)],
            model: Some(path.clone()),
            description: "Character template; spawns at the reticle on the ground.".into(),
            properties: vec![order.clone()],
        });
    }
    let props = exports(&play.cache, "gltf/Data/PhysicsEntities");
    for name in &files(&play.content, "Data/PhysicsEntities", "xnb") {
        let stem = name.trim_end_matches(".xnb");
        let mut category = vec!["Props".to_string()];
        category.extend(folder(stem));
        entries.push(Spawnable {
            id: format!("prop:{stem}"),
            label: stem.rsplit('/').next().unwrap_or(stem).replace('_', " "),
            category,
            model: props
                .get(&format!(
                    "gltf/data/physicsentities/{}",
                    stem.to_ascii_lowercase()
                ))
                .cloned(),
            description: "Physics entity with template collision and mass.".into(),
            properties: Vec::new(),
        });
    }
    for (name, file, kind) in &wizard_items(&play, &models) {
        entries.push(Spawnable {
            id: format!("item:{name}"),
            label: name.replace('_', " "),
            category: vec!["Items".into(), (*kind).into()],
            model: Some(file.to_string()),
            description: "Equipment pickup; click it in play to collect.".into(),
            properties: vec![movable.clone()],
        });
    }
    let book_model = format!("gltf/{}.glb", magicka_runtime::magick_books::BOOK_MODEL);
    let book_model = play.cache.join(&book_model).is_file().then_some(book_model);
    for magick in magicka_runtime::magick_books::BOOK_MAGICKS
        .iter()
        .filter(|n| !n.is_empty() && **n != "none")
    {
        entries.push(Spawnable {
            id: format!("book:{magick}"),
            label: magick.replace('_', " "),
            category: vec!["Magick books".into()],
            model: book_model.clone(),
            description: "Unlocks its magick when picked up.".into(),
            properties: vec![movable.clone()],
        });
    }
    // Decorations are viewer-side model previews with no game entity, so
    // nothing replicates them: networked sessions do not offer them.
    let offline = network.is_none();
    let decorations = models.values().filter(|p| {
        offline
            && p.starts_with("gltf/Models/")
            && !["/Characters/", "/Effects/", "/Missiles/"]
                .iter()
                .any(|v| p.contains(v))
    });
    let mut decorations: Vec<&String> = decorations.collect();
    decorations.sort();
    for path in decorations {
        let mut category = vec!["Decorations".to_string()];
        category.extend(folder(path.strip_prefix("gltf/Models/").unwrap_or(path)));
        entries.push(Spawnable {
            id: format!("deco:{path}"),
            label: path
                .rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".glb")
                .replace('_', " "),
            category,
            model: Some(path.to_string()),
            description: "Viewer-only model preview: no collision, not shared over the network."
                .into(),
            properties: Vec::new(),
        });
    }
    let mut effects: Vec<&str> = play
        .session
        .effects
        .library
        .effects
        .values()
        .map(|e| e.name.as_str())
        .collect();
    effects.sort();
    for name in effects {
        entries.push(Spawnable {
            id: format!("fx:{name}"),
            label: name.to_string(),
            category: vec!["Effects".into(), family(name)],
            model: None,
            description: "Particle effect started at the reticle.".into(),
            properties: Vec::new(),
        });
    }
    catalogue.replace(entries);
}

/// Keep the game's material converters off editor ghosts (see
/// `lighting::EditorGhost`); runs before them so new meshes are tagged
/// the frame they appear.
fn mark_ghost_meshes(
    mut commands: Commands,
    ghosts: Query<Entity, With<bevy_toolmode::Ghost>>,
    children: Query<&Children>,
    untagged: Query<(), (With<Mesh3d>, Without<crate::lighting::EditorGhost>)>,
) {
    for ghost in &ghosts {
        for e in children.iter_descendants(ghost) {
            if untagged.contains(e) {
                commands.entity(e).try_insert(crate::lighting::EditorGhost);
            }
        }
    }
}

/// Print new dungeon-master, refusal and error lines of the session log
/// (a host records DM results there) so they are not silent.
fn forward_log(play: Res<Play>, mut seen: Local<(u32, usize)>) {
    let log = &play.session.world.log;
    if seen.0 != play.reloads || seen.1 > log.len() {
        *seen = (play.reloads, 0);
    }
    for line in log[seen.1..]
        .iter()
        .filter(|l| l.starts_with("dungeon master") || l.contains("refused") || l.contains("error"))
    {
        info!("session: {line}");
    }
    seen.1 = log.len();
}

/// `--editor-fixture`: a deterministic editor scene for screenshots: tool
/// mode on with a selected staged goblin (gizmo shown), a staged troll and a
/// realized prop in front of the wizard.
fn fixture(
    mut frames: Local<u32>,
    play: Res<Play>,
    mut mode: ResMut<ToolMode>,
    mut staged: ResMut<bevy_toolmode::StagedSpawns>,
    catalogue: Res<SpawnCatalogue>,
    mut spawns: MessageWriter<SpawnRequest>,
) {
    *frames += 1;
    if *frames != 120 || !std::env::args().any(|a| a == "--editor-fixture") {
        return;
    }
    // Around the camera's focus: the wizard offline, the followed host
    // player for a dungeon master (whose own avatar is dormant elsewhere).
    let feet = Vec3::from(play.session.camera().ground_position());
    let forward = Vec3::from(play.session.body().direction())
        .with_y(0.0)
        .normalize_or(Vec3::NEG_Z);
    let right = forward.cross(Vec3::Y);
    let at = |ahead: f32, side: f32| {
        Transform::from_translation(feet + forward * ahead + right * side)
            .with_rotation(Quat::from_rotation_y(0.6))
    };
    let goblin = staged.add(
        "npc:Goblin_pirate".into(),
        at(4.0, -4.0),
        [("order".to_string(), PropertyValue::Text("idle".into()))].into(),
    );
    staged.add("npc:Troll_forest".into(), at(6.0, 3.0), Default::default());
    spawns.write(SpawnRequest {
        // The spawn menu's own entry, so the catalogue's ids are exercised.
        spawnable: catalogue
            .entries
            .iter()
            .find(|e| e.label == "barrel explosive" && e.id.starts_with("prop:"))
            .map_or_else(|| "missing barrel_explosive entry".into(), |e| e.id.clone()),
        transform: at(3.0, 0.5),
        properties: Default::default(),
        from_stage: None,
    });
    mode.active = true;
    mode.tool = bevy_toolmode::tools::SELECT.into();
    mode.selection = Some(bevy_toolmode::Selection::Staged(goblin));
}

fn mark_selectables(
    mut commands: Commands,
    play: Res<Play>,
    npcs: Query<(Entity, &NpcRoot), Without<Selectable>>,
    avatars: Query<Entity, (With<AvatarRoot>, Without<Selectable>)>,
    held: Query<(Entity, &HeldItem), Without<Selectable>>,
) {
    for (entity, npc) in &npcs {
        commands.entity(entity).try_insert(Selectable {
            id: u64::from(npc.handle),
            label: format!("NPC #{}", npc.handle),
            radius: 0.9,
            center_height: 1.0,
        });
    }
    if let Some(handle) = play.session.world.players[0].as_ref().map(|p| p.handle) {
        for entity in &avatars {
            commands.entity(entity).try_insert(Selectable {
                id: u64::from(handle),
                label: "Player".into(),
                radius: 0.9,
                center_height: 1.0,
            });
        }
    }
    for (entity, item) in &held {
        // Pickups, books and physics props render as owner/slot -300..-302.
        if (-302..=-300).contains(&item.slot) {
            commands.entity(entity).try_insert(Selectable {
                id: u64::from(item.owner),
                label: format!("Entity #{}", item.owner),
                radius: 0.7,
                center_height: 0.3,
            });
        }
    }
}

/// Network seats: a dungeon master edits through the host; other clients
/// may only plan (staging).
fn role_policy(
    network: Option<NonSend<NetworkPlay>>,
    host_role: Option<Res<super::HostRole>>,
    mut mode: ResMut<ToolMode>,
    mut last: Local<Option<Role>>,
    mut freecam_in: Local<Option<u32>>,
) {
    let role = match network.as_deref() {
        Some(NetworkPlay::Client(c)) if !c.disconnected() => Some(c.role()),
        // A wizardless host: the role picked on the lobby badge.
        Some(NetworkPlay::Host(_)) => host_role.as_ref().map(|r| r.0),
        _ => None,
    };
    let read_only = match role {
        None | Some(Role::DungeonMaster) => None,
        Some(_) => Some("World edits need the dungeon master role".to_string()),
    };
    if mode.read_only != read_only {
        mode.read_only = read_only;
    }
    if role != *last {
        if role == Some(Role::DungeonMaster) {
            mode.active = true;
            mode.message = "Dungeon master: tools are on (F1 menu, F2 tools, F3 freecam)".into();
            // The observer camera reaches the followed player over the
            // first frames; the freecam starts from it once it has.
            *freecam_in = Some(FREECAM_DELAY_FRAMES);
        }
        // A spectator host has no observer camera of its own: freecam.
        if role == Some(Role::Spectator) && host_role.is_some() {
            *freecam_in = Some(FREECAM_DELAY_FRAMES);
        }
        *last = role;
    }
    if let Some(frames) = freecam_in.as_mut() {
        if *frames == 0 {
            mode.freecam = role == Some(Role::DungeonMaster)
                || (role == Some(Role::Spectator) && host_role.is_some());
            *freecam_in = None;
        } else {
            *frames -= 1;
        }
    }
}

/// Frames between a dungeon master's role grant and the automatic freecam.
const FREECAM_DELAY_FRAMES: u32 = 60;

fn freecam_view(
    mode: Res<ToolMode>,
    mut cam: ResMut<FreeCam>,
    mut play: ResMut<Play>,
    mut was_on: Local<bool>,
) {
    if mode.freecam && !*was_on {
        // Start from the session camera's view: the Bevy camera transform
        // is not posed yet on a session's first frame (a dungeon master gets
        // the freecam as the role is granted).
        let camera = play.session.camera();
        let position = Vec3::from(camera.position());
        cam.look_from(
            &Transform::from_translation(position)
                .looking_to(Vec3::from(camera.direction()), Vec3::from(camera.up())),
        );
        cam.fov = camera.fov();
    }
    *was_on = mode.freecam;
    if !mode.freecam {
        return;
    }
    let t = cam.transform();
    play.session
        .world
        .camera
        .set_inspection_view(Some(magicka_runtime::camera::CameraView {
            position: t.translation.to_array(),
            direction: t.forward().to_array(),
            up: t.up().to_array(),
            fov: cam.fov,
        }));
}

/// Reticle on level collision; the plane fallback follows the camera's
/// ground height.
fn pick(
    play: Res<Play>,
    ray: Res<ToolRay>,
    mut reticle: ResMut<Reticle>,
    mut settings: ResMut<ToolModeSettings>,
) {
    let ground = play.session.camera().ground_position()[1];
    if settings.ground_plane_y != ground {
        settings.ground_plane_y = ground;
    }
    let Some(r) = ray.ray else {
        return;
    };
    let segment = magicka_sim::spray_pulse::Segment {
        origin: r.origin.to_array(),
        delta: (*r.direction * 250.0).to_array(),
    };
    if let Some(hit) = play
        .session
        .world
        .collision
        .collision()
        .segment_intersect(segment, false)
    {
        reticle.hit = Some(ReticleHit {
            point: Vec3::from(hit.hit.position),
            normal: Vec3::from(hit.hit.normal).normalize_or(Vec3::Y),
        });
    }
}

fn matrix(t: &Transform) -> [f32; 16] {
    t.to_matrix().to_cols_array()
}

fn apply_spawns(
    mut requests: MessageReader<SpawnRequest>,
    mut play: ResMut<Play>,
    mut network: Option<NonSendMut<NetworkPlay>>,
    mut mode: ResMut<ToolMode>,
    mut commands: Commands,
    assets: Res<AssetServer>,
) {
    for request in requests.read() {
        if let Some(path) = request.spawnable.strip_prefix("deco:") {
            commands.spawn((
                EditorDecoration,
                WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path.to_string()))),
                request.transform,
            ));
            continue;
        }
        let order = request
            .properties
            .get("order")
            .and_then(|v| v.as_str())
            .map(String::from);
        let movable = request
            .properties
            .get("movable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let transform = matrix(&request.transform);
        let result = match network.as_deref_mut() {
            Some(NetworkPlay::Client(view)) => view
                .dm(DmCommand::Spawn {
                    spawnable: request.spawnable.clone(),
                    transform,
                    order,
                    movable,
                })
                .map(|()| "sent to the host".to_string()),
            _ => play
                .session
                .editor_spawn(
                    &request.spawnable,
                    transform,
                    &EditorSpawnOptions { order, movable },
                )
                .map(|h| h.map_or("started".into(), |h| format!("#{h}"))),
        };
        mode.message = match result {
            Ok(what) => format!("{}: {what}", request.spawnable),
            Err(e) => format!("{}: {e}", request.spawnable),
        };
    }
}

fn apply_edits(
    mut edits: MessageReader<LiveEdit>,
    mut play: ResMut<Play>,
    mut network: Option<NonSendMut<NetworkPlay>>,
    mut mode: ResMut<ToolMode>,
) {
    for edit in edits.read() {
        let handle = edit.id as u16;
        let result = match (&edit.edit, network.as_deref_mut()) {
            (LiveEditKind::Move(t), Some(NetworkPlay::Client(view))) => view.dm(DmCommand::Move {
                handle,
                transform: matrix(t),
            }),
            (LiveEditKind::Remove, Some(NetworkPlay::Client(view))) => {
                view.dm(DmCommand::Remove { handle })
            }
            (LiveEditKind::Move(t), _) => play.session.editor_move(handle, matrix(t)),
            (LiveEditKind::Remove, _) => play.session.editor_remove(handle),
        };
        if let Err(e) = result {
            mode.message = format!("#{handle}: {e}");
        }
    }
}

fn apply_actions(
    mut actions: MessageReader<ToolAction>,
    mut play: ResMut<Play>,
    mut network: Option<NonSendMut<NetworkPlay>>,
    mut mode: ResMut<ToolMode>,
) {
    for action in actions.read() {
        let target = action.target.map(|t| t as u16);
        let client = match network.as_deref_mut() {
            Some(NetworkPlay::Client(view)) => Some(view),
            _ => None,
        };
        let result: Result<String, String> =
            match (action.tool.as_str(), action.button, target, client) {
                (POSSESS, ToolButton::Secondary, _, Some(view)) => {
                    view.dm(DmCommand::Release).map(|()| "released".into())
                }
                (POSSESS, ToolButton::Primary, Some(npc), Some(view)) => view
                    .dm(DmCommand::Possess { npc })
                    .map(|()| format!("possessing #{npc}")),
                (POSSESS, _, _, None) => {
                    Err("possession needs the dungeon master role in a network session".into())
                }
                (SMITE, ToolButton::Primary, Some(handle), Some(view)) => view
                    .dm(DmCommand::Smite { handle })
                    .map(|()| format!("smote #{handle}")),
                (SMITE, ToolButton::Primary, Some(handle), None) => play
                    .session
                    .smite(handle)
                    .map(|()| format!("smote #{handle}")),
                (HEAL, ToolButton::Primary, Some(handle), Some(view)) => view
                    .dm(DmCommand::Heal { handle })
                    .map(|()| format!("healed #{handle}")),
                (HEAL, ToolButton::Primary, Some(handle), None) => play
                    .session
                    .heal_character(handle)
                    .map(|()| format!("healed #{handle}")),
                (TELEPORT, ToolButton::Primary, _, None) => {
                    match (action.reticle, play.session.world.players[0].as_ref()) {
                        (Some(hit), Some(p)) => {
                            let handle = p.handle;
                            let mut t = Transform::from_translation(hit.point);
                            let current = play.session.body().orientation;
                            t.rotation = Quat::from_mat3(&Mat3::from_cols(
                                Vec3::from(current.right),
                                Vec3::from(current.up),
                                Vec3::from(current.backward),
                            ));
                            play.session
                                .editor_move(handle, matrix(&t))
                                .map(|()| "teleported".into())
                        }
                        _ => Err("no reticle or player".into()),
                    }
                }
                (TELEPORT, _, _, Some(_)) => Err("teleporting runs on the host".into()),
                (_, ToolButton::Primary, None, _) => Err("point at a character".into()),
                _ => continue,
            };
        mode.message = match result {
            Ok(m) | Err(m) => m,
        };
    }
}

/// Inspector for a selected live entity (handle).
pub(super) fn live_ui(ui: &mut bevy_egui::egui::Ui, play: &mut Play, handle: u16) {
    let s = &mut play.session;
    let Some(e) = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == handle && !e.removed)
    else {
        ui.label("Gone.");
        return;
    };
    ui.label(format!(
        "{:?}  {}\nposition {:.1} {:.1} {:.1}\nHP {:.0}/{:.0}{}",
        e.kind,
        e.template_name.as_deref().unwrap_or(""),
        e.position[0],
        e.position[1],
        e.position[2],
        e.hit_points,
        e.max_hit_points,
        if e.dead { "  (dead)" } else { "" }
    ));
    ui.horizontal(|ui| {
        if ui.button("Heal").clicked() {
            let _ = s.heal_character(handle);
        }
        if ui.button("Smite").clicked() {
            let _ = s.smite(handle);
        }
        if ui.button("Remove").clicked() {
            let _ = s.editor_remove(handle);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_files_are_named_relative_to_their_folder() {
        let root = std::env::temp_dir().join(format!("editor-files-{}", std::process::id()));
        let dir = root.join("Data/PhysicsEntities/Barrels");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(root.join("Data/PhysicsEntities/barrel_explosive.xnb"), b"").unwrap();
        std::fs::write(dir.join("keg.XNB"), b"").unwrap();
        std::fs::write(dir.join("notes.txt"), b"").unwrap();
        let found = files(&root, "Data/PhysicsEntities", "xnb");
        std::fs::remove_dir_all(&root).unwrap();
        // Spawn ids are `prop:<this without .xnb>`; a content-root-relative
        // name (`Data/PhysicsEntities/...`) cannot be loaded.
        assert_eq!(found, ["Barrels/keg.XNB", "barrel_explosive.xnb"]);
    }

    #[test]
    fn family_and_folder_group_catalogue_entries() {
        assert_eq!(family("goblin_pirate_captain"), "Goblin");
        assert_eq!(family("Troll forest"), "Troll");
        assert_eq!(folder("Furniture/Chairs/chair"), ["Furniture", "Chairs"]);
        assert!(folder("barrel").is_empty());
    }
}
