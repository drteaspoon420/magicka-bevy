//! Barrier (ShieldSpell Force/Area with elements other than the pure-Shield
//! and mine cases): cast placement, the wall chain, Solid/Elemental state,
//! timers, self-decay, death and damage rules. Probes, contacts, effects,
//! models and the entity pool live in the host; see research/barrier.md.
use crate::character_impact::result;
use crate::damage::{calculate_damage, CastType, Damage, DamageError};
use crate::spell_mine::yaw_rotate;
use crate::spells::{inclusive_opposites, Elements as E, Spell, SpellType};
use crate::spray_pulse::HitList;

pub type Vector = [f32; 3];

/// Barrier.GetRadius(true) / GetRadius(false).
pub fn radius(physical: bool) -> f32 {
    if physical {
        1.25
    } else {
        0.75
    }
}

/// Animation ClipSpeed of both barrier controllers.
pub const CLIP_SPEED: f32 = 2.0;
/// mNextBarrierTTL: the next wall spawns this long after Initialize.
pub const NEXT_TTL: f32 = 0.075;
/// Self-decay: 10 HP every 0.2 s, starting on the first update.
pub const DECAY_STEP: f32 = 10.0;
pub const DECAY_INTERVAL: f32 = 0.2;
/// Detonate (Solid barriers with Arcane/Life when they die).
pub const DETONATE_RADIUS: f32 = 5.0;
/// Contact damage cooldown in the shared hit list.
pub const HIT_COOLDOWN: f32 = 0.25;
/// PlayState: Barrier.InitializeCache(128).
pub const POOL_SIZE: usize = 128;
/// Probe drop of the cast placements.
pub const CAST_PROBE_DROP: f32 = 4.0;

const STATUS_EFFECTS: u32 = 0x64E;
/// Water | Cold | Fire | Lightning | Steam (the geyser/particle choice).
const SPRAYISH: u32 = 0x21E;

fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn length(v: Vector) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}
fn normalize(v: Vector) -> Vector {
    let l = length(v);
    v.map(|x| x / l)
}

