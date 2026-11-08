//! `mod.kv` manifests, the `load_order.toml` file, and dependency resolution.
use crate::kv;
use semver::{Version, VersionReq};
use std::{collections::BTreeMap, fmt, path::Path};

/// The implicit base layer every mod may depend on.
pub const BASE_ID: &str = "base.game";
pub const BASE_VERSION: Version = Version::new(1, 0, 0);
/// Mod API versions this engine accepts (`ApiVersion` in `mod.kv`).
pub const SUPPORTED_API: std::ops::RangeInclusive<u32> = 1..=1;

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub version: Version,
    pub api_version: u32,
    pub display_name_key: Option<String>,
    /// Ships server-side Lua (authoritative).
    pub server_scripts: bool,
    /// Ships client presentation assets.
    pub client_assets: bool,
    /// Presentation-only: may not carry authoritative files.
    pub client_only: bool,
    pub dependencies: Vec<(String, VersionReq)>,
    pub conflicts: Vec<(String, VersionReq)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModError {
    Kv(kv::KvError),
    Manifest(String, String),
    UnknownMod(String),
    Missing {
        by: String,
        needs: String,
        req: String,
    },
    Version {
        by: String,
        needs: String,
        req: String,
        found: String,
    },
    Conflict(String, String),
    Cycle(Vec<String>),
    Duplicate(String),
    Order(String),
    Mount(String),
}
impl fmt::Display for ModError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kv(e) => write!(f, "{e}"),
            Self::Manifest(m, e) => write!(f, "{m}/mod.kv: {e}"),
            Self::UnknownMod(m) => write!(f, "enabled mod {m:?} is not installed"),
            Self::Missing { by, needs, req } => {
                write!(f, "{by} needs {needs} {req}, which is not enabled")
            }
            Self::Version {
                by,
                needs,
                req,
                found,
            } => write!(f, "{by} needs {needs} {req}, found {found}"),
            Self::Conflict(a, b) => write!(f, "{a} conflicts with {b}"),
            Self::Cycle(c) => write!(f, "dependency cycle: {}", c.join(" -> ")),
            Self::Duplicate(m) => write!(f, "mod {m:?} is installed or enabled twice"),
            Self::Order(e) => write!(f, "load_order.toml: {e}"),
            Self::Mount(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for ModError {}

pub fn valid_mod_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        && !id.starts_with('.')
        && !id.ends_with('.')
}

fn flag(node: &kv::KvNode, key: &str, dir: &str) -> Result<bool, ModError> {
    match node.get_str(key) {
        None | Some("0") => Ok(false),
        Some("1") => Ok(true),
        Some(v) => Err(ModError::Manifest(
            dir.into(),
            format!("{key} must be 0 or 1, not {v:?}"),
        )),
    }
}
fn requirements(
    node: &kv::KvNode,
    key: &str,
    dir: &str,
) -> Result<Vec<(String, VersionReq)>, ModError> {
    let Some(block) = node.get(key) else {
        return Ok(vec![]);
    };
    if block.str().is_some() {
        return Err(ModError::Manifest(
            dir.into(),
            format!("{key} must be a block"),
        ));
    }
    block
        .children()
        .iter()
        .map(|n| {
            let id = n.key.to_ascii_lowercase();
            if !valid_mod_id(&id) {
                return Err(ModError::Manifest(
                    dir.into(),
                    format!("{key}: bad mod id {:?}", n.key),
                ));
            }
            let text = n.str().unwrap_or("*");
            let req = VersionReq::parse(if text.is_empty() { "*" } else { text })
                .map_err(|e| ModError::Manifest(dir.into(), format!("{key} {id}: {e}")))?;
            Ok((id, req))
        })
        .collect()
}

