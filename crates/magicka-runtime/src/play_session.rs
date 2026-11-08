//! Live single-player session: the start scene's SceneWorld (live collision,
//! character physics, camera, player avatar) driven by its ScriptRuntime in
//! PlayState.Update order. See research/play-session.md.
//!
//! Script actions without a reconstruction are skipped and reported
//! (`SessionEvent::ScriptSkipped`) instead of stopping the scene.
use crate::{
    animation_controller::{AnimationSet, ClipCue},
    avatar::{AvatarEvent, PlayerAvatar, PlayerInput},
    camera::MagickCamera,
    level_variables::LevelVariables,
    play_content::{
        level_dialogs, load_character, load_start_scene, move_abilities, FsSceneContent,
        PlayContentError,
    },
    scene_loading::custom_hash,
    scene_script::SceneScriptLayout,
    scene_world::SceneWorld,
    script_runtime::ScriptRuntime,
    CharacterDefinition,
};
use magicka_network::lobby::PLAYER_SLOT_CAP;
use magicka_sim::{
    character_physics::{CharacterBody, Orientation},
    impact_motion::Vector,
    spray_pulse::Segment,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod campaign;
mod chant;
mod editor;
mod item_chants;
mod roles;
mod versus;
pub use editor::{EditorSpawn, EditorSpawnOptions, NPC_ORDERS};

/// Game.ThreadedUpdate clamps the logic delta to 0.05 s.
pub const MAX_DELTA: f32 = 0.05;

fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: Vector, s: f32) -> Vector {
    [a[0] * s, a[1] * s, a[2] * s]
}

// ---------------------------------------------------------------------------
// Spawn (GameScene.Initialize, SpawnPlayers)

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub position: Vector,
    pub orientation: Orientation,
}

fn locator_parts(m: &[f32; 16]) -> (Vector, Orientation) {
    (
        [m[12], m[13], m[14]],
        Orientation {
            right: [m[0], m[1], m[2]],
            up: [m[4], m[5], m[6]],
            backward: [m[8], m[9], m[10]],
        },
    )
}

/// Rewrite extension: authored placements cover the original four slots.
/// Slot `k` beyond them stands 1.5 units behind slot `k % 4` per extra row,
/// so a larger party lines up behind the authored positions.
pub fn slot_matrix(matrices: &[[f32; 16]], slot: usize) -> [f32; 16] {
    if let Some(m) = matrices.get(slot) {
        return *m;
    }
    let mut m = matrices
        .get(slot % 4)
        .or(matrices.first())
        .copied()
        .unwrap_or(magicka_content::skinned::IDENTITY);
    let row = (slot / 4) as f32 * 1.5;
    for axis in 0..3 {
        m[12 + axis] += m[8 + axis] * row;
    }
    m
}

/// Four player-slot spawn points and the camera start (their average before
/// ground snapping). `ground` is the scene segment query (backfaces kept).
/// Slots beyond four extend the last-locator offset rule (rewrite extension).
pub fn spawn_slots(
    locators: &HashMap<i32, [f32; 16]>,
    locations: [i32; 4],
    feet_offset: f32,
    ground: impl Fn(Segment) -> Option<Vector>,
) -> ([Placement; 4], Vector) {
    let (slots, camera) = spawn_slots_n(locators, locations, feet_offset, ground, 4);
    (std::array::from_fn(|k| slots[k]), camera)
}

/// `spawn_slots` for `count` slots. The camera start averages the first four.
pub fn spawn_slots_n(
    locators: &HashMap<i32, [f32; 16]>,
    locations: [i32; 4],
    feet_offset: f32,
    ground: impl Fn(Segment) -> Option<Vector>,
    count: usize,
) -> (Vec<Placement>, Vector) {
    let mut camera = [0.0; 3];
    let slots = (0..count.max(4)).map(|k| {
        let mut orientation = Orientation::IDENTITY;
        let mut position = [0.0; 3];
        let mut num = (k as i32).min(3);
        while num >= 0 {
            if let Some(m) = locators.get(&locations[num as usize]) {
                (position, orientation) = locator_parts(m);
                break;
            }
            num -= 1;
        }
        position[0] += 2.0 * (k as i32 - num) as f32;
        position[1] += 1.0;
        if k < 4 {
            camera = add(camera, position);
        }
        let segment = Segment {
            origin: add(position, [0.0, 1.0, 0.0]),
            delta: [0.0, -10.0, 0.0],
        };
        if let Some(hit) = ground(segment) {
            position = hit;
            position[1] += feet_offset;
        }
        Placement {
            position,
            orientation,
        }
    });
    let slots = slots.collect();
    (slots, scale(camera, 0.25))
}

// ---------------------------------------------------------------------------
// Session

#[derive(Clone, Debug, PartialEq)]
pub enum SessionEvent {
    /// Exact missile ownership of a cue started by its authored event.
    MissileSound {
        missile: u32,
        key: u32,
    },
    NpcSpell {
        owner: u16,
        event: AvatarEvent,
    },
    /// Animation controller crossfade/start for the presentation.
    Clip(ClipCue),
    /// The same for a live NPC (entity handle).
    NpcClip(u16, ClipCue),
    Avatar(AvatarEvent),
    /// First execution of an action type that is not reconstructed yet.
    ScriptSkipped(String),
    PickupFailed(String),
    PhysicsFailed(String),
    /// Script failure; scripts stop, the world keeps running.
    ScriptError(String),
    /// TriggerCheckpoint's "#add_checkpoint" text box for the presentation
    /// (a DialogManager TextBox in the original; see the notice fields).
    CheckpointReached(crate::scene_world::CheckpointNotice),
}

#[derive(Debug)]
pub enum SessionError {
    Content(PlayContentError),
    Effects(crate::particles::EffectError),
    Template(String),
    World(String),
    Script(String),
}

/// A one-shot spell cue: (name, emitter, magnitude).
type OneShotCue = (&'static str, Option<[f32; 3]>, Option<f32>);

/// PlayState/RenderManager state carried into a new scene.
#[derive(Clone, Copy, Debug, Default)]
struct Persistent {
    cutscene: crate::cutscene::CutsceneState,
    render_transition: Option<crate::render_transition::RenderTransition>,
    /// Checkpoint restart: player spawn matrices instead of locators.
    spawn_override: Option<[[f32; 16]; 4]>,
    /// GameScene.Initialize's SpawnPlayers gate for scripted-event reset.
    spawn_players: bool,
    /// A fresh level start (Level.GoToScene with no current scene) of a
    /// Level.NoItems level.
    no_items: bool,
}

/// What a departed GameScene keeps for its next Initialize.
#[derive(Clone)]
struct SavedScene {
    runtime: ScriptRuntime,
    animations: Vec<crate::animated_pose::PartPlayback>,
    entities: Vec<crate::scene_world::saved_entities::SavedEntity>,
}

/// PlayState.State captured by TriggerCheckpoint (offline subset): the scene,
/// spawn matrices, ignored triggers, the script runtime (trigger and action
/// state, spawned-NPC health, executed-action history, level variables),
/// animated level-part playback, the camera, music, dialog collection,
/// permanent pickables, player equipment/fairies and the avatar-section
/// flags. See research/checkpoints.md.
#[derive(Clone)]
pub struct Checkpoint {
    pub scene: i32,
    pub matrices: [[f32; 16]; 4],
    pub ignored: Vec<i32>,
    runtime: ScriptRuntime,
    animations: Vec<crate::animated_pose::PartPlayback>,
    players: serde_json::Value,
    dialogs: Option<serde_json::Value>,
    physics: Vec<crate::scene_world::saved_entities::PhysicsSnapshot>,
    /// Level.State snapshots every scene, including those inactive at capture.
    saved_scenes: BTreeMap<String, SavedScene>,
    /// MagickCamera.State.
    camera: crate::camera::CameraState,
    /// PlayState.mMusicBank/mMusicCue (None: no music playing) and
    /// mMusicFocusValue.
    music: Option<(u16, i32)>,
    music_focus_value: Option<f32>,
    /// EntityStateStorage's permanent pickables of the checkpoint scene.
    pickables: Vec<crate::scene_world::saved_entities::PickableSnapshot>,
    /// State.mUnequipWeaponsSection / mDefaultAvatarSection.
    unequip_weapons_section: bool,
    default_avatar_section: bool,
}

impl Checkpoint {
    /// Diagnostics: the stored script runtime (fired triggers, variables).
    pub fn runtime(&self) -> &ScriptRuntime {
        &self.runtime
    }
    /// Diagnostics: the stored camera state.
    pub fn camera(&self) -> &crate::camera::CameraState {
        &self.camera
    }
    /// Diagnostics: the stored music (bank, cue) and focus value.
    pub fn music(&self) -> (Option<(u16, i32)>, Option<f32>) {
        (self.music, self.music_focus_value)
    }
    /// Diagnostics: the stored permanent pickables.
    pub fn pickables(&self) -> &[crate::scene_world::saved_entities::PickableSnapshot] {
        &self.pickables
    }
    /// Diagnostics: the stored (unequip-weapons, default-avatar) sections.
    pub fn sections(&self) -> (bool, bool) {
        (self.unequip_weapons_section, self.default_avatar_section)
    }
    /// Diagnostics: the stored player state.
    pub fn players(&self) -> &serde_json::Value {
        &self.players
    }
    /// Diagnostics: the stored dialog state, if any.
    pub fn dialogs(&self) -> Option<&serde_json::Value> {
        self.dialogs.as_ref()
    }
    /// Diagnostics: number of stored animated-part playbacks.
    pub fn animation_count(&self) -> usize {
        self.animations.len()
    }
}

type PlayerFactory = Box<dyn FnOnce(u16, usize) -> PlayerAvatar>;

type BookEffectRefs = (
    u64,
    Option<crate::particles::EffectRef>,
    Option<crate::particles::EffectRef>,
);
/// Magicka.Cursors used by KeyboardMouseController.
/// NetworkManager.State as menus see it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetworkMode {
    #[default]
    Offline,
    Server,
    Client,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Default,
    Attack,
    PickUp,
    Examine,
    Talk,
    Interact,
}

mod disk_save;
mod hosted;
pub mod remote_visuals;

pub struct PlaySession {
    /// Resolved mod definitions of this match (rewrite extension). The
    /// built-in registry unless a modded match set others.
    pub definitions: Arc<magicka_definitions::Definitions>,
    /// Nonzero once [`PlaySession::set_definitions`] installed definitions;
    /// items record the revision that patched them.
    pub definitions_revision: u64,
    /// Interactions performed this step as (interactable ID, interactor
    /// handle), for mod `OnInteract` callbacks (rewrite extension).
    pub interactions: Vec<(i32, u16)>,
    /// Slots whose wizard a mod script is driving (a scripted cast): the
    /// player's own input is ignored until it ends.
    pub scripted_input: crate::slots::PerSlot<bool>,
    /// Serializable mod state (rewrite extension) carried in campaign saves;
    /// the mod runtime restores and refreshes it.
    pub mod_state: Option<serde_json::Value>,
    /// Registered (mod) Magick combinations by registry index.
    pub registered_magicks: Arc<Vec<(u16, Vec<magicka_sim::spells::Elements>)>>,
    /// Registered Magicks each player may combine (sorted indices); the mod
    /// runtime keeps this current.
    pub registered_unlocked: crate::slots::PerSlot<Vec<u16>>,
    /// Stable local profile identities, independent of entity handles/network slots.
    pub player_ids: crate::slots::PerSlot<Option<String>>,
    /// Player.GamerTag and Defines.PLAYERCOLORS stand-in per slot (profile
    /// name and robe colour; hosts publish joined players' profiles).
    pub player_profiles: crate::slots::PerSlot<Option<(String, [f32; 3])>>,
    /// Network presentation role for menus: hosting a session, or a client
    /// mirroring one (NetworkState Server / Client).
    pub network_mode: NetworkMode,
    pub checkpoint_generation: u64,
    /// Offline global unlock bitfields; save/profile and Versus team integration remain pending.
    pub unlocked_magicks: crate::slots::PerSlot<u64>,
    /// PlayState.UnequipWeaponsSection / DefaultAvatarSection (persist
    /// across scene changes like the PlayState).
    pub unequip_weapons_section: bool,
    pub default_avatar_section: bool,
    /// Player.IconRenderer.TomeMagick per slot (research/magick-hint.md).
    pub tome: crate::slots::PerSlot<crate::tome_magick::TomeMagick>,
    /// PlayState.InventoryBox (one shared box).
    pub inventory: crate::inventory_box::InventoryBox,
    /// Released equipment velocities for the next physics registration pass.
    pending_pickup_velocities: Vec<(magicka_physics::BodyKey, [f32; 3])>,
    inventory_cutscene: bool,
    book_effects: BTreeMap<u16, BookEffectRefs>,
    item_despawn_effects: BTreeMap<u16, Option<crate::particles::EffectRef>>,
    pub world: SceneWorld<FsSceneContent>,
    pub runtime: ScriptRuntime,
    pub events: Vec<SessionEvent>,
    pub play_time: f64,
    /// Content-relative level model path (exported glTF mirrors it).
    pub model_path: PathBuf,
    pub script_error: Option<String>,
    reported: BTreeSet<String>,
    move_was_down: bool,
    cast_was_down: bool,
    remote_dialog_down: crate::slots::PerSlot<bool>,
    pickup_pressed_target: Option<(u16, u64)>,
    pickup_move_locked: bool,
    pickup_bounds: BTreeMap<PathBuf, Option<crate::pickups::PickupBounds>>,
    pub pickup_physics: Option<crate::pickup_physics::PickupPhysics>,
    /// Modern networking lifetime, advanced on every scene rebuild.
    network_epoch: u64,
    remote_characters: BTreeMap<usize, String>,
    /// Network clients: the host's live missiles, drawn without simulation.
    pub remote_missiles: Vec<magicka_network::world::MissileState>,
    /// Network clients: the host's per-wizard weapon trails (handle →
    /// visuals), drawn without simulation (play_session/remote_visuals.rs).
    pub remote_avatars: BTreeMap<u16, remote_visuals::RemoteAvatarVisuals>,
    /// Network clients: the host's SpellManager.IsMagickAllowed mask.
    pub mirrored_magicks_allowed: Option<u64>,
    /// Saved players whose profile is not in the game (rewrite extension):
    /// kept verbatim, written back on save and restored when that profile
    /// joins (play_session/disk_save.rs).
    pub parked_players: Vec<serde_json::Value>,
    /// Slots played on this machine (shared camera); `None`: all of them.
    pub camera_slots: Option<BTreeSet<usize>>,
    /// Polymorph: each transformed slot's robe and pre-polymorph equipment.
    polymorph_saved: BTreeMap<usize, (String, Vec<(i32, crate::avatar::AvatarItem)>)>,
    physics_templates: BTreeMap<String, Option<magicka_content::physics::PhysicsEntityTemplate>>,
    /// Cursor pickup under the pointer, useful for native interaction feedback.
    pub hovered_pickup: Option<u16>,
    /// Interactable (trigger id) under the pointer.
    pub hovered_interactable: Option<i32>,
    pub hovered_character: Option<u16>,
    character_pressed_target: Option<u16>,
    /// mCursorPressedTarget when it is an Interactable.
    interact_pressed_target: Option<i32>,
    /// Run the dialog boxes' render-thread phase here (no presentation
    /// attached), at this logical screen size.
    pub headless_dialogs: Option<[f32; 2]>,
    /// Logic delta of the last step (input runs after it, like PlayState).
    last_dt: f32,
    /// EffectManager and ParticleSystem (Content/Effects).
    pub effects: crate::particles::EffectSystem,
    pub keyboard_hud: crate::keyboard_hud::KeyboardHud,
    /// Level Start Area name (player locators are `Area0..3`).
    start_area: Option<String>,
    /// Each avatar's SpraySpell.mVisualEffects.
    spray_effects: BTreeMap<u16, Vec<crate::particles::EffectRef>>,
    spray_marks: BTreeMap<u16, SprayMarks>,
    /// Live LightningBolt visuals (SpellManager effects until mTTL ≤ 0).
    pub bolts: Vec<LiveBolt>,
    /// One-shot spell cues gathered from avatar events: (name, emitter,
    /// magnitude).
    audio_one_shots: Vec<OneShotCue>,
    /// Railgun stage/hit cue keys by rail.
    rail_cues: BTreeMap<usize, (Option<u32>, Option<u32>)>,
    /// Spell loop cues per caster (SpellEffect.mSpellCues): the effect kind
    /// that started them and their keys.
    spell_loops: BTreeMap<u16, (magicka_sim::spell_cast::EffectKind, Vec<u32>)>,
    /// Audio requests for the presentation, with resolved locator positions.
    pub audio: Vec<(crate::audio::AudioRequest, Option<[f32; 3]>)>,
    /// The latest TriggerCheckpoint state (Restart(Checkpoint) target).
    pub checkpoint: Option<Checkpoint>,
    /// Level-owned GameScene state of scenes left earlier in this level,
    /// by scene name: trigger state, saved animations and saved entities.
    saved_scenes: BTreeMap<String, SavedScene>,
    /// Avatar template name (a checkpoint restart creates a fresh avatar).
    character: String,
    /// Persistent barrier effects (particle walls, geysers) by barrier slot.
    barrier_effects: HashMap<usize, crate::particles::EffectRef>,
    /// Gib trail and landing-splash effects by gib id.
    gib_trails: HashMap<u32, crate::particles::EffectRef>,
    gib_splashes: HashMap<u32, crate::particles::EffectRef>,
    /// UnderGroundAttack effects by attack id.
    underground_effects: HashMap<u32, Vec<crate::particles::EffectRef>>,
    /// StatusEffect emitters by (character handle, status bit).
    status_emitters: HashMap<(u16, u16), (&'static str, crate::particles::EffectRef)>,
    /// Character.mFearEffect per feared character.
    fear_emitters: HashMap<u16, crate::particles::EffectRef>,
    /// Character.mCharmEffect per charmed character.
    charm_emitters: HashMap<u16, crate::particles::EffectRef>,
    /// Confuse.mEffect per confused character.
    confuse_emitters: HashMap<u16, crate::particles::EffectRef>,
    /// PerformanceEnchantment.mEffect per owner while boosted.
    performance_emitters: HashMap<u16, crate::particles::EffectRef>,
    /// Napalm.mSmokeEffect and the target it was started at.
    napalm_smoke: Option<([f32; 3], crate::particles::EffectRef)>,
    /// Character.mLevitateEffect per levitating character.
    levitate_emitters: HashMap<u16, crate::particles::EffectRef>,
    /// GreaseField mParticleEffect / mBurnEffect per pool slot, keyed by the
    /// field serial so a recycled slot restarts them.
    grease_emitters: HashMap<
        usize,
        (
            u32,
            Option<crate::particles::EffectRef>,
            Option<crate::particles::EffectRef>,
        ),
    >,
    /// Rain.mEffect: `magick_rain` following the camera while it rains.
    rain_effect: Option<crate::particles::EffectRef>,
    /// GreaseSpray.mEffect per live spray, oldest first per owner.
    grease_spray_effects: Vec<(u16, crate::particles::EffectRef)>,
    /// TornadoEntity.mDustEffect per live tornado serial.
    tornado_dust: HashMap<u64, crate::particles::EffectRef>,
    /// VortexEntity.mParticleEffect per live vortex serial.
    vortex_effects: HashMap<u64, crate::particles::EffectRef>,
    /// Character.mStunEffect (`stunned`) per stunned character handle.
    stun_effects: HashMap<u16, crate::particles::EffectRef>,
    bolt_serial: u32,
    /// PlayState end-game state (defeat → checkpoint restart).
    pub end_game: crate::end_game::EndGame,
    /// MissileEntity.mSoundList (only StopOnRemove cues follow the missile).
    missile_cues: BTreeMap<u32, Vec<u32>>,
    /// MissileEntity.mEffectReferences (followed effects by name).
    missile_effects: BTreeMap<u32, BTreeMap<&'static str, crate::particles::EffectRef>>,
    self_effects: BTreeMap<u16, Vec<crate::particles::EffectRef>>,
    /// `weapon_arcane_*` effects per (avatar, ArcaneBlade id).
    arcane_blade_effects: BTreeMap<(u16, u32), Vec<crate::particles::EffectRef>>,
    /// Attached-item effects per (owner, slot, effect slot): chant effects
    /// 0..11 by element index, definition effects at 100 + index, and
    /// Item.mFollowEffects at 200..204.
    item_effects: BTreeMap<(u16, i32, usize), crate::particles::EffectRef>,
    /// PhysicsEntity.mLiveEffects by (handle, physics generation, effect index).
    prop_effects: BTreeMap<(u16, u64, usize), crate::particles::EffectRef>,
    animation_effects: crate::animation_effects::AnimationEffects,
    /// Railgun source (false) and hit (true) effects per rail.
    rail_effects: BTreeMap<(usize, bool), crate::particles::EffectRef>,
    /// Railgun render data of the last step (RenderData per live rail).
    pub beams: Vec<magicka_sim::railgun::RailRender>,
    /// Blast.FullBlast calls of the last step: (position, radius, elements).
    pub beam_blasts: Vec<([f32; 3], f32, u32)>,
    /// Live ArcaneBlast spell effects (beam-element FullBlast discs).
    pub arcane_blasts: Vec<crate::arcane_blast::ArcaneBlast>,
    /// Live IceSpikes (GroundBlast with an Ice damage).
    pub ice_spikes: Vec<magicka_sim::ice_spikes::IceSpikes>,
    /// Live DynamicLights (ArcaneBlast flashes), drawn as point lights.
    pub dynamic_lights: Vec<crate::arcane_blast::DynamicLight>,
    pub spray_lights: crate::spell_lights::SprayLights,
    pub status_lights: crate::spell_lights::StatusLights,
    pub(crate) remote_capsule_lights: Option<Vec<crate::arcane_blade::CapsuleLight>>,
    /// ChantSpellManager: conjured element orbs.
    pub chants: crate::chant_spells::ChantSpellManager,
    /// GameScene.mEffects: level-placed effects (flames, dust, …).
    level_effects: Vec<crate::level_effects::LevelEffect>,
    scene_effect_refs: BTreeMap<u64, crate::particles::EffectRef>,
    /// MagickCamera.mVisualEffects (sorted by effect hash); the camera and
    /// its effects outlive scene changes.
    pub camera_effects: BTreeMap<i32, crate::particles::EffectRef>,
    /// Credits (RollCredits); a PlayState-level singleton across scenes.
    pub credits: Option<crate::credits::Credits>,
    /// Seconds into the credits' 1 s fade to black.
    pub credits_fade: f32,
    /// OnTransitionEnd ran: the end-game menu follows.
    pub credits_finished: bool,
    /// The presentation's screen size (RenderManager.ScreenSize) for
    /// screen-space simulation such as the credits scroll.
    pub screen: [f32; 2],
    /// Light.Update variation state per LevelModel light, and the
    /// resulting intensity multiplier of the last step.
    light_variations: Vec<crate::light_variation::LightVariation>,
    pub level_light_intensity: Vec<f32>,
    light_random: crate::particles::DotNetRandom,
    content: PathBuf,
    level_file: String,
    scene_name: String,
    scene_path: PathBuf,
    /// Level.GoToScene waiting for TransitionEnd of its fade-out.
    pending_scene: Option<crate::scene_flow::PendingScene>,
    /// Completed scene changes (presentation hosts reload level assets).
    pub scene_changes: u32,
    /// The scene's Filter/Bloom RenderManager settings.
    pub post: crate::play_content::ScenePost,
}

impl PlaySession {
    /// The selected template for an initialized player, shared by simulation
    /// and presentation after campaign restoration.
    pub fn player_robe(&self, slot: usize) -> Option<&str> {
        self.world.players.get(slot)?.as_ref()?;
        if slot == 0 {
            Some(&self.character)
        } else {
            self.remote_characters.get(&slot).map(String::as_str)
        }
    }

    /// Developer loadout replacement. Resets the player in place, without
    /// restarting scripts, NPCs or the scene. This is not a retail pickup path.
    pub fn debug_player_loadout(
        &mut self,
        robe: &str,
        staff: &str,
        weapon: &str,
    ) -> Result<(), SessionError> {
        self.set_player_loadout(0, robe, staff, weapon)
    }
    /// Host-owned loadout restoration and profile robe replacement.
    /// Presentation mirror: rebuild only when the selected robe or equipment changes.
    pub fn set_player_loadout_if_changed(
        &mut self,
        slot: usize,
        robe: &str,
        staff: &str,
        weapon: &str,
    ) -> Result<(), SessionError> {
        let player = self.player_avatar(slot)?;
        let character = if slot == 0 {
            &self.character
        } else {
            self.remote_characters
                .get(&slot)
                .ok_or_else(|| SessionError::World("missing remote character".into()))?
        };
        if character == robe
            && [(0, weapon), (1, staff)].iter().all(|(slot, name)| {
                player
                    .items
                    .iter()
                    .any(|(s, item)| s == slot && item.definition.name == *name)
            })
        {
            return Ok(());
        }
        self.set_player_loadout(slot, robe, staff, weapon)
    }
    pub fn set_player_loadout(
        &mut self,
        slot: usize,
        robe: &str,
        staff: &str,
        weapon: &str,
    ) -> Result<(), SessionError> {
        self.player_avatar(slot)?;
        let mut template = load_character(&self.content, robe).map_err(SessionError::Content)?;
        for (slot, name) in [(0, weapon), (1, staff)] {
            let attachment = template
                .equipment
                .iter_mut()
                .find(|(s, _)| *s == slot)
                .ok_or_else(|| {
                    SessionError::Template(format!("{robe}: no equipment slot {slot}"))
                })?;
            attachment.1.item = Some(format!("../items/Wizard/{name}"));
        }
        let items = crate::play_content::avatar_items(&self.content, &template);
        if ![0, 1]
            .iter()
            .all(|slot| items.iter().any(|(s, _)| s == slot))
        {
            return Err(SessionError::Template(
                "could not load selected equipment".into(),
            ));
        }
        self.rebuild_avatar(slot, &template, items)?;
        if slot == 0 {
            self.character = robe.into();
        } else {
            self.remote_characters.insert(slot, robe.into());
        }
        Ok(())
    }

    /// Avatar.Initialize(template) in place: a fresh avatar with the same
    /// handle, slot, body index and playing flag, `items` equipped and the
    /// body rebuilt at the old position/orientation. Returns the replaced
    /// avatar so callers can carry state across.
    fn rebuild_avatar(
        &mut self,
        slot: usize,
        template: &magicka_content::character::CharacterTemplate,
        items: Vec<(i32, crate::avatar::AvatarItem)>,
    ) -> Result<PlayerAvatar, SessionError> {
        let definition = CharacterDefinition::from_content(template, 1)
            .map_err(|e| SessionError::Template(format!("{e:?}")))?;
        let rig = crate::play_content::load_rig(&self.content, template)
            .map_err(SessionError::Content)?;
        let animations = Arc::new(AnimationSet::with_weapons(
            &template.animation_sets,
            &crate::play_content::weapon_classes(rig.as_ref()),
            rig.as_ref()
                .map(|r| r.skeleton.clip_durations())
                .unwrap_or_default(),
        ));
        let old = self.player_avatar(slot)?;
        let (index, handle, playing) = (old.body, old.handle, old.playing);
        let mut player = PlayerAvatar::new(
            handle,
            slot,
            index,
            definition,
            animations,
            move_abilities(template),
            custom_hash(&template.id.to_ascii_lowercase()),
        );
        player.rig = rig;
        player.items = items;
        let old_body = &self.world.physics.bodies[index];
        player.playing = playing;
        let mut body = CharacterBody::new(
            player.character.definition.radius,
            player.character.definition.length,
            player.character.definition.mass,
            player.character.definition.speed,
        );
        body.move_to(old_body.position, old_body.orientation);
        body.owner.turn_speed = player.character.definition.turn_speed;
        body.owner.hit_points = player.character.definition.max_hit_points;
        self.world.physics.bodies[index] = body;
        self.world.physics.bodies[index].active = player.playing;
        // The rebuilt avatar keeps the handle; it is no longer gibbed.
        self.world.overkilled.remove(&player.handle);
        let old = self.world.players[slot].replace(player).unwrap();
        self.events
            .push(SessionEvent::Avatar(AvatarEvent::SelfEffectsStopped));
        Ok(old)
    }
    /// Polymorph.PolymorphAvatar: GetPolymorphValues, Initialize with the
    /// new template (a form keeps its own equipment; restoration re-equips
    /// the items worn before), ApplyPolymorphValues, then MoveTo keeping the
    /// feet and orientation. The robe and items are saved on entering.
    fn polymorph_avatar(
        &mut self,
        request: crate::scene_world::polymorph_live::AvatarPolymorph,
    ) -> Result<(), SessionError> {
        use magicka_sim::polymorph as pm;
        let slot = self
            .world
            .players
            .iter()
            .position(|p| p.as_ref().is_some_and(|p| p.handle == request.handle))
            .ok_or_else(|| SessionError::Template("polymorph target left".into()))?;
        let robe = if slot == 0 {
            self.character.clone()
        } else {
            self.remote_characters
                .get(&slot)
                .cloned()
                .unwrap_or_else(|| self.character.clone())
        };
        let (template, items) = match request.form {
            Some(form) => {
                let template =
                    load_character(&self.content, form).map_err(SessionError::Content)?;
                let items = crate::play_content::avatar_items(&self.content, &template);
                (template, items)
            }
            None => {
                let (robe, items) = self.polymorph_saved.remove(&slot).ok_or_else(|| {
                    SessionError::Template("polymorph restore without saved state".into())
                })?;
                let template =
                    load_character(&self.content, &robe).map_err(SessionError::Content)?;
                (template, items)
            }
        };
        // GetPolymorphValues.
        let old = self.player_avatar(slot)?;
        let old_body = self.world.physics.bodies[old.body].clone();
        let health =
            old.character.combat.health.hit_points / old.character.combat.health.max_hit_points;
        let time_since_last_damage = old.character.combat.time_since_last_damage;
        let faction = old.faction;
        let ethereal = old.character.ethereal;
        let fear = old.character.fear;
        let charm = old.character.charm;
        let statuses = *old.character.combat.statuses.slots();
        let old = self.rebuild_avatar(slot, &template, items)?;
        if request.form.is_some() {
            self.polymorph_saved
                .entry(slot)
                .or_insert_with(|| (robe, old.items));
        }
        // ApplyPolymorphValues.
        let p = self.world.players[slot].as_mut().unwrap();
        p.polymorphed = request.form.is_some();
        p.faction = faction;
        let h = &mut p.character.combat.health;
        h.hit_points = health * h.max_hit_points;
        p.character.combat.time_since_last_damage = time_since_last_damage;
        if ethereal {
            p.character.set_ethereal(true);
        }
        p.character.fear = fear;
        p.character.charm = charm;
        let mut events = Vec::new();
        for effect in statuses {
            let _ = p.add_status_effect(effect, &mut events);
        }
        let body = &mut self.world.physics.bodies[p.body];
        let position = pm::replacement_position(
            old_body.position,
            old_body.radius,
            old_body.length,
            body.radius,
            body.length,
        );
        body.move_to(position, old_body.orientation);
        body.set_desired_direction(old_body.desired_direction());
        Ok(())
    }

