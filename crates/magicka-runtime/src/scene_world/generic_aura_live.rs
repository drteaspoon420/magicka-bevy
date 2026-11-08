//! General auras on characters and held items, and their buffs
//! (research/staff-items.md). AuraBuff hands its BuffStorage to every
//! non-ethereal character within the radius (ignoring height) that passes
//! the AuraTarget filter; AuraLove charms enemies; AuraDeflect pushes
//! missiles away. Buffs then age, and DealDamage hits once a second.
use super::*;
use crate::generic_aura::{AuraKind, AuraParty, AuraTarget, BuffSpec, GenericAura};

enum Grant {
    Buff {
        target: u16,
        spec: BuffSpec,
        self_casted: bool,
        owner: u16,
    },
    Charm {
        target: u16,
        owner: u16,
        ttl: f32,
    },
    Deflect {
        owner: [f32; 3],
        strength: f32,
        dt: f32,
    },
}

impl<C: SceneContent> SceneWorld<C> {
    fn aura_parties(&self) -> Vec<AuraParty> {
        let type_name = |handle: u16| {
            self.players
                .iter()
                .flatten()
                .find(|p| p.handle == handle)
                .map(|p| p.character.definition.id.clone())
                .or_else(|| {
                    self.npcs
                        .get(&handle)
                        .map(|n| n.character.definition.id.clone())
                })
                .unwrap_or_default()
        };
        self.combatants()
            .into_iter()
            .filter(|c| {
                self.players.iter().flatten().any(|p| p.handle == c.handle)
                    || self.npcs.contains_key(&c.handle)
            })
            .map(|c| AuraParty {
                handle: c.handle,
                position: c.position,
                radius: c.radius,
                faction: c.faction,
                type_name: type_name(c.handle),
                ethereal: c.ethereal,
                dead: c.dead,
            })
            .collect()
    }

    fn execute_aura(
        aura: &mut GenericAura,
        owner: &AuraParty,
        parties: &[AuraParty],
        dt: f32,
        authority: bool,
        grants: &mut Vec<Grant>,
    ) {
        let fires = aura.tick(dt);
        for _ in 0..fires {
            match &aura.kind {
                AuraKind::Buff(spec) => {
                    if aura.target == AuraTarget::Own {
                        grants.push(Grant::Buff {
                            target: owner.handle,
                            spec: spec.clone(),
                            self_casted: aura.self_casted,
                            owner: owner.handle,
                        });
                        continue;
                    }
                    for t in parties {
                        let (dx, dz) = (
                            t.position[0] - owner.position[0],
                            t.position[2] - owner.position[2],
                        );
                        // GetEntities(iIgnoreYAxis): INFERRED reach of the
                        // target's radius beyond the aura radius.
                        if t.ethereal
                            || (dx * dx + dz * dz).sqrt() - t.radius > aura.radius
                            || !aura.accepts(owner, t)
                        {
                            continue;
                        }
                        grants.push(Grant::Buff {
                            target: t.handle,
                            spec: spec.clone(),
                            // Only AuraTarget.All rewrites SelfCasted.
                            self_casted: if aura.target == AuraTarget::All {
                                t.handle == owner.handle
                            } else {
                                aura.self_casted
                            },
                            owner: owner.handle,
                        });
                    }
                }
                AuraKind::Love { radius, ttl } => {
                    if !authority {
                        continue;
                    }
                    for t in parties {
                        let d: f32 = (0..3)
                            .map(|i| (t.position[i] - owner.position[i]).powi(2))
                            .sum::<f32>()
                            .sqrt();
                        if t.handle != owner.handle
                            && d - t.radius <= *radius
                            && t.faction & owner.faction == 0
                        {
                            grants.push(Grant::Charm {
                                target: t.handle,
                                owner: owner.handle,
                                ttl: *ttl,
                            });
                        }
                    }
                }
                AuraKind::Deflect { strength } => grants.push(Grant::Deflect {
                    owner: owner.position,
                    strength: *strength,
                    dt,
                }),
                AuraKind::Inert => {}
            }
        }
    }

