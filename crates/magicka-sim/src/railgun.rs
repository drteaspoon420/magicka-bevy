//! RailGunSpell (Force/Area) and the Railgun beam network: growth, entity
//! and scene hits, Reflect-material/shield reflection, beam crossing
//! (merge, branch or explode) and timed damage. See research/beams.md.
//!
//! Not modelled: CthulhuMist,
//! achievements, and audio cues (reported as events only).
use crate::{
    damage::{attack, calculate_damage, CastType, Damage, DamageError, SpellType},
    geometry::jiglib_segment_segment,
    impact_motion::Vector,
    lightning::spell_color,
    spell_cast::{EffectKind, EffectLifecycle, EffectOwner},
    spells::{inclusive_opposites, Elements as E, Spell},
    spray_pulse::Segment,
};
use std::collections::BTreeMap;

/// Railgun.MAX_RAY_DEPTH.
pub const MAX_RAY_DEPTH: u32 = 6;
/// Railgun.SPEED (growth is SPEED × 3 m/s, capped at 75 m).
pub const SPEED: f32 = 30.0;
pub const MAX_LENGTH: f32 = 75.0;
/// Railgun.EXPLODE_COOLDOWN and the 0.25 s damage interval.
pub const EXPLODE_COOLDOWN: f32 = 0.25;
pub const DAMAGE_INTERVAL: f32 = 0.25;
/// Elements a beam carries as an additional (textured) element.
pub const ADDITIONAL: E = E(0x2 | 0x4 | 0x8 | 0x10 | 0x200 | 0x400);

fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: Vector, s: f32) -> Vector {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn length_sq(a: Vector) -> f32 {
    dot(a, a)
}
fn dist_sq(a: Vector, b: Vector) -> f32 {
    length_sq(sub(a, b))
}
/// XNA Vector3.Normalize (multiply by the reciprocal length).
fn normalize(v: Vector) -> Vector {
    scale(v, 1.0 / length_sq(v).sqrt())
}
/// XNA Vector3.Lerp.
fn lerp(a: Vector, b: Vector, t: f32) -> Vector {
    add(a, scale(sub(b, a), t))
}
/// XNA Vector3.Reflect (no normalisation of `n`).
fn reflect(v: Vector, n: Vector) -> Vector {
    sub(v, scale(n, 2.0 * dot(v, n)))
}

/// Character.SegmentIntersect: the capsule axis (from the feet-side end
/// along Backward) against `seg` widened by `seg_radius`. `axis` is the
/// capsule orientation's Backward vector (Up for upright characters).
pub fn character_segment_intersect(
    position: Vector,
    axis: Vector,
    length: f32,
    radius: f32,
    ethereal: bool,
    seg: Segment,
    seg_radius: f32,
) -> Option<Vector> {
    if ethereal {
        return None;
    }
    let mut origin = position;
    origin[1] -= length * 0.5;
    let delta = scale(axis, length);
    let (d, t, t2) = jiglib_segment_segment(origin, delta, seg.origin, seg.delta);
    let r = seg_radius + radius;
    if d > r * r {
        return None;
    }
    let point = add(origin, scale(delta, t));
    let point2 = add(seg.origin, scale(seg.delta, t2));
    Some(add(point, scale(normalize(sub(point2, point)), radius)))
}

/// What GetEntities returned, as far as the Railgun filters care.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailEntityKind {
    /// IDamageable that is none of the special kinds below.
    Damageable { character: bool },
    /// A Shield entity (reflects the ray).
    Shield,
    /// A Character with CurrentSelfShieldType == Shield (reflects).
    SelfShielded,
    /// Grease fields and non-solid barriers (skipped).
    Ignored,
    /// A connected portal (continues the ray at the other portal).
    Portal,
    /// Not IDamageable (skipped).
    Other,
}
#[derive(Clone, Copy, Debug)]
pub struct RailEntity {
    pub handle: u16,
    pub kind: RailEntityKind,
    pub dead: bool,
    pub position: Vector,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceHit {
    pub fraction: f32,
    pub position: Vector,
    pub normal: Vector,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneRayHit {
    pub hit: SurfaceHit,
    /// oPrim, or the animated part's CollisionMaterial.
    pub material: i32,
}
/// CollisionMaterials.Reflect.
pub const MATERIAL_REFLECT: i32 = 7;

/// World queries and callbacks. Calls happen in original order.
pub trait RailWorld {
    /// EntityManager.GetEntities(center, radius, ignoreProtected: false).
    fn rail_entities(&mut self, center: Vector, radius: f32) -> Vec<RailEntity>;
    /// IDamageable.SegmentIntersect(out pos, seg, radius).
    fn rail_segment(&mut self, handle: u16, seg: Segment, radius: f32) -> Option<Vector>;
    /// Connected portal exit identity and position.
    fn rail_portal_exit(&mut self, _handle: u16) -> Option<(u16, Vector)> {
        None
    }
    /// Body.CollisionSkin.SegmentIntersect (shields and self-shielded characters).
    fn rail_skin(&mut self, handle: u16, seg: Segment) -> Option<SurfaceHit>;
    /// Level.CurrentScene.SegmentIntersect (backfaces kept).
    fn rail_scene(&mut self, seg: Segment) -> Option<SceneRayHit>;
    /// IDamageable.Damage(DamageCollection5, attacker, timestamp, position).
    fn rail_damage(
        &mut self,
        target: u16,
        attacker: u16,
        damages: [Damage; 5],
        timestamp: f64,
        position: Vector,
    ) -> u16;
    /// Character facts read after damage: (HitPoints, Bloating).
    fn rail_character_state(&mut self, target: u16) -> Option<(f32, bool)>;
    /// Character.AccumulateArcaneDamage.
    fn rail_accumulate(&mut self, target: u16, element: u32, magnitude: f32);
    /// Character.BloatKill(element, killer).
    fn rail_bloat_kill(&mut self, target: u16, element: u32, killer: u16);
    /// Blast.FullBlast(collection) with owner/ignore entities.
    fn rail_blast(
        &mut self,
        owner: Option<u16>,
        timestamp: f64,
        radius: f32,
        position: Vector,
        damages: [Damage; 5],
    ) -> u16;
}

pub type RailId = usize;

/// Presentation and audio, in order. Effect names are the original hashes'
/// strings.
#[derive(Clone, Debug, PartialEq)]
pub enum RailEvent {
    /// EffectManager.StartEffect for the source (root rails) or hit effect.
    EffectStarted {
        rail: RailId,
        hit: bool,
        name: &'static str,
        position: Vector,
        direction: Vector,
    },
    EffectMoved {
        rail: RailId,
        hit: bool,
        position: Vector,
        direction: Vector,
    },
    EffectStopped {
        rail: RailId,
        hit: bool,
    },
    /// AudioManager.PlayCue (Spells bank) at the emitter.
    Cue {
        rail: RailId,
        name: &'static str,
        position: Vector,
    },
    HitCueStopped {
        rail: RailId,
    },
    /// Explode: Blast.FullBlast(s) issued at `position` with `radius`.
    Exploded {
        rail: RailId,
        position: Vector,
        radius: f32,
    },
    Removed {
        rail: RailId,
    },
}

/// Per-frame render data (RenderData / AdditionalRenderData).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RailRender {
    pub rail: RailId,
    pub position: Vector,
    pub direction: Vector,
    pub length: f32,
    pub cut: f32,
    /// mTime (30 per second).
    pub time: f32,
    pub color_center: Vector,
    pub color_edge: Vector,
    /// RenderData.Nr (spiral count) and Branch.
    pub spirals: u32,
    pub branch: bool,
    /// Non-empty selects AdditionalRenderData (element textured quads).
    pub additional: E,
    /// CapsuleLight start/end (Update's mLight assignments).
    pub light: (Vector, Vector),
    pub light_color: Vector,
}

#[derive(Clone, Debug)]
pub struct Rail {
    pub position: Vector,
    pub direction: Vector,
    pub length: f32,
    pub time: f32,
    pub cut: f32,
    pub dead: bool,
    pub locked: bool,
    pub child: Option<RailId>,
    pub parents: Vec<RailId>,
    /// SortedList<ushort, DamageCollection5> keyed by owner handle.
    pub damages: Vec<(u16, [Damage; 5])>,
    pub color: Vector,
    pub light_ambient: Vector,
    pub arcane: bool,
    pub additional: E,
    pub all_elements: E,
    pub damage_timer: f32,
    pub will_explode: bool,
    pub explosion_companion: Option<RailId>,
    pub explosion_countdown: f32,
    pub explosion_position: Vector,
    pub opposites: bool,
    pub imma_firin: bool,
    pub timestamp: f64,
    pub depth: u32,
    /// Exit portal skipped by this continuation (mOutPortal).
    pub out_portal: Option<u16>,
    pub hit_cue: bool,
    pub light: (Vector, Vector),
}
impl Rail {
    fn has_key(&self, key: u16) -> bool {
        self.damages.iter().any(|(k, _)| *k == key)
    }
    fn insert_damage(&mut self, key: u16, damages: [Damage; 5]) {
        if let Err(i) = self.damages.binary_search_by_key(&key, |(k, _)| *k) {
            self.damages.insert(i, (key, damages));
        }
    }
    fn set_position(&mut self, p: Vector) {
        if !self.locked {
            self.position = p;
        }
    }
    fn set_direction(&mut self, d: Vector) {
        if !self.locked {
            self.direction = d;
        }
    }
}

/// Railgun pool, sActiveRails and the rails' place in SpellManager's
/// effect list.
#[derive(Clone, Debug, Default)]
pub struct RailSystem {
    slots: Vec<Option<Rail>>,
    /// sActiveRails (crossing tests iterate this order).
    active: Vec<RailId>,
    /// SpellManager.mEffects order (Update and removal).
    effects: Vec<RailId>,
    pub events: Vec<RailEvent>,
    pub render: Vec<RailRender>,
    /// Last (position, direction, length) of removed rails whose slot is not
    /// yet reused. OnRemove returns a Railgun to sCache with its fields
    /// intact, so a child that still lists it as a parent (its own
    /// `mParents` is only cleared for the removed rail) keeps averaging the
    /// stale geometry until the object is reinitialized.
    removed: BTreeMap<RailId, (Vector, Vector, f32)>,
}

/// Root rail parameters (RailGunSpell.CastForce → Railgun.Initialize).
#[derive(Clone, Copy, Debug)]
pub struct RailStart {
    pub owner: u16,
    /// iOwner.Position / Direction (effects only; Position/Direction are
    /// assigned by CastUpdate before the first rail update).
    pub owner_position: Vector,
    pub owner_direction: Vector,
    pub color: Vector,
    pub damages: [Damage; 5],
    pub element: E,
    pub timestamp: f64,
}

fn source_effect(arcane: bool) -> &'static str {
    if arcane {
        "arcane_source"
    } else {
        "life_source"
    }
}
fn hit_effect(arcane: bool) -> &'static str {
    if arcane {
        "arcane_hit"
    } else {
        "life_hit"
    }
}
/// ARCANE/LIFESTAGESOUNDSHASH.
pub fn stage_cue(arcane: bool, stage: usize) -> &'static str {
    const ARCANE: [&str; 4] = [
        "spell_arcane_ray_stage1",
        "spell_arcane_ray_stage2",
        "spell_arcane_ray_stage3",
        "spell_arcane_ray_stage4",
    ];
    const LIFE: [&str; 4] = [
        "spell_life_ray_stage1",
        "spell_life_ray_stage2",
        "spell_life_ray_stage3",
        "spell_life_ray_stage4",
    ];
    if arcane {
        ARCANE[stage]
    } else {
        LIFE[stage]
    }
}

