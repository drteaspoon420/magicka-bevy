//! Scene host for Assatur (research/boss-assatur.md): the BossSpellCasterZone
//! capsule, its spell ownership and the class's world operations.
use super::*;
use crate::boss_assatur::{self as policy, Assatur, Command, Inputs};
use magicka_content::common::Event;
use magicka_sim::{spray_pulse::SpraySource, status::Status};

#[derive(Clone, Debug)]
pub struct AssaturHost {
    pub actor: Box<Assatur>,
    /// The body zone (BossSpellCasterZone, "#boss_n15").
    pub zone: u16,
    /// mProtectiveShield.
    shield: Option<u16>,
    cues: Vec<(&'static str, u32)>,
    beam: bool,
}
impl PartialEq for AssaturHost {
    fn eq(&self, other: &Self) -> bool {
        self.zone == other.zone
    }
}

/// The capsule below the body origin: Capsule((0, −0.75, 0), RotationX(−π/2),
/// radius 1.5, length 5), i.e. vertical, 5 m long.
const CAPSULE_OFFSET: f32 = -0.75;

impl<C: SceneContent> SceneWorld<C> {
    pub(super) fn spawn_assatur(&mut self, area: i32) -> Result<AssaturHost, WorldError> {
        let orientation = self.scene_locator(area)?;
        let model = self
            .content
            .skinned_model(policy::MODEL)
            .ok_or("missing Assatur model")?;
        let actor = Box::new(Assatur::new(model, orientation)?);
        let zone = self.allocate_physics(true);
        let dummy = magicka_content::physics::CollisionBox {
            name: "boss_capsule".into(),
            position: [0.; 3],
            sides: [0.; 3],
            orientation: [0., 0., 0., 1.],
        };
        let mut damage = crate::physics_damage::PhysicsDamage::boss_zone(0, dummy);
        damage.collision_box = None;
        damage.solid = false;
        let mut origin = actor.position();
        origin[1] += CAPSULE_OFFSET;
        damage.boss_capsule = Some(crate::physics_damage::BossCapsule {
            origin,
            previous: origin,
            length: 5.,
            radius: 1.5,
            ethereal: false,
        });
        self.physics_damage.insert(zone, damage);
        let e = self.entity_mut(zone)?;
        e.added = true;
        e.orientation = actor.orientation;
        e.position = actor.position();
        e.hit_points = policy::MAX_HP;
        e.max_hit_points = policy::MAX_HP;
        e.radius = 1.5;
        e.length = 2.25;
        e.unique_id = custom_hash("#boss_n15");
        self.add_boss_caster(zone, 1.5);
        let mut host = AssaturHost {
            actor,
            zone,
            shield: None,
            cues: Vec::new(),
            beam: false,
        };
        // Initialize: the first player's avatar is the initial target.
        host.actor.target = self
            .players
            .iter()
            .flatten()
            .find(|p| p.playing)
            .map(|p| p.handle);
        self.sync_assatur(&mut host);
        self.log
            .push(format!("boss assatur: live caster zone {zone}"));
        Ok(host)
    }

    fn assatur_inputs(&self, host: &AssaturHost, running: bool) -> Inputs {
        let mut players = vec![None; self.players.len().max(4)];
        for (slot, p) in self.players.iter().enumerate() {
            if let Some(p) = p {
                if p.playing && !p.dead() {
                    players[slot] = Some(p.handle);
                }
            }
        }
        let combatants = self.combatants();
        let target = host
            .actor
            .target
            .and_then(|t| combatants.iter().find(|c| c.handle == t));
        let at = host.actor.position();
        let within = |c: &crate::npc_combat::Combatant, r: f32| {
            (0..3).map(|i| (c.position[i] - at[i]).powi(2)).sum::<f32>() <= r * r
        };
        let shield_alive = host
            .shield
            .and_then(|h| self.shields.iter().find(|s| s.handle == h))
            .is_some_and(|s| !s.shield.dead);
        // Characters inside a live shield count as protected (INFERRED:
        // EntityManager's protected test is not ported).
        let protected = |c: &crate::npc_combat::Combatant| {
            self.shields.iter().any(|s| {
                !s.shield.dead
                    && (0..3)
                        .map(|i| (c.position[i] - s.shield.position[i]).powi(2))
                        .sum::<f32>()
                        <= s.shield.radius * s.shield.radius
            })
        };
        let self_shields = combatants
            .iter()
            .filter(|c| within(c, 50.))
            .filter(|c| {
                self.character_ref(c.handle).is_some_and(|ch| {
                    ch.combat.health.shield.active()
                        && ch.combat.health.shield.kind
                            == magicka_sim::character_health::SelfShieldType::Shield
                })
            })
            .count();
        let far: Vec<_> = combatants
            .iter()
            .filter(|c| c.handle != host.zone && within(c, 50.))
            .collect();
        Inputs {
            fight_started: running,
            target: target.map(|c| c.position),
            target_dead: target.is_none_or(|c| c.dead),
            players,
            vortex: !self.vortices.is_empty(),
            meteor_shower: self
                .ability_effects
                .effects
                .iter()
                .any(|e| matches!(e, magicka_sim::magicks::AbilityEffect::MeteorShower { .. })),
            blizzard: self.blizzard_cue.is_some(),
            time_warp: self.time_modifier < 1.,
            shield_alive,
            shields: (self.shields.iter().filter(|s| !s.shield.dead).count() + self_shields) as f32,
            // The body zone and the boss's own entry count as in the native list.
            near: combatants
                .iter()
                .filter(|c| within(c, 10.) && !protected(c))
                .count() as f32
                + 1.,
            far_unprotected: far.iter().filter(|c| !protected(c)).count() as f32,
            far_all: far.len() as f32,
        }
    }

