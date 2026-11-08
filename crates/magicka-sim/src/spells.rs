//! Element queue, element combination and magick recognition.
//!
//! Transcribed from `Magicka.exe` (SHA-256 a896e05a…2ee7), decompiled with
//! ILSpy 9.1:
//! - `Magicka.Elements` (flags enum)
//! - `Magicka.GameLogic.Spells.Spell` (`DefaultSpell`, `operator +`,
//!   `GetSpellType`, `ElementIndex`)
//! - `Magicka.GameLogic.Spells.SpellManager` (ctor tables, `Combine`,
//!   `Opposites`, `InclusiveOpposites`, `Revertable`, `Combinable`, `Find*`,
//!   `FindOppositesAndCombinables`, `TryAddToQueue`, `HandleCombo`,
//!   `GetMagickType`, `IsMagick`, `CombineMagick`, `Equatable`)
//! - `SpellTree`/`SpellNode`, `MagickTree`/`MagickNode`
//!
//! Index-order quirks of the original loops are preserved deliberately.
//! Presentation side effects (icon renderer, chant visuals, audio, network)
//! are returned as [`QueueEvent`]s instead of performed.

use std::ops::{BitAnd, BitOr, BitOrAssign};

/// `Magicka.Elements` flags.
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, Default, Debug, serde::Serialize, serde::Deserialize,
)]
pub struct Elements(pub u32);

impl Elements {
    pub const NONE: Self = Self(0);
    pub const EARTH: Self = Self(0x1);
    pub const WATER: Self = Self(0x2);
    pub const COLD: Self = Self(0x4);
    pub const FIRE: Self = Self(0x8);
    pub const LIGHTNING: Self = Self(0x10);
    pub const ARCANE: Self = Self(0x20);
    pub const LIFE: Self = Self(0x40);
    pub const SHIELD: Self = Self(0x80);
    pub const ICE: Self = Self(0x100);
    pub const STEAM: Self = Self(0x200);
    pub const POISON: Self = Self(0x400);
    /// Also `Elements.Magick`: marks a spell slot holding a magick.
    pub const ALL: Self = Self(0xFFFF);
    pub const BASIC: Self = Self(0xFF);
    pub const PHYSICAL_ELEMENTS: Self = Self(0x101);
    pub const BEAMS: Self = Self(0x60);

    /// The 11 magnitude-carrying elements, in `ElementIndex` order.
    pub const BY_INDEX: [Self; 11] = [
        Self::EARTH,
        Self::WATER,
        Self::COLD,
        Self::FIRE,
        Self::LIGHTNING,
        Self::ARCANE,
        Self::LIFE,
        Self::SHIELD,
        Self::ICE,
        Self::STEAM,
        Self::POISON,
    ];

    /// `(a & e) == e`, the test idiom used throughout the original.
    pub fn has(self, e: Self) -> bool {
        self.0 & e.0 == e.0
    }

    pub fn intersects(self, e: Self) -> bool {
        self.0 & e.0 != 0
    }

    /// `Spell.ElementIndex`: `All` -> 15, otherwise
    /// `(int)(Math.Log(e) * (1 / Math.Log(2)) + 0.5)`. Computed exactly like
    /// the original (double precision, truncating cast) so multi-bit values
    /// map the same way. `None` maps to `i32::MIN`, as .NET's cast of -inf on
    /// x86 does (the original then indexes out of bounds; callers never pass it).
    pub fn index(self) -> i32 {
        if self == Self::ALL {
            return 15;
        }
        let one_over_ln2 = 1.0f64 / 2.0f64.ln();
        ((self.0 as f64).ln() * one_over_ln2 + 0.5) as i32
    }
}

impl BitOr for Elements {
    type Output = Self;
    fn bitor(self, r: Self) -> Self {
        Self(self.0 | r.0)
    }
}

impl BitOrAssign for Elements {
    fn bitor_assign(&mut self, r: Self) {
        self.0 |= r.0;
    }
}

impl BitAnd for Elements {
    type Output = Self;
    fn bitand(self, r: Self) -> Self {
        Self(self.0 & r.0)
    }
}

