//! Playable single-player session (`--play <Content>`): the runtime
//! PlaySession (spawn, CharacterBody physics on live level collision, Idle/
//! Move animation choice, MagickCamera) drives the exported level glTF and the
//! skinned Wizard. See research/play-session.md.
//!
//! Controls (original keyboard/mouse defaults): hold the left mouse button to
//! walk toward the cursor; click pickups to collect them, or hold a click to
//! approach and collect. Shift-click attacks. F5 restarts, Esc opens the pause menu.
//!
//! `--autopilot <dir>` runs headless with scripted input and a fixed 1/60 s
//! step, saving frames and a JSON log of session state for automated checks.
//! `--hide-level` omits level geometry (avatar silhouette checks).
use crate::native_dialog::{DialogView, DialogViewMut, NativeDialogBox, NativeDialogRenderPlugin};
use crate::native_text::{upload_font, NativeBitmapText, NativeTextPlugin};
use crate::terrain::{ScenePath, TerrainNoDiscard, TerrainPlugin, TerrainUnlit};
use crate::text_box_visual::{
    read_asset, upload_background, TextBoxBackgroundPlugin, TextBoxMaterial,
};
use bevy::animation::graph::{AnimationGraph, AnimationGraphHandle, AnimationNodeIndex};
use bevy::animation::transition::AnimationTransitions;
use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::gltf::{Gltf, GltfAssetLabel};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use bevy::window::{ExitCondition, PrimaryWindow};
use bevy::winit::WinitPlugin;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot};
use lifecycle::{PlayAppExt, PlaySystems};
use magicka_content::assets::Asset;
use magicka_runtime::{
    animation_controller::{animation_name, ClipCue},
    avatar::PlayerInput,
    camera::{FAR_CLIP, NEAR_CLIP},
    dialog_world::SceneDialogWorld,
    play_session::{PlaySession, SessionEvent},
    text_geometry::TextStyle,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

mod assatur_models;
mod spell_gallery;
use spell_gallery::{SPELL_GALLERY_SCRIPT, SPELL_OBSERVER_SCRIPT};
mod audio;
mod authored_level;
mod barrier_material;
mod barrier_runes;
mod beam_target_qa;
mod boss_bar_visual;
mod boss_models;
mod boss_qa;
mod clip_models;
mod country_defense_visual;
mod credits_visual;
mod cthulhu_models;
mod cursor;
mod cutaway;
mod death_models;
#[cfg(feature = "debug-ui")]
mod debug_menu;
#[cfg(feature = "editor")]
mod editor;
mod egg_models;
mod entrance_qa;
mod equipment;
mod ethereal_lit;
mod fafnir_models;
mod frontend_loading;
mod generic_bar_visual;
mod grimnir2_models;
mod grimnir_models;
mod halo;
mod highlights;
mod house_qa;
mod ice_spikes;
mod inventory_visual;
mod join_profile;
mod level_parts;
mod lifecycle;
mod lighting_qa;
mod local_coop;
mod machine_models;
mod machine_ruler;
mod magick_models;
mod menu;
mod mod_panel;
mod network;
mod npc_projectile_qa;
mod parker_models;
mod particle_depth;
mod portal_visual;
mod ruleset_hud;
mod save_slots;
pub(crate) mod spatial;
mod tick_clock;
mod title_visual;
mod vlad_afterimages;
mod warlord_qa;
mod wave_visual;
mod zap_flash;

const AUTOPILOT_SIZE: UVec2 = UVec2::new(1280, 720);
const FIXED_DT: f32 = magicka_runtime::fixed_tick::TICK_DT;

/// The local host's role without a wizard (`--host-role`).
#[derive(Resource, Clone, Copy)]
pub(crate) struct HostRole(pub(crate) magicka_network::lobby::Role);

#[derive(Resource)]
pub(crate) struct Play {
    pub(crate) session: PlaySession,
    content: PathBuf,
    cache: PathBuf,
    #[cfg(feature = "debug-ui")]
    debug_loadout: Option<(String, String, String)>,
    level: String,
    character: String,
    scripts: bool,
    /// `--demo-goblin`: spawn an attacking goblin after (re)load.
    demo_goblin: bool,
    /// `--versus SPEC`: the lobby's Versus mode, re-applied on reloads.
    versus: Option<magicka_runtime::ruleset::VersusSettings>,
    /// `--packs SPEC`: the lobby's enabled PackMan packs, re-applied on
    /// reloads (PlaySession::load restores the cache's default pools).
    packs: Option<magicka_runtime::ruleset::SurvivalPacks>,
    /// Latest unconsumed clip cue (kept until the player exists).
    pending: Option<ClipCue>,
    /// Latest clip cue per live NPC with a sequence number, so animators
    /// attached late still start the current clip.
    npc_cues: HashMap<u16, (u64, ClipCue)>,
    npc_seq: u64,
    /// F5/defeat reloads, so presentation entities tied to the old world
    /// (level glTF, NPC handles) are rebuilt.
    reloads: u32,
    hide_level: bool,
    #[cfg(feature = "debug-ui")]
    scenarios: PathBuf,
    /// Last logic delta and logical screen size, for dialog rendering.
    dt: f32,
    screen: [f32; 2],
    /// Hosted by the menu (`--menu-loading`): a completed campaign level
    /// returns to the Tome instead of loading the next level in place.
    menu_hosted: bool,
    /// `--audit-complete-after SECONDS`: audit instrumentation that issues
    /// the `<endgame type="LevelComplete"/>` action after that much play.
    complete_after: Option<f32>,
    /// KeyboardBindings from settings.json (SubMenuOptionsKeyboard).
    bindings: crate::bindings::KeyboardBindings,
}

impl DialogView<SceneDialogWorld> for Play {
    fn view(&mut self) -> Option<DialogViewMut<'_, SceneDialogWorld>> {
        let world = &mut self.session.world;
        Some(DialogViewMut {
            host: world.dialog_text.as_mut()?,
            channel: 0,
            dt: self.dt,
            screen: self.screen,
            matrix: world.dialog_matrix,
            failure: &mut world.dialog_failure,
        })
    }
}

#[derive(Resource)]
struct AvatarAssets {
    gltf: Handle<Gltf>,
    scene: Handle<WorldAsset>,
    level: Option<Handle<WorldAsset>>,
    graph: Option<(Handle<AnimationGraph>, HashMap<String, AnimationNodeIndex>)>,
}

#[derive(Component)]
#[require(tick_clock::TickInterpolated)]
pub(crate) struct AvatarRoot;
/// The current scene's level glTF root.
#[derive(Component)]
struct LevelRoot;
/// Full-screen black quad for scene transitions (RenderManager fade).
#[derive(Component)]
struct FadeOverlay(Handle<ColorMaterial>);
#[derive(Component)]
#[require(tick_clock::TickInterpolated)]
struct PlayCamera;
/// The 2D overlay camera (dialogs, HUD, cutscene bars, fades).
#[derive(Component)]
struct OverlayCamera;

#[derive(Resource)]
struct AdditionalDialogAssets {
    body: Arc<magicka_runtime::text_geometry::GeometryFont>,
    name: Arc<magicka_runtime::text_geometry::GeometryFont>,
    body_material: Handle<ColorMaterial>,
    name_material: Handle<ColorMaterial>,
    background_material: Handle<TextBoxMaterial>,
}
/// User-requested inspection controls; these are not recovered retail behavior.
#[derive(Resource, Default)]
struct InspectionCamera {
    yaw: f32,
    tilt: f32,
    zoom: f32,
    pan: Vec3,
    unlocked_target: Option<Vec3>,
}

fn liquid_inspection_target(
    level: &magicka_content::level::LevelModel,
    kind: &str,
) -> Result<Vec3, String> {
    use magicka_content::level::LiquidMaterial;
    if !matches!(kind, "water" | "lava") {
        return Err("--inspect-liquid expects water or lava".into());
    }
    let mut largest = None;
    for liquid in &level.liquids {
        if !matches!(
            (&liquid.material, kind),
            (LiquidMaterial::Water(_), "water") | (LiquidMaterial::Lava(_), "lava")
        ) {
            continue;
        }
        let geometry = liquid
            .collision_geometry()
            .map_err(|e| format!("liquid inspection: {e:?}"))?;
        for triangle in geometry.triangles {
            let [a, b, c] = triangle.map(|i| Vec3::from(geometry.vertices[i as usize]));
            let area = (b - a).cross(c - a).length_squared();
            if area.is_finite() && area > 0.0 && largest.is_none_or(|(best, _)| area > best) {
                largest = Some((area, (a + b + c) / 3.0));
            }
        }
    }
    let (_, target) =
        largest.ok_or_else(|| format!("no top-level {kind} triangles in the selected scene"))?;
    eprintln!("liquid inspection: {kind} target={target:?}");
    Ok(target)
}
/// The avatar's AnimationPlayer once its scene has spawned.
#[derive(Component)]
struct AvatarAnimator;

/// Windowed self-test: screenshot the primary window once the avatar has
/// animated for a while, then exit.
#[derive(Resource)]
struct WindowSnapshot {
    path: PathBuf,
    frames: u32,
    at: u32,
}

#[derive(Resource)]
struct Autopilot {
    dir: PathBuf,
    frame: u32,
    started: Option<u32>,
    log: Vec<String>,
    target: Handle<Image>,
    pending_shot: bool,
    script: &'static [Step],
    /// `fafnir-wing`: where the mortal wizard stands between attacks.
    anchor: Option<[f32; 3]>,
}

/// Autopilot input held for a step.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Entrance(u8),
    Houses(u8),
    LightingSpawn,
    EtherealNpcSpawn,
    EtherealNpcAlpha(u8),
    EtherealNpcAttackSpawn(u8),
    MachineBreak,
    LightingBright,
    LightingDark,
    SpellLightsVisible,
    SpellLightsHidden,
    StatusLightsVisible,
    StatusLightsHidden,
    BossStart,
    BossWait,
    BossVisible,
    BossHidden,
    BossMotion(u8),
    DeathAfterimages(u8),
    VladAfterimages(u8),
    Grimnir(u8),
    Grimnir2(u8),
    WarlordStart,
    WarlordWait,
    WarlordFire,
    WarlordClear,
    WarlordArcane,
    /// Cutaway QA (play/cutaway.rs): mode (0 off, 1 on, 2 debug tint) and
    /// inspection view preset.
    Cutaway {
        mode: u8,
        view: u8,
    },
    /// Set every magick bit in player 0's unlock mask.
    UnlockMagicks,
    /// KeyboardBindings.Inventory (Tab) pressed this frame.
    Inventory,
    /// KeyboardBindings.PrevMagick / NextMagick (scroll wheel) this frame.
    MagickPrev,
    MagickNext,
    SpawnBook,
    PickupBook,
    SpawnTimedBook,
    SpawnFallingBook,
    SpawnPhysicsProp,
    NpcEarthCaster,
    NpcBeholder,
    NpcBomber,
    NpcWarrior,
    BreakWarriorShield,
    NpcProjectileWait,
    NpcProjectileVisible,
    NpcProjectileHidden,
    NpcModelHidden,
    NpcTrailHidden,
    TutorialHighlights,
    EtherealBody,
    CorporealBody,
    LabPortal,
    BeamTarget(u8, u8),
    SpawnPickup,
    Pickup,
    Idle,
    /// QA: the pause menu open at the Players page.
    PlayersPage,
    /// QA: the Players page's confirmation for its first player.
    KickConfirm,
    /// QA: confirm it, as choosing Yes there does.
    KickYes,
    /// Move (left mouse) held.
    Move,
    /// Fire element key (F) held.
    Fire,
    Life,
    Arcane,
    /// Earth element key (D) held.
    Earth,
    /// Lightning element key (A) held.
    Lightning,
    /// Shield element key (E) held.
    Shield,
    Boost,
    /// Water element key (Q) held.
    Water,
    /// Cold element key (R) held.
    Cold,
    /// Test harness only: terminate the avatar (as the kill plane does).
    Die,
    /// Test harness only: the `<endgame type="LevelComplete"/>` script action.
    Complete,
    /// Test harness only: move the avatar into the `trigger_exit` area.
    Exit,
    /// A cutscene-skipping key press (Space).
    Skip,
    /// Pause menu "Restart at checkpoint" (F5): the checkpoint, else a reload.
    Restart,
    /// Test harness only: move the avatar into the `trigger_gate` area.
    Gate,
    /// Test harness only: Character.OverKill on every live NPC.
    Gib,
    /// Test harness only: Water.FreezeAll(2) on every retained water.
    FreezeWater,
    /// Harness-only atlas fixture, independent of spell input and damage.
    DecalFixture,
    LayeringFixture,
    LayeringVisible,
    LayeringHidden,
    LayeringDarkVisible,
    LayeringDarkHidden,
    ParticleDepthBaseline,
    ParticleDepthBehind,
    ParticleDepthOldSource,
    ParticleDepthFront,
    ParticleDepthNoItems,
    SelfCast,
    AreaCast,
    /// Test harness only: time pinned, every NPC kept zapped; the LightningHit
    /// flash drawn (Visible) or suppressed (Hidden) for a paired capture.
    ZapFlashVisible,
    ZapFlashHidden,
    /// Shift + Move (left mouse) held: charge or swing an enchanted weapon.
    WeaponSwing,
    /// Shift held alone.
    ShiftHold,
    /// Test harness only: WeaponSwing input with time pinned; the IceBlade
    /// model drawn (Visible) or suppressed (Hidden) for a paired capture.
    IceBladeVisible,
    IceBladeHidden,
    EarthTrailVisible,
    EarthTrailHidden,
    CapsuleLightVisible,
    CapsuleLightHidden,
    ShieldWallVisible,
    ShieldWallHidden,
    BarrierStrong,
    BarrierWeak,
    /// Test harness only: time pinned; IceSpikes drawn (Visible) or
    /// suppressed (Hidden) for a paired capture.
    SpikesVisible,
    SpikesHidden,
    /// Test harness only: time pinned; FloatingTextManager numbers drawn
    /// (Visible) or suppressed (Hidden) for a paired capture.
    NumbersVisible,
    NumbersHidden,
    /// Cast (right mouse) held.
    Cast,
}
/// Scripted input: (frames, cursor NDC, action, capture name at end).
type Step = (u32, [f32; 2], Act, &'static str);
const SCRIPT: &[Step] = &[
    (90, [0.0, 0.0], Act::Idle, "idle"),
    (75, [0.6, 0.0], Act::Move, "walk_right"),
    (75, [-0.6, 0.0], Act::Move, "walk_left"),
    (60, [0.0, -0.6], Act::Move, "walk_down"),
    (45, [0.0, -0.6], Act::Idle, "stopped"),
];
/// `--autopilot-script outline`: walk down-screen (toward the camera) so
/// level walls come between the camera and the Wizard.
const ETHEREAL_SCRIPT: &[Step] = &[
    (60, [0.0, 0.0], Act::Idle, "before"),
    (1, [0.0, 0.0], Act::EtherealBody, "ethereal_request"),
    (60, [0.0, 0.0], Act::Idle, "ethereal"),
    (1, [0.0, 0.0], Act::CorporealBody, "corporeal_request"),
    (60, [0.0, 0.0], Act::Idle, "corporeal"),
];
const TUTORIAL_HIGHLIGHTS_SCRIPT: &[Step] = &[
    (60, [0.0, 0.0], Act::Idle, "before"),
    (1, [0.0, 0.0], Act::TutorialHighlights, "requested"),
    (60, [0.0, 0.0], Act::Idle, "highlighted"),
    (180, [0.0, 0.0], Act::Idle, "expired"),
];
const OUTLINE_SCRIPT: &[Step] = &[
    (60, [0.0, 0.0], Act::Idle, "start"),
    (60, [0.0, -0.6], Act::Move, "down1"),
    (60, [0.0, -0.6], Act::Move, "down2"),
    (60, [0.0, -0.6], Act::Move, "down3"),
    (60, [0.0, -0.6], Act::Move, "down4"),
    (60, [0.3, -0.6], Act::Move, "down5"),
    (60, [-0.3, -0.6], Act::Move, "down6"),
];
/// `--autopilot-script cast`: face up-screen (−Z), conjure Fire twice, hold
/// Force, release and wait until every particle has expired.
const CAST_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (2, [0.0, 0.5], Act::Fire, "fire1"),
    (12, [0.0, 0.5], Act::Idle, "gap"),
    (2, [0.0, 0.5], Act::Fire, "fire2"),
    (12, [0.0, 0.5], Act::Idle, "queued"),
    (40, [0.0, 0.5], Act::Cast, "spray_a"),
    (40, [0.0, 0.5], Act::Cast, "spray_b"),
    (45, [0.0, 0.5], Act::Idle, "released"),
    (90, [0.0, 0.5], Act::Idle, "faded"),
];
/// `--autopilot-script damage-numbers` (with `--demo-goblin`): wait for the
/// goblin beside the Wizard, Fire spray on it, then a
/// pinned pair with its locked direct-damage number drawn and suppressed.
const DAMAGE_NUMBERS_SCRIPT: &[Step] = &[
    (300, [-0.17, -0.45], Act::Idle, "idle"),
    (2, [-0.17, -0.45], Act::Fire, "fire1"),
    (12, [-0.17, -0.45], Act::Idle, "gap"),
    (2, [-0.17, -0.45], Act::Fire, "fire2"),
    (12, [-0.17, -0.45], Act::Idle, "queued"),
    (30, [-0.17, -0.45], Act::Cast, "spray"),
    (20, [-0.17, -0.45], Act::NumbersVisible, "numbers_visible"),
    (20, [-0.17, -0.45], Act::NumbersHidden, "numbers_hidden"),
    (150, [-0.17, -0.45], Act::Idle, "faded"),
];
// Pin simulation time while comparing the same wizard and flame particles.
const DECAL_LAYERING_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (2, [0.0, 0.5], Act::Fire, "fire"),
    (12, [0.0, 0.5], Act::Idle, "queued"),
    (60, [0.0, 0.5], Act::Cast, "spray"),
    (1, [0.0, 0.5], Act::LayeringFixture, "fixture"),
    (30, [0.0, 0.5], Act::LayeringVisible, "visible"),
    (30, [0.0, 0.5], Act::LayeringHidden, "hidden"),
    (30, [0.0, 0.5], Act::LayeringDarkVisible, "dark_visible"),
    (30, [0.0, 0.5], Act::LayeringDarkHidden, "dark_hidden"),
];
const SPRAY_COLD_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (2, [0.0, 0.5], Act::Cold, "element"),
    (12, [0.0, 0.5], Act::Idle, "queued"),
    (80, [0.0, 0.5], Act::Cast, "spray"),
    (180, [0.0, 0.5], Act::Idle, "marks"),
];
const SPRAY_WATER_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (2, [0.0, 0.5], Act::Water, "element"),
    (12, [0.0, 0.5], Act::Idle, "queued"),
    (80, [0.0, 0.5], Act::Cast, "spray"),
    (180, [0.0, 0.5], Act::Idle, "marks"),
];
/// Real input: damage oneself with Arcane, then heal with Life.
const SELF_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Arcane, "arcane"),
    (12, [0., 0.5], Act::Idle, "arcane_queued"),
    (12, [0., 0.5], Act::SelfCast, "arcane_self"),
    (90, [0., 0.5], Act::Idle, "damaged"),
    (2, [0., 0.5], Act::Life, "life"),
    (12, [0., 0.5], Act::Idle, "life_queued"),
    (12, [0., 0.5], Act::SelfCast, "life_self"),
    (90, [0., 0.5], Act::Idle, "healed"),
];

/// Hold an empty-queue push until the native charge clip loops, then release.
const PUSH_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (90, [0., 0.5], Act::Cast, "charging"),
    (120, [0., 0.5], Act::Cast, "charged"),
    (12, [0., 0.5], Act::Idle, "push"),
    (90, [0., 0.5], Act::Idle, "faded"),
];
const BOOK_SCRIPT: &[Step] = &[
    (90, [0.0, 0.0], Act::Idle, "before"),
    (1, [0.0, 0.0], Act::SpawnBook, "spawn"),
    (60, [0.0, 0.0], Act::Idle, "book_visible"),
    (1, [0.0, 0.0], Act::PickupBook, "unlock"),
    (60, [0.0, 0.0], Act::Idle, "collected"),
    (1, [0.0, 0.0], Act::SpawnTimedBook, "timed_spawn"),
    (30, [0.0, 0.0], Act::Idle, "timeout_warning"),
    (90, [0.0, 0.0], Act::Idle, "expired"),
];
const FEATURE_LAB_SCRIPT: &[Step] = &[
    (150, [0.0, 0.0], Act::Idle, "lab_stations"),
    (120, [0.0, 0.0], Act::Idle, "lab_settled"),
    (1, [0.0, 0.0], Act::LabPortal, "portal"),
    (120, [0.0, 0.0], Act::Idle, "corridor"),
];
const PHYSICS_PROPS_SCRIPT: &[Step] = &[
    (90, [0.0, 0.0], Act::Idle, "before"),
    (1, [0.0, 0.0], Act::SpawnPhysicsProp, "spawn"),
    (20, [0.0, 0.0], Act::Idle, "falling"),
    (240, [0.0, 0.0], Act::Idle, "landed"),
];
const PICKUP_PHYSICS_SCRIPT: &[Step] = &[
    (90, [0.0, 0.0], Act::Idle, "before"),
    (1, [0.0, 0.0], Act::SpawnFallingBook, "spawn_above_floor"),
    (20, [0.0, 0.0], Act::Idle, "falling"),
    (140, [0.0, 0.0], Act::Idle, "landed"),
    (1, [0.0, 0.0], Act::PickupBook, "mouse_collect"),
    (60, [0.0, 0.0], Act::Idle, "collected"),
];
const PICKUP_SCRIPT: &[Step] = &[
    (90, [0.0, 0.0], Act::Idle, "before"),
    (1, [0.0, 0.0], Act::SpawnPickup, "spawn"),
    (60, [0.0, 0.0], Act::Idle, "pickup_visible"),
    (1, [0.0, 0.0], Act::Pickup, "exchange"),
    (60, [0.0, 0.0], Act::Idle, "equipped_and_dropped"),
];
/// Idle long enough for an opening scene dialog to type out.
const DIALOG_IDLE_SCRIPT: &[Step] = &[
    (240, [0., 0.5], Act::Idle, "early"),
    (360, [0., 0.5], Act::Idle, "dialog"),
];
/// A named boss fight left to run: the intro, then three battle captures.
const BOSS_IDLE_SCRIPT: &[Step] = &[
    (240, [0., 0.5], Act::Idle, "intro"),
    (600, [0., 0.5], Act::Idle, "battle"),
    (600, [0., 0.5], Act::Idle, "later"),
    (600, [0., 0.5], Act::Idle, "late"),
];
const DEATH_BOSS_SCRIPT: &[Step] = &[
    (120, [0., 0.5], Act::Idle, "seated"),
    (600, [0., 0.5], Act::Idle, "battle"),
    (600, [0., 0.5], Act::Idle, "clones_pending"),
    (600, [0., 0.5], Act::Idle, "clones"),
];
/// havindr_s7BF: the Machine before the fight, then drilling with its ruler
/// (run with --run-trigger introTalk2done: activateBoss after 5 s, then the
/// orc's nod clip).
const MACHINE_SCRIPT: &[Step] = &[
    (120, [0., 0.5], Act::Idle, "intro"),
    (480, [0., 0.5], Act::Idle, "drilling"),
];
const MACHINE_BREAK_SCRIPT: &[Step] = &[
    (420, [0., 0.5], Act::Idle, "fight"),
    (90, [0., 0.5], Act::MachineBreak, "machine_broken"),
    (120, [0., 0.5], Act::Idle, "rider_breakfree"),
    (150, [0., 0.5], Act::Idle, "window"),
    (180, [0., 0.5], Act::Idle, "rider_done"),
];
/// havindr_s7BF drilling, after walking up-left so the camera frames the
/// drill tip and its MachineRuler.
const MACHINE_RULER_SCRIPT: &[Step] = &[
    (420, [0., 0.5], Act::Idle, "fight"),
    (150, [-0.6, 0.6], Act::Move, "walk"),
    (30, [0., 0.5], Act::Idle, "ruler"),
];
/// ru_s6BF: walk north into Fafnir's hall (he lies ~33 m ahead of the
/// start), then watch him sleep, wake (intro) and fight. The intro dialog
/// types out until about frame 1300 (2026-11-05 capture), so the fight
/// capture waits for it to end.
const FAFNIR_SCRIPT: &[Step] = &[
    (300, [0., 0.9], Act::Move, "hall"),
    (240, [0., 0.5], Act::Idle, "intro"),
    (900, [0., 0.5], Act::Idle, "dialog_done"),
    (600, [0., 0.5], Act::Idle, "fight"),
];
/// ru_s6BF attack survey: the `fafnir` walk and intro, a short run towards
/// the head (LanceLow is only weighted within 8.5 m of it), then an immortal
/// wizard watches ten 4-second windows at full boss health, Fafnir is hit to
/// 34% (Confuse <= 60%, Ceiling <= 40%, quakes per 20%) and ten more follow.
/// The viewer log prints his state, health, lance and breath every 30 frames.
const FAFNIR_ATTACKS_SCRIPT: &[Step] = &[
    (300, [0., 0.9], Act::Move, "hall"),
    (240, [0., 0.5], Act::Idle, "intro"),
    (900, [0., 0.5], Act::Idle, "dialog_done"),
    (45, [0., 0.9], Act::Move, "closer"),
    (240, [0., 0.5], Act::Idle, "full_1"),
    (240, [0., 0.5], Act::Idle, "full_2"),
    (240, [0., 0.5], Act::Idle, "full_3"),
    (240, [0., 0.5], Act::Idle, "full_4"),
    (240, [0., 0.5], Act::Idle, "full_5"),
    (240, [0., 0.5], Act::Idle, "full_6"),
    (240, [0., 0.5], Act::Idle, "full_7"),
    (240, [0., 0.5], Act::Idle, "full_8"),
    (240, [0., 0.5], Act::Idle, "full_9"),
    (240, [0., 0.5], Act::Idle, "full_10"),
    (240, [0., 0.5], Act::Idle, "low_1"),
    (240, [0., 0.5], Act::Idle, "low_2"),
    (240, [0., 0.5], Act::Idle, "low_3"),
    (240, [0., 0.5], Act::Idle, "low_4"),
    (240, [0., 0.5], Act::Idle, "low_5"),
    (240, [0., 0.5], Act::Idle, "low_6"),
    (240, [0., 0.5], Act::Idle, "low_7"),
    (240, [0., 0.5], Act::Idle, "low_8"),
    (240, [0., 0.5], Act::Idle, "low_9"),
    (240, [0., 0.5], Act::Idle, "low_10"),
];
/// ru_s6BF fire breath: the `fafnir` hall walk and intro (the cutscene
/// walks the wizard to its own spot, so a teleport does not hold), then the
/// immortal wizard runs north-east from (-0.9, -23.9) to within 8.5 m of
/// the head zone near (10.7, -34.3), where FirelanceLow's weight is 1.2
/// (head-distance² clamped 9..73), and twelve windows follow.
const FAFNIR_BREATH_SCRIPT: &[Step] = &[
    (300, [0., 0.9], Act::Move, "hall"),
    (240, [0., 0.5], Act::Idle, "intro"),
    (900, [0., 0.5], Act::Idle, "dialog_done"),
    (140, [0.72, 0.54], Act::Move, "approach"),
    (240, [0., 0.5], Act::Idle, "near_1"),
    (240, [0., 0.5], Act::Idle, "near_2"),
    (240, [0., 0.5], Act::Idle, "near_3"),
    (240, [0., 0.5], Act::Idle, "near_4"),
    (240, [0., 0.5], Act::Idle, "near_5"),
    (240, [0., 0.5], Act::Idle, "near_6"),
    (240, [0., 0.5], Act::Idle, "near_7"),
    (240, [0., 0.5], Act::Idle, "near_8"),
    (240, [0., 0.5], Act::Idle, "near_9"),
    (240, [0., 0.5], Act::Idle, "near_10"),
    (240, [0., 0.5], Act::Idle, "near_11"),
    (240, [0., 0.5], Act::Idle, "near_12"),
];
/// ru_s6BF wing: after the hall walk and intro the wizard runs to about
/// (-2.6, -31.6), within 10 m of the spine base (logged near
/// (-5.9, 2.3, -38.5)) and on its world +Z side (Wing's weight needs the
/// flat spine-base-to-avatar direction's Z above 0.5); twelve windows
/// follow. He is healed and re-placed rather than immortal, since an
/// immortal character ignores pushes.
const FAFNIR_WING_SCRIPT: &[Step] = &[
    (300, [0., 0.9], Act::Move, "hall"),
    (240, [0., 0.5], Act::Idle, "intro"),
    (900, [0., 0.5], Act::Idle, "dialog_done"),
    (95, [-0.24, 0.87], Act::Move, "approach"),
    (240, [0., 0.5], Act::Idle, "behind_1"),
    (240, [0., 0.5], Act::Idle, "behind_2"),
    (240, [0., 0.5], Act::Idle, "behind_3"),
    (240, [0., 0.5], Act::Idle, "behind_4"),
    (240, [0., 0.5], Act::Idle, "behind_5"),
    (240, [0., 0.5], Act::Idle, "behind_6"),
    (240, [0., 0.5], Act::Idle, "behind_7"),
    (240, [0., 0.5], Act::Idle, "behind_8"),
    (240, [0., 0.5], Act::Idle, "behind_9"),
    (240, [0., 0.5], Act::Idle, "behind_10"),
    (240, [0., 0.5], Act::Idle, "behind_11"),
    (240, [0., 0.5], Act::Idle, "behind_12"),
];
/// The frame of `FAFNIR_ATTACKS_SCRIPT` at which Fafnir's body is hit.
const FAFNIR_ATTACKS_HIT_FRAME: u32 = 300 + 240 + 900 + 45 + 10 * 240;
/// hl_s5BF after skip_intro: the torchers raid house0 (~20 s) until it
/// explodes and its CountryDefense icon shows the burnt slice.
const HIGHLANDS_RAID_SCRIPT: &[Step] = &[
    (900, [0., 0.5], Act::Idle, "raid"),
    (1500, [0., 0.5], Act::Idle, "burnt"),
];
/// Open the InventoryBox with Tab, let its scale animation finish.
const INVENTORY_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "settled"),
    (1, [0., 0.5], Act::Inventory, "open"),
    (30, [0., 0.5], Act::Idle, "inventory"),
];
/// ew2_s2: Assatur descends (activateBoss at 6 s), then fights.
const ASSATUR_SCRIPT: &[Step] = &[
    (300, [0., 0.5], Act::Idle, "descend"),
    (600, [0., 0.5], Act::Idle, "fight"),
    (600, [0., 0.5], Act::Idle, "casting"),
];
const DEBUG_IDLE_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "settled"),
    (30, [0., 0.5], Act::Idle, "debug"),
];
/// Virtual-pad local co-op audit (`--qa-virtual-pads`): joins by frame ~130,
/// walks 180..260.
const LOCAL_COOP_SCRIPT: &[Step] = &[
    (150, [0., 0.5], Act::Idle, "joined"),
    (120, [0., 0.5], Act::Idle, "walked"),
    (40, [0., 0.5], Act::Idle, "done"),
];
const AREA_PUSH_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (90, [0., 0.5], Act::AreaCast, "charging"),
    (120, [0., 0.5], Act::AreaCast, "charged"),
    (12, [0., 0.5], Act::Idle, "push"),
    (90, [0., 0.5], Act::Idle, "faded"),
];

// Inspect charging away from the starting cart's occlusion.
const CHARGE_FORCE_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "start"),
    (90, [-0.6, 0.0], Act::Move, "walked_out"),
    (60, [0., 0.5], Act::Idle, "idle"),
    (90, [0., 0.5], Act::Cast, "charging"),
    (120, [0., 0.5], Act::Cast, "charged"),
    (12, [0., 0.5], Act::Idle, "push"),
    (90, [0., 0.5], Act::Idle, "faded"),
];
const CHARGE_AREA_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "start"),
    (90, [-0.6, 0.0], Act::Move, "walked_out"),
    (60, [0., 0.5], Act::Idle, "idle"),
    (90, [0., 0.5], Act::AreaCast, "charging"),
    (120, [0., 0.5], Act::AreaCast, "charged"),
    (12, [0., 0.5], Act::Idle, "push"),
    (90, [0., 0.5], Act::Idle, "faded"),
];

/// Conjure Earth, charge a Force projectile for 0.75 s and release;
/// captures follow the rock's flight.
const ROCK_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Earth, "earth"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (45, [0., 0.5], Act::Cast, "charged"),
    (4, [0., 0.5], Act::Idle, "flight_a"),
    (4, [0., 0.5], Act::Idle, "flight_b"),
    (6, [0., 0.5], Act::Idle, "flight_c"),
    (90, [0., 0.5], Act::Idle, "landed"),
];

/// Real Water+Cold input, charged Force release, facing across the screen.
/// The missile render audit records the actual root transform before stepping.
const ICE_FLIGHT_SCRIPT: &[Step] = &[
    (90, [0.55, 0.0], Act::Idle, "idle"),
    (2, [0.55, 0.0], Act::Water, "water"),
    (12, [0.55, 0.0], Act::Idle, "gap"),
    (2, [0.55, 0.0], Act::Cold, "ice"),
    (12, [0.55, 0.0], Act::Idle, "queued"),
    (45, [0.55, 0.0], Act::Cast, "charged"),
    (4, [0.55, 0.0], Act::Idle, "flight_a"),
    (4, [0.55, 0.0], Act::Idle, "flight_b"),
    (6, [0.55, 0.0], Act::Idle, "flight_c"),
    (90, [0.55, 0.0], Act::Idle, "landed"),
];

/// `--autopilot-script magick-hint`: unlock every magick, scroll up from
/// None to Amalgameddon (Arcane, Water, Lightning) and let the hint fade in,
/// queue Arcane over its first slot, then scroll down back to None.
const MAGICK_HINT_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (1, [0.0, 0.5], Act::UnlockMagicks, "unlock"),
    (1, [0.0, 0.5], Act::MagickPrev, "prev"),
    (40, [0.0, 0.5], Act::Idle, "hint"),
    (2, [0.0, 0.5], Act::Arcane, "arcane"),
    (20, [0.0, 0.5], Act::Idle, "queue_over_hint"),
    (1, [0.0, 0.5], Act::MagickNext, "next"),
    (40, [0.0, 0.5], Act::Idle, "cleared"),
];

/// `--autopilot-script rain`: unlock magicks, Water, (cooldown) Water, Fire
/// and Space: the Rain magick. Captures before, while the light dims, at
/// full rain, and after it ended.
const RAIN_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (1, [0.0, 0.5], Act::UnlockMagicks, "unlock"),
    (2, [0.0, 0.5], Act::Water, "water1"),
    (60, [0.0, 0.5], Act::Idle, "cooldown"),
    (2, [0.0, 0.5], Act::Water, "water2"),
    (8, [0.0, 0.5], Act::Idle, "gap"),
    (2, [0.0, 0.5], Act::Fire, "steam"),
    (8, [0.0, 0.5], Act::Idle, "queued"),
    (2, [0.0, 0.5], Act::Boost, "cast"),
    (30, [0.0, 0.5], Act::Idle, "dimming"),
    (90, [0.0, 0.5], Act::Idle, "raining"),
    // cast_magick_global plays about 2 s before Rain executes; 17.5 s of
    // rain and the light's 1/s recovery fit in 22 s.
    (1320, [0.0, 0.5], Act::Idle, "ended"),
];

/// `--autopilot-script tornado`: Earth, Steam, Water, Steam and Space. Captures
/// the funnel fading in, at full strength, wandering, and after it expired.
const TORNADO_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (1, [0.0, 0.5], Act::UnlockMagicks, "unlock"),
    (2, [0.0, 0.5], Act::Earth, "earth"),
    (8, [0.0, 0.5], Act::Idle, "gap1"),
    (2, [0.0, 0.5], Act::Water, "water1"),
    (8, [0.0, 0.5], Act::Idle, "gap2"),
    (2, [0.0, 0.5], Act::Fire, "steam1"),
    (8, [0.0, 0.5], Act::Idle, "gap3"),
    (2, [0.0, 0.5], Act::Water, "water2"),
    (60, [0.0, 0.5], Act::Idle, "cooldown"),
    (2, [0.0, 0.5], Act::Water, "water3"),
    (8, [0.0, 0.5], Act::Idle, "gap4"),
    (2, [0.0, 0.5], Act::Fire, "steam2"),
    (8, [0.0, 0.5], Act::Idle, "queued"),
    (2, [0.0, 0.5], Act::Boost, "cast"),
    (75, [0.0, 0.5], Act::Idle, "spawned"),
    (120, [0.0, 0.5], Act::Idle, "funnel"),
    (300, [0.0, 0.5], Act::Idle, "wandering"),
    (900, [0.0, 0.5], Act::Idle, "ended"),
];

/// `--autopilot-script vortex`: Ice, Arcane, Ice, Shield, Ice and Space (with
/// the demo goblin). Captures the suction sphere opening and collapsing.
const VORTEX_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (1, [0.0, 0.5], Act::UnlockMagicks, "unlock"),
    (2, [0.0, 0.5], Act::Water, "water1"),
    (8, [0.0, 0.5], Act::Idle, "gap1"),
    (2, [0.0, 0.5], Act::Cold, "ice1"),
    (8, [0.0, 0.5], Act::Idle, "gap2"),
    (2, [0.0, 0.5], Act::Arcane, "arcane"),
    (8, [0.0, 0.5], Act::Idle, "gap3"),
    (2, [0.0, 0.5], Act::Water, "water2"),
    (8, [0.0, 0.5], Act::Idle, "gap4"),
    (2, [0.0, 0.5], Act::Cold, "ice2"),
    (8, [0.0, 0.5], Act::Idle, "gap5"),
    (2, [0.0, 0.5], Act::Shield, "shield"),
    (8, [0.0, 0.5], Act::Idle, "gap6"),
    (2, [0.0, 0.5], Act::Water, "water3"),
    (8, [0.0, 0.5], Act::Idle, "gap7"),
    (2, [0.0, 0.5], Act::Cold, "ice3"),
    (8, [0.0, 0.5], Act::Idle, "queued"),
    (2, [0.0, 0.5], Act::Boost, "cast"),
    (150, [0.0, 0.5], Act::Idle, "opened"),
    (20, [0.0, 0.5], Act::Idle, "pulling1"),
    (20, [0.0, 0.5], Act::Idle, "pulling2"),
    (40, [0.0, 0.5], Act::Idle, "pulling3"),
    (80, [0.0, 0.5], Act::Idle, "pulling4"),
    (600, [0.0, 0.5], Act::Idle, "ended"),
];

