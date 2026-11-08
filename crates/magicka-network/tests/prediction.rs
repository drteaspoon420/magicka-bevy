//! Plan milestone M8: prediction and adaptive presentation under latency.
use magicka_network::{
    control::Controls,
    prediction::{step, Predictor},
    presentation::{GamePresentation, INTERPOLATION_DELAY, MAX_DELAY},
};
use std::time::{Duration, Instant};

/// A host that applies inputs one tick after arrival and reports state
/// `latency` ticks later; the client predicts with the same speed.
#[test]
fn prediction_cuts_owned_motion_lag_under_latency_and_corrects_divergence() {
    let speed = 5.0;
    let dt = 1.0 / 60.0;
    for latency in [3usize, 6, 12] {
        let mut host = [0.0f32; 3];
        let mut predictor = Predictor::default();
        let mut sent: Vec<(u64, Controls)> = Vec::new();
        let mut states: Vec<([f32; 3], u64)> = Vec::new();
        let (mut predicted_error, mut delayed_error) = (0.0f32, 0.0f32);
        for tick in 1..=240u64 {
            // Walk in a square toward moving aim points.
            let aim = match (tick / 60) % 4 {
                0 => [100.0, 0.0, 0.0],
                1 => [100.0, 0.0, 100.0],
                2 => [-100.0, 0.0, 100.0],
                _ => [-100.0, 0.0, -100.0],
            };
            let controls = Controls {
                aim: Some(aim),
                move_down: tick % 90 < 70,
                ..Default::default()
            };
            sent.push((tick, controls));
            predictor.record(tick, dt, controls, speed);
            // The host applies the input sent `latency/2` ticks ago.
            if let Some((seq, c)) = sent.iter().rev().nth(latency / 2).copied() {
                host = step(host, &c, speed, dt);
                states.push((host, seq));
            }
            // The state reaches the client another `latency/2` ticks later.
            if let Some(&(pos, seq)) = states.iter().rev().nth(latency / 2) {
                predictor.reconcile(pos, seq, 1, speed, false);
                delayed_error += distance(pos, truth(&sent, speed, dt));
            }
            predictor.update(dt);
            if let Some(p) = predictor.visible() {
                predicted_error += distance(p, truth(&sent, speed, dt));
            }
        }
        // Prediction tracks the wizard's input-driven position far better
        // than showing the newest delayed authoritative state.
        assert!(
            predicted_error < delayed_error * 0.2,
            "latency {latency}: predicted {predicted_error} vs delayed {delayed_error}"
        );
    }
    // A host-side divergence (a wall, a knockback) is corrected smoothly,
    // and a large one snaps.
    let mut p = Predictor::default();
    p.reconcile([0.0; 3], 0, 1, 5.0, false);
    p.reconcile([0.5, 0.0, 0.0], 0, 1, 5.0, false);
    assert!(p.visible().unwrap()[0] < 0.01, "small corrections blend");
    for _ in 0..60 {
        p.update(1.0 / 60.0);
    }
    assert!((p.visible().unwrap()[0] - 0.5).abs() < 0.01);
    p.reconcile([10.0, 0.0, 0.0], 0, 1, 5.0, false);
    assert_eq!(p.visible().unwrap()[0], 10.0, "large errors snap");
    p.reconcile([20.0, 0.0, 0.0], 0, 2, 5.0, false);
    assert_eq!(p.visible().unwrap()[0], 20.0, "epoch changes snap");
}

/// What the wizard would be at if every sent input applied instantly.
fn truth(sent: &[(u64, Controls)], speed: f32, dt: f32) -> [f32; 3] {
    sent.iter()
        .fold([0.0; 3], |p, (_, c)| step(p, c, speed, dt))
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}

#[test]
fn adaptive_delay_grows_with_jitter_within_bounds_and_stays_fixed_on_clean_links() {
    let start = Instant::now();
    let mut clean = GamePresentation::adaptive();
    let mut jittery = GamePresentation::adaptive();
    let mut fixed = GamePresentation::default();
    let mut seed = 7u64;
    let mut arrival = start;
    for tick in 1..=300u64 {
        let mut frame = base(tick);
        frame.time = tick as f64 / 20.0;
        let on_time = start + Duration::from_millis(tick * 50);
        clean.apply(frame.clone(), on_time).unwrap();
        fixed.apply(frame.clone(), on_time).unwrap();
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let late = Duration::from_millis((seed >> 33) % 60);
        // Receive time is monotonic: a late packet delays the next one too.
        arrival = arrival.max(on_time + late);
        jittery.apply(frame, arrival).unwrap();
    }
    assert_eq!(fixed.delay(), INTERPOLATION_DELAY);
    assert_eq!(
        clean.delay(),
        INTERPOLATION_DELAY,
        "no jitter, no extra delay"
    );
    assert!(
        jittery.delay() > Duration::from_millis(80),
        "{:?}",
        jittery.delay()
    );
    assert!(jittery.delay() <= MAX_DELAY);
    assert!(jittery.jitter() > 0.01);
}

fn base(tick: u64) -> magicka_network::session::GameFrame {
    magicka_network::session::GameFrame {
        physics: magicka_network::PhysicsFrame {
            version: magicka_network::PROTOCOL_VERSION,
            session: 1,
            epoch: 1,
            tick,
            scene: "lab".into(),
            bodies: vec![],
        },
        level: "x.lvl".into(),
        players: vec![],
        npcs: vec![],
        effects: vec![],
        rules: None,
        lobby: Default::default(),
        participants: vec![],
        world: Default::default(),
        mod_ui: vec![],
        time: 0.0,
        events: vec![],
    }
}

#[test]
fn a_wizard_the_host_will_not_move_stops_being_predicted() {
    use magicka_network::{control::Controls, prediction::Predictor};
    let walk = Controls {
        aim: Some([100.0, 0.0, 0.0]),
        move_down: true,
        ..Default::default()
    };
    let (speed, dt) = (4.0, 1.0 / 60.0);
    let mut predictor = Predictor::default();
    predictor.reconcile([0.0; 3], 0, 1, speed, false);
    let mut sequence = 0;
    // The host applies every input and the wizard does not move: locked.
    for update in 0..8u64 {
        for _ in 0..3 {
            sequence += 1;
            predictor.record(sequence, dt, walk, speed);
        }
        // It has applied all but the newest few.
        predictor.reconcile([0.0; 3], sequence.saturating_sub(3), 1, speed, false);
        predictor.update(dt);
        if update >= 4 {
            assert!(predictor.is_stalled());
        }
    }
    for _ in 0..60 {
        predictor.update(dt);
    }
    let at = predictor.visible().unwrap();
    assert!(at[0].abs() < 0.01, "stays where the host has it: {at:?}");
    // The host moves it again: prediction resumes.
    for _ in 0..3 {
        sequence += 1;
        predictor.record(sequence, dt, walk, speed);
    }
    predictor.reconcile([0.5, 0.0, 0.0], sequence.saturating_sub(3), 1, speed, false);
    assert!(!predictor.is_stalled());
    sequence += 1;
    predictor.record(sequence, dt, walk, speed);
    // Once the small correction has blended away, it leads the host again.
    for _ in 0..60 {
        predictor.update(dt);
    }
    assert!(predictor.visible().unwrap()[0] > 0.5);
}
