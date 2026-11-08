//! Scene, damage zones and spell ownership for the physical Grimnir boss.
use super::*;
use crate::boss_grimnir2::{self as policy, Command, Grimnir2, Inputs};
use magicka_sim::{spray_pulse::SpraySource, status::Status};

#[derive(Clone, Debug)]
pub struct Grimnir2Host {
    pub actor: Box<Grimnir2>,
    pub zones: [u16; 3],
    target: Option<u16>,
    target_timer: f32,
    cliff_inverse: Option<[f32; 16]>,
}
impl PartialEq for Grimnir2Host {
    fn eq(&self, other: &Self) -> bool {
        self.zones == other.zones
    }
}
fn cliff(
    parts: &[crate::animated_hierarchy::AnimatedPartState],
) -> Option<&crate::animated_hierarchy::AnimatedPartState> {
    for p in parts {
        if p.id == custom_hash("cliff") {
            return Some(p);
        }
        if let Some(p) = cliff(&p.children) {
            return Some(p);
        }
    }
    None
}
impl<C: SceneContent> SceneWorld<C> {
    pub(super) fn spawn_grimnir2(&mut self, area: i32) -> Result<Grimnir2Host, WorldError> {
        let orientation = self.scene_locator(area)?;
        let model = self
            .content
            .skinned_model(policy::ANIMATION)
            .ok_or("missing Grimnir2 animation model")?;
        let assatur = self
            .content
            .skinned_model(policy::ASSATUR)
            .ok_or("missing Assatur model")?;
        let actor = Box::new(Grimnir2::new(model, assatur, orientation)?);
        let mut zones = [0; 3];
        for (i, zone) in zones.iter_mut().enumerate() {
            let h = self.allocate_physics(true);
            *zone = h;
            let dummy = magicka_content::physics::CollisionBox {
                name: "boss_capsule".into(),
                position: [0.; 3],
                sides: [0.; 3],
                orientation: [0., 0., 0., 1.],
            };
            let mut damage = crate::physics_damage::PhysicsDamage::boss_zone(i as u8, dummy);
            damage.collision_box = None;
            damage.solid = false;
            let mut origin = [orientation[12], orientation[13], orientation[14]];
            origin[1] += 1.125;
            damage.boss_capsule = Some(crate::physics_damage::BossCapsule {
                origin,
                previous: origin,
                length: 1.5,
                radius: 0.75,
                ethereal: i != 0,
            });
            self.physics_damage.insert(h, damage);
            let e = self.entity_mut(h)?;
            e.added = true;
            e.orientation = orientation;
            e.position = origin;
            e.hit_points = 1.;
            e.max_hit_points = 1.;
            e.radius = if i == 0 { 1. } else { 1.5 };
            e.length = 2.;
            if i == 0 {
                e.unique_id = custom_hash("#boss_n07");
            }
            self.add_boss_caster(h, if i == 0 { 1. } else { 1.5 });
        }
        let mut host = Grimnir2Host {
            actor,
            zones,
            target: self
                .players
                .iter()
                .flatten()
                .find(|p| p.playing)
                .map(|p| p.handle),
            target_timer: 30.,
            cliff_inverse: cliff(self.collision.parts())
                .and_then(|p| crate::live_collision::inverse(&p.world).ok()),
        };
        let inputs = self.grimnir2_inputs(&mut host, 0., false);
        // Establish usable bind/idle poses and cast sources on initialization.
        let commands = host.actor.update(0., inputs);
        self.sync_grimnir2_zones(&mut host);
        for c in commands {
            self.grimnir2_command(&mut host, c);
        }
        self.log.push(format!(
            "boss grimnir2: live physical caster zones {:?}",
            zones
        ));
        Ok(host)
    }
    fn grimnir2_inputs(&mut self, host: &mut Grimnir2Host, dt: f32, running: bool) -> Inputs {
        host.target_timer -= dt;
        let players: Vec<_> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.playing)
            .map(|p| {
                (
                    p.handle,
                    self.physics.bodies[p.body].position,
                    p.dead(),
                    p.character.combat.statuses.has(Status::Wet),
                )
            })
            .collect();
        if host.target_timer <= 0. || !players.iter().any(|p| Some(p.0) == host.target && !p.2) {
            host.target_timer = 30.;
            if !players.is_empty() {
                let index = (self.random() * players.len() as f32) as usize;
                host.target = Some(players[index.min(players.len() - 1)].0);
            }
        }
        let origin = &host.actor.orientation[12..15];
        Inputs {
            fight_started: running,
            nearest_distance_squared: players
                .iter()
                .map(|p| (0..3).map(|i| (p.1[i] - origin[i]).powi(2)).sum::<f32>())
                .fold(f32::MAX, f32::min),
            tornado: !self.tornadoes.is_empty(),
            rain: self.rain_cue.is_some(),
            wet_players: if players.is_empty() {
                0.
            } else {
                players.iter().filter(|p| p.3).count() as f32 / players.len() as f32
            },
            conflagration: self
                .ability_effects
                .effects
                .iter()
                .any(|e| matches!(e, magicka_sim::magicks::AbilityEffect::Conflagration { .. })),
            blizzard: self.blizzard_cue.is_some(),
            target: players
                .iter()
                .find(|p| Some(p.0) == host.target)
                .map_or([origin[0], origin[1], origin[2]], |p| p.1),
        }
    }
    pub(super) fn update_grimnir2(&mut self, host: &mut Grimnir2Host, dt: f32, running: bool) {
        let mut commands = Vec::new();
        for (i, h) in host.zones.iter().enumerate() {
            let inbox = self
                .physics_damage
                .get_mut(h)
                .map(|s| s.take_boss_inbox(self.boss_damage_divisor))
                .unwrap_or_default();
            let position = self.entity(*h).map_or([0.; 3], |e| e.position);
            for (damage, features) in inbox {
                commands.extend(host.actor.receive_damage(i, damage, position, features));
            }
        }
        let inputs = self.grimnir2_inputs(host, dt, running);
        commands.extend(host.actor.update(dt, inputs));
        if let (Some(inverse), Some(cliff)) = (host.cliff_inverse, cliff(self.collision.parts())) {
            if cliff.playback.time > 0. {
                let delta = magicka_content::skinned::mul(&cliff.world, &inverse);
                host.actor.pose.parent =
                    magicka_content::skinned::mul(&delta, &host.actor.orientation);
            }
        }
        self.sync_grimnir2_zones(host);
        for c in commands {
            self.grimnir2_command(host, c);
        }
    }
    fn sync_grimnir2_zones(&mut self, host: &mut Grimnir2Host) {
        let g = &host.actor;
        let skeleton = &g.skeleton;
        let yrot = magicka_content::skinned::pose_matrix([0.; 3], [0., 1., 0., 0.], [1.; 3]);
        for i in 0..3 {
            let pose = if i == 0 {
                &g.pose
            } else {
                &g.spirits[i - 1].pose
            };
            let mut body_matrix = if i == 0 { g.orientation } else { pose.parent };
            body_matrix[13] += if i == 0 { 1.5 } else { 0.75 };
            let position = [body_matrix[12], body_matrix[13], body_matrix[14]];
            let bone_source = |name: &str| -> Option<[f32; 16]> {
                let bone = skeleton.bone(name)?;
                let bind =
                    crate::live_collision::inverse(&skeleton.skeleton.bones[bone].inverse_bind)
                        .ok()?;
                let matrices = skeleton.skinned_from_local(&pose.local, &pose.parent);
                Some(magicka_content::skinned::mul(
                    &magicka_content::skinned::mul(&yrot, &bind),
                    matrices.get(bone)?,
                ))
            };
            let cast = bone_source("leftattach").unwrap_or(body_matrix);
            let weapon = bone_source("rightattach").unwrap_or(body_matrix);
            let point = |m: &[f32; 16]| [m[12], m[13], m[14]];
            let forward = |m: &[f32; 16]| [-m[8], -m[9], -m[10]];
            let h = host.zones[i];
            self.set_boss_caster_source(
                h,
                SpraySource {
                    owner_handle: h,
                    owner_position: position,
                    owner_direction: forward(&body_matrix),
                    cast_position: point(&cast),
                    cast_direction: forward(&cast),
                    weapon_position: point(&weapon),
                    weapon_direction: forward(&weapon),
                },
            );
            if let Ok(e) = self.entity_mut(h) {
                e.position = position;
                e.orientation = body_matrix;
                e.hit_points = if i == 0 { g.status.hit_points } else { 1. };
                e.max_hit_points = if i == 0 { policy::MAX_HP } else { 1. };
                e.dead = false;
            }
            if let Some(d) = self.physics_damage.get_mut(&h) {
                if let Some(c) = d.boss_capsule.as_mut() {
                    c.previous = c.origin;
                    c.origin = position;
                    c.origin[1] -= 0.375;
                    c.ethereal = i != 0 || g.state == policy::State::Die;
                }
            }
        }
    }
    pub(super) fn grimnir2_command(&mut self, host: &mut Grimnir2Host, command: Command) {
        match command {
            Command::Trigger(name) => {
                if self.scene_trigger_ids.contains(&custom_hash(name)) {
                    self.pending_triggers.push((custom_hash(name), None));
                }
            }
            Command::Cast {
                part,
                spell,
                cast,
                power,
            } => self.cast_boss_spell(host.zones[part], spell, cast, power),
            Command::StopSpell(part) => self.stop_boss_spell(host.zones[part]),
            Command::Magick(m) => self.cast_magick_from(host.zones[0], m),
            Command::CameraBias => self.camera.set_bias([0., 4., -7.], 3.),
            Command::HealEffect => {
                let mut p = [
                    host.actor.orientation[12],
                    host.actor.orientation[13],
                    host.actor.orientation[14],
                ];
                p[1] += 1.5;
                self.mine_effects.push(("assatur_life", p, [0., 0., -1.]));
                self.play_cue(16, "spell_life_self", Some(p));
            }
            Command::Boss(event) => {
                use magicka_sim::boss_status::BossEvent as B;
                let mut p = [
                    host.actor.orientation[12],
                    host.actor.orientation[13],
                    host.actor.orientation[14],
                ];
                p[1] += 4.;
                match event {
                    B::Blood { position } => self.status_contact_effects.push((
                        "gore_splash_regular",
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
    pub(super) fn corporealize_grimnir2(&mut self, position: [f32; 3]) {
        let talks = self
            .scene_trigger_ids
            .contains(&custom_hash("assatur_talks"));
        let mut bosses = std::mem::take(&mut self.boss_fight.bosses);
        for boss in &mut bosses {
            let boss_live::BossRef::Grimnir2(g) = boss else {
                continue;
            };
            if g.zones.iter().any(|h| {
                self.entity(*h).is_ok_and(|e| {
                    (0..3)
                        .map(|i| (e.position[i] - position[i]).powi(2))
                        .sum::<f32>()
                        <= (30. + e.radius).powi(2)
                })
            }) {
                for c in g.actor.corporealize(talks) {
                    self.grimnir2_command(g, c);
                }
            }
        }
        self.boss_fight.bosses = bosses;
    }
    pub(super) fn nullify_grimnir2(&mut self) {
        let mut bosses = std::mem::take(&mut self.boss_fight.bosses);
        for boss in &mut bosses {
            if let boss_live::BossRef::Grimnir2(g) = boss {
                for c in g.actor.nullify() {
                    self.grimnir2_command(g, c);
                }
            }
        }
        self.boss_fight.bosses = bosses;
        let handles: Vec<_> = self.boss_casters.keys().copied().collect();
        for h in handles {
            self.stop_boss_spell(h);
        }
    }
}
