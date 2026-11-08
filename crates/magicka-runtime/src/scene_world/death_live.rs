//! SummonDeath singleton host; remaining discrepancies are in summon-death.md.
use super::*;
use crate::summon_death_actor::{DeathActor, DeathFrame};
use magicka_sim::summon_death::{self as sd, Candidate, MagickDeath, Target};

#[derive(Clone, Copy)]
struct Placement {
    target: u16,
    original: [f32; 3],
    previous: [f32; 3],
}

#[derive(Default)]
pub struct DeathHost {
    /// Allocated once and retained by the native singleton entity.
    pub handle: Option<u16>,
    pub actor: Option<DeathActor>,
    pub frame: Option<DeathFrame>,
    pending: Option<Placement>,
    slowdown: Option<u32>,
    pursuit: Option<u32>,
}
impl DeathHost {
    pub fn callback_capsule(&self) -> Option<crate::missiles::MissileCallbackCapsule> {
        let a = self.actor.as_ref().filter(|a| !a.state.dead)?;
        let axis = |position, orientation| {
            let m = magicka_content::skinned::pose_matrix([0.0; 3], orientation, [1.0; 3]);
            Segment {
                origin: position,
                delta: [m[4] * 1.2, m[5] * 1.2, m[6] * 1.2],
            }
        };
        Some(crate::missiles::MissileCallbackCapsule {
            solid: false,
            handle: self.handle?,
            old_axis: axis(a.previous_position, a.previous_orientation),
            new_axis: axis(a.state.position, a.body_orientation),
            radius: 0.6,
            old_body_position: a.previous_position,
        })
    }
    pub fn spray_entity(&self) -> Option<crate::spray_world::WorldEntity> {
        let a = self.actor.as_ref().filter(|a| !a.state.dead)?;
        Some(crate::spray_world::WorldEntity {
            handle: self.handle?,
            position: a.state.position,
            // MagickDeath inherits Entity.mRadius (zero); this differs from
            // the actual collision capsule radius and visual bounding sphere.
            radius: 0.0,
            dead: false,
            damageable: true,
            frozen: false,
            geometry: crate::spray_world::ArcGeometry::MagickDeath,
        })
    }

    pub fn deliver(
        &mut self,
        damages: &[magicka_sim::damage::Damage],
        damage_features: bool,
    ) -> bool {
        let Some(a) = self.actor.as_mut().filter(|a| !a.state.dead) else {
            return false;
        };
        let mut hit = false;
        for d in damages {
            hit |= a
                .state
                .receive_damage(magicka_sim::spells::Elements(d.element), damage_features);
        }
        hit
    }

    /// CollisionSkin.SegmentIntersect for the dedicated capsule's body pose.
    pub fn segment_fraction(&self, segment: Segment) -> Option<f32> {
        let a = self.actor.as_ref().filter(|a| !a.state.dead)?;
        let matrix = magicka_content::skinned::pose_matrix([0.0; 3], a.body_orientation, [1.0; 3]);
        magicka_sim::capsule_intersection::segment_fraction(
            segment,
            a.state.position,
            [matrix[4], matrix[5], matrix[6]],
            crate::summon_death_actor::CAPSULE_LENGTH,
            crate::summon_death_actor::CAPSULE_RADIUS,
        )
    }

    pub fn active(&self) -> bool {
        self.pending.is_some() || self.actor.as_ref().is_some_and(|a| !a.state.dead)
    }
}

impl<C: SceneContent> SceneWorld<C> {
    /// Blast.BlastDamage collection route for the dedicated non-character.
    pub(super) fn blast_death(
        &mut self,
        position: [f32; 3],
        radius: f32,
        mut damages: [magicka_sim::damage::Damage; 5],
    ) {
        let Some(target) = self.death.spray_entity() else {
            return;
        };
        if !within(target.position, position, radius) {
            return;
        }
        let mut segment = Segment {
            origin: position,
            delta: std::array::from_fn(|i| target.position[i] - position[i]),
        };
        if let Some(fraction) = self.death.segment_fraction(segment) {
            segment.delta = segment.delta.map(|v| v * fraction);
        }
        if self
            .collision
            .collision()
            .segment_intersect(segment, false)
            .is_some()
        {
            return;
        }
        let distance = segment.delta.iter().map(|v| v * v).sum::<f32>().sqrt();
        if distance <= radius {
            let beams = damages
                .iter()
                .any(|d| d.element & magicka_sim::spells::Elements::BEAMS.0 != 0);
            if !beams {
                let ratio = (distance / radius).max(0.0);
                let scale = 1.0 - ratio * ratio * ratio;
                for d in &mut damages {
                    d.magnitude *= scale;
                }
            }
            self.death
                .deliver(&damages, self.role != NetworkRole::Client);
        }
    }
    pub fn summon_death_from(&mut self, owner: u16) -> bool {
        if self.character_ref(owner).is_none() {
            return false;
        }
        self.cast_death(Some(owner))
    }
    /// Native position argument does not affect its camera-centered selection.
    pub fn summon_death_at(&mut self, _position: [f32; 3]) -> bool {
        self.cast_death(None)
    }

