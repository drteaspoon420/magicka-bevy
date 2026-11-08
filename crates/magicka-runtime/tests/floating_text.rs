//! FloatingTextManager, the Numbers shader cell selection and the Character
//! damage-number routes (research/floating-text.md). The retail test skips
//! when user-supplied content is absent.
use magicka_runtime::floating_text::{
    cell_count, glyph, quads, FloatingTextManager, NumberInstance, NumberRequest, BLEED_COLOR,
    FIRE_COLOR, GLYPH_PLUS, GLYPH_XP, POISON_COLOR,
};
use magicka_sim::damage_update::{DamageUpdateEvent as E, NumberKind as K};

/// Glyphs left to right, as the pixel shader selects them per cell.
fn text(value: f32, xp: bool) -> Vec<u8> {
    let n = cell_count(value, xp);
    (0..n).rev().map(|k| glyph(value, xp, n, k)).collect()
}

#[test]
fn numbers_shader_selects_digits_plus_sign_and_xp_suffix() {
    // Values.X is value + 0.5 (CopyToArrays), so positive values round.
    assert_eq!(text(15.0 + 0.5, false), [1, 5]);
    assert_eq!(text(0.0 + 0.5, false), [0]);
    assert_eq!(text(1234.0 + 0.5, false), [1, 2, 3, 4]);
    assert_eq!(text(99.6 + 0.5, false), [1, 0, 0]);
    // Healing is negative: '+' in the leftmost cell. The +0.5 offset makes
    // integral heals display one less (shader bytecode, not a port choice).
    assert_eq!(text(-10.0 + 0.5, false), [GLYPH_PLUS, 9]);
    assert_eq!(text(-1.0 + 0.5, false), [GLYPH_PLUS, 0]);
    // Below the -0.1 threshold no sign cell is added.
    assert_eq!(text(-0.55 + 0.5, false), [0]);
    // Experience points: digits shifted one cell, "XP" in the rightmost.
    assert_eq!(text(100.0 + 0.5, true), [1, 0, 0, GLYPH_XP]);
}

#[test]
fn quads_are_centred_cells_scaled_by_instance() {
    let n = NumberInstance {
        world_position: [0.0; 3],
        scale: [0.5, 0.5],
        value: 42.5,
        is_xp: false,
        color: [1.0, 0.9, 0.0, 1.0],
    };
    let q = quads(&n, [100.0, 50.0]);
    assert_eq!(q.len(), 2);
    assert_eq!(q[0].bounds, [89.0, 42.0, 100.0, 58.0]);
    assert_eq!(q[1].bounds, [100.0, 42.0, 111.0, 58.0]);
    assert_eq!(q[0].atlas, [88.0, 0.0, 110.0, 32.0], "digit 4");
    assert_eq!(q[1].atlas, [44.0, 0.0, 66.0, 32.0], "digit 2");
    assert!(q.iter().all(|q| q.color == n.color));
}

#[test]
fn manager_allocates_lowest_free_slot_and_recycles_after_expiry() {
    let mut m = FloatingTextManager::default();
    assert_eq!(m.add(5.0, [0.0; 3], 0.1, false), 0);
    assert_eq!(m.add(6.0, [0.0; 3], 1.0, false), 1);
    assert_eq!(m.last_number(), 1);
    // The snapshot precedes aging: both numbers fully opaque this frame.
    let first = m.update(0.2, false);
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].color[3], 1.0);
    // Slot 0 expired and is popped again before slot 2.
    assert_eq!(m.add(7.0, [0.0; 3], 1.0, false), 0);
    assert_eq!(m.add(8.0, [0.0; 3], 1.0, false), 2);
    // Trailing expired numbers shrink the drawn range.
    let mut m = FloatingTextManager::default();
    m.add(1.0, [0.0; 3], 1.0, false);
    m.add(1.0, [0.0; 3], 0.1, false);
    m.update(0.2, false);
    assert_eq!(m.last_number(), 0);
}

#[test]
fn fade_scale_and_lock_follow_copy_to_arrays_and_update() {
    let mut m = FloatingTextManager::default();
    let i = m.add(1500.0, [0.0; 3], 0.4, true);
    for _ in 0..30 {
        m.update(0.1, false);
    }
    // Locked numbers never age; scale grows with |value|/1000 up to 2.
    let n = *m.number(i as usize);
    assert_eq!(n.ttl, [0.4, 0.4]);
    assert_eq!(n.scale, [1.25, 1.25]);
    m.add_to(i, 1000.0);
    m.update(0.0, false);
    assert_eq!(m.number(i as usize).scale, [1.5, 1.5]);
    m.release(i);
    m.update(0.2, false);
    let half = m.update(0.0, false)[0].color[3];
    let expected = 1.0 - (1.0f64 - 0.5).powi(5) as f32;
    assert!((half - expected).abs() < 1e-6, "{half} vs {expected}");
    m.update(0.3, false);
    assert_eq!(m.update(0.0, false).len(), 0, "expired number not drawn");
}

