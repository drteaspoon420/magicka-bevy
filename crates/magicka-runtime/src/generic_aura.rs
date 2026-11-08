//! General AuraStorage/BuffStorage (research/staff-items.md): item auras
//! (Item.Update executes every held item's ActiveAura with the holder as
//! owner) and special-ability auras (FireImmunityAura, LightningDamageAura,
//! LoveAura, DeflectionAura). The older resistance-only aura list in
//! `character.rs`/`buffs.rs` remains for self-cast Shield immunities.
use crate::buffs::next_buff_id;
use magicka_sim::damage::Damage;
use magicka_sim::resistance::Resistance;
use magicka_sim::spells::Elements;

/// `Buffs.AuraTarget`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuraTarget {
    Friendly,
    FriendlyButSelf,
    Enemy,
    All,
    AllButSelf,
    Own,
    Type,
    TypeButSelf,
    Faction,
    FactionButSelf,
}
impl AuraTarget {
    pub fn from_byte(b: u8) -> Self {
        use AuraTarget::*;
        [
            Friendly,
            FriendlyButSelf,
            Enemy,
            All,
            AllButSelf,
            Own,
            Type,
            TypeButSelf,
            Faction,
            FactionButSelf,
        ]
        .get(b as usize)
        .copied()
        .unwrap_or(Own)
    }
}

/// `BuffStorage`'s typed payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BuffKind {
    BoostDamage(Damage),
    /// BuffDealDamage: the owner's Damage every second from the aura owner.
    DealDamage(Damage),
    Resistance(Resistance),
    Undying,
    Boost(f32),
    ReduceAgro(f32),
    ModifyHitPoints {
        multiplier: f32,
        modifier: f32,
    },
    ModifySpellTtl {
        multiplier: f32,
        modifier: f32,
    },
    ModifySpellRange {
        multiplier: f32,
        modifier: f32,
    },
}

/// A BuffStorage as held by an aura (UniqueID shared by every copy).
#[derive(Clone, Debug, PartialEq)]
pub struct BuffSpec {
    pub id: u32,
    pub kind: BuffKind,
    pub category: u8,
    pub color: [f32; 3],
    pub ttl: f32,
    pub effect: String,
}

/// `AuraType` payloads. Boost and LifeSteal Execute do nothing natively
/// (AuraBoost is read directly by Avatar's boost).
#[derive(Clone, Debug, PartialEq)]
pub enum AuraKind {
    Buff(BuffSpec),
    Deflect { strength: f32 },
    Love { radius: f32, ttl: f32 },
    Inert,
}

/// One ActiveAura (AuraStorage + period) on a character or held item.
#[derive(Clone, Debug, PartialEq)]
pub struct GenericAura {
    pub target: AuraTarget,
    pub kind: AuraKind,
    pub category: u8,
    pub color: [f32; 3],
    pub effect: String,
    pub ttl: f32,
    /// ActiveAura.StartTTL: the TTL at AddAura (aura HUD bar full width).
    pub start_ttl: f32,
    pub radius: f32,
    pub target_types: Vec<String>,
    pub target_faction: i32,
    pub self_casted: bool,
    pub period: f32,
    /// The AddAura effect instance following the owner (presentation).
    pub effect_instance: Option<u64>,
    /// ActiveAura's ring of six decals (VisualCategory other than None).
    pub ring: Option<crate::buffs::AuraVisual>,
}

impl GenericAura {
    pub fn from_content(a: &magicka_content::common::AuraStorage) -> Self {
        use magicka_content::common::Aura;
        let kind = match &a.kind {
            Aura::Buff(b) => match buff_from_content(b) {
                Some(spec) => AuraKind::Buff(spec),
                None => AuraKind::Inert,
            },
            Aura::Deflect { strength } => AuraKind::Deflect {
                strength: *strength,
            },
            Aura::Love { radius, ttl } => AuraKind::Love {
                radius: *radius,
                ttl: *ttl,
            },
            _ => AuraKind::Inert,
        };
        Self {
            target: AuraTarget::from_byte(a.target),
            kind,
            category: a.visual_category,
            color: a.color,
            effect: a.effect.to_ascii_lowercase(),
            ttl: a.ttl,
            start_ttl: a.ttl,
            radius: a.radius,
            target_types: a
                .target_types
                .to_ascii_lowercase()
                .split(',')
                .map(str::to_owned)
                .collect(),
            target_faction: a.target_faction,
            self_casted: false,
            period: 0.0,
            effect_instance: None,
            ring: None,
        }
        .with_ring()
    }

