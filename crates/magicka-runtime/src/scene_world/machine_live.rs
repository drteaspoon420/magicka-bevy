//! Scene host for the Machine boss (research/boss-battles.md): its two
//! damage zones, the warlock Gunter and the commands of
//! `boss_machine::Machine`.
use super::*;
use crate::boss_machine::{Machine, MachineCommand, MachineInputs, ZONES};

/// A live Machine and the scene entities it owns.
#[derive(Clone, Debug)]
pub struct MachineHost {
    pub machine: Box<Machine>,
    pub warlock: u16,
    pub zones: [u16; 2],
}

impl PartialEq for MachineHost {
    fn eq(&self, other: &Self) -> bool {
        self.warlock == other.warlock && self.zones == other.zones
    }
}

fn forward(m: &[f32; 16]) -> [f32; 3] {
    [-m[8], -m[9], -m[10]]
}

impl<C: SceneContent> SceneWorld<C> {
    /// MethodDef 0x0600020c IL_0063–IL_01b6. This is a boss-owned NPC;
    /// no corresponding <spawn> exists in the scene script.
    pub(super) fn initialize_machine_warlock(&mut self) -> Result<u16, WorldError> {
        const TEMPLATE: &str = "boss_machine_warlock";
        crate::spawn_action::SpawnLevel::load_character_template(self, Some(TEMPLATE))?;
        let locator = self.scene_locator(custom_hash("spawn_warlock"))?;
        let origin = [locator[12], locator[13] + 1., locator[14]];
        let mut position = self
            .collision
            .collision()
            .segment_intersect(
                Segment {
                    origin,
                    delta: [0., -5., 0.],
                },
                false,
            )
            .map_or([locator[12], locator[13], locator[14]], |hit| {
                hit.hit.position
            });
        let template = &self.templates[&custom_hash(TEMPLATE)];
        // Native Machine adds this lift before Character.Initialize applies
        // its own HeightOffset. Keep both operations in their original order.
        position[1] += template.radius + template.length * 0.5 + 0.1;
        let handle = SpawnHost::acquire_npc(self)?.handle;
        SpawnHost::initialize_npc(
            self,
            handle,
            custom_hash(TEMPLATE),
            0,
            position,
            custom_hash(TEMPLATE),
        )?;
        let n = self
            .npcs
            .get_mut(&handle)
            .ok_or("Machine warlock requires a live actor")?;
        n.set_order(1, 0, 4, 0, None);
        self.physics.bodies[n.body].set_desired_direction([-locator[8], -locator[9], -locator[10]]);
        SpawnHost::add_entity(self, handle)?;
        let n = self.npcs.get_mut(&handle).unwrap();
        n.character.combat.cannot_die_without_explicit_kill = true;
        n.character.immortal_time = f32::INFINITY;
        n.set_order(1, 0, 1, 0, None);
        let e = self.entity_mut(handle)?;
        e.cannot_die_without_explicit_kill = true;
        e.order = Some(1);
        Ok(handle)
    }

    /// Machine.Initialize at the spawn locator: zones, warlock and models.
    pub(super) fn spawn_machine(&mut self, area: i32) -> Result<MachineHost, WorldError> {
        let model = self
            .content
            .skinned_model(crate::boss_machine::MACHINE_MODEL)
            .ok_or("missing Machine model")?;
        let king = self
            .content
            .skinned_model(crate::boss_machine::KING_MODEL)
            .ok_or("missing King model")?;
        let mut machine = Machine::new(model, king, 0x6d61_6368)?;
        let locator = |w: &Self, name: &str| {
            w.locators
                .get(&custom_hash(name))
                .copied()
                .ok_or_else(|| format!("machine: missing locator {name}"))
        };
        let orientation = self
            .locators
            .get(&area)
            .copied()
            .ok_or("machine: missing spawn area")?;
        let king_locator = locator(self, "spawn_king")?;
        let emperor = locator(self, "teleport_warlock")?;
        machine.initialize(orientation, king_locator, emperor);
        // The warlock: boss-owned (no <spawn> in the scene script), idle,
        // immortal and unable to die without an explicit kill.
        let warlock = self.initialize_machine_warlock()?;
        // Two immovable damage zones rotated by INV_SCENE_ROTATION.
        let mut zones = [0u16; 2];
        for (index, (corner, sides, radius)) in ZONES.iter().enumerate() {
            let handle = self.allocate_physics(true);
            let position = machine.zone_position(index);
            let mut transform = crate::boss_machine::inv_scene_rotation();
            transform[12..15].copy_from_slice(&position);
            let collision = magicka_content::physics::CollisionBox {
                name: format!("machine_zone{index}"),
                position: *corner,
                sides: *sides,
                orientation: [0.0, 0.0, 0.0, 1.0],
            };
            self.physics_damage.insert(
                handle,
                crate::physics_damage::PhysicsDamage::boss_zone(index as u8, collision.clone()),
            );
            self.static_prop_boxes.insert(handle, collision);
            let e = self.entity_mut(handle)?;
            e.position = position;
            e.orientation = transform;
            e.radius = *radius;
            e.hit_points = 1.0;
            e.max_hit_points = 1.0;
            e.added = true;
            zones[index] = handle;
        }
        self.log
            .push(format!("boss machine: warlock {warlock}, zones {zones:?}"));
        Ok(MachineHost {
            machine: Box::new(machine),
            warlock,
            zones,
        })
    }

