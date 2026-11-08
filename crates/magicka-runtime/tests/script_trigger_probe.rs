//! Script host coverage probe over retail content: for every level and scene,
//! run each trigger with its conditions ignored, then step up to
//! PROBE_SECONDS (default 8) of simulation, recording the first script error
//! (any error halts a session's scripts) or panic. Each trigger runs in the
//! same session as the previous ones; a scene change reloads the scene.
//!
//! Ignored by default (slow). Run in release:
//! `cargo test --locked --release -p magicka-runtime --test all -- script_trigger_probe:: --ignored --nocapture`
//! Filters: PROBE_LEVELS=Wizard_Castle.lvl,Versus/vs_boat.lvl; PROBE_OUT=<absolute report path>;
//! PROBE_PANIC_HOOK=1 prints caught panics (with RUST_BACKTRACE=1).
use magicka_runtime::avatar::PlayerInput;
use magicka_runtime::play_session::PlaySession;
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    root.is_dir().then_some(root)
}

fn levels(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lvl")) {
                out.push(
                    p.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let base = root.join("Levels");
    let mut out = Vec::new();
    walk(&base, &base, &mut out);
    out.sort();
    out
}

/// (scene stem, scene file) for the level's <Scene> entries (paths may
/// use backslashes and are relative to the level file).
fn scenes(root: &Path, level: &str) -> Vec<(String, PathBuf)> {
    let file = root.join("Levels").join(level);
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    text.split("<Scene>")
        .skip(1)
        .filter_map(|s| s.split("</Scene>").next())
        .map(|s| s.trim().replace('\\', "/"))
        .filter_map(|s| {
            let stem = Path::new(&s)
                .file_stem()?
                .to_string_lossy()
                .to_ascii_lowercase();
            // Scene paths do not always match the directory casing.
            let path = magicka_runtime::play_content::resolve(file.parent()?, Path::new(&s))
                .unwrap_or_else(|| file.parent().unwrap().join(&s));
            Some((stem, path))
        })
        .collect()
}

/// Static parse coverage: unsupported action/condition tags in a scene.
fn unsupported(file: &Path) -> Result<BTreeMap<String, usize>, String> {
    use magicka_runtime::scene_script::{ActionDefinition, ConditionDefinition, SceneScriptLayout};
    let bytes = std::fs::read(file).map_err(|e| format!("read: {e}"))?;
    let layout = SceneScriptLayout::read(&bytes, false).map_err(|e| format!("parse: {e:?}"))?;
    let mut gaps = BTreeMap::new();
    for a in &layout.actions {
        if let ActionDefinition::Unsupported(n) = a {
            *gaps
                .entry(format!("action:{}", n.name.to_ascii_lowercase()))
                .or_default() += 1;
        }
    }
    for c in &layout.conditions {
        if let ConditionDefinition::Unsupported(n) = c {
            *gaps
                .entry(format!("condition:{}", n.name.to_ascii_lowercase()))
                .or_default() += 1;
        }
    }
    Ok(gaps)
}

/// Runtime log lines that admit a missing feature.
fn admits_gap(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    [
        "stand-in",
        "not implemented",
        "unsupported",
        "not ported",
        "not modelled",
        "missing",
    ]
    .iter()
    .any(|k| l.contains(k))
}

fn open(root: &Path, level: &str, scene: &str) -> Result<PlaySession, String> {
    catch_unwind(AssertUnwindSafe(|| -> Result<PlaySession, String> {
        let mut s = PlaySession::load(root, level, "Wizard").map_err(|e| format!("load: {e:?}"))?;
        if s.scene_name() != scene {
            s.change_scene_named(scene, "start")
                .map_err(|e| format!("change scene: {e:?}"))?;
        }
        for _ in 0..3 {
            s.step(1.0 / 60.0);
        }
        Ok(s)
    }))
    .unwrap_or_else(|p| Err(format!("panic: {}", panic_text(&p))))
}

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "?".into())
}

/// Collapse handles/numbers so the same gap groups together.
fn kind(e: &str) -> String {
    let mut out = String::new();
    let mut digits = false;
    for c in e.chars() {
        if c.is_ascii_digit() {
            if !digits {
                out.push('#');
            }
            digits = true;
        } else {
            digits = false;
            out.push(c);
        }
    }
    out.chars().take(160).collect()
}

