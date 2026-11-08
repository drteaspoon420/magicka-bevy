//! DLC campaign scripted props (Dungeons ch1, Tsar Mountaindale) on retail content.
use magicka_runtime::avatar::PlayerInput;
use magicka_runtime::play_session::PlaySession;
use magicka_runtime::scene_loading::custom_hash;
use magicka_runtime::spawn_phys::SpawnPhysHost;
use std::path::PathBuf;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
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

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}

fn prop_at(s: &PlaySession, template: &str, at: [f32; 3]) -> Option<u16> {
    s.world
        .entities
        .iter()
        .filter(|e| e.added && !e.removed && !e.dead)
        .filter(|e| {
            e.template_name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(template))
        })
        .find(|e| dist(e.position, at) < 0.05)
        .map(|e| e.handle)
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
        if hit.hit.normal[1] < 0.8 || (hit.hit.position[1] - centre[1]).abs() > 2.0 {
            continue;
        }
        let eye = [x, hit.hit.position[1] + 1.0, z];
        let to = [centre[0] - x, 0.0, centre[2] - z].map(|v| v * 0.6);
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
    let feet = best.expect("floor facing the prop");
    let p = [feet[0], feet[1] - offset, feet[2]];
    let o = magicka_sim::character_physics::Orientation::from_forward_up(
        [centre[0] - p[0], 0.0, centre[2] - p[2]],
        [0.0, 1.0, 0.0],
    );
    s.world.physics.bodies[body].move_to(p, o);
    s.world.physics.bodies[body].set_desired_direction(o.forward());
}

/// Conjure one element key (avatar ELEMENT_KEYS order) and hold cast.
fn cast(s: &mut PlaySession, key: usize, frames: usize) {
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
    for i in 0..frames + 60 {
        s.step(1.0 / 60.0);
        s.handle_input(
            PlayerInput {
                cast_down: i < frames,
                ..Default::default()
            },
            16.0 / 9.0,
        );
        s.events.clear();
    }
}

/// The 6,000-HP southern gate needs repeated casts, not a forced script kill.
#[test]
fn repeated_arcane_casts_break_the_tsar_castle_southern_gate() {
    let Some(root) = content() else {
        return;
    };
    let mut s = PlaySession::load(&root, "Tsar/Tsar_WizardCastle.lvl", "Wizard").unwrap();
    s.change_scene_named("twc_s2", "start").unwrap();
    // The chapter starts without items; the first scene normally grants them
    // during Vlad's intro. This fixture jumps straight to the second scene.
    s.reset_player_equipment(0, "Wizard").unwrap();
    for _ in 0..420 {
        frame(&mut s);
    }
    assert_eq!(s.script_error, None);
    assert!(!s.input_limited(), "gate fixture input is limited");
    assert!(!s.world.flags.cutscene, "gate fixture is in a cutscene");
    assert!(
        s.world.tutorial.dialog_done(),
        "gate fixture has a modal dialog"
    );
    assert_eq!(s.world.flags.tutorial_elements_disabled, 0);
    assert!(s.world.flags.tutorial_cast_types_disabled.is_empty());
    let at = locator(&mut s, "door_phys");
    let door = prop_at(&s, "goblin_doors", at).expect("southern gate spawned");
    assert_eq!(s.world.physics_damage[&door].health, 6000.0);
    // A position the traversal bot reaches by walking, on the approach side
    // of the gate.
    let body = s.avatar().body;
    let position = [44.0, 1.0, -103.0];
    let direction = [at[0] - position[0], 0.0, at[2] - position[2]];
    let orientation =
        magicka_sim::character_physics::Orientation::from_forward_up(direction, [0.0, 1.0, 0.0]);
    s.world.physics.bodies[body].move_to(position, orientation);
    s.world.physics.bodies[body].set_desired_direction(orientation.forward());
    for tick in 0..3600 {
        let phase = tick % 132;
        let mut elements = [false; 8];
        elements[5] = phase == 0;
        s.handle_input(
            PlayerInput {
                elements,
                pad_movement: Some([0.0; 2]),
                cast_down: (18..120).contains(&phase),
                ..Default::default()
            },
            16.0 / 9.0,
        );
        s.step(1.0 / 60.0);
        s.events.clear();
        if prop_at(&s, "goblin_doors", at).is_none() {
            eprintln!("southern gate broken by Arcane at tick {tick}");
            break;
        }
    }
    assert_eq!(s.script_error, None);
    assert!(
        prop_at(&s, "goblin_doors", at).is_none(),
        "repeated Arcane should break the gate: hp {}, state {:?}, beams {}, position {:?}, dead {}",
        s.world.physics_damage.get(&door).map_or(0.0, |z| z.health),
        s.avatar().motion.state,
        s.beams.len(),
        s.body().position,
        s.dead(),
    );
    assert!(
        s.runtime
            .trigger(custom_hash("doorsdestroyed"))
            .unwrap()
            .has_triggered
    );
}

