//! SpellMine (ShieldSpell Force/Area with Arcane or Life and no physical
//! element): cast placement, the mine chain, trigger/arming, detonation
//! timing and damage reaction. Scene probes, blasts, effects and the entity
//! pool are host work; see research/spell-mine.md.
use crate::damage::{attack, calculate_damage, CastType, Damage, DamageError};
use crate::spells::{Elements as E, Spell, SpellType};

pub type Vector = [f32; 3];

/// SpellMine.RADIUS (Entity radius used by blasts and entity queries).
pub const RADIUS: f32 = 1.0;
/// Initialize shrinks the collision sphere to 0.9.
pub const COLLISION_RADIUS: f32 = 0.9;
/// SpellMine.MAX_DISTANCE_SQ: owner farther away detonates the mine.
pub const MAX_DISTANCE_SQ: f32 = 900.0;
/// SpellMine.DETONATION_TIMER, armed by a trigger contact.
pub const DETONATION_TIMER: f32 = 0.5;
/// InternalDamage that Damaged the mine detonates it this soon.
pub const DAMAGED_TIMER: f32 = 0.1;
/// Detonate: Blast.FullBlast radius.
pub const BLAST_RADIUS: f32 = 3.0;
/// AnimationController.Speed of the spawn clip.
pub const CLIP_SPEED: f32 = 3.0;
/// mModelMatrix uniform scale (mScale is never used for drawing).
pub const MODEL_SCALE: f32 = 2.1;
/// PlayState: SpellMine.InitializeCache(64).
pub const POOL_SIZE: usize = 64;
/// The cast probes straight down this far from each placement.
pub const CAST_PROBE_DROP: f32 = 4.0;

/// Vector3.Transform by Quaternion.CreateFromYawPitchRoll(yaw, 0, 0).
pub fn yaw_rotate(v: Vector, yaw: f32) -> Vector {
    let (s, c) = ((yaw as f64).sin() as f32, (yaw as f64).cos() as f32);
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}
fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn length(v: Vector) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}
fn normalize(v: Vector) -> Vector {
    let l = length(v);
    v.map(|x| x / l)
}

/// True for the mine branch of ShieldSpell.CastForce/CastArea: Shield
/// plus Arcane or Life, without physical elements.
pub fn is_mine_spell(spell: &Spell) -> bool {
    let rest = spell.element.0 & !E::SHIELD.0;
    spell.element.0 & E::SHIELD.0 != 0
        && rest != 0
        && rest & E::PHYSICAL_ELEMENTS.0 == 0
        && rest & (E::ARCANE.0 | E::LIFE.0) != 0
}

/// One of the two mines a cast tries to place. The host probes
/// `probe_origin` downwards by `CAST_PROBE_DROP` and, on a scene hit,
/// initializes a mine at the hit point with these arguments.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinePlacement {
    pub probe_origin: Vector,
    pub direction: Vector,
    pub scale: f32,
    pub range: f32,
    pub next_direction: Vector,
    pub next_yaw: f32,
    pub distance: f32,
}

/// ShieldSpell mine placement: a 4 m ring split into ⌊4π⌋ = 12 arcs.
/// Force centres the ring 1 m behind the caster and allows a range of
/// circumference/8 per side; Area centres it on the caster with half the
/// circumference minus one spacing. Area's scale is (n + k)/k where Force
/// uses 1 + (n − k)/k (CIL-proven; scale only propagates).
pub fn cast_placements(
    area: bool,
    owner_position: Vector,
    owner_direction: Vector,
) -> [MinePlacement; 2] {
    let ring = 4.0f32;
    let circumference = ring * std::f32::consts::TAU;
    let half = circumference * 0.5;
    let count = half as i32;
    let scale = if area {
        (half + count as f32) / count as f32
    } else {
        1.0 + (half - count as f32) / count as f32
    };
    let step = std::f32::consts::TAU / count as f32;
    let center = if area {
        owner_position
    } else {
        sub(owner_position, owner_direction.map(|v| v * (ring - 3.0)))
    };
    let reach = owner_direction.map(|v| v * ring);
    let left = yaw_rotate(reach, step * 0.5);
    let right = yaw_rotate(reach, step * -0.5);
    let (left_origin, right_origin) = (add(center, left), add(center, right));
    let to_left = sub(left_origin, right_origin);
    let to_right = sub(right_origin, left_origin);
    let distance = length(to_left);
    let range = if area {
        circumference * 0.5 - distance
    } else {
        circumference * 0.125
    };
    let placement = |origin, offset, next_direction, next_yaw| MinePlacement {
        probe_origin: origin,
        direction: normalize(offset),
        scale,
        range,
        next_direction,
        next_yaw,
        distance,
    };
    [
        placement(left_origin, left, to_left, step),
        placement(right_origin, right, to_right, -step),
    ]
}