    fn cast_death(&mut self, owner: Option<u16>) -> bool {
        if self.death.active() {
            self.play_cue(16, "magick_fail", None);
            return false;
        }
        self.play_cue(16, "magick_summon_death", None);
        self.death.slowdown = Some(self.play_cue(16, "magick_timewarp", None));
        let center =
            std::array::from_fn(|i| self.camera.position()[i] - crate::camera::CAMERA_OFFSET[i]);
        let people = self.combatants();
        let records: Vec<_> = self
            .entities
            .iter()
            .filter(|e| e.added && !e.removed)
            .collect();
        let query: Vec<_> = records
            .iter()
            .map(|e| {
                let c = people.iter().find(|c| c.handle == e.handle);
                magicka_sim::entity_query::QueryEntity {
                    position: c.map_or(e.position, |c| c.position),
                    radius: c.map_or(e.radius, |c| c.radius),
                    dead: e.dead || c.is_some_and(|c| c.dead),
                    shield: false,
                    damageable: c.is_some(),
                }
            })
            .collect();
        let indices = magicka_sim::entity_query::EntityQueryGrid::build(&query)
            .ok()
            .and_then(|g| {
                g.query(&query, center, sd::QUERY_RANGE, false, false, |_, _| {
                    (false, [0.0; 3])
                })
                .ok()
            })
            .unwrap_or_default();
        let candidates: Vec<_> = indices
            .into_iter()
            .map(|i| {
                let e = records[i];
                let c = people.iter().find(|c| c.handle == e.handle);
                Candidate {
                    handle: e.handle,
                    character: c.is_some(),
                    dead: query[i].dead,
                    undead: c.is_some_and(|c| c.faction & 0x10 != 0),
                    hit_points: c.map_or(e.hit_points, |c| c.hit_points),
                    max_hit_points: c.map_or(e.max_hit_points, |c| c.max_hit_points),
                }
            })
            .collect();
        let start = (self.random() * candidates.len() as f32) as usize;
        let Some(target) = sd::select_target(&candidates, start, owner) else {
            self.play_cue(16, "magick_fail", None);
            return false;
        };
        let Some(body) = self.character_body(target) else {
            return false;
        };
        let original = self.physics.bodies[body].position;
        self.death.pending = Some(Placement {
            target,
            original,
            previous: original,
        });
        self.advance_death_placement();
        true
    }

