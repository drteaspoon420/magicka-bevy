//! Typed definitions compiled from KV blocks. Game logic sees these structs,
//! never raw KV maps.
use crate::schema::{self, float, int, FieldSchema};
use crate::{DefinitionId, Source};
use magicka_mods::kv::KvNode;
use magicka_sim::spells::Elements;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Numeric fields an item definition may set. Names follow the retail item
/// template fields they override (`magicka_content::item::Item`).
pub const ITEM_FIELDS: &[FieldSchema] = &[
    int("BlockValue", "points", -1000.0, 1000.0).signed(),
    float("CooldownTime", "s", 0.0, 600.0),
    float("PassiveValue", "", -1000.0, 1000.0).signed(),
    float("MeleeRange", "m", 0.0, 100.0),
    float("RangedRange", "m", 0.0, 500.0),
    float("RangedElevation", "deg", -90.0, 90.0).signed(),
    float("Homing", "", 0.0, 100.0),
    float("GunRange", "m", 0.0, 500.0),
    int("GunClip", "rounds", 0.0, 10_000.0),
    int("GunRate", "rounds/s", 0.0, 1000.0),
    float("GunAccuracy", "", 0.0, 1.0),
    float("TracerVelocity", "m/s", 0.0, 10_000.0).presentation(),
    float("Scale", "", 0.01, 100.0).presentation().fixed(),
];
pub const ABILITY_FIELDS: &[FieldSchema] = &[
    float("Cooldown", "s", 0.0, 3600.0),
    float("CastRange", "m", 0.0, 1000.0),
    float("Radius", "m", 0.0, 100.0),
    float("Duration", "s", 0.0, 3600.0),
    float("Damage", "hp", -100_000.0, 100_000.0).signed(),
];
pub const MODIFIER_FIELDS: &[FieldSchema] = &[
    float("Duration", "s", 0.0, 86_400.0).fixed(),
    int("MaxStacks", "", 1.0, 1000.0).fixed(),
    float("TickInterval", "s", 0.0, 60.0).fixed(),
];
/// Stat properties a modifier can change on its holder.
pub const MODIFIER_PROPERTIES: &[FieldSchema] = &[
    float("MoveSpeedPercent", "%", -100.0, 1000.0).signed(),
    float("DamagePercent", "%", -100.0, 1000.0).signed(),
    float("DamageTakenPercent", "%", -100.0, 1000.0).signed(),
    float("HealthRegen", "hp/s", -10_000.0, 10_000.0).signed(),
    float("MaxHealthBonus", "hp", -100_000.0, 100_000.0).signed(),
    float("CooldownPercent", "%", -100.0, 1000.0).signed(),
];
pub const MAGICK_FIELDS: &[FieldSchema] = &[float("Cooldown", "s", 0.0, 3600.0)];
pub const SPELL_FIELDS: &[FieldSchema] = &[
    float("DamageMultiplier", "×", 0.0, 100.0),
    float("Range", "m", 0.0, 1000.0),
    float("Radius", "m", 0.0, 100.0),
];
pub const UNIT_FIELDS: &[FieldSchema] = &[
    float("Health", "hp", 1.0, 1_000_000.0),
    float("MoveSpeed", "m/s", 0.0, 100.0),
    float("Radius", "m", 0.05, 50.0).fixed(),
];
pub const EFFECT_FIELDS: &[FieldSchema] = &[
    float("Duration", "s", 0.0, 3600.0).presentation(),
    float("Scale", "", 0.0, 100.0).presentation(),
];

/// Common identity and presentation data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Common {
    pub id: DefinitionId,
    pub name_key: Option<String>,
    pub description_key: Option<String>,
    pub icon: Option<String>,
    /// Validated numbers by schema key.
    pub numbers: BTreeMap<String, f64>,
    #[serde(skip)]
    pub source: Source,
}

