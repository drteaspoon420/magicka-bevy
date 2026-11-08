//! Scene host for character grips: the Grip/DamageGrip/ReleaseGrip action
//! requests, ShouldReleaseGrip, the per-frame grip block of Character.Update
//! and ReleaseAttachedCharacter. Only the Hold type moves bodies; Pickup and
//! Ride need joint transforms and are refused. See research/npc-grip.md.
use super::*;
use crate::character_motion::CharacterState;
use crate::grip::{GripRequest, VictimFacts, GRIP_HOLD};

/// A character's grip-relevant view, from either an avatar or an NPC.
struct GripView {
    body: usize,
    dead: bool,
    hit_points: f32,
    radius: f32,
    state: CharacterState,
}

impl<C: SceneContent> SceneWorld<C> {
    fn grip_character_mut(
        &mut self,
        handle: u16,
    ) -> Option<&mut crate::character::RuntimeCharacter> {
        if let Some(n) = self.npcs.get_mut(&handle) {
            return Some(&mut n.character);
        }
        self.players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == handle)
            .map(|p| &mut p.character)
    }
    fn grip_character(&self, handle: u16) -> Option<&crate::character::RuntimeCharacter> {
        if let Some(n) = self.npcs.get(&handle) {
            return Some(&n.character);
        }
        self.players
            .iter()
            .flatten()
            .find(|p| p.handle == handle)
            .map(|p| &p.character)
    }
    fn grip_view(&self, handle: u16) -> Option<GripView> {
        if let Some(n) = self.npcs.get(&handle) {
            return Some(GripView {
                body: n.body,
                dead: n.character.combat.dead,
                hit_points: n.character.combat.health.hit_points,
                radius: n.definition.radius,
                state: n.motion.state,
            });
        }
        let p = self.players.iter().flatten().find(|p| p.handle == handle)?;
        Some(GripView {
            body: p.body,
            dead: p.character.combat.dead,
            hit_points: p.character.combat.health.hit_points,
            radius: p.character.definition.radius,
            state: p.motion.state,
        })
    }

    /// ShouldReleaseGrip's victim half for `gripper` (None: no live victim).
    pub(super) fn grip_victim_facts(
        &self,
        gripper: &crate::grip::GripLinks,
    ) -> Option<VictimFacts> {
        let victim = self.grip_character(gripper.gripped?)?;
        Some(VictimFacts {
            burning: victim
                .combat
                .statuses
                .has(magicka_sim::status::Status::Burning),
            immortal: victim.is_immortal(),
            break_free_counter: victim.grip.break_free_counter,
        })
    }

    /// Refresh a gripper's cached ShouldReleaseGrip before its update.
    pub(super) fn refresh_should_release(
        &self,
        character: &mut crate::character::RuntimeCharacter,
    ) {
        let victim = self.grip_victim_facts(&character.grip);
        character.grip.should_release = crate::grip::should_release(
            &character.grip,
            victim,
            character.definition.hit_tolerance,
        );
    }

    /// Character.ReleaseAttachedCharacter on `gripper`.
    pub fn release_grip(&mut self, gripper: u16) {
        let Some(view) = self.grip_view(gripper) else {
            return;
        };
        let Some(g) = self.grip_character_mut(gripper) else {
            return;
        };
        g.grip.damage_accumulation = 0.0;
        let Some(victim) = g.grip.gripped.take() else {
            return;
        };
        // The gripper's own state decides the victim's AllowMove.
        let hold_still = matches!(
            view.state,
            CharacterState::Dead
                | CharacterState::Idle
                | CharacterState::Move
                | CharacterState::Entangled
                | CharacterState::GripAttack
                | CharacterState::Resurrection
        );
        let Some(v) = self.grip_view(victim) else {
            return;
        };
        if let Some(c) = self.grip_character_mut(victim) {
            c.grip.break_free_counter = 0;
            c.grip.gripper = None;
        }
        // Drop the victim onto the ground below (a 6 m segment from 3 m up).
        let body = &self.physics.bodies[v.body];
        let mut origin = body.position;
        origin[1] += 3.0;
        let hit = self
            .collision
            .collision()
            .segment_intersect(
                magicka_sim::spray_pulse::Segment {
                    origin,
                    delta: [0.0, -6.0, 0.0],
                },
                false,
            )
            .map(|h| h.hit.position);
        let body = &mut self.physics.bodies[v.body];
        body.allow_move = !hold_still;
        body.allow_rotate = true;
        if let Some(mut p) = hit {
            p[1] += body.radius + body.length * 0.5 + 0.1;
            let o = body.orientation;
            body.move_to(p, o);
        }
        self.log.push(format!("grip release {gripper} {victim}"));
    }

    /// AnimationActions.Grip / DamageGrip / ReleaseGrip of this frame's NPC
    /// updates (after the owners' Character.Update, like DealDamage).
    pub(super) fn execute_grip_requests(&mut self) {
        let owners: Vec<u16> = self
            .npcs
            .iter()
            .filter(|(_, n)| !n.grip_requests.is_empty())
            .map(|(&h, _)| h)
            .collect();
        for owner in owners {
            let requests = std::mem::take(&mut self.npcs.get_mut(&owner).unwrap().grip_requests);
            for request in requests {
                match request {
                    GripRequest::Grip {
                        grip_type,
                        radius,
                        tolerance,
                        uses_attach,
                        finish_on_grip,
                    } => self.grip_action(
                        owner,
                        grip_type,
                        radius,
                        tolerance,
                        uses_attach,
                        finish_on_grip,
                    ),
                    GripRequest::Damage { to_owner, damages } => {
                        let target = if to_owner {
                            Some(owner)
                        } else {
                            self.npcs[&owner].character.grip.gripped
                        };
                        if let Some(target) = target {
                            let position = self.physics.bodies[self.npcs[&owner].body].position;
                            self.grip_damage(target, owner, &damages, position);
                        }
                    }
                    // SetCollisionDamage(empty) on the victim: collision
                    // damage is not hosted for characters here.
                    GripRequest::Release => self.release_grip(owner),
                }
            }
        }
    }

    fn grip_action(
        &mut self,
        owner: u16,
        grip_type: u8,
        radius: f32,
        tolerance: i32,
        uses_attach: bool,
        finish_on_grip: bool,
    ) {
        let npc = &self.npcs[&owner];
        if npc.character.grip.gripping() {
            return;
        }
        let Some(target) = npc.combat.current_target() else {
            return;
        };
        let Some(victim) = self.grip_view(target) else {
            return;
        };
        let Some(vc) = self.grip_character(target) else {
            return;
        };
        let drowning = self
            .npcs
            .get(&target)
            .map(|n| n.motion.drowning)
            .or_else(|| {
                self.players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == target)
                    .map(|p| p.motion.drowning)
            })
            .unwrap_or(false);
        if vc.grip.gripper.is_some() || victim.dead || drowning || vc.combat.health.shield.solid() {
            return;
        }
        // INFERRED: an authored grip attach bone is approximated by the owner
        // position with the authored radius (joint poses are not hosted).
        let reach = if uses_attach {
            radius
        } else {
            npc.definition.radius
        };
        let from = self.physics.bodies[npc.body].position;
        let to = self.physics.bodies[victim.body].position;
        let (dx, dz) = (from[0] - to[0], from[2] - to[2]);
        let r = victim.radius + reach;
        if dx * dx + dz * dz >= r * r {
            return;
        }
        let segment = magicka_sim::spray_pulse::Segment {
            origin: from,
            delta: [to[0] - from[0], to[1] - from[1], to[2] - from[2]],
        };
        if self
            .collision
            .collision()
            .segment_intersect(segment, false)
            .is_some()
        {
            return;
        }
        // Shield.SegmentIntersect (INFERRED: the action radius sweep is not
        // applied; the shield primitive is tested with the bare segment).
        if self
            .shields
            .iter()
            .any(|s| !s.shield.dead && s.shield.geometry().segment_fraction(segment).is_some())
        {
            return;
        }
        self.grip_character_link(owner, target, grip_type, tolerance, finish_on_grip);
    }

    /// Character.GripCharacter (offline).
    fn grip_character_link(
        &mut self,
        owner: u16,
        target: u16,
        grip_type: u8,
        tolerance: i32,
        finish_on_grip: bool,
    ) {
        let (Some(victim), Some(gripper)) = (self.grip_view(target), self.grip_view(owner)) else {
            return;
        };
        let vc = self.grip_character(target).unwrap();
        let polymorphed = self
            .players
            .iter()
            .flatten()
            .any(|p| p.handle == target && p.polymorphed);
        if vc.grip.gripper.is_some()
            || victim.dead
            || vc.is_immortal()
            || vc.ethereal
            || polymorphed
            || self.physics.bodies[victim.body].mass > 500.0
            || gripper.state == CharacterState::KnockDown
        {
            return;
        }
        if grip_type != GRIP_HOLD {
            // Pickup and Ride follow joint transforms (not hosted).
            self.log.push(format!("grip type {grip_type} not hosted"));
            return;
        }
        let victim_links = vc.grip;
        if let Some(other) = victim_links
            .gripper
            .filter(|_| victim_links.gripper_attached())
        {
            self.release_grip(other);
        }
        if victim_links.gripping() {
            self.release_grip(target);
        }
        let g = &mut self.npcs.get_mut(&owner).unwrap().character.grip;
        g.grip_type = grip_type;
        g.tolerance = if tolerance > 0 { tolerance as f32 } else { 3.0 };
        g.damage_accumulation = 0.0;
        g.gripped = Some(target);
        g.break_free_counter = 0;
        let vc = self.grip_character_mut(target).unwrap();
        // Character.GripCharacter: iVictim.ReleaseEntanglement().
        vc.entanglement.release(&mut vc.entangle_events);
        let v = &mut vc.grip;
        v.gripper = Some(owner);
        v.grip_type = grip_type;
        v.break_free_counter = 0;
        // Body.Immovable for the gripper (and a Pickup victim) is not modelled.
        if finish_on_grip {
            self.npcs.get_mut(&owner).unwrap().motion.forced_state = Some(CharacterState::Gripping);
        }
        self.log.push(format!("grip {owner} {target}"));
    }

    /// Character.Damage(DamageCollection5, gripper, PlayTime, gripper position).
    pub(super) fn grip_damage(
        &mut self,
        target: u16,
        attacker: u16,
        damages: &[magicka_sim::damage::Damage],
        position: [f32; 3],
    ) {
        let mut slots = [magicka_sim::damage::Damage::default(); 5];
        for (slot, d) in slots.iter_mut().zip(damages) {
            *slot = *d;
        }
        let payload = magicka_sim::damage_delivery::DamagePayload::Collection(slots);
        let play_time = self.play_time;
        if let Some(npc) = self.npcs.get_mut(&target) {
            let result = crate::avatar::deliver_to_body(
                &mut npc.character,
                &mut self.physics.bodies[npc.body],
                &[magicka_sim::resistance::Resistance::IDENTITY; 11],
                false,
                target,
                payload,
                Some(attacker),
                position,
                play_time,
            );
            if result & magicka_sim::character_impact::result::DAMAGED != 0 && npc.on_damage != 0 {
                self.pending_triggers.push((npc.on_damage, Some(target)));
            }
        } else if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == target)
        {
            let body = &mut self.physics.bodies[p.body];
            p.receive_payload(body, payload, Some(attacker), position, play_time);
        }
    }

    /// The grip block of every character's Character.Update (after its state
    /// update): gripper release tests and Hold placement, then the victim's
    /// own release and the gripper's speed lock.
    pub(super) fn update_grips(&mut self) {
        let mut handles: Vec<u16> = self.npcs.keys().copied().collect();
        handles.extend(self.players.iter().flatten().map(|p| p.handle));
        for handle in handles.clone() {
            let Some(c) = self.grip_character(handle) else {
                continue;
            };
            let Some(victim) = c.grip.gripped else {
                continue;
            };
            let links = c.grip;
            let hit_tolerance = c.definition.hit_tolerance;
            let facts = self.grip_victim_facts(&links);
            let view = self.grip_view(handle).unwrap();
            let victim_view = self.grip_view(victim);
            let victim_dead = victim_view.as_ref().is_none_or(|v| v.dead);
            if view.hit_points < 0.0
                || victim_dead
                || crate::grip::should_release(&links, facts, hit_tolerance)
            {
                self.release_grip(handle);
                continue;
            }
            if links.grip_type != GRIP_HOLD {
                continue;
            }
            let victim_view = victim_view.unwrap();
            let v = &mut self.physics.bodies[victim_view.body];
            v.set_speed_multiplier(0.0);
            v.allow_rotate = false;
            v.allow_move = false;
            let victim_position = v.position;
            let g = &mut self.physics.bodies[view.body];
            let mut forward = [
                victim_position[0] - g.position[0],
                victim_position[1] - g.position[1],
                victim_position[2] - g.position[2],
            ];
            let l2 = forward.iter().map(|x| x * x).sum::<f32>();
            forward = if l2 < 1e-6 {
                [0.0, 0.0, -1.0]
            } else {
                forward.map(|x| x / l2.sqrt())
            };
            let reach = -view.radius - victim_view.radius;
            let position = [0, 1, 2].map(|i| victim_position[i] + forward[i] * reach);
            // Matrix.CreateWorld(position, forward, Up) with the translation
            // cleared.
            let orientation = magicka_sim::character_physics::Orientation::from_forward_up(
                forward,
                [0.0, 1.0, 0.0],
            );
            g.move_to(position, orientation);
            let v = &mut self.physics.bodies[victim_view.body];
            v.allow_rotate = false;
            v.allow_move = false;
        }
        for handle in handles {
            let Some(c) = self.grip_character(handle) else {
                continue;
            };
            let Some(gripper) = c.grip.gripper else {
                continue;
            };
            let grip_type = c.grip.grip_type;
            let dead = c.combat.dead;
            match self.grip_view(gripper) {
                Some(g) if !(dead | g.dead) => {
                    if grip_type != crate::grip::GRIP_RIDE {
                        self.physics.bodies[g.body].set_speed_multiplier(0.0);
                    }
                }
                Some(_) => {
                    self.release_grip(gripper);
                    if let Some(c) = self.grip_character_mut(handle) {
                        c.grip.gripper = None;
                    }
                }
                None => {
                    // The gripper is gone (removed entity): unlink.
                    if let Some(c) = self.grip_character_mut(handle) {
                        c.grip.gripper = None;
                        c.grip.break_free_counter = 0;
                    }
                    let body = self.grip_view(handle).unwrap().body;
                    let b = &mut self.physics.bodies[body];
                    b.allow_move = true;
                    b.allow_rotate = true;
                }
            }
        }
    }
}
