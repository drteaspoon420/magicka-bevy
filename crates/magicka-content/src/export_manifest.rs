//! Shared `--missing` / `--manifest <file>` options of the export binaries.
//!
//! `--missing` skips outputs that already exist, so an interrupted or
//! partially deleted cache can be completed without re-exporting everything.
//! `--manifest <file>` lists every output the run wrote or found present, one
//! absolute path per line, so callers (`play.sh`) can cheaply verify the
//! cache on later runs.
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct ExportManifest {
    missing: bool,
    manifest: Option<PathBuf>,
    outputs: Vec<PathBuf>,
}

impl ExportManifest {
    /// Removes the manifest options from `args`.
    pub fn from_args(args: &mut Vec<String>) -> Self {
        let mut options = Self::default();
        let mut rest = Vec::with_capacity(args.len());
        let mut iter = std::mem::take(args).into_iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--missing" => options.missing = true,
                "--manifest" => options.manifest = iter.next().map(PathBuf::from),
                _ => rest.push(arg),
            }
        }
        *args = rest;
        options
    }

    /// True (and records them) when `--missing` is set and all `paths` exist.
    pub fn skip(&mut self, paths: &[&Path]) -> bool {
        if self.missing && paths.iter().all(|p| p.exists()) {
            self.outputs.extend(paths.iter().map(|p| p.to_path_buf()));
            true
        } else {
            false
        }
    }

    /// Records a written output.
    pub fn record(&mut self, path: &Path) {
        self.outputs.push(path.to_path_buf());
    }

    /// Writes the manifest, if one was requested.
    pub fn finish(&self) {
        let Some(manifest) = &self.manifest else {
            return;
        };
        let mut text = String::new();
        for path in &self.outputs {
            let path = std::path::absolute(path).unwrap_or_else(|_| path.clone());
            text.push_str(&path.to_string_lossy());
            text.push('\n');
        }
        if let Some(dir) = manifest.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(manifest, text).expect("write manifest");
    }
}
