use magicka_content::{
    level::Keyframe,
    skinned::{pose_matrix, IDENTITY},
};
use magicka_runtime::animated_pose::*;
fn key(t: f32, x: f32) -> Keyframe {
    Keyframe {
        time: t,
        translation: [x, 0.0, 0.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    }
}
#[test]
fn pose_interpolates_and_holds_final_key() {
    let k = [key(0.0, 0.0), key(1.0, 10.0)];
    assert_eq!(local_pose(&k, 0.5).unwrap()[12], 5.0);
    assert_eq!(local_pose(&k, 2.0).unwrap()[12], 10.0);
    assert_eq!(local_pose(&k, -1.0).unwrap(), IDENTITY);
    assert_eq!(local_pose(&[], 0.0), Err(PoseError::EmptyChannel));
}
#[test]
fn initialize_and_update_use_distinct_matrix_orders() {
    let keys = [key(0.0, 1.0)];
    let parent = pose_matrix([0.0; 3], [0.0, 0.0, 0.0, 1.0], [2.0; 3]);
    assert_eq!(initial_world_pose(&keys, 0.0, &parent).unwrap()[12], 1.0);
    assert_eq!(updated_world_pose(&keys, 0.0, &parent).unwrap()[12], 2.0);
}
#[test]
fn forward_loop_wraps_once_and_crossing_uses_previous_time() {
    let mut p = PartPlayback::default();
    p.play(1.0, 0.0, 1.0, 1.0, true, false);
    p.advance(3.0);
    assert_eq!(p.time, 2.0);
    p.advance(1.0);
    assert_eq!(p.time, 3.0);
    assert!(p.playing);
    assert_eq!(p.resting_timer, 1.0);
}
#[test]
fn reverse_loop_arithmetic_and_nonloop_boundary_are_preserved() {
    let mut p = PartPlayback::default();
    p.play(1.0, 0.0, 1.0, -1.0, true, false);
    p.advance(1.5);
    assert_eq!(p.time, -1.5);
    p.play(1.0, 0.0, 1.0, 1.0, false, false);
    p.advance(1.0);
    assert!(p.playing);
    p.advance(0.1);
    assert!(!p.playing);
    assert_eq!(p.time, 1.0);
}
#[test]
fn resume_clamps_directional_interval() {
    let mut p = PartPlayback {
        time: 5.0,
        ..Default::default()
    };
    p.play(4.0, -1.0, -1.0, -1.0, false, true);
    assert_eq!((p.start, p.end, p.time), (4.0, 0.0, 4.0));
    p.time = 2.0;
    p.play(4.0, 0.0, 4.0, 1.0, false, true);
    assert_eq!(p.time, 2.0);
}

#[test]
fn constructor_and_stop_preserve_playback_position() {
    let mut p = PartPlayback::new(4.0);
    assert_eq!((p.time, p.start, p.end, p.speed), (0.0, 0.0, 4.0, 1.0));
    assert!(!p.playing && !p.looping);
    assert_eq!(p.resting_timer, -1.0);
    p.play(4.0, 1.0, 3.0, 2.0, true, false);
    p.advance(0.25);
    p.stop();
    let stopped = p;
    p.advance(10.0);
    assert_eq!(p, stopped);
    assert_eq!(p.time, 1.5);
}

#[test]
fn resume_command_keeps_start_and_time_and_uses_directional_length() {
    let mut p = PartPlayback::new(10.0);
    p.start = 1.0;
    p.time = 3.0;
    p.resume(10.0, 2.0, -1.0, Some(true)).unwrap();
    assert_eq!((p.start, p.time, p.end), (1.0, 3.0, 1.0));
    assert!(p.playing && p.looping);
    p.resume(10.0, 2.0, 1.0, None).unwrap();
    assert_eq!(p.end, 5.0);
    assert!(p.looping);
    // Zero length uses full duration even during reverse playback.
    p.resume(10.0, 0.0, -1.0, Some(false)).unwrap();
    assert_eq!(p.end, 10.0);
    assert!(!p.looping);
}

#[test]
fn resume_uses_clr_subnormal_threshold_and_keeps_pre_error_mutations() {
    let mut p = PartPlayback::new(10.0);
    for length in [f32::from_bits(1), -f32::from_bits(1), f32::NAN] {
        p.resume(10.0, length, 1.0, None).unwrap();
        assert_eq!(p.end, 10.0);
    }
    p.time = 0.0;
    p.resume(10.0, f32::from_bits(2), 1.0, None).unwrap();
    assert_eq!(p.end.to_bits(), 2);
    p.stop();
    assert_eq!(
        p.resume(10.0, -f32::from_bits(2), -2.0, Some(true)),
        Err(ResumeError::NegativeLength)
    );
    assert_eq!(p.speed, -2.0);
    assert!(p.looping);
    assert!(!p.playing);
    assert_eq!(p.end.to_bits(), 2);
}
