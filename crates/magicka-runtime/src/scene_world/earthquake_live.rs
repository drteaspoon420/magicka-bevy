//! EarthQuake's live host. Static behavior and remaining parity gaps are
//! recorded in research/earthquake.md.
use super::*;
use magicka_sim::earthquake::{CharacterTarget, EarthquakeHost, Target};

/// A prop/pickup body supplied by the session's separate rigid-body solver.
#[derive(Clone, Copy, Debug)]
pub struct RigidTarget {
    pub key: magicka_physics::BodyKey,
    pub position: [f32; 3],
    pub radius: f32,
    pub mass: f32,
}

impl<C: SceneContent> SceneWorld<C> {
    pub(crate) fn earthquake_effect_instance(&self) -> Option<u64> {
        self.earthquake_effect
    }

    pub fn earthquake_from(&mut self, owner: u16) -> bool {
        let Some(body) = self.character_body(owner) else {
            return false;
        };
        self.cast_earthquake(Some(owner), self.physics.bodies[body].position)
    }

    pub fn earthquake_at(&mut self, position: [f32; 3]) -> bool {
        self.cast_earthquake(None, position)
    }

    fn cast_earthquake(&mut self, owner: Option<u16>, position: [f32; 3]) -> bool {
        let mut quake = std::mem::take(&mut self.earthquake);
        let result = quake.cast(owner, position, self);
        self.earthquake = quake;
        result
    }

    pub(super) fn update_earthquake(&mut self, dt: f32) {
        let mut quake = std::mem::take(&mut self.earthquake);
        quake.update(dt, self);
        self.earthquake = quake;
    }

    pub(super) fn clear_earthquake(&mut self) {
        let mut quake = std::mem::take(&mut self.earthquake);
        quake.remove(self);
        self.earthquake = quake;
    }

    /// Apply a received center without generating another outgoing action.
    pub fn receive_earthquake_center(&mut self, position: [f32; 3], duration: f32) {
        self.earthquake.quake_position = position;
        self.earthquake.quake_ttl = duration;
        self.camera.shake(0.5, duration);
    }
}

