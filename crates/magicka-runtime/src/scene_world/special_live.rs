//! NPC item/animation special ability execution. Unsupported classes stay explicit.
use super::*;
use crate::item_special::SpecialInvocation;
use crate::npc::{ORDER_ATTACK, REACT_ATTACK, REACT_PROXIMITY};

impl<C: SceneContent> SceneWorld<C> {
    fn unsupported_special(&mut self, name: &str) {
        if self.unsupported_specials.insert(name.to_owned()) {
            self.log
                .push(format!("special ability not reconstructed: {name}"));
        }
    }
    pub(super) fn apply_npc_specials(&mut self) {
        let casts: Vec<_> = self
            .npcs
            .iter_mut()
            .flat_map(|(owner, n)| {
                std::mem::take(&mut n.special_requests)
                    .into_iter()
                    .map(|a| (*owner, a))
            })
            .collect();
        for (owner, cast) in casts {
            self.execute_special(owner, cast);
        }
    }

    pub(super) fn execute_special(&mut self, owner: u16, cast: SpecialInvocation) {
        use magicka_sim::spells::MagickType as M;
        let name = cast.ability.type_name.to_ascii_lowercase();
        let magick = match name.as_str() {
            "fear" => Some(M::Fear),
            "nullify" => Some(M::Nullify),
            "teleport" => Some(M::Teleport),
            "etherealclone" => Some(M::EtherealClone),
            "summoncross" => Some(M::SummonCross),
            "vladzap" => Some(M::VladZap),
            _ => None,
        };
        if let Some(magick) = magick.or_else(|| Self::staff_magick(&name)) {
            self.cast_magick_from(owner, magick);
            return;
        }
        if self.execute_staff_special(owner, &name)
            || self.execute_staff_effect_special(owner, &name)
        {
            return;
        }
        let Some(body) = self.character_body(owner) else {
            return;
        };
        let position = self.physics.bodies[body].position;
        let direction = self.physics.bodies[body].direction();
        match name.as_str() {
            "regenerate" => {
                // Regenerate.Execute 0x06002f45 calls scalar Damage(-13000,
                // Life), which bypasses attack boosts and resistance delivery.
                let avatar = self.players.iter().flatten().any(|p| p.handle == owner);
                let network_client = self.role == magicka_sim::damage_delivery::NetworkRole::Client;
                let entangled = self.physics.bodies[body].owner.entangled;
                if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, owner) {
                    let context = magicka_sim::character_health::HealthContext {
                        immortal: c.is_immortal(),
                        network_client,
                        avatar,
                        entangled,
                        ..Default::default()
                    };
                    let mut events = Vec::new();
                    c.combat.health.damage(
                        -13000.,
                        magicka_sim::spells::Elements::LIFE,
                        c.combat.statuses.current_mask,
                        context,
                        &mut events,
                    );
                    self.physics.bodies[body].owner.hit_points = c.combat.health.hit_points;
                }
            }
            "spawnslime" => {
                let elements = cast
                    .ability
                    .elements
                    .iter()
                    .fold(0u32, |mask, e| mask | *e as u32);
                let config = super::slime_live::SlimeCast {
                    elements: magicka_sim::spells::Elements(elements),
                    ..Default::default()
                };
                if let Err(error) = self.spawn_slimes(owner, config) {
                    self.log.push(format!("NPC spawn slime: {error}"));
                }
            }
            "specialanimation" => {} // Execute is true; the item already queued its animation.
            "epicbuff" => {
                let Some(id) = cast.buff_id else {
                    self.unsupported_special("direct EpicBuff identity");
                    return;
                };
                let cast_position = self
                    .npcs
                    .get(&owner)
                    .map(|n| n.spray_source(&self.physics.bodies[n.body]).cast_position)
                    .unwrap_or(position);
                self.start_named_effect("epic_buff", cast_position, [1., 0., 0.]);
                self.play_cue(16, "magick_revive_cast", Some(position));
                if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, owner) {
                    c.nonvisual_auras.push(crate::buffs::NonvisualAura {
                        id,
                        ttl: 8.,
                        period: 0.,
                        kind: crate::buffs::NonvisualKind::BoostDamage(
                            magicka_sim::damage::Damage {
                                attack: magicka_sim::damage::attack::DAMAGE,
                                element: 0xffff,
                                amount: 1.,
                                magnitude: 1.1,
                            },
                        ),
                    });
                }
            }
            "randomteleport" => {
                if self.role == magicka_sim::damage_delivery::NetworkRole::Client {
                    return;
                }
                let radius = f64::from(self.random()).powf(0.25) as f32;
                let angle = self.random() * std::f32::consts::TAU;
                let offset = [
                    (f64::from(radius) * f64::from(angle).cos()) as f32,
                    0.,
                    (f64::from(radius) * f64::from(angle).sin()) as f32,
                ];
                let destination = std::array::from_fn(|i| position[i] + 20. * offset[i]);
                let length = (offset[0] * offset[0] + offset[2] * offset[2]).sqrt();
                self.teleport_character(owner, destination, offset.map(|v| v / length));
            }
            "otherworldlydischarge" => self.otherworldly_discharge_from(owner),
            "smokebomb" => {
                if self.role == magicka_sim::damage_delivery::NetworkRole::Client {
                    return;
                }
                let angle = (self.random() - 0.5) * std::f32::consts::FRAC_PI_4;
                let sin = f64::from(angle).sin() as f32;
                let cos = f64::from(angle).cos() as f32;
                let facing = [
                    direction[0] * cos + direction[2] * sin,
                    direction[1],
                    -direction[0] * sin + direction[2] * cos,
                ];
                let destination = std::array::from_fn(|i| position[i] - 10. * facing[i]);
                self.teleport_character_kind(owner, destination, facing, true);
            }
            "summonspirit" => {
                self.summon_spirit(owner, position, direction);
            }
            _ => {
                let elements = cast.ability.elements.clone();
                if !self.execute_npc_ability(owner, &name, &elements) {
                    self.unsupported_special(&cast.ability.type_name);
                }
            }
        }
    }

    fn summon_spirit(&mut self, owner: u16, position: [f32; 3], direction: [f32; 3]) {
        let kind = magicka_sim::summons::SummonKind::Normal;
        self.summon_ahead(owner, "treespirit", position, direction, (3., 0.), kind);
    }

    /// SummonSpirit/SummonBug/SummonFlamer.Execute (host only): 4 m ahead,
    /// raised by `lift.0` before the nearest navmesh point and by `lift.1`
    /// after it, dropped onto the scene within 10 m, then the summoned NPC
    /// attacks (alert radius 12) after its spawn resurrection.
    pub(super) fn summon_ahead(
        &mut self,
        owner: u16,
        template: &str,
        position: [f32; 3],
        direction: [f32; 3],
        lift: (f32, f32),
        kind: magicka_sim::summons::SummonKind,
    ) {
        if self.role == magicka_sim::damage_delivery::NetworkRole::Client {
            return;
        }
        let mut point: [f32; 3] = std::array::from_fn(|i| position[i] + direction[i] * 4.);
        point[1] += lift.0;
        if let Some((_, nearest, _)) = self
            .nav
            .as_ref()
            .and_then(|nav| nav.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
        {
            point = nearest;
        }
        point[1] += lift.1;
        if let Some(hit) = self.collision.collision().segment_intersect(
            Segment {
                origin: point,
                delta: [0., -10., 0.],
            },
            false,
        ) {
            point = hit.hit.position;
        }
        let events = [];
        let Some(handle) = self.summon_npc(
            Some(owner),
            template,
            point,
            direction,
            kind,
            crate::spawn_action::SpawnOrder {
                order: ORDER_ATTACK,
                react_to: REACT_ATTACK | REACT_PROXIMITY,
                reaction: ORDER_ATTACK,
                priority_target: 0,
                priority_ability: 0,
                trigger: 0,
                events: &events,
            },
        ) else {
            return;
        };
        if let Some(n) = self.npcs.get_mut(&handle) {
            n.weights.alert_radius = 12.;
            let facts = n.motion_facts(&self.physics.bodies[n.body], self.flags.cutscene, 0.);
            n.motion.begin_resurrection(
                &mut self.physics.bodies[n.body],
                &facts,
                170,
                &mut crate::character_motion::NoCasting {
                    died: &mut false,
                    entered_dead: &mut false,
                },
            );
        }
    }
}
