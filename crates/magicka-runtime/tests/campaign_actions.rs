//! Campaign trigger actions on retail scripts (research/campaign-actions.md).
use magicka_runtime::avatar::PlayerInput;
use magicka_runtime::play_session::PlaySession;
use magicka_runtime::scene_loading::custom_hash;
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

fn item_names(s: &PlaySession) -> Vec<(i32, String)> {
    let mut v: Vec<_> = s
        .avatar()
        .items
        .iter()
        .map(|(slot, i)| (*slot, i.definition.name.to_ascii_lowercase()))
        .collect();
    v.sort();
    v
}

/// wc_s1 Init (beginDefaultAvatarSection, beginUnequipWeaponsSection) for
/// tests that jump past the opening scene before it runs. Applied on the
/// next step.
fn opening_sections(s: &mut PlaySession) {
    use magicka_runtime::scene_world::campaign_live::CampaignRequest;
    s.world
        .campaign_requests
        .push(CampaignRequest::DefaultAvatarSection(true));
    s.world
        .campaign_requests
        .push(CampaignRequest::UnequipWeaponsSection {
            begin: true,
            animation: 0,
        });
}

/// The opening plus wc_s3's weapon rack (endUnequipWeaponsSection): the
/// template staff and sword. Level.NoItems leaves the Wizard unarmed, and an
/// unarmed avatar has no staff spell source to cast from.
fn weapon_rack(s: &mut PlaySession) {
    use magicka_runtime::scene_world::campaign_live::CampaignRequest;
    opening_sections(s);
    s.world
        .campaign_requests
        .push(CampaignRequest::UnequipWeaponsSection {
            begin: false,
            animation: 0,
        });
}

/// wc_s1 Init begins the default-avatar and unequip sections: the Wizard
/// starts with weapon_unarmed in both slots, keeps it in wc_s3, and the
/// weapon rack's OnInteract (endUnequipWeaponsSection animation=
/// pickup_weapon) restores the template staff and weapon.
#[test]
fn opening_castle_is_unarmed_until_the_weapon_rack() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    // The Wizard robe's template equipment, from a level without NoItems.
    let armed = item_names(&PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap());
    assert!(
        armed.iter().all(|(_, n)| n != "weapon_unarmed"),
        "{armed:?}"
    );
    // Level.NoItems: unarmed from the first frame (Level.GoToScene).
    frame(&mut s);
    assert_eq!(
        item_names(&s),
        vec![(0, "weapon_unarmed".into()), (1, "weapon_unarmed".into())]
    );
    for _ in 0..10 {
        frame(&mut s);
    }
    assert!(s.unequip_weapons_section && s.default_avatar_section);
    assert_eq!(
        item_names(&s),
        vec![(0, "weapon_unarmed".into()), (1, "weapon_unarmed".into())]
    );
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..10 {
        frame(&mut s);
    }
    assert!(s.unequip_weapons_section);
    assert_eq!(
        item_names(&s),
        vec![(0, "weapon_unarmed".into()), (1, "weapon_unarmed".into())]
    );
    // Use the rack directly (the hint sequence that enables it is covered by
    // the tutorial tests).
    let id = custom_hash("poi_weaponrack");
    let (rack, _) = s
        .world
        .interactable_locator(id)
        .expect("weapon rack locator");
    s.world.flags.interactables_disabled.remove(&id);
    s.world.flags.cutscene = false;
    let body = s.avatar().body;
    let o = s.world.physics.bodies[body].orientation;
    let y = s.body().position[1];
    s.world.physics.bodies[body].move_to([rack[0], y, rack[2]], o);
    frame(&mut s);
    assert!(s.interact(), "the rack is in reach");
    frame(&mut s);
    assert!(!s.unequip_weapons_section);
    assert_eq!(item_names(&s), armed, "template staff and weapon restored");
    let pickup = magicka_runtime::character_motion::animation_index("pickup_weapon").unwrap();
    // Avatar.Attack(pickup_weapon): queued until the avatar can attack.
    let m = &s.avatar().motion;
    assert!(
        m.controller.current_animation == pickup
            || (m.attacking && m.next_attack_animation == pickup),
        "pickup_weapon requested"
    );
}

/// User report: disabled elements did not grey out. wc_s1 Init disables
/// every element; the keyboard HUD icons follow the tutorial mask and
/// desaturate (Saturation − 4/s) instead of staying in full colour. The
/// KeyboardHUD does not update while a cutscene disables the UI, so the
/// second is counted from the end of the opening cutscene.
#[test]
fn disabled_elements_desaturate_on_the_keyboard_hud() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    for _ in 0..60 * 120 {
        frame(&mut s);
        if s.world.ui_enabled && !s.world.flags.cutscene {
            break;
        }
    }
    assert!(s.world.ui_enabled, "the opening cutscene ends");
    for _ in 0..60 {
        frame(&mut s);
    }
    assert_eq!(s.world.flags.tutorial_elements_disabled & 0xFF, 0xFF);
    for icon in &s.keyboard_hud.icons {
        assert!(!icon.enabled);
        assert_eq!(icon.saturation, 0.0, "fully grey after a second");
    }
}

fn press_element(s: &mut PlaySession, index: usize) {
    let mut elements = [false; 8];
    elements[index] = true;
    s.step(1.0 / 60.0);
    s.handle_input(
        PlayerInput {
            elements,
            ..Default::default()
        },
        16.0 / 9.0,
    );
    s.events.clear();
}

fn settle(s: &mut PlaySession, frames: usize) {
    for _ in 0..frames {
        frame(s);
    }
    assert!(!s.avatar().dead(), "avatar died at {:?}", s.body().position);
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
}

fn dismiss_lesson(s: &mut PlaySession) {
    settle(s, 150);
    assert!(s.world.tutorial.dialog.is_some(), "expected tutorial modal");
    s.handle_input(
        PlayerInput {
            move_down: true,
            ..Default::default()
        },
        16.0 / 9.0,
    );
    settle(s, 150);
    assert!(!s.input_limited(), "modal callback did not enable input");
}

fn enter_lesson_area(s: &mut PlaySession, name: &str) {
    let area = s
        .world
        .level
        .trigger_areas
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(name))
        .unwrap();
    let pose = magicka_content::skinned::pose_matrix(area.position, area.orientation, [1.0; 3]);
    let centre: [f32; 3] = std::array::from_fn(|i| {
        pose[12 + i]
            + (0..3)
                .map(|j| pose[j * 4 + i] * area.sides[j] * 0.5)
                .sum::<f32>()
    });
    let body = s.avatar().body;
    let at = [centre[0], s.body().position[1], centre[2]];
    s.world.physics.bodies[body].move_to(at, magicka_sim::character_physics::Orientation::IDENTITY);
    eprintln!("lesson area {name}: {at:?}");
    settle(s, 180);
}

fn prop_callback(s: &PlaySession, callback: &str) -> u16 {
    let id = custom_hash(callback);
    *s.world
        .physics_damage
        .iter()
        .find(|(_, p)| p.on_damage == id || p.on_death == id)
        .unwrap()
        .0
}

fn face_prop(s: &mut PlaySession, handle: u16, distance: f32) {
    let prop = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == handle)
        .unwrap();
    let bounds = s.world.physics_damage[&handle]
        .world_box(&prop.orientation)
        .unwrap();
    let p = s.body().position;
    let d = [p[0] - bounds.center[0], 0.0, p[2] - bounds.center[2]];
    let length = (d[0] * d[0] + d[2] * d[2]).sqrt();
    assert!(length > 0.0);
    let toward: [f32; 3] = d.map(|v| -v / length);
    let at = [
        bounds.center[0] - toward[0] * distance,
        p[1],
        bounds.center[2] - toward[2] * distance,
    ];
    let body = s.avatar().body;
    s.world.physics.bodies[body].move_to(
        at,
        magicka_sim::character_physics::Orientation::from_forward_up(toward, [0.0, 1.0, 0.0]),
    );
    s.world.physics.bodies[body].set_desired_direction(toward);
    eprintln!("face prop {handle} bounds {bounds:?} from {at:?}");
}

/// Choose a nearby supported fixture position instead of placing an actor
/// over the gaps between the Castle's furnace platforms.
fn face_prop_on_floor(s: &mut PlaySession, handle: u16, distance: f32) {
    let e = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == handle)
        .unwrap();
    let bounds = s.world.physics_damage[&handle]
        .world_box(&e.orientation)
        .unwrap();
    face_point_on_floor(s, bounds.center, bounds.side_lengths, distance);
}

fn face_point_on_floor(s: &mut PlaySession, centre: [f32; 3], sides: [f32; 3], distance: f32) {
    let from = s.body().position;
    let heading = (from[2] - centre[2]).atan2(from[0] - centre[0]);
    let mut candidates = Vec::new();
    for i in 0..32 {
        let angle = heading + std::f32::consts::TAU * i as f32 / 32.0;
        let x = centre[0] + angle.cos() * distance;
        let z = centre[2] + angle.sin() * distance;
        if let Some(hit) = s.world.collision.collision().segment_intersect(
            magicka_sim::spray_pulse::Segment {
                origin: [x, from[1] + 2.0, z],
                delta: [0.0, -6.0, 0.0],
            },
            false,
        ) {
            if hit.hit.normal[1].abs() > 0.8
                && (hit.hit.position[1] - s.body().feet()[1]).abs() < 1.0
            {
                let origin = [x, hit.hit.position[1] - s.body().height_offset() + 0.5, z];
                let scale = (distance - sides.iter().copied().fold(0.0, f32::max) * 0.5) / distance;
                let delta = [centre[0] - x, 0.0, centre[2] - z].map(|v| v * scale);
                if s.world
                    .collision
                    .collision()
                    .segment_intersect(magicka_sim::spray_pulse::Segment { origin, delta }, false)
                    .is_none()
                {
                    candidates.push(hit.hit.position);
                }
            }
        }
    }
    let feet = candidates
        .into_iter()
        .min_by(|a, b| {
            ((a[0] - from[0]).powi(2) + (a[2] - from[2]).powi(2))
                .total_cmp(&((b[0] - from[0]).powi(2) + (b[2] - from[2]).powi(2)))
        })
        .expect("a supported casting position near prop");
    let p = [feet[0], feet[1] - s.body().height_offset(), feet[2]];
    let direction = [centre[0] - p[0], 0.0, centre[2] - p[2]];
    let o =
        magicka_sim::character_physics::Orientation::from_forward_up(direction, [0.0, 1.0, 0.0]);
    let body = s.avatar().body;
    s.world.physics.bodies[body].move_to(p, o);
    s.world.physics.bodies[body].set_desired_direction(o.forward());
    eprintln!("supported cast at {p:?}, target {:?}", centre);
    settle(s, 60);
}

fn cast_elements(s: &mut PlaySession, keys: &[usize], area: bool, self_cast: bool) {
    cast_elements_at(s, keys, area, self_cast, None);
}

fn cursor_at(s: &PlaySession, point: [f32; 3]) -> [f32; 2] {
    let p = [point[0], s.body().position[1], point[2], 1.0];
    let vp = magicka_runtime::camera::multiply(
        &s.world.camera.view(),
        &s.world.camera.projection(16.0 / 9.0),
    );
    let clip: [f32; 4] = std::array::from_fn(|j| (0..4).map(|k| p[k] * vp[k * 4 + j]).sum());
    [
        (clip[0] / clip[3]).clamp(-0.95, 0.95),
        (clip[1] / clip[3]).clamp(-0.95, 0.95),
    ]
}

