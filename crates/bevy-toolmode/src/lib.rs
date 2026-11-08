//! Editor / dungeon-master "tool mode" for Bevy games.
//!
//! A game opts in by adding [`ToolModePlugin`] (after `bevy_egui`'s
//! `EguiPlugin`), marking its gameplay camera with [`ToolCamera`], filling a
//! [`SpawnCatalogue`] and reacting to the messages this crate writes:
//!
//! - [`SpawnRequest`]: make a catalogue entry real at a transform (directly
//!   from the spawn tool, or when a staged spawn is realized);
//! - [`LiveEdit`]: move or remove a live entity carrying [`Selectable`];
//! - [`ToolAction`]: a game-defined tool ([`ToolKind::Custom`]) was used.
//!
//! The inspect tool and [`Inspector`] window edit any entity's reflected
//! components directly (useful for games without their own editor).
//!
//! The crate owns presentation and editing state only: the tool menu
//! (spawn browser with generated thumbnails, tool list, utility pages), the
//! hotbar that replaces a game's ability bar while tool mode is on, a free
//! camera, the cursor reticle, translate/rotate gizmos and *staged*
//! (inactive) spawns that can be placed, oriented and configured before
//! they are realized. The game decides what any of it means.
//!
//! Systems run in [`ToolModeSet`] order inside `Update`. A game with its own
//! picking (collision meshes, navmeshes) writes [`Reticle`] in
//! [`ToolModeSet::Pick`]; otherwise the reticle falls on a horizontal plane.
pub mod catalogue;
pub mod freecam;
pub mod gizmo;
mod hud;
pub mod inspector;
pub mod menu;
pub mod reticle;
pub mod staging;
pub mod thumbnails;
pub mod tools;

pub use catalogue::{
    PropertyDef, PropertyKind, PropertyValue, PropertyValues, SpawnCatalogue, Spawnable,
};
pub use freecam::FreeCam;
pub use gizmo::GizmoMode;
pub use inspector::Inspector;
pub use menu::{MenuHost, NoHost};
pub use reticle::{Reticle, ReticleHit, ToolRay};
pub use staging::{Ghost, Staged, StagedSpawns};
pub use thumbnails::{ThumbnailSettings, Thumbnails};
pub use tools::{ToolInfo, ToolKind, ToolRegistry};

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;

/// Update-schedule order of the tool mode systems.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolModeSet {
    /// Toggles, hotbar keys, freecam movement, button edges.
    Input,
    /// The cursor ray from the [`ToolCamera`] and the plane fallback reticle.
    Ray,
    /// Game picking: overwrite [`Reticle`] here.
    Pick,
    /// Tools, gizmo drags and staging edits.
    Interact,
    /// Gizmo, reticle and ghost drawing.
    Draw,
}

/// The camera the reticle ray and gizmos use (and freecam drives when
/// [`ToolModeSettings::apply_freecam_to_camera`] is set).
#[derive(Component, Default)]
pub struct ToolCamera;

/// A live game entity tools can select, move and remove. `id` is the game's
/// own handle and comes back in [`LiveEdit`] and [`ToolAction`].
#[derive(Component, Clone, Debug)]
pub struct Selectable {
    pub id: u64,
    pub label: String,
    /// Pick sphere radius around the entity origin raised by `center_height`.
    pub radius: f32,
    pub center_height: f32,
}

/// What the gizmo and the inspector work on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Staged(u64),
    Live(u64),
}

/// Number of hotbar slots (one per ability key the hotbar replaces).
pub const HOTBAR_SLOTS: usize = 8;

/// Shared tool mode state. Games read it to suppress their own input while
/// [`ToolMode::captures_input`] or the freecam is on.
#[derive(Resource)]
pub struct ToolMode {
    /// Tool mode on: the hotbar replaces the game's ability keys and the
    /// mouse buttons drive the selected tool.
    pub active: bool,
    /// The spawn/tool menu is shown.
    pub menu_open: bool,
    /// Camera decoupled from the game into [`FreeCam`].
    pub freecam: bool,
    /// Selected tool id (see [`ToolRegistry`]).
    pub tool: String,
    pub hotbar: [Option<String>; HOTBAR_SLOTS],
    pub selection: Option<Selection>,
    /// Catalogue entry the spawn tool places.
    pub spawnable: Option<String>,
    /// Spawn tool places inactive [`Staged`] entries instead of spawning.
    pub stage_spawns: bool,
    /// Spawn tool yaw (radians), stepped with the secondary button.
    pub spawn_yaw: f32,
    /// Property overrides the spawn tool applies to new spawns.
    pub spawn_properties: PropertyValues,
    pub gizmo: GizmoMode,
    /// Translation snap step in world units (0 = off).
    pub snap: f32,
    /// Rotation snap step in degrees (0 = off).
    pub angle_snap: f32,
    /// Last status line shown on the HUD.
    pub message: String,
    /// When set, world-changing tools are refused with this reason (for
    /// example a network client that may only send requests).
    pub read_only: Option<String>,
}

