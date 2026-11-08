//! Scene host for Cthulhu (research/boss-cthulhu.md): the body
//! BossSpellCasterZone, the four tentacle entities, the mist zone, the
//! OtherworldlyBolt cache and the [`CthulhuWorld`] operations.
use super::otherworldly_live::Bolt;
use super::*;
use crate::boss_cthulhu::{self as policy, Cthulhu, CthulhuWorld, Kind, Near};
use crate::xna_math::{self as m, V3};
use magicka_sim::damage::Damage;
use magicka_sim::spray_pulse::HitList;
use magicka_sim::status::Status;

#[derive(Clone, Debug)]
pub struct CthulhuHost {
    pub cthulhu: Box<Cthulhu>,
    pub zone: u16,
    pub mist: u16,
    pub tentacles: [u16; 4],
    pub bolts: Vec<Bolt>,
    /// The tentacles' locked damage numbers.
    numbers: [Option<i32>; 4],
}
impl PartialEq for CthulhuHost {
    fn eq(&self, other: &Self) -> bool {
        self.zone == other.zone
    }
}

struct Scene<'a, C: SceneContent> {
    w: &'a mut SceneWorld<C>,
    bolts: &'a mut Vec<Bolt>,
    numbers: &'a mut [Option<i32>; 4],
}

impl<C: SceneContent> Scene<'_, C> {
    fn near(
        &self,
        e: &crate::spray_world::WorldEntity,
        combatants: &[crate::npc_combat::Combatant],
    ) -> Near {
        use crate::spray_world::ArcGeometry as G;
        let starspawn = custom_hash(policy::STARSPAWN);
        let kind = match e.geometry {
            G::Character { .. } => {
                if self
                    .w
                    .players
                    .iter()
                    .flatten()
                    .any(|p| p.handle == e.handle)
                {
                    Kind::Avatar
                } else {
                    Kind::Npc
                }
            }
            G::Shield(_) => Kind::Shield,
            G::Barrier { .. } => Kind::Barrier,
            G::Mine => Kind::Mine,
            G::Missile => Kind::Missile,
            _ => Kind::Other,
        };
        let c = combatants.iter().find(|c| c.handle == e.handle);
        let mass = self
            .w
            .character_body(e.handle)
            .map_or(0.0, |b| self.w.physics.bodies[b].mass);
        Near {
            handle: e.handle,
            kind,
            position: e.position,
            radius: e.radius,
            mass,
            ethereal: c.is_some_and(|c| c.ethereal),
            invisible: c.is_some_and(|c| c.invisible),
            dead: e.dead,
            drowning: false,
            burning: c.is_some_and(|c| c.status_mask & Status::Burning as u16 != 0),
            friendly: c.is_some_and(|c| c.faction & 1 != 0),
            starspawn: matches!(
                self.w.entity(e.handle).map(|x| x.kind.clone()),
                Ok(EntityKind::Npc { type_id, .. }) if type_id == starspawn
            ),
            lightning_resistance: c.map_or(0.0, |c| c.resistance_against(E::LIGHTNING.0)),
            gripped: c.is_some_and(|c| c.gripped),
            length: c.map_or(0.0, |c| c.length),
        }
    }
}

use magicka_content::skinned::Mat;
use magicka_sim::spells::Elements as E;

