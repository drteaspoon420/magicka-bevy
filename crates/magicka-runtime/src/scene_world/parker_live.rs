//! Scene host for Parker (research/boss-parker.md): the abdomen and mouth
//! BossDamageZones, the limb contacts, the web strands and the commands of
//! `boss_parker::Parker`.
use super::*;
use crate::boss_parker::{self as policy, Contact, Emitter, Parker, ParkerCommand, ParkerWorld};
use crate::xna_math::{self as m, V3};
use magicka_sim::status::Status;

/// BossDamageZone(0, 2.5, Sphere(Up, 2.5)): the abdomen sphere sits 1 m
/// above the zone position.
const ABDOMEN_RADIUS: f32 = 2.5;
/// BossDamageZone(1, 0.9, Sphere(Zero, 0.8)).
const MOUTH_RADIUS: f32 = 0.8;

#[derive(Clone, Debug)]
pub struct ParkerHost {
    pub parker: Box<Parker>,
    pub abdomen: u16,
    pub mouth: u16,
    /// The last strand of the current web (WebState.mEntity).
    last_web: Option<usize>,
}
impl PartialEq for ParkerHost {
    fn eq(&self, other: &Self) -> bool {
        (self.abdomen, self.mouth) == (other.abdomen, other.mouth)
    }
}

struct Scene<'a, C: SceneContent>(&'a mut SceneWorld<C>);

impl<C: SceneContent> ParkerWorld for Scene<'_, C> {
    fn ground(&mut self, origin: V3, delta: V3) -> Option<V3> {
        self.0
            .collision
            .collision()
            .segment_intersect(magicka_sim::spray_pulse::Segment { origin, delta }, false)
            .map(|h| h.hit.position)
    }
    fn shield_blocks(&mut self, origin: V3, delta: V3) -> bool {
        let segment = magicka_sim::spray_pulse::Segment { origin, delta };
        self.0
            .shields
            .iter()
            .any(|s| !s.shield.dead && s.shield.geometry().segment_fraction(segment).is_some())
    }
    fn count(&mut self, template: &str) -> usize {
        // TriggerArea("any").GetCount(type).
        let id = custom_hash(template);
        EntityCommandHost::present_characters(self.0, custom_hash("any"))
            .ok()
            .flatten()
            .map_or(0, |v| v.iter().filter(|e| e.type_id == id).count())
    }
    fn locator(&mut self, name: &str) -> Option<[f32; 16]> {
        self.0.scene_locator(custom_hash(name)).ok()
    }
    fn nav_nearest(&mut self, point: V3) -> V3 {
        self.0
            .nav
            .as_ref()
            .and_then(|n| n.nearest(point, 0))
            .map_or(point, |(_, p, _)| p)
    }
}

fn sphere_zone(index: u8, center: V3, radius: f32) -> crate::physics_damage::PhysicsDamage {
    let dummy = magicka_content::physics::CollisionBox {
        name: "parker_zone".into(),
        position: [0.; 3],
        sides: [0.; 3],
        orientation: [0., 0., 0., 1.],
    };
    let mut damage = crate::physics_damage::PhysicsDamage::boss_zone(index, dummy);
    damage.collision_box = None;
    damage.solid = false;
    damage.boss_capsule = Some(crate::physics_damage::BossCapsule {
        origin: center,
        previous: center,
        length: 0.,
        radius,
        ethereal: false,
    });
    damage
}