/// The barrier branch of ShieldSpell.CastForce/CastArea: Shield plus other
/// elements that are neither the mine case nor pure Shield.
pub fn is_barrier_spell(spell: &Spell) -> bool {
    let rest = spell.element.0 & !E::SHIELD.0;
    spell.element.0 & E::SHIELD.0 != 0 && rest != 0 && !crate::spell_mine::is_mine_spell(spell)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarrierPlacement {
    pub probe_origin: Vector,
    pub direction: Vector,
    pub scale: f32,
    pub range: f32,
    pub next_direction: Vector,
    pub next_yaw: f32,
    pub distance: f32,
}

/// The two seed placements plus the damage every wall of the cast carries.
#[derive(Debug, Clone, PartialEq)]
pub struct BarrierCast {
    pub placements: [BarrierPlacement; 2],
    pub damage: [Damage; 5],
}

/// ShieldSpell barrier placement.
///
/// Both modes split a ring into k = ⌊C / (2·GetRadius(physical))⌋ arcs, with
/// scale 1 + frac/k and the chord between the ±half-step seeds as spacing.
/// Force: ring radius 4. The ring is centred 2 m behind the caster unless
/// the spell has status, Steam or Lightning elements. Range is C/8, the
/// damage is Shield/Force × magnitude 0.25, and only the first probe starts
/// 1 m higher (CIL-proven). Area: ring radius Shield + 3 around the caster,
/// range (C − chord)/2, Shield/Area damage.
pub fn cast_placements(
    area: bool,
    owner_position: Vector,
    owner_direction: Vector,
    spell: &Spell,
) -> Result<BarrierCast, DamageError> {
    let physical = spell.element.0 & E::PHYSICAL_ELEMENTS.0 != 0;
    let ring = if area {
        spell.get(E::SHIELD) + 3.0
    } else {
        4.0
    };
    let mut circumference = ring * std::f32::consts::TAU;
    let count = circumference * (1.0 / (radius(physical) * 2.0));
    let k = count as i32;
    let scale = 1.0 + (count - k as f32) / k as f32;
    let step = std::f32::consts::TAU / k as f32;
    let mut center = owner_position;
    let el = spell.element.0;
    if !area && el & STATUS_EFFECTS == 0 && el & E::STEAM.0 == 0 && el & E::LIGHTNING.0 == 0 {
        center = sub(center, owner_direction.map(|v| v * (ring - 2.0)));
    }
    let reach = owner_direction.map(|v| v * ring);
    let left = yaw_rotate(reach, step * 0.5);
    let right = yaw_rotate(reach, step * -0.5);
    let (left_origin, right_origin) = (add(center, left), add(center, right));
    let to_left = sub(left_origin, right_origin);
    let to_right = sub(right_origin, left_origin);
    let distance = length(to_left);
    let cast_type = if area {
        CastType::Area
    } else {
        CastType::Force
    };
    let mut damage = [Damage::default(); 5];
    for (slot, d) in damage
        .iter_mut()
        .zip(calculate_damage(spell, SpellType::Shield, cast_type)?)
    {
        *slot = d;
    }
    let range = if area {
        circumference -= distance;
        circumference * 0.5
    } else {
        for d in &mut damage {
            d.magnitude *= 0.25;
        }
        circumference * 0.125
    };
    let mut first = left_origin;
    if !area {
        first[1] += 1.0;
    }
    let placement = |origin, offset, next_direction, next_yaw| BarrierPlacement {
        probe_origin: origin,
        direction: normalize(offset),
        scale,
        range,
        next_direction,
        next_yaw,
        distance,
    };
    Ok(BarrierCast {
        placements: [
            placement(first, left, to_left, step),
            placement(right_origin, right, to_right, -step),
        ],
        damage,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BarrierKind {
    Solid,
    Elemental,
}

/// Which skinned models a Solid barrier shows (Initialize's choice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IceModel {
    Plain,
    Geyser,
    Tesla,
    Earth,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EarthModel {
    /// earthBarrier0_mesh: Initialize indexes the five earth meshes with
    /// mRandom.Next(sIceBarrierModels.Length) = Next(1) (CIL-proven).
    Plain,
    Volcano,
}

/// A non-looping controller clip (forward or backward) at ClipSpeed 2.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Clip {
    pub duration: f32,
    pub time: f32,
    pub playing: bool,
    pub backward: bool,
}
impl Clip {
    pub fn start(duration: f32, backward: bool) -> Self {
        Self {
            duration,
            time: if backward { duration } else { 0.0 },
            playing: true,
            backward,
        }
    }
    pub fn update(&mut self, dt: f32) {
        if !self.playing {
            return;
        }
        let step = dt * CLIP_SPEED;
        if self.backward {
            self.time -= step;
            if self.time < 0.0 {
                self.time = 0.0;
                self.playing = false;
            }
        } else {
            self.time += step;
            if self.time > self.duration {
                self.time = self.duration;
                self.playing = false;
            }
        }
    }
}

/// Clip durations the host loads from the barrier models.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarrierClips {
    /// iceBarrier_animation, emerge0 first (also the death clip).
    pub ice_appear: f32,
    pub ice_geyser: f32,
    pub ice_tesla: f32,
    pub ice_earth: f32,
    /// earthBarrier_animation, emerge0 first (also the death clip).
    pub earth_appear: f32,
    pub earth_volcano: f32,
}

/// Effects/sounds the host starts for an Initialize.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BarrierVisuals {
    /// barrier_ice_spawn / barrier_earth_spawn at the hit point.
    pub spawn_effects: Vec<&'static str>,
    /// A geyser/volcano effect attached to the model bone, or the
    /// Elemental particle wall (barrier_<element>1_b).
    pub effect: Option<&'static str>,
    /// spell_*_barrier loop cue.
    pub cue: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Barrier {
    pub owner: Option<u16>,
    /// Shared hit-list identity (HitListWithBarriers of the cast).
    pub hit_list: usize,
    pub kind: BarrierKind,
    pub spell: Spell,
    pub damage: [Damage; 5],
    pub position: Vector,
    pub direction: Vector,
    pub scale: f32,
    /// mRadius: GetRadius(Solid) × scale.
    pub radius: f32,
    pub hit_points: f32,
    pub max_hit_points: f32,
    pub armour: i32,
    pub timestamp: f64,
    pub effect_ttl: f32,
    pub ice_barrier_ttl: f32,
    pub ice_earth_ttl: f32,
    pub draw_ice: bool,
    pub kill_ice: bool,
    pub added_effect: bool,
    pub dead: bool,
    pub damage_timer: f32,
    pub next_ttl: f32,
    pub next_direction: Vector,
    pub next_yaw: f32,
    pub next_range: f32,
    pub distance: f32,
    pub ice_model: Option<IceModel>,
    pub earth_model: Option<EarthModel>,
    pub ice_clip: Clip,
    pub earth_clip: Clip,
    /// Arcane/Life runes under Solid walls (runesLife over runesArcane).
    pub runes: Option<E>,
    pub rune_rotation: f32,
    pub normalized_damage: f32,
    /// sIceAppearClips[0] / sEarthAppearClips[0] durations (death clips).
    pub death_clips: (f32, f32),
    /// Set for a WaveEntity pillar (research/wave.md).
    pub wave: Option<WavePillar>,
}

/// WaveEntity's own state on top of the Barrier it derives from.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WavePillar {
    /// The parent Wave effect (its hit list gates contact damage).
    pub wave: u64,
    /// mAnimationTimer: rockpillar_animation's first clip duration; when it
    /// runs out HitPoints drop to 0 and the pillar is removed at once.
    pub animation_timer: f32,
}

/// Host requests of one Barrier.Update, in call order.
#[derive(Debug, Clone, PartialEq)]
pub enum BarrierEvent {
    /// SpawnNextBarrier with the probe and chain arguments.
    SpawnNext(NextBarrier),
    /// RunIceEarthDeathAnimation: barrier_ice_death at the base.
    IceDeath,
    /// RunDeathAnimation's reverse clips and death effects.
    Death { ice: bool, earth: bool },
    /// Detonate: FullBlast radius 5 (the barrier ignored), mine_explosion.
    Detonate,
    /// InternalDamage's DeathVisuals when HitPoints − amount < 0.
    DeathVisuals { ice: bool, earth: bool },
    /// The attached geyser/volcano effect ends.
    StopEffect,
}

/// Models/Effects/runesArcane and runesLife.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rune {
    Arcane,
    Life,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NextBarrier {
    pub from: Vector,
    pub probe_origin: Vector,
    pub probe_drop: f32,
    pub direction: Vector,
    pub scale: f32,
    pub range: f32,
    pub next_direction: Vector,
    pub next_yaw: f32,
    pub distance: f32,
    /// GetEntities radius around the hit for kills/detonations: 1.5·mRadius.
    pub clear_radius: f32,
}

/// The largest W/C/F/L/S magnitude, with Lightning winning outright
/// (Initialize's geyser/volcano selection loop).
fn geyser_element(spell: &Spell) -> E {
    let (mut chosen, mut best) = (E::NONE, -1.0f32);
    for e in E::BY_INDEX {
        if e.has(E::LIGHTNING) && spell.element.has(E::LIGHTNING) {
            return E::LIGHTNING;
        }
        if e.0 & SPRAYISH != 0 && spell.get(e) > best {
            chosen = e;
            best = spell.get(e);
        }
    }
    chosen
}

/// The Elemental loop: Steam wins when both Steam and Lightning are
/// present, Lightning wins without Steam, otherwise the largest W/C/F/L/S.
fn elemental_element(spell: &Spell) -> E {
    let has = |e: E| spell.element.has(e);
    let (mut chosen, mut best) = (E::NONE, -1.0f32);
    for e in E::BY_INDEX {
        if e.has(E::STEAM) && has(E::LIGHTNING) && has(E::STEAM) {
            return e;
        }
        if e.has(E::LIGHTNING) && !has(E::STEAM) && has(E::LIGHTNING) {
            return e;
        }
        if e.0 & SPRAYISH != 0 && spell.get(e) > best {
            chosen = e;
            best = spell.get(e);
        }
    }
    chosen
}

fn geyser_effect(e: E) -> Option<&'static str> {
    Some(match e {
        E::WATER => "barrier_geyser_water",
        E::COLD => "barrier_geyser_cold",
        E::FIRE => "barrier_geyser_fire",
        E::LIGHTNING => "barrier_ice_tesla",
        E::STEAM => "barrier_geyser_steam",
        E::POISON => "barrier_geyser_poison",
        _ => return None,
    })
}

