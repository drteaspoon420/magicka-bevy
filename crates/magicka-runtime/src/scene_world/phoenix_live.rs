//! Live Summon Phoenix (research/phoenix.md): the singleton's cast, the
//! descend-clip timer, the CircleDamage blast and the revival of dead
//! players around the landing point. The phoenix model is not rendered.
use super::*;
use magicka_sim::phoenix::{self as ph, Cast, PhoenixEvent, SummonPhoenix};

/// Models/Magicks/Phoenix: the skinned model whose clip times the blast.
const MODEL: &str = "Models/Magicks/Phoenix";

impl<C: SceneContent> SceneWorld<C> {
    /// SummonPhoenix.Execute(owner).
    pub fn phoenix_from(&mut self, owner: u16) -> bool {
        let Some(body) = self.character_body(owner) else {
            return false;
        };
        let position = self.physics.bodies[body].position;
        self.cast_phoenix(Some(owner), position)
    }

    fn cast_phoenix(&mut self, owner: Option<u16>, position: [f32; 3]) -> bool {
        let timestamp = if owner.is_some() {
            self.play_time
        } else {
            self.phoenix.timestamp
        };
        match self.phoenix.cast(self.scene_indoors, owner, timestamp) {
            Cast::Indoors => {
                self.play_cue(16, "magick_summon_phoenix_indoors", Some(position));
                return true;
            }
            Cast::Busy => return owner.is_some(),
            Cast::Started => {}
        }
        let (x, z) = (self.random(), self.random());
        let offset = SummonPhoenix::offset(owner.is_some(), x, z);
        let point: [f32; 3] = std::array::from_fn(|i| position[i] + offset[i]);
        let mut point = self
            .nav
            .as_ref()
            .and_then(|nav| nav.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
            .map_or(point, |(_, p, _)| p);
        if let Some(y) = self.ground_below(point) {
            point[1] = y;
            // The position overload lifts the landing point by 1 m; the
            // owner overload does not.
            if owner.is_none() {
                point[1] += 1.0;
            }
        }
        self.phoenix.point = point;
        self.play_cue(16, "magick_summon_phoenix", Some(point));
        true
    }

    /// The scene hit of a 3 m probe down from 1 m above `point`.
    fn ground_below(&self, point: [f32; 3]) -> Option<f32> {
        self.collision
            .collision()
            .segment_intersect(
                magicka_sim::spray_pulse::Segment {
                    origin: [point[0], point[1] + 1.0, point[2]],
                    delta: [0.0, -3.0, 0.0],
                },
                false,
            )
            .map(|h| h.hit.position[1])
    }

    /// SpellManager.Update for the singleton.
    pub(super) fn update_phoenix(&mut self, dt: f32) {
        if !self.phoenix.registered {
            return;
        }
        let duration = match self.phoenix_clip {
            Some(d) => d,
            None => {
                let d = self
                    .content
                    .clip_duration(MODEL, ph::CLIP)
                    .unwrap_or(f32::INFINITY);
                self.phoenix_clip = Some(d);
                d
            }
        };
        match self.phoenix.update(dt, duration) {
            Some(PhoenixEvent::FailShake) => self.camera.shake(4.0, 0.2),
            Some(PhoenixEvent::Blast) => self.phoenix_blast(),
            None => {}
        }
    }

    /// The blast: CircleDamage (no ignored entity, so the caster is hit
    /// too), CameraShake(1, 1), `magick_phoenix_blast`, `spell_fire_area`,
    /// then on the host a specific-player Revive for each playing dead
    /// player at its quarter-turn offset (full health).
    fn phoenix_blast(&mut self) {
        let p = self.phoenix;
        let origin = [p.point[0], p.point[1] + ph::BLAST_HEIGHT, p.point[2]];
        self.arc_damage(
            p.owner,
            None,
            origin,
            [0.0, 0.0, -1.0],
            ph::BLAST_RANGE,
            std::f32::consts::PI,
            &ph::damage()[..3],
            p.timestamp,
        );
        self.camera.shake(1.0, 1.0);
        self.mine_effects
            .push(("magick_phoenix_blast", origin, [1.0, 0.0, 0.0]));
        self.play_cue(16, "spell_fire_area", Some(p.point));
        if self.role == NetworkRole::Client {
            return;
        }
        for slot in 0..self.players.len() {
            let dead = self.players[slot]
                .as_ref()
                .is_some_and(|a| a.playing && a.dead());
            if !dead {
                continue;
            }
            let o = SummonPhoenix::revive_offset(slot);
            let at: [f32; 3] = std::array::from_fn(|i| p.point[i] + o[i]);
            let mut at = self
                .nav
                .as_ref()
                .and_then(|nav| nav.nearest(at, magicka_sim::navmesh::movement::DEFAULT))
                .map_or(at, |(_, q, _)| q);
            if let Some(y) = self.ground_below(at) {
                at[1] = y;
            }
            self.execute_revive(None, at, 1.0, Some(slot), None);
        }
    }
}
