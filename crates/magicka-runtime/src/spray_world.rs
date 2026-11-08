//! Spray geometry adapter with synchronous, world-owned damage callbacks.
use crate::level_collision::SceneCollision;
use magicka_sim::{
    arc_intersection::{self, ArcQuery},
    damage::Damage,
    entity_query::{EntityQueryGrid, QueryEntity, QueryError},
    impact_motion::Vector,
    shield_intersection::ShieldGeometry,
    spray_pulse::{Segment, SprayTarget, SprayWorld},
};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug)]
pub enum ArcGeometry {
    Entity,
    Boss,
    BossZone {
        ethereal: bool,
    },
    /// Native Dispenser arc ignores the height gate and is not a lightning target.
    Dispenser,
    /// MissileEntity ignores the arc height-difference gate.
    Missile,
    /// Generic Entity.ArcIntersect, but not LightningBolt's native type filter.
    MagickDeath,
    /// Generic Entity arc; explicitly accepted by LightningBolt's type filter.
    ElementalEgg,
    Character {
        ethereal: bool,
    },
    Shield(ShieldGeometry),
    /// Barrier.ArcIntersect (the generic entity arc with the capsule
    /// radius). Only Solid barriers are lightning targets.
    Barrier {
        solid: bool,
    },
    /// SpellMine (Entity.ArcIntersect; never a lightning target).
    Mine,
}
#[derive(Clone, Copy, Debug)]
pub struct WorldEntity {
    pub handle: u16,
    pub position: Vector,
    pub radius: f32,
    pub dead: bool,
    pub damageable: bool,
    /// HasStatus(Frozen) (lightning skips frozen targets).
    pub frozen: bool,
    pub geometry: ArcGeometry,
}
impl WorldEntity {
    fn query_entity(self) -> QueryEntity {
        QueryEntity {
            position: self.position,
            radius: self.radius,
            dead: self.dead,
            damageable: self.damageable,
            shield: matches!(self.geometry, ArcGeometry::Shield(_)),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SprayDamage {
    pub target: u16,
    pub attacker: u16,
    pub damages: [Damage; 5],
    pub timestamp: f64,
    pub position: Vector,
}
/// Callbacks run immediately. A host can deliver through ImpactCharacter.deliver
/// and reflect resulting geometry changes before the next arc query. Entity
/// identities/order must remain stable for the grid snapshot's lifetime.
pub trait SprayEffects {
    fn damage(&mut self, entities: &mut [WorldEntity], damage: SprayDamage);
    fn electrocute(
        &mut self,
        entities: &mut [WorldEntity],
        target: u16,
        attacker: u16,
        multiplier: f32,
    );
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SprayWorldError {
    Query(QueryError),
    DuplicateHandle(u16),
    InvalidShield(u16),
}

pub struct RuntimeSprayWorld<'a, H> {
    scene: Option<&'a SceneCollision>,
    entities: &'a mut [WorldEntity],
    shield_order: &'a [u16],
    grid: EntityQueryGrid,
    effects: &'a mut H,
    pub query_error: Option<QueryError>,
    /// The scene's Railgun pool (RailGunSpell Force steering), if hosted.
    rails: Option<&'a mut magicka_sim::railgun::RailSystem>,
}
impl<'a, H: SprayEffects> RuntimeSprayWorld<'a, H> {
    pub fn new(
        scene: Option<&'a SceneCollision>,
        entities: &'a mut [WorldEntity],
        shield_order: &'a [u16],
        effects: &'a mut H,
    ) -> Result<Self, SprayWorldError> {
        let mut handles = HashSet::new();
        for e in entities.iter() {
            if !handles.insert(e.handle) {
                return Err(SprayWorldError::DuplicateHandle(e.handle));
            }
        }
        for &shield in shield_order {
            if !entities
                .iter()
                .any(|e| e.handle == shield && matches!(e.geometry, ArcGeometry::Shield(_)))
            {
                return Err(SprayWorldError::InvalidShield(shield));
            }
        }
        let query = entities
            .iter()
            .map(|e| e.query_entity())
            .collect::<Vec<_>>();
        let grid = EntityQueryGrid::build(&query).map_err(SprayWorldError::Query)?;
        Ok(Self {
            scene,
            entities,
            shield_order,
            grid,
            effects,
            query_error: None,
            rails: None,
        })
    }
    /// Host Railgun beams created by casters in this world.
    pub fn with_rails(mut self, rails: &'a mut magicka_sim::railgun::RailSystem) -> Self {
        self.rails = Some(rails);
        self
    }
    fn entity(&self, handle: u16) -> Option<&WorldEntity> {
        self.entities.iter().find(|e| e.handle == handle)
    }
}
impl<H: SprayEffects> SprayWorld for RuntimeSprayWorld<'_, H> {
    fn scene_intersection(&mut self, segment: Segment) -> Option<f32> {
        self.scene?
            .segment_intersect(segment, false)
            .map(|hit| hit.hit.fraction)
    }
    fn shields(&mut self) -> Vec<u16> {
        self.shield_order.to_vec()
    }
    fn shield_intersection(&mut self, handle: u16, segment: Segment) -> Option<f32> {
        match self.entity(handle)?.geometry {
            ArcGeometry::Shield(s) => s.segment_fraction(segment),
            _ => None,
        }
    }
    fn targets(&mut self, center: Vector, range: f32) -> Vec<SprayTarget> {
        let query = self
            .entities
            .iter()
            .map(|e| e.query_entity())
            .collect::<Vec<_>>();
        let result = self
            .grid
            .query(&query, center, range, true, false, |i, segment| {
                let hit = match self.entities[i].geometry {
                    ArcGeometry::Shield(s) => s.segment_fraction(segment),
                    _ => None,
                };
                (
                    hit.is_some(),
                    hit.map_or([0.0; 3], |t| {
                        std::array::from_fn(|axis| segment.origin[axis] + segment.delta[axis] * t)
                    }),
                )
            });
        match result {
            Ok(indices) => indices
                .into_iter()
                .map(|i| SprayTarget {
                    handle: self.entities[i].handle,
                    shield: query[i].shield,
                    damageable: query[i].damageable,
                })
                .collect(),
            Err(error) => {
                self.query_error = Some(error);
                vec![]
            }
        }
    }
    fn arc_intersects(
        &mut self,
        target: u16,
        origin: Vector,
        direction: Vector,
        range: f32,
        angle: f32,
        height: f32,
    ) -> bool {
        let Some(entity) = self.entity(target) else {
            return false;
        };
        let query = ArcQuery {
            origin,
            direction,
            range,
            angle,
            height_difference: height,
        };
        match entity.geometry {
            ArcGeometry::Boss => {
                crate::jormungandr::Jormungandr::arc(entity.position, entity.radius, query)
            }
            ArcGeometry::BossZone { ethereal } => {
                if ethereal {
                    None
                } else {
                    crate::jormungandr::Jormungandr::arc(entity.position, entity.radius, query)
                }
            }
            ArcGeometry::Entity
            | ArcGeometry::MagickDeath
            | ArcGeometry::ElementalEgg
            | ArcGeometry::Barrier { .. }
            | ArcGeometry::Mine => {
                arc_intersection::entity_arc(entity.position, entity.radius, query)
            }
            ArcGeometry::Dispenser | ArcGeometry::Missile => arc_intersection::entity_arc(
                entity.position,
                entity.radius,
                ArcQuery {
                    height_difference: f32::INFINITY,
                    ..query
                },
            ),
            ArcGeometry::Character { ethereal } => {
                arc_intersection::character_arc(entity.position, entity.radius, ethereal, query)
            }
            ArcGeometry::Shield(s) => s.arc_intersect(query),
        }
        .is_some()
    }
    fn damage(
        &mut self,
        target: u16,
        attacker: u16,
        damages: [Damage; 5],
        timestamp: f64,
        position: Vector,
    ) {
        self.effects.damage(
            self.entities,
            SprayDamage {
                target,
                attacker,
                damages,
                timestamp,
                position,
            },
        );
    }
    fn electrocute(&mut self, target: u16, attacker: u16, multiplier: f32) {
        self.effects
            .electrocute(self.entities, target, attacker, multiplier);
    }
}

/// A world with nothing to hit (CastUpdate of a caster without a spell).
pub struct NoSprayWorld;
impl SprayWorld for NoSprayWorld {
    fn scene_intersection(&mut self, _: Segment) -> Option<f32> {
        None
    }
    fn shields(&mut self) -> Vec<u16> {
        Vec::new()
    }
    fn shield_intersection(&mut self, _: u16, _: Segment) -> Option<f32> {
        None
    }
    fn targets(&mut self, _: Vector, _: f32) -> Vec<SprayTarget> {
        Vec::new()
    }
    fn arc_intersects(&mut self, _: u16, _: Vector, _: Vector, _: f32, _: f32, _: f32) -> bool {
        false
    }
    fn damage(&mut self, _: u16, _: u16, _: [Damage; 5], _: f64, _: Vector) {}
    fn electrocute(&mut self, _: u16, _: u16, _: f32) {}
}

/// Push uses the native single-damage overload, independently of spray batches.
pub trait PushEffects: SprayEffects {
    fn single_push(&mut self, entities: &mut [WorldEntity], damage: SprayDamage);
    fn impulse_push(
        &mut self,
        target: u16,
        direction: Vector,
        elevation: f32,
        force: f32,
        distance: f32,
    );
}
impl<H: PushEffects> magicka_sim::push_spell::PushWorld for RuntimeSprayWorld<'_, H> {
    fn push_targets(
        &mut self,
        center: Vector,
        range: f32,
    ) -> Vec<magicka_sim::push_spell::PushTarget> {
        self.targets(center, range)
            .into_iter()
            .filter_map(|t| {
                let e = self.entity(t.handle)?;
                Some(magicka_sim::push_spell::PushTarget {
                    handle: e.handle,
                    position: e.position,
                    dead: e.dead,
                    damageable: e.damageable,
                    missile: matches!(e.geometry, ArcGeometry::Missile),
                })
            })
            .collect()
    }
    fn push_arc(
        &mut self,
        target: u16,
        origin: Vector,
        direction: Vector,
        range: f32,
        angle: f32,
        height: f32,
    ) -> bool {
        self.arc_intersects(target, origin, direction, range, angle, height)
    }
    fn push_obstructed(&mut self, segment: Segment) -> bool {
        self.scene_intersection(segment).is_some()
    }
    fn push_damage(
        &mut self,
        target: u16,
        attacker: u16,
        damage: Damage,
        timestamp: f64,
        position: Vector,
    ) {
        let mut damages = [Damage::default(); 5];
        damages[0] = damage;
        self.effects.single_push(
            self.entities,
            SprayDamage {
                target,
                attacker,
                damages,
                timestamp,
                position,
            },
        );
    }
    fn push_impulse(
        &mut self,
        target: u16,
        direction: Vector,
        elevation: f32,
        force: f32,
        distance: f32,
    ) {
        self.effects
            .impulse_push(target, direction, elevation, force, distance);
    }
}
impl magicka_sim::push_spell::PushWorld for NoSprayWorld {
    fn push_targets(&mut self, _: Vector, _: f32) -> Vec<magicka_sim::push_spell::PushTarget> {
        vec![]
    }
    fn push_arc(&mut self, _: u16, _: Vector, _: Vector, _: f32, _: f32, _: f32) -> bool {
        false
    }
    fn push_obstructed(&mut self, _: Segment) -> bool {
        false
    }
    fn push_damage(&mut self, _: u16, _: u16, _: Damage, _: f64, _: Vector) {}
    fn push_impulse(&mut self, _: u16, _: Vector, _: f32, _: f32, _: f32) {}
}

impl<H: SprayEffects> magicka_sim::lightning::LightningScene for RuntimeSprayWorld<'_, H> {
    fn lightning_targets(
        &mut self,
        center: Vector,
        range: f32,
    ) -> Vec<magicka_sim::lightning::LightningTarget> {
        self.targets(center, range)
            .into_iter()
            .filter_map(|t| {
                let e = self.entity(t.handle)?;
                Some(magicka_sim::lightning::LightningTarget {
                    handle: e.handle,
                    position: e.position,
                    dead: e.dead,
                    eligible: e.damageable
                        && match e.geometry {
                            ArcGeometry::Character { ethereal } => !ethereal,
                            ArcGeometry::Shield(_)
                            | ArcGeometry::Boss
                            | ArcGeometry::BossZone { ethereal: false }
                            | ArcGeometry::Entity
                            | ArcGeometry::ElementalEgg => true,
                            ArcGeometry::Barrier { solid } => solid,
                            ArcGeometry::Dispenser
                            | ArcGeometry::BossZone { ethereal: true }
                            | ArcGeometry::Mine
                            | ArcGeometry::MagickDeath
                            | ArcGeometry::Missile => false,
                        },
                    frozen: e.frozen,
                    ends_chain: matches!(
                        e.geometry,
                        ArcGeometry::Shield(_) | ArcGeometry::Barrier { .. }
                    ),
                })
            })
            .collect()
    }
    fn lightning_shielded(&mut self, segment: Segment) -> bool {
        self.shield_order
            .iter()
            .any(|&h| self.shield_intersection(h, segment).is_some())
    }
    fn lightning_scene(&mut self, segment: Segment) -> Option<f32> {
        self.scene_intersection(segment)
    }
    fn lightning_damage(
        &mut self,
        target: u16,
        attacker: u16,
        damages: [Damage; 5],
        timestamp: f64,
        position: Vector,
    ) {
        self.effects.damage(
            self.entities,
            SprayDamage {
                target,
                attacker,
                damages,
                timestamp,
                position,
            },
        );
    }
}
impl magicka_sim::lightning::LightningScene for NoSprayWorld {
    fn lightning_targets(
        &mut self,
        _: Vector,
        _: f32,
    ) -> Vec<magicka_sim::lightning::LightningTarget> {
        vec![]
    }
    fn lightning_shielded(&mut self, _: Segment) -> bool {
        false
    }
    fn lightning_scene(&mut self, _: Segment) -> Option<f32> {
        None
    }
    fn lightning_damage(&mut self, _: u16, _: u16, _: [Damage; 5], _: f64, _: Vector) {}
}