impl Manifest {
    pub fn parse(dir: &str, text: &str) -> Result<Self, ModError> {
        let doc = kv::parse(&format!("{dir}/mod.kv"), text).map_err(ModError::Kv)?;
        if !doc.bases.is_empty() {
            return Err(ModError::Manifest(
                dir.into(),
                "#base is not allowed in mod.kv".into(),
            ));
        }
        let [root] = doc.nodes.as_slice() else {
            return Err(ModError::Manifest(
                dir.into(),
                "expected one \"Mod\" block".into(),
            ));
        };
        if !root.key.eq_ignore_ascii_case("Mod") || root.str().is_some() {
            return Err(ModError::Manifest(
                dir.into(),
                "expected one \"Mod\" block".into(),
            ));
        }
        let id = root
            .get_str("Id")
            .ok_or_else(|| ModError::Manifest(dir.into(), "missing Id".into()))?
            .to_string();
        if !valid_mod_id(&id) || id == BASE_ID {
            return Err(ModError::Manifest(dir.into(), format!("invalid Id {id:?}")));
        }
        let version = root
            .get_str("Version")
            .ok_or_else(|| ModError::Manifest(dir.into(), "missing Version".into()))
            .and_then(|v| {
                Version::parse(v)
                    .map_err(|e| ModError::Manifest(dir.into(), format!("Version: {e}")))
            })?;
        let api_version = root
            .get_str("ApiVersion")
            .ok_or_else(|| ModError::Manifest(dir.into(), "missing ApiVersion".into()))?
            .parse::<u32>()
            .map_err(|_| ModError::Manifest(dir.into(), "ApiVersion must be an integer".into()))?;
        if !SUPPORTED_API.contains(&api_version) {
            return Err(ModError::Manifest(
                dir.into(),
                format!(
                    "ApiVersion {api_version} is not supported (engine supports {SUPPORTED_API:?})"
                ),
            ));
        }
        let manifest = Self {
            id,
            version,
            api_version,
            display_name_key: root.get_str("DisplayNameKey").map(str::to_string),
            server_scripts: flag(root, "ServerScripts", dir)?,
            client_assets: flag(root, "ClientAssets", dir)?,
            client_only: flag(root, "ClientOnly", dir)?,
            dependencies: requirements(root, "Dependencies", dir)?,
            conflicts: requirements(root, "Conflicts", dir)?,
        };
        if manifest.client_only && manifest.server_scripts {
            return Err(ModError::Manifest(
                dir.into(),
                "a ClientOnly mod cannot have ServerScripts".into(),
            ));
        }
        Ok(manifest)
    }
    pub fn label(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }
}

/// `load_order.toml`. Supported subset: comments, blank lines, and one
/// `enabled = [ "id", ... ]` array (possibly spanning lines).
pub fn parse_load_order(text: &str) -> Result<Vec<String>, ModError> {
    let stripped: String = text
        .lines()
        .map(|l| {
            // '#' starts a comment outside strings (ids cannot contain '#').
            let mut in_str = false;
            let mut out = String::new();
            for c in l.chars() {
                if c == '"' {
                    in_str = !in_str;
                }
                if c == '#' && !in_str {
                    break;
                }
                out.push(c);
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }
    let rest = trimmed
        .strip_prefix("enabled")
        .map(str::trim_start)
        .and_then(|r| r.strip_prefix('='))
        .map(str::trim)
        .ok_or_else(|| ModError::Order("expected `enabled = [ ... ]`".into()))?;
    let inner = rest
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .ok_or_else(|| ModError::Order("`enabled` must be a single array".into()))?;
    let mut out = Vec::new();
    for item in inner.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let id = item
            .strip_prefix('"')
            .and_then(|i| i.strip_suffix('"'))
            .ok_or_else(|| ModError::Order(format!("{item} must be a quoted mod id")))?;
        if !valid_mod_id(id) {
            return Err(ModError::Order(format!("invalid mod id {id:?}")));
        }
        if out.iter().any(|o| o == id) {
            return Err(ModError::Duplicate(id.into()));
        }
        out.push(id.to_string());
    }
    Ok(out)
}

/// An installed mod package.
#[derive(Clone, Debug)]
pub struct Package {
    pub manifest: Manifest,
    pub root: std::path::PathBuf,
}

/// Every `<mods>/<dir>/mod.kv`, by mod ID.
pub fn discover(mods: &Path) -> Result<BTreeMap<String, Package>, ModError> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(mods) else {
        return Ok(out);
    };
    let mut dirs: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    dirs.sort();
    for dir in dirs {
        let manifest_path = dir.join("mod.kv");
        if !manifest_path.is_file() {
            continue;
        }
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&manifest_path)
            .map_err(|e| ModError::Manifest(name.clone(), e.to_string()))?;
        let manifest = Manifest::parse(&name, &text)?;
        let id = manifest.id.clone();
        if out
            .insert(
                id.clone(),
                Package {
                    manifest,
                    root: dir,
                },
            )
            .is_some()
        {
            return Err(ModError::Duplicate(id));
        }
    }
    Ok(out)
}

