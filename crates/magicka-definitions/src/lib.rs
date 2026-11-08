//! Typed, data-driven definitions (rewrite extension, plan workstream D).
//!
//! Authored KV files under the mod filesystem compile to typed registries:
//!
//! | Directory                   | Root block   | Type          |
//! |-----------------------------|--------------|---------------|
//! | `scripts/npc/items/`        | `Items`      | [`ItemDef`]   |
//! | `scripts/npc/abilities/`    | `Abilities`  | [`AbilityDef`]|
//! | `scripts/npc/modifiers/`    | `Modifiers`  | [`ModifierDef`]|
//! | `scripts/npc/magicks/`      | `Magicks`    | [`MagickDef`] |
//! | `scripts/npc/spells/`       | `Spells`     | [`SpellDef`]  |
//! | `scripts/npc/units/`        | `Units`      | [`UnitDef`]   |
//! | `scripts/npc/effects/`      | `Effects`    | [`EffectDef`] |
//! | `scripts/levels/`           | `Levels`     | [`LevelDef`]  |
//! | `scripts/npc/patches/`      | `Patches`    | typed patches |
//! | `localization/<lang>/`      | `Tokens`     | strings       |
//!
//! Built-in Magicks are registered from the native tables first, so
//! original behaviour is unchanged without mods; KV may reconfigure them.
pub mod defs;
pub mod schema;

pub use defs::*;
use magicka_mods::{
    kv::{self, KvNode, KvValue},
    vfs::LogicalPath,
    ModSet,
};
use magicka_sim::spells::{Elements, MagickType, Spell, MAGICK_COMBOS, SPELL_QUEUE_CAP};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Bumped when a definition schema changes incompatibly.
pub const SCHEMA_VERSION: u32 = 1;
/// The namespace of authored base definitions and built-ins.
pub const BASE_NAMESPACE: &str = "base";
const MAX_INCLUDE_DEPTH: usize = 8;

/// A stable, namespaced logical name (`base:revive`, `example:storm_lance`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DefinitionId(String);
impl DefinitionId {
    /// Parse `ns:name`; an unqualified name takes `namespace`.
    pub fn parse(raw: &str, namespace: &str) -> Result<Self, String> {
        let raw = raw.trim().to_ascii_lowercase();
        let (ns, name) = raw.split_once(':').unwrap_or((namespace, &raw));
        let ok = |s: &str| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        };
        if !ok(ns) || !ok(name) {
            return Err(format!("invalid definition id {raw:?}"));
        }
        Ok(Self(format!("{ns}:{name}")))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn namespace(&self) -> &str {
        self.0.split_once(':').unwrap().0
    }
}
impl std::fmt::Display for DefinitionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a definition was authored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    pub file: String,
    pub line: u32,
}

/// A match-local compact index; valid only for one registry hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RegistryIndex(pub u16);

/// Definitions of one type in deterministic index order.
#[derive(Clone, Debug)]
pub struct Registry<T> {
    entries: Vec<T>,
    index: BTreeMap<DefinitionId, RegistryIndex>,
}
impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            index: BTreeMap::new(),
        }
    }
}
pub trait Defined {
    fn common(&self) -> &Common;
    fn id(&self) -> &DefinitionId {
        &self.common().id
    }
}
macro_rules! defined {
    ($($t:ty),*) => {$(impl Defined for $t { fn common(&self) -> &Common { &self.common } })*};
}
defined!(
    ItemDef,
    AbilityDef,
    ModifierDef,
    MagickDef,
    SpellDef,
    UnitDef,
    EffectDef,
    LevelDef
);

impl<T: Defined> Registry<T> {
    fn from_entries(entries: Vec<T>) -> Self {
        let index = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id().clone(), RegistryIndex(i as u16)))
            .collect();
        Self { entries, index }
    }
    pub fn get(&self, id: &DefinitionId) -> Option<&T> {
        self.index.get(id).map(|i| &self.entries[i.0 as usize])
    }
    pub fn get_str(&self, id: &str) -> Option<&T> {
        self.get(&DefinitionId::parse(id, BASE_NAMESPACE).ok()?)
    }
    pub fn index_of(&self, id: &DefinitionId) -> Option<RegistryIndex> {
        self.index.get(id).copied()
    }
    pub fn at(&self, index: RegistryIndex) -> Option<&T> {
        self.entries.get(index.0 as usize)
    }
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn contains(&self, id: &DefinitionId) -> bool {
        self.index.contains_key(id)
    }
}

