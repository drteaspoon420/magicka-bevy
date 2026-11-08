//! GameScene.Destroy(saveNPCs) / AddSavedEntities: entities a scene keeps
//! for its next visit (research/wizard-castle-pending.md, scene revisits).
//!
//! EntityManager.ClearAndStore keeps non-summoned NPCs with positive health
//! (or Undying) after disabling their agent, and non-removable physics
//! entities; everything else is deinitialized. AddSavedEntities re-enables
//! bodies and agents. Live NPC objects move between scene worlds here, with
//! fresh handles and body indices in the receiving world. Pickables and
//! elemental eggs, which ClearAndStore also keeps, are not carried yet.
use super::*;

/// PhysicsEntity.State: placement, template and damage callbacks; native
/// ApplyTo initializes template health afresh rather than storing current HP.
#[derive(Clone)]
pub struct PhysicsSnapshot {
    template: String,
    transform: [f32; 16],
    unique_id: i32,
    on_death: i32,
    on_damage: i32,
}

impl PhysicsSnapshot {
    pub(crate) fn unique_id(&self) -> i32 {
        self.unique_id
    }
    pub(crate) fn disk_state(&self) -> serde_json::Value {
        serde_json::json!({"template":self.template,"transform":self.transform,"unique_id":self.unique_id,
            "on_death":self.on_death,"on_damage":self.on_damage})
    }
    pub(crate) fn from_disk(v: &serde_json::Value) -> Result<Self, String> {
        let integer = |key: &str| {
            v[key]
                .as_i64()
                .and_then(|i| i32::try_from(i).ok())
                .ok_or_else(|| format!("invalid saved physics {key}"))
        };
        let transform: [f32; 16] =
            serde_json::from_value(v["transform"].clone()).map_err(|e| e.to_string())?;
        if transform.iter().any(|v| !v.is_finite()) {
            return Err("invalid saved physics transform".into());
        }
        Ok(Self {
            template: v["template"]
                .as_str()
                .ok_or("missing saved physics template")?
                .to_owned(),
            transform,
            unique_id: integer("unique_id")?,
            on_death: integer("on_death")?,
            on_damage: integer("on_damage")?,
        })
    }
}

/// Pickable.State's identity: a BookOfMagick's magick or an Item's name.
#[derive(Clone, Debug, PartialEq)]
pub enum PickableKind {
    BookOfMagick { magick: u8 },
    Item { name: String },
}

/// Pickable.State of a permanent pickable (EntityStateStorage.Store keeps
/// `IsPickable && Permanent`: a pickable, living Item with no despawn time,
/// or a living BookOfMagick with no timeout). The original decomposes the
/// body transform into position and orientation; the matrix is kept here.
#[derive(Clone, Debug, PartialEq)]
pub struct PickableSnapshot {
    pub kind: PickableKind,
    pub transform: [f32; 16],
    pub immovable: bool,
    pub unique_id: i32,
}

impl PickableSnapshot {
    pub(crate) fn disk_state(&self) -> serde_json::Value {
        let (kind, magick, name) = match &self.kind {
            PickableKind::BookOfMagick { magick } => ("book", Some(*magick), None),
            PickableKind::Item { name } => ("item", None, Some(name.as_str())),
        };
        serde_json::json!({"kind":kind,"magick":magick,"name":name,"transform":self.transform,
            "immovable":self.immovable,"unique_id":self.unique_id})
    }
    pub(crate) fn from_disk(v: &serde_json::Value) -> Result<Self, String> {
        let transform: [f32; 16] =
            serde_json::from_value(v["transform"].clone()).map_err(|e| e.to_string())?;
        if transform.iter().any(|v| !v.is_finite()) {
            return Err("invalid saved pickable transform".into());
        }
        let kind = match v["kind"].as_str() {
            Some("book") => PickableKind::BookOfMagick {
                magick: v["magick"]
                    .as_u64()
                    .and_then(|m| u8::try_from(m).ok())
                    .filter(|m| {
                        (*m as usize) < crate::magick_books::BOOK_MAGICKS.len()
                            && *m != crate::magick_books::SENTINEL
                    })
                    .ok_or("invalid saved book magick")?,
            },
            Some("item") => PickableKind::Item {
                name: v["name"]
                    .as_str()
                    .filter(|n| !n.is_empty() && !n.contains(['/', '\\', '.']))
                    .ok_or("invalid saved item name")?
                    .to_owned(),
            },
            _ => return Err("invalid saved pickable kind".into()),
        };
        Ok(Self {
            kind,
            transform,
            immovable: v["immovable"]
                .as_bool()
                .ok_or("missing saved pickable immovable flag")?,
            unique_id: v["unique_id"]
                .as_i64()
                .and_then(|i| i32::try_from(i).ok())
                .ok_or("invalid saved pickable unique id")?,
        })
    }
}

/// One entity retained by a departing scene.
#[derive(Clone)]
pub struct SavedEntity {
    entity: SceneEntity,
    npc: Option<SavedNpc>,
    dialog: Option<i32>,
    physics_damage: Option<crate::physics_damage::PhysicsDamage>,
    static_box: Option<magicka_content::physics::CollisionBox>,
}