    /// Continue cumulative placement across frames instead of hanging in the
    /// native loop whose nominal attempt counter is never incremented.
    fn advance_death_placement(&mut self) {
        let Some(mut p) = self.death.pending.take() else {
            return;
        };
        let point = sd::placement_step(p.previous, self.random() as f64);
        let mut point = self
            .nav
            .as_ref()
            .and_then(|n| n.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
            .map_or(point, |(_, q, _)| q);
        if !sd::placement_accepted(point, p.original) {
            p.previous = point;
            self.death.pending = Some(p);
            return;
        }
        // Initialize derives its direction before the six-metre ground snap.
        let direction = std::array::from_fn::<_, 3, _>(|i| p.original[i] - point[i]);
        let inverse = direction.iter().map(|v| v * v).sum::<f32>().sqrt().recip();
        let orientation = crate::summon_death_actor::turn_orientation(
            [0.0, 0.0, 0.0, 1.0],
            direction.map(|v| v * inverse),
            1.0,
            1.0,
        );
        if let Some(hit) = self.collision.collision().segment_intersect(
            Segment {
                origin: [point[0], point[1] + 1.0, point[2]],
                delta: [0.0, -6.0, 0.0],
            },
            false,
        ) {
            point = hit.hit.position;
        }
        let Some(body) = self.character_body(p.target) else {
            return;
        };
        let state = MagickDeath::new(p.target, point, self.physics.bodies[body].radius);
        if let Some(a) = &mut self.death.actor {
            a.reinitialize(state, orientation);
        } else {
            let Some(asset) = self.content.skinned_model(sd::MODEL) else {
                self.log.push("summon death: missing model".into());
                return;
            };
            match DeathActor::new(asset, state, orientation) {
                Ok(a) => self.death.actor = Some(a),
                Err(e) => {
                    self.log.push(format!("summon death: {e}"));
                    return;
                }
            }
        }
        if self.death.handle.is_none() {
            self.death.handle = Some(self.new_handle());
        }
        self.death.pursuit = Some(self.play_cue(0x20, "boss_death_proximity", Some(point)));
        self.mine_effects
            .push(("death_ethereal_spawn", point, [0.0, 0.0, -1.0]));
    }

    pub fn clear_death(&mut self) {
        self.death.frame = None;
        if !self.death.active() && self.death.slowdown.is_none() && self.death.pursuit.is_none() {
            return;
        }
        self.death.pending = None;
        if let Some(a) = &mut self.death.actor {
            a.state.remove(&mut self.time_modifier);
        }
        for cue in [self.death.slowdown.take(), self.death.pursuit.take()]
            .into_iter()
            .flatten()
        {
            self.stop_cue(cue);
        }
        self.render_saturation = Some(1.0);
    }

    pub(super) fn update_death(&mut self, entity_dt: f32) {
        self.advance_death_placement();
        let Some(mut a) = self.death.actor.take() else {
            return;
        };
        if a.state.dead {
            // A naturally removed entity's last queued draw survives only
            // until the next entity-update phase. Nullify clears immediately.
            self.death.frame = None;
            self.death.actor = Some(a);
            return;
        }
        let dt = a
            .state
            .time_step(entity_dt, &mut self.time_modifier, self.time_multiplier);
        let target = self.character_body(a.state.target).and_then(|b| {
            self.character_ref(a.state.target).map(|c| Target {
                position: self.physics.bodies[b].position,
                dead: c.combat.dead,
            })
        });
        let ground = self
            .collision
            .collision()
            .segment_intersect(a.ground_probe(dt), false)
            .map(|h| h.hit.position);
        self.death.frame = None;
        match a.update(dt, ground, target) {
            Ok(frame) => {
                let at = a.state.position;
                if frame.events.swing_cue {
                    self.play_cue(0x80, "wep_death_scythe_swing", Some(at));
                }
                if frame.events.despawn_effect_and_cue {
                    self.play_cue(0x20, "boss_death_death", Some(at));
                    self.mine_effects
                        .push(("death_ethereal_despawn", at, [0.0, 0.0, -1.0]));
                }
                if let Some(cue) = self.death.pursuit {
                    self.move_cue(cue, at);
                }
                if let Some((d, speed)) = frame.events.turn {
                    a.orientation =
                        crate::summon_death_actor::turn_orientation(a.orientation, d, speed, dt);
                }
                if frame.events.scythe_window {
                    let seg = frame.scythe_segment;
                    for c in self.combatants() {
                        let flat =
                            (c.position[0] - seg.origin[0]).hypot(c.position[2] - seg.origin[2]);
                        if flat <= 3.0 + c.radius
                            && a.can_hit(c.handle)
                            && crate::npc_combat::capsule_segment_hit(
                                &c, seg.origin, seg.delta, 0.0,
                            )
                        {
                            self.death_scythe_damage(c.handle, c.hit_points);
                            a.record_hit(c.handle);
                            self.play_cue(0x80, "wep_death_scythe", Some(at));
                        }
                    }
                }
                self.death.frame = Some(frame);
                self.render_saturation = Some(0.5);
            }
            Err(e) => {
                self.log.push(format!("summon death update: {e}"));
                a.state.dead = true;
            }
        }
        let dead = a.state.dead;
        self.death.actor = Some(a);
        if dead {
            // Native Update submits body/scythe before its target-death gate
            // and before Disappear finishes. Deinitialize must not cancel
            // those already queued transforms for this presentation frame.
            let final_frame = self.death.frame.take();
            self.clear_death();
            self.death.frame = final_frame;
        }
    }

    pub(super) fn death_scythe_damage(&mut self, target: u16, amount: f32) {
        let client = self.role == NetworkRole::Client;
        let avatar = self.players.iter().flatten().any(|p| p.handle == target);
        let Some(body) = self.character_body(target) else {
            return;
        };
        let entangled = self.physics.bodies[body].owner.entangled;
        let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, target) else {
            return;
        };
        let ctx = magicka_sim::character_health::HealthContext {
            immortal: c.is_immortal(),
            network_client: client,
            avatar,
            entangled,
            ..Default::default()
        };
        let mut events = Vec::new();
        c.combat.health.damage(
            amount,
            magicka_sim::spells::Elements::ARCANE,
            c.combat.statuses.current_mask,
            ctx,
            &mut events,
        );
        self.physics.bodies[body].owner.hit_points = c.combat.health.hit_points;
    }

    /// The dedicated entity's InternalDamage API. General spell/collision
    /// queries still need to route MagickDeath contacts to this entry point.
    pub fn receive_death_damage(
        &mut self,
        element: magicka_sim::spells::Elements,
        damage_features: bool,
    ) -> bool {
        self.death
            .actor
            .as_mut()
            .filter(|a| !a.state.dead)
            .is_some_and(|a| a.state.receive_damage(element, damage_features))
    }
}
