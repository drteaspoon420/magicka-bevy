//! Editor / dungeon-master world edits: spawn a catalogue entry at a
//! placement, move or remove a live entity. A rewrite extension (not an
//! original feature) shared by local tool mode and a host applying a
//! dungeon master's requests; see research/editor-mode.md.
//!
//! Transforms are model ("visual") transforms in XNA storage: for
//! characters the translation is the feet point the renderer draws at.
use super::*;
use crate::entity_command::EntityCommandHost;

/// A spawnable catalogue entry, written `kind:name` on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorSpawn<'a> {
    /// Character template (Data/Characters).
    Npc(&'a str),
    /// Physics entity template (Data/PhysicsEntities).
    Prop(&'a str),
    /// Wizard item pickup (Data/Items/Wizard).
    Item(&'a str),
    /// Magick book by magick name.
    Book(&'a str),
    /// Effect by name.
    Effect(&'a str),
}

impl<'a> EditorSpawn<'a> {
    pub fn parse(id: &'a str) -> Option<Self> {
        let (kind, name) = id.split_once(':')?;
        if name.is_empty() {
            return None;
        }
        Some(match kind {
            "npc" => Self::Npc(name),
            "prop" => Self::Prop(name),
            "item" => Self::Item(name),
            "book" => Self::Book(name),
            "fx" => Self::Effect(name),
            _ => return None,
        })
    }

    pub fn id(&self) -> String {
        let (kind, name) = match self {
            Self::Npc(n) => ("npc", n),
            Self::Prop(n) => ("prop", n),
            Self::Item(n) => ("item", n),
            Self::Book(n) => ("book", n),
            Self::Effect(n) => ("fx", n),
        };
        format!("{kind}:{name}")
    }
}

/// Optional spawn settings from the editor's property panel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorSpawnOptions {
    /// NPC order ("attack", "idle", "defend", ...); default "attack".
    pub order: Option<String>,
    /// Items and books: movable (physics) instead of fixed in place.
    pub movable: bool,
}

/// The NPC orders the spawn action accepts (scene script `order=`).
pub const NPC_ORDERS: [&str; 6] = ["attack", "idle", "defend", "flee", "wander", "none"];

/// A character's orientation from a model transform. Characters are rigid, and
/// a dungeon master can send any finite matrix: a scaled or skewed one would make
/// every published frame fail validation, ending the game for all clients. A
/// matrix with no usable axes leaves the character facing as it was.
fn rigid_orientation(
    m: &[f32; 16],
    current: magicka_sim::character_physics::Orientation,
) -> magicka_sim::character_physics::Orientation {
    fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
        let length = v.iter().map(|c| c * c).sum::<f64>().sqrt();
        (length.is_finite() && length > 1e-6).then(|| v.map(|c| c / length))
    }
    let wide = |i: usize| [f64::from(m[i]), f64::from(m[i + 1]), f64::from(m[i + 2])];
    let axes = unit(wide(4)).and_then(|up| {
        let raw = wide(8);
        let along: f64 = (0..3).map(|i| raw[i] * up[i]).sum();
        let backward = unit(std::array::from_fn(|i| raw[i] - along * up[i]))?;
        Some((up, backward))
    });
    let Some((up, backward)) = axes else {
        return current;
    };
    let right = [
        up[1] * backward[2] - up[2] * backward[1],
        up[2] * backward[0] - up[0] * backward[2],
        up[0] * backward[1] - up[1] * backward[0],
    ];
    magicka_sim::character_physics::Orientation {
        right: right.map(|c| c as f32),
        up: up.map(|c| c as f32),
        backward: backward.map(|c| c as f32),
    }
}

impl PlaySession {
    /// Spawn catalogue entry `id` at `transform`. Returns the new entity
    /// handle (effects have none).
    pub fn editor_spawn(
        &mut self,
        id: &str,
        transform: [f32; 16],
        options: &EditorSpawnOptions,
    ) -> Result<Option<u16>, String> {
        let spawn = EditorSpawn::parse(id).ok_or_else(|| format!("unknown spawnable {id}"))?;
        let position = [transform[12], transform[13], transform[14]];
        match spawn {
            EditorSpawn::Npc(template) => {
                let order = options.order.as_deref().unwrap_or("attack");
                if !NPC_ORDERS.contains(&order) {
                    return Err(format!("unknown order {order}"));
                }
                let handle = self.spawn_npc(template, order, 0.0)?;
                self.place_character_on_ground(handle, position, &transform)?;
                Ok(Some(handle))
            }
            EditorSpawn::Prop(name) => self
                .spawn_physics_prop(name, transform)
                .map(Some)
                .map_err(|e| format!("{e:?}")),
            EditorSpawn::Item(name) => {
                let handle = self
                    .spawn_item_pickup(name, transform)
                    .map_err(|e| format!("{e:?}"))?;
                if let Some(p) = self.world.item_pickups.get_mut(&handle) {
                    p.immovable = !options.movable;
                }
                Ok(Some(handle))
            }
            EditorSpawn::Book(name) => {
                let magick = crate::magick_books::magick_id(name)
                    .ok_or_else(|| format!("unknown magick {name}"))?;
                let handle = self
                    .spawn_magick_book(magick, transform)
                    .map_err(|e| format!("{e:?}"))?;
                if let Some(book) = self.world.book_pickups.get_mut(&handle) {
                    book.immovable = !options.movable;
                    book.body_enabled = options.movable;
                }
                Ok(Some(handle))
            }
            EditorSpawn::Effect(name) => {
                let hash = self
                    .effects
                    .library
                    .effects
                    .iter()
                    .find(|(_, e)| e.name.eq_ignore_ascii_case(name))
                    .map(|(&hash, _)| hash)
                    .ok_or_else(|| format!("unknown effect {name}"))?;
                let forward = [-transform[8], -transform[9], -transform[10]];
                self.effects
                    .start_at(hash, position, forward)
                    .ok_or("effect pool full")?;
                Ok(None)
            }
        }
    }

    /// Put a character's feet on the ground below `position` (or at it when
    /// nothing is below) facing `transform`'s orientation.
    fn place_character_on_ground(
        &mut self,
        handle: u16,
        position: Vector,
        transform: &[f32; 16],
    ) -> Result<(), String> {
        let ground = self
            .world
            .collision
            .collision()
            .segment_intersect(
                Segment {
                    origin: [position[0], position[1] + 2.0, position[2]],
                    delta: [0.0, -8.0, 0.0],
                },
                false,
            )
            .map(|hit| hit.hit.position);
        let mut feet_transform = *transform;
        feet_transform[12..15].copy_from_slice(&ground.unwrap_or(position));
        self.editor_move(handle, feet_transform)
    }

    /// Move a live character, prop, item or book so its model transform
    /// becomes `transform`.
    pub fn editor_move(&mut self, handle: u16, transform: [f32; 16]) -> Result<(), String> {
        let position = [transform[12], transform[13], transform[14]];
        let kind = self
            .world
            .entities
            .iter()
            .find(|e| e.handle == handle && !e.removed)
            .map(|e| e.kind.clone())
            .ok_or("no such entity")?;
        match kind {
            crate::scene_world::EntityKind::Npc { .. }
            | crate::scene_world::EntityKind::Avatar { .. } => {
                let body = self
                    .world
                    .npcs
                    .get(&handle)
                    .map(|n| n.body)
                    .or_else(|| {
                        self.world
                            .players
                            .iter()
                            .flatten()
                            .find(|p| p.handle == handle)
                            .map(|p| p.body)
                    })
                    .ok_or("character has no body")?;
                let b = &mut self.world.physics.bodies[body];
                // The model stands on the body's feet point.
                let offset: [f32; 3] = std::array::from_fn(|i| b.position[i] - b.feet()[i]);
                let center: [f32; 3] = std::array::from_fn(|i| position[i] + offset[i]);
                b.velocity = [0.0; 3];
                let rigid = rigid_orientation(&transform, b.orientation);
                b.move_to(center, rigid);
                let e = self
                    .world
                    .entity_mut(handle)
                    .map_err(|e| format!("{e:?}"))?;
                e.position = center;
                let mut o = transform;
                o[0..3].copy_from_slice(&rigid.right);
                o[4..7].copy_from_slice(&rigid.up);
                o[8..11].copy_from_slice(&rigid.backward);
                o[12..15].copy_from_slice(&center);
                e.orientation = o;
            }
            _ => {
                if let Some(p) = self.world.item_pickups.get_mut(&handle) {
                    p.transform = transform;
                }
                if let Some(book) = self.world.book_pickups.get_mut(&handle) {
                    book.transform = transform;
                    book.velocity = [0.0; 3];
                }
                if let Some(physics) = &mut self.pickup_physics {
                    physics.place(handle, transform);
                }
                let e = self
                    .world
                    .entity_mut(handle)
                    .map_err(|e| format!("{e:?}"))?;
                e.position = position;
                e.orientation = transform;
            }
        }
        Ok(())
    }

    /// Remove a live NPC, prop, item or book (players cannot be removed).
    pub fn editor_remove(&mut self, handle: u16) -> Result<(), String> {
        let kind = self
            .world
            .entities
            .iter()
            .find(|e| e.handle == handle && !e.removed)
            .map(|e| e.kind.clone())
            .ok_or("no such entity")?;
        match kind {
            crate::scene_world::EntityKind::Avatar { .. } => {
                return Err("players cannot be removed".into())
            }
            crate::scene_world::EntityKind::Npc { .. } => {
                EntityCommandHost::terminate(&mut self.world, handle)
                    .map_err(|e| format!("{e:?}"))?;
            }
            _ => {
                if let Some(book) = self.world.book_pickups.get_mut(&handle) {
                    book.kill();
                }
                self.world
                    .entity_mut(handle)
                    .map_err(|e| format!("{e:?}"))?
                    .removed = true;
            }
        }
        if let Some(physics) = &mut self.pickup_physics {
            physics.remove(handle);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_ids_round_trip() {
        for id in [
            "npc:Goblin_pirate",
            "prop:barrel",
            "item:staff_x",
            "book:haste",
            "fx:fire",
        ] {
            assert_eq!(EditorSpawn::parse(id).unwrap().id(), id);
        }
        assert!(EditorSpawn::parse("npc:").is_none());
        assert!(EditorSpawn::parse("goblin").is_none());
        assert!(EditorSpawn::parse("deco:a.glb").is_none());
    }
}