impl Default for ToolMode {
    fn default() -> Self {
        let mut hotbar: [Option<String>; HOTBAR_SLOTS] = Default::default();
        for (slot, id) in [
            tools::SPAWN,
            tools::SELECT,
            tools::REMOVE,
            tools::REALIZE,
            tools::INSPECT,
        ]
        .into_iter()
        .enumerate()
        {
            hotbar[slot] = Some(id.to_string());
        }
        Self {
            active: false,
            menu_open: false,
            freecam: false,
            tool: tools::SPAWN.into(),
            hotbar,
            selection: None,
            spawnable: None,
            stage_spawns: false,
            spawn_yaw: 0.0,
            spawn_properties: PropertyValues::new(),
            gizmo: GizmoMode::Universal,
            snap: 0.0,
            angle_snap: 15.0,
            message: String::new(),
            read_only: None,
        }
    }
}

impl ToolMode {
    /// The game should ignore its own gameplay input (movement, aiming,
    /// abilities): tool mode or the freecam owns the mouse and keys.
    pub fn captures_input(&self) -> bool {
        self.active || self.freecam
    }

    /// Put `tool` in the first empty hotbar slot unless it is already there.
    pub fn add_to_hotbar(&mut self, tool: &str) {
        if self.hotbar.iter().flatten().any(|t| t == tool) {
            return;
        }
        if let Some(slot) = self.hotbar.iter_mut().find(|s| s.is_none()) {
            *slot = Some(tool.to_string());
        }
    }
}

/// Key bindings and behaviour switches.
#[derive(Resource, Clone)]
pub struct ToolModeSettings {
    pub toggle_menu: Option<KeyCode>,
    pub toggle_tool_mode: Option<KeyCode>,
    pub toggle_freecam: Option<KeyCode>,
    /// Hotbar slot keys; defaults to the QWER ASDF row Magicka-like games use
    /// for abilities.
    pub hotbar_keys: [KeyCode; HOTBAR_SLOTS],
    /// Run the crate's own menu system. Games that add utility pages call
    /// [`menu::show`] from their own egui system instead.
    pub draw_menu: bool,
    /// Write the [`FreeCam`] pose into the [`ToolCamera`] transform. Games
    /// that own their camera (and its picking) apply the pose themselves.
    pub apply_freecam_to_camera: bool,
    /// Height of the fallback reticle plane.
    pub ground_plane_y: f32,
    /// Draw the hotbar/status HUD while tool mode is on.
    pub hud: bool,
}

impl Default for ToolModeSettings {
    fn default() -> Self {
        use KeyCode::*;
        Self {
            toggle_menu: Some(F1),
            toggle_tool_mode: Some(F2),
            toggle_freecam: Some(F3),
            hotbar_keys: [KeyQ, KeyW, KeyE, KeyR, KeyA, KeyS, KeyD, KeyF],
            draw_menu: true,
            apply_freecam_to_camera: true,
            ground_plane_y: 0.0,
            hud: true,
        }
    }
}