fn wall_effect(e: E) -> Option<(&'static str, &'static str)> {
    Some(match e {
        E::WATER => ("barrier_water1_b", "spell_water_barrier"),
        E::COLD => ("barrier_cold1_b", "spell_cold_barrier"),
        E::FIRE => ("barrier_fire1_b", "spell_fire_barrier"),
        E::LIGHTNING => ("barrier_lightning1_b", "spell_lightning_barrier"),
        E::STEAM => ("barrier_steam1_b", "spell_steam_barrier"),
        _ => return None,
    })
}

/// Initialize failures.
#[derive(Debug, Clone, PartialEq)]
pub enum BarrierError {
    Damage(DamageError),
    /// No effect element. Unreachable with retail rules: the selection loops
    /// start at magnitude −1, so Water (magnitude 0) is always a fallback.
    NoEffectElement,
}

/// Mutable state a pooled Barrier instance carries into its next
/// Initialize. Only fields that Initialize does not reset are kept: the
/// stale mEffectTTL (pure Earth and Elemental walls never assign it).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Stale {
    pub effect_ttl: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarrierInit {
    pub owner: Option<u16>,
    pub hit_list: usize,
    pub hit: Vector,
    pub direction: Vector,
    pub scale: f32,
    pub range: f32,
    pub next_direction: Vector,
    pub next_yaw: f32,
    pub distance: f32,
    pub timestamp: f64,
}

impl Barrier {
    /// Barrier.Initialize sets each model's OverlayAlpha from its own element.
    pub fn overlay_alpha(&self, ice: bool) -> f32 {
        let magnitude = self.spell.get(if ice { E::ICE } else { E::EARTH });
        ((magnitude - 1.0) * 0.333333).clamp(0.0, 1.0)
    }

