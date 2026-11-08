#![allow(dead_code)]
use magicka_definitions::Definitions;
use magicka_mods::ModSet;
use magicka_script::{EntityInfo, EntityRef, Limits, PlayerInfo, ScriptHost, WorldSnapshot};
use std::{path::PathBuf, sync::Arc};

pub struct Tree(pub PathBuf);
impl Tree {
    pub fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "magicka-script-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("mods")).unwrap();
        Self(root)
    }
    pub fn file(&self, path: &str, text: &str) -> &Self {
        let p = self.0.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
        self
    }
    /// A mod with server scripts and an init.lua.
    pub fn script_mod(&self, id: &str, init: &str) -> &Self {
        self.file(
            &format!("mods/{id}/mod.kv"),
            &format!(
                "\"Mod\" {{ \"Id\" \"{id}\" \"Version\" \"1.0.0\" \"ApiVersion\" \"1\" \"ServerScripts\" \"1\" }}"
            ),
        )
        .file(&format!("mods/{id}/scripts/vscripts/{id}/init.lua"), init)
    }
    pub fn host(&self, enabled: &[&str], limits: Limits) -> ScriptHost {
        let enabled: Vec<String> = enabled.iter().map(|s| s.to_string()).collect();
        let set = ModSet::load(None, &self.0.join("mods"), Some(&enabled)).unwrap();
        let defs = Definitions::load(&set).unwrap_or_else(|e| panic!("{e}"));
        ScriptHost::new(Arc::new(set), Arc::new(defs), limits, 42).unwrap()
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub const HERO: EntityRef = EntityRef {
    handle: 7,
    generation: 3,
};
pub const GOBLIN: EntityRef = EntityRef {
    handle: 9,
    generation: 1,
};

pub fn snapshot(tick: u64) -> WorldSnapshot {
    WorldSnapshot {
        tick,
        time: tick as f64 / 60.0,
        level: "test.lvl".into(),
        scene: "lab".into(),
        players: vec![PlayerInfo {
            slot: 0,
            name: "Zed".into(),
            profile_id: None,
            entity: Some(HERO),
            magicks: vec![],
            items: vec![],
        }],
        entities: vec![
            EntityInfo {
                entity: Some(HERO),
                name: "Wizard".into(),
                position: [0.0, 0.0, 0.0],
                health: 500.0,
                max_health: 1000.0,
                alive: true,
                player: Some(0),
                ..Default::default()
            },
            EntityInfo {
                entity: Some(GOBLIN),
                name: "Goblin".into(),
                position: [3.0, 0.0, 4.0],
                health: 200.0,
                max_health: 200.0,
                alive: true,
                ..Default::default()
            },
        ],
        conditions: Default::default(),
        upgrades: vec![],
        indoors: false,
    }
}
