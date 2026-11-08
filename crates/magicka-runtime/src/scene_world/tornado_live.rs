//! Live Tornado magick: Tornado.Execute spawning TornadoEntity, its update
//! and OnCollision damage (research/tornado.md). The dust effect follows in
//! PlaySession; the skinned funnel model is not rendered yet.
use super::*;
use magicka_sim::tornado::{self as tn, Tornado};

#[derive(Clone, Debug)]
pub struct LiveTornado {
    /// Identity for presentation (effects keyed per instance).
    pub serial: u64,
    pub tornado: Tornado,
    /// mAmbience (`magick_tornado_rumble`, Spells bank).
    cue: u32,
}

impl<C: SceneContent> SceneWorld<C> {
    /// Tornado.Execute(owner): host only, 4 m ahead snapped to the navmesh
    /// (MovementProperties.Default), facing the 4× scaled direction.
    pub fn tornado_from(&mut self, owner: u16) -> bool {
        let Some(body) = self.character_body(owner) else {
            return false;
        };
        let (position, direction) = (
            self.physics.bodies[body].position,
            self.physics.bodies[body].direction(),
        );
        let (point, forward) = tn::spawn(position, direction);
        self.spawn_tornado(point, forward, Some(owner));
        true
    }

    /// Tornado.Execute(position): the scripted overload, identity facing.
    pub fn tornado_at(&mut self, position: [f32; 3]) -> bool {
        self.spawn_tornado(position, [0.0, 0.0, -1.0], None);
        true
    }

    fn spawn_tornado(&mut self, point: [f32; 3], forward: [f32; 3], owner: Option<u16>) {
        if self.role == NetworkRole::Client {
            return;
        }
        let point = self
            .nav
            .as_ref()
            .and_then(|nav| nav.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
            .map_or(point, |(_, p, _)| p);
        // GetInstance cycles a ring of 16: a 17th tornado reinitializes the
        // oldest one.
        if self.tornadoes.len() >= tn::POOL_SIZE {
            let old = self.tornadoes.remove(0);
            self.stop_cue(old.cue);
        }
        self.effect_serial = self.effect_serial.wrapping_add(1);
        let cue = self.play_cue(16, "magick_tornado_rumble", Some(point));
        self.tornadoes.push(LiveTornado {
            serial: self.effect_serial,
            tornado: Tornado::new(point, forward, owner, self.play_time),
            cue,
        });
    }

    /// TornadoEntity.Update for each live tornado, its collision hits, then
    /// removal (Removable once TTL < 0) with Deinitialize's cue stop.
    pub(super) fn update_tornadoes(&mut self, dt: f32) {
        if self.tornadoes.is_empty() {
            return;
        }
        let mut tornadoes = std::mem::take(&mut self.tornadoes);
        for live in &mut tornadoes {
            // SceneWorld::random's xorshift on a local copy, drawn only when
            // the tornado picks a new wander target.
            let mut rng = self.rng;
            let nav = self.nav.as_ref();
            let collision = self.collision.collision();
            live.tornado.update(
                dt,
                || {
                    rng ^= rng << 13;
                    rng ^= rng >> 7;
                    rng ^= rng << 17;
                    (rng >> 40) as f32 / (1u64 << 24) as f32
                },
                |start, end| {
                    let nav = nav?;
                    match nav.find_shortest_path(
                        start,
                        end,
                        magicka_sim::navmesh::movement::DEFAULT,
                        &|_| false,
                    ) {
                        Ok((true, nodes)) => Some(nodes.into_iter().map(|n| n.position).collect()),
                        _ => None,
                    }
                },
                |origin| {
                    collision
                        .segment_intersect(
                            magicka_sim::spray_pulse::Segment {
                                origin,
                                delta: [0.0, -3.0, 0.0],
                            },
                            false,
                        )
                        .map(|h| h.hit.position[1])
                },
            );
            self.rng = rng;
            self.move_cue(live.cue, live.tornado.position);
            self.tornado_hits(&mut live.tornado, dt);
        }
        tornadoes.retain(|live| {
            let keep = !live.tornado.dead();
            if !keep {
                self.stop_cue(live.cue);
            }
            keep
        });
        self.tornadoes = tornadoes;
    }

    /// OnCollision: every damageable touching the capsule and not hit in the
    /// last second takes the Knockback Earth collection from the owner, with
    /// `magick_tornado_hit` at the capsule surface towards it.
    fn tornado_hits(&mut self, tornado: &mut Tornado, dt: f32) {
        let damages = tn::damage();
        let owner = tornado.owner;
        let timestamp = tornado.timestamp;
        // User-requested enhancement (not native): suction, lift, fling.
        for c in self.combatants() {
            // The caster is spared the enhanced field (it would drag the
            // wizard into its own tornado); native contact still applies.
            if c.dead || c.ethereal || Some(c.handle) == tornado.owner {
                continue;
            }
            let Some(body) = self.character_body(c.handle) else {
                continue;
            };
            use magicka_sim::tornado::field::{self, Effect};
            match field::effect(tornado, c.handle, c.position, c.radius, dt) {
                Some(Effect::Pull(force)) => self.physics.bodies[body].set_additional_force(force),
                Some(Effect::Lift(mut v)) => {
                    let b = &mut self.physics.bodies[body];
                    // A grounded body only leaves the ground on an impulse
                    // above 2.7 m/s: pop it off first.
                    if b.is_touching_ground() {
                        v[1] += 3.0;
                    }
                    b.add_impulse_velocity(v);
                }
                Some(Effect::Fling(v)) => {
                    self.physics.bodies[body].add_impulse_velocity(v);
                    tornado.flung.add_for(c.handle, field::FLING_COOLDOWN);
                }
                None => {}
            }
        }
        let mut hits = Vec::new();
        for c in self.combatants() {
            let half = 0.5 * c.length;
            let bottom = [c.position[0], c.position[1] - half, c.position[2]];
            let top = [c.position[0], c.position[1] + half, c.position[2]];
            if !c.dead && tn::touches(tornado, bottom, top, c.radius) {
                if let Some(point) = tornado.hit(c.handle, c.position) {
                    hits.push((c.handle, point));
                }
            }
        }
        for (handle, point) in &hits {
            self.mine_effects
                .push(("magick_tornado_hit", *point, [0.0, 0.0, -1.0]));
            self.with_rail_world(|world| {
                world.deliver_collection(*handle, owner, damages, timestamp, *point);
            });
        }
        let shields: Vec<(u16, [f32; 3])> = self
            .shields
            .iter()
            .filter(|s| {
                !s.shield.dead
                    && tn::touches(
                        tornado,
                        s.shield.position,
                        s.shield.position,
                        s.shield.radius,
                    )
            })
            .map(|s| (s.handle, s.shield.position))
            .collect();
        for (handle, position) in shields {
            if let Some(point) = tornado.hit(handle, position) {
                self.mine_effects
                    .push(("magick_tornado_hit", point, [0.0, 0.0, -1.0]));
                for d in damages.iter().filter(|d| d.element != 0) {
                    self.damage_shield_handle(handle, *d);
                }
            }
        }
    }
}