/// `Magicka.GameLogic.Spells.MagickType` (values used by the magick tree).
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, Debug, Default, serde::Serialize, serde::Deserialize,
)]
#[repr(u8)]
pub enum MagickType {
    #[default]
    None = 0,
    Revive,
    Grease,
    Haste,
    Invisibility,
    Teleport,
    Fear,
    Charm,
    ThunderB,
    Rain,
    Tornado,
    Blizzard,
    MeteorS,
    Conflagration,
    ThunderS,
    TimeWarp,
    Vortex,
    SUndead,
    SElemental,
    SDeath,
    SPhoenix,
    Nullify,
    Corporealize,
    Ctd,
    Napalm,
    Portal,
    TractorPull,
    ProppMagick,
    Levitate,
    ChainLightning,
    Confuse,
    Wave,
    PerformanceEnchantment,
    JudgementSpray,
    Amalgameddon,
    // 35 is the original NrOfMagicks sentinel, not a castable type.
    Grow = 36,
    Shrink,
    Etherealize,
    Polymorph,
    Earthquake,
    Votal,
    KnockDownChance,
    SummonCross,
    VladZap,
    SpawnSlime,
    SpawnSlimeOverkill,
    GreaseSplash,
    EtherealClone,
    InflictBleedDamage,
    InflictPoisonDamage,
    /// Rewrite extension: a mod Magick from the definition registry. The
    /// spell's magnitude slot 0 carries its registry index; it never
    /// appears in the native tree, unlock masks or tome indices.
    Registered = 63,
}
/// Index capacity, including None and the non-castable native sentinel 35.
pub const MAGICK_INDEX_COUNT: u8 = 51;
/// All registered castable magicks; original IDs and the sentinel gap survive.
pub const ALL_MAGICKS_MASK: u64 = ((1u64 << MAGICK_INDEX_COUNT) - 1) & !1 & !(1u64 << 35);
impl TryFrom<u8> for MagickType {
    type Error = ();
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        use MagickType::*;
        const TYPES: &[MagickType] = &[
            None,
            Revive,
            Grease,
            Haste,
            Invisibility,
            Teleport,
            Fear,
            Charm,
            ThunderB,
            Rain,
            Tornado,
            Blizzard,
            MeteorS,
            Conflagration,
            ThunderS,
            TimeWarp,
            Vortex,
            SUndead,
            SElemental,
            SDeath,
            SPhoenix,
            Nullify,
            Corporealize,
            Ctd,
            Napalm,
            Portal,
            TractorPull,
            ProppMagick,
            Levitate,
            ChainLightning,
            Confuse,
            Wave,
            PerformanceEnchantment,
            JudgementSpray,
            Amalgameddon,
            None, // NrOfMagicks gap; rejected below.
            Grow,
            Shrink,
            Etherealize,
            Polymorph,
            Earthquake,
            Votal,
            KnockDownChance,
            SummonCross,
            VladZap,
            SpawnSlime,
            SpawnSlimeOverkill,
            GreaseSplash,
            EtherealClone,
            InflictBleedDamage,
            InflictPoisonDamage,
        ];
        if value == 35 {
            return Err(());
        }
        if value == Registered as u8 {
            return Ok(Registered);
        }
        TYPES.get(value as usize).copied().ok_or(())
    }
}

/// `SpellManager..ctor` `AddMagick` calls in original order, followed by
/// user-authorized custom combinations (research/custom-magicks.md).
pub const MAGICK_COMBOS: &[(MagickType, &[Elements])] = {
    use Elements as E;
    use MagickType as M;
    const A: E = E::ARCANE;
    const C: E = E::COLD;
    const EA: E = E::EARTH;
    const F: E = E::FIRE;
    const I: E = E::ICE;
    const L: E = E::LIGHTNING;
    const LF: E = E::LIFE;
    const S: E = E::SHIELD;
    const ST: E = E::STEAM;
    const W: E = E::WATER;
    &[
        (M::Revive, &[LF, L]),
        (M::Haste, &[L, A, F]),
        (M::Rain, &[W, ST]),
        (M::Grease, &[W, EA, LF]),
        (M::Teleport, &[L, A, L]),
        (M::ThunderB, &[ST, L, A, L]),
        (M::Charm, &[LF, S, EA]),
        (M::MeteorS, &[F, EA, ST, EA, F]),
        (M::Fear, &[C, A, S]),
        (M::SUndead, &[I, EA, A, C]),
        (M::Conflagration, &[ST, F, ST, F, ST]),
        (M::SDeath, &[A, C, I, C, A]),
        (M::SPhoenix, &[LF, L, F]),
        (M::Invisibility, &[A, S, ST, A]),
        (M::SElemental, &[A, S, EA, ST, A]),
        (M::Blizzard, &[C, I, C]),
        (M::ThunderS, &[ST, ST, L, A, L]),
        (M::Tornado, &[EA, ST, W, ST]),
        (M::Nullify, &[A, S]),
        (M::Vortex, &[I, A, I, S, I]),
        (M::Corporealize, &[A, ST, L, S, A]),
        (M::TimeWarp, &[C, S]),
        (M::Ctd, &[L, L, F, LF]),
        (M::Napalm, &[ST, EA, LF, F, F]),
        (M::Portal, &[ST, L, S]),
        (M::TractorPull, &[EA, A]),
        (M::ProppMagick, &[F, ST, A]),
        (M::Levitate, &[ST, A, ST]),
        (M::ChainLightning, &[L, L, L]),
        (M::Confuse, &[A, S, L]),
        (M::Wave, &[EA, ST, EA, ST, EA]),
        (M::PerformanceEnchantment, &[LF, F, L, F, LF]),
        (M::JudgementSpray, &[I, I, A, S]),
        (M::Amalgameddon, &[A, W, L]),
        // Rewrite combinations, explicitly requested by the user. These
        // native effect types have no player combo in SpellManager's tree.
        // Regressions must construct each queue through try_add_to_queue.
        (M::Grow, &[LF, EA, S, EA, LF]),
        (M::Shrink, &[A, C, S, C, A]),
        (M::Etherealize, &[A, L, S, A]),
        (M::Polymorph, &[LF, W, EA, S, LF]),
        (M::Earthquake, &[EA, EA, S, EA, EA]),
        (M::Votal, &[A, EA, S, A, EA]),
        (M::KnockDownChance, &[EA, C, S, EA, C]),
        (M::SummonCross, &[LF, EA, LF, S, EA]),
        (M::VladZap, &[L, A, L, S, L]),
        (M::SpawnSlime, &[W, EA, EA, LF, S]),
        (M::SpawnSlimeOverkill, &[W, EA, LF, EA, S]),
        (M::GreaseSplash, &[W, EA, LF, S, EA]),
        (M::EtherealClone, &[A, S, C, A, EA]),
        (M::InflictBleedDamage, &[A, EA, C, S, EA]),
        (M::InflictPoisonDamage, &[LF, W, LF, EA, S]),
    ]
};