#[test]
#[ignore]
fn every_level_trigger_runs_without_script_errors() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    // PROBE_PANIC_HOOK=1 keeps the default hook (with RUST_BACKTRACE) so a
    // caught panic still prints where it happened.
    if std::env::var_os("PROBE_PANIC_HOOK").is_none() {
        std::panic::set_hook(Box::new(|_| {}));
    }
    let seconds: usize = std::env::var("PROBE_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let filter: Option<Vec<String>> = std::env::var("PROBE_LEVELS")
        .ok()
        .map(|v| v.split(',').map(|s| s.to_ascii_lowercase()).collect());
    let mut lines = Vec::new();
    let mut kinds: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut parse_gaps: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut log_gaps: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let (mut triggers, mut failures) = (0usize, 0usize);
    for level in levels(&root) {
        if filter
            .as_ref()
            .is_some_and(|f| !f.contains(&level.to_ascii_lowercase()))
        {
            continue;
        }
        for (scene, file) in scenes(&root, &level) {
            match unsupported(&file) {
                Ok(gaps) => {
                    for (tag, n) in gaps {
                        parse_gaps
                            .entry(tag)
                            .or_default()
                            .push(format!("{level} {scene} ×{n}"));
                    }
                }
                Err(e) => lines.push(format!("{level} {scene} PARSE: {e}")),
            }
            let mut s = match open(&root, &level, &scene) {
                Ok(s) => s,
                Err(e) => {
                    lines.push(format!("{level} {scene} SCENE: {e}"));
                    kinds
                        .entry(kind(&e))
                        .or_default()
                        .push(format!("{level} {scene}"));
                    failures += 1;
                    continue;
                }
            };
            for name in s.trigger_names() {
                triggers += 1;
                if s.scene_name() != scene || s.script_error.is_some() {
                    match open(&root, &level, &scene) {
                        Ok(fresh) => s = fresh,
                        Err(_) => break,
                    }
                }
                let result = catch_unwind(AssertUnwindSafe(|| -> Option<String> {
                    if let Err(e) = s.run_trigger(&name, true) {
                        return Some(format!("immediate: {e}"));
                    }
                    for _ in 0..60 * seconds {
                        s.step(1.0 / 60.0);
                        s.handle_input(PlayerInput::default(), 16.0 / 9.0);
                        s.events.clear();
                        if s.script_error.is_some() || s.scene_name() != scene {
                            break;
                        }
                    }
                    s.script_error.clone()
                }));
                let error = match result {
                    Ok(e) => e,
                    Err(p) => {
                        // The session may be inconsistent after a panic.
                        s.script_error = Some("panicked".into());
                        Some(format!("panic: {}", panic_text(&p)))
                    }
                };
                for line in s.world.log.drain(..) {
                    if admits_gap(&line) {
                        let key = kind(&line);
                        let entry = log_gaps.entry(key).or_default();
                        if entry.len() < 3 {
                            entry.push(format!("{level} {scene} [{name}]"));
                        }
                    }
                }
                if let Some(e) = error {
                    failures += 1;
                    lines.push(format!("{level} {scene} [{name}]: {e}"));
                    kinds
                        .entry(kind(&e))
                        .or_default()
                        .push(format!("{level} {scene} [{name}]"));
                }
            }
        }
    }
    let mut report = format!("triggers {triggers}, failures {failures}\n\n== by kind\n");
    let mut sorted: Vec<_> = kinds.iter().collect();
    sorted.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    for (k, v) in sorted {
        report += &format!("{:4} {k}\n       e.g. {}\n", v.len(), v[0]);
    }
    report += "\n== unsupported tags (static parse)\n";
    for (tag, places) in &parse_gaps {
        report += &format!("{tag}: {}\n", places.join(", "));
    }
    report += "\n== runtime log admissions (first 3 places)\n";
    for (line, places) in &log_gaps {
        report += &format!("{line}\n       at {}\n", places.join("; "));
    }
    report += "\n== all\n";
    for l in &lines {
        report += l;
        report += "\n";
    }
    let _ = std::panic::take_hook();
    // Always print: a long run must not be lost to a bad PROBE_OUT (a
    // relative path resolves against the crate directory).
    eprintln!("{report}");
    if let Some(path) = std::env::var_os("PROBE_OUT") {
        if let Err(e) = std::fs::write(&path, &report) {
            panic!("write PROBE_OUT {path:?}: {e}");
        }
    }
}
