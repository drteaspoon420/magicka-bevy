//! IconRenderer.TomeMagick: the magick hint under the player. The mouse wheel
//! (KeyboardBindings PrevMagick/NextMagick) cycles through the allowed
//! magicks. The hint shows the selected magick's element combo at half alpha
//! in the queue slots, and its name below them. See research/magick-hint.md.
use crate::hud::HudQuad;
use magicka_sim::spells::{Elements, MagickType, MAGICK_COMBOS};

/// Registered pool capacity, including custom entries and native sentinel gap.
const MAGICK_COUNT: u8 = magicka_sim::spells::MAGICK_INDEX_COUNT;
/// IconRenderer.Update: text/icon alpha change per second.
const FADE_RATE: f32 = 4.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TomeMagick {
    /// mDisplayedMagick (the TomeMagick getter).
    pub displayed: MagickType,
    /// mMagickDirty: fading out before the shown combo switches.
    dirty: bool,
    /// mMagickTextAlpha.
    pub alpha: f32,
    /// The magick whose combo and name are currently drawn (the last value
    /// SetTomeMagick received; differs from `displayed` while fading out).
    pub shown: MagickType,
    /// Rewrite extension: a selected registered (mod) Magick by registry
    /// index; `displayed` is None while one is selected.
    pub registered: Option<u16>,
    /// The registered Magick currently drawn.
    pub shown_registered: Option<u16>,
}

fn combo(t: MagickType) -> Option<&'static [Elements]> {
    MAGICK_COMBOS.iter().find(|(m, _)| *m == t).map(|(_, c)| *c)
}

fn from_index(i: i32) -> MagickType {
    u8::try_from(i)
        .ok()
        .and_then(|i| MagickType::try_from(i).ok())
        .unwrap_or_default()
}

impl TomeMagick {
    /// The TomeMagick setter: walk upward from `value` to an allowed magick
    /// (past the last one wraps to None; a full cycle keeps the current one).
    /// A change restarts the fade.
    pub fn set(&mut self, value: MagickType, allowed: impl Fn(MagickType) -> bool) {
        let mut t = value;
        while t != MagickType::None && !allowed(t) {
            let mut next = t as i32 + 1;
            if next == 35 {
                next += 1;
            }
            t = if next < i32::from(MAGICK_COUNT) {
                from_index(next)
            } else {
                MagickType::None
            };
            if t == value {
                t = self.displayed;
            }
        }
        if t != self.displayed {
            self.dirty = true;
        }
        self.displayed = t;
    }

    /// KeyboardMouseController PrevMagick (scroll up): step down to the
    /// previous allowed magick; below None wraps to the last registered type.
    pub fn prev(&mut self, allowed: impl Fn(MagickType) -> bool) {
        let mut i = self.displayed as i32 - 1;
        if i == i32::from(MAGICK_COUNT) || i < 0 {
            i = i32::from(MAGICK_COUNT) - 1;
        }
        while i != 0 && (i == 35 || !allowed(from_index(i))) {
            i -= 1;
        }
        self.set(from_index(i), allowed);
    }

    /// NextMagick (scroll down): the next allowed magick, else None.
    pub fn next(&mut self, allowed: impl Fn(MagickType) -> bool) {
        let mut i = self.displayed as i32 + 1;
        while i != i32::from(MAGICK_COUNT) && (i == 35 || !allowed(from_index(i))) {
            i += 1;
        }
        if i == i32::from(MAGICK_COUNT) {
            i = 0;
        }
        self.set(from_index(i), allowed);
    }

    fn select_registered(&mut self, index: Option<u16>) {
        if self.registered != index {
            self.dirty = true;
        }
        self.registered = index;
        if index.is_some() {
            if self.displayed != MagickType::None {
                self.dirty = true;
            }
            self.displayed = MagickType::None;
        }
    }

    /// NextMagick over native Magicks, then the player's unlocked
    /// registered ones (`registered`, ascending registry indices), then
    /// back to none.
    pub fn next_with(&mut self, allowed: impl Fn(MagickType) -> bool, registered: &[u16]) {
        match self.registered {
            Some(i) => match registered.iter().find(|&&r| r > i) {
                Some(&r) => self.select_registered(Some(r)),
                None => self.select_registered(None),
            },
            None => {
                self.next(&allowed);
                if self.displayed == MagickType::None {
                    if let Some(&r) = registered.first() {
                        self.select_registered(Some(r));
                    }
                }
            }
        }
    }
    /// PrevMagick over the same cycle in reverse.
    pub fn prev_with(&mut self, allowed: impl Fn(MagickType) -> bool, registered: &[u16]) {
        match self.registered {
            Some(i) => match registered.iter().rev().find(|&&r| r < i) {
                Some(&r) => self.select_registered(Some(r)),
                None => {
                    self.select_registered(None);
                    self.prev(&allowed);
                }
            },
            None if self.displayed == MagickType::None && !registered.is_empty() => {
                self.select_registered(registered.last().copied());
            }
            None => self.prev(allowed),
        }
    }

    /// IconRenderer.Update's tome part: while dirty, fade out at 4/s, then
    /// switch the shown magick (GetMagickCombo, SetTomeMagick); otherwise
    /// fade in to 1.
    pub fn update(&mut self, dt: f32) {
        if self.dirty {
            if self.alpha > 0.0 {
                self.alpha = (self.alpha - dt * FADE_RATE).max(0.0);
            } else {
                self.dirty = false;
                self.shown = self.displayed;
                self.shown_registered = self.registered;
            }
        } else {
            self.alpha = (self.alpha + dt * FADE_RATE).min(1.0);
        }
    }

