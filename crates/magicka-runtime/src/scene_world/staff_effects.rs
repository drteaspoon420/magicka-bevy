//! Staff special abilities with SpellManager lifetimes or several targets
//! (research/staff-items.md). Each IAbilityEffect updates once per frame and
//! runs its OnRemove when IsDead.
use super::*;
use magicka_sim::damage::{attack, Damage};
use magicka_sim::spells::Elements as E;

#[derive(Clone, Debug, PartialEq)]
pub enum StaffEffectKind {
    /// The drained target's start position and the amount returned.
    DrainLife {
        from: [f32; 3],
        amount: f32,
        effect: u64,
    },
    ChillyBlast {
        hit: Vec<u16>,
        effect: u64,
        cue: u32,
    },
    PoisonSpray {
        effect: u64,
        cue: u32,
    },
    /// One ConfuseWho victim (charmed, avatars inverted).
    ConfuseWho {
        victim: u16,
        effect: u64,
    },
    DrinkBlood {
        found: bool,
    },
    // NPC abilities (npc_abilities.rs).
    BreakBarriers,
    Charge(super::npc_abilities::ChargeTrack),
    FloorStomp {
        stomps: [f32; 3],
    },
    GreaseLump {
        missile: u32,
        interval: f32,
        /// The lump's last position while it flies.
        last: Option<[f32; 3]>,
    },
    ArrowRain(super::npc_abilities::ArrowRainState),
    Starfall {
        cast_delay: f32,
        damage_delay: f32,
        position: [f32; 3],
    },
    HealingRain {
        wet_timer: f32,
        effect: u64,
        cue: u32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct StaffEffect {
    pub owner: u16,
    pub ttl: f32,
    pub timestamp: f64,
    pub kind: StaffEffectKind,
}

/// Rotate `v` by `angle` about +Y (Quaternion.CreateFromYawPitchRoll yaw).
fn yaw(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 {
        v.map(|x| x / l)
    } else {
        v
    }
}

impl<C: SceneContent> SceneWorld<C> {
    pub(super) fn body_of(&self, handle: u16) -> Option<(usize, [f32; 3], [f32; 3])> {
        let body = self.character_body(handle)?;
        let b = &self.physics.bodies[body];
        Some((body, b.position, b.direction()))
    }

    /// Character.Damage(float, Elements): the direct scalar route.
    pub(super) fn direct_damage(&mut self, handle: u16, amount: f32, element: E) {
        let Some(body) = self.character_body(handle) else {
            return;
        };
        let avatar = self.players.iter().flatten().any(|p| p.handle == handle);
        let network_client = self.role == NetworkRole::Client;
        let entangled = self.physics.bodies[body].owner.entangled;
        if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, handle) {
            let context = magicka_sim::character_health::HealthContext {
                immortal: c.is_immortal(),
                network_client,
                avatar,
                entangled,
                ..Default::default()
            };
            let mut events = Vec::new();
            c.combat.health.damage(
                amount,
                element,
                c.combat.statuses.current_mask,
                context,
                &mut events,
            );
            self.physics.bodies[body].owner.hit_points = c.combat.health.hit_points;
        }
    }

    /// LightningBolt.Cast from `source` along `direction` for the
    /// thunderbolt-style specials (Zap, Doom_Zap): one damage slot, twice
    /// the lightning colour, scale 2.
    pub(super) fn special_bolt(
        &mut self,
        owner: u16,
        source: [f32; 3],
        direction: [f32; 3],
        hit_owner: bool,
        amount: f32,
        range: f32,
    ) {
        let mut hits = magicka_sim::spray_pulse::HitList::default();
        if hit_owner {
            hits.add(owner);
        }
        let mut damages = [Damage::default(); 5];
        damages[0] = Damage {
            attack: attack::DAMAGE,
            element: E::LIGHTNING.0,
            amount,
            magnitude: 1.0,
        };
        let color = LIGHTNING_COLOR.map(|c| c * 2.0);
        let play_time = self.play_time;
        let mut bolts = Vec::new();
        let mut rng = self.rng;
        self.with_rail_world(|world| {
            let mut host = BarrierLightning {
                world,
                bolts: &mut bolts,
                rng: &mut rng,
            };
            magicka_sim::lightning::cast_bolt(
                &mut host, owner, play_time, source, direction, &mut hits, color, 2.0, range,
                damages,
            );
        });
        self.rng = rng;
        self.barrier_visuals.extend(bolts);
    }

    /// Returns false when `name` is not handled here.
    pub(super) fn execute_staff_effect_special(&mut self, owner: u16, name: &str) -> bool {
        let Some((body, position, direction)) = self.body_of(owner) else {
            return true;
        };
        let authority = self.role != NetworkRole::Client;
        let source = self.cast_source_of(owner).unwrap_or(position);
        match name {
            "selfconfuse" => {
                self.confuse_character(position, owner);
            }
            "charmmetal" => {
                let found = self.facing_target(owner, 14.0, |w, t, c| {
                    !w.character_ref(t.handle).is_some_and(|r| r.is_charmed())
                        && t.faction & c.faction == 0
                });
                if let Some((_, Some((target, _)))) = found {
                    self.charm_character(target, owner, 15.0);
                }
            }
            "randomcharm" => {
                // INFERRED: native draws among every entity in range (props
                // included, 50 tries); the rewrite draws among characters.
                let candidates: Vec<(u16, [f32; 3])> = self
                    .combatants()
                    .into_iter()
                    .filter(|t| {
                        let d: f32 = (0..3).map(|i| (t.position[i] - position[i]).powi(2)).sum();
                        t.handle != owner
                            && !t.avatar
                            && d.sqrt() - t.radius <= 15.0
                            && !self.character_ref(t.handle).is_some_and(|r| r.is_charmed())
                    })
                    .map(|t| (t.handle, t.position))
                    .collect();
                if candidates.is_empty() {
                    self.play_cue(16, "magick_fail", Some(position));
                } else {
                    let pick = ((self.random() * candidates.len() as f32) as usize)
                        .min(candidates.len() - 1);
                    let (target, at) = candidates[pick];
                    self.play_cue(16, "magick_charm", Some(at));
                    self.charm_character(target, owner, 3.0);
                }
            }
            "randomteleportother" => {
                let found = self.facing_target(owner, 14.0, |_, _, _| true);
                let Some((_, Some((target, at)))) = found else {
                    self.play_cue(16, "magick_fail", Some(position));
                    return true;
                };
                let radius = f64::from(self.random()).powf(0.25) as f32;
                let angle = self.random() * std::f32::consts::TAU;
                let offset = [radius * angle.cos(), 0.0, radius * angle.sin()];
                let destination = [at[0] + 20.0 * offset[0], at[1], at[2] + 20.0 * offset[2]];
                let facing = normalize([destination[0] - at[0], 0.0, destination[2] - at[2]]);
                self.start_named_effect("horrible_staff", source, [1.0, 0.0, 0.0]);
                if authority {
                    self.teleport_character(target, destination, facing);
                }
            }
            "drainlife" => {
                // The nearest character within 8 m (+ its radius, Y ignored).
                let target = self
                    .combatants()
                    .into_iter()
                    .filter(|t| t.position != position)
                    .filter_map(|t| {
                        let d = (t.position[0] - position[0]).powi(2)
                            + (t.position[2] - position[2]).powi(2)
                            + (t.position[1] - position[1]).powi(2);
                        (d <= (8.0 + t.radius).powi(2)).then_some((d, t))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, t)| t);
                let Some(t) = target else {
                    return true;
                };
                let me = self.character_ref(owner).map(|c| &c.combat.health);
                let missing = me.map_or(0.0, |h| h.max_hit_points - h.hit_points);
                let amount = missing.min(t.hit_points);
                self.direct_damage(t.handle, amount, E::ARCANE);
                let effect = self.start_named_effect(
                    "drainlife",
                    t.position,
                    normalize(std::array::from_fn(|i| position[i] - t.position[i])),
                );
                self.staff_effects.push(StaffEffect {
                    owner,
                    ttl: 1.0,
                    timestamp: self.play_time,
                    kind: StaffEffectKind::DrainLife {
                        from: t.position,
                        amount,
                        effect,
                    },
                });
            }
            "doom_zap" => {
                let left = yaw(direction, 30f32.to_radians());
                self.special_bolt(owner, source, left, true, 300.0, 10.0);
                self.play_cue(16, "magick_thunderbolt", Some(position));
                self.camera.shake(1.5, 0.333);
                let glove = self
                    .players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == owner)
                    .and_then(|p| {
                        let w = p.items.iter().find(|(slot, _)| *slot == 0)?;
                        w.1.definition
                            .name
                            .eq_ignore_ascii_case("weapon_doomgauntlet")
                            .then(|| p.spray_source(&self.physics.bodies[body]).weapon_position)
                    });
                if let Some(at) = glove {
                    let right = yaw(direction, -30f32.to_radians());
                    self.camera.shake(1.5, 0.333);
                    self.play_cue(16, "magick_thunderbolt", Some(position));
                    self.special_bolt(owner, at, right, true, 300.0, 10.0);
                }
            }
            "chillyblast" => {
                let cue = self.play_cue(16, "spell_poison_spray", Some(position));
                let effect = self.start_named_effect("spray_cold3", source, direction);
                self.staff_effects.push(StaffEffect {
                    owner,
                    ttl: 1.0,
                    timestamp: self.play_time,
                    kind: StaffEffectKind::ChillyBlast {
                        hit: Vec::new(),
                        effect,
                        cue,
                    },
                });
            }
            "poisonspray" => {
                let cue = self.play_cue(16, "spell_poison_spray", Some(position));
                let effect = self.start_named_effect("scythe_spray", source, direction);
                self.staff_effects.push(StaffEffect {
                    owner,
                    ttl: 0.5,
                    timestamp: self.play_time,
                    kind: StaffEffectKind::PoisonSpray { effect, cue },
                });
            }
            "tractorpush" => self.tractor(Some(owner), position, true),
            "summonbug" => {
                let kind = magicka_sim::summons::SummonKind::Normal;
                self.summon_ahead(owner, "bugswarm", position, direction, (0.0, 1.0), kind);
            }
            "summonflamer" => {
                let kind = magicka_sim::summons::SummonKind::Flamer;
                self.summon_ahead(
                    owner,
                    "conscious_blaze",
                    position,
                    direction,
                    (3.0, 0.0),
                    kind,
                );
            }
            "fearlight" => {
                self.start_named_effect("gandalf_fear", position, [1.0, 0.0, 0.0]);
                if authority {
                    let faction = self
                        .combatants()
                        .into_iter()
                        .find(|c| c.handle == owner)
                        .map_or(0, |c| c.faction);
                    let targets: Vec<u16> = self
                        .combatants()
                        .into_iter()
                        .filter(|t| {
                            let d: f32 =
                                (0..3).map(|i| (t.position[i] - position[i]).powi(2)).sum();
                            t.handle != owner
                                && t.faction & faction == 0
                                && d.sqrt() - t.radius <= 10.0
                        })
                        .map(|t| t.handle)
                        .collect();
                    for t in targets {
                        self.fear_character(t, magicka_sim::fear::FearSource::Character(owner));
                    }
                }
                self.play_cue(16, "spell_arcane_ray_stage4", Some(position));
            }
            "grenade" | "holygrenade" | "fireflask" => {
                let mut velocity = [direction[0], 1.0, direction[2]];
                velocity = normalize(velocity).map(|v| v * 13.0);
                self.spawn_grenade(owner, name, source, velocity);
            }
            "timewarpstaff" => {
                if self
                    .ability_effects
                    .time_warp(Some(owner), self.scene_saturation)
                {
                    self.start_named_effect("timewarp_effect", source, direction);
                    self.time_warp_cue = Some(self.play_cue(16, "magick_timewarp", None));
                }
            }
            "confusewho" => self.confuse_who(owner, position, direction, 5.0),
            "drinkblood" => {
                self.ability_effects.haste(owner, Some(0.78));
                self.staff_effects.push(StaffEffect {
                    owner,
                    ttl: 0.78,
                    timestamp: self.play_time,
                    kind: StaffEffectKind::DrinkBlood { found: false },
                });
            }
            _ => return false,
        }
        true
    }

