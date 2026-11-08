use magicka_sim::napalm::*;

#[test]
fn napalm_waits_five_seconds_then_six_hits_cross_the_target() {
    let mut n = Napalm::new(1, [0.0, 0.0, 0.0], [0.0, 0.0, -1.0]);
    assert_eq!(n.target, [0.0, 0.0, -15.0]);
    // Facing -Z turned 90° about Y: the track runs along -X.
    assert_eq!(n.forward, [-1.0, 0.0, 0.0]);
    let dt = 1.0 / 60.0;
    let mut events = vec![];
    let mut frames = 0;
    while !n.airstrike {
        assert!(!n.update(dt, true, &mut events));
        frames += 1;
    }
    assert_eq!(events, [NapalmEvent::PlanePass]);
    assert!((299..=301).contains(&frames), "5 s wait: {frames}");
    events.clear();
    let mut dead = false;
    while !dead {
        dead = n.update(dt, true, &mut events);
    }
    let strikes: Vec<[f32; 3]> = events
        .iter()
        .filter_map(|e| match e {
            NapalmEvent::Strike(p) => Some(*p),
            _ => None,
        })
        .collect();
    assert_eq!(strikes.len(), 6, "{strikes:?}");
    for p in &strikes {
        assert!(p[1] == 0.0 && p[2] == -15.0, "on the track: {p:?}");
        assert!(p[0].abs() < 12.0, "near the target: {p:?}");
    }
    // The Lerp runs from target + forward·120 to target - forward·120:
    // against the focal Forward, here towards +X.
    assert!(strikes.windows(2).all(|w| w[1][0] > w[0][0]));
    // A client gets the plane pass but no hits.
    let mut c = Napalm::new(1, [0.0; 3], [0.0, 0.0, -1.0]);
    let mut events = vec![];
    while !c.update(dt, false, &mut events) {}
    assert_eq!(events, [NapalmEvent::PlanePass]);
}
