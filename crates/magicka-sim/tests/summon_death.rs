use magicka_sim::{spells::Elements, summon_death::*};

#[test]
fn afterimages_catch_up_with_current_pose_freeze_below_gate_and_retain_recast_timers() {
    let mut trail = Afterimages::default();
    assert!(!trail.update(0.2, 0.5, 8.0, 1));
    assert_eq!(trail.timer, 0.0);
    assert!(trail.update(0.08, 1.0, 8.0, 2));
    assert_eq!(trail.poses, [Some(2), Some(2), Some(2), None, None]);
    assert!(trail.timer > 0.0 && trail.timer <= 0.0375);
    assert!(trail.intensity > 0.0 && trail.intensity <= 0.05);
    let timers = (trail.timer, trail.intensity);
    assert!(!trail.update(1.0, 0.0, 8.0, 3));
    assert_eq!((trail.timer, trail.intensity), timers);
    trail.clear_poses();
    assert_eq!(trail.poses, [None; 5]);
    assert_eq!((trail.timer, trail.intensity), timers);
    trail.update(1.0, 1.0, 0.0, 4);
    assert!(trail.intensity < 0.0);
    assert!(trail.draw_alphas().iter().all(|alpha| *alpha < 0.0));
}

#[test]
fn target_uses_health_fraction_rotated_ties_and_unfiltered_owner_fallback() {
    let c = |handle, hp| Candidate {
        handle,
        character: true,
        dead: false,
        undead: false,
        hit_points: hp,
        max_hit_points: 100.0,
    };
    let entries = [c(1, 100.0), c(2, 50.0), c(3, 50.0)];
    assert_eq!(select_target(&entries, 0, Some(9)), Some(3));
    assert_eq!(select_target(&entries, 2, Some(9)), Some(2));
    let mut excluded = c(4, 1.0);
    excluded.undead = true;
    assert_eq!(select_target(&[excluded], 0, Some(9)), Some(9));
    assert_eq!(select_target(&[], 0, None), None);
}

#[test]
fn life_hit_waits_for_fall_and_disappearance_restores_time_target() {
    let mut death = MagickDeath::new(2, [0.0; 3], 0.6);
    let target = Some(Target {
        position: death.position,
        dead: false,
    });
    assert!(!death.receive_damage(Elements::LIFE, true));
    let step = death.update(0.1, death.position[1], target, AnimationClock::default());
    assert_eq!(step.crossfade, Some((Phase::ScytheRise, 0.33)));
    assert!(death.receive_damage(Elements::LIFE, true));
    let finished = AnimationClock {
        finished: true,
        crossfade: false,
        normalized_time: 1.0,
    };
    death.update(0.1, death.position[1], target, finished);
    assert_eq!(death.phase, Phase::ScytheFall);
    death.update(0.1, death.position[1], target, AnimationClock::default());
    assert_eq!(death.phase, Phase::Disappear);
    assert_eq!(death.time_target, 0.25);
    let step = death.update(0.1, death.position[1], target, AnimationClock::default());
    assert!(step.despawn_effect_and_cue);
    assert_eq!(death.time_target, 1.0);
    let mut modifier = 0.25;
    death.remove(&mut modifier);
    assert_eq!(modifier, 1.0);
}