    /// Code-built AuraStorage(…, iTargetType: null, Factions.NONE).
    pub fn new(target: AuraTarget, kind: AuraKind, ttl: f32, radius: f32, category: u8) -> Self {
        Self {
            target,
            kind,
            category,
            color: [0.0; 3],
            effect: String::new(),
            ttl,
            start_ttl: ttl,
            radius,
            target_types: Vec::new(),
            target_faction: 0,
            self_casted: false,
            period: 0.0,
            effect_instance: None,
            ring: None,
        }
        .with_ring()
    }

    pub fn with_color(mut self, color: [f32; 3]) -> Self {
        self.color = color;
        self.with_ring()
    }

    /// ActiveAura decal ring (Decal 29 + VisualCategory − 1).
    fn with_ring(mut self) -> Self {
        self.ring = (1..=3).contains(&self.category).then(|| {
            let mut ring = crate::buffs::AuraVisual::new(self.radius, self.color);
            ring.decal_cell = 29 + self.category - 1;
            ring
        });
        self
    }

    pub fn with_effect(mut self, effect: &str) -> Self {
        self.effect = effect.to_owned();
        self
    }

    /// AuraStorage.Execute's TTL step and how many periodic executions run
    /// (Buff and Love every 0.25 s; Deflect every frame, returned as 1).
    pub fn tick(&mut self, dt: f32) -> usize {
        self.ttl -= dt;
        match self.kind {
            AuraKind::Deflect { .. } => 1,
            AuraKind::Inert => 0,
            _ => {
                self.period -= dt;
                let mut fires = 0;
                while self.period <= 0.0 {
                    self.period += 0.25;
                    fires += 1;
                }
                fires
            }
        }
    }

    /// AuraBuff.Execute's target filter for one candidate character.
    pub fn accepts(&self, owner: &AuraParty, target: &AuraParty) -> bool {
        use AuraTarget::*;
        let own = owner.handle == target.handle;
        let types = || {
            self.target_types
                .iter()
                .any(|t| t.eq_ignore_ascii_case(&target.type_name))
        };
        match self.target {
            Own => own,
            All => true,
            AllButSelf => !own,
            Enemy => target.faction & owner.faction == 0,
            Friendly => target.faction & owner.faction != 0,
            FriendlyButSelf => !own && target.faction & owner.faction != 0,
            Type => types(),
            TypeButSelf => !own && types(),
            Faction => target.faction & self.target_faction != 0,
            FactionButSelf => !own && target.faction & self.target_faction != 0,
        }
    }
}

/// The aura-relevant view of a character.
#[derive(Clone, Debug)]
pub struct AuraParty {
    pub handle: u16,
    pub position: [f32; 3],
    pub radius: f32,
    pub faction: i32,
    pub type_name: String,
    pub ethereal: bool,
    pub dead: bool,
}

pub fn buff_from_content(b: &magicka_content::common::BuffStorage) -> Option<BuffSpec> {
    use magicka_content::common::Buff;
    let damage = |d: &magicka_content::common::Damage| Damage {
        attack: d.attack_property as u16,
        element: d.element as u32,
        amount: d.amount,
        magnitude: d.magnitude,
    };
    let kind = match &b.kind {
        Buff::BoostDamage(d) => BuffKind::BoostDamage(damage(d)),
        Buff::DealDamage(d) => BuffKind::DealDamage(damage(d)),
        Buff::Resistance {
            element,
            modifier,
            multiplier,
            status_resistance,
        } => BuffKind::Resistance(Resistance {
            against: Elements(*element as u32),
            multiplier: *multiplier,
            modifier: *modifier,
            status_resistance: *status_resistance,
        }),
        Buff::Undying => BuffKind::Undying,
        Buff::Boost { amount } => BuffKind::Boost(*amount),
        Buff::ReduceAgro { amount } => BuffKind::ReduceAgro(*amount),
        Buff::ModifyHitPoints {
            multiplier,
            modifier,
        } => BuffKind::ModifyHitPoints {
            multiplier: *multiplier,
            modifier: *modifier,
        },
        Buff::ModifySpellTtl {
            multiplier,
            modifier,
        } => BuffKind::ModifySpellTtl {
            multiplier: *multiplier,
            modifier: *modifier,
        },
        Buff::ModifySpellRange {
            multiplier,
            modifier,
        } => BuffKind::ModifySpellRange {
            multiplier: *multiplier,
            modifier: *modifier,
        },
        Buff::Unknown(_) => return None,
    };
    Some(BuffSpec {
        id: next_buff_id(),
        kind,
        category: b.visual_category,
        color: b.color,
        ttl: b.ttl,
        effect: b.effect.to_ascii_lowercase(),
    })
}

