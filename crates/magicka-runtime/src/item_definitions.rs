//! KV item definitions applied to retail item templates (rewrite extension,
//! plan workstream D). Only fields a definition states change; everything
//! else stays as decoded from the user's content.
use crate::avatar::AvatarItem;
use magicka_content::item::Item;
use magicka_definitions::Definitions;
use std::sync::Arc;

/// Apply the definition overriding this item's retail template, if any.
/// Absolute values: applying twice is the same as once.
pub fn patch(defs: &Definitions, item: &mut AvatarItem) -> bool {
    let Some(def) = defs.retail_item(&item.definition.name) else {
        return false;
    };
    for (key, &v) in &def.common.numbers {
        set_number(&mut item.definition, key, v);
    }
    // KV values are the new base for upgrade overlays.
    item.base_definition = None;
    refresh_melee(item);
    true
}

/// A schema number of an item template (`ITEM_FIELDS` keys).
pub fn number(d: &Item, key: &str) -> Option<f64> {
    Some(match key {
        "BlockValue" => f64::from(d.block_value),
        "CooldownTime" => f64::from(d.cooldown_time),
        "PassiveValue" => f64::from(d.passive_value),
        "MeleeRange" => f64::from(d.melee_range),
        "RangedRange" => f64::from(d.ranged_range),
        "RangedElevation" => f64::from(d.ranged_elevation_deg),
        "Homing" => f64::from(d.homing),
        "GunRange" => f64::from(d.gun_range),
        "GunClip" => f64::from(d.gun_clip),
        "GunRate" => f64::from(d.gun_rate),
        "GunAccuracy" => f64::from(d.gun_accuracy),
        "TracerVelocity" => f64::from(d.tracer_velocity),
        "Scale" => f64::from(d.scale),
        _ => return None,
    })
}
/// Set a schema number of an item template.
pub fn set_number(d: &mut Item, key: &str, v: f64) {
    let f = v as f32;
    match key {
        "BlockValue" => d.block_value = v as i32,
        "CooldownTime" => d.cooldown_time = f,
        "PassiveValue" => d.passive_value = f,
        "MeleeRange" => d.melee_range = f,
        "RangedRange" => d.ranged_range = f,
        "RangedElevation" => d.ranged_elevation_deg = f,
        "Homing" => d.homing = f,
        "GunRange" => d.gun_range = f,
        "GunClip" => d.gun_clip = v as i32,
        "GunRate" => d.gun_rate = v as i32,
        "GunAccuracy" => d.gun_accuracy = f,
        "TracerVelocity" => d.tracer_velocity = f,
        "Scale" => d.scale = f,
        _ => {}
    }
}

/// Resolve upgrade overlays onto one held item: every schema number
/// recomputed from the captured base (`schema::resolve`). No overlays
/// restore the base. Returns whether the item changed.
pub fn apply_upgrades(
    item: &mut AvatarItem,
    overlays: &[(&str, magicka_definitions::schema::Change, &str)],
) -> bool {
    use magicka_definitions::schema::{self, Contribution, Layer};
    if overlays.is_empty() {
        let Some(base) = item.base_definition.take() else {
            return false;
        };
        item.definition = *base;
        refresh_melee(item);
        return true;
    }
    let base = item
        .base_definition
        .get_or_insert_with(|| Box::new(item.definition.clone()))
        .clone();
    let mut next = (*base).clone();
    let keys: std::collections::BTreeSet<&str> = overlays.iter().map(|(k, _, _)| *k).collect();
    for key in keys {
        let Some(s) = schema::find(magicka_definitions::ITEM_FIELDS, key) else {
            continue;
        };
        let Some(b) = number(&base, s.key) else {
            continue;
        };
        let contributions: Vec<Contribution> = overlays
            .iter()
            .filter(|(k, _, _)| *k == key)
            .map(|(_, change, source)| Contribution {
                layer: Layer::Upgrade,
                change: *change,
                source: source.to_string(),
            })
            .collect();
        set_number(&mut next, s.key, schema::resolve(s, b, &contributions));
    }
    let changed = next != item.definition;
    if changed {
        item.definition = next;
        refresh_melee(item);
    }
    changed
}

fn refresh_melee(item: &mut AvatarItem) {
    // Melee reach is cached at construction.
    let ranged = item.melee.ranged.take();
    item.melee = crate::npc_combat::MeleeWeapon::new(
        item.definition.melee_range,
        item.definition.melee_multi_hit,
        Arc::new(item.definition.melee_conditions.0.clone()),
    );
    item.melee.ranged = ranged;
}
