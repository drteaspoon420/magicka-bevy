//! Live Force projectiles (MissileEntity) in a play scene.
//!
//! Original logic comes from `magicka-sim`: the launch controller
//! (`projectile_launch`), condition recipes (`missile_events`), OnCollision
//! decisions and the non-homing Update (`missile_collision`). The flight and
//! contact step that JigLibX performed is a documented reimplementation
//! (research/live-missiles.md): gravity-only integration of the original
//! spawn velocity, native round-shield contacts, swept contacts against
//! level collision and character capsules, and a restitution/friction response with the missile skin's
//! MaterialProperties(0.1, 0.8, 0.8). Contacts still go through the original
//! OnCollision predicate; damage goes through the public route.
use crate::npc_combat::{CombatView, Combatant};
use magicka_sim::{
    damage::Damage,
    impact_motion::Vector,
    missile_collision::{
        CollisionTarget, Contact, MissileCollisionHost, MissileCollisionState, StaticContact,
        TargetKind,
    },
    missile_events::{EffectTable, MissileEvent, MissileEventHost, MissileModel, MissileRecipe},
    projectile_launch::ProjectileLaunch,
    rigid_body::Gravity,
    spells::Elements as E,
    spray_pulse::Segment,
};

/// JigLibX PhysicsSystem.collToll (Magicka keeps the default): contacts
/// are generated within this distance before penetration.
pub const COLLISION_TOLERANCE: f32 = 0.05;
/// Missile skin MaterialProperties(elasticity, static, dynamic roughness).
pub const RESTITUTION: f32 = 0.1;
pub const FRICTION: f32 = 0.8;
/// MissileEntity constructor: SetMass(4), retained across initialization.
pub const MISSILE_MASS: f32 = 4.0;

fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: Vector, s: f32) -> Vector {
    a.map(|x| x * s)
}
fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn length(a: Vector) -> f32 {
    dot(a, a).sqrt()
}

/// Defines.PROJECTILE_* effect names by element index.
const TRAIL_BIG: [&str; 11] = [
    "projectile_earth4",
    "projectile_water4",
    "projectile_cold4",
    "projectile_fire4",
    "projectile_lightning",
    "projectile_arcane4",
    "projectile_life4",
    "projectile_fire4",
    "projectile_ice4",
    "projectile_steam4",
    "projectile_poison4",
];
const TRAIL_SMALL: [&str; 11] = [
    "projectiletrail_earth",
    "projectiletrail_water",
    "projectiletrail_cold",
    "projectiletrail_fire",
    "projectiletrail_lightning",
    "projectiletrail_arcane",
    "projectiletrail_life",
    "projectiletrail_fire",
    "projectiletrail_ice",
    "projectiletrail_steam",
    "projectiletrail_poison",
];
const HIT_SMALL: [&str; 11] = [
    "projectilehit_earth",
    "projectilehit_water",
    "projectilehit_cold",
    "projectilehit_fire",
    "projectilehit_lightning",
    "projectilehit_arcane",
    "projectilehit_life",
    "projectilehit_fire",
    "projectilehit_ice",
    "projectilehit_steam",
    "projectilehit_poison",
];
const HIT_BIG: [&str; 11] = [
    "projectilehit_big_earth",
    "projectilehit_big_water",
    "projectilehit_big_cold",
    "projectilehit_big_fire",
    "projectilehit_big_lightning",
    "projectilehit_big_arcane",
    "projectilehit_big_life",
    "projectilehit_big_fire",
    "projectilehit_big_ice",
    "projectilehit_big_steam",
    "projectilehit_big_poison",
];

/// The effect name of a PlayEffectEvent table entry.
pub fn effect_name(table: EffectTable, element: E) -> Option<&'static str> {
    let index = (0..11).find(|i| element.0 == 1 << i)?;
    Some(match table {
        EffectTable::TrailBig => TRAIL_BIG[index],
        EffectTable::TrailSmall => TRAIL_SMALL[index],
        EffectTable::HitBig => HIT_BIG[index],
        EffectTable::HitSmall => HIT_SMALL[index],
    })
}

/// `Models/Missiles/<name>` for a recipe model.
pub fn model_name(model: MissileModel) -> String {
    match model {
        MissileModel::Earth(n) => format!("Earth{n}"),
        MissileModel::IceEarth(n) => format!("IceEarth{n}"),
        MissileModel::Ice3 => "Ice3".into(),
    }
}

/// Presentation requests (effects are owned by the session).
#[derive(Clone, Debug, PartialEq)]
pub enum MissileVisual {
    Blast {
        position: Vector,
        radius: f32,
        damages: [Damage; 5],
    },
    Spawned {
        missile: u32,
        model: Option<MissileModel>,
    },
    /// EffectManager.StartEffect(name, position, direction); `follow` keeps
    /// the reference on the missile.
    Effect {
        missile: u32,
        name: &'static str,
        position: Vector,
        direction: Vector,
        follow: bool,
    },
    /// Followed effects: UpdatePositionDirection(position, Forward).
    Follow {
        missile: u32,
        position: Vector,
        direction: Vector,
    },
    Sound {
        missile: u32,
        bank: u16,
        name: &'static str,
        magnitude: f32,
        position: Vector,
        stop_on_remove: bool,
    },
    Removed {
        missile: u32,
    },
}

#[derive(Clone, Debug)]
pub struct MissileBody {
    pub id: u32,
    pub owner: u16,
    pub position: Vector,
    pub old_position: Vector,
    pub velocity: Vector,
    pub radius: f32,
    pub model: Option<MissileModel>,
    /// An item projectile model (Content-relative, no extension).
    pub item_model: Option<String>,
    /// Body.ApplyGravity = false (e.g. Fafnir's fireball).
    pub no_gravity: bool,
    pub dead: bool,
    /// Homing missiles (JudgementSpray): target, power, tolerance.
    pub homing: Option<magicka_sim::missile_homing::Homing>,
}