    /// Starting equipment comes from the robe, independently of current inventory.
    pub fn reset_player_equipment(&mut self, slot: usize, robe: &str) -> Result<(), SessionError> {
        let template = load_character(&self.content, robe).map_err(SessionError::Content)?;
        let items = crate::play_content::avatar_items(&self.content, &template);
        let name = |slot| {
            items
                .iter()
                .find(|(s, _)| *s == slot)
                .map(|(_, item)| item.definition.name.clone())
                .ok_or_else(|| {
                    SessionError::Template(format!("{robe}: missing starting equipment {slot}"))
                })
        };
        self.set_player_loadout(slot, robe, &name(1)?, &name(0)?)
    }
    /// Revive.Update's avatar creation for a still-playing revivee:
    /// Avatar.Initialize at the revive point (ground probe from 1 m above,
    /// 10 m down: hit Y + 0.1 - HeightOffset; identity orientation),
    /// HitPoints = MaxHitPoints · health, SpawnAnimation revive and
    /// RessurectionState, then the magick_revive cue. The robe, staff and
    /// weapon reset to robe defaults; the handle and unlocks are retained.
    fn revive_player(&mut self, revive: &magicka_sim::magicks::Revive) -> Result<(), SessionError> {
        let slot = revive.revivee;
        if !self
            .world
            .players
            .get(slot)
            .and_then(Option::as_ref)
            .is_some_and(|p| p.playing)
        {
            return Ok(());
        }
        let robe = if slot == 0 {
            self.character.clone()
        } else {
            self.remote_characters
                .get(&slot)
                .cloned()
                .ok_or_else(|| SessionError::World("missing remote character".into()))?
        };
        self.reset_player_equipment(slot, &robe)?;
        // Revive: RemoveAllWeapons while the unequip section is active.
        if self.unequip_weapons_section {
            self.set_equipment(slot, 1, "weapon_unarmed")?;
            self.set_equipment(slot, 0, "weapon_unarmed")?;
        }
        let body_index = self.world.players[slot].as_ref().unwrap().body;
        let body = &self.world.physics.bodies[body_index];
        let mut position = revive.position;
        let probe = Segment {
            origin: [position[0], position[1] + 1.0, position[2]],
            delta: [0.0, -10.0, 0.0],
        };
        if let Some(hit) = self
            .world
            .collision
            .collision()
            .segment_intersect(probe, false)
        {
            position = hit.hit.position;
            position[1] += 0.1;
        }
        position[1] += body.radius + body.length * 0.5;
        let body = &mut self.world.physics.bodies[body_index];
        body.move_to(position, Default::default());
        body.set_movement([0.; 3]);
        let player = self.world.players[slot].as_mut().unwrap();
        let health = &mut player.character.combat.health;
        health.hit_points = player.character.definition.max_hit_points * revive.health;
        body.owner.hit_points = health.hit_points;
        let revive_clip = crate::character_motion::animation_index("revive")
            .expect("revive is an Animations value");
        player.begin_resurrection(body, revive_clip);
        self.world.play_cue(16, "magick_revive", Some(position));
        Ok(())
    }

    /// Server respawn at this scene's checkpoint/start placement with robe defaults.
    pub fn respawn_player(&mut self, slot: usize, robe: &str) -> Result<(), SessionError> {
        self.player_avatar(slot)?;
        let checkpoint = self
            .checkpoint
            .as_ref()
            .filter(|c| c.scene == custom_hash(self.scene_name()))
            .map(|c| slot_matrix(&c.matrices, slot));
        let locations = std::array::from_fn(|i| {
            custom_hash(&format!(
                "{}{i}",
                self.start_area.as_deref().unwrap_or("start")
            ))
        });
        self.reset_player_equipment(slot, robe)?;
        if self.unequip_weapons_section {
            self.set_equipment(slot, 1, "weapon_unarmed")?;
            self.set_equipment(slot, 0, "weapon_unarmed")?;
        }
        let body_index = self.world.players[slot].as_ref().unwrap().body;
        let handle = self.world.players[slot].as_ref().unwrap().handle;
        self.world.cancel_player_abilities(slot, handle);
        let body = &self.world.physics.bodies[body_index];
        let placement = if let Some(matrix) = checkpoint {
            let (mut position, orientation) = locator_parts(&matrix);
            if let Some(hit) = self.world.collision.collision().segment_intersect(
                Segment {
                    origin: position,
                    delta: [0., -5., 0.],
                },
                false,
            ) {
                position = hit.hit.position;
                position[1] += body.length * 0.5 + body.radius;
            }
            Placement {
                position,
                orientation,
            }
        } else {
            self.player_placement(body, locations, slot)
        };
        self.world.physics.bodies[body_index].move_to(placement.position, placement.orientation);
        self.world.physics.bodies[body_index].set_movement([0.; 3]);
        Ok(())
    }

    /// Level.ctor + GameScene load/Initialize for one local player.
    pub fn load(content: &Path, level_file: &str, character: &str) -> Result<Self, SessionError> {
        Self::load_at(content, level_file, character, None, None)
    }

    /// `load` for a given scene (`None`: the level's start scene), optionally
    /// spawning at checkpoint matrices.
    fn load_at(
        content: &Path,
        level_file: &str,
        character: &str,
        scene: Option<i32>,
        spawn_override: Option<[[f32; 16]; 4]>,
    ) -> Result<Self, SessionError> {
        let start = match scene {
            Some(scene) => crate::play_content::load_level_scene(content, level_file, Some(scene)),
            None => load_start_scene(content, level_file),
        }
        .map_err(SessionError::Content)?;
        let (body, make) = Self::prepare_player(content, character, 0)?;
        let locations = start.start.locations;
        let no_items = scene.is_none() && spawn_override.is_none() && start.start.no_items;
        Self::build(
            content,
            level_file,
            start,
            locations,
            body,
            make,
            LevelVariables::default(),
            0.0,
            Persistent {
                spawn_override,
                spawn_players: true,
                no_items,
                ..Default::default()
            },
            character,
        )
    }

    fn prepare_player(
        content: &Path,
        character: &str,
        slot: usize,
    ) -> Result<(CharacterBody, PlayerFactory), SessionError> {
        let template = load_character(content, character).map_err(SessionError::Content)?;
        let definition = CharacterDefinition::from_content(&template, 1)
            .map_err(|e| SessionError::Template(format!("{e:?}")))?;
        let rig =
            crate::play_content::load_rig(content, &template).map_err(SessionError::Content)?;
        let durations = rig
            .as_ref()
            .map(|r| r.skeleton.clip_durations())
            .unwrap_or_default();
        let animations = Arc::new(AnimationSet::with_weapons(
            &template.animation_sets,
            &crate::play_content::weapon_classes(rig.as_ref()),
            durations,
        ));
        let abilities = move_abilities(&template);
        let items = crate::play_content::avatar_items(content, &template);
        let template_type = custom_hash(&template.id.to_ascii_lowercase());
        let mut body = CharacterBody::new(
            definition.radius,
            definition.length,
            definition.mass,
            definition.speed,
        );
        body.owner.turn_speed = definition.turn_speed;
        body.owner.hit_points = definition.max_hit_points;
        Ok((
            body,
            Box::new(move |handle, index| {
                let mut avatar = PlayerAvatar::new(
                    handle,
                    slot,
                    index,
                    definition,
                    animations,
                    abilities,
                    template_type,
                );
                avatar.rig = rig;
                avatar.items = items;
                avatar
            }),
        ))
    }

    /// Modern host join: one avatar/body in the existing shared scene.
    pub fn join_player_at_start(
        &mut self,
        slot: usize,
        character: &str,
    ) -> Result<u16, SessionError> {
        let area = self.start_area.clone().unwrap_or_else(|| "start".into());
        self.join_player(slot, character, &area)
    }
    pub fn join_player(
        &mut self,
        slot: usize,
        character: &str,
        spawn_point: &str,
    ) -> Result<u16, SessionError> {
        // PLAYER_SLOT_CAP itself is the observer clients' mirror slot for the
        // host's slot 0 (network_session::OBSERVER_VIEW_SLOT).
        if slot > PLAYER_SLOT_CAP || self.world.players.get(slot).is_some_and(Option::is_some) {
            return Err(SessionError::World("remote player slot unavailable".into()));
        }
        // The level authors four numbered locators; slots beyond them share
        // the last one with a lateral offset (spawn_slots).
        let locator = custom_hash(&format!(
            "{}{}",
            spawn_point.to_ascii_lowercase(),
            slot.min(3)
        ));
        if !self
            .world
            .level
            .locators
            .iter()
            .any(|l| custom_hash(&l.name) == locator)
        {
            return Err(SessionError::World(
                "remote player spawn locator missing".into(),
            ));
        }
        let (mut body, make) = Self::prepare_player(&self.content, character, slot)?;
        let locations = std::array::from_fn(|i| {
            custom_hash(&format!("{}{i}", spawn_point.to_ascii_lowercase()))
        });
        let placement = self.player_placement(&body, locations, slot);
        body.move_to(placement.position, placement.orientation);
        body.set_desired_direction(placement.orientation.forward());
        self.add_joined_player(slot, character, body, make)
    }

    /// Hot join (rewrite extension; the original game had no mid-game join):
    /// spawn beside a living player so the joiner arrives with the party
    /// instead of at a possibly distant level start. Falls back to the
    /// start locators when nobody is alive.
    pub fn join_player_near_party(
        &mut self,
        slot: usize,
        character: &str,
    ) -> Result<u16, SessionError> {
        if slot >= PLAYER_SLOT_CAP || self.world.players.get(slot).is_some_and(Option::is_some) {
            return Err(SessionError::World("remote player slot unavailable".into()));
        }
        let Some(anchor) = self.party_anchor(slot) else {
            return self.join_player_at_start(slot, character);
        };
        let (mut body, make) = Self::prepare_player(&self.content, character, slot)?;
        let placement = self.placement_near(&body, anchor, slot);
        body.move_to(placement.position, placement.orientation);
        body.set_desired_direction(placement.orientation.forward());
        self.add_joined_player(slot, character, body, make)
    }

    /// The living, playing avatar a hot joiner spawns beside: the lowest
    /// slot other than `slot` (deterministic).
    fn party_anchor(&self, slot: usize) -> Option<(Vector, Orientation)> {
        self.world
            .players
            .iter()
            .enumerate()
            .filter(|(s, _)| *s != slot)
            .filter_map(|(_, p)| p.as_ref())
            .find(|p| p.playing && !p.dead())
            .map(|p| {
                let body = &self.world.physics.bodies[p.body];
                (body.position, body.orientation)
            })
    }

    /// A free spot around `anchor`: candidate offsets on two rings, the first
    /// with ground below and no character within a body width. The ring
    /// start rotates with the slot so simultaneous joiners spread out.
    fn placement_near(
        &self,
        body: &CharacterBody,
        (center, orientation): (Vector, Orientation),
        slot: usize,
    ) -> Placement {
        let feet = body.length * 0.5 + body.radius;
        let clearance = body.radius * 2.0 + 0.25;
        let occupied = |p: Vector| {
            self.world.physics.bodies.iter().any(|b| {
                b.active && {
                    let d = [b.position[0] - p[0], b.position[2] - p[2]];
                    (d[0] * d[0] + d[1] * d[1]).sqrt() < clearance
                        && (b.position[1] - p[1]).abs() < 2.0
                }
            })
        };
        for ring in [1.5f32, 3.0] {
            for k in 0..8 {
                let angle = std::f32::consts::TAU * ((k + slot) % 8) as f32 / 8.0;
                let mut position = [
                    center[0] + ring * angle.cos(),
                    center[1] + 1.0,
                    center[2] + ring * angle.sin(),
                ];
                let segment = Segment {
                    origin: position,
                    delta: [0.0, -4.0, 0.0],
                };
                let Some(hit) = self
                    .world
                    .collision
                    .collision()
                    .segment_intersect(segment, false)
                    .map(|hit| hit.hit.position)
                else {
                    continue;
                };
                position = hit;
                position[1] += feet;
                if !occupied(position) {
                    return Placement {
                        position,
                        orientation,
                    };
                }
            }
        }
        Placement {
            position: center,
            orientation,
        }
    }

    fn add_joined_player(
        &mut self,
        slot: usize,
        character: &str,
        body: CharacterBody,
        make: PlayerFactory,
    ) -> Result<u16, SessionError> {
        // Slots the party is currently denied: the lowest other playing
        // avatar's weapon_unarmed slots (NoItems, RemoveAllWeapons).
        let denied: Vec<i32> = self
            .world
            .players
            .iter()
            .enumerate()
            .filter(|(s, _)| *s != slot)
            .filter_map(|(_, p)| p.as_ref())
            .find(|p| p.playing)
            .map(|p| {
                p.items
                    .iter()
                    .filter(|(_, i)| i.definition.name.eq_ignore_ascii_case("weapon_unarmed"))
                    .map(|(s, _)| *s)
                    .collect()
            })
            .unwrap_or_default();
        self.world.add_player(slot, body, make);
        self.remote_characters.insert(slot, character.into());
        // Rewrite join (the original spawned every avatar at level start):
        // the joiner arrives in the party's equipment state rather than with
        // robe defaults, like respawn/revive inside an unequip section. Picked
        // up or assigned items are not copied.
        for s in [1, 0] {
            if self.unequip_weapons_section || denied.contains(&s) {
                self.set_equipment(slot, s, "weapon_unarmed")?;
            }
        }
        if self.world.versus().is_some() {
            // VersusRuleset.Sync → PostInitialisePlayer.
            self.unlocked_magicks[slot] = 0;
            let settings = self.world.versus_settings;
            let team = match self.world.player_teams[slot] {
                0 => settings.default_teams(slot + 1)[slot],
                team => team,
            };
            self.world.set_player_team(slot, team);
            let handle = self.world.players[slot].as_ref().unwrap().handle;
            if let Ok(e) = self.world.entity_mut(handle) {
                e.faction &= !crate::camera::FACTION_FRIENDLY;
            }
        }
        Ok(self.world.players[slot].as_ref().unwrap().handle)
    }
    /// Modern host ownership departure. Body indices remain stable tombstones.
    pub fn depart_network_player(&mut self, slot: usize) -> Result<(), SessionError> {
        self.handle_remote_input(slot, magicka_network::control::Controls::default())?;
        let player = self.world.players[slot].as_mut().unwrap();
        let handle = player.handle;
        player.playing = false;
        player.stop_mouse_move();
        player.current_spell = None;
        player.character.combat.dead = true;
        let body = &mut self.world.physics.bodies[player.body];
        body.set_movement([0.; 3]);
        body.active = false;
        if let Some(entity) = self.world.entities.iter_mut().find(|e| e.handle == handle) {
            entity.added = false;
            entity.removed = true;
            entity.unique_id = 0;
        }
        // The session camera/end-game paths require slot zero even with no clients.
        // Keep its dormant avatar internally; HostedSession omits it from snapshots.
        if slot != 0 {
            self.world.players[slot] = None;
        }
        self.remote_characters.remove(&slot);
        self.world.cancel_player_abilities(slot, handle);
        self.world.dead_age[slot] = 0.0;
        self.unlocked_magicks[slot] = 0;
        for mut reference in self
            .spray_effects
            .remove(&handle)
            .unwrap_or_default()
            .into_iter()
            .chain(self.self_effects.remove(&handle).unwrap_or_default())
        {
            self.effects.stop(&mut reference);
        }
        self.spray_marks.remove(&handle);
        let effects = &mut self.effects;
        self.item_effects.retain(|&(owner, _, _), reference| {
            if owner == handle {
                effects.stop(reference);
                false
            } else {
                true
            }
        });
        self.status_emitters.retain(|&(owner, _), (_, reference)| {
            if owner == handle {
                effects.stop(reference);
                false
            } else {
                true
            }
        });
        self.arcane_blade_effects.retain(|&(owner, _), references| {
            if owner == handle {
                for reference in references {
                    effects.stop(reference);
                }
                false
            } else {
                true
            }
        });
        if let Some((_, keys)) = self.spell_loops.remove(&handle) {
            for key in keys {
                self.world.stop_cue(key);
            }
        }
        Ok(())
    }
    pub fn fresh_network_player_at_start(&mut self, slot: usize) -> Result<u16, SessionError> {
        if self.world.players.get(slot).is_some_and(Option::is_some) {
            self.depart_network_player(slot)?;
            self.world.players[slot] = None;
        }
        self.join_player_at_start(slot, "Wizard")
    }
    fn player_placement(
        &self,
        body: &CharacterBody,
        locations: [i32; 4],
        slot: usize,
    ) -> Placement {
        let locators = self
            .world
            .level
            .locators
            .iter()
            .map(|l| (custom_hash(&l.name), l.transform))
            .collect();
        spawn_slots_n(
            &locators,
            locations,
            body.length * 0.5 + body.radius,
            |segment| {
                self.world
                    .collision
                    .collision()
                    .segment_intersect(segment, false)
                    .map(|hit| hit.hit.position)
            },
            slot + 1,
        )
        .0[slot]
    }
    /// RenderManager.Saturation: the scene's, or TimeWarp's while it runs.
    pub fn render_saturation(&self) -> f32 {
        self.world.render_saturation.unwrap_or(self.post.saturation)
    }
    /// SpellManager.IsMagickAllowed (Campaign/Mythos): a magick is castable
    /// when any playing player has unlocked it.
    pub fn magicks_allowed(&self) -> u64 {
        if let Some(mask) = self.mirrored_magicks_allowed {
            return mask;
        }
        self.world
            .players
            .iter()
            .enumerate()
            .filter(|(_, p)| p.as_ref().is_some_and(|p| p.playing))
            .fold(0, |acc, (slot, _)| acc | self.unlocked_magicks[slot])
    }
    /// Apply already authorized remote input; aim is a point, never a position update.
    pub fn handle_remote_input(
        &mut self,
        slot: usize,
        controls: magicka_network::control::Controls,
    ) -> Result<(), SessionError> {
        if self.world.players.get(slot).is_none_or(Option::is_none) {
            return Err(SessionError::World("remote player slot unavailable".into()));
        }
        if !controls.valid() {
            return Err(SessionError::World("invalid remote controls".into()));
        }
        let dialog_down = controls.move_down
            || controls.pad_attack
            || controls.boost_down
            || controls.cast_self_down
            || controls.cast_down;
        let select_pressed = dialog_down && !self.remote_dialog_down[slot];
        self.remote_dialog_down[slot] = dialog_down;
        if !self.world.tutorial.dialog_done() {
            let avatar = self.world.players[slot].as_mut().unwrap();
            avatar.stop_mouse_move();
            self.world.physics.bodies[avatar.body].set_movement([0.; 3]);
            if select_pressed && !self.end_game.input_limited {
                self.dismiss_tutorial_dialog();
            }
            return Ok(());
        }
        let limited = self.world.flags.input_limited
            || self.world.camera.lock_input
            || self.world.player_input_locked[slot]
            || self.end_game.input_limited;
        let magicks_allowed = self.magicks_allowed();
        let pressed = self.world.players[slot]
            .as_ref()
            .unwrap()
            .action_pressed(controls.move_down);
        let can_advance = self
            .world
            .dialogs
            .as_ref()
            .is_some_and(|m| m.can_advance(Some(slot as i32)));
        if pressed && !limited && !self.world.flags.cutscene && can_advance {
            self.world.advance_dialog(slot as i32);
        }
        if controls.pad_movement.is_some() {
            self.handle_pad_action_for(slot, pressed, can_advance, limited);
        }
        self.handle_player_ui(
            slot,
            controls.magick_prev,
            controls.magick_next,
            controls.inventory,
        );
        let world = &mut self.world;
        let avatar = world.players[slot].as_mut().unwrap();
        avatar.registered_magicks = self.registered_magicks.clone();
        avatar
            .registered_unlocked
            .clone_from(&self.registered_unlocked[slot]);
        let body = &mut world.physics.bodies[avatar.body];
        let input = PlayerInput {
            cursor_ndc: None,
            pad_movement: controls.pad_movement,
            pad_attack: controls.pad_attack,
            pad_block: controls.pad_block,
            move_down: controls.move_down,
            cast_down: controls.cast_down,
            cast_self_down: controls.cast_self_down,
            shift_down: controls.shift_down,
            boost_down: controls.boost_down,
            elements: controls.elements,
            magick_prev: controls.magick_prev,
            magick_next: controls.magick_next,
            inventory: controls.inventory,
        };
        avatar.handle_input(
            body,
            input,
            crate::avatar::InputFrame {
                pick: controls.aim,
                input_limited: limited,
                in_cutscene: world.flags.cutscene,
                dt: self.last_dt,
                play_time: world.play_time,
                weapon_cast_disabled: world
                    .flags
                    .tutorial_cast_types_disabled
                    .contains(&(magicka_sim::damage::CastType::Weapon as i32)),
                boost_target: magicka_sim::shield_boost::shield_to_boost(
                    body.position,
                    body.direction(),
                    world.shields.iter().map(|s| &s.shield),
                )
                .map(|i| world.shields[i].handle),
                move_locked: false,
                magicks_allowed,
                magick_cast_disabled: world
                    .flags
                    .tutorial_cast_types_disabled
                    .contains(&(magicka_sim::damage::CastType::Magick as i32)),
                elements_disabled: world.flags.tutorial_elements_disabled as u32,
                force_cast_disabled: world
                    .flags
                    .tutorial_cast_types_disabled
                    .contains(&(magicka_sim::damage::CastType::Force as i32)),
                area_cast_disabled: world
                    .flags
                    .tutorial_cast_types_disabled
                    .contains(&(magicka_sim::damage::CastType::Area as i32)),
                self_cast_disabled: world
                    .flags
                    .tutorial_cast_types_disabled
                    .contains(&(magicka_sim::damage::CastType::SelfCast as i32)),
                push_disabled: world.flags.tutorial_push_disabled,
            },
        );
        Ok(())
    }

    /// GameScene.LoadLevel + Initialize (SpawnPlayers) for `scene`, placing
    /// `body` at the `locations` spawn locators and running startup actions.
    #[allow(clippy::too_many_arguments)]
    fn build(
        content: &Path,
        level_file: &str,
        start: crate::play_content::StartScene,
        locations: [i32; 4],
        mut body: CharacterBody,
        make: PlayerFactory,
        variables: LevelVariables,
        play_time: f64,
        persistent: Persistent,
        character: &str,
    ) -> Result<Self, SessionError> {
        let model = Arc::new(start.model);
        let mut world = SceneWorld::new(model.clone(), FsSceneContent::new(content), 0x5eed)
            .map_err(SessionError::World)?;
        world.mine_clip_duration = crate::play_content::mine_clip_duration(content).ok();
        world.barrier_clips = crate::play_content::barrier_clips(content).ok();
        world.wave_pillar_clip = crate::play_content::wave_pillar_clip(content).ok();
        // PlayState and RenderManager state outlives the scene.
        world.flags.cutscene = persistent.cutscene.active;
        world.cutscene = persistent.cutscene;
        world.render_transition = persistent.render_transition;
        world.physics.kill_plane = start.scene.kill_plane;
        // Looped (new game plus) saves are not modelled: campaigns read false.
        world.campaign_looped = (crate::end_game::GameType::for_level(level_file)
            == crate::end_game::GameType::Campaign)
            .then_some(false);
        world.game_type = crate::end_game::GameType::for_level(level_file);
        world.level_spawn_fairy = start.start.spawn_fairy;

        let locators: HashMap<i32, [f32; 16]> = model
            .locators
            .iter()
            .map(|l| (custom_hash(&l.name), l.transform))
            .collect();
        let feet_offset = body.length * 0.5 + body.radius;
        let (slots, camera_start) = {
            let collision = world.collision.collision();
            let ground = |segment| {
                collision
                    .segment_intersect(segment, false)
                    .map(|hit: crate::level_collision::SceneHit| hit.hit.position)
            };
            match persistent.spawn_override {
                // State.ApplyState: a 5 m drop from each checkpoint matrix,
                // then radius + length/2 above the hit. Without a hit the
                // original uses the origin; the matrix position is kept here.
                Some(matrices) => {
                    let slots = matrices.map(|m| {
                        let (mut position, orientation) = locator_parts(&m);
                        if let Some(hit) = ground(Segment {
                            origin: position,
                            delta: [0.0, -5.0, 0.0],
                        }) {
                            position = hit;
                            position[1] += feet_offset;
                        }
                        Placement {
                            position,
                            orientation,
                        }
                    });
                    let camera = slots[0].position;
                    (slots, camera)
                }
                None => spawn_slots(&locators, locations, feet_offset, ground),
            }
        };
        world.camera.set_position(camera_start, true);

        body.move_to(slots[0].position, slots[0].orientation);
        let forward = slots[0].orientation.forward();
        body.set_desired_direction(forward);
        body.set_movement([0.0; 3]);
        world.add_player(0, body, make);
        {
            // GameScene.Initialize clears scripted Avatar.Events and Path,
            // not just the separate presentation-output queue.
            let avatar = world.players[0].as_mut().unwrap();
            let body = &mut world.physics.bodies[avatar.body];
            if persistent.spawn_players && !avatar.dead() {
                avatar.set_events(body, None);
                avatar.ai.path.clear();
                body.set_movement([0.; 3]);
            }
        }
        // PlayState.CreatePlayer → SetSpawnFairies (level start defaults);
        // GameScene.Initialize → SpawnFairies once the players are placed.
        // Scene changes and checkpoint restarts overwrite the flags with the
        // PlayState's persistent mSpawnFairies afterwards.
        world.set_spawn_fairies();
        if persistent.spawn_players {
            world.spawn_fairies();
        }

        // A hosted scene (no scene file) runs without level scripts.
        let layout = if start.scene.path.as_os_str().is_empty() {
            SceneScriptLayout::default()
        } else {
            let scene_bytes = std::fs::read(&start.scene.path).map_err(|e| {
                SessionError::Content(PlayContentError::Io(start.scene.path.clone(), e))
            })?;
            SceneScriptLayout::read(&scene_bytes, false)
                .map_err(|e| SessionError::Script(format!("{e:?}")))?
        };
        world.register_interactables(&layout);
        if let Some(dialogs) = level_dialogs(content, level_file) {
            let level = std::fs::read(&dialogs).unwrap_or_default();
            let fairy = std::fs::read(content.join("Levels/Dialog_Fairy.xml")).unwrap_or_default();
            if let Ok(definitions) =
                crate::dialog_definition::DialogDefinition::read_collection(&[&level, &fairy])
            {
                install_dialogs(&mut world, content, definitions);
            }
        }
        let library = crate::particles::EffectLibrary::load_directory(&content.join("Effects"))
            .map_err(SessionError::Effects)?;
        world.effect_types = library
            .effects
            .iter()
            .map(|(&id, e)| (id, e.kind))
            .collect();
        let mut runtime = ScriptRuntime::new(layout, variables);
        runtime.skip_unsupported = true;
        let post = start.scene.post;
        world.scene_indoors = start.scene.indoor;
        let mut session = Self {
            definitions: Arc::new(magicka_definitions::Definitions::builtin()),
            definitions_revision: 0,
            registered_magicks: Arc::new(Vec::new()),
            registered_unlocked: Default::default(),
            mod_state: None,
            scripted_input: Default::default(),
            interactions: Vec::new(),
            player_ids: Default::default(),
            player_profiles: Default::default(),
            network_mode: NetworkMode::Offline,
            checkpoint_generation: 0,
            unlocked_magicks: crate::slots::PerSlot::filled(4, 0),
            unequip_weapons_section: false,
            default_avatar_section: false,
            tome: Default::default(),
            inventory: Default::default(),
            pending_pickup_velocities: Vec::new(),
            inventory_cutscene: false,
            book_effects: BTreeMap::new(),
            item_despawn_effects: BTreeMap::new(),
            post,
            world,
            runtime,
            events: Vec::new(),
            play_time,
            model_path: start.model_path,
            script_error: None,
            reported: BTreeSet::new(),
            move_was_down: false,
            cast_was_down: false,
            remote_dialog_down: Default::default(),
            pickup_pressed_target: None,
            pickup_move_locked: false,
            pickup_bounds: BTreeMap::new(),
            pickup_physics: None,
            network_epoch: 1,
            remote_characters: BTreeMap::new(),
            remote_missiles: Vec::new(),
            remote_avatars: BTreeMap::new(),
            mirrored_magicks_allowed: None,
            parked_players: Vec::new(),
            camera_slots: None,
            polymorph_saved: BTreeMap::new(),
            physics_templates: BTreeMap::new(),
            hovered_pickup: None,
            hovered_interactable: None,
            hovered_character: None,
            character_pressed_target: None,
            interact_pressed_target: None,
            headless_dialogs: None,
            last_dt: 0.0,
            effects: crate::particles::EffectSystem::new(library),
            keyboard_hud: crate::keyboard_hud::KeyboardHud::default(),
            start_area: start.start.area.clone(),
            spray_effects: BTreeMap::new(),
            spray_marks: BTreeMap::new(),
            missile_cues: BTreeMap::new(),
            missile_effects: BTreeMap::new(),
            end_game: crate::end_game::EndGame {
                game_type: crate::end_game::GameType::for_level(level_file),
                ..Default::default()
            },
            bolts: Vec::new(),
            audio: Vec::new(),
            audio_one_shots: Vec::new(),
            rail_cues: BTreeMap::new(),
            spell_loops: BTreeMap::new(),
            checkpoint: None,
            // Known before Initialize so startup item requests can load
            // the robe template (drain at the end of build).
            character: character.to_string(),
            barrier_effects: HashMap::new(),
            gib_trails: HashMap::new(),
            gib_splashes: HashMap::new(),
            status_emitters: HashMap::new(),
            fear_emitters: HashMap::new(),
            rain_effect: None,
            charm_emitters: HashMap::new(),
            confuse_emitters: HashMap::new(),
            levitate_emitters: HashMap::new(),
            napalm_smoke: None,
            performance_emitters: HashMap::new(),
            grease_emitters: HashMap::new(),
            grease_spray_effects: Vec::new(),
            tornado_dust: HashMap::new(),
            vortex_effects: HashMap::new(),
            stun_effects: HashMap::new(),
            underground_effects: HashMap::new(),
            bolt_serial: 0,
            self_effects: BTreeMap::new(),
            arcane_blade_effects: BTreeMap::new(),
            item_effects: BTreeMap::new(),
            prop_effects: BTreeMap::new(),
            animation_effects: Default::default(),
            rail_effects: BTreeMap::new(),
            beams: Vec::new(),
            beam_blasts: Vec::new(),
            arcane_blasts: Vec::new(),
            ice_spikes: Vec::new(),
            dynamic_lights: Vec::new(),
            spray_lights: Default::default(),
            status_lights: Default::default(),
            remote_capsule_lights: None,
            chants: Default::default(),
            level_effects: Vec::new(),
            scene_effect_refs: BTreeMap::new(),
            camera_effects: BTreeMap::new(),
            credits: None,
            credits_fade: 0.0,
            credits_finished: false,
            screen: [1280.0, 720.0],
            light_variations: Vec::new(),
            level_light_intensity: Vec::new(),
            light_random: crate::particles::DotNetRandom::new(0x4c69_6768),
            content: content.to_path_buf(),
            level_file: level_file.to_string(),
            scene_name: start.scene.name.clone(),
            scene_path: start.scene.path.clone(),
            pending_scene: None,
            saved_scenes: BTreeMap::new(),
            scene_changes: 0,
        };
        // GameScene.Initialize: LevelModel.GetAllEffects starts the level's
        // placed effects.
        session.level_effects = crate::level_effects::placements(&session.world.level);
        for placement in &session.level_effects {
            if session.world.effects.effects.contains_key(&placement.id) {
                return Err(SessionError::World(format!(
                    "duplicate level effect ID {}",
                    placement.id
                )));
            }
            use crate::effect_action::EffectActionHost;
            session
                .world
                .add_effect(
                    placement.id,
                    placement.hash,
                    placement.local,
                    placement.range,
                )
                .map_err(SessionError::World)?;
            if let Some(storage) = session.world.effects.effects.get_mut(&placement.id) {
                storage.animation = placement.animation;
            }
        }
        session.sync_scene_effects();
        if persistent.no_items {
            // Queued ahead of the scene's own requests; applied once the
            // session knows its character (drain_campaign_requests).
            session
                .world
                .campaign_requests
                .push(crate::scene_world::campaign_live::CampaignRequest::NoItems);
        }
        if let Err(e) = session.runtime.initialize_actions(&mut session.world) {
            session.fail_scripts(format!("initialize: {e:?}"));
        }
        // PackMan pools: read from the user's Magicka.exe, when present.
        if let Ok(packs) = crate::ruleset::SurvivalPacks::for_content(content) {
            session.set_survival_packs(packs);
        }
        let ruleset = session.runtime.layout().ruleset.clone();
        session
            .world
            .initialize_ruleset(ruleset.as_ref(), &mut session.runtime.variables)
            .map_err(SessionError::World)?;
        session.apply_versus_requests();
        if let Err(e) = session.runtime.initialize_wave_actions(&mut session.world) {
            session.fail_scripts(format!("wave initialize: {e:?}"));
        }
        session.drain();
        Ok(session)
    }