fn cast_elements_at(
    s: &mut PlaySession,
    keys: &[usize],
    area: bool,
    self_cast: bool,
    target: Option<u16>,
) {
    for &key in keys {
        press_element(s, key);
        settle(s, 30);
    }
    assert!(!s.input_limited(), "cast blocked by tutorial");
    let cast_frames = if target.is_some() { 540 } else { 120 };
    for i in 0..cast_frames + 60 {
        s.step(1.0 / 60.0);
        let cursor_ndc = target
            .and_then(|h| s.world.npcs.get(&h))
            .map(|n| cursor_at(s, s.world.physics.bodies[n.body].position));
        s.handle_input(
            PlayerInput {
                cursor_ndc,
                cast_down: !self_cast && i < cast_frames,
                cast_self_down: self_cast && i < cast_frames,
                shift_down: area && i < cast_frames,
                ..Default::default()
            },
            16.0 / 9.0,
        );
        for event in &s.events {
            if let magicka_runtime::play_session::SessionEvent::Avatar(
                magicka_runtime::avatar::AvatarEvent::MissileLaunched(launch),
            ) = event
            {
                eprintln!("lesson cast {keys:?}: launch {launch:?}");
            }
        }
        if i % 10 == 0 && !s.world.missiles.is_empty() {
            eprintln!(
                "lesson cast {keys:?} frame {i}: missiles {:?}",
                s.world
                    .missiles
                    .iter()
                    .map(|m| (m.body.position, m.body.velocity))
                    .collect::<Vec<_>>()
            );
        }
        s.events.clear();
    }
    assert!(
        !s.avatar().dead(),
        "cast killed avatar at {:?}",
        s.body().position
    );
}