/// `--autopilot-script portal`: Steam, Lightning, Shield and Space, a walk
/// back, then a second cast: the orange and blue portals connect.
const PORTAL_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (1, [0.0, 0.5], Act::UnlockMagicks, "unlock"),
    (2, [0.0, 0.5], Act::Water, "water1_1"),
    (8, [0.0, 0.5], Act::Idle, "gap1_1"),
    (2, [0.0, 0.5], Act::Fire, "fire1_1"),
    (8, [0.0, 0.5], Act::Idle, "gap2_1"),
    (2, [0.0, 0.5], Act::Lightning, "lightning1_1"),
    (8, [0.0, 0.5], Act::Idle, "gap3_1"),
    (2, [0.0, 0.5], Act::Shield, "shield1_1"),
    (8, [0.0, 0.5], Act::Idle, "gap4_1"),
    (2, [0.0, 0.5], Act::Boost, "cast1"),
    (90, [0.0, 0.5], Act::Idle, "first"),
    (70, [0.0, -0.5], Act::Move, "walk"),
    (20, [0.0, -0.5], Act::Idle, "turned"),
    (2, [0.0, -0.5], Act::Water, "water1_2"),
    (8, [0.0, -0.5], Act::Idle, "gap1_2"),
    (2, [0.0, -0.5], Act::Fire, "fire1_2"),
    (8, [0.0, -0.5], Act::Idle, "gap2_2"),
    (2, [0.0, -0.5], Act::Lightning, "lightning1_2"),
    (8, [0.0, -0.5], Act::Idle, "gap3_2"),
    (2, [0.0, -0.5], Act::Shield, "shield1_2"),
    (8, [0.0, -0.5], Act::Idle, "gap4_2"),
    (2, [0.0, -0.5], Act::Boost, "cast2"),
    (90, [0.0, -0.5], Act::Idle, "second"),
    (30, [0.0, -0.5], Act::Idle, "connected"),
    (60, [0.0, -0.5], Act::Idle, "ended"),
];

/// `--autopilot-script elemental`: Arcane, Shield, Earth, Steam, Arcane and
/// Space. Captures the ElementalEgg's spawn clip in front of the Wizard.
const ELEMENTAL_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (1, [0.0, 0.5], Act::UnlockMagicks, "unlock"),
    (2, [0.0, 0.5], Act::Arcane, "arcane1"),
    (8, [0.0, 0.5], Act::Idle, "gap1"),
    (2, [0.0, 0.5], Act::Shield, "shield"),
    (8, [0.0, 0.5], Act::Idle, "gap2"),
    (2, [0.0, 0.5], Act::Earth, "earth"),
    (8, [0.0, 0.5], Act::Idle, "gap3"),
    (2, [0.0, 0.5], Act::Water, "water"),
    (8, [0.0, 0.5], Act::Idle, "gap4"),
    (2, [0.0, 0.5], Act::Fire, "fire"),
    (8, [0.0, 0.5], Act::Idle, "gap5"),
    (2, [0.0, 0.5], Act::Arcane, "arcane2"),
    (8, [0.0, 0.5], Act::Idle, "gap6"),
    (2, [0.0, 0.5], Act::Boost, "cast"),
    (60, [0.0, 0.5], Act::Idle, "egg1"),
    (60, [0.0, 0.5], Act::Idle, "egg2"),
    (120, [0.0, 0.5], Act::Idle, "egg3"),
];

/// `--autopilot-script wave`: Earth, Steam, Earth, Steam, Earth and Space.
/// Captures the pillar chain rising and the distortion arc sweeping ahead.
const WAVE_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (1, [0.0, 0.5], Act::UnlockMagicks, "unlock"),
    (2, [0.0, 0.5], Act::Earth, "earth1"),
    (8, [0.0, 0.5], Act::Idle, "gap1"),
    (2, [0.0, 0.5], Act::Water, "water1"),
    (8, [0.0, 0.5], Act::Idle, "gap2"),
    (2, [0.0, 0.5], Act::Fire, "fire1"),
    (8, [0.0, 0.5], Act::Idle, "gap3"),
    (2, [0.0, 0.5], Act::Earth, "earth2"),
    (8, [0.0, 0.5], Act::Idle, "gap4"),
    (2, [0.0, 0.5], Act::Water, "water2"),
    (8, [0.0, 0.5], Act::Idle, "gap5"),
    (2, [0.0, 0.5], Act::Fire, "fire2"),
    (8, [0.0, 0.5], Act::Idle, "gap6"),
    (2, [0.0, 0.5], Act::Earth, "earth3"),
    (8, [0.0, 0.5], Act::Idle, "gap7"),
    (2, [0.0, 0.5], Act::Boost, "cast"),
    (20, [0.0, 0.5], Act::Idle, "sweep1"),
    (10, [0.0, 0.5], Act::Idle, "sweep2"),
    (10, [0.0, 0.5], Act::Idle, "sweep3"),
    (10, [0.0, 0.5], Act::Idle, "sweep4"),
    (10, [0.0, 0.5], Act::Idle, "sweep5"),
    (20, [0.0, 0.5], Act::Idle, "sweep6"),
    (120, [0.0, 0.5], Act::Idle, "ended"),
];

/// `--autopilot-script phoenix`: Life, Lightning, Fire and Space. Captures
/// the descent and the frame after the blast.
const PHOENIX_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (1, [0.0, 0.5], Act::UnlockMagicks, "unlock"),
    (2, [0.0, 0.5], Act::Life, "life"),
    (8, [0.0, 0.5], Act::Idle, "gap1"),
    (2, [0.0, 0.5], Act::Lightning, "lightning"),
    (8, [0.0, 0.5], Act::Idle, "gap2"),
    (2, [0.0, 0.5], Act::Fire, "fire"),
    (8, [0.0, 0.5], Act::Idle, "queued"),
    (2, [0.0, 0.5], Act::Boost, "cast"),
    (90, [0.0, 0.5], Act::Idle, "descending"),
    (40, [0.0, 0.5], Act::Idle, "landing"),
    (300, [0.0, 0.5], Act::Idle, "after"),
];

/// Real Cold/Ice/Cold + Space cast, full overlay and post-expiry frame.
const BLIZZARD_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (1, [0.0, 0.5], Act::UnlockMagicks, "unlock"),
    (2, [0.0, 0.5], Act::Cold, "cold1"),
    (60, [0.0, 0.5], Act::Idle, "cooldown"),
    (2, [0.0, 0.5], Act::Cold, "cold_for_ice"),
    (8, [0.0, 0.5], Act::Idle, "gap"),
    (2, [0.0, 0.5], Act::Water, "ice"),
    (60, [0.0, 0.5], Act::Idle, "cooldown2"),
    (2, [0.0, 0.5], Act::Cold, "cold2"),
    (8, [0.0, 0.5], Act::Idle, "queued"),
    (2, [0.0, 0.5], Act::Boost, "cast"),
    (240, [0.0, 0.5], Act::Idle, "blizzard"),
    (240, [0.0, 0.5], Act::Idle, "blizzard_later"),
    (480, [0.0, 0.5], Act::Idle, "ended"),
];

/// `--autopilot-script haste-invisibility`: Haste (Lightning, Arcane, Fire,
/// Space), run so the afterimage ghosts trail the Wizard, stop; then
/// Invisibility (Arcane, Shield, Water+Fire = Steam, Arcane, Space) and walk
/// while only the distortion pass draws.
const HASTE_INVISIBILITY_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (1, [0.0, 0.5], Act::UnlockMagicks, "unlock"),
    (2, [0.0, 0.5], Act::Lightning, "lightning"),
    (8, [0.0, 0.5], Act::Idle, "gap1"),
    (2, [0.0, 0.5], Act::Arcane, "arcane"),
    (8, [0.0, 0.5], Act::Idle, "gap2"),
    (2, [0.0, 0.5], Act::Fire, "fire"),
    (8, [0.0, 0.5], Act::Idle, "gap3"),
    (2, [0.0, 0.5], Act::Boost, "haste"),
    (60, [0.0, 0.5], Act::Idle, "hasted"),
    (30, [-0.1, 0.7], Act::Move, "haste_running"),
    (60, [-0.1, 0.7], Act::Idle, "haste_stopped"),
    (2, [0.0, 0.5], Act::Arcane, "inv_arcane1"),
    (8, [0.0, 0.5], Act::Idle, "gap4"),
    (2, [0.0, 0.5], Act::Shield, "inv_shield"),
    (8, [0.0, 0.5], Act::Idle, "gap5"),
    (2, [0.0, 0.5], Act::Water, "inv_water"),
    (8, [0.0, 0.5], Act::Idle, "gap6"),
    (2, [0.0, 0.5], Act::Fire, "inv_steam"),
    (40, [0.0, 0.5], Act::Idle, "gap7"),
    (2, [0.0, 0.5], Act::Arcane, "inv_arcane2"),
    (8, [0.0, 0.5], Act::Idle, "gap8"),
    (2, [0.0, 0.5], Act::Boost, "invisibility"),
    (60, [0.0, 0.5], Act::Idle, "invisible"),
    (30, [-0.6, -0.3], Act::Move, "invisible_walking"),
];

const SHIELD_BARS_SCRIPT: &[Step] = &[
    (90, [0.0, 0.5], Act::Idle, "idle"),
    (2, [0.0, 0.5], Act::Shield, "shield"),
    (12, [0.0, 0.5], Act::Idle, "queued"),
    (2, [0.0, 0.5], Act::AreaCast, "sphere"),
    (30, [0.0, 0.5], Act::Idle, "settled"),
    (60, [0.0, -0.6], Act::Move, "walked_back"),
    (30, [0.0, -0.6], Act::Idle, "world_bar"),
    (2, [0.0, -0.6], Act::Shield, "self_queued"),
    (12, [0.0, -0.6], Act::Idle, "gap"),
    (2, [0.0, -0.6], Act::SelfCast, "self_cast"),
    (30, [0.0, -0.6], Act::Idle, "both_bars"),
];

/// S + Area cast: the Arcane area blast (a single press releases at once).
const ARCANE_AREA_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Arcane, "arcane"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (4, [0., 0.5], Act::AreaCast, "cast"),
    (14, [0., 0.5], Act::Idle, "windup"),
    (4, [0., 0.5], Act::Idle, "blast_a"),
    (4, [0., 0.5], Act::Idle, "blast_b"),
    (4, [0., 0.5], Act::Idle, "blast_c"),
    (4, [0., 0.5], Act::Idle, "blast_d"),
    (4, [0., 0.5], Act::Idle, "blast_e"),
    (60, [0., 0.5], Act::Idle, "faded"),
];
/// S D + Area cast: native GroundBlast dust/cracks and colored light.
const EARTH_ARCANE_AREA_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Arcane, "arcane"),
    (12, [0., 0.5], Act::Idle, "gap"),
    (2, [0., 0.5], Act::Earth, "earth"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (4, [0., 0.5], Act::AreaCast, "cast"),
    (14, [0., 0.5], Act::Idle, "windup"),
    (6, [0., 0.5], Act::Idle, "burst"),
    (12, [0., 0.5], Act::Idle, "flash"),
    (30, [0., 0.5], Act::Idle, "dust"),
    (180, [0., 0.5], Act::Idle, "faded"),
];

/// Water + Cold (Ice) Area cast: the GroundBlast's IceSpikes, with a
/// pinned paired frame showing and hiding them.
const ICE_AREA_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Water, "water"),
    (12, [0., 0.5], Act::Idle, "gap"),
    (2, [0., 0.5], Act::Cold, "cold"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (4, [0., 0.5], Act::AreaCast, "cast"),
    (14, [0., 0.5], Act::Idle, "windup"),
    (8, [0., 0.5], Act::Idle, "rising"),
    (20, [0., 0.5], Act::SpikesVisible, "spikes_visible"),
    (20, [0., 0.5], Act::SpikesHidden, "spikes_hidden"),
    (60, [0., 0.5], Act::Idle, "gone"),
];

/// W + Area cast: the Life area blast.
const LIFE_AREA_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Life, "life"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (4, [0., 0.5], Act::AreaCast, "cast"),
    (14, [0., 0.5], Act::Idle, "windup"),
    (4, [0., 0.5], Act::Idle, "blast_a"),
    (4, [0., 0.5], Act::Idle, "blast_b"),
    (4, [0., 0.5], Act::Idle, "blast_c"),
    (4, [0., 0.5], Act::Idle, "blast_d"),
    (4, [0., 0.5], Act::Idle, "blast_e"),
    (60, [0., 0.5], Act::Idle, "faded"),
];

/// D F + charge/release: an Earth+Fire rock whose Blast event starts the
/// FullBlast burst wedges (`area_fire1_b`, `area_ground_earth`) on impact.
const ROCK_FIRE_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Earth, "earth"),
    (12, [0., 0.5], Act::Idle, "queued_e"),
    (2, [0., 0.5], Act::Fire, "fire"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (45, [0., 0.5], Act::Cast, "charged"),
    (6, [0., 0.5], Act::Idle, "flight"),
    (10, [0., 0.5], Act::Idle, "impact_a"),
    (10, [0., 0.5], Act::Idle, "impact_b"),
    (20, [0., 0.5], Act::Idle, "impact_c"),
    (120, [0., 0.5], Act::Idle, "settled"),
];

/// A A + hold right mouse: eight Force lightning bolts over one second.
/// `--autopilot-script ice`: freeze all water (harness), then watch it thaw.
const ICE_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "water"),
    (1, [0., 0.5], Act::FreezeWater, "freeze"),
    (5, [0., 0.5], Act::Idle, "ice"),
    // The harness sets f=2; native Water.Update subtracts 0.1/s.
    (1260, [0., 0.5], Act::Idle, "thawed"),
];

const LIQUID_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "liquid"),
    (240, [0., 0.5], Act::Idle, "flowing"),
];

/// Inspect retained lava freeze values and the resulting rock material.
const LAVA_FREEZE_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "molten"),
    (1, [0., 0.5], Act::FreezeWater, "freeze"),
    (5, [0., 0.5], Act::Idle, "rock"),
    (1260, [0., 0.5], Act::Idle, "remelted"),
];

const DECAL_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "ground"),
    (1, [0., 0.5], Act::DecalFixture, "placed"),
    (30, [0., 0.5], Act::Idle, "decals"),
    (3900, [0., 0.5], Act::Idle, "expired"),
];

/// `--autopilot-script freeze` (with `--demo-goblin`): Water then Cold
/// sprays on the goblin beside the Wizard; capture the ice block.
const FREEZE_SCRIPT: &[Step] = &[
    (300, [-0.12, -0.72], Act::Idle, "idle"),
    (2, [-0.12, -0.72], Act::Water, "water"),
    (12, [-0.12, -0.72], Act::Idle, "queued_water"),
    (60, [-0.12, -0.72], Act::Cast, "wet"),
    (30, [-0.12, -0.72], Act::Idle, "wet_idle"),
    (2, [-0.12, -0.72], Act::Cold, "cold"),
    (12, [-0.12, -0.72], Act::Idle, "queued_cold"),
    (60, [-0.12, -0.72], Act::Cast, "freezing"),
    (30, [-0.12, -0.72], Act::Idle, "frozen"),
    (90, [-0.12, -0.72], Act::Idle, "frozen_later"),
];

/// `--autopilot-script ice-blade`: conjure Ice (Water + Cold), charge the
/// sword with Shift + left mouse, swing it, and pin a paired frame with the
/// IceBlade model shown and hidden.
const ICE_BLADE_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Water, "water"),
    (12, [0., 0.5], Act::Idle, "water_queued"),
    (2, [0., 0.5], Act::Cold, "cold"),
    (12, [0., 0.5], Act::Idle, "cold_queued"),
    (1, [0., 0.5], Act::WeaponSwing, "charge_press"),
    (89, [0., 0.5], Act::ShiftHold, "charged"),
    (14, [0., 0.5], Act::WeaponSwing, "swing"),
    (20, [0., 0.5], Act::IceBladeVisible, "blade_visible"),
    (20, [0., 0.5], Act::IceBladeHidden, "blade_hidden"),
    (16, [0., 0.5], Act::WeaponSwing, "swing_late"),
    (60, [0., 0.5], Act::Idle, "done"),
];

/// Real Earth enchantment and swing, then pin the surviving trail particles.
const EARTH_WEAPON_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Earth, "earth"),
    (12, [0., 0.5], Act::Idle, "earth_queued"),
    (1, [0., 0.5], Act::WeaponSwing, "charge_press"),
    (89, [0., 0.5], Act::ShiftHold, "charged"),
    (45, [0., 0.5], Act::WeaponSwing, "swing"),
    (20, [0., 0.5], Act::EarthTrailVisible, "trail_visible"),
    (20, [0., 0.5], Act::EarthTrailHidden, "trail_hidden"),
    (20, [0., 0.5], Act::EarthTrailVisible, "trail_restored"),
    (180, [0., 0.5], Act::Idle, "done"),
];

/// Network QA sends ordinary inputs; the client does not run the spell host.
const NETWORK_SPRAY_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "joined"),
    (2, [0., 0.5], Act::Fire, "fire"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (120, [0., 0.5], Act::Cast, "remote_spray"),
    (60, [0., 0.5], Act::Idle, "released"),
];

const NETWORK_OBSERVER_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "joined"),
    (134, [0., 0.5], Act::Idle, "remote_spray"),
    (60, [0., 0.5], Act::Idle, "released"),
];
const NETWORK_TIMEOUT_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "joined"),
    (4000, [0., 0.5], Act::Idle, "wait_for_disconnect"),
];
const NETWORK_REJOIN_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "joined"),
    (4000, [0., 0.5], Act::Idle, "wait_for_rejoin"),
];

/// `--autopilot-script arcane-blade`: Arcane + Fire sword charge and swing
/// with the IceBlade pin acts (they hide every weapon blade visual).
const SPELL_LIGHT_BURNING_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Fire, "fire"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (1, [0., 0.5], Act::SelfCast, "ignite"),
    (60, [0., 0.5], Act::Idle, "burning"),
    (20, [0., 0.5], Act::StatusLightsVisible, "light_visible"),
    (20, [0., 0.5], Act::StatusLightsHidden, "light_hidden"),
    (20, [0., 0.5], Act::StatusLightsVisible, "light_restored"),
    (60, [0., 0.5], Act::Idle, "released"),
];
const SPELL_LIGHT_FIRE_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Fire, "fire"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (60, [0., 0.5], Act::Cast, "casting"),
    (20, [0., 0.5], Act::SpellLightsVisible, "light_visible"),
    (20, [0., 0.5], Act::SpellLightsHidden, "light_hidden"),
    (20, [0., 0.5], Act::SpellLightsVisible, "light_restored"),
    (60, [0., 0.5], Act::Idle, "released"),
];
const SPELL_LIGHT_COLD_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Cold, "cold"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (60, [0., 0.5], Act::Cast, "casting"),
    (20, [0., 0.5], Act::SpellLightsVisible, "light_visible"),
    (20, [0., 0.5], Act::SpellLightsHidden, "light_hidden"),
    (20, [0., 0.5], Act::SpellLightsVisible, "light_restored"),
    (60, [0., 0.5], Act::Idle, "released"),
];
const SPELL_LIGHT_BEAM_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Arcane, "arcane"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (60, [0., 0.5], Act::Cast, "casting"),
    (20, [0., 0.5], Act::SpellLightsVisible, "light_visible"),
    (20, [0., 0.5], Act::SpellLightsHidden, "light_hidden"),
    (20, [0., 0.5], Act::SpellLightsVisible, "light_restored"),
    (60, [0., 0.5], Act::Idle, "released"),
];
const SPELL_LIGHT_LIGHTNING_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Lightning, "lightning"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (60, [0., 0.5], Act::Cast, "casting"),
    (20, [0., 0.5], Act::SpellLightsVisible, "light_visible"),
    (20, [0., 0.5], Act::SpellLightsHidden, "light_hidden"),
    (20, [0., 0.5], Act::SpellLightsVisible, "light_restored"),
    (60, [0., 0.5], Act::Idle, "released"),
];

const ARCANE_BLADE_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Arcane, "arcane"),
    (12, [0., 0.5], Act::Idle, "arcane_queued"),
    (2, [0., 0.5], Act::Fire, "fire"),
    (12, [0., 0.5], Act::Idle, "fire_queued"),
    (1, [0., 0.5], Act::WeaponSwing, "charge_press"),
    (89, [0., 0.5], Act::ShiftHold, "charged"),
    (28, [0., 0.5], Act::WeaponSwing, "swing"),
    (20, [0., 0.5], Act::IceBladeVisible, "blade_visible"),
    (20, [0., 0.5], Act::IceBladeHidden, "blade_hidden"),
    (20, [0., 0.5], Act::CapsuleLightVisible, "light_visible"),
    (20, [0., 0.5], Act::CapsuleLightHidden, "light_hidden"),
    (20, [0., 0.5], Act::CapsuleLightVisible, "light_restored"),
    (16, [0., 0.5], Act::WeaponSwing, "swing_late"),
    (60, [0., 0.5], Act::Idle, "done"),
];

/// `--autopilot-script zap` (with `--demo-goblin`): let the goblin come
/// close (it stops beside the Wizard, down-right on screen), then hold a
/// lightning cast on it and capture the zap skeleton.
const ZAP_SCRIPT: &[Step] = &[
    (300, [0., 0.5], Act::Idle, "idle"),
    (2, [-0.12, -0.72], Act::Lightning, "lightning_a"),
    (12, [-0.12, -0.72], Act::Idle, "queued_a"),
    (2, [-0.12, -0.72], Act::Lightning, "lightning_b"),
    (12, [-0.12, -0.72], Act::Idle, "queued"),
    (22, [-0.12, -0.72], Act::Cast, "zap_1"),
    (3, [-0.12, -0.72], Act::Cast, "zap_2"),
    (3, [-0.12, -0.72], Act::Cast, "zap_3"),
    (3, [-0.12, -0.72], Act::Cast, "zap_4"),
    (20, [-0.12, -0.72], Act::ZapFlashVisible, "flash_visible"),
    (20, [-0.12, -0.72], Act::ZapFlashHidden, "flash_hidden"),
    (3, [-0.12, -0.72], Act::Cast, "zap_5"),
    (60, [0., 0.5], Act::Idle, "done"),
];

/// E + self cast: native pure shield body coating, noise and expiry.
const SELF_SHIELD_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Shield, "shield_queued"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (45, [0., 0.5], Act::SelfCast, "cast"),
    (45, [0., 0.5], Act::Idle, "shield"),
    (90, [0., 0.5], Act::Idle, "shield_later"),
    (240, [0., 0.5], Act::Idle, "expired"),
];

const SOLID_SELF_SHIELD_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Shield, "earth_shield"),
    (12, [0., 0.5], Act::Idle, "gap"),
    (2, [0., 0.5], Act::Earth, "earth_queued"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (45, [0., 0.5], Act::SelfCast, "earth_cast"),
    (45, [0., 0.5], Act::Idle, "earth"),
    (2, [0., 0.5], Act::Shield, "ice_shield"),
    (12, [0., 0.5], Act::Idle, "gap"),
    (2, [0., 0.5], Act::Water, "water"),
    (12, [0., 0.5], Act::Idle, "gap"),
    (2, [0., 0.5], Act::Cold, "ice_queued"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (45, [0., 0.5], Act::SelfCast, "ice_cast"),
    (45, [0., 0.5], Act::Idle, "ice"),
    (2, [0., 0.5], Act::Shield, "combined_shield"),
    (12, [0., 0.5], Act::Idle, "gap"),
    (2, [0., 0.5], Act::Earth, "combined_earth"),
    (12, [0., 0.5], Act::Idle, "gap"),
    (2, [0., 0.5], Act::Water, "combined_water"),
    (12, [0., 0.5], Act::Idle, "gap"),
    (2, [0., 0.5], Act::Cold, "combined_queued"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (45, [0., 0.5], Act::SelfCast, "combined_cast"),
    (45, [0., 0.5], Act::Idle, "combined"),
    (2, [0., 0.5], Act::Shield, "aura_shield"),
    (12, [0., 0.5], Act::Idle, "gap"),
    (2, [0., 0.5], Act::Fire, "aura_queued"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (45, [0., 0.5], Act::SelfCast, "aura_cast"),
    (90, [0., 0.5], Act::Idle, "removed"),
];

/// E F + self cast: Fire immunity aura (buff decal and aura ring).
const SHIELD_FIRE_SELF_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Shield, "shield_queued"),
    (12, [0., 0.5], Act::Idle, "queued_a"),
    (2, [0., 0.5], Act::Fire, "fire_queued"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (45, [0., 0.5], Act::SelfCast, "cast"),
    (60, [0., 0.5], Act::Idle, "aura"),
    (60, [0., 0.5], Act::Idle, "aura_later"),
    (1320, [0., 0.5], Act::Idle, "aura_expired"),
];

/// E D + self cast: Earth armour.
const SHIELD_EARTH_SELF_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Shield, "shield_queued"),
    (12, [0., 0.5], Act::Idle, "queued_a"),
    (2, [0., 0.5], Act::Earth, "earth_queued"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (45, [0., 0.5], Act::SelfCast, "cast"),
    (60, [0., 0.5], Act::Idle, "armour"),
];

const LIGHTNING_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Lightning, "lightning_a"),
    (12, [0., 0.5], Act::Idle, "queued_a"),
    (2, [0., 0.5], Act::Lightning, "lightning_b"),
    (12, [0., 0.5], Act::Idle, "queued"),
    // CastSpell fires after the cast animation's spell frame (~18 frames).
    (21, [0., 0.5], Act::Cast, "bolt_1"),
    (8, [0., 0.5], Act::Cast, "bolt_2"),
    (40, [0., 0.5], Act::Cast, "casting"),
    (60, [0., 0.5], Act::Idle, "done"),
];

/// S + hold right mouse: an Arcane Force beam (RailGunSpell) for about
/// 1.3 s at the demo goblin, then release.
const BEAM_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Arcane, "arcane"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (30, [0., 0.5], Act::Cast, "beam_start"),
    (30, [0., 0.5], Act::Cast, "beam"),
    (20, [0., 0.5], Act::Cast, "beam_b"),
    (60, [0., 0.5], Act::Idle, "done"),
];

/// Level.ChangeScene: step into Havindr s1's `trigger_exit`; the fade-out,
/// switch to havindr_s2 and fade-in are captured.
const TRANSITION_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (1, [0., 0.5], Act::Exit, "exit"),
    (30, [0., 0.5], Act::Idle, "fading_out"),
    (40, [0., 0.5], Act::Idle, "switched"),
    (30, [0., 0.5], Act::Idle, "black_after_switch"),
    // havindr_s2 holds black (fade override) and fades in at 13.5 s.
    (700, [0., 0.5], Act::Idle, "before_fade_in"),
    (120, [0., 0.5], Act::Idle, "second_scene"),
];

/// Havindr intro: cutscene bars, first Space shows the skip tip, the second
/// runs `skip_intro` (fade out, reposition, fade in, end cutscene).
/// Checkpoint restart soak: play, restart once, keep running (slowdown and
/// error-loop check after the restart).
const RESTART_SCRIPT: &[Step] = &[
    (600, [0., 0.], Act::Idle, "before"),
    (1, [0., 0.], Act::Restart, "restart"),
    (120, [0., 0.], Act::Idle, "after_2s"),
    (480, [0., 0.], Act::Idle, "after_10s"),
];

const SKIP_SCRIPT: &[Step] = &[
    (120, [0., 0.5], Act::Idle, "intro"),
    (1, [0., 0.5], Act::Skip, "tip_requested"),
    (40, [0., 0.5], Act::Idle, "tip"),
    (1, [0., 0.5], Act::Skip, "skipped"),
    (45, [0., 0.5], Act::Idle, "faded"),
    (60, [0., 0.5], Act::Idle, "fading_in"),
    (120, [0., 0.5], Act::Idle, "playing"),
];

/// Havindr gate: skip the intro, step into `trigger_gate`; the guard
/// dialog/gate cutscene plays `gate` (rendered from the live part pose).
const GATE_SCRIPT: &[Step] = &[
    (120, [0., 0.5], Act::Idle, "intro"),
    (1, [0., 0.5], Act::Skip, "tip"),
    (1, [0., 0.5], Act::Skip, "skip"),
    (240, [0., 0.5], Act::Idle, "skipped"),
    (1, [0., 0.5], Act::Gate, "at_gate"),
    (60, [0., 0.5], Act::Idle, "cutscene"),
    (60, [0., 0.5], Act::Cast, "dialog_a"),
    (60, [0., 0.5], Act::Idle, "dialog_b"),
    (120, [0., 0.5], Act::Idle, "opening"),
    (240, [0., 0.5], Act::Idle, "opened"),
];

/// S F + hold right mouse: an Arcane+Fire beam, drawn through
/// AdditionalRenderData (fire billboards) instead of the arcane spiral.
const BEAM_FIRE_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Arcane, "arcane"),
    (12, [0., 0.5], Act::Idle, "queued_a"),
    (2, [0., 0.5], Act::Fire, "fire"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (30, [0., 0.5], Act::Cast, "beam_start"),
    (30, [0., 0.5], Act::Cast, "beam"),
    (60, [0., 0.5], Act::Idle, "done"),
];

/// E + right mouse: a Shield Force disc; E + Shift+right mouse: an Area
/// sphere around the caster.
/// Pure Shield weapon enchantment, followed by a pinned wall visibility pair.
const SHIELD_WEAPON_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Shield, "shield"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (1, [0., 0.5], Act::WeaponSwing, "charge_press"),
    (89, [0., 0.5], Act::ShiftHold, "charged"),
    (60, [0., 0.5], Act::WeaponSwing, "swing"),
    (20, [0., 0.5], Act::Idle, "wall_up"),
    (20, [0., 0.5], Act::ShieldWallVisible, "wall_visible"),
    (20, [0., 0.5], Act::ShieldWallHidden, "wall_hidden"),
    (20, [0., 0.5], Act::ShieldWallVisible, "wall_restored"),
    (600, [0., 0.5], Act::Idle, "expired"),
];

const SHIELD_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Shield, "shield"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (30, [0., 0.5], Act::Cast, "disc"),
    (30, [0., 0.5], Act::Idle, "disc_up"),
    (2, [0., 0.5], Act::Shield, "shield_b"),
    (12, [0., 0.5], Act::Idle, "queued_b"),
    (30, [0., 0.5], Act::AreaCast, "sphere"),
    (30, [0., 0.5], Act::Idle, "both_up"),
    (900, [0., 0.5], Act::Idle, "expired"),
];

/// E S + Shift+right mouse: a ring of Arcane SpellMines around the caster
/// (spawn clip, then armed and resting).
const MINE_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Shield, "shield"),
    (12, [0., 0.5], Act::Idle, "queued_a"),
    (2, [0., 0.5], Act::Arcane, "arcane"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (30, [0., 0.5], Act::AreaCast, "cast"),
    (10, [0., 0.5], Act::Idle, "rising"),
    (90, [0., 0.5], Act::Idle, "mines_up"),
];

/// E D + right mouse: Solid Earth walls rise ahead of the caster, then
/// decay (1000 HP at 50/s) and sink back.
const EARTH_WALL_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Shield, "shield"),
    (12, [0., 0.5], Act::Idle, "queued_a"),
    (2, [0., 0.5], Act::Earth, "earth"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (30, [0., 0.5], Act::Cast, "cast"),
    (60, [0., 0.5], Act::Idle, "walls_up"),
    (1260, [0., 0.5], Act::Idle, "walls_gone"),
];

/// Real pure-Shield area cast followed by held/released/repressed Space.
const SHIELD_BOOST_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Shield, "shield"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (30, [0., 0.5], Act::AreaCast, "cast"),
    (60, [0., 0.5], Act::Idle, "before"),
    (6, [0., 0.5], Act::Boost, "boost"),
    (30, [0., 0.5], Act::Boost, "held"),
    (2, [0., 0.5], Act::Idle, "released"),
    (6, [0., 0.5], Act::Boost, "repressed"),
];

/// Real Shield + four Earth cast; frozen native-strength/weak/restored pair.
const BARRIER_STRENGTH_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Shield, "shield"),
    (12, [0., 0.5], Act::Idle, "queued_shield"),
    (2, [0., 0.5], Act::Earth, "earth1"),
    (12, [0., 0.5], Act::Idle, "queued1"),
    (2, [0., 0.5], Act::Earth, "earth2"),
    (12, [0., 0.5], Act::Idle, "queued2"),
    (2, [0., 0.5], Act::Earth, "earth3"),
    (12, [0., 0.5], Act::Idle, "queued3"),
    (2, [0., 0.5], Act::Earth, "earth4"),
    (12, [0., 0.5], Act::Idle, "queued4"),
    (30, [0., 0.5], Act::Cast, "cast"),
    (60, [0., 0.5], Act::Idle, "walls_up"),
    // Settle repeated zero-dt direction normalization before the pixel pair.
    (40, [0., 0.5], Act::BarrierStrong, "settled"),
    (20, [0., 0.5], Act::BarrierStrong, "strong"),
    (20, [0., 0.5], Act::BarrierWeak, "weak"),
    (20, [0., 0.5], Act::BarrierStrong, "restored"),
];

/// E D S + right mouse: Earth/Arcane walls with their spinning arcane runes.
const EARTH_ARCANE_WALL_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Shield, "shield"),
    (12, [0., 0.5], Act::Idle, "queued_a"),
    (2, [0., 0.5], Act::Earth, "earth"),
    (12, [0., 0.5], Act::Idle, "queued_b"),
    (2, [0., 0.5], Act::Arcane, "arcane"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (30, [0., 0.5], Act::Cast, "cast"),
    (60, [0., 0.5], Act::Idle, "walls_up"),
    (60, [0., 0.5], Act::Idle, "runes"),
];

/// E F + right mouse: Elemental fire walls (particle walls) on a 4 m arc.
const FIRE_WALL_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (2, [0., 0.5], Act::Shield, "shield"),
    (12, [0., 0.5], Act::Idle, "queued_a"),
    (2, [0., 0.5], Act::Fire, "fire"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (30, [0., 0.5], Act::Cast, "cast"),
    (60, [0., 0.5], Act::Idle, "walls_burning"),
    (300, [0., 0.5], Act::Idle, "walls_gone"),
];

/// Campaign LevelComplete: "Level Complete" fades in and out, then the next
/// campaign level (Havindr → Battlefield) loads.
const COMPLETE_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (1, [0., 0.5], Act::Complete, "complete"),
    (60, [0., 0.5], Act::Idle, "complete_text"),
    (240, [0., 0.5], Act::Idle, "next_level"),
];

/// `--autopilot-script gibs` (with `--demo-goblin`): overkill the goblin,
/// then capture the gore burst, flight, landed gibs and their expiry.
const GIB_SCRIPT: &[Step] = &[
    // The demo goblin walks out from under the trees toward the Wizard.
    (300, [0., 0.5], Act::Idle, "idle"),
    (1, [0., 0.5], Act::Gib, "gib"),
    (6, [0., 0.5], Act::Idle, "burst"),
    (24, [0., 0.5], Act::Idle, "flying"),
    (120, [0., 0.5], Act::Idle, "landed"),
    (1200, [0., 0.5], Act::Idle, "expired"),
];

/// Rewrite-only capture of the shield stage's authored debris/replacement.
const WARRIOR_SHIELD_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "before"),
    (1, [0., 0.5], Act::NpcWarrior, "spawn"),
    (240, [0., 0.5], Act::Idle, "shielded"),
    (1, [0., 0.5], Act::BreakWarriorShield, "hit"),
    (6, [0., 0.5], Act::Idle, "debris"),
    (24, [0., 0.5], Act::Idle, "flight"),
    (120, [0., 0.5], Act::Idle, "landed"),
];

/// The Castle 2 opening's nine physical tower chunks, at fixed frames.
const TOWER_DEBRIS_SCRIPT: &[Step] = &[
    (120, [0., 0.5], Act::Idle, "before"),
    (35, [0., 0.5], Act::Idle, "spawned"),
    (6, [0., 0.5], Act::Idle, "early"),
    (30, [0., 0.5], Act::Idle, "flight"),
    (90, [0., 0.5], Act::Idle, "landed"),
];

/// Defeat presentation: the avatar dies, the end-game text fades in and
/// the level restarts from its start.
const DEFEAT_SCRIPT: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "idle"),
    (1, [0., 0.5], Act::Die, "die"),
    (60, [0., 0.5], Act::Idle, "dead"),
    (180, [0., 0.5], Act::Idle, "defeated_text"),
    (180, [0., 0.5], Act::Idle, "restarted"),
];

fn script_act(script: &[Step], frame: u32) -> Option<Act> {
    let mut start = 0;
    for &(frames, _, act, _) in script {
        if frame < start + frames {
            return Some(act);
        }
        start += frames;
    }
    None
}

fn arg(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1).cloned())
}