/// .NET `Math.Max(float, float)` (NaN in the first argument propagates).
fn net_max(a: f32, b: f32) -> f32 {
    if a > b || a.is_nan() {
        a
    } else {
        b
    }
}

/// .NET `Math.Min(float, float)`.
fn net_min(a: f32, b: f32) -> f32 {
    if a < b || a.is_nan() {
        a
    } else {
        b
    }
}

/// `Magicka.GameLogic.Spells.SpellType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(i32)]
pub enum SpellType {
    Push = 0,
    Spray = 1,
    Projectile = 2,
    Shield = 3,
    Beam = 4,
    Lightning = 5,
    Magick = 6,
}

/// `Magicka.GameLogic.Spells.Spell`: an element set plus per-element
/// magnitudes (indexed by [`Elements::index`]). When `element == ALL` the
/// original reinterprets the struct as a `Magick`; that is modeled by
/// `magick` here.
#[derive(Clone, Copy, PartialEq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Spell {
    pub element: Elements,
    pub mag: [f32; 11],
    pub magick: MagickType,
}

// Magnitude slots.
const EARTH: usize = 0;
const WATER: usize = 1;
const COLD: usize = 2;
const FIRE: usize = 3;
const LIGHTNING: usize = 4;
const ARCANE: usize = 5;
const LIFE: usize = 6;
const SHIELD: usize = 7;
const ICE: usize = 8;
const STEAM: usize = 9;
const POISON: usize = 10;

impl Spell {
    /// `Spell.DefaultSpell(Elements, out Spell)`: magnitude 1 for every set bit.
    pub fn default_for(elements: Elements) -> Self {
        let mut s = Spell {
            element: elements,
            ..Default::default()
        };
        for (i, e) in Elements::BY_INDEX.iter().enumerate() {
            if (elements & *e) == *e {
                s.mag[i] = 1.0;
            }
        }
        s
    }

    /// `Spell.DefaultSpell(Elements[], out Spell)`: magnitudes accumulate.
    pub fn default_for_list(list: &[Elements]) -> Self {
        let mut s = Spell::default();
        for &el in list {
            s.element |= el;
            for (i, e) in Elements::BY_INDEX.iter().enumerate() {
                if (el & *e) == *e {
                    s.mag[i] += 1.0;
                }
            }
        }
        s
    }

    /// `SpellMagickConverter { Magick = { Element = All, MagickType = t } }.Spell`.
    pub fn magick(t: MagickType) -> Self {
        Spell {
            element: Elements::ALL,
            magick: t,
            ..Default::default()
        }
    }

    /// A registered (mod) Magick; the registry index rides in magnitude
    /// slot 0, so queue mirroring and serialization carry it losslessly.
    pub fn registered_magick(index: u16) -> Self {
        let mut s = Self::magick(MagickType::Registered);
        s.mag[0] = f32::from(index);
        s
    }
    /// The registry index of a registered Magick spell.
    pub fn registered_index(&self) -> Option<u16> {
        (self.element == Elements::ALL && self.magick == MagickType::Registered)
            .then_some(self.mag[0] as u16)
    }

    pub fn get(&self, e: Elements) -> f32 {
        match Elements::BY_INDEX.iter().position(|x| *x == e) {
            Some(i) => self.mag[i],
            None => 0.0,
        }
    }

    /// `Spell.ContainsElement`.
    pub fn contains_element(&self, e: Elements) -> bool {
        self.get(e) > f32::EPSILON
    }

    /// `Spell.TotalMagnitude` (summed in the original's field order).
    pub fn total_magnitude(&self) -> f32 {
        let m = &self.mag;
        m[EARTH]
            + m[WATER]
            + m[COLD]
            + m[FIRE]
            + m[LIGHTNING]
            + m[ARCANE]
            + m[LIFE]
            + m[SHIELD]
            + m[ICE]
            + m[STEAM]
            + m[POISON]
    }

