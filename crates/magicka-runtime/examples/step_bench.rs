//! What `PlaySession::step` costs, away from rendering (`research/performance.md`).
//!
//!     cargo run --locked --release -p magicka-runtime --example step_bench -- <content-dir>
//!
//! Reports time per tick and when the worst ticks happen, for a level and NPC count.
use magicka_runtime::play_session::PlaySession;
use std::{
    hash::{Hash, Hasher},
    io::Write,
    path::Path,
    time::Instant,
};

const TICKS: usize = 1200;
const HITCH_MS: f64 = 16.7;

/// Bytes this process has read, including from disk (Linux; 0 elsewhere).
fn bytes_read() -> u64 {
    std::fs::read_to_string("/proc/self/io")
        .ok()
        .and_then(|io| {
            io.lines()
                .find_map(|l| l.strip_prefix("rchar: ")?.parse().ok())
        })
        .unwrap_or(0)
}

fn run(content: &Path, level: &str, npcs: usize) {
    // Optional deterministic state trace for before/after comparisons. Hashing
    // is outside the measured step; run timings separately with tracing off.
    // These hashes compare this scenario, not complete simulation parity, and
    // are only comparable with the same Rust toolchain and trace format.
    let args: Vec<String> = std::env::args().collect();
    let mut trace = args
        .iter()
        .position(|a| a == "--trace")
        .map(|i| std::io::BufWriter::new(std::fs::File::create(&args[i + 1]).expect("trace file")));
    let mut s = match PlaySession::load(content, level, "Wizard") {
        Ok(s) => s,
        Err(e) => {
            println!("{level}: load failed {e:?}");
            return;
        }
    };
    s.disable_scripts();
    for _ in 0..30 {
        s.step(1. / 60.);
    }
    let mut spawned = 0;
    for i in 0..npcs {
        if s.spawn_npc("Goblin_pirate", "attack", 4.0 + (i % 10) as f32)
            .is_ok()
        {
            spawned += 1;
        }
    }
    for _ in 0..60 {
        s.step(1. / 60.);
    }
    let mut times = Vec::with_capacity(TICKS);
    let mut died = None;
    let mut reads = Vec::new();
    for _ in 0..TICKS {
        let read_before = bytes_read();
        let start = Instant::now();
        s.step(1. / 60.);
        times.push(start.elapsed().as_secs_f64() * 1000.);
        let read = bytes_read() - read_before;
        if let Some(trace) = trace.as_mut() {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            format!("{:?}{:?}", s.world.entities, s.world.combatants()).hash(&mut hash);
            for (&handle, npc) in &s.world.npcs {
                handle.hash(&mut hash);
                for (slot, weapon) in &npc.weapons {
                    slot.hash(&mut hash);
                    weapon.hitlist.hash(&mut hash);
                }
            }
            for player in s.world.players.iter().flatten() {
                player.handle.hash(&mut hash);
                for (slot, item) in &player.items {
                    slot.hash(&mut hash);
                    item.melee.hitlist.hash(&mut hash);
                }
            }
            writeln!(trace, "{} {:016x}", times.len() - 1, hash.finish()).expect("trace write");
        }
        // Reading /proc/self/io itself counts for about 100 bytes.
        if read > 1000 {
            reads.push((times.len() - 1, read));
        }
        if died.is_none() && s.dead() {
            died = Some(times.len() - 1);
        }
    }
    let hitches: Vec<(usize, f64)> = times
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, t)| *t > HITCH_MS)
        .collect();
    let mean = times.iter().sum::<f64>() / TICKS as f64;
    let mut sorted = times;
    sorted.sort_by(f64::total_cmp);
    println!(
        "{level} npcs={spawned} live={} mean={mean:.2} p50={:.2} p95={:.2} p99={:.2} max={:.2} ms; {} over {HITCH_MS} ms",
        s.world.npcs.len(),
        sorted[TICKS / 2],
        sorted[TICKS * 95 / 100],
        sorted[TICKS * 99 / 100],
        sorted[TICKS - 1],
        hitches.len(),
    );
    if !reads.is_empty() {
        println!("  file reads during steps (tick, bytes): {reads:?}");
    }
    if !hitches.is_empty() && hitches.len() < 40 {
        let list: Vec<String> = hitches
            .iter()
            .map(|(tick, t)| format!("{tick}:{t:.0}"))
            .collect();
        println!(
            "  hitches (tick:ms): {}; wizard died at {died:?}",
            list.join(" ")
        );
    }
}

fn main() {
    let content = std::env::args().nth(1).expect("content directory");
    let content = Path::new(&content);
    // `--bench-one <level> <npcs> [--trace <path>]`: one run, for a profiler
    // or deterministic state comparison. Traces are overwritten on each run.
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--bench-one") {
        run(
            content,
            &args[i + 1],
            args[i + 2].parse().expect("npc count"),
        );
        return;
    }
    let quick = std::env::args().any(|a| a == "--quick");
    for level in ["Wizard_Castle.lvl", "Havindr.lvl", "Mountaindale.lvl"] {
        let counts: &[usize] = if quick { &[30] } else { &[0, 30, 100, 300] };
        for &npcs in counts {
            run(content, level, npcs);
        }
    }
}