    pub(super) fn update_assatur(&mut self, host: &mut AssaturHost, dt: f32, running: bool) {
        // UpdateBoss divides out the time warp (TimeModifier).
        let dt = dt / self.time_modifier.max(f32::EPSILON);
        let inputs = self.assatur_inputs(host, running);
        let mut commands = Vec::new();
        let inbox = self
            .physics_damage
            .get_mut(&host.zone)
            .map(|s| s.take_boss_inbox(self.boss_damage_divisor))
            .unwrap_or_default();
        let position = host.actor.position();
        for (damage, features) in inbox {
            commands.extend(
                host.actor
                    .receive_damage(damage, position, features, &inputs),
            );
        }
        commands.extend(host.actor.update(dt, inputs));
        self.sync_assatur(host);
        for c in commands {
            self.assatur_command(host, c);
        }
        if let Some(shield) = host
            .shield
            .and_then(|h| self.shields.iter_mut().find(|s| s.handle == h))
        {
            if shield.shield.dead {
                host.shield = None;
            } else {
                let spine = host
                    .actor
                    .joint("spinelower")
                    .unwrap_or(host.actor.orientation);
                shield.shield.position = [spine[12], spine[13], spine[14]];
            }
        }
        for key in ["nullify", "death"] {
            let tracked = host.actor.tracked.iter().find(|e| e.0 == key).copied();
            self.track_effect(
                &format!("assatur_live_{key}"),
                tracked.map_or("", |e| e.1),
                tracked.map(|e| e.2),
            );
        }
    }

    /// EffectManager.UpdateOrientation for a scene-owned effect reference:
    /// start under `key` when absent or inactive, move it, or stop it.
    pub(super) fn track_effect(&mut self, key: &str, effect: &str, transform: Option<[f32; 16]>) {
        use crate::effect_action::EffectActionHost;
        let id = custom_hash(key);
        let Some(t) = transform else {
            if let Err(e) = self.stop_effect(id) {
                self.log.push(format!("{key}: {e}"));
            }
            return;
        };
        if let Err(e) = self.add_effect(id, custom_hash(effect), t, 0.) {
            self.log.push(format!("{key}: {e}"));
            return;
        }
        if let Some(stored) = self.effects.effects.get_mut(&id) {
            stored.transform = t;
        }
    }

    fn sync_assatur(&mut self, host: &mut AssaturHost) {
        let a = &host.actor;
        let position = a.position();
        let point = |m: &[f32; 16]| [m[12], m[13], m[14]];
        let forward = |m: &[f32; 16]| [-m[8], -m[9], -m[10]];
        let (cast, weapon) = (a.cast_source(), a.weapon_source());
        let mut source = SpraySource {
            owner_handle: host.zone,
            owner_position: position,
            owner_direction: forward(&a.orientation),
            cast_position: point(&cast),
            cast_direction: forward(&cast),
            weapon_position: point(&weapon),
            weapon_direction: forward(&weapon),
        };
        if a.state == policy::State::Water {
            // Spell.Cast(iFromStaff) sprays from the staff.
            let staff = a.staff();
            source.weapon_position = point(&staff);
            source.weapon_direction = forward(&staff);
        }
        self.set_boss_caster_source(host.zone, source);
        let dead = a.state == policy::State::Dead;
        let hp = a.status.hit_points;
        let orientation = a.orientation;
        if let Ok(e) = self.entity_mut(host.zone) {
            e.position = position;
            e.orientation = orientation;
            e.hit_points = hp;
            e.max_hit_points = policy::MAX_HP;
            e.dead = false;
        }
        if let Some(d) = self.physics_damage.get_mut(&host.zone) {
            if let Some(c) = d.boss_capsule.as_mut() {
                c.previous = c.origin;
                c.origin = position;
                c.origin[1] += CAPSULE_OFFSET;
                c.ethereal = dead;
            }
        }
    }

