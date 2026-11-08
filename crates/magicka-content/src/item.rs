//! `Magicka.ContentReaders.ItemReader` -> `Items.Item.Read` (`Data/items/**`).

use crate::common::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub sounds: Vec<(String, i32)>,
    pub pickable: bool,
    pub bound: bool,
    pub block_value: i32,
    pub weapon_class: u8,
    pub cooldown_time: f32,
    pub hide_model: bool,
    pub hide_effect: bool,
    pub pause_sounds: bool,
    pub resistances: Vec<Resistance>,
    pub passive_ability: u8,
    pub passive_value: f32,
    pub effects: Vec<String>,
    /// At most one (the original throws on a second light).
    pub light: Option<PointLight>,
    pub special_ability: Option<(f32, SpecialAbility)>,
    pub melee_range: f32,
    pub melee_multi_hit: bool,
    pub melee_conditions: ConditionCollection,
    pub ranged_range: f32,
    pub facing: bool,
    pub homing: f32,
    /// Degrees as stored (the game converts to radians).
    pub ranged_elevation_deg: f32,
    pub ranged_danger: f32,
    pub gun_range: f32,
    pub gun_clip: i32,
    pub gun_rate: i32,
    pub gun_accuracy: f32,
    /// `"Bank/cue"` or `"cue"` (bank defaults to Weapons).
    pub gun_sound: String,
    pub gun_muzzle_effect: String,
    pub gun_shells_effect: String,
    pub tracer_velocity: f32,
    /// Sprite spec `"[A-E]/n"` or `"n"`; see `Item.Read`.
    pub non_tracer_sprite: String,
    pub tracer_sprite: String,
    pub gun_conditions: ConditionCollection,
    pub projectile_model: Option<String>,
    pub ranged_conditions: ConditionCollection,
    pub scale: f32,
    pub model: Option<String>,
    pub auras: Vec<AuraStorage>,
}

impl Item {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let name = r.string()?;
        let display_name = r.string()?;
        let description = r.string()?;
        let n = r.count()?;
        let sounds = (0..n)
            .map(|_| Ok((r.string()?, r.i32()?)))
            .collect::<Result<_>>()?;
        let pickable = r.bool()?;
        let bound = r.bool()?;
        let block_value = r.i32()?;
        let weapon_class = r.u8()?;
        let cooldown_time = r.f32()?;
        let hide_model = r.bool()?;
        let hide_effect = r.bool()?;
        let pause_sounds = r.bool()?;
        let n = r.count()?;
        let resistances = (0..n).map(|_| Resistance::read(r)).collect::<Result<_>>()?;
        let passive_ability = r.u8()?;
        let passive_value = r.f32()?;
        let effects = r.strings()?;
        let n = r.count()?;
        if n > 1 {
            return invalid("Items may Only have One Light!");
        }
        let light = if n == 1 {
            Some(PointLight::read(r)?)
        } else {
            None
        };
        let special_ability = if r.bool()? {
            Some((r.f32()?, SpecialAbility::read(r)?))
        } else {
            None
        };
        Ok(Item {
            name,
            display_name,
            description,
            sounds,
            pickable,
            bound,
            block_value,
            weapon_class,
            cooldown_time,
            hide_model,
            hide_effect,
            pause_sounds,
            resistances,
            passive_ability,
            passive_value,
            effects,
            light,
            special_ability,
            melee_range: r.f32()?,
            melee_multi_hit: r.bool()?,
            melee_conditions: ConditionCollection::read(r)?,
            ranged_range: r.f32()?,
            facing: r.bool()?,
            homing: r.f32()?,
            ranged_elevation_deg: r.f32()?,
            ranged_danger: r.f32()?,
            gun_range: r.f32()?,
            gun_clip: r.i32()?,
            gun_rate: r.i32()?,
            gun_accuracy: r.f32()?,
            gun_sound: r.string()?,
            gun_muzzle_effect: r.string()?,
            gun_shells_effect: r.string()?,
            tracer_velocity: r.f32()?,
            non_tracer_sprite: r.string()?,
            tracer_sprite: r.string()?,
            gun_conditions: ConditionCollection::read(r)?,
            projectile_model: r.external()?,
            ranged_conditions: ConditionCollection::read(r)?,
            scale: r.f32()?,
            model: r.external()?,
            auras: {
                let n = r.count()?;
                (0..n)
                    .map(|_| AuraStorage::read(r))
                    .collect::<Result<_>>()?
            },
        })
    }
}
