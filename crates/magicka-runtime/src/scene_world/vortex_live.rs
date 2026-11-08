//! Live Vortex magick: Vortex.Execute spawning VortexEntity and its update
//! over characters and shields (research/vortex.md). The distortion post
//! effect is not rendered; the particle effect follows in PlaySession.
use super::*;
use magicka_sim::vortex::{self as vx, Vortex};

/// MissileEntity.Initialize: SetMass(4).
const MISSILE_MASS: f32 = 4.0;

#[derive(Clone, Debug)]
pub struct LiveVortex {
    pub serial: u64,
    pub vortex: Vortex,
    /// mVortexCue (`magick_vortex`, Spells bank).
    cue: u32,
}

impl<C: SceneContent> SceneWorld<C> {
    /// Vortex.Execute(owner): the owning machine spawns it 10 m ahead (no
    /// navmesh snap). The rewrite's authority is the host.
    pub fn vortex_from(&mut self, owner: u16) -> bool {
        if self.role == NetworkRole::Client {
            return true;
        }
        let Some(body) = self.character_body(owner) else {
            return false;
        };
        let b = &self.physics.bodies[body];
        let center = Vortex::spawn_point(b.position, b.direction());
        // GetInstance cycles a ring of 8: a ninth vortex reinitializes the
        // oldest one.
        if self.vortices.len() >= vx::POOL_SIZE {
            let old = self.vortices.remove(0);
            self.stop_cue(old.cue);
        }
        self.effect_serial = self.effect_serial.wrapping_add(1);
        let cue = self.play_cue(16, "magick_vortex", Some(center));
        self.vortices.push(LiveVortex {
            serial: self.effect_serial,
            vortex: Vortex::new(Some(owner), center),
            cue,
        });
        true
    }

    /// VortexEntity.Update for each vortex, then removal once Dead.
    pub(super) fn update_vortices(&mut self, dt: f32) {
        if self.vortices.is_empty() {
            return;
        }
        let mut vortices = std::mem::take(&mut self.vortices);
        for live in &mut vortices {
            let Some(radius) = live.vortex.begin_update(dt) else {
                continue;
            };
            self.vortex_pass(&mut live.vortex, radius);
            live.vortex.end_update(dt);
        }
        vortices.retain(|live| {
            let keep = !live.vortex.dead();
            if !keep {
                self.stop_cue(live.cue);
            }
            keep
        });
        self.vortices = vortices;
    }

    /// The entity loop: GetEntities(centre, radius, ignoreProtected: true)
    /// minus dead and ethereal characters. Bodies whose bounding box touches
    /// the core are overkilled (characters without gibs are terminated) and
    /// feed the mass; the rest are pulled toward the centre.
    fn vortex_pass(&mut self, vortex: &mut Vortex, radius: f32) {
        let center = vortex.center;
        let within = |p: [f32; 3], r: f32| {
            (0..3).map(|i| (p[i] - center[i]).powi(2)).sum::<f32>() <= (radius + r).powi(2)
        };
        let spheres: Vec<_> = self
            .shields
            .iter()
            .filter(|s| {
                !s.shield.dead && s.shield.kind == magicka_sim::shield_entity::ShieldKind::Sphere
            })
            .map(|s| (s.shield.position, s.shield.radius))
            .collect();
        let protected = |p: [f32; 3]| {
            spheres
                .iter()
                .any(|(c, r)| (0..3).map(|i| (p[i] - c[i]).powi(2)).sum::<f32>() < r * r)
        };
        let owner_faction = vortex
            .owner
            .and_then(|o| self.players.iter().flatten().find(|p| p.handle == o))
            .map(|p| p.faction);
        for c in self.combatants() {
            if c.dead || c.ethereal || !within(c.position, c.radius) || protected(c.position) {
                continue;
            }
            let Some(body) = self.character_body(c.handle) else {
                continue;
            };
            let half = c.radius + 0.5 * c.length;
            let min = [
                c.position[0] - c.radius,
                c.position[1] - half,
                c.position[2] - c.radius,
            ];
            let max = [
                c.position[0] + c.radius,
                c.position[1] + half,
                c.position[2] + c.radius,
            ];
            if vortex.core_touches(min, max) {
                let gibs = self
                    .character_ref(c.handle)
                    .is_some_and(|r| r.definition.gib_count > 0);
                if gibs {
                    self.overkill(c.handle);
                } else {
                    self.terminate_character(c.handle);
                }
                if owner_faction.is_some_and(|f| c.faction & f != f) {
                    vortex.hostile_absorbed += 1;
                }
                let dead = self.character_ref(c.handle).is_none_or(|r| r.combat.dead);
                if dead {
                    vortex.absorb(self.physics.bodies[body].mass);
                }
            } else if let Some(force) = vortex.pull(c.position) {
                self.physics.bodies[body].set_additional_force(force);
            }
        }
        // MissileEntity (IDamageable, Body mass 4): OverKill at the core sets
        // Dead, which feeds the vortex; otherwise the pull adds to the
        // missile's velocity like other non-character bodies.
        for m in &mut self.missiles {
            if m.body.dead || !within(m.body.position, m.body.radius) {
                continue;
            }
            let (p, r) = (m.body.position, m.body.radius);
            if vortex.core_touches(p.map(|v| v - r), p.map(|v| v + r)) {
                m.body.dead = true;
                vortex.absorb(MISSILE_MASS);
            } else if let Some(force) = vortex.pull(p) {
                m.body.velocity = std::array::from_fn(|i| m.body.velocity[i] + force[i]);
            }
        }
        // Shield.OverKill: HitPoints = -MaxHitPoints (then the usual fade).
        let shields: Vec<u16> = self
            .shields
            .iter()
            .filter(|s| !s.shield.dead && within(s.shield.position, s.shield.radius))
            .filter(|s| {
                let r = s.shield.radius;
                let p = s.shield.position;
                vortex.core_touches(p.map(|v| v - r), p.map(|v| v + r))
            })
            .map(|s| s.handle)
            .collect();
        for handle in shields {
            if let Some(s) = self.shields.iter_mut().find(|s| s.handle == handle) {
                s.shield.hit_points = -s.shield.max_hit_points;
            }
        }
    }
}