/// A handler: built-in Rust behaviour or a server-side Lua function.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Handler {
    None,
    Builtin(String),
    Lua { file: String, function: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemSlot {
    Weapon,
    Staff,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemDef {
    pub common: Common,
    /// The retail item template this definition overrides
    /// (`Data/Items/Wizard/<name>`), if any.
    pub retail: Option<String>,
    pub slot: Option<ItemSlot>,
    pub model: Option<String>,
    pub sounds: Vec<String>,
    pub active_ability: Option<DefinitionId>,
    pub modifiers: Vec<DefinitionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AbilityBehavior {
    Passive,
    Active,
    Toggle,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetType {
    None,
    Point,
    Unit,
    Direction,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AbilityDef {
    pub common: Common,
    pub behavior: AbilityBehavior,
    pub target: TargetType,
    pub handler: Handler,
    /// Per-level special values (`"Values" { "damage" "10 20 30" }`).
    pub values: BTreeMap<String, Vec<f64>>,
    pub modifiers: Vec<DefinitionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stacking {
    /// A second application refreshes the duration.
    Refresh,
    /// Stacks up to MaxStacks; each application refreshes.
    Stack,
    /// Independent instances.
    Independent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModifierDef {
    pub common: Common,
    pub stacking: Stacking,
    pub debuff: bool,
    pub hidden: bool,
    pub properties: BTreeMap<String, f64>,
    pub handler: Handler,
    pub effect: Option<DefinitionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CastType {
    Instant,
    Channel,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MagickDef {
    pub common: Common,
    /// Element queue that combines into this Magick.
    pub combination: Vec<Elements>,
    pub cast: CastType,
    pub handler: Handler,
    /// Legacy `MagickType` value of a built-in Magick (saves, network).
    pub legacy: Option<u8>,
    pub animation: Option<String>,
    pub particle: Option<String>,
    pub sound: Option<String>,
    pub unlocked_by_default: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Delivery {
    Projectile,
    Spray,
    Beam,
    Area,
    SelfCast,
    Weapon,
}
impl Delivery {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "projectile" | "force" => Self::Projectile,
            "spray" => Self::Spray,
            "beam" | "lightning" => Self::Beam,
            "area" | "blast" => Self::Area,
            "self" => Self::SelfCast,
            "weapon" | "imbue" => Self::Weapon,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpellDef {
    pub common: Common,
    pub elements: Vec<Elements>,
    pub delivery: Delivery,
    pub effects: Vec<DefinitionId>,
    pub handler: Handler,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitDef {
    pub common: Common,
    /// Retail character template name.
    pub template: Option<String>,
    pub model: Option<String>,
    pub abilities: Vec<DefinitionId>,
    pub equipment: BTreeMap<String, DefinitionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectKind {
    Emitter,
    Sound,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lifecycle {
    OneShot,
    Loop,
}

/// Non-authoritative presentation: clients simulate these locally.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectDef {
    pub common: Common,
    pub kind: EffectKind,
    pub asset: String,
    pub attachment: Option<String>,
    pub lifecycle: Lifecycle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LevelDef {
    pub common: Common,
    /// Retail or mod level file (`*.lvl`).
    pub level: String,
    pub scenes: Vec<String>,
    pub script: Handler,
    pub objectives: Vec<String>,
    pub assets: Vec<String>,
}

/// A declarative mod UI panel (presentation; `ui/*.kv`, root `Panels`).
/// Templates bind replicated net tables: `{table:key}` (in keys, `$slot`
/// is the viewer's player slot) and, in list items, `{field}` of each
/// list element. Choosing an item sends a mod request on `action`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiPanel {
    pub id: DefinitionId,
    pub title: String,
    pub lines: Vec<String>,
    pub list: Option<UiList>,
    /// Shown only to players (not spectators); toggled with the panel key.
    pub players_only: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiList {
    /// `table:key` of a JSON array.
    pub source: String,
    pub item: String,
    pub action: String,
}

// ---------------------------------------------------------------------------
// Parsing

pub(crate) struct Ctx<'a> {
    pub file: &'a str,
    pub namespace: &'a str,
    pub errors: &'a mut Vec<String>,
}
impl Ctx<'_> {
    pub fn error(&mut self, node: &KvNode, message: impl std::fmt::Display) {
        self.errors
            .push(format!("{}:{}: {message}", self.file, node.span));
    }
    pub fn id(&mut self, node: &KvNode, raw: &str) -> Option<DefinitionId> {
        match DefinitionId::parse(raw, self.namespace) {
            Ok(id) => Some(id),
            Err(e) => {
                self.error(node, e);
                None
            }
        }
    }
}

const COMMON_KEYS: &[&str] = &["NameKey", "DescriptionKey", "Icon"];

pub fn element(name: &str) -> Option<Elements> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "earth" => Elements::EARTH,
        "water" => Elements::WATER,
        "cold" => Elements::COLD,
        "fire" => Elements::FIRE,
        "lightning" => Elements::LIGHTNING,
        "arcane" => Elements::ARCANE,
        "life" => Elements::LIFE,
        "shield" => Elements::SHIELD,
        "ice" => Elements::ICE,
        "steam" => Elements::STEAM,
        "poison" => Elements::POISON,
        _ => return None,
    })
}
/// The element names KV and Lua accept, in `ElementIndex` order.
pub fn element_names() -> impl Iterator<Item = &'static str> {
    Elements::BY_INDEX.iter().map(|e| element_name(*e))
}
pub fn element_name(e: Elements) -> &'static str {
    const NAMES: [&str; 11] = [
        "earth",
        "water",
        "cold",
        "fire",
        "lightning",
        "arcane",
        "life",
        "shield",
        "ice",
        "steam",
        "poison",
    ];
    Elements::BY_INDEX
        .iter()
        .position(|x| *x == e)
        .map_or("?", |i| NAMES[i])
}

/// Read a block's fields, reporting unknown keys. `extra` lists the
/// type-specific non-numeric keys.
pub(crate) fn common(
    ctx: &mut Ctx,
    node: &KvNode,
    id: DefinitionId,
    numbers: &[FieldSchema],
    extra: &[&str],
) -> Common {
    let mut out = Common {
        id,
        name_key: node.get_str("NameKey").map(str::to_string),
        description_key: node.get_str("DescriptionKey").map(str::to_string),
        icon: node.get_str("Icon").map(str::to_string),
        numbers: BTreeMap::new(),
        source: Source {
            file: ctx.file.into(),
            line: node.span.line,
        },
    };
    let mut seen = std::collections::BTreeSet::new();
    for child in node.children() {
        let key = child.key.as_str();
        if !seen.insert(key.to_ascii_lowercase()) {
            ctx.error(child, format!("duplicate field {key}"));
            continue;
        }
        if let Some(s) = schema::find(numbers, key) {
            match child.str().map(|t| s.parse(t)) {
                Some(Ok(v)) => {
                    out.numbers.insert(s.key.to_string(), v);
                }
                Some(Err(e)) => ctx.error(child, e),
                None => ctx.error(child, format!("{key} must be a value")),
            }
        } else if !COMMON_KEYS
            .iter()
            .chain(extra)
            .any(|k| k.eq_ignore_ascii_case(key))
        {
            ctx.error(child, format!("unknown field {key}"));
        }
    }
    out
}

pub(crate) fn handler(ctx: &mut Ctx, node: &KvNode) -> Handler {
    let script = node.get_str("ScriptFile");
    let function = node.get_str("Handler");
    let builtin = node.get_str("Builtin");
    match (script, function, builtin) {
        (None, None, None) => Handler::None,
        (None, None, Some(b)) => Handler::Builtin(b.to_string()),
        (Some(file), function, None) => Handler::Lua {
            file: file.to_string(),
            function: function.unwrap_or("OnSpellStart").to_string(),
        },
        _ => {
            ctx.error(node, "use either Builtin or ScriptFile/Handler");
            Handler::None
        }
    }
}

pub(crate) fn id_list(ctx: &mut Ctx, node: &KvNode, key: &str) -> Vec<DefinitionId> {
    match node.get(key) {
        None => vec![],
        Some(n) => match n.str() {
            Some(text) => text
                .split([',', ' '])
                .filter(|s| !s.is_empty())
                .filter_map(|s| ctx.id(n, s))
                .collect(),
            None => n
                .children()
                .iter()
                .filter_map(|c| ctx.id(c, &c.key.clone()))
                .collect(),
        },
    }
}
fn flag(ctx: &mut Ctx, node: &KvNode, key: &str) -> bool {
    match node.get_str(key) {
        None | Some("0") => false,
        Some("1") => true,
        Some(v) => {
            ctx.error(node, format!("{key} must be 0 or 1, not {v:?}"));
            false
        }
    }
}
fn choice<T: Copy>(
    ctx: &mut Ctx,
    node: &KvNode,
    key: &str,
    default: T,
    options: &[(&str, T)],
) -> T {
    match node.get_str(key) {
        None => default,
        Some(v) => match options.iter().find(|(n, _)| n.eq_ignore_ascii_case(v)) {
            Some((_, t)) => *t,
            None => {
                let names: Vec<_> = options.iter().map(|(n, _)| *n).collect();
                ctx.error(node, format!("{key} must be one of {names:?}, not {v:?}"));
                default
            }
        },
    }
}
pub(crate) fn elements(ctx: &mut Ctx, node: &KvNode, key: &str) -> Vec<Elements> {
    let Some(text) = node.get_str(key) else {
        return vec![];
    };
    text.split([',', ' '])
        .filter(|s| !s.is_empty())
        .filter_map(|name| {
            let e = element(name);
            if e.is_none() {
                ctx.error(node, format!("unknown element {name:?}"));
            }
            e
        })
        .collect()
}

pub(crate) fn item(ctx: &mut Ctx, node: &KvNode, id: DefinitionId) -> ItemDef {
    let common = common(
        ctx,
        node,
        id,
        ITEM_FIELDS,
        &["Retail", "Slot", "Model", "Sounds", "Ability", "Modifiers"],
    );
    let slot = node
        .get_str("Slot")
        .and_then(|s| match s.to_ascii_lowercase().as_str() {
            "weapon" => Some(ItemSlot::Weapon),
            "staff" => Some(ItemSlot::Staff),
            _ => {
                ctx.error(node, format!("Slot must be weapon or staff, not {s:?}"));
                None
            }
        });
    let active_ability = node.get_str("Ability").and_then(|a| ctx.id(node, a));
    ItemDef {
        retail: node.get_str("Retail").map(str::to_string),
        slot,
        model: node.get_str("Model").map(str::to_string),
        sounds: node
            .get_str("Sounds")
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        active_ability,
        modifiers: id_list(ctx, node, "Modifiers"),
        common,
    }
}

pub(crate) fn ability(ctx: &mut Ctx, node: &KvNode, id: DefinitionId) -> AbilityDef {
    let common = common(
        ctx,
        node,
        id,
        ABILITY_FIELDS,
        &[
            "Behavior",
            "Target",
            "ScriptFile",
            "Handler",
            "Builtin",
            "Values",
            "Modifiers",
        ],
    );
    let mut values = BTreeMap::new();
    if let Some(block) = node.get("Values") {
        for v in block.children() {
            let parsed: Result<Vec<f64>, _> = v
                .str()
                .unwrap_or("")
                .split_whitespace()
                .map(str::parse::<f64>)
                .collect();
            match parsed {
                Ok(list) if !list.is_empty() && list.iter().all(|x| x.is_finite()) => {
                    values.insert(v.key.to_ascii_lowercase(), list);
                }
                _ => ctx.error(v, format!("value {} must be a list of numbers", v.key)),
            }
        }
    }
    AbilityDef {
        behavior: choice(
            ctx,
            node,
            "Behavior",
            AbilityBehavior::Active,
            &[
                ("passive", AbilityBehavior::Passive),
                ("active", AbilityBehavior::Active),
                ("toggle", AbilityBehavior::Toggle),
            ],
        ),
        target: choice(
            ctx,
            node,
            "Target",
            TargetType::None,
            &[
                ("none", TargetType::None),
                ("point", TargetType::Point),
                ("unit", TargetType::Unit),
                ("direction", TargetType::Direction),
            ],
        ),
        handler: handler(ctx, node),
        values,
        modifiers: id_list(ctx, node, "Modifiers"),
        common,
    }
}

pub(crate) fn modifier(ctx: &mut Ctx, node: &KvNode, id: DefinitionId) -> ModifierDef {
    let common = common(
        ctx,
        node,
        id,
        MODIFIER_FIELDS,
        &[
            "Stacking",
            "Debuff",
            "Hidden",
            "Properties",
            "ScriptFile",
            "Handler",
            "Builtin",
            "Effect",
        ],
    );
    let mut properties = BTreeMap::new();
    if let Some(block) = node.get("Properties") {
        for p in block.children() {
            match schema::find(MODIFIER_PROPERTIES, &p.key) {
                Some(s) => match p.str().map(|t| s.parse(t)) {
                    Some(Ok(v)) => {
                        properties.insert(s.key.to_string(), v);
                    }
                    Some(Err(e)) => ctx.error(p, e),
                    None => ctx.error(p, "property must be a value"),
                },
                None => ctx.error(p, format!("unknown modifier property {}", p.key)),
            }
        }
    }
    ModifierDef {
        stacking: choice(
            ctx,
            node,
            "Stacking",
            Stacking::Refresh,
            &[
                ("refresh", Stacking::Refresh),
                ("stack", Stacking::Stack),
                ("independent", Stacking::Independent),
            ],
        ),
        debuff: flag(ctx, node, "Debuff"),
        hidden: flag(ctx, node, "Hidden"),
        properties,
        handler: handler(ctx, node),
        effect: node.get_str("Effect").and_then(|e| ctx.id(node, e)),
        common,
    }
}

pub(crate) fn magick(ctx: &mut Ctx, node: &KvNode, id: DefinitionId) -> MagickDef {
    let common = common(
        ctx,
        node,
        id,
        MAGICK_FIELDS,
        &[
            "Combination",
            "CastType",
            "ScriptFile",
            "Handler",
            "Builtin",
            "Animation",
            "Particle",
            "Sound",
            "UnlockedByDefault",
        ],
    );
    let combination = elements(ctx, node, "Combination");
    if combination.is_empty() {
        ctx.error(node, "a Magick needs a Combination");
    }
    MagickDef {
        combination,
        cast: choice(
            ctx,
            node,
            "CastType",
            CastType::Instant,
            &[
                ("instant", CastType::Instant),
                ("channel", CastType::Channel),
            ],
        ),
        handler: handler(ctx, node),
        legacy: None,
        animation: node.get_str("Animation").map(str::to_string),
        particle: node.get_str("Particle").map(str::to_string),
        sound: node.get_str("Sound").map(str::to_string),
        unlocked_by_default: flag(ctx, node, "UnlockedByDefault"),
        common,
    }
}

pub(crate) fn spell(ctx: &mut Ctx, node: &KvNode, id: DefinitionId) -> SpellDef {
    let common = common(
        ctx,
        node,
        id,
        SPELL_FIELDS,
        &[
            "Elements",
            "Delivery",
            "Effects",
            "ScriptFile",
            "Handler",
            "Builtin",
        ],
    );
    let delivery = match node.get_str("Delivery") {
        Some(d) => Delivery::parse(d).unwrap_or_else(|| {
            ctx.error(node, format!("unknown Delivery {d:?}"));
            Delivery::Projectile
        }),
        None => {
            ctx.error(node, "a spell needs a Delivery");
            Delivery::Projectile
        }
    };
    SpellDef {
        elements: elements(ctx, node, "Elements"),
        delivery,
        effects: id_list(ctx, node, "Effects"),
        handler: handler(ctx, node),
        common,
    }
}

pub(crate) fn unit(ctx: &mut Ctx, node: &KvNode, id: DefinitionId) -> UnitDef {
    let common = common(
        ctx,
        node,
        id,
        UNIT_FIELDS,
        &["Template", "Model", "Abilities", "Equipment"],
    );
    let mut equipment = BTreeMap::new();
    if let Some(block) = node.get("Equipment") {
        for e in block.children() {
            if let Some(item) = e.str().and_then(|v| ctx.id(e, v)) {
                equipment.insert(e.key.to_ascii_lowercase(), item);
            }
        }
    }
    UnitDef {
        template: node.get_str("Template").map(str::to_string),
        model: node.get_str("Model").map(str::to_string),
        abilities: id_list(ctx, node, "Abilities"),
        equipment,
        common,
    }
}

pub(crate) fn effect(ctx: &mut Ctx, node: &KvNode, id: DefinitionId) -> EffectDef {
    let common = common(
        ctx,
        node,
        id,
        EFFECT_FIELDS,
        &["Kind", "Asset", "Attachment", "Lifecycle"],
    );
    let asset = node.get_str("Asset").unwrap_or_default().to_string();
    if asset.is_empty() {
        ctx.error(node, "an effect needs an Asset");
    }
    EffectDef {
        kind: choice(
            ctx,
            node,
            "Kind",
            EffectKind::Emitter,
            &[
                ("emitter", EffectKind::Emitter),
                ("sound", EffectKind::Sound),
            ],
        ),
        asset,
        attachment: node.get_str("Attachment").map(str::to_string),
        lifecycle: choice(
            ctx,
            node,
            "Lifecycle",
            Lifecycle::OneShot,
            &[("oneshot", Lifecycle::OneShot), ("loop", Lifecycle::Loop)],
        ),
        common,
    }
}

pub(crate) fn level(ctx: &mut Ctx, node: &KvNode, id: DefinitionId) -> LevelDef {
    let common = common(
        ctx,
        node,
        id,
        &[],
        &[
            "Level",
            "Scenes",
            "ScriptFile",
            "Handler",
            "Builtin",
            "Objectives",
            "Assets",
        ],
    );
    let list = |key: &str| -> Vec<String> {
        node.get_str(key)
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    let level = node.get_str("Level").unwrap_or_default().to_string();
    if level.is_empty() {
        ctx.error(node, "a level needs a Level file");
    }
    LevelDef {
        level,
        scenes: list("Scenes"),
        script: handler(ctx, node),
        objectives: list("Objectives"),
        assets: list("Assets"),
        common,
    }
}
