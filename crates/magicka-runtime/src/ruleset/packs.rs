//! PackMan reward pools for Survival and Versus luggage (research/rulesets.md,
//! "Lobby pack selection"). The native pack lists are compiled into the
//! executable; the private `output/rewrite/tools/extract_survival_packs.py`
//! decodes them statically into `output/.cache/survival-packs.json`, which
//! hosts load here. Nothing from the target is embedded in source.
//!
//! PackMan holds thirteen ItemPacks then six MagickPacks (AllPacks order).
//! Each pack has a licence (HackHelper.CheckLicense: every item's or
//! magick's DLC app owned) and an Enabled flag that the lobby host toggles
//! in SubMenuCharacterSelect's "Allowed drops" list; a pack without a
//! licence is disabled and cannot be enabled. SurvivalRuleset's constructor
//! concatenates the enabled packs' items and magicks in pack order.
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackKind {
    Item,
    Magick,
}

/// One ItemPack or MagickPack.
#[derive(Clone, Debug, PartialEq)]
pub struct Pack {
    pub kind: PackKind,
    /// The localization key of the pack's name (`#pack_*`, `#mpack_*`),
    /// lowercased: the stable identity of selections and saved settings.
    pub name: String,
    /// Steam app ID of the DLC the pack belongs to (0: the base game).
    pub app: u32,
    /// Thumbnail cell in UI/Menu/PackThumbs (64 px grid: column, row).
    pub thumb: [u32; 2],
    /// Item template names (lowercase); item packs only.
    pub items: Vec<String>,
    /// MagickType values (the BookOfMagick magick index); magick packs only.
    pub magicks: Vec<u8>,
    /// The extractor's default: packs of app ID 0 are enabled.
    pub default_enabled: bool,
}

/// PackMan.AllPacks: item packs then magick packs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackCatalogue {
    pub packs: Vec<Pack>,
}

/// Item packs then magick packs: the enabled pools in pack order
/// (SurvivalRuleset constructor).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurvivalPacks {
    /// Item template names (Data/Items/Wizard/<name>), lowercase.
    pub items: Vec<String>,
    /// MagickType values (the BookOfMagick magick index).
    pub magicks: Vec<u8>,
}

/// MagickTypes SurvivalRuleset.Initialize drops from indoor scenes:
/// Blizzard, MeteorS, Napalm, Rain, SPhoenix, ThunderB, ThunderS.
pub const OUTDOOR_MAGICKS: [u8; 7] = [11, 12, 24, 9, 20, 8, 14];
/// MagickType.Corporealize (never chosen) and the fallback Revive.
pub const CORPOREALIZE: u8 = 22;
pub const REVIVE: u8 = 1;

#[derive(Debug)]
pub enum PacksError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Format(&'static str),
}

/// A pack name as the catalogue stores it: lowercase with a leading `#`.
fn pack_key(name: &str) -> String {
    let trimmed = name.trim().trim_start_matches('#').to_ascii_lowercase();
    if trimmed.is_empty() {
        trimmed
    } else {
        format!("#{trimmed}")
    }
}

impl PackCatalogue {
    /// Read the extractor's JSON: `items`/`magicks` arrays of packs with
    /// `name`, `app`, `thumb`, `enabled` and `values`.
    pub fn parse(bytes: &[u8]) -> Result<Self, PacksError> {
        let v: serde_json::Value = serde_json::from_slice(bytes).map_err(PacksError::Json)?;
        let mut packs = Vec::new();
        for (kind, key) in [(PackKind::Item, "items"), (PackKind::Magick, "magicks")] {
            let list = v[key]
                .as_array()
                .ok_or(PacksError::Format("missing pack list"))?;
            for pack in list {
                let name = pack_key(
                    pack["name"]
                        .as_str()
                        .ok_or(PacksError::Format("pack name"))?,
                );
                let app = pack["app"]
                    .as_u64()
                    .map_or(Ok(0), u32::try_from)
                    .map_err(|_| PacksError::Format("pack app"))?;
                let thumb = match pack["thumb"].as_array() {
                    Some(cell) if cell.len() == 2 => {
                        let axis = |i: usize| {
                            cell[i]
                                .as_u64()
                                .and_then(|n| u32::try_from(n).ok())
                                .ok_or(PacksError::Format("pack thumb"))
                        };
                        [axis(0)?, axis(1)?]
                    }
                    _ => [0, 0],
                };
                let default_enabled = pack["enabled"]
                    .as_bool()
                    .ok_or(PacksError::Format("pack enabled"))?;
                let values = pack["values"]
                    .as_array()
                    .ok_or(PacksError::Format("pack values"))?;
                let (items, magicks) = match kind {
                    PackKind::Item => (
                        values
                            .iter()
                            .map(|v| v.as_str().map(str::to_ascii_lowercase))
                            .collect::<Option<Vec<_>>>()
                            .ok_or(PacksError::Format("item name"))?,
                        Vec::new(),
                    ),
                    PackKind::Magick => (
                        Vec::new(),
                        values
                            .iter()
                            .map(|v| v.as_u64().and_then(|m| u8::try_from(m).ok()))
                            .collect::<Option<Vec<_>>>()
                            .ok_or(PacksError::Format("magick value"))?,
                    ),
                };
                packs.push(Pack {
                    kind,
                    name,
                    app,
                    thumb,
                    items,
                    magicks,
                    default_enabled,
                });
            }
        }
        Ok(Self { packs })
    }