    pub(super) fn machine_health(&self, host: &MachineHost) -> (f32, f32) {
        let (hp, max) = self
            .npcs
            .get(&host.warlock)
            .map(|n| {
                (
                    n.character.combat.health.hit_points,
                    n.character.combat.health.max_hit_points,
                )
            })
            .unwrap_or((0.0, 1.0));
        host.machine.health(hp, max)
    }

    /// Machine.UpdateBoss with the zone deliveries of the last frame.
    pub(super) fn update_machine(&mut self, host: &mut MachineHost, dt: f32, fight_started: bool) {
        let mut commands = Vec::new();
        for (part, zone) in host.zones.iter().enumerate() {
            let position = host.machine.zone_position(part);
            let inbox = self
                .physics_damage
                .get_mut(zone)
                .map(|z| z.take_boss_inbox(self.boss_damage_divisor))
                .unwrap_or_default();
            for (damage, features) in inbox {
                host.machine
                    .damage(part, damage, position, features, &mut commands);
            }
        }
        let (hp, max) = self
            .npcs
            .get(&host.warlock)
            .map(|n| {
                (
                    n.character.combat.health.hit_points,
                    n.character.combat.health.max_hit_points,
                )
            })
            .unwrap_or((0.0, 1.0));
        commands.extend(host.machine.update(
            dt,
            MachineInputs {
                fight_started,
                warlock_hit_points: hp,
                warlock_max_hit_points: max,
            },
        ));
        for command in commands {
            self.machine_command(host, command);
        }
    }

