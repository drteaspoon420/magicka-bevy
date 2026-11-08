use magicka_network::*;
use magicka_physics::{BodyKey, BodyState};
fn frame(epoch: u64, tick: u64, generation: u64, x: f32) -> PhysicsFrame {
    PhysicsFrame {
        version: PROTOCOL_VERSION,
        session: 1,
        epoch,
        tick,
        scene: "lab".into(),
        bodies: vec![RemoteBody::new(
            BodyKey {
                handle: 5,
                generation,
            },
            "Models/Items_Wizard/magickbook_major".into(),
            [0.; 3],
            BodyState::at([x, 1., 0.]),
        )],
    }
}
#[test]
fn updates_interpolate_but_stale_epochs_and_dead_lifetimes_cannot_return() {
    let mut r = PhysicsReplica::default();
    r.apply(frame(1, 1, 0, 0.)).unwrap();
    r.apply(frame(1, 2, 0, 10.)).unwrap();
    assert_eq!(r.sample(0.5)[0].position[0], 5.);
    assert_eq!(r.apply(frame(1, 1, 0, 0.)), Err(FrameError::Stale));
    let mut removed = frame(1, 3, 0, 0.);
    removed.bodies.clear();
    r.apply(removed).unwrap();
    assert!(r.sample(0.).is_empty());
    assert_eq!(r.apply(frame(1, 4, 0, 0.)), Err(FrameError::Lifetime));
    r.apply(frame(1, 5, 1, 20.)).unwrap();
    assert_eq!(r.sample(0.)[0].position[0], 20.);
    r.apply(frame(2, 0, 0, 30.)).unwrap();
    assert_eq!(r.sample(0.)[0].position[0], 30.);
    assert_eq!(r.apply(frame(1, 100, 2, 0.)), Err(FrameError::Stale));
    let mut other = frame(3, 0, 0, 40.);
    other.session = 2;
    assert_eq!(r.apply(other.clone()), Err(FrameError::Session));
    r.reset();
    r.apply(other).unwrap();
}
#[test]
fn invalid_frames_are_atomic_and_asset_paths_cannot_escape() {
    let mut r = PhysicsReplica::default();
    let valid = frame(1, 1, 0, 0.);
    r.apply(valid.clone()).unwrap();
    let mut bad = frame(1, 2, 0, 0.);
    bad.bodies[0].position[0] = f32::NAN;
    assert_eq!(r.apply(bad), Err(FrameError::InvalidBody));
    assert_eq!(r.current(), Some(&valid));
    let mut bad = frame(1, 2, 0, 0.);
    bad.bodies.push(bad.bodies[0].clone());
    assert_eq!(r.apply(bad), Err(FrameError::DuplicateHandle));
    assert!(canonical_asset("../../etc/passwd").is_err());
    assert!(canonical_asset("http://test").is_err());
    assert!(canonical_asset("/Models/item").is_err());
    assert_eq!(
        canonical_asset("Data/Items/Wizard/../../../Models/Items_Wizard/staff").unwrap(),
        "Models/Items_Wizard/staff"
    );
}