#[derive(Clone)]
struct SavedNpc {
    npc: crate::npc::NpcCharacter,
    body: magicka_sim::character_physics::CharacterBody,
    template: Option<(NpcTemplate, Option<String>)>,
}

impl<C: SceneContent> SceneWorld<C> {
    /// Compatibility for rewrite saves predating PhysicsSnapshot. Only recover
    /// living, named, stationary props whose executed SpawnPhys action supplies
    /// an exact static locator. This cannot recover moved or anonymous objects.
    pub(crate) fn legacy_checkpoint_physics(
        &mut self,
        runtime: &ScriptRuntime,
    ) -> Vec<PhysicsSnapshot> {
        let mut seen = std::collections::BTreeSet::new();
        runtime
            .history_excluding(&[])
            .into_iter()
            .rev()
            .filter_map(|index| {
                let crate::scene_script::ActionDefinition::SpawnPhys(action) =
                    &runtime.layout().actions[index]
                else {
                    return None;
                };
                if action.unique_id == 0 || !seen.insert(action.unique_id) {
                    return None;
                }
                let health = runtime.spawn_phys_state(index)?.health.health;
                let name = action.object.as_ref()?;
                let template = self.content.physics_template(name)?;
                if health <= 0.
                    || template.max_hitpoints <= 0
                    || template.movable
                    || template.pushable
                {
                    return None;
                }
                Some(PhysicsSnapshot {
                    template: name.clone(),
                    transform: *self.locators.get(&action.area)?,
                    unique_id: action.unique_id,
                    on_death: action.on_death,
                    on_damage: action.on_damage,
                })
            })
            .collect()
    }

    /// EntityStateStorage.Store retains living physics entities independently
    /// of SpawnPhys's empty QuickExecute override.
    pub fn checkpoint_physics(&self) -> Vec<PhysicsSnapshot> {
        self.entities
            .iter()
            .filter_map(|e| {
                if !matches!(e.kind, EntityKind::Physics { .. })
                    || !e.added
                    || e.dead
                    || e.removed
                    || self.item_pickups.contains_key(&e.handle)
                    || self.book_pickups.contains_key(&e.handle)
                {
                    return None;
                }
                let mut transform = e.orientation;
                transform[12..15].copy_from_slice(&e.position);
                let damage = self.physics_damage.get(&e.handle);
                Some(PhysicsSnapshot {
                    template: e.template_name.clone()?,
                    transform,
                    unique_id: e.unique_id,
                    on_death: damage.map_or(0, |d| d.on_death),
                    on_damage: damage.map_or(0, |d| d.on_damage),
                })
            })
            .collect()
    }

    pub fn restore_checkpoint_physics(
        &mut self,
        states: &[PhysicsSnapshot],
    ) -> Result<(), WorldError> {
        for state in states {
            if state.transform.iter().any(|v| !v.is_finite()) {
                return Err("invalid saved physics transform".into());
            }
            let max = SpawnPhysLevel::load_physics_template(self, Some(&state.template))?;
            let handle = self.allocate_physics(max > 0);
            SpawnPhysHost::initialize_entity(
                self,
                handle,
                Some(&state.template),
                state.transform,
                state.unique_id,
            )?;
            if max > 0 {
                SpawnPhysHost::set_damage_triggers(self, handle, state.on_death, state.on_damage)?;
            }
            SpawnPhysHost::add_entity(self, handle)?;
        }
        Ok(())
    }

    /// EntityStateStorage.Store's permanent pickables (Pickable.State).
    pub fn checkpoint_pickables(&self) -> Vec<PickableSnapshot> {
        self.entities
            .iter()
            .filter(|e| e.added && !e.removed && !e.dead)
            .filter_map(|e| {
                if let Some(book) = self.book_pickups.get(&e.handle) {
                    // BookOfMagick.Permanent: mTimeOutTimer <= 0 && !Dead.
                    if !book.permanent() {
                        return None;
                    }
                    return Some(PickableSnapshot {
                        kind: PickableKind::BookOfMagick {
                            magick: book.magick,
                        },
                        transform: book.transform,
                        immovable: book.immovable,
                        unique_id: e.unique_id,
                    });
                }
                let pickup = self.item_pickups.get(&e.handle)?;
                let item = pickup.item.as_ref()?;
                // Item.Permanent: mPickable && !Dead && mDespawnTime <= 0.
                if !item.definition.pickable || item.attached || pickup.despawn > 0.0 {
                    return None;
                }
                Some(PickableSnapshot {
                    kind: PickableKind::Item {
                        name: item.definition.name.clone(),
                    },
                    transform: pickup.transform,
                    immovable: pickup.immovable,
                    unique_id: e.unique_id,
                })
            })
            .collect()
    }

