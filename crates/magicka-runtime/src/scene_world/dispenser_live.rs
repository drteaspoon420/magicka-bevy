//! Scene host for native dispenser actions and finite NPC production.
use super::*;
use crate::dispenser::{Dispenser, DispenserAction, DispenserCommand, CLIPS, MODELS};
use crate::spawn_action::SpawnOrder;

impl<C: SceneContent> SceneWorld<C> {
    pub(super) fn live_dispenser_action(
        &mut self,
        action: &DispenserAction,
    ) -> Result<(), WorldError> {
        match &action.command {
            DispenserCommand::Spawn {
                model,
                area,
                types,
                amounts,
                interval,
                active,
                ..
            } => {
                let path = MODELS
                    .get(*model)
                    .ok_or("dispenser model index outside native table")?;
                let types = types.clone().ok_or("SpawnDispenser has no Types")?;
                let amounts = amounts.clone().ok_or("SpawnDispenser has no Amount")?;
                for name in &types {
                    SpawnLevel::load_character_template(self, Some(name))?;
                }
                let mut transform = self.scene_locator(*area)?;
                let ground = Segment {
                    origin: [transform[12], transform[13] + 1.0, transform[14]],
                    delta: [0.0, -4.0, 0.0],
                };
                if let Some(hit) = self.collision.collision().segment_intersect(ground, false) {
                    transform[12..15].copy_from_slice(&hit.hit.position);
                }
                transform[13] += 1.75;
                // The constructor never loads idle, although the enum reserves slot 2.
                let durations = std::array::from_fn(|i| {
                    if i == 2 {
                        None
                    } else {
                        self.content.clip_duration(path, CLIPS[i])
                    }
                });
                let dispenser = Dispenser::new(
                    *model, transform, types, amounts, *interval, *active, durations,
                );
                let handle = self.new_handle();
                self.entities.push(SceneEntity {
                    handle,
                    unique_id: 0,
                    kind: EntityKind::Dispenser,
                    position: dispenser.position(),
                    orientation: transform,
                    hit_points: 500.0,
                    max_hit_points: 500.0,
                    original_faction: 0,
                    faction: 0,
                    radius: 1.25,
                    length: 1.0,
                    template_name: None,
                    animation: 0,
                    special_idle: 0,
                    undying: false,
                    cannot_die_without_explicit_kill: false,
                    dead: false,
                    added: true,
                    removed: false,
                    order: None,
                    event_count: 0,
                });
                self.dispensers.insert(handle, dispenser);
                self.log.push(format!("dispenser {handle} model={model}"));
            }
            DispenserCommand::Activate {
                id, model, area, ..
            } => {
                if let Some(id) = id {
                    if let Some(handle) = (*id != 0)
                        .then(|| self.by_unique_id(*id))
                        .flatten()
                        .map(|e| e.handle)
                    {
                        if let Some(d) = self.dispensers.get_mut(&handle) {
                            d.active = true;
                        }
                    }
                } else {
                    // Native area loop looks up ID 0 repeatedly instead of using
                    // each area's entity handle. Initialize() registers no ID.
                    self.presence.get(*area).map_err(|e| format!("{e:?}"))?;
                    // No script-created dispenser is registered as ID zero.
                    // Enumerating unnamed SceneEntity records here would turn
                    // the native no-op into an accidental last-entity match.
                    let _ = model;
                }
            }
        }
        Ok(())
    }

    pub(super) fn update_dispensers(&mut self, dt: f32) -> Result<(), WorldError> {
        let handles: Vec<_> = self.dispensers.keys().copied().collect();
        for handle in handles {
            if !self
                .entities
                .iter()
                .any(|e| e.handle == handle && e.added && !e.removed)
            {
                continue;
            }
            let rng = &mut self.rng;
            let (spawn, position, mut orientation, removable) = {
                let d = self.dispensers.get_mut(&handle).unwrap();
                let spawn = d.update(dt, |n| {
                    *rng ^= *rng << 13;
                    *rng ^= *rng >> 7;
                    *rng ^= *rng << 17;
                    let random = (*rng >> 40) as f32 / (1u64 << 24) as f32;
                    ((random * n as f32) as usize).min(n - 1)
                })?;
                (spawn, d.position(), d.transform, d.removable())
            };
            if let Some(name) = spawn {
                let npc = SpawnHost::acquire_npc(self)?.handle;
                let hp = SpawnHost::initialize_npc(self, npc, custom_hash(&name), 0, position, 0)?;
                orientation[12..15].fill(0.0);
                SpawnHost::configure(
                    self,
                    npc,
                    &SpawnConfig {
                        hit_points: Some(hp),
                        on_death: 0,
                        on_damage: 0,
                        wander_speed: None,
                        target_area: None,
                        order: SpawnOrder {
                            order: 2,
                            react_to: 0,
                            reaction: 0,
                            priority_target: 0,
                            priority_ability: -1,
                            trigger: 0,
                            events: &[],
                        },
                        dialog: 0,
                        orientation,
                        desired_direction: [-orientation[8], -orientation[9], -orientation[10]],
                        remove_after_death: true,
                        force_camera: None,
                        force_nav_mesh: None,
                        cannot_die_without_explicit_kill: None,
                    },
                )?;
                GiveOrderHost::go_to_animation(self, npc, 170, 0.001)?;
                SpawnHost::add_entity(self, npc)?;
            }
            let health = self.dispensers[&handle].health;
            let e = self.entity_mut(handle)?;
            e.hit_points = health;
            e.dead = health <= 0.0;
            if removable {
                e.removed = true;
                self.dispensers.remove(&handle);
            }
        }
        Ok(())
    }
}