/// Localized strings with a fallback chain (requested language → English).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Localization {
    /// language → key (without `#`) → text.
    pub tokens: BTreeMap<String, BTreeMap<String, String>>,
}
impl Localization {
    pub fn lookup(&self, language: &str, key: &str) -> Option<&str> {
        let key = key.trim_start_matches('#').to_ascii_lowercase();
        [language.to_ascii_lowercase(), "english".into()]
            .iter()
            .find_map(|l| self.tokens.get(l)?.get(&key))
            .map(String::as_str)
    }
}

/// Every resolved registry of one match.
#[derive(Clone, Debug, Default)]
pub struct Definitions {
    pub items: Registry<ItemDef>,
    pub abilities: Registry<AbilityDef>,
    pub modifiers: Registry<ModifierDef>,
    pub magicks: Registry<MagickDef>,
    pub spells: Registry<SpellDef>,
    pub units: Registry<UnitDef>,
    pub effects: Registry<EffectDef>,
    pub levels: Registry<LevelDef>,
    pub localization: Localization,
    /// Mod UI panels (presentation only).
    pub panels: Vec<UiPanel>,
    /// Non-fatal diagnostics (e.g. missing localization).
    pub warnings: Vec<String>,
}

/// Definition loading failed; every problem found is listed.
#[derive(Clone, Debug, PartialEq)]
pub struct DefinitionErrors(pub Vec<String>);
impl std::fmt::Display for DefinitionErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for e in &self.0 {
            writeln!(f, "{e}")?;
        }
        Ok(())
    }
}
impl std::error::Error for DefinitionErrors {}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Kind {
    Items,
    Abilities,
    Modifiers,
    Magicks,
    Spells,
    Units,
    Effects,
    Levels,
}
const KINDS: [(Kind, &str, &str); 8] = [
    (Kind::Items, "scripts/npc/items", "Items"),
    (Kind::Abilities, "scripts/npc/abilities", "Abilities"),
    (Kind::Modifiers, "scripts/npc/modifiers", "Modifiers"),
    (Kind::Magicks, "scripts/npc/magicks", "Magicks"),
    (Kind::Spells, "scripts/npc/spells", "Spells"),
    (Kind::Units, "scripts/npc/units", "Units"),
    (Kind::Effects, "scripts/npc/effects", "Effects"),
    (Kind::Levels, "scripts/levels", "Levels"),
];
fn kind_by_root(name: &str) -> Option<Kind> {
    KINDS
        .iter()
        .find(|(_, _, root)| root.eq_ignore_ascii_case(name))
        .map(|(k, _, _)| *k)
}

/// The namespace of a VFS layer (`base.game` → `base`).
fn namespace(layer: &str) -> &str {
    if layer == magicka_mods::BASE_ID {
        BASE_NAMESPACE
    } else {
        layer
    }
}

/// Parse a KV file and splice its `#base` includes (resolved relative to
/// the file, through the VFS only) before its own nodes.
fn load_kv(
    set: &ModSet,
    path: &LogicalPath,
    depth: usize,
    chain: &mut Vec<LogicalPath>,
    errors: &mut Vec<String>,
) -> Vec<KvNode> {
    if depth > MAX_INCLUDE_DEPTH || chain.contains(path) {
        errors.push(format!(
            "{path}: #base cycle or nesting deeper than {MAX_INCLUDE_DEPTH}"
        ));
        return vec![];
    }
    let text = match set.vfs.read_string(path) {
        Some(Ok(t)) => t,
        Some(Err(e)) => {
            errors.push(e);
            return vec![];
        }
        None => {
            errors.push(format!("{path}: not found"));
            return vec![];
        }
    };
    let doc = match kv::parse(path.as_str(), &text) {
        Ok(d) => d,
        Err(e) => {
            errors.push(e.to_string());
            return vec![];
        }
    };
    chain.push(path.clone());
    let mut nodes = Vec::new();
    for (base, span) in &doc.bases {
        match path.sibling(base) {
            Ok(included) => nodes.extend(load_kv(set, &included, depth + 1, chain, errors)),
            Err(e) => errors.push(format!("{path}:{span}: #base {base:?}: {e}")),
        }
    }
    chain.pop();
    nodes.extend(doc.nodes);
    nodes
}

