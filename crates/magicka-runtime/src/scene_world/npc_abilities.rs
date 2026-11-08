//! NPC special abilities of the DLC characters (research/npc-abilities.md):
//! Potion, PotionLeft/Right, BreakBarriers, AlucartDeathExplosion,
//! HomingCharge, StopCharge, FloorStomp, GreaseLump, ArrowRain, Starfall,
//! StarGaze, MutateBeastman and HealingRain. Ongoing ones run as
//! [`StaffEffect`]s in the SpellManager update.
use super::staff_effects::{StaffEffect, StaffEffectKind};
use super::staff_specials::spell_of;
use super::*;
use crate::xna_math::{self as m, V3};
use magicka_sim::damage::{attack, Damage};
use magicka_sim::spells::{Elements as E, Spell};

const CHARGE_TIME: f32 = 5.0;
const START_CHARGE: f32 = 0.8;
const STOP_CHARGE: f32 = 2.5;
const CHARGE_THRESHOLD: f32 = 0.015;
const ARROW_LIFE: f32 = 4.5;
const ARROW_START: f32 = 1.9;
const ARROW_RADIUS: f32 = 3.5;
const ARROW_MODEL: &str = "Models/Missiles/goblin_arrow_0";
const FLASK_MODEL: &str = "Models/Missiles/flask_healing_potion";
const LUMP_MODEL: &str = "Models/Missiles/JudgementSprayMissile";

/// HomingCharge/StopCharge's running averages.
#[derive(Clone, Debug, PartialEq)]
pub struct ChargeTrack {
    pub homing: bool,
    pub charging: bool,
    pub thresholds: [f32; 5],
    pub deltas: [f32; 3],
    pub last: V3,
}

impl ChargeTrack {
    fn new(homing: bool) -> Self {
        Self {
            homing,
            charging: false,
            thresholds: [CHARGE_THRESHOLD; 5],
            deltas: [0.0; 3],
            last: [0.0; 3],
        }
    }
    fn threshold(&self) -> f32 {
        self.thresholds[..3].iter().sum::<f32>() / 3.0
    }
    fn delta(&self) -> f32 {
        self.deltas.iter().sum::<f32>() / 3.0
    }
}

/// ArrowRain's singleton state.
#[derive(Clone, Debug, PartialEq)]
pub struct ArrowRainState {
    pub launched: bool,
    pub spawn_timer: f32,
    pub last_arrow: i32,
    pub position: V3,
    pub spell: Spell,
    pub lightning: bool,
    pub intensity: f64,
}