impl<C: SceneContent> CthulhuWorld for Scene<'_, C> {
    fn random(&mut self) -> f32 {
        self.w.random()
    }
    fn locator(&mut self, name: &str) -> Option<Mat> {
        self.w.scene_locator(custom_hash(name)).ok()
    }
    fn nav_nearest(&mut self, point: V3) -> V3 {
        self.w
            .nav
            .as_ref()
            .and_then(|n| n.nearest(point, 0))
            .map_or(point, |(_, p, _)| p)
    }
    fn water_height(&mut self, origin: V3, delta: V3) -> Option<f32> {
        let segment = Segment { origin, delta };
        self.w
            .collision
            .collision()
            .liquid_segment_intersect(segment, false, false, false)
            .map(|h| origin[1] + delta[1] * h.fraction)
    }
    fn count(&mut self, template: &str) -> usize {
        let id = custom_hash(template);
        EntityCommandHost::present_characters(self.w, custom_hash("any"))
            .ok()
            .flatten()
            .map_or(0, |v| v.iter().filter(|e| e.type_id == id).count())
    }
    fn time_warp_active(&mut self) -> bool {
        self.w.time_modifier < 1.0
    }
    fn entities(
        &mut self,
        center: V3,
        radius: f32,
        _ignore_protected: bool,
        ignore_y: bool,
    ) -> Vec<Near> {
        let (all, _) = self.w.spray_entities();
        let combatants = self.w.combatants();
        all.iter()
            .filter(|e| {
                !e.dead
                    || matches!(
                        e.geometry,
                        crate::spray_world::ArcGeometry::Character { .. }
                    )
            })
            .filter(|e| {
                let mut d = m::sub(e.position, center);
                if ignore_y {
                    d[1] = 0.0;
                }
                m::length(d) - e.radius <= radius
            })
            .map(|e| self.near(e, &combatants))
            .collect()
    }
    fn entity(&mut self, handle: u16) -> Option<Near> {
        let (all, _) = self.w.spray_entities();
        let combatants = self.w.combatants();
        all.iter()
            .find(|e| e.handle == handle)
            .map(|e| self.near(e, &combatants))
    }
    fn segment_hit(&mut self, handle: u16, origin: V3, delta: V3, radius: f32) -> Option<V3> {
        let segment = Segment { origin, delta };
        if let Some(s) = self.w.shields.iter().find(|s| s.handle == handle) {
            return s
                .shield
                .geometry()
                .segment_fraction(segment)
                .map(|f| m::add(origin, m::scale(delta, f)));
        }
        let (all, _) = self.w.spray_entities();
        let e = all.iter().find(|e| e.handle == handle)?;
        let end = m::add(origin, delta);
        let (a, b, r) = match self.w.combatants().into_iter().find(|c| c.handle == handle) {
            Some(c) => {
                let base = m::sub(c.position, [0.0, c.length * 0.5, 0.0]);
                (base, m::add(base, [0.0, c.length, 0.0]), c.radius)
            }
            None => (e.position, e.position, e.radius),
        };
        if m::segment_segment_distance_squared(origin, end, a, b) > (r + radius).powi(2) {
            return None;
        }
        // The closest point of the segment to the target's centre.
        let l2 = m::length_squared(delta).max(1e-9);
        let t = (m::dot(m::sub(e.position, origin), delta) / l2).clamp(0.0, 1.0);
        Some(m::add(origin, m::scale(delta, t)))
    }
    fn shield_blocks(&mut self, origin: V3, delta: V3) -> bool {
        let segment = Segment { origin, delta };
        self.w
            .shields
            .iter()
            .any(|s| !s.shield.dead && s.shield.geometry().segment_fraction(segment).is_some())
    }
    fn shield_nearest(&mut self, handle: u16, point: V3) -> V3 {
        // INFERRED: the shield as its sphere (Shield.GetNearestPosition).
        self.w
            .shields
            .iter()
            .find(|s| s.handle == handle)
            .map_or(point, |s| {
                let d = m::sub(point, s.shield.position);
                if m::length_squared(d) < 1e-9 {
                    s.shield.position
                } else {
                    m::add(
                        s.shield.position,
                        m::scale(m::normalize(d), s.shield.radius),
                    )
                }
            })
    }
    fn cue(&mut self, bank: u16, name: &'static str, at: V3) -> u32 {
        self.w.play_cue(bank, name, Some(at))
    }
    fn stop_cue(&mut self, key: u32) {
        self.w.stop_cue(key);
    }
    fn effect(&mut self, name: &'static str, t: Mat) {
        self.w
            .status_contact_effects
            .push((name, [t[12], t[13], t[14]], [-t[8], -t[9], -t[10]]));
    }
    fn track(&mut self, key: &str, effect: &'static str, transform: Option<Mat>) {
        self.w.track_effect(key, effect, transform);
    }
    fn circle_damage(&mut self, owner: u16, position: V3, radius: f32, damage: Damage) {
        let t = self.w.play_time;
        self.w.arc_damage(
            Some(owner),
            Some(owner),
            position,
            [0.0, 0.0, -1.0],
            radius,
            std::f32::consts::PI,
            &[damage],
            t,
        );
    }
    fn freeze(&mut self, origin: V3, direction: V3, damage: Damage) {
        self.w
            .freeze_water(origin, direction, std::f32::consts::TAU, 2.0, &[damage]);
    }
    fn time_warp(&mut self, caster: u16) {
        self.w
            .cast_magick_from(caster, magicka_sim::spells::MagickType::TimeWarp);
    }
    fn spawn_npc(
        &mut self,
        template: &'static str,
        position: V3,
        orientation: Mat,
        desired: V3,
        animation: &str,
    ) {
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
        let facing = m::forward(&orientation);
        let Some(h) = self.w.summon_npc(
            None,
            template,
            position,
            facing,
            magicka_sim::summons::SummonKind::Normal,
            order,
        ) else {
            self.w
                .log
                .push(format!("cthulhu failed to spawn {template}"));
            return;
        };
        if let Some(b) = self.w.character_body(h) {
            self.w.physics.bodies[b].set_desired_direction(desired);
        }
        let animation = crate::character_motion::animation_index(animation).unwrap_or(170);
        if let Err(e) = self.w.begin_resurrection(h, animation) {
            self.w.log.push(format!("cthulhu spawn animation: {e}"));
        }
    }
    fn lightning(
        &mut self,
        caster: u16,
        from: V3,
        target: Option<u16>,
        direction: V3,
        range: f32,
        damage: Damage,
        hits: &mut HitList,
    ) {
        let mut damages = [Damage::default(); 5];
        damages[0] = damage;
        let target = target.and_then(|t| {
            self.w
                .combatants()
                .into_iter()
                .find(|c| c.handle == t)
                .map(|c| magicka_sim::lightning::LightningTarget {
                    handle: c.handle,
                    position: c.position,
                    dead: c.dead,
                    eligible: !c.ethereal,
                    frozen: c.status_mask & Status::Frozen as u16 != 0,
                    ends_chain: false,
                })
        });
        let play_time = self.w.play_time;
        let mut bolts = Vec::new();
        let mut rng = self.w.rng;
        // LightningBolt.Cast colour (0.2, 0.2, 0.2), scale 1.
        let color = [0.2; 3];
        self.w.with_rail_world(|world| {
            let mut host = BarrierLightning {
                world,
                bolts: &mut bolts,
                rng: &mut rng,
            };
            match target {
                Some(t) => magicka_sim::lightning::cast_bolt_at(
                    &mut host, caster, play_time, from, t, hits, color, 1.0, range, damages,
                ),
                None => magicka_sim::lightning::cast_bolt(
                    &mut host, caster, play_time, from, direction, hits, color, 1.0, range, damages,
                ),
            }
        });
        self.w.rng = rng;
        self.w.barrier_visuals.extend(bolts);
    }
    fn hypnotize(&mut self, avatar: u16, direction: V3) {
        if let Some(p) = self
            .w
            .players
            .iter_mut()
            .flatten()
            .find(|p| p.handle == avatar)
        {
            let body = &mut self.w.physics.bodies[p.body];
            p.character.hypnotize(body, direction);
        }
    }
    fn charm_and_confuse(&mut self, avatar: u16, mist: u16) {
        let Some(body) = self.w.character_body(avatar) else {
            return;
        };
        let at = self.w.physics.bodies[body].position;
        // Confuse.Execute(position): Execute(owner, target) resets the TTL
        // to 15 s after CharmAndConfuse set 7.5 (native order).
        self.w.confuse_character(at, avatar);
        self.w.charm_character(avatar, mist, 7.5);
    }
    fn damage(&mut self, target: u16, damage: Damage, attacker: Option<u16>, position: V3) {
        self.w.damage_any(target, damage, attacker, position);
    }
    fn kill(&mut self, target: u16) {
        self.w.kill_damageable(target);
    }
    fn overkill(&mut self, target: u16) {
        self.w.overkill(target);
    }
    fn set_velocity(&mut self, target: u16, velocity: V3) {
        if let Some(b) = self.w.character_body(target) {
            self.w.physics.bodies[b].velocity = velocity;
        }
    }
    fn move_body(&mut self, target: u16, position: V3) {
        if let Some(b) = self.w.character_body(target) {
            let body = &mut self.w.physics.bodies[b];
            body.position = position;
            body.velocity = [0.0; 3];
        }
    }
    fn additional_force(&mut self, target: u16, force: V3) {
        if let Some(b) = self.w.character_body(target) {
            self.w.physics.bodies[b].set_additional_force(force);
        }
    }
    fn missile_impulse(&mut self, target: u16, force: V3, dt: f32) {
        if let Some(mi) = self
            .w
            .missiles
            .iter_mut()
            .find(|mi| mi.handle == Some(target))
        {
            let k = dt / crate::missiles::MISSILE_MASS;
            mi.body.velocity = m::add(mi.body.velocity, m::scale(force, k));
        }
    }
    fn release_grip(&mut self, target: u16) {
        // Character.Gripper.ReleaseAttachedCharacter.
        let gripper = self
            .w
            .players
            .iter()
            .flatten()
            .map(|p| (p.handle, &p.character))
            .chain(self.w.npcs.iter().map(|(h, n)| (*h, &n.character)))
            .find(|(_, c)| c.grip.gripped == Some(target))
            .map(|(h, _)| h);
        if let Some(g) = gripper {
            self.w.release_grip(g);
        }
    }
    fn terminate(&mut self, target: u16) {
        self.w.terminate_character(target);
    }
    fn scale_model(&mut self, target: u16, scale: f32) {
        if let Some(c) =
            SceneWorld::<C>::character_mut(&mut self.w.players, &mut self.w.npcs, target)
        {
            c.graphic_scale = scale;
        }
    }
    fn trigger(&mut self, name: &str) {
        let id = custom_hash(&name.to_ascii_lowercase());
        if self.w.scene_trigger_ids.contains(&id) {
            self.w.pending_triggers.push((id, None));
        }
    }
    fn camera_shake(&mut self, _at: V3, magnitude: f32, time: f32) {
        self.w.camera.shake(magnitude, time);
    }
    fn deflect(&mut self, at: V3, strength: f32, dt: f32) {
        self.w.deflect_missiles(at, strength, dt);
    }
    fn spawn_bolt(&mut self, caster: u16, position: V3, direction: V3, speed: f32) -> usize {
        self.w
            .spawn_bolt(self.bolts, caster, position, direction, speed)
    }
    fn bolt_hunt(&mut self, bolt: usize) {
        if let Some(b) = self.bolts.get_mut(bolt) {
            let mut b2 = std::mem::take(b);
            self.w.bolt_hunt(&mut b2);
            *b = b2;
        }
    }
    fn kill_bolts(&mut self) {
        for i in 0..self.bolts.len() {
            let mut b = std::mem::take(&mut self.bolts[i]);
            self.w.destroy_bolt(&mut b, false, false, None, true);
            self.bolts[i] = b;
        }
    }
    fn boss_event(
        &mut self,
        tentacle: Option<usize>,
        event: magicka_sim::boss_status::BossEvent,
        notifier: V3,
    ) {
        match tentacle {
            None => self.w.present_boss_event(event, notifier, policy::BLOOD),
            Some(i) => {
                let mut slot = self.numbers[i];
                self.w
                    .present_boss_event_number(event, notifier, policy::BLOOD, &mut slot);
                self.numbers[i] = slot;
            }
        }
    }
}

