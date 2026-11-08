//! Session operations behind the modern non-player roles (dungeon master,
//! invader) and hot join. Rewrite extensions; see
//! research/multiplayer-scaling.md.
use super::*;
use crate::entity_command::EntityCommandHost;

impl PlaySession {
    /// Dungeon master: spawn `template` at `position` (snapped to the ground
    /// below), attacking players like an ordinary scripted spawn.
    pub fn spawn_npc_at(&mut self, template: &str, position: Vector) -> Result<u16, String> {
        let handle = self.spawn_npc(template, "attack", 0.0)?;
        let body_index = self
            .world
            .npcs
            .get(&handle)
            .ok_or("spawned NPC is not live")?
            .body;
        let ground = self
            .world
            .collision
            .collision()
            .segment_intersect(
                Segment {
                    origin: [position[0], position[1] + 2.0, position[2]],
                    delta: [0.0, -8.0, 0.0],
                },
                false,
            )
            .map(|hit| hit.hit.position);
        let body = &mut self.world.physics.bodies[body_index];
        let mut target = ground.unwrap_or(position);
        target[1] += body.length * 0.5 + body.radius;
        let orientation = body.orientation;
        body.move_to(target, orientation);
        Ok(handle)
    }

    /// Dungeon master: kill any character (player or NPC).
    pub fn smite(&mut self, handle: u16) -> Result<(), String> {
        if !self.world.entities.iter().any(|e| {
            e.handle == handle
                && matches!(
                    e.kind,
                    crate::scene_world::EntityKind::Avatar { .. }
                        | crate::scene_world::EntityKind::Npc { .. }
                )
        }) {
            return Err("no such character".into());
        }
        EntityCommandHost::kill(&mut self.world, handle).map_err(|e| format!("{e:?}"))
    }

    /// Dungeon master: restore a living character to full health.
    pub fn heal_character(&mut self, handle: u16) -> Result<(), String> {
        if let Some(npc) = self.world.npcs.get_mut(&handle).filter(|n| !n.dead()) {
            let max = npc.character.combat.health.max_hit_points;
            npc.character.combat.health.hit_points = max;
            self.world.physics.bodies[npc.body].owner.hit_points = max;
            return Ok(());
        }
        let player = self
            .world
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == handle && !p.dead())
            .ok_or("no living character")?;
        let max = player.character.combat.health.max_hit_points;
        player.character.combat.health.hit_points = max;
        self.world.physics.bodies[player.body].owner.hit_points = max;
        Ok(())
    }

    /// Where a party member is, for placing hot joiners, invaders and
    /// spectator cameras: the first living playing avatar.
    pub fn party_position(&self) -> Option<Vector> {
        self.world
            .players
            .iter()
            .flatten()
            .find(|p| p.playing && !p.dead())
            .map(|p| self.world.physics.bodies[p.body].position)
    }

    /// Move an existing avatar beside the party (dedicated hosts reuse slot
    /// zero's dormant avatar, which the start-locator join places first).
    pub fn move_near_party(&mut self, slot: usize) -> Result<(), SessionError> {
        let Some(anchor) = self.party_anchor(slot) else {
            return Ok(());
        };
        let player = self.player_avatar(slot)?;
        let index = player.body;
        let body = self.world.physics.bodies[index].clone();
        let placement = self.placement_near(&body, anchor, slot);
        let body = &mut self.world.physics.bodies[index];
        body.move_to(placement.position, placement.orientation);
        body.set_movement([0.0; 3]);
        Ok(())
    }

    /// Translate ordinary controls into possession controls for `npc`.
    pub fn pilot_npc(&mut self, npc: u16, controls: magicka_network::control::Controls) {
        let Some(body) = self.world.npcs.get(&npc).map(|n| n.body) else {
            return;
        };
        let position = self.world.physics.bodies[body].position;
        let toward_aim = controls.aim.map(|a| {
            let d = [a[0] - position[0], a[2] - position[2]];
            let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
            if l > 0.25 {
                [d[0] / l, d[1] / l]
            } else {
                [0.0, 0.0]
            }
        });
        let movement = controls
            .pad_movement
            .filter(|m| m[0].abs() + m[1].abs() > 0.05)
            .or(if controls.move_down { toward_aim } else { None });
        // The NPC's ability families (npc_combat::PilotAction), on the
        // wizard's layout: primary Cross/R2/right mouse, secondary L2/Shift,
        // self cast Triangle/middle mouse, boost Circle/Space, block R1.
        let actions = [
            controls.pad_attack || (controls.cast_down && !controls.shift_down),
            controls.shift_down,
            controls.cast_self_down,
            controls.boost_down,
            controls.pad_block,
        ];
        let acting = actions.iter().any(|a| *a);
        // Aim straight ahead when the pilot gave no aim point.
        let ahead = {
            let d = self.world.physics.bodies[body].direction();
            [
                position[0] + d[0] * 3.0,
                position[1],
                position[2] + d[2] * 3.0,
            ]
        };
        self.world.set_possession_controls(
            npc,
            crate::scene_world::possession::Possession {
                movement,
                attack: acting.then(|| controls.aim.unwrap_or(ahead)),
                aim: controls.aim,
                actions,
                boost_was_down: false,
            },
        );
    }
}
