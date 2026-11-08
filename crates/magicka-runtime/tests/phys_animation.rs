//! PhysAnimation and DamageablePhysicsEntity's health-driven part playback
//! (research/script-schema.md).
use magicka_runtime::phys_animation::*;
use magicka_runtime::scene_loading::custom_hash;

#[test]
fn reads_reflected_properties_with_native_defaults() {
    let a = PhysAnimationAction::read([("Id", "Gate"), ("name", "Castle/Gate_L")]).unwrap();
    assert_eq!(a.entity, custom_hash("gate"));
    assert_eq!(a.path, [custom_hash("castle"), custom_hash("gate_l")]);
    assert_eq!(
        a.control,
        PhysAnimationControl {
            start: 0.0,
            end: 0.0,
            speed: 1.0,
            children: true
        }
    );
    let a = PhysAnimationAction::read([
        ("speed", "2"),
        ("start", "1"),
        ("end", "3"),
        ("children", "False"),
        ("delay", "0.25"),
    ])
    .unwrap();
    assert_eq!(
        (a.delay, a.control.speed, a.control.children),
        (0.25, 2.0, false)
    );
    assert!(PhysAnimationAction::read([("children", "1")]).is_err());
}

#[test]
fn health_changes_scrub_between_start_and_end() {
    let control = PhysAnimationControl {
        start: 1.0,
        end: 3.0,
        speed: 2.0,
        children: false,
    };
    let mut a = PhysAnimations::new(100.0);
    // No controls, or no health change: no playback.
    assert!(a.update(50.0, 100.0).is_empty());
    a.add(vec![7], control);
    assert!(a.update(100.0, 100.0).is_empty());
    // The previous fraction starts at 1 (End), so the first hit plays back
    // from End to the 25% point with negated speed.
    let plays = a.update(75.0, 100.0);
    assert_eq!(plays.len(), 1);
    let (path, children, p) = &plays[0];
    assert_eq!((path.as_slice(), *children), (&[7][..], false));
    assert_eq!(
        (p.start, p.end, p.speed, p.looping, p.resume),
        (1.5, 3.0, -2.0, false, false)
    );
    // Further damage plays forward from the last fraction.
    let p = a.update(25.0, 100.0)[0].2;
    assert_eq!((p.start, p.end, p.speed), (1.5, 2.5, 2.0));
    // Healing plays backwards again.
    let p = a.update(50.0, 100.0)[0].2;
    assert_eq!((p.start, p.end, p.speed), (2.0, 2.5, -2.0));
}