    /// Character and item general auras, then the general buffs' ages and
    /// DealDamage hits. Called from the aura/buff section of the update.
    pub(super) fn update_generic_auras(&mut self, dt: f32) {
        let parties = self.aura_parties();
        let authority = self.role != NetworkRole::Client;
        let mut grants = Vec::new();
        let mut effects = Vec::new();
        let cutscene = self.flags.cutscene;
        let collision = self.collision.collision();
        let decals = &mut self.decals;
        let ground = |origin: [f32; 3], delta: [f32; 3]| {
            collision
                .segment_intersect(magicka_sim::spray_pulse::Segment { origin, delta }, false)
                .map(|hit| {
                    let f = hit.hit.fraction;
                    std::array::from_fn(|i| origin[i] + delta[i] * f)
                })
        };
        let mut ring = |aura: &mut GenericAura, at: [f32; 3]| {
            if let Some(r) = aura.ring.as_mut() {
                r.update_ring(dt, at, cutscene, ground, decals);
            }
        };
        for p in self.players.iter_mut().flatten() {
            let Some(owner) = parties.iter().find(|o| o.handle == p.handle) else {
                continue;
            };
            for (_, item) in p.items.iter_mut().filter(|(_, i)| i.attached) {
                for aura in &mut item.auras {
                    Self::execute_aura(aura, owner, &parties, dt, authority, &mut grants);
                    ring(aura, owner.position);
                }
            }
            for aura in &mut p.character.generic_auras {
                Self::execute_aura(aura, owner, &parties, dt, authority, &mut grants);
                ring(aura, owner.position);
                effects.push((aura.effect_instance, owner.position, aura.ttl));
            }
            p.character.generic_auras.retain(|a| a.ttl > 0.0);
        }
        for n in self.npcs.values_mut() {
            let Some(owner) = parties.iter().find(|o| o.handle == n.handle) else {
                continue;
            };
            for (_, weapon) in &mut n.weapons {
                for aura in &mut weapon.auras {
                    Self::execute_aura(aura, owner, &parties, dt, authority, &mut grants);
                    ring(aura, owner.position);
                }
            }
            for aura in &mut n.character.generic_auras {
                Self::execute_aura(aura, owner, &parties, dt, authority, &mut grants);
                ring(aura, owner.position);
                effects.push((aura.effect_instance, owner.position, aura.ttl));
            }
            n.character.generic_auras.retain(|a| a.ttl > 0.0);
        }
        for (instance, position, ttl) in effects {
            let Some(instance) = instance else { continue };
            self.effect_commands.push(if ttl > 0.0 {
                SceneEffectCommand::Move {
                    instance,
                    position,
                    direction: [0.0, 0.0, 1.0],
                }
            } else {
                SceneEffectCommand::Stop { instance }
            });
        }
        for grant in grants {
            match grant {
                Grant::Buff {
                    target,
                    spec,
                    self_casted,
                    owner,
                } => {
                    if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, target)
                    {
                        let dead = c.combat.dead;
                        if c.buffs.general.add(&spec, self_casted, owner, dead) {
                            c.apply_resistance_buff_to_statuses(&spec.kind);
                        }
                    }
                }
                Grant::Charm { target, owner, ttl } => {
                    let charmed = self.character_ref(target).is_some_and(|c| c.is_charmed());
                    if !charmed {
                        self.charm_character(target, owner, ttl);
                    }
                }
                Grant::Deflect {
                    owner,
                    strength,
                    dt,
                } => self.deflect_missiles(owner, strength, dt),
            }
        }
        let mut hits = Vec::new();
        let bodies = &self.physics.bodies;
        let characters = self
            .players
            .iter_mut()
            .flatten()
            .map(|p| (p.handle, p.body, &mut p.character))
            .chain(
                self.npcs
                    .iter_mut()
                    .map(|(h, n)| (*h, n.body, &mut n.character)),
            );
        for (handle, body, c) in characters {
            let d = &c.definition;
            let view = crate::generic_aura::BuffView {
                decals: &mut self.decals,
                position: bodies[body].position,
                radius: d.radius,
                height_offset: -d.radius - 0.5 * d.length,
                in_cutscene: self.flags.cutscene,
            };
            // Character.Update: mUndying = Template.Undying, then the buffs
            // execute (BuffUndying sets it); the undie timer runs while dead.
            for (damage, attacker) in c.buffs.general.update(dt, Some(view)) {
                hits.push((handle, damage, attacker));
            }
            c.undying = c.definition.undying || c.buffs.general.undying();
            c.tick_undie(dt);
        }
        for (target, damage, attacker) in hits {
            let Some(body) = self.character_body(target) else {
                continue;
            };
            let position = self.physics.bodies[body].position;
            self.hit_character(target, damage, attacker.or(Some(target)), position);
        }
    }

    /// AuraDeflect.Execute: live missiles within Strength of the owner are
    /// pushed outwards (XZ, ±π/8 random yaw), scaled by their speed, the
    /// frame time and how deep inside the sphere they are.
    pub(super) fn deflect_missiles(&mut self, owner: [f32; 3], strength: f32, dt: f32) {
        let indices: Vec<usize> = (0..self.missiles.len())
            .filter(|&i| {
                let m = &self.missiles[i].body;
                let d: f32 = (0..3).map(|k| (m.position[k] - owner[k]).powi(2)).sum();
                !m.dead && d.sqrt() <= strength
            })
            .collect();
        for i in indices {
            let angle = (self.random() - 0.5) * std::f32::consts::FRAC_PI_4;
            let m = &mut self.missiles[i].body;
            let mut away = [m.position[0] - owner[0], 0.0, m.position[2] - owner[2]];
            let distance = (0..3)
                .map(|k| (m.position[k] - owner[k]).powi(2))
                .sum::<f32>()
                .sqrt();
            let length = (away[0] * away[0] + away[2] * away[2]).sqrt();
            if length > 0.0 {
                away = [away[0] / length, 0.0, away[2] / length];
            }
            let (s, c) = angle.sin_cos();
            let away = [away[0] * c + away[2] * s, 0.0, -away[0] * s + away[2] * c];
            let speed = m.velocity.iter().map(|v| v * v).sum::<f32>().sqrt();
            let scale = speed * dt * 100.0 * (1.0 - distance / strength);
            for (v, a) in m.velocity.iter_mut().zip(away) {
                *v += a * scale;
            }
        }
    }

    /// Character.AddAura for a general AuraStorage, with its effect.
    pub(super) fn add_generic_aura(&mut self, owner: u16, mut aura: GenericAura) {
        if !aura.effect.is_empty() {
            if let Some(body) = self.character_body(owner) {
                let b = &self.physics.bodies[body];
                let (position, direction) = (b.position, b.direction());
                aura.effect_instance =
                    Some(self.start_named_effect(&aura.effect, position, direction));
            }
        }
        if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, owner) {
            // ActiveAura.StartTTL is the TTL at AddAura (aura HUD bar width).
            aura.start_ttl = aura.ttl;
            c.generic_auras.push(aura);
        }
    }
}
