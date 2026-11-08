//! Local rewrite saves. This JSON format is independent of retail save files.
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const VERSION: u64 = 1;
pub const SLOT_COUNT: u8 = 8;
pub fn new_player_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("player identity: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn valid_player_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Clone, Debug)]
pub struct SaveStore {
    pub root: PathBuf,
}
impl SaveStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn slot_path(&self, slot: u8) -> Result<PathBuf, String> {
        if slot >= SLOT_COUNT {
            return Err("save slot outside 0..8".into());
        }
        Ok(self.root.join(format!("campaign-{slot}.json")))
    }
    pub fn load_slot(&self, slot: u8) -> Result<Option<Value>, String> {
        read_json(&self.slot_path(slot)?)
    }
    pub fn save_slot(&self, slot: u8, state: &Value) -> Result<(), String> {
        if state["version"] != VERSION {
            return Err("unsupported campaign save version".into());
        }
        atomic_json(&self.slot_path(slot)?, state)
    }
    /// SubMenuCampaignSelect_SaveSlotSelect.DeleteCallback: the slot becomes
    /// a new campaign. Deleting an empty slot is not an error.
    pub fn delete_slot(&self, slot: u8) -> Result<(), String> {
        match fs::remove_file(self.slot_path(slot)?) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
    pub fn settings(&self) -> Result<Value, String> {
        let state = read_json(&self.root.join("settings.json"))?.unwrap_or_else(
            || json!({"version":VERSION,"music":0.8,"sfx":1.0,"history":[],"profiles":[]}),
        );
        validate_settings(&state)?;
        Ok(state)
    }
    pub fn save_settings(&self, state: &Value) -> Result<(), String> {
        validate_settings(state)?;
        atomic_json(&self.root.join("settings.json"), state)
    }
}
pub fn read_json(path: &Path) -> Result<Option<Value>, String> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if file.metadata().map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
        return Err("save exceeds 16 MiB".into());
    }
    let v: Value = serde_json::from_reader(file).map_err(|e| format!("{}: {e}", path.display()))?;
    if v["version"] != VERSION {
        return Err("unsupported save version".into());
    }
    Ok(Some(v))
}
pub fn atomic_json(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let parent = path.parent().ok_or("save path needs a parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(".save-{}.tmp", new_player_id()?));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e: std::io::Error| format!("{}: {e}", path.display()))
}

fn validate_settings(state: &Value) -> Result<(), String> {
    if state["version"] != VERSION {
        return Err("unsupported settings version".into());
    }
    for key in ["music", "sfx"] {
        if !state[key]
            .as_f64()
            .is_some_and(|v| v.is_finite() && (0.0..=1.0).contains(&v))
        {
            return Err(format!("invalid {key} volume"));
        }
    }
    if !state["history"]
        .as_array()
        .is_some_and(|v| v.iter().all(Value::is_string))
    {
        return Err("invalid connection history".into());
    }
    let profiles = state["profiles"]
        .as_array()
        .ok_or("invalid player profiles")?;
    let mut ids = std::collections::BTreeSet::new();
    for p in profiles {
        let id = p["id"]
            .as_str()
            .filter(|id| valid_player_id(id))
            .ok_or("invalid profile identity")?;
        if !ids.insert(id) {
            return Err("duplicate profile identity".into());
        }
        if p["name"].as_str().is_none_or(str::is_empty)
            || p["robe"].as_str().is_none_or(str::is_empty)
        {
            return Err("profile needs a name and robe".into());
        }
        let colour: Vec<f32> = serde_json::from_value(p["colour"].clone())
            .map_err(|e| format!("profile colour: {e}"))?;
        if !magicka_network::session::valid_player_colour(&colour) {
            return Err("invalid profile colour".into());
        }
    }
    Ok(())
}
