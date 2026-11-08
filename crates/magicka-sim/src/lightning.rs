//! LightningSpell (Force/Area) and LightningBolt chaining. See
//! research/lightning.md.
use crate::{
    damage::{calculate_damage, CastType, Damage, DamageError, SpellType},
    spell_cast::{EffectKind, EffectLifecycle, EffectOwner},
    spells::{Elements as E, Spell},
    spray_pulse::{HitList, Segment},
};

type Vector = [f32; 3];

/// One entity returned by EntityManager.GetEntities(center, range,
/// ignoreProtected: true), in original order.
#[derive(Clone, Copy, Debug)]
pub struct LightningTarget {
    pub handle: u16,
    pub position: Vector,
    pub dead: bool,
    /// The type filter: ElementalEgg, DamageablePhysicsEntity, non-ethereal
    /// Character, solid Barrier, MissileEntity, Shield, BossDamageZone or
    /// Tentacle.
    pub eligible: bool,
    pub frozen: bool,
    /// Barrier, Shield or BossDamageZone: the target overload does not chain
    /// onward from it.
    pub ends_chain: bool,
}

/// A drawn bolt: LightningBolt.InitializeEffect.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BoltVisual {
    pub from: Vector,
    pub to: Vector,
    pub direction: Vector,
    pub color: Vector,
    pub scale: f32,
    /// mTTL (0.25 for a hit or a miss, 0.4 for the target overload).
    pub ttl: f32,
}

/// World queries and damage delivery.
pub trait LightningScene {
    fn lightning_targets(&mut self, center: Vector, range: f32) -> Vec<LightningTarget>;
    /// EntityManager.Shields segment test (target hidden behind a shield).
    fn lightning_shielded(&mut self, segment: Segment) -> bool;
    /// Level SegmentIntersect for cut bolts: the fraction along `segment`.
    fn lightning_scene(&mut self, segment: Segment) -> Option<f32>;
    /// IDamageable.Damage(damages, caster, PlayTime, source).
    fn lightning_damage(
        &mut self,
        target: u16,
        attacker: u16,
        damages: [Damage; 5],
        timestamp: f64,
        position: Vector,
    );
}

/// The scene plus presentation and randomness owned by the caster.
pub trait LightningWorld: LightningScene {
    fn lightning_bolt(&mut self, bolt: BoltVisual);
    /// Character.GetSpellRangeModifier of the caster (multiplier, modifier).
    fn caster_range_modifier(&self, _caster: u16) -> (f32, f32) {
        (1.0, 0.0)
    }
    /// MagickaMath.Random.NextDouble (stand-in).
    fn next_double(&mut self) -> f64;
}

/// Spell.GetColor.
pub fn spell_color(spell: &Spell) -> Vector {
    let mut color = [0.0f32; 3];
    let mut weight = 0.0f32;
    let mut add = |c: Vector, w: f32| {
        for i in 0..3 {
            color[i] += c[i] * w;
        }
        weight += w;
    };
    let has = |e: E| spell.element.has(e);
    if has(E::LIGHTNING) {
        add([0.75, 0.5, 1.0], spell.get(E::LIGHTNING));
    }
    if has(E::FIRE) {
        add([1.8, 0.6, 0.4], spell.get(E::FIRE));
    }
    for (e, c) in [
        (E::COLD, [1.0, 1.0, 1.4]),
        (E::LIFE, [0.2, 1.6, 0.2]),
        (E::ARCANE, [2.0, 0.4, 0.6]),
        (E::WATER, [0.0, 0.7, 1.3]),
        (E::ICE, [0.8, 0.9, 1.4]),
        (E::EARTH, [0.3, 0.2, 0.1]),
        (E::SHIELD, [2.0, 1.5, 1.0]),
    ] {
        if has(e) {
            add(c, 1.0);
        }
    }
    color.map(|v| v / weight)
}

