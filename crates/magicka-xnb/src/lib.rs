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
/// `MAGICKA_CONTENT` from the environment wins, then `MAGICKA_CONTENT` saved in
/// `<workspace>/.env` (by `play.sh`). Otherwise the first existing of
/// `<workspace>/Magicka/Content` (public repository: the game install linked as
/// `./Magicka`) and the research lab's `<lab>/target/Magicka/Content`. When
/// neither exists the lab path is returned, so callers report a clear
/// missing-file error or skip.
pub fn retail_content_dir() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("MAGICKA_CONTENT") {
        return path.into();
    }
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if let Some(path) = std::fs::read_to_string(workspace.join(".env"))
        .ok()
        .and_then(|text| dotenv_value(&text, "MAGICKA_CONTENT"))
    {
        return path.into();
    }
    let public = workspace.join("Magicka/Content");
    if public.is_dir() {
        return public;
    }
    workspace.join("../../target/Magicka/Content")
}

/// The last `KEY=value` (optionally `export`ed and quoted) for `key` in a
/// `.env` file, as `play.sh` writes and reads it.
pub fn dotenv_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
            let value = line.strip_prefix(key)?.strip_prefix('=')?;
            let unquoted = ['"', '\'']
                .iter()
                .find_map(|q| value.strip_prefix(*q)?.strip_suffix(*q));
            Some(unquoted.unwrap_or(value).to_string())
        })
        .last()
}

#[cfg(test)]
mod dotenv_tests {
    use super::dotenv_value;

    #[test]
    fn reads_plain_quoted_and_exported_values_and_ignores_other_keys() {
        let text =
            "# saved by play.sh\nMAGICKA_CACHE=/c\nMAGICKA_CONTENT=\"/games/Magicka/Content\"\n";
        assert_eq!(
            dotenv_value(text, "MAGICKA_CONTENT").as_deref(),
            Some("/games/Magicka/Content")
        );
        assert_eq!(
            dotenv_value("export MAGICKA_CONTENT='/a b'\r\n", "MAGICKA_CONTENT").as_deref(),
            Some("/a b")
        );
        assert_eq!(
            dotenv_value("MAGICKA_CONTENT=/x\nMAGICKA_CONTENT=/y", "MAGICKA_CONTENT").as_deref(),
            Some("/y")
        );
        assert_eq!(dotenv_value("MAGICKA_CONTENTS=/x", "MAGICKA_CONTENT"), None);
        assert_eq!(dotenv_value("", "MAGICKA_CONTENT"), None);
    }
}