fn capsule_zone(
    index: u8,
    origin: V3,
    length: f32,
    radius: f32,
) -> crate::physics_damage::PhysicsDamage {
    let dummy = magicka_content::physics::CollisionBox {
        name: "cthulhu_zone".into(),
        position: [0.; 3],
        sides: [0.; 3],
        orientation: [0., 0., 0., 1.],
    };
    let mut damage = crate::physics_damage::PhysicsDamage::boss_zone(index, dummy);
    damage.collision_box = None;
    damage.solid = false;
    damage.boss_capsule = Some(crate::physics_damage::BossCapsule {
        origin,
        previous: origin,
        length,
        radius,
        ethereal: false,
    });
    damage
}

impl<C: SceneContent> SceneWorld<C> {
    /// Cthulhu's constructor and Initialize at the spawn locator.
    pub(super) fn spawn_cthulhu(&mut self, area: i32) -> Result<CthulhuHost, WorldError> {
        let animation = self
            .content
            .skinned_model(policy::ANIMATION_MODEL)
            .ok_or("missing Cthulhu animations")?;
        let tentacle = self
            .content
            .skinned_model(policy::TENTACLE_ANIMATION)
            .ok_or("missing tentacle animations")?;
        for t in [policy::STARSPAWN, policy::DEEP_ONE, policy::CULTIST] {
            crate::spawn_action::SpawnLevel::load_character_template(self, Some(t))?;
        }
        let player_count = self.players.iter().flatten().filter(|p| p.playing).count();
        let mut cthulhu = Box::new(Cthulhu::new(
            &animation,
            &tentacle,
            player_count,
            0x6374_6875,
        )?);
        let orientation = self.scene_locator(area)?;
        // The body zone (BossSpellCasterZone), the mist (part 5) and the
        // four tentacle entities.
        let zone = self.allocate_physics(true);
        self.physics_damage.insert(
            zone,
            capsule_zone(
                0,
                m::translation(&orientation),
                policy::ZONE_LENGTH,
                policy::ZONE_RADIUS,
            ),
        );
        self.add_boss_caster(zone, policy::ZONE_RADIUS);
        let mist = self.allocate_physics(true);
        let mut mist_zone = capsule_zone(5, [0.0, -1000.0, 0.0], 0.0, policy::MIST_RADIUS);
        if let Some(c) = mist_zone.boss_capsule.as_mut() {
            c.ethereal = true;
        }
        self.physics_damage.insert(mist, mist_zone);
        let mut tentacles = [0u16; 4];
        for (i, slot) in tentacles.iter_mut().enumerate() {
            let h = self.allocate_physics(true);
            let mut z = capsule_zone(
                i as u8 + 1,
                [0.0, -1000.0 - i as f32, 0.0],
                0.0,
                policy::TENTACLE_RADIUS,
            );
            if let Some(c) = z.boss_capsule.as_mut() {
                c.ethereal = true;
            }
            self.physics_damage.insert(h, z);
            *slot = h;
        }
        for (h, hp) in [(zone, policy::MAX_HIT_POINTS), (mist, 1.0)]
            .into_iter()
            .chain(tentacles.iter().map(|&t| (t, policy::TENTACLE_HIT_POINTS)))
        {
            let e = self.entity_mut(h)?;
            e.added = true;
            e.hit_points = hp;
            e.max_hit_points = hp;
        }
        cthulhu.zone = zone;
        cthulhu.mist.handle = mist;
        for (t, &h) in cthulhu.tentacles.iter_mut().zip(&tentacles) {
            t.handle = h;
        }
        let mut host = CthulhuHost {
            cthulhu,
            zone,
            mist,
            tentacles,
            bolts: Vec::new(),
            numbers: [None; 4],
        };
        let mut bolts = std::mem::take(&mut host.bolts);
        let mut numbers = host.numbers;
        host.cthulhu.initialize(
            orientation,
            &mut Scene {
                w: self,
                bolts: &mut bolts,
                numbers: &mut numbers,
            },
        );
        host.bolts = bolts;
        host.numbers = numbers;
        self.place_cthulhu(&host);
        self.log.push(format!(
            "boss cthulhu: zone {zone}, tentacles {tentacles:?}"
        ));
        Ok(host)
    }