/// Where and how the caster fires this tick.
#[derive(Clone, Copy, Debug)]
pub struct LightningSource {
    pub owner: u16,
    pub owner_direction: Vector,
    /// CastSource Forward and Translation.
    pub cast_direction: Vector,
    pub cast_position: Vector,
    /// WeaponSource Forward and Translation (Weapon casts).
    pub weapon_direction: Vector,
    pub weapon_position: Vector,
    /// Character and not in PanicCastState-cleared staff mode.
    pub character: bool,
    /// HasStatus(Wet) && !HasPassiveAbility(WetLightning).
    pub wet: bool,
    /// PlayState.PlayTime (each bolt's damage timestamp).
    pub play_time: f64,
}

#[derive(Clone, Debug)]
pub struct LightningSpell {
    pub lifecycle: EffectLifecycle,
    pub spell: Spell,
    pub all_around: bool,
    pub ttl: f32,
    pub time_between: f32,
    pub to_cast: i32,
    pub range: f32,
    pub scale: f32,
    pub damages: [Damage; 5],
    pub hits: HitList,
}

fn damages_of(spell: &Spell, cast: CastType, factor: f32) -> Result<[Damage; 5], DamageError> {
    let mut out = [Damage::default(); 5];
    for (slot, d) in out
        .iter_mut()
        .zip(calculate_damage(spell, SpellType::Lightning, cast)?)
    {
        *slot = d;
    }
    // DamageCollection5.MultiplyMagnitude.
    for d in &mut out {
        d.magnitude *= factor;
    }
    Ok(out)
}

impl LightningSpell {
    /// LightningSpell.CastForce. `from_staff` is kept only in PanicCastState
    /// (callers pass the already-resolved flag).
    pub fn cast_force(spell: Spell, from_staff: bool, timestamp: f64) -> Result<Self, DamageError> {
        let count = 8;
        Ok(Self {
            lifecycle: EffectLifecycle::begin(CastType::Force, from_staff, timestamp).unwrap(),
            spell,
            all_around: false,
            ttl: 0.0,
            time_between: 1.0 / count as f32,
            to_cast: count,
            range: 4.0 + spell.get(E::LIGHTNING) / 5.0 * 8.0 * 0.5,
            scale: 0.8 + spell.total_magnitude() / 5.0 * 0.4,
            damages: damages_of(&spell, CastType::Force, 1.0 / count as f32)?,
            hits: HitList::default(),
        })
    }

    /// LightningSpell.CastWeapon: CastForce, then mCastType = Weapon (bolts
    /// leave from WeaponSource; damage keeps the Force calculation).
    pub fn cast_weapon(
        spell: Spell,
        from_staff: bool,
        timestamp: f64,
    ) -> Result<Self, DamageError> {
        let mut s = Self::cast_force(spell, from_staff, timestamp)?;
        s.lifecycle.cast_type = CastType::Weapon;
        Ok(s)
    }

    /// LightningSpell.CastArea (mFromStaff = false).
    pub fn cast_area(spell: Spell, timestamp: f64) -> Result<Self, DamageError> {
        let count = 32;
        Ok(Self {
            lifecycle: EffectLifecycle::begin(CastType::Area, false, timestamp).unwrap(),
            spell,
            all_around: true,
            ttl: 0.0,
            time_between: 1.0 / count as f32,
            to_cast: count,
            range: 2.0 + spell.get(E::LIGHTNING) / 5.0 * 4.0 * 0.5,
            scale: 0.8 + spell.total_magnitude() / 5.0 * 0.4,
            damages: damages_of(&spell, CastType::Area, 0.0625)?,
            hits: HitList::default(),
        })
    }

    pub fn deinitialize(&mut self) {
        self.lifecycle.active = false;
    }