impl BuffSpec {
    /// BuffStorage(payload, VisualCategory, color): TTL 0.5.
    pub fn new(kind: BuffKind, category: u8, color: [f32; 3]) -> Self {
        Self {
            id: next_buff_id(),
            kind,
            category,
            color,
            ttl: 0.5,
            effect: String::new(),
        }
    }
}

/// A BuffStorage copy in Character.mBuffs.
#[derive(Clone, Debug, PartialEq)]
pub struct GenericBuff {
    pub id: u32,
    pub kind: BuffKind,
    pub category: u8,
    pub color: [f32; 3],
    pub ttl: f32,
    pub self_casted: bool,
    /// BuffDealDamage.mAuraOwnerHandle (None: 65535, the buff owner).
    pub owner: Option<u16>,
    /// BuffDealDamage.mInternalIntervall.
    pub interval: f32,
    pub decal: Option<crate::decals::Reference>,
    pub effect: String,
}

/// The general part of Character.mBuffs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GenericBuffs {
    pub list: Vec<GenericBuff>,
    /// mBuffRotation and the Offensive/Defensive/Special cycle timers.
    pub rotation: f32,
    pub cycles: [f32; 3],
}

/// Where a character's buff decals are drawn this frame.
pub struct BuffView<'a> {
    pub decals: &'a mut crate::decals::Decals,
    pub position: [f32; 3],
    pub radius: f32,
    pub height_offset: f32,
    pub in_cutscene: bool,
}

impl GenericBuffs {
    /// Character.AddBuff: a matching UniqueID only refreshes TTL; dead
    /// characters receive nothing new. Returns true for a new insertion.
    pub fn add(&mut self, spec: &BuffSpec, self_casted: bool, owner: u16, dead: bool) -> bool {
        if let Some(b) = self.list.iter_mut().find(|b| b.id == spec.id) {
            b.ttl = spec.ttl;
            return false;
        }
        if dead {
            return false;
        }
        self.list.push(GenericBuff {
            id: spec.id,
            kind: spec.kind,
            category: spec.category,
            color: spec.color,
            ttl: spec.ttl,
            self_casted,
            owner: Some(owner),
            interval: 1.0,
            decal: None,
            effect: spec.effect.clone(),
        });
        true
    }