/// SpellMine.Initialize recomputes CalculateDamage(Shield, Force); the
/// caller's damage argument is never read (CIL-proven), so Area mines also
/// carry Force damage.
pub fn mine_damage(spell: &Spell) -> Result<[Damage; 5], DamageError> {
    let mut damages = [Damage::default(); 5];
    for (slot, d) in
        damages
            .iter_mut()
            .zip(calculate_damage(spell, SpellType::Shield, CastType::Force)?)
    {
        *slot = d;
    }
    Ok(damages)
}

/// SpellMine.InternalDamage(Damage): Damaged when |amount × magnitude| > 0
/// and the attack carries Damage or Status.
pub fn damages_mine(d: &Damage) -> bool {
    (d.amount * d.magnitude).abs() > 0.0 && d.attack & (attack::DAMAGE | attack::STATUS) != 0
}

/// The host side of SpawnNextMine: a shield test from the mine to
/// `probe_origin` (any hit cancels), then a scene probe from `probe_origin`
/// downwards by 1.5. On a scene hit the host kills barriers and detonates
/// mines within 1 m of the hit and initializes a pooled mine there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NextMine {
    pub from: Vector,
    pub probe_origin: Vector,
    pub probe_drop: f32,
    pub direction: Vector,
    pub scale: f32,
    pub range: f32,
    pub next_direction: Vector,
    pub next_yaw: f32,
    pub distance: f32,
}

/// Classification of an OnCollision partner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toucher {
    /// A Character (ethereal/levitating characters are ignored).
    Character {
        owner: bool,
        dead: bool,
        ethereal: bool,
        levitating: bool,
    },
    /// A MissileEntity or BossDamageZone.
    Other { owner: bool, dead: bool },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpellMine {
    pub owner: u16,
    pub position: Vector,
    pub direction: Vector,
    pub scale: f32,
    pub spell: Spell,
    pub damage: [Damage; 5],
    pub timestamp: f64,
    pub activated: bool,
    pub owner_collision: bool,
    pub detonation_timer: f32,
    pub next_mine_ttl: f32,
    pub next_mine_direction: Vector,
    pub next_mine_yaw: f32,
    pub next_mine_range: f32,
    pub distance_between_mines: f32,
    pub dead: bool,
    /// Spawn clip time (Take 001 at Speed 3); HasFinished once it passes the
    /// clip duration.
    pub clip_time: f32,
    pub clip_duration: f32,
    pub clip_finished: bool,
    /// The random model yaw (RANDOM.NextDouble() × 2π).
    pub model_yaw: f32,
    /// ACTIVATION_EFFECT already started (mActivationEffect.Hash != 0).
    pub activation_started: bool,
}

/// What a contact did to the mine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactResult {
    Ignored,
    /// The owner touches an unactivated mine and keeps it unarmed.
    OwnerHeld,
    /// The 0.5 s fuse started; `first` starts mine_activation and its cue.
    Armed {
        first: bool,
    },
}

/// Ordered side effects of one SpellMine.Update.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MineUpdate {
    pub spawn_next: Option<NextMine>,
    /// Each entry is one Detonate call: FullBlast, mine_explosion and the
    /// Area sounds. The owner-distance check can repeat it while a dead mine
    /// waits for its clip to finish.
    pub detonations: u8,
}