/// `joined_level`: the level a joined server reported ([`connect`]).
pub(crate) fn prepare(
    content: &str,
    args: &[String],
    joined_level: Option<&str>,
) -> Result<Prepared, String> {
    let content = std::fs::canonicalize(content).map_err(|e| format!("{content}: {e}"))?;
    let cache = arg(args, "--root").unwrap_or_else(|| "output/.cache".into());
    let cache = std::fs::canonicalize(&cache).map_err(|e| format!("--root {cache}: {e}"))?;
    let mut level = arg(args, "--level").unwrap_or_else(|| "Wizard_Castle.lvl".into());
    let mut character = arg(args, "--character").unwrap_or_else(|| "Wizard".into());
    let robe_colour = robe_colour_arg(args)?;
    let autopilot = arg(args, "--autopilot").map(PathBuf::from);
    // QA captures stay on the constructor defaults, whatever this user chose.
    if autopilot.is_none() {
        save_slots::apply_quality(&cache)?;
    }
    let script = match arg(args, "--autopilot-script").as_deref() {
        Some("cast") => CAST_SCRIPT,
        Some("damage-numbers") => DAMAGE_NUMBERS_SCRIPT,
        Some("decal-layering") => DECAL_LAYERING_SCRIPT,
        Some("spray-cold") => SPRAY_COLD_SCRIPT,
        Some("spell-light-fire") => SPELL_LIGHT_FIRE_SCRIPT,
        Some("spell-light-burning") => SPELL_LIGHT_BURNING_SCRIPT,
        Some("spell-light-cold") => SPELL_LIGHT_COLD_SCRIPT,
        Some("spell-light-beam") => SPELL_LIGHT_BEAM_SCRIPT,
        Some("spell-light-lightning") => SPELL_LIGHT_LIGHTNING_SCRIPT,
        Some("spray-water") => SPRAY_WATER_SCRIPT,
        Some("self") => SELF_SCRIPT,
        Some("push") => PUSH_SCRIPT,
        Some("books") => BOOK_SCRIPT,
        Some("pickup-physics") => PICKUP_PHYSICS_SCRIPT,
        Some("physics-props") => PHYSICS_PROPS_SCRIPT,
        Some("ethereal") => ETHEREAL_SCRIPT,
        Some("tutorial-highlights") => TUTORIAL_HIGHLIGHTS_SCRIPT,
        Some("feature-lab") => FEATURE_LAB_SCRIPT,
        Some("pickups") => PICKUP_SCRIPT,
        Some("debug-idle") => DEBUG_IDLE_SCRIPT,
        Some("local-coop") => LOCAL_COOP_SCRIPT,
        Some("assatur") => ASSATUR_SCRIPT,
        Some("npc-entrance") => entrance_qa::SCRIPT,
        Some("character-lighting") => lighting_qa::SCRIPT,
        Some("npc-ethereal") => lighting_qa::ETHEREAL_SCRIPT,
        Some("npc-ethereal-blend") => lighting_qa::ETHEREAL_BLEND_SCRIPT,
        Some("npc-ethereal-attacks") => lighting_qa::ETHEREAL_ATTACK_SCRIPT,
        Some("npc-ghost-attacks") => lighting_qa::GHOST_ATTACK_SCRIPT,
        Some("npc-projection-attacks") => lighting_qa::PROJECTION_ATTACK_SCRIPT,
        Some("npc-ethereal-attacks-direct") => lighting_qa::ETHEREAL_ATTACK_DIRECT_SCRIPT,
        Some("inventory") => INVENTORY_SCRIPT,
        Some("dialog-idle") => DIALOG_IDLE_SCRIPT,
        Some("death-boss") => DEATH_BOSS_SCRIPT,
        Some("boss-idle") => BOSS_IDLE_SCRIPT,
        Some("death-afterimages") => boss_qa::DEATH_AFTERIMAGES_SCRIPT,
        Some("vlad-afterimages") => vlad_afterimages::SCRIPT,
        Some("grimnir-dreams") => boss_qa::GRIMNIR_SCRIPT,
        Some("grimnir2") => boss_qa::GRIMNIR2_SCRIPT,
        Some("machine") => MACHINE_SCRIPT,
        Some("machine-break") => MACHINE_BREAK_SCRIPT,
        Some("machine-ruler") => MACHINE_RULER_SCRIPT,
        Some("fafnir") => FAFNIR_SCRIPT,
        Some("fafnir-attacks") => FAFNIR_ATTACKS_SCRIPT,
        Some("fafnir-breath") => FAFNIR_BREATH_SCRIPT,
        Some("fafnir-wing") => FAFNIR_WING_SCRIPT,
        Some("highlands-raid") => HIGHLANDS_RAID_SCRIPT,
        Some("highlands-save") => house_qa::SCRIPT,
        Some("area-push") => AREA_PUSH_SCRIPT,
        Some("charge-force") => CHARGE_FORCE_SCRIPT,
        Some("charge-area") => CHARGE_AREA_SCRIPT,
        Some("rock") => ROCK_SCRIPT,
        Some("ice-flight") => ICE_FLIGHT_SCRIPT,
        Some("npc-projectile") => npc_projectile_qa::EARTH,
        Some("npc-laser") => npc_projectile_qa::LASER,
        Some("boss-jormungandr") => boss_qa::SCRIPT,
        Some("boss-warlord-chants") => warlord_qa::SCRIPT,
        Some("boss-jormungandr-motion") => boss_qa::MOTION_SCRIPT,
        Some("npc-bomb") => npc_projectile_qa::BOMB,
        Some("shield-bars") => SHIELD_BARS_SCRIPT,
        Some("magick-hint") => MAGICK_HINT_SCRIPT,
        Some("rain") => RAIN_SCRIPT,
        Some("blizzard") => BLIZZARD_SCRIPT,
        Some("tornado") => TORNADO_SCRIPT,
        Some("phoenix") => PHOENIX_SCRIPT,
        Some("vortex") => VORTEX_SCRIPT,
        Some("wave") => WAVE_SCRIPT,
        Some("portal") => PORTAL_SCRIPT,
        Some("beam-portal") => beam_target_qa::PORTAL,
        Some("beam-elemental") => beam_target_qa::ELEMENTAL,
        Some("elemental") => ELEMENTAL_SCRIPT,
        Some("haste-invisibility") => HASTE_INVISIBILITY_SCRIPT,
        Some("rock-fire") => ROCK_FIRE_SCRIPT,
        Some("arcane-area") => ARCANE_AREA_SCRIPT,
        Some("earth-arcane-area") => EARTH_ARCANE_AREA_SCRIPT,
        Some("self-shield") => SELF_SHIELD_SCRIPT,
        Some("solid-self-shield") => SOLID_SELF_SHIELD_SCRIPT,
        Some("shield-fire-self") => SHIELD_FIRE_SELF_SCRIPT,
        Some("shield-earth-self") => SHIELD_EARTH_SELF_SCRIPT,
        Some("life-area") => LIFE_AREA_SCRIPT,
        Some("shield") => SHIELD_SCRIPT,
        Some("shield-weapon") => SHIELD_WEAPON_SCRIPT,
        Some("mines") => MINE_SCRIPT,
        Some("complete") => COMPLETE_SCRIPT,
        Some("earth-wall") => EARTH_WALL_SCRIPT,
        Some("shield-boost") => SHIELD_BOOST_SCRIPT,
        Some("barrier-strength") => BARRIER_STRENGTH_SCRIPT,
        Some("fire-wall") => FIRE_WALL_SCRIPT,
        Some("earth-arcane-wall") => EARTH_ARCANE_WALL_SCRIPT,
        Some("defeat") => DEFEAT_SCRIPT,
        Some("lightning") => LIGHTNING_SCRIPT,
        Some("beam") => BEAM_SCRIPT,
        Some("beam-fire") => BEAM_FIRE_SCRIPT,
        Some("transition") => TRANSITION_SCRIPT,
        Some("skip") => SKIP_SCRIPT,
        Some("gate") => GATE_SCRIPT,
        Some("outline") => OUTLINE_SCRIPT,
        Some("particle-depth") => particle_depth::SCRIPT,
        Some("restart") => RESTART_SCRIPT,
        Some("cutaway") => cutaway::SCRIPT,
        Some("gibs") => GIB_SCRIPT,
        Some("warrior-shield") => WARRIOR_SHIELD_SCRIPT,
        Some("tower-debris") => TOWER_DEBRIS_SCRIPT,
        Some("zap") => ZAP_SCRIPT,
        Some("ice-blade") => ICE_BLADE_SCRIPT,
        Some("earth-weapon") => EARTH_WEAPON_SCRIPT,
        Some("network-spray") => NETWORK_SPRAY_SCRIPT,
        Some("spell-gallery") => SPELL_GALLERY_SCRIPT,
        Some("spell-observer") => SPELL_OBSERVER_SCRIPT,
        Some("players-page") => spell_gallery::PLAYERS_PAGE_SCRIPT,
        Some("network-observer") => NETWORK_OBSERVER_SCRIPT,
        Some("network-timeout") => NETWORK_TIMEOUT_SCRIPT,
        Some("network-rejoin") => NETWORK_REJOIN_SCRIPT,
        Some("ice-area") => ICE_AREA_SCRIPT,
        Some("arcane-blade") => ARCANE_BLADE_SCRIPT,
        Some("freeze") => FREEZE_SCRIPT,
        Some("ice") => ICE_SCRIPT,
        Some("liquid") => LIQUID_SCRIPT,
        Some("lava-freeze") => LAVA_FREEZE_SCRIPT,
        Some("decals") => DECAL_SCRIPT,
        _ => SCRIPT,
    };
    let shield_bars_inspection = autopilot.is_some() && script == SHIELD_BARS_SCRIPT;
    let map_overview = args.iter().any(|a| a == "--map-overview");
    let charge_inspection = autopilot.is_some() && args.iter().any(|a| a == "--inspect-charge");
    let hide_level = args.iter().any(|a| a == "--hide-level");
    let snapshot = arg(args, "--snapshot").map(PathBuf::from);

    if let Some(joined) = joined_level {
        let explicit = PathBuf::from(&level);
        level = joined.to_owned();
        let authored_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenarios")
            .join(&level);
        if explicit.is_file() && explicit.file_name().and_then(|s| s.to_str()) == Some(&level) {
            level = explicit.to_string_lossy().into_owned();
        } else if authored_path.is_file() {
            level = authored_path.to_string_lossy().into_owned();
        }
    }
    let scripts = !args.iter().any(|a| a == "--no-scripts");
    let versus = arg(args, "--versus")
        .map(|spec| magicka_runtime::ruleset::VersusSettings::parse(&spec))
        .transpose()?;
    // PackMan: the lobby's enabled packs; without the option the session
    // keeps the cache's default pools.
    let packs = arg(args, "--packs")
        .map(|spec| -> Result<_, String> {
            use magicka_runtime::ruleset::PackCatalogue;
            let path = PackCatalogue::default_cache(std::path::Path::new(&content));
            let catalogue =
                PackCatalogue::load(&path).map_err(|e| format!("{}: {e:?}", path.display()))?;
            Ok(catalogue.pools(&catalogue.selection(&spec)))
        })
        .transpose()?;
    let mut session =
        PlaySession::load(&content, &level, &character).map_err(|e| format!("{level}: {e:?}"))?;
    save_slots::restore(&mut session, args, &cache, &content)?;
    character = session
        .player_robe(0)
        .ok_or("missing restored player")?
        .into();
    if let Some(scene) = arg(args, "--scene") {
        // `--scene-spawn` picks the scene's spawn point (default "start").
        let spawn = arg(args, "--scene-spawn").unwrap_or_else(|| "start".into());
        session
            .change_scene_named(&scene, &spawn)
            .map_err(|e| format!("{scene}: {e:?}"))?;
    }
    if !scripts {
        session.disable_scripts();
    }
    // The local GamerTag and colour (statistics sheets, Versus panels).
    session.player_profiles[0] = Some((
        arg(args, "--name").unwrap_or_else(|| "Player".into()),
        robe_colour,
    ));
    // QA/inspection only: a dormant second player (Versus HUD captures).
    if args.iter().any(|a| a == "--qa-second-player")
        && session.join_player_at_start(1, &character).is_err()
    {
        session
            .join_player(1, &character, "versus_spawn")
            .map_err(|e| format!("second player: {e:?}"))?;
    }
    if let Some(v) = versus {
        session
            .set_versus(v, v.default_teams(session.world.players.len().max(4)))
            .map_err(|e| format!("versus: {e:?}"))?;
    }
    if let Some(p) = &packs {
        session.set_survival_packs(p.clone());
    }
    // QA/inspection only: run a named scene trigger (conditions ignored).
    if let Some(trigger) = arg(args, "--run-trigger") {
        run_inspection_trigger(&mut session, &trigger)?;
    }
    // QA/inspection only: place the first avatar at `--teleport X,Y,Z`.
    if let Some(value) = arg(args, "--teleport") {
        let p = value
            .split(',')
            .map(str::parse::<f32>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .filter(|p| p.len() == 3)
            .ok_or("--teleport expects X,Y,Z")?;
        let body = session.avatar().body;
        session.world.physics.bodies[body].position = [p[0], p[1], p[2]];
        session.world.physics.bodies[body].velocity = [0.0; 3];
    }
    // QA/inspection only: centre the view on a real liquid triangle rather
    // than assuming the level's initial player camera contains water/lava.
    let liquid_target = arg(args, "--inspect-liquid")
        .map(|kind| liquid_inspection_target(&session.world.level, &kind))
        .transpose()?;
    if liquid_target.is_some() {
        // Inspection scenes may have no floor at the default player spawn.
        // Keep the off-camera harness player from falling and restarting.
        let body = session.avatar().body;
        session.world.physics.bodies[body].apply_gravity = false;
        session.world.physics.bodies[body].velocity = [0.0; 3];
    }
    let network_host = arg(args, "--network-host");
    if network_host.is_some() && joined_level.is_some() {
        return Err("choose host or join".into());
    }
    let demo_goblin = args.iter().any(|a| a == "--demo-goblin");
    if demo_goblin {
        for _ in 0..30 {
            session.step(1.0 / 60.0);
        }
        if let Err(e) = session.spawn_npc("Goblin_pirate", "attack", 7.0) {
            eprintln!("demo goblin: {e}");
        }
    }

    let level_glb = level_glb_path(&session.model_path);
    let character_glb = format!("gltf/Data/Characters/{character}.glb");
    let authored = session.model_path.extension().is_some_and(|e| e == "arena");
    for rel in [(!authored).then_some(&level_glb), Some(&character_glb)]
        .into_iter()
        .flatten()
    {
        if !cache.join(rel).is_file() {
            return Err(format!(
                "missing exported asset {} (run level-gltf / character-gltf into --root)",
                cache.join(rel).display()
            ));
        }
    }

    let save_binding = save_slots::prepare(&mut session, args, &cache)?;
    Ok(Prepared {
        content,
        cache,
        args: args.to_vec(),
        session,
        level,
        character,
        robe_colour,
        autopilot,
        script,
        map_overview,
        charge_inspection,
        shield_bars_inspection,
        hide_level,
        snapshot,
        network_host,
        scripts,
        demo_goblin,
        versus,
        packs,
        liquid_target,
        level_glb,
        character_glb,
        authored,
        save_binding,
    })
}

/// `--lobby players=8,hotjoin=off,...` onto the mode's defaults (Versus
/// locks hot join). Rewrite extension; the original had four players.
/// The game's key and dungeon master credential, for hosting and joining:
/// `--passphrase-file FILE` or `MAGICKA_PASSPHRASE`, `--dm-key-file FILE` or
/// `MAGICKA_DM_KEY`. `--passphrase` carries what was typed into the menu, within
/// this process; give it on a real command line and other local users can read it.
pub(crate) fn network_security(
    args: &[String],
) -> Result<magicka_network::security::HostSecurity, String> {
    use magicka_network::security::{secret_from, HostSecurity};
    let passphrase = match arg(args, "--passphrase") {
        Some(typed) => Some(typed),
        None => secret_from(
            arg(args, "--passphrase-file").as_deref(),
            "MAGICKA_PASSPHRASE",
        )?,
    };
    let mut security = match passphrase {
        Some(passphrase) => HostSecurity::private(&passphrase)?,
        None => HostSecurity::default(),
    };
    security.dm_credential = secret_from(arg(args, "--dm-key-file").as_deref(), "MAGICKA_DM_KEY")?;
    Ok(security)
}

pub(crate) fn lobby_settings(
    args: &[String],
) -> Result<magicka_network::lobby::LobbySettings, String> {
    use magicka_network::lobby::LobbySettings;
    let base = if args.iter().any(|a| a == "--versus") {
        LobbySettings::versus()
    } else {
        LobbySettings::default()
    };
    match arg(args, "--lobby") {
        Some(spec) => base.parse_onto(&spec),
        None => Ok(base),
    }
}

/// `--mods DIR [--enable a,b] [--mods-base DIR]` (rewrite extension): the
/// enabled mods (default: `DIR/load_order.toml`) over the authored base.
pub(crate) fn mod_set(args: &[String]) -> Result<Option<magicka_mods::ModSet>, String> {
    let Some(dir) = arg(args, "--mods") else {
        return Ok(None);
    };
    let base = arg(args, "--mods-base").map(PathBuf::from).or_else(|| {
        let authored = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game/base");
        authored.is_dir().then_some(authored)
    });
    let enabled: Option<Vec<String>> =
        arg(args, "--enable").map(|list| list.split(',').map(|s| s.trim().to_string()).collect());
    magicka_mods::ModSet::load(
        base.as_deref(),
        std::path::Path::new(&dir),
        enabled.as_deref(),
    )
    .map(Some)
    .map_err(|e| format!("mods: {e}"))
}
/// The rules hash a joining client states (must equal the host's).
fn client_rules_hash(args: &[String]) -> Result<Option<String>, String> {
    let Some(set) = mod_set(args)? else {
        return Ok(None);
    };
    let defs = magicka_definitions::Definitions::load(&set).map_err(|e| format!("mods: {e}"))?;
    let mut manifest = set.content_manifest()?;
    if let Some(hash) = &mut manifest.rules_hash {
        *hash = magicka_mods::sha256_hex(format!("{hash}{}", defs.rules_hash()).as_bytes());
    }
    Ok(manifest.rules_hash)
}

/// `--robe-colour r,g,b` (linear multipliers; default white).
fn robe_colour_arg(args: &[String]) -> Result<[f32; 3], String> {
    let Some(value) = arg(args, "--robe-colour") else {
        return Ok([1.0; 3]);
    };
    let error = "--robe-colour expects three finite nonnegative RGB multipliers";
    let rgb = value
        .split(',')
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| error.to_string())?;
    if !magicka_network::session::valid_player_colour(&rgb) {
        return Err(error.into());
    }
    Ok([rgb[0], rgb[1], rgb[2]])
}

/// `--network-join`: connect on the calling thread (the view owns a
/// loopback App) and report the server's level for [`prepare`].
pub(crate) fn connect(args: &[String]) -> Result<Option<Client>, String> {
    // `--role player|spectator|dm|invader` (rewrite extension).
    let role = match arg(args, "--role") {
        Some(r) => magicka_network::lobby::Role::parse(&r).ok_or(format!("unknown role {r:?}"))?,
        None => magicka_network::lobby::Role::Player,
    };
    let name = arg(args, "--name").unwrap_or_else(|| role.label().into());
    // Protocol 11: a player's selected profile is part of the join request
    // and validated by the host before its wizard exists.
    let profile = (role == magicka_network::lobby::Role::Player)
        .then(|| -> Result<_, String> {
            Ok(magicka_network::session::ProfileSubmit {
                profile: magicka_network::session::PlayerProfile {
                    name: name.clone(),
                    robe: arg(args, "--character").unwrap_or_else(|| "Wizard".into()),
                    colour: robe_colour_arg(args)?,
                },
                profile_id: arg(args, "--profile-id").filter(|id| !id.is_empty()),
            })
        })
        .transpose()?;
    if let Some(problem) = profile.as_ref().and_then(|p| p.problem()) {
        return Err(format!("cannot join: {problem}"));
    }
    arg(args, "--network-join")
        .map(|address| {
            // Derived once: it is deliberately slow.
            let security = network_security(args)?;
            network::ClientView::connect(
                &address,
                magicka_network::transport::JoinRequest {
                    role,
                    name: name.clone(),
                    profile: profile.clone(),
                    rules_hash: client_rules_hash(args)?,
                    key: security.key,
                    credential: security.dm_credential,
                    ..Default::default()
                },
            )
            .map(|c| Client(Box::new(c)))
        })
        .transpose()
}
pub(crate) struct Client(Box<network::ClientView>);
impl Client {
    pub(crate) fn level(&self) -> &str {
        &self.0.frame.level
    }
}

pub fn run(content: &str, args: &[String]) -> Result<(), String> {
    let client = connect(args)?;
    let prepared = prepare(content, args, client.as_ref().map(Client::level))?;
    let mut app = App::new();
    let asset = AssetPlugin {
        file_path: prepared.cache.to_string_lossy().into_owned(),
        ..default()
    };
    if prepared.autopilot.is_some() {
        app.add_plugins(
            DefaultPlugins
                .set(asset)
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        )
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 120.0,
        )));
    } else {
        app.add_plugins(DefaultPlugins.set(asset).set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("Magicka rewrite — {}", prepared.level),
                ..default()
            }),
            ..default()
        }));
    }
    // Audio is on for interactive play; autopilot captures stay silent
    // unless `--audio` is given.
    let audio = !args.iter().any(|a| a == "--silent")
        && (prepared.autopilot.is_none() || args.iter().any(|a| a == "--audio"));
    install(&mut app, &prepared.content, audio);
    app.add_systems(Last, exit_on_leave.in_set(lifecycle::SessionEnd));
    insert_session(app.world_mut(), prepared, client)?;
    app.add_systems(Startup, |world: &mut World| {
        world.run_schedule(lifecycle::PlayStartup)
    })
    .add_systems(PostStartup, |world: &mut World| {
        world.run_schedule(lifecycle::PlayPostStartup)
    });
    app.run();
    crate::finish_process(app)
}

fn exit_on_leave(
    mut leave: MessageReader<lifecycle::LeaveSession>,
    mut exit: MessageWriter<AppExit>,
    network: Option<NonSend<network::NetworkPlay>>,
    mut pending: Local<bool>,
) {
    if leave.read().next().is_some() {
        *pending = true;
    }
    let finished = match network.as_deref() {
        Some(network::NetworkPlay::Client(client)) => client.leave_finished(),
        _ => true,
    };
    if *pending && finished {
        exit.write(AppExit::Success);
    }
}

pub(crate) fn network_departure_finished(world: &World) -> bool {
    match world.get_non_send::<network::NetworkPlay>() {
        Some(network::NetworkPlay::Client(client)) => client.leave_finished(),
        _ => true,
    }
}

/// Session inputs computed without ECS access, so a host can prepare them
/// away from its render loop while it presents the loading screen.
pub(crate) struct Prepared {
    content: PathBuf,
    cache: PathBuf,
    args: Vec<String>,
    session: PlaySession,
    level: String,
    character: String,
    robe_colour: [f32; 3],
    autopilot: Option<PathBuf>,
    script: &'static [Step],
    map_overview: bool,
    charge_inspection: bool,
    shield_bars_inspection: bool,
    hide_level: bool,
    snapshot: Option<PathBuf>,
    /// Hosting binds in [`insert_session`]: network sessions own a loopback
    /// App, which stays on the host's thread.
    network_host: Option<String>,
    scripts: bool,
    demo_goblin: bool,
    versus: Option<magicka_runtime::ruleset::VersusSettings>,
    packs: Option<magicka_runtime::ruleset::SurvivalPacks>,
    liquid_target: Option<Vec3>,
    level_glb: String,
    character_glb: String,
    authored: bool,
    save_binding: Option<save_slots::SaveBinding>,
}

/// Start a prepared session in a running host application.
pub(crate) fn begin(
    world: &mut World,
    prepared: Prepared,
    client: Option<Client>,
) -> Result<(), String> {
    insert_session(world, prepared, client)?;
    lifecycle::run_startup(world);
    Ok(())
}
pub(crate) use frontend_loading::present as present_loading;

/// InGameMenu.HideInstant before the end-of-game screen capture.
pub(crate) fn hide_pause_menu(world: &mut World) {
    if let Some(mut menu) = world.get_resource_mut::<menu::PauseMenu>() {
        menu.visible = false;
    }
}
pub(crate) use lifecycle::{capture_baseline, tear_down, LeaveSession, SessionEnd};

/// The decoded retail banks, shared with the menu host's audio.
#[derive(Resource, Clone)]
pub(crate) struct AudioLibraryShare(pub Arc<magicka_runtime::audio::AudioLibrary>);

/// Per-session presentation inputs for [`session_startup`].
#[derive(Resource)]
struct SessionSetup {
    content: PathBuf,
    character_glb: String,
    level_path: Option<String>,
    autopilot: Option<PathBuf>,
    script: &'static [Step],
    authored: bool,
    hide_level: bool,
}

fn insert_session(world: &mut World, p: Prepared, client: Option<Client>) -> Result<(), String> {
    save_slots::insert(world, p.save_binding, &p.cache)?;
    // ModShadowResolution; medium keeps Bevy's default shadow map.
    world.insert_resource(bevy::light::DirectionalLightShadowMap {
        size: magicka_runtime::global_settings::shadow_quality()
            .shadow_resolution(bevy::light::DirectionalLightShadowMap::default().size),
    });
    frontend_loading::present(world, &p.content, &p.args)?;
    if let Some(path) = p.snapshot {
        // `--snapshot-frame N`: rendered frames after the avatar animates.
        let at = arg(&p.args, "--snapshot-frame")
            .and_then(|v| v.parse().ok())
            .unwrap_or(240);
        world.insert_resource(WindowSnapshot {
            path,
            frames: 0,
            at,
        });
    }
    // Magenta isolates the avatar silhouette when level geometry is hidden.
    world.insert_resource(ClearColor(if p.hide_level {
        Color::srgb(1.0, 0.0, 1.0)
    } else {
        Color::srgb(0.0, 0.0, 0.0)
    }));
    world.insert_resource(InspectionCamera {
        unlocked_target: if p.map_overview {
            Some(Vec3::ZERO)
        } else {
            p.liquid_target
        },
        tilt: if p.map_overview { -0.5 } else { 0.0 },
        // QA framing: the authored close Havindr camera clips the sphere's
        // native front-rim bar anchor. Keep both bars in this fixture's view.
        zoom: if p.map_overview {
            2.2
        } else if p.shield_bars_inspection || p.charge_inspection {
            0.4
        } else {
            0.0
        },
        ..default()
    });
    world.insert_resource(ScenePath(p.level_glb.clone()));
    world.insert_resource(TerrainUnlit(p.args.iter().any(|a| a == "--unlit")));
    world.insert_resource(NpcAssets::new(&p.cache, &p.content));
    world.insert_resource(crate::robe::RobeColour(p.robe_colour));
    world.insert_resource(SessionSetup {
        content: p.content.clone(),
        character_glb: p.character_glb,
        level_path: (!p.hide_level && !p.authored).then_some(p.level_glb),
        autopilot: p.autopilot,
        script: p.script,
        authored: p.authored,
        hide_level: p.hide_level,
    });
    let bindings = crate::bindings::KeyboardBindings::from_settings(
        &magicka_runtime::saves::SaveStore::new(p.cache.join("saves"))
            .settings()
            .unwrap_or_default(),
    );
    world.insert_resource(Play {
        session: p.session,
        content: p.content,
        cache: p.cache,
        #[cfg(feature = "debug-ui")]
        debug_loadout: None,
        level: p.level,
        character: p.character,
        scripts: p.scripts,
        demo_goblin: p.demo_goblin,
        versus: p.versus,
        packs: p.packs,
        pending: None,
        npc_cues: HashMap::new(),
        npc_seq: 0,
        reloads: 0,
        hide_level: p.hide_level,
        #[cfg(feature = "debug-ui")]
        scenarios: arg(&p.args, "--scenarios")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scenarios")),
        dt: 0.0,
        screen: [AUTOPILOT_SIZE.x as f32, AUTOPILOT_SIZE.y as f32],
        menu_hosted: p.args.iter().any(|a| a == "--menu-loading"),
        complete_after: arg(&p.args, "--audit-complete-after").and_then(|v| v.parse().ok()),
        bindings,
    });
    let coop = local_coop::LocalCoop::from_args(&p.args)?;
    let mut devices = world
        .remove_resource::<crate::controller::LocalDevices>()
        .unwrap_or_default();
    if let Some(primary) = arg(&p.args, "--local-primary") {
        let pads: Vec<_> = world
            .query_filtered::<Entity, With<Gamepad>>()
            .iter(world)
            .collect();
        devices.restore_primary(&primary, pads.into_iter());
    } else if !devices.locked {
        devices.primary = world
            .get_resource::<crate::controller::ControllerInput>()
            .filter(|c| c.active)
            .and_then(|c| c.entity);
    }
    devices.locked = true;
    devices.occupied = coop.pending_pads().collect();
    world.insert_resource(devices);
    world.insert_resource(coop);
    if let Some(audit) = local_coop::JoinAudit::from_args(&p.args) {
        world.insert_resource(audit);
    }
    if let Some(qa) = local_coop::VirtualPads::from_args(&p.args) {
        world.insert_resource(qa);
    }
    if let Some(Client(mut client)) = client {
        // Rewrite extension: predict the owned wizard's movement, rolling back
        // to each host state and replaying what it has not applied yet.
        client.predict = !p.args.iter().any(|a| a == "--no-predict");
        // Presentation definitions of the mods this client loaded.
        if let Some(set) = mod_set(&p.args)? {
            let defs =
                magicka_definitions::Definitions::load(&set).map_err(|e| format!("mods: {e}"))?;
            let defs = std::sync::Arc::new(defs);
            // Registered Magick tables and item display values match the host.
            world
                .resource_mut::<Play>()
                .session
                .set_definitions(defs.clone());
            client.definitions = Some(defs);
        }
        // The profile was admitted with the hello; later edits (debug
        // menu, role changes) still use profile requests.
        client.set_profile_id(arg(&p.args, "--profile-id").filter(|id| !id.is_empty()));
        world.insert_non_send(network::NetworkPlay::Client(client));
    } else if let Some(address) = p.network_host {
        let play = world.resource::<Play>();
        let profile = magicka_network::session::PlayerProfile {
            name: arg(&p.args, "--name").unwrap_or_else(|| "Host".into()),
            robe: play.character.clone(),
            colour: p.robe_colour,
        };
        // `--host-role dm|spectator` (lobby badge): a host without a wizard.
        let host_role = arg(&p.args, "--host-role")
            .map(|r| {
                magicka_network::lobby::Role::parse(&r)
                    .filter(|r| {
                        matches!(
                            r,
                            magicka_network::lobby::Role::DungeonMaster
                                | magicka_network::lobby::Role::Spectator
                        )
                    })
                    .ok_or(format!("host role {r:?}: dm or spectator"))
            })
            .transpose()?;
        let address: std::net::SocketAddr =
            address.parse().map_err(|e| format!("host address: {e}"))?;
        let lobby = lobby_settings(&p.args)?;
        let security = network_security(&p.args)?;
        for warning in security.warnings(!address.ip().is_loopback(), lobby.dungeon_masters) {
            warn!("{warning}");
        }
        let hosted = magicka_runtime::network_session::HostedSession::bind_secured(
            address,
            &play.level,
            host_role.is_some(),
            profile,
            &play.session,
            lobby,
            security,
        )?;
        world.resource_mut::<Play>().session.network_mode =
            magicka_runtime::play_session::NetworkMode::Server;
        let mut hosted = hosted;
        if let Some(set) = mod_set(&p.args)? {
            let mods = magicka_runtime::mod_runtime::ModRuntime::from_set(set, hosted.identity)?;
            hosted.set_content(mods.content.rules_hash.clone(), mods.content.mods.clone());
            mods.attach(&mut world.resource_mut::<Play>().session);
            world.insert_non_send(mods);
        }
        hosted.set_host_role(host_role);
        world.insert_non_send(network::NetworkPlay::Host(Box::new(hosted)));
        if let Some(role) = host_role {
            world.insert_resource(HostRole(role));
        }
    } else if let Some(set) = mod_set(&p.args)? {
        // Offline play is its own authority: mods run locally.
        let mods = magicka_runtime::mod_runtime::ModRuntime::from_set(set, 0x5eed)?;
        mods.attach(&mut world.resource_mut::<Play>().session);
        world.insert_non_send(mods);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn session_startup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut text_materials: ResMut<Assets<ColorMaterial>>,
    mut box_materials: ResMut<Assets<TextBoxMaterial>>,
    mut hud_materials: ResMut<Assets<crate::play_hud::HudMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut surface_materials: ResMut<Assets<StandardMaterial>>,
    play: Res<Play>,
    spec: Res<SessionSetup>,
) {
    if spec.authored && !spec.hide_level {
        authored_level::spawn(
            &mut commands,
            &mut meshes,
            &mut surface_materials,
            &play,
            &assets,
        );
    }
    let target = setup(
        &mut commands,
        &assets,
        &mut images,
        &spec.character_glb,
        spec.level_path.as_deref(),
        spec.autopilot.clone(),
        spec.script,
    );
    if let Err(e) = setup_dialogs(
        &mut commands,
        &spec.content,
        target,
        &mut images,
        &mut text_materials,
        &mut box_materials,
        &mut meshes,
    ) {
        error!("dialog rendering unavailable: {e}");
    }
    if let Err(e) = crate::play_hud::setup(
        &mut commands,
        &spec.content,
        &play.session.definitions,
        &mut images,
        &mut hud_materials,
        &mut meshes,
    ) {
        error!("HUD rendering unavailable: {e}");
    }
    if let Err(e) = crate::floating_text::setup(
        &mut commands,
        &spec.content,
        &mut images,
        &mut hud_materials,
        &mut meshes,
    ) {
        error!("damage number rendering unavailable: {e}");
    }
}

/// Every plugin and system of play mode. Session data arrives later through
/// [`begin`]; until then the play systems are idle.
pub(crate) fn install(app: &mut App, content: &std::path::Path, audio: bool) {
    if !app.is_plugin_added::<crate::controller::ControllerPlugin>() {
        app.add_plugins(crate::controller::ControllerPlugin);
    }
    app.add_systems(
        Update,
        controller_context
            .after(lifecycle::PlaySystems)
            .before(crate::controller::ControllerPrompts),
    );
    lifecycle::configure(app);
    app.add_systems(Last, network::begin_leave.before(lifecycle::SessionEnd));
    app.add_systems(Last, spell_gallery::spell_gallery);
    vlad_afterimages::install(app);
    save_slots::plugin(app);
    if audio {
        audio::plugin(app, content);
    }
    menu::plugin(app);
    inventory_visual::plugin(app);
    boss_bar_visual::plugin(app);
    generic_bar_visual::plugin(app);
    machine_ruler::plugin(app);
    country_defense_visual::plugin(app);
    title_visual::plugin(app);
    ruleset_hud::plugin(app);
    credits_visual::plugin(app);
    highlights::plugin(app);
    tick_clock::plugin(app);
    cutaway::plugin(app);
    app.add_play_systems(Update, sync_additional_dialogs.after(step_session));
    app.add_play_systems(Update, authored_level::water_materials);
    app.add_play_systems(Update, leave_after_credits.after(step_session));
    app.add_play_systems(Update, leave_completed_level.after(step_session));
    app.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 1500.0,
        ..default()
    })
    .insert_resource(TerrainNoDiscard(false))
    // Session values replace this; shared robe/outline systems require one.
    .insert_resource(crate::robe::RobeColour([1.0; 3]))
    .add_plugins(TerrainPlugin)
    .add_plugins(crate::blizzard::BlizzardPlugin(content.to_path_buf()))
    .add_plugins(crate::decals::DecalPlugin)
    .add_plugins(crate::particles::ParticlePlugin {
        sheets: crate::particles::SheetSource::Content(content.to_path_buf()),
    })
    .add_plugins((
        crate::play_hud::PlayHudPlugin,
        crate::outline::OutlinePlugin,
        crate::post::PostPlugin,
        crate::zap::ZapPlugin,
        crate::self_shield::SelfShieldPlugin,
        crate::magick_visuals::MagickVisualsPlugin,
        wave_visual::WaveVisualPlugin,
        crate::aura::AuraPlugin,
        NativeTextPlugin,
        TextBoxBackgroundPlugin,
        NativeDialogRenderPlugin::<Play, SceneDialogWorld>::default(),
    ));
    app.add_play_systems(
        Update,
        (
            network::visuals,
            network::physics_visuals,
            network::connection_notice,
            network::role_overlay,
            mod_panel::mod_panel,
        )
            .after(step_session),
    );
    app.add_play_systems(Update, network::rejoin_capture.after(step_session));
    app.add_play_systems(Update, local_coop::local_coop.before(step_session));
    local_coop::install_virtual_pads(app);
    app.add_play_systems(
        Update,
        local_coop::audit_join
            .run_if(resource_exists::<local_coop::JoinAudit>)
            .after(step_session)
            .after(local_coop::local_coop),
    );
    frontend_loading::plugin(app);
    app.add_play_systems(Startup, session_startup);
    app.init_play_resource::<BeamPool>()
        .init_play_resource::<AdditionalBeamPool>()
        .init_play_resource::<ArcaneBlastDraw>()
        .init_play_resource::<ShieldDraws>()
        .init_play_resource::<npc_projectile_qa::Audit>()
        .init_play_resource::<beam_target_qa::Audit>()
        .init_play_resource::<boss_qa::Audit>()
        .init_play_resource::<entrance_qa::Audit>()
        .init_play_resource::<house_qa::Audit>()
        .add_plugins(debug_ui_plugin)
        .add_play_systems(
            Startup,
            (
                setup_end_text,
                fafnir_models::setup_lance,
                parker_models::setup_strands,
                cursor::load_cursors,
                setup_bolt_pool,
                zap_flash::setup_zap_flash,
                setup_fade,
                setup_cutscene_overlay,
            ),
        )
        .add_play_systems(
            Update,
            (
                build_graph,
                attach_animator,
                sync_npcs,
                (sync_missiles, sync_gibs, sync_decals),
                (
                    (sync_mines, animate_mines).chain(),
                    (
                        magick_models::sync_magick_models,
                        magick_models::tint_magick_models
                            .before(crate::lighting::apply_lit_materials),
                        magick_models::animate_magick_models,
                        magick_models::sync_vortex_spheres,
                        wave_visual::sync_wave_arcs,
                        portal_visual::sync_portals,
                    )
                        .chain(),
                    (
                        death_models::sync_models,
                        grimnir_models::sync_models,
                        grimnir2_models::sync_models,
                        assatur_models::sync_models,
                        boss_models::sync_models,
                        death_models::tint_models.before(crate::lighting::apply_lit_materials),
                        grimnir_models::tint_models.before(crate::lighting::apply_lit_materials),
                        machine_models::sync_models,
                        machine_models::split_meshes,
                        machine_models::audit,
                        lighting_qa::blend_visibility,
                        fafnir_models::sync_model,
                        fafnir_models::present_lance,
                        (
                            parker_models::sync_model,
                            cthulhu_models::sync_models,
                            parker_models::present_strands,
                            parker_models::sync_entangle,
                            parker_models::entangle_materials
                                .before(crate::lighting::apply_lit_materials),
                        ),
                        tint_npcs,
                        egg_models::sync_eggs,
                        clip_models::sync_models,
                        clip_models::animate_models,
                    )
                        .chain(),
                    (
                        sync_barriers,
                        animate_barriers,
                        barrier_material::apply_barrier_materials,
                        ice_spikes::sync_ice_spikes,
                        ice_spikes::animate_ice_spikes,
                        barrier_runes::sync_barrier_runes,
                    )
                        .chain(),
                    halo::sync_halos,
                ),
                sync_bolts,
                update_end_text,
                animate_npcs,
                step_session.run_if(frontend_loading::ready),
                (
                    sync_level,
                    level_parts::tag_parts,
                    level_parts::sync_parts,
                    sync_level_lights,
                )
                    .chain(),
                update_cutscene_overlay,
                (
                    (sync_avatar_template, sync_avatar).chain(),
                    sync_outline,
                    sync_post,
                    sync_blizzard,
                    (
                        sync_zap,
                        sync_ethereal,
                        ethereal_lit::sync,
                        grimnir2_models::materials,
                    )
                        .chain(),
                    sync_self_shields,
                    sync_solid_self_shields,
                    sync_water_freeze,
                ),
                (sync_invisibility, sync_afterimages),
                (sync_camera, sync_aura_halos, zap_flash::sync_zap_flashes).chain(),
                (
                    sync_beams,
                    sync_additional_beams,
                    sync_shields,
                    sync_arcane_blasts,
                )
                    .chain(),
                (
                    drive_animation,
                    equipment::sync,
                    equipment::dim_recharging_items,
                    equipment::sync_ice_blade_trail,
                    equipment::sync_arcane_blades,
                    highlights::sync,
                )
                    .chain(),
                (update_native_hud, update_floating_text).chain(),
                (update_keyboard_hud, cursor::update_cursor).chain(),
                autopilot_capture,
                window_snapshot.run_if(frontend_loading::ready),
            )
                .chain(),
        );
    app.add_play_systems(Update, cutaway::fixture.before(step_session));
    app.add_play_systems(
        PostUpdate,
        entrance_qa::visibility
            .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
    );
    app.add_play_systems(
        PostUpdate,
        (
            lighting_qa::audit
                .after(bevy::transform::TransformSystems::Propagate)
                .after(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
            lighting_qa::isolate
                .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
        ),
    );
    app.add_play_systems(
        Update,
        particle_depth::fixture
            .after(step_session)
            .after(sync_camera),
    );
    app.add_play_systems(
        PostUpdate,
        (
            equipment::pose_bodies,
            death_models::pose_body,
            grimnir_models::pose_models,
            grimnir2_models::pose_models,
            assatur_models::pose_models,
            boss_models::pose_models,
            machine_models::pose_models,
            fafnir_models::pose_model,
            parker_models::pose_model,
            cthulhu_models::pose_models,
            egg_models::pose_eggs,
        )
            .after(bevy::app::AnimationSystems)
            .before(bevy::transform::TransformSystems::Propagate),
    );
    app.add_play_systems(
        PostUpdate,
        equipment::verify_pose.after(bevy::transform::TransformSystems::Propagate),
    );
}

