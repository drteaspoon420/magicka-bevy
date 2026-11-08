//! The Niflheim boss host; separate from SummonDeath's death_live singleton.
use super::*;
use crate::boss_death::{Avatar, Command, Death, Environment};
use std::sync::{Arc, Mutex};

const SKELETONS: [&str; 4] = [
    "skeleton_darksoul_arcane",
    "skeleton_darksoul_frost",
    "skeleton_darksoul_lightning",
    "skeleton_darksoul_poison",
];

#[derive(Clone, Debug)]
pub struct DeathBossHost {
    pub actor: Arc<Mutex<Death>>,
    pub zone: u16,
    pub skeletons: Vec<u16>,
    pub scythe_bounds: crate::pickups::PickupBounds,
    effects: BTreeMap<&'static str, i32>,
    swing: Option<u32>,
    number: i32,
    clone_target: Option<u16>,
    rng: u64,
    previous_scythe: [f32; 16],
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn player_melee_hits_death_capsule_with_native_range_and_ethereal_rules() {
        use crate::npc_combat::{MeleeRequest, MeleeWeapon};
        use magicka_content::common::{Damage, Event, EventCollection, EventCondition};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../target/Magicka/Content");
        if !root.is_dir() {
            return;
        }
        let mut s =
            crate::play_session::PlaySession::load(&root, "Niflheim.lvl", "Wizard").unwrap();
        s.change_scene_named("nh_s3BF", "start").unwrap();
        s.step(1. / 60.);
        s.run_trigger("Move to the throne", true).unwrap();
        s.step(1. / 60.);
        let (zone, actor) = s
            .world
            .boss_fight
            .bosses
            .iter()
            .find_map(|b| {
                if let super::super::boss_live::BossRef::Death(h) = b {
                    Some((h.zone, h.actor.clone()))
                } else {
                    None
                }
            })
            .unwrap();
        let position = s.world.entity(zone).unwrap().position;
        {
            let mut d = actor.lock().unwrap();
            d.drawn = true;
            d.ethereal = false;
            d.body_position = position;
        }
        let conditions = Arc::new(vec![EventCollection {
            condition: EventCondition {
                condition_type: 2,
                hitpoints: 0,
                elements: 0,
                threshold: 0.,
                time: 0.,
            },
            repeat: false,
            events: vec![Event::Damage {
                damage: Damage {
                    attack_property: magicka_sim::damage::attack::DAMAGE as i32,
                    element: magicka_sim::spells::Elements::LIFE.0 as i32,
                    amount: -125.,
                    magnitude: 0.,
                },
                velocity_based: false,
            }],
        }]);
        // Both caps and the cylinder are hittable. Target bit 4 is required;
        // even a large item range must not inflate BossDamageZone's skin.
        for (x, y, targets, ethereal, hit) in [
            (0., 0.6, 4, false, true),
            (0., 1.5, 4, false, true),
            (0., -0.3, 4, false, true),
            (1., 0.6, 4, false, false),
            (0., 2., 4, false, false),
            (0., 0.6, 3, false, false),
            (0., 0.6, 4, true, false),
        ] {
            actor.lock().unwrap().ethereal = ethereal;
            let before = actor.lock().unwrap().combat.hit_points();
            let player = s.world.players[0].as_mut().unwrap();
            let (slot, item) = player.items.first_mut().expect("Wizard held item");
            item.melee = MeleeWeapon::new(5., true, conditions.clone());
            item.melee.attach = [position[0] + x, position[1] + y, position[2] - 2.];
            item.melee.last_attach = [position[0] + x, position[1] + y, position[2] + 2.];
            let slot = *slot;
            player.melee_requests.push(MeleeRequest::Execute {
                weapon: slot,
                targets,
                first: true,
            });
            s.world.execute_melee();
            let item = &s.world.players[0].as_ref().unwrap().items[0].1;
            assert_eq!(
                item.melee.hitlist.contains(&zone),
                hit,
                "x={x} y={y} targets={targets} ethereal={ethereal}"
            );
            assert_eq!(
                actor.lock().unwrap().combat.hit_points(),
                before - if hit { 125. } else { 0. }
            );
            // Continued Execute calls must retain Item's per-swing hit list.
            s.world.players[0]
                .as_mut()
                .unwrap()
                .melee_requests
                .push(MeleeRequest::Execute {
                    weapon: slot,
                    targets,
                    first: false,
                });
            s.world.execute_melee();
            assert_eq!(
                actor.lock().unwrap().combat.hit_points(),
                before - if hit { 125. } else { 0. }
            );
        }
        assert!(s.world.physics_damage[&zone].collision_box.is_none());
    }