    fn assatur_cue(&mut self, host: &mut AssaturHost, bank: u16, name: &'static str) {
        let key = self.play_cue(bank, name, Some(host.actor.position()));
        host.cues.retain(|c| c.0 != name);
        host.cues.push((name, key));
    }

    fn assatur_command(&mut self, host: &mut AssaturHost, command: Command) {
        use magicka_sim::damage::{attack, Damage};
        use magicka_sim::spells::{Elements as E, Spell};
        let zone = host.zone;
        match command {
            Command::Trigger(name) => {
                if self.scene_trigger_ids.contains(&custom_hash(name)) {
                    self.pending_triggers.push((custom_hash(name), None));
                }
            }
            Command::Magick(m) => self.cast_magick_from(zone, m),
            Command::Effect { name, transform } => self.status_contact_effects.push((
                name,
                [transform[12], transform[13], transform[14]],
                [-transform[8], -transform[9], -transform[10]],
            )),
            Command::Cue { bank, name } => self.assatur_cue(host, bank, name),
            Command::StopCue(name) => {
                if let Some(i) = host.cues.iter().position(|c| c.0 == name) {
                    let (_, key) = host.cues.remove(i);
                    self.stop_cue(key);
                }
            }
            Command::CameraShake { magnitude, time } => self.camera.shake(magnitude, time),
            Command::Missile {
                position,
                velocity,
                tolerance,
            } => {
                let hit = magicka_content::common::Damage {
                    attack_property: attack::DAMAGE as i32,
                    element: E::ARCANE.0 as i32,
                    amount: 100.,
                    magnitude: 1.,
                };
                let explode = |remove_then_damage: bool| {
                    let mut events = vec![
                        Event::Effect {
                            follow: false,
                            world_align: false,
                            effect: "assatur_missile_explode".into(),
                        },
                        Event::Remove { bounce: 0 },
                    ];
                    if remove_then_damage {
                        events.push(Event::Damage {
                            damage: hit.clone(),
                            velocity_based: false,
                        });
                    }
                    events
                };
                let conditions = vec![
                    assatur_condition(
                        1,
                        0.,
                        true,
                        vec![Event::Effect {
                            follow: true,
                            world_align: false,
                            effect: "assatur_missile_trail".into(),
                        }],
                    ),
                    assatur_condition(2, 0., false, explode(true)),
                    assatur_condition(4, 0., false, explode(true)),
                    assatur_condition(0x10, 5., false, vec![Event::Remove { bounce: 0 }]),
                    assatur_condition(8, 0., false, explode(false)),
                ];
                let (conditions, skipped) = crate::missiles::item_conditions(&conditions);
                for event in skipped {
                    self.log
                        .push(format!("assatur missile event lacks host: {event}"));
                }
                let id = self.next_missile;
                self.next_missile = self.next_missile.wrapping_add(1);
                let mut missile = crate::missiles::LiveMissile::spawn_custom(
                    id, zone, position, velocity, 0.25, false, conditions,
                );
                let mut homing = magicka_sim::missile_homing::Homing::new(host.actor.target, 1.);
                homing.tolerance = tolerance;
                missile.body.homing = Some(homing);
                let mut visuals = Vec::new();
                self.with_rail_world(|world| {
                    crate::missiles::initialize(&mut missile, &mut world.hit, &mut visuals)
                });
                self.missile_visuals.extend(visuals);
                self.register_missile(missile);
            }
            Command::Lightning { damages } => {
                let Some(target) = host.actor.target else {
                    return;
                };
                let Some(c) = self.combatants().into_iter().find(|c| c.handle == target) else {
                    return;
                };
                let target = magicka_sim::lightning::LightningTarget {
                    handle: c.handle,
                    position: c.position,
                    dead: c.dead,
                    eligible: !c.ethereal,
                    frozen: c.status_mask & Status::Frozen as u16 != 0,
                    ends_chain: false,
                };
                let sources = [host.actor.cast_source(), host.actor.weapon_source()];
                let play_time = self.play_time;
                let mut bolts = Vec::new();
                let mut rng = self.rng;
                self.with_rail_world(|world| {
                    let mut host = BarrierLightning {
                        world,
                        bolts: &mut bolts,
                        rng: &mut rng,
                    };
                    // Each cast clears the hit list first (mHitlist.Clear()
                    // precedes both casts, which then share it).
                    let mut hits = magicka_sim::spray_pulse::HitList::default();
                    for s in sources {
                        magicka_sim::lightning::cast_bolt_at(
                            &mut host,
                            zone,
                            play_time,
                            [s[12], s[13], s[14]],
                            target,
                            &mut hits,
                            LIGHTNING_COLOR,
                            1.,
                            30.,
                            damages,
                        );
                    }
                });
                self.rng = rng;
                self.barrier_visuals.extend(bolts);
            }
            Command::Blade {
                position,
                direction,
                damages,
            } => {
                let t = self.play_time;
                let damages: Vec<Damage> = damages.into_iter().filter(|d| d.attack != 0).collect();
                self.arc_damage(
                    Some(zone),
                    Some(zone),
                    position,
                    direction,
                    18.,
                    std::f32::consts::PI * 9. / 20.,
                    &damages,
                    t,
                );
            }
            Command::BladeDetonation {
                position,
                direction,
            } => {
                let segment = Segment {
                    origin: position,
                    delta: direction.map(|v| v * 18.),
                };
                if let Some(hit) = self.collision.collision().segment_intersect(segment, false) {
                    let f = hit.hit.fraction;
                    let at = std::array::from_fn(|i| position[i] + segment.delta[i] * f);
                    self.status_contact_effects
                        .push(("arcane_detonation", at, direction));
                }
            }
            Command::Beam {
                position,
                direction,
            } => {
                let l = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
                if l > 1e-6 {
                    let d = direction.map(|v| v / l);
                    if let Some(c) = self.boss_casters.get_mut(&zone) {
                        c.source.cast_position = position;
                        c.source.cast_direction = d;
                        c.source.weapon_position = position;
                        c.source.weapon_direction = d;
                    }
                    if !host.beam {
                        host.beam = true;
                        let mut spell = Spell {
                            element: E::ARCANE,
                            ..Spell::default()
                        };
                        spell.mag[E::ARCANE.index() as usize] = 5.;
                        self.cast_boss_spell(zone, spell, magicka_sim::damage::CastType::Force, 1.);
                    }
                }
            }
            Command::StopBeam | Command::StopSpell => {
                host.beam = false;
                self.stop_boss_spell(zone);
            }
            Command::Water => {
                let mut spell = Spell::default();
                spell.element |= E::WATER;
                spell.mag[E::WATER.index() as usize] = 5.;
                self.cast_boss_spell(zone, spell, magicka_sim::damage::CastType::Force, 1.);
            }
            Command::Shield {
                position,
                direction,
            } => {
                let handle = self.spawn_shield(
                    zone,
                    position,
                    5.,
                    direction,
                    magicka_sim::shield_entity::ShieldKind::Disc,
                    5000.,
                    magicka_sim::shield_entity::SHIELD_COLOR,
                );
                host.shield = Some(handle);
            }
            Command::ShieldDecay => {
                if let Some(s) = host
                    .shield
                    .and_then(|h| self.shields.iter_mut().find(|s| s.handle == h))
                {
                    let d = Damage {
                        attack: attack::DAMAGE,
                        element: 0,
                        amount: 100.,
                        magnitude: 1.,
                    };
                    s.shield.internal_damage(&d, true);
                }
            }
            Command::Boss(event) => {
                use magicka_sim::boss_status::BossEvent as B;
                let mut p = host
                    .actor
                    .joint("spinelower")
                    .map_or(host.actor.position(), |m| [m[12], m[13], m[14]]);
                p[1] += 1.5 * 1.2;
                match event {
                    B::Blood { position } => self.status_contact_effects.push((
                        "gore_splash_black",
                        position,
                        [1., 0., 0.],
                    )),
                    B::LockedNumber { amount } => {
                        self.boss_number = Some(self.floating_text.add(amount, p, 0.4, true))
                    }
                    B::AddToNumber { amount } => {
                        if let Some(index) = self.boss_number {
                            self.floating_text.add_to(index, amount);
                        }
                    }
                    B::MoveNumber => {
                        if let Some(index) = self.boss_number {
                            self.floating_text.update_position(index, p);
                        }
                    }
                    B::ReleaseNumber => {
                        if let Some(index) = self.boss_number.take() {
                            self.floating_text.release(index);
                        }
                    }
                    B::Number { amount } => {
                        self.floating_text.add(amount, p, 0.7, false);
                    }
                }
            }
        }
    }

    pub(super) fn assatur_message(&mut self, host: &mut AssaturHost, name: &str) {
        let inputs = self.assatur_inputs(host, self.boss_fight.running);
        for c in host.actor.message(name, &inputs) {
            self.assatur_command(host, c);
        }
    }
}

fn assatur_condition(
    kind: u8,
    time: f32,
    repeat: bool,
    events: Vec<Event>,
) -> magicka_content::common::EventCollection {
    magicka_content::common::EventCollection {
        condition: magicka_content::common::EventCondition {
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
