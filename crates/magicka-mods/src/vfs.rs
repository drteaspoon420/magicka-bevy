//! Layered virtual filesystem: base content, then mods in resolved order.
//! Later layers win for the same normalized logical path; collisions are
//! whole-file replacement (typed patches are a definitions-level feature).
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

/// A platform-independent content path: lowercase, `/`-separated, relative,
/// without `.`/`..` segments.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalPath(String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathError {
    Empty,
    Absolute(String),
    Escape(String),
    Invalid(String),
}
impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty path"),
            Self::Absolute(p) => write!(f, "absolute path {p:?}"),
            Self::Escape(p) => write!(f, "path escapes its root: {p:?}"),
            Self::Invalid(p) => write!(f, "invalid path {p:?}"),
        }
    }
}

pub const MAX_PATH: usize = 256;

impl LogicalPath {
    pub fn new(raw: &str) -> Result<Self, PathError> {
        if raw.is_empty() {
            return Err(PathError::Empty);
        }
        if raw.len() > MAX_PATH || raw.chars().any(|c| c.is_control()) {
            return Err(PathError::Invalid(raw.into()));
        }
        let unified = raw.replace('\\', "/");
        if unified.starts_with('/')
            || unified.as_bytes().get(1) == Some(&b':')
            || unified.contains("://")
        {
            return Err(PathError::Absolute(raw.into()));
        }
        let mut parts = Vec::new();
        for part in unified.split('/') {
            match part {
                "" | "." => {}
                ".." => return Err(PathError::Escape(raw.into())),
                p if p.contains(':') || p.ends_with(' ') || p.ends_with('.') => {
                    return Err(PathError::Invalid(raw.into()))
                }
                p => parts.push(p.to_lowercase()),
            }
        }
        if parts.is_empty() {
            return Err(PathError::Empty);
        }
        Ok(Self(parts.join("/")))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Resolve `relative` against this path's directory (for `#base`,
    /// `require`): `..` may not climb above the content root.
    pub fn sibling(&self, relative: &str) -> Result<Self, PathError> {
        let dir = self.0.rsplit_once('/').map_or("", |(d, _)| d);
        let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
        let relative_unified = relative.replace('\\', "/");
        for part in relative_unified.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop().ok_or(PathError::Escape(relative.into()))?;
                }
                p => parts.push(p),
            }
        }
        Self::new(&parts.join("/"))
    }
    /// Whether this path is `prefix` or lies under it (`""`: everything).
    pub fn starts_with(&self, prefix: &str) -> bool {
        let prefix = prefix.trim_end_matches('/').to_lowercase();
        prefix.is_empty()
            || self.0 == prefix
            || self
                .0
                .strip_prefix(&prefix)
                .is_some_and(|r| r.starts_with('/'))
    }
    pub fn extension(&self) -> Option<&str> {
        self.0
            .rsplit_once('.')
            .map(|(_, e)| e)
            .filter(|e| !e.contains('/'))
    }
}
impl fmt::Display for LogicalPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One mounted directory.
#[derive(Clone, Debug)]
pub struct Layer {
    /// `base` or the mod ID.
    pub name: String,
    pub root: PathBuf,
    files: BTreeMap<LogicalPath, PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MountError {
    Io(String),
    Path(String, PathError),
    /// Two files differ only in case (or separators) inside one layer.
    CaseCollision(String, String),
    /// A symlink resolves outside its layer root.
    SymlinkEscape(String),
    TooManyFiles(String),
}
impl fmt::Display for MountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Path(file, e) => write!(f, "{file}: {e}"),
            Self::CaseCollision(a, b) => write!(f, "{a} and {b} collide when case is ignored"),
            Self::SymlinkEscape(p) => write!(f, "{p} links outside its mod"),
            Self::TooManyFiles(r) => write!(f, "{r} has too many files"),
        }
    }
}

pub const MAX_LAYER_FILES: usize = 65_536;