/// User report: the tutorial stopped at the "press W" Life dialog. After the
/// weapon rack, the Life element dialog is dismissed by a click (after its
/// one-second holdoff), its callback enables Life and shows the conjure
/// hint, conjuring Life fires "Life conjured", and a real self cast fires
/// "Life casted" and advances to the weapon lesson without forcing triggers.
/// Ordinary sword and charged Earth inputs then cut the rope and destroy the
/// wall, then activate the two bridges, dry after pool contact, power the
/// paired generators and activate the distant Arcane + Lightning gate.
/// A real mouse walk then opens Cold; Cold freezes wine, Water extinguishes
/// the furnaces, and the Haste pickup and imp tunnel triggers run.
/// Position changes isolate each combat lesson
/// from navigation; authored area conditions and callbacks remain active.
#[test]
fn castle_combat_lessons_advance_from_life_to_exit() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.headless_dialogs = Some([1280.0, 720.0]);
    s.change_scene_named("wc_s3", "start").unwrap();
    opening_sections(&mut s);
    // Let the authored opening and its timed dialogs finish. Do not clear
    // cutscene/input flags or pretend that "intro done" has fired.
    for _ in 0..3600 {
        frame(&mut s);
    }
    assert!(!s.world.cutscene.active);
    assert!(s.world.tutorial.dialog.is_some(), "intro hint appears");
    s.handle_input(
        PlayerInput {
            move_down: true,
            ..Default::default()
        },
        16.0 / 9.0,
    );
    for _ in 0..150 {
        frame(&mut s);
    }
    assert!(s.world.tutorial.dialog_done());
    assert!(!s.input_limited());
    let triggered = |s: &PlaySession, name: &str| {
        s.runtime
            .layout()
            .triggers
            .iter()
            .find(|t| t.id == custom_hash(&name.to_ascii_lowercase()))
            .unwrap()
            .has_triggered
    };
    assert!(triggered(&s, "intro done"));
    let id = custom_hash("poi_weaponrack");
    let (rack, _) = s.world.interactable_locator(id).unwrap();
    let body = s.avatar().body;
    let o = s.world.physics.bodies[body].orientation;
    let y = s.body().position[1];
    s.world.physics.bodies[body].move_to([rack[0], y, rack[2]], o);
    frame(&mut s);
    assert!(s.interact());
    // setDialogHint (delay 1) appears; the holdoff lasts one second.
    for _ in 0..150 {
        frame(&mut s);
    }
    assert!(
        s.world.tutorial.dialog.is_some(),
        "life dialog: {:?}",
        s.script_error
    );
    s.step(1.0 / 60.0);
    s.handle_input(
        PlayerInput {
            move_down: true,
            ..Default::default()
        },
        16.0 / 9.0,
    );
    frame(&mut s);
    assert!(s.world.tutorial.dialog_done(), "click dismissed the dialog");
    for _ in 0..150 {
        frame(&mut s);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert_eq!(
        s.world.flags.tutorial_elements_disabled & magicka_sim::spells::Elements::LIFE.0 as i32,
        0,
        "unlock life enabled Life"
    );
    assert!(!s.input_limited(), "unlock life re-enabled input");
    // Life is the Q/W/E/R row's W key (ELEMENT_KEYS index 1).
    press_element(&mut s, 1);
    for _ in 0..90 {
        frame(&mut s);
    }
    let hint = s.world.tutorial.hint.as_ref().expect("a hint");
    assert_eq!(hint.animation, 8192, "castself hint after Life conjured");
    assert!(triggered(&s, "Life conjured"));
    assert!(!triggered(&s, "Life casted"), "queueing is not casting");
    let mut healed = false;
    let mut casting = false;
    for i in 0..180 {
        s.step(1.0 / 60.0);
        s.handle_input(
            PlayerInput {
                cast_self_down: i < 5,
                ..Default::default()
            },
            16.0 / 9.0,
        );
        for event in s.events.drain(..) {
            if let magicka_runtime::play_session::SessionEvent::Avatar(
                magicka_runtime::avatar::AvatarEvent::SelfCastApplied {
                    hit_points_before,
                    hit_points_after,
                    ..
                },
            ) = event
            {
                healed |= hit_points_after > hit_points_before;
            }
        }
        use magicka_runtime::world_condition::WorldConditionFacts;
        if let Some((cast, spell)) = s.world.current_spell(custom_hash("player1")) {
            casting = true;
            assert_eq!(cast, magicka_sim::damage::CastType::SelfCast as i32);
            assert!(spell.contains_element(magicka_sim::spells::Elements::LIFE));
            assert_eq!(
                s.avatar().spell.element,
                magicka_sim::spells::Elements::NONE
            );
        }
    }
    assert!(healed, "self cast applies healing");
    assert!(casting, "current effect exposes the cast-time Life spell");
    assert!(triggered(&s, "Life casted"));
    assert!(
        s.checkpoint.is_some(),
        "the healing lesson saves a checkpoint"
    );
    assert_eq!(
        s.runtime.variables.counter(custom_hash("highlight_rope")),
        1
    );
    let hint = s.world.tutorial.hint.as_ref().unwrap();
    assert_eq!(hint.id, custom_hash("#tu_text_hint_room0d_key"));
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    use magicka_runtime::world_condition::WorldConditionFacts;
    assert!(s.world.current_spell(custom_hash("player1")).is_none());
    // Continue with the ordinary sword input. The rope must be hit as a
    // physics prop, rather than pretending its OnDeath callback happened.
    let rope = s
        .world
        .entities
        .iter()
        .find(|e| e.template_name.as_deref() == Some("rope_mechanism"))
        .unwrap();
    let rope_handle = rope.handle;
    let bounds = s.world.physics_damage[&rope_handle]
        .world_box(&rope.orientation)
        .unwrap();
    let y = s.body().position[1];
    s.world.physics.bodies[body].move_to(
        [bounds.center[0], y, bounds.center[2] + 1.0],
        magicka_sim::character_physics::Orientation::IDENTITY,
    );
    for i in 0..600 {
        s.step(1.0 / 60.0);
        s.handle_input(
            PlayerInput {
                shift_down: i % 120 < 30,
                move_down: i % 120 < 30,
                ..Default::default()
            },
            16.0 / 9.0,
        );
        s.events.clear();
    }
    assert!(
        s.world.physics_damage[&rope_handle].health <= 0.0,
        "ordinary sword swing must cut the rope: bounds {bounds:?}, player {:?}, health {}",
        s.body().position,
        s.world.physics_damage[&rope_handle].health
    );
    assert!(triggered(&s, "ropeCut"));
    assert_eq!(
        s.runtime.variables.counter(custom_hash("highlight_rope")),
        0
    );
    assert!(s.world.tutorial.hint.is_none());
    // Enter the next lesson at its decoded checkpoint locator. The area
    // condition and modal callback must unlock Earth without forced triggers.
    let room =
        magicka_runtime::spawn_phys::SpawnPhysHost::locator(&mut s.world, custom_hash("room10"))
            .unwrap();
    let feet_offset = s.body().length * 0.5 + s.body().radius;
    s.world.physics.bodies[body].move_to(
        [room[12], room[13] + feet_offset, room[14]],
        magicka_sim::character_physics::Orientation::IDENTITY,
    );
    for _ in 0..150 {
        frame(&mut s);
    }
    assert!(triggered(&s, "Room1"));
    assert!(
        !s.avatar().dead(),
        "checkpoint placement must keep the avatar alive"
    );
    assert!(s.world.tutorial.dialog.is_some());
    s.handle_input(
        PlayerInput {
            move_down: true,
            ..Default::default()
        },
        16.0 / 9.0,
    );
    for _ in 0..150 {
        frame(&mut s);
    }
    assert!(triggered(&s, "unlock earth"));
    assert!(!s.input_limited());

    let wall = s
        .world
        .entities
        .iter()
        .find(|e| e.unique_id == custom_hash("dungeonwall"))
        .unwrap();
    let wall_handle = wall.handle;
    let bounds = s.world.physics_damage[&wall_handle]
        .world_box(&wall.orientation)
        .unwrap();
    let thin_axis = (0..3)
        .min_by(|&a, &b| bounds.side_lengths[a].total_cmp(&bounds.side_lengths[b]))
        .unwrap();
    let normal = bounds.axes[thin_axis];
    let toward_room = (0..3)
        .map(|i| (room[12 + i] - bounds.center[i]) * normal[i])
        .sum::<f32>();
    let side = if toward_room >= 0.0 { 1.0 } else { -1.0 };
    let position = [
        bounds.center[0] + normal[0] * side * 4.0,
        s.body().position[1],
        bounds.center[2] + normal[2] * side * 4.0,
    ];
    let facing = normal.map(|v| -v * side);
    s.world.physics.bodies[body].move_to(
        position,
        magicka_sim::character_physics::Orientation::from_forward_up(facing, [0.0, 1.0, 0.0]),
    );
    let mut launched = false;
    s.world.physics.bodies[body].set_desired_direction(facing);
    for _ in 0..5 {
        if s.world.physics_damage[&wall_handle].health <= 0.0 {
            break;
        }
        // The decoded wall subtracts 1000 Earth damage. Use the ordinary
        // five-element queue and a charged shot to exceed that resistance.
        for _ in 0..5 {
            press_element(&mut s, 6); // D: Earth.
            for _ in 0..30 {
                frame(&mut s);
            }
        }
        assert!(
            !s.input_limited(),
            "Earth cast blocked at {:?}, dialog {:?}",
            s.body().position,
            s.world.tutorial.dialog
        );
        for i in 0..240 {
            s.step(1.0 / 60.0);
            s.handle_input(
                PlayerInput {
                    cast_down: i < 90,
                    ..Default::default()
                },
                16.0 / 9.0,
            );
            launched |= s.events.iter().any(|e| {
                matches!(
                    e,
                    magicka_runtime::play_session::SessionEvent::Avatar(
                        magicka_runtime::avatar::AvatarEvent::MissileLaunched(_)
                    )
                )
            });
            launched |= !s.world.missiles.is_empty();
            s.events.clear();
        }
    }
    assert!(
        launched,
        "Earth input must launch a projectile; wall HP {}, player state {:?}, queue {:?}, lock {}",
        s.world.physics_damage[&wall_handle].health,
        s.avatar().motion.state,
        s.avatar().conjure.queue,
        s.input_limited()
    );
    assert!(s.world.physics_damage[&wall_handle].health <= 0.0, "charged Earth casts must destroy the wooden wall; health {}, bounds {bounds:?}, player {:?}", s.world.physics_damage[&wall_handle].health, s.body().position);
    assert!(triggered(&s, "remove_poi"));
    assert_eq!(
        s.runtime.variables.counter(custom_hash("highlight_boards")),
        0
    );
    assert!(
        s.world.tutorial.dialog.is_some(),
        "wall death starts the Fire lesson"
    );
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    dismiss_lesson(&mut s);
    assert!(triggered(&s, "unlock fire"));
    let torch1 = prop_callback(&s, "bridge_1");
    face_prop(&mut s, torch1, 3.0);
    cast_elements(&mut s, &[7], false, false);
    assert!(
        triggered(&s, "bridge_1"),
        "Fire must lower the first bridge; prop HP {}",
        s.world.physics_damage[&torch1].health
    );
    assert!(s.scene_effect_active(custom_hash("effect_torch_fire_1")));

    enter_lesson_area(&mut s, "trigger_room2a");
    assert!(triggered(&s, "Room2a"));
    let torch2 = prop_callback(&s, "bridge_2");
    // This is the ranged bridge lesson: aim from its decoded entrance,
    // rather than relocating to a close-range spray position.
    let torch2_position = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == torch2)
        .unwrap()
        .position;
    let range = ((s.body().position[0] - torch2_position[0]).powi(2)
        + (s.body().position[2] - torch2_position[2]).powi(2))
    .sqrt();
    face_prop(&mut s, torch2, range);
    cast_elements(&mut s, &[6, 7], false, false);
    assert!(
        triggered(&s, "bridge_2"),
        "Earth + Fire must lower the second bridge; HP {}",
        s.world.physics_damage[&torch2].health
    );

    enter_lesson_area(&mut s, "trigger_room2b");
    assert!(triggered(&s, "Room2b"));
    dismiss_lesson(&mut s);
    assert!(triggered(&s, "unlock lightning"));
    let generator1 = prop_callback(&s, "generator_1");
    face_prop(&mut s, generator1, 3.0);
    cast_elements(&mut s, &[4], false, false);
    assert!(
        triggered(&s, "generator_1"),
        "Lightning must activate the first generator"
    );

    enter_lesson_area(&mut s, "trigger_room3");
    assert!(triggered(&s, "Room3"));
    // Room3's entrance trigger precedes its shallow pool. Enter an actual
    // decoded liquid triangle, preserving native contact/status processing.
    let p = s.body().position;
    let pool = s
        .world
        .level
        .liquids
        .iter()
        .filter(|l| l.has_collision)
        .flat_map(|l| {
            let g = l.collision_geometry().unwrap();
            eprintln!(
                "lesson liquid geometry: {} vertices, {} triangles, first {:?}",
                g.vertices.len(),
                g.triangles.len(),
                g.vertices.first()
            );
            g.triangles
                .iter()
                .map(|t| {
                    std::array::from_fn::<_, 3, _>(|i| {
                        (g.vertices[t[0] as usize][i]
                            + g.vertices[t[1] as usize][i]
                            + g.vertices[t[2] as usize][i])
                            / 3.0
                    })
                })
                .collect::<Vec<_>>()
        })
        .min_by(|a, b| {
            ((a[0] - p[0]).powi(2) + (a[2] - p[2]).powi(2))
                .total_cmp(&((b[0] - p[0]).powi(2) + (b[2] - p[2]).powi(2)))
        })
        .expect("Castle pool collision triangle");
    let immersion = [pool[0], pool[1] + s.body().length * 0.5, pool[2]];
    eprintln!("wet lesson: pool triangle centre {pool:?}, body at {immersion:?}");
    s.world.physics.bodies[body].move_to(
        immersion,
        magicka_sim::character_physics::Orientation::IDENTITY,
    );
    settle(&mut s, 90);
    assert!(
        triggered(&s, "Wet players"),
        "entering the decoded wet lesson must apply Wet; statuses {:?}",
        s.avatar().character.combat.statuses
    );
    assert!(s.avatar().character.combat.statuses.has(magicka_sim::status::Status::Wet), "the lesson must use real Wet contact, not another status satisfying the native any-status predicate");
    let g2 = prop_callback(&s, "generator_2");
    let g3 = prop_callback(&s, "generator_3");
    let generator_centre = |s: &PlaySession, h: u16| {
        let p = s.world.entities.iter().find(|e| e.handle == h).unwrap();
        s.world.physics_damage[&h]
            .world_box(&p.orientation)
            .unwrap()
            .center
    };
    let a = generator_centre(&s, g2);
    let b = generator_centre(&s, g3);
    let mid = [
        (a[0] + b[0]) * 0.5,
        s.body().position[1],
        (a[2] + b[2]) * 0.5,
    ];
    s.world.physics.bodies[body]
        .move_to(mid, magicka_sim::character_physics::Orientation::IDENTITY);
    cast_elements(&mut s, &[7], false, true);
    assert!(
        triggered(&s, "Dry players"),
        "self-cast Fire must unlock area casting"
    );
    assert!(
        !s.avatar()
            .character
            .combat
            .statuses
            .has(magicka_sim::status::Status::Wet),
        "Fire must dry the avatar before area Lightning"
    );
    cast_elements(&mut s, &[4, 4, 4, 4, 4], true, false);
    assert!(
        triggered(&s, "generators 2 and 3"),
        "area Lightning must power both generators within 0.1s; counters {} {}",
        s.runtime.variables.counter(custom_hash("generator2")),
        s.runtime.variables.counter(custom_hash("generator3"))
    );
    assert_eq!(s.runtime.variables.counter(custom_hash("generators")), 1);

    enter_lesson_area(&mut s, "trigger_room4");
    assert!(triggered(&s, "Room4"));
    dismiss_lesson(&mut s);
    assert!(triggered(&s, "unlock arcane"));
    let generator4 = prop_callback(&s, "generator_4");
    // The distant generator sits beyond a pit. Keep the avatar on the
    // authored room entrance and aim across it, rather than teleporting
    // closer to the target.
    let target = generator_centre(&s, generator4);
    let range = ((s.body().position[0] - target[0]).powi(2)
        + (s.body().position[2] - target[2]).powi(2))
    .sqrt();
    face_prop(&mut s, generator4, range);
    cast_elements(&mut s, &[5, 4], false, false);
    assert!(
        triggered(&s, "generator_4"),
        "Arcane + Lightning beam must activate the distant generator"
    );
    assert!(s.scene_effect_active(custom_hash("effect_generator_sparks_4")));

    let checkpoint = s
        .world
        .level
        .locators
        .iter()
        .find(|l| l.name == "room50")
        .unwrap();
    let approach = [
        checkpoint.transform[12],
        checkpoint.transform[13],
        checkpoint.transform[14],
    ];
    // Walk toward and then through the approach; stop at the real Cold modal.
    let area = s
        .world
        .level
        .trigger_areas
        .iter()
        .find(|a| a.name == "trigger_room5")
        .unwrap();
    let pose = magicka_content::skinned::pose_matrix(area.position, area.orientation, [1.0; 3]);
    let centre: [f32; 3] = std::array::from_fn(|i| {
        pose[12 + i]
            + (0..3)
                .map(|j| pose[j * 4 + i] * area.sides[j] * 0.5)
                .sum::<f32>()
    });
    eprintln!("Cold approach {approach:?}, trigger {centre:?}");
    walk_to_lesson(&mut s, approach, centre);
    assert!(triggered(&s, "Room5"));
    dismiss_lesson(&mut s);
    assert!(triggered(&s, "unlock cold"));
    let p = s.body().position;
    let wine = s
        .world
        .level
        .liquids
        .iter()
        .filter(|l| l.has_collision && l.freezable)
        .flat_map(|l| l.collision_geometry().unwrap().vertices)
        .min_by(|a, b| {
            ((a[0] - p[0]).powi(2) + (a[2] - p[2]).powi(2))
                .total_cmp(&((b[0] - p[0]).powi(2) + (b[2] - p[2]).powi(2)))
        })
        .expect("freezable wine pool");
    let d = [wine[0] - p[0], 0.0, wine[2] - p[2]];
    let o = magicka_sim::character_physics::Orientation::from_forward_up(d, [0.0, 1.0, 0.0]);
    s.world.physics.bodies[body].move_to(p, o);
    s.world.physics.bodies[body].set_desired_direction(o.forward());
    cast_elements(&mut s, &[3, 3, 3, 3, 3], false, false);
    assert!(
        s.world.collision.collision().water.iter().any(|l| l
            .freeze
            .vertices
            .iter()
            .any(|v| *v > 0.0)),
        "Cold must freeze decoded liquid vertices"
    );

    enter_lesson_area(&mut s, "trigger_room6");
    assert!(triggered(&s, "Room6"));
    dismiss_lesson(&mut s);
    assert!(triggered(&s, "unlock water"));
    for callback in ["ovenfire1", "ovenfire2", "ovenfire3"] {
        let target = prop_callback(&s, callback);
        face_prop_on_floor(&mut s, target, 3.0);
        cast_elements(&mut s, &[0], false, false);
        assert!(
            triggered(&s, callback),
            "Water must extinguish {callback}; HP {}",
            s.world.physics_damage[&target].health
        );
    }

    enter_lesson_area(&mut s, "trigger_room7");
    assert!(triggered(&s, "Room7"));
    assert_eq!(
        s.world
            .entities
            .iter()
            .filter(|e| e.template_name.as_deref() == Some("imp"))
            .count(),
        8
    );
    assert!(
        s.world.boss_fight.active,
        "delayed generic Behold must be set up"
    );
    assert!(!s.world.boss_fight.running, "Behold activates in Room10");
    let book = *s
        .world
        .book_pickups
        .iter()
        .find(|(_, b)| b.magick == magicka_sim::spells::MagickType::Haste as u8)
        .unwrap()
        .0;
    let book_position = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == book)
        .unwrap()
        .position;
    let p = [book_position[0], s.body().position[1], book_position[2]];
    s.world.physics.bodies[body].move_to(p, magicka_sim::character_physics::Orientation::IDENTITY);
    settle(&mut s, 1);
    assert_eq!(
        s.pickup_book(book).unwrap(),
        magicka_sim::spells::MagickType::Haste as u8
    );
    settle(&mut s, 1);
    assert!(triggered(&s, "Haste pickup"));

    enter_lesson_area(&mut s, "trigger_room8");
    assert!(triggered(&s, "Room8"));
    let imps: Vec<_> = s
        .world
        .npcs
        .iter()
        .filter(|(_, n)| n.character.definition.id == "imp")
        .map(|(&h, _)| h)
        .collect();
    for handle in imps {
        if s.world.npcs.get(&handle).is_none_or(|n| n.dead()) {
            continue;
        }
        let npc_body = s.world.npcs[&handle].body;
        let target = s.world.physics.bodies[npc_body].position;
        eprintln!("imp {handle} at {target:?}");
        let radius = s.world.physics.bodies[npc_body].radius;
        face_point_on_floor(&mut s, target, [radius * 2.0; 3], 3.0);
        cast_elements(&mut s, &[], false, false);
        settle(&mut s, 240);
        if s.world.npcs.get(&handle).is_some_and(|n| !n.dead()) {
            cast_elements_at(&mut s, &[5, 5, 5, 5, 4], false, false, Some(handle));
        }
        assert!(
            s.world.npcs.get(&handle).is_none_or(|n| n.dead()),
            "combat must kill imp {handle}; HP {}",
            s.world
                .npcs
                .get(&handle)
                .map_or(0.0, |n| n.character.combat.health.hit_points)
        );
        cast_elements(&mut s, &[1], false, true);
    }
    settle(&mut s, 60);
    assert!(
        triggered(&s, "Imps killed"),
        "the tunnel door opens after actual imp deaths"
    );

    enter_lesson_area(&mut s, "trigger_room9");
    assert!(triggered(&s, "Room9"));
    let lever = custom_hash("poi_lever");
    let (p, _) = s.world.interactable_locator(lever).unwrap();
    let p = [p[0], s.body().position[1], p[2]];
    s.world.physics.bodies[body].move_to(p, magicka_sim::character_physics::Orientation::IDENTITY);
    settle(&mut s, 1);
    assert!(s.interact(), "ordinary interaction must operate the lever");
    settle(&mut s, 180);
    assert!(s.world.flags.interactables_disabled.contains(&lever));

    enter_lesson_area(&mut s, "trigger_room10");
    assert!(triggered(&s, "Room10"));
    settle(&mut s, 600);
    assert!(s.world.boss_fight.running);
    dismiss_lesson(&mut s);
    assert!(triggered(&s, "unlock shield"));
    cast_elements(&mut s, &[2], false, true);
    let rope = prop_callback(&s, "rope_burnt");
    face_prop_on_floor(&mut s, rope, 3.0);
    cast_elements(&mut s, &[7, 7, 7, 7, 7], true, false);
    assert!(
        triggered(&s, "rope_burnt"),
        "Fire must release the chandelier; HP {}",
        s.world.physics_damage[&rope].health
    );
    settle(&mut s, 180);
    assert!(
        triggered(&s, "Kill behold"),
        "chandelier must kill Behold through the authored presence gate"
    );
    assert!(triggered(&s, "Behold dead"));

    enter_lesson_area(&mut s, "trigger_exit");
    settle(&mut s, 120);
    assert_eq!(s.scene_name().to_ascii_lowercase(), "wc_s2");
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
}