impl SpellMine {
    /// SpellMine.Initialize at a scene probe hit. `model_yaw` comes from the
    /// shared System.Random of the original; hosts supply it.
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        owner: u16,
        hit: Vector,
        direction: Vector,
        scale: f32,
        range: f32,
        next_direction: Vector,
        next_yaw: f32,
        distance: f32,
        spell: Spell,
        timestamp: f64,
        clip_duration: f32,
        model_yaw: f32,
    ) -> Result<Self, DamageError> {
        Ok(Self {
            owner,
            position: [hit[0], hit[1] + 0.1, hit[2]],
            direction,
            scale,
            damage: mine_damage(&spell)?,
            spell,
            timestamp,
            activated: false,
            owner_collision: false,
            detonation_timer: 0.0,
            next_mine_ttl: 0.1,
            next_mine_direction: yaw_rotate(next_direction, next_yaw),
            next_mine_yaw: next_yaw,
            next_mine_range: range - distance,
            distance_between_mines: distance,
            dead: false,
            clip_time: 0.0,
            clip_duration,
            clip_finished: false,
            model_yaw,
            activation_started: false,
        })
    }

    /// Removable: dead and the spawn clip has finished.
    pub fn removable(&self) -> bool {
        self.dead && self.clip_finished
    }

    /// SpellMine.OnCollision (always rejects the physical contact).
    pub fn on_collision(&mut self, toucher: Toucher) -> ContactResult {
        if self.dead || self.detonation_timer > 0.0 || !self.clip_finished {
            return ContactResult::Ignored;
        }
        let owner = match toucher {
            Toucher::Character { dead: true, .. } | Toucher::Other { dead: true, .. } => {
                return ContactResult::Ignored
            }
            Toucher::Character {
                ethereal,
                levitating,
                ..
            } if ethereal || levitating => return ContactResult::Ignored,
            Toucher::Character { owner, .. } | Toucher::Other { owner, .. } => owner,
        };
        if owner && !self.activated {
            self.owner_collision = true;
            return ContactResult::OwnerHeld;
        }
        self.detonation_timer = DETONATION_TIMER;
        let first = !self.activation_started;
        self.activation_started = true;
        ContactResult::Armed { first }
    }

    /// SpellMine.InternalDamage(DamageCollection5): the result is Damaged
    /// when any slot damages the mine, which sets the timer to 0.1 even
    /// for a dead mine.
    pub fn internal_damage(&mut self, damages: &[Damage; 5]) -> bool {
        let damaged = damages.iter().any(damages_mine);
        if damaged {
            self.detonation_timer = DAMAGED_TIMER;
        }
        damaged
    }

    /// SpellMine.SegmentIntersect delegates to its radius-0.9 skin sphere;
    /// the caller's beam/weapon radius is ignored. Preserve JigLibX's short
    /// segment rejection as well as its entry fraction (including zero).
    pub fn segment_intersect(&self, seg: crate::spray_pulse::Segment) -> Option<Vector> {
        let dot = |a: Vector, b: Vector| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let offset = sub(seg.origin, self.position);
        let a = dot(seg.delta, seg.delta);
        let r2 = COLLISION_RADIUS * COLLISION_RADIUS;
        if a < r2 {
            return None;
        }
        let b = dot(offset, seg.delta);
        let discriminant = b * b - a * (dot(offset, offset) - r2);
        if discriminant < 0.0 {
            return None;
        }
        let root = (discriminant as f64).sqrt() as f32;
        let near = (-b - root) / a;
        let far = (-b + root) / a;
        if near > 1.0 || far < 0.0 {
            return None;
        }
        let t = near.max(0.0);
        Some(std::array::from_fn(|i| seg.origin[i] + seg.delta[i] * t))
    }

    /// Kill (Barrier/mine placement overlap): dead without a blast.
    pub fn kill(&mut self) {
        self.dead = true;
    }

    /// Detonate's state change; the host performs the blast and effects.
    pub fn detonate(&mut self) {
        self.dead = true;
    }

    /// SpellMine.Update. The host performs `spawn_next` before the
    /// detonations, matching the inline call order.
    pub fn update(&mut self, dt: f32, owner_position: Option<Vector>) -> MineUpdate {
        let mut out = MineUpdate::default();
        // float.Epsilon is the smallest denormal.
        if self.next_mine_range > f32::from_bits(1) {
            self.next_mine_ttl -= dt;
            if self.next_mine_ttl < 0.0 {
                let probe = add(self.position, self.next_mine_direction);
                out.spawn_next = Some(NextMine {
                    from: self.position,
                    probe_origin: [probe[0], probe[1] + 0.75, probe[2]],
                    probe_drop: 1.5,
                    // Body orientation is Identity: Forward rotated by yaw.
                    direction: yaw_rotate([0.0, 0.0, -1.0], self.next_mine_yaw),
                    scale: self.scale,
                    range: self.next_mine_range,
                    next_direction: self.next_mine_direction,
                    next_yaw: self.next_mine_yaw,
                    distance: self.distance_between_mines,
                });
                self.next_mine_range = 0.0;
            }
        }
        if !self.owner_collision {
            self.activated = true;
            if self.detonation_timer > 0.0 {
                self.detonation_timer -= dt;
                if self.detonation_timer <= 0.0 {
                    self.detonate();
                    out.detonations += 1;
                }
            }
        }
        if let Some(owner) = owner_position {
            let d = sub(owner, self.position);
            if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > MAX_DISTANCE_SQ {
                self.detonate();
                out.detonations += 1;
            }
        }
        if !self.clip_finished {
            self.clip_time += dt * CLIP_SPEED;
            if self.clip_time > self.clip_duration {
                self.clip_time = self.clip_duration;
                self.clip_finished = true;
            }
        }
        self.owner_collision = false;
        out
    }
}

