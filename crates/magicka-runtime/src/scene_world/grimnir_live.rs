//! The first Grimnir encounter's authored dream venues and minion ownership.
use super::*;
use crate::{
    boss_fafnir::ModelPose,
    boss_grimnir::{self as policy, Command, Dreams},
    character_pose::CharacterSkeleton,
};

#[derive(Clone, Debug)]
pub struct GrimnirHost {
    pub dreams: Dreams,
    pub minions: Vec<u16>,
    pub skeleton: CharacterSkeleton,
    pub pose: ModelPose,
    pub failed: Option<String>,
    controller: crate::animation_controller::AnimationController,
    time: f32,
    rng: u64,
}
impl PartialEq for GrimnirHost {
    fn eq(&self, other: &Self) -> bool {
        self.dreams == other.dreams && self.minions == other.minions && self.failed == other.failed
    }
}

impl<C: SceneContent> SceneWorld<C> {
    pub(super) fn spawn_grimnir(&mut self) -> Result<GrimnirHost, WorldError> {
        for name in policy::MINIONS {
            SpawnLevel::load_character_template(self, Some(name))?;
        }
        // Fail before installing the encounter if any authored locator is absent.
        for venue in policy::VENUES {
            for slot in 0..4 {
                self.scene_locator(custom_hash(&format!("start{venue}{slot}")))?;
                self.scene_locator(custom_hash(&format!("spawn{venue}{slot}")))?;
            }
        }
        let model = self
            .content
            .skinned_model(policy::ANIMATION)
            .ok_or("missing Grimnir animation model")?;
        let skeleton = CharacterSkeleton::new(model.skeleton, model.clips);
        let duration = *skeleton
            .clip_durations()
            .get("idle")
            .ok_or("missing Grimnir idle clip")?;
        let mut controller = crate::animation_controller::AnimationController::default();
        controller.start_named_clip("idle", duration, true);
        self.render_transition =
            crate::render_transition::RenderTransition::start(1, false, [255; 4], 0.25);
        for p in self.players.iter_mut().flatten().filter(|p| p.playing) {
            p.go_to_animation(131 + p.slot as u16, 0.);
        }
        self.log.push("boss grimnir: six dream venues".into());
        Ok(GrimnirHost {
            dreams: Dreams::default(),
            minions: vec![],
            skeleton,
            pose: ModelPose::default(),
            failed: None,
            controller,
            time: 0.,
            rng: 0x67_72_69_6d_6e_69_72,
        })
    }

    fn grimnir_teleport(&mut self, venue: usize) -> Result<(), WorldError> {
        for slot in 0..self.players.len() {
            let Some(p) = self.players[slot]
                .as_ref()
                .filter(|p| p.playing && !p.dead())
            else {
                continue;
            };
            let body = p.body;
            // Four authored locators; extra slots stand behind them
            // (play_session::slot_matrix).
            let base = self.scene_locator(custom_hash(&format!(
                "start{}{}",
                policy::VENUES[venue],
                slot % 4
            )))?;
            let m = crate::play_session::slot_matrix(&[base], slot / 4 * 4);
            let origin = [m[12], m[13], m[14]];
            let mut position = self
                .collision
                .collision()
                .segment_intersect(
                    Segment {
                        origin,
                        delta: [0., -7., 0.],
                    },
                    false,
                )
                .map_or(origin, |h| h.hit.position);
            let b = &mut self.physics.bodies[body];
            position[1] += b.radius + b.length * 0.5 + 0.1;
            b.move_to(
                position,
                magicka_sim::character_physics::Orientation::from_forward_up(
                    [-m[8], -m[9], -m[10]],
                    [m[4], m[5], m[6]],
                ),
            );
        }
        let m = self.scene_locator(custom_hash(&format!("start{}1", policy::VENUES[venue])))?;
        self.camera.set_position([m[12], m[13], m[14]], false);
        Ok(())
    }