    fn machine_command(&mut self, host: &mut MachineHost, command: MachineCommand) {
        let warlock = host.warlock;
        match command {
            MachineCommand::Trigger(name) => self.pending_triggers.push((custom_hash(name), None)),
            MachineCommand::EffectTransform { name, transform } => {
                self.status_contact_effects.push((
                    name,
                    [transform[12], transform[13], transform[14]],
                    forward(&transform),
                ));
            }
            MachineCommand::Effect {
                name,
                position,
                direction,
            } => self
                .status_contact_effects
                .push((name, position, direction)),
            MachineCommand::Cue {
                bank,
                name,
                position,
            } => {
                self.play_cue(bank, name, Some(position));
            }
            MachineCommand::WarlockDialog(dialog) => {
                let action = crate::dialog_action::DialogAction {
                    delay: 0.0,
                    kind: crate::dialog_action::DialogActionKind::Start(
                        crate::dialog_action::StartDialogAction {
                            dialog: custom_hash(dialog),
                            owner: custom_hash(crate::boss_machine::WARLOCK_TEMPLATE),
                            position: [0.0; 3],
                            is_2d: false,
                            is_3d: false,
                            force_on_screen: false,
                        },
                    ),
                };
                if let Some(Err(e)) = self.world_dialog_action(&action, None) {
                    self.log.push(format!("machine taunt: {e}"));
                }
            }
            MachineCommand::KillSpellEntities => {
                let barriers: Vec<usize> = self.barriers.iter().map(|(slot, _)| slot).collect();
                for slot in barriers {
                    self.kill_barrier(slot);
                }
                for missile in &mut self.missiles {
                    missile.body.dead = true;
                }
                let mines: Vec<usize> = self.mines.iter().map(|(slot, _)| slot).collect();
                for slot in mines {
                    if let Some(m) = self.mines.get_mut(slot) {
                        m.kill();
                    }
                }
                let grease: Vec<usize> = self.grease.iter().map(|(slot, _)| slot).collect();
                for slot in grease {
                    self.kill_grease(slot);
                }
            }
            MachineCommand::WarlockOrder {
                order,
                react_to,
                reaction,
                emperor,
                pop_to_idle: _,
            } => {
                let events = emperor.then(|| {
                    let spec_action0 =
                        crate::animation_controller::animation_value("spec_action0").unwrap_or(0);
                    vec![
                        crate::ai_event::AiEvent::Animation(crate::ai_event::AnimationEvent {
                            animation: spec_action0,
                            idle_animation: 0,
                            delay: 0.15,
                            blend_time: 0.01,
                            trigger: 0,
                        });
                        2
                    ]
                });
                // SetOrder pops the AI back to idle, releasing attack targets.
                if let Some(n) = self.npcs.get_mut(&warlock) {
                    n.set_order(order, react_to, reaction, 0, events);
                }
                if let Ok(e) = self.entity_mut(warlock) {
                    e.order = Some(order);
                }
            }
            MachineCommand::WarlockImmortal(t) => {
                if let Some(n) = self.npcs.get_mut(&warlock) {
                    n.character.immortal_time = t;
                }
            }
            MachineCommand::WarlockFullHealth => {
                if let Some(n) = self.npcs.get_mut(&warlock) {
                    let h = &mut n.character.combat.health;
                    h.hit_points = h.max_hit_points;
                }
            }
            MachineCommand::WarlockStopStatuses(mask) => {
                if let Some(n) = self.npcs.get_mut(&warlock) {
                    n.character.stop_status_effects(mask, false);
                }
            }
            MachineCommand::WarlockTeleport(m) => self.machine_teleport_warlock(warlock, m),
            MachineCommand::WarlockHold(m) => {
                let Some(body) = self.npcs.get(&warlock).map(|n| n.body) else {
                    return;
                };
                let mut position = [m[12], m[13], m[14]];
                let segment = magicka_sim::spray_pulse::Segment {
                    origin: position,
                    delta: [0.0, -2.0, 0.0],
                };
                if let Some(hit) = self.collision.collision().segment_intersect(segment, false) {
                    position = hit.hit.position;
                }
                let b = &mut self.physics.bodies[body];
                position[1] -= b.height_offset();
                let o = magicka_sim::character_physics::Orientation {
                    right: [m[0], m[1], m[2]],
                    up: [m[4], m[5], m[6]],
                    backward: [m[8], m[9], m[10]],
                };
                b.move_to(position, o);
            }
            MachineCommand::EmperorLightning => self.machine_emperor_lightning(warlock),
            MachineCommand::PlayLevelPart(name) => {
                let id = custom_hash(name);
                let action = crate::animation_action::AnimationAction {
                    delay: 0.0,
                    path: self.collision.part_path(id).unwrap_or_else(|| vec![id]),
                    children: true,
                    operation: crate::animation_action::AnimationOperation::Play(
                        crate::animated_hierarchy::PlayCommand {
                            start: -1.0,
                            end: -1.0,
                            speed: 1.0,
                            looping: false,
                            resume: false,
                        },
                    ),
                };
                if let Err(e) = self.collision.execute_animation_action(&action) {
                    self.log.push(format!("machine {name}: {e:?}"));
                }
            }
            MachineCommand::WarlockTerminate => {
                if let Some(n) = self.npcs.get_mut(&warlock) {
                    n.character.combat.cannot_die_without_explicit_kill = false;
                    n.character.immortal_time = 0.0;
                    let body = n.body;
                    let o = self.physics.bodies[body].orientation;
                    self.physics.bodies[body].move_to([0.0, -101.0, 0.0], o);
                }
                if let Ok(e) = self.entity_mut(warlock) {
                    e.cannot_die_without_explicit_kill = false;
                }
                self.terminate_character(warlock);
            }
            MachineCommand::EndGameDefeat => {
                self.pending_end_game =
                    Some((crate::end_game::EndGameCondition::Defeat as u8, true, false));
            }
            MachineCommand::GibKing { transform } => self.machine_gib_king(warlock, transform),
            MachineCommand::Boss(event) => self.machine_boss_event(host, event),
        }
    }

    fn machine_boss_event(
        &mut self,
        host: &mut MachineHost,
        event: magicka_sim::boss_status::BossEvent,
    ) {
        // NotifierTextPostion: the orc zone position, 2 m up.
        let mut notifier = host.machine.zone_position(1);
        notifier[1] += 2.0;
        self.present_boss_event(event, notifier, "gore_splash_black");
    }

