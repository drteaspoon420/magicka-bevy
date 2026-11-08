use magicka_content::{
    level::{AnimatedLevelPart, Keyframe, LevelModel, NavMesh, TriMesh},
    skinned::{mul, pose_matrix, IDENTITY},
};
use magicka_runtime::{
    animated_hierarchy::PlayCommand, live_collision::*, scene_loading::custom_hash,
};
use magicka_sim::spray_pulse::Segment;

fn part(name: &str, z: f32) -> AnimatedLevelPart {
    AnimatedLevelPart {
        name: name.into(),
        affect_shields: false,
        model: None,
        mesh_settings: vec![],
        liquids: vec![],
        locators: vec![],
        animation_duration: 1.0,
        animation: vec![Keyframe {
            time: 0.0,
            translation: [0.0, 0.0, z],
            orientation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }],
        effects: vec![],
        lights: vec![],
        collision: None,
        nav_mesh: None,
        children: vec![],
    }
}
fn scene() -> LevelModel {
    let mut root = part("Root", 1.0);
    root.animation.push(Keyframe {
        time: 1.0,
        translation: [0.0, 0.0, 3.0],
        ..root.animation[0].clone()
    });
    let mut child = part("Door", 1.0);
    child.collision = Some((
        7,
        TriMesh {
            vertices: vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]],
            triangles: vec![[0, 1, 2]],
        },
    ));
    root.children.push(child);
    LevelModel {
        model: None,
        animated_parts: vec![root],
        lights: vec![],
        effects: vec![],
        physics_entities: vec![],
        liquids: vec![],
        force_fields: vec![],
        collision_layers: vec![TriMesh::default(); 10],
        camera_mesh: None,
        trigger_areas: vec![],
        locators: vec![],
        nav_mesh: NavMesh {
            vertices: vec![],
            triangles: vec![],
        },
    }
}
fn query(live: &LiveCollisionScene) -> f32 {
    let hit = live
        .collision()
        .segment_intersect(
            Segment {
                origin: [0.0; 3],
                delta: [0.0, 0.0, 8.0],
            },
            false,
        )
        .unwrap();
    assert_eq!(hit.primitive, 7);
    assert_eq!(hit.animated_part, Some(custom_hash("door")));
    hit.hit.fraction
}

