use magicka_runtime::{entity_command::EntityCommandHost, play_session::PlaySession};
use std::path::Path;

fn speaker_session() -> Option<(PlaySession, u16)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Mountaindale.lvl", "Wizard").unwrap();
    s.change_scene_named("md_s3", "start").unwrap();
    for _ in 0..180 {
        s.step(1.0 / 60.0);
    }
    s.disable_scripts();
    s.headless_dialogs = Some([1280.0, 720.0]);
    let speaker = s
        .world
        .npcs
        .values()
        .find(|n| n.definition.id.eq_ignore_ascii_case("villager_male06_axe"))
        .expect("scripted woodcutter")
        .handle;
    assert!(s.world.interact_character(speaker, 0).unwrap());
    assert!(s
        .world
        .dialog_text
        .as_ref()
        .unwrap()
        .boxes
        .iter()
        .any(|b| b.state.visible()));
    Some((s, speaker))
}

fn assert_cleans_up(mut s: PlaySession) {
    for _ in 0..120 {
        s.step(1.0 / 60.0);
    }
    assert!(
        s.world.dialog_failure.is_none(),
        "{:?}",
        s.world.dialog_failure
    );
    assert!(s
        .world
        .dialogs
        .as_ref()
        .unwrap()
        .active()
        .iter()
        .all(Option::is_none));
    assert!(s
        .world
        .dialog_text
        .as_ref()
        .unwrap()
        .boxes
        .iter()
        .all(|b| !b.state.visible()));
}

#[test]
fn dead_speaker_ends_non_cutscene_dialog() {
    let Some((mut s, speaker)) = speaker_session() else {
        return;
    };
    s.world.kill(speaker).unwrap();
    assert_cleans_up(s);
}

#[test]
fn removed_speaker_ends_non_cutscene_dialog() {
    let Some((mut s, speaker)) = speaker_session() else {
        return;
    };
    s.world.terminate(speaker).unwrap();
    assert_cleans_up(s);
}
