//! Cached trigger-area membership/counts with live entity facts at predicate time.
use crate::{scene_loading::custom_hash, script_attributes::ScriptReadError};
use magicka_content::{level::TriggerArea, skinned::pose_matrix};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug)]
pub struct PresenceCharacter {
    pub type_id: i32,
    pub original_faction: i32,
    pub faction: i32,
    pub undying: bool,
    pub invisible: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct PresenceEntity {
    /// Scene entity handle (unique IDs repeat for ID-less spawns).
    pub handle: u16,
    pub unique_id: i32,
    pub position: [f32; 3],
    pub radius: f32,
    pub dead: bool,
    /// True only for an Avatar with IgnoreTriggers set.
    pub ignored_avatar: bool,
    pub character: Option<PresenceCharacter>,
    /// A PhysicsEntity: `Some(true)` for DamageablePhysicsEntity and its
    /// subclasses.
    pub physics: Option<bool>,
}
#[derive(Default, Debug)]
pub struct AreaPresence {
    pub entities: Vec<usize>,
    pub characters: Vec<usize>,
    total_characters: i32,
    types: BTreeMap<i32, i32>,
    factions: BTreeMap<i32, i32>,
}
impl AreaPresence {
    fn add(&mut self, index: usize, entity: PresenceEntity, any: bool) {
        if entity.ignored_avatar {
            return;
        }
        self.entities.push(index);
        let Some(character) = entity.character else {
            return;
        };
        if !any {
            self.characters.push(index);
        }
        if entity.dead && !character.undying {
            return;
        }
        if any {
            self.characters.push(index);
        }
        let count = self.types.entry(character.type_id).or_default();
        *count = count.wrapping_add(1);
        for bit in 0..if any { 8 } else { 16 } {
            let mask = 1 << bit;
            if character.original_faction & mask != 0 {
                let count = self.factions.entry(mask).or_default();
                *count = count.wrapping_add(1);
            }
        }
        self.total_characters = self.total_characters.wrapping_add(1);
    }
    pub fn type_count(&self, id: i32) -> i32 {
        if id == custom_hash("any") {
            self.total_characters
        } else {
            *self.types.get(&id).unwrap_or(&0)
        }
    }
    pub fn faction_count(&self, mask: i32) -> i32 {
        let mut count = 0i32;
        for bit in 0..16 {
            let faction = 1 << bit;
            if mask & faction != 0 {
                count = count.wrapping_add(*self.factions.get(&faction).unwrap_or(&0));
            }
        }
        count
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum PresenceError {
    MissingArea(i32),
    MissingEntity(usize),
    MissingWorldFacts,
}
#[derive(Debug, PartialEq, Eq)]
pub enum AreaLoadError {
    UnsupportedName(String),
    DuplicateArea(i32),
}
pub struct PresenceAreas {
    areas: BTreeMap<i32, (Option<TriggerArea>, AreaPresence)>,
}
fn distance(area: &TriggerArea, point: [f32; 3]) -> f32 {
    let matrix = pose_matrix([0.0; 3], area.orientation, [1.0; 3]);
    let offset: [f32; 3] = std::array::from_fn(|i| point[i] - area.position[i]);
    let mut squared = 0.0;
    for i in 0..3 {
        let local = offset[0] * matrix[i * 4]
            + offset[1] * matrix[i * 4 + 1]
            + offset[2] * matrix[i * 4 + 2];
        if local < 0.0 {
            squared += local * local;
        } else if local > area.sides[i] {
            let excess = local - area.sides[i];
            squared += excess * excess;
        }
    }
    (squared as f64).sqrt() as f32
}
impl PresenceAreas {
    /// GameScene.ClearTriggerAreas resets every cache before counting global 'any'.
    pub fn reset(&mut self) {
        for (_, presence) in self.areas.values_mut() {
            *presence = AreaPresence::default();
        }
    }
    /// AnyTriggerArea.Count appends without resetting its existing counts.
    pub fn count_any(&mut self, entities: &[PresenceEntity]) {
        let (_, presence) = self.areas.get_mut(&custom_hash("any")).unwrap();
        for (index, &entity) in entities.iter().enumerate() {
            presence.add(index, entity, true);
        }
    }
    /// TriggerArea.AddEntity, used by native collision callbacks; no deduplication.
    pub fn add_entity(
        &mut self,
        area: i32,
        index: usize,
        entities: &[PresenceEntity],
    ) -> Result<(), PresenceError> {
        let entity = *entities
            .get(index)
            .ok_or(PresenceError::MissingEntity(index))?;
        let (_, presence) = self
            .areas
            .get_mut(&area)
            .ok_or(PresenceError::MissingArea(area))?;
        presence.add(index, entity, false);
        Ok(())
    }
    pub fn from_content(areas: &[TriggerArea]) -> Result<Self, AreaLoadError> {
        let mut result = Self {
            areas: BTreeMap::new(),
        };
        for area in areas {
            if !area.name.is_ascii() {
                return Err(AreaLoadError::UnsupportedName(area.name.clone()));
            }
            let id = custom_hash(&area.name.to_ascii_lowercase());
            if result
                .areas
                .insert(id, (Some(area.clone()), AreaPresence::default()))
                .is_some()
            {
                return Err(AreaLoadError::DuplicateArea(id));
            }
        }
        let any = custom_hash("any");
        if result
            .areas
            .insert(any, (None, AreaPresence::default()))
            .is_some()
        {
            return Err(AreaLoadError::DuplicateArea(any));
        }
        Ok(result)
    }
    /// Explicit UpdatePresent scans, not the usual collision-callback population.
    /// Entity indices/order must remain valid through subsequent predicate calls.
    /// Counts are cached; dead/invisible/current-faction facts may change afterwards.
    pub fn update_present(&mut self, entities: &[PresenceEntity]) {
        for (geometry, presence) in self.areas.values_mut() {
            *presence = AreaPresence::default();
            for (index, &entity) in entities.iter().enumerate() {
                if geometry
                    .as_ref()
                    .is_none_or(|area| distance(area, entity.position) <= entity.radius)
                {
                    presence.add(index, entity, geometry.is_none());
                }
            }
        }
    }
    /// Normal named-area population comes from collision callbacks, not the
    /// explicit point/radius scan. Character capsules use the native detector's
    /// old/new distance predicate and the physics collision tolerance.
    pub fn update_contacts(
        &mut self,
        entities: &[PresenceEntity],
        capsule: impl Fn(&PresenceEntity) -> Option<magicka_sim::capsule_pair::CapsuleMotion>,
    ) {
        use magicka_sim::{
            box_intersection::OrientedBox, capsule_box::segment_distance,
            character_physics::COLLISION_TOLERANCE,
        };
        for (geometry, presence) in self.areas.values_mut() {
            *presence = AreaPresence::default();
            for (index, &entity) in entities.iter().enumerate() {
                let hit = geometry.as_ref().is_none_or(|area| {
                    if let Some(capsule) = capsule(&entity) {
                        let matrix = pose_matrix([0.; 3], area.orientation, [1.; 3]);
                        let axes: [[f32; 3]; 3] = std::array::from_fn(|i| {
                            [matrix[i * 4], matrix[i * 4 + 1], matrix[i * 4 + 2]]
                        });
                        let center = std::array::from_fn(|j| {
                            area.position[j]
                                + (0..3)
                                    .map(|i| axes[i][j] * area.sides[i] * 0.5)
                                    .sum::<f32>()
                        });
                        let box_ = OrientedBox {
                            center,
                            axes,
                            side_lengths: area.sides,
                        };
                        let old = segment_distance(capsule.old_axis, box_).0;
                        let new = segment_distance(capsule.new_axis, box_).0;
                        let reach = capsule.old_radius + COLLISION_TOLERANCE;
                        old.min(new).partial_cmp(&(reach * reach)) == Some(std::cmp::Ordering::Less)
                    } else {
                        // Non-character primitive callback hosting is still partial.
                        distance(area, entity.position) <= entity.radius
                    }
                });
                if hit {
                    presence.add(index, entity, geometry.is_none());
                }
            }
        }
    }

    /// GiveOrder explicitly refreshes only its selected area. AnyTriggerArea's
    /// override is empty; its global membership remains the frame's count.
    pub fn update_present_area(
        &mut self,
        id: i32,
        entities: &[PresenceEntity],
    ) -> Result<(), PresenceError> {
        let (geometry, presence) = self
            .areas
            .get_mut(&id)
            .ok_or(PresenceError::MissingArea(id))?;
        if let Some(area) = geometry {
            *presence = AreaPresence::default();
            for (index, &entity) in entities.iter().enumerate() {
                if distance(area, entity.position) <= entity.radius {
                    presence.add(index, entity, false);
                }
            }
        }
        Ok(())
    }

    /// A WaveActions Area property refers to the selected authored area.
    pub(crate) fn alias(&mut self, alias: i32, target: i32) {
        if let Some((geometry, _)) = self.areas.get(&target) {
            self.areas
                .insert(alias, (geometry.clone(), AreaPresence::default()));
        }
    }

    pub fn get(&self, id: i32) -> Result<&AreaPresence, PresenceError> {
        self.areas
            .get(&id)
            .map(|(_, a)| a)
            .ok_or(PresenceError::MissingArea(id))
    }
}
#[derive(Clone, Copy)]
pub struct PresenceFacts<'a> {
    pub areas: &'a PresenceAreas,
    pub entities: &'a [PresenceEntity],
}
#[derive(Clone, Copy, Debug)]
pub enum PresenceKind {
    Type {
        type_id: i32,
        compare: i8,
        number: i32,
        include_invisible: bool,
    },
    Faction {
        mask: i32,
        compare: i8,
        number: i32,
        include_invisible: bool,
    },
    Id {
        id: i32,
    },
    /// PhysicsPresent: PhysicsEntity (or only DamageablePhysicsEntity)
    /// entries of PresentEntities, duplicates included.
    Physics {
        only_damageable: bool,
        compare: i8,
        number: i32,
    },
}
#[derive(Clone, Copy, Debug)]
pub struct PresenceCondition {
    pub area: i32,
    pub invert: bool,
    pub kind: PresenceKind,
}
fn compare(actual: i32, expected: i32, method: i8) -> bool {
    match method {
        -1 => actual < expected,
        0 => actual == expected,
        1 => actual > expected,
        _ => false,
    }
}
impl PresenceCondition {
    pub fn is_met(self, facts: PresenceFacts<'_>) -> Result<bool, PresenceError> {
        let area = facts.areas.get(self.area)?;
        let entity = |index: usize| {
            facts
                .entities
                .get(index)
                .ok_or(PresenceError::MissingEntity(index))
        };
        let met = match self.kind {
            PresenceKind::Id { id } => {
                let mut found = false;
                for &index in &area.entities {
                    let e = entity(index)?;
                    if e.unique_id == id {
                        found = !e.dead;
                        break;
                    }
                }
                found
            }
            PresenceKind::Type {
                type_id,
                compare: method,
                number,
                include_invisible,
            } => {
                let mut count = area.type_count(type_id);
                if !include_invisible {
                    for &index in &area.characters {
                        if let Some(c) = entity(index)?.character {
                            if c.type_id == type_id && c.invisible {
                                count = count.wrapping_sub(1);
                            }
                        }
                    }
                }
                if type_id == custom_hash("wizard") {
                    count = crate::slots::scripted_party_count(count, number, method);
                }
                compare(count, number, method)
            }
            PresenceKind::Physics {
                only_damageable,
                compare: method,
                number,
            } => {
                let mut count = 0i32;
                for &index in &area.entities {
                    match entity(index)?.physics {
                        Some(damageable) if damageable || !only_damageable => count += 1,
                        _ => {}
                    }
                }
                compare(count, number, method)
            }
            PresenceKind::Faction {
                mask,
                compare: method,
                number,
                include_invisible,
            } => {
                let mut count = area.faction_count(mask);
                if !include_invisible {
                    for &index in &area.characters {
                        if let Some(c) = entity(index)?.character {
                            if c.faction & mask != 0 && c.invisible {
                                count = count.wrapping_sub(1);
                            }
                        }
                    }
                }
                compare(count, number, method)
            }
        };
        Ok(self.invert ^ met)
    }
    pub fn read<'a>(
        tag: &str,
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        use crate::script_attributes::{boolean, comparison, identifier, number};
        let kind = match tag.to_ascii_lowercase().as_str() {
            "present" => PresenceKind::Type {
                type_id: 0,
                compare: 0,
                number: 0,
                include_invisible: true,
            },
            "factionpresent" => PresenceKind::Faction {
                mask: 0,
                compare: 0,
                number: 0,
                include_invisible: true,
            },
            "idpresent" => PresenceKind::Id { id: 0 },
            "physicspresent" => PresenceKind::Physics {
                only_damageable: false,
                compare: 0,
                number: 0,
            },
            _ => return Err(ScriptReadError::UnsupportedTag(tag.into())),
        };
        let mut condition = Self {
            area: if matches!(kind, PresenceKind::Id { .. } | PresenceKind::Physics { .. }) {
                custom_hash("any")
            } else {
                0
            },
            invert: false,
            kind,
        };
        for (key, value) in attrs {
            match key.to_ascii_lowercase().as_str() {
                "area" => condition.area = identifier(value)?,
                "invert" => condition.invert = boolean(value, key)?,
                "type" => {
                    if let PresenceKind::Type { type_id, .. } = &mut condition.kind {
                        *type_id = identifier(value)?;
                    }
                }
                "id" => {
                    if let PresenceKind::Id { id } = &mut condition.kind {
                        *id = identifier(value)?;
                    }
                }
                "nr" => match &mut condition.kind {
                    PresenceKind::Type { number: n, .. }
                    | PresenceKind::Faction { number: n, .. }
                    | PresenceKind::Physics { number: n, .. } => *n = number(value, key)?,
                    _ => {}
                },
                "onlydamagable" => {
                    if let PresenceKind::Physics {
                        only_damageable, ..
                    } = &mut condition.kind
                    {
                        *only_damageable = boolean(value, key)?;
                    }
                }
                "comparemethod" => match &mut condition.kind {
                    PresenceKind::Type { compare, .. }
                    | PresenceKind::Faction { compare, .. }
                    | PresenceKind::Physics { compare, .. } => *compare = comparison(value, key)?,
                    _ => {}
                },
                "includeinvisible" => match &mut condition.kind {
                    PresenceKind::Type {
                        include_invisible, ..
                    }
                    | PresenceKind::Faction {
                        include_invisible, ..
                    } => *include_invisible = boolean(value, key)?,
                    _ => {}
                },
                "factions" => {
                    if let PresenceKind::Faction { mask, .. } = &mut condition.kind {
                        *mask = factions(value, key)?;
                    }
                }
                _ => {}
            }
        }
        Ok(condition)
    }
}
pub(crate) fn factions(value: &str, key: &str) -> Result<i32, ScriptReadError> {
    if let Ok(number) = value.trim().parse() {
        return Ok(number);
    }
    let mut mask = 0;
    for item in value.split(',') {
        mask |= match item.trim().to_ascii_lowercase().as_str() {
            "none" => 0,
            "evil" => 1,
            "wild" => 2,
            "friendly" => 4,
            "demon" => 8,
            "undead" => 16,
            "human" => 32,
            "wizard" => 64,
            "neutral" => 255,
            "player0" => 256,
            "player1" => 512,
            "player2" => 1024,
            "player3" => 2048,
            "team_red" => 4096,
            "team_blue" => 8192,
            "players" => 0x3f00,
            "number_of_factions" => 15,
            _ => return Err(ScriptReadError::InvalidAttribute(key.into())),
        };
    }
    Ok(mask)
}

#[cfg(test)]
mod contact_tests {
    use super::*;
    use magicka_sim::{capsule_pair::CapsuleMotion, spray_pulse::Segment};

