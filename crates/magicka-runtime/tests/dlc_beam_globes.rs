//! Tsar R'lyeh beam-globe switches on moving isles
//! (research/moving-platform-carry.md, "Static props ride animated parts").
//!
//! tr_s5 spawns a `beamglobe_minor_arcane` at a locator on each isle. Its
//! OnDeath trigger respawns a `beamglobe_minor_life` at the same locator and
//! plays the isle loop; killing that one respawns the arcane globe and stops
//! the isle. The globes are static physics entities, so PhysicsEntity.OnSpawn
//! attaches them to the isle under them and AnimatedLevelPart.Update moves
//! them with it.
use magicka_runtime::avatar::PlayerInput;
use magicka_runtime::entity_command::EntityCommandHost;
use magicka_runtime::play_session::PlaySession;
use magicka_runtime::scene_loading::custom_hash;
use magicka_runtime::spawn_phys::SpawnPhysHost;
use std::path::{Path, PathBuf};

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    root.is_dir().then_some(root)
}

fn frame(s: &mut PlaySession) {
    s.step(1.0 / 60.0);
    s.handle_input(PlayerInput::default(), 16.0 / 9.0);
    s.events.clear();
}

fn locator(s: &mut PlaySession, area: &str) -> [f32; 3] {
    let m = SpawnPhysHost::locator(&mut s.world, custom_hash(area)).unwrap();
    [m[12], m[13], m[14]]
}

/// The live globe of `template` nearest `at`.
fn globe(s: &PlaySession, template: &str, at: [f32; 3]) -> Option<u16> {
    s.world
        .entities
        .iter()
        .filter(|e| e.added && !e.removed && !e.dead)
        .filter(|e| e.template_name.as_deref() == Some(template))
        .min_by(|a, b| dist(a.position, at).total_cmp(&dist(b.position, at)))
        .filter(|e| dist(e.position, at) < 0.05)
        .map(|e| e.handle)
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}

fn position(s: &PlaySession, handle: u16) -> [f32; 3] {
    s.world
        .entities
        .iter()
        .find(|e| e.handle == handle)
        .unwrap()
        .position
}

#[test]
fn isle_globes_ride_their_isle_and_toggle_its_motion() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Tsar/Tsar_Rlyeh.lvl", "Wizard").unwrap();
    s.change_scene_named("tr_s5", "start").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    s.script_error = None;
    assert!(s.run_trigger("StartOfScene", true).unwrap());
    for _ in 0..5 {
        frame(&mut s);
    }
    assert_eq!(s.script_error, None);

    let area = "beamglobe_minor_arcane0";
    let start = locator(&mut s, area);
    let arcane = globe(&s, "beamglobe_minor_arcane", start).expect("arcane globe on isle 0");
    assert!(
        s.world.attached_props.contains_key(&arcane),
        "the static globe attaches to the isle under it"
    );
    // A stopped isle leaves its globe where it is, up to the float round-off
    // of the unconditional inverse(old) * new product (native does the same).
    for _ in 0..60 {
        frame(&mut s);
    }
    assert!(dist(position(&s, arcane), start) < 0.01);

    // Arcane in: OnDeath isle_0_moving spawns the life globe and plays isle_0.
    EntityCommandHost::kill(&mut s.world, arcane).unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    assert_eq!(s.script_error, None);
    let at = locator(&mut s, area);
    let life = globe(&s, "beamglobe_minor_life", at).expect("life globe after the arcane one");
    assert!(s.world.attached_props.contains_key(&life));
    let spawned = position(&s, life);
    // While the isle moves, the globe stays on its locator (part world times
    // the locator), not where it spawned.
    let mut travelled: f32 = 0.0;
    for _ in 0..240 {
        frame(&mut s);
        let now = position(&s, life);
        let target = locator(&mut s, area);
        assert!(
            dist(now, target) < 0.01,
            "globe {now:?} left its locator {target:?}"
        );
        travelled = travelled.max(dist(now, spawned));
    }
    assert!(travelled > 0.5, "isle 0 moves the globe ({travelled} m)");

    // Life in: OnDeath isle_0_stop respawns the arcane globe and stops isle_0.
    EntityCommandHost::kill(&mut s.world, life).unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    let at = locator(&mut s, area);
    let again = globe(&s, "beamglobe_minor_arcane", at).expect("arcane globe after the life one");
    assert!(!s.world.attached_props.contains_key(&life) || life == again);
    let rest = position(&s, again);
    for _ in 0..120 {
        frame(&mut s);
    }
    assert!(
        dist(position(&s, again), rest) < 0.01,
        "a stopped isle leaves its globe in place"
    );
}