    /// BossMessages to Cthulhu.
    pub(super) fn cthulhu_message(&mut self, host: &mut CthulhuHost, name: &str) {
        host.cthulhu.message(name);
    }

    /// The zones' positions and capsules from the boss pose.
    fn place_cthulhu(&mut self, host: &CthulhuHost) {
        let c = &host.cthulhu;
        let hp = c.status.hit_points;
        let origin = c.zone_origin;
        let axis = c.zone_axis;
        let mut caster_source = magicka_sim::spray_pulse::SpraySource {
            owner_handle: host.zone,
            owner_position: origin,
            owner_direction: m::forward(&c.transform),
            cast_position: m::translation(&c.mouth()),
            cast_direction: m::forward(&c.transform),
            weapon_position: m::translation(&c.mouth()),
            weapon_direction: m::forward(&c.transform),
        };
        caster_source.owner_direction[1] = 0.0;
        self.set_boss_caster_source(host.zone, caster_source);
        if let Some(d) = self.physics_damage.get_mut(&host.zone) {
            if let Some(cap) = d.boss_capsule.as_mut() {
                cap.previous = cap.origin;
                // INFERRED: the capsule kept vertical (the rewrite's boss
                // capsules are upright) from spine1, spanning the
                // spine1→spine5 axis height.
                cap.origin = origin;
                cap.length = policy::ZONE_LENGTH * axis[1].abs().max(0.25);
                cap.ethereal = c.dead;
            }
        }
        if let Ok(e) = self.entity_mut(host.zone) {
            e.position = origin;
            e.orientation = c.transform;
            e.hit_points = hp;
            e.max_hit_points = policy::MAX_HIT_POINTS;
            e.radius = policy::ZONE_RADIUS;
            e.length = policy::ZONE_LENGTH;
        }
        // The mist: live while active.
        let mist_active = c.mist.active;
        let mist_at = c.mist.position;
        if let Some(d) = self.physics_damage.get_mut(&host.mist) {
            if let Some(cap) = d.boss_capsule.as_mut() {
                cap.previous = cap.origin;
                cap.origin = if mist_active {
                    mist_at
                } else {
                    [0.0, -1000.0, 0.0]
                };
                cap.ethereal = !mist_active;
            }
        }
        if let Ok(e) = self.entity_mut(host.mist) {
            e.position = mist_at;
            e.radius = policy::MIST_RADIUS;
        }
        // Each tentacle: one upright capsule from its base to the top joint
        // (INFERRED: the native skin is ten capsules along the spine).
        for (t, &h) in c.tentacles.iter().zip(&host.tentacles) {
            let alive = t.active && !t.dead;
            let base = m::translation(&t.transform);
            let top = m::translation(&t.top_joint);
            if let Some(d) = self.physics_damage.get_mut(&h) {
                if let Some(cap) = d.boss_capsule.as_mut() {
                    cap.previous = cap.origin;
                    cap.origin = base;
                    cap.length = (top[1] - base[1]).max(0.5);
                    cap.radius = policy::TENTACLE_RADIUS;
                    cap.ethereal = !alive;
                }
            }
            if let Ok(e) = self.entity_mut(h) {
                e.position = base;
                e.hit_points = t.hit_points;
                e.dead = !alive;
            }
        }
    }