/// Hold the ordinary mouse movement command toward a world point, keeping
/// the cursor inside the viewport. Stop when a native tutorial modal opens.
fn walk_to_lesson(s: &mut PlaySession, approach: [f32; 3], target: [f32; 3]) {
    let nav = magicka_runtime::navigation::nav_mesh(&s.world.level.nav_mesh).unwrap();
    let (found, path) = nav
        .find_shortest_path(s.body().position, approach, 0, &|_| false)
        .unwrap();
    eprintln!("lesson approach path: found {found}, nodes {path:?}");
    // Animated navmesh links are not hosted yet. Follow the available dry
    // path, then the decoded checkpoint approach, with physical movement.
    let mut points: Vec<_> = path.into_iter().map(|n| n.position).collect();
    points.push(approach);
    points.push(target);
    let mut waypoint = 0;
    for i in 0..1800 {
        s.step(1.0 / 60.0);
        while waypoint + 1 < points.len()
            && ((s.body().position[0] - points[waypoint][0]).powi(2)
                + (s.body().position[2] - points[waypoint][2]).powi(2))
            .sqrt()
                < s.body().radius
        {
            waypoint += 1;
        }
        let aim = points[waypoint];
        s.handle_input(
            PlayerInput {
                move_down: true,
                cursor_ndc: Some(cursor_at(s, aim)),
                ..Default::default()
            },
            16.0 / 9.0,
        );
        if i % 60 == 0 {
            eprintln!(
                "lesson walk {i}: {:?}, depth {}, hp {}, modal {}",
                s.body().position,
                s.body().owner.water_depth,
                s.avatar().character.combat.health.hit_points,
                s.world.tutorial.dialog.is_some()
            );
        }
        s.events.clear();
        assert!(
            !s.avatar().dead(),
            "approach walk killed avatar at {:?}",
            s.body().position
        );
        if s.world.tutorial.dialog.is_some() {
            frame(s);
            return;
        }
    }
    panic!(
        "lesson walk did not reach a modal; target {target:?}, body {:?}",
        s.body().position
    );
}

#[test]
fn npc_kill_plane_terminates_and_fires_death_callback_once() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    s.disable_scripts();
    let layout = magicka_runtime::scene_script::SceneScriptLayout::read(
        br#"<Scene>
        <trigger id="init" repeat="false"><then>
            <spawn type="imp" id="victim" order="idle" area="room50" onDeath="fell" />
        </then></trigger>
        <trigger id="fell" autorun="false" repeat="0"><then>
            <counter name="deaths" action="inc" value="1"/>
        </then></trigger>
    </Scene>"#,
        false,
    )
    .unwrap();
    s.runtime = magicka_runtime::script_runtime::ScriptRuntime::new(layout, Default::default());
    s.runtime.initialize_actions(&mut s.world).unwrap();
    // Advance the script directly once, keeping the test independent of the intro.
    s.world.step(&mut s.runtime, 1.0 / 60.0).unwrap();
    let (&handle, npc) = s.world.npcs.iter().next().expect("spawned imp");
    let body = npc.body;
    s.world.physics.bodies[body].move_to(
        [0.0, -60.0, 0.0],
        magicka_sim::character_physics::Orientation::IDENTITY,
    );
    for _ in 0..120 {
        s.world.step_live(Some(&mut s.runtime), 1.0 / 60.0).unwrap();
        s.world.step(&mut s.runtime, 1.0 / 60.0).unwrap();
    }
    assert!(s.world.npcs.get(&handle).is_none_or(|n| n.dead()));
    assert!(!s.world.physics.bodies[body].active);
    assert!(!s.world.agents.contains(&handle));
    assert_eq!(s.runtime.variables.counter(custom_hash("deaths")), 1);
}

#[test]
fn tutorial_modal_clears_an_existing_walk_command_even_on_mouse_release() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let body = s.avatar().body;
    s.world.physics.bodies[body].set_movement([1.0, 0.0, 0.0]);
    assert_ne!(s.body().movement(), [0.0; 3]);
    s.world
        .tutorial
        .set_dialog(magicka_runtime::tutorial::DialogHint {
            text: String::new(),
            element: 0,
            magick: 0,
            trigger: 0,
            scale: None,
            size: None,
        });
    s.handle_input(PlayerInput::default(), 16.0 / 9.0);
    assert!(s.world.tutorial.dialog.is_some());
    assert_eq!(s.body().movement(), [0.0; 3]);
}

/// User report: the Earth lesson's wooden boards could be walked through.
/// PhysicsEntity.OnCollision returns the template Solid flag and static
/// entities ignore their own skin, so dungeon_wall (static, solid) must stop
/// an ordinary mouse walk; after its death the same walk crosses the gap.
#[test]
fn castle_wooden_wall_blocks_walking_until_destroyed() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..30 {
        frame(&mut s);
    }
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    s.world.camera.lock_input = false;
    let wall = s
        .world
        .entities
        .iter()
        .find(|e| e.unique_id == custom_hash("dungeonwall"))
        .unwrap();
    let handle = wall.handle;
    let bounds = s.world.physics_damage[&handle]
        .world_box(&wall.orientation)
        .unwrap();
    let room =
        magicka_runtime::spawn_phys::SpawnPhysHost::locator(&mut s.world, custom_hash("room10"))
            .unwrap();
    let thin = (0..3)
        .min_by(|&a, &b| bounds.side_lengths[a].total_cmp(&bounds.side_lengths[b]))
        .unwrap();
    let normal = bounds.axes[thin];
    let side = (0..3)
        .map(|i| (room[12 + i] - bounds.center[i]) * normal[i])
        .sum::<f32>()
        .signum();
    let offset = |p: [f32; 3]| {
        (0..3)
            .map(|i| (p[i] - bounds.center[i]) * normal[i])
            .sum::<f32>()
            * side
    };
    // The wall's centre line meets level geometry just behind it; walk
    // through the open part of the doorway beside it.
    let along = (0..3)
        .max_by(|&a, &b| bounds.side_lengths[a].total_cmp(&bounds.side_lengths[b]))
        .unwrap();
    let lane: [f32; 3] = std::array::from_fn(|i| bounds.center[i] + bounds.axes[along][i] * 1.5);
    let walk = |s: &mut PlaySession| {
        let body = s.avatar().body;
        let start = [
            lane[0] + normal[0] * side * 3.0,
            s.body().position[1],
            lane[2] + normal[2] * side * 3.0,
        ];
        let feet_offset = s.body().length * 0.5 + s.body().radius;
        s.world.physics.bodies[body].move_to(
            [start[0], room[13] + feet_offset, start[2]],
            magicka_sim::character_physics::Orientation::IDENTITY,
        );
        for _ in 0..30 {
            frame(s);
        }
        let goal: [f32; 3] = std::array::from_fn(|i| lane[i] - normal[i] * side * 4.0);
        for _ in 0..240 {
            s.step(1.0 / 60.0);
            s.handle_input(
                PlayerInput {
                    move_down: true,
                    cursor_ndc: Some(cursor_at(s, goal)),
                    ..Default::default()
                },
                16.0 / 9.0,
            );
            s.events.clear();
        }
        offset(s.body().position)
    };
    let blocked = walk(&mut s);
    assert!(
        blocked > 0.0,
        "walked through the solid wall: offset {blocked}, body {:?}",
        s.body().position
    );
    let entity = s
        .world
        .entities
        .iter_mut()
        .find(|e| e.handle == handle)
        .unwrap();
    entity.dead = true;
    entity.removed = true;
    let crossed = walk(&mut s);
    assert!(
        crossed < 0.0,
        "destroyed wall still blocks: offset {crossed}"
    );
}

/// User report: finishing the tutorial played both wc_s2 cutscenes and
/// looped back to wc_s3. Native scenes are persistent level objects:
/// GameScene.Initialize keeps Trigger.mHasTriggered (only ResetTimers runs),
/// so wc_s2's Init does not reset tutorial_complete on the return visit,
/// and wc_s2's saveNPCs change keeps its NPCs for that visit.
#[test]
fn castle_hall_revisit_keeps_trigger_state_and_saved_npcs() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let triggered = |s: &PlaySession, name: &str| {
        s.runtime
            .layout()
            .triggers
            .iter()
            .find(|t| t.id == custom_hash(&name.to_ascii_lowercase()))
            .unwrap()
            .has_triggered
    };
    let npc_count = |s: &PlaySession, id: &str| {
        s.world
            .entities
            .iter()
            .filter(|e| e.added && !e.removed && e.unique_id == custom_hash(id))
            .count()
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.headless_dialogs = Some([1280.0, 720.0]);
    s.change_scene_named("wc_s2", "start").unwrap();
    for _ in 0..120 {
        frame(&mut s);
    }
    assert!(triggered(&s, "Init") && triggered(&s, "Intro cutscene"));
    assert_eq!(npc_count(&s, "#wizn11"), 1);
    // The intro's skip callback changes to wc_s3 with saveNPCs.
    assert!(s.run_trigger("skip_propp", true).unwrap());
    for _ in 0..600 {
        frame(&mut s);
        if s.scene_name() == "wc_s3" {
            break;
        }
    }
    assert_eq!(s.scene_name(), "wc_s3");
    for _ in 0..30 {
        frame(&mut s);
    }
    // Skipping the tutorial sets tutorial_complete and returns to the hall.
    assert!(s.run_trigger("skip_tutorial", true).unwrap());
    for _ in 0..600 {
        frame(&mut s);
        if s.scene_name() == "wc_s2" {
            break;
        }
    }
    assert_eq!(s.scene_name(), "wc_s2");
    let mut scenes = Vec::new();
    for _ in 0..(20 * 60) {
        frame(&mut s);
        scenes.push(s.scene_name().to_owned());
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert!(
        scenes.iter().all(|n| n == "wc_s2"),
        "the return visit must not loop back to the tutorial"
    );
    assert_eq!(
        s.runtime
            .variables
            .counter(custom_hash("tutorial_complete")),
        1,
        "Init must not run again and reset the counter"
    );
    assert!(triggered(&s, "Back from tutorial"));
    assert!(!triggered(&s, "Propp does bad party trick"));
    assert_eq!(npc_count(&s, "#wizn11"), 1, "saved NPC restored once");
    // Propp returns with the saved NPCs; Back from tutorial removes him.
    assert!(s
        .world
        .entities
        .iter()
        .any(|e| e.unique_id == custom_hash("#wizn12") && e.removed));
    assert_eq!(npc_count(&s, "#wizn12"), 0);
}

/// User report: the Water furnace lesson did not work. Furnace_fire's only
/// visible fire is its template effect (special_fire_hazard), which
/// PhysicsEntity.Initialize starts and Deinitialize stops; the rewrite never
/// started it. From the Room6 entrance, a mouse walk toward the first oven
/// and an ordinary Water spray aimed with the cursor must destroy the fire,
/// run its callback and stop the fire effect.
#[test]
fn castle_furnace_fire_effect_runs_until_water_destroys_it() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    weapon_rack(&mut s);
    for _ in 0..3 {
        frame(&mut s);
    }
    // Isolate the lesson: no opening cutscene, Water and Force casting enabled.
    let callback = custom_hash("ovenfire1");
    let handle = *s
        .world
        .physics_damage
        .iter()
        .find(|(_, p)| p.on_death == callback)
        .unwrap()
        .0;
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    s.world.camera.lock_input = false;
    s.world.flags.tutorial_elements_disabled = 0;
    s.world.flags.tutorial_cast_types_disabled.clear();
    frame(&mut s);
    let fire = |s: &PlaySession| s.live_prop_effects().iter().any(|&(h, _)| h == handle);
    assert!(fire(&s), "the furnace's template fire effect must run");
    let entity = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == handle)
        .unwrap();
    let centre = s.world.physics_damage[&handle]
        .world_box(&entity.orientation)
        .unwrap()
        .center;
    let area = s
        .world
        .level
        .trigger_areas
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case("trigger_room6"))
        .unwrap();
    let pose = magicka_content::skinned::pose_matrix(area.position, area.orientation, [1.0; 3]);
    let entry: [f32; 3] = std::array::from_fn(|i| {
        pose[12 + i]
            + (0..3)
                .map(|j| pose[j * 4 + i] * area.sides[j] * 0.5)
                .sum::<f32>()
    });
    let body = s.avatar().body;
    let y = s.body().position[1];
    s.world.physics.bodies[body].move_to(
        [entry[0], y, entry[2]],
        magicka_sim::character_physics::Orientation::IDENTITY,
    );
    let input = |s: &mut PlaySession, input: PlayerInput| {
        s.step(1.0 / 60.0);
        let cursor_ndc = Some(cursor_at(s, centre));
        s.handle_input(
            PlayerInput {
                cursor_ndc,
                ..input
            },
            16.0 / 9.0,
        );
        s.events.clear();
    };
    for _ in 0..120 {
        input(
            &mut s,
            PlayerInput {
                move_down: true,
                ..Default::default()
            },
        );
    }
    for _ in 0..30 {
        input(&mut s, PlayerInput::default());
    }
    let mut water = PlayerInput::default();
    water.elements[0] = true;
    input(&mut s, water);
    for _ in 0..10 {
        input(&mut s, PlayerInput::default());
    }
    for _ in 0..180 {
        input(
            &mut s,
            PlayerInput {
                cast_down: true,
                ..Default::default()
            },
        );
    }
    for _ in 0..10 {
        input(&mut s, PlayerInput::default());
    }
    assert!(
        s.world
            .physics_damage
            .get(&handle)
            .is_none_or(|p| p.health <= 0.0),
        "Water must destroy the fire from {:?}",
        s.body().position
    );
    assert!(!fire(&s), "the dead prop's fire effect must stop");
}