impl<C: SceneContent> SceneWorld<C> {
    /// Returns false when `name` is not one of these abilities.
    pub(super) fn execute_npc_ability(&mut self, owner: u16, name: &str, elements: &[i32]) -> bool {
        let Some(body) = self.character_body(owner) else {
            return matches!(
                name,
                "potion"
                    | "potionleft"
                    | "potionright"
                    | "breakbarriers"
                    | "alucartdeathexplosion"
                    | "homingcharge"
                    | "stopcharge"
                    | "floorstomp"
                    | "greaselump"
                    | "arrowrain"
                    | "starfall"
                    | "stargaze"
                    | "mutatebeastman"
                    | "healingrain"
            );
        };
        let position = self.physics.bodies[body].position;
        let direction = self.physics.bodies[body].direction();
        let source = self.cast_source_of(owner).unwrap_or(position);
        let timestamp = self.play_time;
        let push = |w: &mut Self, ttl: f32, kind: StaffEffectKind| {
            w.staff_effects.push(StaffEffect {
                owner,
                ttl,
                timestamp,
                kind,
            })
        };
        match name {
            "potion" | "potionleft" | "potionright" => {
                // Potion.Execute: 501 Life damage to itself from the cast
                // source; Left/Right also toss the empty flask sideways.
                let heal = Damage {
                    attack: attack::DAMAGE,
                    element: E::LIFE.0,
                    amount: -501.0,
                    magnitude: 1.0,
                };
                self.hit_character(owner, heal, Some(owner), source);
                if name != "potion" {
                    let hand = if name == "potionleft" {
                        source
                    } else {
                        self.npcs
                            .get(&owner)
                            .map_or(source, |n| m::translation(&n.weapon_transform))
                    };
                    let up = m::normalize([direction[0], 1.0, direction[2]]);
                    let side = m::normalize(m::cross(up, [0.0, 1.0, 0.0]));
                    let speed = 5.0 + (0.5 + self.random()) * 10.0 / 2.0;
                    let sign = if name == "potionleft" { -1.0 } else { 1.0 };
                    self.spawn_flask(owner, hand, m::scale(side, sign * speed));
                }
            }
            "breakbarriers" => push(self, 5.0, StaffEffectKind::BreakBarriers),
            "alucartdeathexplosion" => {
                // A stationary Earth 3 / Fire 100 missile, splash 5.
                self.spawn_missiles(vec![(
                    owner,
                    magicka_sim::projectile_launch::ProjectileLaunch {
                        position: source,
                        velocity: [0.0; 3],
                        spell: spell_of(&[(E::EARTH, 3.0), (E::FIRE, 100.0)]),
                        splash: 5.0,
                        total_projectiles: 1,
                    },
                )]);
            }
            "homingcharge" | "stopcharge" => {
                let mut track = ChargeTrack::new(name == "homingcharge");
                track.last = position;
                push(self, CHARGE_TIME, StaffEffectKind::Charge(track));
            }
            "floorstomp" => push(
                self,
                8.0,
                StaffEffectKind::FloorStomp {
                    stomps: [1.15, 2.1, 3.05],
                },
            ),
            "greaselump" => {
                let velocity = m::scale(m::normalize([direction[0], 0.5, direction[2]]), 15.0);
                let missile = self.spawn_grease_lump(owner, source, velocity);
                push(
                    self,
                    1.2,
                    StaffEffectKind::GreaseLump {
                        missile,
                        interval: 0.0,
                        last: Some(source),
                    },
                );
            }
            "arrowrain" => self.arrow_rain(owner, elements),
            "starfall" => {
                let target = self
                    .npcs
                    .get(&owner)
                    .and_then(|n| n.combat.current_target())
                    .and_then(|t| self.character_body(t))
                    .map(|b| self.physics.bodies[b].position)
                    .unwrap_or_else(|| {
                        self.starfall_target(m::add(position, m::scale(direction, 10.0)))
                    });
                push(
                    self,
                    1.0,
                    StaffEffectKind::Starfall {
                        cast_delay: 1.0,
                        damage_delay: 0.5,
                        position: target,
                    },
                );
            }
            "stargaze" => self.confuse_who(owner, position, direction, 20.0),
            "mutatebeastman" => self.mutate_beastman(owner),
            "healingrain" => self.healing_rain(owner),
            _ => return false,
        }
        true
    }

    /// Potion.SpawnFlask: a cosmetic flask removed when damaged or after 15 s.
    fn spawn_flask(&mut self, owner: u16, position: V3, velocity: V3) {
        use magicka_content::common::{Event, EventCollection, EventCondition};
        let condition = |kind: u8, time: f32, hitpoints: i32, events: Vec<Event>| EventCollection {
            condition: EventCondition {
                condition_type: kind,
                hitpoints,
                elements: 0,
                threshold: 0.0,
                time,
            },
            repeat: false,
            events,
        };
        // EventConditionType: Damaged = 8, Timer = 0x10.
        let conditions = vec![
            condition(8, 0.0, 20, vec![Event::Remove { bounce: 0 }]),
            condition(0x10, 15.0, 0, vec![Event::Remove { bounce: 0 }]),
        ];
        self.spawn_custom_model_missile(owner, FLASK_MODEL, position, velocity, conditions);
    }

