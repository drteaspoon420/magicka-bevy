//! Summon Elemental scene host; physics/spell/viewer/network gaps are recorded
//! in research/summon-elemental.md. Eggs retain dedicated stable identities.
use super::*;
use crate::elemental_egg_actor::{EggHatch, ElementalEggActor, ElementalEggCache};
use magicka_sim::elemental_egg as egg;

impl<C: SceneContent> crate::spawn_elemental::SpawnElementalHost for SceneWorld<C> {
    type Error = WorldError;
    fn is_client(&self) -> bool {
        self.role == NetworkRole::Client
    }

    fn elemental_dead(&self, handle: u16) -> bool {
        self.elemental_eggs
            .as_ref()
            .and_then(|cache| cache.slots.iter().find(|s| s.handle == handle))
            .is_none_or(|s| !s.actor.registered || s.actor.state.dead())
    }

    fn spawn_elemental(
        &mut self,
        action: &crate::spawn_elemental::SpawnElementalAction,
        proximity: Option<f32>,
    ) -> Result<u16, WorldError> {
        self.ensure_elemental_cache()?;
        let locator = self.scene_locator(action.area)?;
        let mut point = [locator[12], locator[13], locator[14]];
        if action.snap_to_navmesh {
            point = self
                .nav
                .as_ref()
                .and_then(|nav| nav.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
                .ok_or("script elemental placement has no Default navmesh point")?
                .1;
        }
        let direction = [-locator[8], -locator[9], -locator[10]];
        let ground = self
            .collision
            .collision()
            .segment_intersect(ElementalEggActor::ground_probe(point), false)
            .map(|hit| hit.hit.position);
        let slot = self.elemental_eggs.as_mut().unwrap().acquire();
        if let Some(proximity) = proximity {
            slot.actor.state.proximity = proximity;
        }
        slot.actor.initialize(point, direction, ground);
        slot.frame = None;
        let handle = slot.handle;
        let record = SceneEntity {
            handle,
            unique_id: action.unique_id,
            kind: EntityKind::ElementalEgg,
            position: slot.actor.position,
            orientation: slot.actor.orientation,
            hit_points: 0.,
            max_hit_points: 0.,
            original_faction: 0,
            faction: 0,
            radius: slot.actor.geometry.radius,
            length: slot.actor.geometry.length,
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
        };
        if let Some(entity) = self.entities.iter_mut().find(|e| e.handle == handle) {
            *entity = record;
        } else {
            self.entities.push(record);
        }
        // Native server TriggerActionMessage translation remains separate from
        // the rewrite's transport. Do not substitute a summon-magick request.
        Ok(handle)
    }
}

fn template(element: magicka_sim::spells::Elements) -> Option<&'static str> {
    use magicka_sim::spells::Elements as E;
    Some(match element {
        E::WATER => "Elemental_Water",
        E::COLD => "Elemental_Cold",
        E::FIRE => "Elemental_Fire",
        E::LIGHTNING => "Elemental_Lightning",
        E::ARCANE => "Elemental_Arcane",
        E::LIFE => "Elemental_Life",
        E::STEAM => "Elemental_Steam",
        E::POISON => "Elemental_Poison",
        _ => return None,
    })
}

impl<C: SceneContent> SceneWorld<C> {
    /// Collection BlastDamage intersects the Body skin, not the egg's
    /// closest-point SegmentIntersect helper, before scene occlusion/falloff.
    pub(super) fn blast_elemental_eggs(
        &mut self,
        position: [f32; 3],
        radius: f32,
        damages: [magicka_sim::damage::Damage; 5],
    ) {
        if self.role == NetworkRole::Client {
            return;
        }
        let Some(cache) = self.elemental_eggs.as_mut() else {
            return;
        };
        for slot in cache
            .slots
            .iter_mut()
            .filter(|slot| slot.actor.registered && !slot.actor.state.dead())
        {
            let actor = &mut slot.actor;
            if !within(actor.position, position, radius + actor.geometry.radius) {
                continue;
            }
            let mut segment = Segment {
                origin: position,
                delta: std::array::from_fn(|i| actor.position[i] - position[i]),
            };
            // Constructor: capsule at local zero, rotated -pi/2 around X.
            // Its Backward axis becomes the body's Up, with no half-length
            // translation (unlike the centred Character capsule).
            let axis = [
                actor.orientation[4],
                actor.orientation[5],
                actor.orientation[6],
            ];
            if let Some(fraction) = magicka_sim::capsule_intersection::segment_fraction(
                segment,
                actor.position,
                axis,
                actor.geometry.length,
                actor.geometry.radius,
            ) {
                segment.delta = segment.delta.map(|v| v * fraction);
            }
            if self
                .collision
                .collision()
                .segment_intersect(segment, false)
                .is_some()
            {
                continue;
            }
            let distance =
                segment.delta.iter().map(|v| v * v).sum::<f32>().sqrt() - actor.geometry.radius;
            if distance <= radius {
                for damage in magicka_sim::blast_damage::collection(damages, distance, radius) {
                    actor.state.internal_damage(damage);
                }
            }
        }
    }