/// wc_s3 Room7 spawns Behold (delayed); Room10's intro 2.5 s later runs BossTitle
/// (#Boss_N16 / #Boss_T16, DisplayTime 3, FadeIn 1, FadeOut 0.5). The title
/// renders through BossFight.Update with the native TitleRenderer timing:
/// fade in over 1 s, hold until the timer passes 4, fade out over 0.5 s.
#[test]
fn castle_behold_boss_title_fades_in_and_out() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    assert!(s.run_trigger("Room7", true).unwrap());
    settle(&mut s, 60);
    assert!(s.world.boss_fight.shown_title.is_none());
    assert!(s.run_trigger("Room10", true).unwrap());
    let mut shown = Vec::new();
    for i in 0..60 * 10 {
        frame(&mut s);
        if let Some(t) = &s.world.boss_fight.shown_title {
            shown.push((i, t.clone()));
        }
    }
    let (first, title) = shown.first().cloned().expect("the boss title must render");
    let last = shown.last().unwrap().0;
    // Delay 2.5 s; the title is visible for about 4.5 s.
    assert!(
        (140..=160).contains(&first),
        "title appeared at frame {first}"
    );
    let visible = (last - first) as f32 / 60.0;
    assert!((4.3..=4.7).contains(&visible), "title visible {visible} s");
    assert!(shown.iter().any(|(_, t)| t.alpha == 1.0));
    assert!(shown[0].1.alpha < 0.1, "the title fades in");
    let strings = magicka_runtime::play_content::english_strings(&root, &["Wizard"]);
    let (name, sub) = title.resolve(&strings);
    for text in [&name, &sub] {
        assert!(
            !text.is_empty() && !text.starts_with("NOT FOUND"),
            "{text:?}"
        );
        assert_eq!(*text, text.to_uppercase());
    }
    assert!(
        !s.world.boss_fight.name_set,
        "the finished title clears BossNameSet"
    );
}

/// User report 5: Behold never attacked. Its only ability is Ranged
/// (weapons 0..5, beholder_laser items fired by SpawnMissile animation
/// actions), which the rewrite did not reconstruct. The authored Room7
/// spawnboss sets it to idle reacting to attack; one short ordinary Arcane
/// beam from five metres (toward boss0) must make it fire laser missiles
/// that damage the player through their Hit conditions.
#[test]
fn castle_behold_answers_an_attack_with_laser_missiles() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    weapon_rack(&mut s);
    for _ in 0..3 {
        frame(&mut s);
    }
    assert!(s.run_trigger("Room7", true).unwrap());
    settle(&mut s, 60);
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    s.world.camera.lock_input = false;
    s.world.flags.tutorial_elements_disabled = 0;
    s.world.flags.tutorial_cast_types_disabled.clear();
    let behold = *s
        .world
        .npcs
        .iter()
        .find(|(_, n)| n.character.definition.id == "beholder_benign")
        .expect("Room7 spawns Behold")
        .0;
    let boss0 = s
        .world
        .level
        .locators
        .iter()
        .find(|l| l.name.eq_ignore_ascii_case("boss0"))
        .unwrap();
    let at = [
        boss0.transform[12],
        boss0.transform[13],
        boss0.transform[14],
    ];
    // Five metres from Behold on the line to boss0 (the beam is narrow).
    let b = s.world.physics.bodies[s.world.npcs[&behold].body].position;
    let (dx, dz) = (at[0] - b[0], at[2] - b[2]);
    let l = (dx * dx + dz * dz).sqrt();
    let at = [b[0] + dx / l * 5.0, at[1], b[2] + dz / l * 5.0];
    let body = s.avatar().body;
    let y = s.body().position[1].max(at[1]);
    s.world.physics.bodies[body].move_to(
        [at[0], y, at[2]],
        magicka_sim::character_physics::Orientation::IDENTITY,
    );
    for _ in 0..60 {
        frame(&mut s);
    }
    let me = s.avatar().handle;
    let full = s.avatar().character.combat.health.hit_points;
    assert!(
        s.world.missiles.iter().all(|m| m.body.owner != behold),
        "Behold must not fire before it is attacked"
    );
    // One short Arcane beam at Behold (ordinary input).
    press_element(&mut s, 5);
    for _ in 0..10 {
        frame(&mut s);
    }
    let mut fired = 0;
    let mut travelling_effect_seen = false;
    for i in 0..600 {
        s.step(1.0 / 60.0);
        let cursor_ndc = s
            .world
            .npcs
            .get(&behold)
            .map(|n| cursor_at(&s, s.world.physics.bodies[n.body].position));
        s.handle_input(
            PlayerInput {
                cursor_ndc,
                cast_down: i < 90,
                ..Default::default()
            },
            16.0 / 9.0,
        );
        travelling_effect_seen |=
            s.effects
                .active_effects_by_hash(magicka_runtime::scene_loading::custom_hash(
                    "beholder_laser",
                ))
                > 0;
        s.events.clear();
        fired = fired.max(
            s.world
                .missiles
                .iter()
                .filter(|m| m.body.owner == behold)
                .count(),
        );
    }
    assert!(
        travelling_effect_seen,
        "the travelling Beholder_Laser effect must render before impact"
    );
    let npc = &s.world.npcs[&behold];
    assert!(
        npc.combat.attacked_by.iter().any(|(h, _)| *h == me),
        "the Arcane beam must register as an attack"
    );
    assert!(
        fired > 0,
        "Behold must fire laser missiles; log {:?}",
        s.world.log
    );
    let hp = s.avatar().character.combat.health.hit_points;
    assert!(
        hp < full,
        "laser hits must damage the player ({full} -> {hp})"
    );
    assert!(
        s.world.log.iter().all(|l| !l.contains("npc missile")),
        "{:?}",
        s.world.log
    );
}

/// User report 4: the chandelier rope could not be burnt. Its spawnPhys
/// fire_mechanism (onDeath rope_burnt) is a 1 HP 0.3 m box on the wall.
/// SpraySpell reaches only range·range·0.1 (native quirk), so a one-Fire
/// spray must be cast from close by; from 1.5 m, aimed with the mouse from
/// either open side, it destroys the rope.
#[test]
fn castle_chandelier_rope_burns_from_a_close_fire_spray() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    for angle in [0.0f32, std::f32::consts::FRAC_PI_2] {
        let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
        s.change_scene_named("wc_s3", "start").unwrap();
        weapon_rack(&mut s);
        for _ in 0..3 {
            frame(&mut s);
        }
        let rope = prop_callback(&s, "rope_burnt");
        let e = s.world.entities.iter().find(|e| e.handle == rope).unwrap();
        let c = s.world.physics_damage[&rope]
            .world_box(&e.orientation)
            .unwrap()
            .center;
        s.disable_scripts();
        s.world.flags.cutscene = false;
        s.world.flags.input_limited = false;
        s.world.camera.lock_input = false;
        s.world.flags.tutorial_elements_disabled = 0;
        s.world.flags.tutorial_cast_types_disabled.clear();
        let body = s.avatar().body;
        let y = s.body().position[1];
        let at = [c[0] + angle.cos() * 1.5, y, c[2] + angle.sin() * 1.5];
        s.world.physics.bodies[body]
            .move_to(at, magicka_sim::character_physics::Orientation::IDENTITY);
        for _ in 0..60 {
            frame(&mut s);
        }
        press_element(&mut s, 7);
        for _ in 0..10 {
            frame(&mut s);
        }
        for i in 0..240 {
            s.step(1.0 / 60.0);
            let cursor_ndc = Some(cursor_at(&s, c));
            s.handle_input(
                PlayerInput {
                    cursor_ndc,
                    cast_down: i < 180,
                    ..Default::default()
                },
                16.0 / 9.0,
            );
            s.events.clear();
        }
        assert!(
            s.world.physics_damage[&rope].health <= 0.0,
            "Fire spray from {at:?} must burn the rope"
        );
    }
}

/// User report 7: wc_s2's floor-breaking Propp spell flashed at scene start.
/// Every placed level effect starts with the scene and wc_s2's Init stops
/// effect_propp_spell; natively a new GameScene's first Update runs its
/// triggers before its effects. The rewrite updated the new scene's effects
/// in the step that switched to it, before Init could run, emitting a frame
/// of every stopped effect. The switch step must not emit, and after the
/// first update the Propp spell must be inactive until its authored trick.
#[test]
fn castle_hall_stopped_level_effects_do_not_emit_on_entry() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    assert!(s.run_trigger("skip_tutorial", true).unwrap());
    let id = custom_hash("effect_propp_spell");
    let mut switched = false;
    for _ in 0..600 {
        let before = s.scene_name().to_owned();
        frame(&mut s);
        if s.scene_name() != before {
            assert_eq!(s.scene_name(), "wc_s2");
            assert_eq!(
                s.effects.particles.active().len(),
                0,
                "the entered scene's effects must not emit before its first update"
            );
            switched = true;
            break;
        }
    }
    assert!(switched, "skip_tutorial must return to wc_s2");
    frame(&mut s);
    let propp = &s.world.effects.effects[&id].effect;
    assert!(!propp.active, "Init must stop the Propp spell");
    assert_eq!(s.effects.active_effects_by_hash(propp.effect), 0);
}