    #[test]
    fn live_scythe_contacts_respect_immortality_and_synchronous_life_interrupt() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../target/Magicka/Content");
        if !root.is_dir() {
            return;
        }
        let mut s =
            crate::play_session::PlaySession::load(&root, "Niflheim.lvl", "Wizard").unwrap();
        s.change_scene_named("nh_s3BF", "start").unwrap();
        s.step(1. / 60.);
        s.run_trigger("Move to the throne", true).unwrap();
        s.step(1. / 60.);
        let mut host = s
            .world
            .boss_fight
            .bosses
            .iter()
            .find_map(|b| {
                if let super::super::boss_live::BossRef::Death(h) = b {
                    Some(h.as_ref().clone())
                } else {
                    None
                }
            })
            .unwrap();
        let bounds = host.scythe_bounds;
        let local = crate::pickups::PickupBounds {
            min: [bounds.min[0], bounds.max[1] * 0.5, bounds.min[2]],
            max: bounds.max,
        };
        let collision = local.world_box(&magicka_content::skinned::IDENTITY, false);
        {
            let mut d = host.actor.lock().unwrap();
            d.state = crate::boss_death::State::Attack;
            d.update_main = true;
            d.ethereal = false;
            d.drawn = true;
            d.current_slot = 0;
            d.scythes[0] = magicka_content::skinned::IDENTITY;
            d.controller
                .start_named_clip("attack_scythe_fall", 100., false);
        }
        host.previous_scythe = magicka_content::skinned::IDENTITY;
        let mut people = Vec::new();
        for immortal in [false, true, false] {
            let h = s.spawn_npc("Goblin_miner", "idle", 2.).unwrap();
            let n = s.world.npcs.get_mut(&h).unwrap();
            n.character.immortal_time = if immortal { f32::MAX } else { 0. };
            let body = &mut s.world.physics.bodies[n.body];
            let orientation = body.orientation;
            body.move_to(collision.center, orientation);
            people.push(h);
        }
        // Keep the third candidate outside until the interruption control.
        let b = s.world.npcs[&people[2]].body;
        let o = s.world.physics.bodies[b].orientation;
        s.world.physics.bodies[b].move_to([100., 100., 100.], o);
        s.world.death_boss_scythe_contacts(&host);
        assert!(s.world.npcs[&people[0]].character.combat.health.hit_points <= 0.);
        assert!(s.world.npcs[&people[1]].character.combat.health.hit_points > 0.);
        s.world.physics.bodies[b].move_to(collision.center, o);
        let before = s.world.npcs[&people[2]].character.combat.health.hit_points;
        let result = s
            .world
            .damage_physics(
                host.zone,
                magicka_sim::damage::Damage {
                    attack: magicka_sim::damage::attack::DAMAGE,
                    element: magicka_sim::spells::Elements::LIFE.0,
                    amount: 100.,
                    magnitude: 0.,
                },
                0,
            )
            .unwrap();
        assert_eq!(result, magicka_sim::character_impact::result::DEFLECTED);
        s.world.death_boss_scythe_contacts(&host);
        assert_eq!(
            s.world.npcs[&people[2]].character.combat.health.hit_points,
            before
        );
        let radius = collision
            .side_lengths
            .iter()
            .map(|v| v * v)
            .sum::<f32>()
            .sqrt()
            * 0.5;
        let mut p = collision.center;
        p[0] += 1. + radius * 0.5;
        s.world.spawn_shield(
            0,
            p,
            1.,
            [-1., 0., 0.],
            magicka_sim::shield_entity::ShieldKind::Sphere,
            5000.,
            magicka_sim::shield_entity::SHIELD_COLOR,
        );
        s.world.death_boss_scythe_contacts(&host);
        assert_eq!(
            s.world.shields.last().unwrap().shield.hit_points,
            0.,
            "scythe still kills shields while a Life hit prevents character damage"
        );
    }
}
impl PartialEq for DeathBossHost {
    fn eq(&self, other: &Self) -> bool {
        self.zone == other.zone
    }
}

