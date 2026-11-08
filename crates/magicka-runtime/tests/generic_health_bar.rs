//! Havindr's scripted dynamite timer (HealthBar/ActivateHealthBar actions on
//! PlayState.GenericHealthBar). See research/generic-health-bar.md.
use magicka_runtime::play_session::PlaySession;
use std::path::Path;

#[test]
fn havindr_dynamite_timer_burns_for_180_seconds_after_its_delay() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.change_scene_named("havindr_s6", "start").unwrap();
    s.world.flags.cutscene = false;
    assert!(s.run_trigger("Encounter begins", true).unwrap());
    let dt = 1. / 60.;
    let mut t = 0.0f32;
    // The HealthBar action (delay 1) configures it, ActivateHealthBar
    // (delay 1.1) shows it; ticking starts after the 4 s initial delay.
    while !s.world.generic_health_bar.active {
        s.step(dt);
        t += dt;
        assert!(t < 3.0, "bar never activated");
    }
    let bar = &s.world.generic_health_bar;
    assert!(bar.config.dynamite && bar.config.is_scaled && bar.config.has_animated_sprite);
    assert_eq!(bar.config.time, 180.);
    assert_eq!(bar.config.fade_time, 5.);
    // Native defaults the script does not override.
    assert_eq!(bar.config.animation_offset_y, 16.);
    assert_eq!(bar.config.power, 1.1);
    assert_eq!(bar.dynamite_shift, 40.);
    assert!(!bar.ticking);
    for _ in 0..(4.5 / dt) as usize {
        s.step(dt);
    }
    let bar = &s.world.generic_health_bar;
    assert!(bar.ticking, "timer did not start after its 4 s delay");
    assert!(
        bar.alpha > 0.85 && bar.alpha < 1.0,
        "fade-in over 5 s: {}",
        bar.alpha
    );
    let frames = bar.sprite_frames;
    assert!(frames >= 30, "fuse animated {frames} frames in ~4.5 s");
    let early = bar.display_health;
    for _ in 0..(60. / dt) as usize {
        s.step(dt);
    }
    let bar = &s.world.generic_health_bar;
    let burned = early - bar.display_health;
    eprintln!(
        "display health {early} -> {} after 60 s",
        bar.display_health
    );
    assert!((burned - 60. / 180.).abs() < 0.01, "burned {burned}");
    assert!(bar.alpha == 1.0 && !bar.done);
}