    /// Grenade/HolyGrenade/Fireflask.SpawnGrenade: a thrown model missile
    /// (radius 0.75 × its first mesh's bounding sphere, cannot hit its
    /// owner) exploding when damaged by Fire/Lightning/Arcane (20+), after
    /// 3 s or (Fireflask) on a 20+ collision; a trail effect follows it.
    fn spawn_grenade(&mut self, owner: u16, kind: &str, position: [f32; 3], velocity: [f32; 3]) {
        let (model, conditions) = match kind {
            "fireflask" => (
                "Models/Items_Wizard/weapon_fireflask",
                magicka_sim::missile_events::grenade_conditions(
                    magicka_sim::missile_events::GrenadeKind::Fireflask,
                ),
            ),
            "holygrenade" => (
                "Models/Missiles/holy_handgrenade",
                magicka_sim::missile_events::grenade_conditions(
                    magicka_sim::missile_events::GrenadeKind::Holy,
                ),
            ),
            _ => (
                "Models/Missiles/liberty_grenade",
                magicka_sim::missile_events::grenade_conditions(
                    magicka_sim::missile_events::GrenadeKind::Liberty,
                ),
            ),
        };
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
    }

    /// ConfuseWho.Execute: NPC owners use their current target, others the
    /// nearest character in a 5 m, 45° arc (4 m height); the target is
    /// charmed for 10 s and (avatar) its controls inverted.
    /// StarGaze is the same class with a 20 m range.
    pub(super) fn confuse_who(
        &mut self,
        owner: u16,
        position: [f32; 3],
        direction: [f32; 3],
        range: f32,
    ) {
        let cast_direction = if direction[2] > 0.0 {
            direction.map(|v| -v)
        } else {
            direction
        };
        self.start_named_effect("elderthing_stargaze", position, cast_direction);
        let mut target = self
            .npcs
            .get(&owner)
            .and_then(|n| n.combat.current_target())
            .filter(|t| self.character_body(*t).is_some());
        if target.is_none() {
            let query = magicka_sim::arc_intersection::ArcQuery {
                origin: position,
                direction,
                range,
                angle: std::f32::consts::FRAC_PI_4,
                height_difference: 4.0,
            };
            let mut best: Option<(f32, u16)> = None;
            for t in self.combatants() {
                if t.handle == owner {
                    continue;
                }
                let Some(c) = self.character_ref(t.handle) else {
                    continue;
                };
                if let Some(at) = c.arc_intersect(t.position, query) {
                    let d: f32 = (0..3).map(|i| (at[i] - position[i]).powi(2)).sum();
                    if best.is_none_or(|(b, _)| d < b) {
                        best = Some((d, t.handle));
                    }
                }
            }
            target = best.map(|(_, h)| h);
        }
        let Some(target) = target else {
            return;
        };
        let Some((_, at, facing)) = self.body_of(target) else {
            return;
        };
        self.start_named_effect("elderthing_stargaze_hit", at, facing);
        self.charm_character(target, owner, 10.0);
        if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == target)
        {
            p.controls_inverted = true;
        }
        let existing = self.staff_effects.iter_mut().find(
            |e| matches!(e.kind, StaffEffectKind::ConfuseWho { victim, .. } if victim == target),
        );
        if let Some(e) = existing {
            e.ttl = 10.0;
        } else {
            let effect = self.start_named_effect("confused", at, facing);
            self.staff_effects.push(StaffEffect {
                owner,
                ttl: 10.0,
                timestamp: self.play_time,
                kind: StaffEffectKind::ConfuseWho {
                    victim: target,
                    effect,
                },
            });
        }
    }

    /// SpellManager's update of the staff effects, then OnRemove.
    pub(super) fn update_staff_effects(&mut self, dt: f32) {
        let mut effects = std::mem::take(&mut self.staff_effects);
        for e in &mut effects {
            if !self.update_npc_ability(e, dt) {
                self.update_staff_effect(e, dt);
            }
        }
        let (dead, live): (Vec<_>, Vec<_>) = effects.into_iter().partition(|e| e.ttl <= 0.0);
        // Effects added during the updates follow the surviving ones.
        let added = std::mem::replace(&mut self.staff_effects, live);
        self.staff_effects.extend(added);
        for e in dead {
            self.remove_staff_effect(e);
        }
    }

    fn update_staff_effect(&mut self, e: &mut StaffEffect, dt: f32) {
        let owner = e.owner;
        let Some((body, position, direction)) = self.body_of(owner) else {
            e.ttl = 0.0;
            return;
        };
        let authority = self.role != NetworkRole::Client;
        match &mut e.kind {
            StaffEffectKind::DrainLife { from, effect, .. } => {
                e.ttl -= dt;
                let t = 1.0 - e.ttl;
                let at: [f32; 3] = std::array::from_fn(|i| from[i] + (position[i] - from[i]) * t);
                let facing = normalize(std::array::from_fn(|i| position[i] - from[i]));
                self.effect_commands.push(SceneEffectCommand::Move {
                    instance: *effect,
                    position: at,
                    direction: facing,
                });
            }
            StaffEffectKind::ChillyBlast { hit, effect, .. } => {
                e.ttl -= dt;
                self.physics.bodies[body].owner.turn_speed = 0.0;
                if authority {
                    let query = magicka_sim::arc_intersection::ArcQuery {
                        origin: position,
                        direction,
                        range: 6.0,
                        angle: 0.436_332_32,
                        height_difference: 5.0,
                    };
                    for t in self.combatants() {
                        if t.handle == owner {
                            continue;
                        }
                        let d: f32 = (0..3).map(|i| (t.position[i] - position[i]).powi(2)).sum();
                        if d.sqrt() - t.radius > 6.0 {
                            continue;
                        }
                        let Some(c) = self.character_ref(t.handle) else {
                            continue;
                        };
                        let Some(at) = c.arc_intersect(t.position, query) else {
                            continue;
                        };
                        use magicka_sim::character_health::SelfShieldType as T;
                        let shield = &c.combat.health.shield;
                        let iced = shield.active() && matches!(shield.kind, T::Ice | T::IcedEarth);
                        if !hit.contains(&t.handle) {
                            let damage = Damage {
                                attack: attack::PUSHED,
                                element: E::COLD.0,
                                amount: 75.0,
                                magnitude: 2.0,
                            };
                            self.hit_character_at(t.handle, damage, Some(owner), at, e.timestamp);
                            hit.push(t.handle);
                        }
                        if !iced {
                            self.add_status_to(
                                t.handle,
                                magicka_sim::status::StatusEffect::new(
                                    magicka_sim::status::Status::Frozen,
                                    75.0,
                                    2.0,
                                    false,
                                ),
                            );
                        }
                    }
                }
                let source = self.cast_source_of(owner).unwrap_or(position);
                self.effect_commands.push(SceneEffectCommand::Move {
                    instance: *effect,
                    position: source,
                    direction,
                });
            }
            StaffEffectKind::PoisonSpray { effect, .. } => {
                e.ttl -= dt;
                let angle = -(e.ttl / 0.5 * 6.0 - 2.5) / 2.5 * std::f32::consts::FRAC_PI_4;
                let facing = yaw(direction, angle);
                if authority {
                    let range = self.clipped_reach(position, facing, 6.0);
                    let query = magicka_sim::arc_intersection::ArcQuery {
                        origin: position,
                        direction: facing,
                        range,
                        angle: 10f32.to_radians(),
                        height_difference: 5.0,
                    };
                    for t in self.combatants() {
                        if t.handle == owner {
                            continue;
                        }
                        let d: f32 = (0..3).map(|i| (t.position[i] - position[i]).powi(2)).sum();
                        if d.sqrt() - t.radius > range {
                            continue;
                        }
                        let Some(c) = self.character_ref(t.handle) else {
                            continue;
                        };
                        if c.combat.statuses.has(magicka_sim::status::Status::Greased) {
                            continue;
                        }
                        if let Some(at) = c.arc_intersect(t.position, query) {
                            let damage = Damage {
                                attack: attack::STATUS,
                                element: E::POISON.0,
                                amount: 75.0,
                                magnitude: 1.0,
                            };
                            self.hit_character_at(t.handle, damage, Some(owner), at, e.timestamp);
                        }
                    }
                }
                let source = self.cast_source_of(owner).unwrap_or(position);
                self.effect_commands.push(SceneEffectCommand::Move {
                    instance: *effect,
                    position: source,
                    direction: facing,
                });
            }
            StaffEffectKind::ConfuseWho { victim, effect } => {
                e.ttl -= dt;
                let victim = *victim;
                let dead = self.character_ref(victim).is_none_or(|c| c.combat.dead);
                if dead {
                    e.ttl = 0.0;
                } else if let Some((_, at, facing)) = self.body_of(victim) {
                    self.effect_commands.push(SceneEffectCommand::Move {
                        instance: *effect,
                        position: at,
                        direction: facing,
                    });
                }
            }
            StaffEffectKind::DrinkBlood { found } => {
                e.ttl -= dt;
                if *found {
                    return;
                }
                let me = self.combatants().into_iter().find(|c| c.handle == owner);
                let Some(me) = me else {
                    e.ttl = 0.0;
                    return;
                };
                if me.hit_points <= 0.0 {
                    e.ttl = 0.0;
                    return;
                }
                let nearest = self
                    .combatants()
                    .into_iter()
                    .filter(|t| t.handle != owner)
                    .filter_map(|t| {
                        let d: f32 = (0..3)
                            .map(|i| (t.position[i] - position[i]).powi(2))
                            .sum::<f32>()
                            .sqrt();
                        (d - t.radius <= 8.0).then_some((d, t))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                let Some((distance, t)) = nearest else {
                    return;
                };
                if distance >= me.radius + t.radius + 1.0 {
                    return;
                }
                let away = normalize(std::array::from_fn(|i| position[i] - t.position[i]));
                let dot: f32 = (0..3).map(|i| away[i] * direction[i]).sum();
                let angle = dot.clamp(-1.0, 1.0).acos();
                if !(angle < std::f32::consts::PI && angle > std::f32::consts::PI * 0.75) {
                    return;
                }
                let amount = (me.max_hit_points - me.hit_points).min(t.hit_points);
                if amount > 0.0 {
                    *found = true;
                    self.go_to_animation_of(owner, "special4", 0.1);
                    self.direct_damage(t.handle, amount, E::ARCANE);
                    e.ttl = 0.83;
                    self.direct_damage(owner, -amount, E::LIFE);
                } else {
                    self.go_to_animation_of(owner, "idle", 0.05);
                    e.ttl = 0.0;
                }
                if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, t.handle) {
                    c.combat.stun_timer = 1.5;
                }
            }
            // NPC abilities update in npc_abilities.rs.
            _ => {}
        }
    }

    fn remove_staff_effect(&mut self, e: StaffEffect) {
        match e.kind {
            StaffEffectKind::DrainLife { amount, effect, .. } => {
                self.direct_damage(e.owner, -amount, E::LIFE);
                self.effect_commands
                    .push(SceneEffectCommand::Stop { instance: effect });
            }
            StaffEffectKind::ChillyBlast { effect, cue, .. }
            | StaffEffectKind::PoisonSpray { effect, cue } => {
                self.effect_commands
                    .push(SceneEffectCommand::Stop { instance: effect });
                self.stop_cue(cue);
            }
            StaffEffectKind::ConfuseWho { victim, effect } => {
                self.effect_commands
                    .push(SceneEffectCommand::Stop { instance: effect });
                if let Some(n) = self.npcs.get_mut(&victim) {
                    // NonPlayerCharacter.Confuse(Template.Faction).
                    n.faction_override = None;
                } else if let Some(p) = self
                    .players
                    .iter_mut()
                    .flatten()
                    .find(|p| p.handle == victim)
                {
                    p.controls_inverted = false;
                }
            }
            StaffEffectKind::DrinkBlood { .. } => {}
            kind => self.remove_npc_ability(&kind),
        }
    }

    pub(super) fn go_to_animation_of(&mut self, handle: u16, animation: &str, time: f32) {
        let Some(value) = crate::animation_controller::animation_value(animation) else {
            return;
        };
        if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == handle)
        {
            p.go_to_animation(value, time);
        } else if let Some(n) = self.npcs.get_mut(&handle) {
            n.motion.go_to_animation(value, time);
        }
    }

    fn add_status_to(&mut self, handle: u16, effect: magicka_sim::status::StatusEffect) {
        let mut events = Vec::new();
        if let Some(p) = self
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == handle)
        {
            let _ = p.add_status_effect(effect, &mut events);
        } else if let Some(n) = self.npcs.get_mut(&handle) {
            let _ = n.character.add_status_effect(effect, None, &mut events);
        }
    }
}