/// Stand the player on floor `distance` from `centre`, facing it, with a
/// clear line at chest height.
fn stand_facing(s: &mut PlaySession, centre: [f32; 3], distance: f32) {
    let body = s.avatar().body;
    let offset = s.world.physics.bodies[body].height_offset();
    let mut best = None;
    for i in 0..64 {
        let angle = std::f32::consts::TAU * i as f32 / 64.0;
        let (x, z) = (
            centre[0] + angle.cos() * distance,
            centre[2] + angle.sin() * distance,
        );
        let Some(hit) = s.world.collision.collision().segment_intersect(
            magicka_sim::spray_pulse::Segment {
                origin: [x, centre[1] + 3.0, z],
                delta: [0.0, -6.0, 0.0],
            },
            false,
        ) else {
            continue;
        };
        if hit.hit.normal[1] < 0.8 || (hit.hit.position[1] - centre[1]).abs() > 0.5 {
            continue;
        }
        let eye = [x, hit.hit.position[1] + 1.0, z];
        let to = [centre[0] - x, 0.0, centre[2] - z].map(|v| v * 0.8);
        if s.world
            .collision
            .collision()
            .segment_intersect(
                magicka_sim::spray_pulse::Segment {
                    origin: eye,
                    delta: to,
                },
                false,
            )
            .is_some()
        {
            continue;
        }
        best = Some(hit.hit.position);
        break;
    }
    let feet = best.expect("floor facing the globe");
    let p = [feet[0], feet[1] - offset, feet[2]];
    let o = magicka_sim::character_physics::Orientation::from_forward_up(
        [centre[0] - p[0], 0.0, centre[2] - p[2]],
        [0.0, 1.0, 0.0],
    );
    s.world.physics.bodies[body].move_to(p, o);
    s.world.physics.bodies[body].set_desired_direction(o.forward());
}

/// A Life beam kills the red (`_arcane`) globe: its only non-zero
/// resistance is Life ×-1, turning the beam's negative (healing) damage
/// positive. Its OnDeath spawns the green (`_life`) globe and starts the
/// isle.
#[test]
fn a_life_beam_turns_the_isle_globe_green_and_starts_the_isle() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Tsar/Tsar_Rlyeh.lvl", "Wizard").unwrap();
    s.change_scene_named("tr_s5", "start").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    s.script_error = None;
    assert!(s.run_trigger("StartOfScene", true).unwrap());
    for _ in 0..5 {
        frame(&mut s);
    }
    let area = "beamglobe_minor_arcane0";
    let at = locator(&mut s, area);
    let red = globe(&s, "beamglobe_minor_arcane", at).expect("red globe on isle 0");
    stand_facing(&mut s, at, 4.0);
    for _ in 0..30 {
        frame(&mut s);
    }
    // Arcane does not hurt the red globe (×0).
    let cast = |s: &mut PlaySession, key: usize| {
        let mut e = [false; 8];
        e[key] = true;
        s.step(1.0 / 60.0);
        s.handle_input(
            PlayerInput {
                elements: e,
                ..Default::default()
            },
            16.0 / 9.0,
        );
        for _ in 0..10 {
            frame(s);
        }
        for i in 0..150 {
            s.step(1.0 / 60.0);
            s.handle_input(
                PlayerInput {
                    cast_down: i < 120,
                    ..Default::default()
                },
                16.0 / 9.0,
            );
            s.events.clear();
        }
    };
    cast(&mut s, 5);
    let at = locator(&mut s, area);
    assert!(
        globe(&s, "beamglobe_minor_arcane", at) == Some(red),
        "an arcane beam leaves the red globe"
    );
    cast(&mut s, 1);
    assert_eq!(s.script_error, None);
    let at = locator(&mut s, area);
    assert!(
        globe(&s, "beamglobe_minor_arcane", at).is_none(),
        "the life beam kills the red globe"
    );
    let green = globe(&s, "beamglobe_minor_life", at).expect("green globe spawned");
    assert!(s.world.attached_props.contains_key(&green));
    assert!(
        s.world
            .collision
            .parts()
            .iter()
            .any(|p| p.name == "isle_0" && p.playback.playing),
        "isle 0 moves"
    );
}

