//! Scene host for Fafnir (research/boss-battles.md): bone-bound damage
//! zones, the avatars he sees, and the commands of `boss_fafnir::Fafnir`.
use super::*;
use crate::boss_fafnir::{Avatar, Fafnir, FafnirCommand, FafnirInputs};
use magicka_content::common::{Event, EventCollection, EventCondition};

/// Zone boxes (INFERRED from the native capsules: half extents along the
/// zone transform's axes): head, body, right heel, right wrist.
const ZONE_BOXES: [([f32; 3], u8); 4] = [
    ([1.0, 1.0, 2.5], 0),
    ([3.2, 3.2, 4.5], 1),
    ([1.0, 1.0, 2.0], 2),
    ([1.0, 1.0, 2.0], 2),
];
const ZONE_RADII: [f32; 4] = [3.0, 3.2, 2.0, 2.0];

#[derive(Clone, Debug)]
pub struct FafnirHost {
    pub fafnir: Box<Fafnir>,
    pub zones: [u16; 4],
}
impl PartialEq for FafnirHost {
    fn eq(&self, other: &Self) -> bool {
        self.zones == other.zones
    }
}

fn condition(kind: u8, time: f32, repeat: bool, events: Vec<Event>) -> EventCollection {
    EventCollection {
        condition: EventCondition {
            condition_type: kind,
            hitpoints: 0,
            elements: 0,
            threshold: 0.0,
            time,
        },
        repeat,
        events,
    }
}
fn effect(name: &str, follow: bool) -> Event {
    Event::Effect {
        follow,
        world_align: false,
        effect: name.into(),
    }
}
fn sound(name: &str, stop: bool) -> Event {
    Event::Sound {
        bank: crate::audio::banks::SPELLS as i32,
        sound: name.into(),
        magnitude: 1.0,
        stop_on_remove: stop,
    }
}
fn splash(attack: u16, element: u32, amount: f32, magnitude: f32, radius: f32) -> Event {
    Event::Splash {
        damage: magicka_content::common::Damage {
            attack_property: attack as i32,
            element: element as i32,
            amount,
            magnitude,
        },
        radius,
    }
}

impl<C: SceneContent> SceneWorld<C> {
    /// Fafnir.Initialize at the spawn locator.
    pub(super) fn spawn_fafnir(&mut self, area: i32) -> Result<FafnirHost, WorldError> {
        let mesh = self
            .content
            .skinned_model(crate::boss_fafnir::MESH_MODEL)
            .ok_or("missing Fafnir mesh")?;
        let animation = self
            .content
            .skinned_model(crate::boss_fafnir::ANIMATION_MODEL)
            .ok_or("missing Fafnir animation")?;
        let mut fafnir = Fafnir::new(mesh, animation, 0x6661_666e)?;
        let orientation = self
            .locators
            .get(&area)
            .copied()
            .ok_or("fafnir: missing spawn area")?;
        let segment = magicka_sim::spray_pulse::Segment {
            origin: [orientation[12], orientation[13], orientation[14]],
            delta: [0.0, -10.0, 0.0],
        };
        let ground = self
            .collision
            .collision()
            .segment_intersect(segment, false)
            .map(|h| h.hit.position);
        let commands = fafnir.initialize(orientation, ground);
        let mut zones = [0u16; 4];
        for (index, ((half, part), radius)) in ZONE_BOXES.iter().zip(ZONE_RADII).enumerate() {
            let handle = self.allocate_physics(true);
            let collision = magicka_content::physics::CollisionBox {
                name: format!("fafnir_zone{index}"),
                position: half.map(|v| -v),
                sides: half.map(|v| v * 2.0),
                orientation: [0.0, 0.0, 0.0, 1.0],
            };
            self.physics_damage.insert(
                handle,
                crate::physics_damage::PhysicsDamage::boss_zone(*part, collision.clone()),
            );
            self.static_prop_boxes.insert(handle, collision);
            let e = self.entity_mut(handle)?;
            e.radius = radius;
            e.hit_points = 1.0;
            e.max_hit_points = 1.0;
            e.added = true;
            if index == 0 {
                // mHeadZone.Initialize("#boss_n14").
                e.unique_id = custom_hash("#boss_n14");
            }
            zones[index] = handle;
        }
        let mut host = FafnirHost {
            fafnir: Box::new(fafnir),
            zones,
        };
        self.place_fafnir_zones(&host);
        for c in commands {
            self.fafnir_command(&mut host, c);
        }
        self.log.push(format!("boss fafnir: zones {zones:?}"));
        Ok(host)
    }