/// Shadow-casting directional light standing in for the level sun's shadow map.
#[derive(Component)]
struct PlaySun;

/// Shadow-map source for the level's first shadow-casting spot light
/// (e.g. wc_s3's light_1 over the cell, so the repaired boards shadow the
/// floor). Shading still uses the original light list.
#[derive(Component)]
struct PlaySpot;

/// Copy the live level's light list (minus lights disabled by triggers) into
/// the shared lighting buffer. The first shadow-casting directional light
/// drives the PlaySun shadow map.
fn sync_level_lights(
    play: Res<Play>,
    pilot: Option<Res<Autopilot>>,
    mut lights: ResMut<crate::lighting::MagickaLights>,
    mut sun: Query<(&mut Transform, &mut DirectionalLight), (With<PlaySun>, Without<PlaySpot>)>,
    mut spot: Query<
        (&mut Transform, &mut SpotLight, &mut Visibility),
        (With<PlaySpot>, Without<PlaySun>),
    >,
) {
    let world = &play.session.world;
    lights.fog = play.session.post.fog;
    let mut list = Vec::with_capacity(world.level.lights.len());
    let mut shadow = None;
    let mut spot_shadow: Option<&magicka_content::level::LevelLight> = None;
    for (i, light) in world.level.lights.iter().enumerate() {
        let id = magicka_runtime::scene_loading::custom_hash(&light.name);
        if world.flags.lights_disabled.contains(&id) {
            continue;
        }
        // Light.Update's variation multiplier (candle/flicker/sine/strobe).
        let intensity = play
            .session
            .level_light_intensity
            .get(i)
            .copied()
            .unwrap_or(1.0);
        // GameScene LightSettings.Assign: directional lights' ambient,
        // diffuse and specular scale with mLightIntensity (Rain dims it).
        let intensity = if light.light_type == 1 {
            intensity * world.light_intensity
        } else {
            intensity
        };
        let mut gpu = crate::lighting::GpuLight::from_level(light, intensity);
        if light.light_type == 1 && light.cast_shadows && shadow.is_none() {
            gpu.cone.z = 1.0;
            shadow = Some(gpu.direction.truncate());
        }
        if light.light_type == 2 && light.cast_shadows && spot_shadow.is_none() {
            // Sampled from the PlaySpot shadow map (clustered light 0).
            gpu.cone.z = 2.0;
            spot_shadow = Some(light);
        }
        list.push(gpu);
    }
    // DynamicLights (ArcaneBlast flashes): PointLights whose Draw scales
    // diffuse/ambient/specular by mIntensity · mDynamicIntensity.
    let hide_spell_lights = pilot.as_ref().is_some_and(|p| {
        p.started.is_some_and(|start| {
            script_act(p.script, p.frame - start)
                .is_some_and(|act| matches!(act, Act::SpellLightsHidden | Act::StatusLightsHidden))
        })
    });
    for light in play
        .session
        .spell_point_lights()
        .filter(|_| !hide_spell_lights)
    {
        let k = light.draw_intensity();
        let v = Vec3::from_array;
        list.push(crate::lighting::GpuLight {
            position: v(light.position).extend(light.radius),
            direction: Vec4::ZERO,
            diffuse: (v(light.diffuse) * k).extend(light.specular * k),
            ambient: (v(light.ambient) * k).extend(1.0),
            cone: Vec4::ZERO,
        });
    }
    // Effect `<Light>` (PointLightBatcher) and particle SpawnLights
    // (ParticleLightBatcher): BatchedPointLightEffect instances. Its vertex
    // shader scales diffuse/specular/ambient by Intensity and its pixel
    // shader uses PointLightEffect's sat(1 − d/Radius)² attenuation, so
    // they map onto the same point-light path (research/particles.md).
    for light in play.session.effects.draw_lights() {
        let k = light.intensity;
        let v = Vec3::from_array;
        list.push(crate::lighting::GpuLight {
            position: v(light.position).extend(light.radius),
            direction: Vec4::ZERO,
            diffuse: (v(light.diffuse) * k).extend(light.specular * k),
            ambient: (v(light.ambient) * k).extend(1.0),
            cone: Vec4::ZERO,
        });
    }
    let hide_capsule = pilot.as_ref().is_some_and(|p| {
        p.started.is_some_and(|start| {
            script_act(p.script, p.frame - start) == Some(Act::CapsuleLightHidden)
        })
    });
    if !hide_capsule && !hide_spell_lights {
        for light in play.session.spell_capsule_lights() {
            list.push(crate::lighting::GpuLight::from_capsule(&light));
        }
    }
    if pilot.is_some_and(|p| {
        p.started.is_some_and(|start| {
            matches!(
                script_act(p.script, p.frame - start),
                Some(Act::LayeringDarkVisible | Act::LayeringDarkHidden | Act::LightingDark)
            )
        })
    }) {
        list.clear();
        shadow = None;
    }
    lights.lights = list;
    lights.sun_shadow = shadow.is_some();
    if let Ok((mut transform, mut light, mut visibility)) = spot.single_mut() {
        match spot_shadow {
            Some(l) => {
                let direction = Vec3::from(l.direction).normalize_or(Vec3::NEG_Y);
                let up = if direction.y.abs() > 0.99 {
                    Vec3::Z
                } else {
                    Vec3::Y
                };
                *transform =
                    Transform::from_translation(Vec3::from(l.position)).looking_to(direction, up);
                light.range = l.range;
                light.outer_angle = l.cutoff_angle.min(std::f32::consts::FRAC_PI_2 - 0.01);
                light.inner_angle = light.outer_angle * 0.9;
                light.shadow_maps_enabled = true;
                *visibility = Visibility::Inherited;
            }
            None => {
                light.shadow_maps_enabled = false;
                *visibility = Visibility::Hidden;
            }
        }
    }
    if let Ok((mut transform, mut light)) = sun.single_mut() {
        light.shadow_maps_enabled = shadow.is_some();
        if let Some(direction) = shadow.filter(|d| d.length_squared() > 0.0) {
            *transform = Transform::default().looking_to(direction, Vec3::Y);
        }
    }
}