/// Direct beams keep toggling the same isle globe. PhysicsEntity.Removable
/// is Dead, so each killed globe leaves the entity list; before that a dead
/// globe stayed in place, intercepted the beam meant for its replacement
/// (only area spells reached the new one) and stayed behind as a blocker
/// once the isle moved.
#[test]
fn beams_toggle_an_isle_globe_repeatedly_while_the_isle_moves() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Tsar/Tsar_Rlyeh.lvl", "Wizard").unwrap();
    s.change_scene_named("tr_s5", "start").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    s.script_error = None;
    assert!(s.run_trigger("StartOfScene", true).unwrap());
    for _ in 0..5 {
        frame(&mut s);
    }
    let area = "beamglobe_minor_arcane0";
    let isle_playing = |s: &PlaySession| {
        s.world
            .collision
            .parts()
            .iter()
            .any(|p| p.name == "isle_0" && p.playback.playing)
    };
    let beam = |s: &mut PlaySession, key: usize| {
        let mut e = [false; 8];
        e[key] = true;
        s.step(1.0 / 60.0);
        s.handle_input(
            PlayerInput {
                elements: e,
                ..Default::default()
            },
            16.0 / 9.0,
        );
        for _ in 0..10 {
            frame(s);
        }
        for i in 0..90 {
            s.step(1.0 / 60.0);
            s.handle_input(
                PlayerInput {
                    cast_down: i < 40,
                    ..Default::default()
                },
                16.0 / 9.0,
            );
            s.events.clear();
        }
    };
    // (element key, globe hit, globe expected after, isle moving after)
    let steps = [
        (1, "beamglobe_minor_arcane", "beamglobe_minor_life", true),
        (5, "beamglobe_minor_life", "beamglobe_minor_arcane", false),
        (1, "beamglobe_minor_arcane", "beamglobe_minor_life", true),
        (5, "beamglobe_minor_life", "beamglobe_minor_arcane", false),
    ];
    for (round, (key, before, after, moving)) in steps.into_iter().enumerate() {
        let at = locator(&mut s, area);
        let target = globe(&s, before, at).unwrap_or_else(|| panic!("round {round}: {before}"));
        stand_facing(&mut s, at, 4.0);
        for _ in 0..10 {
            frame(&mut s);
        }
        beam(&mut s, key);
        assert_eq!(s.script_error, None);
        let at = locator(&mut s, area);
        assert!(
            s.world
                .entities
                .iter()
                .find(|e| e.handle == target)
                .is_some_and(|e| e.removed),
            "round {round}: the killed {before} leaves the entity list"
        );
        assert!(
            globe(&s, after, at).is_some(),
            "round {round}: a direct beam turns {before} into {after}"
        );
        assert_eq!(isle_playing(&s), moving, "round {round}");
        // Let a moving isle carry the new globe away from the old spot.
        for _ in 0..60 {
            frame(&mut s);
        }
    }
    // No dead globe remains anywhere to block beams.
    assert!(!s.world.entities.iter().any(|e| {
        e.added
            && !e.removed
            && e.dead
            && e.template_name
                .as_deref()
                .is_some_and(|n| n.starts_with("beamglobe"))
    }));
}

/// The R'lyeh mirror parts redirect beams onto globes: Railgun reflects off
/// scene hits whose material is CollisionMaterials.Reflect (7), and an
/// animated part reports its own CollisionMaterial.
#[test]
fn rlyeh_mirror_parts_reflect_beams() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    fn walk(
        parts: &[magicka_content::level::AnimatedLevelPart],
        out: &mut Vec<(String, Option<u8>)>,
    ) {
        for p in parts {
            out.push((
                p.name.to_ascii_lowercase(),
                p.collision.as_ref().map(|c| c.0),
            ));
            walk(&p.children, out);
        }
    }
    for scene in ["tr_s5", "tr_s7"] {
        let mut s = PlaySession::load(&root, "Tsar/Tsar_Rlyeh.lvl", "Wizard").unwrap();
        s.change_scene_named(scene, "start").unwrap();
        let mut parts = Vec::new();
        walk(&s.world.level.animated_parts, &mut parts);
        eprintln!("{scene}: {parts:?}");
        let materials: Vec<_> = parts
            .iter()
            .filter(|(n, _)| n.contains("mirror"))
            .map(|(_, m)| *m)
            .collect();
        assert!(
            materials
                .iter()
                .any(|m| *m == Some(magicka_sim::railgun::MATERIAL_REFLECT as u8)),
            "{scene} mirrors: {materials:?}"
        );
    }
}

/// Hold a beam of one element key briefly (a 1-HP globe dies on the first
/// 0.25 s damage tick).
fn short_beam(s: &mut PlaySession, key: usize) {
    let mut e = [false; 8];
    e[key] = true;
    s.step(1.0 / 60.0);
    s.handle_input(
        PlayerInput {
            elements: e,
            ..Default::default()
        },
        16.0 / 9.0,
    );
    for _ in 0..10 {
        frame(s);
    }
    for i in 0..90 {
        s.step(1.0 / 60.0);
        s.handle_input(
            PlayerInput {
                cast_down: i < 40,
                ..Default::default()
            },
            16.0 / 9.0,
        );
        s.events.clear();
    }
}