    /// The shown magick's combo (`mTomeMagickElements`; None draws no hint).
    pub fn elements(&self) -> &'static [Elements] {
        combo(self.shown).unwrap_or(&[])
    }

    /// `Magick.NAME_LOCALIZATION` key of the shown magick.
    pub fn name_key(&self) -> Option<String> {
        name_key(self.shown)
    }
}

/// `Magick.NAME_LOCALIZATION[t]` (None has no text).
pub fn name_key(t: MagickType) -> Option<String> {
    (t != MagickType::None && t != MagickType::Registered)
        .then(|| format!("#magick_{}", name_stem(t)))
}

/// `Magick.NAME_LOCALIZATION` stems (`#magick_<stem>`).
fn name_stem(t: MagickType) -> &'static str {
    use MagickType as M;
    match t {
        M::None | M::Registered => "",
        M::Revive => "revive",
        M::Grease => "grease",
        M::Haste => "haste",
        M::Invisibility => "invisibility",
        M::Teleport => "teleport",
        M::Fear => "fear",
        M::Charm => "charm",
        M::ThunderB => "thunderb",
        M::Rain => "rain",
        M::Tornado => "tornado",
        M::Blizzard => "blizzard",
        M::MeteorS => "meteors",
        M::Conflagration => "conflagration",
        M::ThunderS => "thunders",
        M::TimeWarp => "timewarp",
        M::Vortex => "vortex",
        M::SUndead => "sundead",
        M::SElemental => "selemental",
        M::SDeath => "sdeath",
        M::SPhoenix => "sphoenix",
        M::Nullify => "nullify",
        M::Corporealize => "corporealize",
        M::Ctd => "ctd",
        M::Napalm => "napalm",
        M::Portal => "portal",
        M::TractorPull => "tractorpull",
        M::ProppMagick => "proppmagick",
        M::Levitate => "levitate",
        M::ChainLightning => "chainlightning",
        M::Confuse => "confuse",
        M::Wave => "wave",
        M::PerformanceEnchantment => "performance",
        M::JudgementSpray => "judgementspray",
        M::Amalgameddon => "amalgameddon",
        M::Grow => "grow",
        M::Shrink => "shrink",
        M::Etherealize => "etherealize",
        M::Polymorph => "polymorph",
        M::Earthquake => "earthquake",
        M::Votal => "votal",
        M::KnockDownChance => "knockdownchance",
        M::SummonCross => "cross",
        M::VladZap => "vladzap",
        M::SpawnSlime => "spawnslime",
        M::SpawnSlimeOverkill => "spawnslimeoverkill",
        M::GreaseSplash => "greasesplash",
        M::EtherealClone => "etherealclone",
        M::InflictBleedDamage => "inflictbleeddamage",
        M::InflictPoisonDamage => "inflictpoisondamage",
    }
}

/// Rewrite display names where the original player's localization table
/// has no entry. These names label the newly assigned combinations.
pub fn custom_name(t: MagickType) -> Option<&'static str> {
    use MagickType as M;
    Some(match t {
        M::Grow => "Grow",
        M::Shrink => "Shrink",
        M::Etherealize => "Etherealize",
        M::Polymorph => "Polymorph",
        M::Earthquake => "Earthquake",
        M::Votal => "Votal",
        M::KnockDownChance => "Knockdown Chance",
        M::SummonCross => "Summon Cross",
        M::VladZap => "Vlad Zap",
        M::SpawnSlime => "Spawn Slime",
        M::SpawnSlimeOverkill => "Slime Overkill",
        M::GreaseSplash => "Grease Splash",
        M::EtherealClone => "Ethereal Clone",
        M::InflictBleedDamage => "Inflict Bleeding",
        M::InflictPoisonDamage => "Inflict Poison",
        _ => return None,
    })
}

/// Defines.ElementIndex for a single element bit.
fn element_index(e: Elements) -> usize {
    (f64::from(e.0).ln() / 2_f64.ln() + 0.5) as usize
}

/// RenderData.Draw, tome part: combo icons at the queue slots (X -50..50,
/// Y +44 from the projected feet), white at alpha 0.5 * alpha. Slots below
/// `queue_len` (settled queue icons, minus combining ones) are covered by
/// the queue and moved off screen.
pub fn hint_quads(tome: &TomeMagick, queue_len: usize, position: [f32; 2]) -> Vec<HudQuad> {
    hint_quads_for(tome.elements(), tome.alpha, queue_len, position)
}

/// `hint_quads` for an explicit combination (registered Magicks).
pub fn hint_quads_for(
    elements: &[Elements],
    alpha: f32,
    queue_len: usize,
    position: [f32; 2],
) -> Vec<HudQuad> {
    elements
        .iter()
        .enumerate()
        .skip(queue_len.min(5))
        .take(5)
        .map(|(slot, &e)| {
            let index = element_index(e);
            let x = position[0] - 50. + 25. * slot as f32;
            let y = position[1] + 44.;
            let u = (index % 5) as f32 * 50.;
            let v = 156. + (index / 5) as f32 * 49.;
            HudQuad {
                bounds: [x - 12.5, y - 12.5, x + 12.5, y + 12.5],
                atlas: [u, v, u + 50., v + 49.],
                color: [1., 1., 1., 0.5 * alpha],
            }
        })
        .collect()
}

/// RenderData.Draw draws the brackets whenever queue or hint icons exist.
pub fn draws_brackets(tome: &TomeMagick, queue_len: usize) -> bool {
    queue_len > 0 || !tome.elements().is_empty()
}

/// The name text anchor: centred at (x, y + 44 + 10), Maiandra14, white at
/// `alpha`.
pub fn name_anchor(position: [f32; 2]) -> [f32; 2] {
    [position[0], position[1] + 54.]
}
