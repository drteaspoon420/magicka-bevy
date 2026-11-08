//! PlayState.State fields beyond scripts: camera, music, dialog collection,
//! permanent pickables, avatar-section flags and the "#add_checkpoint"
//! notice survive Restart(Checkpoint) and the disk save
//! (research/checkpoints.md, 2026-10-30).
use magicka_runtime::audio::AudioRequest;
use magicka_runtime::camera::{CameraBehaviour, CameraBox};
use magicka_runtime::magick_books::{SpawnMagickAction, SpawnMagickHost};
use magicka_runtime::play_session::{PlaySession, SessionEvent};
use magicka_runtime::scene_command::CameraInterpolation;
use magicka_runtime::scene_loading::custom_hash;
use magicka_runtime::scene_world::saved_entities::{PickableKind, PickableSnapshot};
use std::path::{Path, PathBuf};

const DT: f32 = 1.0 / 60.0;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

fn checkpoint_action() -> magicka_runtime::scene_flow::SceneFlowAction {
    use magicka_runtime::scene_script::{ActionDefinition, SceneScriptLayout};
    let layout = SceneScriptLayout::read(
        br#"<Scene><Trigger id="checkpoint"><Then><TriggerCheckpoint spawnpoint="start" spawnfairy="false" savetodisk="false"/></Then></Trigger></Scene>"#,
        false,
    ).unwrap();
    match layout.actions.into_iter().next().unwrap() {
        ActionDefinition::Flow(action) => *action,
        other => panic!("expected checkpoint, got {other:?}"),
    }
}