    /// `Spell.GetSpellType`.
    pub fn spell_type(&self) -> SpellType {
        let e = self.element;
        if e == Elements::ALL {
            SpellType::Magick
        } else if e == Elements::NONE {
            SpellType::Push
        } else if e.intersects(Elements::SHIELD) {
            SpellType::Shield
        } else if e.intersects(Elements::PHYSICAL_ELEMENTS) {
            SpellType::Projectile
        } else if e.intersects(Elements::BEAMS) {
            SpellType::Beam
        } else if e.intersects(Elements::LIGHTNING) && !e.has(Elements::STEAM) {
            SpellType::Lightning
        } else {
            SpellType::Spray
        }
    }

    /// `Spell.operator +`, statement for statement (f32 arithmetic in the
    /// same order; see `spells.md` on x87 precision).
    pub fn combine_with(&self, b: &Spell) -> Spell {
        let a = &self.mag;
        let b = &b.mag;
        let sum = |i: usize| a[i] + b[i];
        let mut r = [0.0f32; 11];
        r[EARTH] = net_max(sum(EARTH) - sum(LIGHTNING), 0.0);
        r[COLD] = net_max(sum(COLD) - sum(FIRE), 0.0);
        r[FIRE] = net_max(sum(FIRE) - sum(COLD), 0.0);
        r[LIGHTNING] = net_max(sum(LIGHTNING) - sum(EARTH), 0.0);
        r[POISON] = net_max(sum(POISON) - sum(LIFE), 0.0);
        r[ICE] += sum(ICE);
        if r[ICE] > 0.0 && sum(FIRE) > 0.0 {
            let num = net_min(r[ICE], sum(FIRE));
            r[WATER] += num;
            r[FIRE] -= num;
            r[ICE] -= num;
        }
        r[STEAM] += sum(STEAM);
        if r[STEAM] > 0.0 && sum(COLD) > 0.0 {
            let num = net_min(r[STEAM], sum(COLD));
            r[WATER] += num;
            r[COLD] -= num;
            r[STEAM] -= num;
        }
        r[LIFE] += sum(LIFE);
        r[ARCANE] += sum(ARCANE);
        r[SHIELD] += sum(SHIELD);
        r[WATER] += sum(WATER);
        let n = net_min(sum(WATER), sum(FIRE));
        r[STEAM] += n;
        r[WATER] -= n;
        r[FIRE] -= n;
        let n = net_min(sum(WATER), sum(COLD));
        r[ICE] += n;
        r[WATER] -= n;
        r[COLD] -= n;
        let mut element = Elements::NONE;
        for (i, e) in [
            (ARCANE, Elements::ARCANE),
            (STEAM, Elements::STEAM),
            (ICE, Elements::ICE),
            (WATER, Elements::WATER),
            (EARTH, Elements::EARTH),
            (LIGHTNING, Elements::LIGHTNING),
            (COLD, Elements::COLD),
            (FIRE, Elements::FIRE),
            (LIFE, Elements::LIFE),
            (SHIELD, Elements::SHIELD),
            (POISON, Elements::POISON),
        ] {
            if r[i] > 0.0 {
                element |= e;
            }
        }
        Spell {
            element,
            mag: r,
            magick: MagickType::None,
        }
    }

    /// `Spell.operator ==`, including its quirk: if any magnitude differs the
    /// result is decided by element equality alone.
    pub fn net_eq(&self, other: &Spell) -> bool {
        if self.mag != other.mag {
            return self.element == other.element;
        }
        true
    }
}

impl std::ops::Add for Spell {
    type Output = Spell;
    fn add(self, rhs: Spell) -> Spell {
        self.combine_with(&rhs)
    }
}

// ---- pairwise element rules (SpellManager static methods) ----

fn either_has(a: Elements, b: Elements, e: Elements) -> bool {
    a.has(e) || b.has(e)
}

/// `SpellManager.Opposites`.
pub fn opposites(a: Elements, b: Elements) -> bool {
    use Elements as E;
    (either_has(a, b, E::ARCANE) && either_has(a, b, E::LIFE))
        || (either_has(a, b, E::FIRE) && either_has(a, b, E::COLD))
        || (either_has(a, b, E::EARTH) && either_has(a, b, E::LIGHTNING))
        || (either_has(a, b, E::WATER) && either_has(a, b, E::LIGHTNING))
        || (a.has(E::SHIELD) && b.has(E::SHIELD))
}

/// `SpellManager.InclusiveOpposites`: `Opposites` plus fire/water.
pub fn inclusive_opposites(a: Elements, b: Elements) -> bool {
    opposites(a, b) || (either_has(a, b, Elements::FIRE) && either_has(a, b, Elements::WATER))
}

fn one_each(a: Elements, b: Elements, x: Elements, y: Elements) -> bool {
    (a.has(x) && b.has(y)) || (b.has(x) && a.has(y))
}

/// `SpellManager.Revertable`: cold+steam, ice+fire.
pub fn revertable(a: Elements, b: Elements) -> bool {
    one_each(a, b, Elements::COLD, Elements::STEAM) || one_each(a, b, Elements::ICE, Elements::FIRE)
}