impl<C: SceneContent> EarthquakeHost for SceneWorld<C> {
    fn random(&mut self) -> f64 {
        f64::from(SceneWorld::random(self))
    }
    fn client(&self) -> bool {
        self.role == NetworkRole::Client
    }
    fn ground(&mut self, position: [f32; 3]) -> Option<[f32; 3]> {
        self.collision
            .collision()
            .segment_intersect(
                Segment {
                    origin: position,
                    delta: [0.0, -20.0, 0.0],
                },
                false,
            )
            .map(|hit| hit.hit.position)
    }
    fn targets(&mut self, position: [f32; 3], radius: f32) -> Vec<u16> {
        use magicka_sim::entity_query::{EntityQueryGrid, QueryEntity};
        let (mut entities, _) = self.spray_entities();
        for e in self.entities.iter().filter(|e| e.added && !e.removed) {
            let Some(body) = self.earthquake_rigid.get(&e.handle) else {
                continue;
            };
            entities.push(crate::spray_world::WorldEntity {
                handle: e.handle,
                position: body.position,
                radius: body.radius,
                dead: e.dead,
                damageable: matches!(e.kind, EntityKind::Physics { damageable: true }),
                frozen: false,
                geometry: crate::spray_world::ArcGeometry::Mine,
            });
        }
        let query: Vec<_> = entities
            .iter()
            .map(|e| QueryEntity {
                position: e.position,
                radius: e.radius,
                dead: e.dead,
                damageable: e.damageable,
                shield: matches!(e.geometry, crate::spray_world::ArcGeometry::Shield(_)),
            })
            .collect();
        match EntityQueryGrid::build(&query).and_then(|grid| {
            grid.query(&query, position, radius, false, false, |_, _| {
                (false, [0.0; 3])
            })
        }) {
            Ok(indices) => indices.into_iter().map(|i| entities[i].handle).collect(),
            Err(error) => {
                self.log.push(format!("earthquake: entity query {error:?}"));
                Vec::new()
            }
        }
    }
    fn target(&self, handle: u16) -> Option<Target> {
        if self.death.handle == Some(handle) {
            let actor = self
                .death
                .actor
                .as_ref()
                .filter(|actor| !actor.state.dead)?;
            return Some(Target {
                handle,
                position: actor.state.position,
                mass: crate::summon_death_actor::BODY_MASS,
                damageable: true,
                character: None,
            });
        }
        if let Some(body) = self.character_body(handle) {
            let c = self.character_ref(handle)?;
            let b = &self.physics.bodies[body];
            let in_event = self
                .players
                .iter()
                .flatten()
                .find(|p| p.handle == handle)
                .map(|p| p.ai.controls_avatar())
                .or_else(|| self.npcs.get(&handle).map(|n| n.ai.controls_avatar()))
                .unwrap_or(false);
            return Some(Target {
                handle,
                position: b.position,
                mass: b.mass,
                damageable: true,
                character: Some(CharacterTarget {
                    levitating: c.levitation.active(c.combat.dead),
                    immortal: c.is_immortal(),
                    ethereal: c.ethereal,
                    in_event,
                    grounded: b.is_touching_ground(),
                }),
            });
        }
        if let Some(b) = self.earthquake_rigid.get(&handle) {
            let e = self
                .entities
                .iter()
                .find(|e| e.handle == handle && e.added && !e.removed && !e.dead)?;
            return Some(Target {
                handle,
                position: b.position,
                mass: b.mass,
                damageable: matches!(e.kind, EntityKind::Physics { damageable: true }),
                character: None,
            });
        }
        // These native entity bodies use JigLibX.Body's default mass (1),
        // without a later SetMass. Their public damage paths remain distinct.
        let position = self
            .shields
            .iter()
            .find(|s| s.handle == handle)
            .map(|s| s.shield.position)
            .or_else(|| match handle & MINE_KEY {
                BARRIER_KEY => self
                    .barriers
                    .get((handle & !MINE_KEY) as usize)
                    .map(|b| b.position),
                MINE_KEY => self
                    .mines
                    .get((handle & !MINE_KEY) as usize)
                    .map(|m| m.position),
                GREASE_KEY => self
                    .grease
                    .get((handle & !MINE_KEY) as usize)
                    .map(|g| g.position),
                _ => None,
            })?;
        Some(Target {
            handle,
            position,
            mass: 1.0,
            damageable: true,
            character: None,
        })
    }
    fn impulse_velocity(&mut self, handle: u16, impulse: [f32; 3]) {
        if let Some(body) = self.character_body(handle) {
            self.physics.bodies[body].add_impulse_velocity(impulse);
        }
    }
    fn damage(
        &mut self,
        handle: u16,
        owner: Option<u16>,
        damage: magicka_sim::damage::Damage,
        timestamp: f64,
        point: [f32; 3],
    ) {
        if self.death.handle == Some(handle) {
            self.death
                .deliver(&[damage], self.role != NetworkRole::Client);
            return;
        }
        if self.earthquake_rigid.contains_key(&handle) {
            // DamageablePhysicsEntity.InternalDamage has no Knockdown branch;
            // pure Knockdown neither moves the body nor subtracts health.
            return;
        }
        magicka_sim::conflagration::World::damage(
            &mut ConflagrationWorld {
                world: self,
                instance: 0,
            },
            handle,
            damage,
            owner,
            timestamp,
            point,
        );
    }
    fn add_velocity(&mut self, handle: u16, velocity: [f32; 3]) {
        if let Some(body) = self.earthquake_rigid.get(&handle) {
            self.earthquake_velocities.push((body.key, velocity));
        }
    }
    fn new_quake(&mut self, owner: Option<u16>, position: [f32; 3], duration: f32) {
        self.camera.shake(0.5, duration);
        if self.role == NetworkRole::Server {
            self.earthquake_centers.push((owner, position, duration));
        }
    }
    fn pulse(&mut self, position: [f32; 3], scale: [f32; 3]) {
        self.stop_presentation();
        self.earthquake_cue = Some(self.play_cue(0x200, "magick_earthquake", Some(position)));
        self.effect_serial = self.effect_serial.wrapping_add(1);
        let instance = self.effect_serial;
        let mut transform = magicka_content::skinned::IDENTITY;
        transform[0] = scale[0];
        transform[5] = scale[1];
        transform[10] = scale[2];
        transform[12..15].copy_from_slice(&position);
        self.effect_commands.push(SceneEffectCommand::Start {
            instance,
            effect: custom_hash("magick_earthquake"),
            transform,
        });
        self.earthquake_effect = Some(instance);
    }
    fn stop_presentation(&mut self) {
        if let Some(cue) = self.earthquake_cue.take() {
            self.stop_cue(cue);
        }
        if let Some(instance) = self.earthquake_effect.take() {
            self.effect_commands
                .push(SceneEffectCommand::Stop { instance });
        }
    }
    fn failed(&mut self, position: [f32; 3]) {
        self.play_cue(16, "magick_fail", Some(position));
    }
}