    /// LightningSpell.CastUpdate: returns (continue, turn speed 0.1).
    pub fn update(
        &mut self,
        dt: f32,
        owner: EffectOwner,
        source: LightningSource,
        world: &mut impl LightningWorld,
    ) -> (bool, f32) {
        self.ttl -= dt;
        if self.ttl <= 0.0 && self.to_cast > 0 {
            self.hits.clear();
            self.hits.add(source.owner);
            self.ttl = self.time_between;
            self.to_cast -= 1;
            let weapon = self.lifecycle.cast_type == CastType::Weapon;
            let mut direction = if self.lifecycle.from_staff && source.character {
                if weapon {
                    source.weapon_direction
                } else {
                    source.cast_direction
                }
            } else {
                source.owner_direction
            };
            if source.wet {
                self.range = 2.0;
                self.all_around = true;
            }
            if self.all_around {
                let yaw = (self.to_cast % 8) as f32 * std::f32::consts::PI / 4.0;
                direction = yaw_rotate(direction, yaw);
            }
            let color = spell_color(&self.spell);
            let damages = self.damages;
            // All-around bolts use CastSource; Weapon casts WeaponSource.
            let origin = if !self.all_around && weapon {
                source.weapon_position
            } else {
                source.cast_position
            };
            cast_bolt(
                world,
                source.owner,
                source.play_time,
                origin,
                direction,
                &mut self.hits,
                color,
                1.0,
                self.range,
                damages,
            );
        } else if self.lifecycle.active && self.to_cast <= 0 {
            self.deinitialize();
            return (false, 0.1);
        }
        let (keep, _) = self
            .lifecycle
            .update(dt, EffectKind::Lightning, owner, |l| l.active = false);
        (keep, 0.1)
    }
}

/// Vector3.Transform by Quaternion.CreateFromYawPitchRoll(yaw, 0, 0).
fn yaw_rotate(v: Vector, yaw: f32) -> Vector {
    let (s, c) = ((yaw as f64).sin() as f32, (yaw as f64).cos() as f32);
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}

fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn normalize(v: Vector) -> Vector {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / l)
}
fn dist_sq(a: Vector, b: Vector) -> f32 {
    let d = sub(a, b);
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}
/// MagickaMath.Angle.
fn angle(a: Vector, b: Vector) -> f32 {
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    (dot.clamp(-1.0, 1.0) as f64).acos() as f32
}

/// LightningBolt.Cast(caster, source, direction, ...): picks the target with
/// both a smaller angle (≤ 90° in XZ) and a smaller distance than the best
/// so far (or one at the source), damages it and chains from it with the
/// full range; with no target, a cut bolt deviating ±π/16 and the hit list
/// is cleared. Shields, blocking and AirToSurface are not modelled here
/// (`lightning_shielded` covers the shield test).
#[allow(clippy::too_many_arguments)]
pub fn cast_bolt(
    world: &mut impl LightningWorld,
    caster: u16,
    play_time: f64,
    source: Vector,
    mut direction: Vector,
    hits: &mut HitList,
    color: Vector,
    size: f32,
    range: f32,
    damages: [Damage; 5],
) {
    let mut source = source;
    // The original recursion, unrolled: each hop starts a new bolt.
    loop {
        if range <= 0.05 {
            return;
        }
        let mut best: Option<LightningTarget> = None;
        let (mut best_dist, mut best_angle) = (f32::MAX, f32::MAX);
        for t in world.lightning_targets(source, range) {
            if t.dead || hits.contains(t.handle) || !t.eligible {
                continue;
            }
            if world.lightning_shielded(Segment {
                origin: source,
                delta: sub(t.position, source),
            }) {
                continue;
            }
            if t.frozen {
                continue;
            }
            let d = dist_sq(source, t.position);
            let mut to = sub(t.position, source);
            to[1] = 0.0;
            let to = normalize(to);
            let mut flat = direction;
            flat[1] = 0.0;
            if flat != [0.0; 3] {
                flat = normalize(flat);
            }
            let a = angle(flat, to);
            if (a <= std::f32::consts::FRAC_PI_2 && a < best_angle && d < best_dist)
                || d < f32::EPSILON
            {
                best_angle = a;
                best_dist = d;
                best = Some(t);
            }
        }
        let Some(target) = best else {
            let yaw = (world.next_double() as f32 - 0.5) * 0.5 * (std::f32::consts::PI / 4.0);
            direction = yaw_rotate(direction, yaw);
            let mut to = std::array::from_fn(|i| source[i] + direction[i] * range);
            // InitializeEffect iCut: stop at the level.
            if let Some(f) = world.lightning_scene(Segment {
                origin: source,
                delta: sub(to, source),
            }) {
                to = std::array::from_fn(|i| source[i] + (to[i] - source[i]) * f);
            }
            world.lightning_bolt(BoltVisual {
                from: source,
                to,
                direction,
                color,
                scale: size,
                ttl: 0.25,
            });
            hits.clear();
            return;
        };
        hits.add(target.handle);
        world.lightning_damage(target.handle, caster, damages, play_time, source);
        let to = target.position;
        let hop = normalize(sub(to, source));
        world.lightning_bolt(BoltVisual {
            from: source,
            to,
            direction: hop,
            color,
            scale: size,
            ttl: 0.25,
        });
        source = to;
        direction = hop;
    }
}