/// `SpellManager.Combinable`: water+cold, water+fire.
pub fn combinable(a: Elements, b: Elements) -> bool {
    one_each(a, b, Elements::WATER, Elements::COLD)
        || one_each(a, b, Elements::WATER, Elements::FIRE)
}

fn find_back(list: &[Spell], last: isize, pred: impl Fn(Elements) -> bool) -> isize {
    let mut i = last;
    while i >= 0 {
        if pred(list[i as usize].element) {
            return i;
        }
        i -= 1;
    }
    -1
}

/// `SpellManager.FindOpposites`: search `list[last..=0]` backwards.
pub fn find_opposites(list: &[Spell], last: isize, e: Elements) -> isize {
    find_back(list, last, |x| opposites(e, x))
}

/// `SpellManager.FindRevertables`.
pub fn find_revertables(list: &[Spell], last: isize, e: Elements) -> isize {
    find_back(list, last, |x| revertable(e, x))
}

/// `SpellManager.FindCombines`.
pub fn find_combines(list: &[Spell], last: isize, e: Elements) -> isize {
    find_back(list, last, |x| combinable(e, x))
}

/// `SpellManager.FindDifferentElement`.
pub fn find_different_element(list: &[Spell], last: isize, e: Elements) -> isize {
    find_back(list, last, |x| x != e)
}

/// Presentation/side-effect calls of the original, reported instead of run.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum QueueEvent {
    /// `IconRenderer.AddIcon(element, ...)`.
    IconAdded(Elements),
    /// `ChantSpellManager.Add(new ChantSpells(element, owner))`.
    ChantAdded(Elements),
    /// `IconRenderer.TransformIconForRemoval(a, b)`: slots `a` and `b` cancelled.
    IconsRemoved { a: usize, b: usize },
    /// `IconRenderer.TransformIconForCombine(result, a, b)`.
    IconsCombined {
        result: Elements,
        a: usize,
        b: usize,
    },
    /// `ChantSpellManager.Merge(owner, a, b, out)` (issued once, with the
    /// last pair processed).
    ChantMerge {
        a: Elements,
        b: Elements,
        out: Elements,
    },
}

/// `SpellManager.FindOppositesAndCombinables`: three backward passes
/// (cancel opposites, then revert, then combine) over the queue.
pub fn find_opposites_and_combinables(list: &mut Vec<Spell>, events: &mut Vec<QueueEvent>) {
    let mut e1 = Elements::NONE;
    let mut e2 = Elements::NONE;
    let mut out = Elements::NONE;

    let mut num = list.len() as isize - 1;
    while num >= 0 {
        let n2 = find_opposites(list, num - 1, list[num as usize].element);
        if n2 >= 0 {
            e1 = list[n2 as usize].element;
            e2 = list[num as usize].element;
            events.push(QueueEvent::IconsRemoved {
                a: num as usize,
                b: n2 as usize,
            });
            list.remove(n2 as usize);
            num -= 1;
            list.remove(num as usize);
        }
        num -= 1;
    }

    for pass_revert in [true, false] {
        let mut i = list.len() as isize - 1;
        while i >= 0 {
            let spell = list[i as usize];
            let j = if pass_revert {
                find_revertables(list, i - 1, spell.element)
            } else {
                find_combines(list, i - 1, spell.element)
            };
            if j >= 0 {
                let value = spell + list[j as usize];
                e1 = list[j as usize].element;
                e2 = list[i as usize].element;
                out = value.element;
                events.push(QueueEvent::IconsCombined {
                    result: value.element,
                    a: i as usize,
                    b: j as usize,
                });
                list[j as usize] = value;
                list.remove(i as usize);
            }
            i -= 1;
        }
    }

    if e1 != Elements::NONE && e2 != Elements::NONE {
        events.push(QueueEvent::ChantMerge { a: e1, b: e2, out });
    }
}

/// Maximum queued elements for avatars (`Defines.SPELL_QUECAP`, also the
/// literal `5` passed by `Avatar`).
pub const SPELL_QUEUE_CAP: usize = 5;

/// `SpellManager.TryAddToQueue`. Returns `true` if the queue grew by one.
pub fn try_add_to_queue(
    list: &mut Vec<Spell>,
    max: usize,
    spell: Spell,
    events: &mut Vec<QueueEvent>,
) -> bool {
    events.push(QueueEvent::IconAdded(spell.element));
    let target = list.len() + 1;
    if list.len() < max {
        events.push(QueueEvent::ChantAdded(spell.element));
        list.push(spell);
        find_opposites_and_combinables(list, events);
    } else {
        let mut e1 = Elements::NONE;
        let mut e2 = Elements::NONE;
        let mut out = Elements::NONE;
        let last = max as isize - 1;
        let mut j = find_opposites(list, last, spell.element);
        if j >= 0 {
            e1 = list[j as usize].element;
            e2 = spell.element;
            events.push(QueueEvent::IconsRemoved {
                a: max,
                b: j as usize,
            });
            list.remove(j as usize);
        } else {
            j = find_revertables(list, last, spell.element);
            if j < 0 {
                j = find_combines(list, last, spell.element);
            }
            if j >= 0 {
                let value = spell + list[j as usize];
                e1 = list[j as usize].element;
                e2 = spell.element;
                out = value.element;
                list[j as usize] = value;
                events.push(QueueEvent::IconsCombined {
                    result: value.element,
                    a: max,
                    b: j as usize,
                });
            }
        }
        events.push(QueueEvent::ChantAdded(spell.element));
        if e1 != Elements::NONE && e2 != Elements::NONE {
            events.push(QueueEvent::ChantMerge { a: e1, b: e2, out });
        }
    }
    target == list.len()
}

