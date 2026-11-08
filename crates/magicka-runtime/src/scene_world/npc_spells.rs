//! Live NPC spell effects and owner-independent scene dispatch.
use super::*;
use crate::avatar::AvatarEvent as E;
impl<C: SceneContent> SceneWorld<C> {
    pub(super) fn update_npc_spell(
        &mut self,
        npc: &mut crate::npc::NpcCharacter,
        dt: f32,
        combatants: Vec<crate::npc_combat::Combatant>,
    ) {
        if npc.current_spell.is_some() {
            let (mut targets, _) = self.spray_entities();
            let shield_order: Vec<u16> = self.shields.iter().map(|s| s.handle).collect();
            let (bodies_before, bodies_after) = self.physics.bodies.split_at_mut(npc.body);
            let (caster_body, bodies_after) = bodies_after.split_first_mut().unwrap();
            let mut effects = SceneSprayEffects {
                missiles: &mut self.missiles,
                physics_damage: &mut self.physics_damage,
                dispensers: &mut self.dispensers,
                entities: &mut self.entities,
                elemental_eggs: &mut self.elemental_eggs,
                caster_boosts: npc.character.buffs.damage_boosts(),
                bodies_before,
                bodies_after,
                caster_index: npc.body,
                combatants,
                npcs: &mut self.npcs,
                players: &mut self.players,
                triggers: &mut self.pending_triggers,
                shields: &mut self.shields,
                mines: &mut self.mines,
                grease: &mut self.grease,
                barriers: &mut self.barriers,
                death: &mut self.death,
                jormungandr: &mut self.jormungandr,
                death_damage_features: self.role != NetworkRole::Client,
                barrier_events: Vec::new(),
            };
            match crate::spray_world::RuntimeSprayWorld::new(
                Some(self.collision.collision()),
                &mut targets,
                &shield_order,
                &mut effects,
            ) {
                Ok(world) => {
                    let mut world = world.with_rails(&mut self.rails);
                    npc.cast_update(caster_body, dt, &mut world)
                }
                Err(e) => self.log.push(format!("spray world: {e:?}")),
            }
            let pending = std::mem::take(&mut effects.barrier_events);
            for (slot, events) in pending {
                self.barrier_events(slot, events);
            }
        } else {
            npc.cast_update(
                &mut self.physics.bodies[npc.body],
                dt,
                &mut crate::spray_world::NoSprayWorld,
            );
        }
        for rail in std::mem::take(&mut npc.rail_kills) {
            self.rails.kill(rail);
        }
    }

    pub(super) fn apply_npc_spell_events(&mut self) {
        let requests: Vec<_> = self
            .npcs
            .values_mut()
            .flat_map(|npc| {
                let handle = npc.handle;
                std::mem::take(&mut npc.spell_requests)
                    .into_iter()
                    .map(move |e| (handle, e))
            })
            .collect();
        self.apply_owner_spell_events(requests);
    }

    pub(super) fn apply_owner_spell_events(&mut self, requests: Vec<(u16, E)>) {
        for (owner, event) in requests {
            let Some(body_index) = self.character_body(owner) else {
                continue;
            };
            let body = &self.physics.bodies[body_index];
            let (position, direction) = (body.position, body.direction());
            match event {
                E::NpcConjure(_) => {
                    if let Some((bank, cue)) = self
                        .npcs
                        .get(&owner)
                        .and_then(|n| n.summon_element_cue.clone())
                    {
                        self.play_cue(bank, &cue, Some(position));
                    }
                }
                E::CastMagick(magick) => self.cast_magick_from(owner, magick),
                E::MissileLaunched(launch) => self.spawn_missiles(vec![(owner, launch)]),
                E::BeamArea {
                    position,
                    radius,
                    damages,
                    timestamp,
                    ..
                } => {
                    self.with_rail_world(|world| {
                        world.collection_blast(
                            Some(owner),
                            Some(owner),
                            timestamp,
                            radius,
                            position,
                            damages,
                        );
                    });
                    self.blast_grease(position, radius, &damages);
                }
                E::GroundArea {
                    position,
                    radius,
                    damages,
                    timestamp,
                } => self.apply_ground_blast(owner, position, radius, damages, timestamp),
                E::SprayArea {
                    spell,
                    position,
                    cast_source,
                    timestamp,
                    plan,
                } => {
                    self.start_spray_area_audio(owner, position, &spell);
                    self.spray_area(owner, position, cast_source, timestamp, &plan);
                }
                E::SprayWeapon {
                    position,
                    direction,
                    cast_source,
                    timestamp,
                    plan,
                } => {
                    self.spray_weapon(owner, position, direction, cast_source, timestamp, &plan);
                }
                E::UndergroundAttack {
                    position,
                    velocity,
                    range,
                    damages,
                    timestamp,
                } => {
                    self.spawn_underground(owner, position, velocity, range, damages, timestamp);
                }
                E::RailWeapon {
                    position,
                    direction,
                    range,
                    damages,
                    timestamp,
                    ..
                } => {
                    self.rail_weapon(owner, position, direction, range, damages, timestamp);
                }
                E::SprayFreeze {
                    origin,
                    delta,
                    spread,
                    multiplier,
                    damages,
                } => self.freeze_water(origin, delta, spread, multiplier, &damages),
                E::SpawnMines { placements, spell } => self.spawn_mines(owner, &placements, spell),
                E::SpawnBarriers { cast, spell } => self.spawn_barriers(owner, &cast, spell),
                E::ShieldWeapon { spell } => {
                    match magicka_sim::shield_weapon::animation_end(
                        &spell,
                        position,
                        direction,
                        body.radius,
                    ) {
                        Ok(weapon) => self.spawn_shield_weapon(owner, weapon, spell),
                        Err(error) => self.log.push(format!("NPC shield weapon: {error:?}")),
                    }
                }
                E::IceBladeStrike {
                    range,
                    damages,
                    timestamp,
                    ..
                } => {
                    self.ice_blade_strike(owner, position, direction, range, damages, timestamp);
                }
                E::SpawnShield {
                    position,
                    radius,
                    direction,
                    kind,
                    hit_points,
                    color,
                } => {
                    self.spawn_shield(owner, position, radius, direction, kind, hit_points, color);
                }
                E::ShieldCircleDamage { radius, damage } => {
                    self.arc_damage(
                        Some(owner),
                        Some(owner),
                        position,
                        direction,
                        radius,
                        std::f32::consts::PI,
                        &[damage],
                        self.play_time,
                    );
                }
                E::SpellNotReconstructed(message) => self.log.push(format!("NPC spell: {message}")),
                _ => {}
            }
        }
    }
}
