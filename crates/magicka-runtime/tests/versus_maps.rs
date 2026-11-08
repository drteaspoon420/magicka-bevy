//! Every installed Versus map under each lobby mode with two players: the
//! ruleset attaches, both players reach authored spawns or stay put, and the
//! scene scripts and ruleset run without errors.
use magicka_runtime::{
    play_session::PlaySession,
    ruleset::{VersusMode, VersusSettings, TEAM_BLUE, TEAM_RED},
};
use std::path::PathBuf;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

#[test]
fn every_versus_map_runs_each_lobby_mode() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let mut levels: Vec<_> = std::fs::read_dir(root.join("Levels/Versus"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lvl")))
        .collect();
    levels.sort();
    assert_eq!(levels.len(), 12);
    let mut runs = 0;
    for path in &levels {
        let level = format!("Versus/{}", path.file_name().unwrap().to_string_lossy());
        for mode in VersusMode::LOBBY {
            for teams in [false, true] {
                let mut s = PlaySession::load(&root, &level, "Wizard").unwrap();
                // The modern host join needs `<start area>1`; one map only
                // authors its Versus spawns (versus_spawnN).
                if s.join_player_at_start(1, "Wizard").is_err() {
                    s.join_player(1, "Wizard", "versus_spawn").unwrap();
                }
                let mut settings = VersusSettings::defaults(mode);
                settings.teams = teams;
                s.set_versus(settings, [TEAM_RED, TEAM_BLUE, 0, 0]).unwrap();
                let r = s.world.versus().expect("versus ruleset attached");
                assert!(!r.areas.all.is_empty(), "{level}: spawn areas");
                for _ in 0..(6 * 30) {
                    s.step(1.0 / 30.0);
                }
                assert!(
                    s.script_error.is_none(),
                    "{level} {mode:?}: {:?}",
                    s.script_error
                );
                let failures: Vec<_> = s
                    .world
                    .log
                    .iter()
                    .filter(|l| l.starts_with("versus"))
                    .collect();
                assert!(
                    failures.is_empty(),
                    "{level} {mode:?} teams={teams}: {failures:?}"
                );
                assert_eq!(
                    s.end_game.condition,
                    magicka_runtime::end_game::EndGameCondition::None,
                    "{level} {mode:?}: two players keep the match running"
                );
                assert!(!s.world.flags.input_limited, "{level}: countdown released");
                runs += 1;
            }
        }
        eprintln!("{level}: ok");
    }
    assert_eq!(runs, 12 * 3 * 2);
}