    /// Character.Update's buff loop for the general buffs: each visible
    /// buff's decal (Decal 26 + category − 1) follows the character,
    /// rotating −0.2 rad/s and cycling alpha within its category; live
    /// buffs execute (TTL step, DealDamage interval), expired ones are
    /// removed. Returns the DealDamage hits (damage, attacker) due.
    pub fn update(
        &mut self,
        dt: f32,
        mut view: Option<BuffView<'_>>,
    ) -> Vec<(Damage, Option<u16>)> {
        use std::f32::consts::{FRAC_PI_2, PI, TAU};
        // Wrap the advanced rotation into [-π, π).
        let mut a = (self.rotation - dt * 0.2 + PI) % TAU;
        if a < 0.0 {
            a += TAU;
        }
        self.rotation = a - PI;
        let mut counts = [0usize; 3];
        for b in &self.list {
            if (1..=3).contains(&b.category) {
                counts[b.category as usize - 1] += 1;
            }
        }
        let transform = view.as_ref().map(|v| {
            // CreateFromYawPitchRoll(0, −π/2, rotation), rows 1–2 × 3·radius,
            // row 3 × radius, at the feet (HeightOffset).
            let (s, c) = self.rotation.sin_cos();
            let _ = FRAC_PI_2;
            let (h, r) = (v.radius * 3.0, v.radius);
            [
                c * h,
                0.0,
                -s * h,
                0.0,
                -s * h,
                0.0,
                -c * h,
                0.0,
                0.0,
                r,
                0.0,
                0.0,
                v.position[0],
                v.position[1] + v.height_offset,
                v.position[2],
                1.0,
            ]
        });
        let mut seen = [0usize; 3];
        let mut hits = Vec::new();
        let mut k = 0;
        while k < self.list.len() {
            let category = self.list[k].category;
            if let (Some(v), Some(transform), true) = (
                view.as_mut(),
                transform.as_ref(),
                (1..=3).contains(&category),
            ) {
                let c = category as usize - 1;
                let (index, cycle, count) = (seen[c] as f32, self.cycles[c], counts[c] as f32);
                seen[c] += 1;
                let distance = (index - cycle).abs().min((index - (cycle - count)).abs());
                let t = distance.clamp(0.0, 1.0);
                let fade = 1.0 - t * t * (3.0 - 2.0 * t);
                let alpha = if v.in_cutscene { 0.0 } else { fade * 0.8 };
                let b = &mut self.list[k];
                match b.decal {
                    Some(r) if v.decals.set(r, 1.0, transform, alpha) => {}
                    _ => {
                        b.decal = v.decals.add(crate::decals::Request {
                            cell: 26 + category - 1,
                            scale: [v.radius * 1.5, v.radius * 1.5],
                            position: v.position,
                            direction: None,
                            normal: [0.0, 1.0, 0.0],
                            ttl: 1.0,
                            color: [b.color[0], b.color[1], b.color[2], 0.0],
                            animated_owner: None,
                        })
                    }
                }
            }
            let b = &mut self.list[k];
            if b.ttl > 0.0 {
                b.ttl -= dt;
                if let BuffKind::DealDamage(damage) = b.kind {
                    b.interval -= dt;
                    if b.interval <= 0.0 {
                        hits.push((damage, b.owner));
                        b.interval = 1.0;
                    }
                }
                k += 1;
                continue;
            }
            self.list.remove(k);
            if (1..=3).contains(&category) {
                let cycle = &mut self.cycles[category as usize - 1];
                if *cycle > k as f32 {
                    *cycle -= 1.0;
                }
            }
        }
        for (cycle, count) in self.cycles.iter_mut().zip(counts) {
            *cycle = if count > 1 {
                (*cycle + dt) % count as f32
            } else {
                (*cycle - dt).max(0.0)
            };
        }
        hits
    }

