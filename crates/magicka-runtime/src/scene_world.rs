//! Headless scene world: binds the recovered script hosts to concrete state
//! (level locators/areas, entity registry, effects, ambient sounds, scene flags,
//! animated collision) so retail scene scripts can run frame by frame.
//!
//! This is an integration harness, not gameplay parity. Approximations are
//! explicit: a seeded generator replaces System.Random, presence is recomputed
//! by geometric scans instead of physics callbacks, there are no players, AI,
//! physics, dialog or rendering, and unrecovered behavior returns errors.
use crate::{
    ai_event::{AiEvent, AiEventLevel},
    animation_action::AnimationAction,
    audio_action::{AmbientBackend, AmbientSounds, AudioActionHost, LocatedSound},
    avatar_move::{AvatarMoveHost, AvatarMoveLevel},
    effect_action::{EffectActionHost, EffectBackend, SceneEffects},
    entity_command::{AreaCharacter, EntityCommandHost, EntityFacts},
    give_order::{GiveOrderHost, OrderCandidate, OrderCommand},
    live_collision::LiveCollisionScene,
    presence::{PresenceAreas, PresenceCharacter, PresenceEntity, PresenceFacts},
    scene_command::{CameraInterpolation, SceneCommandHost},
    scene_loading::custom_hash,
    script_runtime::{ScriptRuntime, ScriptRuntimeError, ScriptWorld, ScriptWorldFacts},
    spawn_action::{NpcInstance, SpawnConfig, SpawnHost, SpawnLevel, SpawnMessage},
    spawn_phys::{SpawnPhysHost, SpawnPhysLevel},
    toggle_action::ToggleHost,
    world_condition::{BossFacts, CharacterHealthFacts, ConditionPlayer, WorldConditionFacts},
};
use crate::{
    avatar::{AvatarFrame, PlayerAvatar},
    camera::{CameraBox, MagickCamera},
};
use magicka_content::{level::LevelModel, skinned::pose_matrix};
use magicka_sim::{character_physics::PhysicsWorld, navmesh::NavMesh};
use magicka_sim::{damage_delivery::NetworkRole, spells::Spell, spray_pulse::Segment};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub mod assatur_live;
pub mod boss_death_live;
pub mod boss_live;
mod boss_spells;
pub mod campaign_live;
pub mod clone_live;
pub mod country_defense_live;
pub mod cross_live;
pub mod cthulhu_live;
pub mod cult_live;
#[cfg(test)]
mod damage_boost_tests;
pub mod death_live;
pub mod dispenser_live;
pub mod earthquake_live;
pub mod elemental_live;
pub mod fafnir_live;
pub mod fairy_live;
pub mod generic_aura_live;
pub mod grimnir2_live;
pub mod grimnir_live;
pub mod grip_live;
mod gun_live;
mod jormungandr_live;
pub mod luggage_live;
pub mod machine_live;
mod npc_abilities;
pub mod npc_events;
mod npc_spells;
mod otherworldly_live;
pub mod parker_live;
pub mod phoenix_live;
pub mod polymorph_live;
pub mod portal_live;
pub mod possession;
#[cfg(test)]
mod projectile_contacts;
mod ruleset_live;
pub mod saved_entities;
pub mod script_ops;
#[cfg(test)]
mod size_tests;
pub mod slime_live;
mod special_live;
pub mod staff_effects;
mod staff_specials;
mod statistics_live;
mod survival_live;
pub mod tornado_live;
mod versus_live;
pub mod vlad_zap_live;
pub mod vortex_live;
#[cfg(test)]
mod warlord_contacts;
pub mod wave_live;

/// NullifyArea: `if (IsLevitating) StopLevitate()` (gravity back on).
fn stop_levitate(
    c: &mut crate::character::RuntimeCharacter,
    body: &mut magicka_sim::character_physics::CharacterBody,
) {
    if c.levitation.active(c.combat.dead) {
        c.levitation.stop();
        body.apply_gravity = true;
        body.owner.levitating = false;
    }
}

/// Nullify's per-character part (statuses, self shield, invisibility,
/// self-cast auras). Returns NullifyArea's `flag`; status stops alone do not
/// set it.
fn nullify_character(c: &mut crate::character::RuntimeCharacter, statuses: u16) -> bool {
    let mut events = Vec::new();
    c.combat.statuses.stop_effects(statuses, &mut events);
    c.pending_status_audio.extend(events);
    let mut flag = false;
    if c.entanglement.entangled() {
        flag = true;
        c.entanglement.release(&mut c.entangle_events);
    }
    let shield = &mut c.combat.health.shield;
    if shield.active() {
        shield.remove(c.definition.mass, false, &mut c.shield_events);
        flag = true;
    }
    if c.fear.is_feared(c.combat.dead) {
        c.fear.remove();
        flag = true;
    }
    if c.combat.health.invisibility_timer > 0.0 {
        c.set_invisible(0.0);
        flag = true;
    }
    // Nullify alone skips ClearAura while the native bubble flag is set.
    if !c.bubble {
        for aura in &mut c.nonvisual_auras {
            aura.ttl = 0.;
        }
        for aura in c.auras.iter_mut().filter(|a| a.self_casted) {
            aura.ttl = 0.0;
        }
    }
    flag
}

/// Teleport.Execute: distance along the caster's facing.
const TELEPORT_DISTANCE: f32 = 10.0;
/// Thunderbolt.RANGE, sDamage and the bolt's height and lifetime.
const THUNDERBOLT_RANGE: f32 = 16.0;
const THUNDERBOLT_DAMAGE: f32 = 5000.0;
const THUNDERBOLT_HEIGHT: f32 = 40.0;
const THUNDERBOLT_BOLT_TTL: f32 = 1.0;
/// Spell.LIGHTNINGCOLOR.
const LIGHTNING_COLOR: [f32; 3] = [0.75, 0.5, 1.0];

/// `SceneWorld::facing_target`: the caster's position and the chosen
/// (handle, position), if any.
type FacingTarget = ([f32; 3], Option<(u16, [f32; 3])>);

/// Charm.RADIUS and CHARM_TIME.
const CHARM_RADIUS: f32 = 14.0;
pub const CHARM_TIME: f32 = 15.0;

pub type WorldError = String;

/// Template facts the world needs from CharacterTemplate content.
#[derive(Clone, Debug, PartialEq)]
pub struct NpcTemplate {
    /// Character.Type: custom hash of the template's lowercased ID.
    pub type_id: i32,
    pub faction: i32,
    pub max_hit_points: f32,
    pub radius: f32,
    pub length: f32,
    pub undying: bool,
}

/// Content lookups the world performs lazily.
pub trait SceneContent {
    fn elemental_egg_geometry(&mut self) -> Option<crate::elemental_egg_actor::EggGeometry> {
        None
    }
    /// Dedicated animated entities retain their resolved model and skeleton.
    fn skinned_model(&mut self, _path: &str) -> Option<magicka_content::skinned::Resolved> {
        None
    }
    fn model_bounds(&mut self, _path: &str) -> Option<crate::pickups::PickupBounds> {
        None
    }
    fn projectile_item(&mut self, _name: &str) -> Option<crate::npc_combat::RangedItem> {
        None
    }
    fn player_ranged_item(&mut self, _name: &str) -> Option<Arc<crate::npc_combat::RangedItem>> {
        None
    }
    fn item(&mut self, _name: &str) -> Option<magicka_content::item::Item> {
        None
    }

    /// Data/Characters/<name>; `None` when the file is missing.
    fn character(&mut self, name: &str) -> Option<NpcTemplate>;
    /// Data/PhysicsEntities/<name> MaxHitpoints; `None` when missing.
    fn physics_max_hit_points(&mut self, name: &str) -> Option<i32>;
    fn physics_template(
        &mut self,
        _name: &str,
    ) -> Option<magicka_content::physics::PhysicsEntityTemplate> {
        None
    }
    /// EffectManager keys (lowercased Content/Effects stems).
    fn effect_exists(&mut self, effect: i32) -> bool;
    /// AudioManager SoundList keys.
    fn cue_exists(&mut self, cue: i32) -> bool;
    /// Full template (definition, animations, AI weights) for live NPCs;
    /// `None` keeps the NPC a static record (headless harnesses).
    fn actor(&mut self, _name: &str) -> Option<crate::npc::NpcActorTemplate> {
        None
    }
    /// Data/PhysicsEntities/<name> gibs with loadable models.
    fn physics_gibs(&mut self, _name: &str) -> Vec<crate::gibs::GibDefinition> {
        Vec::new()
    }
    /// The first mesh's bounding-sphere radius of `Content/<path>.xnb`
    /// (a Model); `None` when missing (headless harnesses).
    fn model_radius(&mut self, _path: &str) -> Option<f32> {
        None
    }
    /// Duration of `clip` in the SkinnedModel `Content/<path>.xnb`; `None`
    /// when missing (headless harnesses).
    fn clip_duration(&mut self, _path: &str, _clip: &str) -> Option<f32> {
        None
    }
}

impl<T: SceneContent + ?Sized> SceneContent for &mut T {
    fn elemental_egg_geometry(&mut self) -> Option<crate::elemental_egg_actor::EggGeometry> {
        (**self).elemental_egg_geometry()
    }
    fn skinned_model(&mut self, path: &str) -> Option<magicka_content::skinned::Resolved> {
        (**self).skinned_model(path)
    }
    fn model_bounds(&mut self, path: &str) -> Option<crate::pickups::PickupBounds> {
        (**self).model_bounds(path)
    }
    fn player_ranged_item(&mut self, name: &str) -> Option<Arc<crate::npc_combat::RangedItem>> {
        (**self).player_ranged_item(name)
    }
    fn item(&mut self, name: &str) -> Option<magicka_content::item::Item> {
        (**self).item(name)
    }

    fn character(&mut self, name: &str) -> Option<NpcTemplate> {
        (**self).character(name)
    }
    fn physics_max_hit_points(&mut self, name: &str) -> Option<i32> {
        (**self).physics_max_hit_points(name)
    }
    fn physics_template(
        &mut self,
        name: &str,
    ) -> Option<magicka_content::physics::PhysicsEntityTemplate> {
        (**self).physics_template(name)
    }
    fn effect_exists(&mut self, effect: i32) -> bool {
        (**self).effect_exists(effect)
    }
    fn cue_exists(&mut self, cue: i32) -> bool {
        (**self).cue_exists(cue)
    }
    fn actor(&mut self, name: &str) -> Option<crate::npc::NpcActorTemplate> {
        (**self).actor(name)
    }
    fn model_radius(&mut self, path: &str) -> Option<f32> {
        (**self).model_radius(path)
    }
    fn clip_duration(&mut self, path: &str, clip: &str) -> Option<f32> {
        (**self).clip_duration(path, clip)
    }
}

/// Neighbour handle of the non-entity danger sources (missiles, mines,
/// barriers, tornadoes, vortices, grease): never an entity handle.
pub const DANGER_HANDLE: u16 = u16::MAX;

#[derive(Clone, Debug, PartialEq)]
pub enum EntityKind {
    Dispenser,
    ElementalEgg,
    Item,
    Npc {
        type_id: i32,
        template: i32,
    },
    Physics {
        damageable: bool,
    },
    /// A player's Avatar (slot index).
    Avatar {
        slot: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneEntity {
    pub handle: u16,
    pub unique_id: i32,
    pub kind: EntityKind,
    pub position: [f32; 3],
    pub orientation: [f32; 16],
    pub hit_points: f32,
    pub max_hit_points: f32,
    pub original_faction: i32,
    pub faction: i32,
    pub radius: f32,
    /// Capsule length for characters (dialog placement).
    pub length: f32,
    /// Character template file name (Data/Characters/<name>), for NPCs.
    pub template_name: Option<String>,
    /// Last animation requested by scripts (Animations value; 1 = idle).
    pub animation: u16,
    pub special_idle: u16,
    pub undying: bool,
    pub cannot_die_without_explicit_kill: bool,
    pub dead: bool,
    /// Added to the EntityManager (Spawn/SpawnPhys AddEntity).
    pub added: bool,
    pub removed: bool,
    pub order: Option<u8>,
    pub event_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectState {
    pub instance: u64,
    pub effect: i32,
    pub active: bool,
    pub transform: [f32; 16],
}

/// Ordered VisualEffect lifecycle calls for the presentation host. Instance
/// identity survives stop/start and distinguishes replacements at one scene ID.
#[derive(Clone, Debug)]
pub enum SceneEffectCommand {
    Start {
        instance: u64,
        effect: i32,
        transform: [f32; 16],
    },
    Stop {
        instance: u64,
    },
    Move {
        instance: u64,
        position: [f32; 3],
        direction: [f32; 3],
    },
}

/// Logged world state not simulated here (camera, music, input, tutorial…).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneFlags {
    pub cutscene: bool,
    /// PlayState.StaffLight (WizardControl); nothing reads it natively.
    pub staff_light: bool,
    pub input_limited: bool,
    pub music: Option<(u16, i32)>,
    /// PlayState.mMusicFocusValue of the last PlayMusic (checkpoint state).
    pub music_focus_value: Option<f32>,
    pub achievements: BTreeSet<String>,
    pub lights_disabled: BTreeSet<i32>,
    pub interactables_disabled: BTreeSet<i32>,
    pub tutorial_elements_disabled: i32,
    pub tutorial_cast_types_disabled: BTreeSet<i32>,
    pub tutorial_push_disabled: bool,
}

/// TriggerCheckpoint.Execute's "#add_checkpoint" TextBox request: the
/// localized text, Maiandra18, screen position (0, 0), anchor (0, 1),
/// forced on screen, 2 s; added to the DialogManager for the presentation
/// to draw (research/checkpoints.md, 2026-10-30).
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointNotice {
    pub text: String,
    pub font: &'static str,
    pub position: [f32; 2],
    pub anchor: [f32; 2],
    pub force_on_screen: bool,
    pub ttl: f32,
}

/// Trigger-area type of an NPC. Rewrite design decision (not a parity
/// claim): `type="wizard"` presence checks are player-only, so an NPC built
/// from the Wizard template gets its own type and never fires them. Player
/// avatars always count as "wizard" (`PlayerAvatar::trigger_type`).
pub fn npc_trigger_type(type_id: i32) -> i32 {
    if type_id == custom_hash("wizard") {
        custom_hash("wizard (npc)")
    } else {
        type_id
    }
}

struct ConflagrationWorld<'a, C: SceneContent> {
    world: &'a mut SceneWorld<C>,
    instance: u64,
}
impl<C: SceneContent> magicka_sim::conflagration::World for ConflagrationWorld<'_, C> {
    fn targets(
        &mut self,
        center: [f32; 3],
        radius: f32,
    ) -> Vec<magicka_sim::conflagration::Target> {
        use magicka_sim::entity_query::{EntityQueryGrid, QueryEntity};
        let (targets, _) = self.world.spray_entities();
        let query: Vec<_> = targets
            .iter()
            .map(|t| QueryEntity {
                position: t.position,
                radius: t.radius,
                dead: t.dead,
                shield: matches!(t.geometry, crate::spray_world::ArcGeometry::Shield(_)),
                damageable: t.damageable,
            })
            .collect();
        let found = EntityQueryGrid::build(&query).and_then(|g| {
            g.query(&query, center, radius, false, false, |_, _| {
                (false, [0.0; 3])
            })
        });
        match found {
            Ok(indices) => indices
                .into_iter()
                .map(|i| {
                    let t = targets[i];
                    magicka_sim::conflagration::Target {
                        handle: t.handle,
                        position: t.position,
                        radius: t.radius,
                        ethereal_character: self.world.combatant_ethereal(t.handle),
                    }
                })
                .collect(),
            Err(error) => {
                self.world
                    .log
                    .push(format!("conflagration: entity query {error:?}"));
                Vec::new()
            }
        }
    }
    fn shielded(&mut self, segment: Segment) -> bool {
        self.world
            .shields
            .iter()
            .any(|s| !s.shield.dead && s.shield.geometry().segment_fraction(segment).is_some())
    }
    fn damage(
        &mut self,
        target: u16,
        damage: magicka_sim::damage::Damage,
        owner: Option<u16>,
        timestamp: f64,
        source: [f32; 3],
    ) {
        self.world
            .damage_any_at(target, damage, owner, source, timestamp);
    }
    fn move_effect(&mut self, position: [f32; 3], direction: [f32; 3]) {
        self.world.effect_commands.push(SceneEffectCommand::Move {
            instance: self.instance,
            position,
            direction,
        });
    }
}

struct Backend<'a> {
    effect_commands: &'a mut Vec<SceneEffectCommand>,
    effect_serial: &'a mut u64,
    content: &'a mut dyn SceneContent,
    log: &'a mut Vec<String>,
    audio: &'a mut Vec<crate::audio::AudioRequest>,
    audio_key: &'a mut u32,
}
impl Backend<'_> {
    fn next_key(&mut self) -> u32 {
        *self.audio_key = self.audio_key.wrapping_add(1);
        *self.audio_key
    }
}
impl EffectBackend for Backend<'_> {
    type Error = WorldError;
    type Effect = EffectState;
    fn get_effect(&mut self, effect: i32) -> Result<EffectState, WorldError> {
        if !self.content.effect_exists(effect) {
            return Err(format!("EffectManager.GetEffect: missing {effect}"));
        }
        *self.effect_serial = self.effect_serial.wrapping_add(1);
        Ok(EffectState {
            instance: *self.effect_serial,
            effect,
            ..Default::default()
        })
    }
    fn is_active(&self, e: &EffectState) -> bool {
        e.active
    }
    fn start(&mut self, e: &mut EffectState, t: [f32; 16]) -> Result<(), WorldError> {
        e.active = true;
        e.transform = t;
        self.effect_commands.push(SceneEffectCommand::Start {
            instance: e.instance,
            effect: e.effect,
            transform: t,
        });
        self.log.push(format!("effect start {}", e.effect));
        Ok(())
    }
    fn stop(&mut self, e: &mut EffectState) -> Result<(), WorldError> {
        e.active = false;
        self.effect_commands.push(SceneEffectCommand::Stop {
            instance: e.instance,
        });
        self.log.push(format!("effect stop {}", e.effect));
        Ok(())
    }
    fn transform(&self, e: &EffectState) -> [f32; 16] {
        e.transform
    }
    fn set_transform(&mut self, e: &mut EffectState, t: [f32; 16]) {
        e.transform = t;
    }
    fn update(&mut self, _: &mut EffectState, _: f32) -> Result<(), WorldError> {
        Ok(())
    }
}
impl AmbientBackend for Backend<'_> {
    type Error = WorldError;
    type Located = (u32, LocatedSound);
    type Global = (u32, u16, i32, f32);
    fn start_located(
        &mut self,
        id: i32,
        s: LocatedSound,
    ) -> Result<(u32, LocatedSound), WorldError> {
        self.log.push(format!("sound located {id} cue {}", s.cue));
        let key = self.next_key();
        self.audio.push(crate::audio::AudioRequest::Start {
            key,
            bank: s.bank,
            cue: s.cue,
            volume_db: s.volume,
            source: crate::audio::SoundSource::Locator {
                area: s.area,
                radius: s.radius,
                apply_3d: s.apply_3d,
            },
            magnitude: None,
        });
        Ok((key, s))
    }
    fn start_global(
        &mut self,
        bank: u16,
        cue: i32,
        volume: f32,
    ) -> Result<(u32, u16, i32, f32), WorldError> {
        self.log.push(format!("sound global cue {cue}"));
        let key = self.next_key();
        self.audio.push(crate::audio::AudioRequest::Start {
            key,
            bank,
            cue,
            volume_db: volume,
            source: crate::audio::SoundSource::Global,
            magnitude: None,
        });
        Ok((key, bank, cue, volume))
    }
    fn replay_global(
        &mut self,
        s: &mut (u32, u16, i32, f32),
        volume: f32,
    ) -> Result<(), WorldError> {
        s.3 = volume;
        self.audio.push(crate::audio::AudioRequest::Replay {
            key: s.0,
            volume_db: volume,
        });
        Ok(())
    }
    fn stop_located(
        &mut self,
        s: &mut (u32, LocatedSound),
        instant: bool,
    ) -> Result<(), WorldError> {
        self.audio
            .push(crate::audio::AudioRequest::Stop { key: s.0, instant });
        Ok(())
    }
    fn stop_global(
        &mut self,
        s: &mut (u32, u16, i32, f32),
        instant: bool,
    ) -> Result<(), WorldError> {
        self.audio
            .push(crate::audio::AudioRequest::Stop { key: s.0, instant });
        Ok(())
    }
}

type SceneAreaTransform = ([f32; 3], [f32; 4], [f32; 3]);

/// A live spell Shield entity (EntityManager.Shields, in creation order).
#[derive(Clone, Debug)]
pub struct LiveShield {
    pub handle: u16,
    pub shield: magicka_sim::shield_entity::ShieldEntity,
}

/// Character.mBloating state set by BloatKill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bloat {
    pub element: u32,
    pub killer: u16,
    /// mBloatTimer; BloatBlast at 0.333 s.
    pub timer: f32,
}

pub struct SceneWorld<C: SceneContent> {
    pub level: Arc<LevelModel>,
    pub collision: LiveCollisionScene,
    pub presence: PresenceAreas,
    pub presence_entities: Vec<PresenceEntity>,
    pub entities: Vec<SceneEntity>,
    pub effects: SceneEffects<EffectState>,
    pub effect_commands: Vec<SceneEffectCommand>,
    pub effect_types: BTreeMap<i32, crate::particles::VisualEffectType>,
    effect_serial: u64,
    pub sounds: AmbientSounds<(u32, LocatedSound), (u32, u16, i32, f32)>,
    /// Audio requests this step (research/audio-xact.md) and the key counter.
    pub audio_requests: Vec<crate::audio::AudioRequest>,
    audio_key: u32,
    /// SpraySpell.mSpellCues for Area casts, retained until effect cleanup.
    spray_area_cues: BTreeMap<u16, Vec<u32>>,
    pub flags: SceneFlags,
    /// GameScene.RuleSet (research/rulesets.md).
    pub ruleset: Option<crate::ruleset::SceneRuleset>,
    /// SurvivalRuleset.mHealthBar: the current wave's remaining health.
    pub survival_health_bar: Option<boss_live::BossHealthBar>,
    /// StatisticsManager (scores, Survival totals).
    pub statistics: crate::statistics::Statistics,
    /// The lobby's Versus mode and settings (GameScene construction input).
    pub versus_settings: crate::ruleset::VersusSettings,
    /// Player.Team per slot (TEAM_RED / TEAM_BLUE, 0 without teams).
    pub player_teams: crate::slots::PerSlot<i32>,
    /// Avatar deaths already reported (statistics and Versus).
    avatar_dead_seen: crate::slots::PerSlot<bool>,
    /// Versus commands PlaySession applies (setup, revive, unlocks).
    pub pending_versus: Vec<crate::ruleset::VersusCommand>,
    /// Avatar.DropMagicks requests (slots) for PlaySession.
    pub versus_drop_magicks: Vec<usize>,
    versus_clock_cue: Option<u32>,
    /// The Versus ruleset waits for a second player before Initialize.
    pub versus_waiting: bool,
    /// TimedObjectiveRuleset.Update queued its HUD box this frame.
    pub timed_objective_hud: bool,
    /// Reward spawning is tracked explicitly until the native loot host is connected.
    pub pending_survival_luggage: Vec<bool>,
    /// The enabled PackMan reward pools (host supplied, indoor-filtered).
    pub survival_packs: Option<crate::ruleset::SurvivalPacks>,
    /// Playing players' UnlockedMagicks (luggage avoids known magicks).
    pub unlocked_magick_mask: u64,
    /// LobbySettings.boss_scale (`boss_scale.rs`). Off keeps native boss health.
    pub boss_scale: bool,
    /// This step's divisor for damage reaching a boss.
    pub boss_damage_divisor: f32,
    area_aliases: BTreeMap<i32, i32>,
    /// PlayState cutscene bars/skip state (`flags.cutscene` mirrors `active`).
    pub cutscene: crate::cutscene::CutsceneState,
    /// RenderManager.mCurrentTransition (script fades and scene changes).
    pub render_transition: Option<crate::render_transition::RenderTransition>,
    /// Level.ClearTransition since the last GoToScene (no fade-in after it).
    pub transition_cleared: bool,
    /// Host operations in execution order, for evidence and diffing.
    pub log: Vec<String>,
    pub unsupported_specials: BTreeSet<String>,
    /// Staff abilities alive in the SpellManager (research/staff-items.md).
    pub staff_effects: Vec<staff_effects::StaffEffect>,
    pub role: NetworkRole,
    /// Level DialogCollection (level file then Dialog_Fairy). Only its facts are
    /// used here; StartDialog/KillDialogs need UI hosts that are not provided.
    pub dialogs: Option<crate::dialog_manager::DialogManager>,
    /// Level.GoToScene request; the Level host performs the switch.
    pub pending_scene: Option<crate::scene_flow::PendingScene>,
    pub checkpoints: Vec<[[f32; 16]; 4]>,
    /// Character bodies (avatars) against the live collision.
    pub physics: PhysicsWorld,
    /// Native MissileEntity.Update uses -50 m. External hosts can override
    /// this independently of native campaign scene/character kill planes.
    pub missile_kill_plane: f32,
    /// External levels without a Magicka navmesh can opt into collision-based
    /// portal floor placement, using their native walkable normal threshold.
    pub host_portal_walkable_normal: Option<f32>,
    pub camera: MagickCamera,
    pub nav: Option<NavMesh>,
    nav_part_transforms: Vec<[f32; 16]>,
    pub players: Vec<Option<PlayerAvatar>>,
    /// Live NPC characters by entity handle (bodies in `physics`).
    pub npcs: BTreeMap<u16, crate::npc::NpcCharacter>,
    /// AIManager.Agents in registration order, and its round-robin index.
    pub agents: Vec<u16>,
    ai_update_index: usize,
    /// GameScene.ExecuteTrigger requests from avatar events (trigger, sender),
    /// executed right after the avatar update.
    pub pending_triggers: Vec<(i32, Option<u16>)>,
    /// Rewrite extension: registered (mod) Magicks cast this step, as
    /// (caster handle, registry index), for the mod runtime to execute.
    pub registered_casts: Vec<(u16, u16)>,
    /// Rewrite extension: built-in Magicks (bit = legacy value) a mod
    /// replaced with a Lua handler. Casting one runs no native effect; the
    /// host queues it in `scripted_magick_casts` for the mod runtime.
    pub scripted_magicks: u64,
    pub scripted_magick_casts: Vec<(u16, u8)>,
    /// Script UnlockMagick grants, consumed by the session with player profiles.
    pub pending_magick_unlocks: Vec<u8>,
    /// Character.Dialog, overwritten on every cached NPC initialization.
    pub character_dialogs: BTreeMap<u16, i32>,
    /// Direct entity references in dialogs, including unnamed boss characters.
    dialog_handle_owners: BTreeMap<i32, u16>,
    pub dispensers: BTreeMap<u16, crate::dispenser::Dispenser>,
    /// SprayEntity web strands (Parker).
    pub web_strands: crate::web_strand::WebPool,
    /// OtherworldlyDischarge's delayed Cultist spawns.
    pub discharge_spawns: Vec<otherworldly_live::DischargeSpawn>,
    /// Characters showing the hypnotize effect.
    hypnotize_shown: BTreeSet<u16>,
    /// The template the next OtherworldlyDischarge raises (MutateBeastman
    /// reuses it); `None` is the Cultist.
    discharge_template: Option<&'static str>,
    /// SpawnAnimProp's AnimatedPhysicsEntity state by physics handle.
    pub anim_props: BTreeMap<u16, crate::anim_prop::AnimProp>,
    pub physics_damage: BTreeMap<u16, crate::physics_damage::PhysicsDamage>,
    /// IsCampaignLooped: the save slot's Looped flag in GameType.Campaign,
    /// `None` in every other game type (the condition then throws).
    pub campaign_looped: Option<bool>,
    /// First template box of solid, static (non-movable) physics entities,
    /// which characters meet as immovable obstacles.
    pub static_prop_boxes: BTreeMap<u16, magicka_content::physics::CollisionBox>,
    /// Damageable props whose SpawnGibs ran, by (handle, physics generation);
    /// SpawnGibs clears mGibs, so a prop gibs once per initialization.
    prop_gibbed: BTreeSet<(u16, u64)>,
    /// Presentation-free dialog text boxes driven with `dialogs`.
    pub dialog_text:
        Option<crate::dialog_text_host::DialogTextHost<crate::dialog_world::SceneDialogWorld>>,
    /// Camera view-projection (column-major rows) for dialog projection.
    pub tutorial: crate::tutorial::TutorialHints,
    pub dialog_matrix: [[f32; 4]; 4],
    pub dialog_failure: Option<String>,
    /// Live Force projectiles and their presentation requests.
    pub missiles: Vec<crate::missiles::LiveMissile>,
    pub missile_visuals: Vec<crate::missiles::MissileVisual>,
    next_missile: u32,
    /// Railgun beams (SpellManager effects) and their presentation events.
    pub rails: magicka_sim::railgun::RailSystem,
    /// Beam blasts this frame (ArcaneBlast visuals): (position, radius, elements).
    pub ground_blasts: Vec<([f32; 3], f32, [magicka_sim::damage::Damage; 5])>,
    pub beam_blasts: Vec<([f32; 3], f32, u32)>,
    /// Spell shields (Shield Force discs, Area spheres).
    pub shields: Vec<LiveShield>,
    pub shield_nullifications: Vec<crate::character_shield::ShieldNullification>,
    /// SpellMines (Shield + Arcane/Life Force/Area casts) in their 64-slot
    /// cache, with the mine clip duration from `Models/Effects/mine`.
    pub mines: magicka_sim::spell_mine::MinePool,
    /// Grease.GreaseField's 72-field cache (research/grease.md), the shared
    /// static burn hit list and each slot's grease decal.
    pub grease: magicka_sim::grease::GreasePool,
    grease_hits: magicka_sim::spray_pulse::HitList,
    /// Character.mCurrentSummons per master handle.
    pub summons: BTreeMap<u16, magicka_sim::summons::SummonList>,
    /// A MeteorShower dimmed the light (OnRemove restores it).
    meteor_light: bool,
    grease_decals: Vec<Option<crate::decals::Reference>>,
    pub mine_clip_duration: Option<f32>,
    /// Mine effects this frame: (effect name, position, direction).
    pub mine_effects: Vec<(&'static str, [f32; 3], [f32; 3])>,
    /// Barriers (Shield + element walls) in the 128-instance cache, the
    /// barrier clip durations, and this frame's presentation requests.
    pub barriers: magicka_sim::barrier::BarrierPool,
    pub barrier_clips: Option<magicka_sim::barrier::BarrierClips>,
    pub barrier_visuals: Vec<BarrierVisual>,
    /// Barrier mSoundCue loops by slot (stopped by Deinitialize).
    barrier_cues: BTreeMap<usize, u32>,
    /// PlayState.Endgame requests from scripts: (condition, freeze, phony).
    pub pending_end_game: Option<(u8, bool, bool)>,
    /// PlayState.IsGameEnded, supplied before scene updates.
    pub game_ended: bool,
    /// PlayState.UpdateCheckPoint this step: (spawn matrices, ignored triggers).
    pub pending_checkpoint: Option<([[f32; 16]; 4], Vec<i32>)>,
    /// TriggerCheckpoint text boxes requested this step, in order.
    pub checkpoint_notices: Vec<CheckpointNotice>,
    /// Collection FullBlasts awaiting application to mines and barriers.
    entity_blasts: Vec<EntityBlast>,
    /// Bloating characters (Railgun BloatKill) by handle.
    pub bloats: BTreeMap<u16, Bloat>,
    /// Overkilled characters (gibbed; their bodies are no longer drawn).
    pub overkilled: BTreeSet<u16>,
    /// Live Gib entities and the gib cache (research/gibs.md).
    pub gibs: crate::gibs::Gibs,
    pub decals: crate::decals::Decals,
    /// SpellManager.mEffects: active magick ability effects.
    pub ability_effects: magicka_sim::magicks::AbilityEffects,
    pub size_effects: crate::size_magicks::SizeEffects,
    /// Polymorph effects (SpellManager) and each one's NPCPolymorphData,
    /// keyed by the current (replacement) target handle.
    pub polymorphs: magicka_sim::polymorph::PolymorphEffects,
    polymorph_storage: BTreeMap<u16, polymorph_live::PolymorphStorage>,
    /// PolymorphAvatar requests for PlaySession, in execution order.
    pub avatar_polymorphs: Vec<polymorph_live::AvatarPolymorph>,
    /// Live TornadoEntity instances, oldest first (a ring of 16).
    pub tornadoes: Vec<tornado_live::LiveTornado>,
    /// Live VortexEntity instances, oldest first (a ring of 8).
    pub vortices: Vec<vortex_live::LiveVortex>,
    /// Wave effects, live or still parenting pillars.
    pub waves: Vec<wave_live::LiveWave>,
    /// WaveEntity mAnimationTimer: rockpillar_animation's first clip.
    pub wave_pillar_clip: Option<f32>,
    /// The two Portal singletons and their presentation.
    pub portals: portal_live::PortalHost,
    /// FoundMoose / KilledAPony profile events (kind, id), in order.
    pub profile_events: Vec<(String, String)>,
    /// AudioManager.SetMusicFocus(position, radius, clearTo).
    pub music_focus: Option<([f32; 3], f32, f32)>,
    /// MagickCamera.Follow(entity) target.
    pub camera_follow: Option<u16>,
    /// ControlManager.LockPlayerInput, owned by Death's clone phase.
    pub player_input_locked: crate::slots::PerSlot<bool>,
    /// Player slots on this machine, which share the camera (as local
    /// players did in the original). `None`: every player is local
    /// (offline). Players on other computers have their own cameras.
    pub camera_slots: Option<BTreeSet<usize>>,
    /// NPCs piloted by invaders/dungeon masters (rewrite extension).
    pub possessed: BTreeMap<u16, possession::Possession>,
    /// Highlight requests for the presentation, drained by it.
    pub highlights: Vec<campaign_live::Highlight>,
    /// DisplayTitleInLevel request for the HUD.
    /// Level.mTitleRenderer while mHasTitleDisplaying.
    pub level_title: Option<(
        crate::title_renderer::TitleRenderer,
        crate::title_renderer::TitleKind,
    )>,
    /// SpawnCloneWizards NPCs and the player slot whose colour they take.
    pub clone_wizards: BTreeMap<u16, usize>,
    /// The level title submitted for drawing this update.
    pub shown_level_title: Option<crate::title_renderer::ShownTitle>,
    /// PlayState.UIEnabled (DisableUI/EnableUI).
    pub ui_enabled: bool,
    /// AddCameraEffect / RemoveCameraEffect for the session's MagickCamera
    /// (which outlives scenes): (effect, add); (0, false) removes all.
    pub camera_effect_requests: Vec<(i32, bool)>,
    /// Equipment work for the session (campaign_action).
    pub campaign_requests: Vec<campaign_live::CampaignRequest>,
    /// PlayState.DiedInLevel.
    pub died_in_level_flag: bool,
    /// BossFight singleton and PlayState.GenericHealthBar.
    pub boss_fight: boss_live::BossFight,
    pub scene_trigger_ids: BTreeSet<i32>,
    pub(super) boss_casters: BTreeMap<u16, boss_spells::BossCaster>,
    pub generic_health_bar: boss_live::GenericHealthBar,
    /// The SummonPhoenix singleton (static state in the original).
    pub phoenix: magicka_sim::phoenix::SummonPhoenix,
    /// The phoenix descend clip duration, loaded on first use.
    phoenix_clip: Option<f32>,
    pub death: death_live::DeathHost,
    pub jormungandr: BTreeMap<u16, crate::jormungandr::Jormungandr>,
    pub elemental_eggs: Option<Box<crate::elemental_egg_actor::ElementalEggCache>>,
    pub time_modifier: f32,
    pub earthquake: magicka_sim::earthquake::Earthquake,
    pub earthquake_rigid: BTreeMap<u16, earthquake_live::RigidTarget>,
    pub earthquake_velocities: Vec<(magicka_physics::BodyKey, [f32; 3])>,
    /// Outgoing authority centers; transport integration remains pending.
    pub earthquake_centers: Vec<(Option<u16>, [f32; 3], f32)>,
    earthquake_cue: Option<u32>,
    earthquake_effect: Option<u64>,
    pub faction_child_spawns: Vec<slime_live::FactionChildSpawn>,
    pub crosses: Vec<u16>,
    bubble_effects: BTreeMap<(u16, u32), u64>,
    /// PlayState.TimeMultiplier (TimeWarp); the host scales the next
    /// frame's world delta by it.
    pub time_multiplier: f32,
    /// Level.CurrentScene.Saturation (set by the host from the scene).
    pub scene_saturation: f32,
    /// RenderManager.Saturation override while a TimeWarp runs.
    pub render_saturation: Option<f32>,
    /// TimeWarp.sCue (stopped by OnRemove).
    time_warp_cue: Option<u32>,
    /// Graphics.Flash singleton: (mIntensity, mTTL) while in SpellManager.
    /// Not a SpecialAbility, so ClearMagicks keeps it.
    pub flash: Option<(f32, f32)>,
    /// Flash.mIntensities of the last update.
    flash_draw: f32,
    /// LightningBolt.InitializeEffect requests from magicks (Thunderbolt).
    pub magick_bolts: Vec<magicka_sim::lightning::BoltVisual>,
    /// GameScene.Indoors (Rain refuses indoors).
    pub scene_indoors: bool,
    /// GameScene.mLightIntensity / LightTargetIntensity: directional light
    /// scale (Rain dims to 0.333).
    pub light_intensity: f32,
    pub light_target_intensity: f32,
    /// Rain.mAmbience (stopped by OnRemove); the presentation follows the
    /// camera with `magick_rain` while `rain_state()` is Some.
    rain_cue: Option<u32>,
    blizzard_cue: Option<u32>,
    /// Player.DeadAge per slot (Player.Update after the entities).
    pub dead_age: crate::slots::PerSlot<f32>,
    /// PlayState.mGameType (the session sets it; Campaign by default).
    pub game_type: crate::end_game::GameType,
    /// Feed AIStateMove leader election / FollowGroup facts to NPC agents.
    /// Rewrite switch for isolating the election (the original always
    /// elects; on by default).
    pub group_leaders_enabled: bool,
    /// Level.SpawnFairy (.lvl Start spawnFairy, default true).
    pub level_spawn_fairy: bool,
    /// PlayState.mSpawnFairies: slots whose fairy (re)spawns at scene entry
    /// and checkpoint restart (research/revival-fairy.md).
    pub spawn_fairies: crate::slots::PerSlot<bool>,
    /// Avatar.RevivalFairy per player slot.
    pub revival_fairies: crate::slots::PerSlot<crate::revival_fairy::RevivalFairy>,
    /// Fairy.mEffectRef (special_fairy) instances by slot.
    fairy_effects: BTreeMap<usize, u64>,
    /// Revive.Update avatar creations for the session to perform (it owns
    /// the content needed to build a new avatar).
    pub revive_spawns: Vec<magicka_sim::magicks::Revive>,
    /// FloatingTextManager and the instances its last Update submitted.
    pub floating_text: crate::floating_text::FloatingTextManager,
    /// A named boss's locked floating damage number (mLastDamageIndex).
    pub boss_number: Option<i32>,
    pub floating_numbers: Vec<crate::floating_text::NumberInstance>,
    /// Gib effect requests this frame (gore, trails, splashes).
    pub gib_visuals: Vec<crate::gibs::GibVisual>,
    /// Live UnderGroundAttacks (Earth weapon swings) with presentation ids.
    pub underground: Vec<(u32, magicka_sim::underground_attack::UndergroundAttack)>,
    /// Underground presentation this frame: (id, effect names when started,
    /// position, direction, removed).
    pub underground_visuals: Vec<UndergroundVisual>,
    next_underground: u32,
    /// Character.Drown effects this frame: (effect name, position, direction).
    pub drown_effects: Vec<(&'static str, [f32; 3], [f32; 3])>,
    /// Contact AddStatusEffect notifications this physics step. Audio consumes
    /// owner notifications; status particles follow the shared live slots.
    pub liquid_status_events: Vec<(u16, magicka_sim::character_status::StatusEvent)>,
    status_cue_keys: BTreeMap<(u16, u16), u32>,
    /// Character.mAttachedSoundCues: (owner, slot) → request key.
    attached_cue_keys: BTreeMap<(u16, usize), u32>,
    pub status_contact_effects: Vec<(&'static str, [f32; 3], [f32; 3])>,
    /// Item PlayEffectEvents this frame (melee events): (effect, position,
    /// direction).
    pub gun_shots: Vec<crate::gun::Shot>,
    pub item_event_effects: Vec<crate::npc_combat::ItemEffectRequest>,
    /// Released NPC items to spawn as pickups: (Wizard item name, item
    /// transform, velocity).
    pub npc_item_releases: Vec<npc_events::ItemRelease>,
    /// Initial pickup velocities: (handle, generation, velocity).
    pub pickup_launches: Vec<(u16, u64, [f32; 3])>,
    /// SpawnItem/SpawnMagick event despawn times of luggage NPCs.
    pub luggage_despawn: BTreeMap<u16, f32>,
    /// Viewport aspect ratio for camera projections.
    pub aspect: f32,
    /// PlayState play time (spell/damage timestamps).
    pub play_time: f64,
    pub secret_areas: Vec<Option<String>>,
    content: C,
    locators: BTreeMap<i32, [f32; 16]>,
    areas: BTreeMap<i32, SceneAreaTransform>,
    templates: BTreeMap<i32, NpcTemplate>,
    template_names: BTreeMap<i32, String>,
    interactables: BTreeSet<i32>,
    /// OnInteract definitions in GameScene.Triggers (SortedList) id order.
    pub interactable_defs: BTreeMap<i32, crate::scene_script::InteractableDef>,
    /// Locator transform and radius by lowercased-name hash (Interactable
    /// ids hash the lowercased locator).
    locator_info: BTreeMap<i32, ([f32; 16], f32)>,
    rng: u64,
    /// MagickaMath.Random's draws for ApplyTemplate's model choice. It is a
    /// separate static stream natively; kept apart from `rng` so default-mesh
    /// spawns do not shift other seeded draws (not the native sequence).
    model_rng: u64,
    pub book_pickups: BTreeMap<u16, crate::magick_books::Book>,
    pub item_pickups: BTreeMap<u16, crate::pickups::WorldItemPickup>,
    pub physics_generations: BTreeMap<u16, u64>,
    /// Static physics entities AnimatedLevelPart.AddEntity attached at
    /// spawn (PhysicsEntity.OnSpawn), by handle -> pre-order part index.
    pub attached_props: BTreeMap<u16, u32>,
    pub(crate) item_templates: BTreeMap<i32, magicka_content::item::Item>,
    next_handle: u16,
}

impl<C: SceneContent> SceneWorld<C> {
    pub fn new(level: Arc<LevelModel>, content: C, seed: u64) -> Result<Self, WorldError> {
        let collision = LiveCollisionScene::load(&level, &magicka_content::skinned::IDENTITY)
            .map_err(|e| format!("collision: {e:?}"))?;
        let presence = PresenceAreas::from_content(&level.trigger_areas)
            .map_err(|e| format!("areas: {e:?}"))?;
        // LevelModel keys: locators case-preserving, trigger areas lowercased.
        let locators = level
            .locators
            .iter()
            .map(|l| (custom_hash(&l.name), l.transform))
            .collect();
        let locator_info = level
            .locators
            .iter()
            .map(|l| {
                (
                    custom_hash(&l.name.to_ascii_lowercase()),
                    (l.transform, l.radius),
                )
            })
            .collect();
        let areas = level
            .trigger_areas
            .iter()
            .map(|a| {
                (
                    custom_hash(&a.name.to_ascii_lowercase()),
                    (a.position, a.orientation, a.sides),
                )
            })
            .collect();
        let snap = level.camera_mesh.as_ref().map(|mesh| {
            mesh.triangles
                .iter()
                .filter_map(|t| {
                    let v = |i: i32| mesh.vertices.get(usize::try_from(i).ok()?).copied();
                    Some([v(t[0])?, v(t[1])?, v(t[2])?])
                })
                .collect()
        });
        let nav = crate::navigation::scene_nav_mesh(&level, collision.parts()).ok();
        let nav_part_transforms =
            crate::navigation::part_transforms(&level.animated_parts, collision.parts());
        Ok(Self {
            ruleset: None,
            survival_health_bar: None,
            statistics: Default::default(),
            versus_settings: crate::ruleset::VersusSettings::defaults(
                crate::ruleset::VersusMode::DeathMatch,
            ),
            player_teams: Default::default(),
            avatar_dead_seen: Default::default(),
            pending_versus: Vec::new(),
            versus_drop_magicks: Vec::new(),
            versus_clock_cue: None,
            versus_waiting: false,
            timed_objective_hud: false,
            pending_survival_luggage: Vec::new(),
            survival_packs: None,
            unlocked_magick_mask: 0,
            boss_scale: true,
            boss_damage_divisor: 1.0,
            area_aliases: BTreeMap::new(),
            physics: PhysicsWorld::new(-50.0),
            missile_kill_plane: -50.0,
            host_portal_walkable_normal: None,
            camera: MagickCamera::new(snap),
            nav,
            nav_part_transforms,
            players: vec![None, None, None, None],
            npcs: BTreeMap::new(),
            agents: Vec::new(),
            ai_update_index: 0,
            pending_triggers: Vec::new(),
            registered_casts: Vec::new(),
            scripted_magicks: 0,
            scripted_magick_casts: Vec::new(),
            dispensers: BTreeMap::new(),
            anim_props: BTreeMap::new(),
            web_strands: Default::default(),
            discharge_spawns: Vec::new(),
            hypnotize_shown: BTreeSet::new(),
            discharge_template: None,
            physics_damage: BTreeMap::new(),
            campaign_looped: None,
            static_prop_boxes: BTreeMap::new(),
            prop_gibbed: BTreeSet::new(),
            pending_magick_unlocks: Vec::new(),
            character_dialogs: BTreeMap::new(),
            dialog_handle_owners: BTreeMap::new(),
            tutorial: Default::default(),
            dialog_text: None,
            dialog_matrix: [[0.0; 4]; 4],
            dialog_failure: None,
            missiles: Vec::new(),
            missile_visuals: Vec::new(),
            rails: Default::default(),
            beam_blasts: Vec::new(),
            ground_blasts: Vec::new(),
            drown_effects: Vec::new(),
            liquid_status_events: Vec::new(),
            status_cue_keys: BTreeMap::new(),
            attached_cue_keys: BTreeMap::new(),
            status_contact_effects: Vec::new(),
            gun_shots: Vec::new(),
            item_event_effects: Vec::new(),
            npc_item_releases: Vec::new(),
            pickup_launches: Vec::new(),
            luggage_despawn: BTreeMap::new(),
            shields: Vec::new(),
            shield_nullifications: Vec::new(),
            mines: magicka_sim::spell_mine::MinePool::new(),
            grease: Default::default(),
            grease_hits: Default::default(),
            summons: BTreeMap::new(),
            meteor_light: false,
            grease_decals: vec![None; magicka_sim::grease::POOL_SIZE],
            mine_clip_duration: None,
            mine_effects: Vec::new(),
            barriers: magicka_sim::barrier::BarrierPool::new(),
            barrier_clips: None,
            barrier_visuals: Vec::new(),
            barrier_cues: BTreeMap::new(),
            pending_end_game: None,
            game_ended: false,
            pending_checkpoint: None,
            checkpoint_notices: Vec::new(),
            entity_blasts: Vec::new(),
            bloats: BTreeMap::new(),
            overkilled: BTreeSet::new(),
            gibs: crate::gibs::Gibs::default(),
            decals: crate::decals::Decals::default(),
            ability_effects: Default::default(),
            size_effects: Default::default(),
            polymorphs: Default::default(),
            polymorph_storage: BTreeMap::new(),
            avatar_polymorphs: Vec::new(),
            tornadoes: Vec::new(),
            vortices: Vec::new(),
            waves: Vec::new(),
            wave_pillar_clip: None,
            portals: Default::default(),
            profile_events: Vec::new(),
            music_focus: None,
            camera_follow: None,
            player_input_locked: Default::default(),
            possessed: BTreeMap::new(),
            camera_slots: None,
            highlights: Vec::new(),
            level_title: None,
            shown_level_title: None,
            clone_wizards: BTreeMap::new(),
            boss_fight: Default::default(),
            scene_trigger_ids: BTreeSet::new(),
            boss_casters: BTreeMap::new(),
            generic_health_bar: Default::default(),
            ui_enabled: true,
            camera_effect_requests: Vec::new(),
            campaign_requests: Vec::new(),
            died_in_level_flag: false,
            phoenix: Default::default(),
            phoenix_clip: None,
            death: Default::default(),
            jormungandr: BTreeMap::new(),
            elemental_eggs: None,
            time_modifier: 1.0,
            earthquake: Default::default(),
            earthquake_rigid: BTreeMap::new(),
            earthquake_velocities: Vec::new(),
            earthquake_centers: Vec::new(),
            earthquake_cue: None,
            earthquake_effect: None,
            faction_child_spawns: Vec::new(),
            crosses: Vec::new(),
            bubble_effects: BTreeMap::new(),
            time_multiplier: 1.0,
            scene_saturation: 1.0,
            render_saturation: None,
            time_warp_cue: None,
            flash: None,
            flash_draw: 0.0,
            magick_bolts: Vec::new(),
            scene_indoors: false,
            light_intensity: 1.0,
            light_target_intensity: 1.0,
            rain_cue: None,
            blizzard_cue: None,
            dead_age: Default::default(),
            game_type: crate::end_game::GameType::Campaign,
            group_leaders_enabled: true,
            level_spawn_fairy: true,
            spawn_fairies: Default::default(),
            revival_fairies: Default::default(),
            fairy_effects: BTreeMap::new(),
            revive_spawns: Vec::new(),
            floating_text: Default::default(),
            boss_number: None,
            floating_numbers: Vec::new(),
            gib_visuals: Vec::new(),
            underground: Vec::new(),
            underground_visuals: Vec::new(),
            next_underground: 1,
            next_missile: 1,
            aspect: 16.0 / 9.0,
            play_time: 0.0,
            level,
            collision,
            presence,
            presence_entities: vec![],
            entities: vec![],
            effects: SceneEffects::default(),
            effect_commands: Vec::new(),
            effect_types: BTreeMap::new(),
            effect_serial: 0,
            sounds: AmbientSounds::default(),
            audio_requests: Vec::new(),
            audio_key: 0,
            spray_area_cues: BTreeMap::new(),
            flags: SceneFlags::default(),
            cutscene: Default::default(),
            render_transition: None,
            transition_cleared: false,
            log: vec![],
            unsupported_specials: BTreeSet::new(),
            staff_effects: Vec::new(),
            role: NetworkRole::Offline,
            dialogs: None,
            pending_scene: None,
            checkpoints: vec![],
            secret_areas: vec![],
            content,
            locators,
            areas,
            templates: BTreeMap::new(),
            template_names: BTreeMap::new(),
            interactables: BTreeSet::new(),
            interactable_defs: BTreeMap::new(),
            locator_info,
            rng: seed | 1,
            model_rng: seed.rotate_left(29) | 1,
            book_pickups: BTreeMap::new(),
            item_pickups: BTreeMap::new(),
            physics_generations: BTreeMap::new(),
            attached_props: BTreeMap::new(),
            item_templates: BTreeMap::new(),
            next_handle: 0,
        })
    }

    /// GameScene.Triggers entries created from OnInteract (lowercased `locator`).
    pub fn register_interactables(&mut self, layout: &crate::scene_script::SceneScriptLayout) {
        self.scene_trigger_ids = layout.triggers.iter().map(|t| t.id).collect();
        self.interactables = layout.interactables.iter().map(|d| d.id).collect();
        self.interactable_defs = layout
            .interactables
            .iter()
            .map(|d| (d.id, d.clone()))
            .collect();
        for d in &layout.interactables {
            if !d.enabled {
                self.flags.interactables_disabled.insert(d.id);
            }
        }
    }

    fn interactable_enabled(&self, id: i32) -> bool {
        self.interactable_defs.contains_key(&id) && !self.flags.interactables_disabled.contains(&id)
    }

    /// Interactable.Locator: the locator position and radius.
    pub fn interactable_locator(&self, id: i32) -> Option<([f32; 3], f32)> {
        let (m, radius) = self.locator_info.get(&id)?;
        Some(([m[12], m[13], m[14]], *radius))
    }

    /// KeyboardMouseController.FindInteractable(ref Segment): the enabled
    /// interactable whose locator is nearest the mouse segment, within the
    /// locator radius (ties keep the later trigger). AudioManager.Threat's
    /// Talk exclusion is not modelled.
    pub fn interactable_under_ray(&self, segment: Segment) -> Option<i32> {
        let mut best: Option<(i32, f32)> = None;
        for &id in self.interactable_defs.keys() {
            if !self.interactable_enabled(id) {
                continue;
            }
            let Some((p, radius)) = self.interactable_locator(id) else {
                continue;
            };
            let d = point_segment_distance_sq(p, segment);
            if d <= radius * radius && best.is_none_or(|(_, b)| d <= b) {
                best = Some((id, d));
            }
        }
        best.map(|(id, _)| id)
    }

    pub fn talk_character_available(&self, handle: u16, faction: i32) -> bool {
        self.character_dialogs
            .get(&handle)
            .is_some_and(|dialog| *dialog != 0)
            && self.entities.iter().any(|e| {
                e.handle == handle
                    && e.added
                    && !e.removed
                    && !e.dead
                    && e.faction & faction != 0
                    && matches!(e.kind, EntityKind::Npc { .. })
            })
    }

    /// Controller FindCharacter: candidates near the mouse plane, intersecting
    /// their character capsule. Native candidate radius is six units.
    pub fn talk_character_under_ray(
        &self,
        segment: Segment,
        plane: [f32; 3],
        faction: i32,
    ) -> Option<u16> {
        self.entities.iter().find_map(|e| {
            if !self.talk_character_available(e.handle, faction) {
                return None;
            }
            let distance = (0..3)
                .map(|i| (e.position[i] - plane[i]).powi(2))
                .sum::<f32>();
            if distance > (6. + e.radius).powi(2) {
                return None;
            }
            magicka_sim::railgun::character_segment_intersect(
                e.position,
                [0., 1., 0.],
                e.length,
                e.radius,
                false,
                segment,
                0.,
            )
            .map(|_| e.handle)
        })
    }

    /// Avatar FindCharacter(false): closest interactable character from the
    /// three-unit entity query; compare horizontal distances.
    pub fn talk_character_in_reach(&self, position: [f32; 3], faction: i32) -> Option<u16> {
        let mut best = None;
        for e in &self.entities {
            if !self.talk_character_available(e.handle, faction) {
                continue;
            }
            let distance = (0..3)
                .map(|i| (e.position[i] - position[i]).powi(2))
                .sum::<f32>();
            if distance > (3. + e.radius).powi(2) {
                continue;
            }
            let horizontal =
                (e.position[0] - position[0]).powi(2) + (e.position[2] - position[2]).powi(2);
            if best.is_none_or(|(_, d)| horizontal < d) {
                best = Some((e.handle, horizontal));
            }
        }
        best.map(|(handle, _)| handle)
    }

    pub fn interact_character(&mut self, handle: u16, controller: i32) -> Result<bool, WorldError> {
        let Some(e) = self
            .entities
            .iter()
            .find(|e| e.handle == handle && !e.dead && !e.removed)
        else {
            return Ok(false);
        };
        let owner = e.unique_id;
        let dialog = self.character_dialogs.get(&handle).copied().unwrap_or(0);
        if dialog == 0 {
            return Ok(false);
        }
        let Some(mut manager) = self.dialogs.take() else {
            return Ok(false);
        };
        let Some(mut text) = self.dialog_text.take() else {
            self.dialogs = Some(manager);
            return Ok(false);
        };
        // Character.Interact turns toward its caller when the upcoming
        // message requests TurnToTarget (the XML Facing property).
        let turn = manager
            .dialog(dialog)
            .and_then(|dialog| {
                let index = usize::try_from(dialog.cursor().next).ok()?;
                let current = usize::try_from(dialog.interaction(index)?.current).ok()?;
                dialog
                    .definition()
                    .interactions
                    .get(index)?
                    .messages
                    .get(current)
                    .map(|message| message.facing)
            })
            .unwrap_or(false);
        if turn {
            let caller = usize::try_from(controller)
                .ok()
                .and_then(|slot| self.players.get(slot))
                .and_then(Option::as_ref)
                .map(|player| self.physics.bodies[player.body].position);
            if let (Some(caller), Some(npc)) = (caller, self.npcs.get(&handle)) {
                let body = &mut self.physics.bodies[npc.body];
                body.set_desired_direction(std::array::from_fn(|i| caller[i] - body.position[i]));
            }
        }
        self.refresh_dialog_world(&mut text);
        let result = manager
            .start(
                dialog,
                crate::interaction_playback::InteractionLocation::Entity(Some(owner)),
                Some(controller),
                self.flags.cutscene,
                &mut text,
            )
            .map(|slot| slot.is_some())
            .map_err(|e| format!("NPC dialog: {e:?}"));
        self.dialogs = Some(manager);
        self.dialog_text = Some(text);
        self.apply_dialog_commands();
        result
    }

    /// Avatar.FindInteractable(checkDirection): the first enabled
    /// interactable (trigger id order) within the avatar's capsule radius or
    /// the locator radius, horizontally; with `direction`, the latter also
    /// needs a facing dot product above 0.7.
    pub fn interactable_in_reach(
        &self,
        position: [f32; 3],
        radius: f32,
        direction: Option<[f32; 3]>,
    ) -> Option<i32> {
        for &id in self.interactable_defs.keys() {
            if !self.interactable_enabled(id) {
                continue;
            }
            let Some((p, locator_radius)) = self.interactable_locator(id) else {
                continue;
            };
            let (dx, dz) = (p[0] - position[0], p[2] - position[2]);
            let d = dx * dx + dz * dz;
            if d <= radius * radius {
                return Some(id);
            }
            if d <= locator_radius * locator_radius {
                let Some(f) = direction else {
                    return Some(id);
                };
                let l = d.sqrt();
                if l > 0.0 && (f[0] * dx + f[2] * dz) / l > 0.7 {
                    return Some(id);
                }
            }
        }
        None
    }
    /// Character.ApplyTemplate's model index: a negative MeshId draws
    /// MagickaMath.Random.Next(0, count); one past the end falls back to 0.
    fn choose_model(&mut self, mesh: i32, count: usize) -> usize {
        if mesh < 0 {
            self.model_rng ^= self.model_rng << 13;
            self.model_rng ^= self.model_rng >> 7;
            self.model_rng ^= self.model_rng << 17;
            ((self.model_rng >> 32) % count.max(1) as u64) as usize
        } else if mesh as usize >= count {
            0
        } else {
            mesh as usize
        }
    }

    /// Seeded xorshift in [0,1); not the native System.Random sequence.
    pub(crate) fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Centre of a trigger area box (developer tooling, not an original
    /// query): the box-local midpoint through the GetRandomLocation transform.
    pub fn area_center(&self, name: &str) -> Option<[f32; 3]> {
        let (position, orientation, sides) =
            *self.areas.get(&custom_hash(&name.to_ascii_lowercase()))?;
        let m = pose_matrix([0.0; 3], orientation, [1.0; 3]);
        let local = sides.map(|v| v * 0.5);
        Some(std::array::from_fn(|j| {
            position[j] + (0..3).map(|i| local[i] * m[i * 4 + j]).sum::<f32>()
        }))
    }

    /// Whether a point lies in a named area's box.
    pub fn area_contains(&self, name: &str, point: [f32; 3]) -> bool {
        let Some(&(position, orientation, sides)) =
            self.areas.get(&custom_hash(&name.to_ascii_lowercase()))
        else {
            return false;
        };
        let m = pose_matrix([0.0; 3], orientation, [1.0; 3]);
        let d: [f32; 3] = std::array::from_fn(|j| point[j] - position[j]);
        (0..3).all(|i| {
            let local: f32 = (0..3).map(|j| d[j] * m[i * 4 + j]).sum();
            (0.0..=sides[i]).contains(&local)
        })
    }

    /// A named locator's position.
    pub fn locator_position(&self, name: &str) -> Option<[f32; 3]> {
        let m = self
            .locators
            .get(&custom_hash(&name.to_ascii_lowercase()))?;
        Some([m[12], m[13], m[14]])
    }

    /// Trigger area names of the scene model, in content order.
    pub fn area_names(&self) -> Vec<String> {
        self.level
            .trigger_areas
            .iter()
            .map(|a| a.name.clone())
            .collect()
    }

    /// TriggerArea.GetRandomLocation: uniform box-local point times the box
    /// transform (rotation rows, then the corner translation).
    fn random_area_point(&mut self, id: i32) -> Option<[f32; 3]> {
        let (position, orientation, sides) = *self.areas.get(&id)?;
        let m = pose_matrix([0.0; 3], orientation, [1.0; 3]);
        let local = [
            self.random() * sides[0],
            self.random() * sides[1],
            self.random() * sides[2],
        ];
        Some(std::array::from_fn(|j| {
            position[j] + (0..3).map(|i| local[i] * m[i * 4 + j]).sum::<f32>()
        }))
    }

    pub(crate) fn entity(&self, handle: u16) -> Result<&SceneEntity, WorldError> {
        self.entities
            .iter()
            .find(|e| e.handle == handle)
            .ok_or_else(|| format!("no entity {handle}"))
    }
    pub(crate) fn entity_mut(&mut self, handle: u16) -> Result<&mut SceneEntity, WorldError> {
        self.entities
            .iter_mut()
            .find(|e| e.handle == handle)
            .ok_or_else(|| format!("no entity {handle}"))
    }
    fn by_unique_id(&self, id: i32) -> Option<&SceneEntity> {
        // Entity.Initialize overwrites mUniqueEntities[id]. A new named spawn
        // supersedes its old corpse; removing it must not expose that corpse.
        self.entities
            .iter()
            .rev()
            .find(|e| e.added && e.unique_id == id)
            .filter(|e| !e.removed)
    }
    fn new_handle(&mut self) -> u16 {
        self.next_handle += 1;
        self.next_handle
    }
    fn backend(&mut self) -> Backend<'_> {
        Backend {
            effect_commands: &mut self.effect_commands,
            effect_serial: &mut self.effect_serial,
            content: &mut self.content,
            log: &mut self.log,
            audio: &mut self.audio_requests,
            audio_key: &mut self.audio_key,
        }
    }

    /// Rebuild named-area collision membership and global live entity facts.
    pub fn refresh_presence(&mut self) {
        self.presence_entities = self
            .entities
            .iter()
            .filter(|e| e.added && !e.removed)
            .map(|e| PresenceEntity {
                handle: e.handle,
                unique_id: e.unique_id,
                position: e.position,
                radius: e.radius,
                dead: e.dead,
                ignored_avatar: false,
                physics: match e.kind {
                    EntityKind::Physics { damageable } => Some(damageable),
                    _ => None,
                },
                character: match e.kind {
                    EntityKind::Npc { type_id, .. } => Some(PresenceCharacter {
                        type_id: npc_trigger_type(type_id),
                        original_faction: e.original_faction,
                        faction: self.faction(e),
                        undying: e.undying,
                        invisible: false,
                    }),
                    EntityKind::Avatar { slot } => {
                        self.players[slot].as_ref().map(|p| PresenceCharacter {
                            type_id: p.trigger_type,
                            original_faction: p.faction,
                            faction: p.faction,
                            undying: false,
                            invisible: false,
                        })
                    }
                    EntityKind::Physics { .. }
                    | EntityKind::Item
                    | EntityKind::ElementalEgg
                    | EntityKind::Dispenser => None,
                },
            })
            .collect();
        let npcs = &self.npcs;
        let players = &self.players;
        let bodies = &self.physics.bodies;
        self.presence
            .update_contacts(&self.presence_entities, |entity| {
                let index = npcs.get(&entity.handle).map(|npc| npc.body).or_else(|| {
                    players
                        .iter()
                        .flatten()
                        .find(|p| p.handle == entity.handle)
                        .map(|p| p.body)
                })?;
                Some(bodies[index].capsule_motion())
            });
    }

    /// One Level.Update: timers, animated parts, presence, then scene triggers.
    pub fn step(
        &mut self,
        runtime: &mut ScriptRuntime,
        dt: f32,
    ) -> Result<(), ScriptRuntimeError<WorldError>> {
        runtime.variables.update(dt, self.timer_frame());
        self.collision
            .update_with_decals(dt, &magicka_content::skinned::IDENTITY, &mut self.decals)
            .map_err(|e| ScriptRuntimeError::World(format!("collision update: {e:?}")))?;
        self.run_scene_scripts(runtime, dt)
    }
}

impl<C: SceneContent> AiEventLevel for SceneWorld<C> {
    type Error = WorldError;
    fn locator(&self, id: i32) -> Option<[f32; 16]> {
        self.locators.get(&id).copied()
    }
    fn trigger_area_location(&mut self, id: i32) -> Result<Option<[f32; 3]>, WorldError> {
        Ok(self.random_area_point(id))
    }
}
impl<C: SceneContent> SpawnLevel for SceneWorld<C> {
    fn initialize_spawn_item(
        &mut self,
        action: &crate::spawn_item::SpawnItemAction,
        state: &mut crate::spawn_item::SpawnItemState,
    ) -> Option<Result<(), crate::spawn_item::SpawnItemError<WorldError>>> {
        Some(action.initialize(state, self))
    }

    fn load_character_template(&mut self, name: Option<&str>) -> Result<(), WorldError> {
        let name = name.ok_or("Content.Load(\"Data/Characters/\")")?;
        let template = self
            .content
            .character(name)
            .ok_or_else(|| format!("missing character template {name}"))?;
        self.templates.insert(custom_hash(name), template);
        self.template_names
            .insert(custom_hash(name), name.to_string());
        Ok(())
    }
}
impl<C: SceneContent> AvatarMoveLevel for SceneWorld<C> {
    fn entity_by_id(&self, id: i32) -> Option<u16> {
        self.by_unique_id(id).map(|e| e.handle)
    }
    fn player_avatars(&self) -> Vec<Option<u16>> {
        self.players
            .iter()
            .map(|p| p.as_ref().map(|p| p.handle))
            .collect()
    }
}
impl<C: SceneContent> SpawnPhysLevel for SceneWorld<C> {
    fn load_physics_template(&mut self, object: Option<&str>) -> Result<i32, WorldError> {
        let name = object.ok_or("Content.Load(\"Data/PhysicsEntities/\")")?;
        self.content
            .physics_max_hit_points(name)
            .ok_or_else(|| format!("missing physics template {name}"))
    }
    fn damageable_from_cache(&mut self) -> Result<u16, WorldError> {
        Ok(self.allocate_physics(true))
    }
    fn new_physics_entity(&mut self) -> Result<u16, WorldError> {
        Ok(self.allocate_physics(false))
    }
    fn physics_template_animated(&mut self, object: Option<&str>) -> bool {
        object
            .and_then(|o| self.content.physics_template(o))
            .is_some_and(|t| crate::anim_prop::is_animated(&t))
    }
}
impl<C: SceneContent> SceneWorld<C> {
    /// DamageablePhysicsEntity public damage health/status path. Both native
    /// callbacks use a null sender, including death before damage on a lethal hit.
    pub fn damage_physics(
        &mut self,
        handle: u16,
        damage: magicka_sim::damage::Damage,
        features: u8,
    ) -> Result<u16, WorldError> {
        if self.role == NetworkRole::Client {
            return Ok(0);
        }
        let state = self
            .physics_damage
            .get_mut(&handle)
            .ok_or_else(|| format!("physics damage template unavailable for {handle}"))?;
        let result = state.deliver(&[damage], features, &mut self.pending_triggers);
        let health = state.health;
        let entity = self.entity_mut(handle)?;
        entity.hit_points = health;
        entity.dead = health <= 0.;
        Ok(result)
    }

    /// The PhysAnimation block at the top of DamageablePhysicsEntity.Update.
    fn update_phys_animations(&mut self) {
        let mut plays = Vec::new();
        for entity in self.entities.iter().filter(|e| e.added && !e.removed) {
            if let Some(state) = self.physics_damage.get_mut(&entity.handle) {
                plays.extend(state.animations.update(state.health, state.maximum));
            }
        }
        for (path, children, command) in plays {
            // Validated when the control was added.
            if let Ok(part) = self.collision.part_by_ids_mut(&path) {
                part.play(children, command);
            }
        }
    }

    fn update_physics_damage(&mut self, dt: f32) {
        if self.role == NetworkRole::Client {
            return;
        }
        for entity in &mut self.entities {
            if !entity.added || entity.removed {
                continue;
            }
            let Some(state) = self.physics_damage.get_mut(&entity.handle) else {
                continue;
            };
            for _ in 0..state.update(dt) {
                if state.on_death != 0 {
                    self.pending_triggers.push((state.on_death, None));
                }
            }
            entity.hit_points = state.health;
            entity.dead = state.health <= 0.;
        }
    }

    /// PhysicsEntity.OnCollision for character contacts: each entity newly
    /// touching a live prop's first collision box (capsule axis within the
    /// capsule radius) enters its 0.25 s HitList and the template conditions
    /// run with Hit | Collision against it (Furnace_fire: Fire status 250,
    /// magnitude 2). Only Damage events are delivered; other entity kinds
    /// and the Collision threshold/Repeat bookkeeping are not modelled.
    fn prop_contact_conditions(&mut self, dt: f32) {
        use magicka_content::common::Event;
        if self.role == NetworkRole::Client {
            return;
        }
        let combatants = self.combatants();
        let mut hits = Vec::new();
        for e in self
            .entities
            .iter()
            .filter(|e| e.added && !e.removed && !e.dead)
        {
            let Some(state) = self.physics_damage.get_mut(&e.handle) else {
                continue;
            };
            state.hit_list.update(dt);
            if state.conditions.is_empty() || state.health <= 0.0 {
                continue;
            }
            let Some(b) = state.world_box(&e.orientation) else {
                continue;
            };
            for c in combatants.iter().filter(|c| !c.dead) {
                if state.hit_list.contains(c.handle) {
                    continue;
                }
                let segment = magicka_sim::spray_pulse::Segment {
                    origin: [c.position[0], c.position[1] - c.length * 0.5, c.position[2]],
                    delta: [0.0, c.length, 0.0],
                };
                let (d2, _, _) = magicka_sim::capsule_box::segment_distance(segment, b);
                if d2 > c.radius * c.radius {
                    continue;
                }
                state.hit_list.add(c.handle);
                for collection in state.conditions.iter() {
                    let k = &collection.condition;
                    let met = k.condition_type & 2 != 0
                        || (k.condition_type & 4 != 0 && k.threshold <= 0.0);
                    if !met {
                        continue;
                    }
                    for event in &collection.events {
                        if let Event::Damage { damage, .. } = event {
                            hits.push((
                                c.handle,
                                magicka_sim::damage::Damage {
                                    attack: damage.attack_property as u16,
                                    element: damage.element as u32,
                                    amount: damage.amount,
                                    magnitude: damage.magnitude,
                                },
                                e.handle,
                                e.position,
                            ));
                        }
                    }
                }
            }
        }
        if hits.is_empty() {
            return;
        }
        let Some(view) =
            crate::npc_combat::CombatView::new(&combatants, Some(self.collision.collision()))
        else {
            return;
        };
        for (target, damage, attacker, position) in hits {
            let mut world = SceneHitWorld {
                physics_damage: &mut self.physics_damage,
                dispensers: &mut self.dispensers,
                entities: &mut self.entities,
                view: &view,
                npcs: &mut self.npcs,
                players: &mut self.players,
                bodies: &mut self.physics.bodies,
                triggers: &mut self.pending_triggers,
                shields: &mut self.shields,
                collision_rng: &mut self.rng,
                play_time: self.play_time,
                entity_blasts: &mut self.entity_blasts,
                barriers: None,
                barrier_events: Vec::new(),
                death: &mut self.death,
                jormungandr: &mut self.jormungandr,
                death_damage_features: self.role != NetworkRole::Client,
            };
            crate::missiles::MissileWorld::damage_character(
                &mut world, target, damage, attacker, position,
            );
        }
    }

    /// EntityManager.RemoveDeadEntities for physics props:
    /// `PhysicsEntity.Removable => Dead` (DamageablePhysicsEntity.Dead is
    /// HitPoints <= 0, AnimatedPhysicsEntity inherits it), so a dead prop
    /// leaves the entity list and stops answering GetEntities queries
    /// (beams, sprays, areas). Without this a killed beam globe stayed at
    /// its spot and took the beam meant for its replacement. Boss-backed
    /// damage states keep their own lifetime.
    fn remove_dead_props(&mut self) {
        for e in &mut self.entities {
            if !e.added || e.removed || !matches!(e.kind, EntityKind::Physics { .. }) {
                continue;
            }
            let Some(state) = self.physics_damage.get(&e.handle) else {
                continue;
            };
            if state.health > 0.0
                || state.death_boss.is_some()
                || state.boss_zone.is_some()
                || state.boss_capsule.is_some()
            {
                continue;
            }
            e.dead = true;
            e.removed = true;
            self.attached_props.remove(&e.handle);
        }
    }

    /// DamageablePhysicsEntity.SpawnGibs for props that died since the last
    /// pass (the OnDeath trigger is dispatched where the death happens).
    fn spawn_prop_gibs(&mut self) {
        let mut dead = Vec::new();
        for e in &self.entities {
            let Some(state) = self.physics_damage.get(&e.handle) else {
                continue;
            };
            if !e.added || state.health > 0.0 {
                continue;
            }
            let generation = *self.physics_generations.get(&e.handle).unwrap_or(&0);
            if self.prop_gibbed.insert((e.handle, generation)) {
                let frozen = state.status_mask & magicka_sim::status::Status::Frozen as u16 != 0;
                dead.push((
                    e.handle,
                    e.position,
                    e.radius,
                    frozen,
                    e.template_name.clone(),
                    state.conditions.clone(),
                ));
            }
        }
        for (handle, position, radius, frozen, name, conditions) in dead {
            // DamageablePhysicsEntity.SpawnGibs executes Death conditions even
            // when the template has no debris. Keep the generation guard above
            // so an explosion is dispatched once, including chain reactions.
            self.melee_event_presentation(&conditions, 32, handle, -1, position, [0., 0., -1.]);
            for collection in conditions
                .iter()
                .filter(|c| c.condition.condition_type & 32 != 0)
            {
                for event in &collection.events {
                    if let magicka_content::common::Event::Splash { damage, radius } = event {
                        self.arc_damage(
                            Some(handle),
                            Some(handle),
                            position,
                            [0., 0., -1.],
                            *radius,
                            std::f32::consts::PI,
                            &[magicka_sim::damage::Damage {
                                attack: damage.attack_property as u16,
                                element: damage.element as u32,
                                amount: damage.amount,
                                magnitude: damage.magnitude,
                            }],
                            self.play_time,
                        );
                    }
                }
            }
            let Some(name) = name else { continue };
            let gibs = self.content.physics_gibs(&name);
            let trail = self
                .content
                .physics_template(&name)
                .map(|t| t.gib_trail_effect)
                .unwrap_or_default();
            // The stronger never-used board launch was requested for the
            // Earth lesson; it must not accelerate every other prop's gibs.
            let fresh_definition_mass = name.eq_ignore_ascii_case("dungeon_wall");
            let visuals = self.gibs.spawn_physics(
                position,
                radius,
                frozen,
                &trail,
                &gibs,
                fresh_definition_mass,
            );
            self.gib_visuals.extend(visuals);
            self.log.push(format!("prop gibs {name}: {}", gibs.len()));
        }
    }

    /// Character.IsEntangled onto the bodies (CharacterBody reads it) and
    /// the webs' presentation: web_hit at the character, facing its
    /// direction, and misc_spider_web_cut on release.
    fn sync_entanglement(&mut self) {
        let mut out = Vec::new();
        let players = self
            .players
            .iter_mut()
            .flatten()
            .map(|p| (p.body, &mut p.character));
        let npcs = self.npcs.values_mut().map(|n| (n.body, &mut n.character));
        for (body, c) in players.chain(npcs) {
            let b = &mut self.physics.bodies[body];
            b.owner.entangled = c.entanglement.entangled();
            for e in c.entangle_events.drain(..) {
                out.push((e, b.position, b.direction()));
            }
        }
        for (e, position, direction) in out {
            match e {
                magicka_sim::entanglement::EntangleEvent::HitEffect => self
                    .status_contact_effects
                    .push((magicka_sim::entanglement::HIT_EFFECT, position, direction)),
                magicka_sim::entanglement::EntangleEvent::BreakSound => {
                    self.play_cue(
                        crate::audio::banks::MISC,
                        magicka_sim::entanglement::BREAK_SOUND,
                        Some(position),
                    );
                }
            }
        }
    }

    /// AnimatedPhysicsEntity.Update: advance each live prop's clip.
    fn update_anim_props(&mut self, dt: f32) {
        for (handle, prop) in &mut self.anim_props {
            let Some(e) = self.entities.iter().find(|e| e.handle == *handle) else {
                continue;
            };
            if e.added && !e.removed {
                prop.update(dt, e.dead);
            }
        }
    }
    /// EntityManager.AddEntity -> PhysicsEntity.OnSpawn: a static
    /// (non-Movable) entity casts 4 m down from its body position and, when
    /// the nearest scene hit is an animated level part, that part's
    /// AddEntity keeps it with a float.MaxValue contact time, so it rides
    /// the part until it is Removable.
    pub(crate) fn attach_static_prop(&mut self, handle: u16) {
        self.attached_props.remove(&handle);
        let Some(e) = self.entities.iter().find(|e| e.handle == handle) else {
            return;
        };
        let Some(template) = e
            .template_name
            .as_deref()
            .and_then(|o| self.content.physics_template(o))
        else {
            return;
        };
        if template.movable {
            return;
        }
        let p = e.position;
        let down = Segment {
            origin: p,
            delta: [0., -4., 0.],
        };
        if let Some(owner) = self
            .collision
            .collision()
            .segment_intersect(down, false)
            .and_then(|hit| hit.animated_owner)
        {
            self.attached_props.insert(handle, owner);
        }
    }

    /// AnimatedLevelPart.Update for attached entities: drop Removable ones
    /// and transform each body (position and orientation) by the part's
    /// `inverse(old) * new` delta. The native loop also moves an entity in
    /// the update that removes it.
    fn carry_attached_props(&mut self) -> Result<(), ScriptRuntimeError<WorldError>> {
        if self.attached_props.is_empty() {
            return Ok(());
        }
        let deltas = self
            .collision
            .part_deltas()
            .map_err(|e| ScriptRuntimeError::World(format!("prop carry: {e:?}")))?;
        let attached = std::mem::take(&mut self.attached_props);
        for (handle, owner) in attached {
            let Some(e) = self.entities.iter_mut().find(|e| e.handle == handle) else {
                continue;
            };
            let Some(delta) = deltas.get(owner as usize) else {
                continue;
            };
            let mut m = e.orientation;
            m[12..15].copy_from_slice(&e.position);
            let m = magicka_content::skinned::mul(&m, delta);
            e.orientation = m;
            e.position = [m[12], m[13], m[14]];
            if !(e.removed || e.dead || !e.added) {
                self.attached_props.insert(handle, owner);
            }
        }
        Ok(())
    }

    fn allocate_physics(&mut self, damageable: bool) -> u16 {
        let handle = self.new_handle();
        self.entities.push(SceneEntity {
            handle,
            unique_id: 0,
            kind: EntityKind::Physics { damageable },
            position: [0.0; 3],
            orientation: magicka_content::skinned::IDENTITY,
            hit_points: 0.0,
            max_hit_points: 0.0,
            original_faction: 0,
            faction: 0,
            radius: 0.0,
            length: 0.0,
            template_name: None,
            animation: 1,
            special_idle: 0,
            undying: false,
            cannot_die_without_explicit_kill: false,
            dead: false,
            added: false,
            removed: false,
            order: None,
            event_count: 0,
        });
        handle
    }
    /// GameScene.GetLocator(int, out Matrix): "any", static locators,
    /// trigger areas, then animated-part locators at the parts' current
    /// world transforms.
    pub(crate) fn scene_locator(&mut self, id: i32) -> Result<[f32; 16], WorldError> {
        let mut id = *self.area_aliases.get(&id).unwrap_or(&id);
        if id == custom_hash("any") {
            // IRuleset.GetAnyArea: Survival picks a spawn area; the other
            // rulesets return the "any" trigger area itself.
            match self
                .ruleset
                .as_ref()
                .map(|r| r.survival().map(|s| s.areas.len()))
            {
                None => return Err("GetLocator(any): scene has no ruleset".into()),
                Some(Some(0)) => return Err("GetLocator(any): no ruleset spawn areas".into()),
                Some(Some(count)) => {
                    let pick = (self.random() * count as f32) as usize;
                    let r = self.ruleset.as_ref().and_then(|r| r.survival()).unwrap();
                    id = custom_hash(&r.areas[pick].to_ascii_lowercase());
                }
                Some(None) => {}
            }
        }
        if let Some(m) = self.locators.get(&id) {
            return Ok(*m);
        }
        if self.areas.contains_key(&id) {
            let angle = self.random() * std::f32::consts::TAU;
            let (s, c) = angle.sin_cos();
            // Matrix.CreateRotationY.
            let mut m = magicka_content::skinned::IDENTITY;
            m[0] = c;
            m[2] = -s;
            m[8] = s;
            m[10] = c;
            let p = self.random_area_point(id).unwrap();
            m[12..15].copy_from_slice(&p);
            return Ok(m);
        }
        crate::level_effects::part_locator(&self.level.animated_parts, self.collision.parts(), id)
            .ok_or_else(|| format!("No area with key {id}"))
    }
    fn area_characters(&self, area: i32) -> Option<Vec<usize>> {
        self.presence.get(area).ok().map(|a| a.characters.clone())
    }
}

impl<C: SceneContent> SpawnHost for SceneWorld<C> {
    type Error = WorldError;
    fn network_role(&self) -> NetworkRole {
        self.role
    }
    fn template_move_abilities(&mut self, type_id: i32) -> Result<i32, WorldError> {
        self.templates
            .get(&type_id)
            .map(|_| 0)
            .ok_or_else(|| format!("GetCachedTemplate({type_id}) before load"))
    }
    fn acquire_npc(&mut self) -> Result<NpcInstance, WorldError> {
        let handle = self.new_handle();
        self.entities.push(SceneEntity {
            handle,
            unique_id: 0,
            kind: EntityKind::Npc {
                type_id: 0,
                template: 0,
            },
            position: [0.0; 3],
            orientation: magicka_content::skinned::IDENTITY,
            hit_points: 0.0,
            max_hit_points: 0.0,
            original_faction: 0,
            faction: 0,
            radius: 0.0,
            length: 0.0,
            template_name: None,
            animation: 1,
            special_idle: 0,
            undying: false,
            cannot_die_without_explicit_kill: false,
            dead: false,
            added: false,
            removed: false,
            order: None,
            event_count: 0,
        });
        Ok(NpcInstance {
            handle,
            boss: false,
        })
    }
    fn locator(&mut self, area: i32) -> Result<[f32; 16], WorldError> {
        self.scene_locator(area)
    }
    fn nearest_nav_position(
        &mut self,
        position: [f32; 3],
        abilities: i32,
    ) -> Result<[f32; 3], WorldError> {
        let nav = self.nav.as_ref().ok_or("scene has no navmesh")?;
        Ok(nav
            .nearest(position, abilities as u8)
            .map_or(position, |(_, point, _)| point))
    }
    fn initialize_npc(
        &mut self,
        npc: u16,
        type_id: i32,
        mesh: i32,
        position: [f32; 3],
        unique_id: i32,
    ) -> Result<f32, WorldError> {
        self.character_dialogs.insert(npc, 0);
        let template = self
            .templates
            .get(&type_id)
            .cloned()
            .ok_or_else(|| format!("template {type_id} not loaded"))?;
        let name = self.template_names.get(&type_id).cloned();
        let e = self.entity_mut(npc)?;
        e.kind = EntityKind::Npc {
            type_id: template.type_id,
            template: type_id,
        };
        e.unique_id = unique_id;
        // Character.Initialize: Y -= HeightOffset (feet to capsule centre).
        e.position = [
            position[0],
            position[1] + template.radius + 0.5 * template.length,
            position[2],
        ];
        e.max_hit_points = template.max_hit_points;
        e.hit_points = template.max_hit_points;
        e.original_faction = template.faction;
        e.faction = template.faction;
        e.radius = template.radius;
        e.length = template.length;
        e.template_name = name.clone();
        e.undying = template.undying;
        let center = e.position;
        if let Some(actor) = name.and_then(|n| self.content.actor(&n)) {
            let d = &actor.definition;
            let mut body = magicka_sim::character_physics::CharacterBody::new(
                d.radius, d.length, d.mass, d.speed,
            );
            body.owner.turn_speed = d.turn_speed;
            body.owner.hit_points = d.max_hit_points;
            body.move_to(
                center,
                magicka_sim::character_physics::Orientation::IDENTITY,
            );
            let index = self.physics.bodies.len();
            self.physics.bodies.push(body);
            let mut character = crate::npc::NpcCharacter::new(npc, index, &actor);
            let model = self.choose_model(mesh, d.model_tints.len());
            character.character.set_model(model);
            self.npcs.insert(npc, character);
            // Agent.Enable.
            if !self.agents.contains(&npc) {
                self.agents.push(npc);
            }
        }
        Ok(template.max_hit_points)
    }
    fn force_draw(&mut self, _: u16) -> Result<(), WorldError> {
        Ok(())
    }
    fn configure(&mut self, npc: u16, c: &SpawnConfig<'_>) -> Result<(), WorldError> {
        self.character_dialogs.insert(npc, c.dialog);
        let e = self.entity_mut(npc)?;
        if let Some(hp) = c.hit_points {
            e.hit_points = hp;
        }
        e.orientation = c.orientation;
        e.order = Some(c.order.order);
        e.event_count = c.order.events.len();
        if let Some(v) = c.cannot_die_without_explicit_kill {
            e.cannot_die_without_explicit_kill = v;
        }
        self.live_set_order(npc, &c.order);
        if let Some(n) = self.npcs.get_mut(&npc) {
            n.on_death = c.on_death;
            n.on_damage = c.on_damage;
            if let Some(area) = c.target_area {
                n.ai.target_area = area;
            }
            if let Some(speed) = c.wander_speed {
                n.ai.wander_speed = speed;
            }
            if let Some(hp) = c.hit_points {
                n.character.combat.health.hit_points = hp;
            }
            if let Some(v) = c.cannot_die_without_explicit_kill {
                n.character.combat.cannot_die_without_explicit_kill = v;
            }
        }
        if let Some(n) = self.npcs.get(&npc) {
            let m = c.orientation;
            let orientation = magicka_sim::character_physics::Orientation {
                right: [m[0], m[1], m[2]],
                up: [m[4], m[5], m[6]],
                backward: [m[8], m[9], m[10]],
            };
            let body = &mut self.physics.bodies[n.body];
            body.move_to(body.position, orientation);
            body.set_desired_direction(orientation.forward());
        }
        Ok(())
    }
    fn force_animation(&mut self, npc: u16, animation: u16) -> Result<(), WorldError> {
        self.entity_mut(npc)?.animation = animation;
        if let Some(n) = self.npcs.get_mut(&npc) {
            let set = n.motion.animations.clone();
            n.motion.controller.force_animation(&set, animation);
        }
        Ok(())
    }
    fn begin_resurrection(&mut self, npc: u16, animation: u16) -> Result<(), WorldError> {
        self.entity_mut(npc)?.animation = animation;
        let n = self
            .npcs
            .get_mut(&npc)
            .ok_or_else(|| format!("NPC {npc} has no spawn-animation actor"))?;
        n.play_time = self.play_time;
        n.begin_resurrection(
            &mut self.physics.bodies[n.body],
            self.flags.cutscene,
            animation,
        );
        Ok(())
    }
    fn set_special_idle(&mut self, npc: u16, animation: u16) -> Result<(), WorldError> {
        let e = self.entity_mut(npc)?;
        e.special_idle = animation;
        e.animation = animation;
        if let Some(n) = self.npcs.get_mut(&npc) {
            n.motion.special_idle = animation;
        }
        Ok(())
    }
    fn in_resurrection_state(&mut self, npc: u16) -> Result<bool, WorldError> {
        self.entity(npc)?;
        Ok(self.npcs.get(&npc).is_some_and(|n| {
            n.motion.state == crate::character_motion::CharacterState::Resurrection
        }))
    }
    fn survival_ruleset(&self) -> bool {
        self.survival().is_some()
    }
    fn survival_added(&mut self, handle: u16) -> Result<(), WorldError> {
        // Spawn.Execute forces EVIL before AddedCharacter(npc, false).
        let entity = self.entity_mut(handle)?;
        entity.faction = 1;
        entity.original_faction = 1;
        let hp = self.entity(handle)?.hit_points;
        if let Some(r) = self.survival_mut() {
            r.added_character(handle, hp, false);
        }
        Ok(())
    }
    fn add_entity(&mut self, npc: u16) -> Result<(), WorldError> {
        self.entity_mut(npc)?.added = true;
        self.log.push(format!("spawn {npc}"));
        Ok(())
    }
    fn set_hit_points(&mut self, npc: u16, hp: f32) -> Result<(), WorldError> {
        self.entity_mut(npc)?.hit_points = hp;
        Ok(())
    }
    fn hit_points(&mut self, handle: u16) -> Result<Option<f32>, WorldError> {
        Ok(self
            .entities
            .iter()
            .find(|e| e.handle == handle && matches!(e.kind, EntityKind::Npc { .. }))
            .map(|e| e.hit_points))
    }
    fn send_spawn(&mut self, _: &SpawnMessage) -> Result<(), WorldError> {
        Ok(())
    }
}

impl<C: SceneContent> GiveOrderHost for SceneWorld<C> {
    type Error = WorldError;
    fn network_role(&self) -> NetworkRole {
        self.role
    }
    fn npc_by_id(&mut self, id: i32) -> Result<Option<u16>, WorldError> {
        Ok(self
            .by_unique_id(id)
            .filter(|e| matches!(e.kind, EntityKind::Npc { .. }))
            .map(|e| e.handle))
    }
    fn present_characters(&mut self, area: i32) -> Result<Option<Vec<OrderCandidate>>, WorldError> {
        let Some(indices) = self.area_characters(area) else {
            return Ok(None);
        };
        Ok(Some(
            indices
                .iter()
                .filter_map(|&i| {
                    let p = self.presence_entities.get(i)?;
                    let c = p.character?;
                    // Resolve by handle: ID-less spawns share unique IDs.
                    let e = self
                        .entities
                        .iter()
                        .find(|e| e.handle == p.handle && e.added && !e.removed)?;
                    Some(OrderCandidate {
                        handle: e.handle,
                        npc: true,
                        type_id: c.type_id,
                        original_faction: c.original_faction,
                    })
                })
                .collect(),
        ))
    }
    fn update_present(&mut self, area: i32) -> Result<bool, WorldError> {
        self.refresh_presence();
        if self.presence.get(area).is_err() {
            return Ok(false);
        }
        self.presence
            .update_present_area(area, &self.presence_entities)
            .map_err(|e| format!("explicit area presence: {e:?}"))?;
        Ok(true)
    }
    fn release_attached(&mut self, _: u16) -> Result<(), WorldError> {
        Ok(())
    }
    fn is_gripped(&mut self, _: u16) -> Result<bool, WorldError> {
        Ok(false)
    }
    fn release_gripper_attached(&mut self, _: u16) -> Result<(), WorldError> {
        Ok(())
    }
    fn apply_order(&mut self, npc: u16, c: &OrderCommand<'_>) -> Result<(), WorldError> {
        let e = self.entity_mut(npc)?;
        e.order = Some(c.order.order);
        e.event_count = c.order.events.len();
        self.live_set_order(npc, &c.order);
        if let Some(n) = self.npcs.get_mut(&npc) {
            n.ai.target_area = c.target_area;
            n.ai.wander_speed = c.wander_speed;
        }
        Ok(())
    }
    fn set_special_idle(&mut self, npc: u16, animation: u16) -> Result<(), WorldError> {
        let e = self.entity_mut(npc)?;
        e.special_idle = animation;
        e.animation = animation;
        if let Some(n) = self.npcs.get_mut(&npc) {
            n.motion.special_idle = animation;
        }
        Ok(())
    }
    fn go_to_animation(&mut self, npc: u16, animation: u16, blend: f32) -> Result<(), WorldError> {
        self.entity_mut(npc)?.animation = animation;
        if let Some(n) = self.npcs.get_mut(&npc) {
            n.motion.go_to_animation(animation, blend);
        }
        Ok(())
    }
    fn order_panic(&mut self, npc: u16) -> Result<(), WorldError> {
        if let Some(n) = self.npcs.get_mut(&npc) {
            n.order_panic();
        }
        Ok(())
    }
    fn send_event_animation(&mut self, _: u16, _: u16, _: f32, _: u16) -> Result<(), WorldError> {
        Ok(())
    }
}

impl<C: SceneContent> SceneCommandHost for SceneWorld<C> {
    type Error = WorldError;
    fn begin_cutscene(
        &mut self,
        on_skip: i32,
        skip_bar_move: bool,
        kill_dialogs: bool,
    ) -> Result<(), WorldError> {
        if kill_dialogs {
            self.end_all_dialogs()?;
        }
        self.flags.cutscene = true;
        for player in self.players.iter_mut().flatten() {
            player.sync_immortality(true);
        }
        // PlayState.BeginCutscene: UIEnabled = false.
        self.ui_enabled = false;
        self.cutscene.begin(on_skip, skip_bar_move);
        self.log.push("cutscene begin".into());
        Ok(())
    }
    fn end_cutscene(&mut self, _: bool) -> Result<(), WorldError> {
        self.end_all_dialogs()?;
        self.flags.cutscene = false;
        for player in self.players.iter_mut().flatten() {
            player.sync_immortality(false);
        }
        // PlayState.EndCutscene: UIEnabled = true.
        self.ui_enabled = true;
        self.cutscene.end();
        self.log.push("cutscene end".into());
        Ok(())
    }
    fn limit_input(&mut self, limited: bool) -> Result<(), WorldError> {
        self.flags.input_limited = limited;
        Ok(())
    }
    fn fade(&mut self, fade_in: bool, color: [u8; 4], time: f32) -> Result<(), WorldError> {
        // Fade.Execute: FadeIn → BeginTransition (Out), else EndTransition.
        self.render_transition =
            crate::render_transition::RenderTransition::start(1, fade_in, color, time);
        Ok(())
    }
    fn clear_transition(&mut self) -> Result<(), WorldError> {
        self.transition_cleared = true;
        Ok(())
    }
    fn camera_release(&mut self, time: f32) -> Result<(), WorldError> {
        self.camera.release(time);
        Ok(())
    }
    /// GameScene.TryGetLocator: static locators, then animated parts.
    fn try_locator(&mut self, id: i32) -> Result<Option<[f32; 16]>, WorldError> {
        Ok(self.locators.get(&id).copied().or_else(|| {
            crate::level_effects::part_locator(
                &self.level.animated_parts,
                self.collision.parts(),
                id,
            )
        }))
    }
    fn camera_ground_position(&mut self) -> Result<[f32; 3], WorldError> {
        Ok(self.camera.ground_position())
    }
    fn camera_move_to(&mut self, target: [f32; 3], time: f32) -> Result<(), WorldError> {
        self.camera.move_to(target, time);
        Ok(())
    }
    fn set_camera_interpolation(&mut self, value: CameraInterpolation) -> Result<(), WorldError> {
        self.camera.interpolation = value;
        Ok(())
    }
    fn set_camera_magnification(&mut self, value: f32) -> Result<(), WorldError> {
        self.camera.set_magnification(value);
        Ok(())
    }
    fn set_camera_time(&mut self, value: f32) -> Result<(), WorldError> {
        self.camera.set_time(value);
        Ok(())
    }
    fn set_camera_lock_input(&mut self, value: bool) -> Result<(), WorldError> {
        self.camera.lock_input = value;
        Ok(())
    }
    fn locator(&mut self, id: i32) -> Result<[f32; 16], WorldError> {
        self.scene_locator(id)
    }
    fn camera_shake(
        &mut self,
        _: Option<[f32; 3]>,
        magnitude: f32,
        time: f32,
    ) -> Result<(), WorldError> {
        self.camera.shake(magnitude, time);
        Ok(())
    }
    fn camera_lock_on(&mut self, area: i32, time: f32) -> Result<bool, WorldError> {
        let Some(found) = self
            .level
            .trigger_areas
            .iter()
            .find(|a| custom_hash(&a.name.to_ascii_lowercase()) == area)
        else {
            return Ok(false);
        };
        self.camera.lock_on(CameraBox::from_area(found), time);
        Ok(true)
    }
}

impl<C: SceneContent> AudioActionHost for SceneWorld<C> {
    type Error = WorldError;
    fn play_global_ambient(
        &mut self,
        id: i32,
        bank: u16,
        cue: i32,
        volume: f32,
    ) -> Result<(), WorldError> {
        let mut sounds = std::mem::take(&mut self.sounds);
        let result = sounds.play_global(&mut self.backend(), id, bank, cue, volume);
        self.sounds = sounds;
        result
    }
    fn play_located_ambient(&mut self, id: i32, sound: LocatedSound) -> Result<(), WorldError> {
        let mut sounds = std::mem::take(&mut self.sounds);
        let result = sounds.play_located(&mut self.backend(), id, sound);
        self.sounds = sounds;
        result
    }
    fn play_cue(&mut self, bank: u16, cue: i32, volume: f32) -> Result<bool, WorldError> {
        if !self.content.cue_exists(cue) {
            return Ok(false);
        }
        let key = self.backend().next_key();
        self.audio_requests.push(crate::audio::AudioRequest::Start {
            key,
            bank,
            cue,
            volume_db: volume,
            source: crate::audio::SoundSource::Global,
            magnitude: None,
        });
        Ok(true)
    }
    fn play_unregistered_locator(&mut self, s: LocatedSound) -> Result<(), WorldError> {
        let key = self.backend().next_key();
        self.audio_requests.push(crate::audio::AudioRequest::Start {
            key,
            bank: s.bank,
            cue: s.cue,
            volume_db: s.volume,
            source: crate::audio::SoundSource::Locator {
                area: s.area,
                radius: s.radius,
                apply_3d: s.apply_3d,
            },
            magnitude: None,
        });
        Ok(())
    }
    fn stop_sound(&mut self, id: i32, instant: bool) -> Result<(), WorldError> {
        let mut sounds = std::mem::take(&mut self.sounds);
        let result = sounds.stop(&mut self.backend(), id, instant);
        self.sounds = sounds;
        result
    }
    fn play_music(&mut self, bank: u16, cue: i32, focus: Option<f32>) -> Result<(), WorldError> {
        self.flags.music = self.content.cue_exists(cue).then_some((bank, cue));
        self.flags.music_focus_value = focus.filter(|_| self.flags.music.is_some());
        if self.flags.music.is_some() {
            self.audio_requests
                .push(crate::audio::AudioRequest::Music { bank, cue });
        }
        Ok(())
    }
    fn stop_music(&mut self) -> Result<(), WorldError> {
        self.flags.music = None;
        self.flags.music_focus_value = None;
        self.audio_requests
            .push(crate::audio::AudioRequest::StopMusic);
        Ok(())
    }
}

impl<C: SceneContent> EffectActionHost for SceneWorld<C> {
    type Error = WorldError;
    fn locator(&mut self, area: i32) -> Result<[f32; 16], WorldError> {
        self.scene_locator(area)
    }
    fn effect_is_single(&mut self, effect: i32) -> Result<bool, WorldError> {
        self.effect_types
            .get(&effect)
            .map(|kind| *kind == crate::particles::VisualEffectType::Single)
            .ok_or_else(|| format!("VisualEffect.Type unavailable: {effect}"))
    }
    fn add_effect(
        &mut self,
        id: i32,
        effect: i32,
        t: [f32; 16],
        range: f32,
    ) -> Result<(), WorldError> {
        let mut effects = std::mem::take(&mut self.effects);
        let result = effects.add(&mut self.backend(), id, effect, t, range);
        self.effects = effects;
        result
    }
    fn stop_effect(&mut self, id: i32) -> Result<bool, WorldError> {
        let mut effects = std::mem::take(&mut self.effects);
        let result = effects.stop(&mut self.backend(), id);
        self.effects = effects;
        result
    }
    fn start_effect(&mut self, id: i32) -> Result<(), WorldError> {
        let mut effects = std::mem::take(&mut self.effects);
        let result = effects.start(&mut self.backend(), id);
        self.effects = effects;
        result
    }
    fn generated_effect_id(&mut self) -> Result<i32, WorldError> {
        Ok(self.effects.generated_id())
    }
}

impl<C: SceneContent> EntityCommandHost for SceneWorld<C> {
    type Error = WorldError;
    fn playing_avatar(&mut self, slot: usize) -> Result<Option<EntityFacts>, WorldError> {
        Ok(self
            .players
            .get(slot)
            .and_then(Option::as_ref)
            .filter(|p| !p.dead())
            .map(|p| EntityFacts {
                handle: p.handle,
                character: Some(self.physics.bodies[p.body].height_offset()),
            }))
    }
    fn entity_by_id(&mut self, id: i32) -> Result<Option<EntityFacts>, WorldError> {
        Ok(self.by_unique_id(id).map(|e| EntityFacts {
            handle: e.handle,
            character: self
                .npcs
                .get(&e.handle)
                .map(|n| self.physics.bodies[n.body].height_offset()),
        }))
    }
    fn locator(&mut self, area: i32) -> Result<[f32; 16], WorldError> {
        self.scene_locator(area)
    }
    fn level_segment(
        &mut self,
        origin: [f32; 3],
        delta: [f32; 3],
    ) -> Result<Option<[f32; 3]>, WorldError> {
        Ok(self
            .collision
            .collision()
            .static_skin
            .segment_intersect(Segment { origin, delta }, false)
            .map(|h| h.hit.position))
    }
    fn body_orientation(&mut self, handle: u16) -> Result<[f32; 16], WorldError> {
        if let Some(b) = self
            .avatar_body(handle)
            .or_else(|| self.npcs.get(&handle).map(|n| n.body))
        {
            let o = self.physics.bodies[b].orientation;
            let mut m = magicka_content::skinned::IDENTITY;
            m[0..3].copy_from_slice(&o.right);
            m[4..7].copy_from_slice(&o.up);
            m[8..11].copy_from_slice(&o.backward);
            return Ok(m);
        }
        Ok(self.entity(handle)?.orientation)
    }
    fn set_desired_direction(
        &mut self,
        handle: u16,
        direction: [f32; 3],
    ) -> Result<(), WorldError> {
        if let Some(b) = self
            .avatar_body(handle)
            .or_else(|| self.npcs.get(&handle).map(|n| n.body))
        {
            self.physics.bodies[b].set_desired_direction(direction);
        }
        Ok(())
    }
    fn move_body(
        &mut self,
        handle: u16,
        position: [f32; 3],
        orientation: [f32; 16],
    ) -> Result<(), WorldError> {
        if let Some(slot) = self.elemental_eggs.as_mut().and_then(|cache| {
            cache
                .slots
                .iter_mut()
                .find(|s| s.handle == handle && s.actor.registered)
        }) {
            slot.actor.position = position;
            slot.actor.orientation = orientation;
            slot.actor.velocity = [0.; 3];
        }
        if let Some(b) = self
            .avatar_body(handle)
            .or_else(|| self.npcs.get(&handle).map(|n| n.body))
        {
            let o = magicka_sim::character_physics::Orientation {
                right: [orientation[0], orientation[1], orientation[2]],
                up: [orientation[4], orientation[5], orientation[6]],
                backward: [orientation[8], orientation[9], orientation[10]],
            };
            self.physics.bodies[b].move_to(position, o);
        }
        let e = self.entity_mut(handle)?;
        e.position = position;
        e.orientation = orientation;
        Ok(())
    }
    fn present_characters(&mut self, area: i32) -> Result<Option<Vec<AreaCharacter>>, WorldError> {
        Ok(GiveOrderHost::present_characters(self, area)?.map(|v| {
            v.into_iter()
                .map(|c| AreaCharacter {
                    handle: c.handle,
                    type_id: c.type_id,
                    original_faction: c.original_faction,
                })
                .collect()
        }))
    }
    fn clear_cannot_die(&mut self, handle: u16) -> Result<(), WorldError> {
        self.entity_mut(handle)?.cannot_die_without_explicit_kill = false;
        Ok(())
    }
    fn kill(&mut self, handle: u16) -> Result<(), WorldError> {
        if let Some(d) = self.dispensers.get_mut(&handle) {
            d.health = 0.0;
            let e = self.entity_mut(handle)?;
            e.hit_points = 0.0;
            e.dead = true;
            return Ok(());
        }
        if let Some(slot) = self.elemental_eggs.as_mut().and_then(|cache| {
            cache
                .slots
                .iter_mut()
                .find(|s| s.handle == handle && s.actor.registered)
        }) {
            slot.actor.state.kill();
            if let Ok(entity) = self.entity_mut(handle) {
                entity.dead = true;
            }
            return Ok(());
        }
        if let Some(state) = self.physics_damage.get_mut(&handle) {
            state.health = 0.;
            if state.on_death != 0 && self.role != NetworkRole::Client {
                self.pending_triggers.push((state.on_death, None));
            }
        }
        if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == handle)
        {
            p.kill();
        }
        if let Some(n) = self.npcs.get_mut(&handle) {
            // Character.Kill: hit points to zero (unless CannotDieWithout-
            // ExplicitKill); DeadState and Die() follow in Character.Update.
            if !n.character.combat.cannot_die_without_explicit_kill {
                n.character.combat.health.hit_points = 0.0;
            }
            self.log.push(format!("kill {handle}"));
            return Ok(());
        }
        let e = self.entity_mut(handle)?;
        // Static NPC records (no live character): marked dead with zero health.
        e.dead = true;
        e.hit_points = 0.0;
        self.disable_npc(handle, false);
        self.log.push(format!("kill {handle}"));
        Ok(())
    }
    fn terminate(&mut self, handle: u16) -> Result<(), WorldError> {
        self.entity_mut(handle)?.removed = true;
        self.disable_npc(handle, true);
        Ok(())
    }
    fn set_dialog(&mut self, handle: u16, dialog: i32) -> Result<(), WorldError> {
        self.character_dialogs.insert(handle, dialog);
        Ok(())
    }
    fn release_npc_grip(&mut self, handle: u16) -> Result<(), WorldError> {
        let Some(gripper) = self.npcs.get(&handle).map(|n| n.character.grip.gripper) else {
            return Ok(());
        };
        self.release_grip(handle);
        if let Some(gripper) = gripper {
            self.release_grip(gripper);
        }
        Ok(())
    }
}

impl<C: SceneContent> SpawnPhysHost for SceneWorld<C> {
    type Error = WorldError;
    fn is_hot_join_client(&self) -> bool {
        false
    }
    fn return_damageable_to_cache(&mut self, _: u16) -> Result<(), WorldError> {
        Ok(())
    }
    fn locator(&mut self, area: i32) -> Result<[f32; 16], WorldError> {
        self.scene_locator(area)
    }
    fn initialize_entity(
        &mut self,
        handle: u16,
        object: Option<&str>,
        t: [f32; 16],
        unique_id: i32,
    ) -> Result<(), WorldError> {
        let max = object
            .and_then(|o| self.content.physics_max_hit_points(o))
            .unwrap_or(0) as f32;
        self.physics_damage.remove(&handle);
        self.static_prop_boxes.remove(&handle);
        self.anim_props.remove(&handle);
        self.attached_props.remove(&handle);
        let mut radius = 0.0;
        if let Some(template) = object.and_then(|o| self.content.physics_template(o)) {
            // AnimatedPhysicsEntity: its own radius box, Immovable = !Pushable.
            let anim_box = crate::anim_prop::collision_box(&template);
            let immovable = if anim_box.is_some() {
                !template.pushable
            } else {
                !template.movable
            };
            if template.solid && immovable {
                if let Some(b) = anim_box.as_ref().or(template.boxes.first()) {
                    self.static_prop_boxes.insert(handle, b.clone());
                }
            }
            if max > 0. {
                let mut state = crate::physics_damage::PhysicsDamage::from_template(&template);
                if anim_box.is_some() {
                    state.collision_box = anim_box.clone();
                }
                radius = state.radius();
                self.physics_damage.insert(handle, state);
            }
            if anim_box.is_some() {
                radius = crate::anim_prop::radius(&template);
            }
        }
        *self.physics_generations.entry(handle).or_default() += 1;
        let e = self.entity_mut(handle)?;
        e.template_name = object.map(str::to_owned);
        e.position = [t[12], t[13], t[14]];
        e.orientation = t;
        e.radius = radius;
        e.unique_id = unique_id;
        e.max_hit_points = max;
        e.hit_points = max;
        e.dead = false;
        e.removed = false;
        Ok(())
    }
    fn set_damage_triggers(
        &mut self,
        handle: u16,
        on_death: i32,
        on_damage: i32,
    ) -> Result<(), WorldError> {
        if let Some(state) = self.physics_damage.get_mut(&handle) {
            state.on_death = on_death;
            state.on_damage = on_damage;
        }
        Ok(())
    }
    fn animate_entity(
        &mut self,
        handle: u16,
        anim: &crate::spawn_phys::AnimProp,
    ) -> Result<(), WorldError> {
        let name = self
            .entity(handle)?
            .template_name
            .clone()
            .ok_or("animated prop without a template")?;
        let template = self
            .content
            .physics_template(&name)
            .ok_or_else(|| format!("missing physics template {name}"))?;
        // Initialize: a massive prop under gravity drops onto the ground
        // found within 10 m below (origin + 1), resting 0.1 above it.
        if template.mass > 0. && anim.gravity {
            let p = self.entity(handle)?.position;
            let ground = Segment {
                origin: [p[0], p[1] + 1., p[2]],
                delta: [0., -10., 0.],
            };
            if let Some(hit) = self.collision.collision().segment_intersect(ground, false) {
                let mut p = hit.hit.position;
                p[1] += 0.1;
                let e = self.entity_mut(handle)?;
                e.position = p;
                e.orientation[12..15].copy_from_slice(&p);
            }
        }
        if let Some(state) = self.physics_damage.get_mut(&handle) {
            state.deflects = (!anim.damageable).then_some(template.pushable);
        }
        let content = &mut self.content;
        let prop = crate::anim_prop::AnimProp::force(
            &template,
            anim.animation.as_deref(),
            anim.damageable,
            anim.gravity,
            |skeleton, clip| content.clip_duration(&crate::anim_prop::content_path(skeleton), clip),
        );
        self.anim_props.insert(handle, prop);
        Ok(())
    }
    fn add_entity(&mut self, handle: u16) -> Result<(), WorldError> {
        self.entity_mut(handle)?.added = true;
        self.log.push(format!("spawnphys {handle}"));
        self.attach_static_prop(handle);
        Ok(())
    }
    fn hit_points(&mut self, handle: u16) -> Result<f32, WorldError> {
        Ok(self.entity(handle)?.hit_points)
    }
}

impl<C: SceneContent> AvatarMoveHost for SceneWorld<C> {
    type Error = WorldError;
    fn player_avatars(&self) -> Vec<Option<u16>> {
        self.players
            .iter()
            .map(|p| p.as_ref().map(|p| p.handle))
            .collect()
    }
    fn set_avatar_events(&mut self, player: usize, events: &[AiEvent]) -> Result<(), WorldError> {
        if let Some(p) = self.players.get_mut(player).and_then(Option::as_mut) {
            let body = &mut self.physics.bodies[p.body];
            p.set_events(body, Some(events.to_vec()));
        }
        Ok(())
    }
}

impl<C: SceneContent> ToggleHost for SceneWorld<C> {
    type Error = WorldError;
    fn set_light(&mut self, id: i32, enable: bool, _: i32, _: f32) -> Result<bool, WorldError> {
        if !self.level.lights.iter().any(|l| custom_hash(&l.name) == id) {
            return Ok(false);
        }
        if enable {
            self.flags.lights_disabled.remove(&id);
        } else {
            self.flags.lights_disabled.insert(id);
        }
        Ok(true)
    }
    fn set_interactable_enabled(&mut self, id: i32, enabled: bool) -> Result<bool, WorldError> {
        if !self.interactables.contains(&id) {
            return Ok(false);
        }
        if enabled {
            self.flags.interactables_disabled.remove(&id);
        } else {
            self.flags.interactables_disabled.insert(id);
        }
        Ok(true)
    }
    fn set_tutorial_element(&mut self, elements: i32, enabled: bool) -> Result<(), WorldError> {
        if enabled {
            self.flags.tutorial_elements_disabled &= !elements;
        } else {
            self.flags.tutorial_elements_disabled |= elements;
        }
        Ok(())
    }
    fn set_tutorial_cast_type(&mut self, cast: i32, enabled: bool) -> Result<(), WorldError> {
        if enabled {
            self.flags.tutorial_cast_types_disabled.remove(&cast);
        } else {
            self.flags.tutorial_cast_types_disabled.insert(cast);
        }
        Ok(())
    }
    fn set_tutorial_push(&mut self, enabled: bool) -> Result<(), WorldError> {
        self.flags.tutorial_push_disabled = !enabled;
        Ok(())
    }
    fn unlock_magick(&mut self, magick: u8) -> Result<(), WorldError> {
        if let Some(text) = self.dialog_text.as_mut() {
            use crate::text_box_initialize::{
                TextBoxFonts, TextBoxInit, TextBoxLocation, TextBoxWidget,
            };
            let key = magicka_sim::spells::MagickType::try_from(magick)
                .ok()
                .and_then(crate::tome_magick::name_key);
            let name = key
                .as_deref()
                .and_then(|key| text.strings.try_get_string(custom_hash(key)).value)
                .unwrap_or("???");
            let message = text
                .strings
                .try_get_string(custom_hash("#magick_pick_up"))
                .value
                .unwrap_or("#1;")
                .replace("#1;", &format!("[c=1,1,1]{name}[/c]"));
            let mut widget = TextBoxWidget::default();
            widget
                .initialize(
                    TextBoxInit {
                        scene: text.scene,
                        location: TextBoxLocation::Screen {
                            position: [0.; 2],
                            name: 0,
                        },
                        text: Some(&message),
                        minimum_size: [0.; 2],
                        force_on_screen: true,
                        ttl: 2.,
                    },
                    &text.strings,
                    TextBoxFonts {
                        body: &text.body_font,
                        owner_name: &text.name_font,
                    },
                    &mut text.world,
                    text.substitution_budget,
                )
                .map_err(|e| format!("unlock textbox: {e:?}"))?;
            text.additional.push(widget);
        }
        self.pending_magick_unlocks.push(magick);
        Ok(())
    }
    fn remove_hint(&mut self) -> Result<(), WorldError> {
        self.tutorial.remove_hint();
        Ok(())
    }
    fn award_achievement(&mut self, name: Option<&str>) -> Result<(), WorldError> {
        if let Some(name) = name {
            self.flags.achievements.insert(name.into());
        }
        Ok(())
    }
}

impl<C: SceneContent> crate::scene_flow::CheckpointLevel for SceneWorld<C> {
    fn scene_locator(&mut self, id: i32) -> Result<[f32; 16], WorldError> {
        SceneWorld::scene_locator(self, id)
    }
}

impl<C: SceneContent> crate::scene_flow::SceneFlowHost for SceneWorld<C> {
    type Error = WorldError;
    fn end_game(&mut self, condition: u8, freeze: bool, phony: bool) -> Result<(), WorldError> {
        self.pending_end_game = Some((condition, freeze, phony));
        Ok(())
    }
    fn network_role(&self) -> NetworkRole {
        self.role
    }
    fn send_scene_change(
        &mut self,
        _: bool,
        _: crate::scene_flow::SpawnPoint,
    ) -> Result<(), WorldError> {
        Ok(())
    }
    fn go_to_scene(
        &mut self,
        spawn: crate::scene_flow::SpawnPoint,
        transition: i32,
        time: f32,
        save_npcs: bool,
        callback: Option<crate::scene_flow::EnterSceneCallback>,
    ) -> Result<(), WorldError> {
        self.log.push(format!("go to scene {}", spawn.scene));
        self.pending_scene = Some(crate::scene_flow::PendingScene {
            spawn,
            transition,
            time,
            save_npcs,
            callback,
        });
        Ok(())
    }
    fn locator(&mut self, id: i32) -> Result<[f32; 16], WorldError> {
        self.scene_locator(id)
    }
    fn show_checkpoint_text(&mut self) -> Result<(), WorldError> {
        // LanguageManager.GetString(CHECKPOINT); without a string table the
        // key itself stands in (the original has no such path).
        let key = custom_hash("#add_checkpoint");
        let text = self
            .dialog_text
            .as_ref()
            .and_then(|host| host.strings.get_string(key))
            .unwrap_or_else(|| "#add_checkpoint".to_owned());
        self.checkpoint_notices.push(CheckpointNotice {
            text,
            font: "Maiandra18",
            position: [0.0, 0.0],
            anchor: [0.0, 1.0],
            force_on_screen: true,
            ttl: 2.0,
        });
        Ok(())
    }
    fn players(&self) -> Vec<crate::scene_flow::FlowPlayer> {
        (0..self.players.len().max(4))
            .map(|slot| {
                let Some(player) = self.players.get(slot).and_then(Option::as_ref) else {
                    return crate::scene_flow::FlowPlayer::default();
                };
                if !player.playing {
                    return crate::scene_flow::FlowPlayer::default();
                }
                crate::scene_flow::FlowPlayer {
                    playing: true,
                    avatar: Some(crate::scene_flow::FlowAvatar {
                        dead: player.dead(),
                        staff_of_war: player.items.iter().any(|(slot, item)| {
                            *slot == 1
                                && custom_hash(&item.definition.name) == custom_hash("staff_of_war")
                        }),
                    }),
                }
            })
            .collect()
    }
    fn single_player_non_versus(&self) -> bool {
        self.single_player_non_versus_game()
    }
    fn initialize_revival_fairy(&mut self, slot: usize) -> Result<(), WorldError> {
        self.initialize_revival_fairy_slot(slot, true);
        Ok(())
    }
    fn update_checkpoint(
        &mut self,
        matrices: &[[f32; 16]; 4],
        ignore: &[i32],
        _: bool,
    ) -> Result<(), WorldError> {
        self.checkpoints.push(*matrices);
        // State.UpdateState captures each avatar's active fairy.
        self.sync_spawn_fairies_from_active();
        self.pending_checkpoint = Some((*matrices, ignore.to_vec()));
        self.log.push("checkpoint".into());
        Ok(())
    }
    fn play_ui_cue(&mut self, cue: i32) -> Result<(), WorldError> {
        let key = self.backend().next_key();
        self.audio_requests.push(crate::audio::AudioRequest::Start {
            key,
            bank: crate::audio::banks::UI,
            cue,
            volume_db: 0.0,
            source: crate::audio::SoundSource::Global,
            magnitude: None,
        });
        Ok(())
    }
    fn revive_player(&mut self, slot: usize, point: [f32; 3]) -> Result<(), WorldError> {
        self.revive_specific(slot, point);
        Ok(())
    }
    fn heal_player(&mut self, slot: usize, multiplier: f32) -> Result<(), WorldError> {
        let player = self
            .players
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or_else(|| "checkpoint player absent".to_string())?;
        let health = player.character.combat.health.max_hit_points * multiplier;
        player.character.combat.health.hit_points = health;
        self.physics.bodies[player.body].owner.hit_points = health;
        Ok(())
    }
    fn found_secret_area(&mut self, id: Option<&str>) -> Result<(), WorldError> {
        self.secret_areas.push(id.map(Into::into));
        Ok(())
    }
}

impl<C: SceneContent> WorldConditionFacts for SceneWorld<C> {
    fn character_health(&self, id: i32) -> Option<CharacterHealthFacts> {
        self.by_unique_id(id)
            .filter(|e| matches!(e.kind, EntityKind::Npc { .. }))
            .map(|e| CharacterHealthFacts {
                dead: e.dead,
                hit_points: e.hit_points,
                max_hit_points: e.max_hit_points,
            })
    }
    fn current_spell(&self, id: i32) -> Option<(i32, Spell)> {
        let entity = self.by_unique_id(id)?;
        let EntityKind::Avatar { slot } = entity.kind else {
            return None;
        };
        let player = self.players.get(slot)?.as_ref()?;
        if !player.playing {
            return None;
        }
        player
            .casting_spell()
            .map(|(cast_type, spell)| (cast_type as i32, spell))
    }
    fn sender_unique_id(&self, sender: u16) -> Option<i32> {
        self.entity(sender).ok().map(|e| e.unique_id)
    }
    fn players(&self) -> Vec<ConditionPlayer> {
        self.players
            .iter()
            .map(|p| match p {
                Some(p) if p.playing => ConditionPlayer {
                    playing: true,
                    avatar: Some(crate::world_condition::ConditionAvatar {
                        template_id: p.template_type,
                        statuses: i32::from(p.character.combat.statuses.current_mask),
                        // Avatar.SpellQueue element bits (PlayerHasElement).
                        queue: p
                            .conjure
                            .queue
                            .iter()
                            .map(|spell| spell.element.0)
                            .collect(),
                    }),
                },
                _ => ConditionPlayer::default(),
            })
            .collect()
    }
    fn boss_fight(&self) -> Option<BossFacts> {
        self.boss_fight.facts()
    }
    fn died_in_level(&self) -> bool {
        self.died_in_level_flag
    }
    fn has_achievement(&self, name: Option<&str>) -> bool {
        name.is_some_and(|n| self.flags.achievements.contains(n))
    }
    /// Helper.CheckDLC asks Steam about the DLC app; the rewrite has no
    /// Steam and treats every DLC as owned (user decision).
    fn has_dlc(&self, _: i32) -> bool {
        true
    }
    fn campaign_looped(&self) -> Option<bool> {
        self.campaign_looped
    }
}

impl<C: SceneContent> ScriptWorld for SceneWorld<C> {
    fn dispenser_action(
        &mut self,
        action: &crate::dispenser::DispenserAction,
    ) -> Option<Result<(), WorldError>> {
        Some(self.live_dispenser_action(action))
    }
    fn damage_action(
        &mut self,
        action: &crate::script_damage::ScriptDamageAction,
    ) -> Option<Result<(), WorldError>> {
        Some((|| {
            use crate::entity_command::CharacterSelector;
            use magicka_sim::damage_delivery::DamagePayload;
            let position = if action.source == 0 {
                [0.; 3]
            } else {
                let locator = self.scene_locator(action.source)?;
                [locator[12], locator[13], locator[14]]
            };
            let (targets, area) = match action.target {
                CharacterSelector::Id(id) => {
                    let player = self.players.iter().enumerate().find_map(|(slot, player)| {
                        let player = player.as_ref()?;
                        (player.playing && id == custom_hash(&format!("player{}", slot + 1)))
                            .then_some(player.handle)
                    });
                    let target = player.or_else(|| self.by_unique_id(id).map(|e| e.handle));
                    (target.into_iter().collect::<Vec<_>>(), false)
                }
                CharacterSelector::Area {
                    area,
                    type_id,
                    factions,
                } => {
                    let targets = EntityCommandHost::present_characters(self, area)?
                        .ok_or_else(|| format!("damage: missing area {area}"))?
                        .into_iter()
                        .filter(|e| {
                            type_id == custom_hash("any")
                                || e.type_id == type_id
                                || e.original_faction & factions != 0
                        })
                        .map(|e| e.handle)
                        .collect();
                    (targets, true)
                }
            };
            for target in targets {
                let body = self.npcs.get(&target).map(|npc| npc.body).or_else(|| {
                    self.players
                        .iter()
                        .flatten()
                        .find(|p| p.handle == target)
                        .map(|p| p.body)
                });
                let Some(body) = body else {
                    if let Some(d) = self.dispensers.get_mut(&target) {
                        for &entry in &action.damages {
                            d.damage(entry, self.role != NetworkRole::Client);
                        }
                        let hp = d.health;
                        let e = self.entity_mut(target)?;
                        e.hit_points = hp;
                        e.dead = hp <= 0.0;
                    }
                    if self.entities.iter().any(|e| {
                        e.handle == target
                            && matches!(e.kind, EntityKind::Physics { damageable: true })
                    }) {
                        for &damage in &action.damages {
                            self.damage_physics(target, damage, 15)?;
                        }
                    }
                    continue;
                };
                let target_position = self.physics.bodies[body].position;
                // IgnoreShields is only consulted by the native area branch.
                if area
                    && !action.ignore_shields
                    && self.shields.iter().any(|s| {
                        if s.shield.dead {
                            return false;
                        }
                        if action.source != 0 {
                            s.shield
                                .geometry()
                                .segment_fraction(Segment {
                                    origin: position,
                                    delta: std::array::from_fn(|i| {
                                        target_position[i] - position[i]
                                    }),
                                })
                                .is_some()
                        } else {
                            matches!(
                                s.shield.kind,
                                magicka_sim::shield_entity::ShieldKind::Sphere
                            ) && (0..3)
                                .map(|i| (target_position[i] - s.shield.position[i]).powi(2))
                                .sum::<f32>()
                                < s.shield.radius.powi(2)
                        }
                    })
                {
                    continue;
                }
                for &damage in &action.damages {
                    if let Some(player) = self
                        .players
                        .iter_mut()
                        .flatten()
                        .find(|p| p.handle == target)
                    {
                        player.receive_payload(
                            &mut self.physics.bodies[body],
                            DamagePayload::Single(damage),
                            None,
                            position,
                            0.,
                        );
                    } else if let Some(npc) = self.npcs.get_mut(&target) {
                        let result = crate::avatar::deliver_to_body(
                            &mut npc.character,
                            &mut self.physics.bodies[body],
                            &[magicka_sim::resistance::Resistance::IDENTITY; 11],
                            false,
                            target,
                            DamagePayload::Single(damage),
                            None,
                            position,
                            0.,
                        );
                        if result & magicka_sim::character_impact::result::DAMAGED != 0
                            && npc.on_damage != 0
                        {
                            self.pending_triggers.push((npc.on_damage, Some(target)));
                        }
                    }
                }
            }
            Ok(())
        })())
    }
    type Error = WorldError;
    fn dialog_action(
        &mut self,
        _runtime: &mut ScriptRuntime,
        action: &crate::dialog_action::DialogAction,
        sender: Option<u16>,
    ) -> Option<Result<(), WorldError>> {
        self.world_dialog_action(action, sender)
    }
    fn facts(&self) -> ScriptWorldFacts {
        ScriptWorldFacts {
            cutscene: self.flags.cutscene,
            player_count: self.players.iter().flatten().filter(|p| p.playing).count() as i32,
        }
    }
    fn spawn_host(&mut self) -> Option<&mut dyn SpawnHost<Error = WorldError>> {
        Some(self)
    }
    fn spawn_elemental_host(
        &mut self,
    ) -> Option<&mut dyn crate::spawn_elemental::SpawnElementalHost<Error = WorldError>> {
        Some(self)
    }
    fn give_order_host(&mut self) -> Option<&mut dyn GiveOrderHost<Error = WorldError>> {
        Some(self)
    }
    fn scene_command_host(&mut self) -> Option<&mut dyn SceneCommandHost<Error = WorldError>> {
        Some(self)
    }
    fn tutorial_done(&self) -> Option<bool> {
        Some(self.tutorial.dialog_done())
    }
    fn campaign_action(
        &mut self,
        action: &crate::campaign_action::CampaignAction,
        quick: bool,
    ) -> Option<Result<(), WorldError>> {
        Some(self.campaign(action, quick))
    }
    fn boss_action(
        &mut self,
        action: &crate::boss_action::BossAction,
        quick: bool,
    ) -> Option<Result<(), WorldError>> {
        Some(self.boss(action, quick))
    }
    fn tutorial_action(
        &mut self,
        action: &crate::tutorial::TutorialAction,
    ) -> Option<Result<(), WorldError>> {
        Some((|| {
            use crate::tutorial::{DialogHint, Hint, TutorialCommand};
            if matches!(action.command, TutorialCommand::RemoveDialog) {
                if let Some(done) = self.tutorial.dismiss_dialog() {
                    if let Some(trigger) = done.trigger {
                        self.pending_triggers.push((trigger, None));
                    }
                }
                return Ok(());
            }
            if let TutorialCommand::Dark(on) = action.command {
                if on {
                    self.tutorial.enable_dark();
                } else {
                    self.tutorial.disable_dark();
                }
                return Ok(());
            }
            let host = self
                .dialog_text
                .as_ref()
                .ok_or("tutorial localization unavailable")?;
            let localize = |id| -> Result<String, WorldError> {
                let text = host
                    .strings
                    .get_string(id)
                    .ok_or_else(|| format!("missing tutorial string {id}"))?;
                host.strings
                    .parse_references(&text, 256)
                    .map_err(|e| format!("{e:?}"))
            };
            match &action.command {
                TutorialCommand::Hint {
                    id,
                    append_controller,
                    animation,
                    position,
                } => {
                    let key = if *append_controller {
                        format!("{id}_key")
                    } else {
                        id.clone()
                    };
                    let id = custom_hash(&key);
                    let text = host
                        .body_font
                        .wrap(&localize(id)?, 300, true)
                        .map_err(|e| format!("{e:?}"))?
                        .text;
                    let controller_text = if *append_controller {
                        let TutorialCommand::Hint { id: base, .. } = &action.command else {
                            unreachable!()
                        };
                        Some(
                            host.body_font
                                .wrap(&localize(custom_hash(&format!("{base}_pad")))?, 300, true)
                                .map_err(|e| format!("{e:?}"))?
                                .text,
                        )
                    } else {
                        None
                    };
                    self.tutorial.set_hint(Hint {
                        id,
                        text,
                        controller_text,
                        animation: *animation,
                        position: *position,
                    });
                }
                TutorialCommand::Dialog {
                    id,
                    element,
                    magick,
                    trigger,
                    scale,
                    size,
                } => {
                    let mut text = localize(*id)?;
                    let title = if *magick == 0 && *element != 0 {
                        [
                            "earth",
                            "water",
                            "cold",
                            "fire",
                            "lightning",
                            "arcane",
                            "life",
                            "shield",
                            "ice",
                            "steam",
                            "poison",
                        ]
                        .get(element.trailing_zeros() as usize)
                        .map(|name| format!("#element_{name}"))
                    } else {
                        None
                    };
                    if let Some(title) = title {
                        text = format!("{}\n\n{text}", localize(custom_hash(&title))?);
                    }
                    let text = host
                        .body_font
                        .wrap(&text, 300, true)
                        .map_err(|e| format!("{e:?}"))?
                        .text;
                    self.tutorial.set_dialog(DialogHint {
                        text,
                        element: *element,
                        magick: *magick,
                        trigger: *trigger,
                        scale: *scale,
                        size: *size,
                    });
                }
                TutorialCommand::RemoveDialog | TutorialCommand::Dark(_) => unreachable!(),
            }
            Ok(())
        })())
    }
    fn audio_host(&mut self) -> Option<&mut dyn AudioActionHost<Error = WorldError>> {
        Some(self)
    }
    fn effect_host(&mut self) -> Option<&mut dyn EffectActionHost<Error = WorldError>> {
        Some(self)
    }
    fn entity_command_host(&mut self) -> Option<&mut dyn EntityCommandHost<Error = WorldError>> {
        Some(self)
    }
    fn spawn_magick_host(
        &mut self,
    ) -> Option<&mut dyn crate::magick_books::SpawnMagickHost<Error = WorldError>> {
        Some(self)
    }
    fn spawn_item_host(
        &mut self,
    ) -> Option<&mut dyn crate::spawn_item::SpawnItemHost<Error = WorldError>> {
        Some(self)
    }
    fn spawn_phys_host(&mut self) -> Option<&mut dyn SpawnPhysHost<Error = WorldError>> {
        Some(self)
    }
    fn avatar_move_host(&mut self) -> Option<&mut dyn AvatarMoveHost<Error = WorldError>> {
        Some(self)
    }
    fn toggle_host(&mut self) -> Option<&mut dyn ToggleHost<Error = WorldError>> {
        Some(self)
    }
    fn scene_flow_host(
        &mut self,
    ) -> Option<&mut dyn crate::scene_flow::SceneFlowHost<Error = WorldError>> {
        Some(self)
    }
    fn condition_facts(&self) -> Option<&dyn WorldConditionFacts> {
        Some(self)
    }
    fn dialogs(&self) -> Option<crate::dialog_state::DialogFacts<'_>> {
        self.dialogs.as_ref().map(|d| d.facts(0))
    }
    fn presence(&self) -> Option<PresenceFacts<'_>> {
        Some(PresenceFacts {
            areas: &self.presence,
            entities: &self.presence_entities,
        })
    }
    fn choose_group(&mut self, count: usize) -> Result<usize, WorldError> {
        Ok(((self.random() * count as f32) as usize).min(count - 1))
    }
    fn notify_trigger(
        &mut self,
        _: &mut ScriptRuntime,
        _: i32,
        _: usize,
        _: Option<u16>,
    ) -> Result<(), WorldError> {
        Ok(())
    }
    fn phys_animation_action(
        &mut self,
        action: &crate::phys_animation::PhysAnimationAction,
    ) -> Option<Result<(), WorldError>> {
        Some((|| {
            // Entity.GetByID(...) as DamageablePhysicsEntity, then
            // AddAnimation: a missing or non-damageable entity throws, as
            // does a missing part ID.
            let handle = self
                .by_unique_id(action.entity)
                .map(|e| e.handle)
                .filter(|h| self.physics_damage.contains_key(h))
                .ok_or_else(|| format!("PhysAnimation: no damageable entity {}", action.entity))?;
            self.collision
                .part_by_ids_mut(&action.path)
                .map_err(|e| format!("PhysAnimation: {e:?}"))?;
            self.physics_damage
                .get_mut(&handle)
                .unwrap()
                .animations
                .add(action.path.clone(), action.control);
            Ok(())
        })())
    }
    fn animation(
        &mut self,
        _: &mut ScriptRuntime,
        action: &AnimationAction,
    ) -> Result<(), WorldError> {
        self.collision
            .execute_animation_action(action)
            .map_err(|e| format!("animation: {e:?}"))
    }
    fn action_executed(&mut self, _: &mut ScriptRuntime, _: usize) -> Result<(), WorldError> {
        Ok(())
    }
}

impl<C: SceneContent> SceneWorld<C> {
    fn avatar_body(&self, handle: u16) -> Option<usize> {
        self.players
            .iter()
            .flatten()
            .find(|p| p.handle == handle)
            .map(|p| p.body)
    }

    /// Register a player's avatar as an entity with a physics body.
    /// Agent.SetOrder on a live NPC.
    fn live_set_order(&mut self, npc: u16, order: &crate::spawn_action::SpawnOrder<'_>) {
        // Agent.SetOrder resolves the priority ID at execution time, then
        // stores both the damageable target and the preferred ability index.
        // Entity.Initialize never registers unique ID 0, so GetByID(0) is null.
        let priority_target = (order.priority_target != 0)
            .then(|| self.by_unique_id(order.priority_target))
            .flatten()
            .and_then(|e| {
                (matches!(e.kind, EntityKind::Npc { .. } | EntityKind::Avatar { .. })
                    || self.physics_damage.contains_key(&e.handle)
                    || matches!(e.kind, EntityKind::ElementalEgg))
                .then_some(e.handle)
            });
        if let Some(n) = self.npcs.get_mut(&npc) {
            n.set_order(
                order.order,
                order.react_to,
                order.reaction,
                order.trigger,
                Some(order.events.to_vec()),
            );
            n.combat.priority_target = priority_target;
            n.combat.priority_ability = order.priority_ability;
        }
    }

    /// Grow/Shrink.Execute for a live Character caster (player or NPC).
    pub fn size_magick(&mut self, owner: u16, kind: magicka_sim::size_magicks::SizeKind) -> bool {
        use magicka_sim::size_magicks::{CastResult, SizeKind};
        let Some(index) = self.character_body(owner) else {
            return false;
        };
        let (at, direction) = (
            self.physics.bodies[index].position,
            self.physics.bodies[index].direction(),
        );
        let result = self.size_effects.cast(owner, kind);
        let name = match kind {
            SizeKind::Grow => "magick_grow",
            SizeKind::Shrink => "magick_shrink",
        };
        if result != CastResult::Refreshed {
            self.play_cue(512, name, Some(at));
        }
        if result == CastResult::Started {
            self.start_named_effect(name, at, direction);
        }
        true
    }

    /// ClearMagicks/OnRemove: restore bodies, retaining cached aura state.
    pub fn clear_size_effects(&mut self) {
        let mut sizes = std::mem::take(&mut self.size_effects);
        sizes.clear(self);
        self.size_effects = sizes;
    }

    /// Magick.Effect.Execute for a player's combined magick (research/magicks.md).
    /// Unported magicks are logged and have no effect.
    pub fn cast_magick(&mut self, slot: usize, magick: magicka_sim::spells::MagickType) {
        let Some(p) = self.players[slot].as_ref() else {
            return;
        };
        let handle = p.handle;
        self.cast_magick_from(handle, magick);
    }

    pub fn cast_magick_from(&mut self, handle: u16, magick: magicka_sim::spells::MagickType) {
        use magicka_sim::spells::MagickType as M;
        if self.scripted_magicks & (1u64 << (magick as u8 & 63)) != 0 {
            if self.role != NetworkRole::Client {
                self.scripted_magick_casts.push((handle, magick as u8));
            }
            return;
        }
        let Some(body) = self.character_body(handle) else {
            return;
        };
        let position = self.physics.bodies[body].position;
        let player = self.players.iter().flatten().find(|p| p.handle == handle);
        let pick = player.and_then(|p| p.mouse_pick);
        let target = self
            .npcs
            .get(&handle)
            .and_then(|n| n.combat.current_target())
            .unwrap_or_else(|| self.mouse_target(handle, pick));
        match magick {
            M::Grow => {
                self.size_magick(handle, magicka_sim::size_magicks::SizeKind::Grow);
            }
            M::Shrink => {
                self.size_magick(handle, magicka_sim::size_magicks::SizeKind::Shrink);
            }
            M::Haste => {
                self.ability_effects.haste(handle, None);
                self.play_cue(16, "magick_haste", Some(position));
            }
            M::Invisibility => {
                if let Some(character) =
                    Self::character_mut(&mut self.players, &mut self.npcs, handle)
                {
                    character.set_invisible(magicka_sim::magicks::INVISIBILITY_TIME);
                }
                self.play_cue(16, "magick_invisibility", Some(position));
            }
            M::Nullify => self.nullify(),
            M::Fear => {
                self.fear_from(handle);
            }
            M::Charm => {
                self.charm_from(handle);
            }
            M::Confuse => {
                self.confuse_from(handle);
            }
            M::Levitate => {
                self.levitate(handle);
            }
            M::Corporealize => {
                self.corporealize(position);
            }
            M::Etherealize => {
                self.etherealize(handle);
            }
            M::Grease => {
                // Grease.Execute: the cue and the effect; PlaySession owns
                // the spray effect's lifetime (sync_grease_sprays).
                self.play_cue(16, "magick_grease", Some(position));
                self.ability_effects.grease(handle);
            }
            M::Polymorph => {
                self.polymorph_from(handle);
            }
            M::Tornado => {
                self.tornado_from(handle);
            }
            M::Vortex => {
                self.vortex_from(handle);
            }
            M::Wave => {
                self.wave_from(handle);
            }
            M::Portal => {
                self.portal_from(handle);
            }
            M::SPhoenix => {
                self.phoenix_from(handle);
            }
            M::SDeath => {
                self.summon_death_from(handle);
            }
            M::SElemental => {
                if let Err(error) = self.summon_elemental_from(handle) {
                    self.log.push(format!("summon elemental: {error}"));
                }
            }
            M::Earthquake => {
                self.earthquake_from(handle);
            }
            M::SpawnSlime | M::SpawnSlimeOverkill => {
                let cast = slime_live::SlimeCast {
                    overkill: magick == M::SpawnSlimeOverkill,
                    ..Default::default()
                };
                if let Err(error) = self.spawn_slimes(handle, cast) {
                    self.log.push(format!("spawn slime: {error}"));
                }
            }
            M::EtherealClone => {
                if let Err(error) = self.ethereal_clone_player(handle) {
                    self.log.push(format!("ethereal clone: {error}"));
                }
            }
            M::SummonCross => {
                self.summon_cross(handle);
            }
            M::GreaseSplash => {
                self.grease_splash(handle);
            }
            M::SUndead => {
                self.summon_undead(handle);
            }
            M::Ctd => {
                self.ctd(handle);
            }
            M::TractorPull => {
                self.tractor_pull(Some(handle), position);
            }
            M::ProppMagick => {
                self.propp_magick(handle);
            }
            M::ChainLightning => {
                self.chain_lightning(handle);
            }
            M::VladZap => {
                self.vlad_zap_player(handle, target);
            }
            M::PerformanceEnchantment => {
                self.performance_enchantment(handle);
            }
            M::Amalgameddon => {
                self.amalgameddon(handle);
            }
            M::JudgementSpray => {
                // JudgementSpray.Execute (the local authority).
                self.ability_effects.judgement_spray(handle);
                self.play_cue(0x200, "woot_magick_judgment_fire", Some(position));
            }
            M::MeteorS => {
                self.meteor_shower(Some(handle), position);
            }
            M::Napalm => {
                let direction = self.physics.bodies[body].direction();
                self.napalm(handle, position, direction);
            }
            M::Conflagration => {
                self.conflagration_from(handle);
            }
            M::ThunderB => {
                let direction = self.physics.bodies[body].direction();
                self.thunderbolt(Some(handle), position, direction);
            }
            M::Rain => {
                self.rain_from(Some(handle));
            }
            M::Blizzard => {
                if self.scene_indoors {
                    self.log.push("blizzard: refused indoors".into());
                } else {
                    if self.blizzard_cue.is_none() {
                        self.blizzard_cue = Some(self.play_cue(16, "magick_blizzard", None));
                    }
                    self.ability_effects.blizzard(Some(handle), self.play_time);
                }
            }
            M::ThunderS => {
                // Thunderstorm.Execute: refused indoors (no cue), then its
                // Rain and the storm itself.
                if self.scene_indoors {
                    self.log.push("thunderstorm: refused indoors".into());
                } else {
                    self.rain_from(Some(handle));
                    self.ability_effects
                        .thunderstorm(Some(handle), position, self.play_time);
                }
            }
            M::TimeWarp => {
                // TimeWarp.Execute: refused while one is active (no cue).
                if self
                    .ability_effects
                    .time_warp(Some(handle), self.scene_saturation)
                {
                    self.time_warp_cue = Some(self.play_cue(16, "magick_timewarp", None));
                }
            }
            M::Revive => {
                use magicka_sim::magicks::{REVIVE_CASTER_HEALTH, REVIVE_DISTANCE};
                let direction = self.physics.bodies[body].direction();
                let point = [
                    position[0] + direction[0] * REVIVE_DISTANCE,
                    position[1] + direction[1] * REVIVE_DISTANCE,
                    position[2] + direction[2] * REVIVE_DISTANCE,
                ];
                let reviver = self
                    .players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == handle)
                    .map(|p| p.slot);
                self.execute_revive(Some(position), point, REVIVE_CASTER_HEALTH, None, reviver);
            }
            M::Teleport => {
                let direction = self.physics.bodies[body].direction();
                let destination = [
                    position[0] + direction[0] * TELEPORT_DISTANCE,
                    position[1] + direction[1] * TELEPORT_DISTANCE,
                    position[2] + direction[2] * TELEPORT_DISTANCE,
                ];
                if !self.teleport_character(handle, destination, direction) {
                    self.log
                        .push("teleport: no ground below the destination".into());
                }
            }
            M::Votal | M::KnockDownChance | M::InflictBleedDamage | M::InflictPoisonDamage => {
                self.target_magick(handle, target, magick);
            }
            other => self
                .log
                .push(format!("magick not reconstructed: {other:?}")),
        }
    }

    /// Rewrite rule for target-only magicks cast by a player (user decision
    /// 2026-10-11, research/custom-magicks.md): the living character whose
    /// centre is horizontally closest to the mouse pick, the caster included.
    /// Without a pick, or when nothing else qualifies, the caster.
    pub fn mouse_target(&self, owner: u16, pick: Option<[f32; 3]>) -> u16 {
        let Some(pick) = pick else {
            return owner;
        };
        let mut best = (owner, f32::INFINITY);
        for c in self.combatants() {
            if c.dead && c.handle != owner {
                continue;
            }
            let d = (c.position[0] - pick[0]).powi(2) + (c.position[2] - pick[2]).powi(2);
            // Strictly closer wins, so ties keep the caster or earlier entry.
            if d < best.1 {
                best = (c.handle, d);
            }
        }
        best.0
    }

    /// ITargetAbility.Execute(owner, target) for Votal, KnockDownChance,
    /// InflictBleedDamage and InflictPoisonDamage (native CIL in
    /// research/custom-magicks.md). The authority applies the payload.
    pub fn target_magick(
        &mut self,
        owner: u16,
        target: u16,
        magick: magicka_sim::spells::MagickType,
    ) {
        use magicka_sim::damage::{attack, Damage};
        use magicka_sim::spells::{Elements as E, MagickType as M};
        use magicka_sim::status::{Status, StatusEffect};
        if self.role == NetworkRole::Client {
            return;
        }
        let Some(source) = self
            .combatants()
            .into_iter()
            .find(|c| c.handle == owner)
            .map(|c| c.position)
        else {
            return;
        };
        let status = |kind, dps| StatusEffect::new(kind, dps, 3.0, true);
        match magick {
            M::Votal => {
                let mut damage = Damage {
                    attack: attack::STATUS,
                    element: E::NONE.0,
                    amount: 50.0,
                    magnitude: 1.0,
                };
                match ((self.random() * 7.0) as u32).min(6) {
                    0 => damage.element = E::POISON.0,
                    1 => damage.element = E::WATER.0,
                    2 => damage.attack = attack::KNOCKDOWN,
                    3 => {
                        let wet = Damage {
                            attack: attack::STATUS,
                            element: E::WATER.0,
                            amount: 1.0,
                            magnitude: 1.0,
                        };
                        self.hit_character(target, wet, Some(owner), source);
                        damage.element = E::COLD.0;
                    }
                    4 => damage.element = E::COLD.0,
                    5 => {
                        damage.amount = 30.0;
                        damage.attack = attack::BLEED;
                    }
                    _ => damage.element = E::FIRE.0,
                }
                self.hit_character(target, damage, Some(owner), source);
            }
            M::KnockDownChance => {
                let knockdown = ((self.random() * 4.0) as u32).min(3) == 0;
                let damage = Damage {
                    attack: if knockdown {
                        attack::KNOCKDOWN
                    } else {
                        attack::STATUS
                    },
                    element: E::NONE.0,
                    amount: 10.0,
                    magnitude: 1.0,
                };
                self.hit_character(target, damage, Some(owner), source);
            }
            M::InflictBleedDamage | M::InflictPoisonDamage => {
                let mut events = Vec::new();
                let effect = |max_hp: f32| {
                    if magick == M::InflictBleedDamage {
                        status(Status::Bleeding, max_hp * (1.0 / 30.0))
                    } else {
                        status(Status::Poisoned, 100.0)
                    }
                };
                if let Some(p) = self
                    .players
                    .iter_mut()
                    .flatten()
                    .find(|p| p.handle == target)
                {
                    let e = effect(p.character.combat.health.max_hit_points);
                    let _ = p.add_status_effect(e, &mut events);
                } else if let Some(n) = self.npcs.get_mut(&target) {
                    let e = effect(n.character.combat.health.max_hit_points);
                    let _ = n.character.add_status_effect(e, None, &mut events);
                }
            }
            _ => {}
        }
    }

    /// Teleport.DoTeleport (TeleportType.Regular) for a player avatar: the
    /// origin effect and cue always play; the move needs ground within 4 m
    /// below the navmesh point nearest the destination.
    pub fn teleport(&mut self, slot: usize, destination: [f32; 3], direction: [f32; 3]) -> bool {
        let Some(p) = self.players[slot].as_ref() else {
            return false;
        };
        self.teleport_character(p.handle, destination, direction)
    }

    fn teleport_character(
        &mut self,
        handle: u16,
        destination: [f32; 3],
        direction: [f32; 3],
    ) -> bool {
        self.teleport_character_kind(handle, destination, direction, false)
    }

    fn teleport_character_kind(
        &mut self,
        handle: u16,
        destination: [f32; 3],
        direction: [f32; 3],
        smoke: bool,
    ) -> bool {
        let Some(body_index) = self.character_body(handle) else {
            return false;
        };
        let origin = self.physics.bodies[body_index].position;
        self.mine_effects.push((
            if smoke {
                "smoke_bomb"
            } else {
                "magick_teleport_disappear"
            },
            origin,
            direction,
        ));
        self.play_cue(
            16,
            if smoke {
                "spell_fire_self"
            } else {
                "magick_teleporta"
            },
            Some(origin),
        );
        // NavMesh.GetNearestPosition(..., MovementProperties.Water).
        let point = self
            .nav
            .as_ref()
            .and_then(|nav| nav.nearest(destination, magicka_sim::navmesh::movement::WATER))
            .map_or(destination, |(_, point, _)| point);
        let probe = Segment {
            origin: point,
            delta: [0.0, -4.0, 0.0],
        };
        let Some(hit) = self.collision.collision().segment_intersect(probe, false) else {
            return false;
        };
        // Teleport.Execute: ReleaseEntanglement on arrival.
        if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, handle) {
            c.entanglement.release(&mut c.entangle_events);
        }
        let body = &mut self.physics.bodies[body_index];
        // Y + 0.1 - HeightOffset, HeightOffset = -(radius + length / 2).
        let mut target = hit.hit.position;
        target[1] += 0.1 + body.radius + body.length * 0.5;
        let orientation = body.orientation;
        body.move_to(target, orientation);
        self.mine_effects.push((
            if smoke {
                "smoke_bomb"
            } else {
                "magick_teleport_appear"
            },
            target,
            direction,
        ));
        self.play_cue(
            16,
            if smoke {
                "spell_fire_self"
            } else {
                "magick_teleportb"
            },
            Some(target),
        );
        true
    }

    /// Nullify.NullifyArea (iHitBoss: true): a global cancel of magicks,
    /// missiles, statuses, self shields, invisibility, current spells, queues,
    /// self-cast auras, barriers, mines and shields. Entities that lost
    /// something get the `magick_nullified` effect. Independent of the
    /// caster: retail also reaches it from NPC/boss magicks (Grimnir2,
    /// Assatur), Krietor and the Nullify trigger action.
    pub fn nullify(&mut self) {
        use magicka_sim::status::Status as S;
        const STATUSES: u16 = S::Burning as u16
            | S::Wet as u16
            | S::Frozen as u16
            | S::Cold as u16
            | S::Poisoned as u16
            | S::Healing as u16
            | S::Greased as u16
            | S::Steamed as u16;
        self.play_cue(16, "magick_nullify", None);
        self.flash(0.2);
        self.kill_portals();
        // SpellManager.ClearMagicks: every SpecialAbility effect (Haste,
        // Revive; a removed Revive clears Ressing).
        self.ability_effects.clear_magicks();
        self.clear_size_effects();
        self.clear_polymorphs();
        self.phoenix.remove();
        self.clear_death();
        self.clear_earthquake();
        self.clear_waves();
        // NullifyArea: TornadoEntity.Kill (TTL 1) and VortexEntity.Kill
        // (target mass 0) on every live instance.
        for t in &mut self.tornadoes {
            t.tornado.kill();
        }
        for v in &mut self.vortices {
            v.vortex.kill();
        }
        // SummonCross.OnRemove is empty: its NPC and bubble survive this.
        self.crosses.clear();
        self.end_conflagrations();
        self.nullify_grimnir2();
        self.end_confusions();
        self.sync_time_warp();
        self.sync_rain();
        self.sync_blizzard();
        self.sync_meteor_shower();
        for missile in &mut self.missiles {
            missile.body.dead = true;
        }
        let mut nullified = Vec::new();
        for p in self.players.iter_mut().flatten() {
            let body = &mut self.physics.bodies[p.body];
            stop_levitate(&mut p.character, body);
            let mut flag = nullify_character(&mut p.character, STATUSES);
            flag |= p.stop_current_spell();
            if !p.conjure.queue.is_empty() {
                p.conjure.queue.clear();
                flag = true;
            }
            if flag {
                nullified.push((body.position, body.direction()));
            }
        }
        let mut summoned = Vec::new();
        for npc in self.npcs.values_mut() {
            let body = &mut self.physics.bodies[npc.body];
            stop_levitate(&mut npc.character, body);
            let mut flag = nullify_character(&mut npc.character, STATUSES);
            flag |= npc.stop_current_spell();
            if !npc.combat.spellcasting.queue.is_empty() {
                npc.combat.spellcasting.queue.clear();
                flag = true;
            }
            // NonPlayerCharacter.EndCharm.
            if npc.character.is_charmed() {
                npc.character.charm = None;
                npc.charm_reset = true;
                flag = true;
            }
            // Summoned NPCs (but the Summon Cross "cross") are killed.
            if npc.summon.is_some() && !npc.definition.id.eq_ignore_ascii_case("cross") {
                summoned.push(npc.handle);
                flag = true;
            }
            if flag {
                nullified.push((body.position, body.direction()));
            }
        }
        for handle in summoned {
            let _ = crate::entity_command::EntityCommandHost::kill(self, handle);
        }
        let barriers: Vec<usize> = self.barriers.iter().map(|(slot, _)| slot).collect();
        for slot in barriers {
            if let Some(b) = self.barriers.get(slot) {
                nullified.push((b.position, [0.0, 0.0, -1.0]));
            }
            self.kill_barrier(slot);
        }
        let mines: Vec<usize> = self.mines.iter().map(|(slot, _)| slot).collect();
        for slot in mines {
            if let Some(m) = self.mines.get_mut(slot) {
                m.kill();
                nullified.push((m.position, [0.0, 0.0, -1.0]));
            }
        }
        let grease: Vec<(usize, [f32; 3])> = self
            .grease
            .iter()
            .map(|(slot, f)| (slot, f.position))
            .collect();
        for (slot, position) in grease {
            self.kill_grease(slot);
            nullified.push((position, [0.0, 0.0, -1.0]));
        }
        for shield in &mut self.shields {
            shield.shield.kill();
        }
        for (position, direction) in nullified {
            self.mine_effects
                .push(("magick_nullified", position, direction));
        }
    }

    /// Conflagration.Execute(ISpellCaster): snapshot the pose; every cast
    /// owns its own sweep and retained emitter, including repeated casts.
    pub fn conflagration_from(&mut self, caster: u16) -> bool {
        let Some(body) = self.character_body(caster) else {
            return false;
        };
        let c = &self.physics.bodies[body];
        let sweep = magicka_sim::conflagration::Conflagration::from_caster(
            Some(caster),
            c.position,
            c.direction(),
            self.play_time,
        );
        self.start_conflagration(sweep);
        true
    }

    /// The explicit position/direction overload does not shift the origin.
    pub fn conflagration_directed(&mut self, position: [f32; 3], direction: [f32; 3]) {
        let sweep = magicka_sim::conflagration::Conflagration::directed(
            position,
            direction,
            self.play_time,
        );
        self.start_conflagration(sweep);
    }

    fn start_conflagration(&mut self, sweep: magicka_sim::conflagration::Conflagration) {
        self.play_cue(16, "magick_conflagration", Some(sweep.position));
        let instance = self.start_effect_transform(
            crate::scene_loading::custom_hash("magick_conflagration"),
            crate::particles::create_world(sweep.position, sweep.direction, [0.0, 1.0, 0.0]),
        );
        self.ability_effects.conflagration(sweep, instance);
    }

    fn end_conflagrations(&mut self) {
        for instance in std::mem::take(&mut self.ability_effects.conflagration_ended) {
            self.effect_commands
                .push(SceneEffectCommand::Stop { instance });
        }
    }

    /// Thunderbolt.Execute for a character caster (`caster` None: the
    /// position overload's random direction is the caller's). Indoors it
    /// fails with magick_fail. Otherwise: Flash 0.125; among living
    /// non-ethereal characters within 8 m (flat) of the point 8 m ahead,
    /// outside sphere shields, the one with the highest collision top is hit
    /// for 5000 Lightning with knockback, unless the caster stands in a
    /// sphere shield (the shield takes it). A miss strikes a random point
    /// within ±4 m. The bolt falls from 40 m; magick_thunderbolt effect, a
    /// Scorched decal on the ground below a non-shield target, the cue and a
    /// 1.5/0.333 camera shake.
    pub fn thunderbolt(
        &mut self,
        caster: Option<u16>,
        position: [f32; 3],
        direction: [f32; 3],
    ) -> bool {
        use magicka_sim::damage::{attack, Damage};
        if self.scene_indoors {
            self.play_cue(16, "magick_fail", Some(position));
            return false;
        }
        if self.role == NetworkRole::Client {
            return true;
        }
        self.flash(0.125);
        let half = THUNDERBOLT_RANGE * 0.5;
        let center = [
            position[0] + direction[0] * half,
            direction[1] * half,
            position[2] + direction[2] * half,
        ];
        let combatants = self.combatants();
        let spheres: Vec<_> = self
            .shields
            .iter()
            .filter(|s| {
                !s.shield.dead && s.shield.kind == magicka_sim::shield_entity::ShieldKind::Sphere
            })
            .map(|s| (s.handle, s.shield.position, s.shield.radius))
            .collect();
        let shield_of = |p: [f32; 3]| {
            spheres.iter().find(|(_, c, r)| {
                let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < r * r
            })
        };
        let caster_shield = caster
            .and_then(|h| combatants.iter().find(|c| c.handle == h))
            .and_then(|c| shield_of(c.position))
            .copied();
        // (target handle, strike point, shield?)
        let mut target: Option<(u16, [f32; 3], bool)> = None;
        if let Some((handle, center_pos, radius)) = caster_shield {
            // Sphere: Y += WorldBoundingBox.Max.Y·0.5 (the box top is
            // centre.y + radius).
            let mut p = center_pos;
            p[1] += (center_pos[1] + radius) * 0.5;
            target = Some((handle, p, true));
        } else {
            let mut best = f32::MIN;
            for c in &combatants {
                if c.dead || c.ethereal || Some(c.handle) == caster {
                    continue;
                }
                let flat =
                    (c.position[0] - center[0]).powi(2) + (c.position[2] - center[2]).powi(2);
                let reach = half + c.radius;
                if flat > reach * reach || shield_of(c.position).is_some() {
                    continue;
                }
                let top = c.position[1] + c.length * 0.5 + c.radius;
                if top > best {
                    best = top;
                    target = Some((c.handle, c.position, false));
                }
            }
        }
        let damage = Damage {
            attack: attack::KNOCKBACK | attack::DAMAGE,
            element: magicka_sim::spells::Elements::LIGHTNING.0,
            amount: THUNDERBOLT_DAMAGE,
            magnitude: 1.0,
        };
        let (mut strike, bolt_direction) = match target {
            Some((handle, point, true)) => {
                let _ = self.damage_shield_handle(handle, damage);
                (point, [0.0, -1.0, 0.0])
            }
            Some((handle, point, false)) => {
                self.hit_character(handle, damage, caster, center);
                (point, [0.0, -1.0, 0.0])
            }
            None => {
                let x = (self.random() - 0.5) * half;
                let z = (self.random() - 0.5) * half;
                ([center[0] + x, center[1], center[2] + z], [1.0, 0.0, 0.0])
            }
        };
        let mut from = center;
        from[1] += THUNDERBOLT_HEIGHT;
        self.magick_bolts.push(magicka_sim::lightning::BoltVisual {
            from,
            to: strike,
            direction: bolt_direction,
            color: LIGHTNING_COLOR,
            scale: 1.0,
            ttl: THUNDERBOLT_BOLT_TTL,
        });
        self.mine_effects
            .push(("magick_thunderbolt", strike, [1.0, 0.0, 0.0]));
        if !matches!(target, Some((_, _, true))) {
            let segment = magicka_sim::spray_pulse::Segment {
                origin: [strike[0], strike[1] + 1.0, strike[2]],
                delta: [0.0, -10.0, 0.0],
            };
            if let Some(hit) = self.collision.collision().segment_intersect(segment, false) {
                strike = hit.hit.position;
                self.decals.add(crate::decals::Request {
                    cell: crate::decals::SCORCHED,
                    scale: [4.0; 2],
                    position: strike,
                    direction: None,
                    normal: hit.hit.normal,
                    ttl: 60.0,
                    color: [1.0; 4],
                    animated_owner: hit.animated_owner,
                });
            }
        }
        self.play_cue(16, "magick_thunderbolt", Some(strike));
        self.camera.shake(1.5, 0.333);
        true
    }

    /// A spray ray's reach: `range` along `direction`, cut by each shield it
    /// crosses (in list order) and then by the scene (Grease, PoisonSpray).
    pub(super) fn clipped_reach(&self, origin: [f32; 3], direction: [f32; 3], range: f32) -> f32 {
        let mut range = range;
        let mut segment = magicka_sim::spray_pulse::Segment {
            origin,
            delta: direction.map(|v| v * range),
        };
        for shield in &self.shields {
            if let Some(frac) = shield.shield.geometry().segment_fraction(segment) {
                range *= frac;
                segment.delta = segment.delta.map(|v| v * frac);
            }
        }
        if let Some(hit) = self.collision.collision().segment_intersect(segment, false) {
            range *= hit.hit.fraction;
        }
        range
    }

    /// EffectManager.StartEffect at a transform; returns the instance.
    pub(crate) fn start_effect_transform(&mut self, effect: i32, transform: [f32; 16]) -> u64 {
        self.effect_serial = self.effect_serial.wrapping_add(1);
        self.effect_commands.push(SceneEffectCommand::Start {
            instance: self.effect_serial,
            effect,
            transform,
        });
        self.effect_serial
    }

    /// EffectManager.StartEffect(name, position, direction) (Up as up).
    pub(crate) fn start_named_effect(
        &mut self,
        name: &str,
        position: [f32; 3],
        direction: [f32; 3],
    ) -> u64 {
        let transform = crate::particles::create_world(position, direction, [0., 1., 0.]);
        self.start_effect_transform(custom_hash(name), transform)
    }

    /// IDamageable.Damage(damage, attacker, PlayTime, source) on a character
    /// through the public route (NPCs record the attacker).
    fn hit_character(
        &mut self,
        target: u16,
        damage: magicka_sim::damage::Damage,
        attacker: Option<u16>,
        source: [f32; 3],
    ) {
        self.hit_character_at(target, damage, attacker, source, self.play_time);
    }

    /// IDamageable.Damage on any damageable handle: grease fields, mines,
    /// barriers and shields by their reserved keys, else characters and
    /// physics entities through the hit route.
    pub(super) fn damage_any(
        &mut self,
        target: u16,
        damage: magicka_sim::damage::Damage,
        owner: Option<u16>,
        source: [f32; 3],
    ) {
        self.damage_any_at(target, damage, owner, source, self.play_time);
    }

    fn damage_any_at(
        &mut self,
        target: u16,
        damage: magicka_sim::damage::Damage,
        owner: Option<u16>,
        source: [f32; 3],
        timestamp: f64,
    ) {
        if target & MINE_KEY == GREASE_KEY {
            self.damage_grease((target & !MINE_KEY) as usize, &[damage]);
        } else if target & MINE_KEY == MINE_KEY {
            if let Some(mine) = self.mines.get_mut((target & !MINE_KEY) as usize) {
                let mut slots = [magicka_sim::damage::Damage::default(); 5];
                slots[0] = damage;
                mine.internal_damage(&slots);
            }
        } else if target & MINE_KEY == BARRIER_KEY {
            let slot = (target & !BARRIER_KEY) as usize;
            if let Some(barrier) = self.barriers.get_mut(slot) {
                let mut events = Vec::new();
                barrier.internal_damage_one(damage, true, &mut events);
                self.barrier_events(slot, events);
            }
        } else if self.shields.iter().any(|s| s.handle == target) {
            self.damage_shield_handle(target, damage);
        } else {
            self.hit_character_at(target, damage, owner, source, timestamp);
        }
    }

    fn hit_character_at(
        &mut self,
        target: u16,
        damage: magicka_sim::damage::Damage,
        attacker: Option<u16>,
        source: [f32; 3],
        timestamp: f64,
    ) {
        let combatants = self.combatants();
        let scene = self.collision.collision();
        let Some(view) = crate::npc_combat::CombatView::new(&combatants, Some(scene)) else {
            return;
        };
        let mut world = SceneHitWorld {
            physics_damage: &mut self.physics_damage,
            dispensers: &mut self.dispensers,
            entities: &mut self.entities,
            barriers: None,
            barrier_events: Vec::new(),
            view: &view,
            npcs: &mut self.npcs,
            players: &mut self.players,
            bodies: &mut self.physics.bodies,
            triggers: &mut self.pending_triggers,
            shields: &mut self.shields,
            collision_rng: &mut self.rng,
            play_time: timestamp,
            entity_blasts: &mut self.entity_blasts,
            death: &mut self.death,
            jormungandr: &mut self.jormungandr,
            death_damage_features: self.role != NetworkRole::Client,
        };
        crate::missiles::MissileWorld::damage_character(
            &mut world,
            target,
            damage,
            attacker.unwrap_or(0),
            source,
        );
    }

    /// Shield.Damage for a live shield entity.
    fn damage_shield_handle(&mut self, handle: u16, damage: magicka_sim::damage::Damage) -> u16 {
        let combatants = self.combatants();
        let scene = self.collision.collision();
        let Some(view) = crate::npc_combat::CombatView::new(&combatants, Some(scene)) else {
            return 0;
        };
        let mut world = SceneHitWorld {
            physics_damage: &mut self.physics_damage,
            dispensers: &mut self.dispensers,
            entities: &mut self.entities,
            barriers: None,
            barrier_events: Vec::new(),
            view: &view,
            npcs: &mut self.npcs,
            players: &mut self.players,
            bodies: &mut self.physics.bodies,
            triggers: &mut self.pending_triggers,
            shields: &mut self.shields,
            collision_rng: &mut self.rng,
            play_time: self.play_time,
            entity_blasts: &mut self.entity_blasts,
            death: &mut self.death,
            jormungandr: &mut self.jormungandr,
            death_damage_features: self.role != NetworkRole::Client,
        };
        crate::missiles::MissileWorld::damage_shield(&mut world, handle, damage)
    }

    /// Character.Faction: NonPlayerCharacter returns FRIENDLY while charmed,
    /// otherwise mFaction, which Confuse sets to NONE.
    pub fn faction(&self, e: &SceneEntity) -> i32 {
        let npc = matches!(e.kind, EntityKind::Npc { .. })
            .then(|| self.npcs.get(&e.handle))
            .flatten();
        match npc {
            Some(n) if n.character.is_charmed() => crate::camera::FACTION_FRIENDLY,
            Some(n) if n.faction_override.is_some() => n.faction_override.unwrap(),
            _ => e.faction,
        }
    }

    /// Charm/Confuse target choice: among living characters within `radius`
    /// (protected ones ignored) that are not the caster and pass `keep(world,
    /// target, caster)`, the one whose normalized flat offset makes the
    /// smallest angle (acos of the clamped dot) with the caster's facing; the
    /// first wins ties. Returns (caster position, target, target position).
    fn facing_target(
        &self,
        caster: u16,
        radius: f32,
        keep: impl Fn(&Self, &crate::npc_combat::Combatant, &crate::npc_combat::Combatant) -> bool,
    ) -> Option<FacingTarget> {
        use magicka_sim::entity_query::{EntityQueryGrid, QueryEntity};
        let characters = self.combatants();
        let c = characters.iter().find(|c| c.handle == caster)?;
        let query: Vec<_> = characters
            .iter()
            .map(|c| QueryEntity {
                position: c.position,
                radius: c.radius,
                dead: c.dead,
                shield: false,
                damageable: true,
            })
            .collect();
        let found = EntityQueryGrid::build(&query)
            .ok()
            .and_then(|g| {
                g.query(&query, c.position, radius, true, false, |_, _| {
                    (false, [0.0; 3])
                })
                .ok()
            })
            .unwrap_or_default();
        let mut best: Option<(f32, u16, [f32; 3])> = None;
        for t in found.into_iter().map(|i| &characters[i]) {
            if t.handle == caster || !keep(self, t, c) {
                continue;
            }
            let mut d = [
                t.position[0] - c.position[0],
                0.0,
                t.position[2] - c.position[2],
            ];
            let inv = 1.0 / (d[0] * d[0] + d[2] * d[2]).sqrt();
            d = [d[0] * inv, 0.0, d[2] * inv];
            let dot = c.direction[0] * d[0] + c.direction[1] * d[1] + c.direction[2] * d[2];
            let angle = dot.clamp(-1.0, 1.0).acos();
            if best.is_none_or(|(a, _, _)| angle < a) {
                best = Some((angle, t.handle, t.position));
            }
        }
        Some((c.position, best.map(|(_, h, p)| (h, p))))
    }

    /// Charm.Execute for any character caster: among living characters
    /// within 14 m (protected ones ignored) that are not the caster, not
    /// charmed and share no faction bit with it, the one whose flat
    /// direction makes the smallest angle with the caster's facing is
    /// charmed for 15 s (cue on it). Without one the magick_fail cue plays.
    pub fn charm_from(&mut self, caster: u16) -> bool {
        let Some((origin, best)) = self.facing_target(caster, CHARM_RADIUS, |w, t, c| {
            !w.character_ref(t.handle).is_some_and(|r| r.is_charmed()) && t.faction & c.faction == 0
        }) else {
            return false;
        };
        let Some((target, position)) = best else {
            self.play_cue(16, "magick_fail", Some(origin));
            return false;
        };
        self.play_cue(16, "magick_charm", Some(position));
        self.charm_character(target, caster, CHARM_TIME);
        true
    }

    /// Character.Charm (NonPlayerCharacter also pops its AI to idle). The
    /// `magick_charm` effect follows the character in presentation.
    pub fn charm_character(&mut self, target: u16, owner: u16, ttl: f32) -> bool {
        if let Some(n) = self.npcs.get_mut(&target) {
            let applied = n.character.apply_charm(owner, ttl);
            n.charm_reset |= applied;
            return applied;
        }
        Self::character_mut(&mut self.players, &mut self.npcs, target)
            .is_some_and(|c| c.apply_charm(owner, ttl))
    }

    /// Rain.Execute: refused indoors; otherwise the ambience (once), the
    /// singleton's TTL/caster/timestamp and the dimmed light target.
    fn rain_from(&mut self, caster: Option<u16>) -> bool {
        if self.scene_indoors {
            self.log.push("rain: refused indoors".into());
            return false;
        }
        if self.rain_cue.is_none() {
            self.rain_cue = Some(self.play_cue(16, "magick_rain", None));
        }
        let timestamp = if caster.is_some() {
            self.play_time
        } else {
            0.0
        };
        self.ability_effects.rain(caster, timestamp);
        self.light_target_intensity = magicka_sim::magicks::RAIN_LIGHT_INTENSITY;
        true
    }

    /// Thunderstorm.Update once its bolt timer ran out (host only): the
    /// next timer is 0.5-1 s, then 1-2 bolts around the owner (or the
    /// cast position without one).
    fn thunderstorm_bolts(&mut self, storm: magicka_sim::magicks::Thunderstorm) {
        let next = 0.5 + self.random() * 0.5;
        if let Some(t) = self.ability_effects.thunderstorm_mut() {
            t.bolt_ttl = next;
        }
        let count = 1 + ((self.random() * 2.0) as usize).min(1);
        let center = storm
            .owner
            .and_then(|h| {
                self.players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == h)
                    .map(|p| p.body)
                    .or_else(|| self.npcs.get(&h).map(|n| n.body))
            })
            .map_or(storm.position, |b| self.physics.bodies[b].position);
        for _ in 0..count {
            self.thunderstorm_strike(storm.owner, center, storm.timestamp);
        }
    }

    /// One Thunderstorm bolt: a uniform point in the 20 m disc around
    /// `center`, moved to the nearest navmesh point (MovementProperties.All).
    /// Among the living, non-ethereal characters and shields within
    /// 5 m of it (Y ignored, shields filter obstructed characters), the
    /// tallest takes Thunderbolt's damage from 20 m above it. The caster is
    /// not excluded. Flash 0.125, a bolt from 40 m above, the thunderbolt
    /// effect, a scorch decal (not on shields), the cue at the bolt top and
    /// a 1.2 / 0.333 camera shake.
    fn thunderstorm_strike(&mut self, owner: Option<u16>, center: [f32; 3], timestamp: f64) {
        use magicka_sim::magicks::THUNDERSTORM_RADIUS;
        let r = self.random().sqrt();
        let angle = self.random() * std::f32::consts::TAU;
        let point = [
            center[0] + THUNDERSTORM_RADIUS * r * angle.cos(),
            center[1],
            center[2] + THUNDERSTORM_RADIUS * r * angle.sin(),
        ];
        self.strike_at(owner, point, timestamp);
    }

    /// The strike of one Thunderstorm bolt at `point` (snapped to the
    /// navmesh): see `thunderstorm_strike`. Also the mod `Spells.Strike`.
    pub(crate) fn strike_at(&mut self, owner: Option<u16>, mut point: [f32; 3], timestamp: f64) {
        use magicka_sim::damage::{attack, Damage};
        use magicka_sim::magicks::THUNDERSTORM_TARGET_RANGE;
        if let Some((_, p, _)) = self
            .nav
            .as_ref()
            .and_then(|nav| nav.nearest(point, magicka_sim::navmesh::movement::ALL))
        {
            point = p;
        }
        let combatants = self.combatants();
        use magicka_sim::entity_query::{EntityQueryGrid, QueryEntity};
        use magicka_sim::shield_intersection::ShieldGeometry;
        let shields: Vec<_> = self.shields.iter().collect();
        let query: Vec<_> = combatants
            .iter()
            .map(|c| QueryEntity {
                position: c.position,
                radius: c.radius,
                dead: c.dead,
                shield: false,
                damageable: true,
            })
            .chain(shields.iter().map(|s| QueryEntity {
                position: s.shield.position,
                radius: s.shield.radius,
                dead: s.shield.dead,
                shield: true,
                damageable: true,
            }))
            .collect();
        let found = EntityQueryGrid::build(&query).and_then(|grid| {
            grid.query(
                &query,
                point,
                THUNDERSTORM_TARGET_RANGE,
                true,
                true,
                |index, seg| {
                    let geometry = shields[index - combatants.len()].shield.geometry();
                    match geometry.segment_fraction(seg) {
                        Some(t) => (
                            true,
                            std::array::from_fn(|i| seg.origin[i] + seg.delta[i] * t),
                        ),
                        None => (false, [0.0; 3]),
                    }
                },
            )
        });
        // (bounding-box top, handle, strike point, shield?)
        let mut target: Option<(f32, u16, [f32; 3], bool)> = None;
        match found {
            Ok(indices) => {
                for index in indices {
                    let candidate = if let Some(c) = combatants.get(index) {
                        if c.ethereal {
                            continue;
                        }
                        (
                            c.position[1] + c.length * 0.5 + c.radius,
                            c.handle,
                            c.position,
                            false,
                        )
                    } else {
                        let s = shields[index - combatants.len()];
                        let c = s.shield.position;
                        let top = match s.shield.geometry() {
                            ShieldGeometry::Sphere(_) => c[1] + s.shield.radius,
                            ShieldGeometry::Wall(b) => {
                                b.center[1]
                                    + (0..3)
                                        .map(|i| b.axes[i][1].abs() * b.side_lengths[i] * 0.5)
                                        .sum::<f32>()
                            }
                        };
                        let at = if s.shield.kind == magicka_sim::shield_entity::ShieldKind::Sphere
                        {
                            [c[0], c[1] + top * 0.5, c[2]]
                        } else {
                            std::array::from_fn(|i| c[i] + s.shield.forward[i] * s.shield.radius)
                        };
                        (top, s.handle, at, true)
                    };
                    if target.is_none_or(|(best, ..)| candidate.0 > best) {
                        target = Some(candidate);
                    }
                }
            }
            Err(error) => self
                .log
                .push(format!("thunderstorm: entity query {error:?}")),
        }
        self.flash(0.125);
        let damage = Damage {
            attack: attack::KNOCKBACK | attack::DAMAGE,
            element: magicka_sim::spells::Elements::LIGHTNING.0,
            amount: THUNDERBOLT_DAMAGE,
            magnitude: 1.0,
        };
        let shield = matches!(target, Some((.., true)));
        if let Some((_, handle, at, is_shield)) = target {
            point = at;
            if is_shield {
                let _ = self.damage_shield_handle(handle, damage);
            } else {
                let source = [at[0], at[1] + 20.0, at[2]];
                self.hit_character_at(handle, damage, owner, source, timestamp);
            }
        }
        let top = [point[0], point[1] + THUNDERBOLT_HEIGHT, point[2]];
        self.magick_bolts.push(magicka_sim::lightning::BoltVisual {
            from: top,
            to: point,
            direction: [0.0, -1.0, 0.0],
            color: LIGHTNING_COLOR,
            scale: 1.0,
            ttl: THUNDERBOLT_BOLT_TTL,
        });
        self.mine_effects
            .push(("magick_thunderbolt", point, [1.0, 0.0, 0.0]));
        if !shield {
            let segment = magicka_sim::spray_pulse::Segment {
                origin: point,
                delta: [0.0, -10.0, 0.0],
            };
            if let Some(hit) = self.collision.collision().segment_intersect(segment, false) {
                self.decals.add(crate::decals::Request {
                    cell: crate::decals::SCORCHED,
                    scale: [4.0; 2],
                    position: hit.hit.position,
                    direction: None,
                    normal: hit.hit.normal,
                    ttl: 60.0,
                    color: [1.0; 4],
                    animated_owner: hit.animated_owner,
                });
            }
        }
        self.play_cue(16, "magick_thunderbolt", Some(top));
        self.camera.shake(1.2, 0.333);
    }

    /// GreaseField.GetInstance + Initialize: the oldest of the 72 slots is
    /// reused (its decal fades over 1 s), and a 3 m grease decal with the
    /// field's 60 s TTL is placed.
    pub fn spawn_grease_field(
        &mut self,
        owner: Option<u16>,
        position: [f32; 3],
        normal: [f32; 3],
        animated_owner: Option<u32>,
    ) -> usize {
        let slot = self.grease.spawn(owner, position, normal, self.play_time);
        if let Some(r) = self.grease_decals[slot].take() {
            if self.decals.ttl(r).is_some_and(|t| t > 1.0) {
                self.decals.set_ttl(r, 1.0);
            }
        }
        self.grease_decals[slot] = self.decals.add(crate::decals::Request {
            cell: crate::decals::GREASE,
            scale: [3.0; 2],
            position,
            direction: None,
            normal,
            ttl: magicka_sim::grease::FIELD_TTL,
            color: [1.0; 4],
            animated_owner,
        });
        slot
    }

    /// GreaseField.InternalDamage for each damage of a delivery.
    fn damage_grease(&mut self, slot: usize, damages: &[magicka_sim::damage::Damage]) {
        if let Some(f) = self.grease.get_mut(slot) {
            for d in damages {
                f.field
                    .damage(magicka_sim::spells::Elements(d.element), d.magnitude);
            }
        }
    }

    /// Blast.FullBlast's IDamageable pass for grease fields (the character
    /// pass is the existing blast code): every field whose sphere reaches
    /// the radius takes the collection. INFERRED: scene occlusion between
    /// the centre and a field on the floor is not tested.
    fn blast_grease(
        &mut self,
        position: [f32; 3],
        radius: f32,
        damages: &[magicka_sim::damage::Damage; 5],
    ) {
        let reach = radius + magicka_sim::grease::FIELD_RADIUS;
        let slots: Vec<usize> = self
            .grease
            .iter()
            .filter(|(_, f)| {
                (0..3)
                    .map(|i| (f.position[i] - position[i]).powi(2))
                    .sum::<f32>()
                    <= reach * reach
            })
            .map(|(slot, _)| slot)
            .collect();
        for slot in slots {
            self.damage_grease(slot, damages);
        }
    }

    /// GreaseField.Kill (Nullify): TTL 0, dead; Deinitialize fades the decal.
    fn kill_grease(&mut self, slot: usize) {
        if let Some(r) = self.grease_decals[slot].take() {
            if self.decals.ttl(r).is_some_and(|t| t > 1.0) {
                self.decals.set_ttl(r, 1.0);
            }
        }
        self.grease.remove(slot);
    }

    /// GreaseField.Update for every field: a burning field damages each
    /// IDamageable within 1.5 m once per shared hit-list period (240 Fire
    /// status damage; other fields heat up and ignite), then the timers and
    /// the decal TTL; dead fields are removed. The hit list ages once.
    fn update_grease(&mut self, dt: f32) {
        use magicka_sim::damage::{attack, Damage};
        use magicka_sim::grease::FIELD_RADIUS;
        let burn = Damage {
            attack: attack::STATUS,
            element: magicka_sim::spells::Elements::FIRE.0,
            amount: magicka_sim::damage::defines::SPELL_DAMAGE_FIRE * 4.0,
            magnitude: 1.0,
        };
        let slots: Vec<usize> = self.grease.iter().map(|(slot, _)| slot).collect();
        for slot in slots {
            let Some(f) = self.grease.get(slot).copied() else {
                continue;
            };
            if f.field.burning {
                let reach = |p: [f32; 3], r: f32| {
                    let d = (0..3).map(|i| (p[i] - f.position[i]).powi(2)).sum::<f32>();
                    d <= (FIELD_RADIUS + r).powi(2)
                };
                let characters: Vec<u16> = self
                    .combatants()
                    .into_iter()
                    .filter(|c| !c.dead && reach(c.position, c.radius))
                    .map(|c| c.handle)
                    .collect();
                for handle in characters {
                    if !self.grease_hits.contains(handle) {
                        self.grease_hits.add(handle);
                        self.hit_character_at(handle, burn, f.owner, f.position, f.timestamp);
                    }
                }
                let fields: Vec<usize> = self
                    .grease
                    .iter()
                    .filter(|(other, g)| *other != slot && reach(g.position, FIELD_RADIUS))
                    .map(|(other, _)| other)
                    .collect();
                for other in fields {
                    let key = GREASE_KEY | other as u16;
                    if !self.grease_hits.contains(key) {
                        self.grease_hits.add(key);
                        self.damage_grease(other, &[burn]);
                    }
                }
            }
            let Some(f) = self.grease.get_mut(slot) else {
                continue;
            };
            let dead = f.field.update(dt);
            let ttl = f.field.ttl;
            if let Some(r) = self.grease_decals[slot] {
                self.decals.set_ttl(r, ttl);
            }
            if dead {
                self.grease_decals[slot] = None;
                self.grease.remove(slot);
            }
        }
        self.grease_hits.update(dt);
    }

    /// GreaseField.OnCollision's contact for one character body: its
    /// capsule touches some field's 1.5 m sphere.
    pub fn grease_touching(&self, body: usize) -> bool {
        use magicka_sim::grease::FIELD_RADIUS;
        let b = &self.physics.bodies[body];
        let half = b.length * 0.5;
        self.grease.iter().any(|(_, f)| {
            let c = f.position;
            let y = (c[1] - b.position[1]).clamp(-half, half);
            let d = [
                c[0] - b.position[0],
                c[1] - (b.position[1] + y),
                c[2] - b.position[2],
            ];
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= (FIELD_RADIUS + b.radius).powi(2)
        })
    }

    /// GreaseField.OnCollision: a character (not IsNonslippery) touching a
    /// field is on grease for this physics step.
    fn grease_contacts(&mut self) {
        if self.grease.is_empty() {
            return;
        }
        let bodies = self
            .players
            .iter()
            .flatten()
            .map(|p| (p.body, p.character.definition.nonslippery))
            .chain(
                self.npcs
                    .values()
                    .map(|n| (n.body, n.character.definition.nonslippery)),
            )
            .collect::<Vec<_>>();
        for (index, nonslippery) in bodies {
            if !nonslippery && self.grease_touching(index) {
                self.physics.bodies[index].set_on_grease(true);
            }
        }
    }

    /// Grease.Update for each spray, before its TTL ages (host only): the
    /// owner cannot turn. The greasing ray sweeps from -63° to +45° around
    /// the facing, 6 m long and clipped by shields and the scene; characters
    /// it arcs (10°, 5 m height) that are not Greased get Greased. At each
    /// whole step 5..0 a field is placed at the clipped reach minus 1.5 m,
    /// on the ground found 2 m above to 2 m below.
    fn update_grease_sprays(&mut self, dt: f32) {
        use magicka_sim::grease::{spray_spawn_step, spray_step, spray_yaw, SPRAY_RANGE};
        if self.role == NetworkRole::Client {
            return;
        }
        for spray in self.ability_effects.grease_sprays() {
            let Some(p) = self
                .players
                .iter()
                .flatten()
                .find(|p| p.handle == spray.owner)
            else {
                continue;
            };
            let body_index = p.body;
            self.physics.bodies[body_index].owner.turn_speed = 0.0;
            let position = self.physics.bodies[body_index].position;
            let facing = self.physics.bodies[body_index].direction();
            let after = spray.ttl - dt;
            let yaw = |y: f32| {
                let (s, c) = y.sin_cos();
                [
                    facing[0] * c + facing[2] * s,
                    facing[1],
                    -facing[0] * s + facing[2] * c,
                ]
            };
            let direction = yaw(spray_yaw(spray_step(after)));
            let mut range = self.clipped_reach(position, direction, SPRAY_RANGE);
            let query = magicka_sim::arc_intersection::ArcQuery {
                origin: position,
                direction,
                range,
                angle: 10f32.to_radians(),
                height_difference: 5.0,
            };
            let targets: Vec<u16> = self
                .combatants()
                .into_iter()
                .filter(|c| !c.dead && c.handle != spray.owner)
                .map(|c| c.handle)
                .collect();
            for handle in targets {
                let body = self
                    .players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == handle)
                    .map(|p| p.body)
                    .or_else(|| self.npcs.get(&handle).map(|n| n.body));
                let Some(body) = body else { continue };
                let at = self.physics.bodies[body].position;
                let mut events = Vec::new();
                if let Some(p) = self
                    .players
                    .iter_mut()
                    .flatten()
                    .find(|p| p.handle == handle)
                {
                    let c = &p.character;
                    if !c.combat.statuses.has(magicka_sim::status::Status::Greased)
                        && c.arc_intersect(at, query).is_some()
                    {
                        let _ = p.add_status_effect(
                            magicka_sim::status::StatusEffect::new(
                                magicka_sim::status::Status::Greased,
                                0.0,
                                1.0,
                                false,
                            ),
                            &mut events,
                        );
                    }
                } else if let Some(n) = self.npcs.get_mut(&handle) {
                    let c = &n.character;
                    if !c.combat.statuses.has(magicka_sim::status::Status::Greased)
                        && c.arc_intersect(at, query).is_some()
                    {
                        let _ = n.character.add_status_effect(
                            magicka_sim::status::StatusEffect::new(
                                magicka_sim::status::Status::Greased,
                                0.0,
                                1.0,
                                false,
                            ),
                            None,
                            &mut events,
                        );
                    }
                }
            }
            range -= magicka_sim::grease::FIELD_RADIUS;
            let Some(step) = spray_spawn_step(spray.ttl, after) else {
                continue;
            };
            if range <= 0.0 {
                continue;
            }
            let direction = yaw(spray_yaw(step));
            let probe = magicka_sim::spray_pulse::Segment {
                origin: [
                    position[0] + direction[0] * range,
                    position[1] + direction[1] * range + 2.0,
                    position[2] + direction[2] * range,
                ],
                delta: [0.0, -4.0, 0.0],
            };
            if let Some(hit) = self.collision.collision().segment_intersect(probe, false) {
                self.spawn_grease_field(
                    Some(spray.owner),
                    hit.hit.position,
                    hit.hit.normal,
                    hit.animated_owner,
                );
            }
        }
    }

    /// MeteorShower.Execute: refused indoors (global magick_fail);
    /// otherwise a new shower dims the light target to 0.4 (its rumble cue
    /// is guarded by `mRumble != null` and never starts), and an active one
    /// is refreshed to 17.5 s.
    pub fn meteor_shower(&mut self, owner: Option<u16>, position: [f32; 3]) -> bool {
        if self.scene_indoors {
            self.play_cue(16, "magick_fail", None);
            return false;
        }
        if self.ability_effects.meteor_shower(owner, position) {
            self.meteor_light = true;
            self.light_target_intensity = magicka_sim::magicks::METEOR_LIGHT_INTENSITY;
        }
        true
    }

    /// MeteorShower.OnRemove: the light target goes back to 1 (even under a
    /// Rain, as in the original).
    fn sync_meteor_shower(&mut self) {
        if self.meteor_light && self.ability_effects.meteor_shower_state().is_none() {
            self.meteor_light = false;
            self.light_target_intensity = 1.0;
        }
    }

    /// MeteorShower.Update once the spawn timer ran out (host only): the
    /// preblast cue, the timer += 0.4-0.8 s, then 1-2 Earth 5 + Fire 5
    /// projectiles 146 m above uniform points in the 20 m disc around the
    /// owner (or the cast position), falling at 20 m/s, splash 4, each with
    /// the preblast cue.
    fn meteors(&mut self, shower: magicka_sim::magicks::MeteorShower) {
        use magicka_sim::magicks::{METEOR_HEIGHT, METEOR_RADIUS, METEOR_SPLASH, METEOR_VELOCITY};
        use magicka_sim::spells::{Elements, Spell};
        self.play_cue(16, "magick_meteor_preblast", None);
        let center = shower
            .owner
            .and_then(|h| {
                self.players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == h)
                    .map(|p| p.body)
                    .or_else(|| self.npcs.get(&h).map(|n| n.body))
            })
            .map_or(shower.position, |b| self.physics.bodies[b].position);
        self.ability_effects.meteor_timer += 0.4 + self.random() * 0.4;
        let count = 1 + ((self.random() * 2.0) as usize).min(1);
        let mut spell = Spell {
            element: Elements(Elements::EARTH.0 | Elements::FIRE.0),
            mag: [0.0; 11],
            magick: Default::default(),
        };
        spell.mag[Elements::EARTH.index() as usize] = 5.0;
        spell.mag[Elements::FIRE.index() as usize] = 5.0;
        let mut launches = Vec::new();
        for _ in 0..count {
            let r = self.random().sqrt();
            let angle = self.random() * std::f32::consts::TAU;
            let position = [
                center[0] + METEOR_RADIUS * r * angle.cos(),
                center[1] + METEOR_HEIGHT,
                center[2] + METEOR_RADIUS * r * angle.sin(),
            ];
            launches.push((
                shower.owner.unwrap_or(0),
                magicka_sim::projectile_launch::ProjectileLaunch {
                    position,
                    velocity: METEOR_VELOCITY,
                    spell,
                    splash: METEOR_SPLASH,
                    total_projectiles: 1,
                },
            ));
            self.play_cue(16, "magick_meteor_preblast", Some(position));
        }
        self.spawn_missiles(launches);
    }

    /// Amalgameddon.Execute (the local authority): refused (global
    /// magick_fail) while one is active; otherwise the first pair, in
    /// entity order, of living characters within 20 m sharing a template
    /// (not the caster) loses gravity and starts rising; without a pair,
    /// magick_fail.
    pub fn amalgameddon(&mut self, owner: u16) -> bool {
        use magicka_sim::amalgameddon::{Amalgameddon, RANGE};
        use magicka_sim::magicks::AbilityEffect;
        if self
            .ability_effects
            .effects
            .iter()
            .any(|e| matches!(e, AbilityEffect::Amalgameddon(_)))
        {
            self.play_cue(16, "magick_fail", None);
            return false;
        }
        let Some(center) = self
            .combatants()
            .into_iter()
            .find(|c| c.handle == owner)
            .map(|c| c.position)
        else {
            return false;
        };
        let units: Vec<(u16, String)> = self
            .combatants()
            .into_iter()
            .filter(|c| {
                c.handle != owner
                    && !c.dead
                    && (0..3)
                        .map(|i| (c.position[i] - center[i]).powi(2))
                        .sum::<f32>()
                        <= (RANGE + c.radius).powi(2)
            })
            .filter_map(|c| {
                let id = self
                    .character_ref(c.handle)?
                    .definition
                    .id
                    .to_ascii_lowercase();
                Some((c.handle, id))
            })
            .collect();
        let pair = units.iter().find_map(|(a, ta)| {
            units
                .iter()
                .find(|(b, tb)| b != a && tb == ta)
                .map(|(b, _)| (*a, *b))
        });
        let Some((first, second)) = pair else {
            self.play_cue(16, "magick_fail", None);
            return false;
        };
        for h in [first, second] {
            if let Some(b) = self.character_body(h) {
                self.physics.bodies[b].apply_gravity = false;
            }
        }
        self.ability_effects
            .effects
            .push(AbilityEffect::Amalgameddon(Amalgameddon::new(
                owner, first, second,
            )));
        true
    }

    fn character_body(&self, handle: u16) -> Option<usize> {
        self.players
            .iter()
            .flatten()
            .find(|p| p.handle == handle)
            .map(|p| p.body)
            .or_else(|| self.npcs.get(&handle).map(|n| n.body))
            .or_else(|| self.boss_casters.get(&handle).map(|c| c.body))
    }

    /// Amalgameddon.Update: TTL; both units' gravity back below TTL 0; the
    /// rise/push/smash step moving the bodies (stunned while rising); the
    /// merge overkills the second unit and heals the first by its hit
    /// points (Damage(-life, Life): immortal units are skipped, invisibility
    /// ends, no cap), restoring its gravity and unstunning it.
    fn update_amalgameddon(&mut self, dt: f32) {
        use magicka_sim::amalgameddon::Step;
        use magicka_sim::magicks::AbilityEffect;
        let Some(index) = self
            .ability_effects
            .effects
            .iter()
            .position(|e| matches!(e, AbilityEffect::Amalgameddon(_)))
        else {
            return;
        };
        let AbilityEffect::Amalgameddon(mut a) = self.ability_effects.effects[index] else {
            return;
        };
        a.ttl -= dt;
        let bodies = (self.character_body(a.first), self.character_body(a.second));
        let (Some(fb), Some(sb)) = bodies else {
            self.ability_effects.effects.remove(index);
            return;
        };
        if a.ttl < 0.0 {
            self.physics.bodies[fb].apply_gravity = true;
            self.physics.bodies[sb].apply_gravity = true;
        }
        let second_hp = self
            .character_ref(a.second)
            .map_or(0.0, |c| c.combat.health.hit_points);
        let (f, s) = (&self.physics.bodies[fb], &self.physics.bodies[sb]);
        let step = a.step(
            dt,
            (f.position, f.radius),
            (s.position, s.radius),
            second_hp,
        );
        match step {
            Some(Step::Move {
                first,
                second,
                stun,
            }) => {
                self.physics.bodies[fb].position = first;
                self.physics.bodies[sb].position = second;
                if stun {
                    for h in [a.first, a.second] {
                        if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, h) {
                            c.combat.stun_timer = 10.0;
                        }
                    }
                }
            }
            Some(Step::Merge { happy, sorry, life }) => {
                self.overkill(sorry);
                if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, happy) {
                    if !c.is_immortal() {
                        c.combat.health.invisibility_timer = 0.0;
                        c.combat.health.hit_points += life;
                    }
                    c.combat.stun_timer = 0.001;
                }
                self.physics.bodies[fb].apply_gravity = true;
            }
            None => {}
        }
        if a.ttl <= 0.0 {
            self.ability_effects.effects.remove(index);
        } else {
            self.ability_effects.effects[index] = AbilityEffect::Amalgameddon(a);
        }
    }

    /// JudgementSpray.Update's missile: the target is the character within
    /// 20 m in front (angle of owner-minus-target to the facing above 90°)
    /// whose distance is closest to 12 m, with the original's signed bug
    /// (after a pick only |12 - d| below the previous signed difference
    /// replaces it). The missile alternates left/right: the facing with Y
    /// set to 1, normalized, crossed with Up or Down, times 15 + (0.5 +
    /// rand)·15; homing 0.55 on the target, radius 0.75 of the model's
    /// bounding sphere, not hitting its owner. It starts at the cast
    /// source (INFERRED stand-in for the left attach point).
    fn judgement_missile(&mut self, owner: u16) {
        let Some(p) = self.players.iter().flatten().find(|p| p.handle == owner) else {
            return;
        };
        let body = &self.physics.bodies[p.body];
        let (position, facing) = (body.position, body.direction());
        let orb = [p.staff_orb[12], p.staff_orb[13], p.staff_orb[14]];
        let source = if orb == [0.0; 3] { position } else { orb };
        let mut target = None;
        let mut best = f32::MAX;
        for c in self.combatants() {
            if c.handle == owner || c.dead {
                continue;
            }
            let d = (0..3)
                .map(|i| (c.position[i] - position[i]).powi(2))
                .sum::<f32>()
                .sqrt();
            if d > 20.0 + c.radius {
                continue;
            }
            let v = [
                position[0] - c.position[0],
                position[1] - c.position[1],
                position[2] - c.position[2],
            ];
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let v = v.map(|x| x / l);
            let dot = (v[0] * facing[0] + v[1] * facing[1] + v[2] * facing[2]).clamp(-1.0, 1.0);
            let angle = dot.acos();
            if angle < 3.926991 && angle > std::f32::consts::FRAC_PI_2 {
                let diff = 12.0 - d;
                if diff.abs() < best {
                    target = Some(c.handle);
                    best = diff;
                }
            }
        }
        let mut direction = [facing[0], 1.0, facing[2]];
        let l = (direction[0].powi(2) + direction[1].powi(2) + direction[2].powi(2)).sqrt();
        direction = direction.map(|x| x / l);
        self.ability_effects.judgement_left = !self.ability_effects.judgement_left;
        let side = if self.ability_effects.judgement_left {
            [direction[2], 0.0, -direction[0]]
        } else {
            [-direction[2], 0.0, direction[0]]
        };
        let speed = 15.0 + (0.5 + self.random()) * 15.0;
        let velocity = side.map(|x| x * speed);
        let Some(radius) = self
            .content
            .model_radius("Models/Missiles/JudgementSprayMissile")
            .map(|r| r * 0.75)
        else {
            self.log
                .push("judgement spray: missile model missing".into());
            return;
        };
        let id = self.next_missile;
        self.next_missile = self.next_missile.wrapping_add(1);
        let mut missile = crate::missiles::LiveMissile::spawn_custom(
            id,
            owner,
            source,
            velocity,
            radius,
            false,
            magicka_sim::missile_events::judgement_conditions(),
        );
        missile.body.homing = Some(magicka_sim::missile_homing::Homing::new(target, 0.55));
        let combatants = self.combatants();
        let Some(view) =
            crate::npc_combat::CombatView::new(&combatants, Some(self.collision.collision()))
        else {
            return;
        };
        let mut world = SceneHitWorld {
            physics_damage: &mut self.physics_damage,
            dispensers: &mut self.dispensers,
            entities: &mut self.entities,
            barriers: None,
            barrier_events: Vec::new(),
            view: &view,
            npcs: &mut self.npcs,
            players: &mut self.players,
            bodies: &mut self.physics.bodies,
            triggers: &mut self.pending_triggers,
            shields: &mut self.shields,
            collision_rng: &mut self.rng,
            play_time: self.play_time,
            entity_blasts: &mut self.entity_blasts,
            death: &mut self.death,
            jormungandr: &mut self.jormungandr,
            death_damage_features: self.role != NetworkRole::Client,
        };
        crate::missiles::initialize(&mut missile, &mut world, &mut self.missile_visuals);
        self.register_missile(missile);
    }

    /// PerformanceEnchantment.Execute for a character: a new enchantment
    /// adds its self aura (all-element resistance ×0.5 for 10 s; the ×2
    /// BoostDamage aura is not delivered yet) and plays the buff cue; a
    /// recast only refreshes it to 10 s and re-boosts (cue again, no new
    /// auras). The `special_woot_performance` effect is presentation.
    pub fn performance_enchantment(&mut self, owner: u16) -> bool {
        const BANK_ADDITIONAL: u16 = 0x200;
        let Some(position) = self
            .combatants()
            .into_iter()
            .find(|c| c.handle == owner)
            .map(|c| c.position)
        else {
            return false;
        };
        if self.ability_effects.performance(owner) {
            if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, owner) {
                c.auras.push(crate::character::ActiveAura {
                    resistance: magicka_sim::resistance::Resistance {
                        against: magicka_sim::spells::Elements::ALL,
                        multiplier: 0.5,
                        modifier: 0.0,
                        status_resistance: false,
                    },
                    ttl: magicka_sim::magicks::PERFORMANCE_TTL,
                    start_ttl: magicka_sim::magicks::PERFORMANCE_TTL,
                    self_casted: true,
                    self_only: true,
                    // AuraTarget.Self, radius 0.
                    visual: crate::buffs::AuraVisual::new(
                        0.0,
                        crate::buffs::immunity_color(magicka_sim::spells::Elements::SHIELD),
                    ),
                });
            }
        }
        self.play_cue(
            BANK_ADDITIONAL,
            "woot_magick_enchantment_buff",
            Some(position),
        );
        true
    }

    /// PerformanceEnchantment withdrawal cues and IsDead's ClearAura.
    fn performance_events(&mut self) {
        const BANK_ADDITIONAL: u16 = 0x200;
        for owner in std::mem::take(&mut self.ability_effects.performance_withdrawn) {
            let at = self
                .combatants()
                .into_iter()
                .find(|c| c.handle == owner)
                .map(|c| c.position);
            self.play_cue(BANK_ADDITIONAL, "woot_magick_enchantment_debuff", at);
        }
        for owner in std::mem::take(&mut self.ability_effects.performance_ended) {
            if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, owner) {
                for aura in c.auras.iter_mut().filter(|a| a.self_casted) {
                    aura.ttl = 0.0;
                }
            }
        }
    }

    /// Zap.Execute (ChainLightning): LightningBolt.Cast from the cast source
    /// along the facing with 850 Lightning damage, range 15, twice the
    /// lightning colour and bolt scale 2; the caster is hit-listed unless
    /// Wet. Then the thunderbolt cue at the caster and a 1.5/0.333 shake.
    /// The bolt's shield test is not modelled for this host (characters
    /// only, as for barrier lightning).
    pub fn chain_lightning(&mut self, owner: u16) -> bool {
        let Some(p) = self.players.iter().flatten().find(|p| p.handle == owner) else {
            return false;
        };
        let body = &self.physics.bodies[p.body];
        let orb = [p.staff_orb[12], p.staff_orb[13], p.staff_orb[14]];
        let source = if orb == [0.0; 3] { body.position } else { orb };
        let (direction, position) = (body.direction(), body.position);
        let wet = p
            .character
            .combat
            .statuses
            .has(magicka_sim::status::Status::Wet);
        self.special_bolt(owner, source, direction, !wet, 850.0, 15.0);
        self.play_cue(16, "magick_thunderbolt", Some(position));
        self.camera.shake(1.5, 0.333);
        true
    }

    /// ProppMagick.Execute (the local authority): a plasma ball lobbed from
    /// 0.5 m above the cast source, the facing turned by up to ±45°,
    /// vertical speed 18 and horizontal speed sqrt(10..28), radius 0.6, not
    /// hitting its owner, with the ProppMagick conditions and the followed
    /// `magick_propp_spell` effect. The random spin and lever only feed the
    /// render (not reconstructed) but their RNG draws are kept.
    pub fn propp_magick(&mut self, owner: u16) -> bool {
        let Some(p) = self.players.iter().flatten().find(|p| p.handle == owner) else {
            return false;
        };
        let body = &self.physics.bodies[p.body];
        let orb = [p.staff_orb[12], p.staff_orb[13], p.staff_orb[14]];
        let mut position = if orb == [0.0; 3] { body.position } else { orb };
        position[1] += 0.5;
        let facing = body.direction();
        let turn = (self.random() - 0.5) * std::f32::consts::FRAC_PI_2;
        let speed_sq = 10.0 + self.random() * 18.0;
        let (s, c) = turn.sin_cos();
        let direction = [
            facing[0] * c + facing[2] * s,
            facing[1],
            -facing[0] * s + facing[2] * c,
        ];
        let velocity = [
            direction[0] * speed_sq.sqrt(),
            18.0,
            direction[2] * speed_sq.sqrt(),
        ];
        for _ in 0..4 {
            self.random();
        }
        let id = self.next_missile;
        self.next_missile = self.next_missile.wrapping_add(1);
        let mut missile = crate::missiles::LiveMissile::spawn_custom(
            id,
            owner,
            position,
            velocity,
            0.6,
            false,
            magicka_sim::missile_events::propp_conditions(),
        );
        let combatants = self.combatants();
        let Some(view) =
            crate::npc_combat::CombatView::new(&combatants, Some(self.collision.collision()))
        else {
            return false;
        };
        let mut world = SceneHitWorld {
            physics_damage: &mut self.physics_damage,
            dispensers: &mut self.dispensers,
            entities: &mut self.entities,
            barriers: None,
            barrier_events: Vec::new(),
            view: &view,
            npcs: &mut self.npcs,
            players: &mut self.players,
            bodies: &mut self.physics.bodies,
            triggers: &mut self.pending_triggers,
            shields: &mut self.shields,
            collision_rng: &mut self.rng,
            play_time: self.play_time,
            entity_blasts: &mut self.entity_blasts,
            death: &mut self.death,
            jormungandr: &mut self.jormungandr,
            death_damage_features: self.role != NetworkRole::Client,
        };
        crate::missiles::initialize(&mut missile, &mut world, &mut self.missile_visuals);
        self.missile_visuals
            .push(crate::missiles::MissileVisual::Effect {
                missile: id,
                name: "magick_propp_spell",
                position,
                direction: [0.0, 0.0, -1.0],
                follow: true,
            });
        self.register_missile(missile);
        true
    }

    /// TractorPull.Execute: every IDamageable within 15 m (protected ones
    /// ignored; not the caster, not ethereal characters) takes an Earth
    /// Pushed damage with its body mass as the amount and magnitude
    /// 2.5·sqrt(d/15), attacked from the far side so it is pulled towards
    /// the centre. Then a 1/0.5 camera shake and `spell_push_area`
    /// (magnitude 1) at the caster.
    pub fn tractor_pull(&mut self, caster: Option<u16>, center: [f32; 3]) {
        self.tractor(caster, center, false);
    }

    /// TractorPull (towards the centre, magnitude 2.5·sqrt(d/15), attacked
    /// from the far side) or TractorPush (away, 1.5·sqrt(15/d), attacked
    /// from the centre's XZ); both then shake and play spell_push_area.
    pub(super) fn tractor(&mut self, caster: Option<u16>, center: [f32; 3], push: bool) {
        use magicka_sim::damage::{attack, Damage};
        const RANGE: f32 = 15.0;
        let spheres: Vec<_> = self
            .shields
            .iter()
            .filter(|s| {
                !s.shield.dead && s.shield.kind == magicka_sim::shield_entity::ShieldKind::Sphere
            })
            .map(|s| (s.shield.position, s.shield.radius))
            .collect();
        let protected = |p: [f32; 3]| {
            spheres
                .iter()
                .any(|(c, r)| (0..3).map(|i| (p[i] - c[i]).powi(2)).sum::<f32>() < r * r)
        };
        let targets: Vec<(u16, [f32; 3], f32)> = self
            .combatants()
            .into_iter()
            .filter(|t| {
                Some(t.handle) != caster
                    && !t.dead
                    && !t.ethereal
                    && !protected(t.position)
                    && (0..3)
                        .map(|i| (t.position[i] - center[i]).powi(2))
                        .sum::<f32>()
                        <= (RANGE + t.radius).powi(2)
            })
            .filter_map(|t| {
                let body = self
                    .players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == t.handle)
                    .map(|p| p.body)
                    .or_else(|| self.npcs.get(&t.handle).map(|n| n.body))?;
                Some((t.handle, t.position, self.physics.bodies[body].mass))
            })
            .collect();
        let timestamp = self.play_time;
        for (handle, position, mass) in targets {
            let d = [
                center[0] - position[0],
                center[1] - position[1],
                center[2] - position[2],
            ];
            let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if distance <= f32::from_bits(1) {
                continue;
            }
            let (magnitude, side) = if push {
                (1.5 * (RANGE / distance).sqrt(), 1.0)
            } else {
                (2.5 * (distance / RANGE).sqrt(), -1.0)
            };
            let damage = Damage {
                attack: attack::PUSHED,
                element: magicka_sim::spells::Elements::EARTH.0,
                amount: mass,
                magnitude,
            };
            let source = [
                position[0] + side * d[0],
                position[1],
                position[2] + side * d[2],
            ];
            self.hit_character_at(handle, damage, caster, source, timestamp);
        }
        self.camera.shake(1.0, 0.5);
        let emitter = caster
            .and_then(|h| self.combatants().into_iter().find(|c| c.handle == h))
            .map_or(center, |c| c.position);
        self.play_cue_magnitude(16, "spell_push_area", Some(emitter), Some(1.0));
    }

    /// CTD.Execute (the local authority): among characters within 20 m
    /// (Y ignored, protected ones skipped; the caster and allies included)
    /// that can die normally and have at most 100000 maximum hit points, one
    /// at random is Terminated with the `magick_ctd` effect and cue
    /// (Additional bank). Without one, magick_fail at the caster.
    pub fn ctd(&mut self, owner: u16) -> bool {
        const BANK_ADDITIONAL: u16 = 0x200;
        let Some(c) = self.combatants().into_iter().find(|c| c.handle == owner) else {
            return false;
        };
        let spheres: Vec<_> = self
            .shields
            .iter()
            .filter(|s| {
                !s.shield.dead && s.shield.kind == magicka_sim::shield_entity::ShieldKind::Sphere
            })
            .map(|s| (s.shield.position, s.shield.radius))
            .collect();
        let candidates: Vec<(u16, [f32; 3])> = self
            .combatants()
            .into_iter()
            .filter(|t| {
                let flat = (t.position[0] - c.position[0]).powi(2)
                    + (t.position[2] - c.position[2]).powi(2);
                let protected = spheres.iter().any(|(p, r)| {
                    (0..3).map(|i| (t.position[i] - p[i]).powi(2)).sum::<f32>() < r * r
                }) && !spheres.iter().any(|(p, r)| {
                    (0..3).map(|i| (c.position[i] - p[i]).powi(2)).sum::<f32>() < r * r
                });
                !t.dead && flat <= (20.0 + t.radius).powi(2) && !protected
            })
            .filter(|t| {
                self.character_ref(t.handle).is_some_and(|r| {
                    !r.combat.cannot_die_without_explicit_kill
                        && r.combat.health.max_hit_points <= 100000.0
                })
            })
            .map(|t| (t.handle, t.position))
            .collect();
        if candidates.is_empty() {
            self.play_cue(16, "magick_fail", Some(c.position));
            return false;
        }
        let pick = ((self.random() * candidates.len() as f32) as usize).min(candidates.len() - 1);
        let (target, position) = candidates[pick];
        self.mine_effects
            .push(("magick_ctd", position, [0.0, 0.0, -1.0]));
        self.play_cue(BANK_ADDITIONAL, "magick_ctd", Some(position));
        if self.role != NetworkRole::Client {
            self.terminate_character(target);
        }
        true
    }

    /// Character.Terminate(iKillItems: true, …): dead at once with
    /// mDeadTimer -100 (an NPC is removed on its next update), hit points 0,
    /// statuses and charm ended; an avatar then Dies. Every caller except
    /// DeadState, Polymorph and network replication kills the equipment
    /// (Item.Kill nulls Model, so a later Attachment.Release drops nothing).
    /// Pickup copies DeadState.OnEnter already released are separate
    /// entities and stay.
    pub fn terminate_character(&mut self, handle: u16) {
        if let Some(n) = self.npcs.get_mut(&handle) {
            let slots: Vec<i32> = n.weapons.iter().map(|(s, _)| *s).collect();
            n.weapons.clear();
            for slot in slots {
                n.character.block_values.remove(&slot);
            }
            if let Some(rig) = n.rig.as_mut() {
                rig.attachments.clear();
            }
            n.held_effects.clear();
        }
        self.terminate_character_keeping_items(handle);
    }

    /// Character.Terminate(iKillItems: false, …) (Polymorph): equipment
    /// stays for the ordinary release path.
    pub fn terminate_character_keeping_items(&mut self, handle: u16) {
        if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == handle)
        {
            p.terminate();
            return;
        }
        if let Some(n) = self.npcs.get_mut(&handle) {
            let c = &mut n.character.combat;
            c.dead = true;
            c.health.hit_points = 0.0;
            c.death_status_mask = c.statuses.current_mask;
            let mut events = Vec::new();
            c.statuses.stop_effects(u16::MAX, &mut events);
            n.character.pending_status_audio.extend(events);
            n.character.charm = None;
            n.motion.dead_timer = -100.0;
            self.log.push(format!("terminate {handle}"));
        }
    }

    /// NonPlayerCharacter.Deinitialize's OnDespawnedSummon on the master.
    fn despawn_summon(&mut self, npc: u16) {
        let Some((master, _)) = self.npcs.get(&npc).and_then(|n| n.summon) else {
            return;
        };
        let overkill = self
            .summons
            .get_mut(&master)
            .map(|l| l.on_despawned(npc))
            .unwrap_or_default();
        for handle in overkill {
            self.overkill(handle);
        }
    }

    /// Spawn a summoned NPC from `Data/Characters/<template>` with its feet at
    /// `position` facing `direction`: NonPlayerCharacter.Initialize,
    /// Summoned(master) when there is one (the master's faction, its summon
    /// list making room first), ForceAnimation(spawn) and the given order.
    /// RessurectionState is not reconstructed (the spawn clip is forced).
    pub fn summon_npc(
        &mut self,
        master: Option<u16>,
        template: &str,
        position: [f32; 3],
        direction: [f32; 3],
        kind: magicka_sim::summons::SummonKind,
        order: crate::spawn_action::SpawnOrder<'_>,
    ) -> Option<u16> {
        self.summon_npc_animation(master, template, position, direction, kind, order, 170)
    }

    fn summon_npc_animation(
        &mut self,
        master: Option<u16>,
        template: &str,
        position: [f32; 3],
        direction: [f32; 3],
        kind: magicka_sim::summons::SummonKind,
        order: crate::spawn_action::SpawnOrder<'_>,
        spawn_animation: u16,
    ) -> Option<u16> {
        use crate::spawn_action::{SpawnHost, SpawnLevel};
        self.load_character_template(Some(template)).ok()?;
        let type_id = custom_hash(template);
        let handle = self.acquire_npc().ok()?.handle;
        self.initialize_npc(handle, type_id, 0, position, 0).ok()?;
        let up = [0.0, 1.0, 0.0];
        let back = direction.map(|v| -v);
        let right = [
            up[1] * back[2] - up[2] * back[1],
            up[2] * back[0] - up[0] * back[2],
            up[0] * back[1] - up[1] * back[0],
        ];
        let orientation = [
            right[0], right[1], right[2], 0.0, up[0], up[1], up[2], 0.0, back[0], back[1], back[2],
            0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        self.configure(
            handle,
            &crate::spawn_action::SpawnConfig {
                hit_points: None,
                on_death: 0,
                on_damage: 0,
                wander_speed: None,
                target_area: None,
                order,
                dialog: 0,
                orientation,
                desired_direction: direction,
                remove_after_death: true,
                force_camera: None,
                force_nav_mesh: None,
                cannot_die_without_explicit_kill: None,
            },
        )
        .ok()?;
        if let Some(master) = master {
            let faction = self
                .entities
                .iter()
                .find(|e| e.handle == master)
                .map(|e| self.faction(e));
            let overkill = self
                .summons
                .entry(master)
                .or_default()
                .on_spawned(handle, kind);
            if let Some(n) = self.npcs.get_mut(&handle) {
                n.summon = Some((master, kind));
                n.faction_override = faction;
            }
            for h in overkill {
                self.overkill(h);
            }
        }
        // Animations.spawn (170).
        let _ = self.force_animation(handle, spawn_animation);
        crate::spawn_action::SpawnHost::add_entity(self, handle).ok()?;
        Some(handle)
    }

    /// SummonUndead.Execute(owner) (host only): 4 m ahead, the navmesh point
    /// nearest 3 m above it (MovementProperties.Default), dropped onto the
    /// scene within 10 m; one of the ghoul, friendly wight and three darksoul
    /// templates at random, an undead summon of the caster, ordered to Attack
    /// reacting to Attack and Proximity.
    pub fn summon_undead(&mut self, owner: u16) -> Option<u16> {
        use crate::npc::{ORDER_ATTACK, REACT_ATTACK, REACT_PROXIMITY};
        const TEMPLATES: [&str; 5] = [
            "ghoul",
            "skeleton_wight_friendly",
            "skeleton_darksoul_arcane",
            "skeleton_darksoul_frost",
            "skeleton_darksoul_lightning",
        ];
        if self.role == NetworkRole::Client {
            return None;
        }
        let p = self.players.iter().flatten().find(|p| p.handle == owner)?;
        let body = &self.physics.bodies[p.body];
        let (origin, direction) = (body.position, body.direction());
        let position = self.summon_point([
            origin[0] + direction[0] * 4.0,
            origin[1] + direction[1] * 4.0,
            origin[2] + direction[2] * 4.0,
        ]);
        let template = TEMPLATES[((self.random() * 5.0) as usize).min(4)];
        let events = Vec::new();
        self.summon_npc(
            Some(owner),
            template,
            position,
            direction,
            magicka_sim::summons::SummonKind::Undead,
            crate::spawn_action::SpawnOrder {
                order: ORDER_ATTACK,
                react_to: REACT_ATTACK | REACT_PROXIMITY,
                reaction: ORDER_ATTACK,
                priority_target: 0,
                priority_ability: 0,
                trigger: 0,
                events: &events,
            },
        )
        .inspect(|handle| {
            // aI.AlertRadius = 12.
            if let Some(n) = self.npcs.get_mut(handle) {
                n.weights.alert_radius = 12.0;
            }
        })
    }

    /// SummonUndead's placement: the navmesh point (MovementProperties.Default)
    /// nearest 3 m above `at`, dropped onto the scene within 10 m. Also the
    /// mod `Entities.Summon` placement.
    pub(crate) fn summon_point(&self, at: [f32; 3]) -> [f32; 3] {
        let ahead = [at[0], at[1] + 3.0, at[2]];
        let mut position = self
            .nav
            .as_ref()
            .and_then(|nav| nav.nearest(ahead, magicka_sim::navmesh::movement::DEFAULT))
            .map_or(ahead, |(_, point, _)| point);
        let probe = magicka_sim::spray_pulse::Segment {
            origin: position,
            delta: [0.0, -10.0, 0.0],
        };
        if let Some(hit) = self.collision.collision().segment_intersect(probe, false) {
            position = hit.hit.position;
        }
        position
    }

    /// Napalm.Execute: refused indoors (magick_fail). While a strike is
    /// waiting, a recast only moves the target 15 m ahead (the track keeps
    /// its direction); during the airstrike it fails. Otherwise a new
    /// singleton starts its 5 s wait. The smoke target is presentation.
    pub fn napalm(&mut self, owner: u16, position: [f32; 3], facing: [f32; 3]) -> bool {
        use magicka_sim::napalm::{Napalm, TARGET_DISTANCE};
        if self.scene_indoors {
            self.play_cue(16, "magick_fail", Some(position));
            return false;
        }
        if let Some(n) = self.ability_effects.napalm_mut() {
            if n.airstrike {
                self.play_cue(16, "magick_fail", Some(position));
                return false;
            }
            n.owner = owner;
            n.target = [0, 1, 2].map(|i| position[i] + facing[i] * TARGET_DISTANCE);
            return true;
        }
        self.ability_effects
            .effects
            .push(magicka_sim::magicks::AbilityEffect::Napalm(Napalm::new(
                owner, position, facing,
            )));
        true
    }

    /// Napalm.Update for the singleton (then removal once dead).
    fn update_napalm(&mut self, dt: f32) {
        use magicka_sim::napalm::NapalmEvent;
        const BANK_ADDITIONAL: u16 = 0x200;
        let host = self.role != NetworkRole::Client;
        let mut events = Vec::new();
        let Some(n) = self.ability_effects.napalm_mut() else {
            return;
        };
        let dead = n.update(dt, host, &mut events);
        let owner = n.owner;
        if dead {
            self.ability_effects
                .effects
                .retain(|e| !matches!(e, magicka_sim::magicks::AbilityEffect::Napalm(_)));
        }
        for event in events {
            match event {
                NapalmEvent::PlanePass => {
                    self.camera.shake(0.5, 2.0);
                    self.play_cue(BANK_ADDITIONAL, "magick_plane_passby", None);
                }
                NapalmEvent::Strike(point) => self.napalm_strike(owner, point),
            }
        }
    }

    /// One airstrike hit: a 20 m vertical probe (scene, else a liquid's ice
    /// surface, water ignored). On a hit: `magick_napalm` facing up, the
    /// global cue, a 0.25/0.5 shake, a grease field lit with Burn(3) (not on
    /// a liquid) and NapalmBlast. mTimeStamp is never set by Napalm (0).
    fn napalm_strike(&mut self, owner: u16, point: [f32; 3]) {
        const BANK_ADDITIONAL: u16 = 0x200;
        let segment = magicka_sim::spray_pulse::Segment {
            origin: [point[0], point[1] + 10.0, point[2]],
            delta: [0.0, -20.0, 0.0],
        };
        let scene = self
            .collision
            .collision()
            .segment_intersect(segment, false)
            .map(|h| (h.hit.position, h.animated_owner, false));
        let hit = scene.or_else(|| {
            self.collision
                .collision()
                .liquid_segment_intersect(segment, true, true, false)
                .map(|h| (h.position, None, true))
        });
        let Some((position, animated, liquid)) = hit else {
            return;
        };
        let up = [0.0, 1.0, 0.0];
        self.mine_effects.push(("magick_napalm", position, up));
        self.play_cue(BANK_ADDITIONAL, "magick_napalm", None);
        self.camera.shake(0.25, 0.5);
        if !liquid {
            let slot = self.spawn_grease_field(Some(owner), position, up, animated);
            if let Some(f) = self.grease.get_mut(slot) {
                f.field.burn(3.0);
            }
        }
        self.napalm_blast(position, owner);
    }

    /// Napalm.NapalmBlast: every IDamageable within 5 m (protected ones
    /// ignored, Y counted) except ethereal characters takes the napalm
    /// collection, the caster included; then Liquid.Freeze with the Fire
    /// status damage (3.75 m arc start, full circle, multiplier 10).
    fn napalm_blast(&mut self, position: [f32; 3], owner: u16) {
        use magicka_sim::napalm::BLAST_RADIUS;
        let damages = magicka_sim::napalm::damage();
        let within = |p: [f32; 3], r: f32| {
            (0..3).map(|i| (p[i] - position[i]).powi(2)).sum::<f32>() <= (BLAST_RADIUS + r).powi(2)
        };
        let spheres: Vec<_> = self
            .shields
            .iter()
            .filter(|s| {
                !s.shield.dead && s.shield.kind == magicka_sim::shield_entity::ShieldKind::Sphere
            })
            .map(|s| (s.shield.position, s.shield.radius))
            .collect();
        let protected = |p: [f32; 3]| {
            spheres
                .iter()
                .any(|(c, r)| (0..3).map(|i| (p[i] - c[i]).powi(2)).sum::<f32>() < r * r)
        };
        let characters: Vec<u16> = self
            .combatants()
            .into_iter()
            .filter(|c| {
                !c.dead && !c.ethereal && within(c.position, c.radius) && !protected(c.position)
            })
            .map(|c| c.handle)
            .collect();
        self.with_rail_world(|world| {
            for handle in characters {
                world.deliver_collection(handle, Some(owner), damages, 0.0, position);
            }
        });
        let shields: Vec<u16> = self
            .shields
            .iter()
            .filter(|s| !s.shield.dead && within(s.shield.position, s.shield.radius))
            .map(|s| s.handle)
            .collect();
        for handle in shields {
            for d in damages.iter().filter(|d| d.element != 0) {
                self.damage_shield_handle(handle, *d);
            }
        }
        let fields: Vec<usize> = self
            .grease
            .iter()
            .filter(|(_, f)| within(f.position, magicka_sim::grease::FIELD_RADIUS))
            .map(|(slot, _)| slot)
            .collect();
        for slot in fields {
            self.damage_grease(slot, &damages);
        }
        let mines: Vec<usize> = self
            .mines
            .iter()
            .filter(|(_, m)| within(m.position, magicka_sim::spell_mine::RADIUS))
            .map(|(slot, _)| slot)
            .collect();
        for slot in mines {
            if let Some(m) = self.mines.get_mut(slot) {
                m.internal_damage(&damages);
            }
        }
        let barriers: Vec<usize> = self
            .barriers
            .iter()
            .filter(|(_, b)| within(b.position, b.radius))
            .map(|(slot, _)| slot)
            .collect();
        for slot in barriers {
            if let Some(b) = self.barriers.get_mut(slot) {
                let (_, events) = b.internal_damage(&damages, true);
                self.barrier_events(slot, events);
            }
        }
        self.freeze_water(
            position,
            [3.75, 0.0, 0.0],
            std::f32::consts::TAU,
            10.0,
            &[damages[2]],
        );
    }

    /// GreaseSplash.Execute (host only): seven fields at 1-4 m around the
    /// owner, each turning the running direction by a further PI/7·i (the
    /// original reuses the rotated vector), all at Y = 0 with an up normal.
    pub fn grease_splash(&mut self, owner: u16) -> bool {
        use magicka_sim::grease::{splash_yaw, SPLASH_FIELDS, SPLASH_RANGE};
        if self.role == NetworkRole::Client {
            return false;
        }
        let Some(p) = self.players.iter().flatten().find(|p| p.handle == owner) else {
            return false;
        };
        let body = &self.physics.bodies[p.body];
        let (position, mut direction) = (body.position, body.direction());
        for i in 0..SPLASH_FIELDS {
            let (s, c) = splash_yaw(i).sin_cos();
            direction = [
                direction[0] * c + direction[2] * s,
                direction[1],
                -direction[0] * s + direction[2] * c,
            ];
            let distance = 1.0 + self.random() * SPLASH_RANGE;
            let mut at = [
                position[0] + direction[0] * distance,
                0.0,
                position[2] + direction[2] * distance,
            ];
            at[1] = 0.0;
            self.spawn_grease_field(Some(owner), at, [0.0, 1.0, 0.0], None);
        }
        true
    }

    /// Etherealize.Execute for a character: Ethereal(true) with no timer
    /// (it lasts until Corporealize), the `magick_etherealize` effect at the
    /// caster and the `chr_daemon_etherealize3` cue (Characters bank).
    pub fn etherealize(&mut self, handle: u16) -> bool {
        const BANK_CHARACTERS: u16 = 0x20;
        let body = self
            .players
            .iter()
            .flatten()
            .find(|p| p.handle == handle)
            .map(|p| p.body)
            .or_else(|| self.npcs.get(&handle).map(|n| n.body));
        let Some(body) = body else {
            return false;
        };
        let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, handle) else {
            return false;
        };
        c.set_ethereal(true);
        let (position, direction) = (
            self.physics.bodies[body].position,
            self.physics.bodies[body].direction(),
        );
        self.mine_effects
            .push(("magick_etherealize", position, direction));
        self.play_cue(BANK_CHARACTERS, "chr_daemon_etherealize3", Some(position));
        true
    }

    /// Corporealize.Execute(owner): the global `magick_corporealize` cue, a
    /// 0.2 s flash, then IsEthereal = false on every living character within
    /// 30 m of the owner (protected ones included), including MagickDeath.
    /// Boss damage zones remain unported.
    pub fn corporealize(&mut self, position: [f32; 3]) {
        use magicka_sim::entity_query::{EntityQueryGrid, QueryEntity};
        const RADIUS: f32 = 30.0;
        self.corporealize_grimnir2(position);
        self.play_cue(16, "magick_corporealize", None);
        self.flash(0.2);
        if let Some(actor) = self.death.actor.as_mut().filter(|a| !a.state.dead) {
            let distance_squared: f32 = (0..3)
                .map(|i| (actor.state.position[i] - position[i]).powi(2))
                .sum();
            // Native MagickDeath inherits Entity's zero query radius.
            let reach = RADIUS;
            if distance_squared <= reach * reach {
                actor.state.ethereal = false;
            }
        }
        let characters = self.combatants();
        let query: Vec<_> = characters
            .iter()
            .map(|c| QueryEntity {
                position: c.position,
                radius: c.radius,
                dead: c.dead,
                shield: false,
                damageable: true,
            })
            .collect();
        let found = EntityQueryGrid::build(&query)
            .ok()
            .and_then(|g| {
                g.query(&query, position, RADIUS, false, false, |_, _| {
                    (false, [0.0; 3])
                })
                .ok()
            })
            .unwrap_or_default();
        for i in found {
            if let Some(c) =
                Self::character_mut(&mut self.players, &mut self.npcs, characters[i].handle)
            {
                c.ethereal = false;
            }
        }
    }

    /// Levitate.CastLevitate for any character: the haste cue at it, then
    /// SetLevitate(30): gravity off while the timer runs.
    pub fn levitate(&mut self, handle: u16) -> bool {
        let body = self
            .players
            .iter()
            .flatten()
            .find(|p| p.handle == handle)
            .map(|p| p.body)
            .or_else(|| self.npcs.get(&handle).map(|n| n.body));
        let Some(body) = body else {
            return false;
        };
        self.play_cue(16, "magick_haste", Some(self.physics.bodies[body].position));
        let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, handle) else {
            return false;
        };
        if c.levitation.set(magicka_sim::magicks::LEVITATE_TTL) {
            let dead = c.combat.dead;
            let b = &mut self.physics.bodies[body];
            b.apply_gravity = false;
            b.owner.levitating = c.levitation.active(dead);
        }
        true
    }

    /// Character.Update's levitation block for every player (`npcs` false)
    /// or NPC: while the timer runs, a 0.75 m probe below the feet (liquids
    /// first, then the scene) lifts the body towards hovering, or it sinks
    /// faster the longer it finds nothing; StopLevitate at the end. Also
    /// mirrors IsLevitating into the body.
    fn update_levitation(&mut self, dt: f32, npcs: bool) {
        use magicka_sim::magicks::Levitation;
        let collision = self.collision.collision();
        let bodies = &mut self.physics.bodies;
        let characters: Vec<(usize, &mut crate::character::RuntimeCharacter)> = if npcs {
            self.npcs
                .values_mut()
                .map(|n| (n.body, &mut n.character))
                .collect()
        } else {
            self.players
                .iter_mut()
                .flatten()
                .map(|p| (p.body, &mut p.character))
                .collect()
        };
        for (body, c) in characters {
            let b = &mut bodies[body];
            if c.levitation.timer > 0.0 {
                let (origin, delta) = Levitation::probe(b.position, b.height_offset());
                let segment = magicka_sim::spray_pulse::Segment { origin, delta };
                let hit = collision
                    .liquid_segment_intersect(segment, false, false, false)
                    .map(|h| h.fraction)
                    .or_else(|| {
                        collision
                            .segment_intersect(segment, false)
                            .map(|h| h.hit.fraction)
                    });
                let (dv, ended) = c.levitation.tick(dt, hit);
                b.velocity[1] += dv;
                if ended {
                    b.apply_gravity = true;
                }
            }
            b.owner.levitating = c.levitation.active(c.combat.dead);
        }
    }

    /// Confuse.Execute for any character caster: the living character within
    /// 14 m (protected ones ignored, any faction, charmed or not) closest to
    /// the caster's facing is confused. Without one the magick_fail cue plays.
    pub fn confuse_from(&mut self, caster: u16) -> bool {
        use magicka_sim::magicks::CONFUSE_RADIUS;
        let Some((origin, best)) = self.facing_target(caster, CONFUSE_RADIUS, |_, _, _| true)
        else {
            return false;
        };
        let Some((target, _)) = best else {
            self.play_cue(16, "magick_fail", Some(origin));
            return false;
        };
        self.confuse_character(origin, target)
    }

    /// Confuse.Execute(owner, target): an already confused target is
    /// refreshed to 15 s (cue at the owner). Otherwise a local player's
    /// controller is inverted, or an NPC takes faction NONE with its AI
    /// popped to idle; the cue plays on the target. Characters only.
    pub fn confuse_character(&mut self, owner_position: [f32; 3], target: u16) -> bool {
        const BANK_CHARACTERS: u16 = 0x20;
        let position = if let Some(p) = self.players.iter().flatten().find(|p| p.handle == target) {
            self.physics.bodies[p.body].position
        } else if let Some(n) = self.npcs.get(&target) {
            self.physics.bodies[n.body].position
        } else {
            return false;
        };
        if !self.ability_effects.confuse(target) {
            self.play_cue(BANK_CHARACTERS, "boss_fafnir_confuse", Some(owner_position));
            return true;
        }
        // Network gamers' avatars are not inverted; all rewrite players are
        // local. The server's TriggerActionMessage is not reconstructed.
        if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == target)
        {
            p.controls_inverted = true;
        } else if let Some(n) = self.npcs.get_mut(&target) {
            n.faction_override = Some(0);
            n.charm_reset = true;
        }
        self.play_cue(BANK_CHARACTERS, "boss_fafnir_confuse", Some(position));
        true
    }

    /// Confuse.OnRemove for the effects that ended: the player's controller
    /// is restored, and an NPC goes back to its template faction with its AI
    /// popped to idle.
    fn end_confusions(&mut self) {
        for target in std::mem::take(&mut self.ability_effects.confuse_ended) {
            if let Some(p) = self
                .players
                .iter_mut()
                .flatten()
                .find(|p| p.handle == target)
            {
                p.controls_inverted = false;
            } else if let Some(n) = self.npcs.get_mut(&target) {
                n.faction_override = None;
                n.charm_reset = true;
            }
        }
    }

    /// Fear.Execute(ISpellCaster) for any character caster (players now,
    /// NPC casters later): effect at the caster, then every other living
    /// character within 6 m that shares no faction bit with the caster and
    /// has no non-solid self shield is feared by the caster.
    pub fn fear_from(&mut self, caster: u16) -> bool {
        let Some(c) = self.combatants().into_iter().find(|c| c.handle == caster) else {
            return false;
        };
        self.mine_effects
            .push(("magick_fear", c.position, [1.0, 0.0, 0.0]));
        let source = magicka_sim::fear::FearSource::Character(caster);
        for handle in self.fear_targets(c.position, Some((caster, c.faction))) {
            self.fear_character(handle, source);
        }
        self.play_cue(16, "magick_fear", Some(c.position));
        true
    }

    /// Fear.Execute(Vector3): the position overload (no faction filter, the
    /// point is the fear source).
    pub fn fear_at(&mut self, position: [f32; 3]) {
        self.mine_effects
            .push(("magick_fear", position, [1.0, 0.0, 0.0]));
        let source = magicka_sim::fear::FearSource::Position(position);
        for handle in self.fear_targets(position, None) {
            self.fear_character(handle, source);
        }
        self.play_cue(16, "magick_fear", Some(position));
    }

    /// EntityManager.GetEntities(position, RANGE, ignoreProtected) over
    /// characters, then Fear's filter. `caster` = (handle, faction).
    fn fear_targets(&self, position: [f32; 3], caster: Option<(u16, i32)>) -> Vec<u16> {
        use magicka_sim::entity_query::{EntityQueryGrid, QueryEntity};
        let characters = self.combatants();
        let query: Vec<_> = characters
            .iter()
            .map(|c| QueryEntity {
                position: c.position,
                radius: c.radius,
                dead: c.dead,
                shield: false,
                damageable: true,
            })
            .collect();
        let Ok(grid) = EntityQueryGrid::build(&query) else {
            return Vec::new();
        };
        let Ok(found) = grid.query(
            &query,
            position,
            magicka_sim::fear::RANGE,
            true,
            false,
            |_, _| (false, [0.0; 3]),
        ) else {
            return Vec::new();
        };
        found
            .into_iter()
            .map(|i| &characters[i])
            .filter(|c| {
                let shield = self
                    .character_ref(c.handle)
                    .map(|r| &r.combat.health.shield);
                let shielded = shield.is_some_and(|s| s.active());
                let solid = shield.is_some_and(|s| {
                    use magicka_sim::character_health::SelfShieldType as T;
                    s.active() && matches!(s.kind, T::Earth | T::Ice | T::IcedEarth)
                });
                match caster {
                    Some((handle, faction)) => {
                        c.handle != handle
                            && magicka_sim::fear::targets(faction, c.faction, shielded, solid)
                    }
                    None => !shielded || solid,
                }
            })
            .map(|c| c.handle)
            .collect()
    }

    fn character_ref(&self, handle: u16) -> Option<&crate::character::RuntimeCharacter> {
        if let Some(p) = self.players.iter().flatten().find(|p| p.handle == handle) {
            return Some(&p.character);
        }
        self.npcs.get(&handle).map(|n| &n.character)
    }

    /// Character.Fear on one character (the feared effect follows it in
    /// presentation while IsFeared).
    pub fn fear_character(&mut self, handle: u16, source: magicka_sim::fear::FearSource) -> bool {
        let faction = self
            .entities
            .iter()
            .find(|e| e.handle == handle)
            .map_or(0, |e| self.faction(e));
        let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, handle) else {
            return false;
        };
        let fearless = c.definition.fearless;
        c.fear.apply(faction, fearless, source)
    }

    /// Character.Update around the state machine: FearPosition before it
    /// (IsFeared && !IsEntangled), the fear timer after it.
    fn refresh_fear_sources(&mut self) {
        let positions: BTreeMap<u16, [f32; 3]> = self
            .combatants()
            .into_iter()
            .map(|c| (c.handle, c.position))
            .collect();
        let bodies = &self.physics.bodies;
        let each = |c: &mut crate::character::RuntimeCharacter, body: usize| {
            let entangled = bodies[body].owner.entangled;
            c.fear_source = (c.fear.is_feared(c.combat.dead) && !entangled)
                .then(|| c.fear.source_position(|h| positions.get(&h).copied()));
        };
        for p in self.players.iter_mut().flatten() {
            each(&mut p.character, p.body);
        }
        for n in self.npcs.values_mut() {
            each(&mut n.character, n.body);
        }
    }

    /// Revive.Execute(position, health) for a caster at `caster` (None: the
    /// position overload, e.g. a script). Picks the eligible dead player with
    /// the greatest DeadAge, snaps the point to the navmesh and starts the
    /// effect; without one the magick_fail cue plays. Versus CanRevive is
    /// not modelled.
    pub fn revive(&mut self, caster: Option<[f32; 3]>, point: [f32; 3], health: f32) -> bool {
        self.execute_revive(caster, point, health, None, None)
    }

    /// TriggerCheckpoint's SetSpecificPlayer + position overload (full health).
    /// An ineligible requested player fails without selecting another player.
    pub fn revive_specific(&mut self, slot: usize, point: [f32; 3]) -> bool {
        self.execute_revive(None, point, 1.0, Some(slot), None)
    }

    /// Modern ownership/respawn boundary: delayed effects belong to the old
    /// player lifetime, even if another owner later reuses the same slot.
    pub fn cancel_player_abilities(&mut self, slot: usize, handle: u16) {
        let mut sizes = std::mem::take(&mut self.size_effects);
        sizes.remove_owner(handle, self);
        self.size_effects = sizes;
        use magicka_sim::magicks::AbilityEffect;
        self.ability_effects.effects.retain(|effect| match effect {
            AbilityEffect::Revive(r) => r.revivee != slot,
            AbilityEffect::Haste(h) => h.owner != handle,
            // Grease sprays from its owner's avatar.
            AbilityEffect::Grease(g) => g.owner != handle,
            // The airstrike outlives its caster (mOwner is only the attacker).
            AbilityEffect::Napalm(_) | AbilityEffect::MeteorShower(_) => true,
            AbilityEffect::Performance(p) => p.owner != handle,
            AbilityEffect::Judgement { owner, .. } => *owner != handle,
            // The units, not the caster, carry the effect.
            AbilityEffect::Amalgameddon(_) => true,
            // TimeWarp belongs to the whole PlayState, not the caster's life.
            AbilityEffect::TimeWarp(_) | AbilityEffect::Rain(_) => true,
            // Confuse ends through its target's HitPoints, not the caster.
            AbilityEffect::Confuse(_)
            | AbilityEffect::Thunderstorm(_)
            | AbilityEffect::Blizzard(_)
            | AbilityEffect::Conflagration { .. } => true,
        });
        self.revive_spawns.retain(|r| r.revivee != slot);
    }

    pub(super) fn execute_revive(
        &mut self,
        caster: Option<[f32; 3]>,
        point: [f32; 3],
        health: f32,
        specific: Option<usize>,
        reviver: Option<usize>,
    ) -> bool {
        // Versus: VersusRuleset.CanRevive(caster's player, revivee).
        let allowed: Vec<bool> = (0..self.players.len())
            .map(|slot| match (reviver, self.versus()) {
                (Some(r), Some(v)) => v.can_revive(self.player_teams[r], self.player_teams[slot]),
                _ => true,
            })
            .collect();
        let ages = self.players.iter().enumerate().map(|(slot, p)| {
            if specific.is_some_and(|wanted| wanted != slot) || !allowed[slot] {
                return None;
            }
            let p = p.as_ref()?;
            if !p.playing {
                return None;
            }
            let c = &p.character.combat;
            // Avatar.Overkilled || (Dead && !Undying && UndyingTimer NaN);
            // the template Undying flag is not modelled for avatars.
            let dead = c.overkilled || p.dead();
            (dead && !self.ability_effects.ressing(slot)).then_some(self.dead_age[slot])
        });
        let Some(slot) = magicka_sim::magicks::select_revivee(ages) else {
            self.play_cue(16, "magick_fail", None);
            return false;
        };
        self.play_cue(16, "magick_revive_cast", caster);
        // NavMesh.GetNearestPosition(..., MovementProperties.Default).
        let point = self
            .nav
            .as_ref()
            .and_then(|nav| nav.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
            .map_or(point, |(_, point, _)| point);
        self.ability_effects.revive(slot, point, health);
        true
    }

    /// Player.Update for each slot: DeadAge grows while the avatar is dead
    /// and resets otherwise.
    fn update_dead_age(&mut self, dt: f32) {
        let n = self.players.len();
        self.dead_age.reserve_slots(n);
        for (age, p) in self.dead_age.iter_mut().zip(&self.players) {
            match p {
                Some(p) if p.playing && p.dead() => *age += dt,
                Some(_) => *age = 0.0,
                None => {}
            }
        }
    }

    /// SpellManager.Update: magick ability effects after the entities.
    fn update_ability_effects(&mut self, dt: f32) {
        let mut sizes = std::mem::take(&mut self.size_effects);
        sizes.update(dt, self);
        self.size_effects = sizes;
        self.update_grease_sprays(dt);
        self.update_staff_effects(dt);
        self.update_polymorphs(dt);
        self.update_tornadoes(dt);
        self.update_vortices(dt);
        self.update_waves(dt);
        self.update_portal_connection();
        self.update_phoenix(dt);
        self.update_earthquake(dt);
        self.update_crosses();
        self.update_napalm(dt);
        self.update_amalgameddon(dt);
        let mut effects = std::mem::take(&mut self.ability_effects);
        let deliver_damage = self.role != NetworkRole::Client;
        effects.update_conflagrations(dt, |sweep, instance, dt| {
            sweep.update(
                dt,
                deliver_damage,
                &mut ConflagrationWorld {
                    world: self,
                    instance,
                },
            );
        });
        self.ability_effects = effects;
        self.end_conflagrations();
        if self.role != NetworkRole::Client {
            self.ability_effects.update_thunderstorm(dt);
        }
        let players = &self.players;
        let npcs = &self.npcs;
        let bodies = &mut self.physics.bodies;
        let spawns = &mut self.revive_spawns;
        let role = self.role;
        let (time_scale, scene_saturation) = (self.time_multiplier, self.scene_saturation);
        self.ability_effects.update(
            dt,
            time_scale,
            scene_saturation,
            |owner, factor| {
                let body = players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == owner)
                    .map(|p| p.body)
                    .or_else(|| npcs.get(&owner).map(|n| n.body));
                match body.map(|b| &mut bodies[b]) {
                    Some(body) if body.active => {
                        let m = body.speed_multiplier() * factor;
                        body.set_speed_multiplier(m);
                        true
                    }
                    _ => false,
                }
            },
            |revive| {
                if role != NetworkRole::Client
                    && players
                        .get(revive.revivee)
                        .and_then(Option::as_ref)
                        .is_some_and(|p| p.playing)
                {
                    spawns.push(*revive);
                }
            },
            |target| {
                players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == target)
                    .map(|p| &p.character)
                    .or_else(|| npcs.get(&target).map(|n| &n.character))
                    .is_some_and(|c| c.combat.health.hit_points > 0.0)
            },
        );
        self.end_confusions();
        self.performance_events();
        for owner in std::mem::take(&mut self.ability_effects.judgement_due) {
            if self.role != NetworkRole::Client {
                self.judgement_missile(owner);
            }
        }
        if let Some(shower) = self.ability_effects.meteors_due.take() {
            if self.role != NetworkRole::Client {
                self.meteors(shower);
            }
        }
        if let Some(storm) = self.ability_effects.storm_bolts_due.take() {
            if self.role != NetworkRole::Client {
                self.thunderstorm_bolts(storm);
            }
        }
        if let Some((caster, timestamp)) = self.ability_effects.rain_pulse.take() {
            // mDoDamage: clients leave the damage to the host.
            if self.role != NetworkRole::Client {
                self.weather_pass(caster, timestamp, magicka_sim::spells::Elements::WATER);
            }
        }
        if let Some((caster, timestamp)) = self.ability_effects.blizzard_pulse.take() {
            if self.role != NetworkRole::Client {
                self.weather_pass(caster, timestamp, magicka_sim::spells::Elements::COLD);
            }
            // Blizzard.Update freezes every liquid independently of mDoDamage.
            for liquid in &mut self.collision.collision_mut().water {
                liquid
                    .freeze
                    .freeze_all(magicka_sim::magicks::BLIZZARD_PERIOD);
            }
        }
        self.sync_time_warp();
        self.sync_rain();
        self.sync_blizzard();
        self.sync_meteor_shower();
        // Flash.Update (its SpellManager slot order is not modelled): the
        // drawn intensity, then removal once mIntensity <= 0.
        self.flash_draw = 0.0;
        if let Some((intensity, ttl)) = self.flash.as_mut() {
            *intensity -= dt;
            self.flash_draw = *intensity / *ttl * 0.75;
            if *intensity <= 0.0 {
                self.flash = None;
            }
        }
    }

    /// Flash.Execute(scene, time): mTTL = mIntensity = max(time, mIntensity).
    pub fn flash(&mut self, time: f32) {
        let current = self.flash.map_or(0.0, |(i, _)| i);
        let v = time.max(current);
        self.flash = Some((v, v));
    }

    /// The Flash quad's additive white level this frame (0 when idle).
    pub fn flash_level(&self) -> f32 {
        self.flash_draw.max(0.0)
    }

    /// Rain/Blizzard.Update's damage pass: Status/Water or Cold, magnitude
    /// 0.25 to every living character whose resistance is not 1 and that is
    /// not inside a sphere shield. Mines, barriers and other damageables are
    /// not reconstructed here (research/rain.md).
    fn weather_pass(
        &mut self,
        caster: Option<u16>,
        timestamp: f64,
        element: magicka_sim::spells::Elements,
    ) {
        use magicka_sim::damage::{attack, Damage};
        let combatants = self.combatants();
        let shields: Vec<_> = self
            .shields
            .iter()
            .filter(|s| {
                !s.shield.dead && s.shield.kind == magicka_sim::shield_entity::ShieldKind::Sphere
            })
            .map(|s| (s.shield.position, s.shield.radius))
            .collect();
        let targets: Vec<_> = combatants
            .iter()
            .filter(|c| !c.dead)
            .filter(|c| c.resistance_against(element.0) != 1.0)
            .filter(|c| {
                !shields.iter().any(|(p, r)| {
                    let d = [
                        c.position[0] - p[0],
                        c.position[1] - p[1],
                        c.position[2] - p[2],
                    ];
                    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < r * r
                })
            })
            .map(|c| (c.handle, c.position))
            .collect();
        let scene = self.collision.collision();
        let Some(view) = crate::npc_combat::CombatView::new(&combatants, Some(scene)) else {
            return;
        };
        let mut world = SceneHitWorld {
            physics_damage: &mut self.physics_damage,
            dispensers: &mut self.dispensers,
            entities: &mut self.entities,
            barriers: None,
            barrier_events: Vec::new(),
            view: &view,
            npcs: &mut self.npcs,
            players: &mut self.players,
            bodies: &mut self.physics.bodies,
            triggers: &mut self.pending_triggers,
            shields: &mut self.shields,
            collision_rng: &mut self.rng,
            play_time: timestamp,
            entity_blasts: &mut self.entity_blasts,
            death: &mut self.death,
            jormungandr: &mut self.jormungandr,
            death_damage_features: self.role != NetworkRole::Client,
        };
        let damage = Damage {
            attack: attack::STATUS,
            element: element.0,
            amount: 0.0,
            magnitude: magicka_sim::magicks::RAIN_MAGNITUDE,
        };
        for (target, _) in targets {
            // Damage(mDamage, mCaster as Entity, mTimeStamp, Vector3.Zero).
            crate::missiles::MissileWorld::damage_character(
                &mut world,
                target,
                damage,
                caster.unwrap_or(0),
                [0.0; 3],
            );
        }
    }

    /// Rain.OnRemove: stop the ambience and restore the light target.
    fn sync_rain(&mut self) {
        if self.ability_effects.rain_state().is_none() {
            if let Some(cue) = self.rain_cue.take() {
                self.stop_cue(cue);
                self.light_target_intensity = 1.0;
            }
        }
    }

    /// Blizzard.OnRemove stops its ambience, without changing scene light.
    fn sync_blizzard(&mut self) {
        if self.ability_effects.blizzard_state().is_none() {
            if let Some(cue) = self.blizzard_cue.take() {
                self.stop_cue(cue);
            }
        }
    }

    /// GameScene.Update: the light intensity steps toward its target (down
    /// by dt, possibly overshooting by one step; up by min(dt, gap)).
    fn update_scene_light(&mut self, dt: f32) {
        let (target, current) = (self.light_target_intensity, self.light_intensity);
        self.light_intensity = if target < current {
            current - dt.max(target - current)
        } else {
            current + dt.min(target - current)
        };
    }

    /// TimeWarp.Update writes TimeMultiplier/Saturation; OnRemove (expiry or
    /// ClearMagicks) stops the cue and restores 1 and the scene saturation.
    fn sync_time_warp(&mut self) {
        let warp = self.ability_effects.time_warp_state().copied();
        self.time_multiplier = warp.map_or(1.0, |w| w.multiplier);
        self.render_saturation = warp.map(|w| w.saturation);
        if warp.is_none() {
            if let Some(cue) = self.time_warp_cue.take() {
                self.stop_cue(cue);
            }
        }
    }

    /// Resolve every character's queued FloatingTextManager requests at its
    /// current capsule top.
    pub fn apply_damage_numbers(&mut self) {
        let bodies = &self.physics.bodies;
        let manager = &mut self.floating_text;
        for (body, c) in self
            .players
            .iter_mut()
            .flatten()
            .map(|p| (p.body, &mut p.character))
            .chain(self.npcs.values_mut().map(|n| (n.body, &mut n.character)))
        {
            if c.damage_numbers.is_empty() {
                continue;
            }
            let b = &bodies[body];
            let top = crate::floating_text::character_top(b.position, b.length, b.radius);
            manager.apply(
                &mut c.damage_bookkeeping.last_damage_index,
                &mut c.damage_numbers,
                top,
            );
        }
    }

    /// Death disables the agent (NonPlayerCharacter.Deinitialize/Kill path);
    /// termination also removes the body from the physics system.
    fn disable_npc(&mut self, handle: u16, remove_body: bool) {
        self.agents.retain(|&h| h != handle);
        if remove_body {
            if let Some(mut n) = self.npcs.remove(&handle) {
                // Character.Deinitialize releases its locked damage number.
                let body = &self.physics.bodies[n.body];
                let top =
                    crate::floating_text::character_top(body.position, body.length, body.radius);
                let c = &mut n.character;
                c.damage_numbers
                    .push(crate::floating_text::NumberRequest::Release);
                self.floating_text.apply(
                    &mut c.damage_bookkeeping.last_damage_index,
                    &mut c.damage_numbers,
                    top,
                );
                self.physics.bodies[n.body].active = false;
            }
        }
    }

    /// Entities as Agent queries see them (alive, added; characters with
    /// their factions), followed by the non-character danger sources of
    /// Agent.GetAvoidance (missiles, mines, barriers, tornadoes, vortices and
    /// grease fields with their Entity.GetDanger values).
    fn agent_neighbours(&self) -> Vec<crate::npc::Neighbour> {
        let mut out = self.agent_entity_neighbours();
        self.push_danger_neighbours(&mut out);
        out
    }

    /// Entity.GetDanger sources that are not scene entities. They carry the
    /// `DANGER_HANDLE` sentinel and no faction, so they only feed the danger
    /// term. MissileEntity.Danger overrides (Potion 0, HolyGrenade 30,
    /// GreaseLump 10) are not carried by LiveMissile: every missile uses the
    /// Initialize default (2).
    fn push_danger_neighbours(&self, out: &mut Vec<crate::npc::Neighbour>) {
        use magicka_sim::agent_steering::{DangerSource, MISSILE_DEFAULT_DANGER};
        let danger =
            |position: [f32; 3], radius: f32, source: DangerSource| crate::npc::Neighbour {
                handle: DANGER_HANDLE,
                position,
                direction: [0.0, 0.0, -1.0],
                radius,
                faction: None,
                danger: source.danger(),
                invisible: false,
            };
        for m in self.missiles.iter().filter(|m| !m.body.dead) {
            out.push(danger(
                m.body.position,
                m.body.radius,
                DangerSource::Missile {
                    danger: MISSILE_DEFAULT_DANGER,
                },
            ));
        }
        for (_, mine) in self.mines.iter().filter(|(_, m)| !m.dead) {
            out.push(danger(
                mine.position,
                0.0,
                DangerSource::SpellMine {
                    detonating: mine.detonation_timer > 0.0,
                },
            ));
        }
        for (_, b) in self.barriers.iter().filter(|(_, b)| !b.dead) {
            out.push(danger(
                b.position,
                b.radius,
                DangerSource::Barrier {
                    damage_magnitude: b.damage.iter().map(|d| d.magnitude).sum(),
                },
            ));
        }
        for t in self.tornadoes.iter().filter(|t| !t.tornado.dead()) {
            out.push(danger(
                t.tornado.position,
                magicka_sim::tornado::RADIUS,
                DangerSource::Tornado {
                    alpha: t.tornado.alpha,
                },
            ));
        }
        for v in self.vortices.iter().filter(|v| !v.vortex.dead()) {
            out.push(danger(
                v.vortex.center,
                v.vortex.radius,
                DangerSource::Vortex {
                    mass: v.vortex.mass,
                },
            ));
        }
        for (_, g) in self.grease.iter() {
            out.push(danger(
                g.position,
                0.0,
                DangerSource::Grease {
                    burning: g.field.burning,
                    temperature: g.field.temperature,
                },
            ));
        }
    }

    /// AIStateMove group facts for one agent: the live same-`Type` NPC
    /// agents other than `handle` (EntityKind::Npc type_id, i.e. the
    /// template ID Character.Type returns) and the facts of its current
    /// leader. `npc` has already been taken out of `self.npcs`.
    fn group_facts(
        &self,
        handle: u16,
        npc: &crate::npc::NpcCharacter,
    ) -> (
        Vec<magicka_sim::agent_steering::GroupCandidate>,
        Option<(u16, crate::avatar_ai::GroupLeader)>,
    ) {
        let own_type = self
            .entities
            .iter()
            .find(|e| e.handle == handle)
            .and_then(|e| match e.kind {
                EntityKind::Npc { type_id, .. } => Some(type_id),
                _ => None,
            });
        let Some(own_type) = own_type else {
            return (Vec::new(), None);
        };
        let goal_of = |n: &crate::npc::NpcCharacter| {
            let w = n.ai.waypoint;
            n.combat
                .current_target()
                .and_then(|t| {
                    self.entities
                        .iter()
                        .find(|e| e.handle == t && e.added && !e.removed)
                })
                .map_or(w, |t| {
                    [
                        w[0] + t.position[0],
                        w[1] + t.position[1],
                        w[2] + t.position[2],
                    ]
                })
        };
        let mut candidates = Vec::new();
        let mut leader = None;
        for (&other, n) in &self.npcs {
            if other == handle {
                continue;
            }
            let Some(e) = self
                .entities
                .iter()
                .find(|e| e.handle == other && e.added && !e.removed && !e.dead)
            else {
                continue;
            };
            let same_type =
                matches!(e.kind, EntityKind::Npc { type_id, .. } if type_id == own_type);
            let body = &self.physics.bodies[n.body];
            let in_move = n.ai.state() == crate::avatar_ai::AiState::Move;
            if same_type {
                candidates.push(magicka_sim::agent_steering::GroupCandidate {
                    handle: other,
                    position: body.position,
                    radius: e.radius,
                    in_move,
                    is_leader: n.ai.is_leader(),
                    target: n.combat.current_target(),
                    waypoint: n.ai.waypoint,
                });
            }
            if npc.ai.leader == Some(other) {
                leader = Some((
                    other,
                    crate::avatar_ai::GroupLeader {
                        position: body.position,
                        direction: body.direction(),
                        in_move,
                        goal: goal_of(n),
                    },
                ));
            }
        }
        (candidates, leader)
    }

    /// Scene entities as Agent queries see them.
    fn agent_entity_neighbours(&self) -> Vec<crate::npc::Neighbour> {
        self.entities
            .iter()
            .filter(|e| e.added && !e.removed && !e.dead)
            .map(|e| {
                let (direction, character) = match e.kind {
                    EntityKind::Avatar { slot } => (
                        self.players[slot].as_ref().map_or([0.0, 0.0, -1.0], |p| {
                            self.physics.bodies[p.body].direction()
                        }),
                        true,
                    ),
                    EntityKind::Npc { .. } => (
                        self.npcs.get(&e.handle).map_or(
                            [-e.orientation[8], -e.orientation[9], -e.orientation[10]],
                            |n| self.physics.bodies[n.body].direction(),
                        ),
                        true,
                    ),
                    EntityKind::Physics { .. }
                    | EntityKind::Item
                    | EntityKind::ElementalEgg
                    | EntityKind::Dispenser => ([0.0, 0.0, -1.0], false),
                };
                crate::npc::Neighbour {
                    handle: e.handle,
                    position: e.position,
                    direction,
                    radius: e.radius,
                    faction: character.then_some(self.faction(e)),
                    // Character.GetDanger = SpellQueue.Count.
                    danger: self.npcs.get(&e.handle).map_or_else(
                        || {
                            self.players
                                .iter()
                                .flatten()
                                .find(|p| p.handle == e.handle)
                                .map_or(0., |p| p.conjure.queue.len() as f32)
                        },
                        |n| n.combat.spellcasting.queue.len() as f32,
                    ),
                    invisible: self
                        .npcs
                        .get(&e.handle)
                        .map(|n| &n.character)
                        .or_else(|| {
                            self.players
                                .iter()
                                .flatten()
                                .find(|p| p.handle == e.handle)
                                .map(|p| &p.character)
                        })
                        .is_some_and(|c| c.is_invisible()),
                }
            })
            .collect()
    }

    /// AIManager.Update: the scheduled agents run with their accumulated
    /// time, then every agent accumulates this frame's delta.
    pub fn update_agents(&mut self, dt: f32) {
        // Presentation cues not drained by a session would grow unbounded.
        for n in self.npcs.values_mut() {
            n.motion.controller.cues.clear();
        }
        let order = crate::npc::scheduled(&mut self.ai_update_index, self.agents.len());
        let handles: Vec<u16> = order.iter().map(|&i| self.agents[i]).collect();
        let mut kills = Vec::new();
        // Combat queries see the characters as of the AI phase.
        let combatants = self.agent_combatants();
        let view =
            crate::npc_combat::CombatView::new(&combatants, Some(self.collision.collision()));
        for handle in handles {
            let Some(mut npc) = self.npcs.remove(&handle) else {
                continue;
            };
            let dead = self.entities.iter().any(|e| e.handle == handle && e.dead);
            if !dead && self.ai_runs_for(handle) {
                let neighbours = self.agent_neighbours();
                let position = self.physics.bodies[npc.body].position;
                let faction = self
                    .entities
                    .iter()
                    .find(|e| e.handle == handle)
                    .map_or(0, |e| self.faction(e));
                let within = |r: f32| -> Vec<crate::npc::Neighbour> {
                    neighbours
                        .iter()
                        .filter(|n| {
                            let d = [
                                n.position[0] - position[0],
                                n.position[1] - position[1],
                                n.position[2] - position[2],
                            ];
                            let reach = r + n.radius;
                            n.handle != handle
                                && d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= reach * reach
                        })
                        .copied()
                        .collect()
                };
                let alert = within(npc.weights.alert_radius);
                let near = within(12.0);
                let weights = npc.weights;
                let avoid = move |p: magicka_sim::impact_motion::Vector| {
                    crate::npc::avoidance(p, faction, &weights, &near)
                };
                let locators = &self.locators;
                let entities = &self.entities;
                let locator = |id: i32| locators.get(&id).copied();
                let closest_area_point = |id: i32, p| {
                    self.level
                        .trigger_areas
                        .iter()
                        .find(|a| custom_hash(&a.name.to_ascii_lowercase()) == id)
                        .map(|a| CameraBox::from_area(a).closest_point(p))
                };
                let entity_position = |id: i32| {
                    entities
                        .iter()
                        .rev()
                        .find(|e| e.added && e.unique_id == id && id != 0)
                        .filter(|e| !e.removed)
                        .map(|e| e.position)
                };
                // AIStateMove leader election / FollowGroup queries (on by
                // default, as natively; npc-agents.md 2026-11-01 traced the
                // two scene regressions once blamed on it to chaotic
                // divergence, not an election defect).
                let (candidates, leader) = if self.group_leaders_enabled {
                    self.group_facts(handle, &npc)
                } else {
                    (Vec::new(), None)
                };
                npc.group_candidates = candidates;
                npc.group_leader = leader;
                let frame = crate::character_motion::ActorFrame {
                    body: &mut self.physics.bodies[npc.body],
                    nav: self.nav.as_ref(),
                    move_abilities: npc.move_abilities,
                    triggers: &mut self.pending_triggers,
                    sender: handle,
                    entity_position: &entity_position,
                    locator: &locator,
                    closest_area_point: &closest_area_point,
                    avoidance: &avoid,
                    in_cutscene: self.flags.cutscene,
                    play_time: self.play_time,
                };
                let combat = view
                    .as_ref()
                    .and_then(|v| v.index(handle).map(|owner| (v, owner)));
                npc.combat.current_spell = npc.current_spell.is_some();
                let outcome = npc.agent_update(frame, faction, &alert, combat);
                npc.group_candidates.clear();
                npc.group_leader = None;
                if let Some(remove) = outcome.killed {
                    kills.push((handle, remove));
                }
            }
            self.npcs.insert(handle, npc);
        }
        for &handle in &self.agents {
            if let Some(n) = self.npcs.get_mut(&handle) {
                n.time_since_update += dt;
            }
        }
        for (handle, remove) in kills {
            let _ = if remove {
                EntityCommandHost::terminate(self, handle)
            } else {
                EntityCommandHost::kill(self, handle)
            };
        }
    }

    /// PlayState.BeginCutscene(killDialogs)/EndCutscene → DialogManager.EndAll.
    fn end_all_dialogs(&mut self) -> Result<(), WorldError> {
        let Some(mut manager) = self.dialogs.take() else {
            return Ok(());
        };
        let Some(mut text) = self.dialog_text.take() else {
            self.dialogs = Some(manager);
            return Ok(());
        };
        self.refresh_dialog_world(&mut text);
        let result = manager
            .end_all(&mut DialogBridge {
                host: &mut text,
                world: &*self,
            })
            .map_err(|e| format!("cutscene dialog cleanup: {e:?}"));
        self.dialogs = Some(manager);
        self.dialog_text = Some(text);
        self.apply_dialog_commands();
        result
    }

    /// Attachment.Release for NPC equipment (DetachItem, DeadState): the
    /// item leaves the hand (no further melee, effects or rendering);
    /// pickable Wizard items become pickups at the item transform with the
    /// release velocity (spawned by the session). Non-pickable items,
    /// which natively drop as inert items, are not spawned.
    fn release_npc_equipment(&mut self, releases: Vec<(u16, i32, [f32; 3], bool)>) {
        for (handle, slot, velocity, death) in releases {
            let undying = self
                .entities
                .iter()
                .any(|e| e.handle == handle && e.undying);
            let Some(npc) = self.npcs.get_mut(&handle) else {
                continue;
            };
            let undying = undying || npc.character.undying;
            if undying && death {
                continue;
            }
            let Some(index) = npc.weapons.iter().position(|(s, _)| *s == slot) else {
                continue;
            };
            let (_, weapon) = npc.weapons.remove(index);
            npc.character.block_values.remove(&slot);
            if let Some(rig) = npc.rig.as_mut() {
                rig.attachments.retain(|a| a.slot != slot);
            }
            npc.held_effects.retain(|h| h.slot != slot);
            if let Some((Some(name), true)) = weapon.release {
                // Release copies pickable equipment into a distinct body and
                // Detach clears that body's velocity. DetachItem subsequently
                // assigns the animation velocity to the retained equipment.
                let _ = velocity;
                self.npc_item_releases.push(npc_events::ItemRelease {
                    name,
                    transform: weapon.item_transform,
                    velocity: [0.0; 3],
                    raise_by_height: false,
                    despawn: 0.0,
                });
            }
        }
    }

    /// Character.Update for live NPCs (damage/status ticks, states, death),
    /// then mirror body facts into entities; dead NPCs sink and are removed
    /// like EntityManager does with Removable characters.
    pub fn update_npcs(&mut self, dt: f32) {
        let cutscene = self.flags.cutscene;
        let mut died = Vec::new();
        let mut entered_dead = Vec::new();
        let mut releases = Vec::new();
        self.refresh_fear_sources();
        let handles: Vec<_> = self.npcs.keys().copied().collect();
        for handle in handles {
            // Rebuilding this list for every NPC took half of all simulation time with
            // 300 NPCs (research/performance.md). Only a live spell reads it.
            let casting = self.npcs[&handle].current_spell.is_some();
            let mut combatants = if casting {
                self.combatants()
            } else {
                Vec::new()
            };
            let mut npc = self.npcs.remove(&handle).unwrap();
            npc.jump_target = npc
                .combat
                .current_target()
                .and_then(|target| self.jump_target(target, handle, &npc));
            npc.play_time = self.play_time;
            self.refresh_should_release(&mut npc.character);
            let (entered, die) =
                npc.update_motion(&mut self.physics.bodies[npc.body], cutscene, dt);
            // A spell begun during this motion update needs the list too, with this NPC
            // in it as usual.
            if !casting && npc.current_spell.is_some() {
                self.npcs.insert(handle, npc);
                combatants = self.combatants();
                npc = self.npcs.remove(&handle).unwrap();
            }
            self.update_npc_spell(&mut npc, dt, combatants);
            let body = &mut self.physics.bodies[npc.body];
            // DetachItem: the action velocity in body space.
            let o = body.orientation;
            for (slot, v) in std::mem::take(&mut npc.detach_requests) {
                let velocity =
                    [0, 1, 2].map(|i| v[0] * o.right[i] + v[1] * o.up[i] + v[2] * o.backward[i]);
                releases.push((handle, slot, velocity, false));
            }
            // DeadState.OnEnter releases every equipment slot unless Undying.
            // A replaced phase (RemoveEvent + SpawnEvent) drops a copy too,
            // then its next phase drops its own: the original's double drop
            // (research/npc-phases.md).
            if entered {
                for (slot, _) in &npc.weapons {
                    releases.push((handle, *slot, [0.0; 3], true));
                }
            }
            let dead = npc.character.combat.dead;
            npc.character.fear.tick(dt, dead);
            if npc.character.tick_charm(dt) {
                // NonPlayerCharacter.EndCharm: AI back to idle.
                npc.charm_reset = true;
            }
            if entered {
                entered_dead.push(handle);
            }
            if die {
                died.push((handle, npc.on_death));
            }
            let mut events = Vec::new();
            let status = npc.character.update_damage_and_status_on(
                body,
                dt,
                true,
                magicka_sim::character_health::StatusUpdateContext {
                    in_cutscene: cutscene,
                    ..Default::default()
                },
                &mut events,
            );
            npc.stop_status_spell();
            npc.finish(body, cutscene, dt);
            npc.motion
                .status_tail(body, status, self.bloats.contains_key(&handle));
            // VladCharacter.Update runs after NonPlayerCharacter.Update,
            // whose base motion/status path resets the multiplier each frame.
            if npc.vlad_character {
                body.set_speed_multiplier(body.speed_multiplier() * 1.5);
            }
            // Character.Update dead tail: sink after the dead timer expires.
            if npc.dead() && npc.motion.dead_timer < 0.0 {
                let mut p = body.position;
                p[1] -= dt * 0.3;
                body.position = p;
            }
            self.npcs.insert(handle, npc);
        }
        self.release_npc_equipment(releases);
        self.update_levitation(dt, true);
        self.prop_contact_conditions(dt);
        self.execute_melee();
        self.execute_item_missiles();
        self.execute_grip_requests();
        self.update_grips();
        self.apply_npc_spell_events();
        self.apply_npc_specials();
        for handle in entered_dead {
            // DeadState.OnEnter: NonPlayerCharacter AI.Disable.
            self.agents.retain(|&h| h != handle);
        }
        for (handle, on_death) in died {
            self.note_kill(handle);
            // NonPlayerCharacter.Die calls its master's RemoveBubbleShield
            // for every summoned child, not only for the cross template.
            if let Some((master, _)) = self.npcs.get(&handle).and_then(|n| n.summon) {
                self.remove_bubble_shield(master);
            }
            // Character.Die: OnDeath trigger with the NPC as sender.
            if let Some(npc) = self.npcs.get_mut(&handle) {
                npc.character.collision_hits.clear();
            }
            self.agents.retain(|&h| h != handle);
            if on_death != 0 {
                self.pending_triggers.push((on_death, Some(handle)));
            }
        }
        self.execute_npc_lifecycle_events();
        // DeadState.Update, Overkilled branch: RemoveAfterDeath, then with
        // gibs (and blood and gore) SpawnGibs and the gib cue.
        let gibbed: Vec<u16> = self
            .npcs
            .iter()
            .filter(|(h, n)| {
                n.dead()
                    && n.character.combat.overkilled
                    && n.definition.gib_count > 0
                    && !self.overkilled.contains(h)
            })
            .map(|(&h, _)| h)
            .collect();
        for handle in gibbed {
            self.spawn_gibs(handle);
        }
        let mut removable = Vec::new();
        for (handle, npc) in &self.npcs {
            let body = &self.physics.bodies[npc.body];
            if let Some(e) = self.entities.iter_mut().find(|e| e.handle == *handle) {
                e.position = body.position;
                let o = body.orientation;
                e.orientation = [
                    o.right[0],
                    o.right[1],
                    o.right[2],
                    0.0,
                    o.up[0],
                    o.up[1],
                    o.up[2],
                    0.0,
                    o.backward[0],
                    o.backward[1],
                    o.backward[2],
                    0.0,
                    body.position[0],
                    body.position[1],
                    body.position[2],
                    1.0,
                ];
                e.animation = npc.motion.controller.current_animation;
                e.special_idle = npc.motion.special_idle;
                e.event_count = usize::from(npc.ai.controls_avatar());
                e.hit_points = npc.character.combat.health.hit_points;
                e.dead = npc.dead();
            }
            // Character.Removable: Dead, mDeadTimer < -10, RemoveAfterDeath.
            if npc.dead() && npc.motion.dead_timer < -10.0 {
                removable.push(*handle);
            }
        }
        for handle in removable {
            if let Ok(e) = self.entity_mut(handle) {
                e.removed = true;
            }
            self.despawn_summon(handle);
            self.disable_npc(handle, true);
        }
    }

    /// SpawnMissile executions from player and NPC animation actions:
    /// Item.ExecuteRanged from the item's attach point with the item's
    /// ranged conditions, not hitting the owner. A Ranged-prepared item
    /// with a current target aims ballistically and homes on the target.
    fn execute_item_missiles(&mut self) {
        let mut handles: Vec<u16> = self
            .npcs
            .iter()
            .filter(|(_, n)| !n.missile_requests.is_empty())
            .map(|(&h, _)| h)
            .collect();
        handles.extend(
            self.players
                .iter()
                .flatten()
                .filter(|p| !p.missile_requests.is_empty())
                .map(|p| p.handle),
        );
        let gravity = magicka_sim::rigid_body::Gravity::magicka();
        for handle in handles {
            let mut launches = Vec::new();
            let (target, facing, orientation) = if let Some(npc) = self.npcs.get_mut(&handle) {
                let requests = std::mem::take(&mut npc.missile_requests);
                let target = npc.combat.current_target();
                let body = &self.physics.bodies[npc.body];
                let (facing, orientation) = (body.direction(), body.orientation);
                for (slot, velocity, item_aligned) in requests {
                    let Some((_, weapon)) = npc.weapons.iter().find(|(s, _)| *s == slot) else {
                        continue;
                    };
                    let Some(item) = weapon.ranged.clone() else {
                        continue;
                    };
                    let source = npc
                        .combat
                        .ranged_source
                        .iter()
                        .find(|(s, ..)| *s == slot)
                        .copied();
                    launches.push((
                        item,
                        weapon.attach,
                        weapon.item_position,
                        source,
                        velocity,
                        item_aligned,
                        None,
                        None,
                    ));
                }
                (target, facing, orientation)
            } else if let Some(player) = self
                .players
                .iter_mut()
                .flatten()
                .find(|p| p.handle == handle)
            {
                let body = &self.physics.bodies[player.body];
                for (slot, velocity, item_aligned) in std::mem::take(&mut player.missile_requests) {
                    let Some((_, held)) = player
                        .items
                        .iter_mut()
                        .find(|(s, i)| *s == slot && i.attached)
                    else {
                        continue;
                    };
                    let Some(item) = held.melee.ranged.clone() else {
                        continue;
                    };
                    // MasterSword's beam requires full health (Item.ExecuteRanged).
                    if held.definition.passive_ability == 15
                        && player.character.combat.health.hit_points
                            < player.character.combat.health.max_hit_points
                    {
                        continue;
                    }
                    if held.definition.passive_ability == 14
                        && held.ranged_missile.is_some_and(|id| {
                            self.missiles
                                .iter()
                                .any(|m| m.body.id == id && !m.body.dead)
                        })
                    {
                        continue;
                    }
                    held.cooldown = held.definition.cooldown_time;
                    launches.push((
                        item,
                        held.melee.attach,
                        held.melee.item_position,
                        None,
                        velocity,
                        item_aligned,
                        held.frame.map(|f| f.item),
                        Some(slot),
                    ));
                }
                (None, body.direction(), body.orientation)
            } else {
                continue;
            };
            for (
                item,
                attach,
                item_position,
                source,
                velocity,
                item_aligned,
                item_orientation,
                player_slot,
            ) in launches
            {
                let target_position = target.and_then(|t| {
                    self.combatants()
                        .into_iter()
                        .find(|c| c.handle == t)
                        .map(|c| c.position)
                });
                let aimed = source.is_some() && target_position.is_some();
                let launch = if let Some(v) = velocity {
                    // Item-aligned velocities use the item body orientation,
                    // which the rewrite does not track: the character's is
                    // used instead (INFERRED equivalent for rigid grips).
                    if item_aligned && item_orientation.is_none() {
                        self.log.push(
                            "npc missile: item-aligned velocity uses the character orientation"
                                .into(),
                        );
                    }
                    if let Some(matrix) = item_orientation.filter(|_| item_aligned) {
                        [0, 1, 2]
                            .map(|i| v[0] * matrix[i] + v[1] * matrix[4 + i] + v[2] * matrix[8 + i])
                    } else {
                        let o = orientation;
                        [0, 1, 2].map(|i| v[0] * o.right[i] + v[1] * o.up[i] + v[2] * o.backward[i])
                    }
                } else if let (Some((_, elevation, accuracy)), Some(p)) = (source, target_position)
                {
                    let spread = self.random() * 2.0 - 1.0;
                    crate::missiles::ranged_launch_velocity(
                        facing,
                        item_position,
                        p,
                        elevation,
                        item.elevation,
                        accuracy,
                        item.range,
                        gravity.vector[1],
                        spread,
                    )
                } else {
                    crate::missiles::plain_launch_velocity(facing, item.elevation, item.range)
                };
                let (conditions, skipped) = crate::missiles::item_conditions(&item.conditions);
                for name in skipped {
                    self.log
                        .push(format!("npc missile: {name} event has no missile host"));
                }
                let id = self.next_missile;
                self.next_missile = self.next_missile.wrapping_add(1);
                let mut missile = crate::missiles::LiveMissile::spawn_custom(
                    id,
                    handle,
                    attach,
                    launch,
                    item.radius,
                    false,
                    conditions,
                );
                missile.body.item_model = item.model.clone();
                if aimed {
                    missile.body.homing = Some(magicka_sim::missile_homing::Homing::new(
                        target,
                        item.homing,
                    ));
                }
                let combatants = self.combatants();
                let Some(view) = crate::npc_combat::CombatView::new(
                    &combatants,
                    Some(self.collision.collision()),
                ) else {
                    return;
                };
                let mut world = SceneHitWorld {
                    physics_damage: &mut self.physics_damage,
                    dispensers: &mut self.dispensers,
                    entities: &mut self.entities,
                    barriers: None,
                    barrier_events: Vec::new(),
                    view: &view,
                    npcs: &mut self.npcs,
                    players: &mut self.players,
                    bodies: &mut self.physics.bodies,
                    triggers: &mut self.pending_triggers,
                    shields: &mut self.shields,
                    collision_rng: &mut self.rng,
                    play_time: self.play_time,
                    entity_blasts: &mut self.entity_blasts,
                    death: &mut self.death,
                    jormungandr: &mut self.jormungandr,
                    death_damage_features: self.role != NetworkRole::Client,
                };
                crate::missiles::initialize(&mut missile, &mut world, &mut self.missile_visuals);
                if let Some(slot) = player_slot {
                    if let Some(held) = self
                        .players
                        .iter_mut()
                        .flatten()
                        .find(|p| p.handle == handle)
                        .and_then(|p| p.items.iter_mut().find(|(s, _)| *s == slot).map(|(_, i)| i))
                    {
                        held.ranged_missile = Some(id);
                    }
                }
                self.register_missile(missile);
            }
        }
    }

    /// EventCollection.ExecuteAll's presentation events for an item's melee
    /// conditions (DealDamage's first execution: Default; each newly hit
    /// target: Hit). PlaySoundEvent plays at the item's emitter with its
    /// Magnitude variable; PlayEffectEvent starts at the item position
    /// facing (0, 1, 0) when world aligned, else the item's forward (the
    /// owner's facing stands in: item orientation is not tracked). Damage
    /// is delivered separately; other event kinds are not presented here.
    fn melee_event_presentation(
        &mut self,
        conditions: &[magicka_content::common::EventCollection],
        flag: u8,
        owner: u16,
        weapon: i32,
        position: [f32; 3],
        facing: [f32; 3],
    ) {
        use magicka_content::common::Event;
        for collection in conditions {
            if collection.condition.condition_type & flag == 0 {
                continue;
            }
            for event in &collection.events {
                match event {
                    Event::Sound {
                        bank,
                        sound,
                        magnitude,
                        ..
                    } => {
                        self.play_cue_magnitude(
                            *bank as u16,
                            sound,
                            Some(position),
                            Some(*magnitude),
                        );
                    }
                    Event::Effect {
                        world_align,
                        effect,
                        follow,
                    } => {
                        let direction = if *world_align {
                            [0.0, 1.0, 0.0]
                        } else {
                            facing
                        };
                        self.item_event_effects
                            .push(crate::npc_combat::ItemEffectRequest::Start {
                                owner,
                                weapon,
                                name: effect.to_ascii_lowercase(),
                                position,
                                direction,
                                follow: *follow,
                            });
                    }
                    _ => {}
                }
            }
        }
    }

    /// DealDamage executions of this frame's NPC updates: Item.Execute per
    /// request against the post-update world, then the Hit damage through
    /// the public route (NPC victims record the attacker first).
    fn execute_melee(&mut self) {
        let mut attackers: Vec<u16> = self
            .npcs
            .iter()
            .filter(|(_, n)| !n.melee_requests.is_empty())
            .map(|(&h, _)| h)
            .collect();
        attackers.extend(
            self.players
                .iter()
                .flatten()
                .filter(|p| !p.melee_requests.is_empty())
                .map(|p| p.handle),
        );
        let mut presentations = Vec::new();
        for handle in attackers {
            let combatants = self.combatants();
            let Some(view) =
                crate::npc_combat::CombatView::new(&combatants, Some(self.collision.collision()))
            else {
                return;
            };
            let Some(owner) = view.index(handle) else {
                continue;
            };
            // combatants() visits this same live-entity sequence, omitting
            // non-characters. Walk that subsequence once instead of scanning
            // the entire character list for each candidate. Keep the snapshot
            // local to this attacker: earlier melee hits can change it.
            let mut characters = combatants.iter().peekable();
            let candidates: Vec<_> = self
                .entities
                .iter()
                .filter(|e| e.added && !e.removed)
                .filter_map(|e| {
                    use crate::npc_combat::{MeleeGeometry, MeleeTarget};
                    let character = characters.next_if(|c| c.handle == e.handle);
                    let geometry = if let Some(boss) = self.jormungandr.get(&e.handle) {
                        if !boss.update_animation || boss.dead {
                            return None;
                        }
                        MeleeGeometry::BossCapsule {
                            position: boss.frame.head,
                            length: 0.,
                            radius: 1.1,
                        }
                    } else if let Some(c) = character {
                        MeleeGeometry::Character(*c)
                    } else if let Some(d) = self.dispensers.get(&e.handle) {
                        MeleeGeometry::Dispenser(d.axis())
                    } else {
                        let state = self.physics_damage.get(&e.handle)?;
                        if state.has_capsule() {
                            let capsule = state.death_capsule(e.handle)?;
                            MeleeGeometry::BossCapsule {
                                position: capsule.new_axis.origin,
                                length: capsule.new_axis.delta[1],
                                radius: capsule.radius,
                            }
                        } else {
                            MeleeGeometry::Prop(state.world_box(&e.orientation)?)
                        }
                    };
                    let position = character.map_or(e.position, |c| c.position);
                    Some(MeleeTarget {
                        handle: e.handle,
                        query: magicka_sim::entity_query::QueryEntity {
                            position,
                            radius: character.map_or(e.radius, |c| c.radius),
                            dead: character.map_or(e.dead, |c| c.dead),
                            shield: false,
                            damageable: true,
                        },
                        geometry,
                    })
                })
                .collect();
            let mut hits = Vec::new();
            let shield_geometries: Vec<_> = self
                .shields
                .iter()
                .map(|s| (s.handle, s.shield.geometry()))
                .collect();
            let mut absorb_executed = false;
            let mut absorb_results: Vec<(u16, u16)> = Vec::new();
            let mut shield_hits = Vec::new();
            // Non-damage melee events: (conditions, condition flag, item position).
            let mut presented: Vec<(
                Option<Arc<Vec<magicka_content::common::EventCollection>>>,
                u8,
                i32,
                _,
            )> = Vec::new();
            if let Some(npc) = self.npcs.get_mut(&handle) {
                let target = npc.combat.current_target();
                for request in std::mem::take(&mut npc.melee_requests) {
                    let crate::npc_combat::MeleeRequest::Execute {
                        weapon: slot,
                        targets,
                        first,
                    } = request
                    else {
                        let crate::npc_combat::MeleeRequest::Stop(weapon) = request else {
                            unreachable!()
                        };
                        presented.push((None, 0, weapon, [0.0; 3]));
                        continue;
                    };
                    if let Some((_, w)) = npc.weapons.iter_mut().find(|(s, _)| *s == slot) {
                        if first {
                            presented.push((
                                Some(w.conditions.clone()),
                                1u8,
                                slot,
                                w.item_position,
                            ));
                        }
                        let before = w.hitlist.len();
                        if slot == 0 && npc.character.combat.absorb_shield.is_some() {
                            if !w.absorb_can_execute() {
                                continue;
                            }
                            if let Some((shield, _point)) = w.absorb_shield_contact(
                                view.combatants[owner].position,
                                &shield_geometries,
                            ) {
                                for collection in w
                                    .conditions
                                    .iter()
                                    .filter(|c| c.condition.condition_type & 2 != 0)
                                {
                                    for event in &collection.events {
                                        if let magicka_content::common::Event::Damage {
                                            damage,
                                            ..
                                        } = event
                                        {
                                            shield_hits.push((
                                                shield,
                                                magicka_sim::damage::Damage {
                                                    attack: damage.attack_property as u16,
                                                    element: damage.element as u32,
                                                    amount: damage.amount,
                                                    magnitude: damage.magnitude,
                                                },
                                            ));
                                        }
                                    }
                                }
                                presented.push((
                                    Some(w.conditions.clone()),
                                    2,
                                    slot,
                                    w.item_position,
                                ));
                                npc.motion.go_to_animation(34, 0.03);
                                continue;
                            }
                            let next = w.next_to_damage;
                            hits.extend(
                                w.execute_absorb_targets(view.combatants[owner], &candidates),
                            );
                            absorb_results.extend(
                                w.hitlist[next..w.next_to_damage]
                                    .iter()
                                    .map(|target| (*target, 0)),
                            );
                            absorb_executed = true;
                        } else {
                            hits.extend(w.execute_targets(
                                view.combatants[owner],
                                target,
                                targets,
                                &candidates,
                            ));
                        }
                        for _ in before..w.hitlist.len() {
                            presented.push((Some(w.conditions.clone()), 2, slot, w.item_position));
                        }
                    }
                }
            }
            if let Some(player) = self
                .players
                .iter_mut()
                .flatten()
                .find(|p| p.handle == handle)
            {
                for request in std::mem::take(&mut player.melee_requests) {
                    let crate::npc_combat::MeleeRequest::Execute {
                        weapon: slot,
                        targets,
                        first,
                    } = request
                    else {
                        let crate::npc_combat::MeleeRequest::Stop(weapon) = request else {
                            unreachable!()
                        };
                        presented.push((None, 0, weapon, [0.0; 3]));
                        continue;
                    };
                    if let Some((_, item)) = player.items.iter_mut().find(|(s, _)| *s == slot) {
                        if first {
                            presented.push((
                                Some(item.melee.conditions.clone()),
                                1,
                                slot,
                                item.melee.item_position,
                            ));
                        }
                        let before = item.melee.hitlist.len();
                        if !item.melee.conditions.is_empty()
                            && (item.melee.multi_hit || item.melee.hitlist.is_empty())
                        {
                            item.cooldown = item.definition.cooldown_time;
                        }
                        hits.extend(item.melee.execute_targets(
                            view.combatants[owner],
                            None,
                            targets,
                            &candidates,
                        ));
                        for _ in before..item.melee.hitlist.len() {
                            presented.push((
                                Some(item.melee.conditions.clone()),
                                2,
                                slot,
                                item.melee.item_position,
                            ));
                        }
                    }
                }
            }
            let attacker = view.combatants[owner];
            presentations.extend(presented.into_iter().map(|(c, flag, slot, position)| {
                (c, flag, handle, slot, position, attacker.direction)
            }));
            for (target, damage) in shield_hits {
                let boosts = self
                    .npcs
                    .get(&handle)
                    .map(|n| n.character.buffs.damage_boosts())
                    .unwrap_or_default();
                let magicka_sim::damage_delivery::DamagePayload::Single(damage) =
                    magicka_sim::damage_delivery::DamagePayload::Single(damage)
                        .with_boosts(&boosts)
                else {
                    unreachable!()
                };
                if let Some(shield) = self.shields.iter_mut().find(|s| s.handle == target) {
                    shield
                        .shield
                        .internal_damage(&damage, self.role != NetworkRole::Client);
                }
            }
            for (target, damage, position) in hits {
                if let Some(state) = self.physics_damage.get_mut(&target) {
                    if self.role != NetworkRole::Client {
                        let boosts = self
                            .npcs
                            .get(&handle)
                            .map(|p| p.character.buffs.damage_boosts())
                            .or_else(|| {
                                self.players
                                    .iter()
                                    .flatten()
                                    .find(|p| p.handle == handle)
                                    .map(|p| p.character.buffs.damage_boosts())
                            })
                            .unwrap_or_default();
                        let magicka_sim::damage_delivery::DamagePayload::Single(damage) =
                            magicka_sim::damage_delivery::DamagePayload::Single(damage)
                                .with_boosts(&boosts)
                        else {
                            unreachable!()
                        };
                        let result = state.deliver(&[damage], 15, &mut self.pending_triggers);
                        if let Some((_, combined)) =
                            absorb_results.iter_mut().find(|(h, _)| *h == target)
                        {
                            *combined |= result;
                        }
                        if let Some(entity) = self.entities.iter_mut().find(|e| e.handle == target)
                        {
                            entity.hit_points = state.health;
                            entity.dead = state.health <= 0.0;
                        }
                    }
                    continue;
                }
                let mut world = SceneHitWorld {
                    physics_damage: &mut self.physics_damage,
                    dispensers: &mut self.dispensers,
                    entities: &mut self.entities,
                    view: &view,
                    npcs: &mut self.npcs,
                    players: &mut self.players,
                    bodies: &mut self.physics.bodies,
                    triggers: &mut self.pending_triggers,
                    shields: &mut self.shields,
                    collision_rng: &mut self.rng,
                    play_time: self.play_time,
                    entity_blasts: &mut self.entity_blasts,
                    barriers: None,
                    barrier_events: Vec::new(),
                    death: &mut self.death,
                    jormungandr: &mut self.jormungandr,
                    death_damage_features: self.role != NetworkRole::Client,
                };
                let result = crate::missiles::MissileWorld::damage_character(
                    &mut world,
                    target,
                    damage,
                    attacker.handle,
                    position,
                );
                if let Some((_, combined)) = absorb_results.iter_mut().find(|(h, _)| *h == target) {
                    *combined |= result;
                }
            }
            if let Some(npc) = self.npcs.get_mut(&handle) {
                if let Some((_, weapon)) = npc.weapons.iter_mut().find(|(slot, _)| *slot == 0) {
                    for (_, result) in absorb_results {
                        if weapon.absorb_result(result) {
                            npc.motion.go_to_animation(34, 0.03);
                        }
                    }
                    if absorb_executed && !weapon.hitlist.is_empty() {
                        npc.release_absorbed_spell(&self.physics.bodies[npc.body]);
                    }
                }
            }
        }
        for (conditions, flag, owner, weapon, position, facing) in presentations {
            if let Some(conditions) = conditions {
                self.melee_event_presentation(&conditions, flag, owner, weapon, position, facing);
            } else {
                self.item_event_effects
                    .push(crate::npc_combat::ItemEffectRequest::Stop { owner, weapon });
            }
        }
    }

    /// AvatarEvent::MissileLaunched requests of this frame: spawn missiles
    /// (Initialize runs their Default conditions immediately).
    fn spawn_missiles(
        &mut self,
        launches: Vec<(u16, magicka_sim::projectile_launch::ProjectileLaunch)>,
    ) {
        if launches.is_empty() {
            return;
        }
        let combatants = self.combatants();
        let Some(view) =
            crate::npc_combat::CombatView::new(&combatants, Some(self.collision.collision()))
        else {
            return;
        };
        let mut spawned = Vec::new();
        for (owner, launch) in launches {
            let id = self.next_missile;
            self.next_missile = self.next_missile.wrapping_add(1);
            let local_avatar = self.players.iter().flatten().any(|p| p.handle == owner);
            let Some(mut missile) =
                crate::missiles::LiveMissile::spawn(id, owner, local_avatar, &launch)
            else {
                continue;
            };
            let mut world = SceneHitWorld {
                physics_damage: &mut self.physics_damage,
                dispensers: &mut self.dispensers,
                entities: &mut self.entities,
                view: &view,
                npcs: &mut self.npcs,
                players: &mut self.players,
                bodies: &mut self.physics.bodies,
                triggers: &mut self.pending_triggers,
                shields: &mut self.shields,
                collision_rng: &mut self.rng,
                play_time: self.play_time,
                entity_blasts: &mut self.entity_blasts,
                barriers: None,
                barrier_events: Vec::new(),
                death: &mut self.death,
                jormungandr: &mut self.jormungandr,
                death_damage_features: self.role != NetworkRole::Client,
            };
            crate::missiles::initialize(&mut missile, &mut world, &mut self.missile_visuals);
            spawned.push(missile);
        }
        for missile in spawned {
            self.register_missile(missile);
        }
    }

    fn register_missile(&mut self, mut missile: crate::missiles::LiveMissile) {
        missile.handle = Some(self.new_handle());
        self.missiles.push(missile);
    }

    /// The missiles' physics step (after character physics).
    fn missile_physics(&mut self, dt: f32) {
        if self.missiles.is_empty() {
            return;
        }
        let combatants = self.combatants();
        let scene = self.collision.collision();
        let Some(view) = crate::npc_combat::CombatView::new(&combatants, Some(scene)) else {
            return;
        };
        let gravity = magicka_sim::rigid_body::Gravity::magicka();
        let mut barrier_events = Vec::new();
        for missile in &mut self.missiles {
            if missile.body.dead {
                continue;
            }
            let mut world = SceneHitWorld {
                physics_damage: &mut self.physics_damage,
                dispensers: &mut self.dispensers,
                entities: &mut self.entities,
                view: &view,
                npcs: &mut self.npcs,
                players: &mut self.players,
                bodies: &mut self.physics.bodies,
                triggers: &mut self.pending_triggers,
                shields: &mut self.shields,
                collision_rng: &mut self.rng,
                play_time: self.play_time,
                entity_blasts: &mut self.entity_blasts,
                barriers: Some(&mut self.barriers),
                barrier_events: Vec::new(),
                death: &mut self.death,
                jormungandr: &mut self.jormungandr,
                death_damage_features: self.role != NetworkRole::Client,
            };
            crate::missiles::physics_step(
                missile,
                dt,
                gravity,
                &mut world,
                Some(scene),
                &mut self.missile_visuals,
            );
            barrier_events.append(&mut world.barrier_events);
        }
        for (slot, events) in barrier_events {
            self.barrier_events(slot, events);
        }
        self.apply_entity_blasts();
    }

    /// MissileEntity.Update for every live missile, then removal of the
    /// dead ones.
    fn update_missiles(&mut self, dt: f32) {
        if self.missiles.is_empty() {
            return;
        }
        let combatants = self.combatants();
        let Some(view) =
            crate::npc_combat::CombatView::new(&combatants, Some(self.collision.collision()))
        else {
            return;
        };
        for missile in &mut self.missiles {
            if missile.body.dead {
                continue;
            }
            let mut world = SceneHitWorld {
                physics_damage: &mut self.physics_damage,
                dispensers: &mut self.dispensers,
                entities: &mut self.entities,
                view: &view,
                npcs: &mut self.npcs,
                players: &mut self.players,
                bodies: &mut self.physics.bodies,
                triggers: &mut self.pending_triggers,
                shields: &mut self.shields,
                collision_rng: &mut self.rng,
                play_time: self.play_time,
                entity_blasts: &mut self.entity_blasts,
                barriers: None,
                barrier_events: Vec::new(),
                death: &mut self.death,
                jormungandr: &mut self.jormungandr,
                death_damage_features: self.role != NetworkRole::Client,
            };
            crate::missiles::update_at_kill_plane(
                missile,
                dt,
                &mut world,
                &mut self.missile_visuals,
                self.missile_kill_plane,
            );
        }
        self.apply_entity_blasts();
        let visuals = &mut self.missile_visuals;
        self.missiles.retain(|m| {
            if m.body.dead {
                visuals.push(crate::missiles::MissileVisual::Removed { missile: m.body.id });
            }
            !m.body.dead
        });
    }

    /// Run `f` against live characters, shields and MagickDeath as a RailWorld.
    fn with_rail_world<R>(
        &mut self,
        f: impl FnOnce(&mut RailSceneWorld<'_, '_>) -> R,
    ) -> Option<R> {
        let combatants = self.combatants();
        let view =
            crate::npc_combat::CombatView::new(&combatants, Some(self.collision.collision()))?;
        let mut world = RailSceneWorld {
            mines: &mut self.mines,
            portals: &self.portals,
            missiles: &mut self.missiles,
            elemental_eggs: &mut self.elemental_eggs,
            hit: SceneHitWorld {
                physics_damage: &mut self.physics_damage,
                dispensers: &mut self.dispensers,
                entities: &mut self.entities,
                view: &view,
                npcs: &mut self.npcs,
                players: &mut self.players,
                bodies: &mut self.physics.bodies,
                triggers: &mut self.pending_triggers,
                shields: &mut self.shields,
                collision_rng: &mut self.rng,
                play_time: self.play_time,
                entity_blasts: &mut self.entity_blasts,
                barriers: Some(&mut self.barriers),
                barrier_events: Vec::new(),
                death: &mut self.death,
                jormungandr: &mut self.jormungandr,
                death_damage_features: self.role != NetworkRole::Client,
            },
            scene: self.collision.collision(),
            blasts: &mut self.beam_blasts,
            collection_visuals: &mut self.missile_visuals,
            bloat_requests: Vec::new(),
            bloating: self.bloats.keys().copied().collect(),
        };
        let r = f(&mut world);
        let barrier_events = std::mem::take(&mut world.hit.barrier_events);
        let bloat_requests = std::mem::take(&mut world.bloat_requests);
        for (slot, events) in barrier_events {
            self.barrier_events(slot, events);
        }
        for (target, element, killer) in bloat_requests {
            self.bloat_kill(target, element, killer);
        }
        self.apply_entity_blasts();
        Some(r)
    }

    /// Character.BloatKill (Railgun damage that leaves a beam victim at
    /// HitPoints ≤ 0): only characters with gibs or positive health react.
    /// With gibs the character bloats (HitPoints 0) and BloatBlast follows
    /// 0.333 s later; otherwise BloatBlast runs at once.
    fn bloat_kill(&mut self, target: u16, element: u32, killer: u16) {
        let explicit = self
            .entities
            .iter()
            .find(|e| e.handle == target)
            .is_some_and(|e| e.cannot_die_without_explicit_kill);
        let Some((hp, gibs)) = self.character_hp_gibs(target) else {
            return;
        };
        if !((gibs || hp > 0.0) && !explicit) || self.bloats.contains_key(&target) {
            return;
        }
        self.log.push(format!("bloat {target}"));
        if gibs {
            self.set_character_hp(target, 0.0);
            self.bloats.insert(
                target,
                Bloat {
                    element,
                    killer,
                    timer: 0.0,
                },
            );
        } else {
            self.bloat_blast(target, element, killer);
        }
    }

    fn character_hp_gibs(&self, handle: u16) -> Option<(f32, bool)> {
        if let Some(n) = self.npcs.get(&handle) {
            return Some((
                n.character.combat.health.hit_points,
                n.definition.gib_count > 0,
            ));
        }
        self.players
            .iter()
            .flatten()
            .find(|p| p.handle == handle)
            .map(|p| {
                (
                    p.character.combat.health.hit_points,
                    p.character.definition.gib_count > 0,
                )
            })
    }

    fn set_character_hp(&mut self, handle: u16, hp: f32) {
        if let Some(n) = self.npcs.get_mut(&handle) {
            n.character.combat.health.hit_points = hp;
        } else if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == handle)
        {
            p.character.combat.health.hit_points = hp;
        }
    }

    /// Shield.Update for every live shield; dead shields are removed.
    fn update_shields(&mut self, dt: f32) {
        for s in &mut self.shields {
            s.shield.update(dt);
        }
        self.shields.retain(|s| !s.shield.dead);
    }

    /// Liquid.Freeze with Water.Freeze's ordered shield segment clipping.
    /// Retained top-level water uses world-space geometry; animated liquids
    /// and their native local-space shield-query quirk remain unsupported.
    pub fn freeze_water(
        &mut self,
        origin: [f32; 3],
        direction: [f32; 3],
        spread: f32,
        multiplier: f32,
        damages: &[magicka_sim::damage::Damage],
    ) {
        let shields = &self.shields;
        self.collision.collision_mut().freeze_water(
            origin,
            direction,
            spread,
            multiplier,
            damages,
            |segment| {
                for shield in shields {
                    if let Some(fraction) = shield.shield.geometry().segment_fraction(*segment) {
                        segment.delta = segment.delta.map(|v| v * fraction);
                    }
                }
            },
        );
    }

    /// Blast.BlastDamage's closing Liquid.Freeze: a full circle of the
    /// blast radius, multiplier 1 (Blast IL near `Vector3.Right`).
    pub fn freeze_blast(
        &mut self,
        position: [f32; 3],
        radius: f32,
        damages: &[magicka_sim::damage::Damage],
    ) {
        self.freeze_water(
            position,
            [radius, 0.0, 0.0],
            std::f32::consts::TAU,
            1.0,
            damages,
        );
    }

    /// ShieldSpell pure-Shield casts: Shield.Initialize (killing overlapped
    /// shields) and EntityManager.AddEntity.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_shield(
        &mut self,
        owner: u16,
        position: [f32; 3],
        radius: f32,
        direction: [f32; 3],
        kind: magicka_sim::shield_entity::ShieldKind,
        hit_points: f32,
        color: [f32; 3],
    ) -> u16 {
        let mut others: Vec<_> = self.shields.iter().map(|s| s.shield.clone()).collect();
        let shield = magicka_sim::shield_entity::ShieldEntity::initialize(
            Some(owner),
            position,
            radius,
            direction,
            kind,
            hit_points,
            color,
            &mut others,
        );
        for (live, updated) in self.shields.iter_mut().zip(others) {
            live.shield = updated;
        }
        let handle = self.new_handle();
        self.log.push(format!("shield {handle} {kind:?}"));
        self.shields.push(LiveShield { handle, shield });
        handle
    }

    /// AudioManager.PlayCue(bank, hash(name), emitter): a gameplay cue at an
    /// optional emitter position. Returns the request key (for Stop).
    pub fn play_cue(&mut self, bank: u16, name: &str, emitter: Option<[f32; 3]>) -> u32 {
        self.play_cue_magnitude(bank, name, emitter, None)
    }

    /// `play_cue` with SpellSoundVariables (the "Magnitude" variable).
    pub fn play_cue_magnitude(
        &mut self,
        bank: u16,
        name: &str,
        emitter: Option<[f32; 3]>,
        magnitude: Option<f32>,
    ) -> u32 {
        let key = self.backend().next_key();
        self.audio_requests.push(crate::audio::AudioRequest::Start {
            key,
            bank,
            cue: custom_hash(&name.to_ascii_lowercase()),
            volume_db: 0.0,
            source: emitter.map_or(crate::audio::SoundSource::Global, |p| {
                crate::audio::SoundSource::Emitter(p)
            }),
            magnitude,
        });
        key
    }

    /// Move a playing cue's emitter.
    pub fn move_cue(&mut self, key: u32, position: [f32; 3]) {
        self.audio_requests
            .push(crate::audio::AudioRequest::Move { key, position });
    }

    pub fn stop_cue(&mut self, key: u32) {
        self.audio_requests.push(crate::audio::AudioRequest::Stop {
            key,
            instant: false,
        });
    }

    /// PlaySound animation actions discard the native Cue return value;
    /// their lifetime is not tied to action Kill or animation completion.
    fn sync_animation_audio(&mut self) {
        use crate::character_sounds::CharacterAudio;
        let mut sounds = Vec::new();
        let mut character_audio = Vec::new();
        let mut owners = std::collections::BTreeSet::new();
        for (handle, character, body) in self
            .players
            .iter_mut()
            .flatten()
            .map(|p| (p.handle, &mut p.character, p.body))
            .chain(
                self.npcs
                    .iter_mut()
                    .map(|(h, n)| (*h, &mut n.character, n.body)),
            )
        {
            let position = self.physics.bodies[body].position;
            owners.insert(handle);
            sounds.extend(
                std::mem::take(&mut character.pending_animation_audio)
                    .into_iter()
                    .map(|(bank, cue)| (bank, cue, position, None)),
            );
            // Attached sounds: Initialize starts them, Die stops them.
            character.sounds.update(character.combat.dead);
            character_audio.extend(
                std::mem::take(&mut character.sounds.pending)
                    .into_iter()
                    .map(|audio| (handle, audio, position)),
            );
        }
        for (handle, audio, position) in character_audio {
            match audio {
                CharacterAudio::OneShot {
                    bank,
                    cue,
                    magnitude,
                    ..
                } => sounds.push((bank, cue, position, magnitude)),
                CharacterAudio::AttachedStart { slot, bank, cue } => {
                    let key = self.backend().next_key();
                    self.audio_requests.push(crate::audio::AudioRequest::Start {
                        key,
                        bank,
                        cue,
                        volume_db: 0.0,
                        source: crate::audio::SoundSource::Emitter(position),
                        magnitude: None,
                    });
                    self.attached_cue_keys.insert((handle, slot), key);
                }
                CharacterAudio::AttachedStop { slot } => {
                    if let Some(key) = self.attached_cue_keys.remove(&(handle, slot)) {
                        self.stop_cue(key);
                    }
                }
            }
        }
        // Deinitialize of a removed owner stops its attached cues.
        let stale: Vec<_> = self
            .attached_cue_keys
            .keys()
            .filter(|(handle, _)| !owners.contains(handle))
            .copied()
            .collect();
        for owner in stale {
            if let Some(key) = self.attached_cue_keys.remove(&owner) {
                self.stop_cue(key);
            }
        }
        for (bank, cue, position, magnitude) in sounds {
            let key = self.backend().next_key();
            self.audio_requests.push(crate::audio::AudioRequest::Start {
                key,
                bank,
                cue,
                volume_db: 0.0,
                source: crate::audio::SoundSource::Emitter(position),
                magnitude,
            });
        }
    }

    /// Character.InternalDamage starts this one-shot at the supplied impact
    /// position, oriented along attackPosition - Body.Position.
    fn sync_block_effects(&mut self) {
        let effects: Vec<_> = self
            .players
            .iter_mut()
            .flatten()
            .map(|p| &mut p.character)
            .chain(self.npcs.values_mut().map(|n| &mut n.character))
            .flat_map(|c| std::mem::take(&mut c.block_effects))
            .collect();
        for (position, direction) in effects {
            self.start_named_effect("melee_block", position, direction);
        }
    }

    /// Owned Character status cues. Playing/stopping track issued requests;
    /// backend completion/disposal feedback remains presentation work.
    fn sync_status_audio(&mut self, move_cues: bool) {
        use magicka_sim::{character_status::StatusEvent as Event, status::Status};
        let mut owners = BTreeMap::new();
        let mut pending = Vec::new();
        for (handle, character, body) in self
            .players
            .iter_mut()
            .flatten()
            .map(|p| (p.handle, &mut p.character, p.body))
            .chain(
                self.npcs
                    .values_mut()
                    .map(|n| (n.handle, &mut n.character, n.body)),
            )
        {
            let body = &self.physics.bodies[body];
            owners.insert(handle, (body.position, body.direction()));
            pending.extend(
                std::mem::take(&mut character.pending_status_audio)
                    .into_iter()
                    .map(|event| (handle, event)),
            );
        }
        for (handle, event) in pending {
            let (position, direction) = owners[&handle];
            let mut cue_change = None;
            match event {
                Event::PlayStatusCue(kind) => {
                    let name = match kind {
                        Status::Burning => "spell_fire_onfire",
                        Status::Wet => "spell_water_drip",
                        Status::Frozen => "spell_ice_frozen",
                        _ => continue,
                    };
                    let key = self.play_cue(crate::audio::banks::SPELLS, name, Some(position));
                    self.status_cue_keys.insert((handle, kind as u16), key);
                    cue_change = Some((kind as u16, true));
                }
                Event::StopCue(kind) => {
                    if let Some(&key) = self.status_cue_keys.get(&(handle, kind as u16)) {
                        self.stop_cue(key);
                        cue_change = Some((kind as u16, false));
                    }
                }
                Event::PlaySteamCue => {
                    self.play_cue(
                        crate::audio::banks::SPELLS,
                        "spell_steam_hit",
                        Some(position),
                    );
                }
                Event::DryingEffect => {
                    self.status_contact_effects
                        .push(("drying_steam", position, direction))
                }
                Event::StopCurrentSpell => {
                    if let Some(player) = self
                        .players
                        .iter_mut()
                        .flatten()
                        .find(|p| p.handle == handle)
                    {
                        player.character.pending_status_audio.push(event);
                        player.apply_status_spell_stop();
                    }
                }
                Event::Hud { .. } => {}
            }
            if let Some((bit, playing)) = cue_change {
                let character = self
                    .players
                    .iter_mut()
                    .flatten()
                    .find(|p| p.handle == handle)
                    .map(|p| &mut p.character)
                    .or_else(|| self.npcs.get_mut(&handle).map(|n| &mut n.character));
                if let Some(character) = character {
                    character.status_cues.exists |= bit;
                    if playing {
                        character.status_cues.playing |= bit;
                        character.status_cues_stopping &= !bit;
                    } else {
                        character.status_cues.playing &= !bit;
                        character.status_cues_stopping |= bit;
                    }
                }
            }
        }
        if move_cues {
            let cues: Vec<_> = self
                .status_cue_keys
                .iter()
                .map(|(owner, key)| (*owner, *key))
                .collect();
            for ((handle, kind), key) in cues {
                if let Some((position, _)) = owners.get(&handle) {
                    self.move_cue(key, *position);
                } else {
                    self.stop_cue(key);
                    self.status_cue_keys.remove(&(handle, kind));
                }
            }
        }
    }

    /// SpraySpell.CastArea calls SpellEffect.PlaySound before FullBlast.
    /// Retain cue keys for the matching effect's deinitialization.
    fn start_spray_area_audio(
        &mut self,
        owner: u16,
        at: [f32; 3],
        spell: &magicka_sim::spells::Spell,
    ) {
        if let Some(old) = self.spray_area_cues.remove(&owner) {
            for key in old {
                self.stop_cue(key);
            }
        }
        let mut keys = Vec::new();
        for (i, element) in magicka_sim::spells::Elements::BY_INDEX.iter().enumerate() {
            let magnitude = spell.get(*element);
            // Iterator MoveNext uses signed ble to skip values <= 0;
            // an unordered (NaN) value falls through like the native code.
            if magnitude <= 0.0 {
                continue;
            }
            // AudioManager.GetCue returns null for a missing SoundList hash;
            // SpellEffect.PlaySound neither owns nor plays that null cue.
            if !self
                .content
                .cue_exists(custom_hash(crate::audio::SOUNDS_AREA[i]))
            {
                continue;
            }
            keys.push(self.play_cue_magnitude(
                crate::audio::banks::SPELLS,
                crate::audio::SOUNDS_AREA[i],
                Some(at),
                Some(magnitude),
            ));
        }
        self.spray_area_cues.insert(owner, keys);
    }

    /// Inactive, replaced or removed owners release their Area effect cues.
    /// Keep the cue for an active Area effect even when another update has
    /// already drained the cast event. Prepared/release-envelope state belongs
    /// to the presentation backend and is not inferred here.
    pub(crate) fn sync_spray_area_audio(&mut self) {
        let ended: Vec<_> = self
            .spray_area_cues
            .keys()
            .copied()
            .filter(|owner| {
                !self.players.iter().flatten().any(|p| {
                    p.handle == *owner
                        && matches!(
                            &p.current_spell,
                            Some(crate::avatar::SpellEffect::SprayArea(area))
                                if area.lifecycle.active
                        )
                })
            })
            .collect();
        for owner in ended {
            if let Some(keys) = self.spray_area_cues.remove(&owner) {
                for key in keys {
                    self.stop_cue(key);
                }
            }
        }
    }

    /// Detonate's SOUNDS_AREA cues for each element of `elements` (Shield
    /// excluded by the caller).
    fn area_sounds(&mut self, elements: u32, at: [f32; 3]) {
        for (i, e) in magicka_sim::spells::Elements::BY_INDEX.iter().enumerate() {
            if elements & e.0 == e.0 {
                self.play_cue(
                    crate::audio::banks::SPELLS,
                    crate::audio::SOUNDS_AREA[i],
                    Some(at),
                );
            }
        }
    }

    /// ShieldSpell mine casts: each placement probes 4 m down and spawns a
    /// mine at the scene hit (`research/spell-mine.md`).
    pub fn spawn_mines(
        &mut self,
        owner: u16,
        placements: &[magicka_sim::spell_mine::MinePlacement; 2],
        spell: magicka_sim::spells::Spell,
    ) {
        for p in placements {
            self.probe_and_place_mine(owner, p, spell);
        }
    }

    /// One placement: probe down CAST_PROBE_DROP; on a scene hit, a mine.
    fn probe_and_place_mine(
        &mut self,
        owner: u16,
        p: &magicka_sim::spell_mine::MinePlacement,
        spell: magicka_sim::spells::Spell,
    ) {
        let segment = Segment {
            origin: p.probe_origin,
            delta: [0.0, -magicka_sim::spell_mine::CAST_PROBE_DROP, 0.0],
        };
        let Some(hit) = self.collision.collision().segment_intersect(segment, false) else {
            return;
        };
        self.place_mine(
            owner,
            hit.hit.position,
            p.direction,
            p.scale,
            p.range,
            p.next_direction,
            p.next_yaw,
            p.distance,
            spell,
        );
    }

    /// ShieldSpell.AnimationEnd (Weapon): the WALL shield, or the first
    /// mine/barrier of a forward line (no camera shake, unlike the casts).
    pub fn spawn_shield_weapon(
        &mut self,
        owner: u16,
        weapon: magicka_sim::shield_weapon::ShieldWeapon,
        spell: magicka_sim::spells::Spell,
    ) {
        use magicka_sim::shield_weapon::ShieldWeapon;
        match weapon {
            ShieldWeapon::Wall {
                position,
                radius,
                direction,
                kind,
                hit_points,
                color,
            } => {
                self.spawn_shield(owner, position, radius, direction, kind, hit_points, color);
            }
            ShieldWeapon::Mine(p) => self.probe_and_place_mine(owner, &p, spell),
            ShieldWeapon::Barrier { placement, damage } => {
                let list = self.barriers.new_hit_list();
                self.probe_and_place_barrier(owner, list, &placement, spell, damage);
            }
        }
    }

    /// SpellMine.Initialize + EntityManager.AddEntity: mines within 1 m
    /// (GetEntities radius + entity radius) of the new mine detonate.
    /// Barriers are not reconstructed yet.
    #[allow(clippy::too_many_arguments)]
    fn place_mine(
        &mut self,
        owner: u16,
        hit: [f32; 3],
        direction: [f32; 3],
        scale: f32,
        range: f32,
        next_direction: [f32; 3],
        next_yaw: f32,
        distance: f32,
        spell: magicka_sim::spells::Spell,
    ) {
        let Some(clip) = self.mine_clip_duration else {
            self.log
                .push("spell mine: mine clip duration unavailable".into());
            return;
        };
        let model_yaw = self.random() * std::f32::consts::TAU;
        let mine = match magicka_sim::spell_mine::SpellMine::initialize(
            owner,
            hit,
            direction,
            scale,
            range,
            next_direction,
            next_yaw,
            distance,
            spell,
            self.play_time,
            clip,
            model_yaw,
        ) {
            Ok(mine) => mine,
            Err(e) => {
                self.log.push(format!("spell mine damage: {e:?}"));
                return;
            }
        };
        let position = mine.position;
        let near: Vec<usize> = self
            .mines
            .iter()
            .filter(|(_, m)| {
                !m.dead && within(m.position, position, 1.0 + magicka_sim::spell_mine::RADIUS)
            })
            .map(|(slot, _)| slot)
            .collect();
        for slot in near {
            self.detonate_mine(slot);
        }
        let barriers: Vec<usize> = self
            .barriers
            .iter()
            .filter(|(_, b)| !b.is_dead() && within(b.position, position, 1.0 + b.radius))
            .map(|(s, _)| s)
            .collect();
        for barrier in barriers {
            self.kill_barrier(barrier);
        }
        self.play_cue(crate::audio::banks::SPELLS, "spell_mine01", Some(position));
        let slot = self.mines.add(mine);
        self.log.push(format!("spell mine {slot}"));
    }

    /// SpellMine.Detonate: FullBlast (radius 3, the mine itself ignored, so
    /// the caster can be hit), mine_explosion facing Forward, then Dead.
    /// Neighbouring mines take the blast through InternalDamage.
    fn detonate_mine(&mut self, slot: usize) {
        let Some(mine) = self.mines.get_mut(slot) else {
            return;
        };
        mine.detonate();
        let (owner, timestamp, position, damage) =
            (mine.owner, mine.timestamp, mine.position, mine.damage);
        let elements = mine.spell.element.0 & !magicka_sim::spells::Elements::SHIELD.0;
        self.area_sounds(elements, position);
        self.mine_effects
            .push(("mine_explosion", position, [0.0, 0.0, -1.0]));
        let radius = magicka_sim::spell_mine::BLAST_RADIUS;
        if self
            .with_rail_world(|world| {
                world.collection_blast(Some(owner), None, timestamp, radius, position, damage);
            })
            .is_none()
        {
            self.log
                .push("spell mine blast: entity grid unavailable".into());
        }
    }

    /// Blast.BlastDamage's non-character damageables for every recorded
    /// FullBlast: physics props, mines and barriers (dead ones are skipped, which also
    /// excludes the detonating mine or barrier).
    fn apply_entity_blasts(&mut self) {
        while !self.entity_blasts.is_empty() {
            for blast in std::mem::take(&mut self.entity_blasts) {
                let (position, radius, damages) = match blast {
                    EntityBlast::Full(position, radius, damages) => (position, radius, damages),
                    EntityBlast::Splash {
                        position,
                        radius,
                        damage,
                        owner,
                        ignore,
                    } => {
                        self.arc_damage(
                            Some(owner),
                            ignore,
                            position,
                            [0., 0., -1.],
                            radius,
                            std::f32::consts::PI,
                            &[damage],
                            self.play_time,
                        );
                        continue;
                    }
                };
                self.blast_physics(position, radius, damages);
                self.blast_missiles(position, radius, damages);
                self.blast_death(position, radius, damages);
                self.blast_jormungandr(position, radius, damages);
                self.blast_elemental_eggs(position, radius, damages);
                self.blast_mines(None, position, radius, damages);
                self.blast_barriers(None, position, radius, damages);
                // BlastDamage tail: Liquid.Freeze(position, Right·radius,
                // 2π, 1) — a Cold blast freezes nearby water and lava.
                self.freeze_blast(position, radius, &damages);
            }
        }
    }

    fn blast_missiles(
        &mut self,
        position: [f32; 3],
        radius: f32,
        damages: [magicka_sim::damage::Damage; 5],
    ) {
        if self.role == NetworkRole::Client {
            return;
        }
        for m in self.missiles.iter_mut().filter(|m| !m.body.dead) {
            if !within(m.body.position, position, radius + m.body.radius) {
                continue;
            }
            let mut seg = Segment {
                origin: position,
                delta: std::array::from_fn(|i| m.body.position[i] - position[i]),
            };
            if let Some(point) =
                crate::missiles::segment_intersect(m.body.position, m.body.radius, seg, 0.0)
            {
                seg.delta = std::array::from_fn(|i| point[i] - position[i]);
            }
            if self
                .collision
                .collision()
                .segment_intersect(seg, false)
                .is_some()
            {
                continue;
            }
            let distance = seg.delta.iter().map(|v| v * v).sum::<f32>().sqrt() - m.body.radius;
            if distance > radius {
                continue;
            }
            for d in magicka_sim::blast_damage::collection(damages, distance, radius) {
                m.state.receive_damage(d, false);
            }
        }
    }

    /// Non-character IDamageable entries in BlastDamage. Clip the segment
    /// against the prop skin before testing scene occlusion and falloff.
    fn blast_physics(
        &mut self,
        position: [f32; 3],
        radius: f32,
        damages: [magicka_sim::damage::Damage; 5],
    ) {
        if self.role == NetworkRole::Client {
            return;
        }
        for entity in self
            .entities
            .iter_mut()
            .filter(|e| e.added && !e.removed && !e.dead)
        {
            let Some(state) = self.physics_damage.get_mut(&entity.handle) else {
                continue;
            };
            if !within(entity.position, position, radius + entity.radius) {
                continue;
            }
            let mut segment = Segment {
                origin: position,
                delta: std::array::from_fn(|i| entity.position[i] - position[i]),
            };
            if let Some(hit) = state
                .world_box(&entity.orientation)
                .and_then(|b| b.segment_intersect(segment))
            {
                segment.delta = segment.delta.map(|v| v * hit.fraction);
            }
            if self
                .collision
                .collision()
                .segment_intersect(segment, false)
                .is_some()
            {
                continue;
            }
            let distance = segment.delta.iter().map(|v| v * v).sum::<f32>().sqrt() - entity.radius;
            if distance > radius {
                continue;
            }
            let damages = magicka_sim::blast_damage::collection(damages, distance, radius);
            state.deliver(&damages, 15, &mut self.pending_triggers);
            entity.hit_points = state.health;
            entity.dead = state.health <= 0.0;
        }
    }

    /// Blast.BlastDamage over SpellMines: the segment to the mine centre is
    /// clipped at its 0.9 m sphere, scene-occluded hits are skipped, and
    /// the remaining distance minus the 1 m entity radius selects falloff.
    fn blast_mines(
        &mut self,
        ignore: Option<usize>,
        position: [f32; 3],
        radius: f32,
        damages: [magicka_sim::damage::Damage; 5],
    ) {
        use magicka_sim::spell_mine::{COLLISION_RADIUS, RADIUS};
        let candidates: Vec<usize> = self
            .mines
            .iter()
            .filter(|(slot, m)| {
                Some(*slot) != ignore && !m.dead && within(m.position, position, radius + RADIUS)
            })
            .map(|(slot, _)| slot)
            .collect();
        for slot in candidates {
            let target = self.mines.get(slot).unwrap().position;
            let full = std::array::from_fn::<f32, 3, _>(|i| target[i] - position[i]);
            let length = full.iter().map(|v| v * v).sum::<f32>().sqrt();
            let delta = if length > COLLISION_RADIUS {
                full.map(|v| v * (length - COLLISION_RADIUS) / length)
            } else {
                full
            };
            if self
                .collision
                .collision()
                .segment_intersect(
                    Segment {
                        origin: position,
                        delta,
                    },
                    false,
                )
                .is_some()
            {
                continue;
            }
            let distance = delta.iter().map(|v| v * v).sum::<f32>().sqrt() - RADIUS;
            if distance > radius {
                continue;
            }
            let d = magicka_sim::blast_damage::collection(damages, distance, radius);
            if let Some(mine) = self.mines.get_mut(slot) {
                mine.internal_damage(&d);
            }
        }
    }

    /// SpellMine.OnCollision for every live mine against characters and
    /// missiles that overlap its 0.9 m sphere this step.
    fn mine_contacts(&mut self) {
        use magicka_sim::spell_mine::{ContactResult, Toucher, COLLISION_RADIUS};
        let tolerance = magicka_sim::character_physics::COLLISION_TOLERANCE;
        let mut characters = Vec::new();
        for p in self.players.iter().flatten() {
            characters.push((p.handle, p.body, p.dead()));
        }
        for (handle, n) in &self.npcs {
            characters.push((*handle, n.body, n.dead()));
        }
        let slots: Vec<usize> = self.mines.iter().map(|(s, _)| s).collect();
        for slot in slots {
            let center = self.mines.get(slot).unwrap().position;
            let mut touchers = Vec::new();
            for &(handle, body, dead) in &characters {
                let b = &self.physics.bodies[body];
                let half = b.length * 0.5;
                let y = center[1].clamp(b.position[1] - half, b.position[1] + half);
                let reach = COLLISION_RADIUS + b.radius + tolerance;
                if within(center, [b.position[0], y, b.position[2]], reach) {
                    let owner = self.mines.get(slot).unwrap().owner == handle;
                    touchers.push(Toucher::Character {
                        owner,
                        dead,
                        ethereal: self.combatant_ethereal(handle),
                        levitating: b.owner.levitating,
                    });
                }
            }
            for m in &self.missiles {
                let reach = COLLISION_RADIUS + m.body.radius + tolerance;
                if within(center, m.body.position, reach) {
                    touchers.push(Toucher::Other {
                        owner: false,
                        dead: m.body.dead,
                    });
                }
            }
            for toucher in touchers {
                let Some(mine) = self.mines.get_mut(slot) else {
                    break;
                };
                if let ContactResult::Armed { first: true } = mine.on_collision(toucher) {
                    self.mine_effects
                        .push(("mine_activation", center, [0.0, 0.0, -1.0]));
                    self.play_cue(
                        crate::audio::banks::SPELLS,
                        "spell_mine_activate",
                        Some(center),
                    );
                }
            }
        }
    }

    fn combatant_ethereal(&self, handle: u16) -> bool {
        self.combatants()
            .iter()
            .any(|c| c.handle == handle && c.ethereal)
    }

    /// SpellMine.Update for every mine in entity order, then removal of
    /// dead mines whose spawn clip has finished.
    fn update_mines(&mut self, dt: f32) {
        let slots: Vec<usize> = self.mines.iter().map(|(s, _)| s).collect();
        for slot in slots {
            let Some(owner) = self.mines.get(slot).map(|m| m.owner) else {
                continue;
            };
            let owner_position = self.character_position(owner);
            let update = self.mines.get_mut(slot).unwrap().update(dt, owner_position);
            if let Some(next) = update.spawn_next {
                self.spawn_next_mine(slot, next);
            }
            for _ in 0..update.detonations {
                self.detonate_mine(slot);
            }
        }
        self.mines.remove_dead();
    }

    /// SpellMine.SpawnNextMine: shields between the mine and the probe
    /// cancel the chain; otherwise a 1.5 m downward probe places the mine.
    fn spawn_next_mine(&mut self, slot: usize, next: magicka_sim::spell_mine::NextMine) {
        let towards = Segment {
            origin: next.from,
            delta: std::array::from_fn(|i| next.probe_origin[i] - next.from[i]),
        };
        if self
            .shields
            .iter()
            .any(|s| s.shield.geometry().segment_fraction(towards).is_some())
        {
            return;
        }
        let probe = Segment {
            origin: next.probe_origin,
            delta: [0.0, -next.probe_drop, 0.0],
        };
        let Some(hit) = self.collision.collision().segment_intersect(probe, false) else {
            return;
        };
        let Some(mine) = self.mines.get(slot) else {
            return;
        };
        let (owner, spell) = (mine.owner, mine.spell);
        self.place_mine(
            owner,
            hit.hit.position,
            next.direction,
            next.scale,
            next.range,
            next.next_direction,
            next.next_yaw,
            next.distance,
            spell,
        );
    }

    fn character_position(&self, handle: u16) -> Option<[f32; 3]> {
        let body = self
            .players
            .iter()
            .flatten()
            .find(|p| p.handle == handle)
            .map(|p| p.body)
            .or_else(|| self.npcs.get(&handle).map(|n| n.body))?;
        Some(self.physics.bodies[body].position)
    }

    /// ShieldSpell barrier casts: a new shared hit list, two probes 4 m down
    /// and a camera shake (0.25 + rand·0.5 for 0.5·Shield seconds).
    pub fn spawn_barriers(
        &mut self,
        owner: u16,
        cast: &magicka_sim::barrier::BarrierCast,
        spell: magicka_sim::spells::Spell,
    ) {
        let list = self.barriers.new_hit_list();
        for p in &cast.placements {
            self.probe_and_place_barrier(owner, list, p, spell, cast.damage);
        }
        let magnitude = 0.25 + self.random() * 0.5;
        self.camera.shake(
            magnitude,
            0.5 * spell.get(magicka_sim::spells::Elements::SHIELD),
        );
    }

    /// One placement: probe down CAST_PROBE_DROP; on a scene hit, a barrier.
    fn probe_and_place_barrier(
        &mut self,
        owner: u16,
        list: usize,
        p: &magicka_sim::barrier::BarrierPlacement,
        spell: magicka_sim::spells::Spell,
        damage: [magicka_sim::damage::Damage; 5],
    ) {
        let segment = Segment {
            origin: p.probe_origin,
            delta: [0.0, -magicka_sim::barrier::CAST_PROBE_DROP, 0.0],
        };
        let Some(hit) = self.collision.collision().segment_intersect(segment, false) else {
            return;
        };
        self.place_barrier(
            Some(owner),
            list,
            hit.hit.position,
            p.direction,
            p.scale,
            p.range,
            p.next_direction,
            p.next_yaw,
            p.distance,
            spell,
            damage,
        );
    }

    /// Barrier.Initialize + AddEntity.
    #[allow(clippy::too_many_arguments)]
    fn place_barrier(
        &mut self,
        owner: Option<u16>,
        list: usize,
        hit: [f32; 3],
        direction: [f32; 3],
        scale: f32,
        range: f32,
        next_direction: [f32; 3],
        next_yaw: f32,
        distance: f32,
        spell: magicka_sim::spells::Spell,
        damage: [magicka_sim::damage::Damage; 5],
    ) {
        use magicka_sim::barrier::{Barrier, BarrierInit};
        let Some(clips) = self.barrier_clips else {
            self.log.push("barrier: clip durations unavailable".into());
            return;
        };
        let (slot, stale) = self.barriers.take();
        let init = BarrierInit {
            owner,
            hit_list: list,
            hit,
            direction,
            scale,
            range,
            next_direction,
            next_yaw,
            distance,
            timestamp: self.play_time,
        };
        let (barrier, visuals, initial) =
            match Barrier::initialize(init, spell, damage, &clips, stale) {
                Ok(v) => v,
                Err(e) => {
                    self.barriers.give_back(slot);
                    self.log.push(format!("barrier: {e:?}"));
                    return;
                }
            };
        // Kill other casts' barriers and detonate mines near the hit.
        let kill_radius = barrier.query_radius(0.8);
        self.clear_for_barrier(list, hit, kill_radius);
        self.barrier_initial_damage(&barrier, hit, initial);
        let forward = direction.map(|v| v * scale);
        for name in &visuals.spawn_effects {
            self.barrier_visuals.push(BarrierVisual::OneShot {
                name,
                position: hit,
                direction: forward,
            });
        }
        if let Some(name) = visuals.effect {
            let transform = if barrier.solid() {
                let mut m = magicka_content::skinned::IDENTITY;
                m[12..15].copy_from_slice(&hit);
                m
            } else {
                barrier_wall_matrix(direction, scale, hit)
            };
            self.barrier_visuals.push(BarrierVisual::Start {
                slot,
                name,
                transform,
            });
        }
        self.log.push(format!("barrier {slot} {:?}", barrier.kind));
        if let Some(cue) = visuals.cue {
            let key = self.play_cue(crate::audio::banks::SPELLS, cue, Some(barrier.position));
            if let Some(old) = self.barrier_cues.insert(slot, key) {
                self.stop_cue(old);
            }
        }
        self.barriers.add(slot, barrier);
    }

    /// Barrier.Initialize's GetEntities pass: other casts' live barriers
    /// within `radius` of the hit are killed, mines detonate.
    pub(super) fn clear_for_barrier(&mut self, list: usize, hit: [f32; 3], radius: f32) {
        let doomed: Vec<usize> = self
            .barriers
            .iter()
            .filter(|(_, b)| {
                b.hit_list != list && !b.is_dead() && within(b.position, hit, radius + b.radius)
            })
            .map(|(s, _)| s)
            .collect();
        for other in doomed {
            self.kill_barrier(other);
        }
        let mines: Vec<usize> = self
            .mines
            .iter()
            .filter(|(_, m)| {
                !m.dead && within(m.position, hit, radius + magicka_sim::spell_mine::RADIUS)
            })
            .map(|(s, _)| s)
            .collect();
        for mine in mines {
            self.detonate_mine(mine);
        }
    }

    /// Initialize's mInitilizeDamage pass: damageables within
    /// scale·radius·1.25 of the hit (not the owner, not already listed, not
    /// this cast's barriers) take the Projectile/Area collection (zero for
    /// Elemental walls) and enter the shared hit list.
    fn barrier_initial_damage(
        &mut self,
        barrier: &magicka_sim::barrier::Barrier,
        hit: [f32; 3],
        initial: [magicka_sim::damage::Damage; 5],
    ) {
        let reach = barrier.query_radius(1.25);
        let list = barrier.hit_list;
        let zero = initial.iter().all(|d| d.attack == 0);
        let candidates: Vec<u16> = self
            .spray_entities()
            .0
            .iter()
            .filter(|c| {
                c.damageable
                    && !matches!(
                        c.geometry,
                        crate::spray_world::ArcGeometry::Barrier { .. }
                            | crate::spray_world::ArcGeometry::Mine
                    )
                    && Some(c.handle) != barrier.owner
                    && within(c.position, hit, reach + c.radius)
            })
            .map(|c| c.handle)
            .collect();
        let mut lists = std::mem::take(&mut self.barriers.hit_lists);
        let owner = barrier.owner;
        let timestamp = barrier.timestamp;
        let victims: Vec<u16> = candidates
            .into_iter()
            .filter(|h| !lists[list].1.contains(*h))
            .collect();
        if !zero && !victims.is_empty() {
            let delivered = self.with_rail_world(|world| {
                for &h in &victims {
                    world.deliver_collection(h, owner, initial, timestamp, hit);
                }
            });
            if delivered.is_none() {
                self.log
                    .push("barrier damage: entity grid unavailable".into());
            }
        }
        for h in victims {
            lists[list].1.add(h);
        }
        self.barriers.hit_lists = lists;
        let others: Vec<usize> = self
            .barriers
            .iter()
            .filter(|(s, b)| {
                b.hit_list != list
                    && !b.is_dead()
                    && within(b.position, hit, reach + b.radius)
                    && !self.barriers.hit_lists[list]
                        .1
                        .contains(BARRIER_KEY | *s as u16)
            })
            .map(|(s, _)| s)
            .collect();
        for other in others {
            self.barriers.hit_lists[list]
                .1
                .add(BARRIER_KEY | other as u16);
            self.damage_barrier(other, &initial);
        }
        let mines: Vec<usize> = self
            .mines
            .iter()
            .filter(|(s, m)| {
                !m.dead
                    && within(m.position, hit, reach + magicka_sim::spell_mine::RADIUS)
                    && !self.barriers.hit_lists[list]
                        .1
                        .contains(MINE_KEY | *s as u16)
            })
            .map(|(s, _)| s)
            .collect();
        for mine in mines {
            self.barriers.hit_lists[list].1.add(MINE_KEY | mine as u16);
            if let Some(m) = self.mines.get_mut(mine) {
                m.internal_damage(&initial);
            }
        }
    }

    fn kill_barrier(&mut self, slot: usize) {
        let Some(b) = self.barriers.get_mut(slot) else {
            return;
        };
        let events = b.kill();
        self.barrier_events(slot, events);
    }

    /// Barrier.InternalDamage(collection) with the Damage feature.
    fn damage_barrier(&mut self, slot: usize, damages: &[magicka_sim::damage::Damage; 5]) {
        let Some(b) = self.barriers.get_mut(slot) else {
            return;
        };
        let (_, events) = b.internal_damage(damages, true);
        self.barrier_events(slot, events);
    }

    /// Presentation and world side effects of barrier events, in order.
    fn barrier_events(&mut self, slot: usize, events: Vec<magicka_sim::barrier::BarrierEvent>) {
        use magicka_sim::barrier::BarrierEvent as Ev;
        for event in events {
            let Some(b) = self.barriers.get(slot) else {
                return;
            };
            let base = b.base();
            let forward = b.direction.map(|v| v * b.scale);
            match event {
                Ev::SpawnNext(next) => self.spawn_next_barrier(slot, next),
                Ev::IceDeath => {
                    self.play_cue(
                        crate::audio::banks::SPELLS,
                        "spell_ice_barrier_break",
                        Some(base),
                    );
                    self.barrier_visuals.push(BarrierVisual::OneShot {
                        name: "barrier_ice_death",
                        position: base,
                        direction: forward,
                    })
                }
                Ev::Death { ice, earth } | Ev::DeathVisuals { ice, earth } => {
                    let cue = if ice && earth {
                        Some("spell_ice_earth_barrier_break")
                    } else if ice {
                        Some("spell_ice_barrier_break")
                    } else if earth {
                        Some("spell_earth_barrier_break")
                    } else {
                        None
                    };
                    if let Some(cue) = cue {
                        self.play_cue(crate::audio::banks::SPELLS, cue, Some(base));
                    }
                    for (on, name) in [(ice, "barrier_ice_death"), (earth, "barrier_earth_death")] {
                        if on {
                            self.barrier_visuals.push(BarrierVisual::OneShot {
                                name,
                                position: base,
                                direction: forward,
                            });
                        }
                    }
                }
                Ev::Detonate => self.detonate_barrier(slot),
                Ev::StopEffect => self.barrier_visuals.push(BarrierVisual::Stop { slot }),
            }
        }
    }

    /// Barrier.Detonate: FullBlast radius 5 of mDamage (the barrier itself
    /// ignored) and mine_explosion facing Forward.
    fn detonate_barrier(&mut self, slot: usize) {
        let Some(b) = self.barriers.get(slot) else {
            return;
        };
        let (owner, timestamp, position, damage) = (b.owner, b.timestamp, b.position, b.damage);
        let forward = b.direction.map(|v| v * b.scale);
        let elements = b.spell.element.0 & !magicka_sim::spells::Elements::SHIELD.0;
        self.area_sounds(elements, position);
        self.barrier_visuals.push(BarrierVisual::OneShot {
            name: "mine_explosion",
            position,
            direction: forward,
        });
        let radius = magicka_sim::barrier::DETONATE_RADIUS;
        if self
            .with_rail_world(|world| {
                world.collection_blast(owner, None, timestamp, radius, position, damage);
            })
            .is_none()
        {
            self.log
                .push("barrier blast: entity grid unavailable".into());
        }
    }

    /// Blast.BlastDamage over barriers: INFERRED skin clip at the capsule
    /// radius, then minus the entity radius (mRadius), scene-occluded hits
    /// skipped.
    fn blast_barriers(
        &mut self,
        ignore: Option<usize>,
        position: [f32; 3],
        radius: f32,
        damages: [magicka_sim::damage::Damage; 5],
    ) {
        let candidates: Vec<(usize, [f32; 3], f32)> = self
            .barriers
            .iter()
            .filter(|(s, b)| {
                Some(*s) != ignore
                    && !b.is_dead()
                    && within(b.position, position, radius + b.radius)
            })
            .map(|(s, b)| (s, b.position, b.radius))
            .collect();
        for (slot, target, r) in candidates {
            let full: [f32; 3] = std::array::from_fn(|i| target[i] - position[i]);
            let length = full.iter().map(|v| v * v).sum::<f32>().sqrt();
            let delta = if length > r {
                full.map(|v| v * (length - r) / length)
            } else {
                full
            };
            if self
                .collision
                .collision()
                .segment_intersect(
                    Segment {
                        origin: position,
                        delta,
                    },
                    false,
                )
                .is_some()
            {
                continue;
            }
            let distance = delta.iter().map(|v| v * v).sum::<f32>().sqrt() - r;
            if distance > radius {
                continue;
            }
            let d = magicka_sim::blast_damage::collection(damages, distance, radius);
            self.damage_barrier(slot, &d);
        }
    }

    /// Barrier.SpawnNextBarrier.
    fn spawn_next_barrier(&mut self, slot: usize, next: magicka_sim::barrier::NextBarrier) {
        let towards = Segment {
            origin: next.from,
            delta: std::array::from_fn(|i| next.probe_origin[i] - next.from[i]),
        };
        if self
            .shields
            .iter()
            .any(|s| s.shield.geometry().segment_fraction(towards).is_some())
        {
            return;
        }
        let probe = Segment {
            origin: next.probe_origin,
            delta: [0.0, -next.probe_drop, 0.0],
        };
        let Some(hit) = self.collision.collision().segment_intersect(probe, false) else {
            return;
        };
        let Some(b) = self.barriers.get(slot) else {
            return;
        };
        let (owner, list, spell, damage, wave) = (b.owner, b.hit_list, b.spell, b.damage, b.wave);
        let at = hit.hit.position;
        self.clear_for_barrier(list, at, next.clear_radius);
        if let Some(pillar) = wave {
            // WaveEntity.SpawnNextBarrier initializes another WaveEntity.
            let placement = magicka_sim::barrier::BarrierPlacement {
                probe_origin: at,
                direction: next.direction,
                scale: next.scale,
                range: next.range,
                next_direction: next.next_direction,
                next_yaw: next.next_yaw,
                distance: next.distance,
            };
            self.place_wave_pillar(owner, list, pillar.wave, &placement, at, spell, damage);
            return;
        }
        self.place_barrier(
            owner,
            list,
            at,
            next.direction,
            next.scale,
            next.range,
            next.next_direction,
            next.next_yaw,
            next.distance,
            spell,
            damage,
        );
    }

    /// Barrier.Update for every barrier in entity order, hit-list ageing by
    /// each list's first owner, particle-wall orientation, then removal.
    fn update_barriers(&mut self, dt: f32) {
        let slots: Vec<usize> = self.barriers.iter().map(|(s, _)| s).collect();
        for slot in slots {
            let Some(b) = self.barriers.get_mut(slot) else {
                continue;
            };
            let mut events = b.update(dt);
            let spawn: Vec<_> = events
                .iter()
                .filter(|e| matches!(e, magicka_sim::barrier::BarrierEvent::SpawnNext(_)))
                .cloned()
                .collect();
            events.retain(|e| !matches!(e, magicka_sim::barrier::BarrierEvent::SpawnNext(_)));
            self.barrier_events(slot, spawn);
            if self.barriers.ages_hit_list(slot) {
                let list = self.barriers.get(slot).unwrap().hit_list;
                self.barriers.hit_lists[list].1.update(dt);
            }
            self.barrier_events(slot, events);
            if let Some(b) = self.barriers.get(slot) {
                if !b.solid() {
                    let transform = barrier_wall_matrix(b.direction, b.scale, b.position);
                    self.barrier_visuals
                        .push(BarrierVisual::Update { slot, transform });
                }
            }
        }
        // Deinitialize stops the barrier's effect and sound cue.
        for slot in self.barriers.remove_dead_slots() {
            self.barrier_visuals.push(BarrierVisual::Stop { slot });
            if let Some(key) = self.barrier_cues.remove(&slot) {
                self.stop_cue(key);
            }
        }
    }

    /// Barrier.OnCollision against characters overlapping either primitive
    /// this step: contact damage (or a lightning bolt) through the shared
    /// hit list. Physical blocking is in the physics pass.
    fn barrier_contacts(&mut self) {
        let tolerance = magicka_sim::character_physics::COLLISION_TOLERANCE;
        struct ContactTarget {
            handle: u16,
            position: [f32; 3],
            radius: f32,
            length: f32,
            dead: bool,
            ethereal: bool,
        }
        let mut combatants: Vec<_> = self
            .combatants()
            .into_iter()
            .map(|c| ContactTarget {
                handle: c.handle,
                position: c.position,
                radius: c.radius,
                length: c.length,
                dead: c.dead,
                ethereal: c.ethereal,
            })
            .collect();
        // BossDamageZone capsules are IDamageable collision participants too.
        // Their centres/lengths differ from the ordinary Character capsule.
        for e in self.entities.iter().filter(|e| e.added && !e.removed) {
            let Some(state) = self.physics_damage.get(&e.handle) else {
                continue;
            };
            let Some(c) = state.death_capsule(e.handle) else {
                continue;
            };
            let origin = c.new_axis.origin;
            let length = c.new_axis.delta[1];
            combatants.push(ContactTarget {
                handle: e.handle,
                position: [origin[0], origin[1] + length * 0.5, origin[2]],
                radius: c.radius,
                length,
                dead: e.dead || state.health <= 0.,
                ethereal: false,
            });
        }
        let mut contacts = Vec::new();
        let mut wave_contacts = Vec::new();
        for (slot, b) in self.barriers.iter() {
            // Primitive 0: vertical capsule from Position − 1 to −0.9 (local
            // position Down, length 0.1). Primitive 1: sphere at Position −
            // mRadius, radius 2·mRadius (Solid) or mRadius.
            let capsule = (b.position[1] - 1.0, b.position[1] - 0.9);
            let sphere_y = b.position[1] - b.radius;
            let sphere_r = if b.solid() { b.radius * 2.0 } else { b.radius };
            for c in &combatants {
                if c.dead {
                    continue;
                }
                let half = c.length * 0.5;
                let body = (c.position[1] - half, c.position[1] + half);
                let flat = ((c.position[0] - b.position[0]).powi(2)
                    + (c.position[2] - b.position[2]).powi(2))
                .sqrt();
                let gap = (capsule.0.max(body.0) - capsule.1.min(body.1)).max(0.0);
                let capsule_hit = flat.hypot(gap) <= b.radius + c.radius + tolerance;
                let dy = (sphere_y.clamp(body.0, body.1) - sphere_y).abs();
                let sphere_hit = flat.hypot(dy) <= sphere_r + c.radius + tolerance;
                if !(capsule_hit || sphere_hit) {
                    continue;
                }
                if b.wave.is_some() {
                    wave_contacts.push((slot, c.handle));
                    continue;
                }
                let list = &self.barriers.hit_lists[b.hit_list].1;
                if b.contact_damages(c.ethereal, list, c.handle) {
                    contacts.push((slot, c.handle, c.position));
                }
            }
        }
        if !wave_contacts.is_empty() {
            self.wave_contacts(wave_contacts);
        }
        if contacts.is_empty() {
            return;
        }
        let mut lists = std::mem::take(&mut self.barriers.hit_lists);
        let mut bolts = Vec::new();
        let mut rng = self.rng;
        let barriers: BTreeMap<usize, magicka_sim::barrier::Barrier> = contacts
            .iter()
            .filter_map(|(slot, _, _)| self.barriers.get(*slot).map(|b| (*slot, b.clone())))
            .collect();
        let delivered = self.with_rail_world(|world| {
            for (slot, target, position) in contacts {
                let b = &barriers[&slot];
                let list = &mut lists[b.hit_list].1;
                if list.contains(target) {
                    continue;
                }
                if b.spell
                    .element
                    .has(magicka_sim::spells::Elements::LIGHTNING)
                {
                    let mut source = b.position;
                    if !b.solid() {
                        source[1] += 1.0;
                    }
                    let t = magicka_sim::lightning::LightningTarget {
                        handle: target,
                        position,
                        dead: false,
                        eligible: true,
                        frozen: false,
                        ends_chain: false,
                    };
                    let mut host = BarrierLightning {
                        world,
                        bolts: &mut bolts,
                        rng: &mut rng,
                    };
                    magicka_sim::lightning::cast_bolt_at(
                        &mut host,
                        b.owner.unwrap_or(u16::MAX),
                        b.timestamp,
                        source,
                        t,
                        list,
                        [1.0; 3],
                        1.0,
                        1.0,
                        b.damage,
                    );
                    if !list.contains(target) {
                        list.add(target);
                    }
                } else {
                    world.deliver_collection(target, b.owner, b.damage, b.timestamp, b.position);
                    list.add(target);
                }
            }
        });
        self.rng = rng;
        self.barriers.hit_lists = lists;
        self.barrier_visuals.extend(bolts);
        if delivered.is_none() {
            self.log
                .push("barrier contact: entity grid unavailable".into());
        }
    }

    /// Character.Update bloat timer (after the entity update).
    fn update_bloats(&mut self, dt: f32) {
        let mut due = Vec::new();
        for (handle, bloat) in &mut self.bloats {
            bloat.timer += dt;
            if bloat.timer >= 0.333 {
                due.push((*handle, bloat.element, bloat.killer));
            }
        }
        for (handle, element, killer) in due {
            self.bloat_blast(handle, element, killer);
        }
        // Bloating characters stand still (SpeedMultiplier/Movement zero).
        for handle in self.bloats.keys().copied().collect::<Vec<_>>() {
            let body = self.npcs.get(&handle).map(|n| n.body).or_else(|| {
                self.players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == handle)
                    .map(|p| p.body)
            });
            if let Some(b) = body {
                self.physics.bodies[b].set_movement([0.0; 3]);
            }
        }
    }

    /// Character.BloatBlast: a FullBlast of the beam element (Life −180,
    /// otherwise 225 damage; magnitude 1 + MaxHitPoints/5000) with radius
    /// Radius + 2.5 at the character, then OverKill with gibs, else Kill.
    fn bloat_blast(&mut self, handle: u16, element: u32, killer: u16) {
        self.bloats.remove(&handle);
        let Some(npc_or_player) = self
            .npcs
            .get(&handle)
            .map(|n| (n.body, n.definition.max_hit_points, n.definition.gib_count))
            .or_else(|| {
                self.players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == handle)
                    .map(|p| {
                        (
                            p.body,
                            p.character.definition.max_hit_points,
                            p.character.definition.gib_count,
                        )
                    })
            })
        else {
            return;
        };
        let (body, max_hp, gibs) = npc_or_player;
        let position = self.physics.bodies[body].position;
        let radius = self.physics.bodies[body].radius + 2.5;
        if element != 0 {
            let damage = magicka_sim::damage::Damage {
                attack: magicka_sim::damage::attack::DAMAGE,
                element,
                amount: if element == magicka_sim::spells::Elements::LIFE.0 {
                    -180.0
                } else {
                    225.0
                },
                magnitude: 1.0 + max_hp / 5000.0,
            };
            let mut damages = [magicka_sim::damage::Damage::default(); 5];
            damages[0] = damage;
            let timestamp = self.play_time;
            self.with_rail_world(|w| {
                w.collection_blast(Some(killer), None, timestamp, radius, position, damages);
            });
        }
        self.log.push(format!("bloat blast {handle}"));
        if gibs > 0 {
            self.overkill(handle);
        } else {
            let _ = crate::entity_command::EntityCommandHost::kill(self, handle);
        }
    }

    /// Character.OverKill: dead, Overkilled, death status mask, visible,
    /// HitPoints −MaxHitPoints − 1000, camera shake (1.5, 0.5). NPC gibs
    /// follow from DeadState (`spawn_gibs`); players keep the earlier
    /// body-hiding stand-in.
    pub fn overkill(&mut self, handle: u16) {
        if self.dispensers.contains_key(&handle) {
            let _ = EntityCommandHost::kill(self, handle);
            return;
        }
        if self.elemental_eggs.as_ref().is_some_and(|cache| {
            cache
                .slots
                .iter()
                .any(|s| s.handle == handle && s.actor.registered)
        }) {
            let _ = crate::entity_command::EntityCommandHost::kill(self, handle);
            return;
        }
        if self.overkilled.contains(&handle) {
            return;
        }
        if let Some(n) = self.npcs.get_mut(&handle) {
            let c = &mut n.character.combat;
            if n.vlad_character || c.absorb_shield.is_some() {
                c.dead = false;
                c.overkilled = true;
                c.cannot_die_without_explicit_kill = true;
                c.health.hit_points = -1.;
                n.motion.controller.stop();
                return;
            }
            if c.overkilled || c.cannot_die_without_explicit_kill {
                return;
            }
            c.dead = true;
            c.overkilled = true;
            c.death_status_mask = c.statuses.current_mask;
            c.health.invisibility_timer = 0.0;
            c.health.hit_points = -n.definition.max_hit_points - 1000.0;
            let master = n.summon.map(|(master, _)| master);
            if let Some(master) = master {
                // Native OverKill calls Die synchronously, including the
                // master's RemoveBubbleShield hook for every summoned NPC.
                self.remove_bubble_shield(master);
            }
            // DeadState.Update spawns the gibs (`spawn_gibs`).
            self.camera.shake(1.5, 0.5);
            self.log.push(format!("overkill {handle}"));
            return;
        } else if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == handle)
        {
            p.character.combat.health.hit_points = -p.character.definition.max_hit_points - 1000.0;
        } else {
            return;
        }
        self.overkilled.insert(handle);
        self.camera.shake(1.5, 0.5);
        self.log.push(format!("overkill {handle}"));
    }

    /// Character.SpawnGibs for an NPC plus DeadState's misc_gib cue. The body
    /// stops being drawn and mDeadTimer = −100 removes it this frame.
    fn spawn_gibs(&mut self, handle: u16) {
        let Some(npc) = self.npcs.get_mut(&handle) else {
            return;
        };
        let body = &self.physics.bodies[npc.body];
        let source = crate::gibs::GibSource {
            position: body.position,
            capsule_radius: npc.definition.radius,
            capsule_length: npc.definition.length,
            radius: npc.definition.radius,
            mass: npc.definition.mass,
            frozen: npc.character.combat.death_status_mask
                & magicka_sim::status::Status::Frozen as u16
                != 0,
            blood: npc.definition.blood,
        };
        npc.motion.dead_timer = -100.0;
        let visuals = self.gibs.spawn(source, &npc.definition.gibs);
        for blood in std::mem::take(&mut npc.character.blood_decals) {
            self.decals.blood_damage(blood, self.collision.collision());
        }
        self.decals.blood_gibs(source, self.collision.collision());
        self.gib_visuals.extend(visuals);
        self.overkilled.insert(handle);
        self.play_cue(
            crate::audio::banks::MISC,
            crate::gibs::SOUND_GIB,
            Some(source.position),
        );
        self.log.push(format!("gibs {handle}"));
    }

    /// UnderGroundAttack.Initialize; nothing is spawned without ground.
    pub fn spawn_underground(
        &mut self,
        owner: u16,
        position: [f32; 3],
        velocity: [f32; 2],
        range: f32,
        damages: [magicka_sim::damage::Damage; 5],
        timestamp: f64,
    ) -> bool {
        let mut host = UndergroundHost::new(self);
        let Some(attack) = magicka_sim::underground_attack::UndergroundAttack::initialize(
            position, velocity, owner, timestamp, range, damages, &mut host,
        ) else {
            return false;
        };
        let id = self.next_underground;
        self.next_underground += 1;
        self.underground_visuals.push(UndergroundVisual::Start {
            id,
            effects: attack.effect_names(),
            position: attack.position,
            direction: attack.direction,
        });
        self.underground.push((id, attack));
        true
    }

    /// SpellManager updates of UnderGroundAttacks; damage applies after the
    /// pass through the public routes.
    fn update_underground(&mut self, dt: f32) {
        let mut attacks = std::mem::take(&mut self.underground);
        let mut host = UndergroundHost::new(self);
        for (id, attack) in &mut attacks {
            // The hit list starts with the owner (Initialize).
            host.owner = attack.hits.first().copied();
            attack.update(dt, &mut host);
            host.visuals.push(if attack.dead() {
                UndergroundVisual::Stop { id: *id }
            } else {
                UndergroundVisual::Move {
                    id: *id,
                    position: attack.position,
                    direction: attack.direction,
                }
            });
        }
        let UndergroundHost {
            visuals, damage, ..
        } = host;
        self.underground_visuals.extend(visuals);
        attacks.retain(|(_, a)| !a.dead());
        self.underground = attacks;
        for (target, owner, damages, timestamp, position) in damage {
            if let Some(shield) = self.shields.iter_mut().find(|s| s.handle == target) {
                for d in damages.iter().filter(|d| d.attack != 0) {
                    shield.shield.internal_damage(d, true);
                }
            } else {
                self.with_rail_world(|w| {
                    w.deliver_collection(target, owner, damages, timestamp, position);
                });
            }
        }
    }

    /// Gib entity updates and their reimplemented physics.
    fn update_gibs(&mut self, dt: f32) {
        let gravity = magicka_sim::rigid_body::Gravity::magicka().vector;
        let mut visuals = Vec::new();
        self.gibs
            .update(dt, gravity, Some(self.collision.collision()), &mut visuals);
        self.gib_visuals.extend(visuals);
    }

    /// HealFriendlies uses the direct Character.Damage(float, Life) route.
    pub fn heal_friendlies(&mut self) {
        let targets: Vec<_> = self
            .combatants()
            .into_iter()
            .filter(|c| !matches!(c.faction, 8 | 1 | 16 | 2))
            .map(|c| c.handle)
            .collect();
        for handle in targets {
            let character = if let Some(p) = self
                .players
                .iter_mut()
                .flatten()
                .find(|p| p.handle == handle)
            {
                Some((&mut p.character, p.body, true))
            } else {
                self.npcs
                    .get_mut(&handle)
                    .map(|n| (&mut n.character, n.body, false))
            };
            let Some((character, body, avatar)) = character else {
                continue;
            };
            let ctx = magicka_sim::character_health::HealthContext {
                immortal: character.is_immortal(),
                avatar,
                entangled: self.physics.bodies[body].owner.entangled,
                ..Default::default()
            };
            let mut events = Vec::new();
            character.combat.health.damage(
                -500.0,
                magicka_sim::spells::Elements::LIFE,
                character.combat.statuses.current_mask,
                ctx,
                &mut events,
            );
            self.physics.bodies[body].owner.hit_points = character.combat.health.hit_points;
        }
    }

    /// Helper.ArcDamage (0x06002dbe): entities within `range` of `position`
    /// (protected ones ignored), minus `ignore` and ethereal characters,
    /// inside the planar angle, each slot a separate Damage call with its
    /// magnitude scaled by the 3D falloff; then Liquid.Freeze with the
    /// unscaled damages. CircleDamage is the angle-π case.
    #[allow(clippy::too_many_arguments)]
    pub fn arc_damage(
        &mut self,
        owner: Option<u16>,
        ignore: Option<u16>,
        position: [f32; 3],
        direction: [f32; 3],
        range: f32,
        angle: f32,
        damages: &[magicka_sim::damage::Damage],
        timestamp: f64,
    ) -> bool {
        use magicka_sim::entity_query::{EntityQueryGrid, QueryEntity};
        use magicka_sim::spray_weapon::target_scale;
        let (targets, _) = self.spray_entities();
        let query: Vec<_> = targets
            .iter()
            .map(|target| QueryEntity {
                position: target.position,
                radius: target.radius,
                dead: target.dead,
                shield: matches!(target.geometry, crate::spray_world::ArcGeometry::Shield(_)),
                damageable: target.damageable,
            })
            .collect();
        let Ok(grid) = EntityQueryGrid::build(&query) else {
            return false;
        };
        let Ok(indices) = grid.query(&query, position, range, true, false, |index, segment| {
            let hit = match targets[index].geometry {
                crate::spray_world::ArcGeometry::Shield(shield) => shield.segment_fraction(segment),
                _ => None,
            };
            (
                hit.is_some(),
                hit.map_or([0.; 3], |t| {
                    std::array::from_fn(|axis| segment.origin[axis] + segment.delta[axis] * t)
                }),
            )
        }) else {
            return false;
        };
        let ethereal: BTreeSet<_> = self
            .combatants()
            .into_iter()
            .filter(|c| c.ethereal)
            .map(|c| c.handle)
            .collect();
        for index in indices {
            let target = targets[index];
            if Some(target.handle) == ignore
                || !target.damageable
                || ethereal.contains(&target.handle)
            {
                continue;
            }
            let Some(scale) = target_scale(
                position,
                direction,
                range,
                angle,
                target.position,
                target.radius,
            ) else {
                continue;
            };
            for &damage in damages {
                let mut damage = damage;
                damage.magnitude *= scale;
                if let Some(m) = self
                    .missiles
                    .iter_mut()
                    .find(|m| m.handle == Some(target.handle) && !m.body.dead)
                {
                    m.state.receive_damage(damage, false);
                } else if self.death.handle == Some(target.handle) {
                    self.death
                        .deliver(&[damage], self.role != NetworkRole::Client);
                } else if self
                    .elemental_eggs
                    .as_mut()
                    .and_then(|cache| cache.deliver(target.handle, &[damage]))
                    .is_some()
                {
                    // Dedicated egg callback already consumed this entry.
                } else if target.handle & MINE_KEY == GREASE_KEY {
                    self.damage_grease((target.handle & !MINE_KEY) as usize, &[damage]);
                } else if target.handle & MINE_KEY == MINE_KEY {
                    if let Some(mine) = self.mines.get_mut((target.handle & !MINE_KEY) as usize) {
                        let mut slots = [magicka_sim::damage::Damage::default(); 5];
                        slots[0] = damage;
                        mine.internal_damage(&slots);
                    }
                } else if target.handle & MINE_KEY == BARRIER_KEY {
                    let slot = (target.handle & !BARRIER_KEY) as usize;
                    if let Some(barrier) = self.barriers.get_mut(slot) {
                        let mut events = Vec::new();
                        barrier.internal_damage_one(damage, true, &mut events);
                        self.barrier_events(slot, events);
                    }
                } else if let Some(shield) =
                    self.shields.iter_mut().find(|s| s.handle == target.handle)
                {
                    shield.shield.internal_damage(&damage, true);
                } else if self.physics_damage.contains_key(&target.handle) {
                    self.with_rail_world(|world| {
                        crate::missiles::MissileWorld::damage_prop(
                            &mut world.hit,
                            target.handle,
                            damage,
                            owner.unwrap_or(0),
                            position,
                        );
                    });
                } else {
                    self.with_rail_world(|world| {
                        world.hit.play_time = timestamp;
                        crate::missiles::MissileWorld::damage_character(
                            &mut world.hit,
                            target.handle,
                            damage,
                            owner.unwrap_or(0),
                            position,
                        );
                    });
                }
            }
        }
        self.freeze_water(
            position,
            direction.map(|v| v * range),
            angle.min(std::f32::consts::PI),
            1.0,
            damages,
        );
        true
    }

    /// SpraySpell.CastWeapon's one-shot arc over characters and spell entities.
    pub fn spray_weapon(
        &mut self,
        owner: u16,
        position: [f32; 3],
        direction: [f32; 3],
        cast_source: [f32; 3],
        timestamp: f64,
        plan: &magicka_sim::spray_weapon::SprayWeaponPlan,
    ) -> bool {
        use magicka_sim::spray_weapon::SprayWeaponPlan;
        if !self.arc_damage(
            Some(owner),
            Some(owner),
            position,
            direction,
            SprayWeaponPlan::RANGE,
            SprayWeaponPlan::ANGLE,
            &plan.damages,
            timestamp,
        ) {
            return false;
        }
        if let Some(damage) = plan.greased_owner_damage {
            self.with_rail_world(|world| {
                world.hit.play_time = timestamp;
                crate::missiles::MissileWorld::damage_character(
                    &mut world.hit,
                    owner,
                    damage,
                    owner,
                    cast_source,
                );
            });
        }
        true
    }

    /// RailGunSpell.CastWeapon's immediate damage: every damageable entity
    /// within `range` of the owner whose ArcIntersect(position, direction,
    /// range, 9π/20, 2) hits takes the Beam/Weapon damage collection at the
    /// hit point. The caster's own arc test degenerates (NaN) and rejects.
    /// Mines and barriers are not damaged here yet. Returns the hit handles.
    /// IceBlade.OnRemove: GetEntities(midpoint, range/2) of the segment
    /// Position → Position + Direction·range; every IDamageable except the
    /// owner whose SegmentIntersect(segment, 1) hits takes the blade damage
    /// at the hit point. Returns the damaged handles.
    pub fn ice_blade_strike(
        &mut self,
        owner: u16,
        position: [f32; 3],
        direction: [f32; 3],
        range: f32,
        damages: [magicka_sim::damage::Damage; 5],
        timestamp: f64,
    ) -> Vec<u16> {
        use magicka_sim::railgun::{RailEntityKind as K, RailWorld};
        let seg = magicka_sim::spray_pulse::Segment {
            origin: position,
            delta: direction.map(|v| v * range),
        };
        let center = std::array::from_fn(|i| position[i] + seg.delta[i] * 0.5);
        let hits = self
            .with_rail_world(|w| {
                let mut out = Vec::new();
                for e in w.rail_entities(center, range * 0.5) {
                    if e.handle == owner || matches!(e.kind, K::Ignored | K::Other | K::Portal) {
                        continue;
                    }
                    if let Some(point) = w.rail_segment(e.handle, seg, 1.0) {
                        let shield = e.kind == K::Shield;
                        if !shield {
                            w.deliver_collection(e.handle, Some(owner), damages, timestamp, point);
                        }
                        out.push((e.handle, shield));
                    }
                }
                out
            })
            .unwrap_or_default();
        for &(handle, shield) in &hits {
            if !shield {
                continue;
            }
            if let Some(s) = self.shields.iter_mut().find(|s| s.handle == handle) {
                for damage in damages.iter().filter(|d| d.attack != 0) {
                    s.shield.internal_damage(damage, true);
                }
            }
        }
        hits.into_iter().map(|(h, _)| h).collect()
    }

    pub fn rail_weapon(
        &mut self,
        owner: u16,
        position: [f32; 3],
        direction: [f32; 3],
        range: f32,
        damages: [magicka_sim::damage::Damage; 5],
        timestamp: f64,
    ) -> Vec<u16> {
        use crate::spray_world::ArcGeometry as G;
        use magicka_sim::arc_intersection::{character_arc, entity_arc, ArcQuery};
        let query = ArcQuery {
            origin: position,
            direction,
            range,
            angle: std::f32::consts::PI * 9.0 / 20.0,
            height_difference: 2.0,
        };
        let (targets, _) = self.spray_entities();
        let mut hits = Vec::new();
        // Railgun skips grease fields.
        for t in targets
            .into_iter()
            .filter(|t| t.handle & MINE_KEY != GREASE_KEY)
        {
            let d = (0..3)
                .map(|i| (t.position[i] - position[i]).powi(2))
                .sum::<f32>()
                .sqrt();
            if !t.damageable || t.dead || d - t.radius > range {
                continue;
            }
            let point = match t.geometry {
                G::Character { ethereal } => character_arc(t.position, t.radius, ethereal, query),
                G::Shield(s) => s.arc_intersect(query),
                G::BossZone { ethereal } => {
                    if ethereal {
                        None
                    } else {
                        crate::jormungandr::Jormungandr::arc(t.position, t.radius, query)
                    }
                }
                G::Boss | G::Entity | G::MagickDeath | G::ElementalEgg => {
                    entity_arc(t.position, t.radius, query)
                }
                G::Dispenser => entity_arc(
                    t.position,
                    t.radius,
                    ArcQuery {
                        height_difference: f32::INFINITY,
                        ..query
                    },
                ),
                G::Barrier { .. } | G::Mine | G::Missile => None,
            };
            let Some(point) = point else { continue };
            if let Some(shield) = self.shields.iter_mut().find(|s| s.handle == t.handle) {
                for damage in damages.iter().filter(|d| d.attack != 0) {
                    shield.shield.internal_damage(damage, true);
                }
            } else {
                self.with_rail_world(|w| {
                    w.deliver_collection(t.handle, Some(owner), damages, timestamp, point);
                });
            }
            hits.push(t.handle);
        }
        hits
    }

    /// SpraySpell.CastArea immediate operations. Lifecycle/cache ownership
    /// belongs to the casting effect; Character.Electrocute is native-empty.
    pub fn spray_area(
        &mut self,
        owner: u16,
        position: [f32; 3],
        cast_source: [f32; 3],
        timestamp: f64,
        plan: &magicka_sim::spray_area::SprayAreaPlan,
    ) -> bool {
        let ran = self
            .with_rail_world(|world| {
                world.collection_blast(
                    Some(owner),
                    Some(owner),
                    timestamp,
                    plan.radius,
                    position,
                    plan.damages,
                );
            })
            .is_some();
        if !ran {
            return false;
        }
        self.freeze_water(
            position,
            [plan.radius, 0.0, 0.0],
            std::f32::consts::TAU,
            1.0,
            &plan.damages,
        );
        self.with_rail_world(|world| {
            // Owner and all current targets are Characters: Electrocute is empty.
            if let Some(damage) = plan.greased_owner_damage {
                if let Some(player) = world
                    .hit
                    .players
                    .iter_mut()
                    .flatten()
                    .find(|p| p.handle == owner)
                {
                    player.receive_payload(
                        &mut world.hit.bodies[player.body],
                        magicka_sim::damage_delivery::DamagePayload::Single(damage),
                        Some(owner),
                        cast_source,
                        timestamp,
                    );
                }
            }
        })
        .is_some()
    }

    /// SpellManager.Update for the Railgun pool.
    fn update_rails(&mut self, dt: f32) {
        for p in self.players.iter_mut().flatten() {
            for id in p.rail_kills.drain(..) {
                self.rails.kill(id);
            }
        }
        if self.rails.is_empty() {
            return;
        }
        let mut rails = std::mem::take(&mut self.rails);
        let ran = self.with_rail_world(|world| rails.update(dt, world));
        if ran.is_none() {
            self.log.push("rail world: entity grid unavailable".into());
        }
        self.rails = rails;
    }

    /// Live characters as the combat code sees them (avatars and NPCs, in
    /// entity order).
    pub fn combatants(&self) -> Vec<crate::npc_combat::Combatant> {
        use crate::npc_combat::Combatant;
        let identity = [magicka_sim::resistance::Resistance::IDENTITY; 11];
        let mut out = Vec::new();
        for e in self.entities.iter().filter(|e| e.added && !e.removed) {
            let entry = match e.kind {
                EntityKind::Npc { .. } => self.npcs.get(&e.handle).map(|n| {
                    (
                        n.body,
                        &n.character,
                        n.motion.state,
                        n.motion.controller.current_animation,
                        false,
                        magicka_sim::spells::combine(&n.combat.spellcasting.queue).mag,
                        identity,
                    )
                }),
                EntityKind::Avatar { slot } => self.players[slot].as_ref().map(|p| {
                    let combined = magicka_sim::spells::combine(&p.conjure.queue);
                    (
                        p.body,
                        &p.character,
                        p.motion.state,
                        p.motion.controller.current_animation,
                        true,
                        combined.mag,
                        p.equipment_resistances(),
                    )
                }),
                EntityKind::Physics { .. }
                | EntityKind::Item
                | EntityKind::ElementalEgg
                | EntityKind::Dispenser => None,
            };
            let Some((body, character, state, current_animation, avatar, queue, equipment)) = entry
            else {
                continue;
            };
            let b = &self.physics.bodies[body];
            use crate::character_motion::CharacterState as S;
            out.push(Combatant {
                special_ready: std::array::from_fn(|slot| match e.kind {
                    EntityKind::Npc { .. } => self
                        .npcs
                        .get(&e.handle)
                        .and_then(|n| n.weapons.iter().find(|(s, _)| *s == slot as i32))
                        .and_then(|(_, w)| w.special.as_ref())
                        .is_some_and(|s| s.ready()),
                    EntityKind::Avatar { slot: player } => self.players[player]
                        .as_ref()
                        .and_then(|p| p.items.iter().find(|(s, _)| *s == slot as i32))
                        .is_some_and(|(_, item)| {
                            item.definition.special_ability.is_some() && item.special_cooldown <= 0.
                        }),
                    _ => false,
                }),
                handle: e.handle,
                unique_id: e.unique_id,
                position: b.position,
                velocity: b.velocity,
                direction: b.direction(),
                movement: b.movement(),
                radius: b.radius,
                length: b.length,
                faction: self.faction(e),
                hit_points: character.combat.health.hit_points,
                max_hit_points: character.combat.health.max_hit_points,
                dead: e.dead || character.combat.dead,
                avatar,
                status_mask: character.combat.statuses.current_mask,
                ethereal: character.ethereal,
                invisible: character.is_invisible(),
                on_ground: b.is_touching_ground(),
                attack_state: state == S::Attack,
                casting: state == S::Cast,
                gripping: character.grip.gripping(),
                gripped: character.grip.gripper.is_some(),
                grip_ratio: character.grip.damage_accumulation / character.definition.hit_tolerance,
                current_animation,
                queue,
                resistances: character.definition.resistances,
                equipment,
                character: true,
                agro_multiplier: character.buffs.general.agro_multiplier(),
            });
        }
        out
    }

    /// A live character's motion, for an NPC about to jump at it. `updating` is that
    /// NPC, already taken out of `npcs`.
    fn jump_target(
        &self,
        target: u16,
        updating: u16,
        npc: &crate::npc::NpcCharacter,
    ) -> Option<magicka_sim::jump::JumpTarget> {
        let e = self
            .entities
            .iter()
            .find(|e| e.handle == target && e.added && !e.removed)?;
        let body = match e.kind {
            EntityKind::Npc { .. } if target == updating => npc.body,
            EntityKind::Npc { .. } => self.npcs.get(&target)?.body,
            EntityKind::Avatar { slot } => self.players[slot].as_ref()?.body,
            _ => return None,
        };
        let b = &self.physics.bodies[body];
        Some(magicka_sim::jump::JumpTarget {
            position: b.position,
            velocity: b.velocity,
        })
    }

    /// Agent.ChooseTarget's view: the characters plus every live damageable
    /// prop that some agent holds as its priority target (SetOrder's
    /// priorityTarget, e.g. hl_s5BF's beastman_torchers and house0..4).
    fn agent_combatants(&self) -> Vec<crate::npc_combat::Combatant> {
        let mut out = self.combatants();
        let identity = [magicka_sim::resistance::Resistance::IDENTITY; 11];
        let targets: BTreeSet<u16> = self
            .npcs
            .values()
            .filter_map(|n| n.combat.priority_target)
            .collect();
        for handle in targets {
            if out.iter().any(|c| c.handle == handle) {
                continue;
            }
            let Some(state) = self.physics_damage.get(&handle) else {
                continue;
            };
            let Some(e) = self
                .entities
                .iter()
                .find(|e| e.handle == handle && e.added && !e.removed)
            else {
                continue;
            };
            let o = &e.orientation;
            out.push(crate::npc_combat::Combatant {
                special_ready: [false; 7],
                handle,
                unique_id: e.unique_id,
                position: e.position,
                velocity: [0.0; 3],
                direction: [-o[8], -o[9], -o[10]],
                movement: [0.0; 3],
                radius: state.radius(),
                length: 0.0,
                faction: 0,
                hit_points: state.health,
                max_hit_points: state.maximum,
                dead: e.dead || state.health <= 0.0,
                avatar: false,
                status_mask: state.status_mask,
                ethereal: false,
                invisible: false,
                on_ground: true,
                attack_state: false,
                casting: false,
                gripping: false,
                gripped: false,
                grip_ratio: 0.0,
                current_animation: 0,
                queue: [0.0; 11],
                resistances: identity,
                equipment: identity,
                character: false,
                agro_multiplier: 1.0,
            });
        }
        out
    }

    /// Spray targets: live damageable props and characters with their arc geometry, and the
    /// facing of each (for damage delivery).
    fn spray_entities(
        &self,
    ) -> (
        Vec<crate::spray_world::WorldEntity>,
        BTreeMap<u16, [f32; 3]>,
    ) {
        let mut out = Vec::new();
        let mut facing = BTreeMap::new();
        for e in self.entities.iter().filter(|e| e.added && !e.removed) {
            if let Some(d) = self.dispensers.get(&e.handle) {
                out.push(crate::spray_world::WorldEntity {
                    handle: e.handle,
                    position: d.position(),
                    radius: 1.25,
                    dead: d.health <= 0.0,
                    damageable: true,
                    frozen: false,
                    geometry: crate::spray_world::ArcGeometry::Dispenser,
                });
                continue;
            }
            if let Some(state) = self.physics_damage.get(&e.handle) {
                out.push(crate::spray_world::WorldEntity {
                    handle: e.handle,
                    position: e.position,
                    radius: e.radius,
                    dead: e.dead || state.health <= 0.0,
                    damageable: true,
                    frozen: state.status_mask & magicka_sim::status::Status::Frozen as u16 != 0,
                    geometry: if state.has_capsule() {
                        crate::spray_world::ArcGeometry::BossZone {
                            ethereal: state.capsule_ethereal(),
                        }
                    } else {
                        crate::spray_world::ArcGeometry::Entity
                    },
                });
                continue;
            }
            let body = match e.kind {
                EntityKind::Npc { .. } => self.npcs.get(&e.handle).map(|n| n.body),
                EntityKind::Avatar { slot } => self.players[slot].as_ref().map(|p| p.body),
                EntityKind::Physics { .. }
                | EntityKind::Item
                | EntityKind::ElementalEgg
                | EntityKind::Dispenser => None,
            };
            let Some(body) = body else { continue };
            let frozen = match e.kind {
                EntityKind::Npc { .. } => self.npcs.get(&e.handle).map(|n| &n.character),
                EntityKind::Avatar { slot } => self.players[slot].as_ref().map(|p| &p.character),
                EntityKind::Physics { .. }
                | EntityKind::Item
                | EntityKind::ElementalEgg
                | EntityKind::Dispenser => None,
            }
            .is_some_and(|c| c.combat.statuses.has(magicka_sim::status::Status::Frozen));
            let b = &self.physics.bodies[body];
            facing.insert(e.handle, b.direction());
            out.push(crate::spray_world::WorldEntity {
                handle: e.handle,
                position: b.position,
                radius: b.radius,
                dead: e.dead,
                damageable: true,
                frozen,
                geometry: crate::spray_world::ArcGeometry::Character { ethereal: false },
            });
        }
        for s in &self.shields {
            out.push(crate::spray_world::WorldEntity {
                handle: s.handle,
                position: s.shield.position,
                radius: s.shield.radius,
                dead: s.shield.dead,
                damageable: true,
                frozen: false,
                geometry: crate::spray_world::ArcGeometry::Shield(s.shield.geometry()),
            });
        }
        // Barriers and mines as IDamageable entities (handles in the
        // reserved BARRIER_KEY/MINE_KEY spaces).
        for (slot, b) in self.barriers.iter() {
            out.push(crate::spray_world::WorldEntity {
                handle: BARRIER_KEY | slot as u16,
                position: b.position,
                radius: b.radius,
                dead: b.is_dead(),
                damageable: true,
                frozen: false,
                geometry: crate::spray_world::ArcGeometry::Barrier { solid: b.solid() },
            });
        }
        for (slot, f) in self.grease.iter() {
            out.push(crate::spray_world::WorldEntity {
                handle: GREASE_KEY | slot as u16,
                position: f.position,
                radius: magicka_sim::grease::FIELD_RADIUS,
                dead: false,
                damageable: true,
                frozen: false,
                geometry: crate::spray_world::ArcGeometry::Mine,
            });
        }
        for (slot, m) in self.mines.iter() {
            out.push(crate::spray_world::WorldEntity {
                handle: MINE_KEY | slot as u16,
                position: m.position,
                radius: magicka_sim::spell_mine::RADIUS,
                dead: m.dead,
                damageable: true,
                frozen: false,
                geometry: crate::spray_world::ArcGeometry::Mine,
            });
        }
        for (&handle, boss) in &self.jormungandr {
            if let Some(zone) = boss.spray_entity(handle) {
                out.push(zone);
                facing.insert(handle, boss.direction);
            }
        }
        if let Some(death) = self.death.spray_entity() {
            out.push(death);
        }
        if let Some(cache) = &self.elemental_eggs {
            for slot in cache.slots.iter().filter(|slot| slot.actor.registered) {
                out.push(crate::spray_world::WorldEntity {
                    handle: slot.handle,
                    position: slot.actor.position,
                    radius: slot.actor.geometry.radius,
                    dead: slot.actor.state.dead(),
                    damageable: true,
                    frozen: false,
                    geometry: crate::spray_world::ArcGeometry::ElementalEgg,
                });
            }
        }
        for m in self.missiles.iter().filter(|m| !m.body.dead) {
            if let Some(handle) = m.handle {
                out.push(crate::spray_world::WorldEntity {
                    handle,
                    position: m.body.position,
                    radius: m.body.radius,
                    dead: false,
                    damageable: true,
                    frozen: false,
                    geometry: crate::spray_world::ArcGeometry::Missile,
                });
            }
        }
        (out, facing)
    }

    pub fn add_player(
        &mut self,
        slot: usize,
        body: magicka_sim::character_physics::CharacterBody,
        make: impl FnOnce(u16, usize) -> PlayerAvatar,
    ) {
        let handle = self.new_handle();
        let index = self.physics.bodies.len();
        let position = body.position;
        let radius = body.radius;
        self.physics.bodies.push(body);
        let avatar = make(handle, index);
        self.entities.push(SceneEntity {
            handle,
            kind: EntityKind::Avatar { slot },
            // Player.UNIQUE_ID: "player1".."player4".
            unique_id: if avatar.playing {
                custom_hash(&format!("player{}", slot + 1))
            } else {
                0
            },
            position,
            orientation: magicka_content::skinned::IDENTITY,
            hit_points: avatar.character.combat.health.hit_points,
            max_hit_points: avatar.character.combat.health.max_hit_points,
            original_faction: avatar.faction,
            faction: avatar.faction,
            radius,
            length: 0.0,
            template_name: None,
            animation: 1,
            special_idle: 0,
            undying: false,
            cannot_die_without_explicit_kill: false,
            dead: false,
            added: avatar.playing,
            removed: !avatar.playing,
            order: None,
            event_count: 0,
        });
        self.reserve_player_slots(slot + 1);
        self.players[slot] = Some(avatar);
    }

    /// Grow the player table (rewrite extension beyond the original four).
    pub fn reserve_player_slots(&mut self, count: usize) {
        if self.players.len() < count {
            self.players.resize_with(count, || None);
        }
    }

    /// Entities phase for player avatars: AI events, state and animation.
    pub fn update_players(&mut self, dt: f32) {
        let cutscene = self.flags.cutscene;
        self.refresh_fear_sources();
        for player in self.players.iter_mut().flatten() {
            player.sync_immortality(cutscene);
        }
        let play_time = self.play_time;
        {
            let locators = &self.locators;
            let entities = &self.entities;
            let locator = |id: i32| locators.get(&id).copied();
            let entity_position = |id: i32| {
                entities
                    .iter()
                    .rev()
                    .find(|e| e.added && e.unique_id == id && id != 0)
                    .filter(|e| !e.removed)
                    .map(|e| e.position)
            };
            for p in self.players.iter_mut().flatten().filter(|p| p.playing) {
                let body = &self.physics.bodies[p.body];
                p.boost.target = magicka_sim::shield_boost::shield_to_boost(
                    body.position,
                    body.direction(),
                    self.shields.iter().map(|s| &s.shield),
                )
                .map(|index| self.shields[index].handle);
                let frame = AvatarFrame {
                    body: &mut self.physics.bodies[p.body],
                    nav: self.nav.as_ref(),
                    move_abilities: p.move_abilities,
                    triggers: &mut self.pending_triggers,
                    sender: p.handle,
                    entity_position: &entity_position,
                    locator: &locator,
                    avoidance: &|_| [0.0; 3],
                    closest_area_point: &|_, _| None,
                    in_cutscene: cutscene,
                    play_time,
                };
                p.update(frame, dt);
                // Character.Update: the fear timer after the state update.
                let dead = p.character.combat.dead;
                p.character.fear.tick(dt, dead);
                p.character.tick_charm(dt);
            }
        }
        self.update_levitation(dt, false);
        // CurrentSpell.CastUpdate against the live world, one caster at a time.
        let mut launches = Vec::new();
        let mut area_blasts = Vec::new();
        let mut ground_blasts = Vec::new();
        let mut spray_blasts = Vec::new();
        let mut spray_freezes = Vec::new();
        let mut rail_weapons = Vec::new();
        let mut undergrounds = Vec::new();
        let mut spray_weapons = Vec::new();
        let mut friendly_heals = 0;
        let mut item_specials = Vec::new();
        let mut ready_effects = Vec::new();
        let mut magick_casts = Vec::new();
        let mut registered_magick_casts = Vec::new();
        let mut mine_spawns = Vec::new();
        let mut barrier_spawns = Vec::new();
        let mut shield_weapons = Vec::new();
        let mut ice_blades = Vec::new();
        let mut pending_barrier_events = Vec::new();
        let mut shield_spawns = Vec::new();
        for slot in 0..self.players.len() {
            // Include the caster in the view used for NPC attacker accounting.
            let combatants = if self.players[slot]
                .as_ref()
                .is_some_and(|p| p.current_spell.is_some())
            {
                self.combatants()
            } else {
                Vec::new()
            };
            let Some(mut p) = self.players[slot].take() else {
                continue;
            };
            if p.current_spell.is_some() {
                let (mut targets, _) = self.spray_entities();
                let shield_order: Vec<u16> = self.shields.iter().map(|s| s.handle).collect();
                let (bodies_before, bodies_after) = self.physics.bodies.split_at_mut(p.body);
                let (caster_body, bodies_after) = bodies_after.split_first_mut().unwrap();
                let mut effects = SceneSprayEffects {
                    missiles: &mut self.missiles,
                    physics_damage: &mut self.physics_damage,
                    dispensers: &mut self.dispensers,
                    entities: &mut self.entities,
                    elemental_eggs: &mut self.elemental_eggs,
                    caster_boosts: p.character.buffs.damage_boosts(),
                    bodies_before,
                    bodies_after,
                    caster_index: p.body,
                    combatants,
                    npcs: &mut self.npcs,
                    players: &mut self.players,
                    triggers: &mut self.pending_triggers,
                    shields: &mut self.shields,
                    mines: &mut self.mines,
                    grease: &mut self.grease,
                    barriers: &mut self.barriers,
                    death: &mut self.death,
                    jormungandr: &mut self.jormungandr,
                    death_damage_features: self.role != NetworkRole::Client,
                    barrier_events: Vec::new(),
                };
                match crate::spray_world::RuntimeSprayWorld::new(
                    Some(self.collision.collision()),
                    &mut targets,
                    &shield_order,
                    &mut effects,
                ) {
                    Ok(world) => {
                        let mut world = world.with_rails(&mut self.rails);
                        p.cast_update(caster_body, dt, &mut world)
                    }
                    Err(e) => self.log.push(format!("spray world: {e:?}")),
                }
                pending_barrier_events.append(&mut effects.barrier_events);
            } else {
                p.cast_update(
                    &mut self.physics.bodies[p.body],
                    dt,
                    &mut crate::spray_world::NoSprayWorld,
                );
            }
            let mut events = Vec::new();
            let status = p.character.update_damage_and_status_on(
                &mut self.physics.bodies[p.body],
                dt,
                true,
                magicka_sim::character_health::StatusUpdateContext {
                    in_cutscene: cutscene,
                    ..Default::default()
                },
                &mut events,
            );
            p.finish(&mut self.physics.bodies[p.body], cutscene, dt);
            let bloating = self.bloats.contains_key(&p.handle);
            p.motion
                .status_tail(&mut self.physics.bodies[p.body], status, bloating);
            for e in &p.events {
                match e {
                    crate::avatar::AvatarEvent::BoostShield { target, amount } => {
                        if let Some(shield) = self.shields.iter_mut().find(|s| s.handle == *target)
                        {
                            shield.shield.hit_points += amount;
                        }
                    }
                    crate::avatar::AvatarEvent::HealFriendlies => friendly_heals += 1,
                    crate::avatar::AvatarEvent::CastMagick(magick) => {
                        magick_casts.push((slot, *magick));
                    }
                    crate::avatar::AvatarEvent::CastRegisteredMagick(index) => {
                        registered_magick_casts.push((slot, *index));
                    }
                    crate::avatar::AvatarEvent::ItemSpecial { name, .. }
                        if name.eq_ignore_ascii_case("invisibility") =>
                    {
                        self.play_cue(
                            16,
                            "magick_invisibility",
                            Some(self.physics.bodies[p.body].position),
                        );
                    }
                    crate::avatar::AvatarEvent::SpecialReady { slot } => {
                        let frame = p
                            .items
                            .iter()
                            .find(|(s, _)| s == slot)
                            .and_then(|(_, item)| item.frame.as_ref());
                        if let Some(frame) = frame {
                            ready_effects.push(frame.attach0);
                        }
                    }
                    crate::avatar::AvatarEvent::ItemSpecial { slot, name }
                        if ![
                            "HealFriendlies",
                            "WardedExpulsion",
                            "IceCacoon",
                            "RapidStrike",
                            "RandomMine",
                        ]
                        .iter()
                        .any(|n| name.eq_ignore_ascii_case(n)) =>
                    {
                        if let Some((_, item)) = p.items.iter().find(|(s, _)| s == slot) {
                            if let Some((_, ability)) = &item.definition.special_ability {
                                item_specials.push((
                                    p.handle,
                                    crate::item_special::SpecialInvocation {
                                        ability: ability.clone(),
                                        buff_id: Some(item.special_buff_id),
                                    },
                                ));
                            }
                        }
                    }
                    crate::avatar::AvatarEvent::SprayWeapon {
                        position,
                        direction,
                        cast_source,
                        timestamp,
                        plan,
                    } => {
                        spray_weapons.push((
                            p.handle,
                            *position,
                            *direction,
                            *cast_source,
                            *timestamp,
                            plan.clone(),
                        ));
                    }
                    crate::avatar::AvatarEvent::MissileLaunched(launch) => {
                        launches.push((p.handle, *launch));
                    }
                    crate::avatar::AvatarEvent::BeamArea {
                        position,
                        radius,
                        damages,
                        timestamp,
                        ..
                    } => area_blasts.push((p.handle, *position, *radius, *damages, *timestamp)),
                    crate::avatar::AvatarEvent::UndergroundAttack {
                        position,
                        velocity,
                        range,
                        damages,
                        timestamp,
                    } => undergrounds
                        .push((p.handle, *position, *velocity, *range, *damages, *timestamp)),
                    crate::avatar::AvatarEvent::RailWeapon {
                        position,
                        direction,
                        range,
                        damages,
                        timestamp,
                        ..
                    } => rail_weapons.push((
                        p.handle, *position, *direction, *range, *damages, *timestamp,
                    )),
                    crate::avatar::AvatarEvent::SprayFreeze {
                        origin,
                        delta,
                        spread,
                        multiplier,
                        damages,
                    } => spray_freezes.push((*origin, *delta, *spread, *multiplier, *damages)),
                    crate::avatar::AvatarEvent::SprayArea {
                        spell,
                        position,
                        cast_source,
                        timestamp,
                        plan,
                    } => {
                        spray_blasts.push((
                            p.handle,
                            *spell,
                            *position,
                            *cast_source,
                            *timestamp,
                            plan.clone(),
                        ));
                    }
                    crate::avatar::AvatarEvent::GroundArea {
                        position,
                        radius,
                        damages,
                        timestamp,
                    } => {
                        ground_blasts.push((p.handle, *position, *radius, *damages, *timestamp));
                    }
                    crate::avatar::AvatarEvent::SpawnMines { placements, spell } => {
                        mine_spawns.push((p.handle, *placements, *spell))
                    }
                    crate::avatar::AvatarEvent::SpawnBarriers { cast, spell } => {
                        barrier_spawns.push((p.handle, (**cast).clone(), *spell))
                    }
                    crate::avatar::AvatarEvent::IceBladeStrike {
                        range,
                        damages,
                        timestamp,
                        ..
                    } => {
                        let body = &self.physics.bodies[p.body];
                        ice_blades.push((
                            p.handle,
                            body.position,
                            body.direction(),
                            *range,
                            *damages,
                            *timestamp,
                        ));
                    }
                    crate::avatar::AvatarEvent::ShieldWeapon { spell } => {
                        let body = &self.physics.bodies[p.body];
                        match magicka_sim::shield_weapon::animation_end(
                            spell,
                            body.position,
                            body.direction(),
                            body.radius,
                        ) {
                            Ok(w) => shield_weapons.push((p.handle, w, *spell)),
                            Err(e) => self.log.push(format!("shield weapon damage: {e:?}")),
                        }
                    }
                    crate::avatar::AvatarEvent::SpawnShield {
                        position,
                        radius,
                        direction,
                        kind,
                        hit_points,
                        color,
                    } => shield_spawns.push((
                        p.handle,
                        *position,
                        *radius,
                        *direction,
                        *kind,
                        *hit_points,
                        *color,
                    )),
                    _ => {}
                }
            }
            self.players[slot] = Some(p);
        }
        self.spawn_missiles(launches);
        for (owner, position, radius, direction, kind, hp, color) in shield_spawns {
            self.spawn_shield(owner, position, radius, direction, kind, hp, color);
        }
        for (owner, placements, spell) in mine_spawns {
            self.spawn_mines(owner, &placements, spell);
        }
        for (slot, events) in pending_barrier_events {
            self.barrier_events(slot, events);
        }
        for _ in 0..friendly_heals {
            self.heal_friendlies();
        }
        for (owner, cast) in item_specials {
            self.execute_special(owner, cast);
        }
        for transform in ready_effects {
            self.start_effect_transform(custom_hash("special_ready"), transform);
        }
        for (slot, magick) in magick_casts {
            self.cast_magick(slot, magick);
        }
        for (slot, index) in registered_magick_casts {
            if let Some(p) = self.players.get(slot).and_then(Option::as_ref) {
                self.registered_casts.push((p.handle, index));
            }
        }
        for (owner, position, direction, cast_source, timestamp, plan) in spray_weapons {
            if !self.spray_weapon(owner, position, direction, cast_source, timestamp, &plan) {
                self.log
                    .push("weapon spray: entity grid unavailable".into());
            }
        }
        for (owner, cast, spell) in barrier_spawns {
            self.spawn_barriers(owner, &cast, spell);
        }
        for (owner, weapon, spell) in shield_weapons {
            self.spawn_shield_weapon(owner, weapon, spell);
        }
        for (owner, position, direction, range, damages, timestamp) in ice_blades {
            self.ice_blade_strike(owner, position, direction, range, damages, timestamp);
        }
        for (owner, position, velocity, range, damages, timestamp) in undergrounds {
            self.spawn_underground(owner, position, velocity, range, damages, timestamp);
        }
        for (owner, position, direction, range, damages, timestamp) in rail_weapons {
            self.rail_weapon(owner, position, direction, range, damages, timestamp);
        }
        for (origin, delta, spread, multiplier, damages) in spray_freezes {
            self.freeze_water(origin, delta, spread, multiplier, &damages);
        }
        for (owner, spell, position, cast_source, timestamp, plan) in spray_blasts {
            self.start_spray_area_audio(owner, position, &spell);
            if !self.spray_area(owner, position, cast_source, timestamp, &plan) {
                self.log.push("spray area: entity grid unavailable".into());
            }
        }
        for (owner, position, radius, damages, timestamp) in area_blasts {
            self.with_rail_world(|world| {
                world.collection_blast(
                    Some(owner),
                    Some(owner),
                    timestamp,
                    radius,
                    position,
                    damages,
                );
            });
            self.blast_grease(position, radius, &damages);
        }
        for (owner, position, radius, damages, timestamp) in ground_blasts {
            self.apply_ground_blast(owner, position, radius, damages, timestamp);
        }
        self.update_auras_and_buffs(dt, cutscene);
        self.sync_spray_area_audio();
        self.execute_melee();
        self.execute_item_missiles();
        self.update_player_guns(dt);
        // Mirror live avatar facts into their entity records.
        for p in self.players.iter().flatten() {
            let body = &self.physics.bodies[p.body];
            if let Some(e) = self.entities.iter_mut().find(|e| e.handle == p.handle) {
                e.position = body.position;
                e.hit_points = p.character.combat.health.hit_points;
                e.dead = p.dead();
            }
        }
    }

    fn apply_ground_blast(
        &mut self,
        owner: u16,
        position: [f32; 3],
        radius: f32,
        damages: [magicka_sim::damage::Damage; 5],
        timestamp: f64,
    ) {
        self.ground_blasts.push((position, radius, damages));
        self.blast_grease(position, radius, &damages);
        self.camera.shake(2.0, 0.9);
        self.with_rail_world(|world| {
            let targets: Vec<_> = world
                .hit
                .view
                .entities(position, radius, true)
                .into_iter()
                .map(|i| world.hit.view.combatants[i])
                .collect();
            for target in targets {
                if target.handle == owner || target.ethereal || target.dead {
                    continue;
                }
                let body = world
                    .hit
                    .npcs
                    .get(&target.handle)
                    .map(|n| n.body)
                    .or_else(|| {
                        world
                            .hit
                            .players
                            .iter()
                            .flatten()
                            .find(|p| p.handle == target.handle)
                            .map(|p| p.body)
                    });
                if body.is_some_and(|i| world.hit.bodies[i].owner.levitating) {
                    continue;
                }
                let delta = crate::npc_combat::blast_target_delta(&target, position);
                let distance = delta.iter().map(|v| v * v).sum::<f32>().sqrt() - target.radius;
                if distance <= radius && !world.hit.view.scene_hit(position, delta) {
                    world.deliver_collection(
                        target.handle,
                        Some(owner),
                        damages,
                        timestamp,
                        position,
                    );
                }
            }
        });
        // GroundBlast → BlastDamage(iGroundBlast) tail: Liquid.Freeze.
        self.freeze_blast(position, radius, &damages);
    }

    /// Character.Drown for the avatar or NPC owning body `index`: Drowning,
    /// HitPoints 0, DeadTimer 10, MoveTo(position + WaterDepth·Up), body
    /// disabled unless CannotDieWithoutExplicitKill, then the water (or lava)
    /// drown effect at the new position along the character direction.
    /// Grip release, stun/fear effect stops, hypnosis and charm are not
    /// represented in the live character yet.
    /// Character.Update's aura and buff sections for every character:
    /// ActiveAura.Execute (AuraBuff to non-ethereal characters within the
    /// radius, ignoring height; decal ring), then each character's buffs.
    fn update_auras_and_buffs(&mut self, dt: f32, cutscene: bool) {
        self.update_generic_auras(dt);
        // (handle, position, radius, height offset, ethereal, dead)
        let mut people: Vec<(u16, [f32; 3], f32, f32, bool, bool)> = Vec::new();
        for p in self.players.iter().flatten() {
            let d = &p.character.definition;
            people.push((
                p.handle,
                self.physics.bodies[p.body].position,
                d.radius,
                -d.radius - 0.5 * d.length,
                p.character.ethereal,
                p.dead(),
            ));
        }
        for (handle, n) in &self.npcs {
            let d = &n.character.definition;
            people.push((
                *handle,
                self.physics.bodies[n.body].position,
                d.radius,
                -d.radius - 0.5 * d.length,
                n.character.ethereal,
                n.character.combat.dead,
            ));
        }
        let mut grants = Vec::new();
        // PlayerAvatar ages its auras in its character update. NPCs need the
        // equivalent AuraStorage.Execute lifetime step here.
        let npc_owners: BTreeSet<_> = self.npcs.keys().copied().collect();
        {
            let collision = self.collision.collision();
            let decals = &mut self.decals;
            let ground = |origin: [f32; 3], delta: [f32; 3]| {
                collision
                    .segment_intersect(magicka_sim::spray_pulse::Segment { origin, delta }, false)
                    .map(|hit| {
                        let f = hit.hit.fraction;
                        [
                            origin[0] + delta[0] * f,
                            origin[1] + delta[1] * f,
                            origin[2] + delta[2] * f,
                        ]
                    })
            };
            let owners = self
                .players
                .iter_mut()
                .flatten()
                .map(|p| (p.handle, &mut p.character))
                .chain(self.npcs.iter_mut().map(|(h, n)| (*h, &mut n.character)));
            for (owner, character) in owners {
                let Some(&(_, at, ..)) = people.iter().find(|p| p.0 == owner) else {
                    continue;
                };
                for aura in &mut character.auras {
                    if npc_owners.contains(&owner) {
                        aura.ttl -= dt;
                    }
                    for _ in 0..aura.visual.tick(dt) {
                        for &(target, position, radius, _, ethereal, dead) in &people {
                            let (dx, dz) = (position[0] - at[0], position[2] - at[2]);
                            // GetEntities(iIgnoreYAxis): INFERRED reach of
                            // the target's radius beyond the aura radius.
                            if aura.self_only {
                                if target != owner {
                                    continue;
                                }
                            } else if ethereal
                                || (dx * dx + dz * dz).sqrt() - radius > aura.visual.radius
                            {
                                continue;
                            }
                            grants.push((
                                target,
                                aura.visual.buff_id,
                                aura.resistance,
                                aura.visual.color,
                                target == owner,
                                dead,
                                position,
                                radius,
                            ));
                        }
                    }
                    aura.visual.update_ring(dt, at, cutscene, ground, decals);
                }
                if npc_owners.contains(&owner) {
                    // Native executes once more before removing expired auras.
                    character.auras.retain(|a| a.ttl > 0.0);
                }
                let colors: Vec<_> = character
                    .auras
                    .iter()
                    .filter(|a| a.ttl > 0.0)
                    .map(|a| a.visual.color)
                    .collect();
                character.halo.update(dt, &colors);
                character.halo_draw = character.halo.draw(!colors.is_empty(), dt);
            }
        }
        for (target, id, resistance, color, own, dead, position, radius) in grants {
            if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, target) {
                c.buffs.add(
                    id,
                    resistance,
                    color,
                    own,
                    dead,
                    position,
                    radius,
                    &mut self.decals,
                );
            }
        }
        for &(handle, position, radius, height, ..) in &people {
            if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, handle) {
                c.buffs
                    .update(dt, position, radius, height, cutscene, &mut self.decals);
                for aura in &mut c.nonvisual_auras {
                    aura.ttl -= dt;
                    aura.period -= dt;
                    while aura.period <= 0. {
                        aura.period += 0.25;
                        c.buffs
                            .add_nonvisual(aura.id, aura.kind, 0.5, c.combat.dead);
                    }
                }
                c.nonvisual_auras.retain(|a| a.ttl > 0.);
                c.buffs
                    .update_nonvisual(dt, c.definition.max_hit_points, &mut c.combat.health);
            }
        }
    }

    fn character_mut<'a>(
        players: &'a mut [Option<crate::avatar::PlayerAvatar>],
        npcs: &'a mut BTreeMap<u16, crate::npc::NpcCharacter>,
        handle: u16,
    ) -> Option<&'a mut crate::character::RuntimeCharacter> {
        if let Some(p) = players.iter_mut().flatten().find(|p| p.handle == handle) {
            return Some(&mut p.character);
        }
        npcs.get_mut(&handle).map(|n| &mut n.character)
    }

    fn drown(&mut self, index: usize, depth: f32, lava: bool) {
        let (motion, character, handle) =
            if let Some(p) = self.players.iter_mut().flatten().find(|p| p.body == index) {
                p.current_spell = None;
                (&mut p.motion, &mut p.character, p.handle)
            } else if let Some(n) = self.npcs.values_mut().find(|n| n.body == index) {
                (&mut n.motion, &mut n.character, n.handle)
            } else {
                return;
            };
        motion.drowning = true;
        motion.dead_timer = 10.0;
        character.combat.health.invisibility_timer = 0.0;
        character.combat.health.hit_points = 0.0;
        let explicit_kill = self
            .entities
            .iter()
            .find(|e| e.handle == handle)
            .is_some_and(|e| e.cannot_die_without_explicit_kill);
        let body = &mut self.physics.bodies[index];
        let mut position = body.position;
        position[1] += depth;
        let orientation = body.orientation;
        body.move_to(position, orientation);
        if !explicit_kill {
            body.active = false;
        }
        let direction = body.direction();
        self.log.push(format!("drown {handle}"));
        self.drown_effects.push((
            if lava { "lava_drown" } else { "water_drown" },
            position,
            direction,
        ));
    }

    /// One PlayState.Update in original order: physics, animated level parts,
    /// entities (avatars), camera, then Level.Update (timers, presence,
    /// triggers). Avatar-requested triggers run after the avatar update.
    /// `runtime: None` keeps simulating the world without scripts.
    pub fn step_live(
        &mut self,
        mut runtime: Option<&mut ScriptRuntime>,
        dt: f32,
    ) -> Result<(), ScriptRuntimeError<WorldError>> {
        let party = self.players.iter().flatten().filter(|p| p.playing).count();
        self.boss_damage_divisor = crate::boss_scale::damage_divisor(self.boss_scale, party);
        for boss in self.jormungandr.values_mut() {
            boss.damage_divisor = self.boss_damage_divisor;
        }
        for zone in self.physics_damage.values_mut() {
            if zone.death_boss.is_some() {
                zone.death_damage_divisor = self.boss_damage_divisor;
            }
        }
        self.update_phys_animations();
        self.update_physics_damage(dt);
        self.spawn_prop_gibs();
        self.remove_dead_props();
        // Broad pair eligibility only identifies hosted characters. Native
        // dead/ethereal filtering runs after detection and shield callbacks.
        // Grip relationships remain unfinished.
        for player in self.players.iter_mut().flatten() {
            player.sync_immortality(self.flags.cutscene);
        }
        self.grease_contacts();
        let mut pair_eligible = vec![false; self.physics.bodies.len()];
        for player in self.players.iter().flatten() {
            pair_eligible[player.body] = true;
        }
        for npc in self.npcs.values() {
            pair_eligible[npc.body] = true;
        }
        let collision_shields: Vec<_> = self
            .shields
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                if s.shield.dead {
                    return None;
                }
                let geometry = s.shield.geometry();
                Some((
                    i,
                    magicka_sim::hollow_sphere_box::PrimitiveMotion {
                        old: geometry,
                        new: geometry,
                        owner_old_position: s.shield.position,
                    },
                ))
            })
            .collect();
        let geometry: Vec<_> = collision_shields.iter().map(|(_, s)| *s).collect();
        // Solid barriers block characters with their primitive-0 capsule.
        let obstacles: Vec<_> = self
            .barriers
            .iter()
            .filter(|(_, b)| b.solid())
            .map(|(_, b)| {
                let axis = Segment {
                    origin: [b.position[0], b.position[1] - 1.0, b.position[2]],
                    delta: [0.0, 0.1, 0.0],
                };
                magicka_sim::capsule_pair::CapsuleMotion {
                    old_axis: axis,
                    new_axis: axis,
                    old_radius: b.radius,
                    new_radius: b.radius,
                    old_body_position: b.position,
                }
            })
            .collect();
        let prop_boxes: Vec<_> = self
            .entities
            .iter()
            .filter(|e| e.added && !e.dead && !e.removed)
            .filter_map(|e| {
                let b = self.static_prop_boxes.get(&e.handle)?;
                Some(crate::physics_props::collision_box(b, &e.orientation))
            })
            .collect();
        let combatants = self.combatants();
        let view = crate::npc_combat::CombatView::new(&combatants, None);
        let events = {
            use crate::character_contact::ContactActor;
            // Distinct actor references are collected once. The two callbacks
            // execute synchronously and share them through a local RefCell.
            let mut actors = BTreeMap::new();
            for player in self.players.iter_mut().flatten() {
                actors.insert(player.body, ContactActor::Player(player));
            }
            for npc in self.npcs.values_mut() {
                actors.insert(npc.body, ContactActor::Npc(npc));
            }
            let actors = std::cell::RefCell::new(actors);
            let nullifications = std::cell::RefCell::new(&mut self.shield_nullifications);
            let shields = &mut self.shields;
            let mut accept_shield =
                |body_index,
                 shield_index: usize,
                 body: &mut magicka_sim::character_physics::CharacterBody| {
                    let mut actors = actors.borrow_mut();
                    let Some(actor) = actors.get_mut(&body_index) else {
                        return false;
                    };
                    let dead = actor.dead();
                    let (accept, effect) = crate::character_shield::contact(
                        actor.character(),
                        body,
                        dead,
                        &mut shields[collision_shields[shield_index].0].shield,
                    );
                    if let Some(effect) = effect {
                        nullifications.borrow_mut().push(effect);
                    }
                    accept
                };
            // Character.OnCollision for a barrier skin: dead, ethereal or
            // collision-ignoring characters reject the contact.
            let mut accept_obstacle =
                |body_index, _: usize, _: &mut magicka_sim::character_physics::CharacterBody| {
                    let mut actors = actors.borrow_mut();
                    actors.get_mut(&body_index).is_some_and(|actor| {
                        !actor.dead() && actor.character().accepts_owned_contact()
                    })
                };
            let triggers = &mut self.pending_triggers;
            let log = &mut self.log;
            let mut accept_pair =
                |i,
                 j,
                 body_a: &mut magicka_sim::character_physics::CharacterBody,
                 body_b: &mut magicka_sim::character_physics::CharacterBody| {
                    let mut actors = actors.borrow_mut();
                    let Some(mut a) = actors.remove(&i) else {
                        return false;
                    };
                    let Some(mut b) = actors.remove(&j) else {
                        actors.insert(i, a);
                        return false;
                    };
                    let result = match (
                        combatants.iter().find(|c| c.handle == a.handle()),
                        combatants.iter().find(|c| c.handle == b.handle()),
                    ) {
                        (Some(attacker_a), Some(attacker_b)) => crate::character_contact::pair(
                            [&mut a, &mut b],
                            [body_a, body_b],
                            [attacker_a, attacker_b],
                            |victim, attacker, amount| {
                                let Some(view) = view.as_ref() else {
                                    victim.combat.add_attacked_by(
                                        attacker.handle,
                                        attacker.faction,
                                        victim.character.definition.faction,
                                        amount,
                                    );
                                    return;
                                };
                                for (handle, amount) in crate::npc_combat::attacked_by_alerts(
                                    victim, view, attacker, amount,
                                ) {
                                    if handle == victim.handle {
                                        crate::npc_combat::add_alert_aggro(
                                            &mut victim.combat,
                                            attacker.handle,
                                            amount,
                                        );
                                    } else if let Some(ContactActor::Npc(npc)) =
                                        actors.values_mut().find(|a| a.handle() == handle)
                                    {
                                        crate::npc_combat::add_alert_aggro(
                                            &mut npc.combat,
                                            attacker.handle,
                                            amount,
                                        );
                                    }
                                }
                            },
                            triggers,
                        ),
                        _ => {
                            log.push(
                                "character contact: hosted actor missing combat metadata"
                                    .to_owned(),
                            );
                            let dead = [a.dead(), b.dead()];
                            Ok(crate::character_shield::pair(
                                [a.character(), b.character()],
                                [body_a, body_b],
                                dead,
                            ))
                        }
                    };
                    let (accept, effect) = result.unwrap_or_else(|e| {
                        log.push(format!("character contact damage: {e:?}"));
                        (false, None)
                    });
                    actors.insert(i, a);
                    actors.insert(j, b);
                    if let Some(effect) = effect {
                        nullifications.borrow_mut().push(effect);
                    }
                    accept
                };
            self.physics.integrate_with_shields(
                dt,
                &crate::character_collision::WithPropBoxes {
                    scene: self.collision.collision(),
                    boxes: &prop_boxes,
                },
                &|_, _| None,
                &|a, b| pair_eligible[a] && pair_eligible[b],
                Some(magicka_sim::character_physics::ShieldContacts {
                    geometry: &geometry,
                    accept: &mut accept_shield,
                    pair: Some(&mut accept_pair),
                    obstacles: &obstacles,
                    accept_obstacle: Some(&mut accept_obstacle),
                }),
            )
        };
        self.liquid_status_events.clear();
        for (index, event) in events {
            if let magicka_sim::character_physics::BodyEvent::LiquidStatus { depth, lava } = event {
                let mut notifications = Vec::new();
                let owner = if let Some(p) =
                    self.players.iter_mut().flatten().find(|p| p.body == index)
                {
                    Some((
                        p.handle,
                        p.liquid_status(depth, dt, lava, &mut notifications),
                    ))
                } else {
                    self.npcs.values_mut().find(|n| n.body == index).map(|n| {
                        (
                            n.handle,
                            n.character
                                .liquid_status(depth, dt, lava, None, &mut notifications),
                        )
                    })
                };
                if let Some((handle, result)) = owner {
                    if let Err(error) = result {
                        self.log.push(format!("liquid contact status: {error:?}"));
                    }
                    self.liquid_status_events
                        .extend(notifications.into_iter().map(|e| (handle, e)));
                }
                self.sync_status_audio(false);
                continue;
            }
            if let magicka_sim::character_physics::BodyEvent::Drown { depth, lava } = event {
                self.drown(index, depth, lava);
                continue;
            }
            if event == magicka_sim::character_physics::BodyEvent::KillPlane {
                if let Some(handle) = self
                    .npcs
                    .values()
                    .find(|n| n.body == index && !n.dead())
                    .map(|n| n.handle)
                {
                    // GameScene's kill-plane callback terminates any Character,
                    // including NPCs; NonPlayerCharacter.Terminate then calls Die.
                    self.terminate_character(handle);
                    if let Some((master, _)) = self.npcs.get(&handle).and_then(|n| n.summon) {
                        self.remove_bubble_shield(master);
                    }
                    if let Some(n) = self.npcs.get_mut(&handle) {
                        n.character.collision_hits.clear();
                        if n.on_death != 0 {
                            self.pending_triggers.push((n.on_death, Some(handle)));
                        }
                    }
                    self.agents.retain(|&h| h != handle);
                    continue;
                }
            }
            let Some(p) = self.players.iter_mut().flatten().find(|p| p.body == index) else {
                continue;
            };
            let body = &mut self.physics.bodies[index];
            match event {
                magicka_sim::character_physics::BodyEvent::ImpactDamage { amount, knockdown } => {
                    if !p.dead() {
                        p.apply_fall_damage(body, amount, knockdown, 0.0);
                    }
                }
                magicka_sim::character_physics::BodyEvent::KnockDown => {
                    p.events.push(crate::avatar::AvatarEvent::KnockDown)
                }
                magicka_sim::character_physics::BodyEvent::Drown { .. } => {}
                magicka_sim::character_physics::BodyEvent::LiquidStatus { .. } => {}
                magicka_sim::character_physics::BodyEvent::KillPlane => {
                    // GameScene kill plane: Character.Terminate.
                    if !p.dead() {
                        p.events.push(crate::avatar::AvatarEvent::KilledByKillPlane);
                        p.terminate();
                    }
                }
            }
        }
        self.mine_contacts();
        self.barrier_contacts();
        self.portal_contacts();
        self.missile_physics(dt);
        self.collision
            .update_with_decals(dt, &magicka_content::skinned::IDENTITY, &mut self.decals)
            .map_err(|e| ScriptRuntimeError::World(format!("collision update: {e:?}")))?;
        self.collision
            .carry_characters(dt, &mut self.physics)
            .map_err(|e| ScriptRuntimeError::World(format!("platform carry: {e:?}")))?;
        self.carry_attached_props()?;
        self.play_time += f64::from(dt);
        let transforms =
            crate::navigation::part_transforms(&self.level.animated_parts, self.collision.parts());
        if self.nav_part_transforms != transforms {
            self.nav = Some(
                crate::navigation::scene_nav_mesh(&self.level, self.collision.parts())
                    .map_err(|e| ScriptRuntimeError::World(format!("navigation update: {e:?}")))?,
            );
            self.nav_part_transforms = transforms;
        }
        // PlayState.Update: AIManager before the entity updates.
        self.update_agents(dt);
        self.update_possessed();
        // Damage numbers are allocated when Character.Damage runs; resolving
        // queued requests around each entity phase keeps Update's view of the
        // locked index current.
        self.apply_damage_numbers();
        self.sync_entanglement();
        self.update_players(dt);
        self.apply_damage_numbers();
        self.update_npcs(dt);
        self.sync_animation_audio();
        self.update_elemental_eggs(dt);
        self.update_dispensers(dt)
            .map_err(ScriptRuntimeError::World)?;
        self.update_anim_props(dt);
        self.update_web_strands(dt);
        self.update_discharges(dt);
        self.sync_hypnotize();
        self.apply_damage_numbers();
        self.sync_status_audio(true);
        self.update_bloats(dt);
        self.update_shields(dt);
        self.update_mines(dt);
        self.update_grease(dt);
        self.update_barriers(dt);
        self.update_portal_entities(dt);
        self.update_camera_follow();
        self.update_boss_fight(dt);
        self.update_level_title(dt);
        // PlayState.DiedInLevel.
        if self.players.iter().flatten().any(|p| p.playing && p.dead()) {
            self.died_in_level_flag = true;
        }
        self.update_missiles(dt);
        self.update_gibs(dt);
        self.update_underground(dt);
        // UpdateMiscA: SpellManager.Update (Railgun beams) after the
        // entities. Triggers/camera ordering follows the existing host.
        self.update_rails(dt);
        self.sync_block_effects();
        self.update_revival_fairies(dt);
        self.update_dead_age(dt);
        self.update_ability_effects(dt);
        self.update_death(dt);
        // GameScene.Update (level phase): directional light intensity.
        self.update_scene_light(dt);
        for character in self
            .players
            .iter_mut()
            .flatten()
            .map(|p| &mut p.character)
            .chain(self.npcs.values_mut().map(|n| &mut n.character))
        {
            for blood in std::mem::take(&mut character.blood_decals) {
                self.decals.blood_damage(blood, self.collision.collision());
            }
        }
        self.apply_damage_statistics();
        self.track_avatar_deaths();
        // UpdateMiscB: FloatingTextManager.Update after the entities.
        self.apply_damage_numbers();
        self.floating_numbers = self.floating_text.update(dt, false);
        let role = self.role;
        for (trigger, sender) in std::mem::take(&mut self.pending_triggers) {
            let Some(runtime) = runtime.as_deref_mut() else {
                break;
            };
            match runtime.execute_scene_trigger(trigger, sender, false, role, self) {
                Ok(_) | Err(ScriptRuntimeError::MissingTrigger(_)) => {}
                Err(e) => return Err(e),
            }
        }
        let subjects: Vec<_> = self
            .players
            .iter()
            .enumerate()
            .filter(|(slot, _)| self.camera_slots.as_ref().is_none_or(|s| s.contains(slot)))
            .filter_map(|(_, p)| p.as_ref())
            .map(|p| {
                let b = &self.physics.bodies[p.body];
                crate::camera::CameraSubject {
                    position: b.position,
                    dead: p.dead(),
                    grounded: !b.is_leaving_ground() && b.is_touching_ground(),
                    feet_offset: b.length * 0.5 + b.radius,
                }
            })
            .collect();
        let hostiles: Vec<_> = self
            .entities
            .iter()
            .filter(|e| {
                e.added
                    && !e.removed
                    && !e.dead
                    && matches!(e.kind, EntityKind::Npc { .. })
                    && self.faction(e) & crate::camera::FACTION_FRIENDLY
                        != crate::camera::FACTION_FRIENDLY
            })
            .map(|e| e.position)
            .collect();
        self.camera.update(&subjects, &hostiles, dt);
        let vp = crate::camera::multiply(&self.camera.view(), &self.camera.projection(self.aspect));
        self.update_dialogs(
            dt,
            std::array::from_fn(|r| std::array::from_fn(|c| vp[r * 4 + c])),
        );
        let Some(runtime) = runtime else {
            self.refresh_presence();
            return Ok(());
        };
        runtime.variables.update(dt, self.timer_frame());
        self.run_scene_scripts(runtime, dt)
    }
}

type LiveTextHost = crate::dialog_text_host::DialogTextHost<crate::dialog_world::SceneDialogWorld>;

/// DialogActionHost over the live text host and scene facts.
struct DialogBridge<'a, C: SceneContent> {
    host: &'a mut LiveTextHost,
    world: &'a SceneWorld<C>,
}
impl<C: SceneContent> crate::interaction_playback::InteractionHost for DialogBridge<'_, C> {
    type Error = <LiveTextHost as crate::interaction_playback::InteractionHost>::Error;
    fn advance_message(
        &mut self,
        index: usize,
        definition: &crate::dialog_definition::MessageDefinition,
        playback: &mut crate::message_playback::MessagePlayback,
        placement: crate::message_playback::MessagePlacement,
    ) -> Result<bool, Self::Error> {
        self.host
            .advance_message(index, definition, playback, placement)
    }
    fn lock_input(&mut self, controller: i32) -> Result<(), Self::Error> {
        self.host.lock_input(controller)
    }
    fn unlock_input(&mut self, controller: i32) -> Result<(), Self::Error> {
        self.host.unlock_input(controller)
    }
    fn end_message(
        &mut self,
        index: usize,
        definition: &crate::dialog_definition::MessageDefinition,
    ) -> Result<(), Self::Error> {
        self.host.end_message(index, definition)
    }
}
impl<C: SceneContent> crate::dialog_manager::DialogManagerHost for DialogBridge<'_, C> {
    fn bind(
        &mut self,
        sink: crate::dialog_manager::DialogSink,
        active: crate::dialog_manager::ActiveInteraction,
    ) {
        self.host.bind(sink, active)
    }
    fn visible(&self, sink: crate::dialog_manager::DialogSink) -> bool {
        self.host.visible(sink)
    }
    fn animating(&self, sink: crate::dialog_manager::DialogSink) -> bool {
        self.host.animating(sink)
    }
    fn manager_effect(
        &mut self,
        effect: crate::dialog_manager::DialogManagerEffect,
    ) -> Result<(), Self::Error> {
        self.host.manager_effect(effect)
    }
}
impl<C: SceneContent> crate::dialog_action::DialogActionHost for DialogBridge<'_, C> {
    fn resolve_character(
        &mut self,
        id: i32,
    ) -> Result<Option<crate::dialog_action::DialogActionCharacter>, Self::Error> {
        Ok(self
            .world
            .by_unique_id(id)
            .filter(|_| id != 0)
            .and_then(|entity| self.world.dialog_character(|e| e.handle == entity.handle)))
    }
    fn resolve_sender(
        &mut self,
        sender: u16,
    ) -> Result<Option<crate::dialog_action::DialogActionCharacter>, Self::Error> {
        Ok(self.world.dialog_character(|e| e.handle == sender))
    }
    fn locator(&mut self, id: i32) -> Result<[f32; 3], Self::Error> {
        // GameScene.GetLocator; a missing key throws in the original. Use the
        // origin like an unresolved owner instead of failing the dialog.
        Ok(self
            .world
            .locators
            .get(&id)
            .map_or([0.0; 3], |m| [m[12], m[13], m[14]]))
    }
    fn cutscene(&self) -> bool {
        self.world.flags.cutscene
    }
}

impl<C: SceneContent> SceneWorld<C> {
    fn dialog_character(
        &self,
        pick: impl Fn(&SceneEntity) -> bool,
    ) -> Option<crate::dialog_action::DialogActionCharacter> {
        let e = self.entities.iter().rev().find(|e| {
            e.added
                && !e.removed
                && matches!(e.kind, EntityKind::Npc { .. } | EntityKind::Avatar { .. })
                && pick(e)
        })?;
        let avatar = matches!(e.kind, EntityKind::Avatar { .. });
        Some(crate::dialog_action::DialogActionCharacter {
            owner: e.unique_id,
            avatar,
            // Avatar.Player.Controller: one local controller per slot.
            controller: match e.kind {
                EntityKind::Avatar { slot } => Some(slot as i32),
                _ => None,
            },
        })
    }

    /// Owner facts for dialog placement and death checks.
    fn refresh_dialog_world(&self, text: &mut LiveTextHost) {
        let owners = &mut text.world.owners;
        owners.clear();
        for e in self
            .entities
            .iter()
            // TextBox retains its Entity reference after removal. Keep its
            // last position until DialogManager can end/hide the interaction;
            // dropping these facts makes TextBox.Update fail before cleanup.
            .filter(|e| e.added && e.unique_id != 0)
        {
            let (capsule, animation_allowed, special_idle) = match e.kind {
                EntityKind::Avatar { slot } => match &self.players[slot] {
                    Some(p) => {
                        let b = &self.physics.bodies[p.body];
                        (
                            Some((b.length, b.radius)),
                            p.ai.events().is_none(),
                            p.motion.special_idle,
                        )
                    }
                    None => (None, true, 0),
                },
                EntityKind::Npc { .. } => (
                    Some((e.length, e.radius)),
                    e.event_count == 0,
                    e.special_idle,
                ),
                EntityKind::Physics { .. }
                | EntityKind::Item
                | EntityKind::ElementalEgg
                | EntityKind::Dispenser => (None, true, 0),
            };
            owners.insert(
                e.unique_id,
                crate::dialog_world::DialogOwnerFacts {
                    position: e.position,
                    capsule,
                    dead: e.dead || e.removed,
                    character: !matches!(
                        e.kind,
                        EntityKind::Physics { .. } | EntityKind::ElementalEgg
                    ),
                    animation_allowed,
                    special_idle,
                },
            );
        }
        for (&owner, &handle) in &self.dialog_handle_owners {
            if let Some(e) = self.entities.iter().find(|e| e.handle == handle && e.added) {
                owners.insert(
                    owner,
                    crate::dialog_world::DialogOwnerFacts {
                        position: e.position,
                        capsule: Some((e.length, e.radius)),
                        dead: e.dead || e.removed,
                        character: true,
                        animation_allowed: e.event_count == 0,
                        special_idle: e.special_idle,
                    },
                );
            }
        }
        text.world.cutscene = self.flags.cutscene;
    }

    /// Apply audio/camera/animation/input effects requested by dialog playback.
    fn apply_dialog_commands(&mut self) {
        use crate::dialog_manager::DialogManagerEffect as M;
        use crate::dialog_world::DialogWorldCommand as D;
        use crate::message_playback::MessageEffect as E;
        let Some(text) = self.dialog_text.as_mut() else {
            return;
        };
        let commands = std::mem::take(&mut text.world.commands);
        for command in commands {
            match command {
                D::PlayCue {
                    message,
                    bank,
                    sound,
                    emitter,
                } => {
                    let position = emitter.and_then(|owner| {
                        self.dialog_text
                            .as_ref()?
                            .world
                            .owners
                            .get(&owner)
                            .map(|o| o.position)
                    });
                    let key = self.backend().next_key();
                    self.audio_requests.push(crate::audio::AudioRequest::Start {
                        key,
                        bank,
                        cue: sound,
                        volume_db: 0.0,
                        source: position.map_or(
                            crate::audio::SoundSource::Global,
                            crate::audio::SoundSource::Emitter,
                        ),
                        magnitude: None,
                    });
                    self.dialog_text
                        .as_mut()
                        .unwrap()
                        .world
                        .cue_keys
                        .insert(message, key);
                }
                D::StopCue { message, instant } => {
                    if let Some(&key) = self
                        .dialog_text
                        .as_ref()
                        .unwrap()
                        .world
                        .cue_keys
                        .get(&message)
                    {
                        self.audio_requests
                            .push(crate::audio::AudioRequest::Stop { key, instant });
                    }
                }
                D::Message(E::MoveCamera { owner, time }) => {
                    if let Some(e) = self
                        .entities
                        .iter()
                        .rev()
                        .find(|e| {
                            owner != 0
                                && (e.unique_id == owner
                                    || self.dialog_handle_owners.get(&owner) == Some(&e.handle))
                        })
                        .filter(|e| !e.removed)
                    {
                        self.camera.move_to(e.position, time);
                    }
                }
                D::Message(E::Magnification(m)) => self.camera.set_magnification(m),
                D::Message(E::ReleaseCamera(time)) => self.camera.release(time),
                D::Message(E::Animate {
                    owner,
                    animation,
                    blend,
                }) => {
                    if let Some(handle) = self
                        .entities
                        .iter()
                        .rev()
                        .find(|e| {
                            owner != 0
                                && (e.unique_id == owner
                                    || self.dialog_handle_owners.get(&owner) == Some(&e.handle))
                        })
                        .filter(|e| !e.removed)
                        .map(|e| e.handle)
                    {
                        if let Some(p) = self
                            .players
                            .iter_mut()
                            .flatten()
                            .find(|p| p.handle == handle)
                        {
                            p.go_to_animation(animation, blend);
                        } else if let Some(npc) = self.npcs.get_mut(&handle) {
                            npc.motion.go_to_animation(animation, blend);
                        }
                    }
                }
                D::Manager(M::LimitInput) => self.flags.input_limited = true,
                D::Manager(M::UnlimitInput) => self.flags.input_limited = false,
                _ => {}
            }
        }
    }

    /// DialogManager.Update in UpdateMiscA (after the camera).
    pub fn update_dialogs(&mut self, dt: f32, view_projection: [[f32; 4]; 4]) {
        if self.dialog_failure.is_some() {
            return;
        }
        let (Some(mut manager), Some(mut text)) = (self.dialogs.take(), self.dialog_text.take())
        else {
            return;
        };
        self.refresh_dialog_world(&mut text);
        self.dialog_matrix = view_projection;
        let mut matrix = view_projection;
        if let Err(e) = manager.update(
            crate::dialog_frame::DialogFrame { channel: 0, dt },
            &mut matrix,
            &mut text,
        ) {
            self.dialog_failure = Some(format!("dialog frame: {e:?}"));
        }
        self.dialogs = Some(manager);
        self.dialog_text = Some(text);
        self.apply_dialog_commands();
    }

    /// KeyboardMouseController: advance the avatar's dialog when allowed.
    pub fn advance_dialog(&mut self, controller: i32) {
        let cutscene = self.flags.cutscene;
        let role = self.role;
        let (Some(mut manager), Some(mut text)) = (self.dialogs.take(), self.dialog_text.take())
        else {
            return;
        };
        if manager.can_advance(Some(controller)) {
            if let Err(e) = manager.advance_controller(Some(controller), cutscene, role, &mut text)
            {
                self.dialog_failure = Some(format!("dialog advance: {e:?}"));
            }
        }
        self.dialogs = Some(manager);
        self.dialog_text = Some(text);
        self.apply_dialog_commands();
    }
}

impl<C: SceneContent> crate::size_magicks::SizeHost for SceneWorld<C> {
    fn size_owner(&self, owner: u16) -> Option<(f32, f32)> {
        let c = self.character_ref(owner)?;
        Some((c.combat.health.hit_points, c.definition.mass))
    }
    fn apply_size(&mut self, owner: u16, scale: f32, mass: f32, speed: f32) {
        let Some(index) = self.character_body(owner) else {
            return;
        };
        let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, owner) else {
            return;
        };
        let body = &mut self.physics.bodies[index];
        let (length, radius) = (c.definition.length * scale, c.definition.radius * scale);
        if body.length != length || body.radius != radius {
            body.set_capsule_form(length, radius);
            c.model_scale = scale;
        }
        body.mass = mass;
        body.set_speed_multiplier(body.speed_multiplier() * speed);
    }
    fn reset_size(&mut self, owner: u16) {
        let Some(index) = self.character_body(owner) else {
            return;
        };
        let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, owner) else {
            return;
        };
        let body = &mut self.physics.bodies[index];
        body.set_capsule_form(c.definition.length, c.definition.radius);
        body.mass = c.definition.mass;
        c.model_scale = 1.0;
    }
    fn size_buff(&mut self, owner: u16, id: u32, kind: crate::buffs::NonvisualKind, ttl: f32) {
        let npc = self.npcs.contains_key(&owner);
        if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, owner) {
            c.add_nonvisual_buff(id, kind, ttl, npc);
        }
    }
}

/// SprayEffects over the scene's props and characters: public damage delivery and the
/// OnDamage trigger. Character.Electrocute is empty in this build.
struct SceneSprayEffects<'a> {
    missiles: &'a mut Vec<crate::missiles::LiveMissile>,
    physics_damage: &'a mut BTreeMap<u16, crate::physics_damage::PhysicsDamage>,
    dispensers: &'a mut BTreeMap<u16, crate::dispenser::Dispenser>,
    entities: &'a mut Vec<SceneEntity>,
    elemental_eggs: &'a mut Option<Box<crate::elemental_egg_actor::ElementalEggCache>>,
    death: &'a mut death_live::DeathHost,
    jormungandr: &'a mut BTreeMap<u16, crate::jormungandr::Jormungandr>,
    death_damage_features: bool,
    // The casting player is temporarily taken out of the players array.
    caster_boosts: Vec<magicka_sim::damage::Damage>,
    /// Characters at the start of the caster's CastUpdate (aggro queries).
    combatants: Vec<crate::npc_combat::Combatant>,
    npcs: &'a mut BTreeMap<u16, crate::npc::NpcCharacter>,
    players: &'a mut [Option<PlayerAvatar>],
    bodies_before: &'a mut [magicka_sim::character_physics::CharacterBody],
    bodies_after: &'a mut [magicka_sim::character_physics::CharacterBody],
    caster_index: usize,
    triggers: &'a mut Vec<(i32, Option<u16>)>,
    shields: &'a mut Vec<LiveShield>,
    mines: &'a mut magicka_sim::spell_mine::MinePool,
    grease: &'a mut magicka_sim::grease::GreasePool,
    barriers: &'a mut magicka_sim::barrier::BarrierPool,
    /// Barrier events raised by spray damage, processed after CastUpdate.
    barrier_events: Vec<(usize, Vec<magicka_sim::barrier::BarrierEvent>)>,
}
impl SceneSprayEffects<'_> {
    fn deliver(
        &mut self,
        damage: crate::spray_world::SprayDamage,
        payload: magicka_sim::damage_delivery::DamagePayload,
    ) {
        let payload = payload.with_boosts(&self.caster_boosts);
        let collection = match payload {
            magicka_sim::damage_delivery::DamagePayload::Collection(d) => d,
            magicka_sim::damage_delivery::DamagePayload::Single(d) => {
                let mut c = [magicka_sim::damage::Damage::default(); 5];
                c[0] = d;
                c
            }
        };
        if let Some(missile) = self
            .missiles
            .iter_mut()
            .find(|m| m.handle == Some(damage.target) && !m.body.dead)
        {
            for entry in collection {
                missile.state.receive_damage(entry, false);
            }
            return;
        }
        if let Some(d) = self.dispensers.get_mut(&damage.target) {
            for entry in collection.iter().copied().filter(|d| d.attack != 0) {
                d.damage(entry, self.death_damage_features);
            }
            if let Some(e) = self.entities.iter_mut().find(|e| e.handle == damage.target) {
                e.hit_points = d.health;
                e.dead = d.health <= 0.0;
            }
            return;
        }
        if let Some(state) = self.physics_damage.get_mut(&damage.target) {
            if self.death_damage_features {
                state.deliver(&collection, 15, self.triggers);
                if let Some(entity) = self.entities.iter_mut().find(|e| e.handle == damage.target) {
                    entity.hit_points = state.health;
                    entity.dead = state.health <= 0.0;
                }
            }
            return;
        }
        if let Some(boss) = self.jormungandr.get_mut(&damage.target) {
            boss.damage(&collection, self.death_damage_features);
            return;
        }
        if self.death.handle == Some(damage.target) {
            self.death.deliver(&collection, self.death_damage_features);
            return;
        }
        if self
            .elemental_eggs
            .as_mut()
            .and_then(|cache| cache.deliver(damage.target, &collection))
            .is_some()
        {
            return;
        }
        if damage.target & MINE_KEY == GREASE_KEY {
            if let Some(f) = self.grease.get_mut((damage.target & !MINE_KEY) as usize) {
                for d in &collection {
                    f.field
                        .damage(magicka_sim::spells::Elements(d.element), d.magnitude);
                }
            }
            return;
        }
        if damage.target & MINE_KEY == MINE_KEY {
            if let Some(m) = self.mines.get_mut((damage.target & !MINE_KEY) as usize) {
                m.internal_damage(&collection);
            }
            return;
        }
        if damage.target & MINE_KEY == BARRIER_KEY {
            let slot = (damage.target & !BARRIER_KEY) as usize;
            if let Some(b) = self.barriers.get_mut(slot) {
                let (_, events) = b.internal_damage(&collection, true);
                self.barrier_events.push((slot, events));
            }
            return;
        }
        // Shield.InternalDamage (collection: each slot; single: one entry).
        if let Some(s) = self.shields.iter_mut().find(|s| s.handle == damage.target) {
            match payload {
                magicka_sim::damage_delivery::DamagePayload::Collection(d) => {
                    for d in &d {
                        s.shield.internal_damage(d, true);
                    }
                }
                magicka_sim::damage_delivery::DamagePayload::Single(d) => {
                    s.shield.internal_damage(&d, true);
                }
            }
            return;
        }
        if self.npcs.contains_key(&damage.target) {
            // NonPlayerCharacter.InternalDamage: AddAttackedBy per entry
            // (amount × magnitude) before the damage itself.
            if let Some(view) = crate::npc_combat::CombatView::new(&self.combatants, None) {
                if let Some(attacker) = view.get(damage.attacker).copied() {
                    for d in collection.iter().filter(|d| d.attack != 0) {
                        crate::npc_combat::add_attacked_by(
                            self.npcs,
                            &view,
                            damage.target,
                            &attacker,
                            d.amount * d.magnitude,
                        );
                    }
                }
            }
        }
        if let Some(npc) = self.npcs.get_mut(&damage.target) {
            let result = crate::avatar::deliver_to_body(
                &mut npc.character,
                if npc.body < self.caster_index {
                    &mut self.bodies_before[npc.body]
                } else {
                    &mut self.bodies_after[npc.body - self.caster_index - 1]
                },
                &[magicka_sim::resistance::Resistance::IDENTITY; 11],
                false,
                damage.target,
                payload,
                Some(damage.attacker),
                damage.position,
                damage.timestamp,
            );
            if result & magicka_sim::character_impact::result::DAMAGED != 0 && npc.on_damage != 0 {
                self.triggers.push((npc.on_damage, Some(damage.target)));
            }
        } else if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == damage.target)
        {
            crate::avatar::deliver_to_body(
                &mut p.character,
                if p.body < self.caster_index {
                    &mut self.bodies_before[p.body]
                } else {
                    &mut self.bodies_after[p.body - self.caster_index - 1]
                },
                &[magicka_sim::resistance::Resistance::IDENTITY; 11],
                true,
                damage.target,
                payload,
                Some(damage.attacker),
                damage.position,
                damage.timestamp,
            );
        }
    }
}
impl crate::spray_world::SprayEffects for SceneSprayEffects<'_> {
    fn damage(
        &mut self,
        entities: &mut [crate::spray_world::WorldEntity],
        damage: crate::spray_world::SprayDamage,
    ) {
        let payload = magicka_sim::damage_delivery::DamagePayload::Collection(damage.damages);
        self.deliver(damage, payload);
        if let Some(state) = self.physics_damage.get(&damage.target) {
            if let Some(target) = entities.iter_mut().find(|e| e.handle == damage.target) {
                target.dead = state.health <= 0.0;
                target.frozen = state.status_mask & magicka_sim::status::Status::Frozen as u16 != 0;
            }
        }
    }
    fn electrocute(&mut self, _: &mut [crate::spray_world::WorldEntity], _: u16, _: u16, _: f32) {}
}
impl crate::spray_world::PushEffects for SceneSprayEffects<'_> {
    fn single_push(
        &mut self,
        _: &mut [crate::spray_world::WorldEntity],
        damage: crate::spray_world::SprayDamage,
    ) {
        let payload = magicka_sim::damage_delivery::DamagePayload::Single(damage.damages[0]);
        self.deliver(damage, payload);
    }
    fn impulse_push(
        &mut self,
        target: u16,
        direction: [f32; 3],
        elevation: f32,
        force: f32,
        distance: f32,
    ) {
        if let Some(missile) = self
            .missiles
            .iter_mut()
            .find(|m| m.handle == Some(target) && !m.body.dead)
        {
            missile.body.push(direction, elevation, force, distance);
            return;
        }
        let index = self.npcs.get(&target).map(|n| n.body).or_else(|| {
            self.players
                .iter()
                .flatten()
                .find(|p| p.handle == target)
                .map(|p| p.body)
        });
        if let Some(i) = index {
            let body = if i < self.caster_index {
                &mut self.bodies_before[i]
            } else {
                &mut self.bodies_after[i - self.caster_index - 1]
            };
            let impulse = magicka_sim::impact_motion::MotionContext {
                position: body.position,
                mass: body.mass,
                gravity_y: magicka_sim::character_physics::GRAVITY[1],
                entangled: body.owner.entangled,
                ..Default::default()
            }
            .impulse(direction, elevation, force, distance);
            body.add_impulse_velocity(impulse);
        }
    }
}

/// Character damage through the public route for scene hits (melee,
/// missiles, blasts): NPC victims record the attacker first.
struct SceneHitWorld<'a, 'v> {
    physics_damage: &'a mut BTreeMap<u16, crate::physics_damage::PhysicsDamage>,
    dispensers: &'a mut BTreeMap<u16, crate::dispenser::Dispenser>,
    entities: &'a mut Vec<SceneEntity>,
    death: &'a mut death_live::DeathHost,
    jormungandr: &'a mut BTreeMap<u16, crate::jormungandr::Jormungandr>,
    death_damage_features: bool,
    view: &'a crate::npc_combat::CombatView<'v>,
    npcs: &'a mut BTreeMap<u16, crate::npc::NpcCharacter>,
    players: &'a mut [Option<PlayerAvatar>],
    bodies: &'a mut Vec<magicka_sim::character_physics::CharacterBody>,
    triggers: &'a mut Vec<(i32, Option<u16>)>,
    play_time: f64,
    shields: &'a mut Vec<LiveShield>,
    collision_rng: &'a mut u64,
    /// Collection FullBlasts for mines and barriers (applied by the host).
    entity_blasts: &'a mut Vec<EntityBlast>,
    /// Barriers missiles collide with (the missile physics pass only).
    barriers: Option<&'a mut magicka_sim::barrier::BarrierPool>,
    /// Barrier events from missile damage, applied by the host afterwards.
    barrier_events: Vec<(usize, Vec<magicka_sim::barrier::BarrierEvent>)>,
}
impl SceneHitWorld<'_, '_> {
    fn boosted_payload(
        &self,
        attacker: Option<u16>,
        payload: magicka_sim::damage_delivery::DamagePayload,
    ) -> magicka_sim::damage_delivery::DamagePayload {
        let character = attacker.and_then(|handle| {
            self.npcs.get(&handle).map(|n| &n.character).or_else(|| {
                self.players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == handle)
                    .map(|p| &p.character)
            })
        });
        character.map_or(payload, |c| payload.with_boosts(&c.buffs.damage_boosts()))
    }
}
impl crate::missiles::MissileWorld for SceneHitWorld<'_, '_> {
    fn props(&self) -> Vec<crate::missiles::MissileProp> {
        self.entities
            .iter()
            .filter(|e| e.added && !e.removed)
            .filter_map(|e| {
                let state = self.physics_damage.get(&e.handle)?;
                Some(crate::missiles::MissileProp {
                    handle: e.handle,
                    position: e.position,
                    bounds: state.world_box(&e.orientation)?,
                    solid: state.solid,
                    dead: e.dead || state.health <= 0.0,
                })
            })
            .collect()
    }

    fn damage_prop(
        &mut self,
        target: u16,
        damage: magicka_sim::damage::Damage,
        attacker: u16,
        _position: [f32; 3],
    ) -> u16 {
        if !self.death_damage_features {
            return 0;
        }
        let magicka_sim::damage_delivery::DamagePayload::Single(damage) = self.boosted_payload(
            Some(attacker),
            magicka_sim::damage_delivery::DamagePayload::Single(damage),
        ) else {
            unreachable!()
        };
        let Some(state) = self.physics_damage.get_mut(&target) else {
            return 0;
        };
        let result = state.deliver(&[damage], 15, self.triggers);
        if let Some(entity) = self.entities.iter_mut().find(|e| e.handle == target) {
            entity.hit_points = state.health;
            entity.dead = state.health <= 0.0;
        }
        result
    }
    fn callback_capsules(&self) -> Vec<crate::missiles::MissileCallbackCapsule> {
        self.death
            .callback_capsule()
            .into_iter()
            .chain(self.physics_damage.iter().filter_map(|(&h, state)| {
                self.entities
                    .iter()
                    .any(|e| e.handle == h && e.added && !e.removed)
                    .then(|| state.death_capsule(h))
                    .flatten()
            }))
            .chain(self.jormungandr.iter().filter_map(|(&handle, boss)| {
                boss.update_animation
                    .then_some(crate::missiles::MissileCallbackCapsule {
                        handle,
                        old_axis: Segment {
                            origin: boss.frame.head,
                            delta: [0.; 3],
                        },
                        new_axis: Segment {
                            origin: boss.frame.head,
                            delta: [0.; 3],
                        },
                        radius: 1.1,
                        old_body_position: boss.frame.head,
                        solid: true,
                    })
            }))
            .chain(self.dispensers.iter().filter_map(|(&handle, d)| {
                self.entities
                    .iter()
                    .any(|e| e.handle == handle && e.added && !e.removed)
                    .then_some(crate::missiles::MissileCallbackCapsule {
                        handle,
                        old_axis: d.axis(),
                        new_axis: d.axis(),
                        radius: 1.25,
                        solid: false,
                        old_body_position: d.position(),
                    })
            }))
            .collect()
    }
    fn barriers(&self) -> Vec<crate::missiles::MissileBarrier> {
        let Some(pool) = self.barriers.as_ref() else {
            return Vec::new();
        };
        pool.iter()
            .filter(|(_, b)| !b.is_dead())
            .map(|(slot, b)| crate::missiles::MissileBarrier {
                handle: BARRIER_KEY | slot as u16,
                position: b.position,
                solid: b.solid(),
                // Primitive 0: from Position − 1, 0.1 long (barrier_contacts).
                bottom: [b.position[0], b.position[1] - 1.0, b.position[2]],
                length: 0.1,
                radius: b.radius,
                sphere_center: [b.position[0], b.position[1] - b.radius, b.position[2]],
                sphere_radius: if b.solid() { b.radius * 2.0 } else { b.radius },
            })
            .collect()
    }
    fn damage_barrier(&mut self, target: u16, damage: magicka_sim::damage::Damage) -> u16 {
        let slot = (target & !BARRIER_KEY) as usize;
        let Some(b) = self.barriers.as_mut().and_then(|pool| pool.get_mut(slot)) else {
            return 0;
        };
        let mut events = Vec::new();
        let result = b.internal_damage_one(damage, self.death_damage_features, &mut events);
        self.barrier_events.push((slot, events));
        result
    }
    fn record_splash(
        &mut self,
        position: [f32; 3],
        radius: f32,
        damage: magicka_sim::damage::Damage,
        owner: u16,
        ignore: Option<u16>,
    ) -> bool {
        self.entity_blasts.push(EntityBlast::Splash {
            position,
            radius,
            damage,
            owner,
            ignore,
        });
        true
    }
    fn record_blast(
        &mut self,
        position: [f32; 3],
        radius: f32,
        damages: [magicka_sim::damage::Damage; 5],
    ) {
        self.entity_blasts
            .push(EntityBlast::Full(position, radius, damages));
    }
    fn shields(&self) -> Vec<crate::missiles::MissileShield> {
        self.shields
            .iter()
            .map(|s| crate::missiles::MissileShield {
                handle: s.handle,
                position: s.shield.position,
                radius: s.shield.radius,
                geometry: s.shield.geometry(),
                dead: s.shield.dead,
            })
            .collect()
    }
    fn damage_shield(&mut self, target: u16, damage: magicka_sim::damage::Damage) -> u16 {
        self.shields
            .iter_mut()
            .find(|s| s.handle == target)
            .map_or(0, |s| s.shield.internal_damage(&damage, true))
    }
    fn collision_random_degrees(&mut self) -> u16 {
        *self.collision_rng ^= *self.collision_rng << 13;
        *self.collision_rng ^= *self.collision_rng >> 7;
        *self.collision_rng ^= *self.collision_rng << 17;
        ((*self.collision_rng >> 40) as f32 / (1u64 << 24) as f32 * 360.0) as u16
    }
    fn view(&self) -> &crate::npc_combat::CombatView<'_> {
        self.view
    }
    fn scene_hit(&self, origin: [f32; 3], delta: [f32; 3]) -> bool {
        self.view.scene_hit(origin, delta)
    }
    fn damage_character(
        &mut self,
        target: u16,
        damage: magicka_sim::damage::Damage,
        attacker: u16,
        position: [f32; 3],
    ) -> u16 {
        if self.jormungandr.contains_key(&target) {
            let payload = self.boosted_payload(
                Some(attacker),
                magicka_sim::damage_delivery::DamagePayload::Single(damage),
            );
            return self
                .jormungandr
                .get_mut(&target)
                .unwrap()
                .damage(&payload.request_slots(), self.death_damage_features);
        }
        if self.death.handle == Some(target) {
            let payload = self.boosted_payload(
                Some(attacker),
                magicka_sim::damage_delivery::DamagePayload::Single(damage),
            );
            return if self
                .death
                .deliver(&payload.request_slots(), self.death_damage_features)
            {
                magicka_sim::character_impact::result::HIT
            } else {
                0
            };
        }
        if self.dispensers.contains_key(&target) {
            let magicka_sim::damage_delivery::DamagePayload::Single(damage) = self.boosted_payload(
                Some(attacker),
                magicka_sim::damage_delivery::DamagePayload::Single(damage),
            ) else {
                unreachable!()
            };
            let d = self.dispensers.get_mut(&target).unwrap();
            let result = d.damage(damage, self.death_damage_features);
            if let Some(e) = self.entities.iter_mut().find(|e| e.handle == target) {
                e.hit_points = d.health;
                e.dead = d.health <= 0.0;
            }
            return result;
        }
        if self.physics_damage.contains_key(&target) {
            return self.damage_prop(target, damage, attacker, position);
        }
        if self.view.get(target).is_none() {
            return 0;
        }
        let magicka_sim::damage_delivery::DamagePayload::Single(damage) = self.boosted_payload(
            Some(attacker),
            magicka_sim::damage_delivery::DamagePayload::Single(damage),
        ) else {
            unreachable!()
        };
        if self.npcs.contains_key(&target) {
            if let Some(a) = self.view.get(attacker).copied() {
                crate::npc_combat::add_attacked_by(
                    self.npcs,
                    self.view,
                    target,
                    &a,
                    damage.amount * damage.magnitude,
                );
            }
            let npc = self.npcs.get_mut(&target).unwrap();
            let result = crate::avatar::deliver_to_body(
                &mut npc.character,
                &mut self.bodies[npc.body],
                &[magicka_sim::resistance::Resistance::IDENTITY; 11],
                false,
                target,
                magicka_sim::damage_delivery::DamagePayload::Single(damage),
                Some(attacker),
                position,
                self.play_time,
            );
            if result & magicka_sim::character_impact::result::DAMAGED != 0 && npc.on_damage != 0 {
                self.triggers.push((npc.on_damage, Some(target)));
            }
            result
        } else if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == target)
        {
            let body = &mut self.bodies[p.body];
            p.receive_damage(body, damage, attacker, position, self.play_time)
        } else {
            0
        }
    }
}

/// The scene as Railgun beams and beam blasts see it: live characters
/// (capsules), level collision, and damage through the public route.
struct RailSceneWorld<'a, 'v> {
    mines: &'a mut magicka_sim::spell_mine::MinePool,
    portals: &'a portal_live::PortalHost,
    missiles: &'a mut Vec<crate::missiles::LiveMissile>,
    elemental_eggs: &'a mut Option<Box<crate::elemental_egg_actor::ElementalEggCache>>,
    hit: SceneHitWorld<'a, 'v>,
    scene: &'a crate::level_collision::SceneCollision,
    blasts: &'a mut Vec<([f32; 3], f32, u32)>,
    collection_visuals: &'a mut Vec<crate::missiles::MissileVisual>,
    bloat_requests: Vec<(u16, u32, u16)>,
    /// Characters bloating now (Character.Bloating), including this pass's.
    bloating: BTreeSet<u16>,
}
impl RailSceneWorld<'_, '_> {
    /// Character.CurrentSelfShieldType (None when no shield is active).
    fn self_shield(&self, handle: u16) -> Option<magicka_sim::character_health::SelfShieldType> {
        let shield = if let Some(n) = self.hit.npcs.get(&handle) {
            &n.character.combat.health.shield
        } else {
            &self
                .hit
                .players
                .iter()
                .flatten()
                .find(|p| p.handle == handle)?
                .character
                .combat
                .health
                .shield
        };
        shield.active().then_some(shield.kind)
    }

    /// IDamageable.Damage(DamageCollection5): NPC victims record the attacker
    /// per entry first (NonPlayerCharacter.InternalDamage).
    fn deliver_collection(
        &mut self,
        target: u16,
        attacker: Option<u16>,
        damages: [magicka_sim::damage::Damage; 5],
        timestamp: f64,
        position: [f32; 3],
    ) -> u16 {
        let payload = self.hit.boosted_payload(
            attacker,
            magicka_sim::damage_delivery::DamagePayload::Collection(damages),
        );
        let damages = payload.request_slots();
        if target & MINE_KEY == MINE_KEY {
            return if self
                .mines
                .get_mut((target & !MINE_KEY) as usize)
                .is_some_and(|mine| mine.internal_damage(&damages))
            {
                magicka_sim::character_impact::result::DAMAGED
            } else {
                0
            };
        }
        if target & MINE_KEY == BARRIER_KEY {
            let slot = (target & !BARRIER_KEY) as usize;
            if let Some(b) = self
                .hit
                .barriers
                .as_mut()
                .and_then(|pool| pool.get_mut(slot))
            {
                let (result, events) = b.internal_damage(&damages, self.hit.death_damage_features);
                self.hit.barrier_events.push((slot, events));
                return result;
            }
        }
        if let Some(d) = self.hit.dispensers.get_mut(&target) {
            let mut result = 0;
            for entry in damages.into_iter().filter(|d| d.attack != 0) {
                result |= d.damage(entry, self.hit.death_damage_features);
            }
            if let Some(e) = self.hit.entities.iter_mut().find(|e| e.handle == target) {
                e.hit_points = d.health;
                e.dead = d.health <= 0.0;
            }
            return result;
        }
        if let Some(missile) = self
            .missiles
            .iter_mut()
            .find(|m| m.handle == Some(target) && !m.body.dead)
        {
            for entry in damages {
                missile.state.receive_damage(entry, false);
            }
            return 0;
        }

        if let Some(hit) = self
            .elemental_eggs
            .as_mut()
            .and_then(|cache| cache.deliver(target, &damages))
        {
            return if hit {
                magicka_sim::character_impact::result::HIT
            } else {
                0
            };
        }
        if let Some(boss) = self.hit.jormungandr.get_mut(&target) {
            return boss.damage(&damages, self.hit.death_damage_features);
        }
        if self.hit.death.handle == Some(target) {
            return if self
                .hit
                .death
                .deliver(&damages, self.hit.death_damage_features)
            {
                magicka_sim::character_impact::result::HIT
            } else {
                0
            };
        }
        if let Some(state) = self.hit.physics_damage.get_mut(&target) {
            let result = state.deliver(
                &damages,
                if self.hit.death_damage_features {
                    15
                } else {
                    14
                },
                self.hit.triggers,
            );
            if let Some(entity) = self.hit.entities.iter_mut().find(|e| e.handle == target) {
                entity.hit_points = state.health;
                entity.dead = state.health <= 0.0;
            }
            return result;
        }
        let view = self.hit.view;
        if let Some(npc) = self.hit.npcs.get(&target) {
            let body = npc.body;
            if let Some(a) = attacker.and_then(|a| view.get(a).copied()) {
                for d in damages.iter().filter(|d| d.attack != 0) {
                    crate::npc_combat::add_attacked_by(
                        self.hit.npcs,
                        view,
                        target,
                        &a,
                        d.amount * d.magnitude,
                    );
                }
            }
            let npc = self.hit.npcs.get_mut(&target).unwrap();
            let result = crate::avatar::deliver_to_body(
                &mut npc.character,
                &mut self.hit.bodies[body],
                &[magicka_sim::resistance::Resistance::IDENTITY; 11],
                false,
                target,
                payload,
                attacker,
                position,
                timestamp,
            );
            if result & magicka_sim::character_impact::result::DAMAGED != 0 && npc.on_damage != 0 {
                self.hit.triggers.push((npc.on_damage, Some(target)));
            }
            result
        } else if let Some(p) = self
            .hit
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == target)
        {
            let body = &mut self.hit.bodies[p.body];
            p.receive_payload(body, payload, attacker, position, timestamp)
        } else {
            0
        }
    }
}
impl magicka_sim::railgun::RailWorld for RailSceneWorld<'_, '_> {
    fn rail_entities(
        &mut self,
        center: [f32; 3],
        radius: f32,
    ) -> Vec<magicka_sim::railgun::RailEntity> {
        use magicka_sim::railgun::RailEntityKind as K;
        let view = self.hit.view;
        let mut out: Vec<_> = view
            .entities(center, radius, false)
            .into_iter()
            .map(|i| {
                let c = view.combatants[i];
                // Character.CurrentSelfShieldType == Shield reflects the ray.
                let kind = if self.self_shield(c.handle)
                    == Some(magicka_sim::character_health::SelfShieldType::Shield)
                {
                    K::SelfShielded
                } else {
                    K::Damageable { character: true }
                };
                magicka_sim::railgun::RailEntity {
                    handle: c.handle,
                    kind,
                    dead: c.dead,
                    position: c.position,
                }
            })
            .collect();
        if let Some(cache) = self.elemental_eggs.as_ref() {
            for slot in cache.slots.iter().filter(|s| s.actor.registered) {
                if within(
                    slot.actor.position,
                    center,
                    radius + slot.actor.geometry.radius,
                ) {
                    out.push(magicka_sim::railgun::RailEntity {
                        handle: slot.handle,
                        kind: K::Damageable { character: false },
                        dead: slot.actor.state.dead(),
                        position: slot.actor.position,
                    });
                }
            }
        }
        if self.portals.portal.connected {
            if let Some(handles) = self.portals.handles {
                for (p, handle) in [self.portals.portal.a, self.portals.portal.b]
                    .into_iter()
                    .zip(handles)
                {
                    if !p.dead && within(p.position, center, radius) {
                        out.push(magicka_sim::railgun::RailEntity {
                            handle,
                            kind: K::Portal,
                            dead: false,
                            position: p.position,
                        });
                    }
                }
            }
        }
        for (slot, mine) in self.mines.iter() {
            if within(
                mine.position,
                center,
                radius + magicka_sim::spell_mine::RADIUS,
            ) {
                out.push(magicka_sim::railgun::RailEntity {
                    handle: MINE_KEY | slot as u16,
                    kind: K::Damageable { character: false },
                    dead: mine.dead,
                    position: mine.position,
                });
            }
        }
        if let Some(pool) = self.hit.barriers.as_ref() {
            for (slot, barrier) in pool.iter().filter(|(_, b)| b.solid()) {
                if within(barrier.position, center, radius + barrier.radius) {
                    out.push(magicka_sim::railgun::RailEntity {
                        handle: BARRIER_KEY | slot as u16,
                        kind: K::Damageable { character: false },
                        dead: barrier.is_dead(),
                        position: barrier.position,
                    });
                }
            }
        }
        // Shield entities (after characters; native entity-list order is
        // INFERRED). Non-solid barriers and grease are skipped by beams.
        for s in self.hit.shields.iter() {
            let p = s.shield.position;
            let d = ((p[0] - center[0]).powi(2)
                + (p[1] - center[1]).powi(2)
                + (p[2] - center[2]).powi(2))
            .sqrt();
            if d <= radius + s.shield.radius {
                out.push(magicka_sim::railgun::RailEntity {
                    handle: s.handle,
                    kind: K::Shield,
                    dead: s.shield.dead,
                    position: p,
                });
            }
        }
        for (&handle, boss) in self.hit.jormungandr.iter() {
            if boss.update_animation && within(boss.frame.head, center, radius + 1.1) {
                out.push(magicka_sim::railgun::RailEntity {
                    handle,
                    kind: K::Damageable { character: false },
                    dead: boss.dead,
                    position: boss.frame.head,
                });
            }
        }
        if let Some(death) = self.hit.death.spray_entity() {
            if within(death.position, center, radius) {
                out.push(magicka_sim::railgun::RailEntity {
                    handle: death.handle,
                    kind: K::Damageable { character: false },
                    dead: false,
                    position: death.position,
                });
            }
        }
        for (&handle, d) in self.hit.dispensers.iter() {
            if within(d.position(), center, radius + 1.25) {
                out.push(magicka_sim::railgun::RailEntity {
                    handle,
                    kind: K::Damageable { character: false },
                    dead: d.health <= 0.0,
                    position: d.position(),
                });
            }
        }
        for entity in self.hit.entities.iter().filter(|e| e.added && !e.removed) {
            let Some(state) = self.hit.physics_damage.get(&entity.handle) else {
                continue;
            };
            if (0..3)
                .map(|i| (entity.position[i] - center[i]).powi(2))
                .sum::<f32>()
                .sqrt()
                <= radius + state.radius()
            {
                out.push(magicka_sim::railgun::RailEntity {
                    handle: entity.handle,
                    kind: K::Damageable { character: false },
                    dead: entity.dead || state.health <= 0.0,
                    position: entity.position,
                });
            }
        }
        for missile in self.missiles.iter().filter(|m| !m.body.dead) {
            if let Some(handle) = missile
                .handle
                .filter(|_| within(missile.body.position, center, radius + missile.body.radius))
            {
                out.push(magicka_sim::railgun::RailEntity {
                    handle,
                    kind: K::Damageable { character: false },
                    dead: false,
                    position: missile.body.position,
                });
            }
        }
        out
    }
    fn rail_segment(
        &mut self,
        handle: u16,
        seg: magicka_sim::spray_pulse::Segment,
        radius: f32,
    ) -> Option<[f32; 3]> {
        if handle & MINE_KEY == MINE_KEY {
            return self
                .mines
                .get((handle & !MINE_KEY) as usize)?
                .segment_intersect(seg);
        }
        if handle & MINE_KEY == BARRIER_KEY {
            return self
                .hit
                .barriers
                .as_ref()?
                .get((handle & !BARRIER_KEY) as usize)?
                .segment_intersect(seg, radius);
        }
        if let Some(slot) = self.elemental_eggs.as_ref().and_then(|cache| {
            cache
                .slots
                .iter()
                .find(|s| s.handle == handle && s.actor.registered)
        }) {
            return slot.actor.segment_intersect(seg, radius);
        }
        if let Some(handles) = self.portals.handles {
            if let Some(index) = handles.iter().position(|&h| h == handle) {
                let p = [self.portals.portal.a, self.portals.portal.b][index];
                let (start, _, r) = p.capsule();
                return magicka_sim::capsule_intersection::segment_fraction(
                    seg,
                    start,
                    [0., 1., 0.],
                    2. * magicka_sim::portal::HALF_LENGTH,
                    r,
                )
                .map(|t| std::array::from_fn(|i| seg.origin[i] + seg.delta[i] * t));
            }
        }
        if let Some(m) = self
            .missiles
            .iter()
            .find(|m| m.handle == Some(handle) && !m.body.dead)
        {
            return crate::missiles::segment_intersect(m.body.position, m.body.radius, seg, radius);
        }
        if let Some(d) = self.hit.dispensers.get(&handle) {
            return d.segment(seg, radius);
        }
        if let Some(boss) = self.hit.jormungandr.get(&handle) {
            return boss
                .segment_fraction(seg)
                .map(|t| std::array::from_fn(|i| seg.origin[i] + seg.delta[i] * t));
        }
        if self.hit.death.handle == Some(handle) {
            // Native MagickDeath.SegmentIntersect ignores the segment radius.
            return self
                .hit
                .death
                .segment_fraction(seg)
                .map(|t| std::array::from_fn(|i| seg.origin[i] + seg.delta[i] * t));
        }
        // Shield.SegmentIntersect: the same collision primitive as the skin
        // test (wall box or hollow sphere); the segment radius is unused.
        if self.hit.shields.iter().any(|s| s.handle == handle) {
            return magicka_sim::railgun::RailWorld::rail_skin(self, handle, seg)
                .map(|h| h.position);
        }
        if let Some(state) = self.hit.physics_damage.get(&handle) {
            if state.has_capsule() {
                return state
                    .death_segment(seg)
                    .map(|t| std::array::from_fn(|i| seg.origin[i] + seg.delta[i] * t));
            }
            let entity = self.hit.entities.iter().find(|e| e.handle == handle)?;
            let bounds = state.world_box(&entity.orientation)?;
            let (distance, fraction, _) = magicka_sim::capsule_box::segment_distance(seg, bounds);
            return (distance <= radius * radius)
                .then(|| std::array::from_fn(|i| seg.origin[i] + seg.delta[i] * fraction));
        }
        let c = self.hit.view.get(handle)?;
        magicka_sim::railgun::character_segment_intersect(
            c.position,
            [0.0, 1.0, 0.0],
            c.length,
            c.radius,
            c.ethereal,
            seg,
            radius,
        )
    }
    fn rail_portal_exit(&mut self, handle: u16) -> Option<(u16, [f32; 3])> {
        let handles = self.portals.handles?;
        if !self.portals.portal.connected {
            return None;
        }
        let index = handles.iter().position(|&h| h == handle)?;
        let other = 1 - index;
        Some((
            handles[other],
            [self.portals.portal.a, self.portals.portal.b][other].position,
        ))
    }
    /// Body.CollisionSkin.SegmentIntersect of a Shield entity (hollow
    /// sphere or wall box) or a self-shielded character's capsule.
    fn rail_skin(
        &mut self,
        handle: u16,
        seg: magicka_sim::spray_pulse::Segment,
    ) -> Option<magicka_sim::railgun::SurfaceHit> {
        use magicka_sim::shield_intersection::ShieldGeometry;
        let at = |t: f32| -> [f32; 3] { std::array::from_fn(|i| seg.origin[i] + seg.delta[i] * t) };
        let unit = |v: [f32; 3]| {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            if l > 0.0 {
                v.map(|x| x / l)
            } else {
                v
            }
        };
        if let Some(boss) = self.hit.jormungandr.get(&handle) {
            let fraction = boss.segment_fraction(seg)?;
            let position = at(fraction);
            return Some(magicka_sim::railgun::SurfaceHit {
                fraction,
                position,
                normal: unit(std::array::from_fn(|i| position[i] - boss.frame.head[i])),
            });
        }
        if let Some(d) = self.hit.dispensers.get(&handle) {
            let axis = d.axis();
            let fraction = magicka_sim::capsule_intersection::segment_fraction(
                seg,
                axis.origin,
                axis.delta,
                1.0,
                1.25,
            )?;
            let position = at(fraction);
            let t = (0..3)
                .map(|i| (position[i] - axis.origin[i]) * axis.delta[i])
                .sum::<f32>()
                .clamp(0.0, 1.0);
            return Some(magicka_sim::railgun::SurfaceHit {
                fraction,
                position,
                normal: unit(std::array::from_fn(|i| {
                    position[i] - axis.origin[i] - axis.delta[i] * t
                })),
            });
        }
        if let Some(s) = self.hit.shields.iter().find(|s| s.handle == handle) {
            return match s.shield.geometry() {
                ShieldGeometry::Wall(wall) => {
                    wall.segment_intersect(seg)
                        .map(|h| magicka_sim::railgun::SurfaceHit {
                            fraction: h.fraction,
                            position: h.position,
                            normal: h.normal,
                        })
                }
                ShieldGeometry::Sphere(sphere) => sphere.segment_fraction(seg).map(|t| {
                    let position = at(t);
                    magicka_sim::railgun::SurfaceHit {
                        fraction: t,
                        position,
                        normal: unit(std::array::from_fn(|i| position[i] - sphere.center[i])),
                    }
                }),
            };
        }
        let c = *self.hit.view.get(handle)?;
        let point = magicka_sim::railgun::character_segment_intersect(
            c.position,
            [0.0, 1.0, 0.0],
            c.length,
            c.radius,
            false,
            seg,
            0.0,
        )?;
        let dd = seg.delta.iter().map(|v| v * v).sum::<f32>();
        let fraction = (0..3)
            .map(|i| (point[i] - seg.origin[i]) * seg.delta[i])
            .sum::<f32>()
            / dd;
        // Capsule normal: from the closest point on the vertical axis.
        let half = 0.5 * c.length;
        let y = point[1].clamp(c.position[1] - half, c.position[1] + half);
        Some(magicka_sim::railgun::SurfaceHit {
            fraction,
            position: point,
            normal: unit([
                point[0] - c.position[0],
                point[1] - y,
                point[2] - c.position[2],
            ]),
        })
    }
    fn rail_scene(
        &mut self,
        seg: magicka_sim::spray_pulse::Segment,
    ) -> Option<magicka_sim::railgun::SceneRayHit> {
        self.scene
            .segment_intersect(seg, false)
            .map(|h| magicka_sim::railgun::SceneRayHit {
                hit: magicka_sim::railgun::SurfaceHit {
                    fraction: h.hit.fraction,
                    position: h.hit.position,
                    normal: h.hit.normal,
                },
                material: h.primitive,
            })
    }
    fn rail_damage(
        &mut self,
        target: u16,
        attacker: u16,
        damages: [magicka_sim::damage::Damage; 5],
        timestamp: f64,
        position: [f32; 3],
    ) -> u16 {
        self.deliver_collection(target, Some(attacker), damages, timestamp, position)
    }
    fn rail_character_state(&mut self, target: u16) -> Option<(f32, bool)> {
        let bloating = self.bloating.contains(&target);
        if let Some(n) = self.hit.npcs.get(&target) {
            return Some((n.character.combat.health.hit_points, bloating));
        }
        self.hit
            .players
            .iter()
            .flatten()
            .find(|p| p.handle == target)
            .map(|p| (p.character.combat.health.hit_points, bloating))
    }
    fn rail_accumulate(&mut self, _: u16, _: u32, _: f32) {
        // mBloatDamageAccumulation is only decayed by the original; no
        // reader exists, so the accumulation is not stored.
    }
    fn rail_bloat_kill(&mut self, target: u16, element: u32, killer: u16) {
        self.bloat_requests.push((target, element, killer));
        self.bloating.insert(target);
    }
    fn rail_blast(
        &mut self,
        owner: Option<u16>,
        timestamp: f64,
        radius: f32,
        position: [f32; 3],
        damages: [magicka_sim::damage::Damage; 5],
    ) -> u16 {
        self.collection_blast(owner, None, timestamp, radius, position, damages)
    }
}

impl RailSceneWorld<'_, '_> {
    fn collection_blast(
        &mut self,
        owner: Option<u16>,
        ignore: Option<u16>,
        timestamp: f64,
        radius: f32,
        position: [f32; 3],
        damages: [magicka_sim::damage::Damage; 5],
    ) -> u16 {
        self.hit
            .entity_blasts
            .push(EntityBlast::Full(position, radius, damages));
        let all = damages.iter().fold(0u32, |a, d| a | d.element);
        if all & magicka_sim::spells::Elements::BEAMS.0 != 0 {
            self.blasts.push((position, radius, all));
        } else {
            self.collection_visuals
                .push(crate::missiles::MissileVisual::Blast {
                    position,
                    radius,
                    damages,
                });
        }
        let view = self.hit.view;
        let targets: Vec<_> = view
            .entities(position, radius, true)
            .into_iter()
            .map(|i| view.combatants[i])
            .collect();
        let mut result = 0;
        for c in targets {
            if ignore == Some(c.handle) || c.ethereal || c.dead {
                continue;
            }
            let delta = crate::npc_combat::blast_target_delta(&c, position);
            if view.scene_hit(position, delta) {
                continue;
            }
            let distance =
                (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt() - c.radius;
            if distance > radius {
                continue;
            }
            let d = magicka_sim::blast_damage::collection(damages, distance, radius);
            result |= self.deliver_collection(c.handle, owner, d, timestamp, position);
        }
        result
    }
}

/// A collection FullBlast (position, radius, damages).
enum EntityBlast {
    Full([f32; 3], f32, [magicka_sim::damage::Damage; 5]),
    Splash {
        position: [f32; 3],
        radius: f32,
        damage: magicka_sim::damage::Damage,
        owner: u16,
        ignore: Option<u16>,
    },
}

/// Barrier presentation requests (research/barrier.md).
#[derive(Clone, Debug, PartialEq)]
pub enum BarrierVisual {
    /// A persistent effect owned by the barrier in `slot` (particle wall or
    /// geyser/volcano), stopped by `Stop` or the barrier's removal.
    Start {
        slot: usize,
        name: &'static str,
        transform: [f32; 16],
    },
    Update {
        slot: usize,
        transform: [f32; 16],
    },
    Stop {
        slot: usize,
    },
    OneShot {
        name: &'static str,
        position: [f32; 3],
        direction: [f32; 3],
    },
    Bolt(magicka_sim::lightning::BoltVisual),
}

/// Barrier particle-wall matrix: Forward = direction, Up = Y, Right =
/// Forward × Up, uniformly scaled, at `position` (XNA row layout).
fn barrier_wall_matrix(direction: [f32; 3], scale: f32, position: [f32; 3]) -> [f32; 16] {
    let up = [0.0, 1.0, 0.0];
    let right = [
        direction[1] * up[2] - direction[2] * up[1],
        direction[2] * up[0] - direction[0] * up[2],
        direction[0] * up[1] - direction[1] * up[0],
    ];
    let mut m = magicka_content::skinned::IDENTITY;
    for i in 0..3 {
        m[i] = right[i] * scale;
        m[4 + i] = up[i] * scale;
        m[8 + i] = -direction[i] * scale;
        m[12 + i] = position[i];
    }
    m
}

/// Hit-list keys for non-character barrier victims (character handles are
/// u16 entity handles; these spaces do not collide in practice).
const BARRIER_KEY: u16 = 0x8000;
const MINE_KEY: u16 = 0xC000;
/// Grease fields as IDamageable entities (`handle & MINE_KEY == GREASE_KEY`).
const GREASE_KEY: u16 = 0x4000;

/// LightningBolt target overload over the scene's characters (Barrier
/// contacts). Barriers, mines and shields are not lightning targets here.
struct BarrierLightning<'w, 'a, 'v> {
    world: &'w mut RailSceneWorld<'a, 'v>,
    bolts: &'w mut Vec<BarrierVisual>,
    rng: &'w mut u64,
}
impl magicka_sim::lightning::LightningScene for BarrierLightning<'_, '_, '_> {
    fn lightning_targets(
        &mut self,
        center: [f32; 3],
        range: f32,
    ) -> Vec<magicka_sim::lightning::LightningTarget> {
        let view = self.world.hit.view;
        view.entities(center, range, true)
            .into_iter()
            .map(|i| {
                let c = view.combatants[i];
                magicka_sim::lightning::LightningTarget {
                    handle: c.handle,
                    position: c.position,
                    dead: c.dead,
                    eligible: !c.ethereal,
                    frozen: c.status_mask & magicka_sim::status::Status::Frozen as u16 != 0,
                    ends_chain: false,
                }
            })
            .collect()
    }
    fn lightning_shielded(&mut self, _: Segment) -> bool {
        false
    }
    fn lightning_scene(&mut self, segment: Segment) -> Option<f32> {
        self.world
            .scene
            .segment_intersect(segment, false)
            .map(|h| h.hit.fraction)
    }
    fn lightning_damage(
        &mut self,
        target: u16,
        attacker: u16,
        damages: [magicka_sim::damage::Damage; 5],
        timestamp: f64,
        position: [f32; 3],
    ) {
        let attacker = (attacker != u16::MAX).then_some(attacker);
        self.world
            .deliver_collection(target, attacker, damages, timestamp, position);
    }
}
impl magicka_sim::lightning::LightningWorld for BarrierLightning<'_, '_, '_> {
    fn lightning_bolt(&mut self, bolt: magicka_sim::lightning::BoltVisual) {
        self.bolts.push(BarrierVisual::Bolt(bolt));
    }
    fn caster_range_modifier(&self, caster: u16) -> (f32, f32) {
        let hit = &self.world.hit;
        hit.players
            .iter()
            .flatten()
            .find(|p| p.handle == caster)
            .map(|p| &p.character)
            .or_else(|| hit.npcs.get(&caster).map(|n| &n.character))
            .map_or((1.0, 0.0), |c| c.buffs.general.spell_range_modifier())
    }
    fn next_double(&mut self) -> f64 {
        *self.rng ^= *self.rng << 13;
        *self.rng ^= *self.rng >> 7;
        *self.rng ^= *self.rng << 17;
        (*self.rng >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn within(a: [f32; 3], b: [f32; 3], reach: f32) -> bool {
    let d: f32 = (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum();
    d <= reach * reach
}

/// UnderGroundAttack presentation requests.
#[derive(Clone, Debug, PartialEq)]
pub enum UndergroundVisual {
    Start {
        id: u32,
        effects: Vec<&'static str>,
        position: [f32; 3],
        direction: [f32; 3],
    },
    Move {
        id: u32,
        position: [f32; 3],
        direction: [f32; 3],
    },
    Stop {
        id: u32,
    },
}

/// Deferred UnderGroundAttack damage: target, attacker, damages, time, position.
type UndergroundDamage = (
    u16,
    Option<u16>,
    [magicka_sim::damage::Damage; 5],
    f64,
    [f32; 3],
);

/// UndergroundWorld over the scene: ground/wall queries, shields, and
/// damageable characters and shields; damage is deferred to the caller.
struct UndergroundHost<'a> {
    scene: &'a crate::level_collision::SceneCollision,
    shields: Vec<(
        u16,
        magicka_sim::shield_intersection::ShieldGeometry,
        [f32; 3],
        f32,
    )>,
    characters: Vec<(u16, [f32; 3], f32)>,
    owner: Option<u16>,
    visuals: Vec<UndergroundVisual>,
    damage: Vec<UndergroundDamage>,
}
impl<'a> UndergroundHost<'a> {
    fn new<C: SceneContent>(world: &'a SceneWorld<C>) -> Self {
        Self {
            scene: world.collision.collision(),
            shields: world
                .shields
                .iter()
                .filter(|s| !s.shield.dead)
                .map(|s| {
                    (
                        s.handle,
                        s.shield.geometry(),
                        s.shield.position,
                        s.shield.radius,
                    )
                })
                .collect(),
            characters: world
                .combatants()
                .into_iter()
                .filter(|c| !c.dead)
                .map(|c| (c.handle, c.position, c.radius))
                .collect(),
            owner: None,
            visuals: Vec::new(),
            damage: Vec::new(),
        }
    }
}
impl magicka_sim::underground_attack::UndergroundWorld for UndergroundHost<'_> {
    fn ground(&mut self, seg: magicka_sim::spray_pulse::Segment) -> Option<([f32; 3], [f32; 3])> {
        self.scene
            .segment_intersect(seg, false)
            .map(|h| (h.hit.position, h.hit.normal))
    }
    fn shield_blocks(&mut self, seg: magicka_sim::spray_pulse::Segment) -> bool {
        self.shields
            .iter()
            .any(|(_, g, _, _)| g.segment_fraction(seg).is_some())
    }
    fn damageables(&mut self, center: [f32; 3], radius: f32) -> Vec<u16> {
        let near = |p: [f32; 3], r: f32| {
            ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) + (p[2] - center[2]).powi(2))
                .sqrt()
                - r
                <= radius
        };
        self.characters
            .iter()
            .filter(|(_, p, r)| near(*p, *r))
            .map(|(h, ..)| *h)
            .chain(
                self.shields
                    .iter()
                    .filter(|(_, _, p, r)| near(*p, *r))
                    .map(|(h, ..)| *h),
            )
            .collect()
    }
    fn damage(
        &mut self,
        target: u16,
        damages: [magicka_sim::damage::Damage; 5],
        timestamp: f64,
        position: [f32; 3],
    ) {
        self.damage
            .push((target, self.owner, damages, timestamp, position));
    }
}

impl<C: SceneContent> SceneWorld<C> {
    /// Network client: create a display copy of a host pickup that this
    /// world's (disabled) scripts never spawned. Returns the local handle.
    pub fn mirror_pickup(
        &mut self,
        kind: &magicka_network::world::PickupKind,
        transform: [f32; 16],
    ) -> Result<u16, WorldError> {
        use crate::magick_books::SpawnMagickHost;
        use crate::spawn_item::SpawnItemHost;
        use magicka_network::world::PickupKind;
        match kind {
            PickupKind::Book { magick } => self.spawn_book(
                &crate::magick_books::SpawnMagickAction {
                    delay: 0.0,
                    area: 0,
                    magick: *magick,
                    immovable: true,
                    timeout: 0.0,
                    unique_id: 0,
                    on_pickup: 0,
                },
                transform,
            ),
            PickupKind::Item { name } => {
                let handle = self.new_placeholder()?;
                self.load_item(&format!("Data/Items/Wizard/{name}"))?;
                self.copy_and_detach(handle, custom_hash(name), 0)?;
                self.move_to(
                    handle,
                    [transform[12], transform[13], transform[14]],
                    transform,
                )?;
                self.entity_mut(handle)?.added = true;
                Ok(handle)
            }
        }
    }
}

impl<C: SceneContent> crate::spawn_item::SpawnItemHost for SceneWorld<C> {
    type Error = WorldError;
    fn is_client(&self) -> bool {
        self.role == NetworkRole::Client
    }
    fn is_server(&self) -> bool {
        self.role == NetworkRole::Server
    }
    fn new_placeholder(&mut self) -> Result<u16, WorldError> {
        let handle = self.allocate_physics(false);
        self.entity_mut(handle)?.kind = EntityKind::Item;
        self.item_pickups.insert(handle, Default::default());
        Ok(handle)
    }
    fn load_item(&mut self, path: &str) -> Result<(), WorldError> {
        let name = path
            .strip_prefix("Data/Items/Wizard/")
            .ok_or("invalid item content path")?;
        let item = self
            .content
            .item(name)
            .ok_or_else(|| format!("missing item template {name}"))?;
        self.item_templates.insert(custom_hash(name), item);
        Ok(())
    }
    fn locator(&mut self, area: i32) -> Result<[f32; 16], WorldError> {
        self.scene_locator(area)
    }
    fn copy_and_detach(
        &mut self,
        handle: u16,
        template: i32,
        on_pickup: i32,
    ) -> Result<(), WorldError> {
        let definition = self
            .item_templates
            .get(&template)
            .ok_or("item template not cached")?
            .clone();
        let ranged = self.content.player_ranged_item(&definition.name);
        let mut item = crate::avatar::AvatarItem::new(definition);
        item.melee.ranged = ranged;
        let pickup = self
            .item_pickups
            .get_mut(&handle)
            .ok_or("missing item placeholder")?;
        pickup.item = Some(item);
        pickup.item.as_mut().unwrap().attached = false;
        pickup.generation = pickup.generation.wrapping_add(1);
        pickup.on_pickup = on_pickup;
        pickup.animated_part = None;
        pickup.despawn = 0.0;
        self.entity_mut(handle)?.removed = false;
        self.entity_mut(handle)?.dead = false;
        Ok(())
    }
    fn move_to(
        &mut self,
        handle: u16,
        position: [f32; 3],
        mut orientation: [f32; 16],
    ) -> Result<(), WorldError> {
        orientation[12..15].copy_from_slice(&position);
        let pickup = self
            .item_pickups
            .get_mut(&handle)
            .ok_or("missing item placeholder")?;
        pickup.transform = orientation;
        let entity = self.entity_mut(handle)?;
        entity.position = position;
        entity.orientation = orientation;
        Ok(())
    }
    fn add_entity(&mut self, handle: u16) -> Result<(), WorldError> {
        self.entity_mut(handle)?.added = true;
        self.log.push(format!("spawnitem {handle}"));
        Ok(())
    }
    fn configure(
        &mut self,
        handle: u16,
        immovable: bool,
        ignore_tractor_pull: bool,
        unique_id: Option<i32>,
    ) -> Result<(), WorldError> {
        let pickup = self
            .item_pickups
            .get_mut(&handle)
            .ok_or("missing item placeholder")?;
        pickup.immovable = immovable;
        pickup.ignore_tractor_pull = ignore_tractor_pull;
        if let Some(id) = unique_id {
            self.entity_mut(handle)?.unique_id = id;
        }
        Ok(())
    }
    fn enable_body(&mut self, handle: u16) -> Result<(), WorldError> {
        self.item_pickups
            .get_mut(&handle)
            .ok_or("missing item placeholder")?
            .body_enabled = true;
        Ok(())
    }
    fn attach_if_present(&mut self, handle: u16, part: i32) -> Result<(), WorldError> {
        fn exists(parts: &[crate::animated_hierarchy::AnimatedPartState], id: i32) -> bool {
            parts
                .iter()
                .any(|p| custom_hash(&p.name) == id || exists(&p.children, id))
        }
        if exists(self.collision.parts(), part) {
            self.item_pickups
                .get_mut(&handle)
                .ok_or("missing item placeholder")?
                .animated_part = Some(part);
        }
        Ok(())
    }
    fn send_spawn(
        &mut self,
        message: crate::spawn_item::SpawnItemMessage,
    ) -> Result<(), WorldError> {
        self.log.push(format!("spawnitem-message {message:?}"));
        Ok(())
    }
}

impl<C: SceneContent> crate::magick_books::SpawnMagickHost for SceneWorld<C> {
    type Error = WorldError;
    fn role(&self) -> NetworkRole {
        self.role
    }
    fn locator(&mut self, area: i32) -> Result<[f32; 16], WorldError> {
        self.scene_locator(area)
    }
    fn spawn_book(
        &mut self,
        a: &crate::magick_books::SpawnMagickAction,
        t: [f32; 16],
    ) -> Result<u16, WorldError> {
        if a.magick as usize >= crate::magick_books::BOOK_MAGICKS.len()
            || a.magick == crate::magick_books::SENTINEL
        {
            return Err("invalid book magick".into());
        }
        let cached = self
            .book_pickups
            .iter()
            .find(|(_, b)| b.dead)
            .map(|(&h, b)| (h, b.generation + 1));
        let (handle, generation) = cached.unwrap_or_else(|| (self.allocate_physics(false), 0));
        let e = self.entity_mut(handle)?;
        e.kind = EntityKind::Item;
        e.added = true;
        e.removed = false;
        e.dead = false;
        e.unique_id = a.unique_id;
        e.position = [t[12], t[13], t[14]];
        e.orientation = t;
        self.book_pickups.insert(
            handle,
            crate::magick_books::Book {
                magick: a.magick,
                transform: t,
                immovable: a.immovable,
                body_enabled: !a.immovable,
                velocity: [0.0; 3],
                timeout: a.timeout,
                resting_timer: 1.0,
                on_pickup: a.on_pickup,
                dead: false,
                generation,
            },
        );
        self.log.push(format!("spawnmagick {handle}"));
        Ok(handle)
    }
    fn send_book_spawn(
        &mut self,
        h: u16,
        a: &crate::magick_books::SpawnMagickAction,
        t: [f32; 16],
    ) -> Result<(), WorldError> {
        self.log
            .push(format!("spawnmagick-message {h} {a:?} {t:?}"));
        Ok(())
    }
}

/// Distance.PointSegmentDistanceSq.
fn point_segment_distance_sq(p: [f32; 3], s: Segment) -> f32 {
    let dd: f32 = s.delta.iter().map(|v| v * v).sum();
    let t = if dd > 0.0 {
        ((0..3)
            .map(|i| (p[i] - s.origin[i]) * s.delta[i])
            .sum::<f32>()
            / dd)
            .clamp(0.0, 1.0)
    } else {
        0.0
    };
    (0..3)
        .map(|i| (s.origin[i] + s.delta[i] * t - p[i]).powi(2))
        .sum()
}

#[cfg(test)]
mod beam_target_tests;

/// Mod-script entry points (rewrite extension). They reuse the native
/// delivery the Damage script action uses, so resistances, shields'
/// absence, death and NPC damage triggers behave the same.
impl<C: SceneContent> SceneWorld<C> {
    /// Deliver one damage entry to a character. Returns false when `target`
    /// is not a live character.
    pub fn mod_damage(
        &mut self,
        target: u16,
        damage: magicka_sim::damage::Damage,
        from: [f32; 3],
    ) -> bool {
        use magicka_sim::damage_delivery::DamagePayload;
        let Some(body) = self.character_body(target) else {
            return false;
        };
        if let Some(player) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == target)
        {
            player.receive_payload(
                &mut self.physics.bodies[body],
                DamagePayload::Single(damage),
                None,
                from,
                0.,
            );
            return true;
        }
        let Some(npc) = self.npcs.get_mut(&target) else {
            return false;
        };
        let result = crate::avatar::deliver_to_body(
            &mut npc.character,
            &mut self.physics.bodies[body],
            &[magicka_sim::resistance::Resistance::IDENTITY; 11],
            false,
            target,
            DamagePayload::Single(damage),
            None,
            from,
            0.,
        );
        if result & magicka_sim::character_impact::result::DAMAGED != 0 && npc.on_damage != 0 {
            self.pending_triggers.push((npc.on_damage, Some(target)));
        }
        true
    }
    /// Restore up to `amount` health to a living character.
    pub fn mod_heal(&mut self, target: u16, amount: f32) -> bool {
        let (health, body) = if let Some(npc) = self.npcs.get_mut(&target).filter(|n| !n.dead()) {
            (&mut npc.character.combat.health, npc.body)
        } else if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == target && !p.dead())
        {
            (&mut p.character.combat.health, p.body)
        } else {
            return false;
        };
        health.hit_points = (health.hit_points + amount.max(0.0)).min(health.max_hit_points);
        let hp = health.hit_points;
        self.physics.bodies[body].owner.hit_points = hp;
        true
    }
    /// The body index of a character handle (players and NPCs).
    pub fn mod_character_body(&self, handle: u16) -> Option<usize> {
        self.character_body(handle)
    }
}