    fn ensure_elemental_cache(&mut self) -> Result<(), WorldError> {
        if self.elemental_eggs.is_some() {
            return Ok(());
        }
        for element in egg::RANDOM_ELEMENTS {
            self.load_character_template(template(element))?;
        }
        let geometry = self
            .content
            .elemental_egg_geometry()
            .ok_or("missing Fire elemental geometry")?;
        let mesh = self
            .content
            .skinned_model(egg::MODEL)
            .ok_or("missing elemental mesh")?;
        let animation = self
            .content
            .skinned_model(egg::ANIMATION_MODEL)
            .ok_or("missing elemental animation")?;
        let handles = std::array::from_fn(|_| self.new_handle());
        self.elemental_eggs = Some(Box::new(ElementalEggCache::new(
            mesh, animation, geometry, handles,
        )?));
        Ok(())
    }

    pub fn summon_elemental_from(&mut self, owner: u16) -> Result<Option<u16>, WorldError> {
        let body = self
            .character_body(owner)
            .ok_or("elemental caster is not a character")?;
        self.cast_elemental(
            Some(owner),
            self.physics.bodies[body].position,
            self.physics.bodies[body].direction(),
        )
    }

    pub fn summon_elemental_at(&mut self, position: [f32; 3]) -> Result<Option<u16>, WorldError> {
        if self.role == NetworkRole::Server {
            return Err(
                "native ownerless server cast has no owner handle for SpawnElemental".into(),
            );
        }
        let direction = egg::random_direction(f64::from(self.random()), f64::from(self.random()));
        self.cast_elemental(None, position, direction)
    }

    fn cast_elemental(
        &mut self,
        owner: Option<u16>,
        position: [f32; 3],
        direction: [f32; 3],
    ) -> Result<Option<u16>, WorldError> {
        let mut point = egg::cast_point(position, direction);
        let handle = if self.role == NetworkRole::Client {
            None
        } else {
            self.ensure_elemental_cache()?;
            // Rewrite extension: hosted scenes (PlaySession::hosted) carry an
            // empty navmesh; the egg then lands at the cast point.
            let empty_nav = self.nav.as_ref().is_none_or(|nav| nav.is_empty());
            point = match self
                .nav
                .as_ref()
                .and_then(|nav| nav.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
            {
                Some((_, p, _)) => p,
                None if empty_nav => point,
                None => return Err("elemental placement has no Default navmesh point".into()),
            };
            let ground = self
                .collision
                .collision()
                .segment_intersect(ElementalEggActor::ground_probe(point), false)
                .map(|hit| hit.hit.position);
            let slot = self.elemental_eggs.as_mut().unwrap().acquire();
            slot.actor.initialize(point, direction, ground);
            slot.actor.state.summoner = owner;
            slot.frame = None;
            let handle = slot.handle;
            // Reuse from the shared cache replaces a previous script identity.
            self.entities
                .retain(|e| e.handle != handle || !matches!(e.kind, EntityKind::ElementalEgg));
            Some(handle)
        };
        self.mine_effects
            .push(("magick_summonelemental", point, direction));
        self.play_cue(16, "magick_summon_elemental", Some(position));
        Ok(handle)
    }

    /// Dedicated egg collection route; general spell/contact adapters follow.
    pub fn damage_elemental_egg(
        &mut self,
        handle: u16,
        damages: &[magicka_sim::damage::Damage],
    ) -> bool {
        let Some(slot) = self.elemental_eggs.as_mut().and_then(|cache| {
            cache
                .slots
                .iter_mut()
                .find(|slot| slot.handle == handle && slot.actor.registered)
        }) else {
            return false;
        };
        let mut hit = false;
        for &damage in damages {
            hit |= slot.actor.state.internal_damage(damage);
        }
        hit
    }

    pub(super) fn update_elemental_eggs(&mut self, dt: f32) {
        let Some(mut cache) = self.elemental_eggs.take() else {
            return;
        };
        let avatars: Vec<_> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.playing && !p.dead())
            .map(|p| self.physics.bodies[p.body].position)
            .collect();
        let mut hatches = Vec::new();
        for slot in &mut cache.slots {
            if self
                .entities
                .iter()
                .any(|e| e.handle == slot.handle && e.removed)
            {
                // Remove deinitializes the native egg just as death does.
                if let Some(hatch) = slot.actor.deinitialize(self.role == NetworkRole::Client) {
                    hatches.push(hatch);
                }
            }
            if !slot.actor.registered {
                slot.frame = None;
                continue;
            }
            // Native Kill/OverKill is removable without another egg Update.
            if slot.actor.state.dead() {
                if let Some(hatch) = slot.actor.deinitialize(self.role == NetworkRole::Client) {
                    hatches.push(hatch);
                }
                slot.frame = None;
                continue;
            }
            let proximity = slot.actor.state.proximity;
            let nearby = avatars
                .iter()
                .any(|position| within(*position, slot.actor.position, proximity));
            slot.frame = slot.actor.update(dt, nearby, |bound| {
                ((self.random() * bound as f32) as usize).min(bound - 1)
            });
            if slot.actor.state.dead() {
                if let Some(hatch) = slot.actor.deinitialize(self.role == NetworkRole::Client) {
                    hatches.push(hatch);
                }
            }
        }
        for entity in &mut self.entities {
            if !matches!(entity.kind, EntityKind::ElementalEgg) {
                continue;
            }
            if let Some(slot) = cache.slots.iter().find(|s| s.handle == entity.handle) {
                entity.position = slot.actor.position;
                entity.orientation = slot.actor.orientation;
                entity.dead = slot.actor.state.dead();
                entity.removed = !slot.actor.registered;
            }
        }
        self.elemental_eggs = Some(cache);
        for hatch in hatches {
            self.hatch_elemental(hatch);
        }
    }