impl MissileBody {
    /// Entity.AddImpulseVelocity replaces the rigid-body velocity. Unlike
    /// characters, missiles do not accumulate an impulse in a motion buffer.
    pub fn push(&mut self, direction: Vector, elevation: f32, force: f32, distance: f32) {
        if self.dead {
            return;
        }
        let velocity = magicka_sim::impact_motion::MotionContext {
            position: self.position,
            mass: MISSILE_MASS,
            gravity_y: Gravity::magicka().vector[1],
            ..Default::default()
        }
        .entity_impulse(direction, elevation, force, distance);
        if dot(velocity, velocity) > 1e-6 {
            self.velocity = velocity;
        }
    }

    /// MissileEntity.GetOrientation: Forward follows velocity, with world Up
    /// defining roll. This is a render transform, independent of body rotation.
    /// Native normalization is undefined for stationary or exactly vertical
    /// flight; retain the finite body basis (identity in this solver) there.
    pub fn render_orientation(&self) -> magicka_sim::character_physics::Orientation {
        Self::orientation_for(self.velocity)
    }
    /// `render_orientation` for a velocity (network clients' host missiles).
    pub fn orientation_for(velocity: Vector) -> magicka_sim::character_physics::Orientation {
        use magicka_sim::character_physics::Orientation;
        let speed = length(velocity);
        if speed == 0.0 {
            return Orientation::IDENTITY;
        }
        let forward = scale(velocity, speed.recip());
        let right = [-forward[2], 0.0, forward[0]];
        let right_length = length(right);
        if right_length == 0.0 {
            return Orientation::IDENTITY;
        }
        let right = scale(right, right_length.recip());
        let up = [
            -right[2] * forward[1],
            right[2] * forward[0] - right[0] * forward[2],
            right[0] * forward[1],
        ];
        Orientation {
            right,
            up: scale(up, length(up).recip()),
            backward: scale(forward, -1.0),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LiveMissile {
    /// Shared world identity for IDamageable spell queries.
    pub handle: Option<u16>,
    pub body: MissileBody,
    pub state: MissileCollisionState,
}

impl LiveMissile {
    /// ProjectileSpell.SpawnMissile + MissileEntity.Initialize: recipe,
    /// radius 0.075 + 0.075·Earth, Velocity = launch velocity, owner in
    /// the hit list (canHitOwner), Default conditions executed at once by
    /// the caller.
    pub fn spawn(
        id: u32,
        owner: u16,
        owner_is_local_avatar: bool,
        launch: &ProjectileLaunch,
    ) -> Option<Self> {
        let recipe =
            MissileRecipe::from_spell(launch.spell, launch.splash, launch.total_projectiles)
                .ok()?;
        let deep_impact = launch.spell.element == E::EARTH && owner_is_local_avatar;
        let state = MissileCollisionState::new(
            Some(owner),
            recipe.can_hit_owner,
            recipe.conditions,
            deep_impact,
        );
        Some(Self {
            handle: None,
            body: MissileBody {
                id,
                owner,
                position: launch.position,
                old_position: launch.position,
                velocity: launch.velocity,
                radius: recipe.radius,
                model: recipe.model,
                item_model: None,
                no_gravity: false,
                dead: false,
                homing: None,
            },
            state,
        })
    }
}

impl LiveMissile {
    /// MissileEntity.Initialize with explicit conditions (ProppMagick and
    /// other scripted missiles): no model, no deep impact.
    pub fn spawn_custom(
        id: u32,
        owner: u16,
        position: Vector,
        velocity: Vector,
        radius: f32,
        can_hit_owner: bool,
        conditions: magicka_sim::missile_events::MissileConditions,
    ) -> Self {
        Self {
            handle: None,
            body: MissileBody {
                id,
                owner,
                position,
                old_position: position,
                velocity,
                radius,
                model: None,
                item_model: None,
                no_gravity: false,
                dead: false,
                homing: None,
            },
            state: MissileCollisionState::new(Some(owner), can_hit_owner, conditions, false),
        }
    }
}

/// The world the missile host acts on.
pub trait MissileWorld {
    fn props(&self) -> Vec<MissileProp> {
        Vec::new()
    }
    fn damage_prop(
        &mut self,
        _target: u16,
        _damage: Damage,
        _attacker: u16,
        _position: Vector,
    ) -> u16 {
        0
    }
    /// Dedicated non-response entities still receive both native callbacks.
    fn callback_capsules(&self) -> Vec<MissileCallbackCapsule> {
        Vec::new()
    }
    fn view(&self) -> &CombatView<'_>;
    /// EntityManager shield order, including shields fading at zero HP.
    fn shields(&self) -> Vec<MissileShield>;
    fn damage_shield(&mut self, target: u16, damage: Damage) -> u16;
    /// Stand-in for HollowSphereBox's unseeded Random.Next(360).
    fn collision_random_degrees(&mut self) -> u16;
    /// Character.Damage through the public route; returns DamageResult.
    fn damage_character(
        &mut self,
        target: u16,
        damage: Damage,
        attacker: u16,
        position: Vector,
    ) -> u16;
    fn scene_hit(&self, origin: Vector, delta: Vector) -> bool;
    /// Record a collection FullBlast for non-character damageables (mines,
    /// barriers) the host applies afterwards.
    fn record_blast(&mut self, _position: Vector, _radius: f32, _damages: [Damage; 5]) {}
    fn record_splash(
        &mut self,
        _position: Vector,
        _radius: f32,
        _damage: Damage,
        _owner: u16,
        _ignore: Option<u16>,
    ) -> bool {
        false
    }
    /// Live Solid barriers' primitive-0 capsules (Barrier.OnCollision's
    /// physical primitive).
    fn barriers(&self) -> Vec<MissileBarrier> {
        Vec::new()
    }
    /// Barrier.InternalDamage for a missile damage event; returns the
    /// DamageResult.
    fn damage_barrier(&mut self, _target: u16, _damage: Damage) -> u16 {
        0
    }
}

/// A Solid barrier as a missile target: a vertical capsule axis from
/// `bottom`, `length` long.
#[derive(Clone, Copy, Debug)]
pub struct MissileBarrier {
    pub handle: u16,
    pub position: Vector,
    pub solid: bool,
    /// Primitive 0 (physical for Solid barriers).
    pub bottom: Vector,
    pub length: f32,
    pub radius: f32,
    /// Primitive 1: the sphere at Position − mRadius, radius 2·mRadius
    /// (Solid) or mRadius. Barrier.OnCollision rejects it, but JigLibX
    /// still runs the missile's callback.
    pub sphere_center: Vector,
    pub sphere_radius: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct MissileCallbackCapsule {
    pub handle: u16,
    pub old_axis: Segment,
    pub new_axis: Segment,
    pub radius: f32,
    pub old_body_position: Vector,
    /// Damage zone with an immovable contact surface (Jormungandr's head).
    /// Callback-only entities such as Death do not stop the projectile.
    pub solid: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct MissileProp {
    pub handle: u16,
    pub position: Vector,
    pub bounds: magicka_sim::box_intersection::OrientedBox,
    pub solid: bool,
    pub dead: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct MissileShield {
    pub handle: u16,
    pub position: Vector,
    pub radius: f32,
    pub geometry: magicka_sim::shield_intersection::ShieldGeometry,
    pub dead: bool,
}

/// MissileCollisionHost over one missile and the world.
pub struct MissileHost<'a, W: MissileWorld> {
    pub body: &'a mut MissileBody,
    pub world: &'a mut W,
    pub visuals: &'a mut Vec<MissileVisual>,
    pub target: Option<u16>,
    /// The missile's own world handle, excluded from its splash queries.
    pub entity_handle: Option<u16>,
}

impl<W: MissileWorld> MissileHost<'_, W> {
    /// Blast.BlastDamage (collection form) with falloff: characters within
    /// the radius (minus their radius) and in scene line of sight. Water
    /// electrocution and Liquid.Freeze are not modelled.
    fn blast(&mut self, radius: f32, position: Vector, damages: [Damage; 5]) -> u16 {
        self.world.record_blast(position, radius, damages);
        let beam_collection = damages.iter().any(|d| d.element & 0x60 != 0);
        let targets: Vec<Combatant> = {
            let view = self.world.view();
            view.entities(position, radius, true)
                .into_iter()
                .map(|i| view.combatants[i])
                .collect()
        };
        let mut result = 0;
        for c in targets {
            if c.ethereal || c.dead {
                continue;
            }
            let delta = crate::npc_combat::blast_target_delta(&c, position);
            if self.world.scene_hit(position, delta) {
                continue;
            }
            let distance = length(delta) - c.radius;
            if distance <= radius {
                let f = (distance / radius).max(0.0);
                let falloff = 1.0 - f * f * f;
                for d in damages.iter().filter(|d| d.attack != 0) {
                    let mut d = *d;
                    if !beam_collection {
                        d.magnitude *= falloff;
                    }
                    result |= self
                        .world
                        .damage_character(c.handle, d, self.body.owner, position);
                }
            }
        }
        // Shield implements IDamageable and participates in the original
        // entity query too. Keep the same collection falloff and scene test.
        for shield in self.world.shields().into_iter().filter(|s| !s.dead) {
            let mut delta = sub(shield.position, position);
            // Entity query precedes skin clipping. A far-away shield must
            // not become a candidate merely because its near surface clips.
            if length(delta) - shield.radius > radius {
                continue;
            }
            if let Some(fraction) = shield.geometry.segment_fraction(Segment {
                origin: position,
                delta,
            }) {
                delta = scale(delta, fraction);
            }
            let distance = length(delta) - shield.radius;
            if distance > radius || self.world.scene_hit(position, delta) {
                continue;
            }
            let f = (distance / radius).max(0.0);
            for damage in damages.iter().filter(|d| d.attack != 0) {
                let mut damage = *damage;
                if !beam_collection {
                    damage.magnitude *= 1.0 - f * f * f;
                }
                result |= self.world.damage_shield(shield.handle, damage);
            }
        }
        result
    }
}

impl<W: MissileWorld> MissileEventHost for MissileHost<'_, W> {
    fn damageable_target(&self) -> bool {
        self.target.is_some_and(|t| {
            self.world.view().get(t).is_some_and(|c| !c.dead)
                || self
                    .world
                    .shields()
                    .iter()
                    .any(|s| s.handle == t && !s.dead)
                || self.world.barriers().iter().any(|b| b.handle == t)
                || self.world.props().iter().any(|p| p.handle == t && !p.dead)
                || self.world.callback_capsules().iter().any(|p| p.handle == t)
        })
    }
    fn normalized_velocity(&self) -> f32 {
        length(self.body.velocity) / 50.0
    }
    fn damage(&mut self, damage: Damage) -> u16 {
        let Some(target) = self.target else {
            return 0;
        };
        let position = self.body.position;
        if self.world.shields().iter().any(|s| s.handle == target) {
            return self.world.damage_shield(target, damage);
        }
        if self.world.barriers().iter().any(|b| b.handle == target) {
            return self.world.damage_barrier(target, damage);
        }
        if self.world.props().iter().any(|p| p.handle == target) {
            return self
                .world
                .damage_prop(target, damage, self.body.owner, position);
        }
        self.world
            .damage_character(target, damage, self.body.owner, position)
    }
    fn kill(&mut self) {
        self.body.dead = true;
    }
    fn event(&mut self, event: &MissileEvent, position: &mut Option<Vector>) {
        match event {
            MissileEvent::Effect {
                table,
                element,
                follow,
            } => {
                if let Some(name) = effect_name(*table, *element) {
                    // PlayEffectEvent uses MissileEntity.GetOrientation.Forward.
                    self.visuals.push(MissileVisual::Effect {
                        missile: self.body.id,
                        name,
                        position: position.unwrap_or(self.body.position),
                        direction: self.body.render_orientation().forward(),
                        follow: *follow,
                    });
                }
            }
            MissileEvent::NamedEffect {
                name,
                follow,
                world_align,
            } => {
                // PlayEffectEvent.Execute 0x060022b4: iWorldAlign keeps
                // (0, 1, 0); otherwise GetOrientation.Forward.
                let direction = if *world_align {
                    [0.0, 1.0, 0.0]
                } else {
                    self.body.render_orientation().forward()
                };
                self.visuals.push(MissileVisual::Effect {
                    missile: self.body.id,
                    name,
                    position: position.unwrap_or(self.body.position),
                    direction,
                    follow: *follow,
                });
            }
            MissileEvent::Splash { radius, damage } => {
                let at = position.unwrap_or(self.body.position);
                if self.world.record_splash(
                    at,
                    *radius,
                    *damage,
                    self.body.owner,
                    self.entity_handle,
                ) {
                    return;
                }
                let targets: Vec<_> = self
                    .world
                    .view()
                    .entities(at, *radius, true)
                    .into_iter()
                    .map(|i| self.world.view().combatants[i])
                    .collect();
                for target in targets {
                    if target.dead {
                        continue;
                    }
                    if target.ethereal {
                        continue;
                    }
                    let Some(falloff) = magicka_sim::spray_weapon::target_scale(
                        at,
                        [0.0, 0.0, -1.0],
                        *radius,
                        std::f32::consts::PI,
                        target.position,
                        target.radius,
                    ) else {
                        continue;
                    };
                    let mut d = *damage;
                    d.magnitude *= falloff;
                    if d.magnitude > 0.0 {
                        self.world
                            .damage_character(target.handle, d, self.body.owner, at);
                    }
                }
                let mut slots = [Damage::default(); 5];
                slots[0] = *damage;
                self.world.record_blast(at, *radius, slots);
            }
            MissileEvent::Blast { radius, damages } => {
                let at = position.unwrap_or(self.body.old_position);
                self.visuals.push(MissileVisual::Blast {
                    position: at,
                    radius: *radius,
                    damages: *damages,
                });
                self.blast(*radius, at, *damages);
            }
            MissileEvent::NamedSound {
                bank,
                name,
                magnitude,
                stop_on_remove,
            } => {
                self.visuals.push(MissileVisual::Sound {
                    missile: self.body.id,
                    bank: *bank,
                    name,
                    magnitude: *magnitude,
                    position: position.unwrap_or(self.body.position),
                    stop_on_remove: *stop_on_remove,
                });
            }
            // Crater decals, lights, table sounds and camera shake are not
            // presented yet.
            _ => {}
        }
    }
}

impl<W: MissileWorld> MissileCollisionHost for MissileHost<'_, W> {
    fn position(&self) -> Vector {
        self.body.position
    }
    fn velocity(&self) -> Vector {
        self.body.velocity
    }
    fn set_velocity(&mut self, velocity: Vector) {
        self.body.velocity = velocity;
    }
    fn target_position(&self, handle: u16) -> Vector {
        self.world
            .view()
            .get(handle)
            .map(|c| c.position)
            .or_else(|| {
                self.world
                    .shields()
                    .iter()
                    .find(|s| s.handle == handle)
                    .map(|s| s.position)
            })
            .or_else(|| {
                self.world
                    .barriers()
                    .iter()
                    .find(|b| b.handle == handle)
                    .map(|b| b.position)
            })
            .or_else(|| {
                self.world
                    .props()
                    .iter()
                    .find(|p| p.handle == handle)
                    .map(|p| p.position)
            })
            .unwrap_or(self.body.position)
    }
    fn select_target(&mut self, target: Option<u16>) {
        self.target = target;
    }
    fn steer(&mut self, dt: f32) {
        let Some(mut homing) = self.body.homing else {
            return;
        };
        let target = homing
            .target
            .and_then(|h| self.world.view().get(h).filter(|c| !c.dead))
            .map(|c| (c.position, c.radius));
        self.body.velocity = homing.steer(target, self.body.position, self.body.velocity, dt);
        self.body.homing = Some(homing);
    }
    fn update_followed_effects(&mut self) {
        self.visuals.push(MissileVisual::Follow {
            missile: self.body.id,
            position: self.body.position,
            direction: self.body.render_orientation().forward(),
        });
    }
    fn award_deep_impact(&mut self) {}
}

/// A discrete round-shield contact or the earliest level/character sweep hit.
enum Hit {
    Boss {
        fraction: f32,
        target: u16,
        normal: Vector,
    },
    Level {
        fraction: f32,
        normal: Vector,
    },
    Character {
        fraction: f32,
        target: u16,
    },
    Shield {
        target: u16,
        normal: Vector,
    },
    Barrier {
        fraction: f32,
        target: u16,
    },
    Prop {
        normal: Vector,
        target: u16,
    },
}

/// One physics step for a missile (reimplemented JigLibX flight): gravity
/// on the velocity, then native round-shield old/new detection before swept
/// level/character tests. An accepted contact (original OnCollision) stops
/// at the old position for shields, or sweep position otherwise, and applies
/// the simplified material response. Shield contacts then integrate the
/// resulting velocity; a rejected contact passes using the original delta.
pub fn physics_step<W: MissileWorld>(
    missile: &mut LiveMissile,
    dt: f32,
    gravity: Gravity,
    world: &mut W,
    scene: Option<&crate::level_collision::SceneCollision>,
    visuals: &mut Vec<MissileVisual>,
) {
    // MagickDeath.OnCollision returns false, but JigLibX still calls the
    // missile callback. Detect the native capsule/box pair and run its damage
    // conditions without adding a physical response or blocking other hits.
    let gravity = if missile.body.no_gravity {
        Gravity {
            vector: [0.0; 3],
            ..gravity
        }
    } else {
        gravity
    };
    let predicted_velocity = add(missile.body.velocity, scale(gravity.vector, dt));
    let predicted_delta = scale(predicted_velocity, dt);
    for capsule in world.callback_capsules().into_iter().filter(|c| !c.solid) {
        if missile.body.dead || missile.state.excluded_owner == Some(capsule.handle) {
            continue;
        }
        let position = missile.body.position;
        let box_at = |center| magicka_sim::box_intersection::OrientedBox {
            center,
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            side_lengths: [missile.body.radius * 2.0; 3],
        };
        let contact = magicka_sim::capsule_box::detect(
            magicka_sim::capsule_pair::CapsuleMotion {
                old_axis: capsule.old_axis,
                new_axis: capsule.new_axis,
                old_radius: capsule.radius,
                new_radius: capsule.radius,
                old_body_position: capsule.old_body_position,
            },
            magicka_sim::hollow_sphere_box::PrimitiveMotion {
                old: box_at(position),
                new: box_at(add(position, predicted_delta)),
                owner_old_position: position,
            },
            COLLISION_TOLERANCE,
            || world.collision_random_degrees(),
        );
        if contact.is_some() {
            let mut host = MissileHost {
                entity_handle: missile.handle,
                body: &mut missile.body,
                world,
                visuals,
                target: None,
            };
            missile.state.on_collision(
                Contact::Body {
                    physics_object: false,
                    target: Some(CollisionTarget {
                        handle: capsule.handle,
                        kind: TargetKind::Other,
                        dead: false,
                        damageable: true,
                    }),
                },
                true,
                &mut host,
            );
        }
    }
    // Barrier sphere primitives: callback-only contacts, like Death's.
    for b in world.barriers() {
        if missile.body.dead {
            break;
        }
        let start = missile.body.position;
        let reach = b.sphere_radius + missile.body.radius + COLLISION_TOLERANCE;
        if point_segment_distance(b.sphere_center, start, predicted_delta) > reach {
            continue;
        }
        let mut host = MissileHost {
            entity_handle: missile.handle,
            body: &mut missile.body,
            world,
            visuals,
            target: None,
        };
        missile.state.on_collision(
            Contact::Body {
                physics_object: false,
                target: Some(CollisionTarget {
                    handle: b.handle,
                    kind: TargetKind::Barrier { solid: b.solid },
                    dead: false,
                    damageable: true,
                }),
            },
            true,
            &mut host,
        );
    }
    // Prop boxes use the native new-world SAT gate. The missile callback
    // still runs for a non-solid prop; its veto suppresses physical response.
    let mut prop_contact = None;
    for prop in world.props() {
        if missile.body.dead || prop.dead || missile.state.excluded_owner == Some(prop.handle) {
            continue;
        }
        let new_box = magicka_sim::box_intersection::OrientedBox {
            center: add(missile.body.position, predicted_delta),
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            side_lengths: [missile.body.radius * 2.0; 3],
        };
        let Some(contact) =
            magicka_sim::box_box::overlap(new_box, prop.bounds, COLLISION_TOLERANCE)
        else {
            continue;
        };
        let mut host = MissileHost {
            entity_handle: missile.handle,
            body: &mut missile.body,
            world,
            visuals,
            target: None,
        };
        let accepted = missile.state.on_collision(
            Contact::Body {
                physics_object: true,
                target: Some(CollisionTarget {
                    handle: prop.handle,
                    kind: TargetKind::Other,
                    dead: prop.dead,
                    damageable: true,
                }),
            },
            true,
            &mut host,
        );
        if accepted && prop.solid && prop_contact.is_none() {
            prop_contact = Some(Hit::Prop {
                normal: contact.normal,
                target: prop.handle,
            });
        }
    }
    if missile.body.dead {
        return;
    }
    let body = &mut missile.body;
    body.old_position = body.position;
    body.velocity = add(body.velocity, scale(gravity.vector, dt));
    let delta = scale(body.velocity, dt);
    let travel = length(delta);
    let mut best: Option<Hit> = prop_contact;
    // Native round-shield contact generation uses old/new box centers,
    // not a ray expanded by the missile radius. The live shields are static.
    // These discrete contacts are handled at the start of the step before
    // the existing swept level/character path (documented solver divergence).
    for shield in world.shields().into_iter().filter(|s| !s.dead) {
        use magicka_sim::{
            box_intersection::OrientedBox,
            hollow_sphere_box::{detect, PrimitiveMotion},
            shield_intersection::ShieldGeometry,
        };
        let ShieldGeometry::Sphere(sphere) = shield.geometry else {
            continue;
        };
        let old = OrientedBox {
            center: body.position,
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            side_lengths: [body.radius * 2.0; 3],
        };
        if let Some(contact) = detect(
            PrimitiveMotion {
                old: sphere,
                new: sphere,
                owner_old_position: shield.position,
            },
            PrimitiveMotion {
                old,
                new: OrientedBox {
                    center: add(body.position, delta),
                    ..old
                },
                owner_old_position: body.position,
            },
            COLLISION_TOLERANCE,
            || world.collision_random_degrees(),
        ) {
            best = Some(Hit::Shield {
                target: shield.handle,
                // Native normal is toward skin 0 (sphere); the missile is
                // skin 1, so its response uses the opposite normal.
                normal: contact.normal.map(|v| -v),
            });
            break;
        }
    }
    if travel > 0.0 && best.is_none() {
        let dir = scale(delta, 1.0 / travel);
        // Level: extend by the box half size plus the collision tolerance.
        let margin = body.radius + COLLISION_TOLERANCE;
        let reach = add(delta, scale(dir, margin));
        if let Some(hit) = scene.and_then(|s| {
            s.segment_intersect(
                Segment {
                    origin: body.position,
                    delta: reach,
                },
                false,
            )
        }) {
            let fraction = (hit.hit.fraction * (travel + margin) - margin).max(0.0) / travel;
            best = Some(Hit::Level {
                fraction: fraction.min(1.0),
                normal: hit.hit.normal,
            });
        }
        let view = world.view();
        for c in view.entities(
            add(body.position, scale(delta, 0.5)),
            travel * 0.5 + body.radius,
            false,
        ) {
            let c = view.combatants[c];
            if c.dead || c.ethereal || missile.state.excluded_owner == Some(c.handle) {
                continue;
            }
            // The missile's collision primitive is an axis-aligned box of
            // half size `radius`. Already overlapping at the start: JigLibX
            // still generates a (penetrating) contact, so the missile
            // hits at once (a self-cast rock falling into its caster's
            // capsule). JigLibX also separates the bodies; the replacement
            // pushes the box horizontally out of the capsule (INFERRED).
            let touch = if box_capsule_distance(&c, body.position, body.radius)
                <= c.radius + COLLISION_TOLERANCE
            {
                if missile.state.hits.contains(c.handle) {
                    continue;
                }
                // Separate horizontally, as the JigLibX contact would.
                let (dx, dz) = (
                    body.position[0] - c.position[0],
                    body.position[2] - c.position[2],
                );
                let l = (dx * dx + dz * dz).sqrt();
                let need = c.radius + body.radius + COLLISION_TOLERANCE - l;
                if l > 1e-4 && need > 0.0 {
                    body.position[0] += dx / l * need;
                    body.position[2] += dz / l * need;
                }
                Some(0.0)
            } else {
                first_box_touch(&c, body.position, delta, body.radius)
            };
            if let Some(fraction) = touch {
                let closer = match &best {
                    Some(Hit::Level { fraction: f, .. })
                    | Some(Hit::Character { fraction: f, .. })
                    | Some(Hit::Barrier { fraction: f, .. })
                    | Some(Hit::Boss { fraction: f, .. }) => fraction < *f,
                    Some(Hit::Shield { .. } | Hit::Prop { .. }) => false,
                    None => true,
                };
                if closer {
                    best = Some(Hit::Character {
                        fraction,
                        target: c.handle,
                    });
                }
            }
        }
    }
    // Solid barriers: the swept box against each primitive-0 capsule.
    if travel > 0.0 && !matches!(best, Some(Hit::Shield { .. })) {
        for b in world.barriers().into_iter().filter(|b| b.solid) {
            let Some(fraction) = first_segment_touch(
                b.bottom,
                b.length,
                b.radius,
                body.position,
                delta,
                body.radius,
            ) else {
                continue;
            };
            let closer = match &best {
                Some(Hit::Level { fraction: f, .. })
                | Some(Hit::Character { fraction: f, .. })
                | Some(Hit::Barrier { fraction: f, .. })
                | Some(Hit::Boss { fraction: f, .. }) => fraction < *f,
                Some(Hit::Shield { .. } | Hit::Prop { .. }) => false,
                None => true,
            };
            if closer {
                best = Some(Hit::Barrier {
                    fraction,
                    target: b.handle,
                });
            }
        }
    }
    for capsule in world.callback_capsules().into_iter().filter(|c| c.solid) {
        if missile.state.excluded_owner == Some(capsule.handle) {
            continue;
        }
        let axis = capsule.new_axis;
        // Current head zone is a sphere, represented by a zero-length capsule.
        // Sweep the same missile box used for characters, including vertical
        // flights that can cross the whole head between two physics ticks.
        let axis_length = length(axis.delta);
        let Some(fraction) = first_segment_touch(
            axis.origin,
            axis_length,
            capsule.radius,
            body.position,
            delta,
            body.radius,
        ) else {
            continue;
        };
        let closer = match best {
            Some(
                Hit::Level { fraction: f, .. }
                | Hit::Character { fraction: f, .. }
                | Hit::Barrier { fraction: f, .. }
                | Hit::Boss { fraction: f, .. },
            ) => fraction < f,
            Some(Hit::Shield { .. } | Hit::Prop { .. }) => false,
            None => true,
        };
        if closer {
            let point = add(body.position, scale(delta, fraction));
            let near = [
                axis.origin[0],
                point[1].clamp(axis.origin[1], axis.origin[1] + axis_length),
                axis.origin[2],
            ];
            let offset = sub(point, near);
            let size = length(offset);
            let normal = if size > 1e-6 {
                scale(offset, 1. / size)
            } else {
                [0., 1., 0.]
            };
            best = Some(Hit::Boss {
                fraction,
                target: capsule.handle,
                normal,
            });
        }
    }
    let Some(hit) = best else {
        body.position = add(body.position, delta);
        return;
    };
    let shield_hit = matches!(hit, Hit::Shield { .. });
    let prop_hit = matches!(hit, Hit::Prop { .. });
    let boss_hit = matches!(hit, Hit::Boss { .. });
    let (contact, fraction, normal) = match hit {
        Hit::Boss {
            fraction,
            target,
            normal,
        } => (
            Contact::Body {
                physics_object: false,
                target: Some(CollisionTarget {
                    handle: target,
                    kind: TargetKind::BossDamage { ethereal: false },
                    dead: false,
                    damageable: true,
                }),
            },
            fraction,
            normal,
        ),
        Hit::Prop { target, normal } => (
            Contact::Body {
                physics_object: true,
                target: Some(CollisionTarget {
                    handle: target,
                    kind: TargetKind::Other,
                    dead: false,
                    damageable: true,
                }),
            },
            0.0,
            normal,
        ),
        Hit::Shield { target, normal } => (
            Contact::Body {
                physics_object: false,
                target: Some(CollisionTarget {
                    handle: target,
                    kind: TargetKind::Shield,
                    dead: false,
                    damageable: true,
                }),
            },
            0.0,
            normal,
        ),
        Hit::Level { fraction, normal } => {
            (Contact::Static(StaticContact::Level), fraction, normal)
        }
        Hit::Barrier { fraction, target } => {
            let b = world
                .barriers()
                .into_iter()
                .find(|b| b.handle == target)
                .expect("barrier");
            let contact_point = add(body.position, scale(delta, fraction));
            let y = contact_point[1].clamp(b.bottom[1], b.bottom[1] + b.length);
            let n = sub(contact_point, [b.bottom[0], y, b.bottom[2]]);
            let l = length(n);
            let normal = if l > 1e-6 {
                scale(n, 1.0 / l)
            } else {
                [0.0, 1.0, 0.0]
            };
            (
                Contact::Body {
                    physics_object: false,
                    target: Some(CollisionTarget {
                        handle: target,
                        kind: TargetKind::Barrier { solid: true },
                        dead: false,
                        damageable: true,
                    }),
                },
                fraction,
                normal,
            )
        }
        Hit::Character { fraction, target } => {
            let c = *world.view().get(target).expect("target");
            let contact_point = add(body.position, scale(delta, fraction));
            let mut n = sub(contact_point, c.position);
            n[1] = 0.0;
            let l = length(n);
            let normal = if l > 1e-6 {
                scale(n, 1.0 / l)
            } else {
                [0.0, 1.0, 0.0]
            };
            (
                Contact::Body {
                    physics_object: false,
                    target: Some(CollisionTarget {
                        handle: target,
                        kind: TargetKind::Character {
                            ethereal: c.ethereal,
                        },
                        dead: c.dead,
                        damageable: true,
                    }),
                },
                fraction,
                normal,
            )
        }
    };
    // Callbacks see the old body position for discrete shield contacts,
    // or the contact-time position for the existing sweep path.
    body.position = add(body.position, scale(delta, fraction));
    let mut host = MissileHost {
        entity_handle: missile.handle,
        body: &mut missile.body,
        world,
        visuals,
        target: None,
    };
    // Prop callbacks already ran for every overlapping box, including the
    // non-solid ones. Only their accepted physical contact reaches this arm.
    let accept = prop_hit || missile.state.on_collision(contact, true, &mut host);
    let body = &mut missile.body;
    if accept {
        // Impulse response: normal impulse (1 + e)·|vn|, Coulomb friction
        // limited to μ times it.
        let vn = dot(body.velocity, normal);
        if vn < 0.0 {
            let normal_part = scale(normal, vn);
            let tangent = sub(body.velocity, normal_part);
            let speed = length(tangent);
            let restitution = if boss_hit { 0. } else { RESTITUTION };
            let impulse = (1.0 + restitution) * -vn;
            // CollisionInfo.Init multiplies material roughness: missile .8
            // times Shield .8 = .64. Restitution is .1 times Shield 1 = .1.
            let friction = if boss_hit {
                0.
            } else if shield_hit {
                FRICTION * 0.8
            } else {
                FRICTION
            };
            let keep = if speed > 1e-6 {
                (1.0 - friction * impulse / speed).max(0.0)
            } else {
                0.0
            };
            body.velocity = add(scale(tangent, keep), scale(normal_part, -restitution));
        }
        if shield_hit || prop_hit {
            // Discrete contacts are detected before position integration.
            // Advance even for a separating cached contact, rather than
            // pinning a bouncing missile in the detector's tolerance band.
            body.position = add(body.position, scale(body.velocity, dt));
        }
    } else {
        body.position = add(body.position, scale(delta, 1.0 - fraction));
    }
}

/// Distance from a character's capsule axis to the axis-aligned box of half
/// size `half` centred at `center` (convex in the axis parameter).
/// Distance from `p` to the segment `origin + t·delta`, t in [0, 1].
fn point_segment_distance(p: Vector, origin: Vector, delta: Vector) -> f32 {
    let dd = dot(delta, delta);
    let t = if dd > 0.0 {
        (dot(sub(p, origin), delta) / dd).clamp(0.0, 1.0)
    } else {
        0.0
    };
    length(sub(p, add(origin, scale(delta, t))))
}

fn box_capsule_distance(c: &Combatant, center: Vector, half: f32) -> f32 {
    let start = [c.position[0], c.position[1] - c.length * 0.5, c.position[2]];
    box_segment_distance(start, c.length, center, half)
}

/// Distance from an axis-aligned box (half size `half`) to a vertical
/// segment of `length` starting at `start`.
fn box_segment_distance(start: Vector, length_y: f32, center: Vector, half: f32) -> f32 {
    let at = |s: f32| {
        let p = [start[0], start[1] + length_y * s, start[2]];
        let d: Vector = std::array::from_fn(|i| ((p[i] - center[i]).abs() - half).max(0.0));
        length(d)
    };
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..40 {
        let (m1, m2) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
        if at(m1) <= at(m2) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    at((lo + hi) * 0.5)
}

/// The fraction along `delta` where the swept box first touches the
/// capsule (distance ≤ capsule radius), if it does: ternary search for the
/// closest approach (the swept distance is convex), then bisection before it.
fn first_box_touch(c: &Combatant, origin: Vector, delta: Vector, half: f32) -> Option<f32> {
    let start = [c.position[0], c.position[1] - c.length * 0.5, c.position[2]];
    first_segment_touch(start, c.length, c.radius, origin, delta, half)
}

fn first_segment_touch(
    start: Vector,
    length_y: f32,
    radius: f32,
    origin: Vector,
    delta: Vector,
    half: f32,
) -> Option<f32> {
    let at = |t: f32| box_segment_distance(start, length_y, add(origin, scale(delta, t)), half);
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..40 {
        let (m1, m2) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
        if at(m1) <= at(m2) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let closest = (lo + hi) * 0.5;
    let reach = radius + COLLISION_TOLERANCE;
    if at(closest) > reach {
        return None;
    }
    let (mut lo, mut hi) = (0.0f32, closest);
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        if at(mid) <= reach {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(hi)
}

/// MissileEntity.Initialize tail: the Default conditions run immediately
/// (trail effects, lights, sounds).
pub fn initialize<W: MissileWorld>(
    missile: &mut LiveMissile,
    world: &mut W,
    visuals: &mut Vec<MissileVisual>,
) {
    visuals.push(MissileVisual::Spawned {
        missile: missile.body.id,
        model: missile.body.model,
    });
    let mut host = MissileHost {
        entity_handle: missile.handle,
        body: &mut missile.body,
        world,
        visuals,
        target: None,
    };
    missile.state.execute_default(&mut host);
}

/// MissileEntity.Update (non-homing) after the physics step.
pub fn update<W: MissileWorld>(
    missile: &mut LiveMissile,
    dt: f32,
    world: &mut W,
    visuals: &mut Vec<MissileVisual>,
) {
    update_at_kill_plane(missile, dt, world, visuals, -50.);
}

pub fn update_at_kill_plane<W: MissileWorld>(
    missile: &mut LiveMissile,
    dt: f32,
    world: &mut W,
    visuals: &mut Vec<MissileVisual>,
    kill_plane: f32,
) {
    let mut host = MissileHost {
        entity_handle: missile.handle,
        body: &mut missile.body,
        world,
        visuals,
        target: None,
    };
    missile
        .state
        .update_without_homing_at_kill_plane(dt, true, kill_plane, &mut host);
}

/// Interned content names for the `&'static str` named missile events
/// (bounded by the distinct effect/cue names in the loaded content).
pub(crate) fn intern(name: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut names = NAMES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(&n) = names.get(name) {
        return n;
    }
    let n: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.insert(n);
    n
}

/// ConditionCollection.CopyTo of an item's ranged conditions into a fresh
/// missile collection. Returns the conditions and the names of content
/// events without a missile host (skipped).
pub fn item_conditions(
    collections: &[magicka_content::common::EventCollection],
) -> (magicka_sim::missile_events::MissileConditions, Vec<String>) {
    use magicka_content::common::Event;
    use magicka_sim::missile_events::{EventCondition, MissileConditions};
    let mut c = MissileConditions::default();
    let mut skipped = Vec::new();
    for (slot, collection) in c.slots.iter_mut().zip(collections) {
        let k = &collection.condition;
        slot.condition = EventCondition {
            flags: k.condition_type,
            hitpoints: k.hitpoints as f32,
            elements: k.elements as u32,
            time: k.time,
            threshold: k.threshold,
            count: 0,
            activated: false,
            repeat: collection.repeat,
        };
        slot.events.clear();
        for event in &collection.events {
            slot.events.push(match event {
                Event::Damage {
                    damage,
                    velocity_based,
                } => MissileEvent::Damage {
                    damage: Damage {
                        attack: damage.attack_property as u16,
                        element: damage.element as u32,
                        amount: damage.amount,
                        magnitude: damage.magnitude,
                    },
                    velocity_based: *velocity_based,
                },
                Event::Splash { damage, radius } => {
                    let mut damages = [Damage::default(); 5];
                    damages[0] = Damage {
                        attack: damage.attack_property as u16,
                        element: damage.element as u32,
                        amount: damage.amount,
                        magnitude: damage.magnitude,
                    };
                    MissileEvent::Splash {
                        radius: *radius,
                        damage: damages[0],
                    }
                }
                Event::Remove { bounce } => MissileEvent::Remove { bounces: *bounce },
                Event::Effect {
                    follow,
                    world_align,
                    effect,
                } => MissileEvent::NamedEffect {
                    name: intern(&effect.to_lowercase()),
                    follow: *follow,
                    world_align: *world_align,
                },
                Event::Sound {
                    bank,
                    sound,
                    stop_on_remove,
                    magnitude,
                } => MissileEvent::NamedSound {
                    bank: *bank as u16,
                    name: intern(sound),
                    magnitude: *magnitude,
                    stop_on_remove: *stop_on_remove,
                },
                Event::Light(light) => MissileEvent::Light {
                    radius: light.radius,
                    diffuse: light.diffuse,
                    ambient: light.ambient,
                    intensity: light.specular,
                },
                other => {
                    skipped.push(
                        format!("{other:?}")
                            .split([' ', '{', '('])
                            .next()
                            .unwrap_or("?")
                            .to_owned(),
                    );
                    continue;
                }
            });
        }
    }
    (c, skipped)
}

/// Item.ExecuteRanged's launch velocity for an NPC whose item was prepared
/// by a Ranged ability and that has a current target: the facing raised
/// by the ability + item elevation, the ballistic speed reaching the
/// target from the item position (gravity `g` < 0; NaN/∞ fall back to
/// twice the ranged range), scaled and yawed by the accuracy spread.
/// `spread` is RandomBetween(-1, 1).
#[allow(clippy::too_many_arguments)]
pub fn ranged_launch_velocity(
    facing: Vector,
    item_position: Vector,
    target: Vector,
    ability_elevation: f32,
    item_elevation: f32,
    ability_accuracy: f32,
    ranged_range: f32,
    g: f32,
    spread: f32,
) -> Vector {
    let dy = target[1] - item_position[1];
    let d =
        ((target[0] - item_position[0]).powi(2) + (target[2] - item_position[2]).powi(2)).sqrt();
    let e = ability_elevation + item_elevation;
    let (sin, cos) = e.sin_cos();
    let v = [facing[0] * cos, sin, facing[2] * cos];
    let mut accuracy = ability_accuracy * spread * (1.0 - ability_accuracy).max(0.0);
    let mut speed = (-g * d * d / (2.0 * (d * sin / cos - dy) * cos * cos)).sqrt();
    if !speed.is_finite() {
        speed = ranged_range * 2.0;
    }
    speed *= accuracy + 1.0;
    accuracy *= std::f32::consts::FRAC_PI_4;
    // Quaternion.CreateFromYawPitchRoll(yaw, 0, 0) about +Y.
    let (s, c) = accuracy.sin_cos();
    let v = [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c];
    v.map(|x| x * speed)
}

/// Item.ExecuteRanged without a Ranged source (or target) and without an
/// action velocity: the facing raised by the item elevation (or a 0.05
/// lift) times twice the ranged range.
pub fn plain_launch_velocity(facing: Vector, item_elevation: f32, ranged_range: f32) -> Vector {
    let mut v = facing;
    if item_elevation > 0.0 {
        let (sin, cos) = item_elevation.sin_cos();
        v = [v[0] * cos, sin, v[2] * cos];
    } else {
        v[1] = 0.05;
    }
    v.map(|x| x * ranged_range * 2.0)
}

/// MissileEntity.SegmentIntersect: sphere against thick segment, with the
/// returned point clamped from the missile centre toward the segment.
pub fn segment_intersect(
    position: Vector,
    radius: f32,
    seg: Segment,
    thickness: f32,
) -> Option<Vector> {
    let relative = sub(position, seg.origin);
    let len_sq = dot(seg.delta, seg.delta);
    let t = if len_sq > 0.0 {
        (dot(relative, seg.delta) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let point = add(seg.origin, scale(seg.delta, t));
    let delta = sub(point, position);
    let distance = length(delta);
    (distance <= radius + thickness)
        .then(|| add(position, scale(delta, (radius / distance).min(1.0))))
}