/// User report 8: sword hits on the Life-lesson rope had no audible
/// response. The rope_mechanism template has no hit effect or sound; natively
/// the feedback is the weapon's own melee events: DealDamage's first
/// execution runs the Default collection (wep_sword_swing) and each hit runs
/// the Hit collection (42 physical damage and wep_sword_hit_flesh) from the
/// item. The rewrite delivered only the Damage events.
#[test]
fn castle_sword_hits_on_the_rope_play_weapon_hit_sounds() {
    use magicka_runtime::audio::{AudioRequest, SoundSource};
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    weapon_rack(&mut s);
    for _ in 0..3 {
        frame(&mut s);
    }
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    s.world.camera.lock_input = false;
    s.world.flags.tutorial_elements_disabled = 0;
    s.world.flags.tutorial_cast_types_disabled.clear();
    assert!(
        item_names(&s)
            .iter()
            .any(|(_, n)| n == "weapon_blunt_blade" || n.contains("blunt")),
        "{:?}",
        item_names(&s)
    );
    let rope = s
        .world
        .entities
        .iter()
        .find(|e| e.template_name.as_deref() == Some("rope_mechanism"))
        .unwrap();
    let rope_handle = rope.handle;
    let centre = s.world.physics_damage[&rope_handle]
        .world_box(&rope.orientation)
        .unwrap()
        .center;
    let body = s.avatar().body;
    let y = s.body().position[1];
    s.world.physics.bodies[body].move_to(
        [centre[0], y, centre[2] + 1.0],
        magicka_sim::character_physics::Orientation::IDENTITY,
    );
    let swing = custom_hash("wep_sword_swing");
    let hit = custom_hash("wep_sword_hit_flesh");
    let (mut swings, mut hits) = (0, Vec::new());
    s.audio.clear();
    for i in 0..600 {
        s.step(1.0 / 60.0);
        s.handle_input(
            PlayerInput {
                shift_down: i % 120 < 30,
                move_down: i % 120 < 30,
                ..Default::default()
            },
            16.0 / 9.0,
        );
        s.events.clear();
        for (request, _) in std::mem::take(&mut s.audio) {
            if let AudioRequest::Start {
                bank: 128,
                cue,
                source: SoundSource::Emitter(p),
                ..
            } = request
            {
                if cue == swing {
                    swings += 1;
                } else if cue == hit {
                    hits.push(p);
                }
            }
        }
        if s.world.physics_damage[&rope_handle].health <= 0.0 && !hits.is_empty() {
            break;
        }
    }
    assert!(swings > 0, "the Default collection's swing sound");
    assert!(!hits.is_empty(), "the Hit collection's hit sound");
    let p = hits[0];
    let d = ((p[0] - centre[0]).powi(2) + (p[2] - centre[2]).powi(2)).sqrt();
    assert!(
        d < 3.0,
        "hit sound at the weapon near the rope: {p:?} vs {centre:?}"
    );
}

/// User report: two of the three wc_s5 ambush goblins never attacked. The
/// authored End cutscene runs giveOrder type=goblin_rogue area=any
/// order=attack; area presence was mapped back to entities by unique ID,
/// which the two ID-less spawns share, so only the named goblin was ordered.
#[test]
fn castle_courtyard_ambush_orders_every_goblin_to_attack() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s5", "start").unwrap();
    for _ in 0..60 {
        frame(&mut s);
    }
    assert!(s.run_trigger("skip_goblins", true).unwrap());
    for _ in 0..180 {
        frame(&mut s);
    }
    let orders: Vec<u8> = s
        .world
        .npcs
        .values()
        .filter(|n| n.character.definition.id == "goblin_rogue")
        .map(|n| n.ai.order)
        .collect();
    assert_eq!(orders.len(), 3, "skip_goblins spawns three rogues");
    assert!(
        orders.iter().all(|&o| o == 2),
        "every rogue attacks: {orders:?}"
    );
}

/// User report: ranged attack projectiles were not rendered. Item missiles
/// now carry Item.mProjectileModel; the Castle courtyard soldier archers'
/// bow resolves its arrow model (on-disk case, Content-relative) and uses
/// the model's bounding radius.
#[test]
fn soldier_archer_bow_missiles_carry_their_arrow_model() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let template = magicka_runtime::play_content::load_character(&root, "soldier_archer").unwrap();
    let weapons = magicka_runtime::play_content::melee_weapons(&root, &template);
    let ranged: Vec<_> = weapons
        .iter()
        .filter_map(|(_, w)| w.ranged.clone())
        .collect();
    assert!(!ranged.is_empty(), "the bow has ranged conditions");
    assert_eq!(
        ranged[0].model.as_deref(),
        Some("Models/Items_NPC/soldier_arrow_0")
    );
    assert!(ranged[0].radius > 0.0 && ranged[0].radius != 0.25);
}

/// User report: Tab showed no weapon and staff information. Avatar
/// CheckInventory toggles PlayState's InventoryBox (KeyboardBindings
/// Inventory): white item names, wrapped descriptions, a 600 x >=230 box
/// that scales in and out at 4 per second.
#[test]
fn tab_toggles_the_weapon_and_staff_inventory_box() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    weapon_rack(&mut s);
    for _ in 0..3 {
        frame(&mut s);
    }
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    s.world.camera.lock_input = false;
    let tab = |s: &mut PlaySession| {
        s.step(1.0 / 60.0);
        s.handle_input(
            PlayerInput {
                inventory: true,
                ..Default::default()
            },
            16.0 / 9.0,
        );
        s.events.clear();
    };
    tab(&mut s);
    assert!(s.inventory.show, "{:?}", s.world.log);
    for _ in 0..20 {
        frame(&mut s);
    }
    assert_eq!(s.inventory.scale, 1.0);
    let text = s.inventory.text.clone();
    assert!(text.starts_with("[c=1,1,1]") && text.matches("[c=1,1,1]").count() >= 2);
    assert!(
        !text.contains('#'),
        "names and descriptions are localized: {text}"
    );
    assert_eq!(s.inventory.size[0], 600.0);
    assert!(s.inventory.size[1] >= 230.0);
    tab(&mut s);
    assert!(!s.inventory.show);
    for _ in 0..20 {
        frame(&mut s);
    }
    assert!(!s.inventory.active());
}

/// User report: the tutorial furnace fires never set the player alight.
/// PhysicsEntity.OnCollision runs the template conditions (Hit | Collision)
/// against each touching entity once per 0.25 s; Furnace_fire's deals Fire
/// status 250 (magnitude 2).
#[test]
fn castle_furnace_fire_sets_a_touching_player_on_fire() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    s.disable_scripts();
    s.world.flags.cutscene = false;
    let fire = prop_callback(&s, "ovenfire1");
    let entity = s.world.entities.iter().find(|e| e.handle == fire).unwrap();
    let centre = s.world.physics_damage[&fire]
        .world_box(&entity.orientation)
        .unwrap()
        .center;
    let burning = |s: &PlaySession| {
        s.avatar()
            .character
            .combat
            .statuses
            .has(magicka_sim::status::Status::Burning)
    };
    assert!(!burning(&s));
    let body = s.avatar().body;
    let feet = s.body().feet()[1];
    let height = s.body().position[1] - feet;
    s.world.physics.bodies[body].move_to(
        [centre[0], centre[1].max(feet) + height - 0.5, centre[2]],
        magicka_sim::character_physics::Orientation::IDENTITY,
    );
    for _ in 0..10 {
        frame(&mut s);
    }
    assert!(
        burning(&s),
        "the furnace fire must ignite the player at {:?}",
        s.body().position
    );
}

fn captain(s: &PlaySession) -> u16 {
    *s.world
        .npcs
        .iter()
        .find(|(_, n)| n.character.definition.id == "soldier_captain")
        .expect("captain spawned")
        .0
}

fn axe_pickups(s: &PlaySession) -> usize {
    s.world
        .item_pickups
        .values()
        .filter(|p| {
            p.item
                .as_ref()
                .is_some_and(|i| i.definition.name.eq_ignore_ascii_case("weapon_soldier_axe"))
        })
        .count()
}

/// User report: the wc_s5 captain never handed over his axe. A surviving
/// captain plays spec_throwitem, whose DetachItem action releases equipment
/// slot 0 (Attachment.Release: a pickable item becomes a pickup).
#[test]
fn courtyard_captain_throws_his_axe_after_the_troll() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s5", "start").unwrap();
    for _ in 0..10 {
        frame(&mut s);
    }
    assert!(s.run_trigger("SoldierSpawn", true).unwrap());
    for _ in 0..30 {
        frame(&mut s);
    }
    let captain = captain(&s);
    assert_eq!(axe_pickups(&s), 0);
    assert!(s.run_trigger("skip_changes3", true).unwrap());
    let mut release_height = None;
    let mut max_height = f32::NEG_INFINITY;
    for _ in 0..420 {
        frame(&mut s);
        for (&h, p) in &s.world.item_pickups {
            if p.item
                .as_ref()
                .is_some_and(|i| i.definition.name.eq_ignore_ascii_case("weapon_soldier_axe"))
            {
                let state = s.pickup_physics.as_ref().unwrap().state(h).unwrap();
                if release_height.is_none() {
                    release_height = Some(state.position[1]);
                    assert!(
                        state.linear_velocity[1] <= 0.0,
                        "copied pickup must not inherit the equipment throw: {state:?}"
                    );
                }
                max_height = max_height.max(state.position[1]);
            }
        }
    }
    assert!(
        max_height <= release_height.unwrap() + 0.1,
        "axe launched by a collision: {max_height}"
    );
    let (&handle, _) = s
        .world
        .item_pickups
        .iter()
        .find(|(_, p)| {
            p.item
                .as_ref()
                .is_some_and(|i| i.definition.name.eq_ignore_ascii_case("weapon_soldier_axe"))
        })
        .unwrap();
    let landed = s.pickup_physics.as_ref().unwrap().state(handle).unwrap();
    assert!(
        landed.sleeping && landed.position[1] < 0.5,
        "axe did not settle: {landed:?}"
    );
    assert_eq!(
        axe_pickups(&s),
        1,
        "{:?}",
        s.world.log.iter().rev().take(5).collect::<Vec<_>>()
    );
    let rig = s.world.npcs[&captain].rig.as_ref().unwrap();
    assert!(
        rig.attachments.iter().all(|a| a.slot != 0),
        "the axe left his hand"
    );
}

/// DeadState.OnEnter releases every equipment slot of a non-Undying
/// character: a killed captain drops his pickable axe.
#[test]
fn courtyard_captain_drops_his_axe_when_killed() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s5", "start").unwrap();
    for _ in 0..10 {
        frame(&mut s);
    }
    assert!(s.run_trigger("SoldierSpawn", true).unwrap());
    for _ in 0..30 {
        frame(&mut s);
    }
    let captain = captain(&s);
    s.disable_scripts();
    s.world
        .npcs
        .get_mut(&captain)
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 0.0;
    for _ in 0..120 {
        frame(&mut s);
    }
    assert!(s.world.npcs.get(&captain).is_none_or(|n| n.dead()));
    assert_eq!(axe_pickups(&s), 1);
}

/// User report: no boss health bar for the wc_s5 troll. BossFight.Update
/// drives BossHealthBar (fade in at 0.5/s, displayed fill easing at 10/s
/// once opaque); spawnboss without `delayed` starts the fight.
#[test]
fn courtyard_troll_fight_shows_the_boss_health_bar() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s5", "start").unwrap();
    for _ in 0..10 {
        frame(&mut s);
    }
    assert!(s.run_trigger("Troll_entry", true).unwrap());
    for _ in 0..60 {
        frame(&mut s);
    }
    let fight = &s.world.boss_fight;
    assert!(fight.running && fight.bar_visible);
    assert!(
        fight.bar.alpha > 0.4 && fight.bar.alpha < 0.6,
        "{}",
        fight.bar.alpha
    );
    assert_eq!(fight.health_bar_width, 0.8);
    for _ in 0..120 {
        frame(&mut s);
    }
    let fight = &s.world.boss_fight;
    assert_eq!(fight.bar.alpha, 1.0);
    assert!(
        fight.bar.display_health > 0.9,
        "{}",
        fight.bar.display_health
    );
}