    /// Level.ChangeScene for the local player: GameScene.Destroy of the old
    /// scene (effects, physics and entities cleared; NPCs not carried), then
    /// LoadLevel/Initialize of the target with the same Avatar moved to the
    /// spawn point. Level counters/timers and the avatar persist.
    pub fn change_scene(
        &mut self,
        spawn: crate::scene_flow::SpawnPoint,
    ) -> Result<(), SessionError> {
        self.change_scene_saving(spawn, false)
    }

    /// Level.ChangeScene with GoToScene's saveNPCs. GameScene.Destroy always
    /// stores animated-part states and stores entities only with saveNPCs;
    /// a revisited scene's Initialize restores them and keeps its triggers.
    pub fn change_scene_saving(
        &mut self,
        spawn: crate::scene_flow::SpawnPoint,
        save_npcs: bool,
    ) -> Result<(), SessionError> {
        let departing = SavedScene {
            runtime: self.runtime.clone(),
            animations: self.world.collision.playback_states(),
            entities: if save_npcs {
                self.world.store_saved_entities()
            } else {
                Vec::new()
            },
        };
        let network_epoch = self.next_network_epoch()?;
        let start = crate::play_content::load_level_scene(
            &self.content,
            &self.level_file,
            Some(spawn.scene),
        )
        .map_err(SessionError::Content)?;
        let mut avatar = self.world.players[0]
            .take()
            .ok_or_else(|| SessionError::World("no player avatar".into()))?;
        let mut body = self.world.physics.bodies[avatar.body].clone();
        let old_handle = avatar.handle;
        // CharacterBody.MoveTo/Movement = 0; Avatar.Events = null.
        body.set_movement([0.0; 3]);
        avatar.events.clear();
        avatar.current_spell = None;
        let variables = std::mem::take(&mut self.runtime.variables);
        let scripts_disabled = self.script_error.as_deref() == Some("scripts disabled");
        let mut next = Self::build(
            &self.content,
            &self.level_file,
            start,
            spawn.locations,
            body,
            Box::new(move |handle, index| {
                avatar.handle = handle;
                avatar.body = index;
                avatar
            }),
            variables,
            self.play_time,
            Persistent {
                spawn_override: None,
                spawn_players: spawn.spawn_players,
                // Level.GoToScene applies NoItems only with no current scene.
                no_items: false,
                // PlayState.ChangingScene runs after Initialize; resetting the
                // skipped flag first is equivalent (startup actions that
                // begin or end a cutscene also clear it).
                cutscene: {
                    let mut c = self.world.cutscene;
                    c.changing_scene();
                    c
                },
                render_transition: self.world.render_transition,
            },
            &self.character,
        )?;
        let mut size_handles = vec![(old_handle, next.avatar().handle)];
        for slot in 1..self.world.players.len() {
            if let Some(mut remote) = self.world.players[slot].take() {
                let old_handle = remote.handle;
                let mut remote_body = self.world.physics.bodies[remote.body].clone();
                let placement = next.player_placement(&remote_body, spawn.locations, slot);
                remote_body.move_to(placement.position, placement.orientation);
                if spawn.spawn_players && !remote.dead() {
                    remote_body.set_desired_direction(placement.orientation.forward());
                    remote.set_events(&mut remote_body, None);
                    remote.ai.path.clear();
                }
                remote_body.set_movement([0.; 3]);
                remote.stop_mouse_move();
                remote.events.clear();
                remote.current_spell = None;
                next.world
                    .add_player(slot, remote_body, move |handle, index| {
                        remote.handle = handle;
                        remote.body = index;
                        remote
                    });
                size_handles.push((
                    old_handle,
                    next.world.players[slot].as_ref().unwrap().handle,
                ));
            }
        }
        next.remote_characters = self.remote_characters.clone();
        let mut sizes = std::mem::take(&mut self.world.size_effects);
        sizes.transition_players(&size_handles, &mut self.world);
        next.world.size_effects = sizes;
        if scripts_disabled {
            next.disable_scripts();
        }
        next.scene_changes = self.scene_changes + 1;
        next.network_epoch = network_epoch;
        next.checkpoint = self.checkpoint.take();
        let mut saved_scenes = std::mem::take(&mut self.saved_scenes);
        saved_scenes.insert(self.scene_name.clone(), departing);
        if let Some(saved) = saved_scenes.remove(&next.scene_name) {
            if let Err(e) = next
                .world
                .collision
                .restore_playback(&saved.animations, &magicka_content::skinned::IDENTITY)
            {
                next.world.log.push(format!("saved animations: {e:?}"));
            }
            next.world.add_saved_entities(saved.entities);
            next.runtime.restore_revisit(&saved.runtime);
        }
        next.saved_scenes = saved_scenes;
        // Player.DeadAge belongs to the persistent player, not GameScene.
        // Keep longest-dead Revive priority across ordinary scene changes.
        next.world.dead_age = self.world.dead_age.clone();
        // PlayState.mSpawnFairies outlives GameScenes; SpawnFairies re-arms
        // the flagged fairy at the new scene's Initialize.
        next.world.spawn_fairies = self.world.spawn_fairies.clone();
        if spawn.spawn_players {
            next.world.apply_spawn_fairies();
        }
        next.character = std::mem::take(&mut self.character);
        next.player_ids = self.player_ids.clone();
        next.player_profiles = self.player_profiles.clone();
        next.network_mode = self.network_mode;
        next.parked_players = self.parked_players.clone();
        next.camera_slots = self.camera_slots.clone();
        next.checkpoint_generation = self.checkpoint_generation;
        next.unlocked_magicks = self.unlocked_magicks.clone();
        next.unequip_weapons_section = self.unequip_weapons_section;
        next.default_avatar_section = self.default_avatar_section;
        next.world.died_in_level_flag = self.world.died_in_level_flag;
        // PlayState's cutscene and UIEnabled outlive GameScenes: only
        // checkpoint restarts and state restores call EndCutscene.
        next.world.cutscene = self.world.cutscene;
        next.world.flags.cutscene = self.world.flags.cutscene;
        next.world.ui_enabled = self.world.ui_enabled;
        next.world.flags.staff_light = self.world.flags.staff_light;
        next.credits = self.credits.take();
        next.credits_fade = self.credits_fade;
        next.credits_finished = self.credits_finished;
        next.screen = self.screen;
        for player in next.world.players.iter_mut().flatten() {
            player.sync_immortality(next.world.flags.cutscene);
        }
        // PlayState state restore re-applies the section to new avatars.
        if next.unequip_weapons_section {
            next.remove_all_weapons()?;
        }
        next.definitions = self.definitions.clone();
        next.world.scripted_magicks = self.world.scripted_magicks;
        next.definitions_revision = self.definitions_revision;
        next.registered_magicks = self.registered_magicks.clone();
        next.registered_unlocked = self.registered_unlocked.clone();
        next.mod_state = self.mod_state.clone();
        *self = next;
        Ok(())
    }

    /// Where a player's revival fairy effect really is, for checks and tools.
    pub fn fairy_effect_position(&self, slot: usize) -> Option<[f32; 3]> {
        let instance = self.world.fairy_effect_instance(slot)?;
        let reference = self.scene_effect_refs.get(&instance)?;
        self.effects.effect_position(reference)
    }

    fn sync_scene_effects(&mut self) {
        use crate::scene_world::SceneEffectCommand;
        for command in std::mem::take(&mut self.world.effect_commands) {
            match command {
                SceneEffectCommand::Start {
                    instance,
                    effect,
                    transform,
                } => {
                    if let Some(reference) = self.scene_effect_refs.get_mut(&instance) {
                        self.effects.stop(reference);
                    }
                    if let Some(reference) = self.effects.start(effect, transform) {
                        self.scene_effect_refs.insert(instance, reference);
                    } else {
                        self.world
                            .log
                            .push(format!("scene effect presentation unavailable: {effect}"));
                    }
                }
                SceneEffectCommand::Stop { instance } => {
                    if let Some(reference) = self.scene_effect_refs.get_mut(&instance) {
                        self.effects.stop(reference);
                    }
                }
                SceneEffectCommand::Move {
                    instance,
                    position,
                    direction,
                } => {
                    if let Some(reference) = self.scene_effect_refs.get_mut(&instance) {
                        self.effects
                            .update_position_direction(reference, position, direction);
                    }
                }
            }
        }
        let collision = &self.world.collision;
        let shields = &self.world.shields;
        for (&id, storage) in &mut self.world.effects.effects {
            let Some(reference) = self.scene_effect_refs.get_mut(&storage.effect.instance) else {
                continue;
            };
            storage.effect.active = self.effects.is_active(reference);
            if !storage.effect.active {
                continue;
            }
            let part = storage.animation.and_then(|animation| {
                self.level_effects
                    .iter()
                    .find(|e| e.id == id && e.animation == Some(animation))
                    .and_then(|e| e.part.clone())
            });
            let descriptor = crate::level_effects::LevelEffect {
                id,
                hash: storage.effect.effect,
                animation: storage.animation,
                effect: *reference,
                local: storage.transform,
                range: storage.range,
                part,
            };
            let transform = crate::level_effects::world_transform(
                &descriptor,
                collision.parts(),
                |origin, delta| {
                    let segment = magicka_sim::spray_pulse::Segment { origin, delta };
                    let level = collision
                        .collision()
                        .segment_intersect(segment, false)
                        .map(|h| h.hit.fraction);
                    shields
                        .iter()
                        .filter_map(|s| s.shield.geometry().segment_fraction(segment))
                        .fold(level, |a, b| Some(a.map_or(b, |a| a.min(b))))
                },
            );
            self.effects.update_orientation(reference, transform);
            storage.effect.transform = transform;
        }
        for placement in &mut self.level_effects {
            placement.effect = self
                .world
                .effects
                .effects
                .get(&placement.id)
                .and_then(|e| self.scene_effect_refs.get(&e.effect.instance))
                .copied()
                .unwrap_or_default();
        }
        // A reference lives as long as its effect. A hand-kept list of the instances
        // worth keeping lost the revival fairy's: its Move commands then did nothing and
        // the effect stayed where it began (research/revival-fairy.md).
        let effects = &self.effects;
        self.scene_effect_refs
            .retain(|_, reference| effects.effect_position(reference).is_some());
    }

    /// Particle-host state for a native named GameScene effect ID.
    pub fn scene_effect_active(&self, id: i32) -> bool {
        self.world
            .effects
            .effects
            .get(&id)
            .and_then(|e| self.scene_effect_refs.get(&e.effect.instance))
            .is_some_and(|r| self.effects.is_active(r))
    }

    /// Stop every level-placed effect (test isolation of spell effects,
    /// which share the effect and particle systems with them).
    pub fn stop_level_effects(&mut self) {
        use crate::effect_action::EffectActionHost;
        for id in self.level_effects.iter().map(|p| p.id) {
            let _ = self.world.stop_effect(id);
        }
        self.sync_scene_effects();
    }

    /// Level-placed effects (GameScene.mEffects) still active.
    pub fn active_level_effects(&self) -> usize {
        self.level_effects
            .iter()
            .filter(|e| self.effects.is_active(&e.effect))
            .count()
    }

    /// PlayState.SkipCutscene (keyboard OnKeyDown with a qualifying key, see
    /// `cutscene::key_skips_cutscene`): only while IsInCutscene. The first
    /// call shows the tip; the next runs the cutscene's OnSkip trigger.
    pub fn skip_cutscene(&mut self) {
        if !self.world.cutscene.in_cutscene() {
            return;
        }
        let client = self.world.role == magicka_sim::damage_delivery::NetworkRole::Client;
        let Some(trigger) = self.world.cutscene.skip(client) else {
            return;
        };
        if self.script_error.is_some() {
            return;
        }
        let role = self.world.role;
        match self
            .runtime
            .execute_trigger(trigger, None, false, role, &mut self.world)
        {
            Ok(_) | Err(crate::script_runtime::ScriptRuntimeError::MissingTrigger(_)) => {}
            Err(e) => self.fail_scripts(format!("skip cutscene: {e:?}")),
        }
        self.drain();
    }

    /// RenderManager transition overlay: colour and amount (0 clear .. 1
    /// fully coloured), or 0 without a current transition.
    pub fn transition_fade(&self) -> f32 {
        self.world
            .render_transition
            .as_ref()
            .map_or(0.0, |t| t.amount())
    }

    pub fn transition_color(&self) -> [u8; 4] {
        self.world
            .render_transition
            .as_ref()
            .map_or([0, 0, 0, 255], |t| t.color)
    }

    // -- Developer tooling (debug menu, autopilot); not original features. --

    /// PlayState.Restart(Checkpoint) → State.ApplyState (offline subset):
    /// rebuild the checkpoint scene with a fresh avatar at the checkpoint
    /// matrices, restore the script runtime (ignored triggers reset), replay
    /// the executed actions through QuickExecute (startup actions: spawns
    /// return with their saved health) and restore level-part playback.
    /// Returns false without a checkpoint (the caller restarts the level).
    pub fn restart_at_checkpoint(&mut self) -> Result<bool, SessionError> {
        let Some(cp) = self.checkpoint.clone() else {
            return Ok(false);
        };
        let network_epoch = self.next_network_epoch()?;
        let mut next = Self::load_at(
            &self.content,
            &self.level_file,
            &self.character,
            Some(cp.scene),
            Some(cp.matrices),
        )?;
        for (&slot, character) in &self.remote_characters {
            let (mut body, make) = Self::prepare_player(&self.content, character, slot)?;
            let (mut position, orientation) = locator_parts(&slot_matrix(&cp.matrices, slot));
            if let Some(hit) = next.world.collision.collision().segment_intersect(
                Segment {
                    origin: position,
                    delta: [0., -5., 0.],
                },
                false,
            ) {
                position = hit.hit.position;
                position[1] += body.length * 0.5 + body.radius;
            }
            body.move_to(position, orientation);
            body.set_desired_direction(orientation.forward());
            next.world.add_player(slot, body, make);
        }
        next.remote_characters = self.remote_characters.clone();
        let scripts_disabled = self.script_error.as_deref() == Some("scripts disabled");
        next.runtime = cp.runtime.clone();
        next.runtime.skip_unsupported = true;
        next.runtime.reset_triggers(&cp.ignored);
        // State.ApplyState: the music fields return, StopMusic then PlayMusic.
        next.world.flags.music = cp.music;
        next.world.flags.music_focus_value = cp.music_focus_value;
        next.world
            .audio_requests
            .push(crate::audio::AudioRequest::StopMusic);
        if let Some((bank, cue)) = cp.music {
            next.world
                .audio_requests
                .push(crate::audio::AudioRequest::Music { bank, cue });
        }
        next.world
            .restore_checkpoint_physics(&cp.physics)
            .map_err(SessionError::World)?;
        // EntityStateStorage.Restore + AddSavedEntities: permanent pickables.
        next.world
            .restore_checkpoint_pickables(&cp.pickables)
            .map_err(SessionError::World)?;
        for index in next.runtime.history_excluding(&cp.ignored) {
            if let Err(e) = next.runtime.quick_execute(index, &mut next.world) {
                next.world
                    .log
                    .push(format!("checkpoint replay {index}: {e:?}"));
            }
        }
        if let Err(e) = next
            .world
            .collision
            .restore_playback(&cp.animations, &magicka_content::skinned::IDENTITY)
        {
            next.world.log.push(format!("checkpoint animations: {e:?}"));
        }
        if scripts_disabled {
            next.disable_scripts();
        }
        // Camera.Release(0.001) then MagickCamera.State.ApplyState (after
        // Level.State.ApplyState); the followed entity is gone with the
        // cleared EntityManager, its last position stays.
        next.world.camera.release(0.001);
        next.world.camera.apply_state(&cp.camera);
        next.world.camera_follow = None;
        if let (Some(dialogs), Some(state)) = (next.world.dialogs.as_mut(), cp.dialogs.as_ref()) {
            dialogs
                .restore_disk_boundary(state)
                .map_err(SessionError::World)?;
        }
        let checkpoint_players = cp.players.clone();
        let (unequip_weapons_section, default_avatar_section) = cp.sections();
        next.saved_scenes = cp.saved_scenes.clone();
        next.checkpoint = Some(cp);
        next.play_time = self.play_time;
        next.scene_changes = self.scene_changes + 1;
        next.network_epoch = network_epoch;
        next.drain();
        next.player_ids = self.player_ids.clone();
        next.player_profiles = self.player_profiles.clone();
        next.network_mode = self.network_mode;
        next.parked_players = self.parked_players.clone();
        next.camera_slots = self.camera_slots.clone();
        next.checkpoint_generation = self.checkpoint_generation;
        next.unlocked_magicks = self.unlocked_magicks.clone();
        next.apply_checkpoint_players(&checkpoint_players)?;
        // State.ApplyState: the checkpoint's fairy flags re-arm or kill fairies.
        next.world.apply_spawn_fairies();
        // State.ApplyState (not mWorldSync): the captured section flags.
        next.unequip_weapons_section = unequip_weapons_section;
        next.default_avatar_section = default_avatar_section;
        next.world.died_in_level_flag = self.world.died_in_level_flag;
        // PlayState state restore re-applies the section to new avatars.
        if next.unequip_weapons_section {
            next.remove_all_weapons()?;
        }
        if !self.avatar().playing {
            let player = next.world.players[0].as_mut().unwrap();
            player.playing = false;
            next.world.physics.bodies[player.body].active = false;
            if let Some(e) = next
                .world
                .entities
                .iter_mut()
                .find(|e| e.handle == player.handle)
            {
                e.added = false;
                e.removed = true;
                e.unique_id = 0;
            }
        }
        next.definitions = self.definitions.clone();
        next.world.scripted_magicks = self.world.scripted_magicks;
        next.definitions_revision = self.definitions_revision;
        next.registered_magicks = self.registered_magicks.clone();
        next.registered_unlocked = self.registered_unlocked.clone();
        next.mod_state = self.mod_state.clone();
        *self = next;
        Ok(true)
    }

    /// The ended ruleset's statistics sheet (shown while
    /// `end_game.statistics`).
    pub fn ruleset_results(&self) -> Option<crate::ruleset::RulesetResults> {
        self.world.ruleset_results(&self.runtime.variables)
    }

    /// Supply the enabled PackMan reward pools (filtered for indoor scenes).
    pub fn set_survival_packs(&mut self, packs: crate::ruleset::SurvivalPacks) {
        self.world.survival_packs = Some(packs.for_scene(self.world.scene_indoors));
    }

    /// The campaign level to load after a completed level (SubMenuEndGame →
    /// next CampaignNode), when the end-of-level step has been reached.
    pub fn next_level(&self) -> Option<&'static str> {
        if !self.end_game.level_complete {
            return None;
        }
        crate::end_game::next_campaign_level(&self.level_file).map(|(l, _)| l)
    }

    /// Restart from the initial level state without dropping seated players or
    /// parked profiles. Equipment resets to each playing wizard's robe defaults.
    pub fn restart_level(&mut self) -> Result<(), SessionError> {
        let level = self.level_file.clone();
        self.change_level(&level)?;
        let players: Vec<_> = self
            .world
            .players
            .iter()
            .enumerate()
            .filter(|(_, p)| p.as_ref().is_some_and(|p| p.playing))
            .filter_map(|(slot, _)| self.player_robe(slot).map(|robe| (slot, robe.to_owned())))
            .collect();
        for (slot, robe) in players {
            self.respawn_player(slot, &robe)?;
        }
        Ok(())
    }

    /// A level/campaign transition carries held equipment; it is not a respawn.
    pub fn change_level(&mut self, level: &str) -> Result<(), SessionError> {
        let epoch = self.next_network_epoch()?;
        let mut next = Self::load(&self.content, level, &self.character)?;
        for (slot, player) in self.world.players.iter().enumerate() {
            let Some(player) = player else { continue };
            let robe = if slot == 0 {
                &self.character
            } else {
                self.remote_characters
                    .get(&slot)
                    .ok_or_else(|| SessionError::World("missing remote robe".into()))?
            };
            if slot != 0 {
                next.join_player_at_start(slot, robe)?;
            }
            let name = |wanted| {
                player
                    .items
                    .iter()
                    .find(|(s, _)| *s == wanted)
                    .map(|(_, item)| item.definition.name.as_str())
                    .ok_or_else(|| SessionError::Template("missing carried equipment".into()))
            };
            next.set_player_loadout(slot, robe, name(1)?, name(0)?)?;
        }
        // Carry the item instances too: stored spells and cooldowns belong to
        // held equipment and are not reset by loading another level.
        for (slot, player) in self.world.players.iter_mut().enumerate() {
            let Some(player) = player else { continue };
            let target = next.world.players[slot].as_mut().unwrap();
            target.playing = player.playing;
            next.world.physics.bodies[target.body].active = player.playing;
            if !target.playing {
                if let Some(e) = next
                    .world
                    .entities
                    .iter_mut()
                    .find(|e| e.handle == target.handle)
                {
                    e.added = false;
                    e.removed = true;
                    e.unique_id = 0;
                }
            }
            target.items = std::mem::take(&mut player.items);
            for (_, item) in &mut target.items {
                item.frame = None;
            }
        }
        next.player_ids = self.player_ids.clone();
        next.player_profiles = self.player_profiles.clone();
        next.network_mode = self.network_mode;
        next.parked_players = self.parked_players.clone();
        next.camera_slots = self.camera_slots.clone();
        next.checkpoint_generation = self.checkpoint_generation;
        next.unlocked_magicks = self.unlocked_magicks.clone();
        next.unequip_weapons_section = self.unequip_weapons_section;
        next.default_avatar_section = self.default_avatar_section;
        next.world.died_in_level_flag = self.world.died_in_level_flag;
        // PlayState state restore re-applies the section to new avatars.
        if next.unequip_weapons_section {
            next.remove_all_weapons()?;
        }
        next.network_epoch = epoch;
        if self.script_error.as_deref() == Some("scripts disabled") {
            next.disable_scripts();
        }
        next.definitions = self.definitions.clone();
        next.world.scripted_magicks = self.world.scripted_magicks;
        next.definitions_revision = self.definitions_revision;
        next.registered_magicks = self.registered_magicks.clone();
        next.registered_unlocked = self.registered_unlocked.clone();
        next.mod_state = self.mod_state.clone();
        *self = next;
        Ok(())
    }

    pub fn level_file(&self) -> &str {
        &self.level_file
    }

    /// Lowercase scene names (file stems) listed by the level file.
    pub fn level_scene_names(&self) -> Vec<String> {
        let levels = self.content.join("Levels");
        let Some(path) = crate::play_content::resolve(&levels, Path::new(&self.level_file)) else {
            return Vec::new();
        };
        let Ok(level) = crate::play_content::read_level(&path) else {
            return Vec::new();
        };
        level
            .scenes
            .iter()
            .filter_map(|s| {
                Path::new(&s.replace('\\', "/"))
                    .file_stem()
                    .map(|n| n.to_string_lossy().to_ascii_lowercase())
            })
            .collect()
    }

    /// Switch scenes as a ChangeScene action with `spawnPoint` would
    /// (locators `<spawn_point>0..3`), without a transition.
    pub fn change_scene_named(
        &mut self,
        scene: &str,
        spawn_point: &str,
    ) -> Result<(), SessionError> {
        let spawn_point = spawn_point.to_ascii_lowercase();
        self.change_scene(crate::scene_flow::SpawnPoint {
            scene: custom_hash(&scene.to_ascii_lowercase()),
            locations: std::array::from_fn(|i| custom_hash(&format!("{spawn_point}{i}"))),
            spawn_players: true,
        })
    }