struct WorldEnvironment<'a, C: SceneContent> {
    world: &'a mut SceneWorld<C>,
    count: i32,
}
impl<C: SceneContent> Environment for WorldEnvironment<'_, C> {
    fn nearest(&mut self, p: [f32; 3]) -> Option<(f32, [f32; 3])> {
        self.world
            .nav
            .as_ref()?
            .nearest(p, magicka_sim::navmesh::movement::DEFAULT)
            .map(|(distance, point, _)| (distance, point))
    }
    fn ground(&mut self, p: [f32; 3]) -> Option<[f32; 3]> {
        self.world
            .collision
            .collision()
            .segment_intersect(
                Segment {
                    origin: [p[0], p[1] + 1., p[2]],
                    delta: [0., -4., 0.],
                },
                false,
            )
            .map(|h| h.hit.position)
    }
    fn skeleton_count(&self) -> i32 {
        self.count
    }
}

impl<C: SceneContent> SceneWorld<C> {
    pub(super) fn spawn_death_boss(&mut self, area: i32) -> Result<DeathBossHost, WorldError> {
        let model = self
            .content
            .skinned_model(crate::boss_death::MODEL)
            .ok_or("missing Death model")?;
        let bounds = self
            .content
            .model_bounds(crate::boss_death::SCYTHE)
            .ok_or("missing Death scythe bounds")?;
        for name in SKELETONS {
            SpawnLevel::load_character_template(self, Some(name))?;
        }
        let orientation = self.scene_locator(area)?;
        let actor = Arc::new(Mutex::new(Death::new(
            model,
            orientation,
            0x64_65_61_74_68,
        )?));
        let zone = self.allocate_physics(true);
        let mut damage = crate::physics_damage::PhysicsDamage::boss_zone(
            0,
            magicka_content::physics::CollisionBox {
                name: "death_unused_box".into(),
                position: [0.; 3],
                sides: [0.; 3],
                orientation: [0., 0., 0., 1.],
            },
        );
        // The native primitive is a capsule, never a prop box.
        damage.collision_box = None;
        damage.solid = false;
        damage.death_boss = Some(actor.clone());
        self.physics_damage.insert(zone, damage);
        let e = self.entity_mut(zone)?;
        e.orientation = orientation;
        e.position = [orientation[12], orientation[13], orientation[14]];
        e.radius = 0.75;
        e.hit_points = 1.;
        e.max_hit_points = 1.;
        e.added = true;
        self.log.push(format!("boss death: live capsule {zone}"));
        Ok(DeathBossHost {
            actor,
            zone,
            skeletons: vec![],
            scythe_bounds: bounds,
            effects: BTreeMap::new(),
            swing: None,
            number: 0,
            clone_target: None,
            rng: 0x736b_656c_6574_6f6e,
            previous_scythe: magicka_content::skinned::IDENTITY,
        })
    }

    pub(super) fn update_death_boss(&mut self, host: &mut DeathBossHost, dt: f32, running: bool) {
        let mut commands = std::mem::take(
            &mut self
                .physics_damage
                .get_mut(&host.zone)
                .unwrap()
                .death_commands,
        );
        // Damage was applied synchronously; consume its effects before changing
        // states so stop/start ordering follows Damage -> UpdateBoss.
        for command in commands.drain(..) {
            self.death_boss_command(host, command);
        }
        let avatars: Vec<_> = self
            .players
            .iter()
            .flatten()
            .map(|p| {
                let b = &self.physics.bodies[p.body];
                Avatar {
                    handle: p.handle,
                    slot: p.slot,
                    position: b.position,
                    direction: b.direction(),
                    radius: b.radius,
                    playing: p.playing,
                    dead: p.dead(),
                }
            })
            .collect();
        let count = self
            .entities
            .iter()
            .filter(|e| {
                e.added
                    && !e.removed
                    && !e.dead
                    && SKELETONS
                        .iter()
                        .any(|s| e.template_name.as_deref() == Some(*s))
            })
            .count() as i32;
        let scale = self.time_modifier * self.time_multiplier;
        let actor = host.actor.clone();
        let (commands, position, previous) = {
            let mut actor = actor.lock().expect("Death actor lock poisoned");
            let previous = actor.body_position;
            host.previous_scythe = actor.scythes[actor.current_slot];
            let commands = actor.update(
                dt,
                running,
                scale,
                &avatars,
                &mut WorldEnvironment { world: self, count },
            );
            (commands, actor.body_position, previous)
        };
        if let Ok(e) = self.entity_mut(host.zone) {
            e.position = position;
            e.orientation[12..15].copy_from_slice(&position);
        }
        self.physics_damage
            .get_mut(&host.zone)
            .unwrap()
            .death_previous = Some(previous);
        for command in commands {
            self.death_boss_command(host, command);
        }
        self.death_boss_scythe_contacts(host);
    }