/// chs_dungeons_ch1_s7 Start spawns four `Havindr_fire` props (50 HP,
/// Water ×1 +100, every other element ×0, not solid); each OnDeath runs
/// IncreaseFireCounter. Fire does not hurt them; a water spray puts one out.
#[test]
fn a_water_spray_puts_out_a_dungeon_fire_and_counts_it() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Challenges/ch_dungeons_ch1.lvl", "Wizard").unwrap();
    s.change_scene_named("chs_dungeons_ch1_s7", "start")
        .unwrap();
    // Start's AvatarMove walks the player in from the spawn point.
    for _ in 0..240 {
        frame(&mut s);
    }
    assert_eq!(s.script_error, None);
    let at = locator(&mut s, "fire0");
    let fire = prop_at(&s, "Havindr_fire", at).expect("fire0 spawned by Start");
    stand_facing(&mut s, at, 3.0);
    for _ in 0..30 {
        frame(&mut s);
    }
    cast(&mut s, 7, 120);
    assert_eq!(
        prop_at(&s, "Havindr_fire", at),
        Some(fire),
        "fire leaves the fire burning"
    );
    cast(&mut s, 0, 120);
    assert_eq!(s.script_error, None);
    assert!(
        prop_at(&s, "Havindr_fire", at).is_none(),
        "water puts the fire out"
    );
    // Each death runs IncreaseFireCounter once; the spray cone can reach a
    // neighbouring fire too.
    let out = (0..4)
        .filter(|i| {
            let at = locator(&mut s, &format!("fire{i}"));
            prop_at(&s, "Havindr_fire", at).is_none()
        })
        .count() as i32;
    assert_eq!(
        s.runtime.variables.counter(custom_hash("countfires")),
        out,
        "OnDeath IncreaseFireCounter per fire put out"
    );
}

/// chs_dungeons_ch1_s2's difficulty brazier: two overlapping 1-HP detectors
/// at `spawn_difficulty_entity`. `dungeons_difficulty_torch_on` takes only
/// Lightning (×10 +10), `_torch_off` only Water (×10 +10); each OnDamage
/// respawns its detector after 1 s and flips `hard_on`.
#[test]
fn the_difficulty_brazier_lights_with_lightning_and_douses_with_water() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Challenges/ch_dungeons_ch1.lvl", "Wizard").unwrap();
    s.change_scene_named("chs_dungeons_ch1_s2", "start")
        .unwrap();
    // Start sets hard_on to 0 after 3 s; let any walk-in finish.
    for _ in 0..300 {
        frame(&mut s);
    }
    assert_eq!(s.script_error, None);
    let hard = |s: &PlaySession| s.runtime.variables.counter(custom_hash("hard_on"));
    assert_eq!(hard(&s), 0);
    let at = locator(&mut s, "spawn_difficulty_entity");
    assert!(prop_at(&s, "dungeons_difficulty_torch_on", at).is_some());
    assert!(prop_at(&s, "dungeons_difficulty_torch_off", at).is_some());
    stand_facing(&mut s, at, 3.0);
    for _ in 0..30 {
        frame(&mut s);
    }
    cast(&mut s, 4, 60);
    for _ in 0..120 {
        frame(&mut s);
    }
    assert_eq!(s.script_error, None);
    assert_eq!(hard(&s), 1, "lightning lights the brazier");
    assert!(
        prop_at(&s, "dungeons_difficulty_torch_on", at).is_some(),
        "the on detector respawns after 1 s"
    );
    cast(&mut s, 0, 60);
    for _ in 0..120 {
        frame(&mut s);
    }
    assert_eq!(hard(&s), 0, "water douses the brazier");
    assert!(prop_at(&s, "dungeons_difficulty_torch_off", at).is_some());
}

