//! Item.Update's element-indexed chant references, shared by player and NPC items.
use crate::{
    item_effects::{chant_effects, ItemFrame, CHANT_EFFECTS},
    particles::{EffectRef, EffectSystem},
    scene_loading::custom_hash,
};
use magicka_sim::spells::Elements;
use std::collections::{BTreeMap, BTreeSet};

type Key = (u16, i32, usize);
pub(super) fn sync(
    effects: &mut EffectSystem,
    references: &mut BTreeMap<Key, EffectRef>,
    seen: &mut BTreeSet<Key>,
    owner_slot: (u16, i32),
    frame: ItemFrame,
    elements: Elements,
) {
    let transform = frame.chant_transform();
    let wanted: BTreeMap<usize, &str> = chant_effects(elements).collect();
    for index in 0..CHANT_EFFECTS.len() {
        let key = (owner_slot.0, owner_slot.1, index);
        if let Some(name) = wanted.get(&index) {
            seen.insert(key);
            let live = references
                .get_mut(&key)
                .is_some_and(|r| effects.update_orientation(r, transform));
            if !live {
                if let Some(reference) = effects.start(custom_hash(name), transform) {
                    references.insert(key, reference);
                }
            }
        } else if let Some(mut reference) = references.remove(&key) {
            effects.stop(&mut reference);
        }
    }
}
