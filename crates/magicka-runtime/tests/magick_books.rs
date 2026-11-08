use magicka_runtime::magick_books::*;
fn book(timeout: f32) -> Book {
    Book {
        magick: 3,
        transform: magicka_content::skinned::IDENTITY,
        immovable: true,
        body_enabled: false,
        velocity: [0.0; 3],
        timeout,
        resting_timer: 1.0,
        on_pickup: 0,
        dead: false,
        generation: 0,
    }
}
#[test]
fn timeout_effects_and_expiry_follow_native_boundaries() {
    let mut b = book(4.5);
    assert!(!b.permanent());
    assert_eq!(
        b.update(0.5, false, false, false),
        BookUpdate {
            start_timeout: true,
            disappear: false
        }
    );
    assert_eq!(b.timeout, 4.0);
    assert_eq!(b.update(0.5, false, false, true), BookUpdate::default());
    assert!(!b.resting()); // zero is not resting, it must be strictly negative
    b.update(0.1, false, false, true);
    assert!(b.resting());
    b.update(0.1, true, false, true);
    assert!(!b.resting());
    assert_eq!(b.resting_timer, 1.0);
    let mut b = book(0.1);
    assert_eq!(
        b.update(0.2, false, false, false),
        BookUpdate {
            start_timeout: false,
            disappear: true
        }
    );
    assert!(b.dead);
    assert_eq!(b.timeout, 0.0);
    assert!(!b.permanent());
    assert_eq!(b.update(1.0, false, false, false), BookUpdate::default());
    let mut b = book(0.1);
    assert!(b.update(0.2, false, true, false).disappear);
    assert!(!b.dead);
    assert!(!b.update(0.2, false, true, false).disappear);
}
#[test]
fn indefinite_books_do_not_expire_and_decoder_matches_xml() {
    for timeout in [0.0, -1.0] {
        let mut b = book(timeout);
        assert!(b.permanent());
        assert_eq!(b.update(100.0, false, false, false), BookUpdate::default());
        assert!(!b.dead);
    }
    let a = SpawnMagickAction::read([
        ("MAGICK", "HaStE"),
        ("Area", "START"),
        ("TimeOut", "5"),
        ("Immovable", "false"),
    ])
    .unwrap();
    assert_eq!(a.magick, 3);
    assert_eq!(a.area, magicka_runtime::scene_loading::custom_hash("start"));
    assert_eq!(a.timeout, 5.0);
    assert!(!a.immovable);
    assert_eq!(magick_id("3"), Some(3));
    assert_eq!(magick_id("invalid"), None);
    assert!(SpawnMagickAction::read([("magick", "35")]).is_err());
}
fn content() -> Option<std::path::PathBuf> {
    let p = magicka_content::retail_content_dir();
    p.is_dir().then_some(p)
}
#[test]
fn dungeon_script_book_is_spawned_and_consumption_unlocks_without_swapping() {
    use magicka_runtime::play_session::PlaySession;
    let Some(content) = content() else {
        return;
    };
    let mut s = PlaySession::load(&content, "Challenges/ch_dungeons_ch1.lvl", "Wizard").unwrap();
    s.change_scene_named("chs_dungeons_ch1_s5", "start")
        .unwrap();
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert_eq!(s.world.book_pickups.len(), 1);
    let (&h, b) = s.world.book_pickups.iter().next().unwrap();
    assert_eq!(b.magick, 3);
    assert!(!b.immovable);
    let position = [b.transform[12], b.transform[13], b.transform[14]];
    let body = s.avatar().body;
    let orientation = s.body().orientation;
    s.world.physics.bodies[body].move_to(position, orientation);
    let equipment: Vec<_> = s
        .avatar()
        .items
        .iter()
        .map(|(slot, i)| (*slot, i.definition.name.clone()))
        .collect();
    let hp = s.avatar().character.combat.health.hit_points;
    assert_eq!(s.nearby_book(), Some(h));
    assert_eq!(s.pickup_book(h).unwrap(), 3);
    assert!(s.world.book_pickups[&h].dead);
    assert_eq!(s.nearby_book(), None);
    assert!(s.pickup_book(h).is_err());
    assert_eq!(s.unlocked_magicks.to_array::<4>(), [1 << 3; 4]);
    assert_eq!(s.avatar().motion.next_attack_animation, 149);
    assert_eq!(
        s.avatar()
            .items
            .iter()
            .map(|(slot, i)| (*slot, i.definition.name.clone()))
            .collect::<Vec<_>>(),
        equipment
    );
    assert_eq!(s.avatar().character.combat.health.hit_points, hp);
    s.change_scene_named("chs_dungeons_ch1_s6", "start")
        .unwrap();
    assert_eq!(s.unlocked_magicks.to_array::<4>(), [1 << 3; 4]);
}
#[test]
fn book_cache_reuses_dead_handle_and_advances_generation() {
    use magicka_runtime::play_session::PlaySession;
    let Some(content) = content() else {
        return;
    };
    let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let mut t = magicka_content::skinned::IDENTITY;
    t[12..15].copy_from_slice(&s.body().position);
    let h = s.spawn_magick_book(3, t).unwrap();
    s.step(1.0 / 60.0);
    assert!(
        s.effects
            .active_effects_by_hash(magicka_runtime::scene_loading::custom_hash(
                "bookofmagick_glimmer"
            ))
            > 0
    );
    s.pickup_book(h).unwrap();
    let second = s.spawn_magick_book(5, t).unwrap();
    assert_eq!(h, second);
    assert_eq!(s.world.book_pickups[&h].generation, 1);
    assert_eq!(s.world.book_pickups[&h].magick, 5);
    assert!(!s.world.book_pickups[&h].dead);
    s.step(1.0 / 60.0);
    s.pickup_book(h).unwrap();
    assert_eq!(s.unlocked_magicks.to_array::<4>(), [1 << 3 | 1 << 5; 4]);
}