    fn place_fafnir_zones(&mut self, host: &FafnirHost) {
        for (i, &h) in host.zones.iter().enumerate() {
            let m = host.fafnir.zones[i];
            if let Ok(e) = self.entity_mut(h) {
                e.orientation = m;
                e.position = [m[12], m[13], m[14]];
            }
        }
    }

    /// The living avatars (Player.Playing with a live Avatar).
    fn fafnir_avatars(&self) -> Vec<Avatar> {
        self.players
            .iter()
            .flatten()
            .filter(|p| p.playing && !p.dead())
            .map(|p| Avatar {
                handle: p.handle,
                position: self.physics.bodies[p.body].position,
            })
            .collect()
    }

    pub(super) fn fafnir_message(&mut self, host: &mut FafnirHost, name: &str) {
        let avatars = self.fafnir_avatars();
        let mut out = Vec::new();
        host.fafnir.message(name, &avatars, &mut out);
        for c in out {
            self.fafnir_command(host, c);
        }
    }

    /// Fafnir.UpdateBoss with the zone deliveries of the last frame.
    pub(super) fn update_fafnir(&mut self, host: &mut FafnirHost, dt: f32, fight_started: bool) {
        let mut commands = Vec::new();
        for (part, zone) in host.zones.iter().enumerate() {
            let position = host.fafnir.zones[part];
            let inbox = self
                .physics_damage
                .get_mut(zone)
                .map(|z| z.take_boss_inbox(self.boss_damage_divisor))
                .unwrap_or_default();
            let index = ZONE_BOXES[part].1 as usize;
            for (damage, features) in inbox {
                host.fafnir.damage(
                    index,
                    damage,
                    false,
                    [position[12], position[13], position[14]],
                    features,
                    &mut commands,
                );
            }
        }
        let avatars = self.fafnir_avatars();
        commands.extend(
            host.fafnir
                .update(dt, FafnirInputs { fight_started }, &avatars),
        );
        self.place_fafnir_zones(host);
        for c in commands {
            self.fafnir_command(host, c);
        }
        if host.fafnir.draw_lance {
            self.clip_fafnir_lance(host);
        }
        self.sync_fafnir_breath(host);
    }

