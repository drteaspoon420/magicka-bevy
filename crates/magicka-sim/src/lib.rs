//! Deterministic, presentation-free reimplementation of Magicka gameplay
//! logic. Each module cites the original method it transcribes; see
//! `output/research/spells.md` for evidence and open parity questions.

pub mod agent_steering;
pub mod ai_expression;
pub mod blast_damage;
pub mod boss_death;
pub mod boss_status;
pub mod capsule_intersection;
pub mod capsule_pair;
pub mod character_health;
pub mod character_status;
pub mod conflagration;
pub mod damage;
pub mod entanglement;
pub mod hash;
pub mod liquid_contact;
pub mod liquid_freeze;
pub mod phoenix;
pub mod polymorph;
pub mod resistance;
pub mod self_shield;
pub mod shield_boost;
pub mod size_magicks;
pub mod spells;
pub mod spray_area;
pub mod status;
pub mod tornado;
pub mod vortex;
pub mod water_freeze;

pub mod impact_motion;
pub mod jump;

pub mod character_impact;

pub mod damage_update;

pub mod avatar_input;

pub mod damage_delivery;

pub mod avatar_cast_input;

pub mod weapon_spells;

pub mod spell_cast;

pub mod self_beam;
pub mod self_lightning;
pub mod self_spray;

pub mod spray_pulse;

pub mod arc_intersection;

pub mod hollow_sphere_box;
pub mod hollow_sphere_capsule;
pub mod ice_spikes;
pub mod shield_entity;
pub mod shield_intersection;
pub mod shield_self;
pub mod shield_weapon;

pub mod box_box;
pub mod box_intersection;

pub mod triangle_intersection;

pub mod mesh_octree;

pub mod collision_skin;

pub mod entity_query;

pub mod force_spray;

pub mod character_physics;

pub mod geometry;

pub mod navmesh;

pub mod push_spell;

pub mod projectile_launch;

pub mod missile_events;
pub mod missile_homing;

pub mod amalgameddon;
pub mod fear;
pub mod grease;
pub mod lightning;
pub mod magicks;
pub mod missile_collision;
pub mod napalm;
pub mod summons;

pub mod avatar_attack;
pub mod barrier;
pub mod railgun;
pub mod rigid_body;
pub mod spell_mine;
pub mod spray_weapon;
pub mod underground_attack;

pub mod shield_wall;

pub mod capsule_box;
pub mod earthquake;
pub mod elemental_egg;
pub mod ethereal_clone;
pub mod math_approximation;
pub mod portal;
pub mod spawn_slime;
pub mod summon_death;
pub mod wave;

pub mod grip;