    /// Barrier.Initialize (numerical part). Returns the barrier, the
    /// visuals to start and the initial damage the host applies to
    /// damageables within `initial_radius` (not the owner, not this cast's
    /// barriers, not already in the hit list), each then hit-listed.
    pub fn initialize(
        init: BarrierInit,
        spell: Spell,
        damage: [Damage; 5],
        clips: &BarrierClips,
        stale: Stale,
    ) -> Result<(Self, BarrierVisuals, [Damage; 5]), BarrierError> {
        let el = spell.element;
        let kind = if el.0 & !E::SHIELD.0 & E::PHYSICAL_ELEMENTS.0 != 0 {
            BarrierKind::Solid
        } else {
            BarrierKind::Elemental
        };
        let solid = kind == BarrierKind::Solid;
        let mut b = Barrier {
            owner: init.owner,
            hit_list: init.hit_list,
            kind,
            spell,
            damage,
            position: init.hit,
            direction: init.direction,
            scale: init.scale,
            radius: radius(solid) * init.scale,
            hit_points: 0.0,
            max_hit_points: 0.0,
            armour: (spell.get(E::ICE) * 50.0) as i32,
            timestamp: init.timestamp,
            effect_ttl: stale.effect_ttl,
            ice_barrier_ttl: 0.0,
            ice_earth_ttl: 0.0,
            draw_ice: false,
            kill_ice: false,
            added_effect: false,
            dead: false,
            damage_timer: 0.0,
            next_ttl: NEXT_TTL,
            next_direction: yaw_rotate(init.next_direction, init.next_yaw),
            next_yaw: init.next_yaw,
            next_range: init.range - init.distance,
            distance: init.distance,
            ice_model: None,
            earth_model: None,
            ice_clip: Clip::default(),
            earth_clip: Clip::default(),
            runes: None,
            rune_rotation: 0.0,
            normalized_damage: 0.0,
            death_clips: (clips.ice_appear, clips.earth_appear),
            wave: None,
        };
        let mut visuals = BarrierVisuals::default();
        let mut initial = [Damage::default(); 5];
        if solid {
            visuals.cue = Some(if el.has(E::PHYSICAL_ELEMENTS) {
                "spell_ice_earth_barrier"
            } else if el.has(E::ICE) {
                "spell_ice_barrier"
            } else {
                "spell_earth_barrier"
            });
            b.runes = if el.has(E::LIFE) {
                Some(E::LIFE)
            } else if el.has(E::ARCANE) {
                Some(E::ARCANE)
            } else {
                None
            };
            for (slot, d) in initial.iter_mut().zip(
                calculate_damage(&spell, SpellType::Projectile, CastType::Area)
                    .map_err(BarrierError::Damage)?,
            ) {
                *slot = d;
            }
            let geyserish = el.0 & (SPRAYISH | E::POISON.0) != 0 && el.0 & E::BEAMS.0 == 0;
            if el.has(E::ICE) {
                b.draw_ice = true;
                visuals.spawn_effects.push("barrier_ice_spawn");
                let mut model = IceModel::Plain;
                let mut clip = clips.ice_appear;
                b.effect_ttl = spell.get(E::ICE);
                if geyserish {
                    b.added_effect = true;
                    model = IceModel::Geyser;
                    clip = clips.ice_geyser;
                    if el.has(E::LIGHTNING) {
                        model = IceModel::Tesla;
                        clip = clips.ice_tesla;
                        b.effect_ttl = spell.get(E::LIGHTNING);
                    }
                }
                if el.has(E::EARTH) {
                    model = IceModel::Earth;
                    clip = clips.ice_earth;
                    b.added_effect = false;
                    b.ice_earth_ttl = spell.get(E::ICE);
                }
                b.ice_barrier_ttl = b.effect_ttl;
                if b.added_effect {
                    let chosen = geyser_element(&spell);
                    visuals.effect =
                        Some(geyser_effect(chosen).ok_or(BarrierError::NoEffectElement)?);
                    b.set_effect_ttl(chosen, 1.0);
                }
                b.ice_model = Some(model);
                b.ice_clip = Clip::start(clip, false);
            }
            if el.has(E::EARTH) {
                visuals.spawn_effects.push("barrier_earth_spawn");
                let mut model = EarthModel::Plain;
                let mut clip = clips.earth_appear;
                if el.0 & (E::WATER.0 | E::COLD.0 | E::FIRE.0 | E::STEAM.0 | E::POISON.0) != 0
                    && el.0 & E::BEAMS.0 == 0
                {
                    b.added_effect = true;
                    model = EarthModel::Volcano;
                    clip = clips.earth_volcano;
                    let chosen = geyser_element(&spell);
                    b.set_effect_ttl(chosen, 5.0);
                    visuals.effect =
                        Some(geyser_effect(chosen).ok_or(BarrierError::NoEffectElement)?);
                }
                b.earth_model = Some(model);
                b.earth_clip = Clip::start(clip, false);
            }
            b.hit_points = (1.0 + spell.get(E::EARTH)) * 500.0;
        } else {
            let chosen = elemental_element(&spell);
            let (effect, cue) = wall_effect(chosen).ok_or(BarrierError::NoEffectElement)?;
            visuals.effect = Some(effect);
            visuals.cue = Some(cue);
            b.hit_points = spell.total_magnitude() * 100.0;
        }
        b.max_hit_points = b.hit_points;
        // Body position: the hit lifted by radius + 0.05.
        b.position[1] += b.radius + 0.05;
        Ok((b, visuals, initial))
    }