/// Resolve the enabled list into a load order: dependencies before
/// dependents; otherwise the user's order. Rejects unknown/missing mods,
/// unmet version ranges, declared conflicts and cycles. Dependencies are
/// never enabled implicitly.
pub fn resolve_order<'a>(
    installed: &'a BTreeMap<String, Package>,
    enabled: &[String],
) -> Result<Vec<&'a Package>, ModError> {
    let mut chosen: Vec<&Package> = Vec::new();
    for id in enabled {
        let package = installed
            .get(id)
            .ok_or_else(|| ModError::UnknownMod(id.clone()))?;
        if chosen.iter().any(|p| p.manifest.id == *id) {
            return Err(ModError::Duplicate(id.clone()));
        }
        chosen.push(package);
    }
    let version_of = |id: &str| -> Option<Version> {
        if id == BASE_ID {
            return Some(BASE_VERSION);
        }
        chosen
            .iter()
            .find(|p| p.manifest.id == id)
            .map(|p| p.manifest.version.clone())
    };
    for p in &chosen {
        for (dep, req) in &p.manifest.dependencies {
            match version_of(dep) {
                None => {
                    return Err(ModError::Missing {
                        by: p.manifest.label(),
                        needs: dep.clone(),
                        req: req.to_string(),
                    })
                }
                Some(v) if !req.matches(&v) => {
                    return Err(ModError::Version {
                        by: p.manifest.label(),
                        needs: dep.clone(),
                        req: req.to_string(),
                        found: v.to_string(),
                    })
                }
                Some(_) => {}
            }
        }
        for (other, req) in &p.manifest.conflicts {
            if let Some(v) = version_of(other) {
                if req.matches(&v) {
                    return Err(ModError::Conflict(
                        p.manifest.label(),
                        format!("{other}@{v}"),
                    ));
                }
            }
        }
    }
    // Kahn's algorithm; among ready mods take the earliest in user order.
    let index: BTreeMap<&str, usize> = chosen
        .iter()
        .enumerate()
        .map(|(i, p)| (p.manifest.id.as_str(), i))
        .collect();
    let deps: Vec<Vec<usize>> = chosen
        .iter()
        .map(|p| {
            p.manifest
                .dependencies
                .iter()
                .filter_map(|(d, _)| index.get(d.as_str()).copied())
                .collect()
        })
        .collect();
    let mut placed = vec![false; chosen.len()];
    let mut order = Vec::new();
    while order.len() < chosen.len() {
        let Some(next) =
            (0..chosen.len()).find(|&i| !placed[i] && deps[i].iter().all(|&d| placed[d]))
        else {
            // Report one cycle among the remaining mods.
            let start = (0..chosen.len()).find(|&i| !placed[i]).unwrap();
            let mut path = vec![start];
            let mut at = start;
            loop {
                at = *deps[at].iter().find(|&&d| !placed[d]).unwrap();
                if let Some(pos) = path.iter().position(|&p| p == at) {
                    let mut cycle: Vec<String> = path[pos..]
                        .iter()
                        .map(|&i| chosen[i].manifest.id.clone())
                        .collect();
                    cycle.push(chosen[at].manifest.id.clone());
                    return Err(ModError::Cycle(cycle));
                }
                path.push(at);
            }
        };
        placed[next] = true;
        order.push(chosen[next]);
    }
    Ok(order)
}
