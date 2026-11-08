//! PlayState.SkipCutscene against user-supplied retail content (skips when
//! absent), plus the pure cutscene state rules. See research/cutscene-skip.md.
use magicka_runtime::{
    cutscene::{key_skips_cutscene, CutsceneState},
    play_session::PlaySession,
};
use std::path::{Path, PathBuf};

const DT: f32 = 1.0 / 60.0;

#[test]
fn wizard_castle_tutorial_intro_ends_without_dialog_input() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    s.headless_dialogs = Some([1280.0, 720.0]);
    for _ in 0..180 {
        s.step(DT);
    }
    assert!(
        s.world.cutscene.active,
        "opening cutscene must have started"
    );
    for _ in 0..3420 {
        s.step(DT);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert!(
        s.world.dialog_failure.is_none(),
        "{:?}",
        s.world.dialog_failure
    );
    assert!(
        !s.world.cutscene.active,
        "cutscene still active; text host {}, dialogs {}",
        s.world.dialog_text.is_some(),
        s.world.dialogs.is_some()
    );
}

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    root.is_dir().then_some(root)
}

#[test]
fn first_skip_shows_tip_second_returns_trigger_once() {
    let mut c = CutsceneState::default();
    c.begin(42, false);
    assert!(c.in_cutscene());
    assert_eq!(c.skip(false), None);
    assert!(c.show_skip_tip);
    c.update(0.25);
    assert_eq!(c.skip_tip_alpha, 0.5);
    assert_eq!(c.timer, 0.25);
    assert_eq!(c.skip(true), None, "clients never skip");
    assert_eq!(c.skip(false), Some(42));
    assert_eq!(c.skip(false), None, "already skipped");
    // A second cutscene without OnSkip: tip already shown, nothing runs.
    c.begin(0, true);
    assert_eq!(c.timer, 1.0);
    assert_eq!(c.skip(false), None);
    assert!(c.skipped);
    c.update(0.1);
    assert!(
        (c.skip_tip_alpha - 0.3).abs() < 1e-6,
        "{}",
        c.skip_tip_alpha
    );
}

#[test]
fn bars_slide_out_and_tip_resets_after_end() {
    let mut c = CutsceneState::default();
    c.begin(7, true);
    let _ = c.skip(false);
    c.update(0.5);
    assert_eq!(c.bars(720.0), Some((72.0, 648.0)));
    c.end();
    assert!(c.in_cutscene(), "IsInCutscene while bars are fully in");
    c.update(0.25);
    assert!(!c.in_cutscene());
    assert_eq!(c.bars(720.0), Some((54.0, 666.0)));
    assert!(c.show_skip_tip);
    c.update(1.0);
    assert_eq!(c.timer, 0.0);
    c.update(DT);
    assert!(!c.show_skip_tip);
    assert_eq!(c.skip_tip_alpha, 0.0);
    assert_eq!(c.bars(720.0), None);
}

#[test]
fn literal_key_mask() {
    // XNA Keys: Space 32, D 68, Enter 13, Escape 27, A 65, F1 112.
    assert!(key_skips_cutscene(32));
    assert!(key_skips_cutscene(68));
    assert!(!key_skips_cutscene(13));
    assert!(!key_skips_cutscene(27));
    assert!(!key_skips_cutscene(65));
    assert!(key_skips_cutscene(96)); // NumPad0
}

#[test]
fn havindr_intro_skip_runs_skip_intro_and_ends_cutscene() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    for _ in 0..120 {
        s.step(DT);
    }
    assert!(s.world.cutscene.active, "intro cutscene running");
    assert_ne!(s.world.cutscene.skip_trigger, 0, "onSkip installed");
    assert!(s.world.flags.input_limited);
    s.skip_cutscene();
    assert!(s.world.cutscene.show_skip_tip);
    assert!(!s.world.cutscene.skipped);
    s.skip_cutscene();
    assert!(s.world.cutscene.skipped);
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    // skip_intro: endCutscene at 2 s, enableInput at 3 s.
    for _ in 0..200 {
        s.step(DT);
    }
    assert!(!s.world.cutscene.active, "endCutscene ran");
    assert!(!s.world.flags.input_limited, "enableInput ran");
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    let move0 = s
        .world
        .level
        .locators
        .iter()
        .find(|l| l.name.eq_ignore_ascii_case("move0"))
        .map(|l| [l.transform[12], l.transform[13], l.transform[14]])
        .expect("move0 locator");
    let p = s.body().position;
    let d = ((p[0] - move0[0]).powi(2) + (p[2] - move0[2]).powi(2)).sqrt();
    assert!(d < 3.0, "avatar {p:?} not at move0 {move0:?} ({d} m)");
}

#[test]
fn skipping_mid_dialog_clears_active_and_delayed_intro_dialogs() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    s.headless_dialogs = Some([1280.0, 720.0]);
    let mut found = false;
    for _ in 0..1800 {
        s.step(DT);
        if s.world.cutscene.active
            && s.world
                .dialogs
                .as_ref()
                .is_some_and(|d| d.active().iter().any(Option::is_some))
        {
            found = true;
            break;
        }
    }
    assert!(found, "intro dialog must be active before skipping");
    s.skip_cutscene();
    s.skip_cutscene();
    for _ in 0..900 {
        s.step(DT);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert!(!s.world.cutscene.active);
    assert!(
        s.world
            .dialogs
            .as_ref()
            .unwrap()
            .active()
            .iter()
            .all(Option::is_none),
        "dialog survived or restarted after skip"
    );
    assert!(!s
        .world
        .dialog_text
        .as_ref()
        .unwrap()
        .cutscene
        .widget
        .state
        .visible());
}

#[test]
fn mountaindale_vlad_dialog_finishes_and_allows_next_scene() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, "Mountaindale.lvl", "Wizard").unwrap();
    s.change_scene_named("md_s2", "start").unwrap();
    s.headless_dialogs = Some([1280.0, 720.0]);
    s.run_trigger("Vlad appears", true).unwrap();
    for _ in 0..5400 {
        s.step(DT);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert!(
        !s.world.cutscene.active,
        "{:?} {:?}",
        s.world.dialog_failure,
        s.world.dialogs.as_ref().map(|d| d.active())
    );
    assert!(s.run_trigger("NextScene", true).unwrap());
    for _ in 0..180 {
        s.step(DT);
    }
    assert_eq!(s.scene_name(), "md_s3");
}