    /// WaveEntity.Initialize (numerical part) for the Wave spell (Earth |
    /// Steam): always Solid, earthBarrier0's mesh playing the
    /// rockpillar_animation clip of `pillar_clip` seconds, no spawn effects,
    /// runes or initial damage (mInitilizeDamage is false), the earth barrier
    /// cue, mNextBarrierTTL 0.1 and the Knockback-only damage overwrite.
    /// Returns the barrier and its cue.
    pub fn initialize_wave(
        init: BarrierInit,
        spell: Spell,
        damage: [Damage; 5],
        clips: &BarrierClips,
        pillar_clip: f32,
        stale: Stale,
        wave: u64,
    ) -> (Self, Option<&'static str>) {
        let el = spell.element;
        let solid = el.0 & !E::SHIELD.0 & E::PHYSICAL_ELEMENTS.0 != 0;
        let mut b = Barrier {
            owner: init.owner,
            hit_list: init.hit_list,
            kind: if solid {
                BarrierKind::Solid
            } else {
                BarrierKind::Elemental
            },
            spell,
            damage: crate::wave::pillar_damage(damage),
            position: init.hit,
            direction: init.direction,
            scale: init.scale,
            radius: radius(solid) * init.scale,
            hit_points: 0.0,
            max_hit_points: 0.0,
            armour: (spell.get(E::ICE) * 50.0) as i32,
            timestamp: init.timestamp,
            effect_ttl: stale.effect_ttl,
            ice_barrier_ttl: 0.0,
            ice_earth_ttl: 0.0,
            draw_ice: false,
            kill_ice: false,
            added_effect: false,
            dead: false,
            damage_timer: 0.0,
            next_ttl: crate::wave::NEXT_TTL,
            next_direction: yaw_rotate(init.next_direction, init.next_yaw),
            next_yaw: init.next_yaw,
            next_range: init.range - init.distance,
            distance: init.distance,
            ice_model: None,
            earth_model: None,
            ice_clip: Clip::default(),
            earth_clip: Clip::default(),
            runes: None,
            rune_rotation: 0.0,
            normalized_damage: 0.0,
            death_clips: (clips.ice_appear, clips.earth_appear),
            wave: Some(WavePillar {
                wave,
                animation_timer: pillar_clip,
            }),
        };
        let mut cue = None;
        if solid {
            cue = if el.has(E::PHYSICAL_ELEMENTS) {
                Some("spell_ice_earth_barrier")
            } else if el.has(E::ICE) {
                Some("spell_ice_barrier")
            } else if el.has(E::EARTH) {
                Some("spell_earth_barrier")
            } else {
                None
            };
            if el.has(E::EARTH) {
                if el.0 & (E::WATER.0 | E::COLD.0 | E::FIRE.0 | E::STEAM.0 | E::POISON.0) != 0
                    && el.0 & E::BEAMS.0 == 0
                {
                    // mAddedEffect is set, but WaveEntity never starts the
                    // effect: only the Steam/Water TTL assignments remain.
                    b.added_effect = true;
                    let chosen = geyser_element(&spell);
                    if chosen.has(E::STEAM) {
                        b.effect_ttl = spell.get(E::STEAM) * 5.0;
                    }
                    if chosen.has(E::WATER) {
                        b.effect_ttl = spell.get(E::WATER) * 5.0;
                    }
                }
                b.earth_model = Some(EarthModel::Plain);
                b.earth_clip = Clip::start(pillar_clip, false);
            }
            b.hit_points = (1.0 + spell.get(E::EARTH)) * 500.0;
        } else {
            b.hit_points = spell.total_magnitude() * 100.0;
        }
        b.max_hit_points = b.hit_points;
        b.position[1] += radius(solid) * init.scale + 0.05;
        (b, cue)
    }

    fn set_effect_ttl(&mut self, chosen: E, per: f32) {
        for e in [E::COLD, E::FIRE, E::POISON, E::STEAM, E::WATER] {
            if chosen.has(e) {
                self.effect_ttl = self.spell.get(e) * per;
            }
        }
    }

    pub fn solid(&self) -> bool {
        self.kind == BarrierKind::Solid
    }