    fn death_boss_command(&mut self, host: &mut DeathBossHost, command: Command) {
        let position = {
            let d = host.actor.lock().unwrap();
            [d.orientation[12], d.orientation[13], d.orientation[14]]
        };
        match command {
            Command::Effect {
                name,
                position,
                direction,
            } => {
                let id = self.effects.generated_id();
                let transform = crate::particles::create_world(position, direction, [0., 1., 0.]);
                if let Err(e) =
                    EffectActionHost::add_effect(self, id, custom_hash(name), transform, 0.)
                {
                    self.log.push(format!("Death effect {name}: {e}"));
                }
                host.effects.insert(name, id);
            }
            Command::StopEffect(name) => {
                if let Some(id) = host.effects.remove(name) {
                    let _ = EffectActionHost::stop_effect(self, id);
                }
            }
            Command::Sound { bank, name } => {
                let key = self.play_cue(bank, name, Some(position));
                if name == "wep_death_scythe_swing" {
                    host.swing = Some(key);
                }
            }
            Command::StopSwing => {
                if let Some(key) = host.swing.take() {
                    self.stop_cue(key);
                }
            }
            Command::Hit {
                amount,
                first,
                position,
            } => {
                if first {
                    host.number = self.floating_text.add(amount, position, 1., false);
                }
                if host.number >= 0 {
                    self.floating_text.add_to(host.number, amount);
                }
            }
            Command::KillSkeletons => {
                for h in std::mem::take(&mut host.skeletons) {
                    self.overkill(h);
                }
            }
            Command::RaiseSkeleton => self.death_boss_raise(host),
            Command::TimeWarp => {
                if self.ability_effects.time_warp(None, self.scene_saturation) {
                    self.time_warp_cue = Some(self.play_cue(16, "magick_timewarp", None));
                }
            }
            Command::Flash(time) => self.flash(time),
            Command::BeginClones { target } => {
                host.clone_target = Some(target);
                self.time_modifier = 0.5;
                let n = self.players.len();
                self.player_input_locked.fill(n, true);
                self.camera_follow = Some(target);
                self.camera.set_time(0.);
                self.camera.set_magnification(1.2);
                for p in self.players.iter_mut().flatten() {
                    self.player_input_locked[p.slot] = p.handle != target;
                    if p.handle == target {
                        p.character.time_warp_modifier = 2.;
                    } else if p.playing {
                        p.character.set_ethereal(true);
                    }
                }
            }
            Command::TargetSpeed { target } => {
                if let Some(body) = self.character_body(target) {
                    let b = &mut self.physics.bodies[body];
                    b.set_speed_multiplier(b.speed_multiplier() * 2.);
                }
            }
            Command::RevealReal => {
                for p in self.players.iter_mut().flatten().filter(|p| p.playing) {
                    p.character.set_ethereal(false);
                }
            }
            Command::EndClones { target } => {
                self.time_modifier = 1.;
                let n = self.players.len();
                self.player_input_locked.fill(n, false);
                self.camera_follow = None;
                self.camera.release_keep_magnification(0.5);
                if let Some(body) = target.and_then(|h| self.character_body(h)) {
                    self.physics.bodies[body].set_speed_multiplier(1.);
                }
                for p in self.players.iter_mut().flatten().filter(|p| p.playing) {
                    p.character.set_ethereal(false);
                }
                host.clone_target = None;
            }
        }
    }