    fn cthulhu_players(&self) -> Vec<policy::Player> {
        (0..self.players.len().max(4))
            .map(|i| {
                let Some(p) = self.players.get(i).and_then(Option::as_ref) else {
                    return policy::Player::default();
                };
                policy::Player {
                    playing: p.playing,
                    avatar: Some(p.handle),
                    dead: p.dead(),
                    position: self.physics.bodies[p.body].position,
                    ethereal: p.character.ethereal,
                    hypnotized: p.character.hypnotized.is_some(),
                }
            })
            .collect()
    }

    /// Cthulhu.UpdateBoss with the zone deliveries of the last frame.
    pub(super) fn update_cthulhu(&mut self, host: &mut CthulhuHost, dt: f32) {
        let mut bolts = std::mem::take(&mut host.bolts);
        let mut numbers = host.numbers;
        let time_scale = self.time_modifier;
        let players = self.cthulhu_players();
        {
            let mut scene = Scene {
                w: self,
                bolts: &mut bolts,
                numbers: &mut numbers,
            };
            // Damage deliveries: the body, the mist and each tentacle.
            let parts: Vec<(Option<usize>, u16)> = std::iter::once((None, host.zone))
                .chain(std::iter::once((Some(usize::MAX), host.mist)))
                .chain(
                    host.tentacles
                        .iter()
                        .enumerate()
                        .map(|(i, &h)| (Some(i), h)),
                )
                .collect();
            for (part, zone) in parts {
                let inbox = scene
                    .w
                    .physics_damage
                    .get_mut(&zone)
                    .map(|z| z.take_boss_inbox(scene.w.boss_damage_divisor))
                    .unwrap_or_default();
                let position = scene.w.entity(zone).map_or([0.; 3], |e| e.position);
                for (damage, features) in inbox {
                    match part {
                        None => {
                            host.cthulhu
                                .damage(0, damage, position, features, &mut scene);
                        }
                        Some(usize::MAX) => {
                            host.cthulhu
                                .damage(5, damage, position, features, &mut scene);
                        }
                        Some(i) => {
                            if host.cthulhu.tentacles[i].active && !host.cthulhu.tentacles[i].dead {
                                host.cthulhu
                                    .tentacle_damage(i, damage, position, &mut scene);
                            }
                        }
                    }
                }
            }
            host.cthulhu.update(dt, time_scale, players, &mut scene);
        }
        self.place_cthulhu(host);
        self.cthulhu_contacts(host, &mut bolts, &mut numbers);
        self.update_bolts(&mut bolts, dt);
        host.bolts = bolts;
        host.numbers = numbers;
    }