    /// GreaseLump.SpawnLump: on collision or hit (or 20 fire damage) the
    /// lump splashes 100 earth in 3 m, greases the character it hit and is
    /// removed. Returns the missile id.
    fn spawn_grease_lump(&mut self, owner: u16, position: V3, velocity: V3) -> u32 {
        use magicka_content::common::{Damage as CDamage, Event, EventCollection, EventCondition};
        let hit = || {
            vec![
                Event::Effect {
                    follow: false,
                    world_align: true,
                    effect: "dungeons_grease_ball_impact".into(),
                },
                Event::Sound {
                    bank: crate::audio::banks::ADDITIONAL as i32,
                    sound: "dungeon_slime_cube_death_big".into(),
                    magnitude: 1.0,
                    stop_on_remove: false,
                },
                Event::Splash {
                    damage: CDamage {
                        attack_property: attack::DAMAGE as i32,
                        element: E::EARTH.0 as i32,
                        amount: 100.0,
                        magnitude: 1.0,
                    },
                    radius: 3.0,
                },
                Event::Remove { bounce: 0 },
            ]
        };
        let condition =
            |kind: u8, elements: i32, hitpoints: i32, repeat: bool, events| EventCollection {
                condition: EventCondition {
                    condition_type: kind,
                    hitpoints,
                    elements,
                    threshold: 0.0,
                    time: 0.0,
                },
                repeat,
                events,
            };
        // EventConditionType: Default = 1, Hit = 2, Collision = 4,
        // Damaged = 8.
        let conditions = vec![
            condition(4, 0, 0, false, hit()),
            condition(2, 0, 0, false, hit()),
            condition(8, E::FIRE.0 as i32, 20, false, hit()),
            condition(
                1,
                0,
                0,
                true,
                vec![Event::Effect {
                    follow: true,
                    world_align: false,
                    effect: "dungeons_grease_ball".into(),
                }],
            ),
        ];
        self.spawn_custom_model_missile(owner, LUMP_MODEL, position, velocity, conditions)
    }

    fn spawn_custom_model_missile(
        &mut self,
        owner: u16,
        model: &str,
        position: V3,
        velocity: V3,
        conditions: Vec<magicka_content::common::EventCollection>,
    ) -> u32 {
        let (conditions, skipped) = crate::missiles::item_conditions(&conditions);
        for event in skipped {
            self.log
                .push(format!("{model} missile event lacks host: {event}"));
        }
        let radius = self.content.model_radius(model).unwrap_or(0.25) * 0.75;
        let id = self.next_missile;
        self.next_missile = self.next_missile.wrapping_add(1);
        let mut missile = crate::missiles::LiveMissile::spawn_custom(
            id, owner, position, velocity, radius, false, conditions,
        );
        missile.body.item_model = Some(model.to_owned());
        let mut visuals = Vec::new();
        self.with_rail_world(|world| {
            crate::missiles::initialize(&mut missile, &mut world.hit, &mut visuals)
        });
        self.missile_visuals.extend(visuals);
        self.register_missile(missile);
        id
    }

    /// Starfall.GetTarget: the damageable closest (XZ) to `source` within
    /// 8 m, else the point itself.
    fn starfall_target(&self, source: V3) -> V3 {
        let (all, _) = self.spray_entities();
        all.iter()
            .filter(|e| e.damageable && m::length(m::sub(e.position, source)) - e.radius <= 8.0)
            .min_by(|a, b| {
                let d = |p: V3| (p[0] - source[0]).powi(2) + (p[2] - source[2]).powi(2);
                d(a.position).total_cmp(&d(b.position))
            })
            .map_or(source, |e| e.position)
    }