#[test]
fn xp_numbers_wait_half_a_second_then_rise_and_fade() {
    let mut m = FloatingTextManager::default();
    let i = m.add_xp(100.0, [0.0, 1.0, 0.0], 1.0, false, [1.0; 3]);
    assert_eq!(m.number(i as usize).ttl, [1.5, 1.5]);
    let shown = m.update(0.25, false);
    assert_eq!(shown[0].color[3], 0.0, "hidden during the delay");
    assert!(shown[0].is_xp);
    m.update(0.25, false);
    let y = m.number(i as usize).position[1];
    assert!((y - (1.0 + 0.02 - 0.25 * 0.03)).abs() < 1e-6, "{y}");
    assert!(m.update(0.0, false)[0].color[3] > 0.0);
    // Pausing stops motion but not aging.
    let before = *m.number(i as usize);
    m.update(0.1, true);
    let after = *m.number(i as usize);
    assert_eq!(after.position, before.position);
    assert!(after.ttl[1] < before.ttl[1]);
}

#[test]
fn disabled_setting_returns_slot_zero_without_drawing() {
    let mut m = FloatingTextManager::default();
    m.enabled = false;
    assert_eq!(m.add(5.0, [0.0; 3], 1.0, true), 0);
    assert_eq!(m.update(0.1, false).len(), 0);
}

#[test]
fn character_requests_accumulate_lock_move_release_and_colour() {
    let mut m = FloatingTextManager::default();
    let mut index = -1;
    let mut q = vec![
        NumberRequest::Direct { amount: 10.0 },
        NumberRequest::Direct { amount: 5.0 },
    ];
    m.apply(&mut index, &mut q, [1.0, 2.0, 3.0]);
    assert!(q.is_empty());
    assert_eq!(index, 0);
    let n = *m.number(0);
    assert_eq!((n.value, n.locked, n.ttl), (15.0, true, [0.4, 0.4]));
    assert_eq!(n.color, [1.0, 0.9, 0.0]);
    m.apply(
        &mut index,
        &mut vec![NumberRequest::Update(E::MoveNumber(0))],
        [4.0, 5.0, 6.0],
    );
    assert_eq!(m.number(0).position, [4.0, 5.0, 6.0]);
    // UpdateDamage already cleared its index when it emitted the release.
    let mut released = -1;
    m.apply(
        &mut released,
        &mut vec![NumberRequest::Update(E::ReleaseNumber(0))],
        [0.0; 3],
    );
    assert!(!m.number(0).locked);
    let mut q: Vec<_> = [
        (K::Healing, -20.0),
        (K::Fire, 3.0),
        (K::Poison, 4.0),
        (K::Bleeding, 2.0),
        (K::Regeneration, -1.0),
    ]
    .into_iter()
    .map(|(kind, amount)| {
        NumberRequest::Update(E::Number {
            kind,
            amount,
            ttl: 0.7,
        })
    })
    .collect();
    let mut idle = -1;
    m.apply(&mut idle, &mut q, [0.0; 3]);
    let colours: Vec<_> = (1..6).map(|i| m.number(i).color).collect();
    assert_eq!(
        colours,
        [
            [0.0, 1.0, 0.0],
            FIRE_COLOR,
            POISON_COLOR,
            BLEED_COLOR,
            [0.0, 1.0, 0.0]
        ]
    );
    assert!((1..6).all(|i| !m.number(i).locked && m.number(i).ttl == [0.7, 0.7]));
    assert_eq!(idle, -1, "status numbers never take the locked index");
    // Deinitialize releases a locked direct number.
    let mut index = m.add(1.0, [0.0; 3], 0.4, true);
    m.apply(&mut index, &mut vec![NumberRequest::Release], [0.0; 3]);
    assert_eq!(index, -1);
}

mod retail {
    use magicka_runtime::{
        avatar::PlayerInput,
        play_content::load_start_scene,
        play_session::PlaySession,
        scene_script::{ActionDefinition, SceneScriptLayout},
        spawn_action::SpawnState,
    };
    use std::path::{Path, PathBuf};

    const DT: f32 = 1.0 / 60.0;
    const LEVEL: &str = "Havindr.lvl";
    const FIRE: usize = 7;