/// Tsar Mountaindale tmd_s5BF: the `pyre` prop (100 HP, only Fire ×10)
/// starts the boss sequence through OnDeath `pyre_lit`, whose first
/// action is a checkpoint at `cp_`. Water leaves it unlit.
#[test]
fn a_fire_spray_lights_the_mountaindale_pyre() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Tsar/Tsar_Mountaindale.lvl", "Wizard").unwrap();
    s.change_scene_named("tmd_s5bf", "start").unwrap();
    for _ in 0..300 {
        frame(&mut s);
    }
    assert_eq!(s.script_error, None);
    let at = locator(&mut s, "spawn_pyre");
    let pyre = prop_at(&s, "pyre", at).expect("pyre spawned");
    stand_facing(&mut s, at, 3.5);
    for _ in 0..30 {
        frame(&mut s);
    }
    cast(&mut s, 0, 90);
    assert_eq!(prop_at(&s, "pyre", at), Some(pyre), "water leaves the pyre");
    let checkpoints = s.world.log.iter().filter(|l| *l == "checkpoint").count();
    cast(&mut s, 7, 120);
    assert_eq!(s.script_error, None);
    assert!(prop_at(&s, "pyre", at).is_none(), "fire lights the pyre");
    assert!(
        s.world.log.iter().filter(|l| *l == "checkpoint").count() > checkpoints,
        "pyre_lit ran (its checkpoint)"
    );
}

/// chs_dungeons_ch2_s2 postboxes (1000 HP, Earth ×1 -200): once
/// `postbox_1` is gone, `idpresent ... invert` lets postbox_break_1 drop
/// the crowbar at `spawn_item2`.
#[test]
fn breaking_the_first_ch2_postbox_drops_the_crowbar() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Challenges/ch_dungeons_ch2.lvl", "Wizard").unwrap();
    s.change_scene_named("chs_dungeons_ch2_s2", "start")
        .unwrap();
    for _ in 0..300 {
        frame(&mut s);
    }
    assert_eq!(s.script_error, None);
    assert_eq!(
        s.runtime.variables.counter(custom_hash("sc2_init_done")),
        1,
        "scene initialised"
    );
    let crowbars = |s: &PlaySession| {
        s.world
            .item_pickups
            .values()
            .filter_map(|p| p.item.as_ref())
            .filter(|i| i.definition.name.eq_ignore_ascii_case("weapon_crowbar"))
            .count()
    };
    let before = crowbars(&s);
    let postbox = s
        .world
        .entities
        .iter()
        .find(|e| e.unique_id == custom_hash("postbox_1") && e.added && !e.removed && !e.dead)
        .expect("postbox_1")
        .handle;
    magicka_runtime::entity_command::EntityCommandHost::kill(&mut s.world, postbox).unwrap();
    for _ in 0..60 {
        frame(&mut s);
    }
    assert_eq!(s.script_error, None);
    assert_eq!(
        crowbars(&s),
        before + 1,
        "postbox_break_1 drops the crowbar"
    );
}