/// The tr_s5 mirror switch respawns its globe at the same locator
/// (turn_mirror_right / turn_mirror_wrong). The replacement overlaps the
/// killed globe exactly, so a dead globe left in the entity list wins the
/// beam's nearest-hit tie and only area spells reached the replacement.
#[test]
fn beams_toggle_the_mirror_globe_in_place_repeatedly() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Tsar/Tsar_Rlyeh.lvl", "Wizard").unwrap();
    s.change_scene_named("tr_s5", "start").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    s.script_error = None;
    assert!(s.run_trigger("StartOfScene", true).unwrap());
    for _ in 0..5 {
        frame(&mut s);
    }
    let area = "beamglobe_minor_arcane01";
    let at = locator(&mut s, area);
    stand_facing(&mut s, at, 4.0);
    for _ in 0..10 {
        frame(&mut s);
    }
    let rounds = [
        (1, "beamglobe_minor_arcane", "beamglobe_minor_life"),
        (5, "beamglobe_minor_life", "beamglobe_minor_arcane"),
        (1, "beamglobe_minor_arcane", "beamglobe_minor_life"),
        (5, "beamglobe_minor_life", "beamglobe_minor_arcane"),
    ];
    for (round, (key, before, after)) in rounds.into_iter().enumerate() {
        let at = locator(&mut s, area);
        globe(&s, before, at).unwrap_or_else(|| panic!("round {round}: {before}"));
        short_beam(&mut s, key);
        assert_eq!(s.script_error, None);
        let at = locator(&mut s, area);
        assert!(
            globe(&s, after, at).is_some(),
            "round {round}: a direct beam turns {before} into {after}"
        );
        for _ in 0..30 {
            frame(&mut s);
        }
    }
}

/// AnimatedLevelPart.PostCollision/Update apply to any entity body: a loose
/// item resting on a playing isle (floor normal Y > 0.7) is retained for
/// 0.25 s per contact and moved by the isle's delta. Here a dropped pickup
/// keeps its place on isle 0 while the isle loops.
#[test]
fn a_loose_item_rides_a_moving_isle() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Tsar/Tsar_Rlyeh.lvl", "Wizard").unwrap();
    s.change_scene_named("tr_s5", "start").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    s.script_error = None;
    assert!(s.run_trigger("StartOfScene", true).unwrap());
    for _ in 0..5 {
        frame(&mut s);
    }
    let at = locator(&mut s, "beamglobe_minor_arcane0");
    let mut t = magicka_content::skinned::IDENTITY;
    t[12..15].copy_from_slice(&[at[0], at[1] + 1.5, at[2]]);
    let item = s.spawn_item_pickup("weapon_octobers_whip", t).unwrap();
    s.world.item_pickups.get_mut(&item).unwrap().immovable = false;
    for _ in 0..120 {
        frame(&mut s);
    }
    let isle = |s: &PlaySession| {
        s.world
            .collision
            .parts()
            .iter()
            .find(|p| p.name == "isle_0")
            .unwrap()
            .world
    };
    let pos = |s: &PlaySession| {
        s.world
            .entities
            .iter()
            .find(|e| e.handle == item)
            .unwrap()
            .position
    };
    let rested = pos(&s);
    assert!(
        rested[1] > at[1] - 1.0,
        "the item rests on the isle: {rested:?}"
    );
    assert!(s.run_trigger("isle_0_moving", true).unwrap());
    for _ in 0..30 {
        frame(&mut s);
    }
    // The item's place in isle space after it starts moving.
    let inv = magicka_runtime::live_collision::inverse(&isle(&s)).unwrap();
    let p0 = pos(&s);
    let local = transform_point(&inv, p0);
    let mut travelled: f32 = 0.0;
    for _ in 0..180 {
        frame(&mut s);
        let want = transform_point(&isle(&s), local);
        let now = pos(&s);
        assert!(
            dist(now, want) < 0.25,
            "item {now:?} slid off its isle spot {want:?}"
        );
        travelled = travelled.max(dist(now, p0));
    }
    assert!(travelled > 0.5, "the isle carries the item ({travelled} m)");
}

fn transform_point(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| p[0] * m[i] + p[1] * m[4 + i] + p[2] * m[8 + i] + m[12 + i])
}