impl RailSystem {
    pub fn rail(&self, id: RailId) -> Option<&Rail> {
        self.slots.get(id).and_then(Option::as_ref)
    }
    fn r(&self, id: RailId) -> &Rail {
        self.slots[id].as_ref().expect("live rail")
    }
    fn m(&mut self, id: RailId) -> &mut Rail {
        self.slots[id].as_mut().expect("live rail")
    }
    pub fn active(&self) -> &[RailId] {
        &self.active
    }
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }
    /// Live (not removed) rail handle and dead flag.
    pub fn is_dead(&self, id: RailId) -> bool {
        self.rail(id).is_none_or(|r| r.dead)
    }

    fn alloc(&mut self, rail: Rail) -> RailId {
        if let Some(i) = self.slots.iter().position(Option::is_none) {
            self.slots[i] = Some(rail);
            self.removed.remove(&i);
            i
        } else {
            self.slots.push(Some(rail));
            self.slots.len() - 1
        }
    }

    fn blank(timestamp: f64, depth: u32) -> Rail {
        Rail {
            position: [0.0; 3],
            direction: [0.0; 3],
            length: 0.0,
            time: 0.0,
            cut: 0.0,
            dead: false,
            locked: false,
            child: None,
            parents: Vec::new(),
            damages: Vec::new(),
            color: [0.0; 3],
            light_ambient: [0.0; 3],
            arcane: false,
            additional: E::NONE,
            all_elements: E::NONE,
            damage_timer: DAMAGE_INTERVAL,
            will_explode: false,
            explosion_companion: None,
            explosion_countdown: 0.0,
            explosion_position: [0.0; 3],
            opposites: false,
            imma_firin: false,
            timestamp,
            depth,
            out_portal: None,
            hit_cue: false,
            light: ([0.0; 3], [0.0; 3]),
        }
    }

    /// Railgun.Initialize(owner, …): a root rail. Its position and
    /// direction are those of a fresh pool object (zero) until the owning
    /// spell's CastUpdate steers it.
    pub fn start(&mut self, s: RailStart) -> RailId {
        let mut rail = Self::blank(s.timestamp, 0);
        rail.arcane = s.element.has(E::ARCANE);
        rail.additional = E(s.element.0 & ADDITIONAL.0);
        rail.all_elements = s.element;
        rail.damages.push((s.owner, s.damages));
        rail.color = s.color;
        rail.light_ambient = [0.0; 3];
        let arcane = rail.arcane;
        let id = self.alloc(rail);
        self.events.push(RailEvent::EffectStarted {
            rail: id,
            hit: false,
            name: source_effect(arcane),
            position: s.owner_position,
            direction: s.owner_direction,
        });
        self.events.push(RailEvent::EffectStarted {
            rail: id,
            hit: true,
            name: hit_effect(arcane),
            position: s.owner_position,
            direction: s.owner_direction,
        });
        self.effects.push(id);
        self.events.push(RailEvent::Cue {
            rail: id,
            name: stage_cue(arcane, 0),
            position: s.owner_position,
        });
        self.active.push(id);
        id
    }

    /// Railgun.Initialize(parentA, parentB, …): a branch, merge or reflection.
    fn start_child(
        &mut self,
        a: RailId,
        b: Option<RailId>,
        position: Vector,
        direction: Vector,
        depth: u32,
    ) -> RailId {
        let pa = self.r(a).clone();
        let mut rail = Self::blank(pa.timestamp, depth);
        rail.parents.push(a);
        if let Some(b) = b {
            rail.parents.push(b);
        }
        rail.arcane = pa.arcane;
        rail.additional = pa.additional;
        rail.all_elements = pa.all_elements;
        rail.damages = pa.damages.clone();
        rail.color = pa.color;
        if let Some(b) = b {
            let pb = self.r(b).clone();
            rail.additional = E(rail.additional.0 | pb.additional.0);
            rail.all_elements = E(rail.all_elements.0 | pb.all_elements.0);
            for (k, d) in &pb.damages {
                rail.insert_damage(*k, *d);
            }
            rail.color = lerp(rail.color, pb.color, 0.5);
            rail.imma_firin = true;
        }
        rail.light_ambient = scale(rail.color, 0.5);
        let arcane = rail.arcane;
        let id = self.alloc(rail);
        self.m(a).child = Some(id);
        if let Some(b) = b {
            self.m(b).child = Some(id);
        }
        self.events.push(RailEvent::EffectStarted {
            rail: id,
            hit: true,
            name: hit_effect(arcane),
            position,
            direction,
        });
        self.effects.push(id);
        self.events.push(RailEvent::Cue {
            rail: id,
            name: stage_cue(arcane, 0),
            position,
        });
        self.active.push(id);
        id
    }

    /// Railgun.Kill.
    pub fn kill(&mut self, id: RailId) {
        if let Some(r) = self.slots.get_mut(id).and_then(Option::as_mut) {
            r.dead = true;
            self.events.push(RailEvent::EffectStopped {
                rail: id,
                hit: false,
            });
        }
    }

    /// RailGunSpell.CastUpdate's Position/Direction assignment (ignored
    /// while locked).
    pub fn steer(&mut self, id: RailId, position: Vector, direction: Vector) {
        if let Some(r) = self.slots.get_mut(id).and_then(Option::as_mut) {
            r.set_position(position);
            r.set_direction(direction);
        }
    }

    fn lock_all(&mut self, id: RailId) {
        // A stale (removed) parent: native writes the cached object only.
        let Some(r) = self.slots[id].as_mut() else {
            return;
        };
        r.locked = true;
        for p in self.r(id).parents.clone() {
            self.lock_all(p);
        }
    }
    fn tail(&self, mut id: RailId) -> RailId {
        while let Some(c) = self.r(id).child {
            id = c;
        }
        id
    }
    /// Railgun.KillAll (throws on a child in the original; tails have none).
    fn kill_all(&mut self, id: RailId) {
        let r = self.m(id);
        r.dead = true;
        r.cut = 1.0;
        debug_assert!(r.child.is_none(), "KillAll on a rail with a child");
        let parents = std::mem::take(&mut r.parents);
        for p in parents {
            // A stale (removed) parent: native writes the cached object only.
            if self.slots[p].is_none() {
                continue;
            }
            self.m(p).child = None;
            self.kill_all(p);
        }
    }
    fn first_junction(&self, id: RailId) -> Vector {
        let r = self.r(id);
        if r.parents.len() == 1 {
            self.first_junction(r.parents[0])
        } else {
            r.position
        }
    }
    fn schedule_explosion(
        &mut self,
        id: RailId,
        companion: Option<RailId>,
        position: Vector,
        lock_from: RailId,
    ) {
        let r = self.m(id);
        r.will_explode = true;
        r.explosion_companion = companion;
        r.explosion_countdown = EXPLODE_COOLDOWN;
        r.explosion_position = position;
        let tail = self.tail(lock_from);
        self.lock_all(tail);
    }

    /// SpellManager.Update over the rails: Update, then OnRemove for dead
    /// ones. Rails created during the pass are updated in the same pass.
    pub fn update(&mut self, dt: f32, world: &mut impl RailWorld) {
        self.render.clear();
        let mut i = 0;
        while i < self.effects.len() {
            let id = self.effects[i];
            self.update_rail(id, dt, world);
            if self.r(id).dead {
                self.on_remove(id);
                self.effects.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Railgun.OnRemove (achievements are not modelled).
    fn on_remove(&mut self, id: RailId) {
        if let Some(c) = self.r(id).child {
            self.kill(c);
            self.m(id).child = None;
        }
        for p in std::mem::take(&mut self.m(id).parents) {
            if let Some(r) = self.slots[p].as_mut() {
                r.child = None;
            }
        }
        self.events.push(RailEvent::EffectStopped {
            rail: id,
            hit: false,
        });
        self.events.push(RailEvent::EffectStopped {
            rail: id,
            hit: true,
        });
        if self.r(id).hit_cue {
            self.events.push(RailEvent::HitCueStopped { rail: id });
        }
        self.active.retain(|&a| a != id);
        let r = self.r(id);
        self.removed.insert(id, (r.position, r.direction, r.length));
        self.slots[id] = None;
        self.events.push(RailEvent::Removed { rail: id });
    }

    /// Reflection off a shield, a self-shielded character or Reflect level
    /// geometry: explode when folding back, else steer/create the child.
    fn reflect_child(&mut self, id: RailId, position: Vector, new_dir: Vector) {
        let dir = self.r(id).direction;
        if dot(new_dir, dir) < -0.99 {
            let junction = self.first_junction(id);
            self.schedule_explosion(id, None, junction, id);
            return;
        }
        if let Some(c) = self.r(id).child {
            if self.r(c).parents.len() > 1 {
                self.kill(c);
                self.m(id).child = None;
            }
        }
        let depth = self.r(id).depth;
        if self.r(id).child.is_none() && depth < MAX_RAY_DEPTH {
            self.start_child(id, None, position, new_dir, depth + 1);
        }
        if let Some(c) = self.r(id).child {
            let child = self.m(c);
            child.set_position(position);
            child.set_direction(new_dir);
        }
    }

    fn update_rail(&mut self, id: RailId, dt: f32, world: &mut impl RailWorld) {
        if self.r(id).dead {
            return;
        }
        let parents = self.r(id).parents.clone();
        if !parents.is_empty() && parents.iter().all(|&p| self.is_dead(p)) {
            self.kill(id);
        }
        if parents.len() > 1 {
            let mut tip = [0.0; 3];
            let mut dir = [0.0; 3];
            for &p in &parents {
                let (position, direction, length) = match self.rail(p) {
                    Some(pr) => (pr.position, pr.direction, pr.length),
                    None => self.removed[&p],
                };
                tip = add(tip, add(position, scale(direction, length)));
                dir = add(dir, direction);
            }
            let r = self.m(id);
            r.position = scale(tip, 1.0 / parents.len() as f32);
            r.direction = normalize(dir);
        }
        self.m(id).time += dt * SPEED;
        if self.r(id).will_explode {
            let r = self.m(id);
            r.imma_firin = false;
            r.explosion_countdown -= dt;
            if r.explosion_countdown <= 0.0 {
                self.explode(id, world);
            }
        } else if !self.r(id).locked {
            self.trace(id, dt, world);
        }
        self.push_render(id);
    }

    /// The `!mLocked` body of Railgun.Update.
    fn trace(&mut self, id: RailId, dt: f32, world: &mut impl RailWorld) {
        {
            let r = self.m(id);
            if !(r.dead && r.child.is_some()) {
                r.length = (r.length + dt * SPEED * 3.0).min(MAX_LENGTH);
            }
        }
        let (position, direction) = (self.r(id).position, self.r(id).direction);
        let mut seg = Segment {
            origin: position,
            delta: scale(direction, self.r(id).length),
        };
        let neg = Segment {
            origin: add(seg.origin, seg.delta),
            delta: scale(seg.delta, -1.0),
        };
        let candidates = world.rail_entities(position, self.r(id).length * 1.25);
        let mut best = f32::MAX;
        let mut entity: Option<RailEntity> = None;
        let mut attack_position = position;
        let only_owner = {
            let d = &self.r(id).damages;
            (d.len() == 1).then(|| d[0].0)
        };
        for e in candidates {
            if matches!(e.kind, RailEntityKind::Other | RailEntityKind::Ignored)
                || self.r(id).out_portal == Some(e.handle)
                || only_owner == Some(e.handle)
            {
                continue;
            }
            let Some(pos) = world.rail_segment(e.handle, seg, 0.25) else {
                continue;
            };
            let d2 = dist_sq(pos, position);
            // Native `d2 < best` branch: NaN distances are skipped.
            #[allow(clippy::neg_cmp_op_on_partial_ord)]
            if !(d2 < best) {
                continue;
            }
            if d2 > 1e-6 {
                best = d2;
                entity = Some(e);
                attack_position = pos;
            } else if let Some(pos) = world.rail_segment(e.handle, seg, 0.25) {
                // The original repeats the identical query.
                let d2 = dist_sq(pos, position);
                if d2 > 1e-6 && d2 < best {
                    best = d2;
                    entity = Some(e);
                    attack_position = pos;
                }
            }
        }
        if entity.is_some() {
            self.m(id).length = best.sqrt();
        } else {
            self.m(id).damage_timer = DAMAGE_INTERVAL;
        }
        let mut hit = entity.is_some();
        seg.delta = scale(direction, self.r(id).length);
        let mut end = [0.0f32; 3];
        let mut last_scene = world.rail_scene(seg);
        if let Some(h) = last_scene {
            if dot(h.hit.normal, direction) > 0.0 {
                let step = scale(direction, 0.1);
                seg.origin = add(seg.origin, step);
                seg.delta = sub(seg.delta, step);
                last_scene = world.rail_scene(seg);
                if let Some(h2) = last_scene {
                    hit = true;
                    self.m(id).length *= h2.hit.fraction;
                    seg.delta = scale(seg.delta, h2.hit.fraction);
                    end = add(position, seg.delta);
                    entity = None;
                }
            } else {
                hit = true;
                self.m(id).length *= h.hit.fraction;
                seg.delta = scale(seg.delta, h.hit.fraction);
                end = add(position, seg.delta);
                entity = None;
            }
        }
        // The last query's outputs (zero on a miss).
        let (scene_pos, scene_normal, material) = last_scene.map_or(([0.0; 3], [0.0; 3], 0), |h| {
            (h.hit.position, h.hit.normal, h.material)
        });
        let reflect_scene = hit
            && dot(scene_normal, direction) < 0.0
            && material == MATERIAL_REFLECT
            && length_sq(scene_normal) > 1e-6;

        if !self.r(id).dead {
            self.cross(id, &mut seg, &mut entity, &mut hit, &mut end, reflect_scene);
        }

        if let Some(e) = entity {
            end = add(position, scale(direction, self.r(id).length));
            if !e.dead {
                match e.kind {
                    RailEntityKind::Shield => {
                        self.m(id).damage_timer = DAMAGE_INTERVAL;
                        seg.delta = scale(direction, self.r(id).length + 1.0);
                        if let Some(mut h) = world.rail_skin(e.handle, seg) {
                            end = h.position;
                            if h.fraction <= 1e-6 {
                                if let Some(h2) = world.rail_skin(e.handle, neg) {
                                    h = h2;
                                    end = h.position;
                                } else {
                                    // Failed out-parameters keep the miss values.
                                    h.normal = [0.0; 3];
                                }
                            }
                            let mut n = h.normal;
                            n[1] = 0.0;
                            let new_dir = normalize(reflect(direction, n));
                            self.reflect_child(id, end, new_dir);
                        } else if let Some(c) = self.r(id).child {
                            self.kill(c);
                        }
                    }
                    RailEntityKind::SelfShielded => {
                        self.m(id).damage_timer = DAMAGE_INTERVAL;
                        seg.delta = scale(direction, self.r(id).length + 1.0);
                        let (frac, n) = if let Some(h) = world.rail_skin(e.handle, seg) {
                            end = h.position;
                            if h.fraction <= 1e-6 {
                                match world.rail_skin(e.handle, neg) {
                                    Some(h2) => {
                                        end = h2.position;
                                        (h2.fraction, h2.normal)
                                    }
                                    None => (f32::MAX, [0.0; 3]),
                                }
                            } else {
                                (h.fraction, h.normal)
                            }
                        } else {
                            let (t, _) = point_segment(e.position, seg);
                            end = add(seg.origin, scale(seg.delta, t));
                            (t, sub(end, e.position))
                        };
                        if frac > 1e-6 {
                            let mut n = n;
                            n[1] = 0.0;
                            let new_dir = normalize(reflect(direction, n));
                            self.reflect_child(id, end, new_dir);
                        } else if let Some(c) = self.r(id).child {
                            self.kill(c);
                        }
                    }
                    RailEntityKind::Portal => {
                        self.m(id).damage_timer = DAMAGE_INTERVAL;
                        if let Some((exit, exit_position)) = world.rail_portal_exit(e.handle) {
                            let child_position = add(exit_position, sub(end, e.position));
                            self.m(id).length += 0.5;
                            if let Some(c) = self.r(id).child {
                                if self.r(c).parents.len() > 1 {
                                    self.kill(c);
                                    self.m(id).child = None;
                                }
                            }
                            let depth = self.r(id).depth;
                            if self.r(id).child.is_none() && depth < MAX_RAY_DEPTH {
                                let c = self.start_child(
                                    id,
                                    None,
                                    child_position,
                                    direction,
                                    depth + 1,
                                );
                                self.m(c).out_portal = Some(exit);
                            }
                            if let Some(c) = self.r(id).child {
                                self.m(c).set_position(child_position);
                                self.m(c).set_direction(direction);
                            }
                        }
                    }
                    RailEntityKind::Damageable { character } => {
                        if self.r(id).damage_timer <= 0.0 {
                            self.m(id).damage_timer += DAMAGE_INTERVAL;
                            let timestamp = self.r(id).timestamp;
                            for (owner, damages) in self.r(id).damages.clone() {
                                let mut d = damages;
                                for x in &mut d {
                                    x.magnitude *= 0.25;
                                }
                                world.rail_damage(e.handle, owner, d, timestamp, attack_position);
                                if !character {
                                    continue;
                                }
                                for x in d.iter().filter(|x| x.element & E::BEAMS.0 != 0) {
                                    world.rail_accumulate(e.handle, x.element, x.magnitude);
                                    if let Some((hp, bloating)) =
                                        world.rail_character_state(e.handle)
                                    {
                                        if hp <= 0.0 && !bloating {
                                            world.rail_bloat_kill(e.handle, x.element, owner);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    RailEntityKind::Ignored | RailEntityKind::Other => {}
                }
                self.m(id).damage_timer -= dt;
            }
        } else if reflect_scene {
            self.m(id).damage_timer = DAMAGE_INTERVAL;
            let new_dir = normalize(reflect(direction, scene_normal));
            self.reflect_child(id, scene_pos, new_dir);
        }

        if hit {
            if !self.r(id).hit_cue {
                let arcane = self.r(id).arcane;
                self.m(id).hit_cue = true;
                self.events.push(RailEvent::Cue {
                    rail: id,
                    name: stage_cue(arcane, 1),
                    position: end,
                });
            }
        } else {
            end = add(position, scale(direction, self.r(id).length));
            if self.r(id).hit_cue {
                self.m(id).hit_cue = false;
                self.events.push(RailEvent::HitCueStopped { rail: id });
            }
        }
        {
            let r = self.m(id);
            if r.dead {
                r.cut += dt / r.length * SPEED * 3.0;
            }
        }
        let r = self.r(id);
        let (p, d, cut) = (r.position, r.direction, r.cut);
        self.events.push(RailEvent::EffectMoved {
            rail: id,
            hit: false,
            position: p,
            direction: d,
        });
        self.events.push(RailEvent::EffectMoved {
            rail: id,
            hit: true,
            position: end,
            direction: d,
        });
        self.m(id).light = (lerp(p, end, cut), end);
    }

    /// The `!mDead` crossing section of Railgun.Update.
    fn cross(
        &mut self,
        id: RailId,
        seg: &mut Segment,
        entity: &mut Option<RailEntity>,
        hit: &mut bool,
        end: &mut Vector,
        reflect_scene: bool,
    ) {
        let mut other: Option<RailId> = None;
        let mut near_end = false;
        for r2 in self.active.clone() {
            let me = self.r(id);
            let o = self.r(r2);
            if r2 == id
                || me.parents.contains(&r2)
                || me.child == Some(r2)
                || o.parents.contains(&id)
                || o.dead
            {
                continue;
            }
            let (d, t, t2) = jiglib_segment_segment(
                seg.origin,
                seg.delta,
                o.position,
                scale(o.direction, o.length),
            );
            if d <= 1.0 && t > 1e-6 {
                let length = me.length;
                let max_t = if near_end {
                    (length - 1.0) / length
                } else {
                    1.0
                };
                let min_t2 = 1.0 / length;
                if t <= max_t && t2 >= min_t2 {
                    *entity = None;
                    near_end = t2 * o.length < 1.0;
                    *hit = false;
                    self.m(id).length *= t;
                    seg.delta = scale(seg.delta, t);
                    other = Some(r2);
                }
            }
        }
        let Some(rg) = other else {
            let blocks = matches!(
                entity.map(|e| e.kind),
                Some(
                    RailEntityKind::Shield | RailEntityKind::SelfShielded | RailEntityKind::Portal
                )
            );
            if let Some(c) = self.r(id).child {
                if !blocks && !reflect_scene {
                    self.kill(c);
                }
            }
            return;
        };
        let (me, o) = (self.r(id), self.r(rg));
        let mut explode = false;
        if !near_end {
            explode = me.damages.iter().any(|(k, _)| o.has_key(*k));
        }
        explode |= inclusive_opposites(me.additional, o.additional);
        explode |= me.arcane != o.arcane;
        *end = add(me.position, seg.delta);
        if explode {
            self.m(id).opposites = true;
            self.schedule_explosion(id, Some(rg), *end, id);
            let r = self.m(id);
            r.length = dist_sq(r.position, *end).sqrt();
            return;
        }
        let child = self.r(id).child;
        let rg_child = self.r(rg).child;
        if rg_child != child && Some(rg) != child {
            if let Some(c) = child {
                self.kill(c);
            }
            if let Some(c) = rg_child {
                if self.r(c).parents.len() <= 2 {
                    self.kill(c);
                }
            }
        }
        if near_end && self.r(id).child.is_none() && !self.r(rg).parents.contains(&id) {
            for (key, damages) in self.r(id).damages.clone() {
                if self.r(rg).has_key(key) {
                    self.schedule_explosion(id, Some(rg), *end, rg);
                    let r = self.m(id);
                    r.length = dist_sq(r.position, *end).sqrt();
                    break;
                }
                self.m(rg).insert_damage(key, damages);
            }
            if !self.r(id).will_explode {
                self.m(id).child = Some(rg);
                self.m(rg).parents.push(id);
            }
        }
        let mid = lerp(self.r(id).direction, self.r(rg).direction, 0.5);
        if length_sq(mid).sqrt() < 0.2 {
            self.schedule_explosion(id, Some(rg), *end, id);
        } else if self.r(id).child.is_none() {
            let depth = self.r(id).depth.max(self.r(rg).depth);
            if depth < MAX_RAY_DEPTH {
                self.start_child(id, Some(rg), *end, normalize(mid), depth + 1);
            }
        }
    }

    /// Railgun.Explode.
    fn explode(&mut self, id: RailId, world: &mut impl RailWorld) {
        let (pos, explosion) = (self.r(id).position, self.r(id).explosion_position);
        let at = lerp(explosion, pos, 0.0001);
        self.m(id).explosion_position = at;
        let tail = self.tail(id);
        let companion = self.r(id).explosion_companion;
        let mut count = self.r(id).damages.len();
        if let Some(c) = companion {
            count += self.rail(c).map_or(0, |r| r.damages.len());
        }
        let radius = 3.0 + count as f32;
        self.kill_all(tail);
        let timestamp = self.r(id).timestamp;
        self.events.push(RailEvent::Exploded {
            rail: id,
            position: at,
            radius,
        });
        for (owner, d) in self.r(id).damages.clone() {
            world.rail_blast(Some(owner), timestamp, radius, at, d);
        }
        if let Some(c) = companion.and_then(|c| self.rail(c).cloned()) {
            for (owner, d) in c.damages {
                world.rail_blast(Some(owner), timestamp, radius, at, d);
            }
        }
        let mut knockback = [Damage::default(); 5];
        knockback[0] = Damage {
            attack: attack::KNOCKBACK,
            element: 0,
            amount: 500.0,
            magnitude: 2.0,
        };
        world.rail_blast(None, timestamp, 5.0, at, knockback);
    }

    fn push_render(&mut self, id: RailId) {
        let r = self.r(id);
        let spirals = if r.parents.is_empty() {
            4
        } else {
            2 + 2 * r.damages.len() as u32
        };
        self.render.push(RailRender {
            rail: id,
            position: r.position,
            direction: r.direction,
            length: r.length,
            cut: r.cut,
            time: r.time,
            color_center: scale(r.color, 0.666),
            color_edge: scale(r.color, 0.333),
            spirals,
            branch: !r.parents.is_empty(),
            additional: r.additional,
            light: r.light,
            light_color: r.color,
        });
    }
}

/// Distance.PointSegmentDistanceSq's parameter (and squared distance).
fn point_segment(pt: Vector, seg: Segment) -> (f32, f32) {
    let mut v = sub(pt, seg.origin);
    let mut t = dot(v, seg.delta);
    if t <= 0.0 {
        t = 0.0;
    } else {
        let len = length_sq(seg.delta);
        if t >= len {
            t = 1.0;
            v = sub(v, seg.delta);
        } else {
            t /= len;
            v = sub(v, scale(seg.delta, t));
        }
    }
    (t, length_sq(v))
}

/// The caster side of the beam (RailGunSpell Force).
pub trait RailControl {
    fn rail_start(&mut self, start: RailStart) -> RailId;
    fn rail_steer(&mut self, id: RailId, position: Vector, direction: Vector);
    fn rail_dead(&self, id: RailId) -> bool;
    fn rail_kill(&mut self, id: RailId);
}
impl RailControl for RailSystem {
    fn rail_start(&mut self, start: RailStart) -> RailId {
        self.start(start)
    }
    fn rail_steer(&mut self, id: RailId, position: Vector, direction: Vector) {
        self.steer(id, position, direction)
    }
    fn rail_dead(&self, id: RailId) -> bool {
        self.is_dead(id)
    }
    fn rail_kill(&mut self, id: RailId) {
        self.kill(id)
    }
}

/// RailGunSpell for Force casts: the effect lifetime that keeps one root
/// rail alive and steered.
#[derive(Clone, Debug)]
pub struct RailGunForce {
    pub lifecycle: EffectLifecycle,
    pub spell: Spell,
    pub ttl: f32,
    pub damages: [Damage; 5],
    pub color: Vector,
    /// Created on the first CastUpdate (same frame as CastForce, before
    /// SpellManager.Update) when the caster hosts the rail system.
    pub rail: Option<RailId>,
    started: bool,
}

/// Owner facts at CastForce/CastUpdate.
#[derive(Clone, Copy, Debug)]
pub struct RailSource {
    pub owner: u16,
    pub owner_position: Vector,
    pub owner_direction: Vector,
    /// CastSource translation.
    pub cast_position: Vector,
}

impl RailGunForce {
    /// RailGunSpell.CastForce. `ttl_modifier` is Character.GetSpellTTLModifier
    /// as (product of multipliers, sum of modifiers).
    pub fn cast(
        spell: Spell,
        from_staff: bool,
        timestamp: f64,
        ttl_modifier: (f32, f32),
    ) -> Result<Self, DamageError> {
        let mut ttl = 1.0 + 2.0 * (spell.get(E::ARCANE) + spell.get(E::LIFE));
        ttl *= ttl_modifier.0;
        ttl += ttl_modifier.1;
        let mut damages = [Damage::default(); 5];
        for (slot, d) in
            damages
                .iter_mut()
                .zip(calculate_damage(&spell, SpellType::Beam, CastType::Force)?)
        {
            *slot = d;
        }
        Ok(Self {
            lifecycle: EffectLifecycle::begin(CastType::Force, from_staff, timestamp).unwrap(),
            spell,
            ttl,
            damages,
            color: spell_color(&spell),
            rail: None,
            started: false,
        })
    }

    /// RailGunSpell.DeInitialize.
    pub fn deinitialize(&mut self, rails: &mut impl RailControl) {
        if self.lifecycle.active {
            if let Some(id) = self.rail.take() {
                rails.rail_kill(id);
            }
            self.lifecycle.active = false;
        }
    }

    /// RailGunSpell.CastUpdate (Force): returns (continue, 0.0325).
    pub fn update(
        &mut self,
        dt: f32,
        owner: EffectOwner,
        source: RailSource,
        rails: &mut impl RailControl,
    ) -> (bool, f32) {
        const TURN: f32 = 0.0325;
        if !self.started {
            self.started = true;
            self.rail = Some(rails.rail_start(RailStart {
                owner: source.owner,
                owner_position: source.owner_position,
                owner_direction: source.owner_direction,
                color: self.color,
                damages: self.damages,
                element: self.spell.element,
                timestamp: self.lifecycle.time_stamp,
            }));
        }
        self.ttl -= dt;
        if let Some(id) = self.rail {
            rails.rail_steer(id, source.cast_position, source.owner_direction);
            if rails.rail_dead(id) {
                self.deinitialize(rails);
                return (false, TURN);
            }
        }
        if self.ttl <= 0.0 || owner.dead || owner.cast_none {
            self.deinitialize(rails);
            return (false, TURN);
        }
        let mut stop = false;
        let (keep, _) = self
            .lifecycle
            .update(dt, EffectKind::RailGun, owner, |_| stop = true);
        if stop {
            self.deinitialize(rails);
        }
        (keep, TURN)
    }
}

/// RailGunSpell.CastArea: one Blast.FullBlast of the Area beam damage
/// around the caster (radius BlastSize × 10). The effect ends on its first
/// CastUpdate (mTTL = 0).
pub fn area_blast(spell: &Spell) -> Result<(f32, [Damage; 5]), DamageError> {
    let mut damages = [Damage::default(); 5];
    for (slot, d) in
        damages
            .iter_mut()
            .zip(calculate_damage(spell, SpellType::Beam, CastType::Area)?)
    {
        *slot = d;
    }
    Ok((spell.blast_size() * 10.0, damages))
}
/// The Area cast cue: spell_arcane_area with the Arcane magnitude, else
/// spell_life_area with the Life magnitude.
pub fn area_cue(spell: &Spell) -> (&'static str, f32) {
    if spell.get(E::ARCANE) > 0.0 {
        ("spell_arcane_area", spell.get(E::ARCANE))
    } else {
        ("spell_life_area", spell.get(E::LIFE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world with characters only, no scene.
    #[derive(Default)]
    struct TestWorld {
        chars: Vec<(u16, Vector, f32)>,
        hp: Vec<f32>,
        damage_log: Vec<(u16, u16, [Damage; 5])>,
        blasts: Vec<(Option<u16>, f32, Vector)>,
        bloats: Vec<(u16, u32, u16)>,
        wall: Option<(f32, i32)>,
    }
    impl RailWorld for TestWorld {
        fn rail_entities(&mut self, center: Vector, radius: f32) -> Vec<RailEntity> {
            self.chars
                .iter()
                .filter(|(_, p, _)| dist_sq(*p, center) <= (radius + 0.5) * (radius + 0.5))
                .map(|&(h, p, _)| RailEntity {
                    handle: h,
                    kind: RailEntityKind::Damageable { character: true },
                    dead: false,
                    position: p,
                })
                .collect()
        }
        fn rail_segment(&mut self, handle: u16, seg: Segment, r: f32) -> Option<Vector> {
            let &(_, p, radius) = self.chars.iter().find(|c| c.0 == handle)?;
            character_segment_intersect(p, [0.0, 1.0, 0.0], 1.0, radius, false, seg, r)
        }
        fn rail_skin(&mut self, _: u16, _: Segment) -> Option<SurfaceHit> {
            None
        }
        fn rail_scene(&mut self, seg: Segment) -> Option<SceneRayHit> {
            // A wall at x = w facing -x.
            let (w, material) = self.wall?;
            let t = (w - seg.origin[0]) / seg.delta[0];
            (seg.delta[0] != 0.0 && (0.0..=1.0).contains(&t)).then(|| SceneRayHit {
                hit: SurfaceHit {
                    fraction: t,
                    position: add(seg.origin, scale(seg.delta, t)),
                    normal: [-seg.delta[0].signum(), 0.0, 0.0],
                },
                material,
            })
        }
        fn rail_damage(&mut self, t: u16, a: u16, d: [Damage; 5], _: f64, _: Vector) -> u16 {
            self.damage_log.push((t, a, d));
            if let Some(i) = self.chars.iter().position(|c| c.0 == t) {
                for x in d {
                    self.hp[i] -= x.amount * x.magnitude;
                }
            }
            0
        }
        fn rail_character_state(&mut self, t: u16) -> Option<(f32, bool)> {
            let i = self.chars.iter().position(|c| c.0 == t)?;
            Some((self.hp[i], false))
        }
        fn rail_accumulate(&mut self, _: u16, _: u32, _: f32) {}
        fn rail_bloat_kill(&mut self, t: u16, e: u32, k: u16) {
            self.bloats.push((t, e, k));
        }
        fn rail_blast(&mut self, o: Option<u16>, _: f64, r: f32, p: Vector, _: [Damage; 5]) -> u16 {
            self.blasts.push((o, r, p));
            0
        }
    }

    fn arcane_damage() -> [Damage; 5] {
        let mut d = [Damage::default(); 5];
        d[0] = Damage {
            attack: attack::DAMAGE,
            element: E::ARCANE.0,
            amount: 100.0,
            magnitude: 1.0,
        };
        d
    }
    fn start(sys: &mut RailSystem, owner: u16, element: E, pos: Vector, dir: Vector) -> RailId {
        let id = sys.start(RailStart {
            owner,
            owner_position: pos,
            owner_direction: dir,
            color: [1.0; 3],
            damages: arcane_damage(),
            element,
            timestamp: 0.0,
        });
        sys.steer(id, pos, dir);
        id
    }

    /// Railgun.Initialize(parentA, parentB) overwrites parentB.mChild, so a
    /// second merge off B leaves the first merged child still listing B.
    /// When B is removed, OnRemove only kills B's current child; the
    /// orphan keeps averaging B's stale cached fields instead of faulting
    /// (chs_dungeons_ch1_s5 probe panic, 2026-11-02).
    #[test]
    fn a_merged_child_keeps_averaging_a_removed_stale_parent() {
        let mut sys = RailSystem::default();
        let mut w = TestWorld::default();
        let a = start(&mut sys, 1, E::ARCANE, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        let b = start(&mut sys, 2, E::ARCANE, [0.0, 0.0, 4.0], [1.0, 0.0, 0.0]);
        let c = start(&mut sys, 3, E::ARCANE, [0.0, 0.0, 8.0], [1.0, 0.0, 0.0]);
        sys.update(0.05, &mut w);
        let first = sys.start_child(a, Some(b), [1.0, 0.0, 2.0], [1.0, 0.0, 0.0], 1);
        let second = sys.start_child(c, Some(b), [1.0, 0.0, 6.0], [1.0, 0.0, 0.0], 1);
        assert_eq!(sys.r(b).child, Some(second));
        assert_eq!(sys.r(first).parents, vec![a, b]);
        // B dies and is removed (Update's dead branch -> OnRemove).
        sys.kill(b);
        sys.on_remove(b);
        assert!(sys.rail(b).is_none());
        assert!(sys.is_dead(second), "OnRemove kills only B's current child");
        assert!(!sys.is_dead(first));
        // The orphan's next update averages A's tip with B's stale tip.
        sys.update_rail(first, 0.05, &mut w);
        let (bp, bd, bl) = sys.removed[&b];
        let ra = sys.r(a);
        let tip = |p: Vector, d: Vector, l: f32| add(p, scale(d, l));
        let want = scale(
            add(tip(ra.position, ra.direction, ra.length), tip(bp, bd, bl)),
            0.5,
        );
        let m = sys.r(first);
        assert!(
            dist_sq(m.position, want) < 1e-8,
            "{:?} vs {want:?}",
            m.position
        );
    }

    #[test]
    fn jiglib_parallel_segments_take_the_parallel_branch() {
        // Antiparallel overlapping segments (two beams meeting head-on).
        let (d, t0, t1) = jiglib_segment_segment(
            [0.0; 3],
            [10.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [-10.0, 0.0, 0.0],
        );
        // Region a01 > 0, -b0 <= a00: seg0's end meets seg1's origin.
        assert_eq!((d, t0, t1), (0.0, 1.0, 0.0));
        // Crossing segments: interior solution.
        let (d, t0, t1) =
            jiglib_segment_segment([0.0; 3], [4.0, 0.0, 0.0], [2.0, 0.0, -1.0], [0.0, 0.0, 2.0]);
        assert!(d.abs() < 1e-6 && (t0 - 0.5).abs() < 1e-6 && (t1 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn grows_at_ninety_metres_per_second_until_a_wall() {
        let mut sys = RailSystem::default();
        let mut w = TestWorld::default();
        let id = start(&mut sys, 1, E::ARCANE, [0.0; 3], [1.0, 0.0, 0.0]);
        sys.update(0.05, &mut w);
        assert!((sys.r(id).length - 4.5).abs() < 1e-5);
        w.wall = Some((6.0, 0));
        sys.update(0.05, &mut w);
        assert!((sys.r(id).length - 6.0).abs() < 1e-4);
        assert!(sys.r(id).hit_cue);
        let render = sys.render[0];
        assert_eq!((render.spirals, render.branch), (4, false));
    }

    #[test]
    fn damage_ticks_every_quarter_second_at_quarter_magnitude() {
        let mut sys = RailSystem::default();
        let mut w = TestWorld {
            // Off the ray: a ray through the capsule axis normalises a zero
            // vector (NaN) and misses, as in the original.
            chars: vec![(7, [3.0, 0.0, 0.1], 0.5)],
            hp: vec![1000.0],
            ..Default::default()
        };
        start(&mut sys, 1, E::ARCANE, [0.0; 3], [1.0, 0.0, 0.0]);
        // Frame 1 finds the target (the timer starts at 0.25).
        for _ in 0..30 {
            sys.update(0.05, &mut w);
        }
        // 1.5 s: first hit after the 0.25 s timer, then every 0.25 s.
        let hits = w.damage_log.len();
        assert!((5..=6).contains(&hits), "{hits}");
        assert!(w
            .damage_log
            .iter()
            .all(|(t, a, d)| *t == 7 && *a == 1 && (d[0].magnitude - 0.25).abs() < 1e-6));
        // The beam stops at the capsule point facing the ray (x = 3,
        // z = 0.1 - 0.5), not at the front of the capsule.
        let expected = (9.0f32 + 0.4 * 0.4).sqrt();
        assert!((sys.render[0].length - expected).abs() < 1e-4);
    }

    #[test]
    fn reflect_material_spawns_a_reflected_child() {
        let mut sys = RailSystem::default();
        let mut w = TestWorld {
            wall: Some((5.0, MATERIAL_REFLECT)),
            ..Default::default()
        };
        let dir = normalize([1.0, 0.0, 1.0]);
        let id = start(&mut sys, 1, E::ARCANE, [0.0; 3], dir);
        for _ in 0..3 {
            sys.update(0.05, &mut w);
        }
        let child = sys.r(id).child.expect("reflected child");
        let c = sys.r(child);
        assert_eq!(c.depth, 1);
        assert!((c.direction[0] + dir[0]).abs() < 1e-5 && (c.direction[2] - dir[2]).abs() < 1e-5);
        assert!((c.position[0] - 5.0).abs() < 1e-4);
    }

    #[test]
    fn head_on_beams_of_different_owners_merge_and_opposites_explode() {
        // Same family, different owners: a merged child at the meeting point.
        let mut sys = RailSystem::default();
        let mut w = TestWorld::default();
        let a = start(
            &mut sys,
            1,
            E::ARCANE,
            [0.0, 0.0, 0.0],
            normalize([1.0, 0.0, 1.0]),
        );
        let b = start(
            &mut sys,
            2,
            E::ARCANE,
            [10.0, 0.0, 0.0],
            normalize([-1.0, 0.0, 1.0]),
        );
        for _ in 0..3 {
            sys.update(0.05, &mut w);
        }
        // `a` updates first while `b` is still empty; `b` then finds the
        // crossing and becomes parent A.
        let child = sys.r(a).child.expect("merged child");
        assert_eq!(sys.r(b).child, Some(child));
        let c = sys.r(child);
        assert_eq!(c.parents, vec![b, a]);
        assert_eq!(
            c.damages.iter().map(|d| d.0).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(c.imma_firin);
        assert!((c.position[0] - 5.0).abs() < 0.05, "{:?}", c.position);
        assert!(c.direction[2] > 0.999);
        assert_eq!(
            sys.render.iter().find(|r| r.rail == child).unwrap().spirals,
            6
        );

        // Arcane against Life: explosion after 0.25 s, 3 + 2 m radius.
        let mut sys = RailSystem::default();
        let a = start(
            &mut sys,
            1,
            E::ARCANE,
            [0.0, 0.0, 0.0],
            normalize([1.0, 0.0, 1.0]),
        );
        start(
            &mut sys,
            2,
            E::LIFE,
            [10.0, 0.0, 0.0],
            normalize([-1.0, 0.0, 1.0]),
        );
        for _ in 0..12 {
            sys.update(0.05, &mut w);
        }
        assert!(sys.rail(a).is_none());
        assert!(sys.is_empty());
        // Both rails explode. `b` (updated second) meets `a`'s grown segment
        // one frame earlier and explodes first: its collection, its
        // companion's, then knockback (radius 3 + 2). `a` then finds its
        // companion removed (OnRemove cleared the damages): radius 3 + 1.
        let summary: Vec<_> = w.blasts.iter().map(|b| (b.0, b.1)).collect();
        assert_eq!(
            summary,
            vec![
                (Some(2), 5.0),
                (Some(1), 5.0),
                (None, 5.0),
                (Some(1), 4.0),
                (None, 5.0)
            ]
        );
        assert!((w.blasts[0].2[0] - 5.0).abs() < 0.05);
    }

    #[test]
    fn character_capsule_intersection_offsets_by_radius() {
        let seg = Segment {
            origin: [0.0, 0.0, 0.0],
            delta: [10.0, 0.0, 0.0],
        };
        let p = character_segment_intersect(
            [5.0, 0.0, 0.3],
            [0.0, 1.0, 0.0],
            1.0,
            0.5,
            false,
            seg,
            0.25,
        )
        .unwrap();
        assert!((p[0] - 5.0).abs() < 1e-5 && (p[2] + 0.2).abs() < 1e-5);
        // Through the axis: normalising the zero offset gives NaN.
        let p = character_segment_intersect(
            [5.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            1.0,
            0.5,
            false,
            seg,
            0.25,
        )
        .unwrap();
        assert!(p[0].is_nan());
        assert!(character_segment_intersect(
            [5.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            1.0,
            0.5,
            false,
            seg,
            0.25
        )
        .is_none());
        assert!(character_segment_intersect(
            [5.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            1.0,
            0.5,
            true,
            seg,
            0.25
        )
        .is_none());
    }

    #[test]
    fn force_ttl_and_area_radius_follow_magnitudes() {
        let mut spell = Spell {
            element: E::ARCANE,
            ..Default::default()
        };
        spell.mag[5] = 2.0;
        let f = RailGunForce::cast(spell, false, 0.0, (1.0, 0.0)).unwrap();
        assert_eq!(f.ttl, 5.0);
        let (radius, _) = area_blast(&spell).unwrap();
        assert!((radius - (0.4f64.sqrt() as f32) * 10.0).abs() < 1e-5);
        assert_eq!(area_cue(&spell), ("spell_arcane_area", 2.0));
    }
}