    /// Machine.TeleportWarlock.
    fn machine_teleport_warlock(&mut self, warlock: u16, m: [f32; 16]) {
        let Some(body) = self.npcs.get(&warlock).map(|n| n.body) else {
            return;
        };
        let (from, from_dir) = {
            let b = &self.physics.bodies[body];
            (b.position, b.direction())
        };
        self.status_contact_effects
            .push(("magick_teleport_disappear", from, from_dir));
        self.play_cue(crate::audio::banks::SPELLS, "magick_teleporta", Some(from));
        let mut position = [m[12], m[13], m[14]];
        let direction = forward(&m);
        let segment = magicka_sim::spray_pulse::Segment {
            origin: [position[0], position[1] + 1.0, position[2]],
            delta: [0.0, -3.0, 0.0],
        };
        if let Some(hit) = self.collision.collision().segment_intersect(segment, false) {
            position = hit.hit.position;
            position[1] += 0.1;
        }
        let b = &mut self.physics.bodies[body];
        position[1] += b.length * 0.5 + b.radius;
        self.status_contact_effects
            .push(("magick_teleport_appear", position, direction));
        self.play_cue(
            crate::audio::banks::SPELLS,
            "magick_teleportb",
            Some(position),
        );
        if let Some(n) = self.npcs.get_mut(&warlock) {
            n.character.stop_status_effects(
                magicka_sim::status::Status::Burning as u16
                    | magicka_sim::status::Status::Cold as u16,
                false,
            );
        }
        let b = &mut self.physics.bodies[body];
        b.velocity = [0.0; 3];
        b.set_movement([0.0; 3]);
        b.set_desired_direction(direction);
        let o = magicka_sim::character_physics::Orientation {
            right: [m[0], m[1], m[2]],
            up: [m[4], m[5], m[6]],
            backward: [m[8], m[9], m[10]],
        };
        b.move_to(position, o);
    }

    /// FinalStage's two lightning bolts from the warlock's hands. INFERRED:
    /// the hand attach points are approximated at shoulder height to each
    /// side of the body; each bolt reaches 10 m along its random direction.
    /// Machine FinalStage's two bolts: from the warlock's animated hands, away from
    /// his hip, with each axis of the direction randomized (`Machine.cs:721-744`).
    pub fn machine_emperor_lightning(&mut self, warlock: u16) {
        let Some(n) = self.npcs.get(&warlock) else {
            return;
        };
        let translation = |m: &[f32; 16]| [m[12], m[13], m[14]];
        // GetRightAttachOrientation, then GetLeftAttachOrientation.
        let hands = [translation(&n.weapon_transform), translation(&n.staff_orb)];
        let hip = translation(&n.hip_transform);
        let normalize = |v: [f32; 3]| {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
            v.map(|c| c / l)
        };
        for hand in hands {
            let mut d = normalize([0, 1, 2].map(|i| hand[i] - hip[i]));
            for v in &mut d {
                *v *= self.random() * 5.0;
            }
            let d = normalize(d);
            self.magick_bolts.push(magicka_sim::lightning::BoltVisual {
                from: hand,
                to: [0, 1, 2].map(|i| hand[i] + d[i] * 10.0),
                direction: d,
                color: LIGHTNING_COLOR,
                scale: 1.0,
                ttl: THUNDERBOLT_BOLT_TTL,
            });
        }
    }

    /// Machine.OverKillKing at the drill tip: the warlock template's gibs
    /// with wood blood, the medium wood gore effect, misc_gib and a shake.
    fn machine_gib_king(&mut self, warlock: u16, transform: [f32; 16]) {
        let position = [transform[12], transform[13], transform[14]];
        let gibs = self
            .npcs
            .get(&warlock)
            .map(|n| n.definition.gibs.clone())
            .unwrap_or_default();
        let source = crate::gibs::GibSource {
            position,
            capsule_radius: 0.5,
            capsule_length: 1.0,
            radius: 0.5,
            mass: 100.0,
            frozen: false,
            blood: 3,
        };
        let visuals = self.gibs.spawn(source, &gibs);
        self.gib_visuals.extend(visuals);
        self.play_cue(
            crate::audio::banks::MISC,
            crate::gibs::SOUND_GIB,
            Some(position),
        );
        self.camera.shake(1.0, 1.0);
        self.log.push("machine: king killed".into());
    }
}