    /// Trigger ids (as written) of the current scene file, in file order.
    pub fn trigger_names(&self) -> Vec<String> {
        let Ok(bytes) = std::fs::read(&self.scene_path) else {
            return Vec::new();
        };
        let Ok(root) = crate::scene_script::xml(&bytes) else {
            return Vec::new();
        };
        root.children
            .iter()
            .filter(|n| n.name.eq_ignore_ascii_case("trigger"))
            .filter_map(|n| {
                n.attributes
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("id"))
                    .map(|(_, v)| v.clone())
            })
            .collect()
    }

    /// GameScene.ExecuteTrigger by name; `ignore_conditions` as the original
    /// argument. Returns whether the trigger ran.
    pub fn run_trigger(&mut self, name: &str, ignore_conditions: bool) -> Result<bool, String> {
        let id = custom_hash(&name.to_ascii_lowercase());
        let role = self.world.role;
        let ran = self
            .runtime
            .execute_trigger(id, None, ignore_conditions, role, &mut self.world)
            .map_err(|e| format!("{e:?}"))?;
        self.drain();
        Ok(ran)
    }

    /// Host scripting: define a native trigger action (class name plus
    /// properties, see `script_schema`) in the current scene. The index can
    /// be invoked repeatedly; it lives as long as the scene's runtime.
    pub fn define_script_action(
        &mut self,
        call: &crate::script_schema::ScriptCall,
    ) -> Result<usize, String> {
        self.runtime
            .define_action(call, &mut self.world)
            .map_err(|e| format!("{e:?}"))
    }

    /// Host scripting: Action.OnTrigger on a defined action, executing it
    /// now unless it has a Delay. Returns the executions performed now.
    pub fn invoke_script_action(
        &mut self,
        index: usize,
        sender: Option<u16>,
    ) -> Result<usize, String> {
        let role = self.world.role;
        let ran = self
            .runtime
            .invoke_action(index, sender, role, &mut self.world)
            .map_err(|e| format!("{e:?}"));
        self.drain();
        ran
    }

    /// Host scripting: define and invoke a one-off action.
    pub fn call_script_action(
        &mut self,
        call: &crate::script_schema::ScriptCall,
        sender: Option<u16>,
    ) -> Result<usize, String> {
        let index = self.define_script_action(call)?;
        self.invoke_script_action(index, sender)
    }

    /// Host scripting: define a native condition for `script_condition_met`.
    pub fn define_script_condition(
        &mut self,
        call: &crate::script_schema::ScriptCall,
    ) -> Result<usize, String> {
        self.runtime
            .define_condition::<crate::scene_world::WorldError>(call)
            .map_err(|e| format!("{e:?}"))
    }

    /// Host scripting: Condition.IsMet against the live scene.
    pub fn script_condition_met(&self, index: usize, sender: Option<u16>) -> Result<bool, String> {
        self.runtime
            .condition_met(index, sender, &self.world)
            .map_err(|e| format!("{e:?}"))
    }

    /// Host scripting: add a `<Trigger>` written in scene XML to the current
    /// scene; it then runs like an authored trigger. Returns its ID hash.
    pub fn add_script_trigger(&mut self, xml: &str) -> Result<i32, String> {
        let element =
            crate::scene_script::XmlElement::parse(xml.as_bytes()).map_err(|e| format!("{e:?}"))?;
        self.runtime
            .add_trigger(&element, false, &mut self.world)
            .map_err(|e| format!("{e:?}"))
    }

    /// GameScene name (lowercase file stem) of the current scene.
    pub fn scene_name(&self) -> &str {
        &self.scene_name
    }

    /// Level.GoToScene: with a Fade/CrossFade transition and positive time,
    /// RenderManager fades out (black) over `time`; TransitionEnd runs
    /// ChangeScene. Otherwise ChangeScene is a load task (next frame here).
    /// After the switch, unless Level.ClearTransition ran, EndTransition
    /// starts the matching fade in (Transitions.None clears the overlay).
    /// Returns whether the scene changed.
    fn advance_scene_transition(&mut self, dt: f32) -> bool {
        const BLACK: [u8; 4] = [0, 0, 0, 255];
        let mut switch = None;
        if let Some(pending) = self.world.pending_scene.take() {
            self.world.transition_cleared = false;
            if pending.transition <= 0 || pending.time < f32::EPSILON {
                switch = Some(pending);
            } else {
                self.world.render_transition = crate::render_transition::RenderTransition::start(
                    pending.transition,
                    true,
                    BLACK,
                    pending.time,
                );
                self.pending_scene = Some(pending);
            }
        }
        let ended = crate::render_transition::update(&mut self.world.render_transition, dt);
        if ended {
            switch = switch.or(self.pending_scene.take());
        }
        let Some(pending) = switch else {
            return false;
        };
        if let Err(e) = self.change_scene_saving(pending.spawn, pending.save_npcs) {
            self.fail_scripts(format!("change scene: {e:?}"));
            return false;
        }
        // mNextSceneTransition != Invalid (ClearTransition sets Invalid).
        if pending.transition >= 0 && !self.world.transition_cleared {
            self.world.render_transition = crate::render_transition::RenderTransition::start(
                pending.transition,
                false,
                BLACK,
                pending.time,
            );
        }
        true
    }

    /// Developer/demo helper (not an original feature): run a real
    /// `<spawn type=... order=... reactTo="proximity"/>` SpawnAction at the
    /// second player locator, then place the NPC `distance` m ahead of the
    /// avatar.
    pub fn spawn_npc(&mut self, template: &str, order: &str, distance: f32) -> Result<u16, String> {
        let area = self.start_area.clone().ok_or("level has no start area")?;
        let xml = format!(
            r#"<Scene><trigger id="t"><then><spawn type="{template}" area="{area}1" order="{order}" reactTo="proximity"/></then></trigger></Scene>"#
        );
        let layout =
            SceneScriptLayout::read(xml.as_bytes(), false).map_err(|e| format!("{e:?}"))?;
        let Some(crate::scene_script::ActionDefinition::Spawn(action)) =
            layout.actions.into_iter().next()
        else {
            return Err("not a spawn action".into());
        };
        let mut state = crate::spawn_action::SpawnState::default();
        action
            .initialize(&mut state, &mut self.world)
            .map_err(|e| format!("{e:?}"))?;
        action
            .execute(&mut state, &mut self.world)
            .map_err(|e| format!("{e:?}"))?;
        let handle = state
            .spawned
            .first()
            .and_then(|s| s.handle)
            .ok_or("nothing spawned")?;
        let (position, forward) = (self.body().position, self.body().direction());
        let body_index = self
            .world
            .npcs
            .get(&handle)
            .ok_or("spawned NPC is not live")?
            .body;
        let body = &mut self.world.physics.bodies[body_index];
        let target = [
            position[0] + forward[0] * distance,
            body.position[1].max(position[1]),
            position[2] + forward[2] * distance,
        ];
        let orientation = body.orientation;
        body.move_to(target, orientation);
        Ok(handle)
    }

    fn fail_scripts(&mut self, message: String) {
        if self.script_error.is_none() {
            self.events.push(SessionEvent::ScriptError(message.clone()));
            self.script_error = Some(message);
        }
    }

    /// Run the world without scene scripts (physics/presentation checks).
    pub fn disable_scripts(&mut self) {
        self.script_error = Some("scripts disabled".into());
    }

    pub fn avatar(&self) -> &PlayerAvatar {
        self.world.players[0].as_ref().expect("player avatar")
    }
    pub fn body(&self) -> &CharacterBody {
        &self.world.physics.bodies[self.avatar().body]
    }
    /// Item.Update (attached): chant effects for the charged spell queue's
    /// elements, re-oriented every frame and stopped when the element leaves;
    /// the definition's effects while visible (not invisible, not
    /// HideEffects), restarted at attach0 whenever they are not active.
    fn item_effect_presentation(&mut self) {
        let mut seen = BTreeSet::new();
        for p in self.world.players.iter().flatten() {
            let invisible = p.character.combat.health.invisibility_timer > 0.0;
            for (slot, item) in &p.items {
                let Some(frame) = item.frame else { continue };
                for index in 200..204 {
                    if let Some(reference) =
                        self.item_effects.get_mut(&(p.handle, *slot + 1000, index))
                    {
                        self.effects.update_orientation(reference, frame.attach1);
                    }
                }
                let mut elements = magicka_sim::spells::Elements::NONE;
                for spell in item.spells.queue() {
                    elements |= spell.element;
                }
                item_chants::sync(
                    &mut self.effects,
                    &mut self.item_effects,
                    &mut seen,
                    (p.handle, *slot),
                    frame,
                    elements,
                );
                let show = !invisible && !item.definition.hide_effect;
                for index in 200..204 {
                    let key = (p.handle, *slot, index);
                    if let Some(reference) = self.item_effects.get_mut(&key) {
                        if show && self.effects.is_active(reference) {
                            seen.insert(key);
                            self.effects.update_position_direction(
                                reference,
                                frame.effect0,
                                frame.direction(),
                            );
                        }
                    }
                }
                for (k, name) in item.definition.effects.iter().take(4).enumerate() {
                    let key = (p.handle, *slot, 100 + k);
                    if !show {
                        if let Some(mut r) = self.item_effects.remove(&key) {
                            self.effects.stop(&mut r);
                        }
                        continue;
                    }
                    seen.insert(key);
                    let live = self.item_effects.get_mut(&key).is_some_and(|r| {
                        self.effects
                            .update_position_direction(r, frame.effect0, frame.direction())
                    });
                    if !live {
                        let hash = custom_hash(&name.to_ascii_lowercase());
                        if let Some(r) = self.effects.start_at(
                            hash,
                            frame.attach0_position(),
                            frame.attach0_forward(),
                        ) {
                            self.item_effects.insert(key, r);
                        }
                    }
                }
            }
        }
        // NPC-held items: the specialized absorption queue belongs to equipment zero.
        for npc in self.world.npcs.values() {
            let invisible = npc.character.combat.health.invisibility_timer > 0.0;
            for held in &npc.held_effects {
                let Some(frame) = held.frame else { continue };
                let elements = if held.slot == 0 {
                    npc.character
                        .combat
                        .absorb_shield
                        .as_ref()
                        .map(|queue| {
                            queue
                                .queue()
                                .iter()
                                .fold(magicka_sim::spells::Elements::NONE, |elements, spell| {
                                    elements | spell.element
                                })
                        })
                        .unwrap_or(magicka_sim::spells::Elements::NONE)
                } else {
                    magicka_sim::spells::Elements::NONE
                };
                // Native chants depend on attachment/queue, before the passive
                // effect visibility and HideEffects checks.
                item_chants::sync(
                    &mut self.effects,
                    &mut self.item_effects,
                    &mut seen,
                    (npc.handle, held.slot),
                    frame,
                    elements,
                );
                if invisible || held.hide {
                    continue;
                }
                for index in 200..204 {
                    let key = (npc.handle, held.slot, index);
                    if let Some(reference) = self.item_effects.get_mut(&key) {
                        if self.effects.is_active(reference) {
                            seen.insert(key);
                            self.effects.update_position_direction(
                                reference,
                                frame.effect0,
                                frame.direction(),
                            );
                        }
                    }
                }
                for (k, name) in held.effects.iter().take(4).enumerate() {
                    let key = (npc.handle, held.slot, 100 + k);
                    seen.insert(key);
                    let live = self.item_effects.get_mut(&key).is_some_and(|r| {
                        self.effects
                            .update_position_direction(r, frame.effect0, frame.direction())
                    });
                    if !live {
                        let hash = custom_hash(&name.to_ascii_lowercase());
                        if let Some(r) = self.effects.start_at(
                            hash,
                            frame.attach0_position(),
                            frame.attach0_forward(),
                        ) {
                            self.item_effects.insert(key, r);
                        }
                    }
                }
            }
        }
        let gone: Vec<_> = self
            .item_effects
            .keys()
            .filter(|k| !seen.contains(k))
            .copied()
            .collect();
        for key in gone {
            if let Some(mut r) = self.item_effects.remove(&key) {
                self.effects.stop(&mut r);
            }
        }
    }

    /// Live physics-entity template effects as (handle, effect index).
    pub fn live_prop_effects(&self) -> Vec<(u16, usize)> {
        self.prop_effects
            .iter()
            .filter(|(_, r)| self.effects.is_active(r))
            .map(|(&(handle, _, k), _)| (handle, k))
            .collect()
    }

    /// PhysicsEntity template effects. Initialize starts each at
    /// `Multiply(transform, storage.Transform)`; Update re-orients them with
    /// the body transform in the same operand order and restarts any whose
    /// reference ended; Deinitialize (removal, including a damageable prop's
    /// death) stops them.
    fn prop_effect_presentation(&mut self) {
        let mut seen = BTreeSet::new();
        for e in &self.world.entities {
            if !matches!(e.kind, crate::scene_world::EntityKind::Physics { .. })
                || !e.added
                || e.removed
                || e.dead
            {
                continue;
            }
            let Some(name) = &e.template_name else {
                continue;
            };
            let template = self
                .physics_templates
                .entry(name.clone())
                .or_insert_with(|| {
                    crate::play_content::load_physics_template(&self.content, name).ok()
                });
            let Some(template) = template.as_ref() else {
                continue;
            };
            let generation = *self.world.physics_generations.get(&e.handle).unwrap_or(&0);
            for (k, (effect, local)) in template.effects.iter().enumerate() {
                let key = (e.handle, generation, k);
                seen.insert(key);
                let transform = magicka_content::skinned::mul(&e.orientation, local);
                let live = self
                    .prop_effects
                    .get_mut(&key)
                    .is_some_and(|r| self.effects.update_orientation(r, transform));
                if !live {
                    let hash = custom_hash(&effect.to_ascii_lowercase());
                    if let Some(r) = self.effects.start(hash, transform) {
                        self.prop_effects.insert(key, r);
                    }
                }
            }
        }
        let gone: Vec<_> = self
            .prop_effects
            .keys()
            .filter(|k| !seen.contains(k))
            .copied()
            .collect();
        for key in gone {
            if let Some(mut r) = self.prop_effects.remove(&key) {
                self.effects.stop(&mut r);
            }
        }
    }

    /// ArcaneBlade element effects: started at Initialize with the item
    /// transform (Up × range), re-oriented every update, stopped OnRemove.
    fn arcane_blade_presentation(&mut self) {
        let mut live = BTreeSet::new();
        for p in self.world.players.iter().flatten() {
            for blade in &p.arcane_blades {
                let key = (p.handle, blade.id);
                live.insert(key);
                match self.arcane_blade_effects.get_mut(&key) {
                    Some(refs) => {
                        for r in refs {
                            self.effects.update_orientation(r, blade.effect_transform);
                        }
                    }
                    None => {
                        let refs = blade
                            .element_effects()
                            .filter_map(|name| {
                                self.effects
                                    .start(custom_hash(name), blade.effect_transform)
                            })
                            .collect();
                        self.arcane_blade_effects.insert(key, refs);
                    }
                }
            }
        }
        let gone: Vec<_> = self
            .arcane_blade_effects
            .keys()
            .filter(|k| !live.contains(k))
            .copied()
            .collect();
        for key in gone {
            for mut r in self.arcane_blade_effects.remove(&key).unwrap_or_default() {
                self.effects.stop(&mut r);
            }
        }
    }

    /// IceSpikes.Initialize: probe the 18 ring points down 3 m; every
    /// IceBarrier_animation clip lasts the ice appear clip's duration (all
    /// five emerge clips are 0.5 s in retail content).
    fn spawn_ice_spikes(&mut self, center: [f32; 3], radius: f32) {
        use magicka_sim::ice_spikes::{probe_origins, IceSpikes, PROBE_DROP, SLOTS};
        let Some(clips) = self.world.barrier_clips else {
            return;
        };
        let origins = probe_origins(center, radius);
        let mut hits = [None; SLOTS];
        for (hit, origin) in hits.iter_mut().zip(origins) {
            *hit = self
                .world
                .collision
                .collision()
                .segment_intersect(
                    Segment {
                        origin,
                        delta: [0.0, -PROBE_DROP, 0.0],
                    },
                    false,
                )
                .map(|h| h.hit.position);
        }
        let world = &mut self.world;
        if let Some(spikes) =
            IceSpikes::initialize(radius, &hits, &[clips.ice_appear; 5], || world.random())
        {
            self.ice_spikes.push(spikes);
        }
    }

    /// Characters drawn with Character.LightningZapRenderData this frame
    /// (`mZapTimer > 0`, not invisible): handle, collision radius, the zap
    /// skeleton's world-space line segments and the LightningHit flash
    /// sphere (research/zap.md).
    pub fn zapped_characters(&self) -> Vec<crate::zap_skeleton::ZapDraw> {
        use crate::character_pose::character_orientation;
        let mut out = Vec::new();
        let mut visit =
            |handle: u16,
             character: &crate::character::RuntimeCharacter,
             rig: Option<&crate::play_content::CharacterRig>,
             controller: &crate::animation_controller::AnimationController,
             body: &magicka_sim::character_physics::CharacterBody| {
                let c = &character.combat;
                if c.zap_timer <= 0.0 || c.health.invisibility_timer > 0.0 {
                    return;
                }
                let Some(rig) = rig else { return };
                let parent = character_orientation(
                    &character.scaled_static_transform(&rig.static_transform),
                    &body.orientation,
                    body.position,
                    -0.5 * body.length - body.radius,
                );
                let skinned = rig.skeleton.skinned_for(controller, &parent);
                // GenerateSkeletonModel scale: 0.125 · radius / model scale.
                let scale = 0.125 * character.definition.radius / rig.static_transform[0];
                let vertices = crate::zap_skeleton::generate(&rig.skeleton.skeleton.bones, scale);
                out.push(crate::zap_skeleton::ZapDraw {
                    handle,
                    radius: character.definition.radius,
                    lines: crate::zap_skeleton::world_lines(&vertices, &skinned),
                    flash: rig
                        .bounding_radius
                        .map(|radius| crate::zap_skeleton::FlashSphere {
                            center: body.position,
                            radius,
                        }),
                });
            };
        for (&handle, n) in &self.world.npcs {
            visit(
                handle,
                &n.character,
                n.rig.as_ref(),
                &n.motion.controller,
                &self.world.physics.bodies[n.body],
            );
        }
        for p in self.world.players.iter().flatten() {
            visit(
                p.handle,
                &p.character,
                p.rig.as_ref(),
                &p.motion.controller,
                &self.world.physics.bodies[p.body],
            );
        }
        out
    }

    /// Character.GetHipAttachOrientation plus the halo's HeightOffset shift.
    /// Only the native hip joint (index1, excluding named hand/leg/mouth joints)
    /// is used. Missing/unresolved rigs are left without presentation.
    pub fn aura_halos(&self) -> Vec<(u16, [f32; 3], [f32; 4])> {
        use crate::character_pose::character_orientation;
        use magicka_content::skinned;
        let mut out = Vec::new();
        let mut visit =
            |handle,
             character: &crate::character::RuntimeCharacter,
             rig: Option<&crate::play_content::CharacterRig>,
             controller: &crate::animation_controller::AnimationController,
             body: &magicka_sim::character_physics::CharacterBody| {
                let Some(color) = character.halo_draw else {
                    return;
                };
                let Some(rig) = rig else { return };
                let height = -0.5 * body.length - body.radius;
                // Unassigned BindJoint has a zero bind matrix (CLR default),
                // hence origin zero rather than an invented root attachment.
                let mut position = [0.0; 3];
                if let Some(hip) = rig.skeleton.skeleton.bones.get(1).filter(|hip| {
                    !["Mouth", "RightAttach", "RightLeg", "LeftAttach", "LeftLeg"]
                        .iter()
                        .any(|name| hip.name.eq_ignore_ascii_case(name))
                }) {
                    let parent = character_orientation(
                        &character.scaled_static_transform(&rig.static_transform),
                        &body.orientation,
                        body.position,
                        height,
                    );
                    let bones = rig.skeleton.skinned_for(controller, &parent);
                    let Ok(bind) = crate::live_collision::inverse(&hip.inverse_bind) else {
                        return;
                    };
                    // The native Y(pi) bind rotation has no effect on the origin.
                    let hip_world = skinned::mul(&bind, &bones[1]);
                    position = [hip_world[12], hip_world[13], hip_world[14]];
                }
                position[1] -= height * 0.5;
                out.push((handle, position, color));
            };
        for (&handle, n) in &self.world.npcs {
            visit(
                handle,
                &n.character,
                n.rig.as_ref(),
                &n.motion.controller,
                &self.world.physics.bodies[n.body],
            );
        }
        for p in self.world.players.iter().flatten() {
            visit(
                p.handle,
                &p.character,
                p.rig.as_ref(),
                &p.motion.controller,
                &self.world.physics.bodies[p.body],
            );
        }
        out
    }

    /// UnderGroundAttack effects: started at Initialize, moved with
    /// UpdatePositionDirection, stopped in OnRemove.
    fn dispatch_underground_visuals(&mut self) {
        use crate::scene_world::UndergroundVisual as V;
        for visual in std::mem::take(&mut self.world.underground_visuals) {
            match visual {
                V::Start {
                    id,
                    effects,
                    position,
                    direction,
                } => {
                    let refs = effects
                        .into_iter()
                        .filter_map(|name| {
                            self.effects
                                .start_at(custom_hash(name), position, direction)
                        })
                        .collect();
                    self.underground_effects.insert(id, refs);
                }
                V::Move {
                    id,
                    position,
                    direction,
                } => {
                    if let Some(refs) = self.underground_effects.get_mut(&id) {
                        let m =
                            crate::particles::create_world(position, direction, [0.0, 1.0, 0.0]);
                        for r in refs {
                            self.effects.update_orientation(r, m);
                        }
                    }
                }
                V::Stop { id } => {
                    for mut r in self.underground_effects.remove(&id).unwrap_or_default() {
                        self.effects.stop(&mut r);
                    }
                }
            }
        }
    }

    /// Running StatusEffect emitters on all characters.
    pub fn active_status_emitters(&self) -> usize {
        self.status_emitters
            .values()
            .filter(|(_, r)| self.effects.is_active(r))
            .count()
    }

    /// Names of the running StatusEffect emitters on a character.
    pub fn status_emitters(&self, handle: u16) -> Vec<&'static str> {
        let mut names: Vec<_> = self
            .status_emitters
            .iter()
            .filter(|((h, _), (_, r))| *h == handle && self.effects.is_active(r))
            .map(|(_, (name, _))| *name)
            .collect();
        names.sort_unstable();
        names
    }

    /// StatusEffect emitters of live characters (`status_visuals`): start,
    /// switch Burning/Cold variants, follow the body, stop with the status.
    /// Dead characters' emitters stop (StopStatusEffects on death).
    fn sync_status_emitters(&mut self) {
        use crate::status_visuals as sv;
        let mut wanted = Vec::new();
        let mut visit = |handle: u16,
                         statuses: &magicka_sim::character_status::CharacterStatuses,
                         radius: f32,
                         length: f32,
                         position: [f32; 3]| {
            for kind in sv::STATUSES {
                if !statuses.has(kind) {
                    continue;
                }
                let Some(e) = statuses.effect(kind).and_then(sv::emitter) else {
                    continue;
                };
                wanted.push((
                    (handle, kind as u16),
                    e,
                    sv::transform(e, radius, length, position),
                ));
            }
        };
        for (&handle, n) in &self.world.npcs {
            if n.dead() || self.world.overkilled.contains(&handle) {
                continue;
            }
            let body = &self.world.physics.bodies[n.body];
            visit(
                handle,
                &n.character.combat.statuses,
                n.definition.radius,
                n.definition.length,
                body.position,
            );
        }
        for p in self.world.players.iter().flatten() {
            if p.dead() {
                continue;
            }
            let body = &self.world.physics.bodies[p.body];
            let d = &p.character.definition;
            visit(
                p.handle,
                &p.character.combat.statuses,
                d.radius,
                d.length,
                body.position,
            );
        }
        let mut old = std::mem::take(&mut self.status_emitters);
        for (key, e, transform) in wanted {
            let entry = match old.remove(&key) {
                Some((name, mut r)) if name != e.name => {
                    self.effects.stop(&mut r);
                    None
                }
                Some((name, mut r)) => {
                    let active = self.effects.update_orientation(&mut r, transform);
                    (active || !e.restart).then_some((name, r))
                }
                None => None,
            };
            let entry = entry.or_else(|| {
                self.effects
                    .start(custom_hash(e.name), transform)
                    .map(|r| (e.name, r))
            });
            if let Some(entry) = entry {
                self.status_emitters.insert(key, entry);
            }
        }
        for (_, (_, mut r)) in old {
            self.effects.stop(&mut r);
        }
    }

    /// Character.mFearEffect: `magick_feared` at the character's position and
    /// facing while IsFeared (UpdatePositionDirection, restarted when it
    /// ended), stopped by RemoveFear and on death.
    fn sync_fear_emitters(&mut self) {
        let mut wanted = Vec::new();
        let bodies = &self.world.physics.bodies;
        let mut visit = |handle: u16, c: &crate::character::RuntimeCharacter, body: usize| {
            if c.fear.is_feared(c.combat.dead) {
                let b = &bodies[body];
                wanted.push((
                    handle,
                    crate::particles::create_world(b.position, b.direction(), [0.0, 1.0, 0.0]),
                ));
            }
        };
        for (&handle, n) in &self.world.npcs {
            visit(handle, &n.character, n.body);
        }
        for p in self.world.players.iter().flatten() {
            visit(p.handle, &p.character, p.body);
        }
        let mut old = std::mem::take(&mut self.fear_emitters);
        for (handle, transform) in wanted {
            let kept = old
                .remove(&handle)
                .filter(|r| self.effects.is_active(r))
                .map(|mut r| {
                    self.effects.update_orientation(&mut r, transform);
                    r
                });
            let r = kept.or_else(|| self.effects.start(custom_hash("magick_feared"), transform));
            if let Some(r) = r {
                self.fear_emitters.insert(handle, r);
            }
        }
        for (_, mut r) in old {
            self.effects.stop(&mut r);
        }
    }

    /// Rain.Execute/Update/OnRemove: `magick_rain` at Camera.Position -
    /// CAMERAOFFSET facing -Z, started with the rain, moved every frame,
    /// stopped once the effect is removed.
    fn sync_rain_effect(&mut self) {
        if self.world.ability_effects.rain_state().is_none() {
            if let Some(mut r) = self.rain_effect.take() {
                self.effects.stop(&mut r);
            }
            return;
        }
        let camera = self.world.camera.position();
        let offset = crate::camera::CAMERA_OFFSET;
        let position = [
            camera[0] - offset[0],
            camera[1] - offset[1],
            camera[2] - offset[2],
        ];
        let transform = crate::particles::create_world(position, [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
        // Update only moves it (a finished effect is not restarted; a recast
        // while it is inactive is not modelled).
        match self.rain_effect.as_mut() {
            Some(r) => {
                self.effects.update_orientation(r, transform);
            }
            None => {
                self.rain_effect = self.effects.start(custom_hash("magick_rain"), transform);
            }
        }
    }

    /// Character.mCharmEffect: `magick_charm` started by Charm, moved with
    /// the character while IsCharmed (not restarted once finished), stopped
    /// by EndCharm.
    fn sync_charm_emitters(&mut self) {
        let mut wanted = Vec::new();
        let bodies = &self.world.physics.bodies;
        let mut visit = |handle: u16, c: &crate::character::RuntimeCharacter, body: usize| {
            if c.is_charmed() {
                let b = &bodies[body];
                wanted.push((
                    handle,
                    crate::particles::create_world(b.position, b.direction(), [0.0, 1.0, 0.0]),
                ));
            }
        };
        for (&handle, n) in &self.world.npcs {
            visit(handle, &n.character, n.body);
        }
        for p in self.world.players.iter().flatten() {
            visit(p.handle, &p.character, p.body);
        }
        let mut old = std::mem::take(&mut self.charm_emitters);
        for (handle, transform) in wanted {
            let r = match old.remove(&handle) {
                Some(mut r) => {
                    self.effects.update_orientation(&mut r, transform);
                    Some(r)
                }
                None => self.effects.start(custom_hash("magick_charm"), transform),
            };
            if let Some(r) = r {
                self.charm_emitters.insert(handle, r);
            }
        }
        for (_, mut r) in old {
            self.effects.stop(&mut r);
        }
    }

    /// Confuse.Update/OnRemove: the `confused` effect sits at the target's
    /// position (identity rotation) and is restarted whenever it is no
    /// longer running; it stops when the Confuse is removed.
    fn sync_confuse_emitters(&mut self) {
        use magicka_sim::magicks::AbilityEffect;
        let world = &self.world;
        let wanted: Vec<_> = world
            .ability_effects
            .effects
            .iter()
            .filter_map(|e| match e {
                AbilityEffect::Confuse(c) => Some(c.target),
                _ => None,
            })
            .filter_map(|target| {
                let body = world
                    .players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == target)
                    .map(|p| p.body)
                    .or_else(|| world.npcs.get(&target).map(|n| n.body))?;
                let position = world.physics.bodies[body].position;
                Some((
                    target,
                    crate::particles::create_world(position, [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
                ))
            })
            .collect();
        let mut old = std::mem::take(&mut self.confuse_emitters);
        for (handle, transform) in wanted {
            let mut r = old.remove(&handle);
            if !r
                .as_mut()
                .is_some_and(|r| self.effects.update_orientation(r, transform))
            {
                r = self.effects.start(custom_hash("confused"), transform);
            }
            if let Some(r) = r {
                self.confuse_emitters.insert(handle, r);
            }
        }
        for (_, mut r) in old {
            self.effects.stop(&mut r);
        }
    }

    /// Character.mLevitateEffect: while the levitate timer runs,
    /// `magick_levitate` sits at the character's feet with its direction,
    /// restarted whenever it is no longer running; StopLevitate stops it.
    fn sync_levitate_emitters(&mut self) {
        let mut wanted = Vec::new();
        let bodies = &self.world.physics.bodies;
        let mut visit = |handle: u16, c: &crate::character::RuntimeCharacter, body: usize| {
            if c.levitation.timer > 0.0 {
                let b = &bodies[body];
                let mut feet = b.position;
                feet[1] += b.height_offset();
                wanted.push((
                    handle,
                    crate::particles::create_world(feet, b.direction(), [0.0, 1.0, 0.0]),
                ));
            }
        };
        for (&handle, n) in &self.world.npcs {
            visit(handle, &n.character, n.body);
        }
        for p in self.world.players.iter().flatten() {
            visit(p.handle, &p.character, p.body);
        }
        let mut old = std::mem::take(&mut self.levitate_emitters);
        for (handle, transform) in wanted {
            let mut r = old.remove(&handle);
            if !r
                .as_mut()
                .is_some_and(|r| self.effects.update_orientation(r, transform))
            {
                r = self
                    .effects
                    .start(custom_hash("magick_levitate"), transform);
            }
            if let Some(r) = r {
                self.levitate_emitters.insert(handle, r);
            }
        }
        for (_, mut r) in old {
            self.effects.stop(&mut r);
        }
    }

    /// GreaseField effects: `magick_grease_particle` once at Initialize and
    /// `magick_grease_burning` (facing -Z) from ignition until it goes out;
    /// both stop when the field is removed or its slot is reused.
    fn sync_grease_emitters(&mut self) {
        let fields: Vec<(usize, u32, [f32; 3], bool)> = self
            .world
            .grease
            .iter()
            .map(|(slot, f)| (slot, f.serial, f.position, f.field.burning))
            .collect();
        let mut old = std::mem::take(&mut self.grease_emitters);
        for (slot, serial, position, burning) in fields {
            let (particle, mut burn) = match old.remove(&slot) {
                Some((s, p, b)) if s == serial => (p, b),
                Some((_, p, b)) => {
                    for mut r in [p, b].into_iter().flatten() {
                        self.effects.stop(&mut r);
                    }
                    let t =
                        crate::particles::create_world(position, [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
                    (
                        self.effects.start(custom_hash("magick_grease_particle"), t),
                        None,
                    )
                }
                None => {
                    let t =
                        crate::particles::create_world(position, [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
                    (
                        self.effects.start(custom_hash("magick_grease_particle"), t),
                        None,
                    )
                }
            };
            match (&mut burn, burning) {
                (None, true) => {
                    let t =
                        crate::particles::create_world(position, [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
                    burn = self.effects.start(custom_hash("magick_grease_burning"), t);
                }
                (Some(r), false) => {
                    self.effects.stop(r);
                    burn = None;
                }
                _ => {}
            }
            self.grease_emitters.insert(slot, (serial, particle, burn));
        }
        for (_, (_, p, b)) in old {
            for mut r in [p, b].into_iter().flatten() {
                self.effects.stop(&mut r);
            }
        }
    }

    /// VortexEntity.mParticleEffect: `magick_vortex` started at Initialize,
    /// then UpdateOrientation at the centre scaled by radius/10·2 (at least
    /// 0.25), never restarted; Deinitialize stops it.
    fn sync_vortex_effects(&mut self) {
        let mut old = std::mem::take(&mut self.vortex_effects);
        for live in &self.world.vortices {
            let v = &live.vortex;
            let mut m = crate::particles::create_world(v.center, [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
            let scale = v.effect_scale();
            for i in [0, 1, 2, 4, 5, 6, 8, 9, 10] {
                m[i] *= scale;
            }
            let r = match old.remove(&live.serial) {
                Some(mut r) => {
                    self.effects.update_orientation(&mut r, m);
                    r
                }
                None => self
                    .effects
                    .start_at(custom_hash("magick_vortex"), v.center, [0.0, 0.0, -1.0])
                    .unwrap_or_default(),
            };
            self.vortex_effects.insert(live.serial, r);
        }
        for (_, mut r) in old {
            self.effects.stop(&mut r);
        }
    }

    /// Character.Update while IsStunned: UpdateOrientation of mStunEffect at
    /// the mouth attach, restarting `stunned` when that fails; it stops once
    /// the timer runs out. The rewrite places it at the capsule top (the
    /// Mouth joint is INFERRED to sit there) and also stops it when the
    /// character dies or leaves.
    fn sync_stun_effects(&mut self) {
        let mut old = std::mem::take(&mut self.stun_effects);
        let world = &self.world;
        let stunned: Vec<(u16, usize)> = world
            .players
            .iter()
            .flatten()
            .filter(|p| p.character.is_stunned())
            .map(|p| (p.handle, p.body))
            .chain(
                world
                    .npcs
                    .iter()
                    .filter(|(_, n)| n.character.is_stunned())
                    .map(|(h, n)| (*h, n.body)),
            )
            .collect();
        for (handle, body) in stunned {
            let b = &self.world.physics.bodies[body];
            let mut top = b.position;
            top[1] += 0.5 * b.length + b.radius;
            let m = crate::particles::create_world(top, b.direction(), [0.0, 1.0, 0.0]);
            let mut r = old.remove(&handle).unwrap_or_default();
            if !self.effects.update_orientation(&mut r, m) {
                r = self
                    .effects
                    .start(custom_hash("stunned"), m)
                    .unwrap_or_default();
            }
            self.stun_effects.insert(handle, r);
        }
        for (_, mut r) in old {
            self.effects.stop(&mut r);
        }
    }

    /// TornadoEntity.mDustEffect: `magick_tornado` at the tornado, scaled
    /// by 0.75. Each Update restarts it when UpdateOrientation fails, so it
    /// loops for the tornado's life; Deinitialize stops it.
    fn sync_tornado_dust(&mut self) {
        let mut old = std::mem::take(&mut self.tornado_dust);
        for live in &self.world.tornadoes {
            let mut m = crate::particles::create_world(
                live.tornado.position,
                [0.0, 0.0, -1.0],
                [0.0, 1.0, 0.0],
            );
            for v in &mut m[..11] {
                *v *= 0.75;
            }
            m[3] = 0.0;
            m[7] = 0.0;
            let mut r = old.remove(&live.serial).unwrap_or_default();
            if !self.effects.update_orientation(&mut r, m) {
                r = self
                    .effects
                    .start(custom_hash("magick_tornado"), m)
                    .unwrap_or_default();
            }
            self.tornado_dust.insert(live.serial, r);
        }
        for (_, mut r) in old {
            self.effects.stop(&mut r);
        }
    }

    /// GreaseSpray.mEffect: `magick_grease_spray` starts at the cast source
    /// along the owner's facing (Execute), follows the cast source each
    /// Update and stops in OnRemove. The server moves it along the swept
    /// direction; a client keeps the raw facing (research/grease.md).
    fn sync_grease_sprays(&mut self) {
        use magicka_sim::grease::{spray_step, spray_yaw};
        let client = self.world.role == magicka_sim::damage_delivery::NetworkRole::Client;
        let sprays = self.world.ability_effects.grease_sprays();
        let mut old = std::mem::take(&mut self.grease_spray_effects);
        for spray in &sprays {
            let source = self
                .world
                .players
                .iter()
                .flatten()
                .find(|p| p.handle == spray.owner)
                .map(|p| {
                    let body = &self.world.physics.bodies[p.body];
                    let facing = body.direction();
                    (crate::avatar::translation(&p.staff_orb), facing)
                });
            // Sprays share one TTL, so the oldest reference belongs to the
            // oldest surviving spray of that owner.
            let existing = old
                .iter()
                .position(|(owner, _)| *owner == spray.owner)
                .map(|i| old.remove(i).1);
            let r = match (existing, source) {
                (Some(mut r), Some((position, facing))) => {
                    let direction = if client {
                        facing
                    } else {
                        let (s, c) = spray_yaw(spray_step(spray.ttl)).sin_cos();
                        [
                            facing[0] * c + facing[2] * s,
                            facing[1],
                            -facing[0] * s + facing[2] * c,
                        ]
                    };
                    self.effects
                        .update_position_direction(&mut r, position, direction);
                    Some(r)
                }
                (Some(r), None) => Some(r),
                (None, Some((position, facing))) => {
                    self.effects
                        .start_at(custom_hash("magick_grease_spray"), position, facing)
                }
                (None, None) => None,
            };
            if let Some(r) = r {
                self.grease_spray_effects.push((spray.owner, r));
            }
        }
        for (_, mut r) in old {
            self.effects.stop(&mut r);
        }
    }

    /// Napalm.mSmokeEffect: `magick_smoke_target` on the focal point while
    /// the strike lives; a recast that moves the target restarts it, and
    /// OnRemove stops it.
    fn sync_napalm_smoke(&mut self) {
        let wanted = self
            .world
            .ability_effects
            .napalm_state()
            .map(|n| (n.target, n.forward));
        match (wanted, self.napalm_smoke.take()) {
            (Some((target, _)), Some((at, r))) if at == target => {
                self.napalm_smoke = Some((at, r));
            }
            (Some((target, forward)), old) => {
                if let Some((_, mut r)) = old {
                    self.effects.stop(&mut r);
                }
                let t = crate::particles::create_world(target, forward, [0.0, 1.0, 0.0]);
                self.napalm_smoke = self
                    .effects
                    .start(custom_hash("magick_smoke_target"), t)
                    .map(|r| (target, r));
            }
            (None, Some((_, mut r))) => self.effects.stop(&mut r),
            (None, None) => {}
        }
    }

    /// PerformanceEnchantment.mEffect: `special_woot_performance` follows a
    /// boosted owner; Withdrawal and OnRemove stop it.
    fn sync_performance_emitters(&mut self) {
        use magicka_sim::magicks::AbilityEffect;
        let world = &self.world;
        let wanted: Vec<(u16, magicka_content::skinned::Mat)> = world
            .ability_effects
            .effects
            .iter()
            .filter_map(|e| match e {
                AbilityEffect::Performance(p) if p.boosted => Some(p.owner),
                _ => None,
            })
            .filter_map(|owner| {
                let body = world
                    .players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == owner)
                    .map(|p| p.body)
                    .or_else(|| world.npcs.get(&owner).map(|n| n.body))?;
                let b = &world.physics.bodies[body];
                Some((
                    owner,
                    crate::particles::create_world(b.position, b.direction(), [0.0, 1.0, 0.0]),
                ))
            })
            .collect();
        let mut old = std::mem::take(&mut self.performance_emitters);
        for (owner, transform) in wanted {
            let r = match old.remove(&owner) {
                Some(mut r) => {
                    self.effects.update_orientation(&mut r, transform);
                    Some(r)
                }
                None => self
                    .effects
                    .start(custom_hash("special_woot_performance"), transform),
            };
            if let Some(r) = r {
                self.performance_emitters.insert(owner, r);
            }
        }
        for (_, mut r) in old {
            self.effects.stop(&mut r);
        }
    }

    /// Owners with a running `special_woot_performance` effect.
    pub fn performance_emitters(&self) -> Vec<u16> {
        let mut out: Vec<_> = self
            .performance_emitters
            .iter()
            .filter(|(_, r)| self.effects.is_active(r))
            .map(|(h, _)| *h)
            .collect();
        out.sort_unstable();
        out
    }

    /// Whether the Napalm smoke target effect is running.
    pub fn napalm_smoke_active(&self) -> bool {
        self.napalm_smoke
            .as_ref()
            .is_some_and(|(_, r)| self.effects.is_active(r))
    }

    /// Grease slots with a running `magick_grease_burning` effect.
    pub fn grease_burning_emitters(&self) -> Vec<usize> {
        let mut out: Vec<_> = self
            .grease_emitters
            .iter()
            .filter(|(_, (_, _, b))| b.as_ref().is_some_and(|r| self.effects.is_active(r)))
            .map(|(slot, _)| *slot)
            .collect();
        out.sort_unstable();
        out
    }

    /// Levitating characters with a running `magick_levitate` effect.
    pub fn levitate_emitters(&self) -> Vec<u16> {
        let mut out: Vec<_> = self
            .levitate_emitters
            .iter()
            .filter(|(_, r)| self.effects.is_active(r))
            .map(|(h, _)| *h)
            .collect();
        out.sort_unstable();
        out
    }

    /// Confused characters with a running `confused` effect.
    pub fn confuse_emitters(&self) -> Vec<u16> {
        let mut out: Vec<_> = self
            .confuse_emitters
            .iter()
            .filter(|(_, r)| self.effects.is_active(r))
            .map(|(h, _)| *h)
            .collect();
        out.sort_unstable();
        out
    }

    /// Charmed characters with a running `magick_charm` effect.
    pub fn charm_emitters(&self) -> Vec<u16> {
        let mut out: Vec<_> = self
            .charm_emitters
            .iter()
            .filter(|(_, r)| self.effects.is_active(r))
            .map(|(h, _)| *h)
            .collect();
        out.sort_unstable();
        out
    }

    /// Whether the `magick_rain` effect is running.
    pub fn rain_effect_active(&self) -> bool {
        self.rain_effect
            .as_ref()
            .is_some_and(|r| self.effects.is_active(r))
    }

    /// Feared characters with a running `magick_feared` effect.
    pub fn fear_emitters(&self) -> Vec<u16> {
        let mut out: Vec<_> = self
            .fear_emitters
            .iter()
            .filter(|(_, r)| self.effects.is_active(r))
            .map(|(h, _)| *h)
            .collect();
        out.sort_unstable();
        out
    }

    /// Gib presentation: gore one-shots, trails that follow their gib
    /// (Gib.Update UpdateOrientation; translation = body position) and
    /// OnCollision splashes skipped while the gib's last splash is active.
    fn dispatch_gib_visuals(&mut self) {
        use crate::gibs::GibVisual as V;
        for visual in std::mem::take(&mut self.world.gib_visuals) {
            match visual {
                V::OneShot {
                    name,
                    position,
                    direction,
                } => {
                    let _ = self
                        .effects
                        .start_at(custom_hash(name), position, direction);
                }
                V::Trail { id, name } => {
                    if let Some(g) = self.world.gibs.live.iter().find(|g| g.id == id) {
                        // Initialize starts it with the orientation only.
                        let transform = crate::gibs::matrix(g.orientation, [0.0; 3]);
                        if let Some(r) = self
                            .effects
                            .start(custom_hash(&name.to_ascii_lowercase()), transform)
                        {
                            self.gib_trails.insert(id, r);
                        }
                    }
                }
                V::Splash {
                    id,
                    name,
                    position,
                    direction,
                } => {
                    if self
                        .gib_splashes
                        .get(&id)
                        .is_some_and(|r| self.effects.is_active(r))
                    {
                        continue;
                    }
                    if let Some(r) = self
                        .effects
                        .start_at(custom_hash(name), position, direction)
                    {
                        self.gib_splashes.insert(id, r);
                    }
                }
            }
        }
        let live: std::collections::HashSet<u32> =
            self.world.gibs.live.iter().map(|g| g.id).collect();
        self.gib_splashes.retain(|id, _| live.contains(id));
        let mut trails = std::mem::take(&mut self.gib_trails);
        trails.retain(|id, r| {
            let Some(g) = self.world.gibs.live.iter().find(|g| g.id == *id) else {
                // Deinitialize leaves the effect to finish where it is.
                return false;
            };
            !g.frozen
                && self
                    .effects
                    .update_orientation(r, crate::gibs::matrix(g.orientation, g.position))
        });
        self.gib_trails = trails;
    }

    /// MagickCamera.StartVisualEffect / RemoveEffect(s) for the scene's
    /// requests, then the camera Update's loop: every effect follows the
    /// camera position facing Forward and ended ones are dropped.
    fn update_camera_effects(&mut self) {
        const FORWARD: [f32; 3] = [0.0, 0.0, -1.0];
        let at = self.world.camera.ground_position();
        for (effect, add) in std::mem::take(&mut self.world.camera_effect_requests) {
            if add {
                // Starts the new instance before stopping a same-key one.
                let started = self.effects.start_at(effect, at, FORWARD);
                if let Some(mut old) = self.camera_effects.remove(&effect) {
                    self.effects.stop(&mut old);
                }
                self.camera_effects.insert(
                    effect,
                    started.unwrap_or(crate::particles::EffectRef { id: -1, hash: 0 }),
                );
            } else if effect == 0 {
                for (_, mut r) in std::mem::take(&mut self.camera_effects) {
                    self.effects.stop(&mut r);
                }
            } else if let Some(mut r) = self.camera_effects.remove(&effect) {
                self.effects.stop(&mut r);
            }
        }
        let effects = &mut self.effects;
        self.camera_effects
            .retain(|_, r| effects.update_position_direction(r, at, FORWARD));
    }

    pub fn camera(&self) -> &MagickCamera {
        &self.world.camera
    }
    pub fn dead(&self) -> bool {
        self.avatar().dead()
    }
    pub fn input_limited(&self) -> bool {
        self.world.flags.input_limited
            || self.world.camera.lock_input
            || self.world.player_input_locked[0]
    }
    /// Character.GetOrientation: body orientation translated to the feet.
    pub fn avatar_transform(&self) -> (Vector, Orientation) {
        (self.body().feet(), self.body().orientation)
    }

    fn dismiss_tutorial_dialog(&mut self) {
        if let Some(done) = self.world.tutorial.dismiss_dialog() {
            if let Some(trigger) = done.trigger {
                if let Err(e) = self.runtime.execute_trigger(
                    trigger,
                    None,
                    false,
                    self.world.role,
                    &mut self.world,
                ) {
                    self.fail_scripts(format!("tutorial continuation: {e:?}"));
                }
            }
        }
    }

    /// KeyboardMouseController avatar section (after the frame update).
    pub fn handle_input(&mut self, input: PlayerInput, aspect: f32) {
        self.handle_input_with_dialog_select(input, aspect, false);
    }

    /// `select_pressed` is an Enter/Space key edge, not a held key.
    /// Native Update combines those edges with the left-mouse press before
    /// advancing, so simultaneous inputs must dispatch only once.
    pub fn handle_input_with_dialog_select(
        &mut self,
        input: PlayerInput,
        aspect: f32,
        select_pressed: bool,
    ) {
        self.world.aspect = aspect;
        // ControlManager.LimitInput from a freezing Endgame.
        let input = if self.end_game.input_limited {
            PlayerInput::default()
        } else {
            input
        };
        // KeyboardMouseController: Enter, Space or left-click advances a dialog this
        // avatar can advance (outside cutscenes) and stops mouse movement.
        let move_pressed = input.move_down && !self.move_was_down;
        let cast_pressed = input.cast_down && !self.cast_was_down;
        self.cast_was_down = input.cast_down;
        let pressed = select_pressed || move_pressed;
        self.move_was_down = input.move_down;
        if !self.world.tutorial.dialog_done() {
            // The modal consumes controller input, including a previously
            // held walk command and the release that ends it.
            if let Some(avatar) = self.world.players[0].as_mut() {
                avatar.stop_mouse_move();
                self.world.physics.bodies[avatar.body].set_movement([0.0; 3]);
            }
            if pressed || cast_pressed {
                self.dismiss_tutorial_dialog();
            }
            return;
        }
        let can_advance = self
            .world
            .dialogs
            .as_ref()
            .is_some_and(|m| m.can_advance(Some(0)));
        if pressed && !self.world.flags.cutscene && can_advance {
            self.world.advance_dialog(0);
            if let Some(avatar) = self.world.players[0].as_mut() {
                avatar.stop_mouse_move();
            }
        }
        let limited = self.input_limited();
        let mut pick = input.cursor_ndc.map(|ndc| {
            self.world
                .camera
                .mouse_pick(ndc, aspect, self.body().position[1])
        });
        if input.pad_movement.is_some() {
            self.handle_pad_action(move_pressed, can_advance, limited);
        } else {
            self.handle_mouse_character(input, move_pressed, can_advance, limited, &mut pick);
            self.handle_mouse_pickup(input, move_pressed, can_advance, limited, &mut pick);
            self.handle_mouse_interact(input, move_pressed, can_advance, limited, &mut pick);
        }
        let dt = self.last_dt;
        let magicks_allowed = self.magicks_allowed();
        self.handle_player_ui(0, input.magick_prev, input.magick_next, input.inventory);
        let world = &mut self.world;
        let cutscene = world.flags.cutscene;
        let play_time = world.play_time;
        let Some(avatar) = world.players[0].as_mut() else {
            return;
        };
        avatar.registered_magicks = self.registered_magicks.clone();
        avatar
            .registered_unlocked
            .clone_from(&self.registered_unlocked[0]);
        let body = &mut world.physics.bodies[avatar.body];
        let first_event = avatar.events.len();
        let queue_before = avatar.conjure.queue.len();
        avatar.handle_input(
            body,
            input,
            crate::avatar::InputFrame {
                pick,
                input_limited: limited,
                in_cutscene: cutscene,
                dt,
                play_time,
                weapon_cast_disabled: world
                    .flags
                    .tutorial_cast_types_disabled
                    .contains(&(magicka_sim::damage::CastType::Weapon as i32)),
                boost_target: magicka_sim::shield_boost::shield_to_boost(
                    body.position,
                    body.direction(),
                    world.shields.iter().map(|s| &s.shield),
                )
                .map(|index| world.shields[index].handle),
                move_locked: self.pickup_move_locked,
                magicks_allowed,
                magick_cast_disabled: world
                    .flags
                    .tutorial_cast_types_disabled
                    .contains(&(magicka_sim::damage::CastType::Magick as i32)),
                elements_disabled: world.flags.tutorial_elements_disabled as u32,
                force_cast_disabled: world
                    .flags
                    .tutorial_cast_types_disabled
                    .contains(&(magicka_sim::damage::CastType::Force as i32)),
                area_cast_disabled: world
                    .flags
                    .tutorial_cast_types_disabled
                    .contains(&(magicka_sim::damage::CastType::Area as i32)),
                self_cast_disabled: world
                    .flags
                    .tutorial_cast_types_disabled
                    .contains(&(magicka_sim::damage::CastType::SelfCast as i32)),
                push_disabled: world.flags.tutorial_push_disabled,
            },
        );
        let mut chant_events = Vec::new();
        for event in &avatar.events[first_event..] {
            if let AvatarEvent::ElementCooldown(element) = event {
                // The eight default keyboard bindings always identify one
                // basic element, so the checked HUD index cannot fail.
                self.keyboard_hud
                    .cooldown(*element)
                    .expect("keyboard element");
            }
            if let AvatarEvent::Conjure(magicka_sim::avatar_input::ConjureEvent::Queue(q)) = event {
                chant_events.push(*q);
            }
        }
        self.chant_events(0, chant_events, queue_before);
    }

    /// Per-player UI is local to this computer, including mirrors on clients.
    pub fn handle_player_ui(&mut self, slot: usize, prev: bool, next: bool, inventory: bool) {
        let limited = self.world.flags.input_limited
            || self.world.camera.lock_input
            || self.world.player_input_locked[slot]
            || self.end_game.input_limited;
        if limited
            || self
                .world
                .players
                .get(slot)
                .and_then(Option::as_ref)
                .is_none_or(|p| !p.playing || p.dead())
        {
            return;
        }
        let mask = self.magicks_allowed();
        let allowed = |t| magicka_sim::magicks::allowed(mask, t);
        let registered = self.registered_unlocked[slot].clone();
        if prev {
            self.tome[slot].prev_with(allowed, &registered);
        } else if next {
            self.tome[slot].next_with(allowed, &registered);
        }
        if inventory {
            self.check_inventory_for(slot);
        }
    }

    /// Avatar.CheckInventory for the local avatar: toggles the InventoryBox
    /// unless another avatar owns it, the avatar is polymorphed, or either
    /// weapon slot holds weapon_unarmed.
    pub fn check_inventory(&mut self) {
        self.check_inventory_for(0);
    }
    fn check_inventory_for(&mut self, slot: usize) {
        let Some(avatar) = self.world.players.get(slot).and_then(Option::as_ref) else {
            return;
        };
        if self.inventory.owner.is_some_and(|o| o != avatar.handle) || avatar.polymorphed {
            return;
        }
        let item = |slot: i32| {
            avatar
                .items
                .iter()
                .find(|(s, _)| *s == slot)
                .map(|(_, i)| &i.definition)
        };
        let (Some(weapon), Some(staff)) = (item(0), item(1)) else {
            return;
        };
        if [weapon, staff]
            .iter()
            .any(|i| i.name.eq_ignore_ascii_case("weapon_unarmed"))
        {
            return;
        }
        if self.inventory.active() {
            self.inventory.close(Some(avatar.handle));
            return;
        }
        let Some(text) = self.world.dialog_text.as_ref() else {
            self.world
                .log
                .push("inventory: no dialog text host (fonts/strings)".into());
            return;
        };
        let (weapon, staff) = (
            crate::inventory_box::ItemText::new(&weapon.display_name, &weapon.description),
            crate::inventory_box::ItemText::new(&staff.display_name, &staff.description),
        );
        if let Err(e) = self.inventory.show_inventory(
            weapon,
            staff,
            avatar.handle,
            &text.strings,
            &text.body_font,
        ) {
            self.world.log.push(format!("inventory: {e:?}"));
        }
    }

    /// One PlayState.Update with the logic delta (clamped like the original).
    /// Install the match's resolved definitions. Gameplay content changes
    /// need a new match: call before play, not mid-match.
    pub fn set_definitions(&mut self, definitions: Arc<magicka_definitions::Definitions>) {
        self.registered_magicks = Arc::new(
            definitions
                .magicks
                .iter()
                .filter(|m| m.legacy.is_none())
                .filter_map(|m| {
                    let index = definitions.magicks.index_of(&m.common.id)?.0;
                    Some((index, m.combination.clone()))
                })
                .collect(),
        );
        self.world.scripted_magicks = definitions
            .magicks
            .iter()
            .filter(|m| matches!(m.handler, magicka_definitions::Handler::Lua { .. }))
            .filter_map(|m| m.legacy)
            .fold(0, |mask, legacy| mask | 1u64 << (legacy & 63));
        self.definitions = definitions;
        self.definitions_revision += 1;
        self.apply_item_definitions();
    }
    /// Patch every held and dropped item not yet patched by the current
    /// definitions (KV overrides of retail item templates).
    pub fn apply_item_definitions(&mut self) {
        if self.definitions_revision == 0 {
            return;
        }
        let (defs, revision) = (&self.definitions, self.definitions_revision);
        let players = self.world.players.iter_mut().flatten();
        let held = players.flat_map(|p| p.items.iter_mut().map(|(_, item)| item));
        let dropped = self
            .world
            .item_pickups
            .values_mut()
            .filter_map(|p| p.item.as_mut());
        for item in held.chain(dropped) {
            if item.patched != revision {
                crate::item_definitions::patch(defs, item);
                item.patched = revision;
            }
        }
    }
    pub fn step(&mut self, dt: f32) {
        let dt = dt.min(MAX_DELTA);
        self.apply_item_definitions();
        self.world.camera_slots.clone_from(&self.camera_slots);
        self.sync_scene_effects();
        self.last_dt = dt;
        self.spray_lights.update(dt);
        self.world.tutorial.update(dt);
        self.play_time += f64::from(dt);
        let n = self.world.players.len();
        self.tome.reserve_slots(n);
        for tome in &mut self.tome {
            tome.update(dt);
        }
        self.world.earthquake_rigid = self
            .pickup_physics
            .as_ref()
            .map(|physics| {
                physics
                    .earthquake_targets()
                    .into_iter()
                    .map(|b| (b.key.handle, b))
                    .collect()
            })
            .unwrap_or_default();
        self.world.game_ended = self.end_game.condition != crate::end_game::EndGameCondition::None;
        self.world.unlocked_magick_mask = self.magicks_allowed();
        let runtime = self.script_error.is_none().then_some(&mut self.runtime);
        // Phony LevelComplete: UpdateMiscA zeroes the level-part and entity
        // delta (the rewrite also pauses physics; research/campaign-progression.md).
        // PlayState.Update: iDeltaTime *= TimeModifier · TimeMultiplier
        // (TimeWarp) before physics, AI and entities.
        self.world.scene_saturation = self.post.saturation;
        let world_dt = if self.end_game.freezes_entities() {
            0.0
        } else {
            dt * self.world.time_multiplier * self.world.time_modifier
        };
        if let Err(e) = self.world.step_live(runtime, world_dt) {
            self.fail_scripts(format!("{e:?}"));
        }
        if let Err(e) = self.release_dead_equipment() {
            self.world.log.push(format!("equipment release: {e:?}"));
        }
        self.apply_versus_requests();
        for (handle, generation, velocity) in std::mem::take(&mut self.world.pickup_launches) {
            self.pending_pickup_velocities
                .push((magicka_physics::BodyKey { handle, generation }, velocity));
        }
        for release in std::mem::take(&mut self.world.npc_item_releases) {
            let (name, mut transform, velocity) =
                (release.name, release.transform, release.velocity);
            let despawn = release.despawn;
            if release.raise_by_height {
                // SpawnItemEvent: Y += the pickup box's side length Y.
                let model = self
                    .world
                    .item_templates
                    .get(&custom_hash(&name))
                    .and_then(|i| i.model.as_ref())
                    .map(|m| Path::new("Data/Items/Wizard").join(m.replace('\\', "/")));
                if let Some(model) = model {
                    let bounds = self.pickup_bounds.entry(model.clone()).or_insert_with(|| {
                        crate::play_content::pickup_model_bounds(&self.content, &model)
                    });
                    if let Some(b) = bounds {
                        transform[13] += b.max[1] - b.min[1];
                    }
                }
            }
            match self.spawn_item_pickup(&name, transform) {
                Ok(handle) => {
                    if let Some(p) = self.world.item_pickups.get_mut(&handle) {
                        p.immovable = false;
                        p.despawn = despawn;
                    }
                    if velocity != [0.0; 3] {
                        self.pending_pickup_velocities.push((
                            magicka_physics::BodyKey {
                                handle,
                                generation: self.world.item_pickups[&handle].generation,
                            },
                            velocity,
                        ));
                    }
                }
                Err(e) => self
                    .world
                    .log
                    .push(format!("npc item release {name}: {e:?}")),
            }
        }
        for revive in std::mem::take(&mut self.world.revive_spawns) {
            if let Err(e) = self.revive_player(&revive) {
                self.world.log.push(format!("revive: {e:?}"));
            }
        }
        for request in std::mem::take(&mut self.world.avatar_polymorphs) {
            if let Err(e) = self.polymorph_avatar(request) {
                self.world.log.push(format!("polymorph: {e:?}"));
            }
        }
        self.magick_prop_fields(world_dt);
        for (key, velocity) in std::mem::take(&mut self.world.earthquake_velocities) {
            if let Some(physics) = self.pickup_physics.as_mut() {
                physics.add_velocity_for(key, velocity);
            }
        }
        self.update_pickup_physics(world_dt);
        self.update_magick_books(world_dt);
        self.update_item_despawn(world_dt);
        if let Some((matrices, ignored)) = self.world.pending_checkpoint.take() {
            self.checkpoint_generation = self.checkpoint_generation.wrapping_add(1);
            self.checkpoint = Some(Checkpoint {
                scene: custom_hash(&self.scene_name.to_ascii_lowercase()),
                matrices,
                ignored,
                runtime: self.runtime.clone(),
                animations: self.world.collision.playback_states(),
                players: self.checkpoint_players(),
                physics: self.world.checkpoint_physics(),
                saved_scenes: self.saved_scenes.clone(),
                // DialogCollection.State.UpdateState captures during playback too.
                dialogs: self.world.dialogs.as_ref().map(|d| d.checkpoint_boundary()),
                camera: self.world.camera.state(),
                music: self.world.flags.music,
                music_focus_value: self.world.flags.music_focus_value,
                pickables: self.world.checkpoint_pickables(),
                unequip_weapons_section: self.unequip_weapons_section,
                default_avatar_section: self.default_avatar_section,
            });
        }
        if let (Some(screen), Some(text)) = (self.headless_dialogs, self.world.dialog_text.as_mut())
        {
            if let Err(e) = text.pre_render_visible(0, dt, screen, self.world.dialog_matrix) {
                self.world.dialog_failure = Some(format!("box pre-render: {e:?}"));
            }
        }
        self.drain();
        let status_owners = self
            .world
            .players
            .iter()
            .flatten()
            .filter(|p| !p.dead())
            .map(|p| (p.handle, p.body, &p.character.combat.statuses))
            .chain(
                self.world
                    .npcs
                    .values()
                    .filter(|n| !n.character.combat.dead)
                    .map(|n| (n.handle, n.body, &n.character.combat.statuses)),
            )
            .map(
                |(handle, body, statuses)| crate::spell_lights::StatusLightOwner {
                    handle,
                    position: self.world.physics.bodies[body].position,
                    kind: crate::spell_lights::StatusLightKind::Character {
                        burning: statuses.has(magicka_sim::status::Status::Burning),
                        steamed: statuses.has(magicka_sim::status::Status::Steamed),
                    },
                },
            )
            .chain(
                self.world
                    .entities
                    .iter()
                    .filter(|e| e.added && !e.removed && !e.dead)
                    .filter_map(|e| {
                        let state = self.world.physics_damage.get(&e.handle)?;
                        Some(crate::spell_lights::StatusLightOwner {
                            handle: e.handle,
                            position: e.position,
                            kind: crate::spell_lights::StatusLightKind::Physics {
                                burning: state.status_mask
                                    & magicka_sim::status::Status::Burning as u16
                                    != 0,
                                magnitude: state
                                    .statuses
                                    .iter()
                                    .find(|s| s.kind == magicka_sim::status::Status::Burning)
                                    .map_or(0.0, |s| s.magnitude),
                            },
                        })
                    }),
            )
            .collect::<Vec<_>>();
        self.status_lights
            .sync(status_owners, dt, &mut self.light_random);
        let scene_changed = self.advance_scene_transition(dt);
        self.world.decals.update(dt);
        // LightningBolt.Update: mTTL -= dt; drawn while mTTL > 0.
        for b in &mut self.bolts {
            b.ttl -= dt;
        }
        self.bolts.retain(|b| b.ttl > 0.0);
        // UpdateMiscA end-game section (single local player).
        if let Some((condition, freeze, phony)) = self.world.pending_end_game.take() {
            self.end_game.begin(
                crate::end_game::EndGameCondition::from_value(condition),
                freeze,
                phony,
                0.0,
            );
        }
        self.end_game.has_ruleset = self.world.ruleset.is_some();
        let all_dead = self
            .world
            .players
            .iter()
            .flatten()
            .filter(|p| p.playing)
            // A single player's active revival fairy exempts the dead avatar.
            .all(|p| p.dead() && !self.world.revival_fairy_blocks_defeat(p.slot))
            // Rolling credits suppress the campaign Defeat check.
            && !self.credits.as_ref().is_some_and(|c| c.started);
        self.end_game.update(dt, all_dead);
        self.world.cutscene.update(dt);
        // PlayState.BeginCutscene closes the InventoryBox; Update animates it.
        if self.world.flags.cutscene && !self.inventory_cutscene {
            self.inventory.close(None);
        }
        self.inventory_cutscene = self.world.flags.cutscene;
        self.inventory.update(dt);
        // GameScene.Update: named level and scripted effects share one registry.
        self.sync_scene_effects();
        // Scene light updates: Light.GetIntensity variation (flicker,
        // candle, sine, strobe) per level light, sharing Light.sRandom.
        let lights = &self.world.level.lights;
        self.light_variations
            .resize(lights.len(), Default::default());
        self.level_light_intensity = lights
            .iter()
            .zip(&mut self.light_variations)
            .map(|(l, v)| {
                v.intensity(
                    l.variation_type,
                    dt,
                    l.variation_speed,
                    l.variation_amount,
                    &mut self.light_random,
                )
            })
            .collect();
        // PlayState UpdateMiscA (EffectManager.Update) and UpdateMiscB
        // (ParticleSystem.UpdateParticles), after the entities.
        // A scene entered this step has not updated yet: natively its first
        // GameScene.Update runs its triggers before its effects, so its
        // level effects must not emit before its Init can stop them.
        self.update_camera_effects();
        self.update_credits(dt);
        if !scene_changed {
            self.effects.update(dt);
        }
        self.effects.update_particles(dt);
        // VisualEffect.IsActive is visible immediately after its update,
        // including to fast-forward/script actions between session steps.
        for storage in self.world.effects.effects.values_mut() {
            if let Some(reference) = self.scene_effect_refs.get(&storage.effect.instance) {
                storage.effect.active = self.effects.is_active(reference);
            }
        }
        // PlayState.Update: HUDManager.Update just before controller input;
        // it neither updates nor draws the KeyboardHUD while UI is disabled.
        if self.world.ui_enabled {
            self.keyboard_hud
                .set_disabled_elements(self.world.flags.tutorial_elements_disabled as u32);
            self.keyboard_hud.update(dt);
        }
    }

    pub fn spell_point_lights(&self) -> impl Iterator<Item = &crate::arcane_blast::DynamicLight> {
        let local = self.remote_capsule_lights.is_none();
        self.dynamic_lights
            .iter()
            .chain(self.spray_lights.lights().filter(move |_| local))
            .chain(self.status_lights.lights().filter(move |_| local))
            .chain(
                self.chants
                    .active()
                    .filter_map(|c| c.light.as_ref())
                    .filter(move |_| local),
            )
    }

    pub fn spell_capsule_lights(&self) -> Vec<crate::arcane_blade::CapsuleLight> {
        if let Some(remote) = &self.remote_capsule_lights {
            return remote.clone();
        }
        self.beams
            .iter()
            .map(crate::spell_lights::beam_light)
            .chain(
                self.bolts
                    .iter()
                    .filter(|b| b.ttl > 0.0)
                    .map(crate::spell_lights::bolt_light),
            )
            .chain(
                self.world
                    .players
                    .iter()
                    .flatten()
                    .flat_map(|p| p.arcane_blades.iter().filter_map(|b| b.capsule_light())),
            )
            .collect()
    }

    /// Spell sounds of avatar casts (research/audio-xact.md):
    /// - SpraySpell.CastForce / LightningSpell Force+Area: SOUNDS_SPRAY per
    ///   element with Magnitude, on the caster's emitter until the effect
    ///   ends (SprayStopped, or the caster's spell ending);
    /// - ProjectileSpell.CastArea: SPELL_AREA_SOUND per element (integer
    ///   magnitude), global;
    /// - ProjectileSpell.CastForce: the Ice or Earth force cue, global;
    /// - PushSpell: spell_push_force/area on the caster;
    /// - RailGunSpell.CastArea: its Arcane/Life area cue.
    fn spell_audio(&mut self) {
        use crate::audio::{banks::SPELLS, SOUNDS_AREA};
        use magicka_sim::damage::CastType;
        use magicka_sim::spell_cast::EffectKind;
        use magicka_sim::spells::{Elements as E, SpellType};
        const SPRAY: [&str; 11] = [
            "spell_earth_spray",
            "spell_water_spray",
            "spell_cold_spray",
            "spell_fire_spray",
            "spell_lightning_spray",
            "spell_arcane_spray",
            "spell_life_spray",
            "spell_shield_spray",
            "spell_ice_spray",
            "spell_steam_spray",
            "spell_poison_spray",
        ];
        let mut actions = Vec::new();
        for (handle, body, events) in self
            .world
            .players
            .iter()
            .flatten()
            .map(|p| (p.handle, p.body, &p.events))
            .chain(
                self.world
                    .npcs
                    .values()
                    .map(|n| (n.handle, n.body, &n.animation_events)),
            )
        {
            let position = self.world.physics.bodies[body].position;
            for e in events {
                match e {
                    AvatarEvent::SprayStarted { spell, .. } => {
                        actions.push((handle, position, Some((EffectKind::Spray, *spell)), None))
                    }
                    AvatarEvent::SprayStopped => actions.push((handle, position, None, None)),
                    AvatarEvent::SpellCast {
                        spell,
                        cast_type,
                        spell_power,
                    } => actions.push((
                        handle,
                        position,
                        None,
                        Some((*spell, *cast_type, *spell_power)),
                    )),
                    AvatarEvent::PushStarted { cast_type, .. } => {
                        let name = if *cast_type == CastType::Area {
                            "spell_push_area"
                        } else {
                            "spell_push_force"
                        };
                        self.audio_one_shots.push((name, Some(position), None));
                    }
                    AvatarEvent::BeamArea {
                        cue: (name, magnitude),
                        ..
                    } => {
                        self.audio_one_shots.push((name, None, Some(*magnitude)));
                    }
                    _ => {}
                }
            }
        }
        for (name, at, magnitude) in std::mem::take(&mut self.audio_one_shots) {
            self.world.play_cue_magnitude(SPELLS, name, at, magnitude);
        }
        for (handle, position, start, cast) in actions {
            let loops = |kind: EffectKind,
                         spell: &magicka_sim::spells::Spell,
                         world: &mut SceneWorld<FsSceneContent>| {
                let mut keys = Vec::new();
                for (i, e) in E::BY_INDEX.iter().enumerate() {
                    let m = spell.get(*e);
                    if m > 0.0 {
                        keys.push(world.play_cue_magnitude(
                            SPELLS,
                            SPRAY[i],
                            Some(position),
                            Some(m),
                        ));
                    }
                }
                (kind, keys)
            };
            if let Some((kind, spell)) = start {
                if let Some((_, old)) = self.spell_loops.remove(&handle) {
                    for k in old {
                        self.world.stop_cue(k);
                    }
                }
                let entry = loops(kind, &spell, &mut self.world);
                self.spell_loops.insert(handle, entry);
                continue;
            }
            let Some((spell, cast_type, spell_power)) = cast else {
                // SprayStopped.
                if let Some((EffectKind::Spray, old)) = self.spell_loops.remove(&handle) {
                    for k in old {
                        self.world.stop_cue(k);
                    }
                }
                continue;
            };
            match (spell.spell_type(), cast_type) {
                (SpellType::Lightning, CastType::Force | CastType::Area) => {
                    if let Some((_, old)) = self.spell_loops.remove(&handle) {
                        for k in old {
                            self.world.stop_cue(k);
                        }
                    }
                    let entry = loops(EffectKind::Lightning, &spell, &mut self.world);
                    self.spell_loops.insert(handle, entry);
                }
                (SpellType::Projectile, CastType::Area) => {
                    for (i, e) in E::BY_INDEX.iter().enumerate() {
                        let m = spell.get(*e) as i32;
                        // SPELL_AREA_SOUND: Shield has no entry; Poison
                        // reuses the Steam cue.
                        let name = match i {
                            7 => continue,
                            10 => SOUNDS_AREA[9],
                            _ => SOUNDS_AREA[i],
                        };
                        if m > 0 {
                            self.world
                                .play_cue_magnitude(SPELLS, name, None, Some(m as f32));
                        }
                    }
                }
                (SpellType::Projectile, CastType::Force) => {
                    let name = if spell.element.has(E::EARTH) && !spell.element.has(E::ICE) {
                        "spell_earth_force"
                    } else {
                        "spell_ice_force"
                    };
                    // ProjectileSpell.CastForce IL_00cc..00d8 captures
                    // SpellPower × 3 before IL_00dd..00e3 resets the owner.
                    self.world
                        .play_cue_magnitude(SPELLS, name, None, Some(spell_power * 3.0));
                }
                _ => {}
            }
        }
        // Loops follow the caster's AudioEmitter; a Lightning effect's
        // DeInitialize stops its cues when the caster's spell ends.
        let mut ended = Vec::new();
        for (&handle, (kind, keys)) in &self.spell_loops {
            let actor = self
                .world
                .players
                .iter()
                .flatten()
                .find(|p| p.handle == handle)
                .map(|p| (p.body, p.current_spell.is_some()))
                .or_else(|| {
                    self.world
                        .npcs
                        .get(&handle)
                        .map(|n| (n.body, n.current_spell.is_some()))
                });
            let Some((body, current)) = actor else {
                ended.push(handle);
                continue;
            };
            if *kind == EffectKind::Lightning && !current {
                ended.push(handle);
                continue;
            }
            let position = self.world.physics.bodies[body].position;
            for &k in keys.clone().iter() {
                self.world.move_cue(k, position);
            }
        }
        for handle in ended {
            if let Some((_, keys)) = self.spell_loops.remove(&handle) {
                for k in keys {
                    self.world.stop_cue(k);
                }
            }
        }
    }

    /// Railgun presentation: source/hit effects follow their rail and stop
    /// with it; ArcaneBlast starts one `beam_<element>_explosion` per beam
    /// blast element (Forward direction). Cues are not played (no audio host).
    fn drain_rails(&mut self) {
        use magicka_sim::railgun::RailEvent as R;
        self.beams.clone_from(&self.world.rails.render);
        for event in std::mem::take(&mut self.world.rails.events) {
            match event {
                R::EffectStarted {
                    rail,
                    hit,
                    name,
                    position,
                    direction,
                } => {
                    let r = self
                        .effects
                        .start_at(custom_hash(name), position, direction)
                        .unwrap_or_default();
                    if let Some(mut old) = self.rail_effects.insert((rail, hit), r) {
                        self.effects.stop(&mut old);
                    }
                }
                R::EffectMoved {
                    rail,
                    hit,
                    position,
                    direction,
                } => {
                    if let Some(r) = self.rail_effects.get_mut(&(rail, hit)) {
                        self.effects
                            .update_position_direction(r, position, direction);
                    }
                }
                R::EffectStopped { rail, hit } => {
                    if let Some(mut r) = self.rail_effects.remove(&(rail, hit)) {
                        self.effects.stop(&mut r);
                    }
                }
                R::Cue {
                    rail,
                    name,
                    position,
                } => {
                    let key =
                        self.world
                            .play_cue(crate::audio::banks::SPELLS, name, Some(position));
                    let entry = self.rail_cues.entry(rail).or_default();
                    // Stage start, then the hit cue.
                    if entry.0.is_none() {
                        entry.0 = Some(key);
                    } else if let Some(old) = entry.1.replace(key) {
                        self.world.stop_cue(old);
                    }
                }
                R::HitCueStopped { rail } => {
                    if let Some(key) = self.rail_cues.get_mut(&rail).and_then(|e| e.1.take()) {
                        self.world.stop_cue(key);
                    }
                }
                R::Removed { rail } => {
                    if let Some((start, hit)) = self.rail_cues.remove(&rail) {
                        for key in start.into_iter().chain(hit) {
                            self.world.stop_cue(key);
                        }
                    }
                }
                R::Exploded { .. } => {}
            }
        }
        const BLAST: [&str; 11] = [
            "",
            "beam_water_explosion",
            "beam_cold_explosion",
            "beam_fire_explosion",
            "beam_lightning_explosion",
            "beam_arcane_explosion",
            "beam_life_explosion",
            "",
            "",
            "beam_steam_explosion",
            "beam_poison_explosion",
        ];
        // SpellManager / Scene updates of existing ArcaneBlasts and lights.
        let dt = self.last_dt;
        for blast in &mut self.arcane_blasts {
            blast.update(dt);
        }
        self.arcane_blasts.retain(|b| !b.is_dead());
        // SpellManager.Update: IsDead effects are removed after updating.
        for spikes in &mut self.ice_spikes {
            spikes.update(dt);
        }
        self.ice_spikes.retain(|s| !s.is_dead());
        self.update_chants(dt);
        for light in &mut self.dynamic_lights {
            light.update(dt);
        }
        self.dynamic_lights.retain(|l| !l.is_removed());
        self.beam_blasts = std::mem::take(&mut self.world.beam_blasts);
        for &(position, radius, elements) in &self.beam_blasts {
            if elements & magicka_sim::spells::Elements::BEAMS.0 != 0 {
                // ArcaneBlast.Initialize (FullBlast IL_0021–IL_004b).
                let blast = crate::arcane_blast::ArcaneBlast::new(position, radius, elements);
                self.dynamic_lights.push(blast.light());
                self.arcane_blasts.push(blast);
            }
            self.world.camera.shake(radius * 0.333, 0.3);
            if elements & magicka_sim::spells::Elements::BEAMS.0 == 0 {
                continue;
            }
            for (i, name) in BLAST.iter().enumerate() {
                if elements & (1 << i) != 0 && !name.is_empty() {
                    let _ = self
                        .effects
                        .start_at(custom_hash(name), position, [0.0, 0.0, -1.0]);
                }
            }
        }
    }

    /// Retained travelling effects belonging to one missile, for presentation
    /// inspection. Impact effects are intentionally not retained on the missile.
    pub fn missile_effect_refs(&self, missile: u32) -> Vec<crate::particles::EffectRef> {
        self.missile_effects
            .get(&missile)
            .map_or_else(Vec::new, |effects| effects.values().copied().collect())
    }

    /// PlayEffectEvent and PlaySoundEvent, followed emitter updates and
    /// Deinitialize stopping retained effects and cues.
    fn drain_missiles(&mut self) {
        use crate::missiles::MissileVisual as V;
        for visual in std::mem::take(&mut self.world.missile_visuals) {
            match visual {
                V::Blast {
                    position,
                    radius,
                    damages,
                } => {
                    // FullBlast with Arcane/Life (Beams) draws an ArcaneBlast
                    // disc instead of wedge bursts (BlastEvent → FullBlast);
                    // the beam-blast path creates it and shakes the camera.
                    let all = damages.iter().fold(0u32, |a, d| a | d.element);
                    if all & magicka_sim::spells::Elements::BEAMS.0 != 0 {
                        self.world.beam_blasts.push((position, radius, all));
                    } else {
                        // FullBlast(DamageCollection5): the position overload
                        // also rumbles local controllers; camera state is global.
                        self.world.camera.shake(radius * 0.333, 0.3);
                    }
                    let scalars = std::array::from_fn(|i| {
                        let direction = crate::blast_visual::direction(i);
                        self.world
                            .collision
                            .collision()
                            .segment_intersect(
                                magicka_sim::spray_pulse::Segment {
                                    origin: position,
                                    delta: direction.map(|v| v * radius),
                                },
                                false,
                            )
                            .map_or(1.0, |hit| hit.hit.fraction)
                    });
                    for (name, transform) in
                        crate::blast_visual::bursts(radius, position, &damages, scalars)
                    {
                        let _ = self.effects.start(custom_hash(name), transform);
                    }
                    self.world.decals.blast(
                        radius,
                        position,
                        &damages,
                        scalars,
                        false,
                        self.world.collision.collision(),
                    );
                }
                V::Spawned { .. } => {}
                V::Effect {
                    missile,
                    name,
                    position,
                    direction,
                    follow,
                } => {
                    let r = self
                        .effects
                        .start_at(custom_hash(name), position, direction)
                        .unwrap_or_default();
                    if follow {
                        self.missile_effects
                            .entry(missile)
                            .or_default()
                            .insert(name, r);
                    }
                }
                V::Sound {
                    missile,
                    bank,
                    name,
                    magnitude,
                    position,
                    stop_on_remove,
                } => {
                    let key =
                        self.world
                            .play_cue_magnitude(bank, name, Some(position), Some(magnitude));
                    self.events
                        .push(SessionEvent::MissileSound { missile, key });
                    if stop_on_remove {
                        self.missile_cues.entry(missile).or_default().push(key);
                    }
                }
                V::Follow {
                    missile,
                    position,
                    direction,
                } => {
                    if let Some(keys) = self.missile_cues.get(&missile) {
                        for &key in keys {
                            self.world.move_cue(key, position);
                        }
                    }
                    if let Some(refs) = self.missile_effects.get_mut(&missile) {
                        for r in refs.values_mut() {
                            self.effects
                                .update_position_direction(r, position, direction);
                        }
                    }
                }
                V::Removed { missile } => {
                    for key in self.missile_cues.remove(&missile).unwrap_or_default() {
                        self.world.stop_cue(key);
                    }
                    for (_, mut r) in self.missile_effects.remove(&missile).unwrap_or_default() {
                        self.effects.stop(&mut r);
                    }
                }
            }
        }
    }

    fn drain(&mut self) {
        self.drain_campaign_requests();
        self.events.extend(
            std::mem::take(&mut self.world.checkpoint_notices)
                .into_iter()
                .map(SessionEvent::CheckpointReached),
        );
        for magick in std::mem::take(&mut self.world.pending_magick_unlocks) {
            let n = self.world.players.len();
            self.unlocked_magicks.reserve_slots(n);
            for bits in &mut self.unlocked_magicks {
                *bits |= 1u64 << magick;
            }
        }
        self.world.sync_spray_area_audio();
        for request in std::mem::take(&mut self.world.audio_requests) {
            let position = match &request {
                crate::audio::AudioRequest::Start {
                    source: crate::audio::SoundSource::Locator { area, .. },
                    ..
                } => self
                    .world
                    .scene_locator(*area)
                    .ok()
                    .map(|m| [m[12], m[13], m[14]]),
                crate::audio::AudioRequest::Start {
                    source: crate::audio::SoundSource::Emitter(p),
                    ..
                } => Some(*p),
                _ => None,
            };
            self.audio.push((request, position));
        }
        for effect in std::mem::take(&mut self.world.shield_nullifications) {
            let _ = self.effects.start_at(
                custom_hash("magick_nullified"),
                effect.position,
                [1.0, 0.0, 0.0],
            );
        }
        for visual in std::mem::take(&mut self.world.barrier_visuals) {
            use crate::scene_world::BarrierVisual as V;
            match visual {
                V::Start {
                    slot,
                    name,
                    transform,
                } => {
                    if let Some(mut old) = self.barrier_effects.remove(&slot) {
                        self.effects.stop(&mut old);
                    }
                    if let Some(r) = self.effects.start(custom_hash(name), transform) {
                        self.barrier_effects.insert(slot, r);
                    }
                }
                V::Update { slot, transform } => {
                    if let Some(r) = self.barrier_effects.get_mut(&slot) {
                        self.effects.update_orientation(r, transform);
                    }
                }
                V::Stop { slot } => {
                    if let Some(mut r) = self.barrier_effects.remove(&slot) {
                        self.effects.stop(&mut r);
                    }
                }
                V::OneShot {
                    name,
                    position,
                    direction,
                } => {
                    let _ = self
                        .effects
                        .start_at(custom_hash(name), position, direction);
                }
                V::Bolt(bolt) => {
                    let _ = self.effects.start_at(
                        custom_hash("lightning_source"),
                        bolt.from,
                        bolt.direction,
                    );
                    let _ = self.effects.start_at(
                        custom_hash("lightning_hit"),
                        bolt.to,
                        bolt.direction,
                    );
                    self.bolt_serial = self.bolt_serial.wrapping_add(1);
                    self.bolts.push(LiveBolt {
                        visual: bolt,
                        ttl: bolt.ttl,
                        serial: self.bolt_serial,
                    });
                }
            }
        }
        self.dispatch_gib_visuals();
        self.dispatch_underground_visuals();
        self.sync_status_emitters();
        self.sync_fear_emitters();
        self.sync_rain_effect();
        self.sync_charm_emitters();
        self.sync_confuse_emitters();
        self.sync_levitate_emitters();
        self.sync_grease_emitters();
        self.sync_grease_sprays();
        self.sync_tornado_dust();
        self.sync_vortex_effects();
        self.sync_stun_effects();
        self.sync_napalm_smoke();
        self.sync_performance_emitters();
        for shot in std::mem::take(&mut self.world.gun_shots) {
            let Some(sprite) = shot.sprite else {
                continue;
            };
            let delta: [f32; 3] = std::array::from_fn(|i| shot.end[i] - shot.origin[i]);
            let length = delta.iter().map(|v| v * v).sum::<f32>().sqrt();
            let ttl = (length - 2.) / shot.velocity;
            if ttl > 0. && ttl.is_finite() {
                let direction = delta.map(|v| v / length);
                self.effects.particles.spawn(&crate::particles::Particle {
                    position: std::array::from_fn(|i| shot.origin[i] + direction[i]),
                    velocity: direction.map(|v| v * shot.velocity),
                    ttl,
                    start_size: 1.,
                    end_size: 1.,
                    drag: 1.,
                    rotation: f32::NAN,
                    rotation_velocity: f32::NAN,
                    sprite,
                    color: [1.; 4],
                    ..Default::default()
                });
            }
        }
        for request in std::mem::take(&mut self.world.item_event_effects) {
            use crate::npc_combat::ItemEffectRequest;
            match request {
                ItemEffectRequest::Start {
                    owner,
                    weapon,
                    name,
                    position,
                    direction,
                    follow,
                } => {
                    let Some(mut reference) =
                        self.effects
                            .start_at(custom_hash(&name), position, direction)
                    else {
                        continue;
                    };
                    if follow {
                        // Native Item.mFollowEffects has four references, first
                        // inactive slot wins; an overflowing reference is stopped.
                        let free = (200..204).find(|&i| {
                            self.item_effects
                                .get(&(owner, weapon, i))
                                .is_none_or(|r| !self.effects.is_active(r))
                        });
                        if let Some(index) = free {
                            self.item_effects.insert((owner, weapon, index), reference);
                        } else {
                            self.effects.stop(&mut reference);
                        }
                    }
                }
                ItemEffectRequest::Stop { owner, weapon } => {
                    for index in 200..204 {
                        if let Some(mut reference) =
                            self.item_effects.remove(&(owner, weapon, index))
                        {
                            self.effects.stop(&mut reference);
                        }
                    }
                }
            }
        }
        for (name, position, direction) in std::mem::take(&mut self.world.status_contact_effects) {
            let _ = self
                .effects
                .start_at(custom_hash(name), position, direction);
        }
        // Magick bolts (Thunderbolt): InitializeEffect as for spell bolts.
        for bolt in std::mem::take(&mut self.world.magick_bolts) {
            let _ =
                self.effects
                    .start_at(custom_hash("lightning_source"), bolt.from, bolt.direction);
            let _ = self
                .effects
                .start_at(custom_hash("lightning_hit"), bolt.to, bolt.direction);
            self.bolt_serial = self.bolt_serial.wrapping_add(1);
            self.bolts.push(LiveBolt {
                visual: bolt,
                ttl: bolt.ttl,
                serial: self.bolt_serial,
            });
        }
        for (name, position, direction) in std::mem::take(&mut self.world.mine_effects) {
            let _ = self
                .effects
                .start_at(custom_hash(name), position, direction);
        }
        for (name, position, direction) in std::mem::take(&mut self.world.drown_effects) {
            let _ = self
                .effects
                .start_at(custom_hash(name), position, direction);
        }
        for (position, radius, damages) in std::mem::take(&mut self.world.ground_blasts) {
            if let Some(light) = crate::blast_visual::ground_light(radius, position, &damages) {
                self.dynamic_lights.push(light);
            }
            self.world.camera.shake(2.0, 0.9);
            self.world.decals.blast(
                radius,
                position,
                &damages,
                [1.0; 8],
                true,
                self.world.collision.collision(),
            );
            for (name, transform) in crate::blast_visual::ground_bursts(radius, position, &damages)
            {
                let _ = self.effects.start(custom_hash(name), transform);
            }
            // GroundBlast: one IceSpikes per damage carrying Ice.
            for _ in damages
                .iter()
                .filter(|d| d.element & magicka_sim::spells::Elements::ICE.0 != 0)
            {
                self.spawn_ice_spikes(position, radius);
            }
        }
        self.drain_missiles();
        self.drain_rails();
        self.spell_audio();
        self.arcane_blade_presentation();
        self.item_effect_presentation();
        self.prop_effect_presentation();
        let mut owners: Vec<_> = self
            .world
            .players
            .iter()
            .flatten()
            .map(|p| p.handle)
            .collect();
        owners.extend(self.world.npcs.keys().copied());
        self.animation_effects
            .retain_owners(&owners, &mut self.effects);
        for n in self.world.npcs.values_mut() {
            let pose = n.rig.as_ref().map(|rig| {
                let body = &self.world.physics.bodies[n.body];
                let parent = crate::character_pose::character_orientation(
                    &n.character.scaled_static_transform(&rig.static_transform),
                    &body.orientation,
                    body.position,
                    -0.5 * body.length - body.radius,
                );
                rig.skeleton.skinned_for(&n.motion.controller, &parent)
            });
            for event in n.animation_events.drain(..) {
                self.spray_lights.event(
                    n.handle,
                    &event,
                    self.world.physics.bodies[n.body].orientation,
                );
                let orientation = match (&event, n.rig.as_ref(), pose.as_ref()) {
                    (AvatarEvent::AnimationEffect { bone, .. }, Some(rig), Some(bones)) => {
                        crate::animation_effects::bone_orientation(
                            &rig.skeleton.skeleton,
                            bones,
                            bone,
                        )
                    }
                    _ => None,
                };
                self.animation_effects
                    .event(n.handle, &event, orientation, &mut self.effects);
                self.spray_marks.entry(n.handle).or_default().event(
                    &event,
                    &mut self.world.decals,
                    self.world.collision.collision(),
                    &mut self.effects,
                );
                self_presentation(
                    &mut self.effects,
                    self.self_effects.entry(n.handle).or_default(),
                    &event,
                );
                spray_presentation(
                    &mut self.effects,
                    self.spray_effects.entry(n.handle).or_default(),
                    &event,
                );
                if let AvatarEvent::LightningBolt(bolt) = &event {
                    let _ = self.effects.start_at(
                        custom_hash("lightning_source"),
                        bolt.from,
                        bolt.direction,
                    );
                    let _ = self.effects.start_at(
                        custom_hash("lightning_hit"),
                        bolt.to,
                        bolt.direction,
                    );
                    self.bolt_serial = self.bolt_serial.wrapping_add(1);
                    self.bolts.push(LiveBolt {
                        visual: *bolt,
                        ttl: bolt.ttl,
                        serial: self.bolt_serial,
                    });
                }
                self.events.push(SessionEvent::NpcSpell {
                    owner: n.handle,
                    event,
                });
            }
        }

        for p in self.world.players.iter_mut().flatten() {
            let pose = p.rig.as_ref().map(|rig| {
                let body = &self.world.physics.bodies[p.body];
                let parent = crate::character_pose::character_orientation(
                    &p.character.scaled_static_transform(&rig.static_transform),
                    &body.orientation,
                    body.position,
                    -0.5 * body.length - body.radius,
                );
                rig.skeleton.skinned_for(&p.motion.controller, &parent)
            });
            self.events
                .extend(p.motion.controller.cues.drain(..).map(SessionEvent::Clip));
            for event in &p.events {
                self.spray_lights.event(
                    p.handle,
                    event,
                    self.world.physics.bodies[p.body].orientation,
                );
                let orientation = match (event, p.rig.as_ref(), pose.as_ref()) {
                    (AvatarEvent::AnimationEffect { bone, .. }, Some(rig), Some(bones)) => {
                        crate::animation_effects::bone_orientation(
                            &rig.skeleton.skeleton,
                            bones,
                            bone,
                        )
                    }
                    _ => None,
                };
                self.animation_effects
                    .event(p.handle, event, orientation, &mut self.effects);
                self.spray_marks.entry(p.handle).or_default().event(
                    event,
                    &mut self.world.decals,
                    self.world.collision.collision(),
                    &mut self.effects,
                );
                self_presentation(
                    &mut self.effects,
                    self.self_effects.entry(p.handle).or_default(),
                    event,
                );
                spray_presentation(
                    &mut self.effects,
                    self.spray_effects.entry(p.handle).or_default(),
                    event,
                );
                if let AvatarEvent::LightningBolt(bolt) = event {
                    // InitializeEffect: lightning_source at the start,
                    // lightning_hit at the end, along the bolt direction.
                    let _ = self.effects.start_at(
                        custom_hash("lightning_source"),
                        bolt.from,
                        bolt.direction,
                    );
                    let _ = self.effects.start_at(
                        custom_hash("lightning_hit"),
                        bolt.to,
                        bolt.direction,
                    );
                    self.bolt_serial = self.bolt_serial.wrapping_add(1);
                    self.bolts.push(LiveBolt {
                        visual: *bolt,
                        ttl: bolt.ttl,
                        serial: self.bolt_serial,
                    });
                }
            }
            self.events
                .extend(p.events.drain(..).map(SessionEvent::Avatar));
        }
        for (&handle, n) in self.world.npcs.iter_mut() {
            self.events.extend(
                n.motion
                    .controller
                    .cues
                    .drain(..)
                    .map(|cue| SessionEvent::NpcClip(handle, cue)),
            );
        }
        let spray_owners = self
            .world
            .players
            .iter()
            .flatten()
            .filter(|p| {
                !p.dead()
                    && matches!(&p.current_spell,
                Some(crate::avatar::SpellEffect::ForceSpray(s)) if s.lifecycle.active)
            })
            .map(|p| p.handle)
            .chain(
                self.world
                    .npcs
                    .values()
                    .filter(|n| {
                        !n.character.combat.dead
                            && matches!(&n.current_spell,
                Some(crate::avatar::SpellEffect::ForceSpray(s)) if s.lifecycle.active)
                    })
                    .map(|n| n.handle),
            )
            .collect();
        self.spray_lights.retain_owners(&spray_owners);
        for index in std::mem::take(&mut self.runtime.skipped) {
            let name = self
                .runtime
                .unsupported_name(index)
                .unwrap_or("?")
                .to_ascii_lowercase();
            if self.reported.insert(name.clone()) {
                self.events.push(SessionEvent::ScriptSkipped(name));
            }
        }
    }
}

/// Retained ground marks and fire-scourge references per live caster.
#[derive(Default)]
struct SprayMarks {
    marks: crate::spray_decals::SprayDecals,
    scourge: [[crate::particles::EffectRef; 2]; 24],
}
impl SprayMarks {
    fn event(
        &mut self,
        event: &AvatarEvent,
        pool: &mut crate::decals::Decals,
        scene: &crate::level_collision::SceneCollision,
        effects: &mut crate::particles::EffectSystem,
    ) {
        match event {
            AvatarEvent::SprayStarted { spell, .. } => {
                self.marks.begin(*spell);
                self.scourge = [[Default::default(); 2]; 24];
            }
            AvatarEvent::SprayStopped => {
                self.marks.clear();
                self.scourge = [[Default::default(); 2]; 24];
            }
            AvatarEvent::SprayDecalUpdate(frame) => {
                if let Some(update) = self.marks.update(*frame, pool, |segment| {
                    scene.segment_intersect(segment, false)
                }) {
                    if update.fire {
                        for (k, mut position) in update.created {
                            if k > 1 && !effects.is_active(&self.scourge[update.key][k - 2]) {
                                position[1] += 0.5;
                                let mut transform = magicka_content::skinned::IDENTITY;
                                transform[12..15].copy_from_slice(&position);
                                self.scourge[update.key][k - 2] = effects
                                    .start(custom_hash("fire_scourge"), transform)
                                    .unwrap_or_default();
                            }
                        }
                        for r in &mut self.scourge[update.key] {
                            effects.restart(r);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn spray_presentation(
    effects: &mut crate::particles::EffectSystem,
    refs: &mut Vec<crate::particles::EffectRef>,
    event: &AvatarEvent,
) {
    use magicka_sim::spells::Elements as E;
    match event {
        AvatarEvent::SprayStarted {
            spell,
            position,
            direction,
            ..
        } => {
            for i in 0..11 {
                let element = E(1 << i);
                if spell.get(element) as i32 <= 0 {
                    continue;
                }
                let name = match element {
                    E::WATER => "spray_water",
                    E::COLD => "spray_cold",
                    E::FIRE => "spray_fire",
                    E::LIGHTNING => "spray_lightning",
                    E::STEAM => "spray_steam",
                    E::POISON => "spray_poison",
                    _ => "",
                };
                let hash = custom_hash(&format!("{name}1"));
                refs.push(
                    effects
                        .start_at(hash, *position, *direction)
                        .unwrap_or_default(),
                );
            }
        }
        AvatarEvent::SprayOrientation(m) => {
            for r in refs.iter_mut() {
                effects.update_orientation(r, *m);
            }
        }
        AvatarEvent::SprayStopped => {
            for mut r in refs.drain(..) {
                effects.stop(&mut r);
            }
        }
        _ => {}
    }
}

/// Level dialog collection with its presentation-free text host: Message
/// layouts measure with Maiandra16 (spacing 1); TextBox bodies use Maiandra14.
fn install_dialogs(
    world: &mut SceneWorld<FsSceneContent>,
    content: &Path,
    definitions: Vec<crate::dialog_definition::DialogDefinition>,
) {
    let fonts = (
        crate::play_content::bitmap_font(content, "Languages/eng/Font/Maiandra14"),
        crate::play_content::bitmap_font(content, "Languages/eng/Font/Maiandra16"),
    );
    let (Ok(body), Ok(name)) = fonts else {
        return;
    };
    let metrics = (
        crate::bitmap_font::FontMetrics::from_content(&body),
        crate::bitmap_font::FontMetrics::from_content(&name),
    );
    let (Ok(body), Ok(mut name)) = metrics else {
        return;
    };
    name.spacing = 1;
    let Ok(manager) = crate::dialog_manager::DialogManager::new(definitions.clone()) else {
        return;
    };
    let strings = crate::play_content::english_strings(content, &["Wizard"]);
    let mut text = crate::dialog_text_host::DialogTextHost::new(
        crate::dialog_world::SceneDialogWorld::default(),
        strings,
        body,
        name,
        100,
    );
    text.scene = Some(crate::dialog_world::LIVE_SCENE);
    text.select_collection(manager.collection_token());
    if text.initialize_layouts(&definitions).is_ok() {
        world.dialog_text = Some(text);
    }
    world.dialogs = Some(manager);
}

fn self_presentation(
    effects: &mut crate::particles::EffectSystem,
    refs: &mut Vec<crate::particles::EffectRef>,
    event: &AvatarEvent,
) {
    match event {
        AvatarEvent::IceBladeSpawned { orientation } => {
            effects.start(custom_hash("weapon_ice_spawn"), *orientation);
        }
        AvatarEvent::IceBladeStrike {
            orientation: Some(orientation),
            ..
        } => {
            effects.start(custom_hash("weapon_ice_death"), *orientation);
        }
        AvatarEvent::ProjectileBlocked {
            element,
            position,
            direction,
        } => {
            let name = if *element == magicka_sim::spells::Elements::EARTH {
                "projectilehit_earth"
            } else {
                "projectilehit_ice"
            };
            effects.start_at(custom_hash(name), *position, *direction);
        }
        AvatarEvent::PushStarted {
            cast_type,
            position,
            direction,
        } => {
            let name = if *cast_type == magicka_sim::damage::CastType::Area {
                "push_area"
            } else {
                "push_force"
            };
            effects.start_at(custom_hash(name), *position, *direction);
        }
        AvatarEvent::SelfEffectStarted {
            name,
            position,
            direction,
            ..
        } => {
            refs.push(
                effects
                    .start_at(custom_hash(name), *position, *direction)
                    .unwrap_or_default(),
            );
        }
        AvatarEvent::SelfEffectsPosition {
            position,
            direction,
        } => {
            for r in refs {
                effects.update_position_direction(r, *position, *direction);
            }
        }
        AvatarEvent::SelfEffectsStopped => {
            for mut r in refs.drain(..) {
                effects.stop(&mut r);
            }
        }
        _ => {}
    }
}

/// A LightningBolt being drawn: `serial` picks one of the pooled strips
/// (GetLightning takes a random cached bolt, each with its own jitter).
#[derive(Clone, Copy, Debug)]
pub struct LiveBolt {
    pub visual: magicka_sim::lightning::BoltVisual,
    pub ttl: f32,
    pub serial: u32,
}

impl PlaySession {
    /// DeadState.OnEnter releases each item as a dynamic, pickable world entity.
    fn release_dead_equipment(&mut self) -> Result<(), SessionError> {
        let mut drops = Vec::new();
        for player in self.world.players.iter().flatten() {
            let Some(rig) = &player.rig else { continue };
            let body = &self.world.physics.bodies[player.body];
            let parent = crate::character_pose::character_orientation(
                &player
                    .character
                    .scaled_static_transform(&rig.static_transform),
                &body.orientation,
                body.position,
                -body.radius - 0.5 * body.length,
            );
            let pose = rig.skeleton.skinned_for(&player.motion.controller, &parent);
            for event in &player.events {
                let AvatarEvent::ItemReleased { slot } = event else {
                    continue;
                };
                let Some(attachment) = rig.attachments.iter().find(|a| a.slot == *slot) else {
                    continue;
                };
                let Some((_, item)) = player.items.iter().find(|(s, _)| s == slot) else {
                    continue;
                };
                drops.push((
                    player.slot,
                    *slot,
                    item.definition.name.clone(),
                    attachment.item_transform(&pose),
                ));
            }
        }
        for (player, slot, name, transform) in drops {
            let handle = self.spawn_item_pickup(&name, transform)?;
            let held = &mut self.world.players[player]
                .as_mut()
                .unwrap()
                .items
                .iter_mut()
                .find(|(s, _)| *s == slot)
                .unwrap()
                .1;
            let mut reference = crate::avatar::AvatarItem::new(held.definition.clone());
            reference.attached = false;
            let mut released = std::mem::replace(held, reference);
            released.frame = None;
            let pickup = self.world.item_pickups.get_mut(&handle).unwrap();
            pickup.item = Some(released);
            pickup.immovable = false;
        }
        Ok(())
    }
    /// Developer spawning uses the same detached-item host as SpawnItem.
    /// Dynamic-body integration is still pending; this creates an immovable pickup.
    pub fn spawn_item_pickup(
        &mut self,
        name: &str,
        transform: [f32; 16],
    ) -> Result<u16, SessionError> {
        use crate::spawn_item::SpawnItemHost;
        self.world
            .load_item(&format!("Data/Items/Wizard/{name}"))
            .map_err(SessionError::World)?;
        let handle = self.world.new_placeholder().map_err(SessionError::World)?;
        self.world
            .copy_and_detach(handle, custom_hash(name), 0)
            .map_err(SessionError::World)?;
        let position = [transform[12], transform[13], transform[14]];
        self.world
            .move_to(handle, position, transform)
            .map_err(SessionError::World)?;
        self.world.add_entity(handle).map_err(SessionError::World)?;
        self.world
            .configure(handle, true, false, None)
            .map_err(SessionError::World)?;
        self.world
            .enable_body(handle)
            .map_err(SessionError::World)?;
        Ok(handle)
    }

    pub fn nearby_item(&self) -> Option<u16> {
        crate::pickups::find_pickup(
            self.world.entities.iter().filter_map(|entity| {
                let pickup = self.world.item_pickups.get(&entity.handle)?;
                Some(crate::pickups::PickupCandidate {
                    handle: entity.handle,
                    position: entity.position,
                    radius: entity.radius,
                    pickable: entity.added
                        && !entity.removed
                        && pickup.item.as_ref()?.definition.pickable,
                    dead: entity.dead,
                })
            }),
            self.body().position,
            self.body().direction(),
            self.body().radius,
            false,
            None,
        )
    }

    fn player_avatar(&self, slot: usize) -> Result<&PlayerAvatar, SessionError> {
        self.world
            .players
            .get(slot)
            .and_then(Option::as_ref)
            .ok_or_else(|| SessionError::World("pickup player unavailable".into()))
    }
    fn player_body(&self, slot: usize) -> Result<&CharacterBody, SessionError> {
        Ok(&self.world.physics.bodies[self.player_avatar(slot)?.body])
    }
    /// Modern host pickup: lifetime and range checks precede gameplay side effects.
    pub fn pickup_remote(
        &mut self,
        slot: usize,
        target: magicka_network::control::PickupTarget,
    ) -> magicka_network::control::PickupOutcome {
        use magicka_network::control::{
            PickupFailure as F, PickupOutcome as O, PickupRejection as R,
        };
        let Ok(avatar) = self.player_avatar(slot) else {
            return O::Rejected(R::Player);
        };
        if avatar.dead() {
            return O::Rejected(R::Player);
        }
        if self.world.flags.input_limited
            || self.world.camera.lock_input
            || self.world.player_input_locked[slot]
            || self.end_game.input_limited
            || self.world.flags.cutscene
        {
            return O::Rejected(R::InputLimited);
        }
        let Some(candidate) = self.pickup_candidate(target.handle) else {
            return O::Rejected(R::Unavailable);
        };
        if self.pickup_generation(target.handle) != Some(target.generation) {
            return O::Rejected(R::Lifetime);
        }
        if candidate.dead || !candidate.pickable {
            return O::Rejected(R::Unavailable);
        }
        let body = self.player_body(slot).unwrap();
        if crate::pickups::find_pickup(
            [candidate],
            body.position,
            body.direction(),
            body.radius,
            false,
            Some(target.handle),
        )
        .is_none()
        {
            return O::Rejected(R::OutOfRange);
        }
        let result = if self.world.book_pickups.contains_key(&target.handle) {
            self.pickup_book_for_player(slot, target.handle)
                .map(|magick| O::Book { magick })
        } else {
            self.pickup_item_for_player(slot, target.handle)
                .map(|exchange| match exchange {
                    crate::pickups::EquipmentExchange::Swapped => O::Swapped,
                    crate::pickups::EquipmentExchange::Bound => O::Bound,
                })
        };
        match result {
            Ok(outcome) => outcome,
            Err(error) => {
                let failure = match &error {
                    SessionError::Content(_) | SessionError::Template(_) => F::Content,
                    SessionError::Script(_) => F::Script,
                    _ => F::World,
                };
                self.events
                    .push(SessionEvent::PickupFailed(format!("{error:?}")));
                O::Failed(failure)
            }
        }
    }

    /// Native equipment exchange without resetting the avatar. The world item
    /// retains its handle and now contains the replaced equipment.
    pub fn pickup_item(
        &mut self,
        handle: u16,
    ) -> Result<crate::pickups::EquipmentExchange, SessionError> {
        self.pickup_item_for_player(0, handle)
    }
    fn pickup_item_for_player(
        &mut self,
        player: usize,
        handle: u16,
    ) -> Result<crate::pickups::EquipmentExchange, SessionError> {
        self.player_avatar(player)?;
        use crate::pickups::{equipment_slot, exchange_equipment, EquipmentExchange};
        let e = self
            .world
            .entities
            .iter()
            .find(|e| e.handle == handle && e.added && !e.removed && !e.dead)
            .ok_or_else(|| SessionError::World("pickup entity is unavailable".into()))?;
        let pickup = self
            .world
            .item_pickups
            .get(&handle)
            .ok_or_else(|| SessionError::World("not an item pickup".into()))?;
        let incoming = pickup
            .item
            .as_ref()
            .ok_or_else(|| SessionError::World("empty item pickup".into()))?;
        let found = crate::pickups::find_pickup(
            [crate::pickups::PickupCandidate {
                handle,
                position: e.position,
                radius: e.radius,
                pickable: incoming.definition.pickable,
                dead: e.dead,
            }],
            self.player_body(player)?.position,
            self.player_body(player)?.direction(),
            self.player_body(player)?.radius,
            false,
            Some(handle),
        );
        if found.is_none() {
            return Err(SessionError::World("item is out of pickup range".into()));
        }
        let on_pickup = pickup.on_pickup;
        let generation = pickup.generation;
        // InternalPickUp queues attacking before executing the pickup trigger,
        // and runs that trigger even when the equipped item is bound.
        let avatar = self.world.players[player]
            .as_mut()
            .ok_or_else(|| SessionError::World("pickup player unavailable".into()))?;
        avatar.motion.attacking = true;
        avatar.motion.allow_attack_rotate = false;
        if on_pickup != 0 {
            let sender = self.player_avatar(player)?.handle;
            self.runtime
                .execute_trigger(
                    on_pickup,
                    Some(sender),
                    false,
                    self.world.role,
                    &mut self.world,
                )
                .map_err(|e| SessionError::Script(format!("{e:?}")))?;
        }
        // A synchronous pickup trigger may remove or replace its sender/target.
        if !self
            .world
            .entities
            .iter()
            .any(|e| e.handle == handle && e.added && !e.removed && !e.dead)
        {
            return Err(SessionError::World(
                "pickup was removed by its trigger".into(),
            ));
        }
        if self.pickup_generation(handle) != Some(generation) {
            return Err(SessionError::World(
                "pickup was replaced by its trigger".into(),
            ));
        }
        let incoming = self
            .world
            .item_pickups
            .get(&handle)
            .and_then(|p| p.item.as_ref())
            .ok_or_else(|| SessionError::World("pickup was emptied by its trigger".into()))?;
        let slot = equipment_slot(incoming);
        let name = incoming.definition.name.clone();
        let display_name = incoming.definition.display_name.clone();
        let index = self
            .player_avatar(player)?
            .items
            .iter()
            .position(|(s, _)| *s == slot)
            .ok_or_else(|| SessionError::Template(format!("no equipment slot {slot}")))?;
        if self.player_avatar(player)?.items[index].1.definition.bound {
            return Ok(EquipmentExchange::Bound);
        }
        // Item.Read replaces #1; only for a non-empty display name.
        self.notify_obtained(
            player,
            "#item_pick_up",
            (!display_name.is_empty()).then_some(display_name.as_str()),
        )?;
        // Prepare content before mutating either item, preserving the live avatar.
        let mut template = load_character(
            &self.content,
            self.remote_characters
                .get(&player)
                .unwrap_or(&self.character),
        )
        .map_err(SessionError::Content)?;
        for (s, attachment) in &mut template.equipment {
            if let Some((_, item)) = self
                .player_avatar(player)?
                .items
                .iter()
                .find(|(i, _)| i == s)
            {
                let n = if *s == slot {
                    &name
                } else {
                    &item.definition.name
                };
                attachment.item = Some(format!("../items/Wizard/{n}"));
            }
        }
        let rig = crate::play_content::load_rig(&self.content, &template)
            .map_err(SessionError::Content)?;
        let animations = Arc::new(AnimationSet::with_weapons(
            &template.animation_sets,
            &crate::play_content::weapon_classes(rig.as_ref()),
            rig.as_ref()
                .map(|r| r.skeleton.clip_durations())
                .unwrap_or_default(),
        ));
        let dropped_transform = self
            .player_avatar(player)?
            .rig
            .as_ref()
            .and_then(|rig| {
                let a = rig.attachments.iter().find(|a| a.slot == slot)?;
                let body = self.player_body(player).ok()?;
                let parent = crate::character_pose::character_orientation(
                    &self
                        .player_avatar(player)
                        .ok()?
                        .character
                        .scaled_static_transform(&rig.static_transform),
                    &body.orientation,
                    body.position,
                    -body.radius - 0.5 * body.length,
                );
                let pose = rig
                    .skeleton
                    .skinned_for(&self.player_avatar(player).ok()?.motion.controller, &parent);
                Some(a.item_transform(&pose))
            })
            .ok_or_else(|| {
                SessionError::Template("pickup requires an equipment attachment".into())
            })?;
        let avatar = self.world.players[player]
            .as_mut()
            .ok_or_else(|| SessionError::World("pickup player unavailable".into()))?;
        let pickup = self
            .world
            .item_pickups
            .get_mut(&handle)
            .ok_or_else(|| SessionError::World("pickup was removed by its trigger".into()))?;
        let ground = pickup
            .item
            .as_mut()
            .ok_or_else(|| SessionError::World("pickup was emptied by its trigger".into()))?;
        exchange_equipment(&mut avatar.items[index].1, ground);
        pickup.transform = dropped_transform;
        pickup.immovable = false;
        pickup.generation = pickup.generation.wrapping_add(1);
        ground.frame = None;
        avatar.rig = rig;
        avatar.motion.animations = animations;
        avatar
            .motion
            .attack(if slot == 1 { 148 } else { 147 }, false);
        let owner = avatar.handle;
        let stopped: Vec<_> = self
            .item_effects
            .keys()
            .filter(|(h, s, _)| *h == owner && *s == slot)
            .copied()
            .collect();
        for key in stopped {
            if let Some(mut r) = self.item_effects.remove(&key) {
                self.effects.stop(&mut r);
            }
        }
        if let Some(entity) = self.world.entities.iter_mut().find(|e| e.handle == handle) {
            entity.position = [
                dropped_transform[12],
                dropped_transform[13],
                dropped_transform[14],
            ];
            entity.orientation = dropped_transform;
        }
        if let Some(physics) = &mut self.pickup_physics {
            physics.remove(handle);
        }
        Ok(EquipmentExchange::Swapped)
    }
}

impl PlaySession {
    /// Spawn the same runtime entity used by script SpawnPhys at a debug transform.
    pub fn spawn_physics_prop(
        &mut self,
        name: &str,
        transform: [f32; 16],
    ) -> Result<u16, SessionError> {
        use crate::spawn_phys::{SpawnPhysHost, SpawnPhysLevel};
        let name = name.trim_end_matches(".xnb").to_ascii_lowercase();
        let template = crate::play_content::load_physics_template(&self.content, &name)
            .map_err(SessionError::Content)?;
        if crate::physics_props::body(0, 0, &name, transform, &template).is_none() {
            return Err(SessionError::World(format!(
                "prop {name} has no supported collision shape"
            )));
        }
        let handle = if template.max_hitpoints > 0 {
            self.world.damageable_from_cache()
        } else {
            self.world.new_physics_entity()
        }
        .map_err(SessionError::World)?;
        self.world
            .initialize_entity(handle, Some(&name), transform, 0)
            .map_err(SessionError::World)?;
        self.world.add_entity(handle).map_err(SessionError::World)?;
        self.physics_templates.insert(name, Some(template));
        Ok(handle)
    }
    /// The scene lifetime counter (advances on scene rebuilds/restarts).
    pub fn network_epoch(&self) -> u64 {
        self.network_epoch
    }
    fn next_network_epoch(&self) -> Result<u64, SessionError> {
        self.network_epoch.checked_add(1).ok_or_else(|| {
            SessionError::World("network scene epoch exhausted; start a new session".into())
        })
    }
    /// Full rigid-body snapshot. Scene rebuilds advance the epoch automatically.
    /// The host owns session identity and strictly increasing snapshot ticks;
    /// replacing this entire PlaySession with a fresh load needs a new session ID.
    pub fn physics_frame(
        &self,
        session: u64,
        tick: u64,
    ) -> Result<magicka_network::PhysicsFrame, magicka_network::FrameError> {
        let bodies = match &self.pickup_physics {
            Some(physics) => physics.network_snapshot()?,
            None => Vec::new(),
        };
        let frame = magicka_network::PhysicsFrame {
            version: magicka_network::PROTOCOL_VERSION,
            session,
            epoch: self.network_epoch,
            tick,
            scene: self.scene_name.clone(),
            bodies,
        };
        frame.validate()?;
        Ok(frame)
    }
    /// Magick fields on rigid props and pickups. Vortex (native): bodies in
    /// reach gain velocity += pull each frame; one at the core is killed and
    /// feeds the mass. Tornado (user-requested enhancement, not native):
    /// suction, lift up the funnel and a fling from the top.
    fn magick_prop_fields(&mut self, dt: f32) {
        use magicka_sim::tornado::field::{self, Effect};
        use magicka_sim::vortex::CORE_RADIUS;
        /// Props are treated as 0.3 m spheres for the field tests.
        const PROP_RADIUS: f32 = 0.3;
        if self.world.vortices.is_empty() && self.world.tornadoes.is_empty() {
            return;
        }
        let Some(physics) = self.pickup_physics.as_mut() else {
            return;
        };
        let mut absorbed = Vec::new();
        for (handle, state, mass) in physics.dynamic_bodies() {
            let p = state.position;
            let mut dv = [0.0f32; 3];
            for live in &mut self.world.vortices {
                let v = &mut live.vortex;
                if v.radius <= 0.0 {
                    continue;
                }
                let d = (0..3)
                    .map(|i| (p[i] - v.center[i]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                if d > v.radius + PROP_RADIUS {
                    continue;
                }
                if d <= CORE_RADIUS + PROP_RADIUS {
                    v.absorb(mass);
                    absorbed.push(handle);
                    break;
                }
                if let Some(f) = v.pull(p) {
                    dv = std::array::from_fn(|i| dv[i] + f[i]);
                }
            }
            for live in &mut self.world.tornadoes {
                match field::effect(&live.tornado, handle, p, PROP_RADIUS, dt) {
                    // The character field is a per-frame velocity on the
                    // ground; loose props take it as an acceleration.
                    Some(Effect::Pull(f)) => dv = std::array::from_fn(|i| dv[i] + f[i] * dt * 4.0),
                    Some(Effect::Lift(v)) => dv = std::array::from_fn(|i| dv[i] + v[i]),
                    Some(Effect::Fling(v)) => {
                        dv = std::array::from_fn(|i| dv[i] + v[i]);
                        live.tornado.flung.add_for(handle, field::FLING_COOLDOWN);
                    }
                    None => {}
                }
            }
            if dv.iter().any(|v| *v != 0.0) {
                physics.add_velocity(handle, dv);
            }
        }
        for handle in absorbed {
            self.world.item_pickups.remove(&handle);
            self.world.book_pickups.remove(&handle);
            if let Some(e) = self.world.entities.iter_mut().find(|e| e.handle == handle) {
                e.dead = true;
                e.removed = true;
            }
        }
    }

    /// Pre-order live motion of every animated level part.
    fn part_motions(&self) -> Vec<crate::pickup_physics::PartMotion> {
        fn walk(
            parts: &[crate::animated_hierarchy::AnimatedPartState],
            out: &mut Vec<(magicka_content::skinned::Mat, bool)>,
        ) {
            for p in parts {
                out.push((p.world, p.playback.playing));
                walk(&p.children, out);
            }
        }
        let mut worlds = Vec::new();
        walk(self.world.collision.parts(), &mut worlds);
        let deltas = self.world.collision.part_deltas().unwrap_or_default();
        worlds
            .into_iter()
            .enumerate()
            .map(|(i, (world, playing))| crate::pickup_physics::PartMotion {
                world,
                delta: deltas
                    .get(i)
                    .copied()
                    .unwrap_or(magicka_content::skinned::IDENTITY),
                playing,
            })
            .collect()
    }
    fn update_pickup_physics(&mut self, dt: f32) {
        use crate::pickup_physics::{PickupBody, PickupPhysics};
        use magicka_physics::BodyKey;
        let descriptors: Vec<_> = self
            .world
            .entities
            .iter()
            .filter(|e| e.added && !e.removed && !e.dead)
            .filter_map(|e| {
                if let Some(book) = self.world.book_pickups.get(&e.handle) {
                    if book.dead {
                        return None;
                    }
                    return Some((
                        e.handle,
                        book.generation,
                        PathBuf::from(crate::magick_books::BOOK_MODEL),
                        book.transform,
                        true,
                        book.immovable,
                    ));
                }
                let pickup = self.world.item_pickups.get(&e.handle)?;
                let model = pickup.item.as_ref()?.definition.model.as_ref()?;
                Some((
                    e.handle,
                    pickup.generation,
                    Path::new("Data/Items/Wizard").join(model.replace('\\', "/")),
                    pickup.transform,
                    false,
                    pickup.immovable,
                ))
            })
            .collect();
        let mut bodies = Vec::new();
        for (handle, generation, path, transform, book, immovable) in descriptors {
            let bounds = self
                .pickup_bounds
                .entry(path.clone())
                .or_insert_with(|| crate::play_content::pickup_model_bounds(&self.content, &path));
            if let Some(bounds) = bounds {
                bodies.push(PickupBody {
                    key: BodyKey { handle, generation },
                    model: path.to_string_lossy().into_owned(),
                    transform,
                    bounds: *bounds,
                    book,
                    immovable,
                    mass: 50.,
                    solid: true,
                    shape: None,
                });
            } else if self
                .reported
                .insert(format!("physics bounds {}", path.display()))
            {
                self.events.push(SessionEvent::PhysicsFailed(format!(
                    "missing pickup bounds: {}",
                    path.display()
                )));
            }
        }
        for entity in &self.world.entities {
            if !entity.added
                || entity.removed
                || entity.dead
                || self.world.item_pickups.contains_key(&entity.handle)
                || self.world.book_pickups.contains_key(&entity.handle)
            {
                continue;
            }
            if !matches!(entity.kind, crate::scene_world::EntityKind::Physics { .. }) {
                continue;
            }
            let Some(name) = &entity.template_name else {
                continue;
            };
            let template = self
                .physics_templates
                .entry(name.clone())
                .or_insert_with(|| {
                    crate::play_content::load_physics_template(&self.content, name).ok()
                });
            let descriptor = template.as_ref().and_then(|t| {
                crate::physics_props::body(
                    entity.handle,
                    *self
                        .world
                        .physics_generations
                        .get(&entity.handle)
                        .unwrap_or(&0),
                    name,
                    entity.orientation,
                    t,
                )
            });
            if let Some(body) = descriptor {
                bodies.push(body);
            } else if self.reported.insert(format!("physics prop {name}")) {
                self.events.push(SessionEvent::PhysicsFailed(format!(
                    "unsupported collision or template for prop {name}"
                )));
            }
        }
        if bodies.is_empty() && self.pickup_physics.is_none() {
            return;
        }
        if self.pickup_physics.is_none() {
            match PickupPhysics::new(&self.world.level) {
                Ok(physics) => self.pickup_physics = Some(physics),
                Err(error) => {
                    if self.reported.insert("pickup physics initialization".into()) {
                        self.events.push(SessionEvent::PhysicsFailed(format!(
                            "initialization: {error:?}"
                        )));
                    }
                    return;
                }
            }
        }
        let release_velocities = std::mem::take(&mut self.pending_pickup_velocities);
        // AnimatedLevelPart.Update / PostCollision for rigid bodies.
        let motions = self.part_motions();
        if let Err(error) = self
            .pickup_physics
            .as_mut()
            .unwrap()
            .carry_on_parts(&motions, dt)
        {
            if self.reported.insert(format!("pickup part carry {error:?}")) {
                self.events.push(SessionEvent::PhysicsFailed(format!(
                    "part carry: {error:?}"
                )));
            }
        }
        let stepped = self
            .pickup_physics
            .as_mut()
            .unwrap()
            .step_with_release_velocities(bodies, dt, &release_velocities);
        if stepped.is_ok() {
            let playing: Vec<_> = motions.iter().map(|m| m.playing).collect();
            let collision = self.world.collision.collision();
            self.pickup_physics.as_mut().unwrap().refresh_part_contacts(
                &playing,
                |origin, depth| {
                    let hit = collision.segment_intersect(
                        magicka_sim::spray_pulse::Segment {
                            origin,
                            delta: [0.0, -depth, 0.0],
                        },
                        false,
                    )?;
                    Some((hit.animated_owner?, hit.hit.normal[1]))
                },
            );
        }
        match stepped {
            Ok(poses) => {
                let transforms: BTreeMap<_, _> = poses
                    .iter()
                    .map(|(handle, transform, _)| (*handle, *transform))
                    .collect();
                for (handle, transform, state) in poses {
                    if let Some(book) = self.world.book_pickups.get_mut(&handle) {
                        book.transform = transform;
                        book.velocity = state.linear_velocity;
                    } else if let Some(pickup) = self.world.item_pickups.get_mut(&handle) {
                        pickup.transform = transform;
                    }
                }
                for entity in &mut self.world.entities {
                    if let Some(transform) = transforms.get(&entity.handle) {
                        entity.orientation = *transform;
                        entity.position = [transform[12], transform[13], transform[14]];
                    }
                }
            }
            Err(error) => {
                if self
                    .reported
                    .insert(format!("pickup physics step {error:?}"))
                {
                    self.events
                        .push(SessionEvent::PhysicsFailed(format!("step: {error:?}")));
                }
            }
        }
    }
    fn pickup_candidate(&self, handle: u16) -> Option<crate::pickups::PickupCandidate> {
        let entity = self.world.entities.iter().find(|e| e.handle == handle)?;
        let pickable = self
            .world
            .book_pickups
            .get(&handle)
            .is_some_and(|b| !b.dead)
            || self
                .world
                .item_pickups
                .get(&handle)
                .and_then(|p| p.item.as_ref())
                .is_some_and(|i| i.definition.pickable);
        Some(crate::pickups::PickupCandidate {
            handle,
            position: entity.position,
            radius: entity.radius,
            pickable: pickable && entity.added && !entity.removed,
            dead: entity.dead,
        })
    }
    pub fn pickup_in_range(&self, handle: u16) -> bool {
        crate::pickups::find_pickup(
            self.pickup_candidate(handle),
            self.body().position,
            self.body().direction(),
            self.body().radius,
            false,
            Some(handle),
        )
        .is_some()
    }
    fn pickup_generation(&self, handle: u16) -> Option<u64> {
        self.world
            .book_pickups
            .get(&handle)
            .map(|b| b.generation)
            .or_else(|| self.world.item_pickups.get(&handle).map(|p| p.generation))
    }
    /// Real keyboard/mouse pickup targeting; caller supplies the camera ray
    /// rather than a viewer entity or a debug selection.
    pub fn cursor_pickup(&mut self, segment: Segment, plane: Vector) -> Option<u16> {
        let mut candidates = Vec::new();
        for entity in &self.world.entities {
            let Some(candidate) = self.pickup_candidate(entity.handle) else {
                continue;
            };
            if !candidate.pickable || candidate.dead {
                continue;
            }
            let (path, transform, book) =
                if let Some(b) = self.world.book_pickups.get(&entity.handle) {
                    (
                        PathBuf::from(crate::magick_books::BOOK_MODEL),
                        b.transform,
                        true,
                    )
                } else {
                    let Some(p) = self.world.item_pickups.get(&entity.handle) else {
                        continue;
                    };
                    let Some(item) = &p.item else {
                        continue;
                    };
                    let Some(model) = &item.definition.model else {
                        continue;
                    };
                    (
                        Path::new("Data/Items/Wizard").join(model.replace('\\', "/")),
                        p.transform,
                        false,
                    )
                };
            let bounds = self
                .pickup_bounds
                .entry(path.clone())
                .or_insert_with(|| crate::play_content::pickup_model_bounds(&self.content, &path));
            if let Some(bounds) = bounds {
                candidates.push((candidate, bounds.world_box(&transform, book)));
            }
        }
        crate::pickups::mouse_pickup(candidates, segment, plane)
    }
    fn handle_mouse_character(
        &mut self,
        input: PlayerInput,
        pressed: bool,
        dialog: bool,
        limited: bool,
        pick: &mut Option<Vector>,
    ) {
        if !input.move_down {
            self.character_pressed_target = None;
        }
        self.hovered_character = None;
        if limited
            || dialog
            || self.world.flags.cutscene
            || self.avatar().dead()
            || self.avatar().polymorphed
            || input.shift_down
            || !self.avatar().conjure.queue.is_empty()
            || matches!(
                self.avatar().motion.state,
                crate::character_motion::CharacterState::Cast
                    | crate::character_motion::CharacterState::Charge
            )
        {
            self.character_pressed_target = None;
            return;
        }
        let faction = self.avatar().faction;
        let target = if input.move_down && !pressed {
            self.character_pressed_target
                .filter(|h| self.world.talk_character_available(*h, faction))
        } else {
            input.cursor_ndc.zip(*pick).and_then(|(ndc, plane)| {
                self.world.talk_character_under_ray(
                    Segment {
                        origin: self.world.camera.position(),
                        delta: self.world.camera.mouse_ray(ndc, self.world.aspect),
                    },
                    plane,
                    faction,
                )
            })
        };
        self.hovered_character = target;
        if pressed {
            self.character_pressed_target = target;
        }
        let Some(handle) = target else { return };
        if !input.move_down {
            return;
        }
        if self
            .world
            .talk_character_in_reach(self.body().position, faction)
            == Some(handle)
        {
            match self.world.interact_character(handle, 0) {
                Ok(true) => {
                    self.character_pressed_target = None;
                    self.pickup_move_locked = true;
                    self.drain();
                }
                Ok(false) => {}
                Err(e) => self.fail_scripts(e),
            }
        } else if let Some(entity) = self.world.entities.iter().find(|e| e.handle == handle) {
            *pick = Some(entity.position);
        }
    }

    fn handle_mouse_pickup(
        &mut self,
        input: PlayerInput,
        pressed: bool,
        dialog: bool,
        limited: bool,
        pick: &mut Option<Vector>,
    ) {
        if !input.move_down {
            self.pickup_pressed_target = None;
            self.pickup_move_locked = false;
        }
        self.hovered_pickup = None;
        if limited
            || self.hovered_character.is_some()
            || dialog
            || self.world.flags.cutscene
            || self.avatar().dead()
            || self.avatar().focused
            || self.avatar().polymorphed
            || input.shift_down
            || !self.avatar().conjure.queue.is_empty()
        {
            self.pickup_pressed_target = None;
            return;
        }
        if let Some((target, generation)) = self.pickup_pressed_target {
            if self
                .pickup_candidate(target)
                .is_none_or(|c| c.dead || !c.pickable)
                || self.pickup_generation(target) != Some(generation)
            {
                self.pickup_pressed_target = None;
            }
        }
        let target = if input.move_down && !pressed {
            self.pickup_pressed_target.map(|(handle, _)| handle)
        } else if let Some(plane) = *pick {
            let origin = self.world.camera.position();
            // The ray must continue past the avatar-height plane to hit low
            // equipment. Native GetMousePick returns the far-plane segment.
            let far = input
                .cursor_ndc
                .map(|ndc| self.world.camera.mouse_ray(ndc, self.world.aspect));
            far.and_then(|delta| self.cursor_pickup(Segment { origin, delta }, plane))
        } else {
            None
        };
        self.hovered_pickup = target;
        if pressed {
            self.pickup_pressed_target = target.and_then(|handle| {
                self.pickup_generation(handle)
                    .map(|generation| (handle, generation))
            });
        }
        if let Some(target) = target {
            if input.move_down && self.pickup_in_range(target) {
                let result = if self.world.book_pickups.contains_key(&target) {
                    self.pickup_book(target).map(|_| ())
                } else {
                    self.pickup_item(target).map(|_| ())
                };
                if let Err(error) = result {
                    self.events
                        .push(SessionEvent::PickupFailed(format!("{error:?}")));
                }
                self.pickup_pressed_target = None;
                // Native near-click consumes Move until release. Arriving at
                // a distant pressed target clears its lock and can keep walking.
                self.pickup_move_locked = pressed;
            } else if input.move_down {
                *pick = self.pickup_candidate(target).map(|c| c.position);
            }
        }
    }
    /// KeyboardMouseController's Interactable branch: hover (after
    /// characters and pickables), press in reach → Avatar.Interact, press out
    /// of reach → walk to the locator and interact on arrival.
    fn handle_mouse_interact(
        &mut self,
        input: PlayerInput,
        pressed: bool,
        dialog: bool,
        limited: bool,
        pick: &mut Option<Vector>,
    ) {
        if !input.move_down {
            self.interact_pressed_target = None;
        }
        self.hovered_interactable = None;
        if limited
            || self.hovered_character.is_some()
            || dialog
            || self.world.flags.cutscene
            || self.avatar().dead()
            || self.avatar().polymorphed
            || input.shift_down
            || self.hovered_pickup.is_some()
            || self.pickup_pressed_target.is_some()
        {
            self.interact_pressed_target = None;
            return;
        }
        let target = if input.move_down && !pressed {
            self.interact_pressed_target
        } else if pick.is_some() {
            let origin = self.world.camera.position();
            input
                .cursor_ndc
                .map(|ndc| self.world.camera.mouse_ray(ndc, self.world.aspect))
                .and_then(|delta| self.world.interactable_under_ray(Segment { origin, delta }))
        } else {
            None
        };
        self.hovered_interactable = target;
        if pressed {
            self.interact_pressed_target = target;
        }
        let Some(id) = target else {
            return;
        };
        if !input.move_down {
            return;
        }
        if self.interactable_reach() == Some(id) {
            self.interact();
            self.interact_pressed_target = None;
            self.pickup_move_locked = pressed;
        } else if let Some((position, _)) = self.world.interactable_locator(id) {
            *pick = Some(position);
        }
    }

    /// Avatar.FindInteractable(false) for the local avatar.
    fn interactable_reach(&self) -> Option<i32> {
        let body = self.body();
        self.world
            .interactable_in_reach(body.position, body.radius, None)
    }

    /// Avatar.Interact: outside Cast/Charge and dialog input, the reachable
    /// interactable's Interact(this) executes its trigger (random action
    /// group) with the avatar as interactor.
    pub fn interact(&mut self) -> bool {
        use crate::character_motion::CharacterState;
        let avatar = self.avatar();
        if matches!(
            avatar.motion.state,
            CharacterState::Cast | CharacterState::Charge
        ) || self
            .world
            .dialogs
            .as_ref()
            .is_some_and(|m| m.can_advance(Some(0)))
        {
            return false;
        }
        let handle = avatar.handle;
        let Some(id) = self.interactable_reach() else {
            return false;
        };
        // Mods observe the interaction as it happens (OnInteract).
        self.interactions.push((id, handle));
        if self.script_error.is_some() {
            return false;
        }
        let role = self.world.role;
        let result = self
            .runtime
            .execute_trigger(id, Some(handle), true, role, &mut self.world);
        match result {
            Ok(ran) => {
                self.drain();
                ran
            }
            Err(e) => {
                self.fail_scripts(format!("interact {id}: {e:?}"));
                false
            }
        }
    }

    /// Avatar.FindPickUp(true), preserving entity query order and hemisphere test.
    pub fn pad_pickup_target(&self) -> Option<u16> {
        self.pad_pickup_target_for(0)
    }
    fn pad_pickup_target_for(&self, slot: usize) -> Option<u16> {
        let body = self.player_body(slot).ok()?;
        crate::pickups::find_pickup(
            self.world
                .entities
                .iter()
                .filter_map(|e| self.pickup_candidate(e.handle)),
            body.position,
            body.direction(),
            body.radius,
            true,
            None,
        )
    }
    fn pad_character_target(&self) -> Option<u16> {
        self.pad_character_target_for(0)
    }
    fn pad_character_target_for(&self, slot: usize) -> Option<u16> {
        let body = self.player_body(slot).ok()?;
        let avatar = self.player_avatar(slot).ok()?;
        self.world.entities.iter().find_map(|entity| {
            if !self
                .world
                .talk_character_available(entity.handle, avatar.faction)
            {
                return None;
            }
            let d = std::array::from_fn::<_, 3, _>(|i| entity.position[i] - body.position[i]);
            if d.iter().map(|v| v * v).sum::<f32>() > (3. + entity.radius).powi(2) {
                return None;
            }
            let length = d[0].hypot(d[2]);
            (length > 0.
                && (d[0] * body.direction()[0] + d[2] * body.direction()[2]) / length >= 0.7)
                .then_some(entity.handle)
        })
    }
    fn pad_interactable_target(&self) -> Option<i32> {
        self.pad_interactable_target_for(0)
    }
    fn pad_interactable_target_for(&self, slot: usize) -> Option<i32> {
        let body = self.player_body(slot).ok()?;
        self.world
            .interactable_in_reach(body.position, body.radius, Some(body.direction()))
    }
    /// Native notifier priority differs from Avatar.Action's character-first dispatch.
    pub fn pad_cursor(&self) -> (Cursor, bool) {
        if self.pad_pickup_target().is_some() {
            return (Cursor::PickUp, true);
        }
        if let Some(id) = self.pad_interactable_target() {
            use crate::scene_script::InteractType as T;
            return (
                match self.world.interactable_defs[&id].kind {
                    T::Talk => Cursor::Talk,
                    T::PickUp => Cursor::PickUp,
                    T::Examine => Cursor::Examine,
                    _ => Cursor::Interact,
                },
                true,
            );
        }
        if self.pad_character_target().is_some() {
            return (Cursor::Talk, true);
        }
        (Cursor::Default, false)
    }
    fn handle_pad_action(&mut self, pressed: bool, dialog: bool, limited: bool) {
        self.hovered_pickup = None;
        self.hovered_character = None;
        self.hovered_interactable = None;
        self.pickup_pressed_target = None;
        self.character_pressed_target = None;
        self.interact_pressed_target = None;
        if limited
            || dialog
            || self.world.flags.cutscene
            || self.avatar().dead()
            || self.avatar().polymorphed
        {
            return;
        }
        self.hovered_pickup = self.pad_pickup_target();
        self.hovered_character = self.pad_character_target();
        self.hovered_interactable = self.pad_interactable_target();
        if !pressed
            || matches!(
                self.avatar().motion.state,
                crate::character_motion::CharacterState::Cast
                    | crate::character_motion::CharacterState::Charge
            )
        {
            return;
        }
        if let Some(target) = self.hovered_character {
            match self.world.interact_character(target, 0) {
                Ok(true) => {
                    self.drain();
                    return;
                }
                Ok(false) => {}
                Err(error) => {
                    self.fail_scripts(error);
                    return;
                }
            }
        }
        if let Some(target) = self.hovered_pickup {
            let result = if self.world.book_pickups.contains_key(&target) {
                self.pickup_book(target).map(|_| ())
            } else {
                self.pickup_item(target).map(|_| ())
            };
            if let Err(error) = result {
                self.events
                    .push(SessionEvent::PickupFailed(format!("{error:?}")));
            }
        }
        if let Some(id) = self.pad_interactable_target() {
            let result = self.runtime.execute_trigger(
                id,
                Some(self.avatar().handle),
                true,
                self.world.role,
                &mut self.world,
            );
            if let Err(error) = result {
                self.fail_scripts(format!("pad interact {id}: {error:?}"));
            }
            self.drain();
        }
    }

    /// Slot-aware controller Action dispatch; keeps the primary player's hover state intact.
    fn handle_pad_action_for(&mut self, slot: usize, pressed: bool, dialog: bool, limited: bool) {
        let Ok(avatar) = self.player_avatar(slot) else {
            return;
        };
        if !pressed
            || limited
            || dialog
            || self.world.flags.cutscene
            || avatar.dead()
            || avatar.polymorphed
            || matches!(
                avatar.motion.state,
                crate::character_motion::CharacterState::Cast
                    | crate::character_motion::CharacterState::Charge
            )
        {
            return;
        }
        let handle = avatar.handle;
        if let Some(target) = self.pad_character_target_for(slot) {
            match self.world.interact_character(target, slot as i32) {
                Ok(true) => {
                    self.drain();
                    return;
                }
                Err(e) => {
                    self.fail_scripts(e);
                    return;
                }
                _ => {}
            }
        }
        if let Some(target) = self.pad_pickup_target_for(slot) {
            let result = if self.world.book_pickups.contains_key(&target) {
                self.pickup_book_for_player(slot, target).map(|_| ())
            } else {
                self.pickup_item_for_player(slot, target).map(|_| ())
            };
            if let Err(e) = result {
                self.events
                    .push(SessionEvent::PickupFailed(format!("{e:?}")));
            }
        }
        if let Some(id) = self.pad_interactable_target_for(slot) {
            if let Err(e) = self.runtime.execute_trigger(
                id,
                Some(handle),
                true,
                self.world.role,
                &mut self.world,
            ) {
                self.fail_scripts(format!("pad interact {id}: {e:?}"));
            }
            self.drain();
        }
    }

    /// Game.SetCursor(active, cursor) for the local controller this frame.
    pub fn cursor(&self, shift: bool) -> (Cursor, bool) {
        if shift && !self.world.flags.cutscene {
            return (Cursor::Attack, true);
        }
        if let Some(handle) = self.hovered_character {
            return (
                Cursor::Talk,
                self.world
                    .talk_character_in_reach(self.body().position, self.avatar().faction)
                    == Some(handle),
            );
        }
        if let Some(handle) = self.hovered_pickup {
            return (Cursor::PickUp, self.pickup_in_range(handle));
        }
        if let Some(id) = self.hovered_interactable {
            use crate::scene_script::InteractType as T;
            let kind = self.world.interactable_defs.get(&id).map(|d| d.kind);
            let cursor = match kind {
                Some(T::PickUp) => Cursor::PickUp,
                Some(T::Examine) => Cursor::Examine,
                Some(T::Talk) => Cursor::Talk,
                _ => Cursor::Interact,
            };
            return (cursor, self.interactable_reach() == Some(id));
        }
        (Cursor::Default, false)
    }

    pub fn spawn_magick_book(
        &mut self,
        magick: u8,
        transform: [f32; 16],
    ) -> Result<u16, SessionError> {
        use crate::magick_books::{SpawnMagickAction, SpawnMagickHost};
        self.world
            .spawn_book(
                &SpawnMagickAction {
                    delay: 0.0,
                    area: 0,
                    magick,
                    immovable: true,
                    timeout: 0.0,
                    unique_id: 0,
                    on_pickup: 0,
                },
                transform,
            )
            .map_err(SessionError::World)
    }
    pub fn nearby_book(&self) -> Option<u16> {
        crate::pickups::find_pickup(
            self.world.entities.iter().filter_map(|e| {
                let book = self.world.book_pickups.get(&e.handle)?;
                Some(crate::pickups::PickupCandidate {
                    handle: e.handle,
                    position: e.position,
                    radius: e.radius,
                    pickable: e.added && !e.removed && !book.dead,
                    dead: e.dead,
                })
            }),
            self.body().position,
            self.body().direction(),
            self.body().radius,
            false,
            None,
        )
    }
    pub fn pickup_book(&mut self, handle: u16) -> Result<u8, SessionError> {
        self.pickup_book_for_player(0, handle)
    }
    fn pickup_book_for_player(&mut self, player: usize, handle: u16) -> Result<u8, SessionError> {
        self.player_avatar(player)?;
        let e = self
            .world
            .entities
            .iter()
            .find(|e| e.handle == handle && e.added && !e.removed && !e.dead)
            .ok_or_else(|| SessionError::World("book entity is unavailable".into()))?;
        let book = self
            .world
            .book_pickups
            .get(&handle)
            .ok_or_else(|| SessionError::World("not a magick book".into()))?;
        if crate::pickups::find_pickup(
            [crate::pickups::PickupCandidate {
                handle,
                position: e.position,
                radius: e.radius,
                pickable: !book.dead,
                dead: e.dead,
            }],
            self.player_body(player)?.position,
            self.player_body(player)?.direction(),
            self.player_body(player)?.radius,
            false,
            Some(handle),
        )
        .is_none()
        {
            return Err(SessionError::World("book is out of pickup range".into()));
        }
        let magick = book.magick;
        let on_pickup = book.on_pickup;
        // BookOfMagick.Unlock: Versus unlocks for the picker's team (or the
        // picker alone); otherwise every player's bit is set.
        if self.world.versus().is_some() {
            let team = self.world.player_teams[player]
                & (crate::ruleset::TEAM_RED | crate::ruleset::TEAM_BLUE);
            let team = if team & crate::ruleset::TEAM_RED != 0 {
                crate::ruleset::TEAM_RED
            } else {
                team
            };
            let n = self.world.players.len().max(player + 1);
            self.unlocked_magicks.reserve_slots(n);
            for (slot, bits) in self.unlocked_magicks.iter_mut().enumerate() {
                let same_team = team != 0 && self.world.player_teams[slot] & team == team;
                if slot == player || same_team {
                    *bits |= 1u64 << magick;
                }
            }
        } else {
            let n = self.world.players.len();
            self.unlocked_magicks.reserve_slots(n);
            for bits in &mut self.unlocked_magicks {
                *bits |= 1u64 << magick;
            }
        }
        self.world.book_pickups.get_mut(&handle).unwrap().kill();
        if let Some(physics) = &mut self.pickup_physics {
            physics.remove(handle);
        }
        self.world
            .entities
            .iter_mut()
            .find(|e| e.handle == handle)
            .unwrap()
            .dead = true;
        self.update_magick_books(0.0);
        if on_pickup != 0 {
            self.runtime
                .execute_trigger(
                    on_pickup,
                    Some(self.player_avatar(player)?.handle),
                    false,
                    self.world.role,
                    &mut self.world,
                )
                .map_err(|e| SessionError::Script(format!("{e:?}")))?;
        }
        self.world.players[player]
            .as_mut()
            .ok_or_else(|| SessionError::World("pickup player unavailable after trigger".into()))?
            .motion
            .attack(149, false);
        let name = magicka_sim::spells::MagickType::try_from(magick)
            .ok()
            .and_then(crate::tome_magick::name_key);
        self.notify_obtained(player, "#magick_pick_up", name.as_deref())?;
        Ok(magick)
    }
    /// Avatar.InternalPickUp's Player.ObtainedTextBox: the pickup string
    /// shown over the picker for 2 s (`hud::obtained_notice`). Without a
    /// dialog text host (no strings) nothing is shown.
    fn notify_obtained(
        &mut self,
        player: usize,
        template: &str,
        name_key: Option<&str>,
    ) -> Result<(), SessionError> {
        let owner = self
            .world
            .entities
            .iter()
            .find(|e| matches!(e.kind, crate::scene_world::EntityKind::Avatar { slot } if slot == player))
            .map_or(0, |e| e.unique_id);
        let Some(text) = self.world.dialog_text.as_mut() else {
            return Ok(());
        };
        let template = text
            .strings
            .get_string(custom_hash(template))
            .unwrap_or_default();
        let name = name_key.map(|key| {
            text.strings
                .get_string(custom_hash(key))
                .unwrap_or_default()
        });
        let message = crate::hud::pickup_text(&template, name.as_deref());
        crate::hud::obtained_notice(text, owner, &message).map_err(SessionError::World)
    }
    /// Item.Update's despawn: the timing-out effect from 4 s left, then the
    /// disappear effect and Kill (host); attached items never despawn.
    fn update_item_despawn(&mut self, dt: f32) {
        let client = self.world.role == magicka_sim::damage_delivery::NetworkRole::Client;
        let mut killed = Vec::new();
        for (&handle, pickup) in &mut self.world.item_pickups {
            let Some(item) = pickup.item.as_ref() else {
                continue;
            };
            if item.attached {
                pickup.despawn = 0.0;
            }
            if pickup.despawn <= 0.0 || item.definition.model.is_none() {
                continue;
            }
            pickup.despawn -= dt;
            let position = [
                pickup.transform[12],
                pickup.transform[13],
                pickup.transform[14],
            ];
            let refs = self.item_despawn_effects.entry(handle).or_insert(None);
            let active = refs.as_mut().is_some_and(|r| {
                self.effects
                    .update_position_direction(r, position, [1.0, 0.0, 0.0])
            });
            if pickup.despawn <= 0.0 {
                if let Some(r) = refs.as_mut() {
                    self.effects.stop(r);
                }
                *refs = None;
                self.effects
                    .start_at(custom_hash("tome_disappear"), position, [1.0, 0.0, 0.0]);
                if !client {
                    killed.push(handle);
                }
            } else if !active && pickup.despawn <= 4.0 {
                *refs = self.effects.start_at(
                    custom_hash("tome_timing_out"),
                    position,
                    [1.0, 0.0, 0.0],
                );
            }
        }
        for handle in killed {
            if let Some(e) = self.world.entities.iter_mut().find(|e| e.handle == handle) {
                e.dead = true;
                e.removed = true;
            }
            if let Some(physics) = &mut self.pickup_physics {
                physics.remove(handle);
            }
        }
    }

    fn update_magick_books(&mut self, dt: f32) {
        let client = self.world.role == magicka_sim::damage_delivery::NetworkRole::Client;
        for (&handle, book) in &mut self.world.book_pickups {
            if !self
                .world
                .entities
                .iter()
                .any(|e| e.handle == handle && e.added && !e.removed && !e.dead)
            {
                book.kill();
            }
            let refs = self
                .book_effects
                .entry(handle)
                .or_insert((book.generation, None, None));
            if refs.0 != book.generation {
                for r in [&mut refs.1, &mut refs.2].into_iter().flatten() {
                    self.effects.stop(r);
                }
                *refs = (book.generation, None, None);
            }
            if !book.dead && refs.1.is_none() {
                refs.1 = self
                    .effects
                    .start(custom_hash("bookofmagick_glimmer"), book.transform);
            }
            let position = [book.transform[12], book.transform[13], book.transform[14]];
            let timeout_active = refs.2.as_mut().is_some_and(|r| {
                self.effects
                    .update_position_direction(r, position, [1.0, 0.0, 0.0])
            });
            let active = self
                .pickup_physics
                .as_ref()
                .and_then(|p| p.state(handle))
                .is_some_and(|s| !book.immovable && !s.sleeping);
            let event = book.update(dt, active, client, timeout_active);
            if event.start_timeout {
                refs.2 = self.effects.start_at(
                    custom_hash("tome_timing_out"),
                    position,
                    [1.0, 0.0, 0.0],
                );
            }
            if event.disappear {
                self.effects
                    .start_at(custom_hash("tome_disappear"), position, [1.0, 0.0, 0.0]);
            }
            if book.dead {
                if let Some(physics) = &mut self.pickup_physics {
                    physics.remove(handle);
                }
                for r in [&mut refs.1, &mut refs.2].into_iter().flatten() {
                    self.effects.stop(r);
                }
                refs.1 = None;
                refs.2 = None;
                if let Some(e) = self.world.entities.iter_mut().find(|e| e.handle == handle) {
                    e.dead = true;
                }
            } else if let Some(r) = &mut refs.1 {
                self.effects.update_orientation(r, book.transform);
            }
        }
    }
}