impl<C: SceneContent> SceneWorld<C> {
    /// Parker.Initialize at the spawn locator.
    pub(super) fn spawn_parker(&mut self, area: i32) -> Result<ParkerHost, WorldError> {
        let mesh = self
            .content
            .skinned_model(policy::MESH_MODEL)
            .ok_or("missing Parker mesh")?;
        let animation = self
            .content
            .skinned_model(policy::ANIMATION_MODEL)
            .ok_or("missing Parker animation")?;
        for t in [
            policy::SPIDERLING,
            policy::FOREST_SPIDER,
            policy::POISON_SPIDER,
        ] {
            crate::spawn_action::SpawnLevel::load_character_template(self, Some(t))?;
        }
        let orientation = self.scene_locator(area)?;
        let mut parker = Box::new(Parker::new(mesh, animation, 0x7061_726b)?);
        parker.initialize(orientation, &mut Scene(self));
        let mut zones = [0u16; 2];
        for (i, slot) in zones.iter_mut().enumerate() {
            let handle = self.allocate_physics(true);
            let (center, radius) = if i == 0 {
                (m::add(parker.abdomen, [0., 1., 0.]), ABDOMEN_RADIUS)
            } else {
                (m::translation(&parker.mouth), MOUTH_RADIUS)
            };
            self.physics_damage
                .insert(handle, sphere_zone(i as u8, center, radius));
            let e = self.entity_mut(handle)?;
            e.added = true;
            e.radius = radius;
            e.hit_points = policy::MAX_HIT_POINTS;
            e.max_hit_points = policy::MAX_HIT_POINTS;
            *slot = handle;
        }
        let mut host = ParkerHost {
            parker,
            abdomen: zones[0],
            mouth: zones[1],
            last_web: None,
        };
        self.place_parker_zones(&host);
        for c in host.parker.take_pending() {
            self.parker_command(&mut host, c);
        }
        self.log.push(format!("boss parker: zones {zones:?}"));
        Ok(host)
    }

    /// Zone positions, capsule history and the abdomen's character
    /// obstacle (a box around the sphere, INFERRED: the rewrite's static
    /// obstacles are boxes).
    fn place_parker_zones(&mut self, host: &ParkerHost) {
        let p = &host.parker;
        let hp = p.status.hit_points;
        for (handle, center) in [
            (host.abdomen, m::add(p.abdomen, [0., 1., 0.])),
            (host.mouth, m::translation(&p.mouth)),
        ] {
            if let Some(d) = self.physics_damage.get_mut(&handle) {
                if let Some(c) = d.boss_capsule.as_mut() {
                    c.previous = c.origin;
                    c.origin = center;
                    c.ethereal = p.dead;
                }
            }
            if let Ok(e) = self.entity_mut(handle) {
                e.position = center;
                e.orientation = magicka_content::skinned::IDENTITY;
                m::set_translation(&mut e.orientation, center);
                e.hit_points = hp;
            }
        }
        let side = ABDOMEN_RADIUS * 1.6;
        self.static_prop_boxes.insert(
            host.abdomen,
            magicka_content::physics::CollisionBox {
                name: "parker_abdomen".into(),
                position: [-side * 0.5; 3],
                sides: [side; 3],
                orientation: [0., 0., 0., 1.],
            },
        );
        if p.dead {
            self.static_prop_boxes.remove(&host.abdomen);
        }
    }

    fn parker_players(&self) -> Vec<policy::Player> {
        let combatants = self.combatants();
        (0..self.players.len().max(4))
            .map(|i| {
                let Some(p) = self.players.get(i).and_then(Option::as_ref) else {
                    return policy::Player::default();
                };
                let body = &self.physics.bodies[p.body];
                let poisoned = combatants
                    .iter()
                    .find(|c| c.handle == p.handle)
                    .is_some_and(|c| c.status_mask & Status::Poisoned as u16 != 0);
                policy::Player {
                    playing: p.playing,
                    avatar: Some(p.handle),
                    dead: p.dead(),
                    position: body.position,
                    entangled: body.owner.entangled,
                    poisoned,
                }
            })
            .collect()
    }

    /// Parker.UpdateBoss with the zone deliveries of the last frame.
    pub(super) fn update_parker(&mut self, host: &mut ParkerHost, dt: f32) {
        let mut commands = Vec::new();
        for (part, zone) in [(0usize, host.abdomen), (1, host.mouth)] {
            let inbox = self
                .physics_damage
                .get_mut(&zone)
                .map(|z| z.take_boss_inbox(self.boss_damage_divisor))
                .unwrap_or_default();
            let position = self.entity(zone).map_or([0.; 3], |e| e.position);
            for (damage, features) in inbox {
                host.parker
                    .damage(part, damage, position, features, &mut commands);
            }
        }
        let players = self.parker_players();
        commands.extend(host.parker.update(dt, players, &mut Scene(self)));
        for _ in 0..host.parker.footsteps() {
            let at = host.parker.abdomen;
            self.play_cue(crate::audio::banks::ADDITIONAL, "parker_footstep", Some(at));
        }
        self.place_parker_zones(host);
        for c in commands {
            self.parker_command(host, c);
        }
        self.parker_contacts(host);
    }