fn setup(
    commands: &mut Commands,
    assets: &AssetServer,
    images: &mut Assets<Image>,
    character_glb: &str,
    level_glb: Option<&str>,
    autopilot: Option<PathBuf>,
    script: &'static [Step],
) -> Option<Handle<Image>> {
    let level = level_glb.map(|path| {
        let scene: Handle<WorldAsset> =
            assets.load(GltfAssetLabel::Scene(0).from_asset(path.to_string()));
        commands.spawn((
            LevelRoot,
            crate::lighting::OutlineOccluder,
            WorldAssetRoot(scene.clone()),
        ));
        scene
    });
    let gltf = assets.load(character_glb.to_string());
    let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(character_glb.to_string()));
    commands.spawn((
        AvatarRoot,
        crate::lighting::NoDepthPrepass,
        Transform::default(),
        Visibility::default(),
        WorldAssetRoot(scene.clone()),
    ));
    commands.insert_resource(AvatarAssets {
        gltf,
        scene,
        level,
        graph: None,
    });
    // Shadow-map source for the level's shadow-casting directional light;
    // shading itself uses the original light list (`sync_level_lights`).
    commands.spawn((
        PlaySun,
        DirectionalLight {
            illuminance: 4000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        // The gameplay camera sits ~225 units from the ground with a 5° FOV.
        bevy::light::CascadeShadowConfigBuilder {
            num_cascades: 2,
            minimum_distance: 100.0,
            first_cascade_far_bound: 260.0,
            maximum_distance: 450.0,
            ..default()
        }
        .build(),
        Transform::from_xyz(3.0, 10.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // Only its shadow map is used: zero intensity keeps Bevy's own shading
    // out of the custom light sum.
    commands.spawn((
        PlaySpot,
        SpotLight {
            intensity: 0.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::default(),
        Visibility::Hidden,
    ));
    let mut camera = commands.spawn((
        PlayCamera,
        crate::effect_depth::EffectDepthCamera,
        Camera3d::default(),
        // Encoded 1 - 2^-x output; post.rs decodes it after the GUI.
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        // Outline/projection occluders; particles use the full opaque depth
        // snapshot taken by effect_depth.rs after the main opaque pass.
        bevy::core_pipeline::prepass::DepthPrepass,
        // FilterEffect before the GUI (post.rs).
        crate::post::MagickaFilterPass,
        crate::post::MagickaPost::default(),
        crate::blizzard::BlizzardPost::default(),
        Projection::Perspective(PerspectiveProjection {
            near: NEAR_CLIP,
            far: FAR_CLIP,
            ..default()
        }),
        Transform::default(),
    ));
    if script == lighting_qa::ETHEREAL_ATTACK_DIRECT_SCRIPT {
        // Diagnostic control only; Bevy requires this at camera creation.
        camera.insert(bevy::render::view::NoIndirectDrawing);
    }
    if let Some(dir) = autopilot {
        std::fs::create_dir_all(&dir).expect("autopilot dir");
        let mut image = Image::new_target_texture(
            AUTOPILOT_SIZE.x,
            AUTOPILOT_SIZE.y,
            TextureFormat::Rgba8UnormSrgb,
            None,
        );
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        let target = images.add(image);
        camera.insert(RenderTarget::Image(ImageRenderTarget {
            handle: target.clone(),
            scale_factor: 1.0,
        }));
        commands.insert_resource(Autopilot {
            dir,
            frame: 0,
            started: None,
            log: Vec::new(),
            target: target.clone(),
            pending_shot: false,
            anchor: None,
            script,
        });
        Some(target)
    } else {
        None
    }
}

/// Eight regular dialog boxes plus the cutscene box over a 2D overlay camera,
/// with the original Maiandra14/16 atlases and Dialog_Say background.
fn setup_dialogs(
    commands: &mut Commands,
    content: &std::path::Path,
    target: Option<Handle<Image>>,
    images: &mut Assets<Image>,
    text_materials: &mut Assets<ColorMaterial>,
    box_materials: &mut Assets<TextBoxMaterial>,
    meshes: &mut Assets<Mesh>,
) -> Result<(), String> {
    let font = |rel: &str| match read_asset(&content.join(rel))? {
        Asset::BitmapFont(f) => Ok(f),
        _ => Err(format!("{rel}: expected font")),
    };
    let body_font = font("Languages/eng/Font/Maiandra14.xnb")?;
    let name_font = font("Languages/eng/Font/Maiandra16.xnb")?;
    let Asset::Texture2D(background) = read_asset(&content.join("UI/HUD/Dialog_Say.xnb"))? else {
        return Err("expected Dialog_Say texture".into());
    };
    let (body, body_material) = upload_font(&body_font, images, text_materials)?;
    let (mut name, name_material) = upload_font(&name_font, images, text_materials)?;
    if let Some(n) = Arc::get_mut(&mut name) {
        n.metrics.spacing = 1;
    }
    let background_material = upload_background(&background, images, box_materials)?;
    // Zero-vertex meshes trip Bevy's slab allocator ("Use-after-free"), so
    // placeholders are a degenerate (zero-size) background grid instead.
    let empty = || -> Result<Mesh, String> {
        Ok(crate::text_box_visual::background_mesh(
            [0.0; 2], [0.0; 2], 0.0, [1.0; 2],
        ))
    };
    for slot in 0..9 {
        let cutscene = slot == 8;
        let background_mesh = meshes.add(empty()?);
        let background = commands
            .spawn((
                Mesh2d(background_mesh.clone()),
                MeshMaterial2d(background_material.clone()),
                Transform::default(),
                Visibility::Hidden,
            ))
            .id();
        let owner_mesh = meshes.add(empty()?);
        let owner = commands
            .spawn((
                Mesh2d(owner_mesh.clone()),
                MeshMaterial2d(name_material.clone()),
                Transform::default(),
                Visibility::Hidden,
            ))
            .id();
        let binding = if cutscene {
            NativeDialogBox::for_cutscene(
                background,
                background_mesh,
                owner,
                owner_mesh,
                name.clone(),
            )
        } else {
            NativeDialogBox::new(
                slot,
                background,
                background_mesh,
                owner,
                owner_mesh,
                name.clone(),
            )
        };
        // Cutscene text uses Maiandra16 for its body as well.
        let (font, material) = if cutscene {
            (name.clone(), name_material.clone())
        } else {
            (body.clone(), body_material.clone())
        };
        commands.spawn((
            NativeBitmapText::new(font, "", TextStyle::dialogue()),
            Mesh2d(meshes.add(empty()?)),
            MeshMaterial2d(material),
            Transform::default(),
            Visibility::Hidden,
            binding,
        ));
    }
    commands.insert_resource(AdditionalDialogAssets {
        body: body.clone(),
        name: name.clone(),
        body_material: body_material.clone(),
        name_material,
        background_material,
    });
    let mut role = NativeBitmapText::new(body.clone(), "", TextStyle::default());
    let panel_font = body.clone();
    role.reveal_enabled = false;
    let mut notice = NativeBitmapText::new(body, "", TextStyle::default());
    notice.reveal_enabled = false;
    let mut panel = NativeBitmapText::new(panel_font, "", TextStyle::default());
    panel.reveal_enabled = false;
    commands.spawn((
        mod_panel::ModPanelOverlay,
        panel,
        Mesh2d(meshes.add(empty()?)),
        MeshMaterial2d(body_material.clone()),
        Transform::default(),
        Visibility::Hidden,
    ));
    commands.spawn((
        network::RoleOverlay,
        role,
        Mesh2d(meshes.add(empty()?)),
        MeshMaterial2d(body_material.clone()),
        Transform::default(),
        Visibility::Hidden,
    ));
    commands.spawn((
        network::ConnectionNotice,
        notice,
        Mesh2d(meshes.add(empty()?)),
        MeshMaterial2d(body_material),
        Transform::default(),
        Visibility::Hidden,
    ));
    let mut camera = commands.spawn((
        OverlayCamera,
        Camera2d,
        // BloomEffect and the final decode after the GUI (post.rs).
        crate::post::MagickaBloomPass,
        crate::post::MagickaPost::default(),
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
    ));
    if let Some(target) = target {
        camera.insert(RenderTarget::Image(ImageRenderTarget {
            handle: target,
            scale_factor: 1.0,
        }));
    }
    Ok(())
}

/// Additional native TextBoxes (including script Magick unlock notices) use
/// the same renderer and original assets as regular dialog slots. Grow the
/// presentation pool as needed; the renderer hides surplus slots after removal.
fn sync_additional_dialogs(
    mut commands: Commands,
    play: Res<Play>,
    assets: Option<Res<AdditionalDialogAssets>>,
    bindings: Query<&NativeDialogBox>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(assets) = assets else { return };
    let count = play
        .session
        .world
        .dialog_text
        .as_ref()
        .map_or(0, |text| text.additional.len());
    let allocated = bindings.iter().filter(|binding| binding.additional).count();
    for slot in allocated..count {
        // Separate material types require separate spawn expressions.
        let background_mesh = meshes.add(crate::text_box_visual::background_mesh(
            [0.; 2], [0.; 2], 0., [1.; 2],
        ));
        let background = commands
            .spawn((
                Mesh2d(background_mesh.clone()),
                MeshMaterial2d(assets.background_material.clone()),
                Transform::default(),
                Visibility::Hidden,
            ))
            .id();
        let mut part = |material| {
            let mesh = meshes.add(crate::text_box_visual::background_mesh(
                [0.; 2], [0.; 2], 0., [1.; 2],
            ));
            let entity = commands
                .spawn((
                    Mesh2d(mesh.clone()),
                    MeshMaterial2d(material),
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id();
            (entity, mesh)
        };
        let (owner, owner_mesh) = part(assets.name_material.clone());
        let mut binding = NativeDialogBox::new(
            slot,
            background,
            background_mesh,
            owner,
            owner_mesh,
            assets.name.clone(),
        );
        binding.additional = true;
        let mut text = NativeBitmapText::new(assets.body.clone(), "", TextStyle::dialogue());
        text.suspended = true;
        commands.spawn((
            text,
            binding,
            Mesh2d(meshes.add(crate::text_box_visual::background_mesh(
                [0.; 2], [0.; 2], 0., [1.; 2],
            ))),
            MeshMaterial2d(assets.body_material.clone()),
            Transform::default(),
            Visibility::Hidden,
        ));
    }
}

fn build_graph(
    mut avatar: ResMut<AvatarAssets>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    if avatar.graph.is_some() {
        return;
    }
    let Some(gltf) = gltfs.get(&avatar.gltf) else {
        return;
    };
    // Every exported clip by name; cues name clips directly.
    let (names, handles): (Vec<String>, Vec<_>) = gltf
        .named_animations
        .iter()
        .map(|(name, handle)| (name.to_string(), handle.clone()))
        .unzip();
    let (graph, nodes) = AnimationGraph::from_clips(handles);
    let map = names.into_iter().zip(nodes).collect();
    avatar.graph = Some((graphs.add(graph), map));
}

/// Tag the AnimationPlayer spawned under the avatar root and give it the graph.
fn attach_animator(
    mut commands: Commands,
    avatar: Res<AvatarAssets>,
    players: Query<Entity, (With<AnimationPlayer>, Without<AvatarAnimator>)>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<AvatarRoot>>,
) {
    let Some((graph, _)) = &avatar.graph else {
        return;
    };
    for entity in &players {
        if parents.iter_ancestors(entity).any(|a| roots.contains(a)) {
            commands.entity(entity).insert((
                AvatarAnimator,
                AnimationGraphHandle(graph.clone()),
                AnimationTransitions::new(),
            ));
        }
    }
}

/// `wheel`: this frame's vertical scroll. KeyboardMouseController binds
/// PrevMagick to ScrollUp and NextMagick to ScrollDown, one step per frame.
fn gather_input(
    window: Option<&Window>,
    mouse: &ButtonInput<MouseButton>,
    keys: &ButtonInput<KeyCode>,
    wheel: f32,
    bindings: &crate::bindings::KeyboardBindings,
) -> (PlayerInput, f32) {
    use crate::bindings as b;
    let held = |f| bindings.pressed(f, keys, mouse, wheel);
    let Some(window) = window else {
        return (PlayerInput::default(), 16.0 / 9.0);
    };
    let size = window.size();
    let aspect = size.x / size.y.max(1.0);
    let cursor_ndc = window
        .cursor_position()
        .map(|p| [-1.0 + 2.0 * p.x / size.x, 1.0 - 2.0 * p.y / size.y]);
    (
        PlayerInput {
            cursor_ndc,
            pad_movement: None,
            pad_attack: false,
            pad_block: false,
            // KeyboardMouseController.IsDown over the user's bindings
            // (defaults: Move left, Cast right, CastSelf middle button).
            move_down: held(b::MOVE),
            cast_down: held(b::CAST),
            cast_self_down: held(b::CAST_SELF),
            shift_down: held(b::SHIFT),
            boost_down: held(b::BOOST),
            elements: std::array::from_fn(held),
            magick_prev: held(b::PREV_MAGICK),
            magick_next: held(b::NEXT_MAGICK),
            inventory: bindings.just_pressed(b::INVENTORY, keys, mouse, wheel),
        },
        aspect,
    )
}

fn script_input(script: &[Step], frame: u32) -> Option<(PlayerInput, Option<&'static str>)> {
    let mut start = 0;
    for &(frames, ndc, act, name) in script {
        if frame < start + frames {
            let capture = (frame + 1 == start + frames).then_some(name);
            let mut elements = [false; 8];
            elements[7] = act == Act::Fire;
            elements[1] = act == Act::Life;
            elements[5] = act == Act::Arcane;
            elements[6] = act == Act::Earth;
            elements[4] = act == Act::Lightning;
            elements[2] = act == Act::Shield;
            elements[0] = act == Act::Water;
            elements[3] = act == Act::Cold;
            return Some((
                PlayerInput {
                    cursor_ndc: Some(ndc),
                    pad_movement: None,
                    pad_attack: false,
                    pad_block: false,
                    move_down: matches!(
                        act,
                        Act::Move
                            | Act::WeaponSwing
                            | Act::IceBladeVisible
                            | Act::IceBladeHidden
                            | Act::CapsuleLightVisible
                            | Act::CapsuleLightHidden
                            | Act::PickupBook
                            | Act::Pickup
                    ),
                    cast_down: matches!(
                        act,
                        Act::Cast
                            | Act::SpellLightsVisible
                            | Act::SpellLightsHidden
                            | Act::AreaCast
                            | Act::BeamTarget(_, 1..=4)
                            | Act::BeamTarget(0, 5)
                    ),
                    shift_down: matches!(
                        act,
                        Act::AreaCast
                            | Act::WeaponSwing
                            | Act::ShiftHold
                            | Act::IceBladeVisible
                            | Act::IceBladeHidden
                            | Act::CapsuleLightVisible
                            | Act::CapsuleLightHidden
                    ),
                    cast_self_down: act == Act::SelfCast,
                    boost_down: act == Act::Boost,
                    elements,
                    magick_prev: act == Act::MagickPrev,
                    inventory: act == Act::Inventory,
                    magick_next: act == Act::MagickNext,
                },
                capture,
            ));
        }
        start += frames;
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn step_session(
    mut play: ResMut<Play>,
    (mut network, mut mods): (
        Option<NonSendMut<network::NetworkPlay>>,
        Option<NonSendMut<magicka_runtime::mod_runtime::ModRuntime>>,
    ),
    time: Res<Time>,
    mut inspection: ResMut<InspectionCamera>,
    windows: Query<&Window, With<PrimaryWindow>>,
    (mouse, scroll): (
        Option<Res<ButtonInput<MouseButton>>>,
        Option<Res<bevy::input::mouse::AccumulatedMouseScroll>>,
    ),
    (keys, controller): (
        Option<Res<ButtonInput<KeyCode>>>,
        Res<crate::controller::ControllerInput>,
    ),
    autopilot: Option<ResMut<Autopilot>>,
    assets: Res<AssetServer>,
    avatar: Res<AvatarAssets>,
    animators: Query<(), With<AvatarAnimator>>,
    mut menu: ResMut<menu::PauseMenu>,
    mut particles: ResMut<crate::particles::ParticleFrame>,
    (mut projectile_audit, mut boss_audit, mut entrance_audit, mut house_audit, mut beam_audit): (
        ResMut<npc_projectile_qa::Audit>,
        ResMut<boss_qa::Audit>,
        ResMut<entrance_qa::Audit>,
        ResMut<house_qa::Audit>,
        ResMut<beam_target_qa::Audit>,
    ),
    (audio_stats, pads, mut ticks, mut coop): (
        Option<Res<audio::AudioStats>>,
        Query<&Gamepad>,
        ResMut<tick_clock::TickClock>,
        ResMut<local_coop::LocalCoop>,
    ),
    #[cfg(feature = "debug-ui")] (mut preview, tool_mode): (
        ResMut<debug_menu::AnimationPreview>,
        Res<bevy_toolmode::ToolMode>,
    ),
    #[cfg(all(feature = "editor", not(feature = "debug-ui")))] tool_mode: Res<
        bevy_toolmode::ToolMode,
    >,
) {
    if (menu.visible || menu.consume_frame) && network.is_none() && !menu.restart {
        play.dt = 0.0;
        ticks.paused();
        return;
    }
    let dm_click = mouse
        .as_ref()
        .is_some_and(|m| m.just_pressed(MouseButton::Left));
    if let Some(keys) = keys.as_ref().filter(|_| !menu.visible || menu.restart) {
        // KeyboardMouseController.OnKeyDown: qualifying keys skip cutscenes.
        if keys
            .get_just_pressed()
            .filter_map(|k| crate::bindings::xna_key(*k))
            .any(magicka_runtime::cutscene::key_skips_cutscene)
        {
            play.session.skip_cutscene();
        }
        if menu.skip_tutorial {
            menu.skip_tutorial = false;
            if let Err(e) = play.session.run_trigger("skip_tutorial", true) {
                error!("skip tutorial: {e}");
            }
        }
        // Players page: who can be removed, and a removal confirmed there.
        menu.kickable = match network.as_deref_mut() {
            Some(network::NetworkPlay::Host(host)) => Some(host.kickable()),
            Some(network::NetworkPlay::Client(view))
                if view.role() == magicka_network::lobby::Role::DungeonMaster =>
            {
                Some(view.kickable())
            }
            _ => None,
        };
        if let Some(seat) = menu.kick.take() {
            match network.as_deref_mut() {
                Some(network::NetworkPlay::Host(host)) => {
                    host.kick(seat);
                }
                Some(network::NetworkPlay::Client(view)) => {
                    if let Err(e) = view.dm(magicka_network::lobby::DmCommand::Kick { seat }) {
                        error!("kick: {e}");
                    }
                }
                None => {}
            }
        }
        // F5 restarts (R is the original Cold binding).
        if keys.just_pressed(KeyCode::F5) || menu.restart {
            menu.restart = false;
            match network.as_deref_mut() {
                Some(network::NetworkPlay::Host(host)) => {
                    if let Err(e) = host.restart(&mut play.session) {
                        error!("host restart: {e}");
                    }
                }
                Some(network::NetworkPlay::Client(view)) => {
                    if let Err(e) = view.respawn() {
                        error!("respawn request: {e}");
                    }
                }
                None => match play.session.restart_at_checkpoint() {
                    Ok(true) => play.reloads += 1,
                    Ok(false) => reload(&mut play),
                    Err(e) => error!("checkpoint restart: {e:?}"),
                },
            }
        }
    }
    // PlayState defeat: Restart(Checkpoint). Before the first checkpoint the
    // level's initial state is the checkpoint.
    if play.session.end_game.restart && network.is_none() {
        match play.session.restart_at_checkpoint() {
            Ok(true) => play.reloads += 1,
            Ok(false) => reload(&mut play),
            Err(e) => {
                error!("checkpoint restart failed: {e:?}");
                reload(&mut play);
            }
        }
    }
    // Campaign LevelComplete: SubMenuEndGame → the next CampaignNode. A
    // menu host presents those (`leave_completed_level`); standalone play
    // loads the next level in place.
    if let Some(next) = play
        .session
        .next_level()
        .filter(|_| network.is_none() && !play.menu_hosted)
    {
        info!("level complete: loading {next}");
        match play.session.change_level(next) {
            Ok(()) => {
                play.level = next.to_string();
                play.reloads += 1;
            }
            Err(e) => error!("campaign transition failed: {e:?}"),
        }
    }
    let mut pickup_fixture_check = None;
    let mut earth_trail_hidden = false;
    let mut boss_frozen = false;
    let scripted = autopilot.is_some();
    let (input, aspect, dt) = if let Some(mut pilot) = autopilot {
        let ready = assets.is_loaded_with_dependencies(&avatar.scene)
            && avatar
                .level
                .as_ref()
                .is_none_or(|l| assets.is_loaded_with_dependencies(l))
            && !animators.is_empty();
        pilot.frame += 1;
        if pilot.started.is_none() {
            if !ready {
                if pilot.frame > 3000 {
                    eprintln!("autopilot: assets never finished loading");
                    std::process::exit(2);
                }
                return;
            }
            pilot.started = Some(pilot.frame);
        }
        let local = pilot.frame - pilot.started.unwrap();
        if pilot.script == FAFNIR_ATTACKS_SCRIPT
            || pilot.script == FAFNIR_BREATH_SCRIPT
            || pilot.script == FAFNIR_WING_SCRIPT
        {
            if pilot.script == FAFNIR_WING_SCRIPT {
                // An immortal character ignores every impact, pushes included,
                // so the wing script heals the wizard and puts him back on
                // his spot between attacks instead.
                let avatar = play.session.world.players[0].as_mut().unwrap();
                if avatar.character.combat.health.hit_points < 500.0 {
                    avatar.character.combat.health.hit_points = 1000.0;
                }
                let body = avatar.body;
                let scripted: u32 = FAFNIR_WING_SCRIPT[..4].iter().map(|s| s.0).sum();
                if local == scripted {
                    pilot.anchor = Some(play.session.world.physics.bodies[body].position);
                }
                let wing = play.session.world.boss_fight.bosses.iter().any(|b| {
                    matches!(b, magicka_runtime::scene_world::boss_live::BossRef::Fafnir(f)
                        if f.fafnir.state == magicka_runtime::boss_fafnir::State::Wing)
                });
                if let (Some(anchor), false) = (pilot.anchor, wing) {
                    let b = &mut play.session.world.physics.bodies[body];
                    let d = [0, 2].map(|i| b.position[i] - anchor[i]);
                    if d[0] * d[0] + d[1] * d[1] > 1.0 {
                        b.position = anchor;
                        b.velocity = [0.0; 3];
                    }
                }
            } else {
                play.session.world.players[0]
                    .as_mut()
                    .unwrap()
                    .character
                    .immortal_time = f32::MAX;
            }
            let fafnir = play.session.world.boss_fight.bosses.iter().find_map(|b| {
                if let magicka_runtime::scene_world::boss_live::BossRef::Fafnir(f) = b {
                    Some(f.as_ref())
                } else {
                    None
                }
            });
            let mut body_zone = None;
            if let Some(host) = fafnir {
                let f = &host.fafnir;
                if local % 30 == 0 || f.state == magicka_runtime::boss_fafnir::State::Wing {
                    let b = play.session.body();
                    eprintln!(
                        "autopilot fafnir frame {local}: state {:?} hp {:?} quakes {} head {:?} spine {:?} lance {} {:?} breath {:?} wizard {:?} {:?}",
                        f.state,
                        f.health(),
                        f.quakes,
                        [f.zones[0][12], f.zones[0][13], f.zones[0][14]],
                        f.spine_base_position,
                        f.draw_lance,
                        f.lance,
                        f.breath,
                        b.position,
                        b.velocity
                    );
                }
                if pilot.script == FAFNIR_ATTACKS_SCRIPT && local == FAFNIR_ATTACKS_HIT_FRAME {
                    body_zone = Some(host.zones[1]);
                }
            }
            if let Some(zone) = body_zone {
                // Only the body zone takes damage; Arcane has multiplier 1.
                play.session
                    .world
                    .damage_physics(
                        zone,
                        magicka_sim::damage::Damage {
                            attack: magicka_sim::damage::attack::DAMAGE,
                            element: magicka_sim::spells::Elements::ARCANE.0,
                            amount: 23000.,
                            magnitude: 1.,
                        },
                        15,
                    )
                    .expect("Fafnir body hit");
            }
        }
        if pilot.script == DEATH_BOSS_SCRIPT {
            play.session.world.players[0]
                .as_mut()
                .unwrap()
                .character
                .immortal_time = f32::MAX;
            if local == 60 {
                play.session
                    .run_trigger("Move to the throne", true)
                    .expect("Death throne trigger");
            }
            if local > 120 && local % 30 == 0 {
                play.session.world.advance_dialog(0);
            }
            let boss = play.session.world.boss_fight.bosses.iter().find_map(|b| {
                if let magicka_runtime::scene_world::boss_live::BossRef::Death(d) = b {
                    Some(d)
                } else {
                    None
                }
            });
            let mut damage_zone = None;
            if let Some(host) = boss {
                let d = host.actor.lock().unwrap();
                if local > 720 && d.combat.hit_points() > 5000. && d.drawn && !d.ethereal {
                    damage_zone = Some(host.zone);
                }
                boss_frozen = d.state == magicka_runtime::boss_death::State::MultiDeath
                    && d.visible.iter().filter(|&&v| v).count() > 1;
            }
            if let Some(zone) = damage_zone {
                play.session
                    .world
                    .damage_physics(
                        zone,
                        magicka_sim::damage::Damage {
                            attack: magicka_sim::damage::attack::DAMAGE,
                            element: magicka_sim::spells::Elements::LIFE.0,
                            amount: 5000.,
                            magnitude: 1.,
                        },
                        15,
                    )
                    .expect("Death clone-phase hit");
            }
        }
        if script_act(pilot.script, local) == Some(Act::MachineBreak) {
            let zone = play
                .session
                .world
                .boss_fight
                .bosses
                .iter()
                .find_map(|b| {
                    if let magicka_runtime::scene_world::boss_live::BossRef::Machine(m) = b {
                        Some(m.zones[0])
                    } else {
                        None
                    }
                })
                .expect("Machine capture boss");
            play.session
                .world
                .damage_physics(
                    zone,
                    magicka_sim::damage::Damage {
                        attack: magicka_sim::damage::attack::DAMAGE,
                        element: magicka_sim::spells::Elements::LIFE.0,
                        amount: 1_000_000.,
                        magnitude: 1.,
                    },
                    15,
                )
                .expect("Machine break capture hit");
        }
        boss_frozen |= beam_audit.prepare(
            &mut play.session,
            script_act(pilot.script, local),
            &pilot.dir,
        );
        projectile_audit.prepare(&mut play.session, script_act(pilot.script, local));
        boss_frozen |= boss_audit.prepare(&mut play.session, script_act(pilot.script, local));
        boss_frozen |= entrance_audit.prepare(
            &mut play.session,
            script_act(pilot.script, local),
            &pilot.dir,
        );
        boss_frozen |= house_audit.prepare(&mut play, script_act(pilot.script, local), &pilot.dir);
        boss_frozen |= matches!(
            script_act(pilot.script, local),
            Some(Act::LightingBright | Act::LightingDark | Act::EtherealNpcAlpha(_))
        );
        if script_act(pilot.script, local) == Some(Act::LightingSpawn) {
            play.session
                .spawn_npc("Goblin_pirate", "idle", 3.)
                .expect("lighting fixture NPC");
        }
        if matches!(
            script_act(pilot.script, local),
            Some(Act::EtherealNpcSpawn | Act::EtherealNpcAttackSpawn(_))
        ) {
            play.session.world.players[0]
                .as_mut()
                .unwrap()
                .character
                .immortal_time = f32::MAX;
            let player_position = play.session.body().position;
            let selection = match script_act(pilot.script, local) {
                Some(Act::EtherealNpcAttackSpawn(selection)) => selection,
                _ => 3,
            };
            let mut actors = Vec::new();
            for (bit, template, x) in [(1, "Ghost_Acolyte", -1.), (2, "Boss_Vlad_Spirit", 1.)] {
                if selection & bit != 0 {
                    let handle = play
                        .session
                        .spawn_npc(template, "idle", 3.)
                        .expect("ethereal fixture actor");
                    actors.push((handle, x));
                }
            }
            // Put selected actors on the open path while keeping native templates.
            for (handle, x) in actors {
                let npc = play.session.world.npcs.get_mut(&handle).unwrap();
                if script_act(pilot.script, local) == Some(Act::EtherealNpcAttackSpawn(1)) {
                    // A single ghost does not choose an attack here. Drive a
                    // native MoveEvent to isolate its walking render path.
                    npc.ai.set_order(
                        0,
                        Some(vec![magicka_runtime::ai_event::AiEvent::Move(
                            magicka_runtime::ai_event::MoveEvent {
                                waypoint: [
                                    player_position[0] + 3.,
                                    player_position[1],
                                    player_position[2] - 10.,
                                ],
                                speed: 1.,
                                ..Default::default()
                            },
                        )]),
                    );
                    npc.react_to = 0;
                    npc.reaction_order = 0;
                }
                if script_act(pilot.script, local) == Some(Act::EtherealNpcSpawn) {
                    npc.ai.set_order(0, None);
                    npc.react_to = 0;
                    npc.reaction_order = 0;
                }
                let body = play.session.world.npcs[&handle].body;
                play.session.world.physics.bodies[body].position = [
                    player_position[0] + x,
                    player_position[1],
                    player_position[2] - 4.,
                ];
            }
        }
        if let Some(Act::EtherealNpcAlpha(phase)) = script_act(pilot.script, local) {
            for npc in play.session.world.npcs.values_mut() {
                npc.character
                    .execute_ethereal_action(true, if phase == 1 { 0. } else { 1. }, 0.);
            }
        }
        earth_trail_hidden = script_act(pilot.script, local) == Some(Act::EarthTrailHidden);
        if script_act(pilot.script, local) == Some(Act::LabPortal) {
            let i = play.session.avatar().body;
            play.session.world.physics.bodies[i].position = [-7., 1.5, 12.];
            eprintln!("feature lab fixture: entered authored portal");
        }
        if script_act(pilot.script, local) == Some(Act::SpawnPhysicsProp) {
            let body = play.session.body();
            let direction = body.direction();
            let mut transform = magicka_content::skinned::IDENTITY;
            for i in 0..3 {
                transform[12 + i] = body.position[i] + direction[i] * 2.;
            }
            transform[13] += 5.;
            let handle = play
                .session
                .spawn_physics_prop("bathball", transform)
                .expect("physics prop fixture");
            eprintln!(
                "physics prop fixture: spawned bathball {handle} at {:?}",
                &transform[12..15]
            );
        }
        if script_act(pilot.script, local) == Some(Act::EtherealBody) {
            let handle = play.session.avatar().handle;
            assert!(play.session.world.etherealize(handle));
        }
        if script_act(pilot.script, local) == Some(Act::CorporealBody) {
            let at = play.session.body().position;
            play.session.world.corporealize(at);
        }
        if script_act(pilot.script, local) == Some(Act::TutorialHighlights) {
            use magicka_runtime::scene_world::campaign_live::Highlight;
            let body = play.session.body();
            let p = body.position;
            let direction = body.direction();
            let mut pose = magicka_content::skinned::IDENTITY;
            for i in 0..3 {
                pose[12 + i] = p[i] + direction[i] * 2.5;
            }
            let handle = play
                .session
                .spawn_physics_prop("generator", pose)
                .expect("highlight fixture prop");
            let id = magicka_runtime::scene_loading::custom_hash("qa highlight generator");
            play.session
                .world
                .entities
                .iter_mut()
                .find(|e| e.handle == handle)
                .unwrap()
                .unique_id = id;
            play.session
                .world
                .highlights
                .push(Highlight::Phys { id, time: 2.0 });
            if let Some(part) = play.session.world.collision.parts().first() {
                let path = vec![part.id];
                play.session
                    .world
                    .highlights
                    .push(Highlight::Anim { path, time: 2.0 });
            }
        }
        if script_act(pilot.script, local) == Some(Act::UnlockMagicks) {
            play.session.unlocked_magicks[0] = ((1u64 << 35) - 1) & !1;
        }
        if matches!(
            script_act(pilot.script, local),
            Some(Act::SpawnBook | Act::SpawnTimedBook | Act::SpawnFallingBook)
        ) {
            let body = play.session.body();
            let direction = body.direction();
            let mut transform = magicka_content::skinned::IDENTITY;
            for i in 0..3 {
                transform[12 + i] = body.position[i] + direction[i] * 1.5;
            }
            if script_act(pilot.script, local) == Some(Act::SpawnFallingBook) {
                transform[13] += 5.0;
            }
            let handle = play
                .session
                .spawn_magick_book(3, transform)
                .expect("book fixture spawn");
            if script_act(pilot.script, local) == Some(Act::SpawnTimedBook) {
                play.session
                    .world
                    .book_pickups
                    .get_mut(&handle)
                    .unwrap()
                    .timeout = 1.0;
            }
            if script_act(pilot.script, local) == Some(Act::SpawnFallingBook) {
                let book = play.session.world.book_pickups.get_mut(&handle).unwrap();
                book.immovable = false;
                book.body_enabled = true;
            }
            eprintln!("book fixture: spawned {handle}");
        }
        if script_act(pilot.script, local) == Some(Act::SpawnPickup) {
            let body = play.session.body();
            let direction = body.direction();
            let mut transform = magicka_content::skinned::IDENTITY;
            for i in 0..3 {
                transform[12 + i] = body.position[i] + direction[i] * 1.5;
            }
            let handle = play
                .session
                .spawn_item_pickup("staff_heirloom_grandfather_clock", transform)
                .expect("pickup fixture spawn");
            eprintln!("pickup fixture: spawned {handle}");
        }
        if script_act(pilot.script, local) == Some(Act::Die) {
            if let Some(p) = play.session.world.players[0].as_mut() {
                p.terminate();
            }
        }
        if script_act(pilot.script, local) == Some(Act::Complete) {
            // EndGame.Execute → PlayState.Endgame(LevelComplete, freeze, false, 0).
            play.session.world.pending_end_game = Some((2, true, false));
        }
        if script_act(pilot.script, local) == Some(Act::Restart) {
            match play.session.restart_at_checkpoint() {
                Ok(true) => play.reloads += 1,
                Ok(false) => reload(&mut play),
                Err(e) => error!("checkpoint restart: {e:?}"),
            }
        }
        if script_act(pilot.script, local) == Some(Act::Skip) {
            debug_assert!(magicka_runtime::cutscene::key_skips_cutscene(
                crate::bindings::xna_key(KeyCode::Space).unwrap()
            ));
            play.session.skip_cutscene();
        }
        if script_act(pilot.script, local) == Some(Act::FreezeWater) {
            for water in &mut play.session.world.collision.collision_mut().water {
                water.freeze.vertices.fill(2.0);
            }
        }
        if matches!(
            script_act(pilot.script, local),
            Some(Act::ZapFlashVisible | Act::ZapFlashHidden)
        ) {
            for npc in play.session.world.npcs.values_mut() {
                let combat = &mut npc.character.combat;
                combat.zap_timer = combat.zap_timer.max(0.1);
            }
        }
        if script_act(pilot.script, local) == Some(Act::LayeringFixture) {
            let center = play.session.body().feet();
            play.session
                .world
                .decals
                .add(magicka_runtime::decals::Request {
                    cell: 22,
                    scale: [12.0; 2],
                    position: center,
                    direction: Some([1.0, 0.0, 0.0]),
                    normal: [0.0, 1.0, 0.0],
                    ttl: 60.0,
                    color: [1.0; 4],
                    animated_owner: None,
                });
        }
        if script_act(pilot.script, local) == Some(Act::DecalFixture) {
            let center = play.session.body().position;
            let world = &mut play.session.world;
            for (i, cell) in [20, 22, 23, 0, 4, 8, 12, 16].into_iter().enumerate() {
                let origin = [
                    center[0] + (i % 2) as f32 * 2.5 - 1.25,
                    center[1] + 2.0,
                    center[2] - 12.0 + (i / 2) as f32 * 2.5,
                ];
                if let Some(hit) = world.collision.collision().segment_intersect(
                    magicka_sim::spray_pulse::Segment {
                        origin,
                        delta: [0.0, -8.0, 0.0],
                    },
                    false,
                ) {
                    world.decals.add(magicka_runtime::decals::Request {
                        cell,
                        scale: [2.0; 2],
                        position: hit.hit.position,
                        direction: Some([1.0, 0.0, 0.0]),
                        normal: hit.hit.normal,
                        ttl: 60.0,
                        color: [1.0; 4],
                        animated_owner: hit.animated_owner,
                    });
                    eprintln!(
                        "decal fixture: cell={cell} position={:?} normal={:?}",
                        hit.hit.position, hit.hit.normal
                    );
                }
            }
        }
        if script_act(pilot.script, local) == Some(Act::NpcWarrior) {
            play.session
                .spawn_npc("Goblin_warrior", "attack", 7.0)
                .expect("warrior fixture");
        }
        if script_act(pilot.script, local) == Some(Act::BreakWarriorShield) {
            let points: Vec<_> = play
                .session
                .world
                .npcs
                .values()
                .filter(|n| n.definition.id == "goblin_warrior")
                .map(|n| play.session.world.physics.bodies[n.body].position)
                .collect();
            for point in points {
                let timestamp = play.session.world.play_time;
                play.session.world.arc_damage(
                    None,
                    None,
                    point,
                    [0., 0., -1.],
                    0.1,
                    std::f32::consts::PI,
                    &[magicka_sim::damage::Damage {
                        attack: magicka_sim::damage::attack::DAMAGE,
                        element: magicka_sim::spells::Elements::ARCANE.0,
                        amount: 1000.0,
                        magnitude: 1.0,
                    }],
                    timestamp,
                );
            }
        }
        if script_act(pilot.script, local) == Some(Act::Gib) {
            let npcs: Vec<u16> = play.session.world.npcs.keys().copied().collect();
            for handle in npcs {
                play.session.world.overkill(handle);
            }
        }
        if script_act(pilot.script, local) == Some(Act::Gate) {
            teleport_to_area(&mut play.session, "trigger_gate");
        }
        if script_act(pilot.script, local) == Some(Act::Exit) {
            teleport_to_area(&mut play.session, "trigger_exit");
        }
        let Some((mut input, capture)) = script_input(pilot.script, local) else {
            return;
        };
        // Combat motion no longer leaves the demo goblin at a fixed screen
        // coordinate. Aim the status fixtures at its current body through
        // the ordinary mouse ray, so captures actually exercise a hit.
        if [FREEZE_SCRIPT, ZAP_SCRIPT].contains(&pilot.script) {
            if let Some(npc) = play.session.world.npcs.values().find(|n| !n.dead()) {
                let point = play.session.world.physics.bodies[npc.body].position;
                let camera = &play.session.world.camera;
                let vp = magicka_runtime::camera::multiply(
                    &camera.view(),
                    &camera.projection(AUTOPILOT_SIZE.x as f32 / AUTOPILOT_SIZE.y as f32),
                );
                let v = [point[0], point[1], point[2], 1.];
                let clip: [f32; 4] =
                    std::array::from_fn(|i| (0..4).map(|j| v[j] * vp[j * 4 + i]).sum());
                input.cursor_ndc = Some([clip[0] / clip[3], clip[1] / clip[3]]);
            }
            if let Some(name) = capture {
                use magicka_sim::status::Status;
                let statuses: Vec<_> = play
                    .session
                    .world
                    .npcs
                    .iter()
                    .map(|(h, n)| {
                        (
                            *h,
                            n.character.combat.statuses.current_mask,
                            n.character.combat.zap_timer,
                        )
                    })
                    .collect();
                eprintln!("status fixture {name}: {statuses:?}");
                if pilot.script == FREEZE_SCRIPT && name == "frozen" {
                    assert!(
                        statuses.iter().any(|s| s.1 & Status::Frozen as u16 != 0),
                        "freeze fixture missed its target"
                    );
                }
            }
        }
        // Exercise the same ray + left-button input as interactive play.
        if matches!(
            script_act(pilot.script, local),
            Some(Act::PickupBook | Act::Pickup)
        ) {
            pickup_fixture_check = Some(script_act(pilot.script, local) == Some(Act::PickupBook));
            let (path, transform, book) =
                if script_act(pilot.script, local) == Some(Act::PickupBook) {
                    let handle = play.session.nearby_book().expect("book fixture in range");
                    (
                        PathBuf::from(magicka_runtime::magick_books::BOOK_MODEL),
                        play.session.world.book_pickups[&handle].transform,
                        true,
                    )
                } else {
                    let handle = play.session.nearby_item().expect("pickup fixture in range");
                    let p = &play.session.world.item_pickups[&handle];
                    let model = p.item.as_ref().unwrap().definition.model.as_ref().unwrap();
                    (
                        std::path::Path::new("Data/Items/Wizard").join(model.replace('\\', "/")),
                        p.transform,
                        false,
                    )
                };
            let point = magicka_runtime::play_content::pickup_model_bounds(&play.content, &path)
                .expect("fixture collision bounds")
                .world_box(&transform, book)
                .center;
            let camera = &play.session.world.camera;
            let vp = magicka_runtime::camera::multiply(
                &camera.view(),
                &camera.projection(AUTOPILOT_SIZE.x as f32 / AUTOPILOT_SIZE.y as f32),
            );
            let v = [point[0], point[1], point[2], 1.0];
            let clip: [f32; 4] =
                std::array::from_fn(|i| (0..4).map(|j| v[j] * vp[j * 4 + i]).sum());
            input.cursor_ndc = Some([clip[0] / clip[3], clip[1] / clip[3]]);
        }
        if let Some(name) = capture {
            pilot.pending_shot = true;
            if pilot.script == TOWER_DEBRIS_SCRIPT {
                let bodies: Vec<_> = play
                    .session
                    .world
                    .gibs
                    .live
                    .iter()
                    .map(|g| (g.id, g.position, g.velocity))
                    .collect();
                eprintln!("tower debris {name}: {bodies:?}");
            }
            if pilot.script == WARRIOR_SHIELD_SCRIPT {
                let gibs: Vec<_> = play
                    .session
                    .world
                    .gibs
                    .live
                    .iter()
                    .map(|g| &g.model)
                    .collect();
                let living: Vec<_> = play
                    .session
                    .world
                    .npcs
                    .values()
                    .filter(|n| !n.dead())
                    .map(|n| &n.definition.id)
                    .collect();
                eprintln!("warrior shield {name}: gibs={gibs:?} living={living:?}");
            }
            if pilot.script == HASTE_INVISIBILITY_SCRIPT {
                let avatar = play.session.avatar();
                eprintln!(
                    "autopilot {name}: hasted={} speed={:.3} invisible={:.2} queue={}",
                    play.session.world.ability_effects.hasted(avatar.handle),
                    play.session.body().speed_multiplier(),
                    avatar.character.combat.health.invisibility_timer,
                    avatar.conjure.queue.len(),
                );
            }
            if pilot.script == TORNADO_SCRIPT
                || pilot.script == PHOENIX_SCRIPT
                || pilot.script == VORTEX_SCRIPT
                || pilot.script == WAVE_SCRIPT
                || pilot.script == PORTAL_SCRIPT
            {
                log_magick_models(name, &play.session);
            }
            if pilot.script == RAIN_SCRIPT {
                eprintln!(
                    "autopilot {name}: rain active={} effect={} light={:.3}/{:.3}",
                    play.session.world.ability_effects.rain_state().is_some(),
                    play.session.rain_effect_active(),
                    play.session.world.light_intensity,
                    play.session.world.light_target_intensity,
                );
            }
            if pilot.script == BLIZZARD_SCRIPT {
                eprintln!(
                    "autopilot {name}: blizzard={:?}",
                    play.session.world.ability_effects.blizzard_state()
                );
            }
            if pilot.script == MAGICK_HINT_SCRIPT {
                let tome = &play.session.tome[0];
                eprintln!(
                    "autopilot {name}: magick hint displayed={:?} shown={:?} alpha={:.3}",
                    tome.displayed, tome.shown, tome.alpha
                );
            }
            let shield = &play.session.avatar().character.combat.health.shield;
            eprintln!(
                "autopilot {name}: self-shield={:?} hp={}",
                shield.kind, shield.health
            );
            eprintln!(
                "autopilot {name}: ground-earth-effects={} lights={:?}",
                play.session.effects.active_effects_by_hash(
                    magicka_runtime::scene_loading::custom_hash("area_ground_earth")
                ),
                play.session
                    .dynamic_lights
                    .iter()
                    .map(|l| (l.radius, l.draw_intensity(), l.diffuse))
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "autopilot {name}: decal cells={:?}",
                play.session
                    .world
                    .decals
                    .frame
                    .iter()
                    .filter(|p| p.args[2] > 0.0)
                    .map(|p| p.args[0] as u8 + 8 * p.args[1] as u8)
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "autopilot {name}: {} live gibs",
                play.session.world.gibs.live.len()
            );
            eprintln!(
                "autopilot {name}: {} zapped",
                play.session.zapped_characters().len()
            );
            log_spell_extras(name, &play.session);
            let b = play.session.body();
            let avatar = play.session.avatar();
            let camera = play.session.camera();
            let g = camera.ground_position();
            pilot.log.push(format!(
                "{{\"capture\":\"{name}\",\"frame\":{local},\"scene\":\"{}\",\"fade\":{},\"cutscene\":{},\"bars\":{},\"skip_tip\":{},\"input_limited\":{},\"gate_time\":{},\"position\":{:?},\"feet\":{:?},\"forward\":{:?},\"velocity\":{:?},\"state\":\"{:?}\",\"animation\":\"{}\",\"animation_speed\":{},\"camera_ground\":{:?},\"camera_position\":{:?},\"fov\":{},\"dead\":{},\"health\":{},\"spell_power\":{},\"normalized_hp\":{},\"radius\":{},\"queue\":{:?},\"bolts\":{},\"beams\":{},\"npc_hp\":{:?},\"audio\":{:?}}}",
                play.session.scene_name(),
                play.session.transition_fade(),
                play.session.world.cutscene.active,
                play.session.world.cutscene.timer,
                play.session.world.cutscene.skip_tip_alpha,
                play.session.world.flags.input_limited,
                level_part_time(&play.session, "gate"),
                b.position,
                b.feet(),
                b.direction(),
                b.velocity,
                avatar.motion.state,
                animation_name(avatar.motion.controller.current_animation),
                avatar.motion.controller.speed,
                g,
                camera.position(),
                camera.fov(),
                play.session.dead(),
                avatar.character.combat.health.hit_points,
                avatar.character.spell_power,
                avatar.character.normalized_hit_points,
                avatar.character.definition.radius,
                avatar.conjure.queue.iter().map(|s| s.element.0).collect::<Vec<_>>(),
                play.session.bolts.len(),
                play.session.beams.len(),
                play.session
                    .world
                    .npcs
                    .values()
                    .map(|n| n.character.combat.health.hit_points)
                    .collect::<Vec<_>>(),
                audio_stats.as_deref().map(|s| s.0).unwrap_or_default()
            ));
        }
        (
            input,
            AUTOPILOT_SIZE.x as f32 / AUTOPILOT_SIZE.y as f32,
            if projectile_audit.missile.is_some()
                || matches!(
                    script_act(pilot.script, local),
                    Some(
                        Act::LayeringVisible
                            | Act::LayeringHidden
                            | Act::LayeringDarkVisible
                            | Act::LayeringDarkHidden
                            | Act::ZapFlashVisible
                            | Act::ZapFlashHidden
                            | Act::IceBladeVisible
                            | Act::IceBladeHidden
                            | Act::EarthTrailVisible
                            | Act::EarthTrailHidden
                            | Act::CapsuleLightVisible
                            | Act::CapsuleLightHidden
                            | Act::SpellLightsVisible
                            | Act::SpellLightsHidden
                            | Act::StatusLightsVisible
                            | Act::StatusLightsHidden
                            | Act::ShieldWallVisible
                            | Act::ShieldWallHidden
                            | Act::BarrierStrong
                            | Act::BarrierWeak
                            | Act::SpikesVisible
                            | Act::SpikesHidden
                            | Act::NumbersVisible
                            | Act::NumbersHidden
                            | Act::ParticleDepthBaseline
                            | Act::ParticleDepthBehind
                            | Act::ParticleDepthOldSource
                            | Act::ParticleDepthFront
                            | Act::ParticleDepthNoItems
                    )
                )
            {
                0.0
            } else {
                FIXED_DT
            },
        )
    } else {
        let mouse = mouse.map(|m| m.into_inner().clone()).unwrap_or_default();
        let window = windows.single().ok();
        if let Some(w) = window {
            play.screen = [w.width(), w.height()];
            play.session.screen = play.screen;
        }
        let keys = keys.as_ref().map(|k| (**k).clone()).unwrap_or_default();
        let wheel = scroll.map_or(0.0, |s| s.delta.y);
        let (mut input, aspect) = gather_input(window, &mouse, &keys, wheel, &play.bindings);
        if controller.active {
            input = controller.player_input(&play.session, aspect);
        } else if controller.exclusive {
            // Player one plays on a pad: keyboard and mouse never take over.
            input = PlayerInput {
                pad_movement: controller.release_movement.then_some([0.0; 2]),
                ..default()
            };
        } else if controller.release_movement {
            input.pad_movement = Some([0.0; 2]);
        }
        (input, aspect, time.delta_secs())
    };
    play.dt = dt.min(magicka_runtime::play_session::MAX_DELTA);
    let play = &mut *play;
    #[cfg(feature = "debug-ui")]
    let previewing = debug_menu::animations::step(&mut preview, play, dt);
    #[cfg(not(feature = "debug-ui"))]
    let previewing = false;
    // Numpad inspection camera: player-local and on the display's clock.
    if let Some(keys) = &keys {
        let target = Vec3::from(play.session.world.camera.ground_position());
        if keys.just_pressed(KeyCode::NumpadDecimal) {
            *inspection = InspectionCamera::default();
        }
        if keys.just_pressed(KeyCode::Numpad0) {
            inspection.unlocked_target = if inspection.unlocked_target.is_some() {
                None
            } else {
                Some(target)
            };
        }
        let axis = |positive, negative| {
            f32::from(keys.pressed(positive)) - f32::from(keys.pressed(negative))
        };
        let dt = time.delta_secs().min(0.1);
        inspection.zoom = (inspection.zoom
            + axis(KeyCode::NumpadSubtract, KeyCode::NumpadAdd) * dt)
            .clamp(-1.4, 2.4);
        inspection.yaw += axis(KeyCode::Numpad6, KeyCode::Numpad4) * dt;
        inspection.tilt =
            (inspection.tilt + axis(KeyCode::Numpad8, KeyCode::Numpad2) * dt).clamp(-0.5, 0.7);
        let pan = Vec3::new(
            axis(KeyCode::Numpad3, KeyCode::Numpad1),
            0.0,
            axis(KeyCode::Numpad9, KeyCode::Numpad7),
        );
        let pan_rotation = Quat::from_rotation_y(inspection.yaw);
        inspection.pan += pan_rotation * pan * (10.0 * dt);
    }
    let input = if menu.visible || menu.consume_frame {
        PlayerInput::default()
    } else {
        input
    };
    // Tool mode and the freecam own the mouse and the element keys.
    #[cfg(feature = "editor")]
    let input = if tool_mode.captures_input() {
        PlayerInput::default()
    } else {
        input
    };
    if !menu.visible && !menu.consume_frame && controller.active && controller.dismiss_hint {
        play.session.skip_cutscene();
    }
    let select_pressed = !menu.visible
        && !menu.consume_frame
        && keys.as_ref().is_some_and(|keys| {
            keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space)
        })
        || (!menu.visible
            && !menu.consume_frame
            && (controller.interact_pressed
                || (!play.session.world.tutorial.dialog_done() && controller.dismiss_hint)));
    if let Some(network::NetworkPlay::Client(view)) = network.as_deref_mut() {
        // A client presents the host's world: nothing here runs the simulation,
        // and its frames are already interpolated.
        ticks.unclocked();
        if let Err(e) = view.poll(play, dt) {
            error!("client: {e}");
        }
        apply_inspection_view(&mut play.session, &inspection);
        if let Some(keys) = keys.as_ref().filter(|_| !menu.visible) {
            let cursor = windows.single().ok().and_then(|w| {
                let size = w.size();
                w.cursor_position()
                    .map(|p| [-1.0 + 2.0 * p.x / size.x, 1.0 - 2.0 * p.y / size.y])
            });
            if let Err(e) =
                network::role_keys(view, play, keys, dm_click, &pads, cursor, aspect, dt)
            {
                error!("role controls: {e}");
            }
        }
        if let Err(e) = view.input(play, input, aspect, select_pressed) {
            error!("client input: {e}");
        }
    } else {
        // QA scripts step once per frame on their own delta (a zero delta holds
        // the world for a capture). Everything else runs on the fixed clock.
        let (tick_count, tick_dt) = if scripted {
            ticks.unclocked();
            (1, dt)
        } else {
            (ticks.advance(dt), magicka_runtime::fixed_tick::TICK_DT)
        };
        ticks.sample(input, select_pressed);
        for _ in 0..tick_count {
            coop.dispatch_seats(&mut play.session);
            match network.as_deref_mut() {
                Some(network::NetworkPlay::Host(host)) => {
                    if let Err(e) = host.poll(&mut play.session) {
                        error!("host: {e}");
                    }
                    if play.level != play.session.level_file() {
                        play.level = play.session.level_file().to_string();
                        play.reloads += 1;
                    }
                    if play.session.end_game.restart {
                        if let Err(e) = host.restart(&mut play.session) {
                            error!("host respawn: {e}");
                        }
                    }
                    // A host without a wizard runs the world only while someone
                    // plays (as magicka-server does).
                    let idle = host.is_dedicated() && !host.has_players();
                    // Server-side mod logic runs on the host only.
                    if let Some(mods) = mods.as_deref_mut().filter(|_| !idle) {
                        host.serve_mod_requests(mods, &mut play.session);
                        mods.step(&mut play.session, tick_dt);
                        let local = mods.local_presentation();
                        magicka_runtime::mod_presentation::play(
                            &mut play.session,
                            Some(&mods.definitions),
                            &local,
                        );
                        host.publish_mods(mods);
                    }
                    if !idle {
                        play.session.step(tick_dt);
                    }
                }
                Some(network::NetworkPlay::Client(_)) => unreachable!("handled above"),
                None => {
                    if !previewing && projectile_audit.missile.is_none() && !boss_frozen {
                        if let Some(mods) = mods.as_deref_mut() {
                            mods.step(&mut play.session, tick_dt);
                            let local = mods.local_presentation();
                            mods.presentation.clear();
                            magicka_runtime::mod_presentation::play(
                                &mut play.session,
                                Some(&mods.definitions),
                                &local,
                            );
                        }
                        play.session.step(tick_dt);
                    }
                }
            }
            apply_inspection_view(&mut play.session, &inspection);
            // A host without a wizard: slot 0 belongs to the first joiner.
            if !previewing
                && !matches!(
                    network.as_deref(),
                    Some(network::NetworkPlay::Host(host)) if host.is_dedicated()
                )
            {
                let (input, select_pressed) = ticks.take();
                play.session
                    .handle_input_with_dialog_select(input, aspect, select_pressed);
            }
            if let Some(network::NetworkPlay::Host(host)) = network.as_deref_mut() {
                host.capture(&play.session);
                if let Err(e) = host.publish(&play.session) {
                    error!("host snapshot: {e}");
                }
            }
        }
        // The inspection view also has to follow on a frame without a tick.
        if tick_count == 0 {
            apply_inspection_view(&mut play.session, &inspection);
        }
    }
    if let Some(book) = pickup_fixture_check {
        if book {
            assert!(
                play.session.unlocked_magicks[0] & (1 << 3) != 0,
                "mouse fixture failed to unlock book"
            );
            assert!(
                play.session.nearby_book().is_none(),
                "mouse fixture did not consume book"
            );
        } else {
            assert!(
                play.session
                    .avatar()
                    .items
                    .iter()
                    .any(|(slot, item)| *slot == 1
                        && item.definition.name == "staff_heirloom_grandfather_clock"),
                "mouse fixture did not exchange staff"
            );
        }
        eprintln!(
            "mouse pickup fixture: collected {} through PlayerInput",
            if book { "book" } else { "equipment" }
        );
    }
    particles.0.clear();
    projectile_audit.copy_particles(&play.session, &mut particles);
    if earth_trail_hidden {
        particles.0.retain(|p| ![161, 7, 197].contains(&p.sprite));
    }
    let mut cue = None;
    for event in play.session.events.drain(..) {
        match event {
            SessionEvent::Clip(c) => cue = Some(c),
            SessionEvent::NpcClip(handle, c) => {
                play.npc_seq += 1;
                let seq = play.npc_seq;
                play.npc_cues.insert(handle, (seq, c));
            }
            SessionEvent::PickupFailed(error) => warn!("pickup failed: {error}"),
            SessionEvent::PhysicsFailed(error) => warn!("pickup physics failed: {error}"),
            SessionEvent::ScriptSkipped(name) => {
                warn!("script action not reconstructed yet, skipped: {name}")
            }
            // Scene scripts stop for the rest of the session after this.
            SessionEvent::ScriptError(e) => error!("scene scripts stopped: {e}"),
            // Routine presentation events (retained NPC spell effects repeat
            // every frame); RUST_LOG=magicka_viewer=debug shows them.
            other => debug!("session event: {other:?}"),
        }
    }
    if cue.is_some() {
        play.pending = cue;
    }
}

/// Poses the rendered camera from the session camera and the numpad inspection state.
fn apply_inspection_view(session: &mut PlaySession, inspection: &InspectionCamera) {
    let camera = &mut session.world.camera;
    camera.set_inspection_view(None);
    let target = Vec3::from(camera.ground_position());
    let center = inspection.unlocked_target.unwrap_or(target) + inspection.pan;
    let rotation = Quat::from_rotation_y(inspection.yaw) * Quat::from_rotation_x(inspection.tilt);
    let position = center + rotation * (Vec3::from(camera.position()) - target);
    let direction = rotation * Vec3::from(camera.direction());
    let up = rotation * Vec3::from(camera.up());
    let fov = camera.fov() * inspection.zoom.exp();
    camera.set_inspection_view(Some(magicka_runtime::camera::CameraView {
        position: position.to_array(),
        direction: direction.to_array(),
        up: up.to_array(),
        fov,
    }));
}

/// Avatar.Update's OutlineRenderData gate (`research/avatar-outline.md`).
/// Play mode is a Campaign game; mDoNotRender is not modelled for avatars.
fn sync_outline(play: Res<Play>, mut outline: ResMut<crate::outline::AvatarOutline>) {
    let world = &play.session.world;
    let timer = play
        .session
        .avatar()
        .character
        .combat
        .health
        .invisibility_timer;
    outline.set_if_neq(crate::outline::AvatarOutline::from_state(
        play.session.dead(),
        false,
        world.cutscene.in_cutscene(),
        timer,
        1,
    ));
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_aura_halos(
    mut commands: Commands,
    play: Res<Play>,
    asset: Res<crate::aura::HaloAsset>,
    gltfs: Res<Assets<Gltf>>,
    meshes: Res<Assets<bevy::gltf::GltfMesh>>,
    gltf_materials: Res<Assets<bevy::gltf::GltfMaterial>>,
    camera: Query<&Transform, (With<PlayCamera>, Without<crate::aura::HaloPass>)>,
    mut halos: Query<
        (
            Entity,
            &crate::aura::HaloPass,
            &MeshMaterial3d<crate::aura::HaloMaterial>,
            &mut Transform,
        ),
        Without<PlayCamera>,
    >,
    mut materials: ResMut<Assets<crate::aura::HaloMaterial>>,
) {
    let Some(camera) = camera.iter().next() else {
        return;
    };
    let mut states: HashMap<_, _> = play
        .session
        .aura_halos()
        .into_iter()
        .map(|(h, p, c)| (h, (Vec3::from_array(p), Vec4::from_array(c))))
        .collect();
    for (entity, owner, material, mut transform) in &mut halos {
        if let Some((position, color)) = states.remove(&owner.0) {
            *transform = crate::aura::billboard(position, camera.translation);
            if let Some(mut m) = materials.get_mut(&material.0) {
                m.color = color;
            }
        } else {
            commands.entity(entity).despawn();
        }
    }
    if states.is_empty() {
        return;
    }
    let Some(gltf) = gltfs.get(&asset.0) else {
        return;
    };
    // AssignParts selects part1 for Halo; parts0/2 are never drawn.
    let Some(primitive) = gltf
        .meshes
        .first()
        .and_then(|m| meshes.get(m))
        .and_then(|m| m.primitives.get(1))
    else {
        return;
    };
    let Some(base) = primitive
        .material
        .as_ref()
        .and_then(|h| gltf_materials.get(h))
    else {
        return;
    };
    let Some(extras) = primitive.material_extras.as_ref() else {
        return;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&extras.value) else {
        return;
    };
    let flags = UVec4::new(
        u32::from(
            json["magicka_additive"]["texture_enabled"]
                .as_bool()
                .unwrap_or(false),
        ),
        u32::from(
            json["magicka_additive"]["vertex_color"]
                .as_bool()
                .unwrap_or(false),
        ),
        0,
        0,
    );
    for (owner, (position, color)) in states {
        commands.spawn((
            crate::aura::HaloPass(owner),
            Mesh3d(primitive.mesh.clone()),
            MeshMaterial3d(materials.add(crate::aura::HaloMaterial {
                color,
                flags,
                texture: base.base_color_texture.clone(),
                depth: Vec4::new(
                    crate::outline::depth_bias_coefficient(0.00125),
                    0.0,
                    0.0,
                    0.0,
                ),
            })),
            crate::aura::billboard(position, camera.translation),
            bevy::camera::visibility::NoFrustumCulling,
        ));
    }
}

/// IsEthereal and LooksEthereal select the native additive body draw.
#[allow(clippy::too_many_arguments)]
fn sync_ethereal(
    play: Res<Play>,
    npc_roots: Query<(Entity, &NpcRoot)>,
    avatar_roots: Query<Entity, With<AvatarRoot>>,
    children: Query<&Children>,
    meshes: Query<
        &MeshMaterial3d<crate::robe::RobeMaterial>,
        Without<crate::self_shield::SolidShieldPass>,
    >,
    mut materials: ResMut<Assets<crate::robe::RobeMaterial>>,
    mut original_alpha: Local<HashMap<AssetId<crate::robe::RobeMaterial>, AlphaMode>>,
) {
    let avatar = play.session.avatar();
    for (root, character) in npc_roots
        .iter()
        .filter_map(|(e, r)| {
            play.session
                .world
                .npcs
                .get(&r.handle)
                .map(|n| (e, &n.character))
        })
        .chain(avatar_roots.iter().map(|e| (e, &avatar.character)))
    {
        for entity in children.iter_descendants(root) {
            let Ok(handle) = meshes.get(entity) else {
                continue;
            };
            let Some(material) = materials.get(&handle.0) else {
                continue;
            };
            let active = (character.ethereal || character.definition.looks_ethereal)
                && material.extension.params.control.z < 0.5;
            let flags = material.extension.params.control.y as u32;
            let flags = (flags & !8) | if active { 8 } else { 0 };
            if material.extension.params.control.y == flags as f32
                && (!active || material.extension.params.spec.w == character.ethereal_alpha)
            {
                continue;
            }
            let mut material = materials.get_mut(&handle.0).unwrap();
            material.extension.params.control.y = flags as f32;
            material.extension.params.spec.w = character.ethereal_alpha;
            if active {
                original_alpha
                    .entry(handle.0.id())
                    .or_insert(material.base.alpha_mode);
                material.base.alpha_mode = AlphaMode::Blend;
            } else if let Some(alpha) = original_alpha.remove(&handle.0.id()) {
                material.base.alpha_mode = alpha;
            }
        }
    }
    original_alpha.retain(|id, _| materials.contains(*id));
}

/// Character.UpdateRenderData while IsInvisibile: the body primitive draws
/// only as the NormalDistortion pass (a twin sharing mesh and skin), and the
/// normal body and held items are not drawn (Pickable.Draw skips invisible
/// items). The self-shield skin still draws.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_invisibility(
    mut commands: Commands,
    play: Res<Play>,
    server: Res<AssetServer>,
    avatar_roots: Query<Entity, With<AvatarRoot>>,
    npc_roots: Query<(Entity, &NpcRoot)>,
    asset_roots: Query<&WorldAssetRoot>,
    parents: Query<&ChildOf>,
    bodies: Query<
        (
            Entity,
            &Mesh3d,
            &bevy::mesh::skinning::SkinnedMesh,
            &Transform,
            &ChildOf,
        ),
        (
            Without<crate::magick_visuals::BodySource>,
            With<MeshMaterial3d<crate::robe::RobeMaterial>>,
        ),
    >,
    drawn: Query<
        (Entity, Option<&crate::magick_visuals::HiddenByInvisibility>),
        (
            With<Mesh3d>,
            Or<(
                With<MeshMaterial3d<crate::robe::RobeMaterial>>,
                With<MeshMaterial3d<crate::lighting::LitMaterial>>,
                With<MeshMaterial3d<crate::lighting::PlayerLitMaterial>>,
                With<MeshMaterial3d<StandardMaterial>>,
            )>,
        ),
    >,
    mut passes: Query<(&crate::magick_visuals::InvisibilityPass, &mut Visibility)>,
    mut held: Query<
        (&equipment::HeldItem, &mut Visibility),
        Without<crate::magick_visuals::InvisibilityPass>,
    >,
    mut materials: ResMut<Assets<crate::magick_visuals::InvisibilityMaterial>>,
) {
    use crate::magick_visuals::{
        BodySource, HiddenByInvisibility, InvisibilityMaterial, InvisibilityPass,
    };
    let world = &play.session.world;
    let mut invisible = HashMap::new();
    for p in world.players.iter().flatten() {
        invisible.insert(p.handle, p.character.is_invisible());
    }
    for (&handle, n) in &world.npcs {
        invisible.insert(handle, n.character.is_invisible());
    }
    let avatar = play.session.avatar().handle;
    let roots: HashMap<Entity, u16> = avatar_roots
        .iter()
        .map(|e| (e, avatar))
        .chain(npc_roots.iter().map(|(e, n)| (e, n.handle)))
        .collect();
    let owner = |entity: Entity| {
        parents
            .iter_ancestors(entity)
            .find_map(|e| roots.get(&e).map(|h| (e, *h)))
    };
    for (entity, mesh, skin, transform, parent) in &bodies {
        let Some((root, handle)) = owner(entity) else {
            continue;
        };
        let Ok(asset_root) = asset_roots.get(root) else {
            continue;
        };
        let (Some(mesh_path), Some(root_path)) =
            (server.get_path(&mesh.0), server.get_path(&asset_root.0))
        else {
            continue;
        };
        // Model.Meshes[0].MeshParts[0], as for the shield skin.
        if mesh_path.path() != root_path.path() || mesh_path.label() != Some("Mesh0/Primitive0") {
            continue;
        }
        commands.entity(entity).insert(BodySource(handle));
        commands.spawn((
            InvisibilityPass(handle),
            Mesh3d(mesh.0.clone()),
            skin.clone(),
            *transform,
            MeshMaterial3d(materials.add(InvisibilityMaterial {
                params: Vec4::new(-0.4, 0.0, 0.0, 0.0),
            })),
            bevy::camera::visibility::NoFrustumCulling,
            Visibility::Hidden,
            ChildOf(parent.parent()),
        ));
    }
    for (pass, mut visible) in &mut passes {
        let wanted = if invisible.get(&pass.0).copied().unwrap_or(false) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        visible.set_if_neq(wanted);
    }
    // Item.IsInvisible: Pickable.Draw skips the model of an invisible owner.
    for (item, mut visible) in &mut held {
        let item_visible = world.npcs.get(&item.owner).is_none_or(|npc| {
            usize::try_from(item.slot)
                .ok()
                .and_then(|slot| npc.equipment_visible.get(slot))
                .copied()
                .unwrap_or(true)
        });
        let wanted = if !item_visible || invisible.get(&item.owner).copied().unwrap_or(false) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        visible.set_if_neq(wanted);
    }
    for (entity, hidden) in &drawn {
        let Some((_, handle)) = owner(entity) else {
            continue;
        };
        let now = invisible.get(&handle).copied().unwrap_or(false);
        match (now, hidden.is_some()) {
            (true, false) => {
                commands
                    .entity(entity)
                    .insert((HiddenByInvisibility, Visibility::Hidden));
            }
            (false, true) => {
                commands
                    .entity(entity)
                    .remove::<HiddenByInvisibility>()
                    .insert(Visibility::Inherited);
            }
            _ => {}
        }
    }
}

/// Avatar.Update's AfterImageRenderData for the local avatar: every 0.05 s
/// the body's joint world transforms are pushed into a five-pose history;
/// poses 1..4 draw as additive ColdColor ghosts with the native alphas.
/// Ghost joints are root entities whose transforms replay the history.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_afterimages(
    mut commands: Commands,
    play: Res<Play>,
    mut state: Local<(
        magicka_runtime::afterimage::AfterImages,
        std::collections::VecDeque<Vec<Mat4>>,
        Vec<Vec<Entity>>,
    )>,
    sources: Query<(
        &crate::magick_visuals::BodySource,
        &Mesh3d,
        &bevy::mesh::skinning::SkinnedMesh,
        &MeshMaterial3d<crate::robe::RobeMaterial>,
    )>,
    globals: Query<&GlobalTransform>,
    mut joints: Query<&mut Transform, Without<crate::magick_visuals::BodySource>>,
    mut ghosts: Query<(
        &crate::magick_visuals::AfterImagePass,
        &MeshMaterial3d<crate::magick_visuals::AfterImageMaterial>,
        &mut Visibility,
    )>,
    robes: Res<Assets<crate::robe::RobeMaterial>>,
    mut materials: ResMut<Assets<crate::magick_visuals::AfterImageMaterial>>,
) {
    use magicka_runtime::afterimage::{ghost_alphas, AfterImageStep, SNAPSHOTS};
    let avatar = play.session.avatar();
    let Some((_, mesh, skin, robe)) = sources.iter().find(|(s, ..)| s.0 == avatar.handle) else {
        return;
    };
    let (images, history, ghost_joints) = &mut *state;
    if ghost_joints.is_empty() {
        // Spawn the four ghosts with their own joint entities.
        let texture = robes
            .get(&robe.0)
            .and_then(|r| r.base.base_color_texture.clone());
        for ghost in 1..SNAPSHOTS {
            let set: Vec<Entity> = skin
                .joints
                .iter()
                .map(|_| commands.spawn(Transform::IDENTITY).id())
                .collect();
            commands.spawn((
                crate::magick_visuals::AfterImagePass(ghost),
                Mesh3d(mesh.0.clone()),
                bevy::mesh::skinning::SkinnedMesh {
                    inverse_bindposes: skin.inverse_bindposes.clone(),
                    joints: set.clone(),
                },
                MeshMaterial3d(materials.add(crate::magick_visuals::AfterImageMaterial {
                    params: Vec4::ZERO,
                    texture: texture.clone(),
                })),
                Transform::IDENTITY,
                bevy::camera::visibility::NoFrustumCulling,
                Visibility::Hidden,
            ));
            ghost_joints.push(set);
        }
        return;
    }
    let body = play.session.body();
    let v = body.velocity;
    let step = images.step(
        play.dt,
        avatar.character.is_invisible(),
        body.speed_multiplier(),
        1.0,
        v[0] * v[0] + v[2] * v[2],
    );
    let alphas = match step {
        AfterImageStep::Hidden => [None; SNAPSHOTS - 1],
        AfterImageStep::Drawn {
            snapshots,
            intensity,
        } => {
            let pose: Vec<Mat4> = skin
                .joints
                .iter()
                .map(|j| globals.get(*j).map_or(Mat4::IDENTITY, |g| g.to_matrix()))
                .collect();
            for _ in 0..snapshots.min(SNAPSHOTS as u32) {
                history.push_front(pose.clone());
            }
            history.truncate(SNAPSHOTS);
            ghost_alphas(intensity)
        }
    };
    for (pass, material, mut visible) in &mut ghosts {
        let alpha = alphas[pass.0 - 1];
        let pose = history.get(pass.0);
        let (Some(alpha), Some(pose)) = (alpha, pose) else {
            visible.set_if_neq(Visibility::Hidden);
            continue;
        };
        for (joint, m) in ghost_joints[pass.0 - 1].iter().zip(pose) {
            if let Ok(mut t) = joints.get_mut(*joint) {
                *t = Transform::from_matrix(*m);
            }
        }
        if let Some(mut m) = materials.get_mut(&material.0) {
            m.params.x = alpha;
        }
        visible.set_if_neq(Visibility::Inherited);
    }
}

/// Character.UpdateRenderData's pure ShieldSkinRenderData pass on the
/// original body primitive (not held equipment or the outline copies).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_self_shields(
    mut commands: Commands,
    play: Res<Play>,
    server: Res<AssetServer>,
    texture: Res<crate::self_shield::ShieldTexture>,
    avatar_roots: Query<Entity, With<AvatarRoot>>,
    remote_roots: Query<(Entity, &network::NetworkAvatar)>,
    npc_roots: Query<(Entity, &NpcRoot)>,
    asset_roots: Query<&WorldAssetRoot>,
    parents: Query<&ChildOf>,
    parts: Query<
        (
            Entity,
            &Mesh3d,
            &bevy::mesh::skinning::SkinnedMesh,
            &Transform,
            &ChildOf,
        ),
        (
            Without<crate::self_shield::ShieldSource>,
            Without<crate::self_shield::ShieldPass>,
            Or<(
                With<MeshMaterial3d<crate::robe::RobeMaterial>>,
                With<MeshMaterial3d<crate::lighting::LitMaterial>>,
            )>,
        ),
    >,
    mut passes: Query<(
        &crate::self_shield::ShieldPass,
        &MeshMaterial3d<crate::self_shield::SelfShieldMaterial>,
        &mut Visibility,
    )>,
    mut materials: ResMut<Assets<crate::self_shield::SelfShieldMaterial>>,
) {
    use crate::self_shield::{SelfShieldMaterial, ShieldParams, ShieldPass, ShieldSource};
    use magicka_sim::character_health::SelfShieldType;
    let avatar = play.session.avatar();
    // Every wizard shown here (local, mirrored) and the NPCs.
    let states: HashMap<u16, ShieldParams> = play
        .session
        .self_shields()
        .into_iter()
        .filter(|d| d.shield.kind == SelfShieldType::Shield)
        .map(|d| {
            (
                d.handle,
                ShieldParams::from_state(
                    Vec3::from_array(d.position),
                    Vec3::from_array(d.direction),
                    d.shield,
                ),
            )
        })
        .collect();
    let roots: HashMap<Entity, u16> = avatar_roots
        .iter()
        .map(|e| (e, avatar.handle))
        .chain(
            remote_roots
                .iter()
                .filter_map(|(e, r)| Some((e, r.mirror_handle(&play)?))),
        )
        .chain(npc_roots.iter().map(|(e, n)| (e, n.handle)))
        .collect();
    for (entity, mesh, skin, transform, parent) in &parts {
        let Some((root, handle)) = parents
            .iter_ancestors(entity)
            .find_map(|e| roots.get(&e).map(|h| (e, *h)))
        else {
            continue;
        };
        // Wait until a real shield exists; skin/assets may load later.
        let Some(params) = states.get(&handle).copied() else {
            continue;
        };
        let Ok(asset_root) = asset_roots.get(root) else {
            continue;
        };
        let (Some(mesh_path), Some(root_path)) =
            (server.get_path(&mesh.0), server.get_path(&asset_root.0))
        else {
            continue;
        };
        if mesh_path.path() != root_path.path() || mesh_path.label() != Some("Mesh0/Primitive0") {
            continue;
        }
        commands.entity(entity).insert(ShieldSource);
        commands.spawn((
            ShieldPass(handle),
            Mesh3d(mesh.0.clone()),
            skin.clone(),
            *transform,
            MeshMaterial3d(materials.add(SelfShieldMaterial {
                params,
                texture: texture.0.clone(),
            })),
            bevy::camera::visibility::NoFrustumCulling,
            ChildOf(parent.parent()),
        ));
    }
    for (pass, material, mut visible) in &mut passes {
        if let Some(params) = states.get(&pass.0) {
            *visible = Visibility::Inherited;
            if let Some(mut material) = materials.get_mut(&material.0) {
                material.params = *params;
            }
        } else {
            *visible = Visibility::Hidden;
        }
    }
}

/// BarrierSkinRenderData uses the original armour meshes and caster skeleton.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_solid_self_shields(
    mut commands: Commands,
    play: Res<Play>,
    server: Res<AssetServer>,
    solid: Res<crate::self_shield::SolidShieldAssets>,
    gltfs: Res<Assets<Gltf>>,
    gltf_meshes: Res<Assets<bevy::gltf::GltfMesh>>,
    gltf_materials: Res<Assets<bevy::gltf::GltfMaterial>>,
    lights: Res<crate::lighting::MagickaLights>,
    cubes: Res<crate::status_look::StatusCubes>,
    // Bevy allows 16 system parameters: the roots share one.
    (avatar_roots, remote_roots, npc_roots): (
        Query<Entity, With<AvatarRoot>>,
        Query<(Entity, &network::NetworkAvatar)>,
        Query<(Entity, &NpcRoot)>,
    ),
    asset_roots: Query<&WorldAssetRoot>,
    parents: Query<&ChildOf>,
    parts: Query<
        (
            Entity,
            &Mesh3d,
            &bevy::mesh::skinning::SkinnedMesh,
            &Transform,
            &ChildOf,
        ),
        (
            Without<crate::self_shield::SolidShieldSource>,
            Or<(
                With<MeshMaterial3d<crate::robe::RobeMaterial>>,
                With<MeshMaterial3d<crate::lighting::LitMaterial>>,
            )>,
        ),
    >,
    mut passes: Query<(
        &crate::self_shield::SolidShieldPass,
        &MeshMaterial3d<crate::robe::RobeMaterial>,
        &mut Visibility,
    )>,
    mut materials: ResMut<Assets<crate::robe::RobeMaterial>>,
) {
    use crate::self_shield::{solid_overlay, SolidShieldPass, SolidShieldSource};
    let avatar = play.session.avatar();
    let shield_of = |h| {
        play.session
            .world
            .players
            .iter()
            .flatten()
            .find(|p| p.handle == h)
            .map(|p| &p.character.combat.health.shield)
            .or_else(|| {
                play.session
                    .world
                    .npcs
                    .get(&h)
                    .map(|n| &n.character.combat.health.shield)
            })
    };
    // Removal retains old magnitudes, so the active kind must gate visibility.
    let strength = |h, ice| {
        shield_of(h).filter(|s| s.solid()).map_or(0.0, |s| {
            if ice {
                s.ice_magnitude
            } else {
                s.earth_magnitude
            }
        })
    };
    for (pass, material, mut visibility) in &mut passes {
        let magnitude = strength(pass.owner, pass.ice);
        *visibility = if magnitude > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.extension.params.control.x = solid_overlay(magnitude);
        }
    }
    let Some(gltf) = gltfs.get(&solid.0) else {
        return;
    };
    let Some(mesh) = gltf.meshes.first().and_then(|m| gltf_meshes.get(m)) else {
        return;
    };
    if mesh.primitives.len() != 2 {
        return;
    }
    let mut plans = Vec::new();
    for primitive in &mesh.primitives {
        let Some(base) = primitive
            .material
            .as_ref()
            .and_then(|h| gltf_materials.get(h))
        else {
            return;
        };
        let Some(extras) = &primitive.material_extras else {
            return;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&extras.value) else {
            return;
        };
        let base = StandardMaterial {
            base_color: base.base_color,
            base_color_texture: base.base_color_texture.clone(),
            ..default()
        };
        let Some(extension) = crate::robe::skinned_extension(
            &json["magicka_skinned"],
            &base,
            "gltf/Models/Effects/combined_armor.glb",
            &server,
            [1.0; 3],
            &lights,
            &cubes,
        ) else {
            return;
        };
        plans.push((primitive.mesh.clone(), base, extension));
    }
    let roots: HashMap<Entity, u16> = avatar_roots
        .iter()
        .map(|e| (e, avatar.handle))
        .chain(
            remote_roots
                .iter()
                .filter_map(|(e, r)| Some((e, r.mirror_handle(&play)?))),
        )
        .chain(npc_roots.iter().map(|(e, n)| (e, n.handle)))
        .collect();
    for (entity, body_mesh, skin, transform, parent) in &parts {
        let Some((root, owner)) = parents
            .iter_ancestors(entity)
            .find_map(|e| roots.get(&e).map(|h| (e, *h)))
        else {
            continue;
        };
        if strength(owner, false) <= 0.0 && strength(owner, true) <= 0.0 {
            continue;
        }
        let Ok(asset_root) = asset_roots.get(root) else {
            continue;
        };
        let (Some(mesh_path), Some(root_path)) = (
            server.get_path(&body_mesh.0),
            server.get_path(&asset_root.0),
        ) else {
            continue;
        };
        if mesh_path.path() != root_path.path() || mesh_path.label() != Some("Mesh0/Primitive0") {
            continue;
        }
        commands.entity(entity).insert(SolidShieldSource);
        for (index, (mesh, base, extension)) in plans.iter().enumerate() {
            let ice = index == 1;
            let magnitude = strength(owner, ice);
            let mut extension = extension.clone();
            extension.params.control.x = solid_overlay(magnitude);
            // Barrier materials do not receive the caster's Frozen/Wet look.
            extension.look = default();
            commands.spawn((
                SolidShieldPass { owner, ice },
                Mesh3d(mesh.clone()),
                skin.clone(),
                *transform,
                MeshMaterial3d(materials.add(crate::robe::RobeMaterial {
                    base: base.clone(),
                    extension,
                })),
                if magnitude > 0.0 {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
                bevy::camera::visibility::NoFrustumCulling,
                ChildOf(parent.parent()),
            ));
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_zap(
    mut commands: Commands,
    play: Res<Play>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut line_materials: ResMut<Assets<crate::zap::ZapLineMaterial>>,
    lines: Query<(Entity, &crate::zap::ZapLines, &Mesh3d)>,
    npc_roots: Query<(Entity, &NpcRoot)>,
    avatar_roots: Query<Entity, With<AvatarRoot>>,
    remote_roots: Query<(Entity, &network::NetworkAvatar)>,
    children: Query<&Children>,
    lit_meshes: Query<
        &MeshMaterial3d<crate::lighting::LitMaterial>,
        Without<crate::self_shield::SolidShieldPass>,
    >,
    robe_meshes: Query<
        &MeshMaterial3d<crate::robe::RobeMaterial>,
        Without<crate::self_shield::SolidShieldPass>,
    >,
    entity_meshes: Query<
        &MeshMaterial3d<crate::lighting::PlayerLitMaterial>,
        Without<crate::self_shield::SolidShieldPass>,
    >,
    mut lit: ResMut<Assets<crate::lighting::LitMaterial>>,
    mut robes: ResMut<Assets<crate::robe::RobeMaterial>>,
    mut entities: ResMut<Assets<crate::lighting::PlayerLitMaterial>>,
    cubes: Res<crate::status_look::StatusCubes>,
) {
    use magicka_sim::status::Status;
    let look_of = |statuses: &magicka_sim::character_status::CharacterStatuses| {
        let magnitude = |s| statuses.effect(s).map_or(0.0, |e| e.magnitude);
        crate::status_look::StatusLook::from_status(
            magnitude(Status::Cold),
            statuses.has(Status::Wet),
            statuses
                .has(Status::Frozen)
                .then(|| magnitude(Status::Frozen)),
        )
    };
    let mut looks: HashMap<u16, crate::status_look::StatusLook> = play
        .session
        .world
        .npcs
        .iter()
        .map(|(&h, n)| (h, look_of(&n.character.combat.statuses)))
        .collect();
    let avatar_character = &play.session.avatar().character;
    looks.insert(
        play.session.avatar().handle,
        look_of(&avatar_character.combat.statuses),
    );
    let zapped: HashMap<u16, (f32, Vec<[[f32; 3]; 2]>)> = play
        .session
        .zapped_characters()
        .into_iter()
        .map(|z| (z.handle, (z.radius, z.lines)))
        .collect();
    let avatar = play.session.avatar().handle;
    let roots = npc_roots
        .iter()
        .map(|(e, r)| (e, r.handle))
        .chain(avatar_roots.iter().map(|e| (e, avatar)))
        .chain(
            remote_roots
                .iter()
                .filter_map(|(e, r)| Some((e, r.mirror_handle(&play)?))),
        );
    for (root, handle) in roots {
        let flag = if zapped.contains_key(&handle) {
            1.0
        } else {
            0.0
        };
        let look = looks.get(&handle).copied().unwrap_or_default();
        let ice = (cubes.ice.clone(), cubes.ice_normal.clone());
        let stale = |zap: f32, l: crate::status_look::StatusLook, has_ice: bool| {
            zap != flag || l != look || (!has_ice && ice.0.is_some())
        };
        for entity in children.iter_descendants(root) {
            if let Ok(m) = entity_meshes.get(entity) {
                if entities.get(&m.0).is_some_and(|m| {
                    let e = &m.extension;
                    stale(e.params.material.w, e.look, e.ice.is_some())
                }) {
                    if let Some(mut m) = entities.get_mut(&m.0) {
                        m.extension.params.material.w = flag;
                        m.extension.look = look;
                        m.extension.ice = ice.0.clone();
                        m.extension.ice_normal = ice.1.clone();
                    }
                }
            }
            if let Ok(m) = lit_meshes.get(entity) {
                if lit.get(&m.0).is_some_and(|m| {
                    let e = &m.extension;
                    stale(e.params.material.w, e.look, e.ice.is_some())
                }) {
                    if let Some(mut m) = lit.get_mut(&m.0) {
                        m.extension.params.material.w = flag;
                        m.extension.look = look;
                        m.extension.ice = ice.0.clone();
                        m.extension.ice_normal = ice.1.clone();
                    }
                }
            }
            if let Ok(m) = robe_meshes.get(entity) {
                if robes.get(&m.0).is_some_and(|m| {
                    let e = &m.extension;
                    stale(e.params.control.z, e.look, e.ice.is_some())
                }) {
                    if let Some(mut m) = robes.get_mut(&m.0) {
                        m.extension.params.control.z = flag;
                        m.extension.look = look;
                        m.extension.ice = ice.0.clone();
                        m.extension.ice_normal = ice.1.clone();
                    }
                }
            }
        }
    }
    let mut pending = zapped;
    for (entity, line, mesh) in &lines {
        match pending.remove(&line.0) {
            Some((_, segments)) => {
                meshes
                    .insert(&mesh.0, crate::zap::line_mesh(&segments))
                    .ok();
            }
            None => commands.entity(entity).despawn(),
        }
    }
    for (handle, (radius, segments)) in pending {
        commands.spawn((
            crate::zap::ZapLines(handle),
            Mesh3d(meshes.add(crate::zap::line_mesh(&segments))),
            MeshMaterial3d(line_materials.add(crate::zap::ZapLineMaterial {
                params: Vec4::new(radius, 0.0, 0.0, 0.0),
            })),
            Transform::default(),
            bevy::camera::visibility::NoFrustumCulling,
        ));
    }
}

/// Render vertices of a liquid surface matched to runtime Water/Lava
/// collision vertices by world position, with the last uploaded values.
#[derive(Component)]
struct FreezeMap {
    water: usize,
    indices: Vec<Option<usize>>,
    uploaded: Vec<f32>,
}

/// Water/Lava vertex stream 1: copy the runtime freeze values onto the matching
/// render vertices (liquid.wgsl reads UV_1.x).
#[allow(clippy::type_complexity)]
fn sync_water_freeze(
    mut commands: Commands,
    play: Res<Play>,
    mut meshes: ResMut<Assets<Mesh>>,
    unmapped: Query<
        (Entity, &Mesh3d, &GlobalTransform),
        (With<crate::liquid::LiquidSurface>, Without<FreezeMap>),
    >,
    mut mapped: Query<(&Mesh3d, &mut FreezeMap)>,
) {
    let waters = &play.session.world.collision.collision().water;
    let key = |p: [f32; 3]| p.map(|v| (v * 1000.0).round() as i64);
    for (entity, mesh, transform) in &unmapped {
        let Some(m) = meshes.get(&mesh.0) else {
            continue;
        };
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            m.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            continue;
        };
        let world: Vec<[f32; 3]> = positions
            .iter()
            .map(|p| transform.transform_point(Vec3::from(*p)).to_array())
            .collect();
        let found = waters.iter().enumerate().find_map(|(w, water)| {
            let lookup: HashMap<[i64; 3], usize> = water
                .vertices()
                .iter()
                .enumerate()
                .map(|(i, v)| (key(*v), i))
                .collect();
            let indices: Vec<Option<usize>> = world
                .iter()
                .map(|p| lookup.get(&key(*p)).copied())
                .collect();
            let hits = indices.iter().flatten().count();
            (hits * 2 > indices.len()).then_some((w, indices))
        });
        // Unmatched animated liquids retain their exported initial stream.
        let (water, indices) = found.unwrap_or((usize::MAX, Vec::new()));
        commands.entity(entity).insert(FreezeMap {
            water,
            indices,
            uploaded: Vec::new(),
        });
    }
    for (mesh, mut map) in &mut mapped {
        let Some(water) = waters.get(map.water) else {
            continue;
        };
        let values: Vec<f32> = map
            .indices
            .iter()
            .map(|i| {
                i.and_then(|i| water.freeze.vertices.get(i))
                    .copied()
                    .unwrap_or(0.0)
            })
            .collect();
        if values == map.uploaded {
            continue;
        }
        if let Some(mut m) = meshes.get_mut(&mesh.0) {
            let uv: Vec<[f32; 2]> = values.iter().map(|&v| [v, 0.0]).collect();
            m.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv);
            map.uploaded = values;
        }
    }
}

/// GameScene Filter/Bloom settings for the post-processing cameras.
/// Credits.OnTransitionEnd: the PlayState is popped and SubMenuEndGame
/// (CreditsEnd) closes the Tome back to the main menu; the play session ends
/// here (the menu front end shows the main menu again). Autopilot captures
/// keep running to their scripted end.
fn leave_after_credits(
    play: Res<Play>,
    autopilot: Option<Res<Autopilot>>,
    mut exit: MessageWriter<LeaveSession>,
) {
    if play.session.credits_finished && autopilot.is_none() {
        exit.write(LeaveSession {
            credits: true,
            ..default()
        });
    }
}

/// PlayState's LevelComplete hand-off when the menu hosts the session: the
/// host captures the last frame, then [`advance_campaign`] moves the save on.
fn leave_completed_level(
    mut play: ResMut<Play>,
    network: Option<NonSend<network::NetworkPlay>>,
    time: Res<Time>,
    mut exit: MessageWriter<LeaveSession>,
) {
    if let Some(left) = play.complete_after.as_mut() {
        *left -= time.delta_secs();
        if *left <= 0.0 {
            play.complete_after = None;
            // EndGame.Execute → PlayState.Endgame(LevelComplete, freeze, false, 0).
            play.session.world.pending_end_game = Some((2, true, false));
        }
    }
    if play.menu_hosted && network.is_none() && play.session.next_level().is_some() {
        exit.write(LeaveSession {
            level_complete: true,
            ..default()
        });
    }
}

/// Before teardown of a completed campaign level: load the next level into
/// the session (carrying equipment, as in place) and save it to the slot,
/// so the next chapter continues from that save. Returns its file name.
pub(crate) fn advance_campaign(world: &mut World) -> Option<String> {
    let next = world.get_resource::<Play>()?.session.next_level()?;
    if let Err(e) = world.resource_mut::<Play>().session.change_level(next) {
        error!("campaign transition failed: {e:?}");
        return None;
    }
    save_slots::save_now(world);
    Some(next.to_string())
}

/// Extra local players to carry through the menu host's chapter intermission.
pub(crate) fn local_party(world: &World) -> Vec<serde_json::Value> {
    let (Some(play), Some(coop)) = (
        world.get_resource::<Play>(),
        world.get_resource::<local_coop::LocalCoop>(),
    ) else {
        return Vec::new();
    };
    coop.continuation_players(&play.session)
        .into_iter()
        .filter(|seat| {
            seat["pad"]
                .as_u64()
                .is_some_and(|bits| world.get::<Gamepad>(Entity::from_bits(bits)).is_some())
        })
        .collect()
}

fn sync_blizzard(
    play: Res<Play>,
    mut cameras: Query<(&Projection, &mut crate::blizzard::BlizzardPost)>,
) {
    let blizzard = play.session.world.ability_effects.blizzard_state();
    for (projection, mut post) in &mut cameras {
        post.control.x = blizzard.map_or(0.0, |b| b.alpha);
        if let Some(b) = blizzard {
            post.offsets = Vec4::new(
                b.offsets[0][0],
                b.offsets[0][1],
                b.offsets[1][0],
                b.offsets[1][1],
            );
        }
        post.view_from_clip = projection.get_clip_from_view().inverse();
    }
}

fn sync_post(play: Res<Play>, mut cameras: Query<&mut crate::post::MagickaPost>) {
    let p = play.session.post;
    let wanted = crate::post::MagickaPost {
        filter: Vec4::new(
            p.brightness,
            p.contrast,
            play.session.render_saturation(),
            play.session.world.flash_level(),
        ),
        bloom: Vec4::new(p.bloom_threshold, p.bloom_multiplier, p.blur_sigma, 0.0),
    };
    for mut post in &mut cameras {
        post.set_if_neq(wanted);
    }
}

/// Avatar.Initialize with another template (Polymorph forms and their
/// reversal): reload the exported model of the local avatar's current
/// template and respawn its root, as the network robe mirror does.
fn sync_avatar_template(
    mut commands: Commands,
    play: Res<Play>,
    server: Res<AssetServer>,
    npcs: Res<NpcAssets>,
    mut avatar_assets: ResMut<AvatarAssets>,
    roots: Query<Entity, With<AvatarRoot>>,
    mut shown: Local<Option<i32>>,
) {
    let current = play.session.avatar().template_type;
    let Some(previous) = shown.replace(current) else {
        return;
    };
    if previous == current {
        return;
    }
    let Some(path) = npcs
        .files
        .iter()
        .find(|(name, _)| magicka_runtime::scene_loading::custom_hash(name) == current)
        .map(|(_, path)| path.clone())
    else {
        return;
    };
    for e in &roots {
        commands.entity(e).despawn();
    }
    avatar_assets.gltf = server.load(path.clone());
    avatar_assets.scene = server.load(GltfAssetLabel::Scene(0).from_asset(path));
    avatar_assets.graph = None;
    commands.spawn((
        AvatarRoot,
        crate::lighting::NoDepthPrepass,
        Transform::default(),
        Visibility::default(),
        WorldAssetRoot(avatar_assets.scene.clone()),
    ));
}

fn sync_avatar(play: Res<Play>, mut roots: Query<&mut Transform, With<AvatarRoot>>) {
    let (feet, o) = play.session.avatar_transform();
    // XNA row basis (Right, Up, Backward) → column-vector rotation columns.
    let rotation = Quat::from_mat3(&Mat3::from_cols(
        Vec3::from(o.right),
        Vec3::from(o.up),
        Vec3::from(o.backward),
    ))
    .normalize();
    for mut t in &mut roots {
        t.translation = Vec3::from(feet);
        t.rotation = rotation;
        // SetStaticTransform's size times ScaleGraphicModel (Cthulhu's devour).
        let c = &play.session.avatar().character;
        t.scale = Vec3::splat(c.model_scale * c.graphic_scale);
    }
}

fn sync_camera(
    mut play: ResMut<Play>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<PlayCamera>>,
) {
    let camera = play.session.camera();
    let position = Vec3::from(camera.position());
    let direction = Vec3::from(camera.direction());
    let up = Vec3::from(camera.up());
    let fov = camera.fov();
    for (mut t, mut projection) in &mut cameras {
        *t = Transform::from_translation(position).looking_to(direction, up);
        if let Projection::Perspective(p) = &mut *projection {
            p.fov = fov;
            p.near = NEAR_CLIP;
            p.far = FAR_CLIP;
        }
    }
    let aspect = play.screen[0] / play.screen[1].max(1.0);
    let camera = play.session.camera();
    let vp = magicka_runtime::camera::multiply(&camera.view(), &camera.projection(aspect));
    play.session.world.dialog_matrix =
        std::array::from_fn(|r| std::array::from_fn(|c| vp[r * 4 + c]));
}

fn drive_animation(
    mut play: ResMut<Play>,
    avatar: Res<AvatarAssets>,
    mut animators: Query<(&mut AnimationPlayer, &mut AnimationTransitions), With<AvatarAnimator>>,
    #[cfg(feature = "debug-ui")] preview: Res<debug_menu::AnimationPreview>,
) {
    let Some((_, nodes)) = &avatar.graph else {
        return;
    };
    let Ok((mut player, mut transitions)) = animators.single_mut() else {
        return;
    };
    if let Some(cue) = play.pending.take() {
        match nodes.get(&cue.clip) {
            Some(&node) => {
                let active = transitions.play(
                    &mut player,
                    node,
                    Duration::from_secs_f32(cue.fade.max(1e-3)),
                );
                if cue.looping {
                    active.repeat();
                }
            }
            None => warn!("clip {} missing from glTF", cue.clip),
        }
    }
    // AnimationController rate = ClipSpeed × Speed.
    let controller = &play.session.avatar().motion.controller;
    let speed = controller.clip_speed() * controller.speed;
    if let Some(node) = transitions.get_main_animation() {
        if let Some(active) = player.animation_mut(node) {
            active.set_speed(speed);
            #[cfg(feature = "debug-ui")]
            if preview.enabled {
                active.set_speed(0.0).seek_to(controller.time);
            }
        }
    }
}

fn update_floating_text(
    play: Res<Play>,
    pilot: Option<Res<Autopilot>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut draws: Query<(&crate::floating_text::FloatingTextGeometry, &mut Visibility)>,
) {
    let act = pilot.as_ref().and_then(|p| {
        p.started
            .and_then(|start| script_act(p.script, p.frame - start))
    });
    let shot = pilot.as_ref().is_some_and(|p| p.pending_shot);
    let numbers: &[_] = if act == Some(Act::NumbersHidden) {
        &[]
    } else {
        &play.session.world.floating_numbers
    };
    let screen = play.screen;
    let camera = play.session.camera();
    let vp = magicka_runtime::camera::multiply(
        &camera.view(),
        &camera.projection(screen[0] / screen[1]),
    );
    if shot && act == Some(Act::NumbersVisible) {
        let world = &play.session.world;
        let owners: Vec<(&str, i32)> = world
            .players
            .iter()
            .flatten()
            .map(|p| ("player", p.character.damage_bookkeeping.last_damage_index))
            .chain(
                world
                    .npcs
                    .values()
                    .map(|n| ("npc", n.character.damage_bookkeeping.last_damage_index)),
            )
            .filter(|(_, i)| *i >= 0)
            .collect();
        eprintln!("damage numbers owners: {owners:?}");
        crate::floating_text::audit(numbers, &vp, screen);
    }
    crate::floating_text::draw(numbers, &vp, screen, &mut meshes, &mut draws);
}

fn update_native_hud(
    play: Res<Play>,
    pilot: Option<Res<Autopilot>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut draws: Query<(&crate::play_hud::HudGeometry, &mut Visibility)>,
    tome_text: Query<&crate::play_hud::TomeTextGeometry>,
    network: Option<NonSend<network::NetworkPlay>>,
) {
    use magicka_runtime::camera::multiply;
    use magicka_runtime::hud::{
        bracket_quads, npc_bars, queue_icon_quads, self_shield_bar, shield_bar, HealthBar,
    };
    use magicka_runtime::tome_magick;
    use magicka_sim::status::Status;
    let screen = play.screen;
    let camera = play.session.camera();
    let vp = multiply(&camera.view(), &camera.projection(screen[0] / screen[1]));
    let project = |p: [f32; 3]| -> Option<[f32; 2]> {
        let mut clip = [0.; 4];
        for (i, c) in clip.iter_mut().enumerate() {
            *c = p[0] * vp[i] + p[1] * vp[4 + i] + p[2] * vp[8 + i] + vp[12 + i];
        }
        if clip[3] <= 0. || clip[2] < 0. || clip[2] > clip[3] {
            return None;
        }
        Some([
            (clip[0] / clip[3] * 0.5 + 0.5) * screen[0],
            (0.5 - clip[1] / clip[3] * 0.5) * screen[1],
        ])
    };
    let mut quads = vec![];
    // Healthbars (players, NPCs, character and Shield entities): submitted
    // only while PlayState.UIEnabled; IconRenderer quads always.
    let mut bars = vec![];
    let mut text_quads = vec![];
    let mut bars_drawn = 0;
    let world = &play.session.world;
    // GlobalSettings.HealthBars: avatars unless Off, NPCs only when On.
    let health_bars = magicka_runtime::global_settings::health_bars();
    let player_bars = health_bars != magicka_runtime::global_settings::HealthBars::Off;
    let npc_health_bars = health_bars == magicka_runtime::global_settings::HealthBars::On;
    for (slot, player) in world.players.iter().enumerate() {
        let Some(player) = player else {
            continue;
        };
        if player.dead() {
            continue;
        }
        let body = &world.physics.bodies[player.body];
        let feet = [
            body.position[0],
            body.position[1] - body.length * 0.5 - body.radius,
            body.position[2],
        ];
        let Some(p) = project(feet) else {
            continue;
        };
        let c = &player.character;
        let age = c
            .combat
            .time_since_last_damage
            .min(c.damage_bookkeeping.time_since_status_damage);
        let poisoned = c.combat.statuses.has(Status::Poisoned);
        bars.extend(
            player_bars
                .then_some(HealthBar {
                    normalized: c.normalized_hit_points,
                    width: c.definition.radius,
                    height: 1.25,
                    damage_age: age,
                    fade: false,
                    color: if poisoned {
                        [0., 1., 0., 1.]
                    } else {
                        [1., 0., 0., 1.]
                    },
                    offset: [0.; 2],
                })
                .into_iter()
                .flat_map(|bar| bar.quads(p)),
        );
        // IconRenderer.Draw: brackets, then queue icons, then the TomeMagick
        // hint in the slots the queue leaves free; the name below them.
        let tome = &play.session.tome[slot];
        // AddIcon takes nothing with SpellWheel off, so the queue draws no icons
        // and leaves its slots to the hint.
        let wheel = magicka_runtime::global_settings::spell_wheel();
        let queue = &player.conjure.queue;
        let icons = if wheel { queue.len() } else { 0 };
        // A registered (mod) Magick draws its registry combination.
        let registered = tome.shown_registered.and_then(|i| {
            play.session
                .registered_magicks
                .iter()
                .find(|(x, _)| *x == i)
                .map(|(_, combo)| (i, combo.clone()))
        });
        let hint = match &registered {
            Some((_, combo)) => tome_magick::hint_quads_for(combo, tome.alpha, icons, p),
            None => tome_magick::hint_quads(tome, icons, p),
        };
        if icons > 0 || !hint.is_empty() || tome_magick::draws_brackets(tome, icons) {
            quads.extend(bracket_quads(p));
        }
        if wheel {
            quads.extend(queue_icon_quads(queue, p));
        }
        quads.extend(hint);
        if let Some(text) = tome_text.iter().next() {
            let anchor = tome_magick::name_anchor(p);
            let name = match &registered {
                Some((i, _)) => text.registered_names.get(i).map_or(&[][..], Vec::as_slice),
                None => &text.names[tome.shown as usize][..],
            };
            text_quads.extend(name.iter().map(|q| magicka_runtime::hud::HudQuad {
                bounds: [
                    q.bounds[0] + anchor[0],
                    q.bounds[1] + anchor[1],
                    q.bounds[2] + anchor[0],
                    q.bounds[3] + anchor[1],
                ],
                atlas: q.atlas,
                color: [1., 1., 1., tome.alpha],
            }));
        }
        bars_drawn += 1;
        if let Some(bar) = self_shield_bar(
            &c.combat.health.shield,
            c.definition.radius,
            c.definition.health_bars,
        ) {
            if bars_drawn < 512 {
                let batch = bar.quads(p);
                if pilot
                    .as_ref()
                    .is_some_and(|p| p.pending_shot && p.script == SHIELD_BARS_SCRIPT)
                {
                    eprintln!(
                        "shield HUD audit: frame={} kind=Self normalized={} bounds={:?}",
                        pilot.as_ref().unwrap().frame,
                        bar.normalized,
                        batch[3].bounds
                    );
                }
                bars.extend(batch);
                bars_drawn += 1;
            }
        }
        // Character.Update: self-casted aura TTL bars at the self-shield offset.
        for bar in magicka_runtime::hud::character_aura_bars(c) {
            if bars_drawn >= 512 {
                break;
            }
            bars.extend(bar.quads(p));
            bars_drawn += 1;
        }
    }
    // Clients draw the host's NPC bars from the snapshot (no live NPCs).
    if let Some(network::NetworkPlay::Client(client)) = network.as_deref() {
        for npc in client.npc_bars().into_iter().filter(|_| npc_health_bars) {
            let Some(p) = project(npc.feet) else {
                continue;
            };
            for bar in npc_bars(npc.normalized, npc.count, npc.radius, npc.damage_age, false) {
                if bars_drawn == 512 {
                    break;
                }
                let batch = bar.quads(p);
                if !batch.is_empty() {
                    bars_drawn += 1;
                    bars.extend(batch);
                }
            }
        }
    }
    for npc in world.npcs.values().filter(|_| npc_health_bars) {
        let c = &npc.character;
        if c.combat.health.hit_points <= 0. {
            continue;
        }
        let body = &world.physics.bodies[npc.body];
        let feet = [
            body.position[0],
            body.position[1] - body.length * 0.5 - body.radius,
            body.position[2],
        ];
        let Some(p) = project(feet) else {
            continue;
        };
        let age = c
            .combat
            .time_since_last_damage
            .min(c.damage_bookkeeping.time_since_status_damage);
        for bar in npc_bars(
            c.normalized_hit_points,
            c.definition.health_bars,
            c.definition.radius,
            age,
            c.combat.statuses.has(Status::Poisoned),
        ) {
            if bars_drawn == 512 {
                break;
            }
            let batch = bar.quads(p);
            if !batch.is_empty() {
                bars_drawn += 1;
                bars.extend(batch);
            }
        }
    }
    for npc in world.npcs.values() {
        let c = &npc.character;
        if c.combat.health.hit_points <= 0.0 || bars_drawn >= 512 {
            continue;
        }
        let Some(bar) = self_shield_bar(
            &c.combat.health.shield,
            c.definition.radius,
            c.definition.health_bars,
        ) else {
            continue;
        };
        let body = &world.physics.bodies[npc.body];
        if let Some(p) = project(body.feet()) {
            bars.extend(bar.quads(p));
            bars_drawn += 1;
        }
    }
    // Character.Update's aura TTL bars for live NPCs (self-casted auras).
    for npc in world.npcs.values() {
        let c = &npc.character;
        if c.combat.health.hit_points <= 0.0 || bars_drawn >= 512 {
            continue;
        }
        let bars_for_npc = magicka_runtime::hud::character_aura_bars(c);
        if bars_for_npc.is_empty() {
            continue;
        }
        let body = &world.physics.bodies[npc.body];
        let Some(p) = project(body.feet()) else {
            continue;
        };
        for bar in bars_for_npc {
            if bars_drawn >= 512 {
                break;
            }
            bars.extend(bar.quads(p));
            bars_drawn += 1;
        }
    }
    for live in &world.shields {
        if live.shield.dead || bars_drawn >= 512 {
            continue;
        }
        let (position, bar) = shield_bar(&live.shield);
        if let Some(p) = project(position) {
            let batch = bar.quads(p);
            if pilot
                .as_ref()
                .is_some_and(|p| p.pending_shot && p.script == SHIELD_BARS_SCRIPT)
            {
                eprintln!(
                    "shield HUD audit: frame={} kind={:?} normalized={} bounds={:?}",
                    pilot.as_ref().unwrap().frame,
                    live.shield.kind,
                    bar.normalized,
                    batch[3].bounds
                );
            }
            bars.extend(batch);
            bars_drawn += 1;
        }
    }
    if world.ui_enabled {
        bars.append(&mut quads);
        quads = bars;
    }
    for (draw, mut visibility) in &mut draws {
        *visibility = if quads.is_empty() {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        if let Some(mut mesh) = meshes.get_mut(&draw.mesh) {
            *mesh = crate::play_hud::geometry(&quads, screen, draw.atlas_size);
        }
    }
    for text in &tome_text {
        if let Some(mut mesh) = meshes.get_mut(&text.mesh) {
            *mesh = crate::play_hud::geometry(&text_quads, screen, text.font_size);
        }
    }
}

fn update_keyboard_hud(
    play: Res<Play>,
    controller: Res<crate::controller::ControllerInput>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::play_hud::HudMaterial>>,
    draws: Query<(
        &crate::play_hud::KeyboardGeometry,
        &MeshMaterial2d<crate::play_hud::HudMaterial>,
    )>,
    mut visibility: Query<
        (&mut Visibility, Has<crate::play_hud::KeyboardLabel>),
        Or<(
            With<crate::play_hud::KeyboardGeometry>,
            With<crate::play_hud::KeyboardLabel>,
        )>,
    >,
    network: Option<NonSend<network::NetworkPlay>>,
    #[cfg(feature = "editor")] tool_mode: Res<bevy_toolmode::ToolMode>,
) {
    // Spectators, dungeon masters and invaders have no wizard to conjure with.
    let wizardless = matches!(
        network.as_deref(),
        Some(network::NetworkPlay::Client(c)) if c.own_slot().is_none()
    ) || matches!(
        network.as_deref(),
        Some(network::NetworkPlay::Host(h)) if h.is_dedicated()
    );
    // Tool mode's hotbar takes the element bar's place.
    #[cfg(feature = "editor")]
    let wizardless = wizardless || tool_mode.active;
    // HUDManager draws the KeyboardHUD only while PlayState.UIEnabled.
    let shown = if play.session.world.ui_enabled && !wizardless {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for (mut v, label) in &mut visibility {
        v.set_if_neq(if label && controller.active {
            Visibility::Hidden
        } else {
            shown
        });
    }
    crate::play_hud::update_keyboard(
        &play.session.keyboard_hud,
        play.screen,
        controller.active.then_some(controller.element_style),
        &mut meshes,
        &mut materials,
        &draws,
    );
}

#[allow(clippy::too_many_arguments)]
fn autopilot_capture(
    mut commands: Commands,
    pilot: Option<ResMut<Autopilot>>,
    mut done: Local<bool>,
    network: Option<NonSend<network::NetworkPlay>>,
    play: Res<Play>,
    npc_roots: Query<&NpcRoot>,
    projectile_audit: Res<npc_projectile_qa::Audit>,
    missile_roots: Query<(Entity, &MissileRoot)>,
    missile_meshes: Query<
        (
            Entity,
            Option<&MeshMaterial3d<crate::terrain::TerrainMaterial>>,
        ),
        With<Mesh3d>,
    >,
    missile_materials: Res<Assets<crate::terrain::TerrainMaterial>>,
    parents: Query<&ChildOf>,
    mut presentation_audit: Local<[usize; 3]>,
    notices: Query<(&NativeBitmapText, &Visibility), With<network::ConnectionNotice>>,
) {
    let Some(mut pilot) = pilot else { return };
    if matches!(network.as_deref(), Some(network::NetworkPlay::Client(_))) {
        let live_particles = play
            .session
            .effects
            .particles
            .active()
            .iter()
            .filter(|p| p.radius > 0. && p.normalized_lifetime < 1.)
            .count();
        let fire = play
            .session
            .effects
            .active_effects_by_hash(magicka_runtime::scene_loading::custom_hash("spray_fire1"));
        if fire > 0 && live_particles > 0 {
            presentation_audit[0] += 1;
            presentation_audit[1] = presentation_audit[1].max(live_particles);
        }
        presentation_audit[2] = presentation_audit[2].max(npc_roots.iter().count());
    }
    if pilot.pending_shot {
        pilot.pending_shot = false;
        let name = pilot.log.len();
        let path = pilot.dir.join(format!("{name:02}.png"));
        if projectile_audit.missile.is_some() {
            let mut report = projectile_audit.report(&play.session);
            let root = missile_roots
                .iter()
                .find(|(_, r)| Some(r.0) == projectile_audit.missile)
                .map(|(e, _)| e);
            report["mesh_descendants"] = serde_json::json!(root.map_or(0, |root| missile_meshes
                .iter()
                .filter(|(e, _)| parents.iter_ancestors(*e).any(|p| p == root))
                .count()));
            report["deferred_surfaces"] = serde_json::json!(missile_meshes.iter()
                .filter(|(e,_)| root.is_some_and(|r| parents.iter_ancestors(*e).any(|p| p == r)))
                .filter_map(|(_,handle)| handle.and_then(|h| missile_materials.get(&h.0)))
                .map(|m| serde_json::json!({"spec":m.params.spec0.to_array(),"color":m.params.color0.to_array()}))
                .collect::<Vec<_>>());
            std::fs::write(
                pilot.dir.join(format!("npc-projectile-{name:02}.json")),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .expect("write NPC projectile audit");
        }
        if let Some(network::NetworkPlay::Client(client)) = network.as_deref() {
            std::fs::write(
                pilot.dir.join(format!("network-{name:02}.json")),
                serde_json::to_vec_pretty(&client.frame).unwrap(),
            )
            .expect("write network snapshot");
            std::fs::write(
                pilot.dir.join(format!("presentation-{name:02}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "active_emitters": play.session.effects.active_effects(),
                    "live_particles": play.session.effects.particles.active().iter()
                        .filter(|p| p.radius > 0. && p.normalized_lifetime < 1.).count(),
                    "npc_handles": client.frame.npcs.iter().map(|n| n.handle).collect::<Vec<_>>(),
                    "rendered_npc_handles": npc_roots.iter().map(|n| n.handle).collect::<Vec<_>>(),
                    "fire_emitters": play.session.effects.active_effects_by_hash(
                        magicka_runtime::scene_loading::custom_hash("spray_fire1")),
                }))
                .unwrap(),
            )
            .expect("write presentation audit");
        }
        commands
            .spawn(Screenshot::image(pilot.target.clone()))
            .observe(save_to_disk(path));
    }
    let notice_ready = notices.iter().any(|(text, visibility)| {
        *visibility == Visibility::Visible
            && text.typing.primitive_count > 0
            && text.typing.visible_characters > 0
            && text.failure.is_none()
    });
    let disconnected =
        matches!(network.as_deref(), Some(network::NetworkPlay::Client(c)) if c.disconnected());
    let rejoin_finished = pilot
        .script
        .last()
        .is_some_and(|s| s.3 == "wait_for_rejoin")
        && pilot.dir.join("rejoin-replaced.png").exists()
        && pilot
            .dir
            .parent()
            .is_some_and(|p| p.join("client3/network.json").exists());
    let finished = rejoin_finished
        || (pilot
            .script
            .last()
            .is_some_and(|step| step.3 == "wait_for_disconnect")
            && disconnected
            && notice_ready)
        || pilot
            .started
            .is_some_and(|s| pilot.frame - s > pilot.script.iter().map(|s| s.0).sum::<u32>() + 30);
    if finished && !*done {
        *done = true;
        if matches!(network.as_deref(), Some(network::NetworkPlay::Client(_))) {
            std::fs::write(
                pilot.dir.join("render-audit.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "fire_frames_with_particles": presentation_audit[0],
                    "max_particles_during_fire": presentation_audit[1],
                    "max_rendered_npcs": presentation_audit[2],
                    "disconnected": disconnected,
                    "disconnect_notice_ready": notice_ready,
                    "status": match network.as_deref() {
                        Some(network::NetworkPlay::Client(c)) => c.status.as_str(), _ => "",
                    },
                }))
                .unwrap(),
            )
            .expect("write complete network presentation audit");
        }
        if let Some(network) = network.as_deref() {
            let frame = match network {
                network::NetworkPlay::Host(host) => host.frame(&play.session).unwrap(),
                network::NetworkPlay::Client(client) => client.frame.clone(),
            };
            std::fs::write(
                pilot.dir.join("network.json"),
                serde_json::to_vec_pretty(&frame).unwrap(),
            )
            .expect("write network capture");
        }
        let log = format!("[\n{}\n]\n", pilot.log.join(",\n"));
        std::fs::write(pilot.dir.join("log.json"), log).expect("write autopilot log");
        commands
            .spawn(Screenshot::image(pilot.target.clone()))
            .observe(save_to_disk(pilot.dir.join("final.png")))
            .observe(
                |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                    exit.write(AppExit::Success);
                },
            );
    }
}

fn window_snapshot(
    mut commands: Commands,
    snapshot: Option<ResMut<WindowSnapshot>>,
    animators: Query<(), With<AvatarAnimator>>,
) {
    let Some(mut snapshot) = snapshot else { return };
    if animators.is_empty() {
        return;
    }
    snapshot.frames += 1;
    if snapshot.frames == snapshot.at {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(snapshot.path.clone()))
            .observe(
                |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                    exit.write(AppExit::Success);
                },
            );
    }
}

// ---------------------------------------------------------------------------
// Script-spawned NPCs: bodies, AI and animation controllers live in the
// runtime; this mirrors their transforms and plays their clip cues.

#[derive(Resource)]
struct NpcAssets {
    /// Lowercase template name -> exported glb asset path.
    files: HashMap<String, String>,
    /// Per template: glTF handle and its clip graph once loaded.
    templates: HashMap<String, NpcGraph>,
}
struct NpcGraph {
    gltf: Handle<Gltf>,
    graph: Option<(Handle<AnimationGraph>, HashMap<String, AnimationNodeIndex>)>,
}
impl NpcAssets {
    fn new(cache: &std::path::Path, _content: &std::path::Path) -> Self {
        let files = std::fs::read_dir(cache.join("gltf/Data/Characters"))
            .map(|d| {
                d.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let stem = name.strip_suffix(".glb")?.to_ascii_lowercase();
                        Some((stem, format!("gltf/Data/Characters/{name}")))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            files,
            templates: HashMap::new(),
        }
    }
}

/// Character.Color on NPCs: the robe material's TintColor.
fn tint_npcs(
    mut commands: Commands,
    play: Res<Play>,
    roots: Query<(Entity, &NpcRoot, Option<&crate::robe::RobeTint>)>,
) {
    for (entity, root, current) in &roots {
        let Some(tint) = play
            .session
            .world
            .npcs
            .get(&root.handle)
            .and_then(|n| n.tint)
        else {
            continue;
        };
        if current.is_none_or(|c| c.0 != tint) {
            commands.entity(entity).insert(crate::robe::RobeTint(tint));
        }
    }
}

#[derive(Component)]
#[require(tick_clock::TickInterpolated)]
pub(crate) struct NpcRoot {
    pub(crate) handle: u16,
    template: String,
}
#[derive(Component)]
struct NpcAnimator {
    root: Entity,
    applied: u64,
    remote_clip: Option<String>,
}

fn npc_rotation(m: &[f32; 16]) -> Quat {
    Quat::from_mat3(&Mat3::from_cols(
        Vec3::new(m[0], m[1], m[2]),
        Vec3::new(m[4], m[5], m[6]),
        Vec3::new(m[8], m[9], m[10]),
    ))
    .normalize()
}

/// Defines.PLAYERCOLORS[Player.Color] of a slot: its profile colour, else
/// the local robe colour for slot 0 and the palette for the rest.
pub(crate) fn slot_colour(session: &PlaySession, robe: [f32; 3], slot: usize) -> [f32; 3] {
    match &session.player_profiles[slot] {
        Some((_, c)) => *c,
        None if slot == 0 => robe,
        None => {
            crate::player_colors::player_color(slot % crate::player_colors::PLAYER_COLORS.len())
        }
    }
}

/// Character.Color of an NPC: SpawnCloneWizards' player colour, else the
/// template model tint. None for white, which keeps the lit material.
fn npc_tint(session: &PlaySession, robe: [f32; 3], handle: u16) -> Option<[f32; 3]> {
    let world = &session.world;
    if let Some(&slot) = world.clone_wizards.get(&handle) {
        return Some(slot_colour(session, robe, slot));
    }
    let tint = world.npcs.get(&handle)?.character.tint;
    (tint != [1.0; 3]).then_some(tint)
}

#[allow(clippy::too_many_arguments)]
fn sync_npcs(
    mut commands: Commands,
    play: Res<Play>,
    mut npc_assets: ResMut<NpcAssets>,
    assets: Res<AssetServer>,
    mut roots: Query<(
        Entity,
        &NpcRoot,
        &mut Transform,
        Option<&mut crate::robe::RobeTint>,
    )>,
    network: Option<NonSend<network::NetworkPlay>>,
    robe: Option<Res<crate::robe::RobeColour>>,
) {
    let robe = robe.map_or([1.0; 3], |r| r.0);
    use magicka_runtime::scene_world::EntityKind;
    let mut seen = std::collections::HashSet::new();
    if let Some(network::NetworkPlay::Client(client)) = network.as_deref() {
        for npc in client.npc_states() {
            let name = npc.template.to_ascii_lowercase();
            seen.insert(npc.handle);
            let transform = Transform::from_matrix(Mat4::from_cols_array(&npc.transform));
            if let Some((entity, root, mut current, _)) =
                roots.iter_mut().find(|(_, r, _, _)| r.handle == npc.handle)
            {
                if root.template == name {
                    *current = transform;
                    continue;
                }
                commands.entity(entity).despawn();
            }
            let Some(file) = npc_assets.files.get(&name).cloned() else {
                continue;
            };
            npc_assets
                .templates
                .entry(name.clone())
                .or_insert_with(|| NpcGraph {
                    gltf: assets.load(file.clone()),
                    graph: None,
                });
            commands.spawn((
                NpcRoot {
                    handle: npc.handle,
                    template: name,
                },
                transform,
                Visibility::default(),
                WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(file))),
            ));
        }
        for (entity, root, _, _) in &roots {
            if !seen.contains(&root.handle) {
                commands.entity(entity).despawn();
            }
        }
        return;
    }
    for e in &play.session.world.entities {
        if !e.added || e.removed || !matches!(e.kind, EntityKind::Npc { .. }) {
            continue;
        }
        // Overkilled (gibbed) characters: the body is gone (`sync_gibs`).
        if play.session.world.overkilled.contains(&e.handle) {
            continue;
        }
        let Some(name) = e.template_name.as_ref().map(|n| n.to_ascii_lowercase()) else {
            continue;
        };
        seen.insert(e.handle);
        let feet = Vec3::new(
            e.position[0],
            e.position[1] - e.radius - 0.5 * e.length,
            e.position[2],
        );
        let rotation = npc_rotation(&e.orientation);
        let (feet, model_scale) =
            play.session
                .world
                .npcs
                .get(&e.handle)
                .map_or((feet, 1.0), |npc| {
                    (
                        Vec3::from(play.session.world.physics.bodies[npc.body].feet()),
                        npc.character.model_scale * npc.character.graphic_scale,
                    )
                });
        let tint = npc_tint(&play.session, robe, e.handle);
        if let Some((_, _, mut t, current)) =
            roots.iter_mut().find(|(_, r, _, _)| r.handle == e.handle)
        {
            t.translation = feet;
            t.rotation = rotation;
            t.scale = Vec3::splat(model_scale);
            if let (Some(mut current), Some(tint)) = (current, tint) {
                if current.0 != tint {
                    current.0 = tint;
                }
            }
            continue;
        }
        let Some(file) = npc_assets.files.get(&name).cloned() else {
            continue;
        };
        npc_assets
            .templates
            .entry(name.clone())
            .or_insert_with(|| NpcGraph {
                gltf: assets.load(file.clone()),
                graph: None,
            });
        let mut root = commands.spawn((
            NpcRoot {
                handle: e.handle,
                template: name,
            },
            Transform::from_translation(feet)
                .with_rotation(rotation)
                .with_scale(Vec3::splat(model_scale)),
            Visibility::default(),
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(file))),
        ));
        // Tinted bodies take the skinned material (overlay × TintColor).
        if let Some(tint) = tint {
            root.insert(crate::robe::RobeTint(tint));
        }
    }
    for (entity, root, _, _) in &roots {
        if !seen.contains(&root.handle) {
            commands.entity(entity).despawn();
        }
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn animate_npcs(
    mut commands: Commands,
    play: Res<Play>,
    mut npc_assets: ResMut<NpcAssets>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    new_players: Query<
        Entity,
        (
            With<AnimationPlayer>,
            Without<NpcAnimator>,
            Without<AvatarAnimator>,
        ),
    >,
    parents: Query<&ChildOf>,
    roots: Query<&NpcRoot>,
    mut animators: Query<(
        &mut NpcAnimator,
        &mut AnimationPlayer,
        &mut AnimationTransitions,
    )>,
    network: Option<NonSend<network::NetworkPlay>>,
) {
    for clips in npc_assets.templates.values_mut() {
        if clips.graph.is_none() {
            if let Some(gltf) = gltfs.get(&clips.gltf) {
                let (names, handles): (Vec<String>, Vec<_>) = gltf
                    .named_animations
                    .iter()
                    .map(|(n, h)| (n.to_string(), h.clone()))
                    .unzip();
                let (graph, nodes) = AnimationGraph::from_clips(handles);
                clips.graph = Some((graphs.add(graph), names.into_iter().zip(nodes).collect()));
            }
        }
    }
    for entity in &new_players {
        let Some(root_entity) = parents.iter_ancestors(entity).find(|a| roots.contains(*a)) else {
            continue;
        };
        let Ok(root) = roots.get(root_entity) else {
            continue;
        };
        let Some(NpcGraph {
            graph: Some((graph, _)),
            ..
        }) = npc_assets.templates.get(&root.template)
        else {
            continue;
        };
        commands.entity(entity).insert((
            NpcAnimator {
                root: root_entity,
                applied: 0,
                remote_clip: None,
            },
            AnimationGraphHandle(graph.clone()),
            AnimationTransitions::new(),
        ));
    }
    for (mut animator, mut player, mut transitions) in &mut animators {
        let Ok(root) = roots.get(animator.root) else {
            continue;
        };
        let Some(NpcGraph {
            graph: Some((_, nodes)),
            ..
        }) = npc_assets.templates.get(&root.template)
        else {
            continue;
        };
        if let Some(network::NetworkPlay::Client(client)) = network.as_deref() {
            if let Some(state) = client.npc_states().iter().find(|n| n.handle == root.handle) {
                if let Some(clip) = &state.clip {
                    if let Some(&node) = nodes.get(clip) {
                        if animator.remote_clip.as_ref() != Some(clip) {
                            transitions.play(&mut player, node, Duration::ZERO);
                            animator.remote_clip = Some(clip.clone());
                        }
                        if let Some(active) = player.animation_mut(node) {
                            if state.looping {
                                active.repeat();
                            }
                            active.seek_to(state.clip_time).pause();
                        }
                    }
                }
            }
            continue;
        }
        if let Some((seq, cue)) = play.npc_cues.get(&root.handle) {
            if *seq != animator.applied {
                animator.applied = *seq;
                match nodes.get(&cue.clip) {
                    Some(&node) => {
                        let active = transitions.play(
                            &mut player,
                            node,
                            Duration::from_secs_f32(cue.fade.max(1e-3)),
                        );
                        if cue.looping {
                            active.repeat();
                        }
                    }
                    None => warn!("NPC clip {} missing from glTF", cue.clip),
                }
            }
        }
        // AnimationController rate = ClipSpeed × Speed.
        if let Some(npc) = play.session.world.npcs.get(&root.handle) {
            let controller = &npc.motion.controller;
            let speed = controller.clip_speed() * controller.speed;
            if let Some(node) = transitions.get_main_animation() {
                if let Some(active) = player.animation_mut(node) {
                    active.set_speed(speed);
                }
            }
        }
    }
}

/// A live missile's model (Models/Missiles/<name>.glb, exported by
/// `model-gltf --missiles`).
#[derive(Component)]
#[require(tick_clock::TickInterpolated)]
struct MissileRoot(u32);

/// MissileEntity render data: GetOrientation faces the current flight velocity.
fn sync_missiles(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    pilot: Option<Res<Autopilot>>,
    audit: Res<npc_projectile_qa::Audit>,
    mut roots: Query<(Entity, &MissileRoot, &mut Transform, &mut Visibility)>,
) {
    let missiles = &play.session.world.missiles;
    let transform = |m: &magicka_runtime::missiles::LiveMissile| {
        let [x, y, z, w] = m.body.render_orientation().to_quaternion();
        Transform {
            translation: Vec3::from(m.body.position),
            rotation: Quat::from_xyzw(x, y, z, w),
            ..default()
        }
    };
    for (entity, root, mut t, mut visibility) in &mut roots {
        *visibility = if audit.hide_model && audit.missile == Some(root.0) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if play.session.remote_missiles.iter().any(|m| m.id == root.0) {
            continue;
        }
        match missiles.iter().find(|m| m.body.id == root.0) {
            Some(m) => {
                *t = transform(m);
                if pilot
                    .as_ref()
                    .is_some_and(|p| p.script == ICE_FLIGHT_SCRIPT)
                {
                    eprintln!(
                        "missile flight audit: frame={} id={} model={} position={:?} velocity={:?} rotation={:?}",
                        pilot.as_ref().unwrap().frame,
                        m.body.id,
                        m.body.model.map(magicka_runtime::missiles::model_name).unwrap_or_default(),
                        t.translation.to_array(),
                        m.body.velocity,
                        t.rotation.to_array(),
                    );
                }
            }
            None => commands.entity(entity).despawn(),
        }
    }
    // Network clients draw the host's missiles (ids from the host's world).
    for m in &play.session.remote_missiles {
        let [x, y, z, w] =
            magicka_runtime::missiles::MissileBody::orientation_for(m.velocity).to_quaternion();
        let pose = Transform {
            translation: Vec3::from(m.position),
            rotation: Quat::from_xyzw(x, y, z, w),
            ..default()
        };
        match roots.iter_mut().find(|(_, r, _, _)| r.0 == m.id) {
            Some((_, _, mut t, _)) => *t = pose,
            None => {
                commands.spawn((
                    MissileRoot(m.id),
                    pose,
                    Visibility::default(),
                    WorldAssetRoot(assets.load(
                        GltfAssetLabel::Scene(0).from_asset(format!("gltf/{}.glb", m.model)),
                    )),
                ));
            }
        }
    }
    for m in missiles {
        if roots.iter().any(|(_, r, _, _)| r.0 == m.body.id) {
            continue;
        }
        let file = match (m.body.model, &m.body.item_model) {
            (Some(model), _) => format!(
                "gltf/Models/Missiles/{}.glb",
                magicka_runtime::missiles::model_name(model)
            ),
            // Item.mProjectileModel (exported by model-gltf --items).
            (None, Some(item)) => format!("gltf/{item}.glb"),
            (None, None) => continue,
        };
        commands.spawn((
            MissileRoot(m.body.id),
            transform(m),
            Visibility::default(),
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(file))),
        ));
    }
}

