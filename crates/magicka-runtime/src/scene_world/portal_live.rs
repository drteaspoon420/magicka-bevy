//! Live Portal magick (research/portal.md): Execute's navmesh/ground
//! placement, Portal.Update's connection and `spell_shield` loops, contact
//! queueing and teleports of characters and missiles, and the per-portal
//! particle effects.
use super::*;
use magicka_sim::portal::{self as pt, EffectChange, Portal};

/// Presentation and audio state of the two portal singletons.
#[derive(Clone, Debug, Default)]
pub struct PortalHost {
    pub portal: Portal,
    /// Stable singleton identities in A/B order; follows the placement swap.
    pub(super) handles: Option<[u16; 2]>,
    /// The particle effect instance of A and B (follows the swap).
    effects: [Option<u64>; 2],
    /// sSoundA / sSoundB: the connected `spell_shield` loops.
    sounds: Option<(u32, u32)>,
    /// mTeleportQueue entries: (entry is A, entity handle).
    queue: Vec<(bool, u16)>,
    /// Whether A and B are in the entity list (Removable = Dead).
    added: [bool; 2],
}

impl<C: SceneContent> SceneWorld<C> {
    /// Particle instances the portals own (kept by PlaySession's registry).
    pub(crate) fn portal_effect_instances(&self) -> impl Iterator<Item = u64> + '_ {
        self.portals.effects.iter().flatten().copied()
    }

    /// Portal.Execute(owner): owner + Direction·2 snapped to the navmesh
    /// (Water | Dynamic); a snap over 5 m fails with magick_fail.
    pub fn portal_from(&mut self, owner: u16) -> bool {
        let Some(body) = self.character_body(owner) else {
            return false;
        };
        let (position, direction) = (
            self.physics.bodies[body].position,
            self.physics.bodies[body].direction(),
        );
        let mut point = Portal::spawn_point(position, direction);
        if self.host_portal_walkable_normal.is_some() {
            let delta = std::array::from_fn(|i| point[i] - position[i]);
            if let Some(hit) = self.collision.collision().segment_intersect(
                Segment {
                    origin: position,
                    delta,
                },
                false,
            ) {
                let distance = delta.iter().map(|v| v * v).sum::<f32>().sqrt();
                if distance > 1e-5 {
                    let fraction =
                        (hit.hit.fraction - self.physics.bodies[body].radius / distance).max(0.);
                    point = std::array::from_fn(|i| position[i] + delta[i] * fraction);
                }
            }
        }
        let Some(snapped) = self
            .portal_snap(point)
            .filter(|p| Portal::snap_accepted(point, *p))
        else {
            self.play_cue(16, "magick_fail", Some(position));
            return false;
        };
        if self.role != NetworkRole::Client {
            self.place_portal(snapped);
        }
        true
    }

    /// Portal.Execute(Vector3): the same snap check, failing silently.
    pub fn portal_at(&mut self, position: [f32; 3]) -> bool {
        if self.role == NetworkRole::Client {
            return true;
        }
        let Some(snapped) = self
            .portal_snap(position)
            .filter(|p| Portal::snap_accepted(position, *p))
        else {
            return false;
        };
        self.place_portal(snapped);
        true
    }

    fn portal_snap(&self, point: [f32; 3]) -> Option<[f32; 3]> {
        let nearest = self
            .nav
            .as_ref()
            .and_then(|nav| nav.nearest(point, pt::NAV_MOVEMENT));
        if let Some((_, p, _)) = nearest {
            return Some(p);
        }
        let Some(normal) = self.host_portal_walkable_normal else {
            return Some(point);
        };
        // Snap to the current floor before Initialize's +2/-4 m probe. Starting
        // that probe at the avatar centre can put it above an indoor roof.
        self.collision
            .collision()
            .segment_intersect(
                Segment {
                    origin: [point[0], point[1] + 0.1, point[2]],
                    delta: [0., -pt::MAX_SNAP_SQ.sqrt(), 0.],
                },
                false,
            )
            .filter(|hit| hit.hit.normal[1] >= normal)
            .map(|hit| hit.hit.position)
    }

    /// Swap, then PortalEntity.Initialize on the new A: Deinitialize (its
    /// effect stops), probe down for the hover point, the effect when
    /// already connected, AddEntity and the `magick_teleportb` cue.
    pub(super) fn place_portal(&mut self, snapped: [f32; 3]) {
        if self.portals.handles.is_none() {
            self.portals.handles = Some([self.new_handle(), self.new_handle()]);
        }
        let probe = Segment {
            origin: [snapped[0], snapped[1] + pt::PROBE_RISE, snapped[2]],
            delta: [0.0, -pt::PROBE_DROP, 0.0],
        };
        let ground = self
            .collision
            .collision()
            .segment_intersect(probe, false)
            .map(|hit| hit.hit.position);
        let mut host = std::mem::take(&mut self.portals);
        host.handles.as_mut().unwrap().swap(0, 1);
        host.effects.swap(0, 1);
        host.added.swap(0, 1);
        if let Some(instance) = host.effects[0].take() {
            self.effect_commands
                .push(SceneEffectCommand::Stop { instance });
        }
        let placed = host.portal.place(snapped, ground);
        if placed.connected {
            host.effects[0] = Some(self.start_portal_effect(placed));
        }
        host.added[0] = true;
        self.portals = host;
        self.play_cue(16, "magick_teleportb", Some(placed.position));
    }

    fn start_portal_effect(&mut self, p: pt::PortalEntity) -> u64 {
        self.effect_serial = self.effect_serial.wrapping_add(1);
        let instance = self.effect_serial;
        // StartEffect(position, Vector3.Forward) / the body transform: both
        // are the identity orientation at the body.
        let mut transform = magicka_content::skinned::IDENTITY;
        transform[12..15].copy_from_slice(&p.position);
        self.effect_commands.push(SceneEffectCommand::Start {
            instance,
            effect: custom_hash(p.kind.effect()),
            transform,
        });
        instance
    }

    /// Portal.Kill (NullifyArea).
    pub(super) fn kill_portals(&mut self) {
        self.portals.portal.kill();
    }

    /// Portal.Update (SpellManager.Update): connection and the two loops.
    pub(super) fn update_portal_connection(&mut self) {
        let magnification = self.camera.magnification();
        let health = self.portals.portal.update_connection(magnification);
        match (health, self.portals.sounds) {
            (Some(_), None) => {
                let (a, b) = (
                    self.portals.portal.a.position,
                    self.portals.portal.b.position,
                );
                let sa = self.play_portal_loop(a);
                let sb = self.play_portal_loop(b);
                self.portals.sounds = Some((sa, sb));
            }
            (None, Some((sa, sb))) => {
                self.stop_cue(sa);
                self.stop_cue(sb);
                self.portals.sounds = None;
            }
            _ => {}
        }
    }

    /// `spell_shield` at Volume −8 (its Health variable, 1 − distance/30,
    /// has no backend counterpart yet).
    fn play_portal_loop(&mut self, at: [f32; 3]) -> u32 {
        let key = self.backend().next_key();
        self.audio_requests.push(crate::audio::AudioRequest::Start {
            key,
            bank: 16,
            cue: custom_hash("spell_shield"),
            volume_db: -8.0,
            source: crate::audio::SoundSource::Emitter(at),
            magnitude: None,
        });
        key
    }

    /// OnCollision for live portals: characters and missiles overlapping
    /// either capsule are queued (Barriers, Shields and grease fields are
    /// excluded natively; none of them is tested here).
    pub(super) fn portal_contacts(&mut self) {
        if !self.portals.portal.connected {
            return;
        }
        let tolerance = magicka_sim::character_physics::COLLISION_TOLERANCE;
        let mut touching = Vec::new();
        for (is_a, p) in [
            (true, self.portals.portal.a),
            (false, self.portals.portal.b),
        ] {
            if p.dead {
                continue;
            }
            let (bottom, top, radius) = p.capsule();
            let axis = std::array::from_fn(|i| top[i] - bottom[i]);
            for c in self.combatants() {
                let half = c.length * 0.5;
                let base = [c.position[0], c.position[1] - half, c.position[2]];
                let d = magicka_sim::geometry::segment_segment_distance_sq(
                    bottom,
                    axis,
                    base,
                    [0.0, c.length, 0.0],
                );
                if d.sqrt() <= radius + c.radius + tolerance {
                    touching.push((is_a, c.handle));
                }
            }
            for m in &self.missiles {
                if m.body.dead {
                    continue;
                }
                let d = magicka_sim::geometry::segment_segment_distance_sq(
                    bottom,
                    axis,
                    m.body.position,
                    [0.0; 3],
                );
                if d.sqrt() <= radius + m.body.radius {
                    touching.push((is_a, missile_handle(m.body.id)));
                }
            }
        }
        for (is_a, handle) in touching {
            if self.portals.portal.touch(handle) {
                self.portals.queue.push((is_a, handle));
            }
        }
    }

    /// PortalEntity.Update for A then B (entity order is approximated):
    /// teleports with `magick_teleporta`, effect restarts, alpha.
    pub(super) fn update_portal_entities(&mut self, dt: f32) {
        let queue = std::mem::take(&mut self.portals.queue);
        for (is_a, handle) in queue {
            self.portal_teleport(is_a, handle);
        }
        for (index, is_a) in [(0, true), (1, false)] {
            if !self.portals.added[index] {
                continue;
            }
            let dead = if is_a {
                self.portals.portal.a.dead
            } else {
                self.portals.portal.b.dead
            };
            if dead {
                // Removable = Dead: Deinitialize stops the effect.
                self.portals.added[index] = false;
                if let Some(instance) = self.portals.effects[index].take() {
                    self.effect_commands
                        .push(SceneEffectCommand::Stop { instance });
                }
                continue;
            }
            let change = self.portals.portal.update_entity(is_a, dt);
            if let Some(EffectChange::Restart { connected }) = change {
                if let Some(instance) = self.portals.effects[index].take() {
                    self.effect_commands
                        .push(SceneEffectCommand::Stop { instance });
                }
                if connected {
                    let p = if is_a {
                        self.portals.portal.a
                    } else {
                        self.portals.portal.b
                    };
                    self.portals.effects[index] = Some(self.start_portal_effect(p));
                }
            }
        }
    }

    fn portal_teleport(&mut self, entry_is_a: bool, handle: u16) {
        let entry = if entry_is_a {
            self.portals.portal.a.position
        } else {
            self.portals.portal.b.position
        };
        if handle & MISSILE_KEY == MISSILE_KEY {
            let id = u32::from(handle & !MISSILE_KEY);
            if let Some(m) = self.missiles.iter_mut().find(|m| m.body.id & 0x3FFF == id) {
                let out = self
                    .portals
                    .portal
                    .out_position(entry_is_a, m.body.position);
                m.body.position = out;
                m.body.old_position = out;
            }
        } else if let Some(body) = self.character_body(handle) {
            let out = self
                .portals
                .portal
                .out_position(entry_is_a, self.physics.bodies[body].position);
            self.physics.bodies[body].set_position(out);
        } else {
            return;
        }
        // AudioManager.PlayCue(SOUND_TELEPORT, the entry portal's emitter).
        self.play_cue(16, "magick_teleporta", Some(entry));
    }
}

/// Portal ignore-list key for a missile (characters use their handles).
const MISSILE_KEY: u16 = 0xC000;
fn missile_handle(id: u32) -> u16 {
    MISSILE_KEY | (id & 0x3FFF) as u16
}
