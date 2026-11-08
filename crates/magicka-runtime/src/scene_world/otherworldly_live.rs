//! OtherworldlyBolt (Cthulhu's homing bolts) and OtherworldlyDischarge
//! (the Starspawn special ability, also fired by a bolt that reaches a
//! character): the victim is overkilled and bloat-killed, and 0.333 s
//! later a Cultist rises where it stood. See research/boss-cthulhu.md.
use super::*;
use crate::xna_math::{self as m, V3};

/// OtherworldlyDischarge.RANGE and SPREAD (30°).
const DISCHARGE_RANGE: f32 = 6.0;
const DISCHARGE_DELAY: f32 = 0.333;
const CULTIST_TEMPLATE: &str = "cultist";
const BOLT_RADIUS: f32 = 0.5;

/// A queued OtherworldlyDischarge spawn (sDelayedSpawns).
#[derive(Clone, Debug, PartialEq)]
pub struct DischargeSpawn {
    pub template: &'static str,
    pub ttl: f32,
    pub victim: u16,
    pub position: V3,
    pub direction: V3,
}

/// One OtherworldlyBolt of Cthulhu's eight-bolt cache.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bolt {
    pub caster: u16,
    pub position: V3,
    pub velocity: V3,
    acceleration: f32,
    terminal: f32,
    to_target: V3,
    target: Option<u16>,
    check_timer: f32,
    /// mOkToStart (GoHunt).
    pub hunting: bool,
    pub done: bool,
    /// In the entity manager and not removable.
    pub live: bool,
    idle_cue: Option<u32>,
    key: usize,
}

impl Bolt {
    fn effect_key(&self) -> String {
        format!("cthulhu_bolt{}", self.key)
    }
}

/// What the bolt touched this frame (OnCollision).
enum BoltHit {
    Character { handle: u16, discharge: bool },
    Blocker(u16),
}

impl<C: SceneContent> SceneWorld<C> {
    /// OtherworldlyDischarge.Execute(target, owner).
    pub(super) fn otherworldly_discharge(&mut self, target: u16, owner: u16) {
        let body = self.character_body(target);
        let (position, direction) = body.map_or(([0.0; 3], [0.0, 0.0, -1.0]), |b| {
            let b = &self.physics.bodies[b];
            (b.position, b.direction())
        });
        self.overkill(target);
        if body.is_some() {
            self.bloat_kill(target, 0, owner);
            self.discharge_spawns.push(DischargeSpawn {
                template: self.discharge_template.unwrap_or(CULTIST_TEMPLATE),
                ttl: DISCHARGE_DELAY,
                victim: target,
                position,
                direction,
            });
        }
    }

    /// The special ability cast by an NPC (Starspawn): its AI target if
    /// within 6 m, inside 30° and not behind a shield, else the closest
    /// damageable in the arc. A non-solid self shield only breaks.
    pub(super) fn otherworldly_discharge_from(&mut self, owner: u16) {
        let Some(body) = self.character_body(owner) else {
            return;
        };
        let (position, direction) = {
            let b = &self.physics.bodies[body];
            (b.position, b.direction())
        };
        let combatants = self.combatants();
        let spread_cos = 30f32.to_radians().cos();
        let mut target = self
            .npcs
            .get(&owner)
            .and_then(|n| n.combat.current_target())
            .and_then(|t| combatants.iter().find(|c| c.handle == t))
            .filter(|c| {
                let d = m::sub(c.position, position);
                let l2 = m::length_squared(d);
                if l2 > DISCHARGE_RANGE * DISCHARGE_RANGE {
                    return false;
                }
                if l2 > 1e-6 && m::dot(m::scale(d, 1.0 / l2.sqrt()), direction) < spread_cos {
                    return false;
                }
                let segment = Segment {
                    origin: position,
                    delta: d,
                };
                !self.shields.iter().any(|s| {
                    !s.shield.dead && s.shield.geometry().segment_fraction(segment).is_some()
                })
            })
            .map(|c| c.handle);
        if target.is_none() {
            // GetTarget: the closest character in the arc (a non-solid
            // self shield counts as 6 m away).
            let mut best = f32::MAX;
            for c in combatants.iter().filter(|c| c.handle != owner && !c.dead) {
                let d = m::sub(c.position, position);
                let distance = m::length(d);
                if distance - c.radius > DISCHARGE_RANGE {
                    continue;
                }
                let flat = [d[0], 0.0, d[2]];
                if m::length_squared(flat) > 1e-6
                    && m::dot(m::normalize(flat), direction) < 30f32.to_radians().cos()
                {
                    continue;
                }
                let soft = self.self_shielded(c.handle).is_some_and(|solid| !solid);
                let score = if soft { 6.0 } else { distance * distance };
                if score < best {
                    best = score;
                    target = Some(c.handle);
                }
            }
        }
        if let Some(t) = target {
            if self.self_shielded(t).is_some_and(|solid| !solid) {
                self.remove_self_shield(t);
            } else {
                self.otherworldly_discharge(t, owner);
            }
        }
        self.play_cue(crate::audio::banks::MISC, "misc_flash", Some(position));
    }