/// Havindr s3 with scripts disabled, past the tutorial/cutscene save gates.
fn session(root: &Path) -> PlaySession {
    let mut s = PlaySession::load(root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene_named("havindr_s3", "start").unwrap();
    for _ in 0..5 {
        s.step(DT);
    }
    s.disable_scripts();
    s
}

/// TriggerCheckpoint.Execute, then the step that snapshots the state.
fn capture(s: &mut PlaySession) -> [[f32; 16]; 4] {
    // The scene's own StartOfLevel checkpoint already queued its notice.
    s.events.clear();
    let mut action = checkpoint_action();
    let matrices = action.initialize(&mut s.world).unwrap().unwrap();
    action.execute(Some(&matrices), &mut s.world).unwrap();
    let generation = s.checkpoint_generation;
    s.step(DT);
    assert_eq!(
        s.checkpoint_generation,
        generation + 1,
        "checkpoint captured"
    );
    matrices
}

fn translation(at: [f32; 3]) -> [f32; 16] {
    let mut m = magicka_content::skinned::IDENTITY;
    m[12..15].copy_from_slice(&at);
    m
}

#[test]
fn camera_music_sections_and_notice_survive_a_checkpoint_restart() {
    let Some(root) = content() else { return };
    let mut s = session(&root);
    // Script-driven camera state: a lock-on box, a move-to and a zoom.
    let camera_box = CameraBox {
        corner: [1.0, 2.0, 3.0],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        sides: [10.0, 10.0, 10.0],
    };
    s.world.camera.lock_on(camera_box, 0.5);
    s.world.camera.move_to([4.0, 5.0, 6.0], 0.7);
    s.world.camera.set_magnification(1.5);
    s.world.camera.lock_input = true;
    s.world.flags.music = Some((2, 4242));
    s.world.flags.music_focus_value = Some(0.5);
    s.default_avatar_section = true;
    capture(&mut s);
    let notice = s
        .events
        .iter()
        .find_map(|e| match e {
            SessionEvent::CheckpointReached(n) => Some(n.clone()),
            _ => None,
        })
        .expect("TriggerCheckpoint requested its text box");
    assert_eq!(notice.font, "Maiandra18");
    assert_eq!((notice.position, notice.anchor), ([0.0, 0.0], [0.0, 1.0]));
    assert!(notice.force_on_screen);
    assert_eq!(notice.ttl, 2.0);
    assert!(
        !notice.text.is_empty() && !notice.text.starts_with("NOT FOUND"),
        "{:?}",
        notice.text
    );
    assert_eq!(
        s.events
            .iter()
            .filter(|e| matches!(e, SessionEvent::CheckpointReached(_)))
            .count(),
        1
    );
    let cp = s.checkpoint.clone().unwrap();
    assert_eq!(cp.camera().behaviour, CameraBehaviour::MoveToTarget);
    assert_eq!(cp.camera().target, [4.0, 5.0, 6.0]);
    assert_eq!(cp.camera().camera_box, Some(camera_box));
    assert_eq!(cp.music(), (Some((2, 4242)), Some(0.5)));
    assert_eq!(cp.sections(), (false, true));
    // Diverge after the capture.
    s.world.camera.release(0.2);
    s.world.camera.set_position([100.0, 0.0, 100.0], false);
    s.world.camera.set_magnification(0.8);
    s.world.flags.music = None;
    s.world.flags.music_focus_value = None;
    s.default_avatar_section = false;
    s.step(DT);
    s.audio.clear();
    assert!(s.restart_at_checkpoint().unwrap());
    // Release(0.001) then State.ApplyState.
    assert_eq!(s.world.camera.state(), *cp.camera());
    assert_eq!(
        s.world.camera.interpolation,
        CameraInterpolation::Interpolated
    );
    assert!(!s.world.camera.lock_input);
    assert_eq!(s.world.flags.music, Some((2, 4242)));
    assert_eq!(s.world.flags.music_focus_value, Some(0.5));
    let music: Vec<_> = s
        .audio
        .iter()
        .filter_map(|(r, _)| match r {
            AudioRequest::StopMusic => Some(None),
            AudioRequest::Music { bank, cue } => Some(Some((*bank, *cue))),
            _ => None,
        })
        .collect();
    assert_eq!(
        music,
        vec![None, Some((2, 4242))],
        "StopMusic then PlayMusic"
    );
    assert!(!s.unequip_weapons_section && s.default_avatar_section);
    assert_eq!(s.checkpoint.as_ref().unwrap().sections(), (false, true));
    // The camera keeps running from the restored state.
    for _ in 0..3 {
        s.step(DT);
    }
    assert_eq!(s.world.camera.behaviour, CameraBehaviour::MoveToTarget);
}

#[test]
fn permanent_pickables_return_and_timed_ones_do_not() {
    let Some(root) = content() else { return };
    let mut s = session(&root);
    // The scene's own scripted weapons are permanent pickables too.
    let baseline = s.world.checkpoint_pickables();
    assert!(baseline
        .iter()
        .all(|p| matches!(p.kind, PickableKind::Item { .. }) && p.immovable));
    let home = s.body().position;
    let near = |dx: f32, dz: f32| translation([home[0] + dx, home[1] + 0.5, home[2] + dz]);
    let permanent_item = s.spawn_item_pickup("weapon_stung", near(2.0, 0.0)).unwrap();
    let timed_item = s
        .spawn_item_pickup("staff_of_fire", near(-2.0, 0.0))
        .unwrap();
    s.world.item_pickups.get_mut(&timed_item).unwrap().despawn = 3.0;
    let permanent_book = s
        .world
        .spawn_book(
            &SpawnMagickAction {
                delay: 0.0,
                area: 0,
                magick: 1,
                immovable: true,
                timeout: 0.0,
                unique_id: 77,
                on_pickup: 0,
            },
            near(0.0, 2.0),
        )
        .unwrap();
    s.world
        .spawn_book(
            &SpawnMagickAction {
                delay: 0.0,
                area: 0,
                magick: 2,
                immovable: true,
                timeout: 5.0,
                unique_id: 78,
                on_pickup: 0,
            },
            near(0.0, -2.0),
        )
        .unwrap();
    let item_entity = |s: &PlaySession, handle: u16| {
        s.world
            .entities
            .iter()
            .find(|e| e.handle == handle)
            .unwrap()
            .clone()
    };
    let item_transform = s.world.item_pickups[&permanent_item].transform;
    let book_transform = s.world.book_pickups[&permanent_book].transform;
    let item_id = item_entity(&s, permanent_item).unique_id;
    capture(&mut s);
    let cp = s.checkpoint.clone().unwrap();
    let mut expected = baseline.clone();
    expected.extend([
        PickableSnapshot {
            kind: PickableKind::Item {
                name: "weapon_stung".into(),
            },
            transform: item_transform,
            immovable: true,
            unique_id: item_id,
        },
        PickableSnapshot {
            kind: PickableKind::BookOfMagick { magick: 1 },
            transform: book_transform,
            immovable: true,
            unique_id: 77,
        },
    ]);
    let mut captured = cp.pickables().to_vec();
    let key = |p: &PickableSnapshot| format!("{:?}", p.kind);
    expected.sort_by_key(key);
    captured.sort_by_key(key);
    assert_eq!(captured, expected, "only permanent pickables are stored");
    assert!(s.restart_at_checkpoint().unwrap());
    let mut items: Vec<_> = s
        .world
        .entities
        .iter()
        .filter(|e| e.added && !e.removed && !e.dead)
        .filter_map(|e| {
            let p = s.world.item_pickups.get(&e.handle)?;
            Some((
                p.item.as_ref()?.definition.name.clone(),
                p.transform,
                p.immovable,
                e.unique_id,
            ))
        })
        .collect();
    let mut expected_items: Vec<_> = baseline
        .iter()
        .filter_map(|p| match &p.kind {
            PickableKind::Item { name } => {
                Some((name.clone(), p.transform, p.immovable, p.unique_id))
            }
            _ => None,
        })
        .collect();
    expected_items.push(("weapon_stung".to_string(), item_transform, true, item_id));
    items.sort_by(|a, b| a.0.cmp(&b.0));
    expected_items.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(items, expected_items, "the timed item does not return");
    let books: Vec<_> = s
        .world
        .book_pickups
        .values()
        .filter(|b| !b.dead)
        .map(|b| (b.magick, b.transform, b.immovable, b.timeout))
        .collect();
    assert_eq!(books, vec![(1, book_transform, true, 0.0)]);
    let (book_handle, _) = s.world.book_pickups.iter().find(|(_, b)| !b.dead).unwrap();
    assert_eq!(item_entity(&s, *book_handle).unique_id, 77);
    for _ in 0..5 {
        s.step(DT);
    }
    assert_eq!(s.world.book_pickups.values().filter(|b| !b.dead).count(), 1);
    assert_eq!(
        s.checkpoint.as_ref().unwrap().pickables().len(),
        baseline.len() + 2
    );
    // The restored scene captures the same set again (pickup physics may
    // renormalize a resting orientation by float noise).
    let mut again = s.world.checkpoint_pickables();
    again.sort_by_key(key);
    assert_eq!(again.len(), captured.len());
    for (a, c) in again.iter().zip(&captured) {
        assert_eq!(
            (&a.kind, a.immovable, a.unique_id),
            (&c.kind, c.immovable, c.unique_id)
        );
        for i in 0..16 {
            assert!(
                (a.transform[i] - c.transform[i]).abs() < 1e-3,
                "{a:?} vs {c:?}"
            );
        }
    }
}

#[test]
fn disk_save_round_trips_camera_music_pickables_and_sections() {
    let Some(root) = content() else { return };
    let mut s = session(&root);
    s.player_ids[0] = Some(magicka_runtime::saves::new_player_id().unwrap());
    let baseline = s.world.checkpoint_pickables().len();
    let home = s.body().position;
    s.spawn_item_pickup(
        "weapon_stung",
        translation([home[0] + 2.0, home[1] + 0.5, home[2]]),
    )
    .unwrap();
    s.world.camera.move_to([4.0, 5.0, 6.0], 0.7);
    s.world.camera.set_magnification(1.5);
    s.world.flags.music = Some((2, 4242));
    s.world.flags.music_focus_value = Some(0.25);
    s.unequip_weapons_section = false;
    s.default_avatar_section = true;
    capture(&mut s);
    let cp = s.checkpoint.clone().unwrap();
    assert_eq!(cp.pickables().len(), baseline + 1);
    let saved = s.disk_save().unwrap();
    let bytes = serde_json::to_vec(&saved).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let restored = PlaySession::load_disk_save(&root, &stored).unwrap();
    let rcp = restored.checkpoint.as_ref().unwrap();
    // MagickCamera.State.Write drops the box and followed entity.
    assert_eq!(*rcp.camera(), cp.camera().persisted());
    assert_eq!(rcp.music(), (Some((2, 4242)), Some(0.25)));
    assert_eq!(rcp.pickables(), cp.pickables());
    assert_eq!(rcp.sections(), (false, true));
    // The loaded scene itself carries the saved camera, music and pickable.
    assert_eq!(restored.world.camera.state(), cp.camera().persisted());
    assert_eq!(restored.world.flags.music, Some((2, 4242)));
    assert_eq!(restored.world.flags.music_focus_value, Some(0.25));
    assert_eq!(restored.world.checkpoint_pickables(), cp.pickables());
    assert!(restored
        .audio
        .iter()
        .any(|(r, _)| matches!(r, AudioRequest::Music { bank: 2, cue: 4242 })));
    let mut restored = restored;
    assert!(restored.restart_at_checkpoint().unwrap());
    assert_eq!(restored.world.camera.state(), cp.camera().persisted());
    assert_eq!(restored.world.checkpoint_pickables(), cp.pickables());
    assert!(!restored.unequip_weapons_section && restored.default_avatar_section);
    // Saves predating these keys still load: the fresh camera, no music
    // restoration and no pickables.
    let mut legacy = stored.clone();
    for key in ["camera", "pickables"] {
        legacy.as_object_mut().unwrap().remove(key);
        legacy["checkpoint"].as_object_mut().unwrap().remove(key);
    }
    for key in ["music", "music_focus", "unequipped", "default_avatar"] {
        legacy["checkpoint"].as_object_mut().unwrap().remove(key);
    }
    legacy["flags"]
        .as_object_mut()
        .unwrap()
        .remove("music_focus_value");
    let old = PlaySession::load_disk_save(&root, &legacy).unwrap();
    assert!(old.checkpoint.as_ref().unwrap().pickables().is_empty());
    assert_eq!(old.world.flags.music, Some((2, 4242)));
    assert_eq!(old.world.flags.music_focus_value, None);
    assert_eq!(old.checkpoint.as_ref().unwrap().sections(), (false, true));
    // Corrupt entries are rejected rather than partially applied.
    let mut bad = stored.clone();
    bad["checkpoint"]["camera"]["behaviour"] = serde_json::json!(9);
    assert!(PlaySession::load_disk_save(&root, &bad).is_err());
    let mut bad = stored.clone();
    bad["checkpoint"]["pickables"][0]["kind"] = serde_json::json!("npc");
    assert!(PlaySession::load_disk_save(&root, &bad).is_err());
    let mut bad = stored;
    bad["checkpoint"]["music_focus"] = serde_json::json!("loud");
    assert!(PlaySession::load_disk_save(&root, &bad).is_err());
}

/// DialogCollection.State.UpdateState captures cursors during playback.
#[test]
fn dialog_collection_state_is_captured_while_a_dialog_is_active() {
    use magicka_runtime::dialog_definition::{DialogDefinition, MessageDefinition};
    use magicka_runtime::dialog_manager::{
        ActiveInteraction, DialogManager, DialogManagerEffect, DialogManagerHost, DialogSink,
    };
    use magicka_runtime::interaction_playback::{InteractionHost, InteractionLocation};
    use magicka_runtime::message_playback::{MessagePlacement, MessagePlayback};
    struct Host;
    impl InteractionHost for Host {
        type Error = &'static str;
        fn advance_message(
            &mut self,
            _: usize,
            _: &MessageDefinition,
            _: &mut MessagePlayback,
            _: MessagePlacement,
        ) -> Result<bool, Self::Error> {
            Ok(false)
        }
        fn lock_input(&mut self, _: i32) -> Result<(), Self::Error> {
            Ok(())
        }
        fn unlock_input(&mut self, _: i32) -> Result<(), Self::Error> {
            Ok(())
        }
        fn end_message(&mut self, _: usize, _: &MessageDefinition) -> Result<(), Self::Error> {
            Ok(())
        }
    }
    impl DialogManagerHost for Host {
        fn bind(&mut self, _: DialogSink, _: ActiveInteraction) {}
        fn visible(&self, _: DialogSink) -> bool {
            false
        }
        fn animating(&self, _: DialogSink) -> bool {
            false
        }
        fn manager_effect(&mut self, _: DialogManagerEffect) -> Result<(), Self::Error> {
            Ok(())
        }
    }
    let xml = br#"<Dialogs><Dialog id="talk" repeatFrom="-1"><interact><message><text>#A</text></message><message><text>#B</text></message></interact><interact><message><text>#C</text></message></interact></Dialog></Dialogs>"#;
    let id = custom_hash("talk");
    let mut host = Host;
    let mut live = DialogManager::new(DialogDefinition::read_collection(&[xml]).unwrap()).unwrap();
    let idle = live.checkpoint_boundary();
    assert_eq!(idle, live.disk_boundary().unwrap());
    live.start(
        id,
        InteractionLocation::Screen([0.0, 0.0]),
        None,
        false,
        &mut host,
    )
    .unwrap();
    assert!(live.dialog_active(id));
    assert!(live.disk_boundary().is_err(), "disk saves defer");
    let active = live.checkpoint_boundary();
    assert_ne!(active, idle, "the advanced cursor is captured");
    assert_eq!(active[0][1], serde_json::json!(1), "mNext");
    // Restart: a reset manager receives the captured collection state.
    let mut restored =
        DialogManager::new(DialogDefinition::read_collection(&[xml]).unwrap()).unwrap();
    restored.restore_disk_boundary(&active).unwrap();
    assert!(!restored.dialog_active(id));
    assert_eq!(restored.disk_boundary().unwrap(), active);
    assert!(!restored.is_done(id, 0).unwrap());
}