    fn death_boss_raise(&mut self, host: &mut DeathBossHost) {
        host.rng ^= host.rng << 13;
        host.rng ^= host.rng >> 7;
        host.rng ^= host.rng << 17;
        let template = SKELETONS[(host.rng % 4) as usize];
        let positions: Vec<_> = self
            .entities
            .iter()
            .filter(|e| e.added && !e.removed)
            .map(|e| e.position)
            .collect();
        let Some(position) = magicka_sim::boss_death::skeleton_position(&positions, || {
            self.random_area_point(custom_hash("checkbox_death"))
                .unwrap_or([f32::NAN; 3])
        }) else {
            return;
        };
        if position.iter().any(|v| !v.is_finite()) {
            self.log.push("Death missing skeleton spawn area".into());
            return;
        }
        let direction = {
            let d = host.actor.lock().unwrap();
            [-d.orientation[8], -d.orientation[9], -d.orientation[10]]
        };
        let order = crate::spawn_action::SpawnOrder {
            order: crate::npc::ORDER_ATTACK,
            react_to: crate::npc::REACT_ATTACK | crate::npc::REACT_PROXIMITY,
            reaction: crate::npc::ORDER_ATTACK,
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
            self.log.push(format!("Death failed to spawn {template}"));
            return;
        };
        if let Some(n) = self.npcs.get_mut(&h) {
            n.summon = Some((host.zone, magicka_sim::summons::SummonKind::Normal));
            n.motion.dead_timer = 0.5;
        }
        if let Err(e) = self.begin_resurrection(h, 170) {
            self.log.push(format!("Death spawn animation: {e}"));
        }
        host.skeletons.push(h);
    }

    fn death_boss_scythe_contacts(&mut self, host: &DeathBossHost) {
        let (active, strike, m) = {
            let d = host.actor.lock().unwrap();
            (
                d.update_main
                    && matches!(
                        d.state,
                        crate::boss_death::State::Attack | crate::boss_death::State::MultiDeath
                    ),
                d.scythe_can_strike(),
                d.scythes[d.current_slot],
            )
        };
        if !active {
            return;
        }
        let min = host.scythe_bounds.min;
        let max = host.scythe_bounds.max;
        // Native uses the upper half of the first scythe mesh's bounds.
        let local = crate::pickups::PickupBounds {
            min: [min[0], max[1] * 0.5, min[2]],
            max,
        };
        let bounds = local.world_box(&m, false);
        let old = local.world_box(&host.previous_scythe, false);
        let motion = magicka_sim::hollow_sphere_box::PrimitiveMotion {
            old,
            new: bounds,
            owner_old_position: [
                host.previous_scythe[12],
                host.previous_scythe[13],
                host.previous_scythe[14],
            ],
        };
        let tolerance = magicka_sim::character_physics::COLLISION_TOLERANCE;
        for c in self.combatants() {
            if c.dead {
                continue;
            }
            let Some(body) = self.character_body(c.handle) else {
                continue;
            };
            let contact = magicka_sim::capsule_box::detect(
                self.physics.bodies[body].capsule_motion(),
                motion,
                tolerance,
                || 0,
            )
            .is_some();
            if contact
                && strike
                && self
                    .character_ref(c.handle)
                    .is_some_and(|c| !c.is_immortal())
            {
                self.floating_text.add(c.hit_points, c.position, 1., false);
                self.death_scythe_damage(c.handle, c.hit_points);
            }
        }
        for shield in &mut self.shields {
            if shield.shield.dead {
                continue;
            }
            let hit = match shield.shield.geometry() {
                magicka_sim::shield_intersection::ShieldGeometry::Sphere(s) => {
                    magicka_sim::hollow_sphere_box::detect(
                        magicka_sim::hollow_sphere_box::PrimitiveMotion {
                            old: s,
                            new: s,
                            owner_old_position: s.center,
                        },
                        motion,
                        tolerance,
                        || 0,
                    )
                    .is_some()
                }
                magicka_sim::shield_intersection::ShieldGeometry::Wall(w) => {
                    magicka_sim::box_box::overlap(w, bounds, tolerance).is_some()
                }
            };
            if hit {
                shield.shield.kill();
            }
        }
    }
}