    /// Barrier.SegmentIntersect uses Entity.Position, not the offset skin
    /// capsule position. Its world Backward axis retains the body's scale
    /// and MakeOrientationMatrix's unnormalised Up vector.
    pub fn segment_intersect(
        &self,
        segment: crate::spray_pulse::Segment,
        segment_radius: f32,
    ) -> Option<Vector> {
        let [x, y, z] = self.direction;
        let right = normalize([-z, 0.0, x]);
        let up = [-right[2] * y, right[2] * x - right[0] * z, right[0] * y].map(|v| v * self.scale);
        crate::railgun::character_segment_intersect(
            self.position,
            up,
            0.1,
            self.radius,
            false,
            segment,
            segment_radius,
        )
    }
    /// Initialize's rune model (Life before Arcane) and its additive
    /// ColorTint: the spell colour without Earth, Ice and Shield (Barrier
    /// IL around mRuneModel; research/barrier.md).
    pub fn rune(&self) -> Option<(Rune, Vector)> {
        let el = self.spell.element;
        let rune = if el.has(E::LIFE) {
            Rune::Life
        } else if el.has(E::ARCANE) {
            Rune::Arcane
        } else {
            return None;
        };
        let mut spell = self.spell;
        spell.element = E(el.0 & !(E::PHYSICAL_ELEMENTS.0 | E::SHIELD.0));
        Some((rune, crate::lightning::spell_color(&spell)))
    }
    /// The rune draw: CreateRotationY(mRuneRotation) at the position,
    /// lowered by GetRadius(Solid)·mScale + 0.05.
    pub fn rune_placement(&self) -> (f32, Vector) {
        let [x, y, z] = self.position;
        (
            self.rune_rotation,
            [x, y - (radius(self.solid()) * self.scale + 0.05), z],
        )
    }
    /// Dead: HitPoints ≤ 0.
    pub fn is_dead(&self) -> bool {
        self.hit_points <= 0.0
    }
    /// Removable: dead with neither controller playing (a Wave pillar:
    /// as soon as it is dead).
    pub fn removable(&self) -> bool {
        self.is_dead()
            && (self.wave.is_some() || !self.ice_clip.playing && !self.earth_clip.playing)
    }
    /// The hit lifted to the body: GetEntities radius for Initialize kills
    /// (0.8) and initial damage (1.25) are scale · GetRadius(Solid) · f.
    pub fn query_radius(&self, factor: f32) -> f32 {
        self.scale * radius(self.solid()) * factor
    }
    /// Body position minus (radius + 0.05): the ground point.
    pub fn base(&self) -> Vector {
        [
            self.position[0],
            self.position[1] - self.radius - 0.05,
            self.position[2],
        ]
    }

    /// Whether OnCollision should deal contact damage to a live, non-
    /// barrier damageable (ethereal characters never take it).
    pub fn contact_damages(&self, ethereal: bool, hit_list: &HitList, handle: u16) -> bool {
        !ethereal && (self.effect_ttl > 0.0 || !self.solid()) && !hit_list.contains(handle)
    }
    /// OnCollision's return: only a Solid barrier's capsule is physical
    /// (ethereal characters included).
    pub fn physical_contact(&self, primitive: usize) -> bool {
        self.solid() && primitive == 0
    }

    fn run_death_animation(&mut self, events: &mut Vec<BarrierEvent>) {
        if self.dead {
            return;
        }
        self.dead = true;
        if self.spell.element.0 & E::BEAMS.0 != 0 {
            self.ice_clip.playing = false;
            self.earth_clip.playing = false;
            events.push(BarrierEvent::Detonate);
        } else if !self.ice_clip.playing && !self.earth_clip.playing {
            let el = self.spell.element;
            let ice = el.has(E::ICE) && !self.kill_ice;
            let earth = el.has(E::EARTH);
            if ice {
                self.ice_clip = Clip::start(self.death_clips.0, true);
            }
            if earth {
                self.earth_clip = Clip::start(self.death_clips.1, true);
            }
            events.push(BarrierEvent::Death { ice, earth });
        }
    }

    /// Kill (overlapping placements of another cast): HitPoints 0 and, for
    /// Solid walls, RunDeathAnimation.
    /// WaveEntity overrides Kill with an empty body.
    pub fn kill(&mut self) -> Vec<BarrierEvent> {
        let mut events = Vec::new();
        if self.wave.is_some() {
            return events;
        }
        self.hit_points = 0.0;
        if self.solid() {
            self.run_death_animation(&mut events);
        }
        events
    }

    /// OverKill: HitPoints = −MaxHitPoints (no death animation call).
    pub fn overkill(&mut self) {
        self.hit_points = -self.max_hit_points;
    }

    /// Barrier.Update. `ages_hit_list` is true when this barrier is the
    /// first owner of its shared hit list (the host then ages that list).
    pub fn update(&mut self, dt: f32) -> Vec<BarrierEvent> {
        if self.wave.is_some() {
            return self.update_wave(dt);
        }
        let mut events = Vec::new();
        let el = self.spell.element;
        self.effect_ttl -= dt;
        self.ice_barrier_ttl -= dt;
        self.ice_earth_ttl -= dt;
        self.rune_rotation -= dt * 0.25;
        if self.ice_barrier_ttl < 0.0 && el.0 & E::PHYSICAL_ELEMENTS.0 == E::ICE.0 {
            self.hit_points = 0.0;
        }
        if self.ice_earth_ttl < 0.0 && !self.kill_ice && el.has(E::ICE) && el.has(E::EARTH) {
            self.kill_ice = true;
            // RunIceEarthDeathAnimation only acts when the ice clip is idle.
            if !self.ice_clip.playing {
                self.ice_clip = Clip::start(self.death_clips.0, true);
                events.push(BarrierEvent::IceDeath);
            }
        }
        if self.ice_earth_ttl < 0.0 && !self.ice_clip.playing && self.kill_ice && self.draw_ice {
            self.draw_ice = false;
        }
        self.chain(dt, &mut events);
        // mDamageSelf is always true after Initialize.
        self.damage_timer -= dt;
        if self.damage_timer < 0.0 {
            self.hit_points -= DECAY_STEP;
            self.damage_timer += DECAY_INTERVAL;
            if self.is_dead() && self.solid() {
                self.run_death_animation(&mut events);
            }
        }
        let target = 1.0 - self.hit_points / self.max_hit_points;
        self.normalized_damage =
            target + (self.normalized_damage - target) * (0.15f64.powf(dt as f64) as f32);
        if el.has(E::ICE) {
            self.ice_clip.update(dt);
        }
        if el.has(E::EARTH) {
            self.earth_clip.update(dt);
        }
        if self.added_effect && self.effect_ttl <= 0.0 {
            self.added_effect = false;
            events.push(BarrierEvent::StopEffect);
        }
        events
    }