    pub fn load(path: &Path) -> Result<Self, PacksError> {
        Self::parse(&std::fs::read(path).map_err(PacksError::Io)?)
    }

    /// The extractor's default output relative to the installed content
    /// root (`<repo>/target/Magicka/Content` → `<repo>/output/.cache`).
    pub fn default_cache(content: &Path) -> PathBuf {
        content.join("../../../output/.cache/survival-packs.json")
    }

    pub fn item_count(&self) -> usize {
        self.packs
            .iter()
            .filter(|p| p.kind == PackKind::Item)
            .count()
    }

    pub fn magick_count(&self) -> usize {
        self.packs.len() - self.item_count()
    }

    /// The extractor's default enablement (app ID 0 packs).
    pub fn default_enabled(&self) -> Vec<bool> {
        self.packs.iter().map(|p| p.default_enabled).collect()
    }

    /// SurvivalRuleset's constructor: the enabled packs' pools, pack order.
    pub fn pools(&self, enabled: &[bool]) -> SurvivalPacks {
        let mut out = SurvivalPacks::default();
        for (pack, on) in self.packs.iter().zip(enabled) {
            if *on {
                out.items.extend(pack.items.iter().cloned());
                out.magicks.extend(&pack.magicks);
            }
        }
        out
    }

    /// The enabled flags named by a selection spec: comma-separated pack
    /// names in any case, with or without `#`. Unknown names are ignored,
    /// so a spec made for another build still enables what it can.
    pub fn selection(&self, spec: &str) -> Vec<bool> {
        let wanted: Vec<String> = spec
            .split(',')
            .map(pack_key)
            .filter(|s| !s.is_empty())
            .collect();
        self.packs
            .iter()
            .map(|p| wanted.contains(&p.name))
            .collect()
    }

    /// The selection spec of a set of flags: enabled names without `#`.
    pub fn spec(&self, enabled: &[bool]) -> String {
        self.packs
            .iter()
            .zip(enabled)
            .filter(|(_, on)| **on)
            .map(|(p, _)| p.name.trim_start_matches('#'))
            .collect::<Vec<_>>()
            .join(",")
    }
}

impl SurvivalPacks {
    /// The extractor's default pools (its `enabled` flags).
    pub fn parse(bytes: &[u8]) -> Result<Self, PacksError> {
        let catalogue = PackCatalogue::parse(bytes)?;
        Ok(catalogue.pools(&catalogue.default_enabled()))
    }

    /// See [`PackCatalogue::default_cache`].
    pub fn default_cache(content: &Path) -> PathBuf {
        PackCatalogue::default_cache(content)
    }

    pub fn load(path: &Path) -> Result<Self, PacksError> {
        Self::parse(&std::fs::read(path).map_err(PacksError::Io)?)
    }

    /// Initialize's indoor filter (order preserved).
    pub fn for_scene(mut self, indoors: bool) -> Self {
        if indoors {
            self.magicks.retain(|m| !OUTDOOR_MAGICKS.contains(m));
        }
        self
    }

    /// SpawnLuggage's item: RANDOM.Next(count).
    pub fn pick_item(&self, next: impl FnOnce(usize) -> usize) -> Option<&str> {
        (!self.items.is_empty()).then(|| self.items[next(self.items.len())].as_str())
    }