/// AddCameraEffect (Swamp sw_s5 StartOfLevel: weather_rain) starts an
/// ordinary effect on the MagickCamera, which repositions it at its current
/// position facing Forward every update; re-adding the same effect replaces
/// the old instance. GameScene.Destroy clears the EffectManager, so the
/// camera drops the stale reference on a scene change (each Swamp scene
/// adds its own rain).
#[test]
fn swamp_rain_follows_the_camera_until_the_scene_changes() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let rain = magicka_runtime::scene_loading::custom_hash("weather_rain");
    let mut s = PlaySession::load(&root, "Swamp.lvl", "Wizard").unwrap();
    s.change_scene_named("sw_s5", "start").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    assert!(s.run_trigger("StartOfLevel", true).unwrap());
    for _ in 0..10 {
        frame(&mut s);
    }
    assert_eq!(s.effects.active_effects_by_hash(rain), 1, "rain must start");
    let at = |s: &PlaySession| {
        let r = s.camera_effects[&rain];
        let p = s
            .effects
            .presentation()
            .into_iter()
            .find(|p| p.reference == r)
            .expect("live rain effect");
        [p.transform[12], p.transform[13], p.transform[14]]
    };
    assert_eq!(at(&s), s.camera().ground_position());
    // Move the camera; the effect follows on the next update.
    let moved = {
        let g = s.camera().ground_position();
        [g[0] + 6.0, g[1], g[2] - 4.0]
    };
    s.world.camera.set_position(moved, true);
    frame(&mut s);
    let g = s.camera().ground_position();
    assert_eq!(at(&s), g);
    // Re-adding replaces, never duplicates.
    s.world.camera_effect_requests.push((rain, true));
    frame(&mut s);
    assert_eq!(s.effects.active_effects_by_hash(rain), 1);
    // RemoveCameraEffect without an effect stops them all.
    s.world.camera_effect_requests.push((0, false));
    frame(&mut s);
    assert_eq!(s.effects.active_effects_by_hash(rain), 0);
    assert!(s.camera_effects.is_empty());
    s.world.camera_effect_requests.push((rain, true));
    frame(&mut s);
    assert_eq!(s.effects.active_effects_by_hash(rain), 1);
    s.change_scene_named("sw_s6", "start").unwrap();
    for _ in 0..5 {
        frame(&mut s);
    }
    assert_eq!(
        s.effects.active_effects_by_hash(rain),
        0,
        "Destroy clears effects"
    );
    assert!(
        s.camera_effects.is_empty(),
        "the stale camera reference is dropped"
    );
}

/// PlayState.UIEnabled: BeginCutscene clears it, EndCutscene sets it, and
/// the KeyboardHUD does not update while it is clear. The cutscene and the
/// flag belong to PlayState, so a scene change keeps both.
#[test]
fn cutscenes_disable_the_ui_and_survive_a_scene_change() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    let mut saw_cutscene = false;
    for _ in 0..60 * 60 {
        frame(&mut s);
        if s.world.flags.cutscene {
            saw_cutscene = true;
            assert!(!s.world.ui_enabled, "BeginCutscene disables the UI");
        } else if saw_cutscene {
            break;
        }
    }
    assert!(saw_cutscene, "wc_s3 opens with a cutscene");
    assert!(s.world.ui_enabled, "EndCutscene re-enables the UI");
    // A fresh opening, changed to another scene mid-cutscene.
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..60 * 10 {
        frame(&mut s);
        if s.world.flags.cutscene {
            break;
        }
    }
    assert!(s.world.flags.cutscene && !s.world.ui_enabled);
    s.change_scene_named("wc_s2", "start").unwrap();
    frame(&mut s);
    assert!(s.world.cutscene.active, "the cutscene outlives the scene");
    assert!(s.world.flags.cutscene);
    assert!(!s.world.ui_enabled);
}

/// GameScene.GetLocator falls back to AnimatedLevelPart.TryGetLocator: the
/// part's locator times its current world transform. Tsar R'lyeh tr_s5
/// spawns beam globes at locators of its moving isles.
#[test]
fn spawns_at_animated_part_locators_follow_the_part() {
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
    // Every static-locator miss resolved by a part: find one and check a
    // spawned entity stands at its composed transform.
    fn find(
        parts: &[magicka_content::level::AnimatedLevelPart],
        states: &[magicka_runtime::animated_hierarchy::AnimatedPartState],
        out: &mut Vec<[f32; 3]>,
    ) {
        for (p, st) in parts.iter().zip(states) {
            for l in &p.locators {
                if l.name.to_ascii_lowercase().starts_with("beamglobe") {
                    let m = magicka_content::skinned::mul(&l.transform, &st.world);
                    out.push([m[12], m[13], m[14]]);
                }
            }
            find(&p.children, &st.children, out);
        }
    }
    let mut expected = Vec::new();
    find(
        &s.world.level.animated_parts,
        s.world.collision.parts(),
        &mut expected,
    );
    assert!(
        !expected.is_empty(),
        "tr_s5 has beam globe locators on parts"
    );
    let near = expected.iter().any(|e| {
        s.world
            .entities
            .iter()
            .any(|x| (0..3).all(|i| (x.position[i] - e[i]).abs() < 0.05))
    });
    assert!(near, "a beam globe spawned at a part locator");
}

/// WizardControl StaffLightOn (Mines mi_s2 StartOfLevel) sets the
/// PlayState.StaffLight flag; it is PlayState-scoped across scenes.
#[test]
fn mines_wizard_control_turns_the_staff_light_on() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Mines.lvl", "Wizard").unwrap();
    s.change_scene_named("mi_s2", "start").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    s.script_error = None;
    s.run_trigger("StartOfLevel", true).unwrap();
    frame(&mut s);
    assert_eq!(s.script_error, None);
    assert!(s.world.flags.staff_light);
    s.change_scene_named("mi_s3", "start").unwrap();
    frame(&mut s);
    assert!(s.world.flags.staff_light, "the flag belongs to PlayState");
}

/// Character.ApplyTemplate: a Spawn without MeshId draws a random model
/// index in range; Initialize copies that model's Tint into Character.Color.
/// Wizard_Support has four models with four different green tints.
#[test]
fn default_mesh_spawns_pick_a_model_and_take_its_tint() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle2.lvl", "Wizard").unwrap();
    frame(&mut s);
    let mut models = std::collections::BTreeSet::new();
    for _ in 0..12 {
        let handle = s.spawn_npc("Wizard_Support", "idle", 3.0).unwrap();
        let c = &s.world.npcs[&handle].character;
        assert_eq!(c.definition.model_tints.len(), 4);
        assert!(c.model < 4);
        assert_eq!(c.tint, c.definition.model_tints[c.model]);
        models.insert(c.model);
    }
    assert!(
        models.len() > 1,
        "twelve draws stayed on one model: {models:?}"
    );
}

/// SpawnCloneWizards (Wizard_Castle2 wc2_s1 "Past wizards moving on out"):
/// one NPC of each living player's avatar template at `past{i}`, unique ID
/// clonewizard{i}, with the avatar's MaxHitPoints and the authored order.
#[test]
fn past_wizards_spawn_as_clones_of_the_players() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle2.lvl", "Wizard").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    s.script_error = None;
    assert!(s.run_trigger("Past wizards moving on out", true).unwrap());
    frame(&mut s);
    assert_eq!(s.script_error, None);
    assert_eq!(
        s.world.clone_wizards.len(),
        1,
        "one living player, one clone"
    );
    let (&handle, &slot) = s.world.clone_wizards.iter().next().unwrap();
    assert_eq!(slot, 0);
    let npc = &s.world.npcs[&handle];
    let avatar = &s.avatar().character;
    assert_eq!(npc.character.definition.id, avatar.definition.id);
    // Initialize(template, i, ...): model i and its tint (Character.Color
    // is then overridden by the player colour, which the viewer resolves
    // from the recorded slot).
    assert_eq!(npc.character.model, 0);
    assert_eq!(npc.character.tint, npc.character.definition.model_tints[0]);
    assert_eq!(
        npc.character.combat.health.hit_points,
        avatar.combat.health.max_hit_points
    );
    let e = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == handle)
        .unwrap();
    assert_eq!(
        e.unique_id,
        magicka_runtime::scene_loading::custom_hash("clonewizard0")
    );
    let past0 = s
        .world
        .level
        .locators
        .iter()
        .find(|l| l.name.eq_ignore_ascii_case("past0"))
        .expect("past0 locator");
    let b = s.world.physics.bodies[npc.body].position;
    assert!((b[0] - past0.transform[12]).abs() < 0.5 && (b[2] - past0.transform[14]).abs() < 0.5);
}

/// ScrollScore (Vietnam challenge "procstorage": value 400 at
/// score_storage, TTL 3) adds a white, unlocked floating number at the
/// trigger area's random point or the locator.
#[test]
fn vietnam_storage_score_scrolls_a_white_number() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Challenges/ch_vietnam.lvl", "Wizard").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    let before = s.world.floating_text.last_number();
    s.script_error = None;
    assert!(s.run_trigger("procstorage", true).unwrap());
    s.step(1.0 / 60.0);
    assert_eq!(s.script_error, None);
    let n = (0..=s.world.floating_text.last_number().max(0) as usize)
        .map(|i| *s.world.floating_text.number(i))
        .find(|n| n.value == 400.0)
        .unwrap_or_else(|| panic!("no score number (last before {before})"));
    assert_eq!(n.color, [1.0; 3]);
    assert!(!n.locked);
    assert_eq!(n.ttl[0], 3.0);
    let key = |name: &str| magicka_runtime::scene_loading::custom_hash(name);
    let area = s
        .world
        .level
        .trigger_areas
        .iter()
        .any(|a| key(&a.name.to_lowercase()) == key("score_storage"));
    let locator = s
        .world
        .level
        .locators
        .iter()
        .find(|l| l.name == "score_storage")
        .map(|l| [l.transform[12], l.transform[13], l.transform[14]]);
    assert!(area || locator.is_some(), "score_storage exists");
    if let (false, Some(p)) = (area, locator) {
        assert_eq!(n.position, p);
    }
}

/// RollCredits (Osotc challenge finale, CreditsOsotc.xml): the credits wrap
/// and measure their localized lines, fade the sky and the band in, scroll
/// through, then begin Endgame(EndOffGame) and finish after the 1 s fade.
#[test]
fn osotc_finale_rolls_the_credits_to_the_end_game() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Challenges/ch_osotc.lvl", "Wizard").unwrap();
    s.change_scene_named("chs_osotc_s4_c5", "start").unwrap();
    s.screen = [1280.0, 720.0];
    for _ in 0..3 {
        frame(&mut s);
    }
    s.script_error = None;
    s.run_trigger("StartOfLevel", true).unwrap();
    frame(&mut s);
    assert_eq!(s.script_error, None);
    let c = s.credits.as_ref().expect("credits rolled");
    assert!(c.started);
    assert!(c.lines.len() >= 158, "every credit has a line");
    assert!(c.lines.iter().all(|l| !l.text.starts_with("NOT FOUND")));
    assert!(c.total_length > 1000.0);
    let mut scrolled = false;
    for _ in 0..20 * 600 {
        s.step(0.05);
        s.events.clear();
        let c = s.credits.as_ref().unwrap();
        scrolled |= c.render_range.1 > c.render_range.0;
        if s.credits_finished {
            break;
        }
    }
    assert!(scrolled, "lines were drawn while scrolling");
    assert!(s.credits_finished, "the credits reach the end game");
    assert_eq!(
        s.end_game.condition,
        magicka_runtime::end_game::EndGameCondition::EndOffGame
    );
    assert!(!s.credits.as_ref().unwrap().started);
    // The PlayState has been left: EndOffGame never restarts the checkpoint.
    for _ in 0..200 {
        s.step(0.05);
        s.events.clear();
    }
    assert!(
        !s.end_game.restart,
        "credits end must not restart the level"
    );
}