    /// The SpawnNextBarrier countdown shared by Barrier and WaveEntity.
    fn chain(&mut self, dt: f32, events: &mut Vec<BarrierEvent>) {
        if self.next_range > f32::from_bits(1) {
            self.next_ttl -= dt;
            if self.next_ttl < 0.0 {
                let probe = add(self.position, self.next_direction);
                events.push(BarrierEvent::SpawnNext(NextBarrier {
                    from: self.position,
                    probe_origin: [probe[0], probe[1] + 0.75 - (0.05 + self.radius), probe[2]],
                    probe_drop: 1.5,
                    // Body Forward of Scale(mScale)·MakeOrientationMatrix(dir)
                    // (not renormalized), rotated by the yaw.
                    direction: yaw_rotate(self.direction.map(|v| v * self.scale), self.next_yaw),
                    scale: self.scale,
                    range: self.next_range,
                    next_direction: self.next_direction,
                    next_yaw: self.next_yaw,
                    distance: self.distance,
                    clear_radius: 1.5 * self.radius,
                }));
                self.next_range = 0.0;
            }
        }
    }

    /// WaveEntity.Update (it does not call Barrier.Update): no self-decay
    /// or ice timers; the animation timer ends the pillar.
    fn update_wave(&mut self, dt: f32) -> Vec<BarrierEvent> {
        let mut events = Vec::new();
        self.effect_ttl -= dt;
        self.rune_rotation -= dt * 0.25;
        self.chain(dt, &mut events);
        if let Some(w) = self.wave.as_mut() {
            w.animation_timer -= dt;
            if w.animation_timer < 0.0 {
                self.hit_points = 0.0;
            }
        }
        let target = 1.0 - self.hit_points / self.max_hit_points;
        self.normalized_damage =
            target + (self.normalized_damage - target) * (0.15f64.powf(dt as f64) as f32);
        if self.spell.element.has(E::EARTH) {
            self.earth_clip.update(dt);
        }
        // EffectManager.Stop on the never-started reference.
        if self.added_effect && self.effect_ttl <= 0.0 {
            self.added_effect = false;
        }
        events
    }

    /// Barrier.InternalDamage(DamageCollection5): the five slots in order.
    pub fn internal_damage(
        &mut self,
        damages: &[Damage; 5],
        feature_damage: bool,
    ) -> (u16, Vec<BarrierEvent>) {
        let mut events = Vec::new();
        let mut r = 0;
        for d in damages {
            r |= self.internal_damage_one(*d, feature_damage, &mut events);
        }
        (r, events)
    }

    /// Barrier.InternalDamage(Damage).
    pub fn internal_damage_one(
        &mut self,
        mut d: Damage,
        feature_damage: bool,
        events: &mut Vec<BarrierEvent>,
    ) -> u16 {
        let mut r = 0;
        let mut elements = E(d.element);
        if elements.has(E::ARCANE) {
            elements = E(elements.0 ^ E::ARCANE.0);
        }
        let own = self.spell.element;
        match self.kind {
            BarrierKind::Solid => {
                let mut quench = false;
                if d.amount > 0.0 {
                    if (own.has(E::COLD) && (elements.has(E::FIRE) || elements.has(E::STEAM)))
                        || (own.has(E::STEAM) && elements.has(E::COLD))
                        || (own.has(E::WATER) && elements.has(E::FIRE))
                        || (own.has(E::FIRE) && elements.has(E::WATER))
                    {
                        self.effect_ttl = 0.0;
                        quench = true;
                    } else {
                        d.amount = if self.spell.get(E::ICE) > 0.0 && elements.has(E::FIRE) {
                            d.amount.max(0.0)
                        } else {
                            (d.amount - self.armour as f32).max(0.0)
                        };
                        if d.amount == 0.0 {
                            r |= result::DEFLECTED;
                        }
                        if d.amount > 0.0 {
                            r |= result::DAMAGED;
                        }
                    }
                }
                // `break` skips the HitPoints change after a quench.
                if !quench && feature_damage && d.amount * d.magnitude > 0.0 {
                    self.hit_points -= d.amount;
                }
            }
            BarrierKind::Elemental => {
                let (mut opposed, mut total) = (0.0f32, 0.0f32);
                for e in E::BY_INDEX {
                    if own.has(e) && !e.has(E::SHIELD) {
                        total += 1.0;
                        if inclusive_opposites(elements, e) {
                            opposed += 1.0;
                        }
                    }
                }
                if feature_damage && d.amount * d.magnitude > 0.0 {
                    self.hit_points -= (opposed / total * d.amount * 5.0) as i32 as f32;
                }
            }
        }
        if self.solid() {
            if self.is_dead() {
                self.run_death_animation(events);
            }
            if self.hit_points - d.amount < 0.0 {
                r |= result::KILLED;
                let el = self.spell.element;
                events.push(BarrierEvent::DeathVisuals {
                    ice: el.has(E::ICE) && !self.kill_ice,
                    earth: el.has(E::EARTH),
                });
            }
        }
        r
    }
}