    pub fn resistances(&self) -> impl Iterator<Item = Resistance> + '_ {
        self.list.iter().filter_map(|b| match b.kind {
            BuffKind::Resistance(r) => Some(r),
            _ => None,
        })
    }

    pub fn damage_boosts(&self) -> impl Iterator<Item = Damage> + '_ {
        self.list.iter().filter_map(|b| match b.kind {
            BuffKind::BoostDamage(d) => Some(d),
            _ => None,
        })
    }

    /// Character.GetSpellTTLModifier as (multiplier, modifier).
    pub fn spell_ttl_modifier(&self) -> (f32, f32) {
        self.list.iter().fold((1.0, 0.0), |(m, a), b| match b.kind {
            BuffKind::ModifySpellTtl {
                multiplier,
                modifier,
            } => (m * multiplier, a + modifier),
            _ => (m, a),
        })
    }

    /// Character.GetSpellRangeModifier as (multiplier, modifier).
    pub fn spell_range_modifier(&self) -> (f32, f32) {
        self.list.iter().fold((1.0, 0.0), |(m, a), b| match b.kind {
            BuffKind::ModifySpellRange {
                multiplier,
                modifier,
            } => (m * multiplier, a + modifier),
            _ => (m, a),
        })
    }

    /// Character.GetAgroMultiplier.
    pub fn agro_multiplier(&self) -> f32 {
        self.list.iter().fold(1.0, |m, b| match b.kind {
            BuffKind::ReduceAgro(amount) => m * amount,
            _ => m,
        })
    }

    /// Avatar boost: `mBoosts *= (int)BuffBoost.Amount` per Boost buff.
    pub fn boost_amounts(&self) -> impl Iterator<Item = f32> + '_ {
        self.list.iter().filter_map(|b| match b.kind {
            BuffKind::Boost(amount) => Some(amount),
            _ => None,
        })
    }

    /// BuffUndying.Execute sets Undying while a live Undying buff executes.
    pub fn undying(&self) -> bool {
        self.list
            .iter()
            .any(|b| b.kind == BuffKind::Undying && b.ttl > 0.0)
    }

    /// BuffModifyHitPoints.Execute over live buffs, in list order.
    pub fn modify_max_hit_points(&self, max: &mut f32) {
        for b in &self.list {
            if let BuffKind::ModifyHitPoints {
                multiplier,
                modifier,
            } = b.kind
            {
                *max *= multiplier;
                *max += modifier;
            }
        }
    }

    pub fn clear(&mut self) {
        self.list.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn party(handle: u16, faction: i32) -> AuraParty {
        AuraParty {
            handle,
            position: [0.0; 3],
            radius: 0.5,
            faction,
            type_name: "wizard".into(),
            ethereal: false,
            dead: false,
        }
    }

    #[test]
    fn targets_follow_faction_bits_and_self() {
        let spec = BuffSpec::new(BuffKind::Undying, 0, [0.0; 3]);
        let aura = |t| GenericAura::new(t, AuraKind::Buff(spec.clone()), 1.0, 5.0, 0);
        let (me, ally, foe) = (party(1, 1), party(2, 3), party(3, 4));
        use AuraTarget::*;
        assert!(aura(Friendly).accepts(&me, &me));
        assert!(aura(Friendly).accepts(&me, &ally));
        assert!(!aura(Friendly).accepts(&me, &foe));
        assert!(!aura(FriendlyButSelf).accepts(&me, &me));
        assert!(aura(Enemy).accepts(&me, &foe));
        assert!(!aura(AllButSelf).accepts(&me, &me));
        assert!(aura(Own).accepts(&me, &me) && !aura(Own).accepts(&me, &ally));
    }

    #[test]
    fn deal_damage_waits_a_second_and_refresh_keeps_interval() {
        let spec = BuffSpec {
            ttl: 1.0,
            ..BuffSpec::new(
                BuffKind::DealDamage(Damage {
                    attack: 32,
                    element: Elements::LIFE.0,
                    amount: -75.0,
                    magnitude: 1.0,
                }),
                3,
                [0.0; 3],
            )
        };
        let mut b = GenericBuffs::default();
        assert!(b.add(&spec, false, 9, false));
        let mut hits = 0;
        for frame in 0..120 {
            if frame % 15 == 0 {
                assert!(!b.add(&spec, false, 9, false) || frame == 0);
            }
            hits += b.update(1.0 / 60.0, None).len();
        }
        assert!((1..=2).contains(&hits), "{hits}");
        assert_eq!(b.list[0].owner, Some(9));
    }

    #[test]
    fn modifiers_multiply_and_add_in_list_order() {
        let mut b = GenericBuffs::default();
        for (m, a) in [(2.0, 0.0), (1.0, 5.0)] {
            b.add(
                &BuffSpec::new(
                    BuffKind::ModifySpellTtl {
                        multiplier: m,
                        modifier: a,
                    },
                    0,
                    [0.0; 3],
                ),
                false,
                1,
                false,
            );
        }
        assert_eq!(b.spell_ttl_modifier(), (2.0, 5.0));
        assert_eq!(b.spell_range_modifier(), (1.0, 0.0));
        // TTL 0 buffs are present for the frame they are added, then removed.
        b.add(
            &BuffSpec {
                ttl: 0.0,
                ..BuffSpec::new(BuffKind::ReduceAgro(0.5), 0, [0.0; 3])
            },
            false,
            1,
            false,
        );
        assert_eq!(b.agro_multiplier(), 0.5);
        b.update(0.0, None);
        assert_eq!(b.agro_multiplier(), 1.0);
    }
}