/// Make catalogue entry `spawnable` real. `from_stage` names the staged
/// entry it came from (already removed from [`StagedSpawns`]).
#[derive(Message, Clone, Debug)]
pub struct SpawnRequest {
    pub spawnable: String,
    pub transform: Transform,
    pub properties: PropertyValues,
    pub from_stage: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LiveEditKind {
    Move(Transform),
    Remove,
}

/// Edit a live [`Selectable`] entity (by its game id).
#[derive(Message, Clone, Debug)]
pub struct LiveEdit {
    pub id: u64,
    pub edit: LiveEditKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolButton {
    Primary,
    Secondary,
}

/// A [`ToolKind::Custom`] tool was used at the reticle.
#[derive(Message, Clone, Debug)]
pub struct ToolAction {
    pub tool: String,
    pub button: ToolButton,
    pub reticle: Option<ReticleHit>,
    /// The [`Selectable`] under the cursor, if any.
    pub target: Option<u64>,
}

/// Mouse button edges for tools this frame, after freecam look filtering:
/// a secondary *click* is a short press without much mouse travel, so the
/// same button can still hold-to-look.
#[derive(Resource, Default, Debug)]
pub struct ToolButtons {
    pub primary_pressed: bool,
    pub primary_held: bool,
    pub primary_released: bool,
    pub secondary_clicked: bool,
    /// The secondary button is held for camera look.
    pub looking: bool,
    secondary_down: Option<(f32, f32)>,
}

pub struct ToolModePlugin;

impl Plugin for ToolModePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ToolMode>()
            .init_resource::<ToolModeSettings>()
            .init_resource::<ToolRegistry>()
            .init_resource::<SpawnCatalogue>()
            .init_resource::<StagedSpawns>()
            .init_resource::<FreeCam>()
            .init_resource::<ToolRay>()
            .init_resource::<Reticle>()
            .init_resource::<ToolButtons>()
            .init_resource::<gizmo::GizmoDrag>()
            .init_resource::<menu::MenuState>()
            .init_resource::<Inspector>()
            .init_resource::<ThumbnailSettings>()
            .init_gizmo_group::<gizmo::ToolGizmos>()
            .add_message::<SpawnRequest>()
            .add_message::<LiveEdit>()
            .add_message::<ToolAction>()
            .configure_sets(
                Update,
                (
                    ToolModeSet::Input,
                    ToolModeSet::Ray,
                    ToolModeSet::Pick,
                    ToolModeSet::Interact,
                    ToolModeSet::Draw,
                )
                    .chain(),
            )
            .add_systems(Startup, (thumbnails::setup, gizmo::configure))
            .add_systems(
                Update,
                (
                    // Button edges first: toggles and the freecam read
                    // this frame's look state.
                    (buttons, toggles, freecam::drive)
                        .chain()
                        .in_set(ToolModeSet::Input),
                    reticle::cast.in_set(ToolModeSet::Ray),
                    (gizmo::interact, tools::use_tools, inspector::pick)
                        .chain()
                        .in_set(ToolModeSet::Interact),
                    (
                        (staging::sync_ghosts, staging::ghost_materials).chain(),
                        reticle::draw,
                        gizmo::draw,
                        staging::draw_markers,
                    )
                        .in_set(ToolModeSet::Draw),
                    thumbnails::process,
                ),
            )
            .add_systems(
                PostUpdate,
                freecam::apply_to_camera.before(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (
                    thumbnails::register_textures,
                    hud::draw,
                    thumbnails::progress_screen,
                    menu::default_menu.run_if(|s: Res<ToolModeSettings>| s.draw_menu),
                    inspector::window,
                )
                    .chain(),
            );
    }
}

fn toggles(
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<ToolModeSettings>,
    registry: Res<ToolRegistry>,
    mut mode: ResMut<ToolMode>,
    buttons: Res<ToolButtons>,
) {
    let pressed = |key: Option<KeyCode>| key.is_some_and(|k| keys.just_pressed(k));
    if pressed(settings.toggle_menu) {
        mode.menu_open = !mode.menu_open;
    }
    if pressed(settings.toggle_tool_mode) {
        mode.active = !mode.active;
        mode.message = if mode.active {
            "Tool mode on".into()
        } else {
            "Tool mode off".into()
        };
    }
    if pressed(settings.toggle_freecam) {
        mode.freecam = !mode.freecam;
        mode.message = if mode.freecam {
            "Freecam on: hold right mouse to look, WASD/QE to fly".into()
        } else {
            "Freecam off".into()
        };
    }
    // Flying with WASD while looking must not switch tools.
    if !mode.active || buttons.looking {
        return;
    }
    for (slot, key) in settings.hotbar_keys.iter().enumerate() {
        if keys.just_pressed(*key) {
            if let Some(tool) = mode.hotbar[slot].clone() {
                if registry.get(&tool).is_some() {
                    mode.tool = tool;
                }
            }
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        mode.selection = None;
    }
}

/// Seconds and mouse travel (pixels) under which a secondary press counts
/// as a click rather than a look drag.
const CLICK_TIME: f32 = 0.3;
const CLICK_TRAVEL: f32 = 6.0;

fn buttons(
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    time: Res<Time<Real>>,
    mode: Res<ToolMode>,
    mut out: ResMut<ToolButtons>,
) {
    out.primary_pressed = mouse.just_pressed(MouseButton::Left);
    out.primary_held = mouse.pressed(MouseButton::Left);
    out.primary_released = mouse.just_released(MouseButton::Left);
    out.secondary_clicked = false;
    if !mode.freecam {
        out.looking = false;
        out.secondary_down = None;
        out.secondary_clicked = mouse.just_pressed(MouseButton::Right);
        return;
    }
    let now = time.elapsed_secs();
    if mouse.just_pressed(MouseButton::Right) {
        out.secondary_down = Some((now, 0.0));
    }
    if let Some((_, travel)) = out.secondary_down.as_mut() {
        *travel += motion.delta.length();
    }
    if let Some((start, travel)) = out.secondary_down {
        let click = now - start < CLICK_TIME && travel < CLICK_TRAVEL;
        out.looking = mouse.pressed(MouseButton::Right) && !click;
        if mouse.just_released(MouseButton::Right) {
            out.secondary_clicked = click;
            out.secondary_down = None;
            out.looking = false;
        }
    }
}

/// Headless end-to-end checks: the real Input/Ray/Interact systems driven
/// by synthetic window cursor, mouse buttons, mouse motion and a camera.
#[cfg(test)]
mod interaction_tests;
