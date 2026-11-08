// xWMA voices decode through FFmpeg. Resolve it as demo.sh does: an explicit
// MAGICKA_FFMPEG, then PATH, then a Nix ffmpeg-headless store path (Nix does
// not put store packages on PATH). The choice is exported once per test binary.
use std::{
    process::{Command, Stdio},
    sync::OnceLock,
};

/// True when an FFmpeg executable is available to `AudioLibrary`.
pub fn available() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| {
        if let Some(path) = std::env::var_os("MAGICKA_FFMPEG") {
            return runs(&path);
        }
        if runs("ffmpeg".as_ref()) {
            return true;
        }
        let Ok(output) = Command::new("nix-shell")
            .args(["-p", "ffmpeg-headless", "--run", "command -v ffmpeg"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        else {
            return false;
        };
        let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !output.status.success() || path.is_empty() || !runs(path.as_ref()) {
            return false;
        }
        std::env::set_var("MAGICKA_FFMPEG", path);
        true
    })
}

fn runs(executable: &std::ffi::OsStr) -> bool {
    let mut command = Command::new(executable);
    // Matches AudioLibrary: store binaries ignore the dev shell library path.
    if std::path::Path::new(executable).starts_with("/nix/store") {
        command.env_remove("LD_LIBRARY_PATH");
    }
    command
        .args(["-hide_banner", "-version"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Resolve FFmpeg, or report why an xWMA test is skipped.
pub fn require() -> bool {
    let found = available();
    if !found {
        eprintln!("SKIP: FFmpeg unavailable (PATH, MAGICKA_FFMPEG or nix-shell ffmpeg-headless)");
    }
    found
}