#[test]
fn timed_and_removed_books_stop_glimmer_and_book_callbacks_run() {
    use magicka_runtime::{
        level_variables::LevelVariables, play_session::PlaySession, scene_loading::custom_hash,
        scene_script::SceneScriptLayout, script_runtime::ScriptRuntime,
    };
    let Some(content) = content() else {
        return;
    };
    let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let mut t = magicka_content::skinned::IDENTITY;
    t[12..15].copy_from_slice(&s.body().position);
    let h = s.spawn_magick_book(3, t).unwrap();
    s.world.book_pickups.get_mut(&h).unwrap().timeout = 0.05;
    s.step(0.01);
    assert!(
        s.effects
            .active_effects_by_hash(custom_hash("tome_timing_out"))
            > 0
    );
    s.step(0.05);
    assert!(s.world.book_pickups[&h].dead);
    assert_eq!(
        s.effects
            .active_effects_by_hash(custom_hash("bookofmagick_glimmer")),
        0
    );
    assert_eq!(
        s.effects
            .active_effects_by_hash(custom_hash("tome_timing_out")),
        0
    );
    assert!(
        s.effects
            .active_effects_by_hash(custom_hash("tome_disappear"))
            > 0
    );
    let h = s.spawn_magick_book(3, t).unwrap();
    s.step(0.01);
    s.world
        .entities
        .iter_mut()
        .find(|e| e.handle == h)
        .unwrap()
        .removed = true;
    s.step(0.01);
    assert!(s.world.book_pickups[&h].dead);
    assert_eq!(
        s.effects
            .active_effects_by_hash(custom_hash("bookofmagick_glimmer")),
        0
    );
    let h = s.spawn_magick_book(3, t).unwrap();
    s.runtime=ScriptRuntime::new(SceneScriptLayout::read(br#"<Scene><trigger id="collected" repeat="false"><if/><then><counter name="books" action="inc" value="1"/></then></trigger></Scene>"#,false).unwrap(),LevelVariables::default());
    s.world.book_pickups.get_mut(&h).unwrap().on_pickup = custom_hash("collected");
    // Bound equipment does not prevent collecting a book.
    s.world.players[0].as_mut().unwrap().items[0]
        .1
        .definition
        .bound = true;
    s.pickup_book(h).unwrap();
    assert!(
        s.runtime
            .trigger(custom_hash("collected"))
            .unwrap()
            .has_triggered
    );
    s.script_error = None;
    s.step(0.01);
    assert_eq!(s.runtime.variables.counter(custom_hash("books")), 1);
}

/// Rewrite addition: semi-custom magicks have books (by MagickType id after
/// the sentinel 35, which is refused) and picking one up unlocks it.
#[test]
fn book_catalog_matches_castable_magick_ids() {
    use magicka_sim::spells::{MagickType, ALL_MAGICKS_MASK, MAGICK_INDEX_COUNT};
    assert_eq!(BOOK_MAGICKS.len(), MAGICK_INDEX_COUNT as usize);
    let mut mask = 0u64;
    for (id, name) in BOOK_MAGICKS.iter().enumerate() {
        if id == SENTINEL as usize {
            assert!(name.is_empty());
            assert!(magick_id(&id.to_string()).is_none());
            continue;
        }
        assert!(MagickType::try_from(id as u8).is_ok());
        assert_eq!(magick_id(name), Some(id as u8));
        assert_eq!(magick_id(&id.to_string()), Some(id as u8));
        if id != 0 {
            mask |= 1u64 << id;
        }
    }
    assert_eq!(mask, ALL_MAGICKS_MASK);
    for invalid in ["", "  ", "35", "51", "255", "256", "unknown"] {
        assert!(magick_id(invalid).is_none(), "accepted {invalid:?}");
        assert!(SpawnMagickAction::read([("magick", invalid)]).is_err());
    }
    assert_eq!(magick_id(" PoLyMoRpH "), Some(39));
}

#[test]
fn semi_custom_books_spawn_and_unlock() {
    use magicka_runtime::magick_books::magick_id;
    use magicka_runtime::play_session::PlaySession;
    assert_eq!(magick_id("grow"), Some(36));
    assert_eq!(magick_id("inflictpoison"), Some(50));
    assert_eq!(magick_id(""), None);
    assert_eq!(magick_id("35"), None);
    let Some(content) = content() else {
        return;
    };
    let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let mut t = magicka_content::skinned::IDENTITY;
    t[12..15].copy_from_slice(&s.body().position);
    assert!(s.spawn_magick_book(35, t).is_err(), "sentinel refused");
    assert!(s.spawn_magick_book(51, t).is_err(), "out of range refused");
    let mut expected = s.unlocked_magicks.to_array::<4>();
    for id in 36..=50 {
        let h = s.spawn_magick_book(id, t).unwrap();
        s.step(1.0 / 60.0);
        assert_eq!(s.pickup_book(h).unwrap(), id);
        assert!(s.pickup_book(h).is_err(), "book consumed only once");
        for bits in &mut expected {
            *bits |= 1u64 << id;
        }
        assert_eq!(s.unlocked_magicks.to_array::<4>(), expected);
    }
}
