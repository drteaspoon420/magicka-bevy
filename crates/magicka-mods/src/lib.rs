//! Mod packages (rewrite extension, plan workstream C): KeyValues syntax,
//! `mod.kv` manifests, `load_order.toml`, deterministic dependency order, a
//! layered virtual filesystem in which later mods win, and canonical
//! content hashes that server and client compare before play.
pub mod kv;
pub mod manifest;
pub mod vfs;

use manifest::{Manifest, ModError, Package};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
use vfs::{Layer, LogicalPath, Vfs};

pub use manifest::{BASE_ID, BASE_VERSION};

/// Bumped when the hashed layout or authoritative path rules change.
pub const CONTENT_SCHEMA: u32 = 1;

/// Logical prefixes whose contents affect gameplay and must match exactly
/// between server and clients. Everything else is presentation.
pub const AUTHORITATIVE: [&str; 3] = ["scripts/npc", "scripts/vscripts", "scripts/levels"];

pub fn is_authoritative(path: &LogicalPath) -> bool {
    AUTHORITATIVE.iter().any(|p| path.starts_with(p))
}

/// What both peers agree on before play.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContentManifest {
    /// Hash of authoritative content; `None` for an unmodded game.
    pub rules_hash: Option<String>,
    /// Gameplay mods in load order, `id@version`.
    pub mods: Vec<String>,
    /// Presentation files by logical path → SHA-256 (hex), for clients to
    /// check the assets they resolve.
    pub presentation: BTreeMap<String, String>,
}

/// The mounted, ordered content of one match.
#[derive(Clone, Debug)]
pub struct ModSet {
    pub vfs: Vfs,
    /// Enabled mods in resolved load order.
    pub mods: Vec<Manifest>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Contents written to `load_order.toml` when it does not exist yet.
pub const DEFAULT_LOAD_ORDER: &str =
    "# Enabled mods, in load order (later mods win for the same file).\nenabled = []\n";

impl ModSet {
    /// Mount `base` (authored base definitions, may be absent) and the
    /// enabled mods under `mods_root` (`load_order.toml` there, unless an
    /// explicit list is given).
    pub fn load(
        base: Option<&Path>,
        mods_root: &Path,
        enabled: Option<&[String]>,
    ) -> Result<Self, ModError> {
        let installed = manifest::discover(mods_root)?;
        let enabled: Vec<String> = match enabled {
            Some(list) => list.to_vec(),
            None => {
                let path = mods_root.join("load_order.toml");
                match std::fs::read_to_string(&path) {
                    Ok(text) => manifest::parse_load_order(&text)?,
                    Err(e) => {
                        // First run: the per-user load order is not tracked;
                        // create the default (nothing enabled) when the mods
                        // folder exists. A read-only tree still loads.
                        if e.kind() == std::io::ErrorKind::NotFound && mods_root.is_dir() {
                            let _ = std::fs::write(&path, DEFAULT_LOAD_ORDER);
                        }
                        vec![]
                    }
                }
            }
        };
        Self::from_packages(base, &installed, &enabled)
    }
    pub fn from_packages(
        base: Option<&Path>,
        installed: &BTreeMap<String, Package>,
        enabled: &[String],
    ) -> Result<Self, ModError> {
        let order = manifest::resolve_order(installed, enabled)?;
        let mut layers = Vec::new();
        if let Some(base) = base.filter(|b| b.is_dir()) {
            layers.push(
                Layer::mount(BASE_ID, base).map_err(|e| ModError::Mount(format!("base: {e}")))?,
            );
        }
        for package in &order {
            let layer = Layer::mount(&package.manifest.id, &package.root)
                .map_err(|e| ModError::Mount(format!("{}: {e}", package.manifest.id)))?;
            if package.manifest.client_only {
                if let Some((path, _)) = layer.files().find(|(p, _)| is_authoritative(p)) {
                    return Err(ModError::Manifest(
                        package.manifest.id.clone(),
                        format!("ClientOnly mod ships authoritative file {path}"),
                    ));
                }
            }
            layers.push(layer);
        }
        let mut vfs = Vfs::new(layers);
        // Authoritative files (and localization, read by the definitions
        // compiler) are pinned at load: a match runs what it hashed.
        vfs.pin(|p| is_authoritative(p) || p.starts_with("localization"))
            .map_err(ModError::Mount)?;
        Ok(Self {
            vfs,
            mods: order.into_iter().map(|p| p.manifest.clone()).collect(),
        })
    }
    /// An unmodded game: no layers but (optionally) base.
    pub fn base_only(base: Option<&Path>) -> Result<Self, ModError> {
        Self::from_packages(base, &BTreeMap::new(), &[])
    }
    pub fn gameplay_mods(&self) -> impl Iterator<Item = &Manifest> {
        self.mods.iter().filter(|m| !m.client_only)
    }
    /// Canonical hashes. The rules hash covers the schema version, gameplay
    /// mod IDs/versions/API versions in order, and every authoritative
    /// logical path with its winning bytes. It is `None` when no gameplay
    /// mod is enabled, so unmodded peers interoperate with older builds.
    pub fn content_manifest(&self) -> Result<ContentManifest, String> {
        let mut presentation = BTreeMap::new();
        let mut rules = Sha256::new();
        rules.update(format!("magicka-content/{CONTENT_SCHEMA}\n"));
        for m in self.gameplay_mods() {
            rules.update(format!("mod {} {} api{}\n", m.id, m.version, m.api_version));
        }
        let client_only: Vec<&str> = self
            .mods
            .iter()
            .filter(|m| m.client_only)
            .map(|m| m.id.as_str())
            .collect();
        for path in self.vfs.list("") {
            let resolved = self.vfs.resolve(&path).unwrap();
            let bytes = self.vfs.read(&path).unwrap()?;
            let digest = sha256_hex(&bytes);
            if is_authoritative(&path) {
                if client_only.contains(&resolved.layer.as_str()) {
                    return Err(format!("{path}: client-only mods cannot override rules"));
                }
                rules.update(format!("file {path} {digest}\n"));
            } else {
                presentation.insert(path.to_string(), digest);
            }
        }
        let gameplay: Vec<String> = self.gameplay_mods().map(Manifest::label).collect();
        Ok(ContentManifest {
            rules_hash: (!gameplay.is_empty()).then(|| hex(&rules.finalize())),
            mods: gameplay,
            presentation,
        })
    }
    /// `mod resolve <path>` diagnostics: each overlapping layer and the
    /// winner, lowest first.
    pub fn explain(&self, raw: &str) -> Result<String, String> {
        let path = LogicalPath::new(raw).map_err(|e| e.to_string())?;
        let layers = self.vfs.overlaps(&path);
        if layers.is_empty() {
            return Ok(format!("{path}: not provided by any layer\n"));
        }
        let mut out = format!("{path}\n");
        for (i, r) in layers.iter().enumerate() {
            let winner = i + 1 == layers.len();
            out.push_str(&format!(
                "  {} {:<24} {}\n",
                if winner { "*" } else { " " },
                r.layer,
                r.physical.display()
            ));
        }
        out.push_str(&format!(
            "  winner: {} ({})\n",
            layers.last().unwrap().layer,
            if is_authoritative(&path) {
                "authoritative"
            } else {
                "presentation"
            }
        ));
        Ok(out)
    }
}
