//! Jormungandr campaign, damage, collision and projectile host.
use super::boss_live::BossRef;
use super::*;
impl<C2: SceneContent> SceneWorld<C2> {
    pub(super) fn update_jormungandr(&mut self, dt: f32) {
        let handles: Vec<_> = self
            .boss_fight
            .bosses
            .iter()
            .filter_map(|b| match b {
                BossRef::Jormungandr(h) => Some(*h),
                _ => None,
            })
            .collect();
        for handle in handles {
            let Some(mut boss) = self.jormungandr.remove(&handle) else {
                continue;
            };
            let target = self
                .players
                .iter()
                .flatten()
                .find(|p| !p.dead())
                .map(|p| self.physics.bodies[p.body].position);
            let blocked = self.barriers.iter().any(|(_, b)| {
                b.solid()
                    && !b.is_dead()
                    && within(
                        b.position,
                        std::array::from_fn(|i| boss.position[i] + boss.direction[i] * 3.),
                        2.,
                    )
            });
            let nav = &self.nav;
            let collision = self.collision.collision();
            let events = boss.update(
                dt,
                self.boss_fight.running,
                self.role == NetworkRole::Client,
                target,
                blocked,
                |target, angle| {
                    let mut found = false;
                    let mut direction = [0.; 3];
                    for i in 0..6 {
                        let angle = angle + i as f32 * std::f32::consts::PI / 3.;
                        let requested = [
                            target[0] + angle.cos() * 14.,
                            target[1],
                            target[2] + angle.sin() * 14.,
                        ];
                        direction = [angle.cos(), 0., angle.sin()];
                        if let Some((_, point, _)) = nav.as_ref().and_then(|n| {
                            n.nearest(requested, magicka_sim::navmesh::movement::DEFAULT)
                        }) {
                            let submerged = collision.water.iter().any(|water| {
                                water
                                    .segment_intersect(
                                        Segment {
                                            origin: point,
                                            delta: [0., 10., 0.],
                                        },
                                        false,
                                        false,
                                        false,
                                    )
                                    .is_some()
                            });
                            if crate::jormungandr::distance(point, requested) <= 2. && !submerged {
                                found = true;
                                let delta = [target[0] - point[0], 0., target[2] - point[2]];
                                let length = crate::jormungandr::distance(delta, [0.; 3]);
                                direction = delta.map(|v| v / length);
                            }
                        }
                    }
                    if !found {
                        return None;
                    }
                    // Native retains the final iteration's direction even when
                    // an earlier candidate is the one that made `found` true.
                    let point = std::array::from_fn(|i| target[i] + direction[i] * 14.);
                    collision
                        .segment_intersect(
                            Segment {
                                origin: point,
                                delta: [0., -10., 0.],
                            },
                            false,
                        )
                        .map(|h| h.hit.position)
                },
            );
            self.sync_jormungandr_ground_effects(&mut boss);
            let position = boss.frame.head;
            let damages = boss.collision_damage();
            let mut shield_hits = Vec::new();
            let mut barrier_hits = Vec::new();
            let mut prop_hits = Vec::new();
            if boss.update_animation && !damages.is_empty() {
                let head = Segment {
                    origin: position,
                    delta: [0.; 3],
                };
                for s in &self.shields {
                    let touches = match s.shield.geometry() {
                        magicka_sim::shield_intersection::ShieldGeometry::Wall(bounds) => {
                            magicka_sim::capsule_box::segment_distance(head, bounds).0
                                < (1.1 + 0.05_f32).powi(2)
                        }
                        magicka_sim::shield_intersection::ShieldGeometry::Sphere(sphere) => {
                            let delta: [f32; 3] =
                                std::array::from_fn(|i| position[i] - sphere.center[i]);
                            let length = crate::jormungandr::distance(delta, [0.; 3]);
                            let angle = ((0..3)
                                .map(|i| delta[i] * sphere.forward[i] / length)
                                .sum::<f32>()
                                .clamp(-1., 1.))
                            .acos();
                            (length - sphere.radius).abs()
                                < 1.1 + magicka_sim::hollow_sphere_capsule::THICKNESS + 0.05
                                && angle <= sphere.max_angle
                        }
                    };
                    if touches {
                        if let Some(d) = boss.hit_barrier(s.handle, s.shield.max_hit_points, true) {
                            shield_hits.push((s.handle, d));
                        }
                    }
                }
                for (slot, b) in self.barriers.iter().filter(|(_, b)| !b.is_dead()) {
                    let sphere = [b.position[0], b.position[1] - b.radius, b.position[2]];
                    let sphere_radius = if b.solid() { b.radius * 2. } else { b.radius };
                    let axis = Segment {
                        origin: [b.position[0], b.position[1] - 1., b.position[2]],
                        delta: [0., 0.1, 0.],
                    };
                    let capsule_distance = magicka_sim::geometry::jiglib_segment_segment(
                        head.origin,
                        head.delta,
                        axis.origin,
                        axis.delta,
                    )
                    .0;
                    if crate::jormungandr::distance(sphere, position) < 1.1 + sphere_radius + 0.05
                        || capsule_distance < (1.1 + b.radius + 0.05).powi(2)
                    {
                        if let Some(d) =
                            boss.hit_barrier(BARRIER_KEY | slot as u16, b.max_hit_points, b.solid())
                        {
                            barrier_hits.push((slot, d));
                        }
                    }
                }
                for e in self
                    .entities
                    .iter()
                    .filter(|e| e.added && !e.removed && !e.dead)
                {
                    let Some(bounds) = self
                        .physics_damage
                        .get(&e.handle)
                        .and_then(|s| s.world_box(&e.orientation))
                    else {
                        continue;
                    };
                    if !boss.hits.contains_key(&e.handle)
                        && magicka_sim::capsule_box::segment_distance(head, bounds).0
                            < (1.1 + 0.05_f32).powi(2)
                    {
                        boss.hits.insert(e.handle, 0.25);
                        prop_hits.push(e.handle);
                    }
                }
            }
            let mut character_hits = Vec::new();
            for c in self
                .combatants()
                .into_iter()
                .filter(|c| !c.dead && !c.ethereal)
            {
                let axis = Segment {
                    origin: [c.position[0], c.position[1] - c.length * 0.5, c.position[2]],
                    delta: [0., c.length, 0.],
                };
                let head = Segment {
                    origin: position,
                    delta: [0.; 3],
                };
                let (distance, _, _) = magicka_sim::geometry::jiglib_segment_segment(
                    head.origin,
                    head.delta,
                    axis.origin,
                    axis.delta,
                );
                if boss.update_animation
                    && !damages.is_empty()
                    && distance < (1.1 + c.radius).powi(2)
                    && !boss.hits.contains_key(&c.handle)
                {
                    boss.hits.insert(c.handle, 0.25);
                    character_hits.push((c.handle, damages.clone()));
                }
                if boss.update_animation
                    && !boss.dead
                    && !boss.spine_hits.contains_key(&c.handle)
                    && boss.frame.spine.iter().any(|s| {
                        magicka_sim::geometry::jiglib_segment_segment(
                            s.axis.origin,
                            s.axis.delta,
                            axis.origin,
                            axis.delta,
                        )
                        .0 < (s.radius + c.radius).powi(2)
                    })
                {
                    boss.spine_hits.insert(c.handle, 0.25);
                    character_hits.push((
                        c.handle,
                        vec![magicka_sim::damage::Damage {
                            attack: magicka_sim::damage::attack::KNOCKBACK,
                            element: 1,
                            amount: 60.,
                            magnitude: 1.,
                        }],
                    ));
                }
            }
            if let Ok(e) = self.entity_mut(handle) {
                e.position = position;
                e.orientation = boss.orientation;
                e.hit_points = boss.health.hp;
                e.dead = boss.dead;
            }
            self.jormungandr.insert(handle, boss);
            for (target, d) in shield_hits {
                self.damage_shield_handle(target, d);
                self.play_cue(
                    crate::audio::banks::CHARACTERS,
                    "boss_jormungandr_hitobject",
                    Some(position),
                );
            }
            for (slot, d) in barrier_hits {
                let mut collection = [magicka_sim::damage::Damage::default(); 5];
                collection[0] = d;
                self.damage_barrier(slot, &collection);
                self.play_cue(
                    crate::audio::banks::CHARACTERS,
                    "boss_jormungandr_hitobject",
                    Some(position),
                );
            }
            for target in prop_hits {
                for &d in &damages {
                    let _ = self.damage_physics(target, d, 15);
                }
            }
            for sound in events.sounds {
                self.play_cue(crate::audio::banks::CHARACTERS, sound, Some(position));
            }
            if let Some((at, radius)) = events.clear_barriers {
                let shields: Vec<_> = self
                    .shields
                    .iter()
                    .filter(|s| within(s.shield.position, at, radius + s.shield.radius))
                    .map(|s| s.handle)
                    .collect();
                for h in shields {
                    if let Some(s) = self.shields.iter_mut().find(|s| s.handle == h) {
                        s.shield.kill();
                    }
                }
                let barriers: Vec<_> = self
                    .barriers
                    .iter()
                    .filter(|(_, b)| within(b.position, at, radius + b.radius))
                    .map(|(i, _)| i)
                    .collect();
                for i in barriers {
                    self.kill_barrier(i);
                }
            }
            for (victim, damages) in character_hits {
                for d in damages {
                    self.hit_character(victim, d, Some(handle), position);
                }
            }
            if let Some(at) = events.tail_whip {
                self.arc_damage(
                    Some(handle),
                    Some(handle),
                    at,
                    [0., 0., -1.],
                    3.,
                    std::f32::consts::PI,
                    &[
                        magicka_sim::damage::Damage {
                            attack: magicka_sim::damage::attack::KNOCKBACK,
                            element: 1,
                            amount: 200.,
                            magnitude: 4.,
                        },
                        magicka_sim::damage::Damage {
                            attack: magicka_sim::damage::attack::DAMAGE,
                            element: 1,
                            amount: 300.,
                            magnitude: 1.,
                        },
                    ],
                    self.play_time,
                );
            }
            for (origin, velocity) in events.spit {
                self.jormungandr_spit(handle, origin, velocity);
            }
        }
    }
    /// Native StartEffect and Back/FrontBodyCollision: actual level crossings,
    /// with stable references so looping warnings cannot accumulate each frame.
    fn sync_jormungandr_ground_effects(&mut self, boss: &mut crate::jormungandr::Jormungandr) {
        use crate::jormungandr::State;
        let collision = self.collision.collision();
        let mut positions = [None; 3];
        if boss.state == State::Risen
            && boss.controller.clip_name() == Some("emerge")
            && !boss.update_animation
        {
            let p = boss.position;
            positions[0] = collision
                .segment_intersect(
                    Segment {
                        origin: [p[0], p[1] + 10., p[2]],
                        delta: [0., -30., 0.],
                    },
                    false,
                )
                .map(|h| h.hit.position);
        }
        if !matches!(boss.state, State::Sleep | State::Underground | State::Dead) {
            let points = &boss.frame.body_points;
            if points.len() == 25 {
                for (slot, indices) in [
                    (1, (0..11).rev().collect::<Vec<_>>()),
                    (2, (12..23).collect::<Vec<_>>()),
                ] {
                    let mut origin = points[11];
                    for i in indices {
                        let next = points[i];
                        if let Some(hit) = collision.segment_intersect(
                            Segment {
                                origin,
                                delta: std::array::from_fn(|j| next[j] - origin[j]),
                            },
                            false,
                        ) {
                            positions[slot] = Some(hit.hit.position);
                            break;
                        }
                        origin = next;
                    }
                }
            }
        }
        for (slot, at) in positions.into_iter().enumerate() {
            match (boss.ground_effects[slot], at) {
                (Some(instance), Some(position)) => {
                    self.effect_commands.push(SceneEffectCommand::Move {
                        instance,
                        position,
                        direction: [1., 0., 0.],
                    })
                }
                (None, Some(position)) => {
                    self.effect_serial = self.effect_serial.wrapping_add(1);
                    let instance = self.effect_serial;
                    let transform = magicka_content::skinned::pose_matrix(
                        position,
                        [
                            0.,
                            -std::f32::consts::FRAC_1_SQRT_2,
                            0.,
                            std::f32::consts::FRAC_1_SQRT_2,
                        ],
                        [1.; 3],
                    );
                    self.effect_commands.push(SceneEffectCommand::Start {
                        instance,
                        effect: custom_hash("jormungandr_dirt_tremor"),
                        transform,
                    });
                    boss.ground_effects[slot] = Some(instance);
                }
                (Some(instance), None) => {
                    self.effect_commands
                        .push(SceneEffectCommand::Stop { instance });
                    boss.ground_effects[slot] = None;
                }
                (None, None) => {}
            }
        }
    }
    fn jormungandr_spit(&mut self, owner: u16, position: [f32; 3], velocity: [f32; 3]) {
        let id = self.next_missile;
        self.next_missile = self.next_missile.wrapping_add(1);
        let mut missile = crate::missiles::LiveMissile::spawn_custom(
            id,
            owner,
            position,
            velocity,
            0.25,
            false,
            crate::jormungandr::spit_conditions(),
        );
        let combatants = self.combatants();
        let Some(view) =
            crate::npc_combat::CombatView::new(&combatants, Some(self.collision.collision()))
        else {
            return;
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
        crate::missiles::initialize(&mut missile, &mut world, &mut self.missile_visuals);
        self.register_missile(missile);
    }
    pub(super) fn blast_jormungandr(
        &mut self,
        position: [f32; 3],
        radius: f32,
        damages: [magicka_sim::damage::Damage; 5],
    ) {
        let collision = self.collision.collision();
        for boss in self
            .jormungandr
            .values_mut()
            .filter(|b| b.update_animation && !b.dead)
        {
            let mut seg = Segment {
                origin: position,
                delta: std::array::from_fn(|i| boss.frame.head[i] - position[i]),
            };
            if let Some(fraction) = boss.segment_fraction(seg) {
                seg.delta = seg.delta.map(|v| v * fraction);
            }
            let distance = crate::jormungandr::distance(seg.delta, [0.; 3]);
            if distance > radius || collision.segment_intersect(seg, false).is_some() {
                continue;
            }
            let mut d = damages;
            if !d
                .iter()
                .any(|d| d.element & magicka_sim::spells::Elements::BEAMS.0 != 0)
            {
                let scale = 1. - (distance / radius).max(0.).powi(3);
                for d in &mut d {
                    d.magnitude *= scale;
                }
            }
            boss.damage(&d, self.role != NetworkRole::Client);
        }
    }
}