    /// The zones' collision callbacks against characters, props, shields
    /// and barriers overlapping them this frame.
    fn parker_contacts(&mut self, host: &mut ParkerHost) {
        if host.parker.dead {
            return;
        }
        let abdomen = m::add(host.parker.abdomen, [0., 1., 0.]);
        let mouth = m::translation(&host.parker.mouth);
        let limbs = host.parker.limbs.clone();
        let touches_body = |p: V3, r: f32, l: f32| {
            let top = m::add(p, [0., l, 0.]);
            m::point_segment_distance_squared(abdomen, p, top) <= (ABDOMEN_RADIUS + r).powi(2)
                || limbs.iter().any(|(a, b, lr)| {
                    m::segment_segment_distance_squared(*a, *b, p, top) <= (lr + r).powi(2)
                })
        };
        let touches_mouth = |p: V3, r: f32, l: f32| {
            let top = m::add(p, [0., l, 0.]);
            m::point_segment_distance_squared(mouth, p, top) <= (MOUTH_RADIUS + r).powi(2)
        };
        let mut body_hits = Vec::new();
        let mut mouth_hits = Vec::new();
        for c in self.combatants() {
            if c.dead {
                continue;
            }
            let base = m::sub(c.position, [0., c.length * 0.5, 0.]);
            let mass = self
                .character_body(c.handle)
                .map_or(0., |b| self.physics.bodies[b].mass);
            if touches_body(base, c.radius, c.length) {
                body_hits.push((c.handle, true, mass));
            }
            if touches_mouth(base, c.radius, c.length) {
                mouth_hits.push((c.handle, false, mass));
            }
        }
        for e in self.entities.iter().filter(|e| {
            e.added
                && !e.removed
                && !e.dead
                && matches!(e.kind, EntityKind::Physics { .. })
                && e.handle != host.abdomen
                && e.handle != host.mouth
        }) {
            let Some(d) = self.physics_damage.get(&e.handle) else {
                continue;
            };
            if d.boss_zone.is_some() {
                continue;
            }
            if touches_body(e.position, e.radius, 0.) {
                body_hits.push((e.handle, false, d.mass));
            }
            if touches_mouth(e.position, e.radius, 0.) {
                mouth_hits.push((e.handle, false, d.mass));
            }
        }
        for (handle, character, mass) in body_hits {
            let c = host.parker.body_contact(handle, character, mass);
            self.parker_contact(host, handle, c, None);
        }
        for (handle, _, mass) in mouth_hits {
            let c = host.parker.mouth_contact(handle, false, mass);
            self.parker_contact(host, handle, c, None);
        }
        // Shields and barriers caught by the mouth.
        let shields: Vec<u16> = self
            .shields
            .iter()
            .filter(|s| !s.shield.dead)
            .filter(|s| {
                m::length(m::sub(s.shield.position, mouth)) <= s.shield.radius + MOUTH_RADIUS
            })
            .map(|s| s.handle)
            .collect();
        for h in shields {
            let c = host.parker.mouth_contact(h, true, 0.);
            self.parker_contact(host, h, c, Some(Blocker::Shield(h)));
        }
        let barriers: Vec<usize> = self
            .barriers
            .iter()
            .filter(|(_, b)| m::length(m::sub(b.position, mouth)) <= b.radius + MOUTH_RADIUS)
            .map(|(slot, _)| slot)
            .collect();
        for slot in barriers {
            let c = host
                .parker
                .mouth_contact(BARRIER_KEY | slot as u16, true, 0.);
            self.parker_contact(host, 0, c, Some(Blocker::Barrier(slot)));
        }
    }