    /// SpawnLuggage's magick: squared-uniform index (favouring early packs),
    /// rejecting Corporealize, unavailable DLC magicks and magicks a playing
    /// player has unlocked; after 100 draws, Revive.
    pub fn pick_magick(
        &self,
        mut next_double: impl FnMut() -> f64,
        mut available: impl FnMut(u8) -> bool,
        unlocked: u64,
    ) -> Option<u8> {
        if self.magicks.is_empty() {
            return None;
        }
        let mut draws = 0;
        loop {
            let i = (next_double().powi(2) * self.magicks.len() as f64) as usize;
            let m = self.magicks[i.min(self.magicks.len() - 1)];
            draws += 1;
            let ok = m != CORPOREALIZE && available(m) && unlocked & (1u64 << m) == 0;
            if draws > 100 {
                return Some(REVIVE);
            }
            if ok {
                return Some(m);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &[u8] = br##"{"items":[
        {"name":"#pack_a","app":0,"thumb":[2,0],"enabled":true,"values":["Staff_A","staff_b"]},
        {"name":"#PACK_X","app":42918,"thumb":[3,1],"enabled":false,"values":["x"]}],
      "magicks":[
        {"name":"#mpack_basic","app":0,"enabled":true,"values":[1,22,9]},
        {"name":"#mpack_napalm","app":42918,"thumb":[9,0],"enabled":false,"values":[24]}]}"##;

    fn packs() -> SurvivalPacks {
        SurvivalPacks::parse(JSON).unwrap()
    }

    #[test]
    fn enabled_pools_and_indoor_filter() {
        let p = packs();
        assert_eq!(p.items, ["staff_a", "staff_b"]);
        assert_eq!(p.magicks, [1, 22, 9]);
        assert_eq!(
            p.clone().for_scene(true).magicks,
            [1, 22],
            "Rain is outdoor-only"
        );
        assert_eq!(p.pick_item(|n| n - 1), Some("staff_b"));
    }

    #[test]
    fn magick_draws_reject_unlocked_and_fall_back_to_revive() {
        let p = packs();
        let mut draws = [0.99f64, 0.8, 0.1].into_iter();
        // 0.99² × 3 → Rain (9); 0.8² × 3 = 1.92 → Corporealize; 0.1 → Revive.
        assert_eq!(
            p.pick_magick(|| draws.next().unwrap(), |_| true, 1 << 9),
            Some(1)
        );
        assert_eq!(
            p.pick_magick(|| 0.99, |_| true, 1 << 9),
            Some(REVIVE),
            "100 draws"
        );
        assert_eq!(p.pick_magick(|| 0.99, |_| true, 0), Some(9));
    }

    /// AllPacks order, lowercased keys, the thumbnail cell (absent in older
    /// caches), and the lobby's enabled flags selecting the pools.
    #[test]
    fn catalogue_pools_follow_the_lobby_flags_in_pack_order() {
        let c = PackCatalogue::parse(JSON).unwrap();
        assert_eq!(c.item_count(), 2);
        assert_eq!(c.magick_count(), 2);
        let names: Vec<_> = c.packs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["#pack_a", "#pack_x", "#mpack_basic", "#mpack_napalm"]
        );
        assert_eq!(c.packs[1].app, 42918);
        assert_eq!(c.packs[1].thumb, [3, 1]);
        assert_eq!(c.packs[2].thumb, [0, 0], "older caches have no cell");
        assert_eq!(c.default_enabled(), [true, false, true, false]);
        let all = c.pools(&[true, true, true, true]);
        assert_eq!(all.items, ["staff_a", "staff_b", "x"]);
        assert_eq!(all.magicks, [1, 22, 9, 24]);
        let dlc_only = c.pools(&[false, true, false, true]);
        assert_eq!(dlc_only.items, ["x"]);
        assert_eq!(dlc_only.magicks, [24]);
        assert_eq!(c.pools(&[]), SurvivalPacks::default());
    }

    /// `--packs` specs: names in any case with or without `#`, unknown
    /// names ignored, and the spec of a selection reads back the same flags.
    #[test]
    fn selection_specs_round_trip_and_ignore_unknown_names() {
        let c = PackCatalogue::parse(JSON).unwrap();
        assert_eq!(
            c.selection("pack_x, #MPACK_BASIC,missing"),
            [false, true, true, false]
        );
        assert_eq!(c.selection(""), [false; 4]);
        let flags = [true, false, false, true];
        assert_eq!(c.spec(&flags), "pack_a,mpack_napalm");
        assert_eq!(c.selection(&c.spec(&flags)), flags);
        assert_eq!(c.spec(&[false; 4]), "");
    }
}
