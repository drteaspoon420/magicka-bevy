//! MissileEntity contact decisions and non-homing update state.
//! Native physics/rendering remain host operations; research/missile-collision.md.
use crate::{
    character_impact::result as R,
    damage::{attack, Damage},
    impact_motion::Vector,
    missile_events::{condition, EventArgs, MissileConditions, MissileEvent, MissileEventHost},
    spray_pulse::HitList,
};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticContact {
    Level,
    Water,
    Lava,
    Other,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Character { ethereal: bool },
    BossDamage { ethereal: bool },
    BossCollision,
    Shield,
    Barrier { solid: bool },
    Missile { owner: Option<u16> },
    Pickable,
    CthulhuMist,
    Grease,
    SpellMine,
    ElementalEgg,
    Tornado,
    Other,
}
#[derive(Clone, Copy, Debug)]
pub struct CollisionTarget {
    pub handle: u16,
    pub kind: TargetKind,
    pub dead: bool,
    pub damageable: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum Contact {
    Static(StaticContact),
    Body {
        physics_object: bool,
        target: Option<CollisionTarget>,
    },
}
pub trait MissileCollisionHost: MissileEventHost {
    fn position(&self) -> Vector;
    fn velocity(&self) -> Vector;
    fn set_velocity(&mut self, velocity: Vector);
    fn target_position(&self, handle: u16) -> Vector;
    fn select_target(&mut self, target: Option<u16>);
    fn update_followed_effects(&mut self);
    fn award_deep_impact(&mut self);
    /// MissileEntity.Update's homing block, between the pending velocity
    /// change and the events (no-op for non-homing missiles).
    fn steer(&mut self, _dt: f32) {}
}
fn length(v: Vector) -> f32 {
    ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) as f64).sqrt() as f32
}
fn normalize(v: Vector) -> Vector {
    let inverse = 1. / length(v);
    v.map(|x| x * inverse)
}
fn clr_max(a: f32, b: f32) -> f32 {
    if a > b || a.is_nan() {
        a
    } else {
        b
    }
}
#[derive(Clone, Debug)]
pub struct MissileCollisionState {
    pub owner: Option<u16>,
    pub conditions: MissileConditions,
    pub piercing: bool,
    pub combined_elements: u32,
    pub hits: HitList,
    pub gibbed_hits: BTreeMap<u16, f32>,
    pub excluded_owner: Option<u16>,
    pub time_alive: f32,
    pub has_collided: bool,
    pub level_timer: f32,
    pub collision_velocity: f32,
    pub collision_target: Option<u16>,
    pub pending_velocity: Option<Vector>,
    pub is_damaged: bool,
    pub damage_amount: f32,
    pub damage_elements: u32,
    pub deep_impact_possible: bool,
    pub deep_impact_count: i32,
}
impl MissileCollisionState {
    /// After copying native conditions, before the default-event initialization.
    /// Deep-impact eligibility comes from exact-Earth/local-Avatar facts at spawn.
    pub fn new(
        owner: Option<u16>,
        can_hit_owner: bool,
        conditions: MissileConditions,
        deep_impact_possible: bool,
    ) -> Self {
        let mut piercing = false;
        let mut combined_elements = 0;
        for slot in &conditions.slots {
            for e in &slot.events {
                match e {
                    MissileEvent::Damage { damage, .. } => {
                        combined_elements |= damage.element;
                        piercing |= damage.attack & attack::PIERCING != 0;
                    }
                    MissileEvent::Blast { damages, .. } => {
                        for d in damages {
                            combined_elements |= d.element;
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut hits = HitList::default();
        if can_hit_owner {
            if let Some(o) = owner {
                hits.add(o);
            }
        }
        Self {
            owner,
            conditions,
            piercing,
            combined_elements,
            hits,
            gibbed_hits: BTreeMap::new(),
            excluded_owner: if can_hit_owner { None } else { owner },
            time_alive: 0.,
            has_collided: false,
            level_timer: 0.,
            collision_velocity: 0.,
            collision_target: None,
            pending_velocity: None,
            is_damaged: false,
            damage_amount: 0.,
            damage_elements: 0,
            deep_impact_possible,
            deep_impact_count: 0,
        }
    }
    pub fn execute_default(&mut self, host: &mut impl MissileCollisionHost) {
        host.select_target(None);
        self.conditions.execute(
            &mut EventArgs {
                flags: condition::DEFAULT,
                ..Default::default()
            },
            host,
        );
        self.time_alive = 0.;
        self.collision_target = None;
    }
    fn mark_collision(&mut self, host: &impl MissileCollisionHost) {
        self.has_collided = true;
        let v = host.velocity();
        let squared = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
        self.collision_velocity = clr_max(self.collision_velocity, squared);
    }
    /// Return true to allow physics contact response, false to pass through.
    pub fn on_collision(
        &mut self,
        contact: Contact,
        may_execute: bool,
        host: &mut impl MissileCollisionHost,
    ) -> bool {
        let target = match contact {
            Contact::Static(kind) => {
                if kind == StaticContact::Other {
                    return false;
                }
                self.collision_target = None;
                if self.has_collided {
                    return true;
                }
                if self.level_timer > 0. {
                    return false;
                }
                self.mark_collision(host);
                return true;
            }
            Contact::Body {
                physics_object,
                target,
            } => {
                if physics_object {
                    self.mark_collision(host);
                    if self.collision_target.is_none() {
                        self.collision_target = target.map(|t| t.handle);
                    }
                }
                target
            }
        };
        let Some(t) = target else {
            return true;
        };
        match t.kind {
            TargetKind::Pickable
            | TargetKind::CthulhuMist
            | TargetKind::Grease
            | TargetKind::Character { ethereal: true }
            | TargetKind::BossDamage { ethereal: true } => return false,
            TargetKind::BossCollision => return true,
            _ => {}
        }
        if t.dead || !t.damageable {
            return false;
        }
        match t.kind {
            TargetKind::Barrier { solid: false }
            | TargetKind::SpellMine
            | TargetKind::ElementalEgg
            | TargetKind::Tornado => return false,
            TargetKind::Missile { owner } if owner == self.owner => return false,
            _ => {}
        }
        let barrier = matches!(t.kind, TargetKind::Barrier { .. });
        let shield = t.kind == TargetKind::Shield;
        if self.gibbed_hits.contains_key(&t.handle) {
            return false;
        }
        if self.hits.contains(t.handle) {
            return shield || barrier || !self.piercing;
        }
        if !may_execute {
            return true;
        }
        host.select_target(Some(t.handle));
        let (_, result) = self.conditions.execute(
            &mut EventArgs {
                flags: condition::HIT,
                ..Default::default()
            },
            host,
        );
        if self.deep_impact_possible
            && matches!(t.kind, TargetKind::Character { .. })
            && result & (R::KILLED | R::OVERKILLED) != 0
        {
            self.deep_impact_count = self.deep_impact_count.wrapping_add(1);
            if self.deep_impact_count >= 5 {
                host.award_deep_impact();
            }
        }
        if result & (R::OVERKILLED | R::PIERCED) != 0 || (result & R::KILLED != 0 && barrier) {
            if result & R::PIERCED == 0 {
                let pos = host.position();
                let target_pos = host.target_position(t.handle);
                let v = host.velocity();
                let speed = length(v);
                let away = normalize(std::array::from_fn(|i| pos[i] - target_pos[i]));
                let forward = normalize(v);
                self.pending_velocity = Some(std::array::from_fn(|i| {
                    away[i] * (speed * 0.3) + forward[i] * (speed * 0.9)
                }));
            }
            self.gibbed_hits.insert(t.handle, 5.);
            return false;
        }
        self.hits.add(t.handle);
        shield || barrier || result & (R::DEFLECTED | R::HIT) != 0
    }
    pub fn receive_damage(&mut self, damage: Damage, attacker_is_grease: bool) -> u16 {
        if damage.amount != 0. && !attacker_is_grease {
            self.is_damaged = true;
            self.damage_amount += damage.amount;
            self.damage_elements |= damage.element;
        }
        0
    }
    /// Native Update through events/fall removal for missiles with homing=0.
    /// Physics integration happens outside this phase, before entity update.
    pub fn update_without_homing(
        &mut self,
        dt: f32,
        may_execute: bool,
        host: &mut impl MissileCollisionHost,
    ) -> EventArgs {
        self.update_without_homing_at_kill_plane(dt, may_execute, -50., host)
    }

    /// Original scenes use -50 m. External maps can supply their own world
    /// kill plane so valid projectiles below Magicka's origin survive.
    pub fn update_without_homing_at_kill_plane(
        &mut self,
        dt: f32,
        may_execute: bool,
        kill_plane: f32,
        host: &mut impl MissileCollisionHost,
    ) -> EventArgs {
        host.update_followed_effects();
        self.hits.update(dt);
        self.gibbed_hits.retain(|_, ttl| {
            *ttl -= dt;
            ttl.is_nan() || *ttl > 0.
        });
        self.time_alive += dt;
        if let Some(v) = self.pending_velocity.take() {
            host.set_velocity(v);
        }
        let pos = host.position();
        host.steer(dt);
        let mut args = EventArgs {
            flags: condition::TIMER,
            time: self.time_alive,
            ..Default::default()
        };
        if self.has_collided {
            if self.level_timer <= f32::from_bits(1) {
                args.flags |= condition::COLLISION;
                args.threshold = self.collision_velocity;
            }
            self.level_timer += dt;
            if self.level_timer >= 0.15 {
                self.has_collided = false;
                self.level_timer = 0.;
            }
            self.collision_target = None;
        }
        self.collision_velocity = 0.;
        if self.is_damaged {
            args.flags |= condition::DAMAGED;
            args.hitpoints = self.damage_amount;
            args.elements = self.damage_elements;
        }
        if may_execute {
            host.select_target(self.collision_target);
            self.conditions.execute(&mut args, host);
        }
        self.damage_elements = 0;
        self.damage_amount = 0.;
        if pos[1] <= kill_plane {
            host.kill();
        }
        args
    }
}
