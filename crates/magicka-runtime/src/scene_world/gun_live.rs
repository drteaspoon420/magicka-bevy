//! Player Item.Update gun shots against the live level and damageables.
use super::*;
use magicka_sim::spray_pulse::Segment;
impl<C: SceneContent> SceneWorld<C> {
    pub(super) fn update_player_guns(&mut self, dt: f32) {
        let mut shots = Vec::new();
        let mut presentations = Vec::new();
        for player in self.players.iter_mut().flatten() {
            for (slot, held) in &mut player.items {
                if !held.attached || player.character.combat.dead {
                    held.gun.stop(false, false, held.definition.weapon_class);
                }
                let frame = held.frame;
                let position = frame.map_or(self.physics.bodies[player.body].position, |f| {
                    f.attach0_position()
                });
                let back =
                    frame.map_or(position, |f| [f.attach1[12], f.attach1[13], f.attach1[14]]);
                if std::mem::take(&mut held.gun.start_presentation) {
                    presentations.push((
                        player.handle,
                        *slot,
                        position,
                        back,
                        held.definition.gun_sound.clone(),
                        held.definition.gun_shells_effect.clone(),
                        true,
                    ));
                }
                if std::mem::take(&mut held.gun.stop_presentation) {
                    if let Some(cue) = held.gun.cue.take() {
                        self.audio_requests.push(crate::audio::AudioRequest::Stop {
                            key: cue,
                            instant: false,
                        });
                    }
                    presentations.push((
                        player.handle,
                        *slot,
                        position,
                        back,
                        String::new(),
                        String::new(),
                        false,
                    ));
                }
                if let Some(cue) = held.gun.cue {
                    self.audio_requests
                        .push(crate::audio::AudioRequest::Move { key: cue, position });
                }
                let Some(frame) = frame.filter(|_| held.attached) else {
                    continue;
                };
                if held.is_gun() && held.gun.tick(dt, held.definition.gun_rate) {
                    let mut direction: [f32; 3] = std::array::from_fn(|i| position[i] - back[i]);
                    let length = direction.iter().map(|x| x * x).sum::<f32>().sqrt();
                    if length > 0. {
                        direction = direction.map(|v| v / length);
                    } else {
                        direction = frame.attach0_forward();
                    }
                    shots.push((
                        player.handle,
                        *slot,
                        position,
                        direction,
                        held.gun.accuracy,
                        held.gun.tracer_count,
                        held.definition.clone(),
                        player.motion.controller.current_animation,
                    ));
                    if player.motion.controller.current_animation != 147 {
                        held.gun.tracer_count = held.gun.tracer_count.wrapping_add(1);
                    }
                }
            }
        }
        for (owner, slot, position, back, sound, shells, start) in presentations {
            if start {
                if !sound.is_empty() {
                    let (bank, name) = sound.split_once('/').map_or(
                        (crate::audio::banks::WEAPONS, sound.as_str()),
                        |(bank, name)| {
                            (
                                match bank.to_ascii_lowercase().as_str() {
                                    "spells" => crate::audio::banks::SPELLS,
                                    "additional" => crate::audio::banks::ADDITIONAL,
                                    _ => crate::audio::banks::WEAPONS,
                                },
                                name,
                            )
                        },
                    );
                    let cue = self.play_cue(bank, name, Some(position));
                    if let Some(held) = self
                        .players
                        .iter_mut()
                        .flatten()
                        .find(|p| p.handle == owner)
                        .and_then(|p| p.items.iter_mut().find(|(s, _)| *s == slot).map(|(_, i)| i))
                    {
                        held.gun.cue = Some(cue);
                    }
                }
                if !shells.is_empty() {
                    self.item_event_effects
                        .push(crate::npc_combat::ItemEffectRequest::Start {
                            owner,
                            weapon: slot + 1000,
                            name: shells.to_ascii_lowercase(),
                            position: back,
                            direction: [0., 1., 0.],
                            follow: true,
                        });
                }
            } else {
                self.item_event_effects
                    .push(crate::npc_combat::ItemEffectRequest::Stop {
                        owner,
                        weapon: slot + 1000,
                    });
            }
        }
        for (owner, slot, origin, direction, accuracy, tracer_count, definition, animation) in shots
        {
            if !definition.gun_muzzle_effect.is_empty() {
                self.item_event_effects
                    .push(crate::npc_combat::ItemEffectRequest::Start {
                        owner,
                        weapon: slot,
                        name: definition.gun_muzzle_effect.to_ascii_lowercase(),
                        position: origin,
                        direction,
                        follow: false,
                    });
            }
            // pickup_weapon suppresses bullets, but still emits muzzle flashes.
            if animation == 147 {
                continue;
            }
            let direction = crate::gun::spread(direction, accuracy, self.random(), self.random());
            let segment = Segment {
                origin,
                delta: direction.map(|v| v * definition.gun_range),
            };
            let scene = self.collision.collision().segment_intersect(segment, false);
            let limit = scene.as_ref().map_or(1., |hit| hit.hit.fraction);
            let combatants = self.combatants();
            let mut nearest: Option<(f32, u16)> = None;
            for target in combatants
                .iter()
                .filter(|c| c.handle != owner && !c.ethereal)
            {
                let start = [
                    target.position[0],
                    target.position[1] - target.length * 0.5,
                    target.position[2],
                ];
                if let Some(fraction) = magicka_sim::capsule_intersection::segment_fraction(
                    segment,
                    start,
                    [0., 1., 0.],
                    target.length,
                    target.radius + 0.05,
                ) {
                    if fraction <= limit && nearest.is_none_or(|(f, _)| fraction < f) {
                        nearest = Some((fraction, target.handle));
                    }
                }
            }
            for entity in self
                .entities
                .iter()
                .filter(|e| e.added && !e.removed && e.handle != owner)
            {
                let Some(bounds) = self
                    .physics_damage
                    .get(&entity.handle)
                    .and_then(|s| s.world_box(&entity.orientation))
                else {
                    continue;
                };
                if let Some(hit) = bounds.segment_intersect(segment) {
                    if hit.fraction <= limit && nearest.is_none_or(|(f, _)| hit.fraction < f) {
                        nearest = Some((hit.fraction, entity.handle));
                    }
                }
            }
            for (&handle, boss) in &self.jormungandr {
                if let Some(fraction) = boss.segment_fraction(segment) {
                    if fraction <= limit && nearest.is_none_or(|(f, _)| fraction < f) {
                        nearest = Some((fraction, handle));
                    }
                }
            }
            for shield in &self.shields {
                if let Some(fraction) = shield.shield.geometry().segment_fraction(segment) {
                    if fraction <= limit && nearest.is_none_or(|(f, _)| fraction < f) {
                        nearest = Some((fraction, shield.handle));
                    }
                }
            }
            for (&handle, dispenser) in &self.dispensers {
                let axis = dispenser.axis();
                let length = axis.delta.iter().map(|v| v * v).sum::<f32>().sqrt();
                if length > 0. {
                    if let Some(fraction) = magicka_sim::capsule_intersection::segment_fraction(
                        segment,
                        axis.origin,
                        axis.delta.map(|v| v / length),
                        length,
                        1.3,
                    ) {
                        if fraction <= limit && nearest.is_none_or(|(f, _)| fraction < f) {
                            nearest = Some((fraction, handle));
                        }
                    }
                }
            }
            let fraction = nearest.map_or(limit, |(f, _)| f);
            let end = std::array::from_fn(|i| origin[i] + segment.delta[i] * fraction);
            let target = nearest.map(|(_, h)| h);
            let flag = if target.is_some() {
                2
            } else if scene.is_some() {
                4
            } else {
                0
            };
            if flag != 0 {
                let Some(view) = crate::npc_combat::CombatView::new(
                    &combatants,
                    Some(self.collision.collision()),
                ) else {
                    continue;
                };
                let mut world = SceneHitWorld {
                    physics_damage: &mut self.physics_damage,
                    dispensers: &mut self.dispensers,
                    entities: &mut self.entities,
                    barriers: None,
                    barrier_events: Vec::new(),
                    view: &view,
                    npcs: &mut self.npcs,
                    players: &mut self.players,
                    bodies: &mut self.physics.bodies,
                    triggers: &mut self.pending_triggers,
                    shields: &mut self.shields,
                    collision_rng: &mut self.rng,
                    play_time: self.play_time,
                    entity_blasts: &mut self.entity_blasts,
                    death: &mut self.death,
                    jormungandr: &mut self.jormungandr,
                    death_damage_features: self.role != NetworkRole::Client,
                };
                for event in definition
                    .gun_conditions
                    .0
                    .iter()
                    .filter(|c| c.condition.condition_type & flag != 0)
                    .flat_map(|c| &c.events)
                {
                    if let (Some(target), magicka_content::common::Event::Damage { damage, .. }) =
                        (target, event)
                    {
                        let damage = magicka_sim::damage::Damage {
                            attack: damage.attack_property as u16,
                            element: damage.element as u32,
                            amount: damage.amount,
                            magnitude: damage.magnitude,
                        };
                        if world.shields.iter().any(|s| s.handle == target) {
                            crate::missiles::MissileWorld::damage_shield(
                                &mut world, target, damage,
                            );
                        } else if world.physics_damage.contains_key(&target) {
                            crate::missiles::MissileWorld::damage_prop(
                                &mut world, target, damage, owner, end,
                            );
                        } else {
                            crate::missiles::MissileWorld::damage_character(
                                &mut world, target, damage, owner, end,
                            );
                        }
                    }
                }
                self.melee_event_presentation(
                    &definition.gun_conditions.0,
                    flag,
                    owner,
                    slot,
                    end,
                    direction,
                );
            }
            if target.is_none() {
                if let Some(hit) = scene.as_ref() {
                    const EFFECTS: [&str; 10] = [
                        "gunhit_generic",
                        "gunhit_gravel",
                        "gunhit_grass",
                        "gunhit_wood",
                        "gunhit_snow",
                        "gunhit_stone",
                        "gunhit_mud",
                        "gunhit_generic",
                        "gunhit_water",
                        "gunhit_lava",
                    ];
                    if let Some(name) = usize::try_from(hit.primitive)
                        .ok()
                        .and_then(|i| EFFECTS.get(i))
                    {
                        self.item_event_effects
                            .push(crate::npc_combat::ItemEffectRequest::Start {
                                owner,
                                weapon: slot,
                                name: (*name).into(),
                                position: end,
                                direction: hit.hit.normal,
                                follow: false,
                            });
                    }
                }
            }
            let sprite = if tracer_count % 5 == 0 {
                crate::gun::sprite(&definition.tracer_sprite)
                    .or_else(|| crate::gun::sprite(&definition.non_tracer_sprite))
            } else {
                crate::gun::sprite(&definition.non_tracer_sprite)
            };
            self.gun_shots.push(crate::gun::Shot {
                owner,
                weapon: slot,
                origin,
                end,
                target,
                velocity: definition.tracer_velocity,
                sprite,
            });
        }
    }
}
