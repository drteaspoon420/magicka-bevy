use magicka_runtime::{
    avatar::PlayerInput,
    camera::multiply,
    pickups::{mouse_pickup, PickupBounds, PickupCandidate},
    play_session::PlaySession,
};
use magicka_sim::spray_pulse::Segment;

fn content() -> Option<std::path::PathBuf> {
    let path = magicka_content::retail_content_dir();
    path.is_dir().then_some(path)
}
fn ndc(s: &PlaySession, point: [f32; 3]) -> [f32; 2] {
    let vp = multiply(
        &s.world.camera.view(),
        &s.world.camera.projection(16.0 / 9.0),
    );
    let v = [point[0], point[1], point[2], 1.0];
    let p: [f32; 4] = std::array::from_fn(|i| (0..4).map(|j| v[j] * vp[j * 4 + i]).sum());
    [p[0] / p[3], p[1] / p[3]]
}
fn target_point(s: &PlaySession, handle: u16) -> [f32; 3] {
    let (path, transform, book) = if let Some(b) = s.world.book_pickups.get(&handle) {
        (
            std::path::PathBuf::from(magicka_runtime::magick_books::BOOK_MODEL),
            b.transform,
            true,
        )
    } else {
        let p = &s.world.item_pickups[&handle];
        let model = p.item.as_ref().unwrap().definition.model.as_ref().unwrap();
        (
            std::path::Path::new("Data/Items/Wizard").join(model.replace('\\', "/")),
            p.transform,
            false,
        )
    };
    magicka_runtime::play_content::pickup_model_bounds(content().as_ref().unwrap(), &path)
        .unwrap()
        .world_box(&transform, book)
        .center
}
fn session() -> Option<PlaySession> {
    let mut s = PlaySession::load(&content()?, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    Some(s)
}
fn place(s: &PlaySession, distance: f32) -> [f32; 16] {
    let mut t = magicka_content::skinned::IDENTITY;
    t[12] = s.body().position[0] - distance;
    t[13] = s.body().position[1];
    t[14] = s.body().position[2];
    t
}

#[test]
fn cursor_query_hits_box_preserves_order_and_rejects_dead_and_vertical_misses() {
    let bounds = PickupBounds {
        min: [-0.5; 3],
        max: [0.5; 3],
    };
    let b = bounds.world_box(&magicka_content::skinned::IDENTITY, false);
    let candidate = PickupCandidate {
        handle: 1,
        position: [0.0; 3],
        radius: 0.0,
        pickable: true,
        dead: false,
    };
    let ray = Segment {
        origin: [0.0, 10.0, 0.0],
        delta: [0.0, -20.0, 0.0],
    };
    assert_eq!(mouse_pickup([(candidate, b)], ray, [0.0; 3]), Some(1));
    assert_eq!(mouse_pickup([(candidate, b)], ray, [0.0, 7.0, 0.0]), None);
    assert_eq!(
        mouse_pickup(
            [(
                PickupCandidate {
                    dead: true,
                    ..candidate
                },
                b
            )],
            ray,
            [0.0; 3]
        ),
        None
    );
    assert_eq!(
        mouse_pickup(
            [(candidate, b)],
            Segment {
                origin: [2.0, 10.0, 0.0],
                ..ray
            },
            [0.0; 3]
        ),
        None
    );
    assert_eq!(
        mouse_pickup(
            [
                (candidate, b),
                (
                    PickupCandidate {
                        handle: 2,
                        ..candidate
                    },
                    b
                )
            ],
            ray,
            [0.0; 3]
        ),
        Some(1)
    );
    assert_eq!(
        bounds
            .world_box(&magicka_content::skinned::IDENTITY, true)
            .side_lengths,
        [1.0, 2.0, 1.0]
    );
}
#[test]
fn normal_left_click_collects_book_and_hold_does_not_collect_another() {
    let Some(mut s) = session() else {
        return;
    };
    let h = s.spawn_magick_book(3, place(&s, 1.5)).unwrap();
    let input = PlayerInput {
        cursor_ndc: Some(ndc(&s, target_point(&s, h))),
        move_down: true,
        ..Default::default()
    };
    s.handle_input(
        PlayerInput {
            move_down: false,
            ..input
        },
        16.0 / 9.0,
    );
    assert_eq!(s.hovered_pickup, Some(h));
    s.handle_input(input, 16.0 / 9.0);
    assert!(s.world.book_pickups[&h].dead);
    assert_eq!(s.unlocked_magicks.to_array::<4>(), [8; 4]);
    let h2 = s.spawn_magick_book(5, place(&s, 1.5)).unwrap();
    s.handle_input(input, 16.0 / 9.0);
    assert!(!s.world.book_pickups[&h2].dead);
    s.handle_input(
        PlayerInput {
            move_down: false,
            ..input
        },
        16.0 / 9.0,
    );
    s.handle_input(input, 16.0 / 9.0);
    assert!(s.world.book_pickups[&h2].dead);
    assert_eq!(s.unlocked_magicks.to_array::<4>(), [8 | 32; 4]);
}
#[test]
fn shift_cutscene_and_input_lock_do_not_collect_then_normal_click_swaps_item() {
    let Some(mut s) = session() else {
        return;
    };
    let h = s.spawn_item_pickup("weapon_kris", place(&s, 1.5)).unwrap();
    let input = PlayerInput {
        cursor_ndc: Some(ndc(&s, target_point(&s, h))),
        move_down: true,
        ..Default::default()
    };
    s.handle_input(
        PlayerInput {
            shift_down: true,
            ..input
        },
        16.0 / 9.0,
    );
    assert_eq!(
        s.world.item_pickups[&h]
            .item
            .as_ref()
            .unwrap()
            .definition
            .name,
        "weapon_kris"
    );
    s.handle_input(PlayerInput::default(), 16.0 / 9.0);
    s.world.flags.cutscene = true;
    s.handle_input(input, 16.0 / 9.0);
    assert_eq!(
        s.world.item_pickups[&h]
            .item
            .as_ref()
            .unwrap()
            .definition
            .name,
        "weapon_kris"
    );
    s.world.flags.cutscene = false;
    s.handle_input(PlayerInput::default(), 16.0 / 9.0);
    s.world.flags.input_limited = true;
    s.handle_input(input, 16.0 / 9.0);
    assert_eq!(
        s.world.item_pickups[&h]
            .item
            .as_ref()
            .unwrap()
            .definition
            .name,
        "weapon_kris"
    );
    s.world.flags.input_limited = false;
    s.handle_input(PlayerInput::default(), 16.0 / 9.0);
    let handle = s.avatar().handle;
    let hp = s.avatar().character.combat.health.hit_points;
    s.handle_input(input, 16.0 / 9.0);
    assert_eq!(
        s.avatar()
            .items
            .iter()
            .find(|(slot, _)| *slot == 0)
            .unwrap()
            .1
            .definition
            .name,
        "weapon_kris"
    );
    assert_eq!(s.avatar().handle, handle);
    assert_eq!(s.avatar().character.combat.health.hit_points, hp);
}
#[test]
fn hold_click_walks_to_distant_book_using_locked_target() {
    let Some(mut s) = session() else {
        return;
    };
    let h = s.spawn_magick_book(3, place(&s, 5.0)).unwrap();
    assert!(!s.pickup_in_range(h));
    let input = PlayerInput {
        cursor_ndc: Some(ndc(&s, target_point(&s, h))),
        move_down: true,
        ..Default::default()
    };
    s.handle_input(input, 16.0 / 9.0);
    assert!(!s.world.book_pickups[&h].dead);
    for _ in 0..240 {
        s.step(1.0 / 60.0);
        // Dragging away keeps the originally pressed target while held.
        s.handle_input(
            PlayerInput {
                cursor_ndc: Some([0.9, 0.9]),
                ..input
            },
            16.0 / 9.0,
        );
        if s.world.book_pickups[&h].dead {
            break;
        }
    }
    assert!(
        s.world.book_pickups[&h].dead,
        "player {:?}",
        s.body().position
    );
    assert_eq!(s.unlocked_magicks.to_array::<4>(), [8; 4]);
}

#[test]
fn removed_or_reused_target_is_not_collected_by_an_old_held_click() {
    let Some(mut s) = session() else {
        return;
    };
    let h = s.spawn_magick_book(3, place(&s, 5.0)).unwrap();
    let input = PlayerInput {
        cursor_ndc: Some(ndc(&s, target_point(&s, h))),
        move_down: true,
        ..Default::default()
    };
    s.handle_input(input, 16.0 / 9.0);
    s.world
        .entities
        .iter_mut()
        .find(|e| e.handle == h)
        .unwrap()
        .removed = true;
    s.step(1.0 / 60.0);
    assert!(s.world.book_pickups[&h].dead);
    let reused = s.spawn_magick_book(5, place(&s, 1.5)).unwrap();
    assert_eq!(reused, h);
    let input = PlayerInput {
        cursor_ndc: Some(ndc(&s, target_point(&s, h))),
        ..input
    };
    s.handle_input(input, 16.0 / 9.0);
    assert!(!s.world.book_pickups[&h].dead);
    assert_eq!(s.unlocked_magicks.to_array::<4>(), [0; 4]);
    s.handle_input(
        PlayerInput {
            move_down: false,
            ..input
        },
        16.0 / 9.0,
    );
    s.handle_input(input, 16.0 / 9.0);
    assert!(s.world.book_pickups[&h].dead);
    assert_eq!(s.unlocked_magicks.to_array::<4>(), [32; 4]);
}