    fn grimnir_spawn_wave(
        &mut self,
        host: &mut GrimnirHost,
        venue: usize,
    ) -> Result<(), WorldError> {
        host.minions.clear();
        host.rng ^= host.rng << 13;
        host.rng ^= host.rng >> 7;
        host.rng ^= host.rng << 17;
        let template = policy::MINIONS[venue];
        let t = self.templates[&custom_hash(template)].clone();
        for slot in policy::spawn_slots(venue, (host.rng % 4) as usize) {
            let m = self.scene_locator(custom_hash(&format!(
                "spawn{}{slot}",
                policy::VENUES[venue]
            )))?;
            let origin = [m[12], m[13], m[14]];
            let mut position = self
                .collision
                .collision()
                .segment_intersect(
                    Segment {
                        origin: [origin[0], origin[1] + 1., origin[2]],
                        delta: [0., -5., 0.],
                    },
                    false,
                )
                .map_or(origin, |h| h.hit.position);
            // Native passes this elevated point as Initialize's feet position.
            position[1] += t.radius + t.length * 0.5 + 0.1;
            let handle = SpawnHost::acquire_npc(self)?.handle;
            SpawnHost::initialize_npc(self, handle, custom_hash(template), 0, position, 0)?;
            let direction = [-m[8], -m[9], -m[10]];
            if let Some(n) = self.npcs.get_mut(&handle) {
                self.physics.bodies[n.body].set_desired_direction(direction);
                n.set_order(
                    crate::npc::ORDER_ATTACK,
                    crate::npc::REACT_ATTACK | crate::npc::REACT_PROXIMITY,
                    crate::npc::ORDER_ATTACK,
                    0,
                    None,
                );
            }
            SpawnHost::add_entity(self, handle)?;
            self.status_contact_effects
                .push(("magick_teleport_appear", position, direction));
            let at = self
                .npcs
                .get(&handle)
                .map(|n| self.physics.bodies[n.body].position);
            // Native intentionally uses the origin cue with the appear effect.
            self.play_cue(crate::audio::banks::SPELLS, "magick_teleporta", at);
            host.minions.push(handle);
        }
        Ok(())
    }

    pub(super) fn update_grimnir(&mut self, host: &mut GrimnirHost, dt: f32) {
        if host.failed.is_none() {
            for command in host.dreams.update(dt, host.minions.len()) {
                let result = match command {
                    Command::Spawn(venue) => self.grimnir_spawn_wave(host, venue),
                    Command::Teleport(venue) => self.grimnir_teleport(venue),
                    Command::FadeOut | Command::FadeIn => {
                        self.render_transition = crate::render_transition::RenderTransition::start(
                            1,
                            command == Command::FadeOut,
                            [255; 4],
                            0.25,
                        );
                        Ok(())
                    }
                };
                if let Err(error) = result {
                    self.log.push(format!("Grimnir encounter stopped: {error}"));
                    host.failed = Some(error);
                    break;
                }
            }
        }
        // UpdateBoss removes Dead entries only after the state has examined them.
        host.minions
            .retain(|h| self.npcs.get(h).is_some_and(|n| !n.dead()));
        host.time += dt;
        host.controller.update(dt);
        // Native Position is ground_position + CAMERA_OFFSET. Inspection is
        // a viewer-only override and must not anchor the simulated apparition.
        let camera = self.camera.ground_position();
        let mut parent = crate::boss_machine::rotation_y_pi();
        parent[12] = camera[0] - (host.time as f64).cos() as f32 * 0.125;
        parent[13] = camera[1] - 28. - (host.time as f64).sin() as f32 * 0.125;
        parent[14] = camera[2] - 30.;
        for i in [0, 1, 2, 4, 5, 6, 8, 9, 10] {
            parent[i] *= 7.;
        }
        host.pose = ModelPose {
            parent,
            local: host.skeleton.local_poses(host.controller.pose_sample()),
        };
    }
}
