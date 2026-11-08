//! Reproducible solver-cost measurement, not a frame-rate or network benchmark.
use magicka_physics::{BodyKey, BodyMode, BodyState, RigidWorld};
use std::time::Instant;
fn main() {
    let count: u16 = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1024".into())
        .parse()
        .expect("body count (1..65535)");
    assert!(count > 0);
    let mut world = RigidWorld::new();
    world
        .add_mesh(
            &[
                [-100., 0., -100.],
                [-100., 0., 100.],
                [100., 0., 100.],
                [100., 0., -100.],
            ],
            &[[0, 1, 2], [0, 2, 3]],
        )
        .unwrap();
    for handle in 0..count {
        // Falling grid, then a settled/sleeping grid; one metre cubes.
        let column = usize::from(handle) % 32;
        let row = usize::from(handle) / 32;
        world
            .spawn_box(
                BodyKey {
                    handle,
                    generation: 0,
                },
                BodyMode::Dynamic,
                [1.; 3],
                1.,
                BodyState::at([
                    column as f32 * 2. - 32.,
                    5. + (row % 4) as f32,
                    row as f32 * 2. - 32.,
                ]),
            )
            .unwrap();
    }
    let mut active = Vec::new();
    let mut settled = Vec::new();
    for tick in 0..480 {
        let start = Instant::now();
        world.step(1. / 60.).unwrap();
        let ms = start.elapsed().as_secs_f64() * 1000.;
        if (30..150).contains(&tick) {
            active.push(ms);
        }
        if tick >= 360 {
            settled.push(ms);
        }
    }
    for (name, mut values) in [("falling/contact", active), ("settled", settled)] {
        values.sort_by(f64::total_cmp);
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        println!(
            "{name}: bodies={count} mean_ms={mean:.3} p95_ms={:.3} max_ms={:.3}",
            values[values.len() * 95 / 100],
            values.last().unwrap()
        );
    }
    let states = world.snapshot();
    assert!(states
        .iter()
        .all(|(_, s)| s.position.iter().all(|v| v.is_finite())));
    println!(
        "sleeping={} total={} ticks={}",
        states.iter().filter(|(_, s)| s.sleeping).count(),
        states.len(),
        world.ticks
    );
}