/// LightningBolt.Cast(caster, source, target, ...), the target overload
/// (Barrier contacts). The target is hit-listed and damaged, then a 0.4 s
/// bolt is drawn. Unless the range is ≤ 0.05 or the target ends chains, the
/// bolt hops from the target to the nearest live, eligible, unfrozen and
/// unlisted entity within the range and a 45° XZ cone along the bolt (no
/// shield test), with range reduced by that distance. With none, a cut bolt
/// continues for the range. Character blocking (deflection into the
/// staff queue) is not modelled. Each hop is a new target-overload call, so
/// the caster's spell-range modifier applies again to the remaining range.
#[allow(clippy::too_many_arguments)]
pub fn cast_bolt_at(
    world: &mut impl LightningWorld,
    caster: u16,
    play_time: f64,
    mut source: Vector,
    mut target: LightningTarget,
    hits: &mut HitList,
    color: Vector,
    scale: f32,
    mut range: f32,
    damages: [Damage; 5],
) {
    let (multiplier, modifier) = world.caster_range_modifier(caster);
    loop {
        range = range * multiplier + modifier;
        hits.add(target.handle);
        world.lightning_damage(target.handle, caster, damages, play_time, source);
        let to = target.position;
        let direction = normalize(sub(to, source));
        world.lightning_bolt(BoltVisual {
            from: source,
            to,
            direction,
            color,
            scale,
            ttl: 0.4,
        });
        if range <= 0.05 || target.ends_chain {
            return;
        }
        let mut flat = direction;
        flat[1] = 0.0;
        if flat != [0.0; 3] {
            flat = normalize(flat);
        }
        let mut best: Option<LightningTarget> = None;
        let mut best_dist = f32::MAX;
        for t in world.lightning_targets(to, range) {
            if t.dead || !t.eligible || t.frozen || hits.contains(t.handle) {
                continue;
            }
            let d = dist_sq(to, t.position);
            let mut second = sub(t.position, to);
            second[1] = 0.0;
            let second = normalize(second);
            if d < best_dist && angle(flat, second) <= std::f32::consts::FRAC_PI_4 {
                best_dist = d;
                best = Some(t);
            }
        }
        let Some(next) = best else {
            let mut end = std::array::from_fn(|i| to[i] + direction[i] * range);
            if let Some(f) = world.lightning_scene(Segment {
                origin: to,
                delta: sub(end, to),
            }) {
                end = std::array::from_fn(|i| to[i] + (end[i] - to[i]) * f);
            }
            world.lightning_bolt(BoltVisual {
                from: to,
                to: end,
                direction,
                color,
                scale,
                ttl: 0.25,
            });
            return;
        };
        range -= best_dist.sqrt();
        source = target.position;
        target = next;
    }
}