    fn parker_contact(
        &mut self,
        host: &ParkerHost,
        target: u16,
        contact: Contact,
        blocker: Option<Blocker>,
    ) {
        match contact {
            Contact::None => {}
            Contact::KillBlocker => match blocker {
                Some(Blocker::Shield(h)) => {
                    if let Some(s) = self.shields.iter_mut().find(|s| s.handle == h) {
                        s.shield.kill();
                    }
                }
                Some(Blocker::Barrier(slot)) => self.kill_barrier(slot),
                None => {}
            },
            Contact::Damage {
                damages,
                mouth,
                position,
            } => {
                let attacker = if mouth { host.mouth } else { host.abdomen };
                if blocker.is_some() {
                    // Damage to a shield/barrier outside the kill window is
                    // not modelled (INFERRED: rare, the bite kills them).
                    return;
                }
                if self.physics_damage.contains_key(&target) {
                    for d in damages {
                        let _ = self.damage_physics(target, d, 15);
                    }
                } else {
                    self.grip_damage(target, attacker, &damages, position);
                }
            }
        }
    }

    fn parker_cue(
        &mut self,
        host: &ParkerHost,
        name: &'static str,
        emitter: Emitter,
        characters: bool,
    ) {
        let at = match emitter {
            Emitter::Abdomen => host.parker.abdomen,
            Emitter::Mouth => m::translation(&host.parker.mouth),
        };
        let bank = if characters {
            crate::audio::banks::CHARACTERS
        } else {
            crate::audio::banks::ADDITIONAL
        };
        self.play_cue(bank, name, Some(at));
    }

    fn parker_command(&mut self, host: &mut ParkerHost, command: ParkerCommand) {
        match command {
            ParkerCommand::Cue {
                name,
                emitter,
                characters,
            } => self.parker_cue(host, name, emitter, characters),
            ParkerCommand::Spawn {
                template,
                position,
                direction,
                attack,
                dead_timer,
            } => {
                use crate::npc::{ORDER_ATTACK, REACT_ATTACK, REACT_PROXIMITY};
                // Order.Idle.
                const ORDER_IDLE: u8 = 1;
                let order = crate::spawn_action::SpawnOrder {
                    order: if attack { ORDER_ATTACK } else { ORDER_IDLE },
                    react_to: REACT_ATTACK | REACT_PROXIMITY,
                    reaction: ORDER_ATTACK,
                    priority_target: 0,
                    priority_ability: -1,
                    trigger: 0,
                    events: &[],
                };
                let Some(h) = self.summon_npc(
                    None,
                    template,
                    position,
                    direction,
                    magicka_sim::summons::SummonKind::Normal,
                    order,
                ) else {
                    self.log.push(format!("Parker failed to spawn {template}"));
                    return;
                };
                if let Some(n) = self.npcs.get_mut(&h) {
                    n.motion.dead_timer = dead_timer;
                }
                if let Err(e) = self.begin_resurrection(h, 170) {
                    self.log.push(format!("Parker spawn animation: {e}"));
                }
            }
            ParkerCommand::Web {
                index,
                position,
                direction,
                velocity,
                mouth_owner,
            } => {
                let slot = self.web_strands.instance();
                self.web_strands.initialize(
                    slot,
                    mouth_owner.then_some(host.mouth),
                    position,
                    direction,
                    velocity,
                );
                if index > 1 {
                    if let Some(prev) = host.last_web {
                        self.web_strands.set_child(prev, slot);
                    }
                }
                host.last_web = Some(slot);
            }
            ParkerCommand::SwipeSweep { center, radius } => {
                let position = m::translation(&host.parker.transform);
                let mut targets: Vec<(u16, f32, bool)> = self
                    .combatants()
                    .into_iter()
                    .filter(|c| !c.dead && m::length(m::sub(c.position, center)) <= radius)
                    .map(|c| {
                        let mass = self
                            .character_body(c.handle)
                            .map_or(0., |b| self.physics.bodies[b].mass);
                        (c.handle, mass, false)
                    })
                    .collect();
                for e in &self.entities {
                    if !e.added || e.removed || e.dead || e.handle == host.abdomen {
                        continue;
                    }
                    if let Some(d) = self.physics_damage.get(&e.handle) {
                        if d.boss_zone.is_none() && m::length(m::sub(e.position, center)) <= radius
                        {
                            targets.push((e.handle, d.mass, true));
                        }
                    }
                }
                for (h, mass, prop) in targets {
                    if host.parker.hit_list.contains(h) {
                        continue;
                    }
                    let damages = Parker::swipe_damages(mass);
                    if prop {
                        for d in damages {
                            let _ = self.damage_physics(h, d, 15);
                        }
                    } else {
                        self.grip_damage(h, host.abdomen, &damages, position);
                    }
                    host.parker.hit_list.add(h);
                    if host.parker.swipe_sound() {
                        self.parker_cue(host, "parker_swipe_sides", Emitter::Abdomen, false);
                    }
                }
            }
            ParkerCommand::Boss(event) => {
                // NotifierTextPostion: the mouth, 3 m up; BloodEffect is
                // GORE_GIB_MEDIUM_EFFECTS[4].
                let mut notifier = m::translation(&host.parker.mouth);
                notifier[1] += 3.0;
                self.present_boss_event(event, notifier, "gore_gib_insect_medium");
            }
        }
    }