    /// ArrowRain.Execute (NPC owner): over the AI's last target; a running
    /// rain restarts its timing instead.
    fn arrow_rain(&mut self, owner: u16, elements: &[i32]) {
        let list: Vec<E> = elements.iter().map(|&e| E(e as u32)).collect();
        let (count, lightning) = list
            .iter()
            .find_map(|e| {
                if e.has(E::LIGHTNING) {
                    Some((7u32, true))
                } else if e.has(E::POISON) {
                    Some((15, false))
                } else {
                    None
                }
            })
            .unwrap_or((10, false));
        if let Some(e) = self
            .staff_effects
            .iter_mut()
            .find(|e| matches!(e.kind, StaffEffectKind::ArrowRain(_)))
        {
            if let StaffEffectKind::ArrowRain(rain) = &mut e.kind {
                rain.launched = false;
            }
            e.ttl = ARROW_LIFE + ARROW_START;
            return;
        }
        let Some(target) = self
            .npcs
            .get(&owner)
            .and_then(|n| n.combat.current_target())
            .and_then(|t| self.character_body(t))
            .map(|b| self.physics.bodies[b].position)
        else {
            return;
        };
        let spell = if list.is_empty() {
            Spell::default()
        } else {
            Spell::default_for_list(&list)
        };
        self.staff_effects.push(StaffEffect {
            owner,
            ttl: ARROW_LIFE + ARROW_START,
            timestamp: self.play_time,
            kind: StaffEffectKind::ArrowRain(ArrowRainState {
                launched: false,
                spawn_timer: 0.0,
                last_arrow: 0,
                position: target,
                spell,
                lightning,
                intensity: f64::from(count + 1).powf(1.0 / f64::from(ARROW_LIFE)),
            }),
        });
    }

    /// MutateBeastman: OtherworldlyDischarge's target rules, raising a
    /// Skeleton_darksoul_elite_noscore.
    fn mutate_beastman(&mut self, owner: u16) {
        self.discharge_template = Some("skeleton_darksoul_elite_noscore");
        self.otherworldly_discharge_from(owner);
        self.discharge_template = None;
    }

    /// HealingRain.Execute: refused indoors; rain effect and ambience for
    /// 8 s, light dimmed to a third.
    fn healing_rain(&mut self, owner: u16) {
        if self.scene_indoors {
            return;
        }
        if self
            .staff_effects
            .iter()
            .any(|e| matches!(e.kind, StaffEffectKind::HealingRain { .. }))
        {
            if let Some(e) = self
                .staff_effects
                .iter_mut()
                .find(|e| matches!(e.kind, StaffEffectKind::HealingRain { .. }))
            {
                e.ttl = 8.0;
            }
            return;
        }
        let cue = self.play_cue(16, "magick_rain", None);
        let at = self.camera_ground();
        let effect = self.start_named_effect("magick_healingrain", at, [0.0, 0.0, -1.0]);
        self.light_target_intensity = 0.333;
        self.staff_effects.push(StaffEffect {
            owner,
            ttl: 8.0,
            timestamp: self.play_time,
            kind: StaffEffectKind::HealingRain {
                wet_timer: 0.0,
                effect,
                cue,
            },
        });
    }

    /// The camera's ground point (Camera.Position − CAMERAOFFSET).
    fn camera_ground(&self) -> V3 {
        self.camera.ground_position()
    }

