//! Readers for Magicka's XNA 3.1 content files.
//!
//! - [`lzx`]: LZX (XMemCompress) decompressor.
//! - [`xnb`]: `.xnb` container, type-reader manifest, Texture2D structure.
//!
//! Format notes and evidence: `output/research/xnb-format.md`.

pub mod lzx;
pub mod xnb;

/// The user's retail `Magicka/Content` folder for tests and tools.
///
/// `MAGICKA_CONTENT` wins. Otherwise the first existing of `<workspace>/Magicka/Content`
/// (public repository: the game install linked as `./Magicka`) and the research
/// lab's `<lab>/target/Magicka/Content`. When neither exists the lab path is
/// returned, so callers report a clear missing-file error or skip.
pub fn retail_content_dir() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("MAGICKA_CONTENT") {
        return path.into();
    }
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let public = workspace.join("Magicka/Content");
    if public.is_dir() {
        return public;
    }
    workspace.join("../../target/Magicka/Content")
}