    /// The effects a Fafnir state owns for as long as it lasts, each with its
    /// native handle semantics (StartEffect once, UpdatePositionDirection or
    /// UpdateOrientation each frame, Stop on exit): FirelanceHigh/Ceiling
    /// keep fafnir_fire_spray on the lance segment, FirelanceHigh keeps the
    /// looping fafnir_lance_weld at the clipped lance end (direction Right),
    /// FirelanceLow keeps fafnir_fire_breath at the mouth, Confuse keeps an
    /// infinite fafnir_charm on each eye and EarthQuake keeps the looping
    /// fafnir_earthquake on the head zone once its floor set has started.
    /// Without them both fire attacks drew no flame; starting these through
    /// the one-shot path left glowing emitters along the lance's path and
    /// charm particles at the eyes for the rest of the fight.
    fn sync_fafnir_breath(&mut self, host: &FafnirHost) {
        use crate::boss_fafnir::{forward, mat_translation, State};
        use crate::effect_action::EffectActionHost;
        let f = &host.fafnir;
        let spray = (f.draw_lance && matches!(f.state, State::LanceHigh | State::Ceiling))
            .then_some((f.lance.0, f.lance.1));
        let weld = (f.draw_lance && f.state == State::LanceHigh).then(|| {
            let end = [0, 1, 2].map(|i| f.lance.0[i] + f.lance.1[i]);
            (end, [1.0, 0.0, 0.0])
        });
        let eye = |i: usize| {
            (f.state == State::Confuse).then(|| {
                (
                    mat_translation(&f.eye_transforms[i]),
                    forward(&f.eye_transforms[i]),
                )
            })
        };
        let quake = (f.state == State::EarthQuake && f.quake_started)
            .then(|| (mat_translation(&f.zones[0]), forward(&f.zones[0])));
        for (name, key, at) in [
            ("fafnir_fire_spray", "fafnir_live_fire_spray", spray),
            ("fafnir_lance_weld", "fafnir_live_lance_weld", weld),
            ("fafnir_fire_breath", "fafnir_live_fire_breath", f.breath),
            ("fafnir_charm", "fafnir_live_charm_right", eye(0)),
            ("fafnir_charm", "fafnir_live_charm_left", eye(1)),
            ("fafnir_earthquake", "fafnir_live_earthquake", quake),
        ] {
            let id = custom_hash(key);
            let Some((position, direction)) = at else {
                if let Err(e) = self.stop_effect(id) {
                    self.log.push(format!("fafnir {name}: {e}"));
                }
                continue;
            };
            let l = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
            if l < 1e-6 {
                continue;
            }
            let d = direction.map(|v| v / l);
            let up = if d[0].abs() < f32::from_bits(1) && d[2].abs() < f32::from_bits(1) {
                [0.0, 0.0, -1.0]
            } else {
                [0.0, 1.0, 0.0]
            };
            let t = crate::particles::create_world(position, d, up);
            if let Err(e) = self.add_effect(id, custom_hash(name), t, 0.0) {
                self.log.push(format!("fafnir {name}: {e}"));
                continue;
            }
            if let Some(stored) = self.effects.effects.get_mut(&id) {
                stored.transform = t;
            }
        }
    }

    /// The lance segment stops at the first shield, else the scene.
    fn clip_fafnir_lance(&mut self, host: &mut FafnirHost) {
        let (origin, delta) = host.fafnir.lance;
        let segment = magicka_sim::spray_pulse::Segment { origin, delta };
        let shield = self
            .shields
            .iter()
            .filter(|s| !s.shield.dead)
            .filter_map(|s| s.shield.geometry().segment_fraction(segment))
            .fold(None, |a: Option<f32>, f| Some(a.map_or(f, |a| a.min(f))));
        let fraction = shield.or_else(|| {
            self.collision
                .collision()
                .segment_intersect(segment, false)
                .map(|h| h.hit.fraction)
        });
        if let Some(f) = fraction {
            host.fafnir.lance.1 = delta.map(|v| v * f);
        }
    }

    fn fafnir_spawn_missile(
        &mut self,
        owner: u16,
        position: [f32; 3],
        velocity: [f32; 3],
        radius: f32,
        conditions: Vec<EventCollection>,
        model: Option<&str>,
        gravity: bool,
    ) {
        let (conditions, skipped) = crate::missiles::item_conditions(&conditions);
        for event in skipped {
            self.log
                .push(format!("fafnir missile event lacks host: {event}"));
        }
        let id = self.next_missile;
        self.next_missile = self.next_missile.wrapping_add(1);
        let mut missile = crate::missiles::LiveMissile::spawn_custom(
            id, owner, position, velocity, radius, false, conditions,
        );
        missile.body.item_model = model.map(str::to_string);
        missile.body.no_gravity = !gravity;
        let mut visuals = Vec::new();
        self.with_rail_world(|world| {
            crate::missiles::initialize(&mut missile, &mut world.hit, &mut visuals)
        });
        self.missile_visuals.extend(visuals);
        self.register_missile(missile);
    }

