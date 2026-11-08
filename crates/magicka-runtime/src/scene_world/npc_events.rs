//! Character Death/OverKill event dispatch (authored template conditions).
use super::*;
use crate::spawn_action::{SpawnHost, SpawnLevel, SpawnOrder};
use magicka_content::common::Event;

/// A pickup created from a character: Release (animation drop) or
/// SpawnItemEvent (raised by the pickup box height, launched).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemRelease {
    pub name: String,
    pub transform: [f32; 16],
    pub velocity: [f32; 3],
    pub raise_by_height: bool,
    /// Item.Despawn time (0: permanent).
    pub despawn: f32,
}

impl<C: SceneContent> SceneWorld<C> {
    pub(super) fn execute_npc_lifecycle_events(&mut self) {
        let mut requests = Vec::new();
        for (&handle, npc) in &mut self.npcs {
            let flags = if npc.character.combat.overkilled {
                64
            } else if npc.dead() && !npc.motion.drowning {
                32
            } else {
                0
            };
            if flags == 0 || npc.lifecycle_dispatched & flags != 0 {
                continue;
            }
            npc.lifecycle_dispatched |= flags;
            for collection in npc.definition.event_conditions.iter() {
                if collection.condition.condition_type & flags != 0
                    && collection.condition.time <= 0.0
                {
                    requests.extend(collection.events.iter().cloned().map(|e| (handle, e)));
                }
            }
        }
        let owners: Vec<_> = requests.iter().map(|(h, _)| *h).collect();
        for (handle, event) in requests {
            if let Err(error) = self.npc_lifecycle_event(handle, event) {
                self.log.push(format!("NPC event failed: {error}"));
            }
        }
        for handle in owners {
            if self
                .entities
                .iter()
                .any(|e| e.handle == handle && e.removed)
            {
                self.disable_npc(handle, true);
            }
        }
    }
    fn npc_lifecycle_event(&mut self, handle: u16, event: Event) -> Result<(), WorldError> {
        let npc = self.npcs.get(&handle).ok_or("missing event owner")?;
        let body = &self.physics.bodies[npc.body];
        let position = body.position;
        let o = body.orientation;
        let direction = body.direction();
        let body_velocity = body.velocity;
        match event {
            Event::Remove { .. } => {
                // RemoveEvent calls Character.Terminate(killItems: true,
                // achievement: false), rather than the ordinary gib death.
                self.terminate_character(handle);
                self.entity_mut(handle)?.removed = true;
                self.physics.bodies[self.npcs[&handle].body].active = false;
            }
            Event::Spawn {
                character,
                idle_animation,
                spawn_animation,
                health,
                order,
                react_to,
                reaction,
                rotation_deg,
                offset,
            } => {
                self.load_character_template(Some(&character))?;
                let (s, c) = rotation_deg.to_radians().sin_cos();
                let right: [f32; 3] = std::array::from_fn(|i| o.right[i] * c - o.backward[i] * s);
                let backward: [f32; 3] =
                    std::array::from_fn(|i| o.right[i] * s + o.backward[i] * c);
                let point = std::array::from_fn(|i| {
                    position[i]
                        + offset[0] * right[i]
                        + offset[1] * o.up[i]
                        + offset[2] * backward[i]
                });
                let instance = self.acquire_npc()?.handle;
                // SpawnEvent.Initialize accepts the owner's capsule position as
                // its input; Character.Initialize adds the new height offset.
                let max_hp = self.initialize_npc(instance, custom_hash(&character), 0, point, 0)?;
                let mut orientation = magicka_content::skinned::IDENTITY;
                orientation[..3].copy_from_slice(&right);
                orientation[4..7].copy_from_slice(&o.up);
                orientation[8..11].copy_from_slice(&backward);
                self.configure(
                    instance,
                    &SpawnConfig {
                        hit_points: Some(max_hp * health),
                        on_death: 0,
                        on_damage: 0,
                        wander_speed: None,
                        target_area: None,
                        order: SpawnOrder {
                            order,
                            react_to,
                            reaction,
                            priority_target: 0,
                            priority_ability: 0,
                            trigger: 0,
                            events: &[],
                        },
                        dialog: 0,
                        orientation,
                        desired_direction: backward.map(|v| -v),
                        remove_after_death: true,
                        force_camera: None,
                        force_nav_mesh: None,
                        cannot_die_without_explicit_kill: None,
                    },
                )?;
                let spawn =
                    crate::animation_controller::animation_value(&spawn_animation).unwrap_or(0);
                if !matches!(spawn, 0 | 1 | 5) {
                    self.begin_resurrection(instance, spawn)?;
                }
                let idle =
                    crate::animation_controller::animation_value(&idle_animation).unwrap_or(0);
                if idle != 0 {
                    SpawnHost::set_special_idle(self, instance, idle)?;
                }
                SpawnHost::add_entity(self, instance)?;
            }
            Event::Splash { damage, radius } => {
                self.arc_damage(
                    Some(handle),
                    Some(handle),
                    position,
                    [0., 0., -1.],
                    radius,
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
            Event::Effect {
                effect,
                follow: _,
                world_align,
            } => {
                self.missile_visuals
                    .push(crate::missiles::MissileVisual::Effect {
                        missile: 0,
                        name: crate::missiles::intern(&effect.to_lowercase()),
                        position,
                        direction: if world_align { [0., 1., 0.] } else { direction },
                        follow: false,
                    });
            }
            Event::Sound { bank, sound, .. } => {
                self.play_cue(bank as u16, &sound, Some(position));
            }
            Event::SpawnMissile {
                missile, velocity, ..
            } => self.spawn_event_missile(handle, &missile, position, velocity)?,
            Event::SpawnGibs { start, end } => {
                let npc = &self.npcs[&handle];
                let selected: Vec<_> = npc
                    .definition
                    .gibs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i >= start as usize && *i <= end as usize)
                    .map(|(_, g)| g.clone())
                    .collect();
                let source = crate::gibs::GibSource {
                    position,
                    capsule_radius: npc.definition.radius,
                    capsule_length: npc.definition.length,
                    radius: npc.definition.radius,
                    mass: npc.definition.mass,
                    frozen: npc
                        .character
                        .combat
                        .statuses
                        .has(magicka_sim::status::Status::Frozen),
                    blood: npc.definition.blood,
                };
                self.gib_visuals
                    .extend(self.gibs.spawn_event(source, body_velocity, &selected));
            }
            Event::SpawnItem { item } => {
                // SpawnItemEvent.Execute (host only; "random" spawns nothing).
                let item = item.to_lowercase();
                if self.role != NetworkRole::Client && item != "random" {
                    use crate::spawn_item::SpawnItemHost;
                    self.load_item(&format!("Data/Items/Wizard/{item}"))?;
                    let velocity = self.event_launch_velocity(2.0);
                    let mut transform = matrix(o);
                    transform[12..15].copy_from_slice(&position);
                    self.npc_item_releases.push(ItemRelease {
                        name: item,
                        transform,
                        velocity,
                        raise_by_height: true,
                        despawn: self.luggage_despawn.get(&handle).copied().unwrap_or(0.0),
                    });
                }
            }
            Event::SpawnMagick { magick } => {
                // SpawnMagickEvent: "random" reads as Revive.
                let m = if magick.eq_ignore_ascii_case("random") {
                    crate::ruleset::REVIVE
                } else {
                    crate::magick_books::magick_id(&magick)
                        .ok_or_else(|| format!("unknown magick {magick}"))?
                };
                let radius = self.npcs[&handle].definition.radius;
                let despawn = self.luggage_despawn.get(&handle).copied().unwrap_or(0.0);
                self.spawn_event_book(
                    m,
                    [position[0], position[1] + radius, position[2]],
                    matrix(o),
                    despawn,
                    3.0,
                )?;
            }
            other => self
                .log
                .push(format!("NPC lifecycle event lacks host: {other:?}")),
        }
        Ok(())
    }
    fn spawn_event_missile(
        &mut self,
        owner: u16,
        name: &str,
        position: [f32; 3],
        velocity: [f32; 3],
    ) -> Result<(), WorldError> {
        let item = self
            .content
            .projectile_item(name)
            .ok_or_else(|| format!("missing projectile item {name}"))?;
        let (conditions, skipped) = crate::missiles::item_conditions(&item.conditions);
        for event in skipped {
            self.log
                .push(format!("NPC projectile event lacks host: {event}"));
        }
        let id = self.next_missile;
        self.next_missile = self.next_missile.wrapping_add(1);
        let mut missile = crate::missiles::LiveMissile::spawn_custom(
            id,
            owner,
            position,
            velocity,
            item.radius,
            false,
            conditions,
        );
        missile.body.item_model = item.model.clone();
        let mut visuals = Vec::new();
        self.with_rail_world(|world| {
            crate::missiles::initialize(&mut missile, &mut world.hit, &mut visuals)
        });
        self.missile_visuals.extend(visuals);
        self.register_missile(missile);
        Ok(())
    }
}

fn matrix(o: magicka_sim::character_physics::Orientation) -> [f32; 16] {
    let mut m = magicka_content::skinned::IDENTITY;
    m[..3].copy_from_slice(&o.right);
    m[4..7].copy_from_slice(&o.up);
    m[8..11].copy_from_slice(&o.backward);
    m
}

impl<C: SceneContent> SceneWorld<C> {
    /// The Spawn{Item,Magick}Event launch: ((r − ½)·s, r·7, (r − ½)·s).
    fn event_launch_velocity(&mut self, spread: f32) -> [f32; 3] {
        let x = (self.random() - 0.5) * spread;
        let y = self.random() * 7.0;
        let z = (self.random() - 0.5) * spread;
        [x, y, z]
    }

    /// SpawnMagickEvent.Execute (host only; also Avatar.DropMagicks): a
    /// movable book at `position` with the owner's orientation, launched
    /// upwards with horizontal `spread`, timing out after `timeout` (0 never).
    pub(crate) fn spawn_event_book(
        &mut self,
        magick: u8,
        position: [f32; 3],
        mut transform: [f32; 16],
        timeout: f32,
        spread: f32,
    ) -> Result<(), WorldError> {
        use crate::magick_books::SpawnMagickHost;
        if self.role == NetworkRole::Client {
            return Ok(());
        }
        let velocity = self.event_launch_velocity(spread);
        transform[12..15].copy_from_slice(&position);
        let action = crate::magick_books::SpawnMagickAction {
            delay: 0.0,
            area: 0,
            magick,
            immovable: false,
            timeout,
            unique_id: 0,
            on_pickup: 0,
        };
        let handle = self.spawn_book(&action, transform)?;
        let generation = self.book_pickups[&handle].generation;
        self.pickup_launches.push((handle, generation, velocity));
        Ok(())
    }
}