    fn hatch_elemental(&mut self, hatch: EggHatch) {
        let Some(template) = template(hatch.element) else {
            return;
        };
        let master = hatch
            .summoner
            .filter(|owner| self.character_ref(*owner).is_some());
        let direction = [
            -hatch.orientation[8],
            -hatch.orientation[9],
            -hatch.orientation[10],
        ];
        let handle = self.summon_npc_animation(
            master,
            template,
            hatch.position,
            direction,
            magicka_sim::summons::SummonKind::Normal,
            crate::spawn_action::SpawnOrder {
                order: 2,
                react_to: 3,
                reaction: 2,
                priority_target: -1,
                priority_ability: -1,
                trigger: 0,
                events: &[],
            },
            214,
        );
        if handle.is_none() {
            self.log.push(format!("elemental hatch failed: {template}"));
        }
    }
}

#[cfg(test)]
mod blast_tests {
    use super::*;
    use magicka_content::level::TriMesh;
    use magicka_sim::{damage::Damage, spells::Elements as E};

    #[test]
    fn egg_blast_skin_falloff_beam_exemption_occlusion_and_authority() {
        let root = magicka_content::retail_content_dir();
        if !root.is_dir() {
            return;
        }
        let mut s = crate::play_session::PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.disable_scripts();
        let owner = s.avatar().handle;
        s.world.summon_elemental_from(owner).unwrap();
        let r = s.world.elemental_eggs.as_ref().unwrap().slots[0]
            .actor
            .geometry
            .radius;
        // At 4r from a 4r blast, skin clipping plus radius subtraction gives
        // distance 2r: Fire 100 becomes 87.5. Competing Cold memories bracket
        // that value, so the selection detects either missing clipping/falloff.
        for (cold, beams, distance, wall, client, expected) in [
            (80., false, 4., None, false, E::FIRE),
            (90., false, 4., None, false, E::COLD),
            (90., true, 4., None, false, E::FIRE),
            (80., false, 8., None, false, E::COLD),
            (80., false, 4., Some(2.), false, E::COLD),
            (80., false, 4., Some(3.5), false, E::FIRE),
            (80., false, 4., None, true, E::COLD),
        ] {
            let empty = TriMesh {
                vertices: vec![],
                triangles: vec![],
            };
            let mut layers = vec![empty; 10];
            if let Some(x) = wall {
                layers[0] = TriMesh {
                    vertices: vec![[x * r, 90., -10.], [x * r, 110., -10.], [x * r, 100., 10.]],
                    triangles: vec![[0, 1, 2]],
                };
            }
            let collision = s.world.collision.collision_mut();
            collision.static_skin =
                crate::level_collision::StaticLevelCollision::from_layers(&layers).unwrap();
            collision.force_fields.clear();
            collision.animated_parts.clear();
            let actor = &mut s.world.elemental_eggs.as_mut().unwrap().slots[0].actor;
            actor.initialize([distance * r, 100. - r, 0.], [0., 0., 1.], None);
            actor.state.internal_damage(Damage {
                element: E::COLD.0,
                amount: cold,
                magnitude: 1.,
                ..Default::default()
            });
            let mut damages = [Damage::default(); 5];
            damages[0] = Damage {
                element: E::FIRE.0,
                amount: 100.,
                magnitude: 1.,
                ..Default::default()
            };
            if beams {
                damages[1].element = E::ARCANE.0;
            }
            s.world.role = if client {
                NetworkRole::Client
            } else {
                NetworkRole::Offline
            };
            s.world
                .entity_blasts
                .push(EntityBlast::Full([0., 100., 0.], 4. * r, damages));
            s.world.apply_entity_blasts();
            let mut state = s.world.elemental_eggs.as_ref().unwrap().slots[0]
                .actor
                .state
                .clone();
            state.update(0., 0., false, |_| unreachable!());
            assert_eq!(
                state.spawn_element, expected,
                "cold={cold} beams={beams} distance={distance} wall={wall:?} client={client}"
            );
        }
    }
}