    /// SprayEntity updates: flight, contacts and fading (all live strands,
    /// whether or not Parker is still fighting).
    pub(super) fn update_web_strands(&mut self, dt: f32) {
        if !self.web_strands.slots.iter().any(|s| s.added) {
            return;
        }
        let owners: std::collections::BTreeMap<u16, V3> = self
            .entities
            .iter()
            .filter(|e| e.added && !e.removed)
            .map(|e| (e.handle, e.position))
            .collect();
        self.web_strands.update(dt, |h| owners.get(&h).copied());
        let avatars: Vec<(u16, V3, f32, f32, bool)> = self
            .players
            .iter()
            .flatten()
            .filter(|p| !p.dead())
            .map(|p| {
                let b = &self.physics.bodies[p.body];
                (p.handle, b.position, b.radius, b.length, b.owner.entangled)
            })
            .collect();
        for i in 0..self.web_strands.slots.len() {
            let s = self.web_strands.slots[i].clone();
            if !s.added || s.dead || s.immovable {
                continue;
            }
            let delta = m::scale(s.velocity, dt);
            let segment = magicka_sim::spray_pulse::Segment {
                origin: s.position,
                delta,
            };
            let level = self
                .collision
                .collision()
                .segment_intersect(segment, false)
                .map(|h| h.hit.position);
            let shield = self
                .shields
                .iter()
                .filter(|sh| !sh.shield.dead)
                .filter_map(|sh| sh.shield.geometry().segment_fraction(segment))
                .fold(None, |a: Option<f32>, f| Some(a.map_or(f, |a| a.min(f))));
            let strand = &mut self.web_strands.slots[i];
            if let Some(f) = shield {
                strand.position = m::add(s.position, m::scale(delta, f));
                strand.immovable = true;
                continue;
            }
            if let Some(p) = level {
                strand.position = p;
                strand.immovable = true;
                continue;
            }
            strand.position = m::add(s.position, delta);
            let (a, b) = (strand.position, strand.tail.unwrap_or(strand.position));
            let hit = avatars.iter().find(|(_, p, r, l, entangled)| {
                let base = m::sub(*p, [0., l * 0.5, 0.]);
                let top = m::add(base, [0., *l, 0.]);
                !entangled
                    && m::segment_segment_distance_squared(a, b, base, top)
                        <= (crate::web_strand::RADIUS + r).powi(2)
            });
            if let Some(&(handle, position, ..)) = hit {
                strand.immovable = true;
                strand.velocity = [0.; 3];
                let owner = strand.owner.unwrap_or(0);
                // Character.Entangle through the Entanglement damage path.
                self.grip_damage(
                    handle,
                    owner,
                    &[magicka_sim::damage::Damage {
                        attack: magicka_sim::damage::attack::ENTANGLEMENT,
                        element: 0,
                        amount: 0.,
                        magnitude: crate::web_strand::ENTANGLE,
                    }],
                    position,
                );
            }
        }
        self.web_strands.remove_dead();
    }
}

#[derive(Clone, Copy, Debug)]
enum Blocker {
    Shield(u16),
    Barrier(usize),
}

/// A key outside entity handles for barriers in Parker's hit list.
const BARRIER_KEY: u16 = 0xF000;