/// `SpellManager.Combine`: fold the queue into the spell that gets cast.
pub fn combine(list: &[Spell]) -> Spell {
    let Some(first) = list.first() else {
        return Spell::default();
    };
    if first.element == Elements::ALL {
        return *first;
    }
    list[1..].iter().fold(*first, |acc, s| acc + *s)
}

/// `SpellManager.Equatable`.
pub fn equatable(combo: &[Elements], queue: &[Spell]) -> bool {
    combo
        .iter()
        .zip(queue)
        .all(|(e, s)| e.intersects(s.element))
}

/// `Magicka.ControllerDirection` (only the four cardinal directions and
/// `Center` have spell-tree children).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Direction {
    Center = 0,
    Right = 1,
    Up = 2,
    Left = 4,
    Down = 8,
}

/// `SpellManager.HandleCombo` over the ctor-built `SpellTree`. The tree
/// holds one element per two-step combo; inner nodes hold the default
/// (element `None`) spell.
///
/// `element_enabled` stands in for `TutorialManager.IsElementEnabled`.
pub fn handle_combo(
    input: &mut Vec<Direction>,
    element_enabled: impl Fn(Elements) -> bool,
) -> Spell {
    use Direction as D;
    // (first, second) -> element, from SpellManager..ctor.
    const TREE: [(Direction, Direction, Elements); 8] = [
        (D::Up, D::Left, Elements::LIFE),
        (D::Left, D::Up, Elements::WATER),
        (D::Right, D::Up, Elements::COLD),
        (D::Right, D::Down, Elements::FIRE),
        (D::Up, D::Right, Elements::ARCANE),
        (D::Left, D::Down, Elements::LIGHTNING),
        (D::Down, D::Right, Elements::SHIELD),
        (D::Down, D::Left, Elements::EARTH),
    ];
    // Tree position: None = root, Some(d) = first-level node `d`.
    let mut node: Option<Direction> = None;
    let mut i = 0;
    while i < input.len() {
        let dir = input[i];
        let child = match node {
            None => TREE
                .iter()
                .any(|(a, _, _)| *a == dir)
                .then_some((Some(dir), Spell::default())),
            Some(first) => TREE
                .iter()
                .find(|(a, b, _)| *a == first && *b == dir)
                .map(|(_, _, e)| (None::<Direction>, Spell::default_for(*e))),
        };
        match child {
            None => {
                // Move failed: original returns to root and drops the oldest
                // input, *then still advances i* (preserved quirk).
                node = None;
                if input.is_empty() {
                    return Spell::default();
                }
                input.remove(0);
            }
            Some((next, spell)) => {
                // A leaf has no children; any further Move from it fails and
                // resets to the root, which `next = None` reproduces.
                node = next;
                if spell.element != Elements::NONE {
                    if element_enabled(spell.element) {
                        return spell;
                    }
                    input.clear();
                }
            }
        }
        i += 1;
    }
    Spell::default()
}

/// Magick recognition over the element queue (`MagickTree`).
pub struct MagickTree {
    // Node 0 is the root; children indexed by `Elements::index()` (0..11).
    nodes: Vec<([Option<usize>; 11], MagickType)>,
}

impl Default for MagickTree {
    fn default() -> Self {
        Self::new(MAGICK_COMBOS)
    }
}

impl MagickTree {
    pub fn new(combos: &[(MagickType, &[Elements])]) -> Self {
        let mut t = MagickTree {
            nodes: vec![([None; 11], MagickType::None)],
        };
        for (m, elems) in combos {
            let mut cur = 0;
            for e in *elems {
                let idx = e.index() as usize;
                cur = match t.nodes[cur].0[idx] {
                    Some(n) => n,
                    None => {
                        t.nodes.push(([None; 11], MagickType::None));
                        let n = t.nodes.len() - 1;
                        t.nodes[cur].0[idx] = Some(n);
                        n
                    }
                };
            }
            t.nodes[cur].1 = *m;
        }
        t
    }

    /// `MagickTree.Move`: returns (moved, content).
    fn step(&self, cur: &mut usize, e: Elements) -> (bool, MagickType) {
        let idx = e.index();
        let child = if (0..11).contains(&idx) {
            self.nodes[*cur].0[idx as usize]
        } else {
            None
        };
        match child {
            None => {
                let content = self.nodes[*cur].1;
                *cur = 0;
                (false, content)
            }
            Some(n) => {
                *cur = n;
                (true, self.nodes[n].1)
            }
        }
    }

