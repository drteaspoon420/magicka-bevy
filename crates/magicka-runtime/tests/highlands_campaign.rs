use magicka_runtime::play_session::PlaySession;
use std::path::Path;

#[test]
fn highlands_outro_reaches_second_scene_without_skipping() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Highlands.lvl", "Wizard").unwrap();
    s.headless_dialogs = Some([1280.0, 720.0]);
    for _ in 0..180 {
        s.step(1.0 / 60.0);
    }
    s.skip_cutscene();
    s.skip_cutscene();
    for _ in 0..300 {
        s.step(1.0 / 60.0);
    }
    assert!(!s.world.cutscene.active, "intro skip did not end");
    // Ordinary traversal consumes the intro Vlad removal trigger before
    // entering the outro. Starting the outro at the intro spawn instead
    // makes that old area trigger kill the newly spawned Vlad.
    assert!(s.run_trigger("Highlands s1 Remove Vlad", true).unwrap());
    for _ in 0..180 {
        s.step(1.0 / 60.0);
    }
    let locator = magicka_runtime::spawn_phys::SpawnPhysHost::locator(
        &mut s.world,
        magicka_runtime::scene_loading::custom_hash("spawn_vlad1"),
    )
    .unwrap();
    let player_body = s.avatar().body;
    let body = &mut s.world.physics.bodies[player_body];
    body.move_to(
        [
            locator[12] + 1.0,
            locator[13] + body.length * 0.5 + body.radius,
            locator[14] + 1.0,
        ],
        magicka_sim::character_physics::Orientation::IDENTITY,
    );
    assert!(s.run_trigger("Vlad and the outro", true).unwrap());
    for _ in 0..7200 {
        s.step(1.0 / 60.0);
        if s.scene_name() == "hl_s2" {
            break;
        }
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert!(
        s.world.dialog_failure.is_none(),
        "{:?}",
        s.world.dialog_failure
    );
    if s.scene_name() != "hl_s2" {
        for n in s
            .world
            .npcs
            .values()
            .filter(|n| n.definition.id.eq_ignore_ascii_case("vlad"))
        {
            eprintln!(
                "Vlad {}: position {:?}; events {:?}",
                n.handle,
                s.world.physics.bodies[n.body].position,
                n.ai.events()
            );
        }
        for name in ["move_vlad0", "move_vlad1", "move_vlad2"] {
            eprintln!(
                "{name}: {:?}",
                magicka_runtime::spawn_phys::SpawnPhysHost::locator(
                    &mut s.world,
                    magicka_runtime::scene_loading::custom_hash(name)
                )
            );
        }
    }
    assert_eq!(
        s.scene_name(),
        "hl_s2",
        "normal outro stalled; log {:?}; dialogs {:?}",
        s.world.log,
        s.world.dialogs.as_ref().map(|d| d.active())
    );
}

#[test]
fn airship_crash_reaches_next_scene_with_player_alive() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Highlands.lvl", "Wizard").unwrap();
    s.headless_dialogs = Some([1280.0, 720.0]);
    s.change_scene_named("hl_s2", "start").unwrap();
    for _ in 0..120 {
        s.step(1. / 60.);
    }
    eprintln!("airship initial {:?}", s.body().position);
    assert!(!s.avatar().dead());
    assert!(s.run_trigger("EndScene", true).unwrap());
    for i in 0..1500 {
        s.step(1. / 60.);
        assert!(
            !s.avatar().dead(),
            "died on frame {i} in {} at {:?}; events {:?}",
            s.scene_name(),
            s.body().position,
            s.avatar().events
        );
        if s.scene_name() == "hl_s3" {
            return;
        }
    }
    panic!("airship did not change scene: {:?}", s.script_error);
}