/// A live Gib's model (`<model>.glb`, exported by `model-gltf --gibs`).
#[derive(Component)]
struct GibRoot(u32);

/// Gib.RenderData: the model at the body transform with a uniform scale.
/// Frozen gibs' iceGib texture and emissive boost are not reproduced.
fn sync_decals(
    play: Res<Play>,
    pilot: Option<Res<Autopilot>>,
    mut frame: ResMut<crate::decals::DecalFrame>,
) {
    if pilot.is_some_and(|p| {
        p.started.is_some_and(|start| {
            matches!(
                script_act(p.script, p.frame - start),
                Some(Act::LayeringHidden | Act::LayeringDarkHidden)
            )
        })
    }) {
        frame.0.clear();
    } else {
        frame.0.clone_from(&play.session.world.decals.frame);
    }
}

fn sync_gibs(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut roots: Query<(Entity, &GibRoot, &mut Transform)>,
) {
    let gibs = &play.session.world.gibs.live;
    let transform = |g: &magicka_runtime::gibs::LiveGib| {
        let [x, y, z, w] = g.orientation;
        Transform {
            translation: Vec3::from(g.position),
            rotation: Quat::from_xyzw(x, y, z, w),
            scale: Vec3::splat(g.scale),
        }
    };
    for (entity, root, mut t) in &mut roots {
        match gibs.iter().find(|g| g.id == root.0) {
            Some(g) => *t = transform(g),
            None => commands.entity(entity).despawn(),
        }
    }
    for g in gibs {
        if roots.iter().any(|(_, r, _)| r.0 == g.id) {
            continue;
        }
        let file = format!("gltf/{}.glb", g.model);
        commands.spawn((
            GibRoot(g.id),
            transform(g),
            Visibility::default(),
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(file))),
        ));
    }
}