    /// AnimatedLevelPart.Play(all, -1, -1, 1, no loop), then Stop when
    /// resetting (Fafnir.Initialize).
    fn fafnir_level_part(&mut self, name: &str, reset: bool) {
        let id = custom_hash(name);
        let path = self.collision.part_path(id).unwrap_or_else(|| vec![id]);
        let mut operations = vec![crate::animation_action::AnimationOperation::Play(
            crate::animated_hierarchy::PlayCommand {
                start: -1.0,
                end: -1.0,
                speed: 1.0,
                looping: false,
                resume: false,
            },
        )];
        if reset {
            operations.push(crate::animation_action::AnimationOperation::Stop);
        }
        for operation in operations {
            let action = crate::animation_action::AnimationAction {
                delay: 0.0,
                path: path.clone(),
                children: true,
                operation,
            };
            if let Err(e) = self.collision.execute_animation_action(&action) {
                self.log.push(format!("fafnir {name}: {e:?}"));
                break;
            }
        }
    }

    fn fafnir_command(&mut self, host: &mut FafnirHost, command: FafnirCommand) {
        use magicka_sim::damage::{attack, Damage};
        use magicka_sim::spells::Elements as E;
        let head = host.zones[0];
        match command {
            FafnirCommand::Trigger(name) => self.pending_triggers.push((custom_hash(name), None)),
            FafnirCommand::Cue { name, position } => {
                self.play_cue(crate::audio::banks::CHARACTERS, name, position);
            }
            FafnirCommand::SpellCue { name } => {
                self.play_cue(crate::audio::banks::SPELLS, name, None);
            }
            FafnirCommand::Effect {
                name,
                position,
                direction,
            } => self
                .status_contact_effects
                .push((name, position, direction)),
            FafnirCommand::WingPush { spine_base } => {
                let push = Damage {
                    attack: attack::PUSHED,
                    element: E::EARTH.0,
                    amount: 150.0,
                    magnitude: 1.0,
                };
                for c in self.combatants() {
                    if c.dead || host.zones.contains(&c.handle) {
                        continue;
                    }
                    let d = [
                        c.position[0] - spine_base[0],
                        0.0,
                        c.position[2] - spine_base[2],
                    ];
                    let l = (d[0] * d[0] + d[2] * d[2]).sqrt();
                    let full = [
                        c.position[0] - spine_base[0],
                        c.position[1] - spine_base[1],
                        c.position[2] - spine_base[2],
                    ];
                    let dist = (full[0] * full[0] + full[1] * full[1] + full[2] * full[2]).sqrt();
                    if dist > 30.0 || l <= 0.0 || d[2] / l <= 0.5 {
                        continue;
                    }
                    if host.fafnir.hit(c.handle, 0.06125) {
                        let mut p = c.position;
                        p[2] -= 2.0;
                        self.grip_damage(c.handle, head, &[push], p);
                    }
                }
            }
            FafnirCommand::TailSweep { segments } => {
                let knock = Damage {
                    attack: attack::KNOCKBACK,
                    element: E::EARTH.0,
                    amount: 200.0,
                    magnitude: 5.0,
                };
                for c in self.combatants() {
                    if c.dead {
                        continue;
                    }
                    for (a, b, r) in &segments {
                        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                        let ap = [
                            c.position[0] - a[0],
                            c.position[1] - a[1],
                            c.position[2] - a[2],
                        ];
                        let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
                        let t = if len2 > 0.0 {
                            ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / len2).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        let q = [0, 1, 2].map(|i| a[i] + ab[i] * t - c.position[i]);
                        let reach = r + c.radius + c.length * 0.5;
                        if q[0] * q[0] + q[1] * q[1] + q[2] * q[2] <= reach * reach {
                            if host.fafnir.hit(c.handle, 1.0) {
                                self.grip_damage(c.handle, head, &[knock], *a);
                            }
                            break;
                        }
                    }
                }
            }
            FafnirCommand::CircleDamage {
                position,
                radius,
                damages,
            } => {
                let t = self.play_time;
                self.arc_damage(
                    None,
                    None,
                    position,
                    [0.0, 0.0, -1.0],
                    radius,
                    std::f32::consts::PI,
                    &damages,
                    t,
                );
            }
            FafnirCommand::ArcDamage {
                position,
                direction,
                range,
                angle,
                damages,
            } => {
                let t = self.play_time;
                self.arc_damage(None, None, position, direction, range, angle, &damages, t);
            }
            FafnirCommand::Fireball { position, velocity } => {
                let hit = |kind| {
                    condition(
                        kind,
                        0.0,
                        false,
                        vec![
                            sound("magick_meteor_blast", false),
                            effect("magick_meteor_explosion", false),
                            effect("fafnir_fireball_splash", false),
                            Event::Remove { bounce: 0 },
                            splash(attack::DAMAGE, E::FIRE.0, 300.0, 1.5, 6.0),
                            splash(attack::KNOCKBACK, E::FIRE.0, 300.0, 5.0, 6.0),
                            splash(attack::STATUS, E::FIRE.0, 200.0, 3.0, 6.0),
                        ],
                    )
                };
                let conditions = vec![
                    condition(
                        1,
                        0.0,
                        true,
                        vec![
                            effect("fafnir_fireball_trail", true),
                            sound("spell_fire_projectile", true),
                        ],
                    ),
                    hit(2),
                    hit(4),
                    condition(0x10, 8.0, false, vec![Event::Remove { bounce: 0 }]),
                ];
                self.fafnir_spawn_missile(head, position, velocity, 0.25, conditions, None, false);
            }
            FafnirCommand::Debris { position } => {
                let hit = |kind| {
                    condition(
                        kind,
                        0.0,
                        false,
                        vec![
                            sound("spell_earth_hit", false),
                            effect("fafnir_debri_hit", false),
                            Event::Remove { bounce: 0 },
                            splash(attack::PUSHED, E::EARTH.0, 250.0, 5.0, 3.0),
                            splash(attack::DAMAGE, E::EARTH.0, 300.0, 1.0, 3.0),
                            Event::CameraShake {
                                ttl: 1.0,
                                magnitude: 0.5,
                                from_position: true,
                            },
                        ],
                    )
                };
                let conditions = vec![
                    condition(1, 0.0, true, vec![effect("fafnir_debri_trail", true)]),
                    hit(2),
                    hit(4),
                    condition(0x10, 8.0, false, vec![Event::Remove { bounce: 0 }]),
                ];
                self.fafnir_spawn_missile(
                    head,
                    position,
                    [0.1, -22.0, 0.1],
                    1.0,
                    conditions,
                    Some(crate::boss_fafnir::DEBRIS_MODEL),
                    true,
                );
            }
            FafnirCommand::InvertControls(on) => {
                for p in self.players.iter_mut().flatten().filter(|p| p.playing) {
                    p.controls_inverted = on;
                }
            }
            FafnirCommand::CameraShake { magnitude, time } => self.camera.shake(magnitude, time),
            FafnirCommand::PlayLevelPart(name) => self.fafnir_level_part(&name, false),
            FafnirCommand::ResetLevelPart(name) => self.fafnir_level_part(&name, true),
            FafnirCommand::StartLevelEffect(name) => {
                let _ = EffectActionHost::start_effect(self, custom_hash(&name));
            }
            FafnirCommand::Boss(event) => {
                let mut notifier = [
                    host.fafnir.zones[0][12],
                    host.fafnir.zones[0][13],
                    host.fafnir.zones[0][14],
                ];
                notifier[1] += 3.0;
                self.present_boss_event(event, notifier, "gore_splash_black");
            }
        }
    }
}