impl Layer {
    /// Index every regular file under `root`. Files whose names begin with
    /// `.` are skipped (editor/VCS metadata).
    pub fn mount(name: &str, root: &Path) -> Result<Self, MountError> {
        let canonical = root
            .canonicalize()
            .map_err(|e| MountError::Io(format!("{}: {e}", root.display())))?;
        let mut files = BTreeMap::new();
        let mut stack = vec![canonical.clone()];
        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir)
                .map_err(|e| MountError::Io(format!("{}: {e}", dir.display())))?;
            let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let name_os = entry.file_name();
                let file_name = name_os.to_string_lossy();
                if file_name.starts_with('.') {
                    continue;
                }
                let path = entry.path();
                let real = path
                    .canonicalize()
                    .map_err(|e| MountError::Io(format!("{}: {e}", path.display())))?;
                let relative = path.strip_prefix(&canonical).unwrap().to_string_lossy();
                if !real.starts_with(&canonical) {
                    return Err(MountError::SymlinkEscape(relative.into_owned()));
                }
                if real.is_dir() {
                    stack.push(path);
                    continue;
                }
                let logical = LogicalPath::new(&relative)
                    .map_err(|e| MountError::Path(relative.to_string(), e))?;
                if let Some(old) = files.insert(logical, real.clone()) {
                    return Err(MountError::CaseCollision(
                        old.strip_prefix(&canonical)
                            .unwrap_or(&old)
                            .display()
                            .to_string(),
                        relative.into_owned(),
                    ));
                }
                if files.len() > MAX_LAYER_FILES {
                    return Err(MountError::TooManyFiles(name.into()));
                }
            }
        }
        Ok(Self {
            name: name.into(),
            root: canonical,
            files,
        })
    }
    /// An in-memory layer for tests and generated content.
    pub fn virtual_layer(
        name: &str,
        files: impl IntoIterator<Item = (LogicalPath, PathBuf)>,
    ) -> Self {
        Self {
            name: name.into(),
            root: PathBuf::new(),
            files: files.into_iter().collect(),
        }
    }
    pub fn files(&self) -> impl Iterator<Item = (&LogicalPath, &PathBuf)> {
        self.files.iter()
    }
    pub fn contains(&self, path: &LogicalPath) -> bool {
        self.files.contains_key(path)
    }
}

/// Where a logical path resolved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub layer: String,
    pub physical: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct Vfs {
    layers: Vec<Layer>,
    /// Winning bytes captured at load (authoritative files), so what runs
    /// is exactly what was hashed even if the disk changes mid-match.
    pinned: BTreeMap<LogicalPath, std::sync::Arc<Vec<u8>>>,
}
impl Vfs {
    pub fn new(layers: Vec<Layer>) -> Self {
        Self {
            layers,
            pinned: BTreeMap::new(),
        }
    }
    /// Capture the current winning bytes of every path `keep` selects.
    pub fn pin(&mut self, keep: impl Fn(&LogicalPath) -> bool) -> Result<(), String> {
        for path in self.list("") {
            if keep(&path) {
                let resolved = self.resolve(&path).unwrap();
                let bytes =
                    std::fs::read(&resolved.physical).map_err(|e| format!("{path}: {e}"))?;
                self.pinned.insert(path, std::sync::Arc::new(bytes));
            }
        }
        Ok(())
    }
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }
    /// The winning (last) layer for `path`.
    pub fn resolve(&self, path: &LogicalPath) -> Option<Resolved> {
        self.layers.iter().rev().find_map(|l| {
            l.files.get(path).map(|p| Resolved {
                layer: l.name.clone(),
                physical: p.clone(),
            })
        })
    }
    /// Every layer providing `path`, lowest first (diagnostics).
    pub fn overlaps(&self, path: &LogicalPath) -> Vec<Resolved> {
        self.layers
            .iter()
            .filter_map(|l| {
                l.files.get(path).map(|p| Resolved {
                    layer: l.name.clone(),
                    physical: p.clone(),
                })
            })
            .collect()
    }
    pub fn read(&self, path: &LogicalPath) -> Option<Result<Vec<u8>, String>> {
        if let Some(bytes) = self.pinned.get(path) {
            return Some(Ok(bytes.as_ref().clone()));
        }
        let resolved = self.resolve(path)?;
        Some(std::fs::read(&resolved.physical).map_err(|e| format!("{path}: {e}")))
    }
    pub fn read_string(&self, path: &LogicalPath) -> Option<Result<String, String>> {
        self.read(path).map(|r| {
            r.and_then(|b| String::from_utf8(b).map_err(|_| format!("{path} is not UTF-8")))
        })
    }
    /// The merged listing under `prefix`, sorted.
    pub fn list(&self, prefix: &str) -> Vec<LogicalPath> {
        let mut out: Vec<_> = self
            .layers
            .iter()
            .flat_map(|l| l.files.keys())
            .filter(|p| p.starts_with(prefix))
            .cloned()
            .collect();
        out.sort();
        out.dedup();
        out
    }
}
