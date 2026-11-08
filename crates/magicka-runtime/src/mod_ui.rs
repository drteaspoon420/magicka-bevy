//! Declarative mod UI (plan H5): panels from `ui/*.kv` rendered from the
//! replicated mod net tables. Clients display the host's state and choose
//! from it; they never author offers or balances.
use magicka_definitions::{Localization, UiPanel};
use magicka_network::session::ModUiEntry;
use serde_json::Value as Json;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelView {
    pub title: String,
    pub lines: Vec<String>,
    /// (text, the list element sent with a choice).
    pub choices: Vec<(String, Json)>,
    pub action: Option<String>,
}

fn value(entries: &[ModUiEntry], table: &str, key: &str) -> Option<Json> {
    entries
        .iter()
        .find(|e| e.table == table && e.key == key)
        .and_then(|e| serde_json::from_str(&e.json).ok())
}
fn text(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        Json::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            Some(f) => format!("{f:.2}"),
            None => n.to_string(),
        },
        Json::Bool(b) => b.to_string(),
        Json::Null => String::new(),
        other => other.to_string(),
    }
}
/// Replace `{...}` placeholders using `lookup`; `#key` localizes.
fn fill(template: &str, loc: &Localization, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &rest[start + 1..start + end];
        out.push_str(&lookup(name).unwrap_or_default());
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    if out.starts_with('#') {
        if let Some(t) = loc.lookup("english", &out) {
            return t.to_string();
        }
    }
    out
}

pub fn render(
    panel: &UiPanel,
    entries: &[ModUiEntry],
    slot: usize,
    loc: &Localization,
) -> PanelView {
    let table_value = |name: &str| -> Option<String> {
        let (table, key) = name.split_once(':')?;
        value(entries, table, &key.replace("$slot", &slot.to_string())).map(|v| text(&v))
    };
    let mut view = PanelView {
        title: fill(&panel.title, loc, &table_value),
        lines: panel
            .lines
            .iter()
            .map(|l| fill(l, loc, &table_value))
            .collect(),
        ..Default::default()
    };
    if let Some(list) = &panel.list {
        view.action = Some(list.action.clone());
        let source = list.source.replace("$slot", &slot.to_string());
        if let Some((table, key)) = source.split_once(':') {
            if let Some(Json::Array(items)) = value(entries, table, key) {
                for item in items.into_iter().take(9) {
                    let field = |name: &str| -> Option<String> {
                        if name.contains(':') {
                            return table_value(name);
                        }
                        item.get(name).map(text)
                    };
                    view.choices
                        .push((fill(&list.item, loc, &field), item.clone()));
                }
            }
        }
    }
    view
}

/// Net tables as replicated entries (for the host's own screen).
pub fn entries(
    tables: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, Json>>,
) -> Vec<ModUiEntry> {
    tables
        .iter()
        .flat_map(|(t, m)| {
            m.iter().map(move |(k, v)| ModUiEntry {
                table: t.clone(),
                key: k.clone(),
                json: v.to_string(),
            })
        })
        .collect()
}