/// Without a hosted pool a beam is dead at once (the spell then ends).
impl<H: SprayEffects> magicka_sim::railgun::RailControl for RuntimeSprayWorld<'_, H> {
    fn rail_start(&mut self, start: magicka_sim::railgun::RailStart) -> usize {
        self.rails
            .as_mut()
            .map_or(usize::MAX, |r| r.rail_start(start))
    }
    fn rail_steer(&mut self, id: usize, position: Vector, direction: Vector) {
        if let Some(r) = self.rails.as_mut() {
            r.rail_steer(id, position, direction);
        }
    }
    fn rail_dead(&self, id: usize) -> bool {
        self.rails.as_ref().is_none_or(|r| r.rail_dead(id))
    }
    fn rail_kill(&mut self, id: usize) {
        if let Some(r) = self.rails.as_mut() {
            r.rail_kill(id);
        }
    }
}
impl magicka_sim::railgun::RailControl for NoSprayWorld {
    fn rail_start(&mut self, _: magicka_sim::railgun::RailStart) -> usize {
        usize::MAX
    }
    fn rail_steer(&mut self, _: usize, _: Vector, _: Vector) {}
    fn rail_dead(&self, _: usize) -> bool {
        true
    }
    fn rail_kill(&mut self, _: usize) {}
}