/// Barrier.mCache: GetFromCache pops the last free instance (a new one when
/// empty); Deinitialize appends the instance back. Instances keep stale
/// fields between uses.
#[derive(Debug, Clone, Default)]
pub struct BarrierPool {
    pub slots: Vec<Option<Barrier>>,
    stale: Vec<Stale>,
    free: Vec<usize>,
    /// Entity-list order of live slots.
    pub order: Vec<usize>,
    /// Shared hit lists by cast: (owners in Initialize order, list).
    pub hit_lists: Vec<(Vec<usize>, HitList)>,
}

impl BarrierPool {
    pub fn new() -> Self {
        Self {
            slots: vec![None; POOL_SIZE],
            stale: vec![Stale::default(); POOL_SIZE],
            free: (0..POOL_SIZE).collect(),
            order: Vec::new(),
            hit_lists: Vec::new(),
        }
    }

    /// HitListWithBarriers.GetFromCache for a new cast.
    pub fn new_hit_list(&mut self) -> usize {
        if let Some(i) = self.hit_lists.iter().position(|(o, _)| o.is_empty()) {
            self.hit_lists[i].1.clear();
            return i;
        }
        self.hit_lists.push((Vec::new(), HitList::default()));
        self.hit_lists.len() - 1
    }

    /// GetFromCache: the slot the next Initialize uses and its stale state.
    pub fn take(&mut self) -> (usize, Stale) {
        let slot = self.free.pop().unwrap_or_else(|| {
            self.slots.push(None);
            self.stale.push(Stale::default());
            self.slots.len() - 1
        });
        (slot, self.stale[slot])
    }

    /// Initialize's hit-list registration and AddEntity.
    pub fn add(&mut self, slot: usize, barrier: Barrier) {
        let list = barrier.hit_list;
        self.slots[slot] = Some(barrier);
        self.hit_lists[list].0.push(slot);
        self.order.push(slot);
    }

    /// A slot taken but not initialized (e.g. Initialize failed).
    pub fn give_back(&mut self, slot: usize) {
        self.free.push(slot);
    }

    /// A network client's display copy: the host's live barriers by slot,
    /// in entity order (no free list or hit lists; clients do not simulate).
    pub fn mirror(live: Vec<(usize, Barrier)>) -> Self {
        let mut pool = Self::new();
        for (slot, barrier) in live {
            if slot < POOL_SIZE && pool.slots[slot].is_none() {
                pool.slots[slot] = Some(barrier);
                pool.order.push(slot);
            }
        }
        pool
    }
    pub fn get(&self, slot: usize) -> Option<&Barrier> {
        self.slots.get(slot).and_then(Option::as_ref)
    }
    pub fn get_mut(&mut self, slot: usize) -> Option<&mut Barrier> {
        self.slots.get_mut(slot).and_then(Option::as_mut)
    }
    pub fn iter(&self) -> impl Iterator<Item = (usize, &Barrier)> {
        self.order
            .iter()
            .filter_map(|&s| self.slots[s].as_ref().map(|b| (s, b)))
    }
    /// True when `slot` is the first owner of its hit list (it ages it).
    pub fn ages_hit_list(&self, slot: usize) -> bool {
        self.get(slot)
            .is_some_and(|b| self.hit_lists[b.hit_list].0.first() == Some(&slot))
    }

    /// Slots removed by `remove_dead`.
    pub fn remove_dead_slots(&mut self) -> Vec<usize> {
        let before = self.order.clone();
        self.remove_dead();
        before
            .into_iter()
            .filter(|s| !self.order.contains(s))
            .collect()
    }

    /// RemoveDeadEntities + Deinitialize for removable barriers.
    pub fn remove_dead(&mut self) -> Vec<Barrier> {
        let mut removed = Vec::new();
        let mut keep = Vec::new();
        for &slot in &self.order {
            let Some(b) = self.slots[slot].as_ref() else {
                continue;
            };
            if b.removable() {
                let b = self.slots[slot].take().unwrap();
                self.hit_lists[b.hit_list].0.retain(|&s| s != slot);
                self.stale[slot] = Stale {
                    effect_ttl: b.effect_ttl,
                };
                if !self.free.contains(&slot) {
                    self.free.push(slot);
                }
                removed.push(b);
            } else {
                keep.push(slot);
            }
        }
        self.order = keep;
        removed
    }
}