/// SpellMine's static Queue cache: GetInstance dequeues and immediately
/// re-enqueues, so the 65th mine re-initializes the oldest instance even
/// while it is live. EntityManager.AddEntity ignores an instance already in
/// the entity list, so a reused live mine keeps its update position.
#[derive(Debug, Clone, Default)]
pub struct MinePool {
    pub slots: Vec<Option<SpellMine>>,
    /// Entity-list order of slot indices.
    pub order: Vec<usize>,
    cursor: usize,
}

impl MinePool {
    pub fn new() -> Self {
        Self {
            slots: vec![None; POOL_SIZE],
            order: Vec::new(),
            cursor: 0,
        }
    }

    /// GetInstance + Initialize + AddEntity; returns the slot.
    pub fn add(&mut self, mine: SpellMine) -> usize {
        let slot = self.cursor;
        self.cursor = (self.cursor + 1) % POOL_SIZE;
        self.slots[slot] = Some(mine);
        if !self.order.contains(&slot) {
            self.order.push(slot);
        }
        slot
    }

    /// A network client's display copy of the host's live mines by slot.
    pub fn mirror(live: Vec<(usize, SpellMine)>) -> Self {
        let mut pool = Self::new();
        for (slot, mine) in live {
            if slot < POOL_SIZE && pool.slots[slot].is_none() {
                pool.slots[slot] = Some(mine);
                pool.order.push(slot);
            }
        }
        pool
    }
    pub fn get(&self, slot: usize) -> Option<&SpellMine> {
        self.slots.get(slot).and_then(Option::as_ref)
    }
    pub fn get_mut(&mut self, slot: usize) -> Option<&mut SpellMine> {
        self.slots.get_mut(slot).and_then(Option::as_mut)
    }

    /// Live mines in entity order.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &SpellMine)> {
        self.order
            .iter()
            .filter_map(|&s| self.slots[s].as_ref().map(|m| (s, m)))
    }

    /// EntityManager.RemoveDeadEntities for mines.
    pub fn remove_dead(&mut self) {
        let slots = &mut self.slots;
        self.order.retain(|&s| {
            let keep = slots[s].as_ref().is_some_and(|m| !m.removable());
            if !keep {
                slots[s] = None;
            }
            keep
        });
    }
}