/// A definition's KV block with where it came from.
struct Raw {
    kind: Kind,
    node: KvNode,
    file: String,
    namespace: String,
}

fn set_child(node: &mut KvNode, key: &str, value: KvValue) {
    let KvValue::Block(children) = &mut node.value else {
        return;
    };
    match children
        .iter_mut()
        .find(|c| c.key.eq_ignore_ascii_case(key))
    {
        Some(c) => c.value = value,
        None => children.push(KvNode {
            key: key.into(),
            value,
            span: node.span.clone(),
        }),
    }
}

impl Definitions {
    /// Built-ins only (an unmodded match).
    pub fn builtin() -> Self {
        Self {
            magicks: Registry::from_entries(builtin_magicks()),
            ..Default::default()
        }
    }

    /// Compile every definition visible through `set`, apply typed patches
    /// in load order, and validate references. Errors are collected so an
    /// author sees all of them before a match starts.
    pub fn load(set: &ModSet) -> Result<Self, DefinitionErrors> {
        let mut errors = Vec::new();
        let layer_rank: BTreeMap<String, usize> = set
            .vfs
            .layers()
            .iter()
            .enumerate()
            .map(|(i, l)| (l.name.clone(), i))
            .collect();
        let ranked = |prefix: &str| {
            let mut files: Vec<(usize, LogicalPath, String)> = set
                .vfs
                .list(prefix)
                .into_iter()
                .filter(|p| p.extension() == Some("kv"))
                .map(|p| {
                    let layer = set.vfs.resolve(&p).unwrap().layer;
                    (layer_rank[&layer], p, layer)
                })
                .collect();
            files.sort();
            files
        };
        // Collect raw blocks by ID.
        let mut raw: BTreeMap<(Kind, DefinitionId), Raw> = BTreeMap::new();
        for (kind, dir, root) in KINDS {
            for (_, path, layer) in ranked(dir) {
                let ns = namespace(&layer).to_string();
                for top in load_kv(set, &path, 0, &mut vec![], &mut errors) {
                    if !top.key.eq_ignore_ascii_case(root) || top.str().is_some() {
                        errors.push(format!(
                            "{path}:{}: expected a \"{root}\" block, found {:?}",
                            top.span, top.key
                        ));
                        continue;
                    }
                    for node in top.children() {
                        let Ok(id) = DefinitionId::parse(&node.key, &ns) else {
                            errors.push(format!("{path}:{}: invalid id {:?}", node.span, node.key));
                            continue;
                        };
                        if node.str().is_some() {
                            errors.push(format!("{path}:{}: {id} must be a block", node.span));
                            continue;
                        }
                        if let Some(old) = raw.get(&(kind, id.clone())) {
                            errors.push(format!(
                                "{path}:{}: duplicate id {id} (also in {})",
                                node.span, old.file
                            ));
                            continue;
                        }
                        raw.insert(
                            (kind, id),
                            Raw {
                                kind,
                                node: node.clone(),
                                file: path.to_string(),
                                namespace: ns.clone(),
                            },
                        );
                    }
                }
            }
        }
        // Typed patches, in mod order: set listed fields, remove listed ones.
        for (_, path, layer) in ranked("scripts/npc/patches") {
            let ns = namespace(&layer).to_string();
            for top in load_kv(set, &path, 0, &mut vec![], &mut errors) {
                if !top.key.eq_ignore_ascii_case("Patches") {
                    errors.push(format!("{path}:{}: expected a \"Patches\" block", top.span));
                    continue;
                }
                for category in top.children() {
                    let Some(kind) = kind_by_root(&category.key) else {
                        errors.push(format!(
                            "{path}:{}: unknown patch category {}",
                            category.span, category.key
                        ));
                        continue;
                    };
                    for patch in category.children() {
                        let Ok(id) = DefinitionId::parse(&patch.key, &ns) else {
                            errors
                                .push(format!("{path}:{}: invalid id {:?}", patch.span, patch.key));
                            continue;
                        };
                        let target = match raw.entry((kind, id.clone())) {
                            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                            std::collections::btree_map::Entry::Vacant(e) => {
                                if !(kind == Kind::Magicks && builtin_id_exists(&id)) {
                                    errors.push(format!(
                                        "{path}:{}: patch targets unknown {id}",
                                        patch.span
                                    ));
                                    continue;
                                }
                                // Reconfiguring a built-in Magick: start from an empty block.
                                e.insert(Raw {
                                    kind,
                                    node: KvNode {
                                        key: id.to_string(),
                                        value: KvValue::Block(vec![]),
                                        span: patch.span.clone(),
                                    },
                                    file: path.to_string(),
                                    namespace: ns.clone(),
                                })
                            }
                        };
                        for field in patch.children() {
                            if field.key.eq_ignore_ascii_case("Remove") {
                                let names: Vec<String> = match &field.value {
                                    KvValue::Str(s) => s
                                        .split([',', ' '])
                                        .filter(|s| !s.is_empty())
                                        .map(str::to_string)
                                        .collect(),
                                    KvValue::Block(b) => b.iter().map(|n| n.key.clone()).collect(),
                                };
                                if let KvValue::Block(children) = &mut target.node.value {
                                    for name in names {
                                        let before = children.len();
                                        children.retain(|c| !c.key.eq_ignore_ascii_case(&name));
                                        if children.len() == before {
                                            errors.push(format!(
                                                "{path}:{}: cannot remove {name}: {id} has no such field",
                                                field.span
                                            ));
                                        }
                                    }
                                }
                            } else {
                                set_child(&mut target.node, &field.key, field.value.clone());
                            }
                        }
                    }
                }
            }
        }
        // Compile.
        let mut defs = Definitions::default();
        let mut items = Vec::new();
        let mut abilities = Vec::new();
        let mut modifiers = Vec::new();
        let mut magicks: BTreeMap<DefinitionId, MagickDef> = BTreeMap::new();
        let mut spells = Vec::new();
        let mut units = Vec::new();
        let mut effects = Vec::new();
        let mut levels = Vec::new();
        for ((_, id), r) in raw {
            let mut ctx = Ctx {
                file: &r.file,
                namespace: &r.namespace,
                errors: &mut errors,
            };
            match r.kind {
                Kind::Items => items.push(defs::item(&mut ctx, &r.node, id)),
                Kind::Abilities => abilities.push(defs::ability(&mut ctx, &r.node, id)),
                Kind::Modifiers => modifiers.push(defs::modifier(&mut ctx, &r.node, id)),
                Kind::Magicks => {
                    let builtin = builtin_magicks().into_iter().find(|b| b.common.id == id);
                    let mut node = r.node.clone();
                    if let Some(b) = &builtin {
                        // A built-in keeps its combination and native handler
                        // unless the KV states otherwise.
                        if node.get("Combination").is_none() {
                            let combo: Vec<&str> =
                                b.combination.iter().map(|e| element_name(*e)).collect();
                            set_child(&mut node, "Combination", KvValue::Str(combo.join(",")));
                        }
                    }
                    let mut m = defs::magick(&mut ctx, &node, id.clone());
                    if let Some(b) = builtin {
                        m.legacy = b.legacy;
                        if m.handler == Handler::None {
                            m.handler = b.handler;
                        }
                        m.unlocked_by_default |=
                            node.get("UnlockedByDefault").is_none() && b.unlocked_by_default;
                    }
                    magicks.insert(id, m);
                }
                Kind::Spells => spells.push(defs::spell(&mut ctx, &r.node, id)),
                Kind::Units => units.push(defs::unit(&mut ctx, &r.node, id)),
                Kind::Effects => effects.push(defs::effect(&mut ctx, &r.node, id)),
                Kind::Levels => levels.push(defs::level(&mut ctx, &r.node, id)),
            }
        }
        // Built-ins first in legacy order, then custom Magicks by ID.
        let mut ordered: Vec<MagickDef> = builtin_magicks()
            .into_iter()
            .map(|b| magicks.remove(&b.common.id).unwrap_or(b))
            .collect();
        ordered.extend(magicks.into_values());
        defs.items = Registry::from_entries(items);
        defs.abilities = Registry::from_entries(abilities);
        defs.modifiers = Registry::from_entries(modifiers);
        defs.magicks = Registry::from_entries(ordered);
        defs.spells = Registry::from_entries(spells);
        defs.units = Registry::from_entries(units);
        defs.effects = Registry::from_entries(effects);
        defs.levels = Registry::from_entries(levels);
        // Localization.
        for (_, path, _) in ranked("localization") {
            let lang = path
                .as_str()
                .strip_prefix("localization/")
                .and_then(|r| r.split('/').next())
                .filter(|l| !l.ends_with(".kv"))
                .unwrap_or("english")
                .to_string();
            for top in load_kv(set, &path, 0, &mut vec![], &mut errors) {
                if !top.key.eq_ignore_ascii_case("Tokens") {
                    errors.push(format!("{path}:{}: expected a \"Tokens\" block", top.span));
                    continue;
                }
                let table = defs.localization.tokens.entry(lang.clone()).or_default();
                for t in top.children() {
                    match t.str() {
                        Some(text) => {
                            table.insert(
                                t.key.trim_start_matches('#').to_ascii_lowercase(),
                                text.into(),
                            );
                        }
                        None => {
                            errors.push(format!("{path}:{}: token {} must be text", t.span, t.key))
                        }
                    }
                }
            }
        }
        // UI panels (presentation; not part of the rules hash).
        for (_, path, layer) in ranked("ui") {
            let ns = namespace(&layer).to_string();
            for top in load_kv(set, &path, 0, &mut vec![], &mut errors) {
                if !top.key.eq_ignore_ascii_case("Panels") {
                    errors.push(format!("{path}:{}: expected a \"Panels\" block", top.span));
                    continue;
                }
                for p in top.children() {
                    let Ok(id) = DefinitionId::parse(&p.key, &ns) else {
                        errors.push(format!("{path}:{}: invalid id {:?}", p.span, p.key));
                        continue;
                    };
                    let lines = p
                        .get("Lines")
                        .map(|l| {
                            l.children()
                                .iter()
                                .filter_map(|c| c.str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    let list = match p.get("List") {
                        None => None,
                        Some(l) => {
                            match (l.get_str("Source"), l.get_str("Item"), l.get_str("Action")) {
                                (Some(s), Some(i), Some(a))
                                    if s.contains(':') && a.contains('.') =>
                                {
                                    Some(UiList {
                                        source: s.into(),
                                        item: i.into(),
                                        action: a.into(),
                                    })
                                }
                                _ => {
                                    errors.push(format!(
                                    "{path}:{}: a List needs Source (table:key), Item and Action (mod.action)",
                                    l.span
                                ));
                                    None
                                }
                            }
                        }
                    };
                    defs.panels.push(UiPanel {
                        id,
                        title: p.get_str("Title").unwrap_or_default().to_string(),
                        lines,
                        list,
                        players_only: p.get_str("PlayersOnly") != Some("0"),
                    });
                }
            }
        }
        defs.validate(set, &mut errors);
        if errors.is_empty() {
            Ok(defs)
        } else {
            Err(DefinitionErrors(errors))
        }
    }

    fn validate(&mut self, set: &ModSet, errors: &mut Vec<String>) {
        let at = |c: &Common| format!("{}:{}: {}", c.source.file, c.source.line, c.id);
        let refs =
            |c: &Common, what: &str, id: &DefinitionId, ok: bool, errors: &mut Vec<String>| {
                if !ok {
                    errors.push(format!("{}: unknown {what} {id}", at(c)));
                }
            };
        for i in self.items.iter() {
            if let Some(a) = &i.active_ability {
                refs(&i.common, "ability", a, self.abilities.contains(a), errors);
            }
            for m in &i.modifiers {
                refs(&i.common, "modifier", m, self.modifiers.contains(m), errors);
            }
        }
        for a in self.abilities.iter() {
            for m in &a.modifiers {
                refs(&a.common, "modifier", m, self.modifiers.contains(m), errors);
            }
        }
        for m in self.modifiers.iter() {
            if let Some(e) = &m.effect {
                refs(&m.common, "effect", e, self.effects.contains(e), errors);
            }
        }
        for s in self.spells.iter() {
            for e in &s.effects {
                refs(&s.common, "effect", e, self.effects.contains(e), errors);
            }
            if s.elements.is_empty() || s.elements.len() > SPELL_QUEUE_CAP {
                errors.push(format!(
                    "{}: a spell needs 1..={SPELL_QUEUE_CAP} elements",
                    at(&s.common)
                ));
            }
        }
        for u in self.units.iter() {
            for a in &u.abilities {
                refs(&u.common, "ability", a, self.abilities.contains(a), errors);
            }
            for i in u.equipment.values() {
                refs(&u.common, "item", i, self.items.contains(i), errors);
            }
        }
        // Lua handlers must exist in the mod filesystem.
        let handlers: Vec<(&Common, &Handler)> = self
            .abilities
            .iter()
            .map(|a| (&a.common, &a.handler))
            .chain(self.modifiers.iter().map(|m| (&m.common, &m.handler)))
            .chain(self.magicks.iter().map(|m| (&m.common, &m.handler)))
            .chain(self.spells.iter().map(|s| (&s.common, &s.handler)))
            .chain(self.levels.iter().map(|l| (&l.common, &l.script)))
            .collect();
        for (c, h) in handlers {
            if let Handler::Lua { file, function } = h {
                let ok = LogicalPath::new(&format!("scripts/vscripts/{file}"))
                    .ok()
                    .is_some_and(|p| set.vfs.resolve(&p).is_some());
                if !ok {
                    errors.push(format!(
                        "{}: script scripts/vscripts/{file} not found",
                        at(c)
                    ));
                }
                if !function
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                    || function.is_empty()
                {
                    errors.push(format!("{}: invalid handler name {function:?}", at(c)));
                }
            }
            if let Handler::Builtin(name) = h {
                if !c.id.as_str().starts_with("base:")
                    && MagickType::try_from(builtin_value(name)).is_err()
                {
                    errors.push(format!("{}: unknown builtin handler {name:?}", at(c)));
                }
            }
        }
        // Assets: `content:` paths are the user's retail files (checked at
        // runtime); anything else must exist in the mod filesystem.
        let mut assets: Vec<(&Common, &str)> = Vec::new();
        for i in self.items.iter() {
            assets.extend(i.model.as_deref().map(|m| (&i.common, m)));
        }
        for e in self.effects.iter() {
            assets.push((&e.common, &e.asset));
        }
        for m in self.magicks.iter() {
            for r in [&m.particle, &m.sound].into_iter().flatten() {
                if !(r.contains(':') && !r.starts_with("content:")) {
                    assets.push((&m.common, r));
                } else {
                    match DefinitionId::parse(r, BASE_NAMESPACE) {
                        Ok(id) if self.effects.contains(&id) => {}
                        _ => errors.push(format!("{}: unknown effect {r}", at(&m.common))),
                    }
                }
            }
        }
        let all_common: Vec<&Common> = self.all_common().collect();
        for c in &all_common {
            if let Some(icon) = &c.icon {
                assets.push((c, icon));
            }
        }
        for (c, asset) in assets {
            if asset.starts_with("content:") {
                continue;
            }
            let found = LogicalPath::new(asset)
                .ok()
                .is_some_and(|p| set.vfs.resolve(&p).is_some());
            if !found {
                errors.push(format!("{}: asset {asset} not found", at(c)));
            }
        }
        // Magick combinations: queue-reachable, unique.
        let mut combos: BTreeMap<Vec<u32>, &DefinitionId> = BTreeMap::new();
        let native: BTreeMap<u8, Vec<Elements>> = builtin_magicks()
            .into_iter()
            .map(|b| (b.legacy.unwrap(), b.combination))
            .collect();
        for m in self.magicks.iter() {
            // Original tables are data (some native combos, e.g.
            // Amalgameddon, are not typeable); only new ones are checked.
            let original = m
                .legacy
                .is_some_and(|l| native.get(&l) == Some(&m.combination));
            if !original {
                if let Err(e) = check_combination(&m.combination) {
                    errors.push(format!("{}: {e}", at(&m.common)));
                }
            }
            let key: Vec<u32> = m.combination.iter().map(|e| e.0).collect();
            if let Some(other) = combos.insert(key, &m.common.id) {
                errors.push(format!(
                    "{}: combination {} is already {other}",
                    at(&m.common),
                    m.combination
                        .iter()
                        .map(|e| element_name(*e))
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
        }
        if self.magicks.len() > u16::MAX as usize {
            errors.push("too many Magicks".into());
        }
        // Localization keys (non-fatal).
        let mut missing = BTreeSet::new();
        for c in &all_common {
            for key in [&c.name_key, &c.description_key].into_iter().flatten() {
                if self.localization.lookup("english", key).is_none()
                    && !c.id.as_str().starts_with("base:")
                {
                    missing.insert(format!("{}: missing localization {key}", at(c)));
                }
            }
        }
        self.warnings.extend(missing);
    }

    fn all_common(&self) -> impl Iterator<Item = &Common> {
        self.items
            .iter()
            .map(|d| &d.common)
            .chain(self.abilities.iter().map(|d| &d.common))
            .chain(self.modifiers.iter().map(|d| &d.common))
            .chain(self.magicks.iter().map(|d| &d.common))
            .chain(self.spells.iter().map(|d| &d.common))
            .chain(self.units.iter().map(|d| &d.common))
            .chain(self.effects.iter().map(|d| &d.common))
            .chain(self.levels.iter().map(|d| &d.common))
    }

    /// Canonical hash of authoritative definition values (presentation-only
    /// effects and localization excluded), stable across platforms.
    pub fn rules_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(format!("magicka-definitions/{SCHEMA_VERSION}\n"));
        let mut feed = |name: &str, value: serde_json::Value| {
            h.update(name);
            h.update(value.to_string());
            h.update("\n");
        };
        for d in self.items.iter() {
            feed("item", serde_json::to_value(d).unwrap());
        }
        for d in self.abilities.iter() {
            feed("ability", serde_json::to_value(d).unwrap());
        }
        for d in self.modifiers.iter() {
            feed("modifier", serde_json::to_value(d).unwrap());
        }
        for d in self.magicks.iter() {
            feed("magick", serde_json::to_value(d).unwrap());
        }
        for d in self.spells.iter() {
            feed("spell", serde_json::to_value(d).unwrap());
        }
        for d in self.units.iter() {
            feed("unit", serde_json::to_value(d).unwrap());
        }
        for d in self.levels.iter() {
            feed("level", serde_json::to_value(d).unwrap());
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The item definition overriding retail item `name`, if any.
    pub fn retail_item(&self, name: &str) -> Option<&ItemDef> {
        self.items.iter().find(|i| {
            i.retail
                .as_deref()
                .is_some_and(|r| r.eq_ignore_ascii_case(name))
        })
    }

    /// The Magick a complete element queue combines into, if registered.
    pub fn magick_for(&self, queue: &[Elements]) -> Option<&MagickDef> {
        self.magicks.iter().find(|m| m.combination == queue)
    }
}

/// Built-in Magicks, as registry entries over the native tables.
pub fn builtin_magicks() -> Vec<MagickDef> {
    MAGICK_COMBOS
        .iter()
        .map(|(m, combo)| {
            let name = format!("{m:?}").to_ascii_lowercase();
            MagickDef {
                common: Common {
                    id: DefinitionId(format!("{BASE_NAMESPACE}:{name}")),
                    name_key: None,
                    description_key: None,
                    icon: None,
                    numbers: BTreeMap::new(),
                    source: Source {
                        file: "<builtin>".into(),
                        line: 0,
                    },
                },
                combination: combo.to_vec(),
                cast: CastType::Instant,
                handler: Handler::Builtin(format!("{m:?}")),
                legacy: Some(*m as u8),
                animation: None,
                particle: None,
                sound: None,
                // Native unlocks stay in the session's legacy mask.
                unlocked_by_default: false,
            }
        })
        .collect()
}
fn builtin_id_exists(id: &DefinitionId) -> bool {
    builtin_magicks().iter().any(|b| &b.common.id == id)
}
fn builtin_value(name: &str) -> u8 {
    builtin_magicks()
        .iter()
        .find(|b| matches!(&b.handler, Handler::Builtin(n) if n.eq_ignore_ascii_case(name)))
        .and_then(|b| b.legacy)
        .unwrap_or(0)
}

/// A combination must be 1..=5 basic elements that the real queue keeps as
/// typed (no merges, cancellations or a second shield).
pub fn check_combination(combo: &[Elements]) -> Result<(), String> {
    if combo.is_empty() || combo.len() > SPELL_QUEUE_CAP {
        return Err(format!("a combination has 1..={SPELL_QUEUE_CAP} elements"));
    }
    let mut queue = Vec::new();
    let mut events = Vec::new();
    for e in combo {
        magicka_sim::spells::try_add_to_queue(
            &mut queue,
            SPELL_QUEUE_CAP,
            Spell::default_for(*e),
            &mut events,
        );
    }
    let kept: Vec<Elements> = queue.iter().map(|s| s.element).collect();
    if kept != combo {
        return Err(format!(
            "combination {} cannot be typed: the element queue becomes {}",
            combo
                .iter()
                .map(|e| element_name(*e))
                .collect::<Vec<_>>()
                .join(","),
            kept.iter()
                .map(|e| element_name(*e))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    Ok(())
}
