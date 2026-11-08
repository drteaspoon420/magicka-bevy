//! Content-to-simulation adapters. No rendering dependency or target execution.
//! See research/runtime-templates.md for recovered loading behavior and gaps.
// Bevy systems and reconstructed native methods legitimately take many
// parameters and complex query types (Bevy's own recommended allows).
#![allow(clippy::too_many_arguments, clippy::type_complexity)]
pub mod anim_prop;
pub mod animation_effects;
pub mod arcane_blade;
pub mod arcane_blast;
pub mod blast_visual;
pub mod boss_assatur;
pub mod boss_cthulhu;
pub mod boss_death;
pub mod boss_fafnir;
pub mod boss_grimnir;
pub mod boss_grimnir2;
pub mod boss_health;
pub mod boss_machine;
pub mod boss_parker;
pub mod boss_rig;
pub mod boss_scale;
pub mod boss_vlad;
pub mod boss_warlord;
pub mod buffs;
pub mod character;
pub mod character_contact;
pub mod character_sounds;
pub mod credits;
pub mod cutscene;
pub mod decals;
pub mod dispenser;
pub mod elemental_egg_actor;
pub mod fixed_tick;
pub mod floating_text;
pub mod frontend_cutscene;
pub mod frontend_intro;
pub mod frontend_loading;
pub mod frontend_tome;
pub mod generic_aura;
pub mod gibs;
pub mod global_settings;
pub mod grip;
pub mod hud;
pub mod inventory_box;
pub mod item_effects;
pub mod item_special;
pub mod jormungandr;
pub mod keyboard_hud;
pub mod level_effects;
pub mod light_variation;
pub mod main_menu;
pub mod npc_spellcasting;
pub mod render_transition;
pub mod ruleset;
pub mod size_magicks;
pub mod slots;
pub mod spell_lights;
pub mod spray_decals;
pub mod statistics;
pub mod status_visuals;
pub mod summon_death_actor;
pub mod water_collision;
pub mod zap_skeleton;

use magicka_content::{character::CharacterTemplate, common, item::Item};
use magicka_sim::{
    character_health::StatusFrame, character_impact::ImpactCharacter, resistance::Resistance,
    spells::Elements,
};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateError {
    InvalidResistanceElement(i32),
    /// CLR invariant casing for non-ASCII IDs has not been reconstructed.
    UnsupportedIdentifier(String),
}

/// Expand sparse stored rows into the game's eleven-slot table. Duplicate rows
/// overwrite earlier rows. A composite mask indexes round(log2(mask)), not each bit.
pub fn resistance_table(rows: &[common::Resistance]) -> Result<[Resistance; 11], TemplateError> {
    let mut out = std::array::from_fn(|i| Resistance {
        against: Elements(1 << i),
        ..Resistance::IDENTITY
    });
    for row in rows {
        // Defines.ElementIndex and Spell.ElementIndex both produce out-of-range
        // indices for All/zero/negative masks. Reject instead of panicking.
        let index = if row.element > 0 {
            ((row.element as f64).ln() * (1.0 / 2f64.ln()) + 0.5) as usize
        } else {
            usize::MAX
        };
        if index >= 11 {
            return Err(TemplateError::InvalidResistanceElement(row.element));
        }
        out[index] = Resistance {
            against: Elements(row.element as u32),
            multiplier: row.multiplier,
            modifier: row.modifier,
            status_resistance: row.status_resistance,
        };
    }
    Ok(out)
}

/// Lowest resistance factor (rewrite extension). The native line reaches this
/// near 136 players, then zero near 151 and negative beyond, where damage would
/// heal enemies. The original's limit of four players never got close.
pub const MIN_RESISTANCE_FACTOR: f32 = 0.1;