    /// Body collisions (shields killed, missiles start the deflection
    /// aura) and the mist's touches.
    fn cthulhu_contacts(
        &mut self,
        host: &mut CthulhuHost,
        bolts: &mut Vec<Bolt>,
        numbers: &mut [Option<i32>; 4],
    ) {
        let c = &host.cthulhu;
        if c.dead {
            return;
        }
        let origin = c.zone_origin;
        let top = m::add(origin, m::scale(c.zone_axis, policy::ZONE_LENGTH));
        let mut capsules = vec![(origin, top, policy::ZONE_RADIUS)];
        capsules.extend(c.arms.iter().copied());
        let touches = |p: V3, r: f32| {
            capsules
                .iter()
                .any(|(a, b, cr)| m::point_segment_distance_squared(p, *a, *b) <= (cr + r).powi(2))
        };
        let shields: Vec<u16> = self
            .shields
            .iter()
            .filter(|s| !s.shield.dead && touches(s.shield.position, s.shield.radius))
            .map(|s| s.handle)
            .collect();
        for h in shields {
            if let Some(s) = self.shields.iter_mut().find(|s| s.handle == h) {
                s.shield.kill();
            }
        }
        if self
            .missiles
            .iter()
            .any(|mi| !mi.body.dead && touches(mi.body.position, mi.body.radius))
        {
            host.cthulhu.missile_contact();
        }
        if !host.cthulhu.mist.active {
            return;
        }
        let at = host.cthulhu.mist.position;
        let mut scene = Scene {
            w: self,
            bolts,
            numbers,
        };
        let touching = scene.entities(at, policy::MIST_RADIUS, false, false);
        for e in touching {
            if e.handle == host.mist || e.handle == host.zone || host.tentacles.contains(&e.handle)
            {
                continue;
            }
            let missile_fire = e.kind == Kind::Missile
                && scene.w.missiles.iter().any(|mi| {
                    mi.handle == Some(e.handle) && mi.state.combined_elements & E::FIRE.0 != 0
                });
            let (immortal, charmed) = scene
                .w
                .character_ref(e.handle)
                .map_or((false, false), |ch| (ch.is_immortal(), ch.is_charmed()));
            host.cthulhu
                .mist_contact(&e, missile_fire, immortal, charmed, &mut scene);
        }
    }
}
