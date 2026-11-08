use magicka_content::{
    level::{AnimatedLevelPart, Keyframe},
    skinned::{pose_matrix, IDENTITY},
};
use magicka_runtime::{animated_hierarchy::*, animated_pose::ResumeError};

fn part(name: &str, duration: f32, x: f32) -> AnimatedLevelPart {
    AnimatedLevelPart {
        name: name.into(),
        affect_shields: false,
        model: None,
        mesh_settings: vec![],
        liquids: vec![],
        locators: vec![],
        animation_duration: duration,
        animation: vec![Keyframe {
            time: 0.0,
            translation: [x, 0.0, 0.0],
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

#[test]
fn stopped_child_follows_parent_and_initialization_keeps_distinct_order() {
    let mut root = part("Root", 1.0, 1.0);
    root.animation.push(Keyframe {
        time: 1.0,
        translation: [3.0, 0.0, 0.0],
        ..root.animation[0].clone()
    });
    root.children.push(part("Child", 1.0, 2.0));
    let sources = [root];
    let parent = pose_matrix([0.0; 3], [0.0, 0.0, 0.0, 1.0], [2.0; 3]);
    let mut states = AnimatedPartState::load(&sources, &parent).unwrap();
    assert_eq!(states[0].world[12], 1.0);
    assert_eq!(states[0].children[0].world[12], 3.0);
    states[0].play(
        false,
        PlayCommand {
            start: 0.0,
            end: -1.0,
            speed: 1.0,
            looping: false,
            resume: false,
        },
    );
    states[0].update(0.5, &parent).unwrap();
    assert_eq!(states[0].world[12], 4.0);
    assert_eq!(states[0].previous_world[12], 1.0);
    let child = &states[0].children[0];
    assert_eq!(child.world[12], 8.0);
    assert_eq!(child.playback.time, 0.0);
    assert!(!child.playback.playing);
    let path = ["root".into(), "child".into()];
    assert_eq!(
        AnimatedPartState::find(&states, &path).unwrap().world,
        child.world
    );
    assert!(AnimatedPartState::find(&states, &["missing".into()]).is_none());
}

#[test]
fn play_resumes_parent_but_resets_all_descendants_using_their_durations() {
    let mut root = part("root", 4.0, 0.0);
    let mut child = part("child", 2.0, 0.0);
    child.children.push(part("leaf", 1.0, 0.0));
    root.children.push(child);
    let sources = [root];
    let mut states = AnimatedPartState::load(&sources, &IDENTITY).unwrap();
    let root = &mut states[0];
    root.playback.time = 3.0;
    root.children[0].playback.time = 1.5;
    root.children[0].children[0].playback.time = 0.5;
    root.play(
        true,
        PlayCommand {
            start: -1.0,
            end: -1.0,
            speed: 1.0,
            looping: true,
            resume: true,
        },
    );
    assert_eq!(root.playback.time, 3.0);
    assert_eq!(root.children[0].playback.time, 0.0);
    assert_eq!(root.children[0].playback.end, 2.0);
    assert_eq!(root.children[0].children[0].playback.time, 0.0);
    assert_eq!(root.children[0].children[0].playback.end, 1.0);
    root.stop(false);
    assert!(root.children[0].playback.playing);
    root.stop(true);
    assert!(!root.children[0].children[0].playback.playing);
}

#[test]
fn recursive_resume_propagates_first_child_error_before_mutating_parent() {
    let mut root = part("root", 4.0, 0.0);
    root.children = vec![part("first", 2.0, 0.0), part("second", 2.0, 0.0)];
    let sources = [root];
    let mut states = AnimatedPartState::load(&sources, &IDENTITY).unwrap();
    let root = &mut states[0];
    assert_eq!(
        root.resume(true, -1.0, -2.0, Some(true)),
        Err(ResumeError::NegativeLength)
    );
    assert_eq!(root.playback.speed, 1.0);
    assert_eq!(root.children[0].playback.speed, -2.0);
    assert!(root.children[0].playback.looping);
    assert_eq!(root.children[1].playback.speed, 1.0);
    root.resume(true, 0.0, 1.0, None).unwrap();
    assert!(root.playback.playing && root.children[1].playback.playing);
}

#[test]
fn invalid_channels_names_and_duplicate_sibling_ids_are_explicit() {
    let mut empty = part("empty", 1.0, 0.0);
    empty.animation.clear();
    assert!(matches!(
        AnimatedPartState::load(&[empty], &IDENTITY),
        Err(HierarchyError::Pose { .. })
    ));
    assert!(matches!(
        AnimatedPartState::load(&[part("RÖOT", 1.0, 0.0)], &IDENTITY),
        Err(HierarchyError::UnsupportedName(_))
    ));
    assert!(matches!(
        AnimatedPartState::load(&[part("ROOT", 1.0, 0.0), part("root", 1.0, 0.0)], &IDENTITY),
        Err(HierarchyError::DuplicateId(_))
    ));
    let mut root = part("root", 1.0, 0.0);
    root.children.push(part("root", 1.0, 0.0));
    assert!(AnimatedPartState::load(&[root], &IDENTITY).is_ok());
}