    fn content() -> Option<PathBuf> {
        let root = std::env::var_os("MAGICKA_CONTENT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
            });
        root.is_dir().then_some(root)
    }
    fn frame(s: &mut PlaySession, input: PlayerInput) {
        s.step(DT);
        s.handle_input(input, 16.0 / 9.0);
        s.events.clear();
    }
    fn spawn_in_front(s: &mut PlaySession, root: &Path, template: &str, distance: f32) -> u16 {
        let area = load_start_scene(root, LEVEL).unwrap().start.area.unwrap();
        let xml = format!(
            r#"<Scene><trigger id="t"><then><spawn type="{template}" area="{area}1" order="idle"/></then></trigger></Scene>"#
        );
        let ActionDefinition::Spawn(action) = SceneScriptLayout::read(xml.as_bytes(), false)
            .unwrap()
            .actions
            .remove(0)
        else {
            panic!("not a spawn action");
        };
        let mut state = SpawnState::default();
        action.initialize(&mut state, &mut s.world).unwrap();
        action.execute(&mut state, &mut s.world).unwrap();
        let handle = state.spawned[0].handle.unwrap();
        let (position, forward) = (s.body().position, s.body().direction());
        let body = &mut s.world.physics.bodies[s.world.npcs[&handle].body];
        let target = [
            position[0] + forward[0] * distance,
            body.position[1].max(position[1]),
            position[2] + forward[2] * distance,
        ];
        let orientation = body.orientation;
        body.move_to(target, orientation);
        handle
    }

    /// A real Fire spray on a spawned goblin: one locked yellow number
    /// accumulates the direct damage at its capsule top, is released after
    /// 0.333 s without damage and fades; burning adds FIRECOLOR numbers.
    #[test]
    fn fire_spray_shows_locked_direct_number_then_burn_ticks() {
        let Some(root) = content() else {
            eprintln!("retail content absent; skipping");
            return;
        };
        let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
        s.disable_scripts();
        for _ in 0..60 {
            frame(&mut s, PlayerInput::default());
        }
        let target = spawn_in_front(&mut s, &root, "Goblin_miner", 2.0);
        let hp = s.world.npcs[&target].character.combat.health.hit_points;
        let mut e = [false; 8];
        e[FIRE] = true;
        frame(
            &mut s,
            PlayerInput {
                elements: e,
                ..Default::default()
            },
        );
        for _ in 0..10 {
            frame(&mut s, PlayerInput::default());
        }
        let mut locked_seen = false;
        for _ in 0..40 {
            frame(
                &mut s,
                PlayerInput {
                    cast_down: true,
                    ..Default::default()
                },
            );
            let npc = &s.world.npcs[&target];
            let index = npc.character.damage_bookkeeping.last_damage_index;
            if index >= 0 {
                let n = *s.world.floating_text.number(index as usize);
                let body = &s.world.physics.bodies[npc.body];
                assert!(n.locked && n.value > 0.0);
                assert_eq!(n.color, [1.0, 0.9, 0.0]);
                assert_eq!(n.ttl[1], 0.4, "locked numbers do not age");
                assert!(
                    (n.position[1] - (body.position[1] + body.length * 0.5 + body.radius)).abs()
                        < 1e-4,
                    "number follows the capsule top"
                );
                locked_seen = true;
            }
        }
        let npc_hp = s
            .world
            .npcs
            .get(&target)
            .map_or(0.0, |n| n.character.combat.health.hit_points);
        assert!(npc_hp < hp, "spray must damage the goblin");
        assert!(locked_seen, "no locked direct-damage number");
        assert!(s
            .world
            .floating_numbers
            .iter()
            .any(|n| n.color[3] > 0.0 && n.value > 0.5));
        let mut fire_seen = false;
        for _ in 0..180 {
            frame(&mut s, PlayerInput::default());
            fire_seen |= s.world.floating_numbers.iter().any(|n| {
                n.color[..3] == magicka_runtime::floating_text::FIRE_COLOR[..] && n.color[3] > 0.0
            });
        }
        assert!(fire_seen, "burning produced no FIRECOLOR number");
        if let Some(npc) = s.world.npcs.get(&target) {
            assert_eq!(
                npc.character.damage_bookkeeping.last_damage_index, -1,
                "released"
            );
        }
        assert!(
            s.world
                .floating_numbers
                .iter()
                .filter(|n| n.color[..3] == [1.0, 0.9, 0.0][..])
                .all(|n| n.color[3] == 0.0),
            "released direct numbers faded out"
        );
    }
}
