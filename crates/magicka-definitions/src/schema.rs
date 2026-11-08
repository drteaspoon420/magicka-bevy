//! Numeric field schemas and the context-dependent stat resolver (plan 7.2).
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumberKind {
    Int,
    Float,
}

/// How layered contributions to one value combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aggregation {
    /// base + Σ additions, then × Π multipliers.
    Additive,
    /// base × Π (1 + percent/100).
    Multiplicative,
    /// The last override wins.
    Override,
}

/// Whether a change alters gameplay (must match on every peer) or only how
/// it looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Affects {
    Simulation,
    Presentation,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldSchema {
    pub key: &'static str,
    pub kind: NumberKind,
    pub units: &'static str,
    pub min: f64,
    pub max: f64,
    /// Whether upgrade systems (Boosted-style mods) may change it.
    pub upgradable: bool,
    /// Negative values have a meaning (otherwise clamped at zero).
    pub signed: bool,
    pub aggregation: Aggregation,
    pub affects: Affects,
}

pub const fn float(key: &'static str, units: &'static str, min: f64, max: f64) -> FieldSchema {
    FieldSchema {
        key,
        kind: NumberKind::Float,
        units,
        min,
        max,
        upgradable: true,
        signed: false,
        aggregation: Aggregation::Additive,
        affects: Affects::Simulation,
    }
}
pub const fn int(key: &'static str, units: &'static str, min: f64, max: f64) -> FieldSchema {
    FieldSchema {
        kind: NumberKind::Int,
        ..float(key, units, min, max)
    }
}
impl FieldSchema {
    pub const fn fixed(self) -> Self {
        Self {
            upgradable: false,
            ..self
        }
    }
    pub const fn signed(self) -> Self {
        Self {
            signed: true,
            ..self
        }
    }
    pub const fn presentation(self) -> Self {
        Self {
            affects: Affects::Presentation,
            ..self
        }
    }
    pub const fn aggregation(self, aggregation: Aggregation) -> Self {
        Self {
            aggregation,
            ..self
        }
    }
    /// Parse and range-check a value written in KV.
    pub fn parse(&self, text: &str) -> Result<f64, String> {
        let v: f64 = text
            .trim()
            .parse()
            .map_err(|_| format!("{} must be a number, not {text:?}", self.key))?;
        if !v.is_finite() {
            return Err(format!("{} must be finite", self.key));
        }
        if self.kind == NumberKind::Int && v.fract() != 0.0 {
            return Err(format!("{} must be an integer, not {text}", self.key));
        }
        if v < self.min || v > self.max {
            return Err(format!(
                "{} = {v} is outside {}..={} {}",
                self.key, self.min, self.max, self.units
            ));
        }
        Ok(v)
    }
    /// Constrain a resolved value to the schema.
    pub fn clamp(&self, v: f64) -> f64 {
        let lower = if self.signed {
            self.min
        } else {
            self.min.max(0.0)
        };
        let v = v.clamp(lower, self.max);
        match self.kind {
            NumberKind::Int => v.round(),
            NumberKind::Float => v,
        }
    }
}

pub fn find<'a>(schemas: &'a [FieldSchema], key: &str) -> Option<&'a FieldSchema> {
    schemas.iter().find(|s| s.key.eq_ignore_ascii_case(key))
}

/// Where a contribution comes from, in resolution order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Layer {
    /// Instance/loadout overlays and persistent upgrades.
    Upgrade,
    /// Temporary buffs and debuffs.
    Temporary,
}

/// One change to one value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Change {
    Add(f64),
    /// Percent: +25 means ×1.25.
    Percent(f64),
    Multiply(f64),
    Set(f64),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Contribution {
    pub layer: Layer,
    pub change: Change,
    /// Owner of the contribution (mod/modifier ID), for diagnostics.
    pub source: String,
}

/// `resolve_value`: the base definition value after file patches, then
/// upgrades, then temporary effects, then the schema's stacking rule and
/// constraints. The base definition itself is never mutated.
pub fn resolve(schema: &FieldSchema, base: f64, contributions: &[Contribution]) -> f64 {
    let mut ordered: Vec<&Contribution> = contributions.iter().collect();
    ordered.sort_by_key(|c| c.layer);
    let mut value = base;
    let mut add = 0.0;
    let mut multiply = 1.0;
    let mut set = None;
    for c in ordered {
        match (c.change, schema.aggregation) {
            (Change::Set(v), _) => set = Some(v),
            (Change::Add(v), _) => add += v,
            (Change::Percent(p), Aggregation::Multiplicative) => multiply *= 1.0 + p / 100.0,
            (Change::Percent(p), _) => multiply += p / 100.0,
            (Change::Multiply(m), _) => multiply *= m,
        }
    }
    if let Some(v) = set {
        value = v;
    }
    if schema.aggregation == Aggregation::Override {
        return schema.clamp(set.unwrap_or(base));
    }
    schema.clamp((value + add) * multiply)
}