    /// Pickable.State.Restore then AddSavedEntities: an Item is loaded from
    /// content, copied, detached, moved to the saved placement with the saved
    /// Immovable flag; a BookOfMagick is initialized at it with no velocity,
    /// no timeout and the saved unique ID. Both are added with enabled bodies.
    pub fn restore_checkpoint_pickables(
        &mut self,
        states: &[PickableSnapshot],
    ) -> Result<(), WorldError> {
        use crate::magick_books::SpawnMagickHost;
        use crate::spawn_item::SpawnItemHost;
        for state in states {
            if state.transform.iter().any(|v| !v.is_finite()) {
                return Err("invalid saved pickable transform".into());
            }
            let position = [
                state.transform[12],
                state.transform[13],
                state.transform[14],
            ];
            match &state.kind {
                PickableKind::Item { name } => {
                    SpawnItemHost::load_item(self, &format!("Data/Items/Wizard/{name}"))?;
                    let handle = SpawnItemHost::new_placeholder(self)?;
                    SpawnItemHost::copy_and_detach(self, handle, custom_hash(name), 0)?;
                    SpawnItemHost::move_to(self, handle, position, state.transform)?;
                    SpawnItemHost::configure(
                        self,
                        handle,
                        state.immovable,
                        false,
                        Some(state.unique_id),
                    )?;
                    SpawnItemHost::add_entity(self, handle)?;
                    SpawnItemHost::enable_body(self, handle)?;
                }
                PickableKind::BookOfMagick { magick } => {
                    let action = crate::magick_books::SpawnMagickAction {
                        delay: 0.0,
                        area: 0,
                        magick: *magick,
                        immovable: state.immovable,
                        timeout: 0.0,
                        unique_id: state.unique_id,
                        on_pickup: 0,
                    };
                    SpawnMagickHost::spawn_book(self, &action, state.transform)?;
                }
            }
        }
        Ok(())
    }

    /// ClearAndStore's retained subset of this world's entities.
    pub fn store_saved_entities(&self) -> Vec<SavedEntity> {
        let mut saved = Vec::new();
        for e in &self.entities {
            if !e.added || e.removed {
                continue;
            }
            match e.kind {
                EntityKind::Npc { template, .. } => {
                    let Some(npc) = self.npcs.get(&e.handle) else {
                        continue;
                    };
                    if npc.summon.is_some() || (e.hit_points <= 0.0 && !e.undying) || e.dead {
                        continue;
                    }
                    saved.push(SavedEntity {
                        entity: e.clone(),
                        npc: Some(SavedNpc {
                            npc: npc.clone(),
                            body: self.physics.bodies[npc.body].clone(),
                            template: self
                                .templates
                                .get(&template)
                                .cloned()
                                .map(|t| (t, self.template_names.get(&template).cloned())),
                        }),
                        dialog: self.character_dialogs.get(&e.handle).copied(),
                        physics_damage: None,
                        static_box: None,
                    });
                }
                EntityKind::Physics { .. } => {
                    if e.dead
                        || self.item_pickups.contains_key(&e.handle)
                        || self.book_pickups.contains_key(&e.handle)
                    {
                        continue;
                    }
                    saved.push(SavedEntity {
                        entity: e.clone(),
                        npc: None,
                        dialog: None,
                        physics_damage: self.physics_damage.get(&e.handle).cloned(),
                        static_box: self.static_prop_boxes.get(&e.handle).cloned(),
                    });
                }
                EntityKind::Item
                | EntityKind::Avatar { .. }
                | EntityKind::ElementalEgg
                | EntityKind::Dispenser => {}
            }
        }
        saved
    }

    /// AddSavedEntities: re-add retained entities, enabling bodies and agents.
    pub fn add_saved_entities(&mut self, saved: Vec<SavedEntity>) {
        for s in saved {
            let handle = self.new_handle();
            let mut entity = s.entity;
            entity.handle = handle;
            if let Some(mut n) = s.npc {
                if let (EntityKind::Npc { template, .. }, Some((t, name))) =
                    (&entity.kind, n.template)
                {
                    self.templates.entry(*template).or_insert(t);
                    if let Some(name) = name {
                        self.template_names.entry(*template).or_insert(name);
                    }
                }
                let index = self.physics.bodies.len();
                n.body.enable();
                self.physics.bodies.push(n.body);
                n.npc.handle = handle;
                n.npc.body = index;
                self.npcs.insert(handle, n.npc);
                // Agent.Enable.
                if !self.agents.contains(&handle) {
                    self.agents.push(handle);
                }
            }
            if let Some(dialog) = s.dialog {
                self.character_dialogs.insert(handle, dialog);
            }
            if let Some(damage) = s.physics_damage {
                self.physics_damage.insert(handle, damage);
            }
            if let Some(b) = s.static_box {
                self.static_prop_boxes.insert(handle, b);
            }
            if matches!(entity.kind, EntityKind::Physics { .. }) {
                *self.physics_generations.entry(handle).or_default() += 1;
            }
            self.log.push(format!("saved entity restored {handle}"));
            let physics = matches!(entity.kind, EntityKind::Physics { .. }) && entity.added;
            self.entities.push(entity);
            // AddSavedEntities -> EntityManager.AddEntity -> OnSpawn.
            if physics {
                self.attach_static_prop(handle);
            }
        }
    }
}