/// CharacterTemplate.Read's non-friendly load-time scaling, including defaults.
/// Apply once to a freshly expanded table, not every spawn or update.
pub fn scale_character_resistances(table: &mut [Resistance; 11], faction: i32, player_count: i32) {
    if faction & 4 != 0 {
        return;
    }
    let factor = (1.0 - (player_count as f32 - 1.0) * 0.00666).max(MIN_RESISTANCE_FACTOR);
    for r in table {
        if r.multiplier < 0.0 {
            r.multiplier /= factor;
        } else {
            r.multiplier *= factor;
        }
        if r.modifier < 0.0 {
            r.modifier /= factor;
        } else {
            r.modifier *= factor;
        }
    }
}

#[derive(Clone, Debug)]
pub struct CharacterDefinition {
    pub id: String,
    pub faction: i32,
    pub ethereal: bool,
    /// Template LooksEthereal persists after Corporealize.
    pub looks_ethereal: bool,
    pub can_have_status: bool,
    pub resistances: [Resistance; 11],
    pub max_hit_points: f32,
    pub health_bars: i32,
    pub event_conditions: Arc<Vec<magicka_content::common::EventCollection>>,
    pub hit_tolerance: f32,
    pub knockdown_tolerance: f32,
    pub max_panic: f32,
    /// CharacterTemplate.IsFearless (Character.Fear ignores it).
    pub fearless: bool,
    /// CharacterTemplate.IsUncharmable (Character.Charm ignores it).
    pub uncharmable: bool,
    /// CharacterTemplate.IsNonslippery: grease fields do not affect it.
    pub nonslippery: bool,
    pub length: f32,
    pub radius: f32,
    pub mass: f32,
    pub speed: f32,
    pub turn_speed: f32,
    pub regeneration: i32,
    pub bleed_rate: f32,
    pub stun_time: f32,
    /// Template gib count (Character.HasGibs: any gibs).
    pub gib_count: usize,
    /// Gibs with loadable models and bounds (filled by the content loader).
    pub gibs: Vec<gibs::GibDefinition>,
    /// CharacterTemplate BloodType (5 = none).
    pub blood: i32,
    /// CharacterTemplate.AttachedSounds (at most four), started at Initialize.
    pub attached_sounds: Vec<character_sounds::AttachedSound>,
    /// CharacterTemplate.ZapModifier (speed/animation while zapped).
    pub zap_modifier: f32,
    /// CharacterTemplate.BreakFreeStrength (BreakFree's counter step).
    pub break_free_strength: f32,
    /// CharacterTemplate.Undying / UndieTime / UndieHitPoints.
    pub undying: bool,
    pub undie_time: f32,
    pub undie_hit_points: f32,
    /// CharacterTemplate.Models[i].Tint: Character.Initialize copies the
    /// chosen model's tint into mMaterial.TintColor (Character.Color).
    pub model_tints: Vec<[f32; 3]>,
    /// CharacterTemplate.ScoreValue (survival scoring).
    pub score_value: i32,
    /// CharacterTemplate.DisplayName (a localization key; may be empty).
    pub display_id: String,
}
impl CharacterDefinition {
    pub fn from_content(
        template: &CharacterTemplate,
        player_count: i32,
    ) -> Result<Self, TemplateError> {
        if !template.id.is_ascii() {
            return Err(TemplateError::UnsupportedIdentifier(template.id.clone()));
        }
        let id = template.id.to_ascii_lowercase();
        let mut resistances = resistance_table(&template.resistances)?;
        scale_character_resistances(&mut resistances, template.faction, player_count);
        Ok(Self {
            can_have_status: !id.contains("elemental"),
            id,
            faction: template.faction,
            ethereal: template.is_ethereal,
            looks_ethereal: template.looks_ethereal,
            resistances,
            max_hit_points: template.max_hitpoints,
            score_value: template.score_value,
            display_id: template.display_id.clone(),
            health_bars: template.number_of_health_bars.max(1),
            event_conditions: Arc::new(template.event_conditions.0.clone()),
            hit_tolerance: template.hit_tolerance as f32,
            knockdown_tolerance: template.knockdown_tolerance,
            max_panic: template.max_panic,
            fearless: template.fearless,
            uncharmable: template.uncharmable,
            nonslippery: template.nonslippery,
            length: if template.length > 0.01 || template.length.is_nan() {
                template.length
            } else {
                0.01
            },
            radius: template.radius,
            mass: template.mass,
            speed: template.speed,
            turn_speed: template.turn_speed,
            regeneration: template.regeneration,
            bleed_rate: template.bleed_rate,
            stun_time: template.stun_time,
            gib_count: template.gibs.len(),
            gibs: Vec::new(),
            blood: template.blood,
            attached_sounds: character_sounds::attached_sounds(template),
            zap_modifier: template.zap_modifier,
            break_free_strength: template.break_free_strength,
            undying: template.undying,
            undie_time: template.undie_time,
            undie_hit_points: template.undie_hitpoints,
            model_tints: template.models.iter().map(|m| m.tint).collect(),
        })
    }
    /// Populate the recovered combat fields. This is not full Character.Initialize:
    /// spawning physics, equipment, buffs, animation and world placement are pending.
    pub fn combat_state(&self) -> ImpactCharacter {
        let mut state = ImpactCharacter {
            time_since_last_damage: f32::MAX,
            ..Default::default()
        };
        state.health.hit_points = self.max_hit_points;
        state.health.max_hit_points = self.max_hit_points;
        state.motion.knockdown_tolerance = self.knockdown_tolerance;
        state
    }
    /// Template-owned status frame values; speed is supplied from the live body.
    pub fn status_frame(&self, speed_multiplier: f32) -> StatusFrame {
        StatusFrame {
            max_panic: self.max_panic,
            speed_multiplier,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug)]
pub struct ItemDefinition {
    pub block_value: i32,
    pub resistances: [Resistance; 11],
    pub passive_ability: u8,
    pub passive_value: f32,
}
impl ItemDefinition {
    pub fn from_content(item: &Item) -> Result<Self, TemplateError> {
        Ok(Self {
            block_value: item.block_value,
            resistances: resistance_table(&item.resistances)?,
            passive_ability: item.passive_ability,
            passive_value: item.passive_value,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(element: i32, multiplier: f32, modifier: f32) -> common::Resistance {
        common::Resistance {
            element,
            multiplier,
            modifier,
            status_resistance: false,
        }
    }
    #[test]
    fn sparse_defaults_composite_index_and_duplicate_overwrite() {
        let table =
            resistance_table(&[row(3, 0.5, -4.0), row(8, 0.0, -1.0), row(8, 2.0, 5.0)]).unwrap();
        assert_eq!(table[0].multiplier, 1.0);
        assert_eq!(table[2].against, Elements(3));
        assert_eq!(table[2].multiplier, 0.5);
        assert_eq!(table[3].multiplier, 2.0);
        assert_eq!(table[3].modifier, 5.0);
        for (i, r) in resistance_table(&[]).unwrap().iter().enumerate() {
            assert_eq!(r.against, Elements(1 << i));
        }
    }
    #[test]
    fn scaling_keeps_the_native_line_and_never_inverts_at_large_player_counts() {
        let scaled = |players| {
            let mut table = resistance_table(&[row(8, -2.0, -10.0)]).unwrap();
            scale_character_resistances(&mut table, 1, players);
            table
        };
        let native = 1.0 - 135.0 * 0.00666;
        assert_eq!(scaled(136)[0].multiplier, native, "native up to the floor");
        for players in [137, 151, 152, 200, 256] {
            let table = scaled(players);
            assert_eq!(table[0].multiplier, MIN_RESISTANCE_FACTOR);
            assert!((table[3].multiplier - -20.0).abs() < 1e-4, "{players}");
        }
    }

    #[test]
    fn scaling_uses_division_for_negatives_and_scales_default_slots() {
        let mut table = resistance_table(&[row(8, -2.0, -10.0), row(4, 2.0, 10.0)]).unwrap();
        scale_character_resistances(&mut table, 1, 4);
        let factor = 1.0 - 3.0 * 0.00666;
        assert_eq!(table[0].multiplier, factor);
        assert_eq!(table[3].multiplier, -2.0 / factor);
        assert_eq!(table[3].modifier, -10.0 / factor);
        assert_eq!(table[2].multiplier, 2.0 * factor);
        assert_eq!(table[2].modifier, 10.0 * factor);
        let before = table;
        scale_character_resistances(&mut table, 0xff, 4);
        assert_eq!(table, before);
    }
    #[test]
    fn invalid_indices_are_errors() {
        for e in [0, -1, 0xffff, 2048] {
            assert!(resistance_table(&[row(e, 1.0, 0.0)]).is_err());
        }
    }
}

pub mod self_cast;

pub mod item_definitions;

pub mod mod_levels;
pub mod mod_presentation;
pub mod mod_runtime;
pub mod mod_ui;
pub mod mod_world;

pub mod level_collision;

pub mod force_field;

pub mod scene_loading;

pub mod animated_pose;

pub mod animated_hierarchy;

pub mod live_collision;

pub mod spray_world;

pub mod afterimage;
pub mod animation_action;

pub mod action_queue;

pub mod trigger;

pub mod level_variables;
pub mod script_condition;

pub mod script_attributes;

pub mod ai_event;
pub mod animation_controller;
pub mod audio;
pub mod audio_action;
pub mod avatar;
pub mod avatar_ai;
pub mod avatar_move;
pub mod beam_mesh;
pub mod bitmap_font;
pub mod bolt_mesh;
pub mod boss_action;
pub mod camera;
pub mod campaign_action;
pub mod chant_spells;
pub mod character_collision;
pub mod character_motion;
pub mod character_pose;
pub mod character_shield;
pub mod cutscene_text;
pub mod dialog_action;
pub mod dialog_definition;
mod dialog_enums;
pub mod dialog_frame;
pub mod dialog_input;
pub mod dialog_manager;
pub mod dialog_state;
pub mod dialog_text_host;
pub mod dialog_world;
pub mod effect_action;
pub mod end_game;
pub mod entity_command;
pub mod error_message_box;
pub mod give_order;
pub mod gui_basic_effect;
pub mod interaction_playback;
pub mod language_state;
pub mod leaderboards;
pub mod localization;
pub mod magick_books;
pub mod menu_text_item;
pub mod message_box;
pub mod message_box_collection;
pub mod message_box_device;
pub mod message_playback;
pub mod message_textbox;
pub mod missiles;
pub mod navigation;
pub mod npc;
pub mod npc_combat;
pub mod particle_lights;
pub mod particles;
pub mod pickup_physics;
pub mod pickups;
pub mod play_content;
pub mod play_session;
pub mod presence;
pub mod retained_text;
pub mod revival_fairy;
pub mod scene_command;
pub mod scene_flow;
pub mod scene_script;
pub mod scene_world;
pub mod script_damage;
pub mod script_runtime;
pub mod script_schema;
pub mod shield_boost;
pub mod spawn_action;
pub mod spawn_elemental;
pub mod spawn_item;
pub mod spawn_phys;
pub mod text_box;
pub mod text_box_background;
pub mod text_box_initialize;
pub mod text_geometry;
pub mod text_input_draw;
pub mod text_input_message_box;
pub mod text_input_presentation;
pub mod title_renderer;
pub mod toggle_action;
pub mod tome_magick;
pub mod trigger_control;
pub mod tutorial;
pub mod waiting_message_box;
pub mod world_condition;

pub mod phys_animation;
pub mod physics_damage;
pub mod physics_props;

pub mod authored_arena;

pub mod network_rules;
pub mod network_session;
pub mod network_world;

pub mod saves;

pub mod gun;
pub mod web_strand;
pub mod xna_math;