    /// Shared core of `IsMagick` / `CombineMagick`: the whole queue must be a
    /// path in the tree ending on a magick.
    pub fn exact_magick(&self, queue: &[Spell]) -> Option<MagickType> {
        let mut cur = 0;
        let mut ok = false;
        let mut m = MagickType::None;
        for s in queue {
            let (moved, content) = self.step(&mut cur, s.element);
            ok = moved;
            m = content;
            if !moved {
                break;
            }
        }
        (ok && m != MagickType::None).then_some(m)
    }

    /// `SpellManager.CombineMagick`: replace the queue with the magick spell.
    /// `allowed` stands in for `IsMagickAllowed`.
    pub fn combine_magick(
        &self,
        queue: &mut Vec<Spell>,
        allowed: impl Fn(MagickType) -> bool,
    ) -> bool {
        match self.exact_magick(queue) {
            Some(m) if allowed(m) => {
                queue.clear();
                queue.push(Spell::magick(m));
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Elements as E;

    fn q(elems: &[Elements]) -> (Vec<Spell>, Vec<QueueEvent>) {
        let mut list = Vec::new();
        let mut ev = Vec::new();
        for e in elems {
            try_add_to_queue(&mut list, SPELL_QUEUE_CAP, Spell::default_for(*e), &mut ev);
        }
        (list, ev)
    }

    fn els(list: &[Spell]) -> Vec<Elements> {
        list.iter().map(|s| s.element).collect()
    }

    #[test]
    fn element_index_matches_log2() {
        for (i, e) in Elements::BY_INDEX.iter().enumerate() {
            assert_eq!(e.index(), i as i32);
        }
        assert_eq!(E::ALL.index(), 15);
        assert_eq!(E::NONE.index(), i32::MIN);
        // multi-bit: round(log2(3)) = round(1.58) = 2
        assert_eq!((E::EARTH | E::WATER).index(), 2);
    }

    #[test]
    fn water_cold_makes_ice_and_water_fire_makes_steam() {
        let (l, _) = q(&[E::WATER, E::COLD]);
        assert_eq!(els(&l), vec![E::ICE]);
        assert_eq!(l[0].get(E::ICE), 1.0);
        let (l, _) = q(&[E::FIRE, E::WATER]);
        assert_eq!(els(&l), vec![E::STEAM]);
    }

    #[test]
    fn revert_cold_steam_and_fire_ice_to_water() {
        let (l, _) = q(&[E::WATER, E::FIRE, E::COLD]); // steam, then cold reverts it
        assert_eq!(els(&l), vec![E::WATER]);
        let (l, _) = q(&[E::WATER, E::COLD, E::FIRE]); // ice, then fire reverts it
        assert_eq!(els(&l), vec![E::WATER]);
    }

    #[test]
    fn opposites_cancel() {
        for (a, b) in [
            (E::ARCANE, E::LIFE),
            (E::FIRE, E::COLD),
            (E::EARTH, E::LIGHTNING),
            (E::WATER, E::LIGHTNING),
            (E::SHIELD, E::SHIELD),
        ] {
            let (l, ev) = q(&[a, b]);
            assert!(l.is_empty(), "{a:?}+{b:?}");
            assert!(ev.contains(&QueueEvent::IconsRemoved { a: 1, b: 0 }));
        }
        // fire/water is NOT a queue opposite (it combines to steam)
        assert!(!opposites(E::FIRE, E::WATER));
        assert!(inclusive_opposites(E::FIRE, E::WATER));
    }

    #[test]
    fn opposite_cancels_most_recent_match() {
        let (l, _) = q(&[E::LIFE, E::EARTH, E::LIFE, E::ARCANE]);
        assert_eq!(els(&l), vec![E::LIFE, E::EARTH]);
    }

    #[test]
    fn full_queue_rejects_unrelated_element_but_still_cancels() {
        let (mut l, _) = q(&[E::EARTH; 5]);
        let mut ev = Vec::new();
        assert!(!try_add_to_queue(
            &mut l,
            5,
            Spell::default_for(E::FIRE),
            &mut ev
        ));
        assert_eq!(l.len(), 5);
        assert!(!try_add_to_queue(
            &mut l,
            5,
            Spell::default_for(E::LIGHTNING),
            &mut ev
        ));
        assert_eq!(l.len(), 4, "lightning cancels one earth even when full");
    }

    #[test]
    fn full_queue_combines_into_existing_slot() {
        let (mut l, _) = q(&[E::EARTH, E::EARTH, E::EARTH, E::EARTH, E::WATER]);
        let mut ev = Vec::new();
        try_add_to_queue(&mut l, 5, Spell::default_for(E::COLD), &mut ev);
        assert_eq!(
            els(&l),
            vec![E::EARTH, E::EARTH, E::EARTH, E::EARTH, E::ICE]
        );
        assert!(ev.contains(&QueueEvent::ChantMerge {
            a: E::WATER,
            b: E::COLD,
            out: E::ICE
        }));
    }

    #[test]
    fn spell_add_cancels_and_accumulates() {
        let s = Spell::default_for(E::EARTH) + Spell::default_for(E::LIGHTNING);
        assert_eq!(s.element, E::NONE);
        let s = Spell::default_for(E::FIRE)
            + Spell::default_for(E::FIRE)
            + Spell::default_for(E::ARCANE);
        assert_eq!(s.element, E::FIRE | E::ARCANE);
        assert_eq!(s.get(E::FIRE), 2.0);
        assert_eq!(s.spell_type(), SpellType::Beam);
    }

    #[test]
    fn spell_types() {
        assert_eq!(
            Spell::default_for(E::SHIELD | E::FIRE).spell_type(),
            SpellType::Shield
        );
        assert_eq!(
            Spell::default_for(E::EARTH | E::FIRE).spell_type(),
            SpellType::Projectile
        );
        assert_eq!(
            Spell::default_for(E::ICE).spell_type(),
            SpellType::Projectile
        );
        assert_eq!(
            Spell::default_for(E::LIGHTNING).spell_type(),
            SpellType::Lightning
        );
        assert_eq!(
            Spell::default_for(E::LIGHTNING | E::STEAM).spell_type(),
            SpellType::Spray
        );
        assert_eq!(Spell::default_for(E::FIRE).spell_type(), SpellType::Spray);
        assert_eq!(Spell::default().spell_type(), SpellType::Push);
        assert_eq!(
            Spell::magick(MagickType::Haste).spell_type(),
            SpellType::Magick
        );
    }

    #[test]
    fn combine_folds_queue() {
        let (l, _) = q(&[E::FIRE, E::FIRE, E::ARCANE]);
        let s = combine(&l);
        assert_eq!(s.element, E::FIRE | E::ARCANE);
        assert_eq!(combine(&[]).element, E::NONE);
        let m = [Spell::magick(MagickType::Rain), Spell::default_for(E::FIRE)];
        assert_eq!(combine(&m).magick, MagickType::Rain);
    }

    #[test]
    fn magick_recognition() {
        let t = MagickTree::default();
        let (l, _) = q(&[E::LIGHTNING, E::ARCANE, E::FIRE]);
        assert_eq!(t.exact_magick(&l), Some(MagickType::Haste));
        let (l, _) = q(&[E::WATER, E::FIRE, E::WATER]); // steam, water -> Rain needs W,ST order
        assert_eq!(els(&l), vec![E::STEAM, E::WATER]);
        assert_eq!(t.exact_magick(&l), None);
        let (l, _) = q(&[E::WATER, E::WATER, E::FIRE]); // water, steam
        assert_eq!(t.exact_magick(&l), Some(MagickType::Rain));
        // prefix of a longer combo that is itself not a magick
        let (l, _) = q(&[E::LIGHTNING, E::ARCANE]);
        assert_eq!(t.exact_magick(&l), None);
        let mut l2 = l.clone();
        l2.push(Spell::default_for(E::LIGHTNING));
        let mut queue = l2;
        assert!(t.combine_magick(&mut queue, |_| true));
        assert_eq!(queue, vec![Spell::magick(MagickType::Teleport)]);
    }

    #[test]
    fn exact_magick_stops_at_a_failed_move() {
        // Fire moves, Life fails from the Fire node, so Arcane and Shield never
        // count, although they would spell Nullify from the root.
        let t = MagickTree::default();
        let l: Vec<Spell> = [E::FIRE, E::LIFE, E::ARCANE, E::SHIELD]
            .iter()
            .map(|e| Spell::default_for(*e))
            .collect();
        assert_eq!(t.exact_magick(&l), None);
    }

    #[test]
    fn all_magick_combos_are_recognized_when_queued_literally() {
        let t = MagickTree::default();
        for (m, combo) in MAGICK_COMBOS {
            let queue: Vec<Spell> = combo.iter().map(|e| Spell::default_for(*e)).collect();
            assert_eq!(t.exact_magick(&queue), Some(*m), "{m:?}");
        }
    }

    #[test]
    fn conjure_combos_produce_elements() {
        use Direction as D;
        for (a, b, e) in [
            (D::Right, D::Down, E::FIRE),
            (D::Up, D::Left, E::LIFE),
            (D::Down, D::Right, E::SHIELD),
            (D::Left, D::Down, E::LIGHTNING),
        ] {
            let mut input = vec![a];
            assert_eq!(handle_combo(&mut input, |_| true).element, E::NONE);
            input.push(b);
            assert_eq!(handle_combo(&mut input, |_| true).element, e);
        }
        // tutorial-disabled element clears the input queue
        let mut input = vec![D::Right, D::Down];
        assert_eq!(handle_combo(&mut input, |e| e != E::FIRE).element, E::NONE);
        assert!(input.is_empty());
    }
}
