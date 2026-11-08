use magicka_runtime::play_content::{level_files, load_start_scene};
use magicka_runtime::play_session::PlaySession;
use std::path::Path;

#[test]
fn nested_levels_and_start_scenes_resolve_from_the_level_directory() {
    let content = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !content.is_dir() {
        return;
    }
    let files = level_files(&content).unwrap();
    assert!(files.contains(&"Challenges/ch_arena.lvl".into()));
    assert!(files.contains(&"Tsar/Tsar_WizardCastle.lvl".into()));
    assert!(files.iter().any(|p| p.starts_with("Versus/")));
    let mut failures = Vec::new();
    for file in &files {
        if let Err(e) = load_start_scene(&content, file) {
            failures.push(format!("{file}: {e:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!(
        "Resolved start scenes and decoded level models for {} levels",
        files.len()
    );
    let mut session = PlaySession::load(&content, "Challenges/ch_arena.lvl", "Wizard").unwrap();
    for _ in 0..30 {
        session.step(1.0 / 60.0);
    }
    assert_eq!(session.scene_name(), "chs_havindr_arena");
    assert!(session.script_error.is_none(), "{:?}", session.script_error);
}