/// A live SpellMine (`Models/Effects/mine.glb`, exported by `model-gltf`):
/// pool slot plus the position bits that identify its Initialize.
#[derive(Component)]
struct MineRoot {
    slot: usize,
    key: [u32; 3],
    tint: Vec3,
    tinted: bool,
}

#[derive(Component)]
struct MineAnimator(Entity);

/// The mine glTF and its "Take 001" graph node.
#[derive(Default)]
struct MineGraph {
    gltf: Option<Handle<Gltf>>,
    graph: Option<(Handle<AnimationGraph>, AnimationNodeIndex)>,
}

const MINE_GLB: &str = "gltf/Models/Effects/mine.glb";

/// SpellMine RenderData: the model matrix Scale(2.1)·RotationY(random) at
/// the body position lowered by 0.1 (undoing Initialize's lift).
fn sync_mines(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    roots: Query<(Entity, &MineRoot)>,
) {
    let mines = &play.session.world.mines;
    let key = |m: &magicka_sim::spell_mine::SpellMine| m.position.map(f32::to_bits);
    for (entity, root) in &roots {
        if mines.get(root.slot).is_none_or(|m| key(m) != root.key) {
            commands.entity(entity).despawn();
        }
    }
    for (slot, m) in mines.iter() {
        if roots.iter().any(|(_, r)| r.slot == slot && r.key == key(m)) {
            continue;
        }
        let [x, y, z] = m.position;
        let tint = Vec3::from(magicka_sim::lightning::spell_color(&m.spell));
        commands.spawn((
            MineRoot {
                slot,
                key: key(m),
                tint,
                tinted: false,
            },
            Transform::from_xyz(x, y - 0.1, z)
                .with_rotation(Quat::from_rotation_y(m.model_yaw))
                .with_scale(Vec3::splat(magicka_sim::spell_mine::MODEL_SCALE)),
            Visibility::default(),
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(MINE_GLB))),
        ));
    }
}

/// The spawn clip sampled at the simulation's clip time (Speed 3, no loop)
/// and the mine material: SkinnedModelDeferredEffect from the model's
/// basic effect with TintColor = Spell.GetColor() (overlay tint) and
/// SpellMine.RenderData.Draw's EmissiveAmount 2 (the glow).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn animate_mines(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut mine_graph: Local<MineGraph>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    new_players: Query<Entity, (With<AnimationPlayer>, Without<MineAnimator>)>,
    parents: Query<&ChildOf>,
    mut roots: Query<(Entity, &mut MineRoot)>,
    children: Query<&Children>,
    mesh_materials: Query<(
        Option<&MeshMaterial3d<StandardMaterial>>,
        Option<&MeshMaterial3d<crate::lighting::LitMaterial>>,
        &bevy::gltf::GltfMaterialExtras,
    )>,
    (materials, lit_materials, mut robes, lights, cubes): (
        Res<Assets<StandardMaterial>>,
        Res<Assets<crate::lighting::LitMaterial>>,
        ResMut<Assets<crate::robe::RobeMaterial>>,
        Res<crate::lighting::MagickaLights>,
        Res<crate::status_look::StatusCubes>,
    ),
    mut animators: Query<(&MineAnimator, &mut AnimationPlayer)>,
) {
    if roots.is_empty() {
        return;
    }
    let gltf = mine_graph
        .gltf
        .get_or_insert_with(|| assets.load(MINE_GLB))
        .clone();
    if mine_graph.graph.is_none() {
        if let Some(clip) = gltfs
            .get(&gltf)
            .and_then(|g| g.named_animations.get("Take 001").cloned())
        {
            let (graph, node) = AnimationGraph::from_clip(clip);
            mine_graph.graph = Some((graphs.add(graph), node));
        }
    }
    let Some((graph, node)) = mine_graph.graph.clone() else {
        return;
    };
    for entity in &new_players {
        if let Some(root) = parents.iter_ancestors(entity).find(|a| roots.contains(*a)) {
            commands
                .entity(entity)
                .insert((MineAnimator(root), AnimationGraphHandle(graph.clone())));
        }
    }
    for (root_entity, mut root) in &mut roots {
        if root.tinted {
            continue;
        }
        let mut found = false;
        let t = root.tint;
        for child in children.iter_descendants(root_entity) {
            let Ok((standard, lit, extras)) = mesh_materials.get(child) else {
                continue;
            };
            let base = standard
                .and_then(|h| materials.get(&h.0))
                .or_else(|| lit.and_then(|h| lit_materials.get(&h.0)).map(|m| &m.base));
            let data = serde_json::from_str::<serde_json::Value>(&extras.value).ok();
            let (Some(base), Some(data)) = (base, data) else {
                continue;
            };
            let Some(skinned) = data.get("magicka_skinned") else {
                continue;
            };
            let Some(mut extension) = crate::robe::skinned_extension(
                skinned,
                base,
                MINE_GLB,
                &assets,
                t.to_array(),
                &lights,
                &cubes,
            ) else {
                continue;
            };
            // SpellMine.RenderData.Draw: EmissiveAmount = 2.
            extension.params.spec.z = 2.0;
            found = true;
            commands
                .entity(child)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .remove::<MeshMaterial3d<crate::lighting::LitMaterial>>()
                .insert(MeshMaterial3d(robes.add(crate::robe::RobeMaterial {
                    base: base.clone(),
                    extension,
                })));
        }
        root.tinted = found;
    }
    let mines = &play.session.world.mines;
    for (animator, mut player) in &mut animators {
        let Ok((_, root)) = roots.get(animator.0) else {
            continue;
        };
        let Some(mine) = mines.get(root.slot) else {
            continue;
        };
        if !player.is_playing_animation(node) {
            player.play(node);
        }
        if let Some(active) = player.animation_mut(node) {
            active.seek_to(mine.clip_time).pause();
        }
    }
}

/// A live Solid Barrier: pool slot plus the position bits of its Initialize.
#[derive(Component)]
struct BarrierRoot {
    slot: usize,
    key: [u32; 3],
}

/// One barrier model under a BarrierRoot: the ice or the earth part.
#[derive(Component, Clone, Copy)]
pub(crate) struct BarrierPart {
    root: Entity,
    ice: bool,
    glb: &'static str,
    clip: &'static str,
}

#[derive(Component)]
struct BarrierAnimator(Entity);

fn barrier_models(b: &magicka_sim::barrier::Barrier) -> Vec<(bool, &'static str, &'static str)> {
    use magicka_sim::barrier::{EarthModel, IceModel};
    let mut parts = Vec::new();
    if let Some(m) = b.ice_model {
        parts.push(match m {
            // Initialize picks one of five emerge clips at random; emerge0 is
            // also the death clip (INFERRED: emerge0 drawn for both).
            IceModel::Plain => (true, "gltf/Models/Effects/iceBarrier0_mesh.glb", "emerge0"),
            IceModel::Geyser => (
                true,
                "gltf/Models/Effects/icebarrier_geyser.glb",
                "Take 001",
            ),
            IceModel::Tesla => (true, "gltf/Models/Effects/icebarrier_tesla.glb", "Take 001"),
            IceModel::Earth => (true, "gltf/Models/Effects/iceBarrier_earth.glb", "Take 001"),
        });
    }
    if let Some(m) = b.earth_model {
        parts.push(match m {
            // WaveEntity: earthBarrier0's mesh and skeleton playing
            // rockpillar_animation's clip, bound by bone name.
            EarthModel::Plain if b.wave.is_some() => (
                false,
                "gltf/Models/Effects/earthBarrier0_mesh+rockpillar_animation.glb",
                "emerge0",
            ),
            EarthModel::Plain => (
                false,
                "gltf/Models/Effects/earthBarrier0_mesh.glb",
                "emerge0",
            ),
            EarthModel::Volcano => (
                false,
                "gltf/Models/Effects/earthBarrier_volcano.glb",
                "Take 001",
            ),
        });
    }
    parts
}

/// Barrier RenderData for Solid walls: the ice/earth skinned models at
/// GetOrientation (Position lowered by GetRadius(Solid) + 0.05, body
/// orientation Scale·MakeOrientationMatrix(direction)).
fn sync_barriers(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    roots: Query<(Entity, &BarrierRoot)>,
) {
    let barriers = &play.session.world.barriers;
    let key = |b: &magicka_sim::barrier::Barrier| b.position.map(f32::to_bits);
    for (entity, root) in &roots {
        if barriers.get(root.slot).is_none_or(|b| key(b) != root.key) {
            commands.entity(entity).despawn();
        }
    }
    for (slot, b) in barriers.iter() {
        if !b.solid() || roots.iter().any(|(_, r)| r.slot == slot && r.key == key(b)) {
            continue;
        }
        let [x, y, z] = b.position;
        let forward = Vec3::from(b.direction).normalize_or(Vec3::NEG_Z);
        let root = commands
            .spawn((
                BarrierRoot { slot, key: key(b) },
                Transform::from_xyz(x, y - 1.3, z)
                    .looking_to(forward, Vec3::Y)
                    .with_scale(Vec3::splat(b.scale)),
                Visibility::default(),
            ))
            .id();
        for (ice, glb, clip) in barrier_models(b) {
            let part = commands
                .spawn((
                    BarrierPart {
                        root,
                        ice,
                        glb,
                        clip,
                    },
                    Transform::default(),
                    Visibility::default(),
                    WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(glb))),
                ))
                .id();
            commands.entity(root).add_child(part);
        }
    }
}