    /// `Some(solid)` while self-shielded (IsSolidSelfShielded: a non-Shield
    /// element armour).
    fn self_shielded(&self, handle: u16) -> Option<bool> {
        let c = self.character_ref(handle)?;
        let s = &c.combat.health.shield;
        s.active()
            .then(|| s.kind != magicka_sim::character_health::SelfShieldType::Shield)
    }

    /// Character.RemoveSelfShield.
    pub(super) fn remove_self_shield(&mut self, handle: u16) {
        if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, handle) {
            let shield = &mut c.combat.health.shield;
            if shield.active() {
                shield.remove(c.definition.mass, false, &mut c.shield_events);
            }
        }
    }

    /// Character.Hypnotize's effect (cthulhu_player_hypnotize) follows the
    /// character's position and direction until StopHypnotize.
    pub(super) fn sync_hypnotize(&mut self) {
        let hypnotized: Vec<(u16, usize)> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.character.hypnotized.is_some())
            .map(|p| (p.handle, p.body))
            .chain(
                self.npcs
                    .iter()
                    .filter(|(_, n)| n.character.hypnotized.is_some())
                    .map(|(h, n)| (*h, n.body)),
            )
            .collect();
        for &(h, body) in &hypnotized {
            let b = &self.physics.bodies[body];
            let mut t = m::create_world([0.0; 3], b.direction(), [0.0, 1.0, 0.0]);
            m::set_translation(&mut t, b.position);
            self.track_effect(
                &format!("hypnotize_{h}"),
                "cthulhu_player_hypnotize",
                Some(t),
            );
            self.hypnotize_shown.insert(h);
        }
        let stopped: Vec<u16> = self
            .hypnotize_shown
            .iter()
            .copied()
            .filter(|h| !hypnotized.iter().any(|(x, _)| x == h))
            .collect();
        for h in stopped {
            self.track_effect(&format!("hypnotize_{h}"), "", None);
            self.hypnotize_shown.remove(&h);
        }
    }

    /// OtherworldlyDischarge.Update: the delayed Cultist spawns.
    pub(super) fn update_discharges(&mut self, dt: f32) {
        let mut ready = Vec::new();
        self.discharge_spawns.retain_mut(|s| {
            s.ttl -= dt;
            if s.ttl <= 0.0 {
                ready.push(s.clone());
                false
            } else {
                true
            }
        });
        for s in ready {
            let position = self
                .character_body(s.victim)
                .map_or(s.position, |b| self.physics.bodies[b].position);
            if let Err(e) =
                crate::spawn_action::SpawnLevel::load_character_template(self, Some(s.template))
            {
                self.log.push(format!("otherworldly discharge: {e}"));
            } else {
                use crate::npc::{ORDER_ATTACK, REACT_ATTACK, REACT_PROXIMITY};
                let order = crate::spawn_action::SpawnOrder {
                    order: ORDER_ATTACK,
                    react_to: REACT_ATTACK | REACT_PROXIMITY,
                    reaction: ORDER_ATTACK,
                    priority_target: 0,
                    priority_ability: -1,
                    trigger: 0,
                    events: &[],
                };
                if self
                    .summon_npc(
                        None,
                        s.template,
                        position,
                        s.direction,
                        magicka_sim::summons::SummonKind::Normal,
                        order,
                    )
                    .is_none()
                {
                    self.log.push("otherworldly discharge: no cultist".into());
                }
            }
            self.status_contact_effects
                .push(("starspawn_owd_hit", position, [0.0, 0.0, -1.0]));
        }
    }

    // --- bolts ---------------------------------------------------------

    /// OtherworldlyBolt.Spawn from the cache (the front entry moves back).
    pub(super) fn spawn_bolt(
        &mut self,
        bolts: &mut Vec<Bolt>,
        caster: u16,
        position: V3,
        direction: V3,
        speed: f32,
    ) -> usize {
        let index = bolts.iter().position(|b| !b.live).unwrap_or_else(|| {
            bolts.push(Bolt {
                key: bolts.len(),
                ..Default::default()
            });
            bolts.len() - 1
        });
        let key = bolts[index].key;
        let b = &mut bolts[index];
        *b = Bolt {
            caster,
            position,
            velocity: m::scale(direction, speed),
            acceleration: speed,
            terminal: speed,
            live: true,
            key,
            ..Default::default()
        };
        let k = b.effect_key();
        self.track_effect(&k, "cthulhu_otherworldly_bolt", Some(translation(position)));
        self.play_cue(
            crate::audio::banks::ADDITIONAL,
            "cthulhu_bolt_spawn",
            Some(position),
        );
        index
    }

    /// GoHunt.
    pub(super) fn bolt_hunt(&mut self, bolt: &mut Bolt) {
        bolt.hunting = true;
        bolt.idle_cue = Some(self.play_cue(
            crate::audio::banks::ADDITIONAL,
            "cthulhu_bolt_idle",
            Some(bolt.position),
        ));
    }

    /// Destroy(iCultistSpawned, iKillTarget, target, iInSilence).
    pub(super) fn destroy_bolt(
        &mut self,
        bolt: &mut Bolt,
        cultist: bool,
        kill_target: bool,
        target: Option<u16>,
        silent: bool,
    ) {
        if !bolt.live {
            return;
        }
        if !cultist {
            self.status_contact_effects.push((
                "cthulhu_otherworldly_bolt_hit",
                bolt.position,
                [1.0, 0.0, 0.0],
            ));
        }
        if let Some(t) = target {
            if kill_target {
                self.kill_damageable(t);
            }
            self.remove_self_shield(t);
        }
        bolt.live = false;
        bolt.done = true;
        let k = bolt.effect_key();
        self.track_effect(&k, "", None);
        if let Some(cue) = bolt.idle_cue.take() {
            self.stop_cue(cue);
        }
        if !silent {
            self.play_cue(
                crate::audio::banks::ADDITIONAL,
                "cthulhu_bolt_hit",
                Some(bolt.position),
            );
        }
    }

    /// Entity.Kill on a shield, barrier, mine or missile handle.
    pub(super) fn kill_damageable(&mut self, target: u16) {
        if let Some(s) = self.shields.iter_mut().find(|s| s.handle == target) {
            s.shield.kill();
        } else if target & MINE_KEY == BARRIER_KEY {
            self.kill_barrier((target & !BARRIER_KEY) as usize);
        } else if target & MINE_KEY == MINE_KEY {
            if let Some(mine) = self.mines.get_mut((target & !MINE_KEY) as usize) {
                mine.dead = true;
            }
        } else if let Some(missile) = self.missiles.iter_mut().find(|m| m.handle == Some(target)) {
            missile.body.dead = true;
        }
    }

    /// OtherworldlyBolt.Update and OnCollision for every live bolt.
    pub(super) fn update_bolts(&mut self, bolts: &mut [Bolt], dt: f32) {
        let starspawn = custom_hash("starspawn");
        for bolt in bolts.iter_mut().filter(|b| b.live && b.hunting) {
            bolt.check_timer -= dt;
            if bolt.check_timer <= 0.0 {
                // FindClosestAvatar (XZ distance).
                bolt.target = self
                    .players
                    .iter()
                    .flatten()
                    .filter(|p| p.playing && !p.dead() && !p.character.ethereal)
                    .min_by(|a, b| {
                        let d = |p: &crate::avatar::PlayerAvatar| {
                            let q = self.physics.bodies[p.body].position;
                            (q[0] - bolt.position[0]).powi(2) + (q[2] - bolt.position[2]).powi(2)
                        };
                        d(a).total_cmp(&d(b))
                    })
                    .map(|p| p.handle);
                bolt.check_timer = 1.0;
            }
            // Non-friendly characters within 5 m push the bolt away.
            for c in self.combatants() {
                if c.faction & 1 != 0 {
                    continue;
                }
                let d = [
                    c.position[0] - bolt.position[0],
                    0.0,
                    c.position[2] - bolt.position[2],
                ];
                let l2 = m::length_squared(d);
                if l2 <= 25.0 && l2 > 1e-6 {
                    let push = m::scale(m::normalize(d), dt / (l2 / 25.0));
                    bolt.velocity = m::sub(bolt.velocity, push);
                }
            }
            // FlyTowardsTarget.
            if let Some(t) = bolt.target {
                if let Some(c) = self
                    .combatants()
                    .into_iter()
                    .find(|c| c.handle == t && !c.dead && !c.ethereal)
                {
                    bolt.to_target = m::sub(c.position, bolt.position);
                }
            }
            if bolt.position[1] <= 0.0 {
                bolt.to_target[1] = 0.0;
                bolt.position[1] = 0.0;
            }
            if m::length_squared(bolt.to_target) > 1e-6 {
                bolt.to_target = m::normalize(bolt.to_target);
                let mut v = m::add(
                    bolt.velocity,
                    m::scale(bolt.to_target, dt * bolt.acceleration * 2.0),
                );
                if m::length_squared(v) > bolt.terminal * bolt.terminal {
                    v = m::scale(m::normalize(v), bolt.terminal);
                }
                bolt.velocity = v;
                bolt.position = m::add(bolt.position, m::scale(v, dt));
                let k = bolt.effect_key();
                let mut t = m::create_world([0.0; 3], bolt.to_target, [0.0, 1.0, 0.0]);
                m::set_translation(&mut t, bolt.position);
                self.track_effect(&k, "cthulhu_otherworldly_bolt", Some(t));
            }
            // OnCollision: the first touching character, shield, barrier or
            // friendly missile.
            let mut hit = None;
            for c in self.combatants() {
                let base = m::sub(c.position, [0.0, c.length * 0.5, 0.0]);
                let top = m::add(base, [0.0, c.length, 0.0]);
                if m::point_segment_distance_squared(bolt.position, base, top)
                    > (c.radius + BOLT_RADIUS).powi(2)
                {
                    continue;
                }
                let is_starspawn = matches!(
                    self.entity(c.handle).map(|e| e.kind.clone()),
                    Ok(EntityKind::Npc { type_id, .. }) if type_id == starspawn
                );
                if is_starspawn || c.dead {
                    continue;
                }
                let soft = self.self_shielded(c.handle).is_some_and(|solid| !solid);
                hit = Some(BoltHit::Character {
                    handle: c.handle,
                    discharge: !c.ethereal && !soft,
                });
                break;
            }
            if hit.is_none() {
                hit = self
                    .shields
                    .iter()
                    .filter(|s| !s.shield.dead)
                    .find(|s| {
                        m::length(m::sub(s.shield.position, bolt.position))
                            <= s.shield.radius + BOLT_RADIUS
                    })
                    .map(|s| BoltHit::Blocker(s.handle))
                    .or_else(|| {
                        self.barriers
                            .iter()
                            .find(|(_, b)| {
                                m::length(m::sub(b.position, bolt.position))
                                    <= b.radius + BOLT_RADIUS
                            })
                            .map(|(slot, _)| BoltHit::Blocker(BARRIER_KEY | slot as u16))
                    })
                    .or_else(|| {
                        self.missiles
                            .iter()
                            .filter(|mi| !mi.body.dead)
                            .filter(|mi| {
                                // A missile of a non-friendly character passes.
                                self.combatants()
                                    .iter()
                                    .find(|c| c.handle == mi.body.owner)
                                    .is_none_or(|c| c.faction & 1 != 0)
                            })
                            .find(|mi| {
                                m::length(m::sub(mi.body.position, bolt.position))
                                    <= mi.body.radius + BOLT_RADIUS
                            })
                            .and_then(|mi| mi.handle)
                            .map(BoltHit::Blocker)
                    });
            }
            match hit {
                Some(BoltHit::Character { handle, discharge }) => {
                    if discharge {
                        self.otherworldly_discharge(handle, bolt.caster);
                    }
                    self.destroy_bolt(bolt, discharge, false, Some(handle), false);
                }
                Some(BoltHit::Blocker(handle)) => {
                    self.destroy_bolt(bolt, false, true, Some(handle), false)
                }
                None => {}
            }
        }
    }
}

fn translation(p: V3) -> magicka_content::skinned::Mat {
    let mut t = magicka_content::skinned::IDENTITY;
    m::set_translation(&mut t, p);
    t
}