    fn fixture() -> (PresenceAreas, PresenceEntity, CapsuleMotion) {
        let areas = PresenceAreas::from_content(&[TriggerArea {
            name: "box".into(),
            position: [0.; 3],
            sides: [1.; 3],
            orientation: [0., 0., 0., 1.],
        }])
        .unwrap();
        let entity = PresenceEntity {
            handle: 7,
            unique_id: 0,
            position: [1.82, 0.5, 0.5],
            radius: 0.8,
            dead: false,
            ignored_avatar: false,
            physics: None,
            character: Some(PresenceCharacter {
                type_id: custom_hash("wizard"),
                original_faction: 1,
                faction: 1,
                undying: false,
                invisible: false,
            }),
        };
        let axis = Segment {
            origin: [1.82, 0.3, 0.5],
            delta: [0., 0.4, 0.],
        };
        let motion = CapsuleMotion {
            old_axis: axis,
            new_axis: axis,
            old_radius: 0.8,
            new_radius: 0.8,
            old_body_position: entity.position,
        };
        (areas, entity, motion)
    }

    #[test]
    fn contact_tolerance_is_distinct_from_explicit_point_scan() {
        let (mut areas, entity, motion) = fixture();
        areas.update_contacts(&[entity], |_| Some(motion));
        assert_eq!(areas.get(custom_hash("box")).unwrap().entities, [0]);
        areas
            .update_present_area(custom_hash("box"), &[entity])
            .unwrap();
        assert!(areas.get(custom_hash("box")).unwrap().entities.is_empty());
        areas.update_present_area(custom_hash("any"), &[]).unwrap();
        assert_eq!(areas.get(custom_hash("any")).unwrap().entities, [0]);
    }

    #[test]
    fn collision_population_keeps_old_capsule_contact_after_it_moves_away() {
        let (mut areas, mut entity, mut motion) = fixture();
        entity.position[0] = 3.;
        motion.new_axis.origin[0] = 3.;
        areas.update_contacts(&[entity], |_| Some(motion));
        assert_eq!(areas.get(custom_hash("box")).unwrap().entities, [0]);
        motion.old_axis = motion.new_axis;
        areas.update_contacts(&[entity], |_| Some(motion));
        assert!(areas.get(custom_hash("box")).unwrap().entities.is_empty());
    }
}