#[test]
fn scene_script_animation_action_moves_live_collision_after_its_delay() {
    use magicka_runtime::{
        animation_action::AnimationAction, level_variables::LevelVariables,
        scene_script::SceneScriptLayout, script_runtime::*,
    };
    use magicka_sim::damage_delivery::NetworkRole;
    struct World(LiveCollisionScene);
    impl ScriptWorld for World {
        type Error = LiveCollisionError;
        fn facts(&self) -> ScriptWorldFacts {
            ScriptWorldFacts::default()
        }
        fn choose_group(&mut self, _: usize) -> Result<usize, Self::Error> {
            Ok(0)
        }
        fn notify_trigger(
            &mut self,
            _: &mut ScriptRuntime,
            _: i32,
            _: usize,
            _: Option<u16>,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        fn animation(
            &mut self,
            _: &mut ScriptRuntime,
            action: &AnimationAction,
        ) -> Result<(), Self::Error> {
            self.0.execute_animation_action(action)
        }
        fn action_executed(&mut self, _: &mut ScriptRuntime, _: usize) -> Result<(), Self::Error> {
            Ok(())
        }
    }
    let source = scene();
    let mut world = World(LiveCollisionScene::load(&source, &IDENTITY).unwrap());
    let layout = SceneScriptLayout::read(
        br#"<Scene><trigger id="start"><then>
        <playAnimation name="root" delay="0.25"/>
        </then></trigger></Scene>"#,
        false,
    )
    .unwrap();
    let mut runtime = ScriptRuntime::new(layout, LevelVariables::default());
    runtime
        .update(0.125, NetworkRole::Offline, &mut world)
        .unwrap();
    world.0.update(0.125, &IDENTITY).unwrap();
    assert_eq!(query(&world.0), 0.25);
    assert!(runtime.history().is_empty());
    runtime
        .update(0.125, NetworkRole::Offline, &mut world)
        .unwrap();
    world.0.update(0.5, &IDENTITY).unwrap();
    assert_eq!(query(&world.0), 0.375);
    assert_eq!(runtime.history(), [0]);
}

#[test]
fn full_inverse_retains_scale_rotation_translation_and_shear() {
    let mut matrix = pose_matrix([2.0, -3.0, 4.0], [0.0, 0.0, 0.6, 0.8], [2.0, 3.0, 4.0]);
    matrix[2] = 0.5;
    let inv = inverse(&matrix).unwrap();
    for product in [mul(&matrix, &inv), mul(&inv, &matrix)] {
        for (actual, expected) in product.into_iter().zip(IDENTITY) {
            assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
        }
    }
    assert_eq!(inverse(&[0.0; 16]), Err(InverseError::Singular));
    matrix[0] = f32::NAN;
    assert_eq!(inverse(&matrix), Err(InverseError::NonFinite));
}

#[test]
fn animation_updates_descendant_collision_without_rebuilding_geometry() {
    let source = scene();
    let mut live = LiveCollisionScene::load(&source, &IDENTITY).unwrap();
    assert_eq!(query(&live), 0.25);
    let mesh_address = &live.collision().animated_parts[0].children[0]
        .collision
        .as_ref()
        .unwrap()
        .mesh as *const _;
    let path = ["root".into()];
    live.play(
        &path,
        false,
        PlayCommand {
            start: 0.0,
            end: -1.0,
            speed: 1.0,
            looping: false,
            resume: false,
        },
    )
    .unwrap();
    live.update(0.5, &IDENTITY).unwrap();
    assert_eq!(query(&live), 0.375);
    live.stop(&path, true).unwrap();
    live.update(1.0, &IDENTITY).unwrap();
    assert_eq!(query(&live), 0.375);
    assert_eq!(
        mesh_address,
        &live.collision().animated_parts[0].children[0]
            .collision
            .as_ref()
            .unwrap()
            .mesh as *const _
    );
    assert!(matches!(
        live.play(
            &["unknown".into()],
            false,
            PlayCommand {
                start: 0.0,
                end: 1.0,
                speed: 1.0,
                looping: false,
                resume: false
            }
        ),
        Err(LiveCollisionError::MissingPart(_))
    ));
}

#[test]
fn scaled_parent_changes_world_hit_and_preserves_forward_scaled_normal() {
    let source = scene();
    let mut live = LiveCollisionScene::load(&source, &IDENTITY).unwrap();
    let parent = pose_matrix([0.0; 3], [0.0, 0.0, 0.0, 1.0], [2.0; 3]);
    live.update(0.0, &parent).unwrap();
    assert_eq!(query(&live), 0.5);
    let hit = live
        .collision()
        .segment_intersect(
            Segment {
                origin: [0.0; 3],
                delta: [0.0, 0.0, 8.0],
            },
            false,
        )
        .unwrap();
    assert_eq!(hit.hit.normal, [0.0, 0.0, 2.0]);
}

#[test]
fn singular_update_reports_path_and_preserves_last_valid_collision() {
    let source = scene();
    let mut live = LiveCollisionScene::load(&source, &IDENTITY).unwrap();
    let singular = pose_matrix([0.0; 3], [0.0, 0.0, 0.0, 1.0], [0.0; 3]);
    assert_eq!(
        live.update(0.0, &singular),
        Err(LiveCollisionError::Inverse {
            path: vec!["root".into(), "door".into()],
            error: InverseError::Singular
        })
    );
    assert_eq!(query(&live), 0.25);
    assert_eq!(live.parts()[0].world[0], 0.0);
    live.update(0.0, &IDENTITY).unwrap();
    assert_eq!(query(&live), 0.25);
}

#[test]
fn script_play_stop_resume_commands_drive_live_collision_and_preserve_loop_when_omitted() {
    use magicka_runtime::animation_action::AnimationAction;
    let source = scene();
    let mut live = LiveCollisionScene::load(&source, &IDENTITY).unwrap();
    let play =
        AnimationAction::read("playAnimation", [("name", "ROOT"), ("loop", "true")]).unwrap();
    live.execute_animation_action(&play).unwrap();
    live.update(0.5, &IDENTITY).unwrap();
    assert_eq!(query(&live), 0.375);
    let stop = AnimationAction::read("stopAnimation", [("name", "root")]).unwrap();
    live.execute_animation_action(&stop).unwrap();
    assert!(!live.parts()[0].playback.playing);
    assert!(!live.parts()[0].children[0].playback.playing);
    let resume = AnimationAction::read(
        "resumeAnimation",
        [
            ("name", "root"),
            ("speed", "-1"),
            ("length", "0.25"),
            ("children", "false"),
        ],
    )
    .unwrap();
    live.execute_animation_action(&resume).unwrap();
    assert!(live.parts()[0].playback.looping);
    live.update(0.25, &IDENTITY).unwrap();
    assert_eq!(query(&live), 0.3125);
    assert!(!live.parts()[0].children[0].playback.playing);
    let missing = AnimationAction::read("stopAnimation", [("name", "root/missing")]).unwrap();
    assert!(matches!(
        live.execute_animation_action(&missing),
        Err(LiveCollisionError::MissingAnimationId(_))
    ));
    let bad = AnimationAction::read("resumeAnimation", [("name", "root/door"), ("length", "-1")])
        .unwrap();
    assert_eq!(
        live.execute_animation_action(&bad),
        Err(LiveCollisionError::Resume(
            magicka_runtime::animated_pose::ResumeError::NegativeLength
        ))
    );
}

#[test]
fn delayed_animation_executes_at_boundary_and_only_notifies_after_success() {
    use magicka_runtime::animation_action::{AnimationAction, ScheduledAnimationAction};
    let source = scene();
    let mut live = LiveCollisionScene::load(&source, &IDENTITY).unwrap();
    let action =
        AnimationAction::read("playAnimation", [("name", "root"), ("delay", "0.5")]).unwrap();
    let mut scheduled = ScheduledAnimationAction::new(action);
    scheduled.trigger();
    assert_eq!(scheduled.update(0.25, &mut live, |_| panic!()).unwrap(), 0);
    assert!(!live.parts()[0].playback.playing);
    let mut notifications = 0;
    assert_eq!(
        scheduled
            .update(0.25, &mut live, |_| notifications += 1)
            .unwrap(),
        1
    );
    assert_eq!(notifications, 1);
    live.update(0.5, &IDENTITY).unwrap();
    assert_eq!(query(&live), 0.375);
    let bad = AnimationAction::read("stopAnimation", [("name", "missing")]).unwrap();
    let mut bad = ScheduledAnimationAction::new(bad);
    bad.trigger();
    assert!(bad.update(0.0, &mut live, |_| panic!()).is_err());
    assert!(bad.queue.finished());
}

#[test]
fn decals_follow_child_parent_motion_without_moving_same_named_other_owner() {
    use magicka_runtime::decals::{Decals, Request};
    let mut source = scene();
    source.animated_parts.push(part("Door", 8.0));
    let mut live = LiveCollisionScene::load(&source, &IDENTITY).unwrap();
    let mut decals = Decals::new(3, 0);
    let request = |owner| Request {
        cell: 20,
        scale: [2.0; 2],
        position: [0.0, 0.0, 2.0],
        normal: [0.0, 0.0, 1.0],
        direction: Some([1.0, 0.0, 0.0]),
        ttl: 60.0,
        color: [1.0; 4],
        animated_owner: owner,
    };
    let attached = decals.add(request(Some(1))).unwrap();
    let other = decals.add(request(Some(2))).unwrap();
    let ground = decals.add(request(None)).unwrap();
    let old = decals.get(attached).unwrap().transform;
    live.play(
        &["root".into()],
        false,
        PlayCommand {
            start: 0.0,
            end: -1.0,
            speed: 1.0,
            looping: false,
            resume: false,
        },
    )
    .unwrap();
    live.update_with_decals(0.5, &IDENTITY, &mut decals)
        .unwrap();
    assert_eq!(decals.get(attached).unwrap().transform[14], 3.0);
    assert_eq!(decals.get(other).unwrap().transform, old);
    assert_eq!(decals.get(ground).unwrap().transform, old);
    let p = decals.get(attached).unwrap();
    for (actual, expected) in mul(&p.transform, &p.inverse).iter().zip(IDENTITY) {
        assert!((actual - expected).abs() < 1e-6);
    }
}

#[test]
fn moving_floor_carries_multiple_characters_and_expires_lost_contact() {
    use magicka_sim::character_physics::{CharacterBody, Orientation, PhysicsWorld};
    let mut source = scene();
    let mut deck = part("Deck", 0.0);
    deck.animation.push(Keyframe {
        time: 1.0,
        translation: [8.0, 0.0, 0.0],
        ..deck.animation[0].clone()
    });
    deck.collision = Some((
        0,
        TriMesh {
            vertices: vec![
                [-20., 0., -20.],
                [-20., 0., 20.],
                [20., 0., 20.],
                [20., 0., -20.],
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        },
    ));
    source.animated_parts = vec![deck];
    let mut live = LiveCollisionScene::load(&source, &IDENTITY).unwrap();
    live.play(
        &["deck".into()],
        false,
        PlayCommand {
            start: 0.,
            end: -1.,
            speed: 1.,
            looping: false,
            resume: false,
        },
    )
    .unwrap();
    let mut physics = PhysicsWorld::new(-50.);
    for x in [-2., 2.] {
        let mut body = CharacterBody::new(0.5, 1., 80., 4.);
        body.move_to([x, 1., 0.], Orientation::IDENTITY);
        physics.bodies.push(body);
    }
    let dt = 1. / 60.;
    for _ in 0..30 {
        physics.integrate(dt, live.collision(), &|_, _| None);
        live.update(dt, &IDENTITY).unwrap();
        live.carry_characters(dt, &mut physics).unwrap();
    }
    for (i, x) in [-2., 2.].into_iter().enumerate() {
        assert!(
            (physics.bodies[i].position[0] - (x + 4.)).abs() < 0.01,
            "{:?}",
            physics.bodies[i].position
        );
    }
    // Leaving the deck retains the native grace interval, then releases it.
    physics.bodies[0].set_position([100., 10., 0.]);
    for _ in 0..20 {
        physics.integrate(dt, live.collision(), &|_, _| None);
        live.update(dt, &IDENTITY).unwrap();
        live.carry_characters(dt, &mut physics).unwrap();
    }
    let released = physics.bodies[0].position[0];
    for _ in 0..5 {
        physics.integrate(dt, live.collision(), &|_, _| None);
        live.update(dt, &IDENTITY).unwrap();
        live.carry_characters(dt, &mut physics).unwrap();
    }
    assert!((physics.bodies[0].position[0] - released).abs() < 0.001);
}
