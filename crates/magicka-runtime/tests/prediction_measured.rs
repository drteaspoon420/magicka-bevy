//! Plan milestone M8 exit gate: measured improvement under latency against
//! the real authoritative wizard. A zero-latency twin session receives
//! every input at once; it is where the owned wizard ideally appears. The
//! host gets inputs `one_way` ticks late and its state reaches the client
//! another `one_way` ticks later.
use magicka_network::{control::Controls, prediction::Predictor};
use magicka_runtime::play_session::PlaySession;
use std::path::PathBuf;

fn load() -> PlaySession {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut s = PlaySession::load(
        &root.join("../../../../target/Magicka/Content"),
        "Havindr.lvl",
        "Wizard",
    )
    .unwrap();
    s.disable_scripts();
    for _ in 0..30 {
        s.step(1.0 / 60.0);
    }
    s
}
fn position(s: &PlaySession) -> [f32; 3] {
    s.world.physics.bodies[s.avatar().body].position
}
fn flat(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn owned_wizard_prediction_beats_delayed_authority_on_real_movement() {
    let dt = 1.0 / 60.0;
    for one_way in [3usize, 6] {
        let (mut host, mut twin) = (load(), load());
        let start = position(&host);
        let speed = {
            let b = &host.world.physics.bodies[host.avatar().body];
            b.max_speed * b.speed_multiplier()
        };
        let mut predictor = Predictor::default();
        let mut sent: Vec<Controls> = Vec::new();
        let mut states: Vec<([f32; 3], u64)> = Vec::new();
        let (mut predicted, mut delayed, mut samples) = (0.0f32, 0.0f32, 0);
        let mut shown_authority = None;
        for tick in 1..=240usize {
            // Run toward far points, turning every second.
            let a = (tick / 60) as f32 * 2.0;
            let controls = Controls {
                aim: Some([
                    start[0] + 40.0 * a.cos(),
                    start[1],
                    start[2] + 40.0 * a.sin(),
                ]),
                move_down: true,
                ..Default::default()
            };
            sent.push(controls);
            predictor.record(tick as u64, dt, controls, speed);
            twin.handle_remote_input(0, controls).unwrap();
            twin.step(dt);
            if tick > one_way {
                let seq = tick - one_way;
                host.handle_remote_input(0, sent[seq - 1]).unwrap();
                host.step(dt);
                states.push((position(&host), seq as u64));
            }
            if states.len() > one_way {
                let (pos, seq) = states[states.len() - 1 - one_way];
                predictor.reconcile(pos, seq, 1, speed, false);
                shown_authority = Some(pos);
            }
            predictor.update(dt);
            if let (Some(p), Some(auth)) = (predictor.visible(), shown_authority) {
                if tick > 60 {
                    predicted += flat(p, position(&twin));
                    delayed += flat(auth, position(&twin));
                    samples += 1;
                }
            }
        }
        let (p, d) = (predicted / samples as f32, delayed / samples as f32);
        eprintln!("one-way {one_way} ticks: mean error predicted {p:.3} m vs delayed {d:.3} m");
        assert!(samples > 100);
        assert!(
            p < d * 0.6,
            "prediction must cut owned-motion error: {p} vs {d}"
        );
    }
}