    /// The SpellManager update of these abilities; true when handled.
    pub(super) fn update_npc_ability(&mut self, e: &mut StaffEffect, dt: f32) -> bool {
        let owner = e.owner;
        match &mut e.kind {
            StaffEffectKind::BreakBarriers => {
                e.ttl -= dt;
                if let Some((_, at, _)) = self.body_of(owner) {
                    let radius = 3.5 * self.combatant_radius(owner);
                    self.kill_blockers_within(at, radius);
                }
            }
            StaffEffectKind::Charge(track) => {
                let Some((body, at, _)) = self.body_of(owner) else {
                    e.ttl = 0.0;
                    return true;
                };
                let speed = self.physics.bodies[body].speed_multiplier();
                e.ttl -= dt * speed;
                track.thresholds.rotate_left(1);
                track.thresholds[4] = CHARGE_THRESHOLD * speed;
                track.deltas.rotate_left(1);
                track.deltas[2] = m::length(m::sub(at, track.last));
                if e.ttl < CHARGE_TIME - START_CHARGE && !track.charging && track.delta() > 0.0 {
                    track.charging = true;
                }
                if track.charging && track.delta() < track.threshold() {
                    track.charging = false;
                    if track.homing {
                        self.go_to_animation_of(owner, "special2", 0.01);
                    } else {
                        self.grease_splash(owner);
                        if e.ttl > CHARGE_TIME - STOP_CHARGE {
                            self.go_to_animation_of(owner, "special2", 0.1);
                        }
                    }
                    e.ttl = 0.0;
                    return true;
                }
                if track.homing {
                    // The last avatar within 5 m decides (native loop).
                    let mut distance = 0.0;
                    for p in self.players.iter().flatten() {
                        let q = self.physics.bodies[p.body].position;
                        let d = m::length(m::sub(q, at));
                        if d - self.physics.bodies[p.body].radius <= 5.0 {
                            distance = d;
                        }
                    }
                    if distance > 0.0 && distance < 2.0 {
                        self.go_to_animation_of(owner, "special2", 0.01);
                        e.ttl = 0.0;
                    }
                }
                track.last = at;
            }
            StaffEffectKind::FloorStomp { stomps } => {
                let mut dt = dt;
                if let Some(c) = self.character_ref(owner) {
                    if let Some(cold) = c
                        .combat
                        .statuses
                        .effect(magicka_sim::status::Status::Cold)
                        .filter(|s| !s.dead())
                    {
                        dt *= cold.slowdown();
                    }
                    if c.combat.zap_timer > 0.0 && !c.definition.zap_modifier.is_nan() {
                        dt *= c.definition.zap_modifier;
                    }
                }
                e.ttl -= dt;
                let magnitudes = [3.0, 5.0, 8.0];
                for (stomp, earth) in stomps.iter_mut().zip(magnitudes) {
                    *stomp -= dt;
                    if *stomp < 0.0 {
                        *stomp = 1_000_000.0;
                        let spell = spell_of(&[(E::EARTH, earth), (E::FIRE, 2.0)]);
                        self.npc_cast_area(owner, spell);
                        if let Some((_, at, _)) = self.body_of(owner) {
                            self.kill_blockers_within(at, spell.blast_size() * 10.0);
                        }
                    }
                }
            }
            StaffEffectKind::GreaseLump {
                missile,
                interval,
                last,
            } => {
                e.ttl -= dt;
                let id = *missile;
                let flying = self
                    .missiles
                    .iter()
                    .find(|mi| mi.body.id == id && !mi.body.dead)
                    .map(|mi| mi.body.position);
                match (flying, *last) {
                    (Some(p), _) => *last = Some(p),
                    (None, Some(p)) => {
                        // The CallbackEvent: the character it struck is
                        // greased (INFERRED: the nearest within 1.5 m).
                        *last = None;
                        self.grease_struck(p, owner);
                    }
                    (None, None) => {}
                }
                *interval -= dt;
                if *interval < 0.0 {
                    *interval = 0.15;
                    // mMissile.Position: the last known position once gone.
                    if let Some(p) = flying.or(*last) {
                        self.spawn_grease_field(
                            Some(owner),
                            [p[0], 0.0, p[2]],
                            [0.0, 1.0, 0.0],
                            None,
                        );
                    }
                }
            }
            StaffEffectKind::ArrowRain(rain) => {
                e.ttl -= dt;
                if e.ttl < ARROW_LIFE && !rain.launched {
                    rain.launched = true;
                    // Launch: one arrow straight up from the left hand.
                    if let Some(hand) = self.cast_source_of(owner) {
                        self.spawn_arrow(owner, hand, [0.01, 50.0, -0.01], rain.spell);
                    }
                }
                if !rain.launched {
                    return true;
                }
                rain.spawn_timer += dt;
                let n = (-1.0 + rain.intensity.powf(f64::from(rain.spawn_timer))) as i32;
                if n == 0 || n == rain.last_arrow {
                    return true;
                }
                rain.last_arrow = n;
                let r = self.random().sqrt();
                let angle = self.random() * std::f32::consts::TAU;
                let mut top = rain.position;
                top[0] += ARROW_RADIUS * r * angle.cos();
                top[2] += ARROW_RADIUS * r * angle.sin();
                top[1] += 146.0;
                let ground = [top[0], 0.0, top[2]];
                let (spell, lightning) = (rain.spell, rain.lightning);
                if lightning {
                    self.flash(0.075);
                    self.camera.shake(0.5, 0.075);
                    self.sky_bolt(owner, top, ground, spell);
                } else {
                    self.spawn_arrow(owner, top, [0.01, -20.0, -0.01], spell);
                }
            }
            StaffEffectKind::Starfall {
                cast_delay,
                damage_delay,
                position,
            } => {
                if *cast_delay > 0.0 {
                    *cast_delay -= dt;
                    e.ttl = 1.0;
                    if *cast_delay <= 0.0 {
                        // A 20/20 diagonal ray onto the scene, clipped by
                        // shields.
                        let delta = [-20.0, -20.0, 0.0];
                        let origin = m::sub(*position, delta);
                        let segment = Segment { origin, delta };
                        if let Some(h) = self.collision.collision().segment_intersect(segment, true)
                        {
                            *position = h.hit.position;
                        }
                        for s in &self.shields {
                            if let Some(f) = s.shield.geometry().segment_fraction(segment) {
                                *position = m::add(origin, m::scale(delta, f));
                            }
                        }
                        let at = *position;
                        self.start_named_effect("starspawn_starfall", at, [-1.0, 0.0, 0.0]);
                        self.play_cue(16, "magick_meteor_preblast", Some(at));
                    }
                } else {
                    *damage_delay -= dt;
                    if *damage_delay <= 0.0 {
                        let at = *position;
                        let damages = [
                            Damage {
                                attack: attack::DAMAGE,
                                element: E::ARCANE.0,
                                amount: 12000.0,
                                magnitude: 1.0,
                            },
                            Damage {
                                attack: attack::DAMAGE | attack::ARMOUR_PIERCING,
                                element: E::EARTH.0,
                                amount: 6000.0,
                                magnitude: 1.0,
                            },
                        ];
                        let t = self.play_time;
                        self.arc_damage(
                            Some(owner),
                            None,
                            at,
                            [0.0, 0.0, -1.0],
                            3.0,
                            std::f32::consts::PI,
                            &damages,
                            t,
                        );
                        self.play_cue(16, "magick_meteor_blast", None);
                        e.ttl = 0.0;
                    }
                }
            }
            StaffEffectKind::HealingRain {
                wet_timer, effect, ..
            } => {
                e.ttl -= dt;
                *wet_timer -= dt;
                let at = self.camera_ground();
                self.effect_commands.push(SceneEffectCommand::Move {
                    instance: *effect,
                    position: at,
                    direction: [0.0, 0.0, -1.0],
                });
                if *wet_timer <= 0.0 {
                    *wet_timer = 0.25;
                    let heal = Damage {
                        attack: attack::DAMAGE,
                        element: E::LIFE.0,
                        amount: -20.0,
                        magnitude: 0.25,
                    };
                    let soak = Damage {
                        attack: attack::STATUS,
                        element: E::WATER.0,
                        amount: 0.0,
                        magnitude: 0.25,
                    };
                    let timestamp = e.timestamp;
                    for c in self.combatants() {
                        if c.dead
                            || c.resistance_against(E::LIFE.0) == 1.0
                            || c.resistance_against(E::WATER.0) == 1.0
                            || self.protected_by_shield(c.position)
                        {
                            continue;
                        }
                        for d in [heal, soak] {
                            self.damage_any_at(c.handle, d, Some(owner), [0.0; 3], timestamp);
                        }
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// OnRemove of these abilities.
    pub(super) fn remove_npc_ability(&mut self, kind: &StaffEffectKind) {
        if let StaffEffectKind::HealingRain { effect, cue, .. } = kind {
            self.stop_cue(*cue);
            self.effect_commands
                .push(SceneEffectCommand::Stop { instance: *effect });
            self.light_target_intensity = 1.0;
        }
    }

    /// GreaseLump.OnCollision on the character nearest the impact.
    fn grease_struck(&mut self, at: V3, owner: u16) {
        let Some(target) = self
            .combatants()
            .into_iter()
            .filter(|c| c.handle != owner && !c.dead)
            .map(|c| (m::length(m::sub(c.position, at)) - c.radius, c.handle))
            .filter(|(d, _)| *d <= 1.5)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, h)| h)
        else {
            return;
        };
        let effect = magicka_sim::status::StatusEffect::new(
            magicka_sim::status::Status::Greased,
            10.0,
            1.0,
            false,
        );
        let mut events = Vec::new();
        if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == target)
        {
            let _ = p.add_status_effect(effect, &mut events);
        } else if let Some(n) = self.npcs.get_mut(&target) {
            let _ = n.character.add_status_effect(effect, None, &mut events);
        }
    }

    fn combatant_radius(&self, handle: u16) -> f32 {
        self.character_body(handle)
            .map_or(0.5, |b| self.physics.bodies[b].radius)
    }

    /// Every barrier and shield within `radius` of `at` is killed.
    fn kill_blockers_within(&mut self, at: V3, radius: f32) {
        let shields: Vec<u16> = self
            .shields
            .iter()
            .filter(|s| {
                !s.shield.dead
                    && m::length(m::sub(s.shield.position, at)) - s.shield.radius <= radius
            })
            .map(|s| s.handle)
            .collect();
        for h in shields {
            self.kill_damageable(h);
        }
        let barriers: Vec<usize> = self
            .barriers
            .iter()
            .filter(|(_, b)| m::length(m::sub(b.position, at)) - b.radius <= radius)
            .map(|(slot, _)| slot)
            .collect();
        for slot in barriers {
            self.kill_barrier(slot);
        }
    }

    /// EntityManager.IsProtectedByShield (a live shield sphere around it).
    fn protected_by_shield(&self, at: V3) -> bool {
        self.shields
            .iter()
            .any(|s| !s.shield.dead && m::length(m::sub(s.shield.position, at)) <= s.shield.radius)
    }

    /// CurrentSpell.CastArea(spell, owner) from a fresh ProjectileSpell.
    fn npc_cast_area(&mut self, owner: u16, spell: Spell) {
        let Some(n) = self.npcs.get_mut(&owner) else {
            return;
        };
        let body = &self.physics.bodies[n.body];
        n.cast_area(body, spell);
    }

    /// ProjectileSpell.SpawnMissile with the goblin arrow model, splash 4.
    fn spawn_arrow(&mut self, owner: u16, position: V3, velocity: V3, spell: Spell) {
        let before = self.next_missile;
        self.spawn_missiles(vec![(
            owner,
            magicka_sim::projectile_launch::ProjectileLaunch {
                position,
                velocity,
                spell,
                splash: 4.0,
                total_projectiles: 1,
            },
        )]);
        if let Some(mi) = self.missiles.iter_mut().find(|mi| mi.body.id == before) {
            mi.body.item_model = Some(ARROW_MODEL.into());
        }
    }

    /// ArrowRain's lightning arrow: an air-to-surface bolt.
    fn sky_bolt(&mut self, owner: u16, top: V3, ground: V3, spell: Spell) {
        let Ok(damages) = magicka_sim::damage::calculate_damage(
            &spell,
            magicka_sim::spells::SpellType::Lightning,
            magicka_sim::damage::CastType::Force,
        ) else {
            return;
        };
        let mut slots = [Damage::default(); 5];
        for (slot, d) in slots.iter_mut().zip(damages) {
            *slot = d;
        }
        let delta = m::sub(ground, top);
        let range = m::length(delta);
        let direction = m::normalize(delta);
        let play_time = self.play_time;
        let mut bolts = Vec::new();
        let mut rng = self.rng;
        self.with_rail_world(|world| {
            let mut host = BarrierLightning {
                world,
                bolts: &mut bolts,
                rng: &mut rng,
            };
            let mut hits = magicka_sim::spray_pulse::HitList::default();
            magicka_sim::lightning::cast_bolt(
                &mut host,
                owner,
                play_time,
                top,
                direction,
                &mut hits,
                LIGHTNING_COLOR,
                1.0,
                range,
                slots,
            );
        });
        self.rng = rng;
        self.barrier_visuals.extend(bolts);
    }
}
