//! Mouse and reach interaction with OnInteract interactables
//! (research/interaction.md).
use magicka_runtime::avatar::PlayerInput;
use magicka_runtime::play_session::{Cursor, PlaySession};
use magicka_runtime::scene_flow::SpawnPoint;
use magicka_runtime::scene_loading::custom_hash;
use std::path::{Path, PathBuf};

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    root.is_dir().then_some(root)
}

fn frame(s: &mut PlaySession, input: PlayerInput) {
    s.step(1.0 / 60.0);
    s.handle_input(input, 16.0 / 9.0);
    s.events.clear();
}

fn cursor_at(s: &PlaySession, point: [f32; 3]) -> [f32; 2] {
    let vp = magicka_runtime::camera::multiply(
        &s.world.camera.view(),
        &s.world.camera.projection(16.0 / 9.0),
    );
    let v = [point[0], point[1], point[2], 1.0];
    let p: [f32; 4] = std::array::from_fn(|i| (0..4).map(|j| v[j] * vp[j * 4 + i]).sum());
    [p[0] / p[3], p[1] / p[3]]
}

fn havindr_s2(root: &Path) -> PlaySession {
    let mut s = PlaySession::load(root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene(SpawnPoint {
        scene: custom_hash("havindr_s2"),
        locations: std::array::from_fn(|i| custom_hash(&format!("start{i}"))),
        spawn_players: true,
    })
    .unwrap();
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    // The scene opens with a cutscene; interaction waits for it to end.
    for _ in 0..(120 * 60) {
        if !s.world.flags.cutscene && !s.input_limited() {
            break;
        }
        s.skip_cutscene();
        frame(&mut s, PlayerInput::default());
    }
    assert!(
        !s.world.flags.cutscene && !s.input_limited(),
        "cutscene {} lock {} limited {}",
        s.world.flags.cutscene,
        s.world.camera.lock_input,
        s.world.flags.input_limited
    );
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    s
}

fn dialog_active(s: &PlaySession) -> bool {
    s.world
        .dialogs
        .as_ref()
        .is_some_and(|m| m.can_advance(Some(0)))
}

#[test]
fn castle_npc_spawn_dialog_can_be_reassigned_and_clicked() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s4", "start").unwrap();
    s.headless_dialogs = Some([1280., 720.]);
    s.step(1. / 60.);
    let id = custom_hash("#wizn4");
    let npc = s
        .world
        .entities
        .iter()
        .find(|e| e.unique_id == id)
        .unwrap()
        .clone();
    assert_eq!(
        s.world.character_dialogs[&npc.handle],
        custom_hash("target")
    );
    assert!(s.run_trigger("No more targets", true).unwrap());
    s.step(1. / 60.);
    assert_eq!(
        s.world.character_dialogs[&npc.handle],
        custom_hash("notarget")
    );
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    // Isolate the mouse interaction from the courtyard's scripted opening walk.
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    s.world.camera.lock_input = false;
    let body = s.avatar().body;
    let orientation = s.world.physics.bodies[body].orientation;
    s.world.physics.bodies[body].move_to(
        [npc.position[0] + 1., npc.position[1], npc.position[2]],
        orientation,
    );
    let avatar = s.world.players[0].as_mut().unwrap();
    avatar.set_events(&mut s.world.physics.bodies[body], None);
    let cursor = cursor_at(&s, npc.position);
    let hover = PlayerInput {
        cursor_ndc: Some(cursor),
        ..Default::default()
    };
    s.handle_input(hover, 16. / 9.);
    assert_eq!(s.hovered_character, Some(npc.handle));
    assert_eq!(s.cursor(false), (Cursor::Talk, true));
    s.handle_input(
        PlayerInput {
            move_down: true,
            ..hover
        },
        16. / 9.,
    );
    assert!(s
        .world
        .dialogs
        .as_ref()
        .unwrap()
        .dialog_active(custom_hash("notarget")));
    let slot = s
        .world
        .dialogs
        .as_ref()
        .unwrap()
        .active()
        .iter()
        .flatten()
        .next()
        .unwrap();
    let interaction = s
        .world
        .dialogs
        .as_ref()
        .unwrap()
        .dialog(slot.dialog)
        .unwrap()
        .interaction(slot.interaction)
        .unwrap();
    assert_eq!(interaction.controller, Some(0));
    assert!(
        s.world.dialog_failure.is_none(),
        "{:?}",
        s.world.dialog_failure
    );
    s.world
        .entities
        .iter_mut()
        .find(|e| e.handle == npc.handle)
        .unwrap()
        .dead = true;
    assert!(!s
        .world
        .talk_character_available(npc.handle, s.avatar().faction));
}

/// Hovering poi_bed1 shows the Examine cursor (dim out of reach); clicking
/// walks the Wizard there and Interact starts the "bed" dialog.
#[test]
fn clicking_an_examine_poi_walks_there_and_starts_its_dialog() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = havindr_s2(&root);
    let id = custom_hash("poi_bed1");
    let (poi, radius) = s.world.interactable_locator(id).expect("poi_bed1 locator");
    assert!(radius > 0.0);
    // Put the Wizard a few metres away so a click must walk.
    let body = s.avatar().body;
    let o = s.world.physics.bodies[body].orientation;
    let start = [poi[0] + radius + 3.0, s.body().position[1], poi[2]];
    s.world.physics.bodies[body].move_to(start, o);
    for _ in 0..5 {
        frame(&mut s, PlayerInput::default());
    }
    let hover = PlayerInput {
        cursor_ndc: Some(cursor_at(&s, poi)),
        ..Default::default()
    };
    frame(&mut s, hover);
    assert_eq!(s.hovered_interactable, Some(id));
    assert_eq!(s.cursor(false), (Cursor::Examine, false));
    let mut started = false;
    for _ in 0..240 {
        let input = PlayerInput {
            cursor_ndc: Some(cursor_at(&s, poi)),
            move_down: true,
            ..Default::default()
        };
        frame(&mut s, input);
        if dialog_active(&s) {
            started = true;
            break;
        }
    }
    assert!(started, "walking to the bed started its dialog");
}

/// Disabled interactables (disablePoi / enabled="false") are not offered.
#[test]
fn disabled_interactable_is_not_found() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = havindr_s2(&root);
    let id = custom_hash("poi_bed1");
    let (poi, _) = s.world.interactable_locator(id).unwrap();
    s.world.flags.interactables_disabled.insert(id);
    let body = s.avatar().body;
    let o = s.world.physics.bodies[body].orientation;
    let y = s.body().position[1];
    s.world.physics.bodies[body].move_to([poi[0], y, poi[2]], o);
    frame(&mut s, PlayerInput::default());
    assert!(!s.interact());
    assert!(!dialog_active(&s));
}
