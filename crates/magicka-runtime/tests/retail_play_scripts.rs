//! Live scene scripts on every retail start scene (skips without content):
//! 45 s with a click every 2 s must run without script or dialog failures,
//! and every intro that limits input must hand control back. Intros wait on
//! NPC event animations (Highlands) and dialog TTLs, so this exercises NPC
//! agents and the headless dialog render phase. Actions not reconstructed
//! yet are skipped and reported, never silent.
use magicka_runtime::{
    avatar::PlayerInput,
    play_session::{PlaySession, SessionEvent},
};
use std::path::PathBuf;

#[test]
fn retail_start_scene_scripts_run_with_dialogs() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    let mut levels: Vec<String> = std::fs::read_dir(root.join("Levels"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.to_ascii_lowercase().ends_with(".lvl"))
        .collect();
    levels.sort();
    let only: Option<Vec<String>> = std::env::var("MAGICKA_PLAY_SCRIPT_LEVELS").ok().map(|v| {
        v.split(',')
            .map(|n| n.trim().to_ascii_lowercase())
            .collect()
    });
    if let Some(only) = &only {
        levels.retain(|l| only.contains(&l.to_ascii_lowercase()));
        assert!(!levels.is_empty(), "no level matched {only:?}");
    }
    let (mut with_dialog, mut moved_by_script, mut live_npcs) = (0, 0, 0);
    for level in &levels {
        let mut s = PlaySession::load(&root, level, "Wizard").unwrap();
        s.headless_dialogs = Some([1280.0, 720.0]);
        let start = s.body().position;
        let (mut skipped, mut dialog_frames, mut limited) = (Vec::new(), 0, 0);
        let mut released = None;
        // (frame, boss fight running, input limited) at the first
        // frame the avatar is dead, and the first frame of a boss fight.
        let mut death = None;
        let mut fight_started = None;
        for frame in 0..2700 {
            s.handle_input(
                PlayerInput {
                    cursor_ndc: Some([0.0, 0.0]),
                    move_down: frame % 120 == 0,
                    ..PlayerInput::default()
                },
                16.0 / 9.0,
            );
            s.step(1.0 / 60.0);
            limited += usize::from(s.input_limited());
            if !s.input_limited() && released.is_none() {
                released = Some(frame);
            }
            if s.input_limited() {
                released = None;
            }
            if s.world.boss_fight.running && fight_started.is_none() {
                fight_started = Some(frame);
            }
            if s.dead() && death.is_none() {
                death = Some((frame, s.world.boss_fight.running, s.input_limited()));
            }
            // Cutscene text is drawn only while DrawCutsceneText is set.
            if s.world.dialog_text.as_ref().is_some_and(|t| {
                t.boxes.iter().any(|b| b.state.visible())
                    || (t.cutscene.widget.state.visible() && t.cutscene_submitted[0])
            }) {
                dialog_frames += 1;
            }
            for event in s.events.drain(..) {
                match event {
                    SessionEvent::ScriptError(e) => panic!("{level}: script error {e}"),
                    SessionEvent::ScriptSkipped(name) => skipped.push(name),
                    _ => {}
                }
            }
        }
        assert!(s.script_error.is_none(), "{level}: {:?}", s.script_error);
        assert!(
            s.world.dialog_failure.is_none(),
            "{level}: {:?}",
            s.world.dialog_failure
        );
        if let Some((frame, in_boss_fight, limited)) = death {
            assert!(
                in_boss_fight && !limited,
                "{level}: avatar died at {} s (boss fight {in_boss_fight}, input limited {limited})",
                frame as f32 / 60.0
            );
        }
        assert!(
            released.is_some() || death.is_some(),
            "{level}: input never handed back"
        );
        let end = s.body().position;
        let moved = ((end[0] - start[0]).powi(2) + (end[2] - start[2]).powi(2)).sqrt();
        with_dialog += usize::from(dialog_frames > 0);
        moved_by_script += usize::from(moved > 1.0);
        let npcs = s.world.npcs.len();
        live_npcs += npcs;
        eprintln!(
            "{level}: dialog frames {dialog_frames}, input-limited frames {limited}, released at {:?} s, boss fight at {:?} s, died at {:?} s, live NPCs {npcs}, scripted move {moved:.1} m, skipped {skipped:?}",
            released.map(|f| f as f32 / 60.0),
            fight_started.map(|f| f as f32 / 60.0),
            death.map(|(f, _, _)| f as f32 / 60.0)
        );
    }
    eprintln!(
        "{} levels, {with_dialog} showed dialogs, {moved_by_script} moved the avatar",
        levels.len()
    );
    if only.is_none() {
        assert!(with_dialog >= 7 && moved_by_script >= 6 && live_npcs >= 50);
    }
}