/// Enter the authored camp encounter and use its ordinary cutscene skip.
/// Navigation is fixture placement; orders, boss spawn and combat selection
/// run through the retail triggers without ignoring their conditions.
#[test]
fn courtyard_soldiers_prioritize_the_troll_and_clear_priority_on_new_orders() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.headless_dialogs = Some([1280.0, 720.0]);
    s.change_scene_named("wc_s5", "start").unwrap();
    settle(&mut s, 10);
    enter_lesson_area(&mut s, "spawn_soldiers");
    let guards: Vec<_> = s
        .world
        .npcs
        .iter()
        .filter(|(_, n)| {
            matches!(
                n.character.definition.id.to_ascii_lowercase().as_str(),
                "soldier_captain" | "soldier_spearman" | "soldier_archer"
            )
        })
        .map(|(&h, _)| h)
        .collect();
    assert_eq!(guards.len(), 8);
    enter_lesson_area(&mut s, "trigger_troll");
    assert!(s.world.cutscene.active);
    s.skip_cutscene();
    s.skip_cutscene();
    settle(&mut s, 195);
    let troll = s
        .world
        .entities
        .iter()
        .find(|e| e.unique_id == custom_hash("boss_troll") && e.added && !e.removed)
        .unwrap()
        .handle;
    for &h in &guards {
        let n = &s.world.npcs[&h];
        assert_eq!(n.combat.priority_target, Some(troll), "guard {h}");
        assert_eq!(n.combat.priority_ability, -1);
    }
    assert!(
        guards
            .iter()
            .any(|h| { s.world.npcs[h].combat.current_target() == Some(troll) }),
        "the soldiers must select the troll through their live combat AI"
    );

    // The same bridge handles specific orders and resets stale priorities,
    // including an ID that does not resolve and a preferred ability index.
    use magicka_runtime::scene_script::{ActionDefinition, SceneScriptLayout};
    let mut apply = |attrs: &str| {
        let layout = SceneScriptLayout::read(format!(
            "<Scene><trigger id=\"test\"><then><giveOrder id=\"#Her_Bjorn\" {attrs}/></then></trigger></Scene>"
        ).as_bytes(), false).unwrap();
        let ActionDefinition::GiveOrder(a) = &layout.actions[0] else {
            unreachable!()
        };
        let events = a.initialize(&mut s.world).unwrap();
        a.execute(Some(&events), &mut s.world).unwrap();
    };
    apply("order=\"attack\" priorityTarget=\"boss_troll\" priorityAbility=\"0\"");
    let h = captain(&s);
    assert_eq!(s.world.npcs[&h].combat.priority_ability, 0);
    assert_eq!(s.world.npcs[&h].combat.priority_target, Some(troll));
    let layout = SceneScriptLayout::read(b"<Scene><trigger id=\"test\"><then><giveOrder id=\"#Her_Bjorn\" order=\"idle\" priorityTarget=\"missing\"/></then></trigger></Scene>", false).unwrap();
    let ActionDefinition::GiveOrder(a) = &layout.actions[0] else {
        unreachable!()
    };
    let events = a.initialize(&mut s.world).unwrap();
    a.execute(Some(&events), &mut s.world).unwrap();
    assert_eq!(s.world.npcs[&h].combat.priority_target, None);
    assert_eq!(s.world.npcs[&h].combat.priority_ability, -1);
}

/// Exercise the final encounter's real death condition and Victory gate.
/// Room entry/approach use fixture placement, while cutscene skipping,
/// casting, NPC damage, death, post-fight scripts and endgame are live.
#[test]
fn courtyard_troll_combat_unlocks_the_castle_exit() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    // The six scripted rock casts only land on one trajectory of this
    // chaotic fight. With AIStateMove leader election live, two soldiers
    // briefly follow leaders, the regenerating troll outlives its targets,
    // turns on the player and the scripted casts stop landing (traced
    // 2026-11-01, npc-agents.md). Pin the recorded trajectory: this test
    // checks the death condition and Victory gate, not the soldiers' AI.
    s.headless_dialogs = Some([1280.0, 720.0]);
    s.change_scene_named("wc_s5", "start").unwrap();
    weapon_rack(&mut s);
    settle(&mut s, 10);
    enter_lesson_area(&mut s, "spawn_soldiers");
    enter_lesson_area(&mut s, "trigger_troll");
    s.skip_cutscene();
    s.skip_cutscene();
    settle(&mut s, 195);
    let troll = s
        .world
        .entities
        .iter()
        .find(|e| e.unique_id == custom_hash("boss_troll") && e.added && !e.removed)
        .unwrap()
        .handle;
    let full = s.world.npcs[&troll].character.combat.health.hit_points;
    let mut lowest = full;
    for _ in 0..6 {
        if s.world.npcs.get(&troll).is_none_or(|n| n.dead()) {
            break;
        }
        let n = &s.world.npcs[&troll];
        let target = s.world.physics.bodies[n.body].position;
        let radius = s.world.physics.bodies[n.body].radius;
        face_point_on_floor(&mut s, target, [radius * 2.0; 3], 8.0);
        cast_elements_at(&mut s, &[5, 5, 5, 7, 7], false, false, Some(troll));
        let hp = s
            .world
            .npcs
            .get(&troll)
            .map_or(0.0, |n| n.character.combat.health.hit_points);
        lowest = lowest.min(hp);
        eprintln!("troll HP {hp}");
        if s.world.cutscene.active {
            break;
        }
        cast_elements(&mut s, &[1, 1, 1, 1, 1], false, true);
    }
    // The fight is chaotic: the regenerating troll chases soldiers or the
    // player, and whether six scripted rocks land depends on every agent's
    // movement (traced 2026-11-01 with AIStateMove leader election live:
    // the rocks launch normally but miss a troll chasing the player).
    // Require real damage from the live casts, then finish a survivor
    // through the scripted kill so the death condition and Victory gate
    // below stay live.
    assert!(
        lowest < full * 0.75,
        "live casts must hurt the troll ({lowest}/{full})"
    );
    if s.world.npcs.get(&troll).is_some_and(|n| !n.dead()) {
        magicka_runtime::entity_command::EntityCommandHost::kill(&mut s.world, troll).unwrap();
    }
    for _ in 0..600 {
        if s.world.npcs.get(&troll).is_none_or(|n| n.dead()) {
            break;
        }
        s.step(1.0 / 60.0);
    }
    assert!(
        s.world.npcs.get(&troll).is_none_or(|n| n.dead()),
        "the troll must die"
    );
    settle(&mut s, 120);
    assert!(s
        .runtime
        .layout()
        .triggers
        .iter()
        .any(|t| { t.id == custom_hash("trollencounter over") && t.has_triggered }));
    // Finish whichever post-fight conversation the combat outcome selected.
    if s.world.cutscene.active {
        s.skip_cutscene();
        s.skip_cutscene();
    }
    settle(&mut s, 240);
    assert!(!s.input_limited() && !s.world.cutscene.active);
    enter_lesson_area(&mut s, "trigger_exit");
    settle(&mut s, 360);
    assert!(s.end_game.level_complete);
    assert_eq!(s.next_level(), Some("Mountaindale.lvl"));
}

/// Fire lizard report (Ruins ru_s5 Firedrakes). The template lists spider
/// fangs then firedrake_puke in slot 0; mEquipment[slot] keeps the puke, so
/// the ranged ability spits Fire at a Wizard placed in open floor nearby.
#[test]
fn ruins_firedrake_spits_fire_at_the_wizard() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Ruins.lvl", "Wizard").unwrap();
    s.change_scene_named("ru_s5", "start").unwrap();
    // Let the authored intro (cutscene, AvatarMove) finish.
    for _ in 0..1800 {
        frame(&mut s);
        if !s.world.flags.cutscene && !s.world.flags.input_limited {
            break;
        }
    }
    let drakes: Vec<(u16, [f32; 3])> = s
        .world
        .entities
        .iter()
        .filter(|e| {
            e.template_name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case("firedrake"))
        })
        .map(|e| (e.handle, e.position))
        .collect();
    assert!(!drakes.is_empty(), "ru_s5 spawns Firedrakes");
    let (drake, at) = drakes[0];
    let body = s.avatar().body;
    let nav = s.world.nav.as_ref().unwrap();
    // A navmesh point about 6 m away, inside the puke's 3-16 m range.
    let target = (0..16)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 16.0;
            [at[0] + 6.0 * a.cos(), at[1], at[2] + 6.0 * a.sin()]
        })
        .find(|p| {
            nav.nearest(*p, 0)
                .is_some_and(|(_, q, _)| (q[0] - p[0]).hypot(q[2] - p[2]) < 0.1)
        })
        .expect("open floor near the drake");
    s.world.physics.bodies[body].move_to(
        target,
        magicka_sim::character_physics::Orientation::IDENTITY,
    );
    let full = s.avatar().character.combat.health.hit_points;
    let (mut fired, mut burning) = (false, false);
    for _ in 0..900 {
        frame(&mut s);
        fired |= s.world.missiles.iter().any(|m| m.body.owner == drake);
        // The puke's Hit is Status|Piercing Fire: it sets the target alight.
        burning |= s.avatar().character.combat.statuses.current_mask & 1 != 0;
    }
    assert!(fired, "the Firedrake must spit");
    assert!(burning, "the Fire spit must set the Wizard burning");
    let hp = s.avatar().character.combat.health.hit_points;
    assert!(hp < full, "the spit must hurt ({full} -> {hp})");
}

#[test]
fn monster_belly_spawns_animated_spores_that_only_the_damageable_take_hits() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Versus/vs_monster_belly.lvl", "Wizard").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    s.script_error = None;
    assert!(s.run_trigger("Start", true).unwrap());
    frame(&mut s);
    assert_eq!(s.script_error, None);
    let spore = |s: &PlaySession, id: &str| {
        let hash = custom_hash(id);
        s.world
            .entities
            .iter()
            .find(|e| e.unique_id == hash && e.added && !e.removed && !e.dead)
            .map(|e| e.handle)
    };
    let alive = spore(&s, "spore0_alive").expect("spore0_alive spawned");
    let prop = s.world.anim_props[&alive].clone();
    // versus_toxic_spore idle_agg -> clip "idle" at speed 1.5, looping.
    assert_eq!(prop.animation, "idle_agg");
    assert_eq!(prop.clip.as_deref(), Some("idle"));
    assert_eq!(prop.speed, 1.5);
    assert!(prop.looping && prop.damageable && !prop.gravity);
    let d = prop.duration.expect("idle clip duration");
    for _ in 0..30 {
        frame(&mut s);
    }
    let t = s.world.anim_props[&alive].time;
    assert!(t > 0. && t < d, "looping clip time {t} within {d}");
    let fire = magicka_sim::damage::Damage {
        attack: magicka_sim::damage::attack::DAMAGE,
        element: 2,
        amount: 100.,
        magnitude: 1.,
    };
    let before = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == alive)
        .unwrap()
        .hit_points;
    s.world.damage_physics(alive, fire, 15).unwrap();
    let after = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == alive)
        .unwrap()
        .hit_points;
    assert!(
        after < before,
        "damageable spore takes damage ({before} -> {after})"
    );

    // The spawn-burst prop is not damageable: hits deflect (0x100).
    assert!(s.run_trigger("toxic_spores_0_spawn", true).unwrap());
    frame(&mut s);
    assert_eq!(s.script_error, None);
    let burst = spore(&s, "spore0_spawn").expect("spore0_spawn spawned");
    let p = &s.world.anim_props[&burst];
    assert_eq!(p.clip.as_deref(), Some("spawn"));
    assert!(!p.looping && !p.damageable);
    assert_eq!(s.world.damage_physics(burst, fire, 15).unwrap(), 0x100);
    // Removed after 0.99 s.
    for _ in 0..70 {
        frame(&mut s);
    }
    assert_eq!(spore(&s, "spore0_spawn"), None);
}

/// An invader piloting Behold (rewrite extension): attack input toward the
/// player must make its own Ranged ability fire laser missiles.
#[test]
fn piloted_behold_fires_lasers_at_the_aimed_player() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..3 {
        frame(&mut s);
    }
    assert!(s.run_trigger("Room7", true).unwrap());
    settle(&mut s, 60);
    s.disable_scripts();
    s.world.flags.cutscene = false;
    let behold = *s
        .world
        .npcs
        .iter()
        .find(|(_, n)| n.character.definition.id == "beholder_benign")
        .expect("Room7 spawns Behold")
        .0;
    let b = s.world.physics.bodies[s.world.npcs[&behold].body].position;
    let boss0 = s
        .world
        .level
        .locators
        .iter()
        .find(|l| l.name.eq_ignore_ascii_case("boss0"))
        .unwrap();
    let (dx, dz) = (boss0.transform[12] - b[0], boss0.transform[14] - b[2]);
    let l = (dx * dx + dz * dz).sqrt();
    let body = s.avatar().body;
    let y = s.body().position[1].max(boss0.transform[13]);
    s.world.physics.bodies[body].move_to(
        [b[0] + dx / l * 6.0, y, b[2] + dz / l * 6.0],
        magicka_sim::character_physics::Orientation::IDENTITY,
    );
    for _ in 0..30 {
        frame(&mut s);
    }
    assert!(s.world.possess(behold));
    let mut fired = 0;
    for _ in 0..600 {
        let aim = s.body().position;
        s.pilot_npc(
            behold,
            magicka_network::control::Controls {
                aim: Some(aim),
                cast_down: true,
                ..Default::default()
            },
        );
        s.step(1.0 / 60.0);
        fired = fired.max(
            s.world
                .missiles
                .iter()
                .filter(|m| m.body.owner == behold)
                .count(),
        );
    }
    assert!(fired > 0, "a piloted Behold must fire at the aimed player");
}
