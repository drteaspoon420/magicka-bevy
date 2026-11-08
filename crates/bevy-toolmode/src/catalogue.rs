//! What can be spawned: entries grouped by a category path (the first
//! segment is a menu tab, the rest a tree), an optional glTF model for the
//! placement ghost and thumbnail, and editable spawn properties.
use bevy::prelude::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Bool(bool),
    Float(f32),
    Int(i64),
    /// Free text, or one of [`PropertyKind::Choice`].
    Text(String),
}

impl PropertyValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Self::Float(v) => Some(*v),
            Self::Int(v) => Some(*v as f32),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PropertyKind {
    Bool,
    Float { min: f32, max: f32 },
    Int { min: i64, max: i64 },
    Text,
    Choice(Vec<String>),
}

/// An editable spawn parameter ("order", "health multiplier", ...).
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyDef {
    pub key: String,
    pub label: String,
    pub kind: PropertyKind,
    pub default: PropertyValue,
}

pub type PropertyValues = BTreeMap<String, PropertyValue>;

#[derive(Clone, Debug, Default)]
pub struct Spawnable {
    /// Stable id (comes back in [`crate::SpawnRequest::spawnable`]).
    pub id: String,
    pub label: String,
    /// Category path: `["Characters", "Goblins"]` puts the entry in the
    /// "Characters" tab under a "Goblins" node.
    pub category: Vec<String>,
    /// glTF asset path (scene 0 is used) for the ghost and thumbnail.
    pub model: Option<String>,
    pub description: String,
    pub properties: Vec<PropertyDef>,
}

impl Spawnable {
    pub fn tab(&self) -> &str {
        self.category.first().map_or("Other", String::as_str)
    }

    /// Defaults overlaid with `overrides` for keys this entry defines.
    pub fn resolve(&self, overrides: &PropertyValues) -> PropertyValues {
        self.properties
            .iter()
            .map(|p| {
                let value = overrides
                    .get(&p.key)
                    .filter(|v| std::mem::discriminant(*v) == std::mem::discriminant(&p.default))
                    .cloned()
                    .unwrap_or_else(|| p.default.clone());
                (p.key.clone(), value)
            })
            .collect()
    }
}

#[derive(Resource, Default)]
pub struct SpawnCatalogue {
    pub entries: Vec<Spawnable>,
    /// Bump after changing `entries` so thumbnails and menus refresh.
    pub revision: u64,
}

impl SpawnCatalogue {
    pub fn get(&self, id: &str) -> Option<&Spawnable> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn replace(&mut self, entries: Vec<Spawnable>) {
        self.entries = entries;
        self.revision += 1;
    }

    /// Tabs in first-appearance order.
    pub fn tabs(&self) -> Vec<String> {
        let mut tabs: Vec<String> = Vec::new();
        for e in &self.entries {
            if !tabs.iter().any(|t| t == e.tab()) {
                tabs.push(e.tab().to_string());
            }
        }
        tabs
    }

    /// The category tree below `tab`.
    pub fn tree(&self, tab: &str) -> CategoryNode {
        let mut root = CategoryNode::default();
        for e in self.entries.iter().filter(|e| e.tab() == tab) {
            let mut node = &mut root;
            for part in e.category.iter().skip(1) {
                let at = match node.children.iter().position(|c| c.name == *part) {
                    Some(at) => at,
                    None => {
                        node.children.push(CategoryNode {
                            name: part.clone(),
                            ..default()
                        });
                        node.children.len() - 1
                    }
                };
                node = &mut node.children[at];
            }
            node.count += 1;
        }
        root.total();
        root
    }

    /// Entries in `tab` under the category `path` (below the tab) whose id,
    /// label or category contains every word of `search`.
    pub fn filter<'a>(&'a self, tab: &str, path: &[String], search: &str) -> Vec<&'a Spawnable> {
        let words: Vec<String> = search
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect();
        self.entries
            .iter()
            .filter(|e| e.tab() == tab)
            .filter(|e| e.category.iter().skip(1).take(path.len()).eq(path.iter()))
            .filter(|e| {
                let text =
                    format!("{} {} {}", e.id, e.label, e.category.join(" ")).to_ascii_lowercase();
                words.iter().all(|w| text.contains(w.as_str()))
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CategoryNode {
    pub name: String,
    /// Entries directly in this node.
    pub count: usize,
    /// Entries in this node and below.
    pub total: usize,
    pub children: Vec<CategoryNode>,
}

impl CategoryNode {
    fn total(&mut self) -> usize {
        self.total = self.count + self.children.iter_mut().map(|c| c.total()).sum::<usize>();
        self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, category: &[&str]) -> Spawnable {
        Spawnable {
            id: id.into(),
            label: id.into(),
            category: category.iter().map(|c| c.to_string()).collect(),
            ..default()
        }
    }

    fn catalogue() -> SpawnCatalogue {
        let mut c = SpawnCatalogue::default();
        c.replace(vec![
            entry("goblin", &["Characters", "Goblins"]),
            entry("goblin_shaman", &["Characters", "Goblins"]),
            entry("troll", &["Characters", "Trolls"]),
            entry("barrel", &["Props"]),
        ]);
        c
    }

    #[test]
    fn tabs_keep_first_appearance_order() {
        assert_eq!(catalogue().tabs(), ["Characters", "Props"]);
    }

    #[test]
    fn tree_counts_entries_per_node_and_below() {
        let tree = catalogue().tree("Characters");
        assert_eq!(tree.total, 3);
        assert_eq!(tree.children[0].name, "Goblins");
        assert_eq!(tree.children[0].count, 2);
        assert_eq!(tree.children[1].total, 1);
    }

    #[test]
    fn filter_matches_path_prefix_and_all_search_words() {
        let c = catalogue();
        let goblins = ["Goblins".to_string()];
        assert_eq!(c.filter("Characters", &goblins, "").len(), 2);
        assert_eq!(c.filter("Characters", &[], "").len(), 3);
        let found: Vec<_> = c
            .filter("Characters", &[], "gob sham")
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(found, ["goblin_shaman"]);
    }

    #[test]
    fn resolve_ignores_overrides_of_the_wrong_type() {
        let mut e = entry("goblin", &["Characters"]);
        e.properties.push(PropertyDef {
            key: "order".into(),
            label: "Order".into(),
            kind: PropertyKind::Choice(vec!["attack".into(), "idle".into()]),
            default: PropertyValue::Text("attack".into()),
        });
        let mut overrides = PropertyValues::new();
        overrides.insert("order".into(), PropertyValue::Bool(true));
        overrides.insert("unrelated".into(), PropertyValue::Int(1));
        let resolved = e.resolve(&overrides);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved["order"], PropertyValue::Text("attack".into()));
        overrides.insert("order".into(), PropertyValue::Text("idle".into()));
        assert_eq!(e.resolve(&overrides)["order"].as_str(), Some("idle"));
    }
}