/// Each part's clip sampled at the simulation clip time (forward appear,
/// backward death; ClipSpeed 2); ice hidden once DrawIce clears.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn animate_barriers(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut cache: Local<
        HashMap<
            &'static str,
            (
                Handle<Gltf>,
                Option<(Handle<AnimationGraph>, AnimationNodeIndex)>,
            ),
        >,
    >,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    new_players: Query<Entity, (With<AnimationPlayer>, Without<BarrierAnimator>)>,
    parents: Query<&ChildOf>,
    roots: Query<&BarrierRoot>,
    mut parts: Query<(Entity, &BarrierPart, &mut Visibility)>,
    mut animators: Query<(&BarrierAnimator, &mut AnimationPlayer)>,
) {
    if parts.is_empty() {
        return;
    }
    for (_, part, _) in &parts {
        let entry = cache
            .entry(part.glb)
            .or_insert_with(|| (assets.load(part.glb), None));
        if entry.1.is_none() {
            if let Some(clip) = gltfs
                .get(&entry.0)
                .and_then(|g| g.named_animations.get(part.clip).cloned())
            {
                let (graph, node) = AnimationGraph::from_clip(clip);
                entry.1 = Some((graphs.add(graph), node));
            }
        }
    }
    for entity in &new_players {
        let Some(part_entity) = parents.iter_ancestors(entity).find(|a| parts.contains(*a)) else {
            continue;
        };
        let Ok((_, part, _)) = parts.get(part_entity) else {
            continue;
        };
        if let Some((_, Some((graph, _)))) = cache.get(part.glb) {
            commands.entity(entity).insert((
                BarrierAnimator(part_entity),
                AnimationGraphHandle(graph.clone()),
            ));
        }
    }
    let barriers = &play.session.world.barriers;
    for (_, part, mut visibility) in &mut parts {
        let Ok(root) = roots.get(part.root) else {
            continue;
        };
        if let Some(b) = barriers.get(root.slot) {
            *visibility = if part.ice && !b.draw_ice {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        }
    }
    for (animator, mut player) in &mut animators {
        let Ok((_, part, _)) = parts.get(animator.0) else {
            continue;
        };
        let (Ok(root), Some((_, Some((_, node))))) = (roots.get(part.root), cache.get(part.glb))
        else {
            continue;
        };
        let Some(b) = barriers.get(root.slot) else {
            continue;
        };
        let clip = if part.ice { b.ice_clip } else { b.earth_clip };
        if !player.is_playing_animation(*node) {
            player.play(*node);
        }
        if let Some(active) = player.animation_mut(*node) {
            active.seek_to(clip.time).pause();
        }
    }
}

/// Autopilot log of the Tornado, Vortex, Wave and Summon Phoenix entities.
fn log_magick_models(name: &str, session: &PlaySession) {
    let w = &session.world;
    for wave in w.waves.iter().filter(|wave| !wave.removed) {
        let d = wave.effect.distortion();
        eprintln!(
            "autopilot {name}: wave at {:?} t {:.2} distortion {:.3}",
            d.center, d.time, d.strength
        );
    }
    for p in [w.portals.portal.a, w.portals.portal.b] {
        if !p.dead {
            eprintln!(
                "autopilot {name}: portal {:?} at {:?} alpha {:.2} connected {}",
                p.kind, p.position, p.effect_alpha, w.portals.portal.connected
            );
        }
    }
    let pillars: Vec<_> = w
        .barriers
        .iter()
        .filter(|(_, b)| b.wave.is_some())
        .map(|(_, b)| (b.position, b.earth_clip.time))
        .collect();
    if !pillars.is_empty() {
        eprintln!("autopilot {name}: wave pillars {pillars:?}");
    }
    for t in &w.tornadoes {
        eprintln!(
            "autopilot {name}: tornado at {:?} ttl {:.2} alpha {:.2}",
            t.tornado.position, t.tornado.ttl, t.tornado.alpha
        );
    }
    let vp = magicka_runtime::camera::multiply(&w.camera.view(), &w.camera.projection(16.0 / 9.0));
    for v in &w.vortices {
        let c = v.vortex.center;
        let p: [f32; 4] = std::array::from_fn(|i| {
            (0..4)
                .map(|j| [c[0], c[1], c[2], 1.0][j] * vp[j * 4 + i])
                .sum()
        });
        eprintln!(
            "autopilot {name}: vortex at {:?} mass {:.3} radius {:.2} screen {:.0},{:.0}",
            c,
            v.vortex.mass,
            v.vortex.radius,
            (p[0] / p[3] * 0.5 + 0.5) * 1280.0,
            (0.5 - p[1] / p[3] * 0.5) * 720.0
        );
    }
    eprintln!(
        "autopilot {name}: tornadoes {} phoenix registered {} finished {} clip {:.2} at {:?}",
        w.tornadoes.len(),
        w.phoenix.registered,
        w.phoenix.finished,
        w.phoenix.clip_time,
        w.phoenix.point
    );
}

/// Autopilot log of IceSpikes and the IceBlade (kept out of the large
/// capture logger: growing it crashes LLVM 22 in release builds).
fn log_spell_extras(name: &str, session: &PlaySession) {
    let avatar = session.avatar();
    eprintln!(
        "autopilot {name}: shield boost state {:?} count {} cooldown {} hp {:?}",
        avatar.motion.state,
        avatar.boost.count,
        avatar.boost.cooldown,
        session
            .world
            .shields
            .iter()
            .map(|s| s.shield.hit_points)
            .collect::<Vec<_>>()
    );
    eprintln!(
        "autopilot {name}: ice spikes {:?}",
        session
            .ice_spikes
            .iter()
            .map(|e| e.spikes.len())
            .collect::<Vec<_>>()
    );
    eprintln!(
        "autopilot {name}: ice blade {:?} trail span {:?} arcane blades {:?}",
        avatar.ice_blade_range(),
        avatar.ice_blade_trail().map(|t| {
            let d: Vec<f32> = (0..3).map(|k| t[30][k] - t[0][k]).collect();
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
        }),
        avatar
            .arcane_blades
            .iter()
            .map(|b| (b.range, b.dead))
            .collect::<Vec<_>>()
    );
}

/// The editor / dungeon master tool mode (`editor` feature, default) and,
/// with `--features debug-ui`, the developer pages in its menu (F1).
fn debug_ui_plugin(_app: &mut App) {
    // The editor (egui, tool mode) first: the developer menu builds on it.
    #[cfg(feature = "editor")]
    editor::plugin(_app);
    #[cfg(feature = "debug-ui")]
    debug_menu::plugin(_app);
}

/// Autopilot log: playback time of the first animated level part named
/// `name` (-1 when absent).
fn level_part_time(session: &PlaySession, name: &str) -> f32 {
    fn find(
        parts: &[magicka_runtime::animated_hierarchy::AnimatedPartState],
        name: &str,
    ) -> Option<f32> {
        parts.iter().find_map(|p| {
            if p.name.eq_ignore_ascii_case(name) {
                Some(p.playback.time)
            } else {
                find(&p.children, name)
            }
        })
    }
    find(session.world.collision.parts(), name).unwrap_or(-1.0)
}

/// Developer tooling: place the avatar at a trigger area's centre.
fn teleport_to_area(session: &mut PlaySession, area: &str) -> bool {
    let Some(center) = session.world.area_center(area) else {
        return false;
    };
    let index = session.avatar().body;
    let body = &mut session.world.physics.bodies[index];
    let orientation = body.orientation;
    body.move_to(center, orientation);
    true
}

/// Flush authored startup actions before an inspection trigger addresses the
/// entities they spawn. A zero-time step does not advance the scene's timers.
fn run_inspection_trigger(session: &mut PlaySession, trigger: &str) -> Result<(), String> {
    session.step(0.0);
    session
        .run_trigger(trigger, true)
        .map(|_| ())
        .map_err(|e| format!("{trigger}: {e}"))
}

fn reload_session(
    session: &mut PlaySession,
    level: &str,
) -> Result<(), magicka_runtime::play_session::SessionError> {
    if session.level_file() == level {
        session.restart_level()
    } else {
        session.change_level(level)
    }
}

fn reload(play: &mut Play) {
    let teams = play.session.world.player_teams.clone();
    match reload_session(&mut play.session, &play.level) {
        Ok(()) => {
            let session = &mut play.session;
            #[cfg(feature = "debug-ui")]
            if let Some((robe, staff, weapon)) = &play.debug_loadout {
                if let Err(e) = session.debug_player_loadout(robe, staff, weapon) {
                    error!("loadout restore failed: {e:?}");
                    return;
                }
            }
            if !play.scripts {
                session.disable_scripts();
            }
            if play.demo_goblin {
                for _ in 0..30 {
                    session.step(1.0 / 60.0);
                }
                let _ = session.spawn_npc("Goblin_pirate", "attack", 7.0);
            }
            if let Some(v) = play.versus {
                if let Err(e) = session.set_versus(v, teams) {
                    error!("versus restore failed: {e:?}");
                }
            }
            if let Some(p) = &play.packs {
                session.set_survival_packs(p.clone());
            }
            play.reloads += 1;
        }
        Err(e) => error!("restart failed: {e:?}"),
    }
}

/// PlayState RenderData's end-game text (ZIndex 999): Stonecross50,
/// centred at (w/2, (h − lineHeight)/2), white with PostTextAlpha.
#[derive(Component)]
struct EndGameText(
    Handle<crate::native_text::OverlayFontMaterial>,
    &'static str,
);

fn setup_end_text(
    mut commands: Commands,
    play: Res<Play>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::native_text::OverlayFontMaterial>>,
) {
    use magicka_content::assets::Asset;
    use magicka_runtime::text_geometry::{TextAlign, TextGeometry};
    let result = (|| -> Result<(), String> {
        let path = play.content.join("Languages/eng/Font/Stonecross50.xnb");
        let Asset::BitmapFont(font) = crate::text_box_visual::read_asset(&path)? else {
            return Err("expected Stonecross50".into());
        };
        let strings = magicka_runtime::play_content::english_strings(&play.content, &["Wizard"]);
        use magicka_runtime::end_game::{DEFEATED_TEXT, LEVEL_COMPLETE_TEXT, VICTORY_TEXT};
        for key in [DEFEATED_TEXT, LEVEL_COMPLETE_TEXT, VICTORY_TEXT] {
            let (geometry_font, material) =
                crate::native_text::upload_overlay_font(&font, &mut images, &mut materials)?;
            let text = strings
                .get_string(magicka_runtime::scene_loading::custom_hash(key))
                .ok_or(format!("missing {key}"))?;
            let utf16: Vec<u16> = text.encode_utf16().collect();
            let geometry = TextGeometry::build(
                &geometry_font,
                &utf16,
                TextStyle {
                    align: TextAlign::Center,
                    ..default()
                },
            )
            .map_err(|e| format!("{e:?}"))?;
            let mesh = crate::native_text::geometry_mesh(&geometry, geometry.quads.len() as i32)?;
            let line_height = geometry_font.metrics.line_height as f32;
            commands.spawn((
                EndGameText(material.clone(), key),
                Mesh2d(meshes.add(mesh)),
                MeshMaterial2d(material),
                Transform::from_xyz(0.0, line_height * 0.5, 999.0),
                Visibility::Hidden,
            ));
        }
        Ok(())
    })();
    if let Err(e) = result {
        warn!("end-game text unavailable: {e}");
    }
}

fn update_end_text(
    play: Res<Play>,
    mut texts: Query<(&EndGameText, &mut Visibility)>,
    mut materials: ResMut<Assets<crate::native_text::OverlayFontMaterial>>,
) {
    let alpha = play.session.end_game.text_alpha;
    let shown = play.session.end_game.text();
    for (text, mut visibility) in &mut texts {
        *visibility = if alpha > f32::from_bits(1) && shown == Some(text.1) {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some(mut m) = materials.get_mut(&text.0) {
            m.color = Vec4::new(1.0, 1.0, 1.0, alpha);
        }
    }
}

/// Rebuild level-bound presentation after Level.ChangeScene or a reload:
/// the level glTF (and terrain texture root), NPC and missile roots, whose
/// entity handles are per scene world.
#[allow(clippy::too_many_arguments)]
fn sync_level(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut seen: Local<Option<(u32, u32)>>,
    mut shown_model: Local<Option<std::path::PathBuf>>,
    levels: Query<Entity, With<LevelRoot>>,
    npcs: Query<Entity, With<NpcRoot>>,
    missiles: Query<Entity, With<MissileRoot>>,
    mut scene_path: ResMut<ScenePath>,
    mut avatar: ResMut<AvatarAssets>,
    mut fades: Query<(&FadeOverlay, &mut Visibility)>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut surface_materials: ResMut<Assets<StandardMaterial>>,
) {
    let fade = play.session.transition_fade();
    let [r, g, b, _] = play
        .session
        .transition_color()
        .map(|c| f32::from(c) / 255.0);
    for (overlay, mut visibility) in &mut fades {
        *visibility = if fade > 0.0 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some(mut m) = materials.get_mut(&overlay.0) {
            m.color = Color::srgba(r, g, b, fade);
        }
    }
    let model = play.session.model_path.clone();
    if shown_model.is_none() {
        // The setup system spawned the start scene.
        *shown_model = Some(model.clone());
    }
    let key = (play.reloads, play.session.scene_changes);
    let previous = seen.replace(key);
    // A network client can mirror the host's scene before this first runs,
    // so the counters alone miss it: also reload while the shown glb (set by
    // setup, then here) is not the current scene's.
    let stale_mesh = !authored_level::is_authored(&play)
        && !play.hide_level
        && scene_path.0 != level_glb_path(&model);
    if !stale_mesh && previous.is_none_or(|p| p == key) {
        return;
    }
    for e in npcs.iter().chain(missiles.iter()) {
        commands.entity(e).despawn();
    }
    if !authored_level::is_authored(&play)
        && !stale_mesh
        && shown_model.as_ref() == Some(&model)
        && play.session.scene_changes == 0
        && previous.is_some_and(|p| p.1 == 0)
    {
        // Reload of the same start scene: the level glTF stays.
        return;
    }
    *shown_model = Some(model);
    for e in &levels {
        commands.entity(e).despawn();
    }
    if play.hide_level {
        return;
    }
    if authored_level::is_authored(&play) {
        authored_level::spawn(
            &mut commands,
            &mut meshes,
            &mut surface_materials,
            &play,
            &assets,
        );
        scene_path.0 = play.session.model_path.to_string_lossy().into_owned();
        avatar.level = None;
        return;
    }
    let path = level_glb_path(&play.session.model_path);
    info!("scene change: {} ({path})", play.session.scene_name());
    scene_path.0 = path.clone();
    let scene: Handle<WorldAsset> = assets.load(GltfAssetLabel::Scene(0).from_asset(path));
    commands.spawn((
        LevelRoot,
        crate::lighting::OutlineOccluder,
        WorldAssetRoot(scene.clone()),
    ));
    avatar.level = Some(scene);
}

/// The converted glTF of a level model, relative to the asset root.
fn level_glb_path(model: &std::path::Path) -> String {
    format!("gltf/{}.glb", model.to_string_lossy().replace('\\', "/"))
}

fn setup_fade(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let material = materials.add(ColorMaterial {
        color: Color::srgba(0.0, 0.0, 0.0, 0.0),
        alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
        ..default()
    });
    commands.spawn((
        FadeOverlay(material.clone()),
        Mesh2d(meshes.add(Rectangle::new(16384.0, 16384.0))),
        MeshMaterial2d(material),
        Transform::from_xyz(0.0, 0.0, 998.0),
        Visibility::Hidden,
    ));
}

/// CutsceneRenderData (ZIndex 990): two black bars sliding in by
/// `CUTSCENE_BLACKBAR_SIZE · Time` and the right-aligned Maiandra18
/// `#cut_press_skip` tip at (w − 20, h − 0.05h − lineHeight/2).
#[derive(Component)]
enum CutsceneOverlay {
    Bar {
        bottom: bool,
    },
    Tip {
        material: Handle<crate::native_text::OverlayFontMaterial>,
        line_height: f32,
    },
}

fn setup_cutscene_overlay(
    mut commands: Commands,
    play: Res<Play>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut fonts: ResMut<Assets<crate::native_text::OverlayFontMaterial>>,
) {
    let black = materials.add(ColorMaterial::from(Color::BLACK));
    for bottom in [false, true] {
        commands.spawn((
            CutsceneOverlay::Bar { bottom },
            Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
            MeshMaterial2d(black.clone()),
            Transform::from_xyz(0.0, 0.0, 990.0),
            Visibility::Hidden,
        ));
    }
    use magicka_content::assets::Asset;
    use magicka_runtime::text_geometry::{TextAlign, TextGeometry};
    let result = (|| -> Result<(), String> {
        let path = play.content.join("Languages/eng/Font/Maiandra18.xnb");
        let Asset::BitmapFont(font) = crate::text_box_visual::read_asset(&path)? else {
            return Err("expected Maiandra18".into());
        };
        let (geometry_font, material) =
            crate::native_text::upload_overlay_font(&font, &mut images, &mut fonts)?;
        let strings = magicka_runtime::play_content::english_strings(&play.content, &["Wizard"]);
        let text = strings
            .get_string(magicka_runtime::scene_loading::custom_hash(
                magicka_runtime::cutscene::SKIP_TIP_TEXT,
            ))
            .ok_or("missing #cut_press_skip")?;
        // Keyboard/mouse: ButtonChar.SpaceBar (810).
        let text = text.replace("#1;", "\u{32a}");
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let geometry = TextGeometry::build(
            &geometry_font,
            &utf16,
            TextStyle {
                align: TextAlign::Right,
                ..default()
            },
        )
        .map_err(|e| format!("{e:?}"))?;
        let mesh = crate::native_text::geometry_mesh(&geometry, geometry.quads.len() as i32)?;
        commands.spawn((
            CutsceneOverlay::Tip {
                material: material.clone(),
                line_height: geometry_font.metrics.line_height as f32,
            },
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(material),
            Transform::from_xyz(0.0, 0.0, 991.0),
            Visibility::Hidden,
        ));
        Ok(())
    })();
    if let Err(e) = result {
        warn!("cutscene skip tip unavailable: {e}");
    }
}

fn update_cutscene_overlay(
    play: Res<Play>,
    mut overlays: Query<(&CutsceneOverlay, &mut Transform, &mut Visibility)>,
    mut materials: ResMut<Assets<crate::native_text::OverlayFontMaterial>>,
) {
    let [w, h] = play.screen;
    let cutscene = &play.session.world.cutscene;
    let bars = cutscene.bars(h);
    for (overlay, mut transform, mut visibility) in &mut overlays {
        let Some((top_height, bottom_top)) = bars else {
            *visibility = Visibility::Hidden;
            continue;
        };
        *visibility = Visibility::Visible;
        let bar = h * magicka_runtime::cutscene::BLACKBAR_SIZE;
        // Screen (top-left, y down) to overlay camera (centre, y up).
        match overlay {
            CutsceneOverlay::Bar { bottom } => {
                let top_edge = if *bottom {
                    bottom_top
                } else {
                    top_height - bar
                };
                transform.scale = Vec3::new(w, bar, 1.0);
                transform.translation.x = 0.0;
                transform.translation.y = h * 0.5 - (top_edge + bar * 0.5);
            }
            CutsceneOverlay::Tip {
                material,
                line_height,
            } => {
                transform.translation.x = w * 0.5 - 20.0;
                transform.translation.y = h * 0.5 - (h - bar * 0.5 - line_height * 0.5);
                if let Some(mut m) = materials.get_mut(material) {
                    m.color = Vec4::new(1.0, 1.0, 1.0, cutscene.skip_tip_alpha);
                }
            }
        }
    }
}

/// LightningBolt render data: one additive ribbon per live bolt, built at
/// InitializeEffect from the camera position then, tinted `2·color` with
/// alpha `sqrt(10·mTTL)`, over EffectTextures/Beams (wrapping U).
#[derive(Component)]
struct BoltDraw {
    serial: u32,
    material: Handle<crate::additive::AdditiveMaterial>,
}

#[derive(Resource)]
struct BoltPool {
    strips: Vec<[magicka_runtime::bolt_mesh::StripVertex; 34]>,
    texture: Handle<Image>,
}

/// LightningBolt.InitializeCache: pooled bolts, each with its own jittered
/// strip (MagickaMath.Random is time-seeded; a fixed seed stands in).
fn setup_bolt_pool(mut commands: Commands, assets: Res<AssetServer>) {
    use bevy::image::{
        ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor,
    };
    let mut rng = magicka_runtime::particles::DotNetRandom::new(0);
    let strips = (0..16)
        .map(|_| magicka_runtime::bolt_mesh::strip(&mut rng))
        .collect();
    let texture = assets
        .load_builder()
        .with_settings(|s: &mut ImageLoaderSettings| {
            s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                ..ImageSamplerDescriptor::linear()
            });
        })
        .load("textures/EffectTextures/Beams.png");
    commands.insert_resource(BoltPool { strips, texture });
}

fn sync_bolts(
    mut commands: Commands,
    play: Res<Play>,
    pool: Res<BoltPool>,
    cameras: Query<&GlobalTransform, With<PlayCamera>>,
    draws: Query<(Entity, &BoltDraw)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::additive::AdditiveMaterial>>,
) {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let bolts = &play.session.bolts;
    for (entity, draw) in &draws {
        match bolts.iter().find(|b| b.serial == draw.serial) {
            Some(b) => {
                if let Some(mut m) = materials.get_mut(&draw.material) {
                    m.color.w = (b.ttl * 10.0).sqrt();
                }
            }
            None => commands.entity(entity).despawn(),
        }
    }
    let eye = cameras
        .iter()
        .next()
        .map_or([0.0, 20.0, 0.0], |t| t.translation().to_array());
    for b in bolts {
        if draws.iter().any(|(_, d)| d.serial == b.serial) {
            continue;
        }
        let v = b.visual;
        let strip = &pool.strips[b.serial as usize % pool.strips.len()];
        let tri = magicka_runtime::bolt_mesh::triangles(strip, v.from, v.to, eye, v.scale);
        if tri.is_empty() {
            continue;
        }
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            tri.iter().map(|t| t.position).collect::<Vec<_>>(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; tri.len()]);
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            tri.iter().map(|t| t.uv).collect::<Vec<_>>(),
        );
        mesh.insert_indices(Indices::U32((0..tri.len() as u32).collect()));
        let material = materials.add(crate::additive::AdditiveMaterial {
            color: Vec4::new(
                v.color[0] * 2.0,
                v.color[1] * 2.0,
                v.color[2] * 2.0,
                (b.ttl * 10.0).sqrt(),
            ),
            flags: UVec4::new(1, 0, 0, 0),
            texture: Some(pool.texture.clone()),
        });
        commands.spawn((
            BoltDraw {
                serial: b.serial,
                material: material.clone(),
            },
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            bevy::light::NotShadowCaster,
        ));
    }
}

/// Railgun RenderData draws (one mesh per ArcaneEffect draw call), reused
/// across frames and hidden when unused.
/// Spell shield draws by entity handle: (entity, material).
#[derive(Resource, Default)]
struct ShieldDraws {
    draws: HashMap<
        u16,
        (
            Entity,
            Handle<crate::additive::AdditiveMaterial>,
            Handle<Mesh>,
        ),
    >,
}

/// Unit hollow-sphere cap around local −Z (Bevy forward) up to `max_angle`,
/// both windings (seen from inside and outside).
fn shield_mesh(max_angle: f32) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let (rings, segments) = (16u32, 48u32);
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for r in 0..=rings {
        let theta = max_angle * r as f32 / rings as f32;
        for k in 0..=segments {
            let phi = std::f32::consts::TAU * k as f32 / segments as f32;
            let p = [
                theta.sin() * phi.cos(),
                theta.sin() * phi.sin(),
                -theta.cos(),
            ];
            positions.push(p);
            normals.push(p);
            uvs.push([k as f32 / segments as f32, r as f32 / rings as f32]);
        }
    }
    let mut indices = Vec::new();
    let row = segments + 1;
    for r in 0..rings {
        for k in 0..segments {
            let (a, b, c, d) = (
                r * row + k,
                r * row + k + 1,
                (r + 1) * row + k,
                (r + 1) * row + k + 1,
            );
            indices.extend([a, c, b, b, c, d, a, b, c, b, d, c]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Spell shields (Shield entities) as additive shells tinted by the shield
/// colour and fade (mTint). INFERRED stand-in for ShieldEffect, whose
/// noise/refraction shader is not ported.
#[allow(clippy::too_many_arguments)]
fn sync_shields(
    mut commands: Commands,
    play: Res<Play>,
    pilot: Option<Res<Autopilot>>,
    mut draws: ResMut<ShieldDraws>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::additive::AdditiveMaterial>>,
    mut transforms: Query<(&mut Transform, &mut Visibility)>,
    cameras: Query<&GlobalTransform, With<PlayCamera>>,
) {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    use magicka_sim::shield_entity::ShieldKind;
    let hide_wall = pilot.as_ref().is_some_and(|p| {
        p.started.is_some_and(|start| {
            script_act(p.script, p.frame - start) == Some(Act::ShieldWallHidden)
        })
    });
    let view_dir = cameras
        .iter()
        .next()
        .map_or(Vec3::NEG_Y, |t| t.forward().as_vec3());
    let live: std::collections::HashSet<u16> = play
        .session
        .world
        .shields
        .iter()
        .map(|s| s.handle)
        .collect();
    draws.draws.retain(|handle, (entity, _, _)| {
        let keep = live.contains(handle);
        if !keep {
            commands.entity(*entity).despawn();
        }
        keep
    });
    for s in &play.session.world.shields {
        let shield = &s.shield;
        let alpha = shield.tint[3].clamp(0.0, 1.0) * 0.12;
        let color = Vec4::new(
            shield.tint[0] * alpha,
            shield.tint[1] * alpha,
            shield.tint[2] * alpha,
            1.0,
        );
        let forward = Vec3::from(shield.forward).normalize_or(Vec3::NEG_Z);
        let transform = Transform::from_translation(Vec3::from(shield.position))
            .looking_to(forward, Vec3::Y)
            .with_scale(Vec3::splat(if shield.kind == ShieldKind::Wall {
                1.0
            } else {
                shield.radius
            }));
        if let std::collections::hash_map::Entry::Vacant(slot) = draws.draws.entry(s.handle) {
            let material = materials.add(crate::additive::AdditiveMaterial {
                color,
                // Vertex colours carry the view-dependent rim.
                flags: UVec4::new(0, 1, 0, 0),
                texture: None,
            });
            let max_angle = if shield.kind == ShieldKind::Disc {
                std::f32::consts::FRAC_PI_4
            } else {
                std::f32::consts::PI
            };
            let mesh = if shield.kind == ShieldKind::Wall {
                let wall = magicka_sim::shield_wall::mesh();
                let mut mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                );
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, wall.positions);
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, wall.normals);
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, wall.uv);
                mesh.insert_indices(Indices::U32(wall.indices));
                meshes.add(mesh)
            } else {
                meshes.add(shield_mesh(max_angle))
            };
            let entity = commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    transform,
                    Visibility::Visible,
                    bevy::light::NotShadowCaster,
                ))
                .id();
            slot.insert((entity, material, mesh));
        }
        let (entity, material, mesh) = draws.draws[&s.handle].clone();
        if let Some(mut m) = materials.get_mut(&material) {
            m.color = color;
        }
        if let Ok((mut t, mut visibility)) = transforms.get_mut(entity) {
            *t = transform;
            *visibility = if shield.kind == ShieldKind::Wall && hide_wall {
                Visibility::Hidden
            } else {
                Visibility::Visible
            };
        }
        // Rim: brighter where the shell is seen edge-on.
        if let Some(mut m) = meshes.get_mut(&mesh) {
            let rim: Option<Vec<[f32; 4]>> = m
                .attribute(Mesh::ATTRIBUTE_NORMAL)
                .and_then(|a| a.as_float3())
                .map(|normals| {
                    normals
                        .iter()
                        .map(|n| {
                            let world = transform.rotation * Vec3::from(*n);
                            let f = 1.0 - world.dot(view_dir).abs();
                            let k = 0.3 + 2.5 * f * f * f;
                            [k, k, k, 1.0]
                        })
                        .collect()
                });
            if let Some(rim) = rim {
                m.insert_attribute(Mesh::ATTRIBUTE_COLOR, rim);
            }
        }
    }
}

/// One pooled AdditiveEffect mesh per rail with additional elements.
#[derive(Resource, Default)]
struct AdditionalBeamPool {
    draws: Vec<(Entity, Handle<Mesh>)>,
    material: Option<Handle<crate::additive::AdditiveMaterial>>,
}

/// Railgun AdditionalRenderData: per additional element two textured
/// billboards over EffectTextures/Beams, tinted ×2 (Lightning: 2·colour)
/// through AdditiveEffect (`beam_mesh::additional_quads`). Both windings are
/// emitted so the camera-facing quad survives back-face culling.
#[allow(clippy::too_many_arguments)]
fn sync_additional_beams(
    mut commands: Commands,
    play: Res<Play>,
    bolts: Res<BoltPool>,
    mut pool: ResMut<AdditionalBeamPool>,
    cameras: Query<&GlobalTransform, With<PlayCamera>>,
    mut visibility: Query<&mut Visibility>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::additive::AdditiveMaterial>>,
) {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let (eye, forward) = cameras
        .iter()
        .next()
        .map_or(([0.0, 20.0, 0.0], [0.0, -1.0, 0.0]), |t| {
            (t.translation().to_array(), t.forward().to_array())
        });
    let material = pool
        .material
        .get_or_insert_with(|| {
            materials.add(crate::additive::AdditiveMaterial {
                color: Vec4::ONE,
                // TextureEnabled, tint carried in vertex colours.
                flags: UVec4::new(1, 1, 0, 0),
                texture: Some(bolts.texture.clone()),
            })
        })
        .clone();
    let rails: Vec<_> = play
        .session
        .beams
        .iter()
        .filter(|r| r.additional != magicka_sim::spells::Elements::NONE)
        .map(|r| magicka_runtime::beam_mesh::additional_quads(r, eye, forward))
        .collect();
    for (i, quads) in rails.iter().enumerate() {
        let mut positions = Vec::new();
        let mut uvs = Vec::new();
        let mut colors = Vec::new();
        let mut indices = Vec::new();
        for q in quads {
            let base = positions.len() as u32;
            positions.extend(q.positions);
            uvs.extend(q.uvs);
            colors.extend([[q.tint[0], q.tint[1], q.tint[2], 1.0]; 4]);
            // TriangleFan (0,1,2),(0,2,3), both windings.
            indices.extend([0, 1, 2, 0, 2, 3, 0, 2, 1, 0, 3, 2].map(|k| base + k));
        }
        let normals = vec![[0.0f32, 1.0, 0.0]; positions.len()];
        if i == pool.draws.len() {
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
            );
            mesh.insert_indices(Indices::U32(indices));
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
            let mesh = meshes.add(mesh);
            let entity = commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::IDENTITY,
                    Visibility::Visible,
                    bevy::light::NotShadowCaster,
                    bevy::camera::visibility::NoFrustumCulling,
                ))
                .id();
            pool.draws.push((entity, mesh));
            continue;
        }
        let (entity, mesh) = pool.draws[i].clone();
        if let Some(mut m) = meshes.get_mut(&mesh) {
            m.insert_indices(Indices::U32(indices));
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        }
        if let Ok(mut v) = visibility.get_mut(entity) {
            *v = Visibility::Visible;
        }
    }
    for (entity, _) in pool.draws.iter().skip(rails.len()) {
        if let Ok(mut v) = visibility.get_mut(*entity) {
            *v = Visibility::Hidden;
        }
    }
}

/// ArcaneBlast RenderData: one additive mesh holding every live blast's
/// discs (EffectTextures/ArcaneDisc rows, tint alpha in vertex colours).
#[derive(Resource, Default)]
struct ArcaneBlastDraw {
    entity: Option<(Entity, Handle<Mesh>)>,
}

fn sync_arcane_blasts(
    mut commands: Commands,
    play: Res<Play>,
    assets: Res<AssetServer>,
    mut draw: ResMut<ArcaneBlastDraw>,
    mut visibility: Query<&mut Visibility>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::additive::AdditiveMaterial>>,
) {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    for blast in &play.session.arcane_blasts {
        for disc in blast.draws() {
            let base = positions.len() as u32;
            positions.extend(disc.positions);
            uvs.extend(disc.uvs);
            colors.extend([[1.0, 1.0, 1.0, disc.alpha]; 4]);
            // TriangleFan (0,1,2),(0,2,3); CullMode None, so both windings.
            indices.extend([0, 1, 2, 0, 2, 3, 0, 2, 1, 0, 3, 2].map(|k| base + k));
        }
    }
    let visible = !positions.is_empty();
    if !visible {
        if let Some((entity, _)) = &draw.entity {
            if let Ok(mut v) = visibility.get_mut(*entity) {
                *v = Visibility::Hidden;
            }
        }
        return;
    }
    let normals = vec![[0.0f32, 1.0, 0.0]; positions.len()];
    let fill = |m: &mut Mesh| {
        m.insert_indices(Indices::U32(indices.clone()));
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.clone());
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals.clone());
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.clone());
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors.clone());
    };
    match &draw.entity {
        Some((entity, mesh)) => {
            if let Some(mut m) = meshes.get_mut(mesh) {
                fill(&mut m);
            }
            if let Ok(mut v) = visibility.get_mut(*entity) {
                *v = Visibility::Visible;
            }
        }
        None => {
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
            );
            fill(&mut mesh);
            let mesh = meshes.add(mesh);
            let material = materials.add(crate::additive::AdditiveMaterial {
                // ColorTint (1, 1, 1, 0.5·Alpha) is carried per vertex.
                color: Vec4::ONE,
                flags: UVec4::new(1, 1, 0, 0),
                texture: Some(assets.load("textures/EffectTextures/ArcaneDisc.png")),
            });
            let entity = commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material),
                    Transform::IDENTITY,
                    Visibility::Visible,
                    bevy::light::NotShadowCaster,
                    bevy::camera::visibility::NoFrustumCulling,
                ))
                .id();
            draw.entity = Some((entity, mesh));
        }
    }
}

#[derive(Resource, Default)]
struct BeamPool {
    draws: Vec<(Entity, Handle<Mesh>, Handle<crate::beam::BeamMaterial>)>,
}

/// Railgun.Update's RenderData → ArcaneEffect Beam draws for this frame's
/// camera. Rails carrying additional elements use AdditionalRenderData
/// instead (`sync_additional_beams`).
#[allow(clippy::too_many_arguments)]
fn sync_beams(
    mut commands: Commands,
    play: Res<Play>,
    bolts: Res<BoltPool>,
    mut pool: ResMut<BeamPool>,
    cameras: Query<&GlobalTransform, With<PlayCamera>>,
    mut visibility: Query<&mut Visibility>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::beam::BeamMaterial>>,
) {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    use magicka_runtime::beam_mesh;
    let eye = cameras
        .iter()
        .next()
        .map_or([0.0, 20.0, 0.0], |t| t.translation().to_array());
    let draws: Vec<_> = play
        .session
        .beams
        .iter()
        .filter(|r| r.additional == magicka_sim::spells::Elements::NONE)
        .flat_map(beam_mesh::rail_draws)
        .collect();
    for (i, d) in draws.iter().enumerate() {
        let vertices = beam_mesh::strip(d, eye);
        let positions: Vec<[f32; 3]> = vertices.iter().map(|v| v.position).collect();
        let uvs: Vec<[f32; 2]> = vertices.iter().map(|v| v.uv).collect();
        let colors: Vec<[f32; 4]> = vertices
            .iter()
            .map(|v| [v.alpha, v.alpha, v.alpha, 1.0])
            .collect();
        let center = Vec4::new(d.color_center[0], d.color_center[1], d.color_center[2], 1.0);
        let edge = Vec4::new(d.color_edge[0], d.color_edge[1], d.color_edge[2], 1.0);
        if i == pool.draws.len() {
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
            );
            mesh.insert_indices(Indices::U32(beam_mesh::strip_triangles()));
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_NORMAL,
                vec![[0.0f32, 1.0, 0.0]; beam_mesh::VERTEX_COUNT],
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.clone());
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.clone());
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors.clone());
            let mesh = meshes.add(mesh);
            let material = materials.add(crate::beam::BeamMaterial {
                color_center: center,
                color_edge: edge,
                texture: bolts.texture.clone(),
            });
            let entity = commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::IDENTITY,
                    Visibility::Visible,
                    bevy::light::NotShadowCaster,
                    // CPU-built world-space strip; skip frustum culling on a
                    // stale bounding box.
                    bevy::camera::visibility::NoFrustumCulling,
                ))
                .id();
            pool.draws.push((entity, mesh, material));
            continue;
        }
        let (entity, mesh, material) = pool.draws[i].clone();
        if let Some(mut m) = meshes.get_mut(&mesh) {
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        }
        if let Some(mut m) = materials.get_mut(&material) {
            m.color_center = center;
            m.color_edge = edge;
        }
        if let Ok(mut v) = visibility.get_mut(entity) {
            *v = Visibility::Visible;
        }
    }
    for (entity, _, _) in pool.draws.iter().skip(draws.len()) {
        if let Ok(mut v) = visibility.get_mut(*entity) {
            *v = Visibility::Hidden;
        }
    }
}

fn controller_context(
    play: Option<Res<Play>>,
    menu: Option<Res<menu::PauseMenu>>,
    network: Option<NonSend<network::NetworkPlay>>,
    loading: Option<Res<frontend_loading::Loading>>,
    mut controller: ResMut<crate::controller::ControllerInput>,
) {
    if !frontend_loading::ready(loading) {
        return;
    }
    if let Some(play) = play {
        controller.update_context(
            &play.session,
            play.screen,
            menu.as_ref().is_some_and(|m| m.visible),
            menu.as_ref().is_some_and(|m| m.show_controller_controls()),
        );
        controller.player_slot = match network.as_deref() {
            Some(network::NetworkPlay::Client(c)) => c.slot(),
            _ => 0,
        };
    }
}

#[cfg(test)]
mod level_reload_tests {
    use super::*;

    #[test]
    fn debug_level_selection_changes_the_session_and_restart_stays_there() {
        let content =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
        if !content.is_dir() {
            eprintln!("retail content absent; skipping");
            return;
        }
        let mut session = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
        session.join_player_at_start(1, "Wizard").unwrap();
        session.player_profiles[1] = Some(("Second wizard".into(), [0.2, 0.4, 0.6]));
        reload_session(&mut session, "Mountaindale.lvl").unwrap();
        assert_eq!(session.level_file(), "Mountaindale.lvl");
        assert_eq!(session.scene_name(), "md_s1");
        assert!(session.world.players[1].as_ref().unwrap().playing);
        assert_eq!(
            session.player_profiles[1].as_ref().unwrap().0,
            "Second wizard"
        );
        session.change_scene_named("md_s2", "start").unwrap();
        reload_session(&mut session, "Mountaindale.lvl").unwrap();
        assert_eq!(session.level_file(), "Mountaindale.lvl");
        assert_eq!(session.scene_name(), "md_s1");
        assert!(reload_session(&mut session, "missing_debug_level.lvl").is_err());
        assert_eq!(session.level_file(), "Mountaindale.lvl");
    }

    #[test]
    fn inspection_boss_trigger_sees_the_scene_startup_spawn() {
        let content =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
        if !content.is_dir() {
            eprintln!("retail content absent; skipping");
            return;
        }
        let mut session = PlaySession::load(&content, "EndofWorld.lvl", "Wizard").unwrap();
        session.change_scene_named("ew_s5", "endspawn").unwrap();
        run_inspection_trigger(&mut session, "Start Grimnir Fight").unwrap();
        assert!(session.world.boss_fight.bosses.iter().any(|b| matches!(
            b,
            magicka_runtime::scene_world::boss_live::BossRef::Grimnir2(_)
        )));
        // The authored activation is delayed by three seconds.
        for _ in 0..190 {
            session.step(FIXED_DT);
        }
        assert!(session.world.boss_fight.running);
    }
}

#[cfg(test)]
mod npc_tint_tests {
    use super::*;

    /// Template model tints colour NPC robes; white ones keep the lit
    /// material; clone wizards take their player's colour.
    #[test]
    fn npc_tints_follow_the_model_or_the_cloned_player() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let Ok(content) = root
            .join("../../../../target/Magicka/Content")
            .canonicalize()
        else {
            eprintln!("retail content absent; skipping");
            return;
        };
        let level = root
            .join("../../scenarios/feature-lab.lvl")
            .canonicalize()
            .unwrap();
        let mut session = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
        session.disable_scripts();
        session.step(1. / 60.);
        let support = session.spawn_npc("Wizard_Support", "idle", 3.0).unwrap();
        let vlad = session.spawn_npc("Vlad", "idle", 5.0).unwrap();
        let robe = [0.2, 0.4, 0.6];
        let tint = session.world.npcs[&support].character.tint;
        assert_ne!(tint, [1.0; 3]);
        assert_eq!(npc_tint(&session, robe, support), Some(tint));
        assert_eq!(npc_tint(&session, robe, vlad), None);
        session.world.clone_wizards.insert(support, 0);
        assert_eq!(npc_tint(&session, robe, support), Some(robe));
        session.player_profiles[0] = Some(("P".into(), [1.0, 0.0, 0.0]));
        assert_eq!(npc_tint(&session, robe, support), Some([1.0, 0.0, 0.0]));
        session.world.clone_wizards.insert(vlad, 2);
        assert_eq!(
            npc_tint(&session, robe, vlad),
            Some(crate::player_colors::player_color(2))
        );
    }
}
