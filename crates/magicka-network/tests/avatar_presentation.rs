//! Protocol 14: per-wizard draw inputs and one-shot pulses travel in the
//! world presentation and survive the wire unchanged.
use magicka_network::world::*;

fn strip(x: f32) -> Vec<[f32; 3]> {
    (0..TRAIL_VERTICES)
        .map(|i| [x + i as f32, (i % 2) as f32, 0.5 * i as f32])
        .collect()
}

fn sample() -> WorldPresentation {
    WorldPresentation {
        avatars: vec![
            AvatarPresentation {
                slot: 0,
                shield: Some(SelfShieldState {
                    kind: 1,
                    health: 40.0,
                    max_health: 60.0,
                    magnitude: 1.5,
                    ice_magnitude: 0.0,
                    earth_magnitude: 0.0,
                    noise_offsets: [[0.1, 0.2], [0.3, 0.4], [0.5, 0.6]],
                    time_since_damage: 0.05,
                    time_sensitive: true,
                }),
                halo: Some([0.2, 0.9, 0.3, 0.666]),
                ice_blade_trail: Some(strip(1.0)),
                arcane_blades: vec![ArcaneBladeState {
                    id: 7,
                    color: [0.5, 0.1, 0.9],
                    vertices: strip(2.0),
                }],
            },
            AvatarPresentation {
                slot: 3,
                shield: Some(SelfShieldState {
                    kind: 4,
                    ice_magnitude: 0.7,
                    earth_magnitude: 1.2,
                    ..Default::default()
                }),
                ..Default::default()
            },
        ],
        pulses: vec![
            AvatarPulse {
                serial: 1,
                slot: 0,
                kind: PulseKind::Zap,
                time: 12.5,
            },
            AvatarPulse {
                serial: 2,
                slot: 3,
                kind: PulseKind::Zap,
                time: 12.55,
            },
        ],
        ..Default::default()
    }
}

#[test]
fn avatar_records_and_pulses_round_trip_through_postcard_and_json() {
    let state = sample();
    assert!(state.valid());
    let mut buffer = vec![0u8; 1 << 16];
    let bytes: &[u8] = postcard::to_slice(&state, &mut buffer).unwrap();
    let back: WorldPresentation = postcard::from_bytes(bytes).unwrap();
    assert_eq!(back, state);
    let json = serde_json::to_vec(&state).unwrap();
    let back: WorldPresentation = serde_json::from_slice(&json).unwrap();
    assert_eq!(back, state);
    // Compact: a wizard with nothing to draw is not a record at all.
    assert!(AvatarPresentation {
        slot: 2,
        ..Default::default()
    }
    .is_empty());
    assert!(!state.avatars[1].is_empty());
    // Two records plus two pulses stay small on the wire.
    assert!(
        bytes.len() < 1024,
        "{} bytes for two avatars and two pulses",
        bytes.len()
    );
}

#[test]
fn malformed_avatar_records_and_pulses_are_rejected() {
    let mut state = sample();
    state.avatars[1].slot = 0;
    assert!(!state.valid(), "duplicate slots");

    let mut state = sample();
    state.avatars[0].ice_blade_trail.as_mut().unwrap().pop();
    assert!(!state.valid(), "trail length");

    let mut state = sample();
    state.avatars[0].arcane_blades[0].vertices[3][1] = f32::NAN;
    assert!(!state.valid(), "non-finite blade vertex");

    let mut state = sample();
    state.avatars[0].shield.as_mut().unwrap().kind = 5;
    assert!(!state.valid(), "unknown shield kind");

    let mut state = sample();
    state.avatars[0].shield.as_mut().unwrap().kind = 0;
    assert!(!state.valid(), "an inactive shield is None, not kind 0");

    let mut state = sample();
    state.pulses[1].serial = 1;
    assert!(!state.valid(), "duplicate pulse serials");

    let mut state = sample();
    state.pulses[0].time = f64::NAN;
    assert!(!state.valid(), "non-finite pulse time");

    let mut state = sample();
    state.pulses = (0..=MAX_PULSES as u32)
        .map(|i| AvatarPulse {
            serial: i + 1,
            slot: 0,
            kind: PulseKind::Zap,
            time: 1.0,
        })
        .collect();
    assert!(!state.valid(), "pulse cap");
}
